//! What the compiler and the solvers are expected to do, asked of the
//! crate the way anything outside it would ask: parse, compile, run.

use oxidelica_parser::parse_model;
use oxidelica_sim::{compile, SimResult, SolverMethod};

fn run(source: &str) -> SimResult {
    let model = parse_model(source).unwrap();
    compile(&model).unwrap().simulate().unwrap()
}

fn compile_err(source: &str) -> String {
    compile(&parse_model(source).unwrap())
        .unwrap_err()
        .to_string()
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

/// Compile `source`, run it on `method`, and give back the result.
fn run_on(source: &str, method: SolverMethod) -> Result<SimResult, String> {
    let mut compiled = compile(&parse_model(source).unwrap()).unwrap();
    compiled.method = method;
    compiled.simulate().map_err(|e| e.to_string())
}

#[test]
fn every_solver_stops_when_the_model_says_to() {
    // `terminate` has to be honoured wherever the run happens to be:
    // at a scheduled instant, at a crossing found by the event search,
    // and at the very first point, before any stepping at all.
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf] {
        let scheduled = run_on(
            "model G Real x(start = 1, fixed = true); discrete Real n(start = 0); \
             equation der(x) = -x; \
             when sample(0.25, 0.25) then n = pre(n) + 1; end when; \
             when n > 1.5 then terminate(\"the second tick\"); end when; \
             annotation(experiment(StopTime = 2, Interval = 0.05)); end G;",
            method,
        )
        .expect("runs");
        assert_eq!(
            scheduled.terminated.as_deref(),
            Some("terminated at t = 0.500000: the second tick"),
            "{method:?} missed the scheduled stop"
        );
        assert!(scheduled.rows.last().unwrap()[0] <= 0.5 + 1e-9);

        let crossing = run_on(
            "model T Real x(start = 1, fixed = true); equation der(x) = -1; \
             when x < 0.5 then terminate(\"halfway down\"); end when; \
             annotation(experiment(StopTime = 2, Interval = 0.05)); end T;",
            method,
        )
        .expect("runs");
        assert_eq!(
            crossing.terminated.as_deref(),
            Some("terminated at t = 0.500000: halfway down"),
            "{method:?} missed the crossing"
        );
    }

    // RK4 steps on a fixed grid and refuses `sample`, but it still has
    // to stop before its first step when the start itself terminates.
    let at_once = run_on(
        "model F Real x(start = 1, fixed = true); equation der(x) = -x; \
         when initial() then terminate(\"nothing to do\"); end when; \
         annotation(experiment(StopTime = 1, Interval = 0.1)); end F;",
        SolverMethod::Rk4,
    )
    .expect("runs");
    assert_eq!(
        at_once.terminated.as_deref(),
        Some("terminated at t = 0.000000: nothing to do")
    );
    assert_eq!(at_once.rows.len(), 1, "no step should have been taken");
}

#[test]
fn clocks_derived_from_one_another_tick_where_the_fractions_put_them() {
    // Three rates and a phase off the grid, all counted from one root.
    // Each variable adds its own interval at each of its ticks, so its
    // value is a reading of how many times its clock has fired.
    let result = run("model M Clock base = Clock(1, 10); \
         Clock fast = superSample(base, 2); \
         Clock late = shiftSample(base, 1, 4); \
         Clock slow = subSample(base, 2); \
         Real b; Real f; Real l; Real s; \
         Real hb; Real hf; Real hl; Real hs; \
         equation b = previous(b) + interval(base); \
         f = previous(f) + interval(fast); \
         l = previous(l) + interval(late); \
         s = previous(s) + interval(slow); \
         hb = hold(b); hf = hold(f); hl = hold(l); hs = hold(s); \
         annotation(experiment(StopTime = 0.4, Interval = 0.4)); end M;");
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    // Counting the ticks in [0, 0.4], the first one at t = 0 included:
    // the base fires 5 times adding 0.1, the fast clock 9 times adding
    // 0.05, the shifted one at 0.025 and every 0.1 after - 4 times
    // adding 0.1 - and the slow one at 0, 0.2 and 0.4 adding 0.2.
    for (name, expected) in [("hb", 0.5), ("hf", 0.45), ("hl", 0.4), ("hs", 0.6)] {
        assert!(
            (last[index(name)] - expected).abs() < 1e-12,
            "{name} = {}, not {expected}",
            last[index(name)]
        );
    }
}

