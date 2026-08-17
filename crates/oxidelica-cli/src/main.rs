//! M0 spike CLI: `oxidelica simulate model.mo [--stop T] [--dt H] [-o out.csv]`
//! and `oxidelica parse model.mo` to dump the compiled model structure.

use oxidelica_sim::compile;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "Oxidelica M0 - a Modelica subset simulator

Usage:
  oxidelica simulate <file.mo> [--stop T] [--dt H] [-o result.csv]
  oxidelica parse <file.mo>";

/// Dispatch the command line to a subcommand.
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str);

    match command {
        Some("simulate") => simulate(&args[1..]),
        Some("parse") => parse(&args[1..]),
        _ => Err(USAGE.to_string()),
    }
}

/// Read and parse a model file.
fn load(path: &str) -> Result<oxidelica_parser::Model, String> {
    let source = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    oxidelica_parser::parse_model(&source).map_err(|e| format!("{path}: {e}"))
}

/// `parse` subcommand: print the compiled model structure.
fn parse(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or(USAGE)?;
    let model = load(path)?;
    let compiled = compile(&model).map_err(|e| e.to_string())?;

    println!("model {}", compiled.name);
    if let Some(desc) = &model.description {
        println!("  \"{desc}\"");
    }
    println!("  parameters:");
    for (name, value) in &compiled.parameters {
        println!("    {name} = {value}");
    }
    println!("  states ({}):", compiled.states.len());
    for (name, init) in compiled.states.iter().zip(&compiled.initial) {
        println!("    {name}(start = {init})");
    }
    println!("  algebraic ({}):", compiled.algebraics.len());
    for name in &compiled.algebraics {
        println!("    {name}");
    }
    println!(
        "  experiment: stop = {}, step = {}",
        compiled.stop_time, compiled.step
    );
    Ok(())
}

/// `simulate` subcommand: run the model and emit CSV.
fn simulate(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or(USAGE)?;
    let mut stop: Option<f64> = None;
    let mut dt: Option<f64> = None;
    let mut out: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        let take_value = |i: &mut usize| -> Result<String, String> {
            *i += 1;
            args.get(*i)
                .cloned()
                .ok_or_else(|| format!("{} requires a value", args[*i - 1]))
        };
        match args[i].as_str() {
            "--stop" => {
                stop = Some(
                    take_value(&mut i)?
                        .parse()
                        .map_err(|e| format!("--stop: {e}"))?,
                )
            }
            "--dt" => {
                dt = Some(
                    take_value(&mut i)?
                        .parse()
                        .map_err(|e| format!("--dt: {e}"))?,
                )
            }
            "-o" | "--out" => out = Some(take_value(&mut i)?),
            other => return Err(format!("unknown flag `{other}`\n\n{USAGE}")),
        }
        i += 1;
    }

    let model = load(path)?;
    let mut compiled = compile(&model).map_err(|e| e.to_string())?;
    if let Some(t) = stop {
        compiled.stop_time = t;
    }
    if let Some(h) = dt {
        compiled.step = h;
    }

    let started = std::time::Instant::now();
    let result = compiled.simulate().map_err(|e| e.to_string())?;
    let elapsed = started.elapsed();

    match out {
        Some(file) => {
            std::fs::write(&file, result.to_csv())
                .map_err(|e| format!("cannot write {file}: {e}"))?;
            println!(
                "{}: {} steps in {:.1?} -> {}",
                compiled.name,
                result.rows.len() - 1,
                elapsed,
                file
            );
        }
        None => print!("{}", result.to_csv()),
    }

    // A short final-point summary on stderr so it does not mix with CSV
    // on stdout.
    if let Some(last) = result.rows.last() {
        let summary: Vec<String> = result
            .columns
            .iter()
            .zip(last)
            .map(|(name, value)| format!("{name} = {value:.6}"))
            .collect();
        eprintln!("final point: {}", summary.join(", "));
    }
    Ok(())
}
