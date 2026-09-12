//! Budgeted cofactoring of a persistent condition-valued index.

use super::{Filter, FilterStatus, Root, Store};
use crate::condition::{Arena, Condition, Progress, Restriction as BooleanRestriction};
use crate::trace::{Cursor, Step, Trace};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Keep `roots()` in store collection, and trace both collected leaf values and
/// `condition_roots()` in arena collection until this continuation is released.
/// Completion preserves the result root; discard abandons staged output.
pub struct Restriction {
    owner: u32,
    filter: Option<Filter<Condition>>,
    bindings: Option<Arc<BTreeMap<u64, bool>>>,
    draining: BTreeMap<u64, bool>,
    boolean: Option<BooleanRestriction>,
    // A terminal restriction validates bindings and binds the arena on the
    // first tick. Once complete it enforces arena identity and its GC lease.
    arena_guard: Option<BooleanRestriction>,
    result: Option<Root>,
    done: bool,
    discarding: bool,
}

impl Store<Condition> {
    pub fn restrict(&self, root: Root, bindings: Arc<BTreeMap<u64, bool>>) -> Restriction {
        Restriction {
            owner: self.owner,
            filter: Some(self.filter(root)),
            bindings: Some(bindings),
            draining: BTreeMap::new(),
            boolean: None,
            arena_guard: None,
            result: None,
            done: false,
            discarding: false,
        }
    }
}

impl Restriction {
    pub fn roots(&self) -> impl Iterator<Item = Root> + '_ {
        self.filter
            .iter()
            .flat_map(Filter::roots)
            .chain(self.result)
    }

    pub fn condition_roots(&self) -> impl Iterator<Item = Condition> + '_ {
        self.filter
            .iter()
            .flat_map(Filter::values)
            .chain(self.boolean.iter().flat_map(BooleanRestriction::roots))
            .chain(self.arena_guard.iter().flat_map(BooleanRestriction::roots))
    }

    fn cleanup_tick(&mut self) -> bool {
        if let Some(bindings) = self.bindings.take() {
            if let Some(bindings) = Arc::into_inner(bindings) {
                self.draining = bindings;
            }
            return false;
        }
        self.draining.pop_first();
        self.draining.is_empty()
    }

    /// Cancel without traversing another leaf or evaluating another cofactor.
    /// Each call drains at most one nested job step or one assignment entry.
    pub fn discard_tick(&mut self) -> bool {
        self.discarding = true;
        self.filter = None; // Filter frames are bounded scalar index handles.
        self.result = None;
        if let Some(boolean) = &mut self.boolean {
            if boolean.discard_tick() {
                self.boolean = None;
            }
            return false;
        }
        if let Some(guard) = &mut self.arena_guard {
            if guard.discard_tick() {
                self.arena_guard = None;
            }
            return false;
        }
        self.done = self.cleanup_tick();
        self.done
    }

    /// Advance one filter transition, one Boolean restriction step, or one
    /// cleanup step. Returns only after assignment cleanup has completed.
    pub fn tick(&mut self, store: &mut Store<Condition>, arena: &mut Arena) -> Option<Root> {
        assert!(!self.discarding, "index restriction has been discarded");
        assert_eq!(self.owner, store.owner, "foreign index restriction");
        store.assert_mutable();
        let guard = self.arena_guard.get_or_insert_with(|| {
            arena.restrict(Condition::TRUE, self.bindings.as_ref().unwrap().clone())
        });
        if guard.tick(arena) == Progress::Pending {
            return None;
        }
        if self.done {
            return self.result;
        }
        if let Some(boolean) = &mut self.boolean {
            if let Progress::Complete(value) = boolean.tick(arena) {
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
                    self.boolean =
                        Some(arena.restrict(value, self.bindings.as_ref().unwrap().clone()));
                }
                FilterStatus::Complete(root) => {
                    self.result = Some(root);
                    self.filter = None;
                }
            }
        } else {
            self.done = self.cleanup_tick();
        }
        self.done.then_some(self.result).flatten()
    }
}

impl Trace for Restriction {
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
            _ => Step::Done,
        }
    }
}
