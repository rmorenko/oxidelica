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
  oxidelica library list
  oxidelica library add <name|git-url> [--version TAG] [--as NAME]
                            names: modelica (the Modelica Standard Library)
  oxidelica library check [<directory>] [--list]

The standard library is looked for as `lib` next to the model, next to
the working directory or next to the binary, and among the libraries
`library add` has fetched; OXIDELICA_LIB names one outright and
MODELICAPATH names a list.";

/// Where the Modelica Standard Library is fetched from, and the release
/// fetched when none is asked for.
const MSL_REPOSITORY: &str = "https://github.com/modelica/ModelicaStandardLibrary.git";
const MSL_VERSION: &str = "v4.1.0";

/// Dispatch the command line to a subcommand.
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str);

    match command {
        Some("simulate") => simulate(&args[1..]),
        Some("parse") => parse(&args[1..]),
        Some("library") => library(&args[1..]),
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

/// `library` subcommand: fetch a library, say which are here, or
/// measure how much of one this compiler can read.
fn library(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("list") => library_list(),
        Some("add") => library_add(&args[1..]),
        Some("check") => library_check(&args[1..]),
        _ => Err(USAGE.to_string()),
    }
}

/// The libraries in view, and where each was found.
fn library_list() -> Result<(), String> {
    let directories = oxidelica_parser::library_directories(None);
    if directories.is_empty() {
        println!("no libraries in view");
        if let Some(root) = oxidelica_parser::download_root() {
            println!(
                "`oxidelica library add modelica` would put one in {}",
                root.display()
            );
        }
        return Ok(());
    }
    for directory in directories {
        let files = oxidelica_parser::library_files_in(&directory);
        println!("{} ({} file(s))", directory.display(), files.len());
    }
    Ok(())
}

/// `library add <name|url> [--version TAG] [--as NAME]`: clone a
/// library into the place the search already looks.
///
/// The fetching is `git clone --depth 1`, run as a command rather than
/// spoken over HTTP from here: a library is a git repository, `git`
/// checks what it downloads against the tag it was asked for, and this
/// compiler stays free of a network stack of its own.
fn library_add(args: &[String]) -> Result<(), String> {
    let what = args.first().ok_or(USAGE)?;
    let (mut url, mut name, mut version) = match what.as_str() {
        "modelica" | "msl" => (
            MSL_REPOSITORY.to_string(),
            "Modelica".to_string(),
            MSL_VERSION.to_string(),
        ),
        url => (url.to_string(), name_of_repository(url), "main".to_string()),
    };
    let mut i = 1;
    while i < args.len() {
        let value = |i: &mut usize| -> Result<String, String> {
            *i += 1;
            args.get(*i)
                .cloned()
                .ok_or_else(|| format!("{} requires a value", args[*i - 1]))
        };
        match args[i].as_str() {
            "--version" => version = value(&mut i)?,
            "--as" => name = value(&mut i)?,
            "--from" => url = value(&mut i)?,
            other => return Err(format!("unknown flag `{other}`\n\n{USAGE}")),
        }
        i += 1;
    }
    let root = oxidelica_parser::download_root()
        .ok_or("no home directory to keep libraries in; set MODELICAPATH instead")?;
    let target = root.join(&name);
    if target.exists() {
        return Err(format!(
            "{} is already there; remove it to fetch again",
            target.display()
        ));
    }
    std::fs::create_dir_all(&root).map_err(|e| format!("cannot make {}: {e}", root.display()))?;
    println!("fetching {url} at {version} into {}", target.display());
    let status = std::process::Command::new("git")
        .args(["clone", "--depth", "1", "--branch", &version, &url])
        .arg(&target)
        .status()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !status.success() {
        return Err(format!("git could not fetch {url} at {version}"));
    }
    println!("{name} is in {}", target.display());
    Ok(())
}

/// The last part of a repository URL, without the `.git`.
fn name_of_repository(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("library")
        .trim_end_matches(".git")
        .to_string()
}

/// `library check [<directory>] [--list]`: read every file of a library
/// and every model in it, and say how far each got. What it prints is a
/// count and the reasons that came up most, so that the distance
/// between this compiler and a library is a number rather than an
/// impression. `--list` names the models that flatten, which is what
/// tells a step forward from a step sideways.
fn library_check(args: &[String]) -> Result<(), String> {
    let list = args.iter().any(|arg| arg == "--list");
    let directory = args.iter().find(|arg| !arg.starts_with("--"));
    let files = match directory {
        Some(path) => oxidelica_parser::library_files_in(std::path::Path::new(path)),
        None => oxidelica_parser::library_files(None),
    };
    if files.is_empty() {
        return Err("no Modelica files to read".to_string());
    }
    let mut classes = Vec::new();
    let (mut read, mut unread) = (0usize, 0usize);
    let mut refusals: Vec<(String, String)> = Vec::new();
    for file in &files {
        let Ok(source) = std::fs::read_to_string(file) else {
            continue;
        };
        match oxidelica_parser::parse_file(&source) {
            Ok(mut found) => {
                read += 1;
                classes.append(&mut found);
            }
            Err(why) => {
                unread += 1;
                refusals.push((why.message, file.display().to_string()));
            }
        }
    }
    println!("files: {read} read, {unread} not read");
    report(&refusals, "  ", list);
    let models: Vec<String> = classes
        .iter()
        .filter(|c| c.kind.is_model() && !c.partial)
        .filter(|c| c.name.contains(".Examples.") || c.name.contains(".Test"))
        .map(|c| c.name.clone())
        .collect();
    let mut flat: Vec<&String> = Vec::new();
    let mut why_not: Vec<(String, String)> = Vec::new();
    for name in &models {
        if std::env::var("OXIDELICA_TRACE").is_ok() {
            eprintln!("{name}");
        }
        match oxidelica_parser::flatten_named(&classes, name) {
            Ok(_) => flat.push(name),
            Err(why) => why_not.push((why, name.clone())),
        }
    }
    println!(
        "classes: {}; example models: {}, of which {} flatten",
        classes.len(),
        models.len(),
        flat.len()
    );
    if list {
        let mut named: Vec<&&String> = flat.iter().collect();
        named.sort();
        for name in named {
            println!("  flat  {name}");
        }
    }
    report(&why_not, "  ", list);
    Ok(())
}

/// The reasons that came up most, with how often each did and one of
/// the things it came up on - which is where to start reading.
fn report(reasons: &[(String, String)], indent: &str, all: bool) {
    let mut counted: std::collections::BTreeMap<&str, (usize, &str)> =
        std::collections::BTreeMap::new();
    for (reason, what) in reasons {
        // The head of the message is what groups them: the tail names
        // the class or the token, and would make every one its own.
        let head = &reason[..reason.len().min(60)];
        let entry = counted.entry(head).or_insert((0, what.as_str()));
        entry.0 += 1;
    }
    let mut ranked: Vec<(&&str, &(usize, &str))> = counted.iter().collect();
    ranked.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(b.0)));
    // Ten is enough to say what to work on next; the whole of it is
    // what says how long the tail is, and `--list` is the flag for
    // wanting everything.
    let shown = match all {
        true => ranked.len(),
        false => 10,
    };
    for (reason, (count, what)) in ranked.iter().take(shown) {
        println!("{indent}{count:5}  {reason}");
        println!("{indent}       first on {what}");
    }
    if ranked.len() > shown {
        println!("{indent}       and {} more kind(s)", ranked.len() - shown);
    }
}
