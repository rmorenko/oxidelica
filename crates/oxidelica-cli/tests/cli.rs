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
