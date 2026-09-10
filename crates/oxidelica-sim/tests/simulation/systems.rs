//! Systems solved rather than evaluated: linear blocks, algebraic loops, and what a singular one says.

use super::shared::*;
use oxidelica_parser::parse_model;
use oxidelica_sim::{compile, lower_the_ceiling_here, SolverMethod};

#[test]
fn algebraic_chain_is_ordered() {
    // y depends on x, x on the state; declaration order is reversed.
    let result = run("model A Real s(start = 1.0); Real y; Real x; \
         equation der(s) = -s; y = 2*x; x = s + 1; \
         annotation(experiment(StopTime=1.0, Interval=0.01)); end A;");
    let first = &result.rows[0];
    // Columns: time, s, x, y (algebraics in evaluation order).
    assert_eq!(result.columns, vec!["time", "s", "x", "y"]);
    assert!((first[2] - 2.0).abs() < 1e-12); // x = s+1 = 2
    assert!((first[3] - 4.0).abs() < 1e-12); // y = 2x = 4
}

#[test]
fn solves_implicit_linear_system() {
    // x + y = 2 and x - y = 0 are not assignments - the matcher
    // pairs them with x and y and Newton solves the block.
    let result = run("model I Real x; Real y; equation x + y = 2; x - y = 0; \
         annotation(experiment(StopTime=0.01, Interval=0.01)); end I;");
    let x_idx = result.columns.iter().position(|c| c == "x").unwrap();
    let y_idx = result.columns.iter().position(|c| c == "y").unwrap();
    assert!((result.rows[0][x_idx] - 1.0).abs() < 1e-9);
    assert!((result.rows[0][y_idx] - 1.0).abs() < 1e-9);
}

#[test]
fn degenerate_algebraic_cycle_is_rejected_at_compile_time() {
    // x = y + 1 and y = x - 1 are the same equation twice: the loop
    // is structurally sound but has a whole family of solutions, so
    // the regularity check rejects it before any stepping happens.
    let model =
        parse_model("model C Real x; Real y; equation x = y + 1; y = x - 1; end C;").unwrap();
    let error = compile(&model).unwrap_err();
    assert!(error.0.contains("underdetermined"), "{}", error.0);
}

#[test]
fn solves_linear_algebraic_loop() {
    // x = y/2 + 1, y = x/2 + 1  ->  x = y = 2.
    let result = run(
        "model L Real x; Real y; equation x = y / 2 + 1; y = x / 2 + 1; \
         annotation(experiment(StopTime=0.01, Interval=0.01)); end L;",
    );
    let x_idx = result.columns.iter().position(|c| c == "x").unwrap();
    let y_idx = result.columns.iter().position(|c| c == "y").unwrap();
    assert!((result.rows[0][x_idx] - 2.0).abs() < 1e-9);
    assert!((result.rows[0][y_idx] - 2.0).abs() < 1e-9);
}

#[test]
fn solves_nonlinear_self_reference() {
    // x = cos(x): the Dottie number 0.739085...
    let result = run("model D Real x(start = 1); equation x = cos(x); \
         annotation(experiment(StopTime=0.01, Interval=0.01)); end D;");
    assert!(
        (result.rows[0][1] - 0.739_085_133_2).abs() < 1e-8,
        "{}",
        result.rows[0][1]
    );
}

#[test]
fn algebraic_loop_follows_a_state() {
    // The loop depends on a state: x = y/2 + s, y = x/2, so
    // x = (2/3) s ... wait: x = y/2 + s and y = x/2 -> x = x/4 + s
    // -> x = (4/3) s, y = (2/3) s.
    let result = run("model F Real s(start = 3.0); Real x; Real y; equation \
         der(s) = 0; x = y / 2 + s; y = x / 2; \
         annotation(experiment(StopTime=0.01, Interval=0.01)); end F;");
    let x_idx = result.columns.iter().position(|c| c == "x").unwrap();
    let y_idx = result.columns.iter().position(|c| c == "y").unwrap();
    assert!(
        (result.rows[0][x_idx] - 4.0).abs() < 1e-9,
        "{}",
        result.rows[0][x_idx]
    );
    assert!(
        (result.rows[0][y_idx] - 2.0).abs() < 1e-9,
        "{}",
        result.rows[0][y_idx]
    );
}

