//! Payload FIFO: moving its small directory never moves suspended tasks.
use super::Waiting;
use std::collections::VecDeque;

const WIDTH: usize = 4;
type Block = Box<[Option<Waiting>; WIDTH]>;

#[derive(Default)]
pub(super) struct WaitingQueue {
    blocks: VecDeque<Block>,
    spare: [Option<Block>; 2],
    head: usize,
    len: usize,
}
impl WaitingQueue {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    #[cfg(any(test, feature = "diagnostics"))]
    pub fn capacity(&self) -> usize {
        (self.blocks.len() + self.spare.iter().filter(|b| b.is_some()).count()) * WIDTH
    }
    #[cfg(feature = "diagnostics")]
    pub fn capacity_bytes(&self) -> usize {
        self.capacity() * size_of::<Option<Waiting>>() + self.blocks.capacity() * size_of::<Block>()
    }
    #[cfg(feature = "diagnostics")]
    pub fn directory_capacity(&self) -> usize {
        self.blocks.capacity()
    }
    #[cfg(feature = "diagnostics")]
    pub fn directory_len(&self) -> usize {
        self.blocks.len()
    }
    pub fn push_back(&mut self, entry: Waiting) {
        let tail = self.head + self.len;
        if tail / WIDTH == self.blocks.len() {
            let block = self.spare[0]
                .take()
                .or_else(|| self.spare[1].take())
                .unwrap_or_else(|| Box::new(std::array::from_fn(|_| None)));
            self.blocks.push_back(block);
        }
        self.blocks[tail / WIDTH][tail % WIDTH] = Some(entry);
        self.len += 1;
    }
    pub fn pop_front(&mut self) -> Option<Waiting> {
        if self.is_empty() {
            return None;
        }
        let entry = self.blocks[0][self.head].take();
        self.head += 1;
        self.len -= 1;
        if self.head == WIDTH || self.len == 0 {
            // Two small reusable blocks cover repeated tiny choice frontiers.
            // Further retired blocks release their allocations immediately.
            let block = self.blocks.pop_front();
            if self.spare[0].is_none() {
                self.spare[0] = block;
            } else if self.spare[1].is_none() {
                self.spare[1] = block;
            }
            self.head = 0;
        }
        let capacity = self.blocks.capacity();
        if capacity > 4 && self.blocks.len() <= capacity / 2 {
            let len = self.blocks.len();
            self.blocks.shrink_to((len + len / 2).max(4));
        }
        entry
    }
    pub fn get(&self, index: usize) -> Option<&Waiting> {
        if index >= self.len {
            return None;
        }
        let index = self.head + index;
        self.blocks[index / WIDTH][index % WIDTH].as_ref()
    }
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Waiting> {
        if index >= self.len {
            return None;
        }
        let index = self.head + index;
        self.blocks[index / WIDTH][index % WIDTH].as_mut()
    }
    #[cfg(test)]
    pub fn front(&self) -> Option<&Waiting> {
        self.get(0)
    }
    pub fn front_mut(&mut self) -> Option<&mut Waiting> {
        self.get_mut(0)
    }
    pub fn iter(&self) -> impl Iterator<Item = &Waiting> {
        self.blocks
            .iter()
            .flat_map(|b| b.iter().filter_map(Option::as_ref))
    }
}
