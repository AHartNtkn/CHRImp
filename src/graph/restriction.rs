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
const PREFIX_ROWS: usize = 128;

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
    pub prefix_splits: u64,
    pub prefix_probes: u64,
    pub reused_prefixes: u64,
    pub rebuilt_prefixes: u64,
    pub prefix_plans_created: u64,
    pub prefix_plans_reused: u64,
    pub cached_prefix_plans: usize,
    pub cached_prefix_links: usize,
}
#[derive(Default)]
pub(crate) struct Cache {
    entries: Mutex<VecDeque<Arc<Entry>>>,
    prefixes: Mutex<VecDeque<Arc<PrefixEntry>>>,
    #[cfg(feature = "diagnostics")]
    stats: Arc<Mutex<RestrictionDiagnostics>>,
}
#[derive(Clone, PartialEq, Eq)]
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
pub(crate) struct RowSubscriber {
    entry: Arc<Entry>,
    values: [u64; MAX_PORTS],
    last: Option<usize>,
}
pub(crate) enum Subscriber {
    Rows(RowSubscriber),
    Prefixes(Box<Prefixes>),
}
pub(crate) struct Prefixes {
    entry: Arc<PrefixEntry>,
    position: usize,
    current: Option<RowSubscriber>,
    values: [u64; MAX_PORTS],
}
struct PrefixEntry {
    key: Key,
    dependency: WeakRoot,
    state: Mutex<PrefixProducer>,
}
struct PrefixProducer {
    pending: Vec<Root>,
    // These producer leases protect all earlier chunks for lagging readers,
    // including after registry eviction or collection clears the cache.
    chunks: Vec<RowSubscriber>,
    subscribers: usize,
    done: bool,
    abandoned: bool,
}
impl Drop for Prefixes {
    fn drop(&mut self) {
        let mut p = self.entry.state.lock().unwrap();
        p.subscribers -= 1;
        if p.subscribers == 0 && !p.done {
            p.pending.clear();
            p.chunks.clear();
            p.abandoned = true;
        }
    }
}
pub(crate) enum Status {
    Pending,
    Found(u64),
    Done,
}
impl Drop for RowSubscriber {
    fn drop(&mut self) {
        let mut p = self.entry.state.lock().unwrap();
        p.subscribers -= 1;
        if p.subscribers == 0 && !p.done {
            p.cursor = None;
            p.abandoned = true;
        }
    }
}
impl RowSubscriber {
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
impl Subscriber {
    pub(crate) fn tick(&mut self, g: &Graph) -> Status {
        let Self::Prefixes(p) = self else {
            let Self::Rows(s) = self else { unreachable!() };
            return s.tick(g);
        };
        if let Some(current) = &mut p.current {
            match current.tick(g) {
                Status::Done => p.current = None,
                status => return status,
            }
            return Status::Pending;
        }
        let mut producer = p.entry.state.lock().unwrap();
        if let Some(chunk) = producer.chunks.get(p.position) {
            chunk.entry.state.lock().unwrap().subscribers += 1;
            p.current = Some(RowSubscriber {
                entry: chunk.entry.clone(),
                values: p.values,
                last: None,
            });
            p.position += 1;
            return Status::Pending;
        }
        let Some(root) = producer.pending.pop() else {
            producer.done = true;
            return Status::Done;
        };
        if let Some([left, right]) = g.index.split_prefix(&root, PREFIX_ROWS) {
            producer.pending.push(right);
            producer.pending.push(left);
            #[cfg(feature = "diagnostics")]
            {
                g.shared_restrictions.stats.lock().unwrap().prefix_splits += 1;
            }
        } else {
            producer
                .chunks
                .push(g.subscribe_prefix(root, &p.entry.key, [0; MAX_PORTS], true));
        }
        Status::Pending
    }
}
impl Cache {
    pub(crate) fn clear(&self) {
        // At most CAPACITY entries; unfinished producers are owned by subscribers.
        self.entries.lock().unwrap().clear();
        self.prefixes.lock().unwrap().clear();
    }
}
impl Graph {
    #[cfg(feature = "diagnostics")]
    pub fn restriction_diagnostics(&self) -> RestrictionDiagnostics {
        let entries = self.shared_restrictions.entries.lock().unwrap().len();
        let plans = self.shared_restrictions.prefixes.lock().unwrap();
        let cached_prefix_plans = plans.len();
        let cached_prefix_links = plans
            .iter()
            .map(|p| p.state.lock().unwrap().chunks.len())
            .sum();
        RestrictionDiagnostics {
            entries,
            cached_prefix_plans,
            cached_prefix_links,
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
        if self.arities[relation] > MAX_PORTS
            || bound.iter().flatten().count() < 2
            || !self.has_port_index(relation, port)
        {
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
        if count > PREFIX_ROWS {
            let mut entries = self.shared_restrictions.prefixes.lock().unwrap();
            for entry in entries.iter() {
                if entry.key == key && entry.dependency.matches(&dependency) {
                    let mut p = entry.state.lock().unwrap();
                    if !p.abandoned {
                        p.subscribers += 1;
                        #[cfg(feature = "diagnostics")]
                        {
                            self.shared_restrictions
                                .stats
                                .lock()
                                .unwrap()
                                .prefix_plans_reused += 1;
                        }
                        return Some(Subscriber::Prefixes(Box::new(Prefixes {
                            entry: entry.clone(),
                            position: 0,
                            current: None,
                            values,
                        })));
                    }
                }
            }
            let entry = Arc::new(PrefixEntry {
                key,
                dependency: dependency.downgrade(),
                state: Mutex::new(PrefixProducer {
                    pending: vec![dependency],
                    chunks: Vec::new(),
                    subscribers: 1,
                    done: false,
                    abandoned: false,
                }),
            });
            if entries.len() == CAPACITY {
                entries.pop_front();
            }
            entries.push_back(entry.clone());
            #[cfg(feature = "diagnostics")]
            {
                self.shared_restrictions
                    .stats
                    .lock()
                    .unwrap()
                    .prefix_plans_created += 1;
            }
            return Some(Subscriber::Prefixes(Box::new(Prefixes {
                entry,
                position: 0,
                current: None,
                values,
            })));
        }
        Some(Subscriber::Rows(
            self.subscribe_prefix(dependency, &key, values, false),
        ))
    }
    fn subscribe_prefix(
        &self,
        dependency: Root,
        key: &Key,
        values: [u64; MAX_PORTS],
        fragmented: bool,
    ) -> RowSubscriber {
        #[cfg(not(feature = "diagnostics"))]
        let _ = fragmented;
        let cache = &self.shared_restrictions;
        let mut entries = cache.entries.lock().unwrap();
        #[cfg(feature = "diagnostics")]
        if fragmented {
            cache.stats.lock().unwrap().prefix_probes += 1;
        }
        for entry in entries.iter() {
            if entry.key == *key && entry.dependency.matches(&dependency) {
                let mut p = entry.state.lock().unwrap();
                if !p.abandoned {
                    p.subscribers += 1;
                    #[cfg(feature = "diagnostics")]
                    {
                        let mut stats = cache.stats.lock().unwrap();
                        stats.reused += 1;
                        stats.reused_prefixes += u64::from(fragmented);
                    }
                    return RowSubscriber {
                        entry: entry.clone(),
                        values,
                        last: None,
                    };
                }
            }
        }
        let entry = Arc::new(Entry {
            key: key.clone(),
            dependency: dependency.downgrade(),
            state: Mutex::new(Producer {
                // The exact bucket subtree is contained in every subscriber's
                // traced graph root. No extra graph/condition GC root is needed.
                cursor: Some(Occurrences {
                    cursor: self.index.range(dependency, [0; 4], [u64::MAX; 4]),
                    id_word: 3,
                    filter: None,
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
            let mut stats = cache.stats.lock().unwrap();
            stats.created += 1;
            stats.rebuilt_prefixes += u64::from(fragmented);
        }
        RowSubscriber {
            entry,
            values,
            last: None,
        }
    }
}