#[test]
fn singularity_reports_step_underflow() {
    // x' = -1/x reaches x = 0 at t = 0.5: a genuine singularity.
    let model = parse_model(
        "model S Real x(start = 1.0); equation der(x) = -1/x; \
         annotation(experiment(StopTime=1.0, Interval=0.01)); end S;",
    )
    .unwrap();
    let error = compile(&model).unwrap().simulate().unwrap_err();
    assert!(
        error.0.contains("step size underflow") || error.0.contains("budget"),
        "{}",
        error.0
    );
}

#[test]
fn truly_singular_system_is_still_rejected() {
    // Two equations for `a`, none for `b`; differentiation cannot
    // help because b never appears.
    let error = compile_err("model M Real a; Real b; equation a = 1; a = 2; end M;");
    assert!(
        error.contains("structurally singular") && error.contains("constrains no state"),
        "{error}"
    );
}

#[test]
fn tearing_shrinks_the_newton_system() {
    // A two-variable algebraic loop: one variable is torn, the other
    // follows from an explicit assignment.
    let model =
        parse_model("model L Real x; Real y; equation x = y / 2 + 1; y = x / 2 + 1; end L;")
            .unwrap();
    let compiled = compile(&model).unwrap();
    let plan = compiled.plan_summary();
    let block = plan
        .iter()
        .find(|line| line.contains("implicit block of 2"))
        .expect("a two-variable block");
    assert!(block.contains("iterating on 1"), "{block}");
    // And it still gets the right answer: x = y = 2.
    let result = compiled.simulate().unwrap();
    assert!((result.rows[0][1] - 2.0).abs() < 1e-9);
    assert!((result.rows[0][2] - 2.0).abs() < 1e-9);
}

#[test]
fn index_reduction_reaches_states_through_algebraic_definitions() {
    // `u = 3` names no state, but `u = 2*x` ties it to one: x is
    // pinned at 1.5 and its velocity has to vanish.
    let result = run("model N Real x(start = 1.0); Real v; Real u; \
         equation der(x) = v; u = 2 * x; u = 3; \
         annotation(experiment(StopTime=1.0, Interval=0.5)); end N;");
    let value = |name: &str| {
        let index = result.columns.iter().position(|c| c == name).unwrap();
        result.rows.last().unwrap()[index]
    };
    assert!((value("x") - 1.5).abs() < 1e-9, "x = {}", value("x"));
    assert!(value("v").abs() < 1e-9, "v = {}", value("v"));
    assert!((value("u") - 3.0).abs() < 1e-9, "u = {}", value("u"));
}

#[test]
fn index_reduction_differentiates_through_an_unsolvable_equation() {
    // The saturating inductor's shape. The current `si` is determined
    // by `Psi = 0.1*si + 1.9*Ipar*atan(si/Ipar)` and no rearrangement
    // gets it alone on a side, so a derivative through it can only come
    // from the implicit function theorem. The two components are joined
    // at both pins, which is what makes each current look defined by
    // the other and neither actually grounded.
    //
    // Written whole, voltages and ground included, because the half
    // without them passes either way: with only the currents there is
    // nothing to reduce, and a model simple enough to leave the
    // voltages out is simple enough never to reach the rule. The shape
    // the library uses is the test.
    //
    // The source sets the current to `2t`, so the voltage is
    // `dPsi/di * di/dt = (0.1 + 1.9/(1 + (i/Ipar)^2)) * 2`, and at
    // t = 1 with i = 2 and Ipar = 0.3 that is 0.283618581907... The
    // number is the point: flattening is not evidence, and a
    // derivative that is merely *taken* can be taken wrongly.
    let result = run("model Sat Real srcy, ri, rpi, rni, rpv, rnv, rv; \
         Real spi, sni, spv, snv, sv, si, Psi, gpi, gpv; \
         parameter Real Ipar = 0.3; \
         equation srcy = 2 * time; ri = srcy; \
         0 = rpi + rni; ri = rpi; rv = rpv - rnv; \
         0 = spi + sni; si = spi; sv = spv - snv; \
         Psi = 0.1 * si + 1.9 * Ipar * atan(si / Ipar); sv = der(Psi); \
         gpv = 0; rpv = gpv; snv = gpv; rpi + gpi + sni = 0; \
         rnv = spv; rni + spi = 0; \
         annotation(experiment(StopTime = 1.0, Interval = 0.5)); end Sat;");
    let value = |name: &str| {
        let index = result.columns.iter().position(|c| c == name).unwrap();
        result.rows.last().unwrap()[index]
    };
    assert!((value("si") - 2.0).abs() < 1e-9, "si = {}", value("si"));
    let expected = (0.1 + 1.9 / (1.0 + (2.0f64 / 0.3).powi(2))) * 2.0;
    assert!(
        (value("sv") - expected).abs() < 1e-9,
        "sv = {}, expected {expected}",
        value("sv")
    );
}

