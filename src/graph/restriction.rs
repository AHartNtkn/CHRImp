//! Snapshot-valid projected bucket arrangements. A bounded registry shares one
//! producer per queried bucket/mask; subscribers own values, position and scope.
//! Only singleton-bound inputs qualify. Cached rows carry no Condition handles.
use super::{Graph, Occurrences, PORT};
use crate::store::{Root, WeakRoot};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

// This configuration bounds planning, retention and teardown. Wider/larger
// restrictions continue through the general matcher without semantic changes.
pub(crate) const MAX_PORTS: usize = 8;
const MAX_BUCKET: usize = 4096;
const CAPACITY: usize = 32;

#[cfg(feature = "diagnostics")]
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct RestrictionDiagnostics {
    pub created: u64,
    pub reused: u64,
    /// Raw row inspections: each includes one projection and partition insertion.
    pub producer_candidates: u64,
    pub projected_ports: u64,
    pub partition_lookups: u64,
    pub retained_partitions: usize,
    pub peak_retained_partitions: usize,
    pub retained_rows: usize,
    pub peak_retained_rows: usize,
    pub entries: usize,
}
#[derive(Default)]
pub(crate) struct Cache {
    entries: Mutex<VecDeque<Arc<Entry>>>,
    #[cfg(feature = "diagnostics")]
    stats: Arc<Mutex<RestrictionDiagnostics>>,
}
#[derive(PartialEq, Eq)]
struct Key {
    relation: usize,
    port: usize,
    variable: u64,
    mask: u8,
}
struct Entry {
    key: Key,
    dependency: WeakRoot,
    state: Mutex<Producer>,
    #[cfg(feature = "diagnostics")]
    stats: Arc<Mutex<RestrictionDiagnostics>>,
}
struct Producer {
    cursor: Option<Occurrences>,
    rows: Vec<PartitionRow>,
    partitions: HashMap<[u64; MAX_PORTS], Partition>,
    subscribers: usize,
    done: bool,
    abandoned: bool,
}
#[cfg(feature = "diagnostics")]
impl Drop for Entry {
    fn drop(&mut self) {
        let p = self.state.get_mut().unwrap();
        let mut stats = self.stats.lock().unwrap();
        stats.retained_rows -= p.rows.len();
        stats.retained_partitions -= p.partitions.len();
    }
}
// Both buffers have trivial elements: eviction frees buffers without walking
// per-partition allocation chains. Links remain stable across vector growth.
struct PartitionRow {
    occurrence: u64,
    next: Option<usize>,
}
struct Partition {
    first: usize,
    last: usize,
}
pub(crate) struct Subscriber {
    entry: Arc<Entry>,
    values: [u64; MAX_PORTS],
    last: Option<usize>,
}
pub(crate) enum Status {
    Pending,
    Found(u64),
    Done,
}
impl Drop for Subscriber {
    fn drop(&mut self) {
        let mut p = self.entry.state.lock().unwrap();
        p.subscribers -= 1;
        if p.subscribers == 0 && !p.done {
            p.cursor = None;
            p.abandoned = true;
        }
    }
}
impl Subscriber {
    /// At most one raw row is examined per call. Any subscriber at the frontier
    /// can advance production; lagging/paused subscribers never block another.
    pub(crate) fn tick(&mut self, g: &Graph) -> Status {
        let mut p = self.entry.state.lock().unwrap();
        let next = if let Some(last) = self.last {
            p.rows[last].next
        } else {
            #[cfg(feature = "diagnostics")]
            {
                self.entry.stats.lock().unwrap().partition_lookups += 1;
            }
            p.partitions
                .get(&self.values)
                .map(|partition| partition.first)
        };
        if let Some(next) = next {
            self.last = Some(next);
            return Status::Found(p.rows[next].occurrence);
        }
        if p.done {
            return Status::Done;
        }
        if let Some((id, _)) = p.cursor.as_mut().expect("live subscriber").next(g) {
            let args = g.arguments(id);
            let mut values = [0; MAX_PORTS];
            for port in 0..MAX_PORTS {
                if self.entry.key.mask & (1 << port) != 0 {
                    values[port] = args[port];
                }
            }
            let next = p.rows.len();
            p.rows.push(PartitionRow {
                occurrence: id,
                next: None,
            });
            if let Some(partition) = p.partitions.get_mut(&values) {
                let last = partition.last;
                partition.last = next;
                p.rows[last].next = Some(next);
            } else {
                p.partitions.insert(
                    values,
                    Partition {
                        first: next,
                        last: next,
                    },
                );
            }
            #[cfg(feature = "diagnostics")]
            {
                let mut stats = self.entry.stats.lock().unwrap();
                stats.producer_candidates += 1;
                stats.projected_ports += self.entry.key.mask.count_ones() as u64;
                stats.retained_rows += 1;
                stats.peak_retained_rows = stats.peak_retained_rows.max(stats.retained_rows);
                // A first link denotes a newly created partition.
                if p.partitions[&values].first == next {
                    stats.retained_partitions += 1;
                    stats.peak_retained_partitions = stats
                        .peak_retained_partitions
                        .max(stats.retained_partitions);
                }
            }
        } else {
            p.cursor = None;
            p.done = true;
        }
        Status::Pending
    }
}
impl Cache {
    pub(crate) fn clear(&self) {
        // At most CAPACITY entries; unfinished producers are owned by subscribers.
        self.entries.lock().unwrap().clear();
    }
}
impl Graph {
    #[cfg(feature = "diagnostics")]
    pub fn restriction_diagnostics(&self) -> RestrictionDiagnostics {
        let entries = self.shared_restrictions.entries.lock().unwrap().len();
        RestrictionDiagnostics {
            entries,
            ..*self.shared_restrictions.stats.lock().unwrap()
        }
    }
    pub(crate) fn restriction(
        &self,
        root: &Root,
        relation: usize,
        port: usize,
        bound: [Option<u64>; MAX_PORTS],
    ) -> Option<Subscriber> {
        if self.arities[relation] > MAX_PORTS || bound.iter().flatten().count() < 2 {
            return None;
        }
        // Recheck on every subscription, including hits. A late conditional merge
        // cannot reuse a raw-identity restriction in its changed snapshot.
        if !bound.iter().flatten().all(|&v| self.singleton(root, v)) {
            return None;
        }
        let variable = bound[port]?;
        let count = self.port_count(root, relation, port, variable);
        if count > MAX_BUCKET {
            return None;
        }
        let prefix = [PORT, self.port_bases[relation] + port as u64, variable, 0];
        let dependency = self.index.prefix_root(root, prefix, 3);
        let mut values = [0; MAX_PORTS];
        let mut mask = 0;
        for (i, value) in bound.iter().enumerate() {
            // The chosen port's value identifies the bucket itself.
            if i != port
                && let Some(value) = value
            {
                mask |= 1 << i;
                values[i] = *value;
            }
        }
        let key = Key {
            relation,
            port,
            variable,
            mask,
        };
        let cache = &self.shared_restrictions;
        let mut entries = cache.entries.lock().unwrap();
        for entry in entries.iter() {
            if entry.key == key && entry.dependency.matches(&dependency) {
                let mut p = entry.state.lock().unwrap();
                if !p.abandoned {
                    p.subscribers += 1;
                    #[cfg(feature = "diagnostics")]
                    {
                        cache.stats.lock().unwrap().reused += 1;
                    }
                    return Some(Subscriber {
                        entry: entry.clone(),
                        values,
                        last: None,
                    });
                }
            }
        }
        let entry = Arc::new(Entry {
            key,
            dependency: dependency.downgrade(),
            state: Mutex::new(Producer {
                // The exact bucket subtree is contained in every subscriber's
                // traced graph root. No extra graph/condition GC root is needed.
                cursor: Some(Occurrences {
                    cursor: self.index.range(
                        dependency,
                        prefix,
                        [prefix[0], prefix[1], prefix[2], u64::MAX],
                    ),
                    id_word: 3,
                }),
                rows: Vec::new(),
                partitions: HashMap::new(),
                subscribers: 1,
                done: false,
                abandoned: false,
            }),
            #[cfg(feature = "diagnostics")]
            stats: cache.stats.clone(),
        });
        if entries.len() == CAPACITY {
            entries.pop_front();
        }
        entries.push_back(entry.clone());
        #[cfg(feature = "diagnostics")]
        {
            cache.stats.lock().unwrap().created += 1;
        }
        Some(Subscriber {
            entry,
            values,
            last: None,
        })
    }
}
