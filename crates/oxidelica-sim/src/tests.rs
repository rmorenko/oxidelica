//! What the compiler and the solvers are expected to do.

use super::*;
use oxidelica_parser::parse_model;

fn run(source: &str) -> SimResult {
    let model = parse_model(source).unwrap();
    compile(&model).unwrap().simulate().unwrap()
}

#[test]
fn decay_matches_analytic() {
    let result = run("model D parameter Real a = 1.0; Real x(start = 1.0); \
         equation der(x) = -a*x; \
         annotation(experiment(StopTime=5.0, Interval=0.001, Tolerance=1e-12)); end D;");
    let last = result.rows.last().unwrap();
    let t = last[0];
    let x = last[1];
    assert!((t - 5.0).abs() < 1e-12);
    assert!(
        (x - (-5.0f64).exp()).abs() < 1e-9,
        "x(5)={x}, expected e^-5"
    );
}

#[test]
fn pendulum_conserves_energy() {
    let result = run("model P parameter Real g = 9.81; parameter Real L = 1.0; \
         Real phi(start = 0.7); Real w(start = 0.0); \
         equation der(phi) = w; der(w) = -(g/L)*sin(phi); \
         annotation(experiment(StopTime=10.0, Interval=0.001, Tolerance=1e-12)); end P;");
    let energy = |row: &Vec<f64>| {
        let (phi, w) = (row[1], row[2]);
        0.5 * w * w + 9.81 * (1.0 - phi.cos())
    };
    let e0 = energy(&result.rows[0]);
    let e_end = energy(result.rows.last().unwrap());
    assert!(
        ((e_end - e0) / e0).abs() < 1e-9,
        "energy drifted: {e0} -> {e_end}"
    );
}

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
fn reports_missing_equation() {
    let model = parse_model("model B Real x; Real y; equation x = 1; end B;").unwrap();
    let error = compile(&model).unwrap_err();
    assert!(error.0.contains("unbalanced"), "{}", error.0);
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
fn if_expression_saturates() {
    // Saturation: y = clamp(x, -1, 1); x grows linearly from 0 to 2.
    let result = run("model S Real x(start = 0.0); Real y; \
         equation der(x) = 1; y = if x > 1 then 1 elseif x < -1 then -1 else x; \
         annotation(experiment(StopTime=2.0, Interval=0.01)); end S;");
    let mid = &result.rows[result.rows.len() / 2]; // t=1: y == x == 1
    let last = result.rows.last().unwrap(); // t=2: x=2, y=1
    assert!((mid[2] - 1.0).abs() < 1e-6, "y(1)={}", mid[2]);
    assert!((last[1] - 2.0).abs() < 1e-9, "x(2)={}", last[1]);
    assert!((last[2] - 1.0).abs() < 1e-12, "y(2)={}", last[2]);
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

fn compile_err(source: &str) -> String {
    compile(&parse_model(source).unwrap())
        .unwrap_err()
        .to_string()
}

#[test]
fn evaluates_every_builtin_function() {
    let result = run("model F Real y; equation \
         y = sin(1) + cos(1) + tan(1) + asin(0.5) + acos(0.5) + atan(1) \
           + atan2(1, 2) + sinh(1) + cosh(1) + tanh(1) + exp(1) + log(2) \
           + log10(100) + sqrt(4) + abs(-3) + sign(-2) + min(1, 2) + max(1, 5) + 2 ^ 10; \
         annotation(experiment(StopTime=0.01, Interval=0.01)); end F;");
    let expected = 1f64.sin()
        + 1f64.cos()
        + 1f64.tan()
        + 0.5f64.asin()
        + 0.5f64.acos()
        + 1f64.atan()
        + 1f64.atan2(2.0)
        + 1f64.sinh()
        + 1f64.cosh()
        + 1f64.tanh()
        + 1f64.exp()
        + 2f64.ln()
        + 100f64.log10()
        + 2.0
        + 3.0
        + (-1.0)
        + 1.0
        + 5.0
        + 1024.0;
    assert!((result.rows[0][1] - expected).abs() < 1e-12);
}

#[test]
fn evaluates_booleans_and_relations() {
    let result = run("model B Real y; Real r; equation \
         y = if true and not false or false then 1 else 0; \
         r = (if 1 < 2 then 1 else 0) + (if 2 <= 2 then 1 else 0) \
           + (if 3 > 2 then 1 else 0) + (if 2 >= 3 then 0 else 1) \
           + (if 1 == 1 then 1 else 0) + (if 1 <> 2 then 1 else 0); \
         annotation(experiment(StopTime=0.01, Interval=0.01)); end B;");
    let names = &result.columns;
    let y_idx = names.iter().position(|n| n == "y").unwrap();
    let r_idx = names.iter().position(|n| n == "r").unwrap();
    assert_eq!(result.rows[0][y_idx], 1.0);
    assert_eq!(result.rows[0][r_idx], 6.0);
}

#[test]
fn compile_error_paths() {
    // Parameter without a value.
    assert!(
        compile_err("model M parameter Real p; Real x; equation x = 1; end M;")
            .contains("has no value")
    );
    // Parameter cycle.
    assert!(compile_err(
        "model M parameter Real a = b; parameter Real b = a; Real x; equation x = 1; end M;"
    )
    .contains("cycle"));
    // der of a parameter.
    assert!(
        compile_err("model M parameter Real p = 1; equation der(p) = 1; end M;")
            .contains("continuous")
    );
    // der on both sides.
    assert!(
        compile_err("model M Real x; Real y; equation der(x) = der(y); y = 1; end M;")
            .contains("must appear alone")
    );
    // Two equations for one state.
    assert!(
        compile_err("model M Real x; equation der(x) = 1; der(x) = 2; end M;")
            .contains("two equations")
    );
    // Two equations for one algebraic unknown: unbalanced.
    assert!(compile_err("model M Real y; equation y = 1; y = 2; end M;").contains("unbalanced"));
    // A state with an extra algebraic equation: unbalanced.
    assert!(
        compile_err("model M Real x; equation der(x) = 1; x = 2; end M;").contains("unbalanced")
    );

    // der inside an algebraic expression.
    assert!(
        compile_err("model M Real x; Real y; equation der(x) = 1; y = der(x) + 1; end M;")
            .contains("appear alone")
    );
    // Reference to an undeclared variable.
    assert!(
        compile_err("model M Real x; equation x = 1; q = 2; end M;").contains("unknown variable")
    );
    // Error in a start expression.
    assert!(
        compile_err("model M Real x(start = q); equation der(x) = 1; end M;")
            .contains("start of x")
    );
}

#[test]
fn expressions_are_checked_before_anything_runs() {
    // Names and functions are resolved while compiling, so none of
    // these can reach a solver: an unknown variable, an unknown
    // function, and a built-in given the wrong number of arguments.
    assert!(compile_err("model M Real y; equation y = z + 1; end M;").contains("unknown variable"));
    assert!(
        compile_err("model M Real y; equation y = frob(1); end M;").contains("unknown function")
    );
    assert!(compile_err("model M Real y; equation y = sin(1, 2); end M;").contains("argument"));
}

#[test]
fn csv_output_and_defaults() {
    let model = parse_model("model M Real x(start=1); equation der(x) = -x; end M;").unwrap();
    let compiled = compile(&model).unwrap();
    // Defaults apply without an annotation.
    assert_eq!(compiled.stop_time, 1.0);
    assert_eq!(compiled.step, 1e-3);
    let result = compiled.simulate().unwrap();
    let csv = result.to_csv();
    let mut lines = csv.lines();
    assert_eq!(lines.next().unwrap(), "time,x");
    // Values are written at full precision in the shortest text that
    // reads back as the same double, so a round number is round.
    assert_eq!(lines.next().unwrap(), "0,1");
    assert_eq!(csv.lines().count(), result.rows.len() + 1);
    // And a value that is not round keeps every digit it needs.
    let last = csv.lines().last().unwrap();
    let value: f64 = last.split(',').nth(1).unwrap().parse().unwrap();
    assert!((value - result.rows.last().unwrap()[1]).abs() == 0.0);
}

#[test]
fn parameter_uses_start_as_fallback_value() {
    let model =
        parse_model("model M parameter Real p(start = 3); Real x; equation x = p; end M;").unwrap();
    let compiled = compile(&model).unwrap();
    assert_eq!(compiled.parameters, vec![("p".to_string(), 3.0)]);
}

#[test]
fn mirrored_equation_forms_and_default_start() {
    // expr = der(v), expr = v, a state without start (zero-initialized),
    // and subtraction.
    let result = run("model M Real x; Real y; equation \
         -x - 1 = der(x); 2 + time = y; \
         annotation(experiment(StopTime=1.0, Interval=0.001, Tolerance=1e-12)); end M;");
    let first = &result.rows[0];
    assert_eq!(first[1], 0.0); // x(0) = 0 by default
    assert!((first[2] - 2.0).abs() < 1e-12); // y(0) = 2
                                             // der(x) = -x - 1, x(0)=0 -> x(t) = e^{-t} - 1
    let last = result.rows.last().unwrap();
    assert!(
        (last[1] - ((-1.0f64).exp() - 1.0)).abs() < 1e-9,
        "x(1)={}",
        last[1]
    );
}

#[test]
fn false_branches_of_relations_and_logic() {
    let result = run("model B Real r; Real y; equation \
         r = (if 2 < 1 then 1 else 0) + (if 3 <= 2 then 1 else 0) \
           + (if 2 > 3 then 1 else 0) + (if 3 >= 2 then 1 else 0) \
           + (if 1 == 2 then 1 else 0) + (if 1 <> 1 then 1 else 0); \
         y = (if false or false then 1 else 0) + (if false and true then 1 else 0) \
           + (if not true then 1 else 0) + (if false then 1 else 0); \
         annotation(experiment(StopTime=0.01, Interval=0.01)); end B;");
    let r_idx = result.columns.iter().position(|n| n == "r").unwrap();
    let y_idx = result.columns.iter().position(|n| n == "y").unwrap();
    assert_eq!(result.rows[0][r_idx], 1.0); // only >= holds
    assert_eq!(result.rows[0][y_idx], 0.0); // all false branches
}

#[test]
fn adaptive_respects_tolerance() {
    let source = |tol: &str| {
        format!(
            "model D Real x(start = 1.0); equation der(x) = -x; \
             annotation(experiment(StopTime=5.0, Interval=0.1, Tolerance={tol})); end D;"
        )
    };
    let error_at = |tol: &str| {
        let result = run(&source(tol));
        (result.rows.last().unwrap()[1] - (-5.0f64).exp()).abs()
    };
    let loose = error_at("1e-3");
    let tight = error_at("1e-10");
    assert!(tight < loose, "tight={tight}, loose={loose}");
    assert!(tight < 1e-8, "tight={tight}");
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
fn rk4_method_is_still_available() {
    let model = parse_model(
        "model D Real x(start = 1.0); equation der(x) = -x; \
         annotation(experiment(StopTime=1.0, Interval=0.001)); end D;",
    )
    .unwrap();
    let mut compiled = compile(&model).unwrap();
    compiled.method = SolverMethod::Rk4;
    let result = compiled.simulate().unwrap();
    let x = result.rows.last().unwrap()[1];
    assert!((x - (-1.0f64).exp()).abs() < 1e-9, "x(1)={x}");
}

#[test]
fn when_terminate_stops_simulation() {
    let result = run("model W Real x(start = 0.0); equation der(x) = 1; \
         when x > 0.5 then terminate(\"threshold reached\"); end when; \
         annotation(experiment(StopTime=2.0, Interval=0.01)); end W;");
    let message = result.terminated.expect("must terminate");
    assert!(message.contains("threshold reached"), "{message}");
    let last_t = result.rows.last().unwrap()[0];
    assert!(
        (0.5..=0.55).contains(&last_t),
        "stopped at t = {last_t}, expected just past 0.5"
    );
}

#[test]
fn when_terminate_can_fire_at_start() {
    let result = run("model W Real x(start = 5.0); equation der(x) = 1; \
         when x > 1 then terminate(\"already past\"); end when; \
         annotation(experiment(StopTime=1.0, Interval=0.01)); end W;");
    assert!(result.terminated.is_some());
    assert_eq!(result.rows.len(), 1); // only the initial point
}

#[test]
fn normal_runs_do_not_terminate() {
    let result = run("model N Real x(start = 0.0); equation der(x) = 1; \
         when x > 100 then terminate(\"never\"); end when; \
         annotation(experiment(StopTime=1.0, Interval=0.1)); end N;");
    assert!(result.terminated.is_none());
    assert!((result.rows.last().unwrap()[0] - 1.0).abs() < 1e-9);
}

#[test]
fn rc_circuit_from_components_matches_analytic() {
    // The full M2 pipeline: connectors, extends, connect, flattening,
    // matching. c.v(t) = V * (1 - e^(-t / (R*C))).
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/rc_circuit.mo"),
    )
    .unwrap();
    let result = compile(&parse_model(&source).unwrap())
        .unwrap()
        .simulate()
        .unwrap();
    let cv = result.columns.iter().position(|c| c == "c.v").unwrap();
    let last = result.rows.last().unwrap();
    let analytic = 1.0 - (-last[0] / (100.0 * 0.001)).exp();
    assert!(
        (last[cv] - analytic).abs() < 1e-9,
        "c.v = {}, analytic = {analytic}",
        last[cv]
    );
}

#[test]
fn index2_dae_tracks_the_constraint() {
    // q = time^2 with der(q) = z: the constraint must be
    // differentiated once (Pantelides) to expose z; z = 2t follows.
    let result = run(
        "model D Real z; Real q(start = 0); equation der(q) = z; q = time ^ 2; \
         annotation(experiment(StopTime=1.0, Interval=0.01, Tolerance=1e-10)); end D;",
    );
    let last = result.rows.last().unwrap();
    let q_idx = result.columns.iter().position(|c| c == "q").unwrap();
    let z_idx = result.columns.iter().position(|c| c == "z").unwrap();
    assert!((last[q_idx] - 1.0).abs() < 1e-6, "q(1) = {}", last[q_idx]);
    assert!((last[z_idx] - 2.0).abs() < 1e-4, "z(1) = {}", last[z_idx]);
}

#[test]
fn index3_cartesian_pendulum_matches_angle_form() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/cartesian_pendulum.mo"),
    )
    .unwrap();
    let cart = compile(&parse_model(&source).unwrap())
        .unwrap()
        .simulate()
        .unwrap();
    let x_idx = cart.columns.iter().position(|c| c == "x").unwrap();
    let y_idx = cart.columns.iter().position(|c| c == "y").unwrap();
    // The length constraint holds throughout.
    let worst = cart
        .rows
        .iter()
        .map(|r| (r[x_idx] * r[x_idx] + r[y_idx] * r[y_idx] - 1.0).abs())
        .fold(0.0f64, f64::max);
    assert!(worst < 1e-6, "constraint violation {worst}");
    // Cross-check against the angle formulation of the same pendulum.
    let angle = run("model P parameter Real g = 9.81; parameter Real L = 1.0; \
         Real phi(start = 0.7); Real w(start = 0.0); Real x; Real y; \
         equation der(phi) = w; der(w) = -(g/L)*sin(phi); \
         x = L * sin(phi); y = -L * cos(phi); \
         annotation(experiment(StopTime=10.0, Interval=0.001, Tolerance=1e-10)); end P;");
    let ax = angle.columns.iter().position(|c| c == "x").unwrap();
    let ay = angle.columns.iter().position(|c| c == "y").unwrap();
    let (cl, al) = (cart.rows.last().unwrap(), angle.rows.last().unwrap());
    assert!(
        (cl[x_idx] - al[ax]).abs() < 1e-4 && (cl[y_idx] - al[ay]).abs() < 1e-4,
        "cartesian ({}, {}) vs angle ({}, {})",
        cl[x_idx],
        cl[y_idx],
        al[ax],
        al[ay]
    );
}

