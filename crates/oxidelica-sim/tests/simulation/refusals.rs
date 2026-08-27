//! What a model that cannot be run is told, and how early.

use super::shared::*;
use oxidelica_parser::parse_model;
use oxidelica_sim::{compile, SolverMethod};

#[test]
fn a_run_that_cannot_go_on_says_so_by_the_method_it_was_asked_for() {
    // A block that has an answer where the run starts and none
    // where it is going: `y * y = 1 - x` runs out of real answers
    // once `x` passes one, whichever way the run is asked for.
    let source = "model M Real x(start = 0); Real y(start = 1); equation der(x) = 1; y * y = 1 - x; annotation(experiment(StopTime = 3, Interval = 0.1)); end M;";
    let model = parse_model(source).unwrap();
    for asked in ["adaptive", "bdf", "rk4", "auto"] {
        let mut compiled = compile(&model).unwrap();
        let outcome = match asked {
            "adaptive" => compiled.simulate_adaptive(),
            "bdf" => compiled.simulate_bdf(),
            "rk4" => compiled.simulate_rk4(),
            _ => {
                compiled.method = SolverMethod::Auto;
                compiled.simulate()
            }
        };
        assert!(outcome.is_err(), "{asked} should not have got through");
    }
}

#[test]
fn the_compiler_names_what_it_cannot_do() {
    assert!(refused("model M Real y; equation y = nowhere; end M;")
        .contains("unknown variable `nowhere`"));
    assert!(
        refused("model M Real y; equation y = atan2(1); end M;").contains("expects 2 arguments")
    );
    assert!(refused("model M Real y; equation y = made_up(1); end M;").contains("unknown function"));
    assert!(refused("model M Real y; equation y = pre(y); end M;").contains("is not discrete"));
    assert!(refused("model M discrete Real d(start = 0); Real y; equation y = 1; when sample(0, 0) then d = 1; end when; end M;")
        .contains("the interval must be positive"));
    assert!(
        refused("model M Real y; equation y = delay(time, 0); end M;")
            .contains("the delay must be positive")
    );
    assert!(
        refused("model M parameter Real p; Real y; equation y = p; end M;")
            .contains("has no value")
    );
}

#[test]
fn differentiation_says_what_it_cannot_reach_through() {
    // An index reduction differentiates the constraint, and what
    // it cannot differentiate it says so about: the pendulum below
    // is held by a length no derivative of ours can take apart.
    assert!(refused("model M Real x(start = 1); Real y(start = 0); Real vx(start = 0); Real vy(start = 0); Real lam; equation der(x) = vx; der(y) = vy; der(vx) = lam * x; der(vy) = lam * y - 9.81; x * x + atan2(y, x) = 1; end M;")
        .contains("differentiate"));
    // A model with more equations than unknowns says so by count.
    assert!(refused("model M Real x(start = 1); Real a; Real b; equation a = b; b = a; x * a = 1; der(x) = -x; end M;")
        .contains("unbalanced model"));
}

/// A parameter waiting on a call says which call, not which cycle.
///
/// A parameter written as a function of literals waits on nobody: it
/// names no free variable at all, so calling that a cycle names a
/// shape the model does not have.
#[test]
fn a_parameter_waiting_on_a_call_says_which_call() {
    // A call nothing works out, with nothing free in it.
    let why =
        compile_err("model M parameter Real a = nowhere(1, 2); Real x; equation x = 1; end M;");
    assert!(why.contains("nothing works out"), "{why}");
    assert!(why.contains("`nowhere`"), "{why}");

    // A name nothing declares is still named outright.
    let why = compile_err("model M parameter Real a = nowhere; Real x; equation x = 1; end M;");
    assert!(why.contains("nothing gives a value"), "{why}");

    // And two parameters naming each other are still a cycle.
    let why = compile_err(
        "model M parameter Real a = b; parameter Real b = a; Real x; equation x = 1; end M;",
    );
    assert!(why.contains("wait on each other"), "{why}");
}