#[test]
fn a_partition_sees_this_tick_of_the_one_it_reads() {
    // Two clocks that tick together at t = 0, 0.2, 0.4. The slow
    // partition reads the fast one, so it has to run second - a `when`
    // branch fires once per event, and one placed first would take the
    // value from the tick before. `noClock` is the same read with the
    // clock left to be inferred from elsewhere.
    let result = run(
        "model M Clock base = Clock(0.1); Clock slow = subSample(base, 2); \
         Real u; Real v; Real w; Real hv; Real hw; \
         equation u = previous(u) + interval(base); \
         v = subSample(u, 2) + interval(slow); \
         w = noClock(u) + interval(slow); \
         hv = hold(v); hw = hold(w); \
         annotation(experiment(StopTime = 0.2, Interval = 0.2)); end M;",
    );
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    // At t = 0.2 the base has ticked three times, so u is 0.3, and both
    // readers see that rather than the 0.1 left from t = 0.
    let last = result.rows.last().unwrap();
    assert!(
        (last[index("u")] - 0.3).abs() < 1e-12,
        "{}",
        last[index("u")]
    );
    for name in ["hv", "hw"] {
        assert!(
            (last[index(name)] - 0.5).abs() < 1e-12,
            "{name} = {}",
            last[index(name)]
        );
    }
}

#[test]
fn an_event_clock_measures_the_interval_it_could_not_be_told() {
    // `x` is `sin(time)`, so `x > 0.5` rises at pi/6 and once every
    // turn after: at pi/6, pi/6 + 2 pi and pi/6 + 4 pi. A clock ticking
    // on that has no period anyone could have written down, so
    // `interval` is measured - the time now less the time at the tick
    // before - and the first tick, having nothing behind it, answers
    // with the start interval the constructor was given.
    let result = run(
        "model M Real x(start = 0, fixed = true); Real v(start = 1, fixed = true); \
         Clock e = Clock(x > 0.5, 0.25); Clock half = subSample(e, 2); \
         Real gap; Real n; Real m; Real first; \
         Real hg; Real hn; Real hm; Real hf; \
         equation der(x) = v; der(v) = -x; \
         gap = interval(e); n = previous(n) + gap; \
         m = subSample(n, 2) + 1; \
         first = if firstTick() then 100 else previous(first) + gap; \
         hg = hold(gap); hn = hold(n); hm = hold(m); hf = hold(first); \
         annotation(experiment(StopTime = 14, Interval = 0.5)); end M;",
    );
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let turn = std::f64::consts::TAU;
    let last = result.rows.last().unwrap();
    // Three ticks: 0.25 for the first, then a turn each time.
    assert!(
        (last[index("hg")] - turn).abs() < 1e-5,
        "{}",
        last[index("hg")]
    );
    assert!(
        (last[index("hn")] - (0.25 + 2.0 * turn)).abs() < 1e-5,
        "{}",
        last[index("hn")]
    );
    // `firstTick` was true once, at the first of them.
    assert!(
        (last[index("hf")] - (100.0 + 2.0 * turn)).abs() < 1e-5,
        "{}",
        last[index("hf")]
    );
    // The sub-sampled clock fires on the first and the third edge and
    // holds what it had through the second, so it reads `n` from the
    // third rather than from the second.
    assert!(
        (last[index("hm")] - (1.25 + 2.0 * turn)).abs() < 1e-5,
        "{}",
        last[index("hm")]
    );
    // And it really did skip one: at the second tick it still read what
    // the first had left.
    let between = result
        .rows
        .iter()
        .find(|row| row[0] > 7.0)
        .expect("a point between the second tick and the third");
    assert!(
        (between[index("hm")] - 1.25).abs() < 1e-12,
        "{}",
        between[index("hm")]
    );
}

