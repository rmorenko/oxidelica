//! CLI спайка M0: `oxidelica simulate model.mo [--stop T] [--dt H] [-o out.csv]`
//! и `oxidelica parse model.mo` для дампа скомпилированной модели.

use oxidelica_sim::compile;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("ошибка: {message}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "Oxidelica M0 — симулятор среза Modelica

Использование:
  oxidelica simulate <файл.mo> [--stop T] [--dt H] [-o результат.csv]
  oxidelica parse <файл.mo>";

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str);

    match command {
        Some("simulate") => simulate(&args[1..]),
        Some("parse") => parse(&args[1..]),
        _ => Err(USAGE.to_string()),
    }
}

fn load(path: &str) -> Result<oxidelica_parser::Model, String> {
    let source =
        std::fs::read_to_string(path).map_err(|e| format!("не удалось прочитать {path}: {e}"))?;
    oxidelica_parser::parse_model(&source).map_err(|e| format!("{path}: {e}"))
}

fn parse(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or(USAGE)?;
    let model = load(path)?;
    let compiled = compile(&model).map_err(|e| e.to_string())?;

    println!("модель {}", compiled.name);
    if let Some(desc) = &model.description {
        println!("  «{desc}»");
    }
    println!("  параметры:");
    for (name, value) in &compiled.parameters {
        println!("    {name} = {value}");
    }
    println!("  состояния ({}):", compiled.states.len());
    for (name, init) in compiled.states.iter().zip(&compiled.initial) {
        println!("    {name}(start = {init})");
    }
    println!("  алгебраические ({}):", compiled.algebraics.len());
    for name in &compiled.algebraics {
        println!("    {name}");
    }
    println!(
        "  эксперимент: stop = {}, шаг = {}",
        compiled.stop_time, compiled.step
    );
    Ok(())
}

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
                .ok_or_else(|| format!("{} требует значения", args[*i - 1]))
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
            other => return Err(format!("неизвестный флаг «{other}»\n\n{USAGE}")),
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
                .map_err(|e| format!("не удалось записать {file}: {e}"))?;
            println!(
                "{}: {} шагов за {:.1?} → {}",
                compiled.name,
                result.rows.len() - 1,
                elapsed,
                file
            );
        }
        None => print!("{}", result.to_csv()),
    }

    // короткая сводка финальной точки в stderr, чтобы не мешать CSV в stdout
    if let Some(last) = result.rows.last() {
        let summary: Vec<String> = result
            .columns
            .iter()
            .zip(last)
            .map(|(name, value)| format!("{name} = {value:.6}"))
            .collect();
        eprintln!("финальная точка: {}", summary.join(", "));
    }
    Ok(())
}