#[test]
fn a_plain_start_is_a_guess_but_fixed_is_an_initial_condition() {
    // q is demoted by index reduction and solved from q = time^2,
    // so a plain start is only a Newton guess: q(0) = 0 wins.
    let result = run(
        "model D Real z; Real q(start = 1); equation der(q) = z; q = time ^ 2; \
         annotation(experiment(StopTime=1.0, Interval=0.01)); end D;",
    );
    let q_idx = result.columns.iter().position(|c| c == "q").unwrap();
    assert!(
        result.rows[0][q_idx].abs() < 1e-9,
        "q(0) = {}",
        result.rows[0][q_idx]
    );
    // Declaring it fixed turns the contradiction into an error.
    let model = parse_model(
        "model D Real z; Real q(start = 1, fixed = true); \
         equation der(q) = z; q = time ^ 2; \
         annotation(experiment(StopTime=1.0, Interval=0.01)); end D;",
    )
    .unwrap();
    let error = compile(&model).unwrap_err();
    assert!(error.0.contains("is fixed at"), "{}", error.0);
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
fn bdf_handles_a_stiff_system_that_starves_explicit_methods() {
    // der(x) = -1e6 * (x - cos t): the explicit method is limited by
    // stability, the implicit one by accuracy only.
    let source = "model S Real x(start = 0.0); \
         equation der(x) = -1000000.0 * (x - cos(time)); \
         annotation(experiment(StopTime=5.0, Interval=0.01, Tolerance=1e-6)); end S;";
    let mut compiled = compile(&parse_model(source).unwrap()).unwrap();
    compiled.method = SolverMethod::Bdf;
    let result = compiled.simulate().unwrap();
    let x = result.rows.last().unwrap()[1];
    // After the transient the solution tracks the quasi-steady cos t.
    assert!(
        (x - 5.0f64.cos()).abs() < 1e-5,
        "x(5) = {x}, expected ~{}",
        5.0f64.cos()
    );
}

#[test]
fn bdf_and_dopri_agree_on_a_non_stiff_model() {
    let source = "model P parameter Real g = 9.81; Real phi(start = 0.7); Real w(start = 0); \
         equation der(phi) = w; der(w) = -g * sin(phi); \
         annotation(experiment(StopTime=2.0, Interval=0.01, Tolerance=1e-10)); end P;";
    let model = parse_model(source).unwrap();
    let dopri = compile(&model).unwrap().simulate().unwrap();
    let mut stiff_solver = compile(&model).unwrap();
    stiff_solver.method = SolverMethod::Bdf;
    let bdf = stiff_solver.simulate().unwrap();
    let (a, b) = (dopri.rows.last().unwrap(), bdf.rows.last().unwrap());
    assert!((a[1] - b[1]).abs() < 1e-6, "phi: {} vs {}", a[1], b[1]);
    assert!((a[2] - b[2]).abs() < 1e-6, "w: {} vs {}", a[2], b[2]);
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
fn solver_names_round_trip() {
    for method in [SolverMethod::Dopri45, SolverMethod::Rk4, SolverMethod::Bdf] {
        assert_eq!(SolverMethod::from_name(method.name()), Some(method));
    }
    assert_eq!(SolverMethod::from_name("nope"), None);
}

fn expr_of(source_expr: &str) -> Expr {
    let model = parse_model(&format!(
        "model E Real a; Real b; Real q; equation q = {source_expr}; a = 1; b = 2; end E;"
    ))
    .unwrap();
    model
        .equations
        .iter()
        .find_map(|e| match (&e.lhs, &e.rhs) {
            (Expr::Ref(n), rhs) if n == "q" => Some(rhs.clone()),
            _ => None,
        })
        .unwrap()
}

/// Evaluate an expression with the given variable bindings.
fn value_of(expr: &Expr, bindings: &[(&str, f64)]) -> f64 {
    let vars: HashMap<String, f64> = bindings
        .iter()
        .map(|(n, v)| ((*n).to_string(), *v))
        .collect();
    eval(
        expr,
        &EvalCtx {
            vars: &vars,
            time: 0.0,
        },
    )
    .unwrap()
}

#[test]
fn simplify_folds_constants_and_identities() {
    let cases = [
        ("2 * 3 + 1", 7.0),
        ("a * 0", 0.0),
        ("0 * a", 0.0),
        ("a * 1", 1.0),
        ("1 * a", 1.0),
        ("a + 0", 1.0),
        ("0 + a", 1.0),
        ("a - 0", 1.0),
        ("0 - a", -1.0),
        ("a / 1", 1.0),
        ("0 / a", 0.0),
        ("a ^ 1", 1.0),
        ("a ^ 0", 1.0),
        ("-(2)", -2.0),
    ];
    for (source, expected) in cases {
        let folded = simplify(&expr_of(source));
        assert_eq!(
            value_of(&folded, &[("a", 1.0), ("b", 2.0)]),
            expected,
            "{source} folded to {folded:?}"
        );
    }
    // Structure-preserving branches still simplify their children.
    let nested = simplify(&expr_of(
        "if a > 0 and b > 0 or not a > 0 then a * 1 else b + 0",
    ));
    assert_eq!(value_of(&nested, &[("a", 1.0), ("b", 2.0)]), 1.0);
    assert_eq!(
        value_of(&simplify(&expr_of("sin(a * 1)")), &[("a", 0.0)]),
        0.0
    );
}

#[test]
fn substitute_replaces_every_occurrence() {
    let expr = expr_of("if a > 0 and a < 5 or not a > 9 then sin(a) + (-a) else a / 2 ^ a");
    let substituted = substitute(&expr, "a", 0.0);
    let mut refs = Vec::new();
    substituted.collect_refs(&mut refs);
    assert!(!refs.contains(&"a"), "a survived: {substituted:?}");
    assert_eq!(value_of(&substituted, &[]), 0.0);
}

#[test]
fn differentiates_every_elementary_function() {
    // d/da of f(a) at a = 0.7, compared with a central difference.
    for name in [
        "sin", "cos", "tan", "exp", "log", "sqrt", "atan", "sinh", "cosh", "tanh",
    ] {
        let expr = expr_of(&format!("{name}(a)"));
        let derivative = simplify(&differentiate(&expr, &DiffTarget::Variable("a")).unwrap());
        let (point, step) = (0.7f64, 1e-6);
        let numeric = (value_of(&expr_of(&format!("{name}(a)")), &[("a", point + step)])
            - value_of(&expr_of(&format!("{name}(a)")), &[("a", point - step)]))
            / (2.0 * step);
        let symbolic = value_of(&derivative, &[("a", point)]);
        assert!(
            (symbolic - numeric).abs() < 1e-5,
            "{name}: symbolic {symbolic} vs numeric {numeric}"
        );
    }
    // Products, quotients, powers and if-expressions.
    let d = |source: &str| {
        simplify(&differentiate(&expr_of(source), &DiffTarget::Variable("a")).unwrap())
    };
    assert_eq!(value_of(&d("a * b"), &[("a", 3.0), ("b", 2.0)]), 2.0);
    assert_eq!(value_of(&d("a / b"), &[("a", 3.0), ("b", 2.0)]), 0.5);
    assert_eq!(value_of(&d("a ^ 3"), &[("a", 2.0)]), 12.0);
    assert_eq!(value_of(&d("-a"), &[("a", 2.0)]), -1.0);
    assert_eq!(
        value_of(&d("if b > 0 then a * 2 else a"), &[("a", 1.0), ("b", 1.0)]),
        2.0
    );
    // Refusals: unknown function, non-constant exponent, time target.
    assert!(differentiate(&expr_of("atan2(a, b)"), &DiffTarget::Variable("a")).is_err());
    assert!(differentiate(&expr_of("a ^ b"), &DiffTarget::Variable("a")).is_err());
    assert_eq!(
        value_of(
            &differentiate(&expr_of("time"), &DiffTarget::Variable("a")).unwrap(),
            &[]
        ),
        0.0
    );
}

#[test]
fn nonlinear_equations_are_not_solved_symbolically() {
    // x * x = 4 is not linear in x, so no closed form is offered.
    let expr = expr_of("a * a");
    assert!(solve_linear_for(&expr, &Expr::Number(4.0), "a").is_none());
    // ... but 3 * x - 6 = 0 is.
    let linear = expr_of("3 * a - 6");
    let solution = solve_linear_for(&linear, &Expr::Number(0.0), "a").unwrap();
    assert!((value_of(&solution, &[]) - 2.0).abs() < 1e-12);
}

#[test]
fn bdf_covers_termination_and_algebraic_only_models() {
    // Termination inside the BDF loop.
    let mut compiled = compile(
        &parse_model(
            "model W Real x(start = 0.0); equation der(x) = 1; \
             when x > 0.5 then terminate(\"done\"); end when; \
             annotation(experiment(StopTime=2.0, Interval=0.01)); end W;",
        )
        .unwrap(),
    )
    .unwrap();
    compiled.method = SolverMethod::Bdf;
    assert!(compiled.simulate().unwrap().terminated.is_some());

    // A model without states: the solver just walks the grid.
    let mut algebraic_only = compile(
        &parse_model(
            "model A Real y; equation y = 2 * time; \
             annotation(experiment(StopTime=1.0, Interval=0.25)); end A;",
        )
        .unwrap(),
    )
    .unwrap();
    algebraic_only.method = SolverMethod::Bdf;
    let result = algebraic_only.simulate().unwrap();
    assert_eq!(result.rows.len(), 5);
    assert!((result.rows.last().unwrap()[1] - 2.0).abs() < 1e-12);

    // Terminating at t = 0 short-circuits before any stepping.
    let mut immediate = compile(
        &parse_model(
            "model I Real x(start = 5.0); equation der(x) = 1; \
             when x > 1 then terminate(\"already\"); end when; end I;",
        )
        .unwrap(),
    )
    .unwrap();
    immediate.method = SolverMethod::Bdf;
    let result = immediate.simulate().unwrap();
    assert!(result.terminated.is_some());
    assert_eq!(result.rows.len(), 1);
}

#[test]
fn bdf_reports_a_singularity_instead_of_guessing() {
    // x' = -1/x runs into x = 0 at t = 0.5.
    let mut compiled = compile(
        &parse_model(
            "model S Real x(start = 1.0); equation der(x) = -1/x; \
             annotation(experiment(StopTime=1.0, Interval=0.01)); end S;",
        )
        .unwrap(),
    )
    .unwrap();
    compiled.method = SolverMethod::Bdf;
    let error = compiled.simulate().unwrap_err();
    assert!(
        error.0.contains("underflow") || error.0.contains("budget"),
        "{}",
        error.0
    );
}

#[test]
fn plan_summary_describes_stages() {
    let compiled =
        compile(&parse_model("model P Real x; Real y; equation x = 2; y = x + 1; end P;").unwrap())
            .unwrap();
    let plan = compiled.plan_summary();
    assert_eq!(plan.len(), 2);
    assert!(plan.iter().all(|line| line.starts_with("explicit")));
}

#[test]
fn dummy_derivatives_demote_a_state_and_keep_the_constraint_exact() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/cartesian_pendulum.mo"),
    )
    .unwrap();
    let compiled = compile(&parse_model(&source).unwrap()).unwrap();
    // Index reduction demoted one position and one velocity: four
    // states became two, and their derivatives became dummies.
    assert_eq!(compiled.states, vec!["x", "vx"]);
    assert!(compiled.algebraics.contains(&"der(y)".to_string()));
    assert!(compiled.algebraics.contains(&"der(vy)".to_string()));

    // The constraint is solved, not stabilized: its residual stays
    // at solver tolerance instead of drifting with time.
    let result = compiled.simulate().unwrap();
    let x = result.columns.iter().position(|c| c == "x").unwrap();
    let y = result.columns.iter().position(|c| c == "y").unwrap();
    let violation = |row: &Vec<f64>| (row[x] * row[x] + row[y] * row[y] - 1.0).abs();
    let early = result.rows[result.rows.len() / 10..result.rows.len() / 5]
        .iter()
        .map(violation)
        .fold(0.0f64, f64::max);
    let late = result.rows[result.rows.len() * 4 / 5..]
        .iter()
        .map(violation)
        .fold(0.0f64, f64::max);
    assert!(late < 1e-8, "late violation {late}");
    assert!(late < 100.0 * early.max(1e-12), "drift: {early} -> {late}");
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

fn example(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name),
    )
    .unwrap()
}

