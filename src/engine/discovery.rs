//! A direct anchor supplies every head variable with no identity comparison.
//! Identity-sensitive single heads and multiheads keep immutable discovery.
//! Commit remains the current-root liveness/history/identity authority.
use super::*;
use crate::trace::{Cursor as TraceCursor, Step, Trace};

pub(super) enum Discovery {
    Indexed(Box<Matches>),
    Anchor {
        arguments: Option<Arc<Vec<u64>>>,
        occurrence: u64,
        scope: Condition,
        bindings: Vec<u64>,
    },
}
impl Discovery {
    pub(super) fn anchor(arguments: Arc<Vec<u64>>, occurrence: u64, scope: Condition) -> Self {
        Self::Anchor {
            arguments: Some(arguments),
            occurrence,
            scope,
            bindings: Vec::new(),
        }
    }
    pub(super) fn root(&self) -> Option<Root> {
        match self {
            Self::Indexed(m) => Some(m.root()),
            Self::Anchor { .. } => None,
        }
    }
    /// Indexed visits only; direct anchors do not enumerate index candidates.
    #[cfg(feature = "diagnostics")]
    pub(super) fn candidate_visits(&self) -> Option<u64> {
        match self {
            Self::Indexed(m) => Some(m.candidate_visits()),
            Self::Anchor { .. } => None,
        }
    }
    pub(super) fn tick(&mut self, graph: &Graph, arena: &mut Arena) -> MatchStatus {
        match self {
            Self::Indexed(m) => m.tick(graph, arena),
            Self::Anchor {
                arguments,
                occurrence,
                scope,
                bindings,
            } => {
                let Some(args) = arguments else {
                    return MatchStatus::Done;
                };
                if bindings.len() < args.len() {
                    bindings.push(args[bindings.len()]);
                    MatchStatus::Pending
                } else {
                    *arguments = None;
                    MatchStatus::Found(Match {
                        occurrences: vec![*occurrence],
                        bindings: std::mem::take(bindings),
                        support: *scope,
                    })
                }
            }
        }
    }
    pub(super) fn discard_tick(&mut self) -> bool {
        match self {
            Self::Indexed(m) => m.discard_tick(),
            Self::Anchor {
                arguments,
                bindings,
                scope,
                ..
            } => {
                *arguments = None;
                *bindings = Vec::new();
                *scope = Condition::FALSE;
                true
            }
        }
    }
}
impl Trace for Discovery {
    fn trace(&self, c: &mut TraceCursor) -> Step {
        match self {
            Self::Indexed(m) => m.trace(c),
            Self::Anchor { scope, .. } => {
                if c.phase == 0 {
                    c.fields(&[*scope])
                } else {
                    Step::Done
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn engine(program: &str, query: &str) -> Engine {
        Engine::new(Arc::new(
            crate::program::prepare(
                &crate::syntax::parse_program(program).unwrap(),
                &crate::syntax::parse_query(query).unwrap(),
            )
            .unwrap(),
        ))
    }
    #[test]
    fn direct_anchor_builds_one_binding_per_tick_without_pinning_graph() {
        let mut d = Discovery::anchor(Arc::new((0..4096).collect()), 7, Condition::TRUE);
        let g = Graph::new(&[]);
        let mut a = Arena::default();
        for i in 0..4096 {
            assert!(d.root().is_none());
            assert!(matches!(d.tick(&g, &mut a), MatchStatus::Pending));
            let Discovery::Anchor { bindings, .. } = &d else {
                panic!()
            };
            assert_eq!(bindings.len(), i + 1);
        }
        let MatchStatus::Found(m) = d.tick(&g, &mut a) else {
            panic!()
        };
        assert_eq!(m.bindings, (0..4096).collect::<Vec<_>>());
        assert_eq!(m.occurrences, [7]);
        assert!(matches!(d.tick(&g, &mut a), MatchStatus::Done));
    }
    #[test]
    fn mixed_trigger_cutoff_includes_identity_sensitive_single_heads() {
        let cases = [
            ("p(X,Y) ==> a(X,Y). p(X,Y) ==> b(X,Y).", vec![false, false]),
            (
                "p(X,Y) ==> a(X,Y). p(X,X) ==> b(X). p(X,Y) ==> c(X,Y).",
                vec![false, true, false],
            ),
            (
                "p(X,X) ==> a(X). p(X,Y),q(X) ==> b(Y). p(X,Y) ==> c(X,Y).",
                vec![true, true, false],
            ),
        ];
        for (program, indexed) in cases {
            let mut e = engine(program, "p(A,B)");
            let mut task = None;
            for _ in 0..1000 {
                if let Some(i) = e
                    .queue
                    .iter()
                    .position(|s| matches!(s.task, Task::Activate { .. }))
                {
                    task = e.queue.remove(i);
                    break;
                }
                e.advance(1);
            }
            let mut s = task.expect("activation");
            for i in 0..indexed.len() {
                let Task::Activate { root, .. } = &s.task else {
                    panic!()
                };
                assert_eq!(root.is_some(), indexed[i..].contains(&true));
                assert!(!e.task(&mut s));
                let Task::Search(search) = &e.queue.back().unwrap().task else {
                    panic!()
                };
                assert_eq!(search.matches.root().is_some(), indexed[i]);
                let Task::Activate { root, .. } = &s.task else {
                    panic!()
                };
                assert_eq!(root.is_some(), indexed[i + 1..].contains(&true));
            }
        }
    }
    #[test]
    fn consumed_anchor_row_can_be_collected_before_commit() {
        let mut e = engine("p(X) <=> bad(X).", "true");
        e.queue.clear();
        e.pending_root = e.obligations.empty();
        let mut update = e
            .graph
            .post(e.state.graph.clone(), 0, vec![77], Condition::TRUE)
            .unwrap();
        let id = update.occurrence();
        e.state.graph = loop {
            if let UpdateStatus::Complete(r) = update.tick(&mut e.graph) {
                break r;
            }
        };
        drop(update);
        let mut d = Discovery::anchor(e.graph.arguments(id), id, Condition::TRUE);
        let candidate = loop {
            if let MatchStatus::Found(m) = d.tick(&e.graph, &mut e.arena) {
                break m;
            }
        };
        e.spawn(
            Condition::TRUE,
            Task::Search(Box::new(Search {
                matches: d,
                rule: 0,
                transport: None,
                candidate: Some(candidate),
                commit: None,
            })),
        );
        let mut consume = e
            .graph
            .set_liveness(e.state.graph.clone(), id, Condition::FALSE)
            .unwrap();
        e.state.graph = loop {
            if let UpdateStatus::Complete(r) = consume.tick(&mut e.graph) {
                break r;
            }
        };
        drop(consume);
        e.request_collection();
        for _ in 0..10000 {
            e.maintain(1);
            if !e.collecting() {
                break;
            }
        }
        assert!(!e.collecting());
        assert_eq!(e.graph.occurrence_count(), 0);
        for _ in 0..10000 {
            e.advance(1);
            e.take_output();
            if e.delivery_done() {
                break;
            }
        }
        assert!(e.delivery_done());
        assert_eq!(e.applications(), 0);
    }
    #[test]
    fn direct_anchor_scope_survives_gc_and_transports_before_commit() {
        for positive in [false, true] {
            let mut e = engine("p() ==> result().", "true");
            e.queue.clear();
            e.pending_root = e.obligations.empty();
            let mut post = e
                .graph
                .post(e.state.graph.clone(), 0, vec![], Condition::TRUE)
                .unwrap();
            let id = post.occurrence();
            e.state.graph = loop {
                if let UpdateStatus::Complete(r) = post.tick(&mut e.graph) {
                    break r;
                }
            };
            drop(post);
            let (choice, x) = e.arena.fresh_choice();
            let scope = if positive { x } else { x.not() };
            let matches = Discovery::anchor(e.graph.arguments(id), id, scope);
            e.spawn(
                Condition::TRUE,
                Task::Search(Box::new(Search {
                    matches,
                    rule: 0,
                    candidate: None,
                    commit: None,
                    transport: None,
                })),
            );
            e.coordinates
                .publish(Arc::new(BTreeMap::from([(choice, Condition::FALSE)])));
            e.request_collection();
            for _ in 0..10000 {
                e.advance(1);
                e.take_output();
                if e.delivery_done() && e.pending_tasks() == 0 {
                    break;
                }
            }
            assert!(e.delivery_done());
            assert_eq!(e.applications(), u64::from(!positive));
        }
    }
}
