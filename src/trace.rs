//! Borrow-free cursors for incrementally enumerating suspended work's roots.
use crate::condition::Condition;
use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Root(Condition),
    Pending,
    Done,
}
pub trait Trace {
    fn trace(&self, cursor: &mut Cursor) -> Step;
}
#[derive(Default)]
pub struct Cursor {
    pub phase: usize,
    pub index: usize,
    pub slot: usize,
    pub key: Option<u64>,
    pub pair: Option<(Condition, Condition)>,
    child: Option<Box<Cursor>>,
}
impl Cursor {
    pub fn advance(&mut self) -> Step {
        self.phase += 1;
        self.index = 0;
        self.slot = 0;
        self.key = None;
        self.pair = None;
        self.child = None;
        Step::Pending
    }
    pub fn fields(&mut self, values: &[Condition]) -> Step {
        if let Some(&c) = values.get(self.index) {
            self.index += 1;
            Step::Root(c)
        } else {
            self.advance()
        }
    }
    pub fn values(&mut self, map: &BTreeMap<u64, Condition>) -> Step {
        let next = match self.key {
            Some(k) => map.range((Excluded(k), Unbounded)).next(),
            None => map.first_key_value(),
        };
        if let Some((&key, &c)) = next {
            self.key = Some(key);
            Step::Root(c)
        } else {
            self.advance()
        }
    }
    pub fn pairs(&mut self, map: &BTreeMap<(Condition, Condition), Condition>) -> Step {
        let next = match self.pair {
            Some(k) => map.range((Excluded(k), Unbounded)).next(),
            None => map.first_key_value(),
        };
        if let Some((&(a, b), &v)) = next {
            let c = [a, b, v][self.slot];
            self.slot += 1;
            if self.slot == 3 {
                self.slot = 0;
                self.pair = Some((a, b));
            }
            Step::Root(c)
        } else {
            self.advance()
        }
    }
    pub fn optional<T: Trace>(&mut self, value: Option<&T>) -> Step {
        if let Some(value) = value {
            match value.trace(self.child.get_or_insert_with(|| Box::new(Self::default()))) {
                Step::Done => self.advance(),
                s => s,
            }
        } else {
            self.advance()
        }
    }
    pub fn vector(
        &mut self,
        len: usize,
        mut trace: impl FnMut(usize, &mut Cursor) -> Step,
    ) -> Step {
        if self.index == len {
            return self.advance();
        }
        match trace(
            self.index,
            self.child.get_or_insert_with(|| Box::new(Self::default())),
        ) {
            Step::Done => {
                self.index += 1;
                self.child = None;
                Step::Pending
            }
            s => s,
        }
    }
}