#[test]
fn bouncing_ball_reinits_at_every_impact() {
    let result = compile(&parse_model(&example("bouncing_ball.mo")).unwrap())
        .unwrap()
        .simulate()
        .unwrap();
    let h = result.columns.iter().position(|c| c == "h").unwrap();
    let v = result.columns.iter().position(|c| c == "v").unwrap();

    // The floor is never breached beyond event-location tolerance.
    let deepest = result
        .rows
        .iter()
        .map(|row| row[h])
        .fold(f64::INFINITY, f64::min);
    assert!(deepest > -1e-6, "ball fell through the floor: {deepest}");

    // First impact: free fall from 1 m, rebound at 0.8 of the
    // impact speed.
    let first = result
        .rows
        .windows(2)
        .find(|w| w[0][v] < 0.0 && w[1][v] > 0.0)
        .expect("at least one bounce");
    let expected_t = (2.0f64 / 9.81).sqrt();
    let expected_v = 0.8 * (2.0 * 9.81f64).sqrt();
    assert!(
        (first[1][0] - expected_t).abs() < 1e-4,
        "t = {}",
        first[1][0]
    );
    assert!(
        (first[1][v] - expected_v).abs() < 1e-3,
        "v = {}",
        first[1][v]
    );

    // Impacts crowd toward the Zeno limit, where terminate fires.
    let message = result.terminated.expect("terminates at rest");
    assert!(message.contains("come to rest"), "{message}");
}

#[test]
fn ideal_diode_never_conducts_while_blocking() {
    let result = compile(&parse_model(&example("rectifier.mo")).unwrap())
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (vs, vc, id) = (index("vs"), index("vc"), index("id"));
    for row in &result.rows {
        if row[vs] - row[vc] < -1e-9 {
            assert!(row[id].abs() < 1e-12, "blocking diode carried {}", row[id]);
        }
    }
    // The load charges toward the source amplitude.
    let peak = result.rows.iter().map(|r| r[vc]).fold(0.0f64, f64::max);
    assert!((0.8..1.0).contains(&peak), "load peaked at {peak}");
}

#[test]
fn events_are_located_rather_than_stepped_over() {
    // A coarse output grid must not blunt the event: the impact is
    // found to solver tolerance even between grid points.
    let result = run(
        "model B parameter Real g = 9.81; Real h(start = 1.0); Real v(start = 0.0); \
         equation der(h) = v; der(v) = -g; \
         when h < 0 then reinit(v, -v); end when; \
         annotation(experiment(StopTime=1.0, Interval=0.25, Tolerance=1e-9)); end B;",
    );
    let v = result.columns.iter().position(|c| c == "v").unwrap();
    let h = result.columns.iter().position(|c| c == "h").unwrap();
    // Perfectly elastic: after the bounce the ball returns to 1 m.
    let peak_after = result
        .rows
        .iter()
        .filter(|row| row[0] > 0.46)
        .map(|row| row[h])
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(peak_after > 0.9, "rebound reached only {peak_after}");
    assert!(result.rows.iter().any(|row| row[v] > 0.0), "never bounced");
}

#[test]
fn reinit_targets_must_be_states() {
    let model = parse_model(
        "model R Real x(start = 1.0); Real y; equation der(x) = -2; y = 2 * x; \
         when x < 0 then reinit(y, 0); end when; \
         annotation(experiment(StopTime=1.0, Interval=0.01)); end R;",
    )
    .unwrap();
    // Caught while compiling: the target of a reinit is resolved to a
    // place in the state vector, so a name that is not one cannot
    // wait until the run to be noticed.
    let error = compile(&model).unwrap_err();
    assert!(error.0.contains("is not a state"), "{}", error.0);
}

#[test]
fn when_clauses_fire_on_the_rising_edge_only() {
    // A single terminate that stays true must not fire twice, and a
    // condition true at t = 0 fires immediately.
    let result = run("model E Real x(start = 0.0); equation der(x) = 1; \
         when x > 0.25 then terminate(\"crossed\"); end when; \
         annotation(experiment(StopTime=1.0, Interval=0.1)); end E;");
    let message = result.terminated.unwrap();
    assert!(message.contains("crossed"), "{message}");
    let last_t = result.rows.last().unwrap()[0];
    assert!((last_t - 0.25).abs() < 1e-6, "stopped at {last_t}");
}

#[test]
fn discretized_heat_conduction_reaches_the_analytic_steady_state() {
    // The M5 pipeline end to end: an array of 40 nodes, a for
    // equation over the interior, and an inlined function.
    let compiled = compile(&parse_model(&example("heat_conduction.mo")).unwrap()).unwrap();
    assert_eq!(compiled.states.len(), 40, "one state per node");
    assert!(compiled.states.contains(&"T[20]".to_string()));

    let result = compiled.simulate().unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();

    // Held ends, so the rod relaxes to a straight line between them.
    for node in 1..=40 {
        let position = node as f64 / 41.0;
        let expected = 100.0 + (20.0 - 100.0) * position;
        let actual = last[index(&format!("T[{node}]"))];
        assert!(
            (actual - expected).abs() < 1e-2,
            "node {node}: {actual} vs {expected}"
        );
    }
    // The inlined steadyState() function measures the same thing.
    assert!(last[index("midError")].abs() < 1e-2);
    // Cold start: the rod begins uniform.
    assert!((result.rows[0][index("T[1]")] - 20.0).abs() < 1e-12);
}