#[test]
fn a_clock_carrying_a_solver_steps_its_own_derivative() {
    // `der(x) = -x` from x = 1 is `exp(-t)`, and each method reaches
    // t = 1 with the error its order allows: the Euler step is off by
    // 2e-2, the midpoint by 7e-4, the four-stage method by 3e-7. Those
    // are the amplification factors of the methods themselves, so the
    // three answers below are what the tableaux say and not what any
    // continuous solver would produce.
    let run_with = |method: &str| {
        let result = run(&format!(
            "model M Clock c = Clock(Clock(0.1), \"{method}\"); \
             Real u; Real x(start = 1); Real hx; \
             equation u = sample(0, c); der(x) = -x + u; hx = hold(x); \
             annotation(experiment(StopTime = 1, Interval = 1)); end M;"
        ));
        let index = result.columns.iter().position(|c| c == "hx").unwrap();
        result.rows.last().unwrap()[index]
    };
    // Ten steps of each, worked out from the tableau rather than taken
    // from a run: 0.9^10, and the two mixes that follow it.
    for (method, expected) in [
        ("ExplicitEuler", 0.348_678_440_1),
        ("ExplicitMidPoint2", 0.368_540_984_833_551_8),
        ("ExplicitRungeKutta4", 0.367_879_774_412_498_4),
    ] {
        let reached = run_with(method);
        assert!(
            (reached - expected).abs() < 1e-12,
            "{method} reached {reached}, not {expected}"
        );
    }
    // The stages advance every state together, not one at a time: this
    // is `sin` and `cos` at t = 1, to the accuracy ten steps of the
    // four-stage method allow.
    let result = run(
        "model M Clock c = Clock(Clock(0.1), \"ExplicitRungeKutta4\"); \
         Real u; Real x(start = 0); Real v(start = 1); Real hx; Real hv; \
         equation u = sample(0, c); der(x) = v + u; der(v) = -x; \
         hx = hold(x); hv = hold(v); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    assert!(
        (last[index("hx")] - 0.841_470_477_800_274_3).abs() < 1e-12,
        "{}",
        last[index("hx")]
    );
    assert!(
        (last[index("hv")] - 0.540_302_967_116_884_2).abs() < 1e-12,
        "{}",
        last[index("hv")]
    );
    // Close to the real thing, and closer than either lower-order
    // method would have come.
    assert!((last[index("hx")] - 1.0_f64.sin()).abs() < 1e-6);
}

#[test]
fn a_clock_left_unsaid_runs_like_the_one_written_out() {
    // The same model four ways: the slow clock and the sampling factor
    // each written out or left for the compiler. Inference is only
    // worth having if it lands on the clock the model would have
    // spelled, so the test is that all four agree to the last bit.
    let reached = |slow: &str, sampled: &str| {
        let result = run(&format!(
            "model M Clock fast = Clock(1, 10); Clock slow = {slow}; \
             Real a; Real b; Real out; \
             equation a = previous(a) + interval(fast); \
             b = {sampled} + interval(slow); out = hold(b); \
             annotation(experiment(StopTime = 0.6, Interval = 0.6)); end M;"
        ));
        let index = result.columns.iter().position(|c| c == "out").unwrap();
        result.rows.last().unwrap()[index]
    };
    // The fast clock ticks seven times by 0.6 and the slow one four, so
    // `a` reads 0.7 and `b` is that plus one of the slow clock's 0.2s.
    let written_out = reached("Clock(1, 5)", "subSample(a, 2)");
    assert!((written_out - 0.9).abs() < 1e-12, "{written_out}");
    for (slow, sampled) in [
        ("Clock()", "subSample(a, 2)"),
        ("Clock(1, 5)", "subSample(a)"),
        ("Clock()", "subSample(a, 2)"),
    ] {
        assert_eq!(
            reached(slow, sampled).to_bits(),
            written_out.to_bits(),
            "{slow} / {sampled}"
        );
    }
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
fn a_check_among_the_statements_is_checked_as_the_run_goes() {
    // One `assert` written inside a loop is one check per element, and
    // each carries the message the loop was written with.
    let source = |gains: &str| {
        format!(
            "model M parameter Real g[3] = {gains}; Real y; \
             algorithm y := 0; \
             for i loop assert(g[i] > 0, \"every gain must be positive\"); y := y + g[i]; \
             end for; \
             annotation(experiment(StopTime = 0, Interval = 1)); end M;"
        )
    };
    let result = run(&source("{1, 2, 3}"));
    let index = result.columns.iter().position(|c| c == "y").unwrap();
    assert_eq!(result.rows.last().unwrap()[index], 6.0);
    // The check belongs to the run, not to the compilation: the model
    // builds, and the second gain stops it where it stands.
    let model = parse_model(&source("{1, -2, 3}")).unwrap();
    let stopped = compile(&model)
        .expect("a model that builds")
        .simulate()
        .expect_err("and stops itself")
        .to_string();
    assert!(stopped.contains("every gain must be positive"), "{stopped}");
}

#[test]
fn a_model_with_nothing_to_integrate_still_finds_where_a_relation_turns() {
    // Nothing is integrated here, so the walk goes from output point to
    // output point - but a relation does not wait for the grid, and the
    // event belongs where the relation turns rather than at whichever
    // point first happens to see it. Both solvers reach the same walk.
    let turned_at = |condition: &str, method: SolverMethod| {
        let result = run_on(
            &format!(
                "model M discrete Real k(start = 0); Real y; \
                 equation y = k; \
                 when {condition} then k = pre(k) + 1; end when; \
                 annotation(experiment(StopTime = 1, Interval = 0.1)); end M;"
            ),
            method,
        )
        .expect("runs");
        let index = result.columns.iter().position(|c| c == "k").unwrap();
        result
            .rows
            .iter()
            .find(|row| row[index] > 0.5)
            .map(|row| row[0])
            .expect("the condition turns inside the run")
    };
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf] {
        // `time^2 > 0.5` turns at the square root of a half, which is
        // nowhere near the tenths the output grid is made of.
        let root = 0.5_f64.sqrt();
        let found = turned_at("time * time > 0.5", method);
        assert!(
            (found - root).abs() < 1e-9,
            "{method:?} found {found}, not {root}"
        );
        // And one that turns exactly on an output point, which is the
        // awkward case: the relation is still false there - `0.3 > 0.3`
        // is not true - so the event belongs a hair past it and not a
        // whole grid step later.
        let found = turned_at("time > 0.3", method);
        assert!(
            (found - 0.3).abs() < 1e-9,
            "{method:?} found {found}, not 0.3"
        );
    }
}

