use chr::engine::Engine;
use chr::observe::Output;
use chr::program::prepare;
use chr::syntax::{parse_program, parse_query};
use std::io::{self, Write};
use std::sync::Arc;

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        return Err("usage: chr PROGRAM.chr --query 'RELATIONS'".into());
    };
    if path == "--help" {
        println!(
            "Usage: chr PROGRAM.chr --query 'RELATIONS'\n\nRun a relational program and stream each answer as JSON graph events."
        );
        return Ok(());
    }
    if args.next().as_deref() != Some("--query") {
        return Err("expected --query after the program path".into());
    }
    let query = args.next().ok_or("expected a query after --query")?;
    if args.next().is_some() {
        return Err("unexpected argument after the query".into());
    }
    let program = parse_program(&std::fs::read_to_string(path)?)?;
    let query = parse_query(&query)?;
    let code = Arc::new(prepare(&program, &query)?);
    let mut engine = Engine::new(code.clone());
    let mut stdout = io::BufWriter::new(io::stdout().lock());
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
            signatures: &code.signatures,
            variables: &code.query_variables,
        },
    )?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    while !engine.delivery_done() {
        engine.advance(1);
        if let Some(event) = engine.take_output() {
            serde_json::to_writer(&mut stdout, &event)?;
            stdout.write_all(b"\n")?;
            if matches!(event, Output::End) {
                stdout.flush()?;
            }
        }
    }
    stdout.flush()?;
    Ok(())
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
