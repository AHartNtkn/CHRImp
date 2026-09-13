use chr::engine::Engine;
use chr::observe::Output;
use chr::program::prepare;
use chr::syntax::{parse_program, parse_query};
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Instant;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        return Err("usage: chr PROGRAM.chr --query 'RELATIONS'".into());
    };
    if path == "--notebook" {
        let port = match args.next().as_deref() {
            None => 7878,
            Some("--port") => args.next().ok_or("expected port")?.parse::<u16>()?,
            _ => return Err("expected --port PORT".into()),
        };
        if args.next().is_some() {
            return Err("unexpected notebook argument".into());
        }
        let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
        println!("Notebook: http://{}", listener.local_addr()?);
        io::stdout().flush()?;
        chr::notebook::serve(listener)?;
        return Ok(());
    }
    if path == "--help" {
        println!(
            "Usage: chr PROGRAM.chr --query 'RELATIONS' [--diagnostics]\n\nRun a relational program and stream each answer as JSON graph events.\n--diagnostics writes native work/memory and phase evidence to stderr (build with --features diagnostics).\nUse --notebook [--port PORT] to open the browser notebook (default port: 7878)."
        );
        return Ok(());
    }
    if args.next().as_deref() != Some("--query") {
        return Err("expected --query after the program path".into());
    }
    let query = args.next().ok_or("expected a query after --query")?;
    let diagnostics = match args.next().as_deref() {
        None => false,
        Some("--diagnostics") => true,
        _ => return Err("expected --diagnostics or end of arguments after the query".into()),
    };
    if args.next().is_some() {
        return Err("unexpected argument after --diagnostics".into());
    }
    if diagnostics && !cfg!(feature = "diagnostics") {
        return Err(
            "build with cargo build --release --features diagnostics to use --diagnostics".into(),
        );
    }
    let setup = diagnostics.then(Instant::now);
    let program = parse_program(&std::fs::read_to_string(path)?)?;
    let query = parse_query(&query)?;
    let code = Arc::new(prepare(&program, &query)?);
    let mut engine = Engine::new(code);
    execute(&mut engine, io::BufWriter::new(io::stdout().lock()), setup)
}
fn execute(
    engine: &mut Engine,
    mut stdout: impl Write,
    setup: Option<Instant>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "diagnostics")]
    let setup_ms = setup.map(|t| t.elapsed().as_secs_f64() * 1000.0);
    let source_start = setup.map(|_| Instant::now());
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let code = engine.program();
        #[derive(serde::Serialize)]
        struct Header<'a> {
            kind: &'static str,
            signatures: &'a [chr::program::Signature],
            variables: &'a [String],
        }
        serde_json::to_writer(
            &mut stdout,
            &Header {
                kind: "program",
                signatures: code.signatures(),
                variables: code.query_variables(),
            },
        )?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
        while !engine.delivery_done() {
            chr::runtime::drive(engine, 4096, |event| {
                serde_json::to_writer(&mut stdout, &event)?;
                stdout.write_all(b"\n")?;
                if matches!(event, Output::End) {
                    stdout.flush()?;
                }
                Ok(())
            })?;
        }
        stdout.flush()?;
        Ok(())
    })();
    #[cfg(feature = "diagnostics")]
    let source = source_start.map(|t| diagnostic_checkpoint(engine, t));
    let cleanup_start = source_start.map(|_| Instant::now());
    engine.cancel();
    while !engine.cancel_done() {
        engine.advance(512);
    }
    #[cfg(feature = "diagnostics")]
    if let Some(start) = cleanup_start {
        let after_cancel = diagnostic_checkpoint(engine, start);
        let report = serde_json::json!({"schema": 1, "kind": "cli_diagnostics",
            "load_prepare_init_ms": setup_ms, "source": source, "after_cancel": after_cancel,
            "output_succeeded": result.is_ok(),
            "rules": engine.program().rules().iter().map(|r| &r.name).collect::<Vec<_>>()});
        // Reporting follows cancellation, so a closed diagnostic sink cannot bypass reclamation.
        let written = (|| -> io::Result<()> {
            let mut stderr = io::stderr().lock();
            serde_json::to_writer(&mut stderr, &report)?;
            stderr.write_all(b"\n")
        })();
        result?;
        written?;
        return Ok(());
    }
    #[cfg(not(feature = "diagnostics"))]
    let _ = cleanup_start;
    result
}

#[cfg(feature = "diagnostics")]
fn diagnostic_checkpoint(engine: &Engine, start: Instant) -> serde_json::Value {
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    serde_json::json!({"elapsed_ms": elapsed_ms, "work": engine.diagnostics(),
        "shared_restrictions": engine.restriction_diagnostics(),
        "memory_counts": engine.memory(), "pending_tasks": engine.pending_tasks(),
        "exhausted": engine.exhausted(), "delivery_done": engine.delivery_done(),
        "cancel_done": engine.cancel_done()})
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct ClosedAfter(usize, usize);
    impl Write for ClosedAfter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.0 == 0 {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
            }
            let n = bytes.len().min(self.0);
            self.0 -= n;
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.1 == 0 {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
            }
            self.1 -= 1;
            Ok(())
        }
    }
    #[test]
    fn cli_reclaims_execution_before_returning_on_success_or_output_error() {
        for limit in [
            None,
            Some((0, usize::MAX)),
            Some((180, usize::MAX)),
            Some((usize::MAX, 0)),
            Some((usize::MAX, 2)),
        ] {
            let query = vec!["p(A)"; 1024].join(",");
            let code = Arc::new(
                prepare(&parse_program("").unwrap(), &parse_query(&query).unwrap()).unwrap(),
            );
            let mut engine = Engine::new(code);
            if let Some((bytes, flushes)) = limit {
                assert_eq!(
                    execute(&mut engine, ClosedAfter(bytes, flushes), None)
                        .unwrap_err()
                        .to_string(),
                    "closed"
                );
            } else {
                let mut bytes = Vec::new();
                execute(&mut engine, &mut bytes, None).unwrap();
                let events: Vec<serde_json::Value> = std::str::from_utf8(&bytes)
                    .unwrap()
                    .lines()
                    .map(|s| serde_json::from_str(s).unwrap())
                    .collect();
                assert_eq!(events.iter().filter(|e| e["kind"] == "fact").count(), 1024);
                assert_eq!(events.last().unwrap()["kind"], "end");
            }
            assert!(engine.cancel_done());
            assert_eq!(engine.pending_tasks(), 0);
            let m = engine.memory();
            assert_eq!(
                [
                    m.graph_nodes,
                    m.release_batches,
                    m.occurrences,
                    m.conditions,
                    m.history_nodes,
                    m.history_records,
                    m.pending_nodes,
                    m.obligation_descriptors
                ],
                [0; 8]
            );
        }
    }
}