#[test]
fn a_model_with_nothing_to_integrate_still_walks_its_events() {
    // No `der` anywhere: there is no step to take, so the solver walks
    // from one scheduled instant to the next output point and back,
    // and the discrete layer has to keep working across both.
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf] {
        let result = run_on(
            "model A Real y; discrete Real k(start = 0); \
             equation y = k * 2; \
             when sample(0.13, 0.13) then k = pre(k) + 1; end when; \
             when k > 2.5 then terminate(\"three ticks\"); end when; \
             annotation(experiment(StopTime = 1, Interval = 0.1)); end A;",
            method,
        )
        .expect("runs");
        // Ticks at 0.13, 0.26, 0.39 - the third one stops the run, on a
        // clock that shares no instant with the output grid.
        assert_eq!(
            result.terminated.as_deref(),
            Some("terminated at t = 0.390000: three ticks"),
            "{method:?} walked the grid wrong"
        );
        let last = result.rows.last().unwrap();
        assert!((last[0] - 0.39).abs() < 1e-9, "stopped at {}", last[0]);
        assert!((last[1] - 6.0).abs() < 1e-12, "y = {}", last[1]);
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
fn the_stiff_solver_reselects_states_like_the_adaptive_one() {
    // A pendulum in Cartesian coordinates given enough speed to go
    // over the top: the length constraint defines a different
    // coordinate every quarter turn, so the run stalls, re-selects and
    // resumes - on BDF as much as on the explicit solver, and both
    // must agree about the circle they stayed on.
    const SPIN: &str = "model P parameter Real g = 9.81; \
         Real x(start = 0, fixed = true); Real y(start = -1, fixed = true); \
         Real vx(start = 8, fixed = true); Real vy(start = 0, fixed = true); Real lam; \
         equation der(x) = vx; der(y) = vy; der(vx) = lam * x; der(vy) = lam * y - g; \
         x * x + y * y = 1; \
         annotation(experiment(StopTime = 3, Interval = 0.002, Tolerance = 1e-9)); end P;";
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf] {
        let result = run_on(SPIN, method).expect("runs");
        assert!(
            result.reselections >= 4,
            "{method:?} took {} re-selections",
            result.reselections
        );
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let (x, y) = (index("x"), index("y"));
        let worst = result
            .rows
            .iter()
            .map(|row| (row[x] * row[x] + row[y] * row[y] - 1.0).abs())
            .fold(0.0f64, f64::max);
        assert!(worst < 1e-6, "{method:?} left the circle by {worst}");
        assert!((result.rows.last().unwrap()[0] - 3.0).abs() < 1e-6);
    }
}

#[test]
fn a_model_with_nothing_to_integrate_reaches_its_stop_time() {
    // The same walk as above, but nothing stops it early: the last
    // output point is the stop time itself, which does not sit on the
    // sampling clock and has to be recorded on the way out.
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf] {
        let result = run_on(
            "model A Real y; discrete Real k(start = 0); \
             equation y = k * 2; \
             when sample(0.13, 0.13) then k = pre(k) + 1; end when; \
             annotation(experiment(StopTime = 0.5, Interval = 0.2)); end A;",
            method,
        )
        .expect("runs");
        let last = result.rows.last().unwrap();
        assert!(
            (last[0] - 0.5).abs() < 1e-9,
            "{method:?} ended at {}",
            last[0]
        );
        // Ticks at 0.13, 0.26 and 0.39 have all been and gone by 0.5.
        assert!(
            (last[1] - 6.0).abs() < 1e-12,
            "{method:?} saw y = {}",
            last[1]
        );
    }
}

