//! M0 spike CLI: `oxidelica simulate model.mo [--stop T] [--dt H] [-o out.csv]`
//! and `oxidelica parse model.mo` to dump the compiled model structure.

use oxidelica_parser::{Variability, WhenAction};
use oxidelica_sim::compile;
use std::process::ExitCode;
use std::sync::atomic::Ordering;

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
  oxidelica why <file.mo|Library.Class> <variable>
                            where a variable's value comes from
  oxidelica library list
  oxidelica library add <name|git-url> [--version TAG] [--as NAME]
                            names: modelica (the Modelica Standard Library)
  oxidelica library check [<directory>] [--list] [--refused]

The standard library is looked for as `lib` next to the model, next to
the working directory or next to the binary, and among the libraries
`library add` has fetched; OXIDELICA_LIB names one outright and
MODELICAPATH names a list.

`library check` reads the models at once, on seven cores in ten, so
that the machine stays usable while the check runs. OXIDELICA_THREADS
says how many threads to use instead.";

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
        Some("why") => why(&args[1..]),
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
    // Where somebody asked to see where each name went: a refusal
    // that names a class of the base rather than of the medium at
    // hand is the whole of the media question, and the trail is what
    // says which of the two a name landed on.
    let _trail = std::env::var("OXIDELICA_NAME_TRAIL")
        .is_ok()
        .then(oxidelica_parser::Trail::kept);
    oxidelica_parser::parse_model_with_libraries(&libraries, &source)
        .map_err(|e| format!("{path}: {e}"))
}

/// Read a model named either as a file or as a class of the libraries.
///
/// Asking about a standard-library model otherwise means writing a
/// file whose only purpose is to name it, and the question is usually
/// asked about a library model: that is where the models that will not
/// run are. A name with a path separator or a `.mo` suffix is a file,
/// and anything else is looked up among the classes.
fn load_named(what: &str) -> Result<oxidelica_parser::Model, String> {
    let looks_like_a_file = what.ends_with(".mo")
        || what.contains('/')
        || what.contains('\\')
        || std::path::Path::new(what).exists();
    if looks_like_a_file {
        return load(what);
    }
    let files = oxidelica_parser::library_files(None);
    if files.is_empty() {
        return Err(format!(
            "no libraries to look for {what} in, and it is not a file"
        ));
    }
    let mut classes = Vec::new();
    for file in &files {
        let Ok(source) = std::fs::read_to_string(file) else {
            continue;
        };
        if let Ok(mut found) = oxidelica_parser::parse_file(&source) {
            classes.append(&mut found);
        }
    }
    if !classes.iter().any(|c| c.name == what) {
        return Err(format!("no class called {what} in the libraries"));
    }
    oxidelica_parser::flatten_named(&classes, what)
}