#[test]
fn every_form_of_loop_comes_out_at_the_right_numbers() {
    // A set, a stepped range, a range the body is left to work out, and
    // two indices at once - all four unrolled and run.
    let result = run("model M Real y[5]; Real a[3]; Real m[2,3]; Real total; \
         equation for i in {1, 3, 5} loop y[i] = i * 10; end for; \
         for i in {2, 4} loop y[i] = -1; end for; \
         for i loop a[i] = i * i; end for; \
         for i in 1:2, j in 1:3 loop m[i,j] = i * 10 + j; end for; \
         total = sum(a); \
         annotation(experiment(StopTime = 0, Interval = 1)); end M;");
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    for (name, expected) in [
        ("y[1]", 10.0),
        ("y[2]", -1.0),
        ("y[3]", 30.0),
        ("y[4]", -1.0),
        ("y[5]", 50.0),
        ("a[1]", 1.0),
        ("a[3]", 9.0),
        ("total", 14.0),
        ("m[1,1]", 11.0),
        ("m[1,3]", 13.0),
        ("m[2,1]", 21.0),
        ("m[2,3]", 23.0),
    ] {
        assert_eq!(last[index(name)], expected, "{name}");
    }
}

#[test]
fn a_solution_that_runs_away_is_reported_where_it_gave_up() {
    // Two ways for a run to come apart, and both must be named rather
    // than returned as numbers. `der(x) = -1/x` from x(0) = 1 reaches
    // x = 0 at t = 1/2 exactly, where the derivative is infinite;
    // `der(x) = -sqrt(x) - 1` reaches x = 0 at a t the square root
    // cannot be continued past, and the corrector is what notices.
    let singular = run_on(
        "model N Real x(start = 1, fixed = true); equation der(x) = -1 / x; \
         annotation(experiment(StopTime = 1, Interval = 0.01)); end N;",
        SolverMethod::Bdf,
    )
    .expect_err("cannot reach the stop time");
    assert!(
        singular.contains("step size underflow at t = 0.49") && singular.contains("singularity"),
        "{singular}"
    );

    let stuck = run_on(
        "model Q Real x(start = 1, fixed = true); equation der(x) = -sqrt(x) - 1; \
         annotation(experiment(StopTime = 2, Interval = 0.01)); end Q;",
        SolverMethod::Bdf,
    )
    .expect_err("cannot reach the stop time");
    assert!(
        stuck.contains("Newton iteration does not converge"),
        "{stuck}"
    );
}

