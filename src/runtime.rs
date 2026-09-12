//! Shared cooperative execution driver. Consumers choose a fallible output sink.
use crate::{engine::Engine, observe::Output};
use std::io;

/// Drive a finite quantum, draining every event before taking another core step.
/// A sink failure cancels source execution and is returned to the owner.
pub fn drive(
    engine: &mut Engine,
    budget: usize,
    mut output: impl FnMut(Output) -> io::Result<()>,
) -> io::Result<()> {
    for _ in 0..budget {
        if engine.delivery_done() || engine.canceled() {
            break;
        }
        engine.advance(1);
        if let Some(event) = engine.take_output()
            && let Err(error) = output(event)
        {
            engine.cancel();
            return Err(error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        program::prepare,
        syntax::{parse_program, parse_query},
    };
    use std::sync::Arc;

    #[test]
    fn fallible_sink_stops_source_execution_and_returns_original_error() {
        let code = prepare(&parse_program("").unwrap(), &parse_query("p;q").unwrap()).unwrap();
        let mut engine = Engine::new(Arc::new(code));
        let failure = drive(&mut engine, 100_000, |_| {
            Err(io::Error::other("storage unavailable"))
        })
        .unwrap_err();
        assert_eq!(failure.to_string(), "storage unavailable");
        assert!(engine.canceled());
        while !engine.cancel_done() {
            engine.advance(512);
        }
        assert_eq!(engine.pending_tasks(), 0);
    }
}
