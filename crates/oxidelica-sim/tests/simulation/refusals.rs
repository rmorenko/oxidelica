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
    assert!(refused("model M Real x(start = 1); Real y(start = 0); Real vx(start = 0); Real vy(start = 0); Real lam; equation der(x) = vx; der(y) = vy; der(vx) = lam * x; der(vy) = lam * y - 9.81; x * x + div(y, x) = 1; end M;")
        .contains("differentiate"));
    // A model with more equations than unknowns says so by count.
    assert!(refused("model M Real x(start = 1); Real a; Real b; equation a = b; b = a; x * a = 1; der(x) = -x; end M;")
        .contains("unbalanced model"));
}

/// And it names *which* expression, and of what kind.
///
/// A refusal that says only "this expression" is one line of the
/// register for every model that meets it, whatever the cause - ten of
/// them shared a line before this - so the kind is what splits the
/// families and the spelling is what finds the model.
#[test]
fn differentiation_names_the_expression_it_refused() {
    let why = refused("model M Real x(start = 1); Real y(start = 0); Real vx(start = 0); Real vy(start = 0); Real lam; equation der(x) = vx; der(y) = vy; der(vx) = lam * x; der(vy) = lam * y - 9.81; x * x + div(y, x) = 1; end M;");
    assert!(why.contains("a call of several arguments"), "{why}");
    assert!(why.contains("div(y, x)"), "{why}");
    assert!(
        !why.contains("cannot differentiate this expression"),
        "{why}"
    );
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
    // And what the evaluator said about it, which is the reason rather
    // than the name: two different faults in a walked body used to come
    // out as this one sentence.
    assert!(
        why.contains("the evaluator said: a: unknown function `nowhere`"),
        "{why}"
    );

    // A name nothing declares is still named outright.
    let why = compile_err("model M parameter Real a = nowhere; Real x; equation x = 1; end M;");
    assert!(why.contains("nothing gives a value"), "{why}");

    // And two parameters naming each other are still a cycle.
    let why = compile_err(
        "model M parameter Real a = b; parameter Real b = a; Real x; equation x = 1; end M;",
    );
    assert!(why.contains("wait on each other"), "{why}");
}

#[test]
fn a_constant_a_constructor_says_nothing_about_is_refused_not_zeroed() {
    // A constructor writes no constant, so its values line up with the
    // fields that are not ones and each constant's place is filled
    // from what the record declared it as. A constant with no
    // declaration to fill from was filled with zero - a number the
    // record never states, handed over quietly where a refusal naming
    // the field is owed, and a wrong number given quietly is the worst
    // thing this compiler can do.
    //
    // Declined where it is noticed, the field reaches the layer that
    // knows how to say what is missing, and the refusal names it.
    let refusal = compile_err(
        "model M record Inner constant Real K; parameter Real R = 1; end Inner; \
         parameter Inner d = Inner(R = 5); Real y; \
         equation y = d.R * time; end M;",
    );
    assert!(
        refusal.contains("d.K"),
        "the refusal names the field nothing gives a value to: {refusal}"
    );
}

#[test]
fn the_size_of_a_record_is_refused_rather_than_counted_by_its_fields() {
    // A name known only as one record spread into its fields, and
    // `size` answered with how many there were. That is how an array
    // of records that had lost its length gave three for four and ran.
    // Now it is refused, and the refusal names the variable.
    let why = parse_model(
        "model M record R Real a; Real b; Real c; end R; \
         R r(a = 1, b = 2, c = 3); Real y = size(r, 1) + time; end M;",
    )
    .expect_err("should have been refused while flattening")
    .message;
    assert!(why.contains("`size` of `r`"), "{why}");
    assert!(why.contains("only as one record"), "{why}");
}

#[test]
fn a_discrete_start_nobody_could_work_out_is_refused_rather_than_zero() {
    // `initialState(ls, 30020)` seeds a generator, and the walk of its
    // body does not finish: the tuple filled from the external call
    // inside the loop leaves `state` unknown. The start used to fall
    // to zero without a word, so the model ran from {0, 0} - which is
    // every `Xorshift64star` noise block of the library drawing 0.5
    // on every tick. A start that was written and could not be worked
    // out is not a start of zero, and the refusal names it.
    let why = refused(
        "model Z function random input Integer stateIn[2]; output Real result; \
           output Integer stateOut[2]; \
           external \"C\" ModelicaRandom_xorshift64star(stateIn, stateOut, result); \
         end random; \
         function initialState input Integer localSeed; input Integer globalSeed; \
           output Integer state[2]; protected Real r; \
         algorithm state := {localSeed, globalSeed}; \
           for i in 1:10 loop (r, state) := random(state); end for; \
         end initialState; \
         parameter Integer ls = 614657; \
         discrete Integer st[2](start = initialState(ls, 30020)); \
         equation when sample(0, 0.1) then st = pre(st); end when; \
         annotation(experiment(StopTime = 0.2, Interval = 0.1)); end Z;",
    );
    assert!(
        why.contains("the start of the discrete variable `st[1]`"),
        "{why}"
    );
    assert!(
        why.contains("could not be worked out before the run"),
        "{why}"
    );
}
