//! CLI integration tests: they run the real `oxidelica` binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_oxidelica"))
}

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A per-test temporary file removed on drop.
struct TempFile(PathBuf);

impl TempFile {
    fn new(name: &str, content: &str) -> TempFile {
        let path =
            std::env::temp_dir().join(format!("oxidelica-test-{}-{name}", std::process::id()));
        std::fs::write(&path, content).unwrap();
        TempFile(path)
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn no_args_prints_usage() {
    let out = bin().output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("Usage"));
}

#[test]
fn unknown_command_prints_usage() {
    let out = bin().arg("frobnicate").output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("Usage"));
}

#[test]
fn parse_dumps_model_structure() {
    let out = bin()
        .args(["parse", example("pendulum.mo").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("model Pendulum"));
    assert!(text.contains("states (2)"));
    assert!(text.contains("algebraic (2)"));
    assert!(text.contains("g = 9.81"));
}

#[test]
fn why_says_where_a_value_comes_from() {
    let model = TempFile::new(
        "why.mo",
        "model M parameter Real k(unit = \"m\") = 3; Real x(start = 1); \
         Real y; equation der(x) = -k * x; y = x + k; end M;",
    );
    let out = bin().args(["why", model.path(), "k"]).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    // What it was declared as, what gave it a value, and the unit it
    // carries: the three things asked of a parameter.
    assert!(text.contains("parameter Real k"), "{text}");
    assert!(text.contains("bound to: 3"), "{text}");
    assert!(text.contains("unit: m"), "{text}");
    // Every equation naming it, on either side.
    assert!(text.contains("y = x + k"), "{text}");
    assert!(text.contains("der(x) = -(k * x)"), "{text}");
    assert!(text.contains("a parameter worth 3"), "{text}");

    // A state is named as one, and its start is shown.
    let out = bin().args(["why", model.path(), "x"]).output().unwrap();
    let text = stdout(&out);
    assert!(text.contains("start: 1"), "{text}");
    assert!(text.contains("a state of the run"), "{text}");

    // An algebraic variable likewise.
    let out = bin().args(["why", model.path(), "y"]).output().unwrap();
    assert!(
        stdout(&out).contains("an algebraic variable of the run"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn why_says_so_when_there_is_nothing_to_say() {
    let model = TempFile::new(
        "why-nothing.mo",
        "model M Real x(start = 1); equation der(x) = -x; end M;",
    );
    let out = bin()
        .args(["why", model.path(), "nowhere"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("declared: nowhere"), "{text}");
    assert!(text.contains("no equation of the flat model"), "{text}");

    // A model the compiler will not take is still worth asking about:
    // the declaration is there to read even where the run is not.
    let broken = TempFile::new(
        "why-broken.mo",
        "model M parameter Real a = missing; Real x; equation x = a; end M;",
    );
    let out = bin().args(["why", broken.path(), "a"]).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("bound to: missing"), "{text}");
    assert!(text.contains("the compiler refused"), "{text}");
}

#[test]
fn why_wants_a_model_and_a_variable() {
    let out = bin().arg("why").output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("Usage"));

    let model = TempFile::new("why-alone.mo", "model M Real x; equation x = 1; end M;");
    let out = bin().args(["why", model.path()]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("Usage"));

    // A name that is neither a file nor a class of the libraries says
    // which of the two it failed to be.
    let out = bin()
        .args(["why", "Nowhere.At.All", "x"])
        .env("OXIDELICA_LIB", "")
        .env("MODELICAPATH", "")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let text = stderr(&out);
    assert!(text.contains("Nowhere.At.All"), "{text}");
}

#[test]
fn why_reads_a_record_field_by_field() {
    // Asking about a record asks about the fields underneath it, which
    // is where a value goes missing.
    let model = TempFile::new(
        "why-record.mo",
        "record P parameter Real a = 1; parameter Real b = 2; end P; \
         model M parameter P p; Real x; equation x = p.a + p.b; end M;",
    );
    let out = bin().args(["why", model.path(), "p"]).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("p.a"), "{text}");
    assert!(text.contains("p.b"), "{text}");
    assert!(text.contains("inside: p"), "{text}");
}

#[test]
fn why_finds_the_when_that_settles_a_discrete_variable() {
    let model = TempFile::new(
        "why-when.mo",
        "model M discrete Real held(start = 0); Real x; \
         equation x = time; \
         when time > 0.5 then held = x; end when; end M;",
    );
    let out = bin().args(["why", model.path(), "held"]).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("discrete Real held"), "{text}");
    assert!(text.contains("when time > 0.5: held = x"), "{text}");
}

#[test]
fn simulate_writes_csv_to_stdout() {
    let out = bin()
        .args([
            "simulate",
            example("decay.mo").to_str().unwrap(),
            "--stop",
            "1",
            "--dt",
            "0.1",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let csv = stdout(&out);
    assert!(csv.starts_with("time,x"));
    assert_eq!(csv.lines().count(), 12); // header + 11 points (0..=1, step 0.1)
    assert!(stderr(&out).contains("final point"));
}

#[test]
fn simulate_writes_csv_to_file() {
    let result = std::env::temp_dir().join(format!("oxidelica-out-{}.csv", std::process::id()));
    let out = bin()
        .args([
            "simulate",
            example("decay.mo").to_str().unwrap(),
            "-o",
            result.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("steps in"));
    let written = std::fs::read_to_string(&result).unwrap();
    assert!(written.starts_with("time,x"));
    let _ = std::fs::remove_file(result);
}

#[test]
fn the_solver_may_be_chosen_on_the_command_line() {
    let decay = example("decay.mo");
    let decay = decay.to_str().unwrap();
    for name in ["auto", "dopri45", "rk4", "bdf"] {
        let out = bin()
            .args(["simulate", decay, "--solver", name, "--stop", "0.5"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{name}: {}", stderr(&out));
        assert!(stderr(&out).contains("final point:"), "{name}");
    }
    let out = bin()
        .args(["simulate", decay, "--solver", "nonsense"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("unknown solver `nonsense`"));
}

#[test]
fn a_run_that_stops_early_says_so() {
    // `terminate` ends the run, and the message reaches the terminal.
    let ball = example("bouncing_ball.mo");
    let out = bin()
        .args(["simulate", ball.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("final point:"));
}

#[test]
fn simulate_reports_flag_errors() {
    let decay = example("decay.mo");
    let decay = decay.to_str().unwrap();

    let out = bin().args(["simulate", decay, "--stop"]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("requires a value"));

    let out = bin()
        .args(["simulate", decay, "--stop", "abc"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--stop"));

    let out = bin()
        .args(["simulate", decay, "--nonsense"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("unknown flag"));

    let out = bin().args(["simulate", decay, "--dt"]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("requires a value"));

    let out = bin()
        .args(["simulate", decay, "--dt", "xyz"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--dt"));

    let out = bin().args(["simulate", decay, "-o"]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("requires a value"));
}

#[test]
fn missing_file_is_reported() {
    let out = bin()
        .args(["simulate", "/nonexistent/model.mo"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("cannot read"));
}

#[test]
fn parse_error_is_reported_with_line() {
    let bad = TempFile::new("bad.mo", "model M Real x end");
    let out = bin().args(["parse", bad.path()]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("line"));
}

#[test]
fn compile_error_is_reported() {
    let bad = TempFile::new("nocov.mo", "model M Real x; Real y; equation x = 1; end M;");
    let out = bin().args(["simulate", bad.path()]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("unbalanced"));
}

#[test]
fn simulation_runtime_error_is_reported() {
    let bad = TempFile::new("runtime.mo", "model M Real y; equation y = frob(1); end M;");
    let out = bin().args(["simulate", bad.path()]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("unknown function"));
}

#[test]
fn output_write_error_is_reported() {
    let out = bin()
        .args([
            "simulate",
            example("decay.mo").to_str().unwrap(),
            "-o",
            "/nonexistent/dir/out.csv",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("cannot write"));
}

/// A throwaway directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let path =
            std::env::temp_dir().join(format!("oxidelica-lib-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_library_is_measured_by_reading_it() {
    let library = TempDir::new("check");
    std::fs::write(
        library.0.join("Good.mo"),
        "package Good package Examples model Run Real x(start = 1); \
         equation der(x) = -x; end Run; end Examples; end Good;",
    )
    .unwrap();
    std::fs::write(library.0.join("Bad.mo"), "model B Real x @ 1; end B;").unwrap();
    let out = bin()
        .args(["library", "check", library.0.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("1 read, 1 not read"), "{text}");
    assert!(text.contains("1, of which 1 flatten"), "{text}");
}

#[test]
fn checking_nothing_says_so() {
    let empty = TempDir::new("empty");
    let out = bin()
        .args(["library", "check", empty.0.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("no Modelica files"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn adding_a_library_that_is_already_there_is_refused() {
    let home = TempDir::new("home");
    let already = home.0.join("oxidelica/libraries/Modelica");
    std::fs::create_dir_all(&already).unwrap();
    let out = bin()
        .args(["library", "add", "modelica"])
        .env("XDG_DATA_HOME", &home.0)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("already there"), "{}", stderr(&out));
}

#[test]
fn the_libraries_in_view_are_listed() {
    let library = TempDir::new("list");
    std::fs::write(library.0.join("Tiny.mo"), "package Tiny end Tiny;").unwrap();
    let out = bin()
        .args(["library", "list"])
        .env("OXIDELICA_LIB", &library.0)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("1 file(s)"), "{}", stdout(&out));

    // An unknown word after `library` prints the usage.
    let out = bin().args(["library", "frobnicate"]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("Usage:"));
}

#[test]
fn a_library_is_fetched_into_the_place_the_search_looks() {
    // A local repository stands in for a remote one: the fetching is
    // `git clone`, and git does not care which of the two it is.
    let source = TempDir::new("repo");
    std::fs::write(
        source.0.join("Tiny.mo"),
        "package Tiny model Run Real x(start = 1); equation der(x) = -x; end Run; end Tiny;",
    )
    .unwrap();
    let git = |args: &[&str]| {
        let done = Command::new("git")
            .args(args)
            .current_dir(&source.0)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .unwrap();
        assert!(done.status.success(), "git {args:?}: {}", stderr(&done));
    };
    git(&["-c", "init.defaultBranch=main", "init", "-q"]);
    git(&["add", "Tiny.mo"]);
    git(&["commit", "-qm", "a library"]);

    let home = TempDir::new("home-add");
    let out = bin()
        .args([
            "library",
            "add",
            source.0.to_str().unwrap(),
            "--version",
            "main",
            "--as",
            "Tiny",
        ])
        .env("XDG_DATA_HOME", &home.0)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(home.0.join("oxidelica/libraries/Tiny/Tiny.mo").exists());

    // And a model that uses it now finds it without being told where.
    let model = TempFile::new("uses-tiny.mo", "model M Tiny.Run r; end M;");
    let out = bin()
        .args(["parse", model.path()])
        .env("XDG_DATA_HOME", &home.0)
        .env_remove("OXIDELICA_LIB")
        .env_remove("MODELICAPATH")
        .current_dir(&home.0)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("r.x"), "{}", stdout(&out));

    // A version that is not there is a failure of the fetching, said
    // as one.
    let again = TempDir::new("home-again");
    let out = bin()
        .args([
            "library",
            "add",
            "anything",
            "--from",
            source.0.to_str().unwrap(),
            "--version",
            "no-such-tag",
        ])
        .env("XDG_DATA_HOME", &again.0)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("could not fetch"), "{}", stderr(&out));
}

#[test]
fn adding_a_library_reads_its_flags() {
    // A flag nobody knows, and one with nothing after it.
    let out = bin()
        .args(["library", "add", "x", "--wat"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("unknown flag"), "{}", stderr(&out));
    let out = bin()
        .args(["library", "add", "x", "--version"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("requires a value"),
        "{}",
        stderr(&out)
    );

    // With no name given, the last part of the URL is the library's.
    let home = TempDir::new("home-named");
    let out = bin()
        .args(["library", "add", "https://example.invalid/Tiny.git"])
        .env("XDG_DATA_HOME", &home.0)
        .output()
        .unwrap();
    assert!(!out.status.success());
    // The last part of the URL is the name, and the name is the last
    // part of where it goes - said without spelling a separator, which
    // is not the same character everywhere.
    let said = stdout(&out) + &stderr(&out);
    let line = said
        .lines()
        .find(|line| line.starts_with("fetching"))
        .unwrap_or_else(|| panic!("{said}"));
    assert!(line.trim_end().ends_with("Tiny"), "{line}");
}

#[test]
fn checking_a_library_ranks_what_it_could_not_read() {
    // More kinds of trouble than the report prints, so the tail is
    // counted rather than dropped in silence.
    let library = TempDir::new("many");
    for (index, source) in [
        "model A Real x @ 1; end A;",
        "model B Real x; equation x = ; end B;",
        "model C Real x[; end C;",
        "model D Real x; equation x = 1 end D;",
        "model E Real x; equation x = 1; end Z;",
        "package F model G Real x; end H; end F;",
        "model I Real x(unit = 3); end I;",
        "model J Real x; algorithm x = 1; end J;",
        "model K import L.{A B}; end K;",
        "model L Real x; equation assert(x > 0, 1); end L;",
        "model N Real x(start = 1 fixed = true); end N;",
        "model O connector Pin end Q; end O;",
    ]
    .iter()
    .enumerate()
    {
        std::fs::write(library.0.join(format!("B{index}.mo")), source).unwrap();
    }
    let out = bin()
        .args(["library", "check", library.0.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("0 read, 12 not read"), "{text}");
    assert!(text.contains("more kind(s)"), "{text}");
}

#[test]
fn checking_with_no_directory_reads_what_is_in_view() {
    // No directory named: the library the search already finds.
    let out = bin().args(["library", "check"]).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("files:"), "{}", stdout(&out));

    // An example that will not flatten is counted and its reason
    // ranked alongside the rest.
    let library = TempDir::new("examples");
    std::fs::write(
        library.0.join("Lib.mo"),
        "package Lib package Examples \
         model Good Real x(start = 1); equation der(x) = -x; end Good; \
         model Bad Missing m; Real x; equation x = m.k; end Bad; \
         end Examples; end Lib;",
    )
    .unwrap();
    let out = bin()
        .args(["library", "check", library.0.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("2, of which 1 flatten"), "{text}");
    assert!(text.contains("unknown type `Missing`"), "{text}");
}

#[test]
fn with_no_home_there_is_nowhere_to_keep_a_library() {
    let out = bin()
        .args(["library", "add", "modelica"])
        .env_remove("HOME")
        .env_remove("USERPROFILE")
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("no home directory"),
        "{}",
        stderr(&out)
    );
}
