//! Budgeted substitution of a persistent condition-valued index.

use super::{Filter, FilterStatus, Root, Store};
use crate::condition::{Arena, Condition, Progress, Transform, cleanup_substitution};
use crate::gc::discard_slot;
use crate::trace::{Cursor, Step, Trace};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Keep `roots()` in store collection, and trace both collected leaf values and
/// `condition_roots()` in arena collection until this continuation is released.
/// Completion preserves the result root; discard abandons staged output.
pub struct Substitution {
    owner: u32,
    filter: Option<Filter<Condition>>,
    bindings: Option<Arc<BTreeMap<u64, Condition>>>,
    draining: BTreeMap<u64, Condition>,
    boolean: Option<Transform>,
    operand: Condition,
    // One frozen substitution shares images across all repeated leaf supports.
    memo: BTreeMap<Condition, Condition>,
    // A terminal substituteion validates bindings and binds the arena on the
    // first tick. Once complete it enforces arena identity and its GC lease.
    arena_guard: Option<Transform>,
    result: Option<Root>,
    done: bool,
    discarding: bool,
}

impl Store<Condition> {
    pub fn substitute(&self, root: Root, bindings: Arc<BTreeMap<u64, Condition>>) -> Substitution {
        Substitution {
            owner: self.owner,
            filter: Some(self.filter(root)),
            bindings: Some(bindings),
            draining: BTreeMap::new(),
            boolean: None,
            operand: Condition::FALSE,
            memo: BTreeMap::new(),
            arena_guard: None,
            result: None,
            done: false,
            discarding: false,
        }
    }
}

impl Substitution {
    pub fn roots(&self) -> impl Iterator<Item = Root> + '_ {
        self.filter
            .iter()
            .flat_map(Filter::roots)
            .chain(self.result.clone())
    }

    pub fn condition_roots(&self) -> impl Iterator<Item = Condition> + '_ {
        self.filter
            .iter()
            .flat_map(Filter::values)
            .chain([self.operand])
            .chain(
                self.memo
                    .iter()
                    .flat_map(|(&input, &output)| [input, output]),
            )
            .chain(self.boolean.iter().flat_map(Transform::roots))
            .chain(self.arena_guard.iter().flat_map(Transform::roots))
            .chain(
                self.bindings
                    .iter()
                    .flat_map(|images| images.values().copied()),
            )
            .chain(self.draining.values().copied())
    }

    fn cleanup_tick(&mut self) -> bool {
        cleanup_substitution(&mut self.memo, &mut self.bindings, &mut self.draining)
    }

    /// Cancel without traversing another leaf or evaluating another substitution.
    /// Each call drains at most one nested job step or one assignment entry.
    pub fn discard_tick(&mut self) -> bool {
        self.discarding = true;
        self.filter = None; // Filter frames are bounded scalar index handles.
        self.result = None;
        self.operand = Condition::FALSE;
        if discard_slot(&mut self.boolean, |child| child.discard_tick()) {
            return false;
        }
        if discard_slot(&mut self.arena_guard, |child| child.discard_tick()) {
            return false;
        }
        self.done = self.cleanup_tick();
        self.done
    }

    /// Advance one filter transition, one Boolean substituteion step, or one
    /// cleanup step. Returns only after assignment cleanup has completed.
    pub fn tick(&mut self, store: &mut Store<Condition>, arena: &mut Arena) -> Option<Root> {
        assert!(!self.discarding, "index substituteion has been discarded");
        assert_eq!(self.owner, store.owner, "foreign index substituteion");
        store.assert_mutable();
        let guard = self.arena_guard.get_or_insert_with(|| {
            arena.substitute(Condition::TRUE, self.bindings.as_ref().unwrap().clone())
        });
        if guard.tick(arena) == Progress::Pending {
            return None;
        }
        if self.done {
            return self.result.clone();
        }
        if let Some(boolean) = &mut self.boolean {
            if let Progress::Complete(value) = boolean.tick(arena) {
                self.memo.insert(self.operand, value);
                self.operand = Condition::FALSE;
                self.filter
                    .as_mut()
                    .unwrap()
                    .replace((value != Condition::FALSE).then_some(value));
                self.boolean = None;
            }
        } else if let Some(filter) = &mut self.filter {
            match filter.tick(store) {
                FilterStatus::Pending => {}
                FilterStatus::Leaf { value, .. } => {
                    if let Some(&image) = self.memo.get(&value) {
                        filter.replace((image != Condition::FALSE).then_some(image));
                    } else {
                        self.operand = value;
                        self.boolean =
                            Some(arena.substitute(value, self.bindings.as_ref().unwrap().clone()));
                    }
                }
                FilterStatus::Complete(root) => {
                    self.result = Some(root);
                    self.filter = None;
                }
            }
        } else {
            self.done = self.cleanup_tick();
        }
        self.done.then_some(self.result.clone()).flatten()
    }
}

impl Trace for Substitution {
    fn trace(&self, cursor: &mut Cursor) -> Step {
        match cursor.phase {
            0 => {
                let value = self
                    .filter
                    .as_ref()
                    .and_then(|filter| filter.values().next());
                cursor.fields(value.as_slice())
            }
            1 => cursor.optional(self.boolean.as_ref()),
            2 => cursor.optional(self.arena_guard.as_ref()),
            3 => match &self.bindings {
                Some(images) => cursor.values(images),
                None => cursor.advance(),
            },
            4 => cursor.values(&self.draining),
            5 => cursor.substitutions(&self.memo),
            6 => cursor.fields(&[self.operand]),
            _ => Step::Done,
        }
    }
}
