use chr::engine::Engine;
use chr::observe::Output;
use chr::program::prepare;
use chr::syntax::{parse_program, parse_query};
use std::sync::Arc;

pub fn engine(program: &str, query: &str) -> Engine {
    Engine::new(Arc::new(
        prepare(
            &parse_program(program).unwrap(),
            &parse_query(query).unwrap(),
        )
        .unwrap(),
    ))
}
#[derive(Debug, PartialEq, Eq)]
pub struct Row {
    pub occurrence: u64,
    pub relation: usize,
    pub ports: Vec<u64>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Answer {
    pub id: (u64, u64),
    pub variables: Vec<u64>,
    pub rows: Vec<Row>,
}
#[derive(Default)]
pub struct Reader {
    answer: Option<Answer>,
    row: Option<Row>,
}
impl Reader {
    pub fn push(&mut self, event: Output) -> Option<Answer> {
        match event {
            Output::Begin {
                completion,
                alternative,
            } => {
                assert!(self.answer.is_none());
                self.answer = Some(Answer {
                    id: (completion, alternative),
                    variables: vec![],
                    rows: vec![],
                });
            }
            Output::Variable { slot, variable } => {
                let a = self.answer.as_mut().unwrap();
                assert_eq!(slot, a.variables.len());
                a.variables.push(variable);
            }
            Output::Fact {
                occurrence,
                relation,
            } => {
                assert!(self.row.is_none());
                self.row = Some(Row {
                    occurrence,
                    relation,
                    ports: vec![],
                });
            }
            Output::Port { variable } => self.row.as_mut().unwrap().ports.push(variable),
            Output::EndFact => self
                .answer
                .as_mut()
                .unwrap()
                .rows
                .push(self.row.take().unwrap()),
            Output::End => {
                assert!(self.row.is_none());
                return self.answer.take();
            }
        }
        None
    }
    pub fn next(&mut self, e: &mut Engine) -> Option<Answer> {
        e.take_output().and_then(|o| self.push(o))
    }
}
pub fn finish(e: &mut Engine) -> Answer {
    let mut reader = Reader::default();
    for _ in 0..500_000 {
        e.advance(1);
        if let Some(a) = reader.next(e) {
            return a;
        }
    }
    panic!("finite answer must complete");
}
pub fn facts(e: &Engine, a: &Answer) -> Vec<String> {
    let mut names = a
        .rows
        .iter()
        .map(|r| e.program().signatures[r.relation].name.clone())
        .collect::<Vec<_>>();
    names.sort();
    names
}