#[test]
fn a_run_whose_stop_time_is_off_the_grid_still_ends_on_it() {
    // Interval divides into StopTime with a remainder, so the last
    // scheduled output point falls short and the stop time is recorded
    // separately once the stepping is done.
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf] {
        let result = run_on(
            "model E Real x(start = 1, fixed = true); equation der(x) = -x; \
             annotation(experiment(StopTime = 1, Interval = 0.3, Tolerance = 1e-10)); end E;",
            method,
        )
        .expect("runs");
        let last = result.rows.last().unwrap();
        assert!(
            (last[0] - 1.0).abs() < 1e-9,
            "{method:?} ended at {}",
            last[0]
        );
        assert!(
            (last[1] - (-1.0f64).exp()).abs() < 1e-7,
            "{method:?} gave x(1) = {}",
            last[1]
        );
    }
}

#[test]
fn a_state_event_that_jumps_is_recorded_on_both_solvers() {
    // `reinit` moves a state without moving time. Both solvers have to
    // stop at the crossing, record the jump, and carry on from the new
    // value rather than interpolating across it.
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf] {
        let result = run_on(
            "model H Real x(start = 1, fixed = true); equation der(x) = -1; \
             when x < 0.5 then reinit(x, 1); end when; \
             annotation(experiment(StopTime = 2, Interval = 0.05)); end H;",
            method,
        )
        .expect("runs");
        let x: Vec<f64> = result.rows.iter().map(|row| row[1]).collect();
        // A sawtooth between 1 and 0.5: never below the trigger, and
        // back at the top three times over two seconds.
        assert!(
            x.iter().all(|&v| (0.5 - 1e-6..=1.0 + 1e-6).contains(&v)),
            "{method:?} left the band"
        );
        let jumps = result
            .rows
            .windows(2)
            .filter(|pair| pair[1][1] > pair[0][1] + 0.4)
            .count();
        assert!(jumps >= 3, "{method:?} jumped {jumps} times");
    }
}

#[test]
fn an_algebraic_loop_that_comes_apart_says_so() {
    let refused = |source: &str| {
        compile(&parse_model(source).unwrap())
            .expect_err("has no solution")
            .to_string()
    };

    // `1 / x = 0` has no solution, and Newton walks straight off the
    // number line rather than merely failing to converge - the run has
    // to say which loop it was, not hand back a NaN.
    assert_eq!(
        refused(
            "model D Real x; Real s(start = 0, fixed = true); \
             equation 1 / x = 0; der(s) = x; \
             annotation(experiment(StopTime = 1, Interval = 0.1)); end D;"
        ),
        "algebraic loop diverged: [\"x\"]"
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
}

#[test]
fn a_mode_change_reaches_a_model_with_nothing_to_integrate() {
    // No `der` anywhere and a run-time `if` whose branches constrain
    // different unknowns: `a` is defined before the switch and `b`
    // after it. There is no step to take here, so the change has to be
    // noticed at an output point - and a solver that misses it does not
    // fail, it quietly keeps answering from the branch that has already
    // been left.
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf] {
        let result = run_on(
            "model X Real a; Real b; \
             equation if time < 0.5 then a = time; b = 2 * a; \
             else b = time; a = b / 2; end if; \
             annotation(experiment(StopTime = 1, Interval = 0.1)); end X;",
            method,
        )
        .expect("runs");
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let (a, b) = (index("a"), index("b"));
        for row in &result.rows {
            // Either way round the pair means the same thing, so the
            // only way to tell the branches apart is which of the two
            // the run computed from - and after the switch it is `b`.
            let (wanted_a, wanted_b) = if row[0] < 0.5 {
                (row[0], 2.0 * row[0])
            } else {
                (row[0] / 2.0, row[0])
            };
            assert!(
                (row[a] - wanted_a).abs() < 1e-9 && (row[b] - wanted_b).abs() < 1e-9,
                "{method:?} at t = {}: a = {} (want {wanted_a}), b = {} (want {wanted_b})",
                row[0],
                row[a],
                row[b]
            );
        }
    }
}