/// `why` subcommand: where a variable's value comes from.
///
/// Debugging a model that will not run means asking the same question
/// over and over: this variable has no value, or the wrong one, so who
/// was supposed to give it one? Answering it by reading the flat model
/// by hand is slow, because the flat model is thousands of lines and
/// the variable is mentioned in a dozen of them. This gathers the
/// dozen: what the variable was declared as, what its declaration
/// bound, and every equation that names it, on either side.
///
/// The model is taken as a file or as a class of the libraries, so a
/// standard-library model can be asked about without writing a file
/// that instantiates it.
fn why(args: &[String]) -> Result<(), String> {
    let what = args.first().ok_or(USAGE)?;
    let wanted = args.get(1).ok_or(USAGE)?;
    let model = load_named(what)?;

    println!("in {}, about `{wanted}`:", model.name);

    // Everything whose name is the one asked for, or which lives
    // inside it: asking about a record or a component is asking about
    // the fields underneath, and nobody wants to ask again for each.
    let named: Vec<&oxidelica_parser::Component> = model
        .components
        .iter()
        .filter(|c| c.name == *wanted || c.name.starts_with(&format!("{wanted}.")))
        .collect();

    if named.is_empty() {
        println!("  declared: nowhere - no component of the flat model is called that");
    }
    for component in &named {
        let variability = match component.variability {
            Variability::Parameter => "parameter ",
            Variability::Constant => "constant ",
            Variability::Discrete => "discrete ",
            Variability::Continuous => "",
        };
        println!(
            "  declared: {variability}{} {}",
            component.type_name, component.name
        );
        match &component.binding {
            Some(expr) => println!("    bound to: {}", expr.describe()),
            None => println!("    bound to: nothing"),
        }
        if let Some(expr) = &component.start {
            println!("    start: {}", expr.describe());
        }
        if let Some(fixed) = component.fixed {
            println!("    fixed: {fixed}");
        }
        if let Some(unit) = &component.unit {
            println!("    unit: {unit}");
        }
        // Where the declaration came from is the question behind the
        // question: a value that went missing went missing in some
        // class, and the flat name says which.
        if let Some((instance, _)) = component.name.rsplit_once('.') {
            println!("    inside: {instance}");
        }
    }

    let mentions = |side: &oxidelica_parser::Expr| {
        let mut found = false;
        side.for_each(&mut |part| {
            if let oxidelica_parser::Expr::Ref(name) = part {
                if name == wanted || name.starts_with(&format!("{wanted}.")) {
                    found = true;
                }
            }
        });
        found
    };

    let mut said = 0usize;
    for (what, equations) in [
        ("equation", &model.equations),
        ("initial equation", &model.initial_equations),
    ] {
        for item in equations.iter() {
            if !mentions(&item.lhs) && !mentions(&item.rhs) {
                continue;
            }
            said += 1;
            println!(
                "  {what}: {} = {}",
                item.lhs.describe(),
                item.rhs.describe()
            );
            if !item.origin.is_empty() {
                println!("    written in: {}", item.origin);
            }
        }
    }

    // A `when` is where a discrete variable gets its value, and a
    // discrete variable with no `when` is the usual reason one is
    // stuck at zero, so their absence is worth as much as their
    // presence.
    for clause in &model.when_clauses {
        for branch in &clause.branches {
            for action in &branch.actions {
                let (target, value) = match action {
                    WhenAction::Assign(name, value) => (name.clone(), value.describe()),
                    WhenAction::Reinit(name, value) => {
                        (name.clone(), format!("reinit to {}", value.describe()))
                    }
                    _ => continue,
                };
                if target != *wanted && !target.starts_with(&format!("{wanted}.")) {
                    continue;
                }
                said += 1;
                println!("  when {}: {target} = {value}", branch.condition.describe());
            }
        }
    }

    if said == 0 {
        println!("  named by: no equation of the flat model");
    }

    // What the compiler made of it, when it got that far. A variable
    // that flattened and still has no value is a different problem
    // from one that never reached the compiler, and the two look the
    // same until this is asked.
    match compile(&model) {
        Ok(compiled) => {
            let value = compiled
                .parameters
                .iter()
                .find(|(name, _)| name == wanted)
                .map(|(_, value)| *value);
            match value {
                Some(value) => println!("  settled as: a parameter worth {value}"),
                None if compiled.states.iter().any(|name| name == wanted) => {
                    println!("  settled as: a state of the run")
                }
                None if compiled.algebraics.iter().any(|name| name == wanted) => {
                    println!("  settled as: an algebraic variable of the run")
                }
                None => println!("  settled as: nothing the compiled model names"),
            }
        }
        Err(refusal) => println!("  the compiler refused the model: {refusal}"),
    }
    Ok(())
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
/// tells a step forward from a step sideways; `--refused` names each
/// model that did not, with what stopped it, which is what tells how
/// many barriers stand behind one another.
fn library_check(args: &[String]) -> Result<(), String> {
    let list = args.iter().any(|arg| arg == "--list");
    let refused_each = args.iter().any(|arg| arg == "--refused");
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
    // Reading the files is the first quiet minute of the check, so it
    // says how far along it is the same way the models below do.
    let quiet = std::env::var("OXIDELICA_QUIET").is_ok();
    let reading = std::time::Instant::now();
    let tenth = files.len().div_ceil(10).max(1);
    for (at, file) in files.iter().enumerate() {
        if !quiet && at > 0 && at % tenth == 0 {
            eprintln!(
                "  read {at} of {} files, {:.0}s so far",
                files.len(),
                reading.elapsed().as_secs_f64()
            );
        }
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
    // Which of them are meant to be run. A library is full of helper
    // models that live under Examples but are parts of examples rather
    // than examples - operational amplifier circuits with open pins,
    // controllers whose parameters the enclosing model sets. Counting
    // those as failures says nothing about the compiler. What marks a
    // real one is what a tool reads: an `experiment` annotation saying
    // how long to run, or the Example icon inherited for the browser.
    let by_name: std::collections::HashMap<&str, &oxidelica_parser::ClassDef> =
        classes.iter().map(|c| (c.name.as_str(), c)).collect();
    let runnable: std::collections::HashSet<&String> = models
        .iter()
        .filter(|name| {
            by_name
                .get(name.as_str())
                .is_some_and(|class| meant_to_run(class, &by_name, 0))
        })
        .collect();
    // Each model is read on its own and tells the others nothing, so
    // they are read at once. What a model comes to is put back in the
    // order the models were listed, so that the report reads the same
    // however many threads did the reading - a rank that moved with
    // the weather would be no measurement at all.
    //
    // Three cores in ten are left alone, and never fewer than one is
    // taken. Reading the standard library takes minutes, and taking
    // every core for all of them leaves the machine it runs on with
    // nothing to answer a keystroke with - the check is something a
    // person waits through, not a batch job. `OXIDELICA_THREADS` says
    // otherwise where the whole machine is the point.
    let hands = std::env::var("OXIDELICA_THREADS")
        .ok()
        .and_then(|given| given.parse().ok())
        .filter(|asked| *asked > 0)
        .unwrap_or_else(|| {
            let cores = std::thread::available_parallelism()
                .map(std::num::NonZeroUsize::get)
                .unwrap_or(1);
            (cores * 7).div_ceil(10).max(1)
        })
        .min(models.len().max(1));
    // Reading a library takes minutes, and a person waiting through it
    // deserves to know it is still going. The count is written to
    // standard error, so nothing that reads the report is disturbed by
    // it, and only on a tenth of the way, so a log that nobody watches
    // holds ten lines rather than a thousand.
    //
    // `OXIDELICA_QUIET` says nothing at all, for a caller that wants
    // the report and no company.
    let done = std::sync::atomic::AtomicUsize::new(0);
    let told = std::sync::atomic::AtomicUsize::new(0);
    let started = std::time::Instant::now();
    let mut answers: Vec<(usize, (Answer, Spent))> = Vec::new();
    std::thread::scope(|threads| {
        let classes = &classes;
        let models = &models;
        let done = &done;
        let told = &told;
        let taken: Vec<_> = (0..hands)
            .map(|hand| {
                // A thread of its own starts with a smaller stack than
                // the one a program starts on, and reading a model is
                // deep work: a class inside a class inside a class,
                // and an expression written out through all of them.
                std::thread::Builder::new()
                    .stack_size(64 * 1024 * 1024)
                    .spawn_scoped(threads, move || {
                        let mut mine = Vec::new();
                        for (at, name) in models.iter().enumerate().skip(hand).step_by(hands) {
                            if std::env::var("OXIDELICA_TRACE").is_ok() {
                                eprintln!("{name}");
                            }
                            mine.push((at, how_far(classes, name)));
                            let now = done.fetch_add(1, Ordering::Relaxed) + 1;
                            if quiet {
                                continue;
                            }
                            // Every tenth of the way. Several threads
                            // arrive at once and each would say a
                            // different number, so the one that moves
                            // the mark says the mark rather than its
                            // own count: the line then reads the same
                            // however many hands are at work, and the
                            // tenths come out in their order.
                            let tenth = models.len().div_ceil(10).max(1);
                            let reached = now / tenth;
                            let mut mark = told.load(Ordering::Relaxed);
                            while reached > mark {
                                match told.compare_exchange(
                                    mark,
                                    mark + 1,
                                    Ordering::Relaxed,
                                    Ordering::Relaxed,
                                ) {
                                    Ok(_) => {
                                        let so_far = (mark + 1) * tenth;
                                        eprintln!(
                                            "  read {} of {} models, {:.0}s so far",
                                            so_far.min(models.len()),
                                            models.len(),
                                            started.elapsed().as_secs_f64()
                                        );
                                        mark += 1;
                                    }
                                    Err(moved) => mark = moved,
                                }
                            }
                        }
                        mine
                    })
                    .expect("a thread to read models with")
            })
            .collect();
        for hand in taken {
            answers.extend(hand.join().unwrap_or_default());
        }
    });
    answers.sort_by_key(|(at, _)| *at);
    // The two halves apart, and how many models reached each: what
    // says whether the check grew because more of it passes or because
    // the same work grew slower.
    let mut flattening = std::time::Duration::ZERO;
    let mut running = std::time::Duration::ZERO;
    let mut ran_count = 0usize;
    for (_, (_, spent)) in &answers {
        flattening += spent.flattening;
        if !spent.running.is_zero() {
            ran_count += 1;
        }
        running += spent.running;
    }
    let answers: Vec<(usize, Answer)> = answers
        .into_iter()
        .map(|(at, (answer, _))| (at, answer))
        .collect();

    let mut flat: Vec<&String> = Vec::new();
    let mut why_not: Vec<(String, String)> = Vec::new();
    let mut ran: Vec<&String> = Vec::new();
    let mut would_not_run: Vec<(String, String)> = Vec::new();
    for (at, answer) in answers {
        let name = &models[at];
        match answer {
            Answer::Refused(why) => why_not.push((why, name.clone())),
            Answer::Flat(why) => {
                flat.push(name);
                would_not_run.push((why, name.clone()));
            }
            Answer::Ran => {
                flat.push(name);
                ran.push(name);
            }
        }
    }
    println!(
        "classes: {}; example models: {}, of which {} flatten and {} run",
        classes.len(),
        models.len(),
        flat.len(),
        ran.len()
    );
    let runnable_flat = flat.iter().filter(|name| runnable.contains(**name)).count();
    let runnable_ran = ran.iter().filter(|name| runnable.contains(**name)).count();
    println!(
        "runnable examples (experiment or Example icon): {}, of which {} flatten and {} run",
        runnable.len(),
        runnable_flat,
        runnable_ran
    );
    // What the work cost, by the half it was spent on and per model
    // that reached that half. The total alone cannot tell more models
    // passing - which is what makes it longer, since the ones that
    // newly pass are the dear ones - from the same work grown slower.
    // Per model it can: that number moving is a change in the
    // compiler, and the total moving alone is a change in coverage.
    let per = |spent: std::time::Duration, count: usize| match count {
        0 => 0.0,
        count => spent.as_secs_f64() * 1000.0 / count as f64,
    };
    println!(
        "time: flattening {:.0}s over {} models ({:.0}ms each); \
         running {:.0}s over {} ({:.0}ms each)",
        flattening.as_secs_f64(),
        models.len(),
        per(flattening, models.len()),
        running.as_secs_f64(),
        ran_count,
        per(running, ran_count)
    );
    if list {
        let mut named: Vec<&&String> = flat.iter().collect();
        named.sort();
        for name in named {
            println!("  flat  {name}");
        }
    }
    // What a model was refused for, model by model rather than gathered
    // into kinds. A count of a kind is the number of models standing at
    // that barrier, not the number a fix would release: behind one
    // barrier there is often another, and only the names say which
    // model is where. This is what to read before choosing what to
    // work on.
    if refused_each {
        let mut each: Vec<&(String, String)> = why_not.iter().collect();
        each.sort_by(|a, b| a.1.cmp(&b.1));
        for (why, name) in each {
            let head = &why[..why.len().min(100)];
            println!("  refused  {name}\t{head}");
        }
    }
    report(&why_not, "  ", list);
    println!("of the {} that flatten, {} run:", flat.len(), ran.len());
    // The same, for a model that was built and then would not run.
    // These are the larger half now - the models that flatten outnumber
    // the ones that run two to one - and until this was printed the
    // only way to see what they stand at was one model at a time.
    if refused_each {
        let mut each: Vec<&(String, String)> = would_not_run.iter().collect();
        each.sort_by(|a, b| a.1.cmp(&b.1));
        for (why, name) in each {
            let head = &why[..why.len().min(100)];
            println!("  built    {name}\t{head}");
        }
    }
    report(&would_not_run, "  ", list);
    Ok(())
}

/// Whether a model is written to be run, rather than to be a part of
/// something else that is.
///
/// Two things say so, and either is enough: an `experiment` annotation
/// with a stop time, which is a tool being told how long to run it, and
/// the `Icons.Example` icon, which is a model asking to be shown in the
/// browser as an example. The icon is often inherited through a local
/// template - `extends ExampleTemplate` where the template extends the
/// icon - so the extends chain is walked, resolving a written base
/// either by its full name or by its tail against the classes in view.
fn meant_to_run(
    class: &oxidelica_parser::ClassDef,
    by_name: &std::collections::HashMap<&str, &oxidelica_parser::ClassDef>,
    depth: usize,
) -> bool {
    if class.experiment.stop_time.is_some() {
        return true;
    }
    if depth > 8 {
        return false;
    }
    class.extends.iter().any(|ext| {
        if ext.base.contains("Icons.Example") {
            return true;
        }
        let named = by_name.get(ext.base.as_str()).copied().or_else(|| {
            let tail = format!(".{}", ext.base);
            by_name
                .iter()
                .find(|(name, _)| name.ends_with(&tail))
                .map(|(_, class)| *class)
        });
        named.is_some_and(|base| meant_to_run(base, by_name, depth + 1))
    })
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

/// How far one model got.
enum Answer {
    /// Flattening said no, and why.
    Refused(String),
    /// It flattened, and then would not run, and why.
    Flat(String),
    /// It flattened and took its steps.
    Ran,
}

/// Read one model as far as it goes.
fn how_far(classes: &[oxidelica_parser::ClassDef], name: &str) -> (Answer, Spent) {
    let mut spent = Spent::default();
    let started = std::time::Instant::now();
    let flattened = oxidelica_parser::flatten_named(classes, name);
    spent.flattening = started.elapsed();
    match flattened {
        // Flattening is not the whole of it. A flat model still has to
        // come out as something that runs, and a model that flattens
        // into equations nothing can solve is one this compiler has
        // said nothing true about.
        Ok(model) => {
            let started = std::time::Instant::now();
            let ran = run_a_little(&model);
            spent.running = started.elapsed();
            match ran {
                Ok(()) => (Answer::Ran, spent),
                Err(why) => (Answer::Flat(why), spent),
            }
        }
        Err(why) => (Answer::Refused(why), spent),
    }
}

/// How long one model took, by the half of the work it was spent on.
///
/// The whole check grows longer as more models pass, since a model
/// that used to be refused early now goes through everything - and the
/// ones that newly pass are the dear ones, which is why they were
/// stuck. Watching the total alone cannot tell that from the same work
/// grown slower, so the halves are counted apart and reported against
/// the number of models that reached each.
#[derive(Default, Clone, Copy)]
struct Spent {
    flattening: std::time::Duration,
    running: std::time::Duration,
}

/// Take a flat model as far as a few steps of a run.
///
/// A model that flattens has not been shown to be worth anything yet:
/// the equations still have to come out as something solvable, and the
/// first steps are where an unbalanced system, a name nothing settles
/// or a call nobody can answer makes itself known. Running to the end
/// would say more and cost minutes apiece; what is asked here is the
/// cheap half of the question.
fn run_a_little(model: &oxidelica_parser::Model) -> Result<(), String> {
    let mut compiled = compile(model).map_err(|e| e.to_string())?;
    // Ten steps of whatever the model calls a step, and never past
    // where the model itself stops: a model asking for a long run is
    // not made to do one, and one asking for a short run is not made
    // to overrun it.
    compiled.stop_time = (compiled.step * 10.0).min(compiled.stop_time);
    compiled.simulate().map(|_| ()).map_err(|e| e.to_string())
}