#[test]
fn an_algebraic_loop_that_comes_apart_says_so() {
    let refused = |source: &str| {
        compile(&parse_model(source).unwrap())
            .expect_err("has no solution")
            .to_string()
    };

    // `1 / x = 0` has no solution, and at the start value of zero it
    // has no residual either. Nothing has diverged - Newton has not
    // taken a step, and cannot, because the equation it was handed
    // cannot be evaluated where the block begins. Saying "diverged"
    // sent the reader to the solver for a fault that is upstream of
    // it, so the refusal names the residual and its value instead.
    assert_eq!(
        refused(
            "model D Real x; Real s(start = 0, fixed = true); \
             equation 1 / x = 0; der(s) = x; \
             annotation(experiment(StopTime = 1, Interval = 0.1)); end D;"
        ),
        "residual 0 of algebraic loop [\"x\"] is inf at t = 0, before any \
         Newton step: the equations cannot be evaluated at the values the \
         block starts from"
    );

    // The other way a loop fails: `x^2 * y = 1` where y is sin(t),
    // which is exactly zero at the start. The residual does not move
    // when x does, so there is no direction to step in - and that is a
    // different complaint from walking off to infinity.
    assert_eq!(
        refused(
            "model S Real x; Real y; Real s(start = 0, fixed = true); \
             equation y = sin(time); x * x * y = 1; der(s) = x; \
             annotation(experiment(StopTime = 1, Interval = 0.1)); end S;"
        ),
        "singular Jacobian in algebraic loop [\"x\"]"
    );

    // A residual is built from the block's inner assignments, so a
    // residual that is NaN was usually handed one. Named by number
    // alone the refusal points at the solver, which is the one place
    // the fault is not; here `u = y/(y - x)` divides by nothing at the
    // start values and is what the reader has to be sent to.
    assert_eq!(
        refused(
            "model N Real x; Real u; Real y; Real s(start = 0, fixed = true); \
             equation u = y / (y - x); y = x; u * x = 1; der(s) = x; \
             annotation(experiment(StopTime = 1, Interval = 0.1)); end N;"
        ),
        "residual 0 of algebraic loop [\"x\"] is NaN at t = 0, before any \
         Newton step: the equations cannot be evaluated at the values the \
         block starts from; the block's own values are not numbers: \
         [\"u = NaN\"]"
    );
}

#[test]
fn a_zero_length_array_field_of_a_connector_writes_no_equation() {
    // A connector carries `Xi[nXi]`, and a single-substance medium has
    // `nXi = 0`, so a fluid port has no `Xi` to equate. The potential
    // equality would name `port.Xi` on both sides, which no component
    // is called, and the model was refused `unknown variable`. The fix
    // skips a connector member the flat model does not carry, so the
    // connection is the two scalars it really is.
    let result = run("package P \
           connector Port Real p; flow Real m; Real Xi[0]; end Port; \
           model Src Port port; Real s(start = 0, fixed = true); \
           equation port.p = 100; der(s) = port.m; end Src; \
           model Snk Port port; equation port.m = 1; end Snk; \
           model M Src src; Snk snk; \
           equation connect(src.port, snk.port); \
             annotation(experiment(StopTime = 1)); end M; \
         end P;");
    // The connection carried the pressure and the flow, and `Xi` wrote
    // nothing: the source integrates the flow it takes back.
    let s = result.columns.iter().position(|c| c == "src.s").unwrap();
    assert!(result.rows.len() > 1, "the model ran");
    assert!(
        result.rows[0][s].is_finite(),
        "src.s = {}",
        result.rows[0][s]
    );
}

#[test]
fn a_constraint_that_grows_past_the_ceiling_names_its_model() {
    // A differentiated constraint that copies itself from reduction
    // to reduction will eat a machine's memory, and a process killed
    // by the operating system says nothing about which model did it.
    // The ceiling turns that into a refusal naming the equation, the
    // reduction and the size reached. The threshold in force is far
    // above anything a healthy model reaches, so the test lowers it
    // rather than building a giant: what is under test is that the
    // guard fires and says something usable, not the number itself.
    //
    // The environment is shared by the tests of this binary, so the
    // variable is set and taken away around the one compile.
    let model = parse_model(
        "model N Real x(start = 1); Real v(start = 0); Real y; Real f; \
         equation der(x) = v; der(v) = f; y = x*x + 1; y = 2; end N;",
    )
    .unwrap();
    // The ceiling is lowered for this thread alone, because the
    // environment belongs to the whole test binary: lowering it there
    // lowered it for whatever test happened to be compiling beside
    // this one, and one of them refused a model it should have run.
    let error = {
        let _guard = lower_the_ceiling_here(1);
        compile(&model).unwrap_err()
    };
    assert!(error.0.contains("grew to"), "{}", error.0);
    assert!(error.0.contains("at reduction 1"), "{}", error.0);
    assert!(error.0.contains(r#"Ref("y") = Number(2.0)"#), "{}", error.0);
}