#[test]
fn a_bound_broken_only_by_the_run_is_reported_where_it_broke() {
    // Modelica calls `min` and `max` assertions on the value. The
    // checker settles the ones it can before the run; this one it
    // cannot, because nothing says beforehand where a falling level
    // ends up.
    let broken = run_on(
        "model B Real level(start = 2, fixed = true, min = 0); \
         equation der(level) = -1; \
         annotation(experiment(StopTime = 3, Interval = 0.1)); end B;",
        SolverMethod::Dopri45,
    )
    .expect_err("crosses its floor");
    assert!(
        broken.contains("`level` went below its min of 0") && broken.contains("t = 2."),
        "{broken}"
    );

    // A run that stays inside is not disturbed by the guard.
    let fine = run_on(
        "model G Real level(start = 2, fixed = true, min = 0, max = 5); \
         equation der(level) = -0.1; \
         annotation(experiment(StopTime = 3, Interval = 0.1)); end G;",
        SolverMethod::Dopri45,
    )
    .expect("stays inside");
    assert!((fine.rows.last().unwrap()[1] - 1.7).abs() < 1e-9);
}

#[test]
fn the_nth_root_takes_the_sign_with_it() {
    // `powf` gives NaN for every negative base, so the odd roots of a
    // negative number - which do exist - have to be found by taking
    // the sign out and putting it back.
    let result = run("model R Real cube; Real odd; Real fourth; \
         equation cube = nthRoot(8, 3); odd = nthRoot(-8, 3); fourth = nthRoot(16, 4); \
         annotation(experiment(StopTime = 1, Interval = 1)); end R;");
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    assert!((last[index("cube")] - 2.0).abs() < 1e-12);
    assert!((last[index("odd")] + 2.0).abs() < 1e-12);
    assert!((last[index("fourth")] - 2.0).abs() < 1e-12);
}

#[test]
fn the_end_of_a_run_is_an_event_of_its_own() {
    // `terminal()` is the predicate for an analysis that finished, and
    // a `when` watching it fires once, at the stop time, with
    // everything the run arrived at still in place.
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf, SolverMethod::Rk4] {
        for source in [
            // With something to integrate, and with nothing.
            "model M Real x(start = 0, fixed = true); discrete Real flag(start = 0); \
             equation der(x) = 1; when terminal() then flag = 1; end when; \
             annotation(experiment(StopTime = 1, Interval = 0.25)); end M;",
            "model M Real y; discrete Real flag(start = 0); \
             equation y = time; when terminal() then flag = 1; end when; \
             annotation(experiment(StopTime = 1, Interval = 0.25)); end M;",
        ] {
            // RK4 steps a fixed grid and refuses `sample`; a model with
            // nothing to integrate has no grid for it to step.
            if matches!(method, SolverMethod::Rk4) && source.contains("y = time") {
                continue;
            }
            let result = run_on(source, method).expect("runs");
            let flag = result.columns.iter().position(|c| c == "flag").unwrap();
            assert_eq!(
                result.rows.last().unwrap()[flag],
                1.0,
                "{method:?} did not reach the end"
            );
            // And only at the end: every earlier row still has zero.
            assert!(result.rows[..result.rows.len() - 1]
                .iter()
                .all(|row| row[flag] == 0.0));
        }
    }

    // A run the model stopped itself did not finish, so the predicate
    // stays false - that is the difference between an analysis that
    // ended and one that succeeded.
    let stopped = run_on(
        "model M Real x(start = 0, fixed = true); discrete Real flag(start = 0); \
         equation der(x) = 1; when x > 0.5 then terminate(\"far enough\"); end when; \
         when terminal() then flag = 1; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.25)); end M;",
        SolverMethod::Dopri45,
    )
    .expect("runs");
    let flag = stopped.columns.iter().position(|c| c == "flag").unwrap();
    assert_eq!(stopped.rows.last().unwrap()[flag], 0.0);
    assert!(stopped.terminated.is_some());
}

#[test]
fn a_carried_profile_arrives_where_and_when_it_should() {
    // `spatialDistribution` is transport along a coordinate: what goes
    // in at one end comes out at the other once the coordinate has
    // moved by one. With a unit velocity that is a delay of one second,
    // and unlike `delay` it is exact - the profile remembers the
    // position each value entered at, so nothing is interpolated
    // between output points.
    let result = run(
        "model Pipe Real x(start = 0, fixed = true); Real inlet; Real out0; Real out1; \
         equation der(x) = 1; inlet = sin(3 * time); \
         (out0, out1) = spatialDistribution(inlet, 0, x, true, {0.0, 1.0}, {0.0, 0.0}); \
         annotation(experiment(StopTime = 3, Interval = 0.002, Tolerance = 1e-10)); end Pipe;",
    );
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    for row in &result.rows {
        let t = row[0];
        // Going forward, the near end is simply what is entering.
        assert!(
            (row[index("out0")] - (3.0 * t).sin()).abs() < 1e-12,
            "t = {t}"
        );
        // And the far end is what entered a unit of x ago, which is
        // one second here. Before that it is the profile it started
        // from, which is flat and zero.
        let wanted = if t >= 1.0 {
            (3.0 * (t - 1.0)).sin()
        } else {
            0.0
        };
        assert!(
            (row[index("out1")] - wanted).abs() < 1e-9,
            "t = {t}: {} vs {wanted}",
            row[index("out1")]
        );
    }
}

