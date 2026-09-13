//! Structural normalization and conditional dispatch controls; source graph events on stdout.
//! Usage: constructor_probe baseline|priority|direct|dispatch --behavior-i [--first] [--steps N] [--seconds S]
//!    or: constructor_probe MODE PROGRAM.chr --query 'BODY' [same limits]
//! Dispatch retains each selected original arm, including its leading post.
//! Historical source stepping is outside this experiment. No production default changes.
use chr::{
    engine::{Engine, NormalizationMode},
    observe::Output,
    program::prepare,
    syntax::{Body, Program, parse_program, parse_query},
};
use std::{
    io::{self, Write},
    sync::Arc,
    time::{Duration, Instant},
};
#[allow(dead_code)]
#[path = "measure/allocation.rs"]
mod allocation;
use allocation::{Phase, during};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let mut args = std::env::args().skip(1);
    let mode_name = args
        .next()
        .ok_or("expected baseline|priority|direct|dispatch")?;
    if mode_name == "--help" {
        println!(
            "constructor_probe baseline|priority|direct|dispatch (--behavior-i | PROGRAM.chr --query 'BODY') [--first] [--steps N] [--seconds S]\nStreams source answer events to stdout; result and work/memory evidence to stderr. --seconds limits source execution, followed by bounded cancellation. Historical rule stepping is unsupported. Build with --features diagnostics for existing allocation/engine counters."
        );
        return Ok(());
    }
    let mode = match mode_name.as_str() {
        "baseline" => NormalizationMode::Baseline,
        "priority" => NormalizationMode::Priority,
        "direct" => NormalizationMode::Direct,
        "dispatch" => NormalizationMode::Dispatch,
        _ => return Err("expected baseline|priority|direct|dispatch".into()),
    };
    let input = args.next().ok_or("expected --behavior-i or PROGRAM.chr")?;
    let (p, q): (Program, Body) =
        during(Phase::Setup, || -> Result<_, Box<dyn std::error::Error>> {
            if input == "--behavior-i" {
                let doc: serde_json::Value =
                    serde_json::from_str(include_str!("behavior-synthesis.chrnb"))?;
                Ok((
                    serde_json::from_value(doc["program"].clone())?,
                    serde_json::from_value(doc["queries"][0]["body"].clone())?,
                ))
            } else {
                if args.next().as_deref() != Some("--query") {
                    return Err("expected --query BODY".into());
                }
                Ok((
                    parse_program(&std::fs::read_to_string(input)?)?,
                    parse_query(&args.next().ok_or("expected query body")?)?,
                ))
            }
        })?;
    let mut limit = 50_000_000u64;
    let mut seconds = None;
    let mut first = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--first" => first = true,
            "--steps" => limit = args.next().ok_or("expected step limit")?.parse()?,
            "--seconds" => {
                let s: f64 = args.next().ok_or("expected seconds")?.parse()?;
                if !s.is_finite() || s <= 0.0 {
                    return Err("seconds must be finite and positive".into());
                }
                seconds = Some(Duration::from_secs_f64(s));
            }
            _ => return Err(format!("unexpected argument {arg}").into()),
        }
    }
    let mut e = during(Phase::Setup, || -> Result<_, Box<dyn std::error::Error>> {
        let code = Arc::new(prepare(&p, &q)?);
        Ok(Engine::with_normalization(code, mode)?)
    })?;
    let setup_seconds = start.elapsed().as_secs_f64();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    serde_json::to_writer(
        &mut stdout,
        &serde_json::json!({"kind":"program","signatures":e.program().signatures(),"variables":e.program().query_variables()}),
    )?;
    writeln!(stdout)?;
    let source = Instant::now();
    let mut steps = 0;
    let mut answers = 0;
    let status = loop {
        if e.delivery_done() {
            break "complete";
        }
        if first && answers > 0 {
            break "first_answer";
        }
        if steps >= limit || seconds.is_some_and(|d| source.elapsed() >= d) {
            break "budget";
        }
        for _ in 0..256 {
            if steps >= limit || e.delivery_done() || first && answers > 0 {
                break;
            }
            during(Phase::Engine, || e.advance(1));
            steps += 1;
            if let Some(event) = e.take_output() {
                if matches!(event, Output::End) {
                    answers += 1;
                }
                during(Phase::Delivery, || -> io::Result<()> {
                    serde_json::to_writer(&mut stdout, &event)?;
                    writeln!(stdout)
                })?;
            }
        }
    };
    stdout.flush()?;
    let source_seconds = source.elapsed().as_secs_f64();
    let memory = e.memory();
    let stats = e.normalization_stats();
    let applications = e.applications();
    #[cfg(feature = "diagnostics")]
    let allocation_before_cleanup = allocation::snapshot();
    #[cfg(feature = "diagnostics")]
    let diagnostics = e.diagnostics().clone();
    let cleanup = Instant::now();
    e.cancel();
    for _ in 0..1_000_000 {
        if e.cancel_done() {
            break;
        }
        during(Phase::Cleanup, || e.advance(1));
    }
    let cleanup_done = e.cancel_done();
    let after = e.memory();
    let mut result = serde_json::json!({"status":status,"mode":mode_name,"steps":steps,"answers":answers,"applications":applications,"setup_seconds":setup_seconds,"source_seconds":source_seconds,"cleanup_seconds":cleanup.elapsed().as_secs_f64(),"cleanup_done":cleanup_done,"normalization":stats,"memory_before_cleanup":{"graph_nodes":memory.graph_nodes,"occurrences":memory.occurrences,"conditions":memory.conditions,"pending_nodes":memory.pending_nodes,"choices":memory.choices},"memory_after_cleanup":{"graph_nodes":after.graph_nodes,"occurrences":after.occurrences,"conditions":after.conditions,"pending_nodes":after.pending_nodes,"choices":after.choices}});
    #[cfg(feature = "diagnostics")]
    {
        result["allocations_before_cleanup"] = serde_json::to_value(allocation_before_cleanup)?;
        result["allocations_after_cleanup"] = serde_json::to_value(allocation::snapshot())?;
        result["engine_diagnostics"] = serde_json::to_value(diagnostics)?;
    }
    result["total_seconds"] = serde_json::json!(start.elapsed().as_secs_f64());
    eprintln!("{}", serde_json::to_string(&result)?);
    if !cleanup_done {
        return Err("bounded cancellation did not finish".into());
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
