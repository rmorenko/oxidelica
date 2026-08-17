//! Интеграционные тесты CLI: запускают настоящий бинарник `oxidelica`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_oxidelica"))
}

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Временный файл, уникальный на тест, удаляется в конце.
struct TempFile(PathBuf);

impl TempFile {
    fn new(name: &str, content: &str) -> TempFile {
        let path = std::env::temp_dir().join(format!("oxidelica-test-{}-{name}", std::process::id()));
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
    assert!(stderr(&out).contains("Использование"));
}

#[test]
fn unknown_command_prints_usage() {
    let out = bin().arg("frobnicate").output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("Использование"));
}

#[test]
fn parse_dumps_model_structure() {
    let out = bin().args(["parse", example("pendulum.mo").to_str().unwrap()]).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("модель Pendulum"));
    assert!(text.contains("состояния (2)"));
    assert!(text.contains("алгебраические (2)"));
    assert!(text.contains("g = 9.81"));
}

#[test]
fn simulate_writes_csv_to_stdout() {
    let out = bin()
        .args(["simulate", example("decay.mo").to_str().unwrap(), "--stop", "1", "--dt", "0.1"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let csv = stdout(&out);
    assert!(csv.starts_with("time,x"));
    assert_eq!(csv.lines().count(), 12); // заголовок + 11 точек (0..=1 с шагом 0.1)
    assert!(stderr(&out).contains("финальная точка"));
}

#[test]
fn simulate_writes_csv_to_file() {
    let result = std::env::temp_dir().join(format!("oxidelica-out-{}.csv", std::process::id()));
    let out = bin()
        .args(["simulate", example("decay.mo").to_str().unwrap(), "-o", result.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("шагов за"));
    let written = std::fs::read_to_string(&result).unwrap();
    assert!(written.starts_with("time,x"));
    let _ = std::fs::remove_file(result);
}

#[test]
fn simulate_reports_flag_errors() {
    let decay = example("decay.mo");
    let decay = decay.to_str().unwrap();

    let out = bin().args(["simulate", decay, "--stop"]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("требует значения"));

    let out = bin().args(["simulate", decay, "--stop", "abc"]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--stop"));

    let out = bin().args(["simulate", decay, "--nonsense"]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("неизвестный флаг"));

    let out = bin().args(["simulate", decay, "--dt"]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("требует значения"));

    let out = bin().args(["simulate", decay, "--dt", "xyz"]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--dt"));

    let out = bin().args(["simulate", decay, "-o"]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("требует значения"));
}

#[test]
fn missing_file_is_reported() {
    let out = bin().args(["simulate", "/nonexistent/model.mo"]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("не удалось прочитать"));
}

#[test]
fn parse_error_is_reported_with_line() {
    let bad = TempFile::new("bad.mo", "model M Real x end");
    let out = bin().args(["parse", bad.path()]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("строка"));
}

#[test]
fn compile_error_is_reported() {
    let bad = TempFile::new("nocov.mo", "model M Real x; Real y; equation x = 1; end M;");
    let out = bin().args(["simulate", bad.path()]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("нет уравнения"));
}

#[test]
fn simulation_runtime_error_is_reported() {
    let bad = TempFile::new("runtime.mo", "model M Real y; equation y = frob(1); end M;");
    let out = bin().args(["simulate", bad.path()]).output().unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("неизвестная функция"));
}

#[test]
fn output_write_error_is_reported() {
    let out = bin()
        .args(["simulate", example("decay.mo").to_str().unwrap(), "-o", "/nonexistent/dir/out.csv"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("не удалось записать"));
}