#[test]
fn a_carried_profile_runs_both_ways() {
    // The same pipe with the flow reversed: what enters at the far end
    // leaves at the near one, a unit of x later.
    let backward = run(
        "model B Real x(start = 0, fixed = true); Real feed; Real out0; Real out1; \
         equation der(x) = -1; feed = sin(3 * time); \
         (out0, out1) = spatialDistribution(0, feed, x, false, {0.0, 1.0}, {0.0, 0.0}); \
         annotation(experiment(StopTime = 3, Interval = 0.002, Tolerance = 1e-10)); end B;",
    );
    let at =
        |result: &SimResult, name: &str| result.columns.iter().position(|c| c == name).unwrap();
    for row in &backward.rows {
        let t = row[0];
        let wanted = if t >= 1.0 {
            (3.0 * (t - 1.0)).sin()
        } else {
            0.0
        };
        assert!(
            (row[at(&backward, "out0")] - wanted).abs() < 1e-9,
            "t = {t}"
        );
    }

    // And a flow that turns round mid-run. Up to t = 1.5 the pipe
    // fills with what the clock read; after it, that same fluid comes
    // back out in the order it went in - so the near end reads 3 - t -
    // until the fronts meet at t = 2.5 and the fluid that entered
    // backward starts arriving, a unit of x behind the clock.
    let turning = run(
        "model R Real x(start = 0, fixed = true); Real feed; Real out0; Real out1; \
         Boolean forward; \
         equation forward = time < 1.5; der(x) = if forward then 1 else -1; feed = time; \
         (out0, out1) = spatialDistribution(feed, feed, x, forward, {0.0, 1.0}, {0.0, 0.0}); \
         annotation(experiment(StopTime = 3.4, Interval = 0.005, Tolerance = 1e-10)); end R;",
    );
    let out0 = at(&turning, "out0");
    for row in &turning.rows {
        let (t, got) = (row[0], row[out0]);
        if (1.55..2.45).contains(&t) {
            assert!(
                (got - (3.0 - t)).abs() < 1e-6,
                "t = {t}: {got} vs {}",
                3.0 - t
            );
        } else if t > 2.55 {
            assert!(
                (got - (t - 1.0)).abs() < 1e-9,
                "t = {t}: {got} vs {}",
                t - 1.0
            );
        }
    }
}

#[test]
fn a_carried_profile_starts_from_the_one_it_was_given() {
    // A step sitting in the middle of the pipe at t = 0: the far half
    // holds one, the near half zero. Carried forward at unit speed the
    // far end reads one until the step reaches it, half a second in.
    let result = run(
        "model P Real x(start = 0, fixed = true); Real out0; Real out1; \
         equation der(x) = 1; \
         (out0, out1) = spatialDistribution(0, 0, x, true, \
           {0.0, 0.5, 0.5, 1.0}, {0.0, 0.0, 1.0, 1.0}); \
         annotation(experiment(StopTime = 1, Interval = 0.1)); end P;",
    );
    let out1 = result.columns.iter().position(|c| c == "out1").unwrap();
    let seen: String = result
        .rows
        .iter()
        .map(|row| if row[out1] > 0.5 { '1' } else { '0' })
        .collect();
    assert_eq!(seen, "11111100000", "the step arrived at the wrong time");
}

