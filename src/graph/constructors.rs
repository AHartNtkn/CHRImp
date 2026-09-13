//! Direct conditional attachments, indexed by union-find representative.
//! Rows remain ordinary occurrences; attachments select their surviving supports.
use super::*;
use crate::condition::{Arena, Job, Operation, poll};
use crate::gc::discard_slot;
use crate::trace::{Cursor as TraceCursor, Step, Trace};

pub(crate) const ATTACHMENT: u64 = 7;
pub(crate) enum AttachmentStatus {
    Pending,
    Overlap(u64, Condition),
    Done,
}
pub(crate) struct Attach {
    representative: u64,
    occurrence: u64,
    remaining: Condition,
    cursor: store::Cursor,
    other: u64,
    hit: Condition,
    job: Option<Job>,
    phase: u8,
}
impl Attach {
    pub fn new(
        g: &Graph,
        root: Root,
        representative: u64,
        occurrence: u64,
        scope: Condition,
    ) -> Self {
        Self {
            representative,
            occurrence,
            remaining: scope,
            cursor: g.index.range(
                root,
                [ATTACHMENT, representative, 0, 0],
                [ATTACHMENT, representative, u64::MAX, 0],
            ),
            other: 0,
            hit: Condition::FALSE,
            job: None,
            phase: 0,
        }
    }
    pub fn root(&self) -> Root {
        self.cursor.root()
    }
    pub fn tick(&mut self, g: &mut Graph, a: &mut Arena, root: &mut Root) -> AttachmentStatus {
        match self.phase {
            0 => {
                if let Some((key, c)) = self.cursor.next(&g.index) {
                    self.other = key[2];
                    self.job = Some(a.start(Operation::And(self.remaining, c)));
                    self.phase = 1;
                } else {
                    let old = g
                        .index
                        .get(root, &[ATTACHMENT, self.representative, self.occurrence, 0])
                        .unwrap_or(Condition::FALSE);
                    self.job = Some(a.start(Operation::Or(old, self.remaining)));
                    self.phase = 3;
                }
            }
            1 => {
                if let Some(hit) = poll(&mut self.job, a) {
                    self.hit = hit;
                    self.job = Some(a.start(Operation::Difference(self.remaining, hit)));
                    self.phase = 2;
                }
            }
            2 => {
                if let Some(rest) = poll(&mut self.job, a) {
                    self.remaining = rest;
                    self.phase = 0;
                    if self.hit != Condition::FALSE && self.other != self.occurrence {
                        return AttachmentStatus::Overlap(self.other, self.hit);
                    }
                }
            }
            3 => {
                if let Some(c) = poll(&mut self.job, a) {
                    *root = g.write(
                        std::mem::take(root),
                        [ATTACHMENT, self.representative, self.occurrence, 0],
                        c,
                    );
                    self.phase = 4;
                }
            }
            _ => return AttachmentStatus::Done,
        }
        AttachmentStatus::Pending
    }
    pub fn discard_tick(&mut self) -> bool {
        self.remaining = Condition::FALSE;
        self.hit = Condition::FALSE;
        !discard_slot(&mut self.job, |child| child.discard_tick())
    }
}
impl Trace for Attach {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.fields(&[self.remaining, self.hit]),
            1 => c.optional(self.job.as_ref()),
            _ => Step::Done,
        }
    }
}

pub(crate) struct Transfer {
    loser: u64,
    scope: Condition,
    cursor: store::Cursor,
    occurrence: u64,
    old: Condition,
    hit: Condition,
    job: Option<Job>,
    phase: u8,
}
impl Transfer {
    pub fn new(g: &Graph, root: Root, loser: u64, scope: Condition) -> Self {
        Self {
            loser,
            scope,
            cursor: g.index.range(
                root,
                [ATTACHMENT, loser, 0, 0],
                [ATTACHMENT, loser, u64::MAX, 0],
            ),
            occurrence: 0,
            old: Condition::FALSE,
            hit: Condition::FALSE,
            job: None,
            phase: 0,
        }
    }
    pub fn root(&self) -> Root {
        self.cursor.root()
    }
    pub fn tick(&mut self, g: &mut Graph, a: &mut Arena, root: &mut Root) -> AttachmentStatus {
        match self.phase {
            0 => {
                if let Some((key, c)) = self.cursor.next(&g.index) {
                    self.occurrence = key[2];
                    self.old = c;
                    self.job = Some(a.start(Operation::And(c, self.scope)));
                    self.phase = 1;
                } else {
                    return AttachmentStatus::Done;
                }
            }
            1 => {
                if let Some(c) = poll(&mut self.job, a) {
                    self.hit = c;
                    self.job = Some(a.start(Operation::Difference(self.old, c)));
                    self.phase = 2;
                }
            }
            _ => {
                if let Some(c) = poll(&mut self.job, a) {
                    *root = g.write(
                        std::mem::take(root),
                        [ATTACHMENT, self.loser, self.occurrence, 0],
                        c,
                    );
                    self.phase = 0;
                    if self.hit != Condition::FALSE {
                        return AttachmentStatus::Overlap(self.occurrence, self.hit);
                    }
                }
            }
        }
        AttachmentStatus::Pending
    }
    pub fn discard_tick(&mut self) -> bool {
        self.scope = Condition::FALSE;
        self.old = Condition::FALSE;
        self.hit = Condition::FALSE;
        !discard_slot(&mut self.job, |child| child.discard_tick())
    }
}
impl Trace for Transfer {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match c.phase {
            0 => c.fields(&[self.scope, self.old, self.hit]),
            1 => c.optional(self.job.as_ref()),
            _ => Step::Done,
        }
    }
}

impl Graph {
    /// Symbolic descriptions at one representative; supports may be disjoint.
    pub(crate) fn constructor_attachments(&self, root: Root, representative: u64) -> store::Cursor {
        self.index.range(
            root,
            [ATTACHMENT, representative, 0, 0],
            [ATTACHMENT, representative, u64::MAX, 0],
        )
    }
}
