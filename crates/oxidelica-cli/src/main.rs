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
  oxidelica simulate <file.mo> [--stop T] [--dt H] [--solver NAME] [-o result.csv]
                            solvers: auto (default), dopri45, bdf (stiff), rk4
  oxidelica parse <file.mo>

The standard library is looked for as `lib` next to the model, next to
the working directory or next to the binary; OXIDELICA_LIB names it
outright.";

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

/// Read and parse a model file in the context of the libraries, which
/// are looked for near the model itself as well as near the binary - a
/// model does not have to sit in the project to use them.
fn load(path: &str) -> Result<oxidelica_parser::Model, String> {
    let source = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let libraries = oxidelica_parser::library_sources(Some(std::path::Path::new(path)));
    oxidelica_parser::parse_model_with_libraries(&libraries, &source)
        .map_err(|e| format!("{path}: {e}"))
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
        "  jacobian: {} evaluation(s) per refresh for {} state(s)",
        compiled.jacobian_cost(),
        compiled.states.len()
    );
    println!("  evaluation plan:");
    for line in compiled.plan_summary() {
        println!("    {line}");
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
    let mut solver: Option<oxidelica_sim::SolverMethod> = None;

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
            "--solver" => {
                let name = take_value(&mut i)?;
                solver = Some(
                    oxidelica_sim::SolverMethod::from_name(&name)
                        .ok_or_else(|| format!("unknown solver `{name}`"))?,
                );
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
    if let Some(method) = solver {
        compiled.method = method;
    }

    let started = std::time::Instant::now();
    let result = compiled.simulate().map_err(|e| e.to_string())?;
    let elapsed = started.elapsed();

    match out {
        Some(file) => {
            std::fs::write(&file, result.to_csv())
                .map_err(|e| format!("cannot write {file}: {e}"))?;
            println!(
                "{}: {} steps in {:.1?} ({}) -> {}",
                compiled.name,
                result.rows.len() - 1,
                elapsed,
                result.method.name(),
                file
            );
        }
        None => print!("{}", result.to_csv()),
    }

    if let Some(message) = &result.terminated {
        eprintln!("{message}");
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