fn with_library(name: &str) -> oxidelica_parser::Model {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let library = std::fs::read_to_string(root.join("lib/Oxidelica.mo")).unwrap();
    let source = std::fs::read_to_string(root.join("examples").join(name)).unwrap();
    oxidelica_parser::parse_model_with_libraries(&[library], &source).unwrap()
}

/// Compile a model and give back what it refused to do.
fn refused(source: &str) -> String {
    let model = parse_model(source).expect("parses");
    match compile(&model) {
        Ok(_) => panic!("should have been refused"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn each_solver_may_be_asked_for_by_itself() {
    // The single-method entry points, on a model with an event in
    // it so the machinery around the step is exercised too.
    let source = "model M Real x(start = 1); discrete Real hits(start = 0); equation der(x) = -x; when x < 0.5 then hits = pre(hits) + 1; reinit(x, 1); end when; annotation(experiment(StopTime = 3, Interval = 0.05, Tolerance = 1e-8)); end M;";
    let model = parse_model(source).unwrap();
    for asked in ["adaptive", "bdf", "rk4"] {
        let compiled = compile(&model).unwrap();
        let result = match asked {
            "adaptive" => compiled.simulate_adaptive().unwrap(),
            "bdf" => compiled.simulate_bdf().unwrap(),
            _ => compiled.simulate_rk4().unwrap(),
        };
        let hits = result.columns.iter().position(|c| c == "hits").unwrap();
        // It falls to a half three times over three seconds.
        let last = result.rows.last().unwrap()[hits];
        assert!(last >= 3.0, "{asked}: only {last} hits");
    }
}

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
fn a_constraint_of_every_shape_is_differentiated() {
    // Index reduction differentiates the constraint, and the
    // walker that puts the derivatives back has every shape to
    // step through on the way.
    let source = "model M parameter Real g = 9.81; Real x(start = 1); Real y(start = 0); Real vx(start = 0); Real vy(start = 0); Real lam; equation der(x) = vx; der(y) = vy; der(vx) = lam * x; der(vy) = lam * y - g; sqrt(x * x + y * y) * (if true then 1 else 2) * (if x > -2 and not (y > 9) or false then 1 else 0) = 1; annotation(experiment(StopTime = 1, Interval = 0.01, Tolerance = 1e-10)); end M;";
    let model = parse_model(source).unwrap();
    let result = compile(&model).unwrap().simulate().unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    // Whatever it was written as, it is still a pendulum on a
    // string of length one.
    for row in &result.rows {
        let (x, y) = (row[index("x")], row[index("y")]);
        assert!(
            (x.hypot(y) - 1.0).abs() < 1e-6,
            "t = {}: length {}",
            row[0],
            x.hypot(y)
        );
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
    assert!(refused(
        "model M Real x(start = 0); Real y; equation der(x) = 1; y = 2 * der(x); end M;"
    )
    .contains("der() must appear alone on one side"));
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

#[test]
fn the_initialisation_problem_is_solved_or_named() {
    // An initial equation that pins nothing.
    assert!(refused(
        "model M Real x(start = 0); equation der(x) = 1; initial equation 0 = 0; end M;"
    )
    .contains("initialization problem is singular"));
    // One that cannot be solved from where it starts.
    let model = parse_model(
        "model M Real x(start = 1); equation der(x) = 1; initial equation x * x = -1; end M;",
    )
    .unwrap();
    let outcome = compile(&model);
    assert!(outcome.is_err(), "an impossible start should be refused");
}

#[test]
fn a_run_reports_what_went_wrong_in_it() {
    // A model with no points at all to stall from.
    let model = parse_model("model M Real x(start = 0); equation der(x) = 1; annotation(experiment(StopTime = 1, Interval = 0.5)); end M;").unwrap();
    let compiled = compile(&model).unwrap();
    // Every method reaches the same answer on a straight line.
    for method in [
        SolverMethod::Auto,
        SolverMethod::Dopri45,
        SolverMethod::Rk4,
        SolverMethod::Bdf,
    ] {
        let mut compiled = compile(&model).unwrap();
        compiled.method = method;
        let result = compiled.simulate().unwrap();
        let last = result.rows.last().unwrap();
        assert!((last[1] - 1.0).abs() < 1e-6, "{method:?}: {}", last[1]);
        assert_eq!(SolverMethod::from_name(method.name()), Some(method));
    }
    assert_eq!(SolverMethod::from_name("nonsense"), None);
    assert_eq!(compiled.states, vec!["x"]);
}

#[test]
fn a_delayed_wave_arrives_unchanged_but_later() {
    // What comes out of the pipe is what went in, a transit time
    // ago and a little smaller. The shape is exact; the shift is
    // as exact as the output grid, which is what a straight line
    // between two remembered points can manage.
    let result = compile(&with_library("transport_delay.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (transit, loss) = (0.53f64, 0.15f64);
    for row in &result.rows {
        let (t, seen) = (row[0], row[index("outlet")]);
        let wanted = if t >= transit {
            (1.0 - loss) * (3.0 * (t - transit)).sin()
        } else {
            0.0
        };
        assert!(
            (seen - wanted).abs() < 1e-5,
            "t = {t}: outlet {seen} vs {wanted}"
        );
    }
    // Before the fluid has crossed, the far end holds what the
    // inlet started at.
    assert_eq!(result.rows[0][index("outlet")], 0.0);
    // And the vessel it pours into really did fill.
    assert!(result
        .rows
        .iter()
        .any(|row| row[index("vessel")].abs() > 0.2));
}

#[test]
fn a_state_machine_holds_up_a_queue_of_cars() {
    // The states are blocks, the arrows are declared, and the
    // machine ticks once a second. Underneath it the queue knows
    // nothing about any of that: it grows at a steady rate and
    // drains only while the light is green, so it is a sawtooth
    // whose corners fall where the machine says they do.
    let result = compile(&with_library("traffic_light.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();

    // One colour per second: red for four, green for five, amber
    // for two, over and over.
    let mut colours = String::new();
    for second in 0..=22 {
        let at = second as f64;
        let row = result
            .rows
            .iter()
            .rev()
            .find(|row| (row[0] - at).abs() < 1e-9)
            .unwrap_or_else(|| panic!("no row at t = {at}"));
        colours.push(match row[index("lamp")] as i64 {
            0 => 'r',
            1 => 'g',
            _ => 'a',
        });
    }
    assert_eq!(colours, "rrrrgggggaarrrrgggggaar");

    // And the queue is exactly the sawtooth that follows from it.
    let (arrivals, departures) = (2.0f64, 5.0f64);
    let (mut queue, mut at, mut green) = (0.0f64, 0.0f64, false);
    for row in &result.rows {
        let step = row[0] - at;
        queue += (arrivals - if green { departures } else { 0.0 }) * step;
        assert!(
            (row[index("queue")] - queue).abs() < 1e-9,
            "t = {}: queue {} vs {queue}",
            row[0],
            row[index("queue")]
        );
        green = row[index("lamp")] == 1.0;
        at = row[0];
    }
    // Four seconds of red and two of amber at two cars a second.
    assert!(
        (result
            .rows
            .iter()
            .map(|row| row[index("queue")])
            .fold(0.0, f64::max)
            - 8.0)
            .abs()
            < 1e-9
    );
}

#[test]
fn a_clocked_controller_follows_its_own_recurrence() {
    // The clock is declared and the equations that belong to it are
    // found rather than marked. What comes out is a sampled-data
    // loop whose every tick can be written by hand: the control is
    // constant between ticks, so the plant relaxes towards it, and
    // the integral advances by one period at a time.
    let result = compile(&with_library("clocked_control.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (period, plant, kp, ki, setpoint) = (0.05f64, 0.4f64, 1.6f64, 4.0f64, 1.0f64);
    let decay = (-period / plant).exp();

    let (mut state, mut integral) = (0.0f64, 0.0f64);
    let mut tick = 0;
    while tick as f64 * period <= 3.0 + 1e-12 {
        let at = tick as f64 * period;
        let error = setpoint - state;
        integral += error * period;
        let command = kp * error + ki * integral;
        // The row a tick leaves behind is the one after the event.
        let row = result
            .rows
            .iter()
            .rev()
            .find(|row| (row[0] - at).abs() < 1e-9)
            .unwrap_or_else(|| panic!("no row at t = {at}"));
        assert!(
            (row[index("u")] - command).abs() < 1e-9,
            "tick {tick}: u = {} vs {command}",
            row[index("u")]
        );
        assert!((row[index("x")] - state).abs() < 1e-9, "tick {tick}");
        state = command + (state - command) * decay;
        tick += 1;
    }
    assert_eq!(tick, 61);
    // And it did settle on the setpoint.
    assert!((result.rows.last().unwrap()[index("x")] - setpoint).abs() < 1e-3);
}

#[test]
fn a_phasor_written_in_complex_arithmetic_predicts_the_circuit() {
    // The impedance is written `R + j * X` and worked out by the
    // record's own operators; the circuit is then integrated from
    // rest with none of that in sight. After the transient has
    // died the two must agree, in amplitude and in phase.
    let result = compile(&with_library("complex_impedance.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let first = &result.rows[0];
    let (amplitude, phase) = (
        first[index("predicted_amplitude")],
        first[index("predicted_phase")],
    );

    // The phasor is exact: complex division of 10 by 2 - 0.5j.
    let (r, l, c, w, v) = (2.0f64, 0.5f64, 0.1f64, 4.0f64, 10.0f64);
    let reactance = w * l - 1.0 / (w * c);
    assert!((amplitude - v / r.hypot(reactance)).abs() < 1e-12);
    assert!((phase - (-reactance).atan2(r)).abs() < 1e-12);

    // And the circuit settles onto exactly that sine.
    for row in result.rows.iter().filter(|row| row[0] >= 10.0) {
        let wanted = amplitude * (w * row[0] + phase).sin();
        assert!(
            (row[index("i")] - wanted).abs() < 1e-6,
            "t = {}: i = {} vs {wanted}",
            row[0],
            row[index("i")]
        );
    }
}

#[test]
fn two_chains_of_different_length_share_their_functions() {
    // One `Chain` component, instantiated at three masses and at
    // five: the length is a parameter and the masses and starts
    // are handed over as whole arrays. `total` and `weighted` are
    // declared once with `[:]` inputs and measure each. Neither
    // chain is pushed from outside, so the physics has two exact
    // statements to make: momentum is constant, and the centre of
    // mass travels in a straight line.
    let result = compile(&with_library("mass_chains.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    for (chain, mass) in [("short", 6.0f64), ("long", 6.0f64)] {
        let (momentum, centre) = (
            index(&format!("{chain}.momentum")),
            index(&format!("{chain}.centre")),
        );
        let first = &result.rows[0];
        let speed = first[momentum] / mass;
        for row in &result.rows {
            assert!(
                (row[momentum] - first[momentum]).abs() < 1e-12,
                "chain {chain} at t = {}: momentum {} vs {}",
                row[0],
                row[momentum],
                first[momentum]
            );
            let wanted = first[centre] + speed * row[0];
            assert!(
                (row[centre] - wanted).abs() < 1e-12,
                "chain {chain} at t = {}: centre {} vs {wanted}",
                row[0],
                row[centre]
            );
        }
    }
    // The two chains really did start out differently.
    assert!(
        (result.rows[0][index("short.momentum")] + 0.5).abs() < 1e-12
            && result.rows[0][index("long.momentum")].abs() < 1e-12
    );
    // Each instance really did get its own length and its own
    // masses, handed over as whole arrays.
    assert!(result.columns.iter().any(|c| c == "long.x[5]"));
    assert!(!result.columns.iter().any(|c| c == "short.x[4]"));
}

#[test]
fn the_textbook_ideal_switch_rectifies_exactly() {
    // The switch's branches constrain different unknowns: blocking
    // is an equation on the current, conducting one on the voltage.
    // Each mode is compiled as its own model - matched and torn for
    // the equations actually in force - and compiled again at the
    // instant the switch flips. Nothing here is approximate: the
    // current is the clipped source to the last bit.
    let result = compile(&with_library("ideal_rectifier.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (mut blocking_rows, mut conducting_rows) = (0, 0);
    for row in &result.rows {
        assert_eq!(
            row[index("switch.i")],
            row[index("clipped")],
            "at t = {}",
            row[0]
        );
        if row[index("switch.blocking")] > 0.5 {
            // The blocking branch is `i = 0`, and it holds exactly.
            assert_eq!(row[index("switch.i")], 0.0, "at t = {}", row[0]);
            blocking_rows += 1;
        } else {
            // The conducting branch is `v = 0`, likewise.
            assert_eq!(row[index("switch.v")], 0.0, "at t = {}", row[0]);
            conducting_rows += 1;
        }
    }
    // Two full periods: the switch really did work both ways.
    assert!(blocking_rows > 400 && conducting_rows > 400);
}

#[test]
fn a_chopped_supply_draws_the_exact_staircase() {
    // The supply's two branches are kept and merged into one
    // equation apiece, decided while the run goes; the relation
    // driving them is an event indicator, so the switching
    // instants land exactly. What comes out is an RC charging
    // towards the supply for half a period and towards zero for
    // the next, which has a closed form.
    let result = compile(&with_library("switched_rc.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (supply, tau, half) = (10.0f64, 0.2f64, 0.5f64);
    let exact = |t: f64| {
        let (mut voltage, mut at, mut on) = (0.0f64, 0.0f64, true);
        while at < t - 1e-15 {
            let next = (at / half + 1e-9).floor() * half + half;
            let until = next.min(t);
            let target = if on { supply } else { 0.0 };
            voltage = target - (target - voltage) * (-(until - at) / tau).exp();
            if until >= next - 1e-12 {
                on = !on;
            }
            at = until;
        }
        voltage
    };
    for row in &result.rows {
        let wanted = exact(row[0]);
        assert!(
            (row[index("capacitor.v")] - wanted).abs() < 1e-6,
            "t = {}: v = {} vs {wanted}",
            row[0],
            row[index("capacitor.v")]
        );
    }
    // The other equation of each branch travelled with it.
    for row in &result.rows {
        let energised = row[index("supply.energised")] > 0.5;
        let delivered = row[index("supply.delivered")];
        if energised {
            assert!((delivered - supply * row[index("supply.p.i")]).abs() < 1e-9);
        } else {
            assert_eq!(delivered, 0.0);
        }
    }
}

#[test]
fn a_control_loop_wired_through_a_bus_closes() {
    // Nothing in the model wires the plant to the controller
    // directly: both talk to an expandable bus, and a sub-bus
    // carries the same members because it is joined to it. The
    // loop that comes out settles at k*r/(1+k) with time constant
    // T/(1+k).
    let result = compile(&with_library("signal_bus.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (k, r, t_plant) = (4.0f64, 1.0f64, 0.5f64);
    let settled = k * r / (1.0 + k);
    let tau = t_plant / (1.0 + k);
    for row in &result.rows {
        let expected = settled * (1.0 - (-row[0] / tau).exp());
        assert!(
            (row[index("plant.x")] - expected).abs() < 1e-6,
            "t = {}: x = {} vs {expected}",
            row[0],
            row[index("plant.x")]
        );
    }
    // The bus and the sub-bus really do carry the same signal.
    let last = result.rows.last().unwrap();
    assert_eq!(
        last[index("bus.measurement.y")],
        last[index("subbus.measurement.y")]
    );
    assert_eq!(
        last[index("bus.command.y")],
        last[index("subbus.command.y")]
    );
    // What the plant is driven with is the law applied to what the
    // controller heard, both of which travelled through the bus.
    let heard = last[index("controller.measurement.y")];
    assert!((last[index("plant.u.y")] - k * (r - heard)).abs() < 1e-12);
    assert!((heard - settled).abs() < 1e-4, "not settled: {heard}");
}

#[test]
fn a_stream_junction_mixes_and_the_tank_relaxes_to_it() {
    // Two sources push 1 kg/s at h=100 and 3 kg/s at h=20 into a
    // three-way node; the junction hands the tank their
    // flow-weighted mix and the tank's contents approach it as a
    // first-order lag with time constant mass / m_flow = 2 s.
    let result = compile(&with_library("stream_mixer.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let mix = (1.0 * 100.0 + 3.0 * 20.0) / 4.0;
    let last = result.rows.last().unwrap();
    assert!((last[index("h_supplied")] - mix).abs() < 1e-9);
    for row in &result.rows {
        let expected = mix * (1.0 - (-row[0] / 2.0).exp());
        assert!(
            (row[index("tank.h")] - expected).abs() < 1e-5,
            "t = {}: h = {} vs {expected}",
            row[0],
            row[index("tank.h")]
        );
    }
}

#[test]
fn a_while_loop_computes_the_exact_large_swing_period() {
    // The function runs an arithmetic-geometric mean to convergence
    // at compile time; the simulated pendulum must come back to its
    // amplitude exactly one such period later.
    let result = compile(&with_library("pendulum_period.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();

    let (length, gravity, amplitude) = (1.2f64, 9.81f64, 1.0f64);
    let (mut a, mut b) = (1.0f64, (amplitude / 2.0).cos());
    while (a - b).abs() > 1e-15 {
        let mean = 0.5 * (a + b);
        b = (a * b).sqrt();
        a = mean;
    }
    let period = 2.0 * std::f64::consts::PI * (length / gravity).sqrt() / a;
    assert!(
        (result.rows[0][index("period")] - period).abs() < 1e-12,
        "period {} vs {period}",
        result.rows[0][index("period")]
    );
    // Small-angle theory would say 2.1972 s; the true period at a
    // 1 rad swing is 2.3430 s, and the trajectory knows it.
    let row = result
        .rows
        .iter()
        .min_by(|p, q| {
            let (dp, dq) = ((p[0] - period).abs(), (q[0] - period).abs());
            dp.partial_cmp(&dq).unwrap()
        })
        .unwrap();
    assert!(
        (row[index("theta")] - amplitude).abs() < 1e-3,
        "theta {} after one period",
        row[index("theta")]
    );
    assert!(
        row[index("w")].abs() < 0.05,
        "w {} at the turn",
        row[index("w")]
    );
}

#[test]
fn the_flight_plan_and_the_flown_trajectory_agree() {
    // `(planned_range, planned_duration) = flight(v0, angle)` fills
    // both targets from one call, with gravity defaulted inside the
    // function; the integrated throw must land where it says.
    let result = compile(&with_library("ballistic_range.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();

    let (v0, angle, g) = (12.0f64, 0.6f64, 9.81);
    let range = v0 * v0 * (2.0 * angle).sin() / g;
    let duration = 2.0 * v0 * angle.sin() / g;
    // The planned values are constants over the whole run.
    assert!((result.rows[0][index("planned_range")] - range).abs() < 1e-12);
    assert!((last[index("planned_range")] - range).abs() < 1e-12);
    assert!((last[index("planned_duration")] - duration).abs() < 1e-12);
    // The run stops within a hair of the planned landing, so the
    // ball is at the planned range and back on the ground.
    assert!(
        (last[index("x")] - range).abs() < 1e-3,
        "landed at {} instead of {range}",
        last[index("x")]
    );
    assert!(
        last[index("y")].abs() < 1e-2,
        "still {} up",
        last[index("y")]
    );
}

#[test]
fn dc_motor_from_library_components_matches_theory() {
    // Three domains at once: an electrical circuit, the EMF
    // coupling and a rotational load, all from library packages.
    let result = compile(&with_library("dc_motor.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();

    // Steady state: k*i = d*w and V = i*R + k*w.
    let (v, r, k, d) = (24.0, 0.5, 0.3, 0.02);
    let speed = v * k / (k * k + d * r);
    assert!(
        (last[index("speed")] - speed).abs() < 1e-2,
        "speed {} vs {speed}",
        last[index("speed")]
    );
    assert!((last[index("current")] - d * speed / k).abs() < 1e-3);

    // The supply steps at t = 0.1, so nothing moves before that.
    let early = result
        .rows
        .iter()
        .find(|row| row[0] >= 0.05)
        .expect("a sample before the step");
    assert!(early[index("speed")].abs() < 1e-9);
}

#[test]
fn pi_control_loop_from_library_blocks_settles_on_the_setpoint() {
    let result = compile(&with_library("control_loop.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    // Integral action removes the steady-state error.
    assert!(
        last[index("e")].abs() < 1e-4,
        "residual error {}",
        last[index("e")]
    );
    assert!((last[index("y")] - 1.0).abs() < 1e-4);
}

#[test]
fn a_world_shared_through_inner_outer_drives_a_projectile() {
    // The point mass reads gravity from the `inner World` of the top
    // model; the trajectory is a polynomial the solver integrates
    // exactly.
    let result = compile(&with_library("projectile.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (x, y, vy) = (index("ball.x"), index("ball.y"), index("ball.vy"));
    for row in &result.rows {
        let t = row[0];
        assert!((row[x] - 12.0 * t).abs() < 1e-12, "x at {t}: {}", row[x]);
        let height = 16.0 * t - 0.5 * 9.81 * t * t;
        assert!((row[y] - height).abs() < 1e-12, "y at {t}: {}", row[y]);
        assert!((row[vy] - (16.0 - 9.81 * t)).abs() < 1e-12);
    }
}

#[test]
fn a_conditional_support_carries_the_reaction_torque() {
    // Two identical drives: one reacting on its internal housing,
    // one on an exposed support flange. The shafts must not be able
    // to tell the difference.
    let result = compile(&with_library("torque_support.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    for row in &result.rows {
        assert!(row[index("difference")].abs() < 1e-12);
    }
    // phi = tau / (2 J) t^2 with tau = 2, J = 0.5, t = 4.
    assert!((last[index("shaftA.phi")] - 32.0).abs() < 1e-9);
    // The exposed support takes the reaction; the internal one hides it.
    assert!((last[index("driveB.support.tau")] - 2.0).abs() < 1e-12);
    assert!(!result.columns.iter().any(|c| c == "driveA.support.tau"));
}

#[test]
fn a_redeclared_controller_changes_the_steady_state() {
    // The example file ships a proportional drive and a derived model
    // that redeclares the controller as a PI. Loading the file as a
    // library reaches both.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let library = std::fs::read_to_string(root.join("lib/Oxidelica.mo")).unwrap();
    let example = std::fs::read_to_string(root.join("examples/pi_drive.mo")).unwrap();
    let run_top = |source: &str| {
        let model = oxidelica_parser::parse_model_with_libraries(
            &[library.clone(), example.clone()],
            source,
        )
        .unwrap();
        compile(&model).unwrap().simulate().unwrap()
    };

    // Proportional only: a gain of 5 on a unit-gain plant leaves
    // 1/(1+5) of the setpoint as a standing error.
    let plain = run_top(
        "model Plain ProportionalDrive drive; Real y; equation y = drive.y; \
         annotation(experiment(StopTime = 6.0, Interval = 0.01, Tolerance = 1e-9)); end Plain;",
    );
    let y = plain.columns.iter().position(|c| c == "y").unwrap();
    assert!((plain.rows.last().unwrap()[y] - 5.0 / 6.0).abs() < 1e-6);

    // With the PI redeclared in, the offset is gone.
    let tuned = run_top(
        "model Tuned PIDrive drive; Real y; equation y = drive.y; \
         annotation(experiment(StopTime = 6.0, Interval = 0.01, Tolerance = 1e-9)); end Tuned;",
    );
    let y = tuned.columns.iter().position(|c| c == "y").unwrap();
    assert!((tuned.rows.last().unwrap()[y] - 1.0).abs() < 1e-4);
}

#[test]
fn an_enumeration_selects_the_shape_of_a_waveform() {
    // Square wave through a first-order lag: on the first half
    // period the answer is the analytic step response, and the jump
    // at the half period is an event the solver stops at.
    let result = compile(&with_library("waveform.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (u, y) = (index("u"), index("y"));
    for row in &result.rows {
        let t = row[0];
        assert!(row[u].abs() == 1.0, "square wave value {} at {t}", row[u]);
        if t <= 1.0 - 1e-9 {
            let expected = 1.0 - (-t / 0.3f64).exp();
            assert!((row[y] - expected).abs() < 1e-6, "y at {t}: {}", row[y]);
        }
    }

    // The triangle shape of the same source is asin of a sine.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let library = std::fs::read_to_string(root.join("lib/Oxidelica.mo")).unwrap();
    let model = oxidelica_parser::parse_model_with_libraries(
        &[library],
        "model T Oxidelica.Blocks.Sources.Waveform source(\
           kind = Oxidelica.Types.WaveformKind.Triangle, f = 0.5); \
         Real u; equation u = source.y; \
         annotation(experiment(StopTime = 3.0, Interval = 0.01)); end T;",
    )
    .unwrap();
    let triangle = compile(&model).unwrap().simulate().unwrap();
    let u = triangle.columns.iter().position(|c| c == "u").unwrap();
    for row in &triangle.rows {
        let expected = 2.0 * (std::f64::consts::PI * row[0]).sin().asin() / std::f64::consts::PI;
        assert!((row[u] - expected).abs() < 1e-12);
    }
}

#[test]
fn a_sampled_controller_holds_its_output_between_ticks() {
    let result = compile(&with_library("sampled_control.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (u, y) = (index("u"), index("y"));
    let (period, plant_time) = (0.1f64, 0.5f64);

    // The control signal changes only on the clock, and the ticks
    // land on the period exactly rather than on the output grid.
    let mut ticks = Vec::new();
    for pair in result.rows.windows(2) {
        if pair[0][u] != pair[1][u] {
            ticks.push(pair[1][0]);
        }
    }
    assert_eq!(ticks.len(), 50, "one tick per period over five seconds");
    for t in &ticks {
        let off = t / period - (t / period).round();
        assert!(off.abs() < 1e-9, "tick off the clock at t = {t}");
    }

    // Between two ticks the plant is a first-order lag relaxing
    // toward the held value, which has a closed form.
    let mut worst = 0.0f64;
    for pair in result.rows.windows(2) {
        let (t0, t1) = (pair[0][0], pair[1][0]);
        if t1 <= t0 || pair[0][u] != pair[1][u] {
            continue;
        }
        let held = pair[1][u];
        let expected = held + (pair[0][y] - held) * (-(t1 - t0) / plant_time).exp();
        worst = worst.max((pair[1][y] - expected).abs());
    }
    assert!(worst < 1e-8, "hold response off by {worst}");

    // Integral action still lands on the setpoint.
    assert!((result.rows.last().unwrap()[y] - 1.0).abs() < 1e-3);
}

#[test]
fn hysteresis_switches_exactly_on_its_band() {
    let result = compile(&with_library("thermostat.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (temperature, heating, switches) = (index("T"), index("heating"), index("switches"));

    let mut switch_on = Vec::new();
    let mut switch_off = Vec::new();
    for pair in result.rows.windows(2) {
        if pair[0][heating] == pair[1][heating] {
            continue;
        }
        if pair[1][heating] > 0.5 {
            switch_on.push((pair[1][0], pair[1][temperature]));
        } else {
            switch_off.push((pair[1][0], pair[1][temperature]));
        }
    }
    // The heater switches on the band edges, located to the same
    // tolerance as any other event.
    for (_, t) in &switch_off {
        assert!((t - 21.0).abs() < 1e-6, "switched off at {t}");
    }
    for (_, t) in &switch_on {
        assert!((t - 19.0).abs() < 1e-6, "switched on at {t}");
    }

    // Heating from 19 to 21 and cooling back is a closed form: the
    // room chases 29 with the heater on and 5 with it off, both with
    // the time constant C / G = 200 s.
    let expected = 200.0 * (10.0f64 / 8.0).ln() + 200.0 * (16.0f64 / 14.0).ln();
    for pair in switch_on.windows(2) {
        let period = pair[1].0 - pair[0].0;
        assert!(
            (period - expected).abs() < 1e-3,
            "cycle {period} vs {expected}"
        );
    }
    // The counter counted the switch-ons, and only those.
    assert_eq!(
        result.rows.last().unwrap()[switches] as i64,
        switch_on.len() as i64 + 1,
        "the heater starts on, so the count leads the switch-ons by one"
    );
}

#[test]
fn the_discrete_library_blocks_run_on_the_clock() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let library = std::fs::read_to_string(root.join("lib/Oxidelica.mo")).unwrap();
    let run_top = |source: &str| {
        let model =
            oxidelica_parser::parse_model_with_libraries(std::slice::from_ref(&library), source)
                .unwrap();
        compile(&model).unwrap().simulate().unwrap()
    };

    // A unit delay carries the previous tick's value, so against a
    // ramp its output trails the input by exactly one period.
    let delayed = run_top(
        "model D Oxidelica.Blocks.Discrete.UnitDelay delay(samplePeriod = 0.25); \
         Real ramp; equation ramp = time; delay.u = ramp; \
         annotation(experiment(StopTime = 2.0, Interval = 0.05)); end D;",
    );
    let index =
        |result: &SimResult, name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (y, held) = (index(&delayed, "delay.y"), index(&delayed, "delay.held"));
    for row in &delayed.rows {
        let t = row[0];
        // A tick instant carries two rows, before and after the
        // event; the value between ticks is the one to check.
        if t < 0.5 || (t / 0.25 - (t / 0.25).round()).abs() < 1e-9 {
            continue;
        }
        // At time t the output is the input from one tick earlier.
        let tick = (t / 0.25).floor() * 0.25;
        let expected = (tick - 0.25).max(0.0);
        assert!(
            (row[y] - expected).abs() < 1e-9,
            "t = {t}: delayed {} vs {expected}",
            row[y]
        );
        assert!((row[held] - tick).abs() < 1e-9);
    }

    // The library controller reproduces the hand-written one of the
    // example, tick for tick.
    let library_pi = run_top(
        "model L Oxidelica.Blocks.Discrete.PI controller(samplePeriod = 0.1, k = 2.0, Ti = 0.5); \
         Real y(start = 0, fixed = true); equation controller.u = 1.0 - y; \
         der(y) = (controller.y - y) / 0.5; \
         annotation(experiment(StopTime = 5.0, Interval = 0.002, Tolerance = 1e-9)); end L;",
    );
    let by_hand = compile(&with_library("sampled_control.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let (a, b) = (index(&library_pi, "y"), index(&by_hand, "y"));
    assert_eq!(library_pi.rows.len(), by_hand.rows.len());
    for (left, right) in library_pi.rows.iter().zip(&by_hand.rows) {
        assert!((left[a] - right[b]).abs() < 1e-12);
    }
}

#[test]
fn event_iteration_chains_the_clauses_of_one_event() {
    // `initial()` fires before the first output point; the edge of a
    // discrete variable and the change of a counter are seen inside
    // the same event that produced them.
    let result = run(
        "model M Real x(start = 0, fixed = true); Boolean started(start = false); \
         Boolean on(start = false); Integer rises(start = 0); Integer changes(start = 0); \
         equation der(x) = 1; \
         when initial() then started = true; end when; \
         when x > 0.5 then on = true; end when; \
         when edge(on) then rises = pre(rises) + 1; end when; \
         when change(rises) then changes = pre(changes) + 1; end when; \
         annotation(experiment(StopTime = 1.0, Interval = 0.05)); end M;",
    );
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let first = &result.rows[0];
    assert_eq!(first[index("started")], 1.0, "initial() fires at t = 0");
    assert_eq!(first[index("rises")], 0.0);

    let last = result.rows.last().unwrap();
    assert_eq!(last[index("on")], 1.0);
    assert_eq!(last[index("rises")], 1.0);
    assert_eq!(last[index("changes")], 1.0);
    // Everything happened in the single event at x = 0.5.
    let switch = result
        .rows
        .iter()
        .find(|row| row[index("rises")] > 0.5)
        .expect("the chain fires");
    assert!((switch[index("x")] - 0.5).abs() < 1e-6);
    assert_eq!(switch[index("changes")], 1.0);
}

#[test]
fn the_discrete_layer_reports_its_error_paths() {
    let error = |source: &str| {
        let model = parse_model(source).unwrap();
        compile(&model).unwrap_err().to_string()
    };
    // `pre` needs a variable that has a value from before the event.
    assert!(error(
        "model M Real x(start = 0, fixed = true); Real y; equation der(x) = 1; y = pre(x); end M;"
    )
    .contains("not discrete"));
    // A clock with no period.
    assert!(error(
        "model M Real x(start = 0, fixed = true); Real u; equation der(x) = 1; \
         when sample(0, 0) then u = x; end when; end M;"
    )
    .contains("interval must be positive"));
    // A `when` assigning something that was never declared.
    assert!(error(
        "model M Real x(start = 0, fixed = true); equation der(x) = 1; \
         when x > 0.5 then u = 1; end when; end M;"
    )
    .contains("never declared"));
    // A discrete variable nothing ever assigns.
    assert!(error(
        "model M discrete Real u; Real x(start = 0, fixed = true); equation der(x) = 1; end M;"
    )
    .contains("never assigned"));
    // `pre` of an expression rather than a variable.
    assert!(error(
        "model M Real x(start = 0, fixed = true); Real u; equation der(x) = 1; \
         when x > 0.5 then u = pre(x + 1); end when; end M;"
    )
    .contains("takes a variable"));

    // The fixed grid of RK4 cannot step onto a time event.
    let model = parse_model(
        "model M Real x(start = 0, fixed = true); Real u; equation der(x) = 1; \
         when sample(0, 0.1) then u = x; end when; \
         annotation(experiment(StopTime = 1.0, Interval = 0.05)); end M;",
    )
    .unwrap();
    let mut compiled = compile(&model).unwrap();
    compiled.method = SolverMethod::Rk4;
    assert!(compiled
        .simulate()
        .unwrap_err()
        .to_string()
        .contains("dopri45 or bdf"));
    // The stiff solver steps onto them like the adaptive one does.
    compiled.method = SolverMethod::Bdf;
    let stiff = compiled.simulate().unwrap();
    let u = stiff.columns.iter().position(|c| c == "u").unwrap();
    assert!((stiff.rows.last().unwrap()[u] - 1.0).abs() < 1e-6);
}

#[test]
fn an_algorithm_converts_a_held_sample_into_a_staircase() {
    let result = compile(&with_library("quantizer.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (sample, quantized, error) = (index("adc.y"), index("quantized"), index("error"));
    let (step, levels, period) = (0.25f64, 4, 0.05f64);

    // The staircase the algorithm describes, evaluated the same way
    // it is written: strictly above a level to reach it.
    let staircase = |value: f64| {
        let mut out = 0.0;
        for i in 1..=levels {
            if value > i as f64 * step {
                out = i as f64 * step;
            }
            if value < -(i as f64) * step {
                out = -(i as f64) * step;
            }
        }
        out
    };
    for row in &result.rows {
        assert_eq!(row[quantized], staircase(row[sample]));
        assert!(row[error].abs() <= step + 1e-12);
        // Between ticks the converter holds the signal it sampled.
        let t = row[0];
        if (t / period - (t / period).round()).abs() < 1e-9 {
            continue;
        }
        let tick = (t / period).floor() * period;
        let held = (2.0 * std::f64::consts::PI * tick).sin();
        assert!(
            (row[sample] - held).abs() < 1e-8,
            "t = {t}: held {} vs {held}",
            row[sample]
        );
    }
}

#[test]
fn an_initial_equation_section_starts_the_model_in_equilibrium() {
    let result = compile(&with_library("steady_start.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (temperature, x, v) = (index("T"), index("x"), index("v"));
    let first = &result.rows[0];

    // Heater against losses, and the spring against gravity.
    assert!((first[temperature] - (5.0 + 3000.0 / 250.0)).abs() < 1e-9);
    assert!((first[x] - (-2.0 * 9.81 / 40.0)).abs() < 1e-9);
    assert!(first[v].abs() < 1e-12);
    // Started at the balance point, nothing moves for the whole run.
    for row in &result.rows {
        assert!((row[temperature] - first[temperature]).abs() < 1e-9);
        assert!((row[x] - first[x]).abs() < 1e-9);
        assert!(row[v].abs() < 1e-9);
    }
}

#[test]
fn initialization_mixes_fixed_starts_with_initial_equations() {
    // One state is pinned by `fixed = true`, the other follows from
    // an initial equation that ties the two together.
    let result = run(
        "model M Real a(start = 2, fixed = true); Real b(start = 0); \
         equation der(a) = -a; der(b) = a - b; \
         initial equation b = 3 * a; \
         annotation(experiment(StopTime = 0.1, Interval = 0.05)); end M;",
    );
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let first = &result.rows[0];
    assert!((first[index("a")] - 2.0).abs() < 1e-12);
    assert!((first[index("b")] - 6.0).abs() < 1e-9);
}

#[test]
fn initialization_reports_what_it_cannot_solve() {
    let error = |source: &str| {
        let model = parse_model(source).unwrap();
        compile(&model).unwrap_err().to_string()
    };
    // Two initial equations for one free state.
    assert!(error(
        "model M Real a(start = 1); equation der(a) = -a; \
         initial equation a = 1; der(a) = 0; end M;"
    )
    .contains("not square"));
    // An initial equation that says nothing about the state.
    assert!(error(
        "model M Real a(start = 1); Real b; equation der(a) = -a; b = 2 * a; \
         initial equation b = 2 * a; end M;"
    )
    .contains("singular"));
    // `der` of something that is not a state.
    assert!(error(
        "model M Real a(start = 1); Real b; equation der(a) = -a; b = a; \
         initial equation der(b) = 0; end M;"
    )
    .contains("is not a state"));
}

#[test]
fn the_solver_picks_itself() {
    // A decay with a time constant of a microsecond over a second of
    // simulated time: the explicit method could only crawl through
    // it, so the run should end up on the implicit one - and land on
    // the analytic answer all the same.
    let stiff = run(
        "model S Real x(start = 1, fixed = true); Real slow(start = 1, fixed = true); \
         equation der(x) = -1e6 * (x - slow); der(slow) = -slow; \
         annotation(experiment(StopTime = 1.0, Interval = 0.05, Tolerance = 1e-6)); end S;",
    );
    assert_eq!(stiff.method, SolverMethod::Bdf, "a stiff model needs bdf");
    let slow = stiff.columns.iter().position(|c| c == "slow").unwrap();
    let last = stiff.rows.last().unwrap();
    assert!(
        (last[slow] - (-1.0f64).exp()).abs() < 1e-5,
        "{}",
        last[slow]
    );
    // The fast state follows the slow one, which is the point of the
    // stiff pair.
    let x = stiff.columns.iter().position(|c| c == "x").unwrap();
    assert!((last[x] - last[slow]).abs() < 1e-5);

    // An ordinary model stays where it started, and says so.
    let gentle = run(
        "model G Real x(start = 1, fixed = true); equation der(x) = -x; \
         annotation(experiment(StopTime = 1.0, Interval = 0.1)); end G;",
    );
    assert_eq!(gentle.method, SolverMethod::Dopri45);
    // The default tolerance is 1e-6, so that is what to expect of it.
    assert!((gentle.rows.last().unwrap()[1] - (-1.0f64).exp()).abs() < 1e-6);

    // Asking for a method by name still overrides the choice.
    assert_eq!(SolverMethod::from_name("auto"), Some(SolverMethod::Auto));
    let model = parse_model(
        "model G Real x(start = 1, fixed = true); equation der(x) = -x; \
         annotation(experiment(StopTime = 1.0, Interval = 0.1)); end G;",
    )
    .unwrap();
    let mut compiled = compile(&model).unwrap();
    compiled.method = SolverMethod::Bdf;
    assert_eq!(compiled.simulate().unwrap().method, SolverMethod::Bdf);
}

#[test]
fn the_banded_solver_agrees_with_the_dense_one() {
    // A tridiagonal system with a dominant diagonal, the shape a
    // discretized field gives: both paths must land on the same
    // answer, and it must satisfy the equations.
    let n = 12usize;
    let band = 1usize;
    let dense: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|j| match i.abs_diff(j) {
                    0 => 4.0 + i as f64 * 0.1,
                    1 => -1.0,
                    _ => 0.0,
                })
                .collect()
        })
        .collect();
    let rhs: Vec<f64> = (0..n).map(|i| (i as f64 * 0.7).sin()).collect();

    let packed: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            (0..2 * band + 1)
                .map(|offset| match (i + offset).checked_sub(band) {
                    Some(column) if column < n => dense[i][column],
                    _ => 0.0,
                })
                .collect()
        })
        .collect();
    let banded = solve_banded(&mut packed.clone(), band, &rhs).expect("diagonally dominant");
    let plain = solve_linear(&mut dense.clone(), &rhs).expect("nonsingular");
    for (a, b) in banded.iter().zip(&plain) {
        assert!((a - b).abs() < 1e-12, "{a} vs {b}");
    }
    // And the answer really solves the system.
    for (i, row) in dense.iter().enumerate() {
        let value: f64 = row.iter().zip(&banded).map(|(a, x)| a * x).sum();
        assert!((value - rhs[i]).abs() < 1e-12);
    }

    // Without a diagonal to pivot on it declines instead of dividing
    // by nothing, and the caller falls back to the dense path.
    let mut hollow = vec![vec![0.0, 0.0, 1.0], vec![1.0, 0.0, 0.0]];
    assert!(solve_banded(&mut hollow, 1, &[1.0, 1.0]).is_none());
}

#[test]
fn a_chain_of_masses_written_with_arrays_conserves_energy() {
    // Five bodies between two walls, everything about them arrays:
    // literals, fill, linspace, an array start, whole-array
    // equations and reductions. The check is physical, not textual:
    // the first body is pushed and the energy must stay put.
    let result = compile(&with_library("spring_chain.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let energy = index("energy");
    let first = result.rows[0][energy];
    // 0.5 * m[1] * push^2 with m[1] = 1 and push = 2.
    assert!((first - 2.0).abs() < 1e-9, "{first}");
    for row in &result.rows {
        assert!(
            (row[energy] - first).abs() < 1e-6,
            "drift at t = {}",
            row[0]
        );
    }
    // The bodies start on the linspace grid.
    assert!((result.rows[0][index("x[1]")] - 0.5).abs() < 1e-12);
    assert!((result.rows[0][index("x[5]")] - 2.5).abs() < 1e-12);
}

#[test]
fn a_ladder_of_resistors_wired_by_a_loop_divides_the_supply() {
    // One array declaration, `each R`, and the wiring written as a
    // loop of connects over the elements. Five equal resistors on
    // 10 V put exactly 8, 6, 4, 2, 0 volts on the taps.
    let result = compile(&with_library("resistor_ladder.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    for i in 1..=5 {
        let expected = 10.0 * (5 - i) as f64 / 5.0;
        let got = last[index(&format!("taps[{i}]"))];
        assert!(
            (got - expected).abs() < 1e-12,
            "tap {i}: {got} vs {expected}"
        );
    }
    // The same current runs through the whole chain.
    let current = last[index("r[1].i")];
    assert!((current - 10.0 / (5.0 * 220.0)).abs() < 1e-15);
    for i in 2..=5 {
        assert!((last[index(&format!("r[{i}].i"))] - current).abs() < 1e-15);
    }
}

#[test]
fn a_pendulum_over_the_top_reselects_its_states() {
    // The known limit of a static selection, now closed: enough
    // speed to rotate fully, so the length constraint has to swap
    // which coordinate it defines every quarter turn.
    let result = compile(&with_library("spinning_pendulum.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    assert!(
        result.reselections >= 4,
        "a full rotation needs several re-selections, saw {}",
        result.reselections
    );
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (x, y, vx, vy) = (index("x"), index("y"), index("vx"), index("vy"));

    let first = &result.rows[0];
    let energy = |row: &Vec<f64>| 0.5 * (row[vx] * row[vx] + row[vy] * row[vy]) + 9.81 * row[y];
    let e0 = energy(first);
    let mut revolutions = 0;
    for pair in result.rows.windows(2) {
        // The rod length holds exactly through every switch.
        let constraint = pair[1][x] * pair[1][x] + pair[1][y] * pair[1][y] - 1.0;
        assert!(constraint.abs() < 1e-6, "constraint {constraint}");
        // And so does the energy - a wrong branch after a switch
        // (the bug this test was written against) zeroes it.
        assert!(
            (energy(&pair[1]) - e0).abs() < 1e-3,
            "energy drifted to {} from {e0}",
            energy(&pair[1])
        );
        if pair[0][y] < 0.0 && pair[1][y] >= 0.0 && pair[1][x] > 0.0 {
            revolutions += 1;
        }
    }
    assert!(revolutions >= 3, "kept rotating, saw {revolutions}");

    // The whole trajectory agrees with the angle form of the same
    // pendulum - an independent formulation of the same physics.
    let reference = run(
        "model SpinAngle Real th(start = 0, fixed = true); Real w(start = 8, fixed = true); \
         Real x; Real y; equation der(th) = w; der(w) = -9.81 * sin(th); \
         x = sin(th); y = -cos(th); \
         annotation(experiment(StopTime = 3.0, Interval = 0.002, Tolerance = 1e-9)); \
         end SpinAngle;",
    );
    let rx = reference.columns.iter().position(|c| c == "x").unwrap();
    let mut worst = 0.0f64;
    let mut checked = 0;
    for row in &result.rows {
        let Some(matching) = reference
            .rows
            .iter()
            .find(|other| (other[0] - row[0]).abs() < 1e-9)
        else {
            continue;
        };
        worst = worst.max((row[x] - matching[rx]).abs());
        checked += 1;
    }
    assert!(checked > 1000, "grids barely overlap: {checked}");
    assert!(worst < 1e-4, "cartesian vs angle form: {worst}");
}

#[test]
fn a_damper_straight_onto_a_fixed_flange_works() {
    // The former known limit: the damper's relative angle is
    // redundant with the shaft angle, and reducing the index means
    // differentiating a connection equality through connector
    // potentials no equation defines explicitly - they are pinned
    // only linearly. J = 0.5, d = 0.4: the shaft speed must decay
    // as 5*exp(-0.8 t).
    let result = compile(&with_library("damper_on_fixed.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (w, phi, phi_rel) = (
        index("shaft.w"),
        index("shaft.phi"),
        index("damper.phi_rel"),
    );
    for row in &result.rows {
        let expected = 5.0 * (-0.8 * row[0]).exp();
        assert!(
            (row[w] - expected).abs() < 1e-6,
            "w at {}: {} vs {expected}",
            row[0],
            row[w]
        );
        // The relative angle mirrors the shaft, holding the
        // redundant pair consistent through the whole run.
        assert!((row[phi_rel] + row[phi]).abs() < 1e-9);
    }
}

#[test]
fn a_start_written_through_a_type_alias_is_kept() {
    // `Units.AngularVelocity w(start = w0)` parses its parenthesis
    // as modifiers, not attributes; the initial condition used to
    // vanish without a sound. Found because the damper test above
    // started from rest instead of 5 rad/s.
    let result = run(
        "package Units type Speed = Real(unit = \"m/s\"); end Units; \
         model M parameter Real w0 = 5; parameter Real tau(unit = \"s\") = 1; \
         Units.Speed w(start = w0); \
         equation der(w) = -w / tau; \
         annotation(experiment(StopTime = 1.0, Interval = 0.1)); end M;",
    );
    assert!(
        (result.rows[0][1] - 5.0).abs() < 1e-12,
        "{}",
        result.rows[0][1]
    );
    // And the declaration's own start wins over an alias default.
    let overridden = run("package Units type Speed = Real(start = 7); end Units; \
         model M Units.Speed w(start = 5); equation der(w) = -w; \
         annotation(experiment(StopTime = 1.0, Interval = 0.1)); end M;");
    assert!((overridden.rows[0][1] - 5.0).abs() < 1e-12);
}

#[test]
fn a_replaceable_medium_changes_what_the_tank_holds() {
    // The example file ends with the oil variant, so that is the
    // entry point: heating follows oil's density and heat capacity.
    let oil = compile(&with_library("replaceable_medium.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index =
        |result: &SimResult, name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let temperature = index(&oil, "T");
    let last = oil.rows.last().unwrap();
    let expected_oil = 20.0 + 600.0 * 50000.0 / (0.2 * 900.0 * 1900.0);
    assert!(
        (last[temperature] - expected_oil).abs() < 1e-6,
        "oil: {} vs {expected_oil}",
        last[temperature]
    );
    // And the viscosity comes from oil's own function.
    let viscosity = index(&oil, "mu");
    let expected_mu = 0.1 * (-0.05f64 * (expected_oil - 20.0)).exp();
    assert!((last[viscosity] - expected_mu).abs() < 1e-6);

    // The same tank with its default medium heats like water.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = std::fs::read_to_string(root.join("examples/replaceable_medium.mo")).unwrap();
    let water_only = source
        .replace("model OilTank", "partial model OilTank")
        .replace(
            "end OilTank;",
            "end OilTank; model WaterTank extends HeatedTank; \
             annotation(experiment(StopTime = 600.0, Interval = 1.0)); end WaterTank;",
        );
    let water = compile(&oxidelica_parser::parse_model(&water_only).unwrap())
        .unwrap()
        .simulate()
        .unwrap();
    let temperature = index(&water, "T");
    let expected_water = 20.0 + 600.0 * 50000.0 / (0.2 * 1000.0 * 4186.0);
    let last = water.rows.last().unwrap();
    assert!(
        (last[temperature] - expected_water).abs() < 1e-6,
        "water: {} vs {expected_water}",
        last[temperature]
    );
}

#[test]
fn the_numeric_builtins_follow_their_definitions() {
    // ceil/floor/integer, div/mod/rem - checked at runtime and in
    // the compile-time path, both against the spec definitions.
    let result = run(
        "model N Real u; Real a; Real b; Real c; Real d; Real e; Real f; \
         equation u = 3 * sin(time); \
         a = ceil(u); b = floor(u); c = integer(u); \
         d = div(u, 2.0); e = mod(u, 2.0); f = rem(u, 2.0); \
         annotation(experiment(StopTime = 3.0, Interval = 0.05)); end N;",
    );
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    for row in &result.rows {
        let u = row[index("u")];
        assert_eq!(row[index("a")], u.ceil());
        assert_eq!(row[index("b")], u.floor());
        assert_eq!(row[index("c")], u.floor());
        assert_eq!(row[index("d")], (u / 2.0).trunc());
        assert_eq!(row[index("e")], u - (u / 2.0).floor() * 2.0);
        assert_eq!(row[index("f")], u - (u / 2.0).trunc() * 2.0);
    }
    // Their derivatives are flat almost everywhere: a staircase as
    // a state source integrates without complaint.
    let stepped = run("model S Real x(start = 0, fixed = true); \
         equation der(x) = floor(time) - floor(time); \
         annotation(experiment(StopTime = 1.0, Interval = 0.1)); end S;");
    assert!(stepped.rows.last().unwrap()[1].abs() < 1e-12);
}

#[test]
fn asserts_stop_the_run_with_their_message() {
    // Holding: the run completes.
    let fine = run("model A Real u; equation u = sin(time); \
         assert(u < 2.0, \"cannot happen\"); \
         annotation(experiment(StopTime = 1.0, Interval = 0.1)); end A;");
    assert!(fine.terminated.is_none());

    // Violated: the run stops at the crossing and names the check.
    let model = parse_model(
        "model B Real u; equation u = 3 * sin(time); \
         assert(u < 2.0, \"the input left its window\", AssertionLevel.error); \
         annotation(experiment(StopTime = 3.0, Interval = 0.01)); end B;",
    )
    .unwrap();
    let error = compile(&model).unwrap().simulate().unwrap_err().to_string();
    assert!(error.contains("the input left its window"), "{error}");
    assert!(error.contains("assertion failed at t = 0.73"), "{error}");

    // `block` is a class kind now.
    let block = run("block G Real y; equation y = 2 * time; \
         annotation(experiment(StopTime = 1.0, Interval = 0.5)); end G;");
    assert!((block.rows.last().unwrap()[1] - 2.0).abs() < 1e-12);
}

#[test]
fn matrix_results_survive_the_csv_round_trip() {
    // A 2-D name holds a comma; the CSV quotes it and the value
    // reads back intact.
    let result = run(
        "model M parameter Real A[2, 2] = [1, 2; 3, 4]; Real mm[2, 2]; \
         equation mm = A * transpose(A); \
         annotation(experiment(StopTime = 1.0, Interval = 0.5)); end M;",
    );
    let csv = result.to_csv();
    assert!(csv.contains("\"mm[1,2]\""), "{csv}");
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    assert_eq!(last[index("mm[1,1]")], 5.0);
    assert_eq!(last[index("mm[1,2]")], 11.0);
    assert_eq!(last[index("mm[2,2]")], 25.0);
}

#[test]
fn the_compile_time_eval_matches_the_runtime_code() {
    // The same builtins exist twice: in `eval` for compile-time
    // folding and in `Code::run` for the solvers. Feeding them
    // through parameter bindings exercises the eval side; the
    // equations above covered the code side.
    let model = parse_model(
        "model M \
         parameter Real a = ceil(2.3) + floor(2.7) + integer(-1.5); \
         parameter Real b = div(-7, 2) + mod(-7, 2) + rem(-7, 2); \
         parameter Real c = abs(-3) + sign(-2) + sqrt(16) + min(1, 2) + max(1, 2); \
         parameter Real d = atan2(1, 1) + log10(100) + sinh(0) + cosh(0) + tanh(0); \
         parameter Real e = asin(1) + acos(1) + atan(0) + tan(0) + exp(0) + log(1); \
         Real x; equation x = a + b + c + d + e; end M;",
    )
    .unwrap();
    let compiled = compile(&model).unwrap();
    let value = |name: &str| {
        compiled
            .parameters
            .iter()
            .find(|(n, _)| n == name)
            .unwrap()
            .1
    };
    assert_eq!(value("a"), 3.0 + 2.0 + (-2.0));
    // div truncates toward zero, mod follows the floor, rem the
    // truncation: -3 + 1 + (-1).
    assert_eq!(value("b"), -3.0);
    assert_eq!(value("c"), 3.0 - 1.0 + 4.0 + 1.0 + 2.0);
    assert!((value("d") - (std::f64::consts::FRAC_PI_4 + 2.0 + 1.0)).abs() < 1e-12);
    assert!((value("e") - (std::f64::consts::FRAC_PI_2 + 1.0)).abs() < 1e-12);
}

#[test]
fn relations_and_logic_fold_at_compile_time() {
    // The comparison and boolean arms of the constant folder.
    let model = parse_model(
        "model M \
         parameter Boolean q = 1 < 2 and 2 <= 2 and 3 > 2 and 2 >= 2 \
           and 1 == 1 and 1 <> 2 or false; \
         parameter Real k = if q and not false then 7 else 9; \
         Real x; equation x = k; end M;",
    )
    .unwrap();
    let compiled = compile(&model).unwrap();
    let k = compiled
        .parameters
        .iter()
        .find(|(n, _)| n == "k")
        .unwrap()
        .1;
    assert_eq!(k, 7.0);
}

#[test]
fn results_carry_the_parameter_values() {
    // Consumers such as the 3D view read sizes and colours from
    // here, since those never appear as columns.
    let result = run(
        "model P parameter Real k = 2.5; parameter Real m = 4; Real y; \
         equation y = k * m * time; \
         annotation(experiment(StopTime=1.0, Interval=0.5)); end P;",
    );
    assert_eq!(
        result.parameters,
        vec![("k".to_string(), 2.5), ("m".to_string(), 4.0)]
    );
    assert!((result.rows.last().unwrap()[1] - 10.0).abs() < 1e-12);
}

#[test]
fn time_is_available_in_equations() {
    let result = run("model T Real y; equation y = 2 * time; \
         annotation(experiment(StopTime=1.0, Interval=0.5)); end T;");
    let last = result.rows.last().unwrap();
    assert!((last[1] - 2.0).abs() < 1e-12);
}