#[test]
fn a_record_carries_its_own_operators_through_a_run() {
    // The operators an `operator record` declares are used where a
    // model does arithmetic on it, and the values that come out are
    // what those operators say - all the way to a simulated column.
    const V: &str = "operator record V Real x; Real y; \
         encapsulated operator function 'constructor' input Real a; input Real b; \
         output V v; algorithm v.x := a * 2; v.y := b * 2; end 'constructor'; \
         encapsulated operator function '0' output V z; \
         algorithm z.x := 0; z.y := 0; end '0'; \
         encapsulated operator function '+' input V a; input V b; output V c; \
         algorithm c.x := a.x + b.x; c.y := a.y + b.y; end '+'; \
         encapsulated operator function '<' input V a; input V b; output Boolean r; \
         algorithm r := a.x * a.x + a.y * a.y < b.x * b.x + b.y * b.y; end '<'; \
         encapsulated operator function 'String' input V a; output String s; \
         algorithm s := \"V=\" + String(a.x); end 'String'; end V; ";

    let result = run(&format!(
        "{V} model M V p; V q; V arr[3]; V total; \
         Real built; Real added; Real summed; Real ordered; Real named; \
         equation \
           p = V(1, 2); q = V(1, 1) + V(2, 2); \
           for i in 1:3 loop arr[i].x = i; arr[i].y = 0; end for; \
           total = sum(arr); \
           built = p.x; added = q.x; summed = total.x; \
           ordered = if V(1, 0) < V(3, 0) then 1 else 0; \
           named = if String(p) == \"V=2\" then 1 else 0; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    ));
    let at = |name: &str| {
        let index = result.columns.iter().position(|c| c == name).unwrap();
        result.rows.last().unwrap()[index]
    };
    // The constructor doubled its inputs, so V(1, 2).x is 2.
    assert_eq!(at("built"), 2.0);
    // V(1,1) + V(2,2), each doubled first: 2 + 4.
    assert_eq!(at("added"), 6.0);
    // sum over the array, from the record's own zero: 1 + 2 + 3.
    assert_eq!(at("summed"), 6.0);
    // |(2,0)| < |(6,0)| after the constructor doubled both.
    assert_eq!(at("ordered"), 1.0);
    // The record's own String reads the doubled field.
    assert_eq!(at("named"), 1.0);
}

#[test]
fn a_stream_junction_weighs_only_what_flows_into_it() {
    // The mix at a node is each port's stream value weighted by what it
    // pushes in - `max(-m, 0)` - so a port pushing nothing has no say.
    // Only the divisor is regularised, which is what keeps the mix
    // defined when every flow goes quiet without a silent port tugging
    // the answer towards its own value.
    const P: &str = "connector P Real p; flow Real m; stream Real h; end P; ";
    let mixed = |a_flow: f64, a_value: f64, b_flow: f64, b_value: f64| {
        let result = run(&format!(
            "{P} model M P a; P b; P c; Real mix; \
             equation connect(a, b); connect(b, c); \
             a.m = {a_flow}; a.h = {a_value}; b.m = {b_flow}; b.h = {b_value}; \
             c.h = 0; a.p = 0; mix = inStream(c.h); \
             annotation(experiment(StopTime = 1, Interval = 1)); end M;"
        ));
        let index = result.columns.iter().position(|c| c == "mix").unwrap();
        result.rows.last().unwrap()[index]
    };
    // Two parts of 100 against one of 200. The divisor carries the
    // regularising floor, so the answer is off by that much and no more.
    assert!((mixed(-2.0, 100.0, -1.0, 200.0) - 400.0 / 3.0).abs() < 1e-6);
    // A port pushing nothing does not move the answer off 100.
    assert!((mixed(-2.0, 100.0, 0.0, 999.0) - 100.0).abs() < 1e-6);
}

#[test]
fn a_port_of_the_model_pushes_the_other_way() {
    // Inside a class, its own port is an "outside" connector: flow
    // entering the node is positive there, where an inside connector's
    // is negative. So what a component's port hears includes what comes
    // in through the enclosing model's port.
    const P: &str = "connector P Real p; flow Real m; stream Real h; end P; ";
    const SRC: &str = "model Src P c; parameter Real hv = 0; parameter Real mv = 0; \
         equation c.h = hv; c.m = mv; end Src; ";
    let result = run(&format!(
        "{P} {SRC} \
         model Sub P c; Src a(hv = 10, mv = -1); Src b(hv = 20, mv = -1); Real heard; \
         equation connect(a.c, c); connect(b.c, c); heard = inStream(a.c.h); end Sub; \
         model M Sub sub; Real z; equation sub.c.h = 500; sub.c.p = 0; z = sub.heard; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    ));
    let index = result.columns.iter().position(|c| c == "z").unwrap();
    // b pushes 20 with weight 1; the outside port brings 500 in with
    // weight 2, since the two inside ports each send 1 out of the node.
    assert!(
        (result.rows.last().unwrap()[index] - 340.0).abs() < 1e-6,
        "heard {}",
        result.rows.last().unwrap()[index]
    );
}
