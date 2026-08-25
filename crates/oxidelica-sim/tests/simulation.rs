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
    // Parameter cycle: neither is missing, they wait on each other.
    assert!(compile_err(
        "model M parameter Real a = b; parameter Real b = a; Real x; equation x = 1; end M;"
    )
    .contains("wait on each other"));
    // A name nothing declares is named outright rather than being
    // called a cycle it is not.
    let missing = compile_err("model M parameter Real a = nowhere; Real x; equation x = 1; end M;");
    assert!(missing.contains("`nowhere`"), "{missing}");
    // der of a parameter.
    assert!(
        compile_err("model M parameter Real p = 1; equation der(p) = 1; end M;")
            .contains("continuous")
    );
    // der on both sides states neither: the equation is solved rather
    // than stepped with, and `y = 1` makes both derivatives nothing.
    assert!(compile(
        &parse_model("model M Real x; Real y; equation der(x) = der(y); y = 1; end M;").unwrap()
    )
    .is_ok());
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

    // der inside an algebraic expression is read where it stands.
    assert!(compile(
        &parse_model("model M Real x; Real y; equation der(x) = 1; y = der(x) + 1; end M;")
            .unwrap()
    )
    .is_ok());
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
fn a_clock_may_be_asked_for_by_the_names_of_its_arguments() {
    // The clocked library writes its operators out with the argument
    // names the specification gives them, so the names have to reach
    // the same clock the positions would. This is the counter of the
    // test below, said the other way round.
    let result = run("model M Clock base = Clock(interval = 0.1); \
         Clock slow = subSample(u = base, factor = 2); \
         Real u; Real hu; \
         equation u = previous(u) + interval(slow); hu = hold(u); \
         annotation(experiment(StopTime = 0.4, Interval = 0.4)); end M;");
    let hold = result.columns.iter().position(|c| c == "hu").unwrap();
    // Ticks at 0, 0.2, 0.4 - the slow clock - each adding its interval.
    assert!((result.rows.last().unwrap()[hold] - 0.6).abs() < 1e-9);
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
fn an_arrow_may_wait_a_tick_and_may_ask_for_a_reset_of_its_own() {
    // Which state the machine is in at each tick, read off the run.
    let states = |source: &str, column: &str| {
        let result = run(source);
        let index = result.columns.iter().position(|c| c == column).unwrap();
        let mut seen = Vec::new();
        let mut out = Vec::new();
        for row in &result.rows {
            if seen.contains(&row[0].to_bits()) {
                continue;
            }
            seen.push(row[0].to_bits());
            out.push(row[index]);
        }
        out
    };
    let machine = |arrow: &str| {
        format!(
            "model M block S Real n(start = 0); equation n = previous(n) + 1; end S; \
             Clock c = Clock(1); S a; S b; Real u; Real lamp; Real hl; \
             equation u = time; initialState(a); \
             transition(a, b, sample(u, c) > 2.5{arrow}); \
             lamp = if activeState(a) then 1 else 2; hl = hold(lamp); \
             annotation(experiment(StopTime = 6, Interval = 1)); end M;"
        )
    };
    // The condition turns at the tick where `u` reaches 3, and the
    // state it names takes over at the next one - that is what 17.3.4
    // calls an immediate arrow, and it is the default.
    assert_eq!(
        states(&machine(""), "hl"),
        vec![1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0]
    );
    // A delayed one keeps the answer for a tick and is taken on what it
    // kept, so everything happens one tick later and nothing else
    // changes.
    assert_eq!(
        states(&machine(", immediate = false"), "hl"),
        vec![1.0, 1.0, 1.0, 1.0, 1.0, 2.0, 2.0]
    );

    // `reset` belongs to the arrow, not to the state it arrives at:
    // `k` is reached from `a` by an arrow that asks and from `b` by one
    // that does not, so its counter starts over the first time and
    // carries on the second.
    let reached = states(
        "model M block S Real n(start = 0); equation n = previous(n) + 1; end S; \
         Clock c = Clock(1); S a; S b; S k; Real hk; \
         equation initialState(a); \
         transition(a, k, a.n >= 1, reset = true); \
         transition(k, b, k.n >= 2, reset = true); \
         transition(b, k, b.n >= 1, reset = false); \
         hk = hold(k.n); \
         annotation(experiment(StopTime = 9, Interval = 1)); end M;",
        "hk",
    );
    assert_eq!(
        reached,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0, 3.0, 3.0],
        "k was reset where the arrow did not ask"
    );
}

#[test]
fn a_declaration_may_ask_not_to_be_written_down() {
    // `HideResult` leaves a variable out of the results and changes
    // nothing else: it is solved for and read like any other, and what
    // reads it still gets the right number.
    let result = run("model M Real y; Real noise annotation(HideResult = true); \
         equation y = noise / 2; noise = 4 * time; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;");
    assert_eq!(result.columns, vec!["time", "y"]);
    let y = result.columns.iter().position(|c| c == "y").unwrap();
    assert_eq!(result.rows.last().unwrap()[y], 2.0);
}

#[test]
fn what_several_states_say_about_one_variable_is_one_definition() {
    // `v` is declared outside the states and written inside them. Each
    // has its say while it is the state in force, and where a state has
    // none - `Mute` writes nothing - the variable keeps what it held.
    let source = |second: &str| {
        format!(
            "model M block Sig outer output Real v; Real n(start = 0); \
             equation n = previous(n) + 1; v = n * 10; end Sig; \
             {second} \
             Clock c = Clock(1); inner Real v; Sig a; Other b; Real hv; \
             equation initialState(a); transition(a, b, a.n >= 2); \
             hv = hold(v); \
             annotation(experiment(StopTime = 5, Interval = 1)); end M;"
        )
    };
    let held = |second: &str| {
        let result = run(&source(second));
        let index = result.columns.iter().position(|c| c == "hv").unwrap();
        let mut seen = Vec::new();
        let mut out = Vec::new();
        for row in &result.rows {
            if seen.contains(&row[0].to_bits()) {
                continue;
            }
            seen.push(row[0].to_bits());
            out.push(row[index]);
        }
        out
    };
    // The second state says `-1` while it is in force.
    assert_eq!(
        held(
            "block Other outer output Real v; Real n(start = 0); \
             equation n = previous(n) + 1; v = -1; end Other;"
        ),
        vec![0.0, 0.0, 10.0, 20.0, -1.0, -1.0]
    );
    // Saying nothing, it leaves the variable at what the first left.
    assert_eq!(
        held("block Other Real n(start = 0); equation n = previous(n) + 1; end Other;"),
        vec![0.0, 0.0, 10.0, 20.0, 20.0, 20.0]
    );
}

#[test]
fn a_state_may_hold_a_machine_of_its_own() {
    // An outer machine of two states, one of which holds a machine of
    // two more. `work.out` says where the inner one is; the outer's
    // return arrow waits for it to have finished.
    let source = |arrow: &str| {
        format!(
            "model M block Leaf Real n(start = 0); equation n = previous(n) + 1; end Leaf; \
             block Inner Leaf p; Leaf q; Real out; \
             equation initialState(p); transition(p, q, p.n >= 2); \
             out = if activeState(p) then 1 else 2; end Inner; \
             block Idle Real n(start = 0); equation n = previous(n) + 1; end Idle; \
             Clock c = Clock(1); Idle rest; Inner work; Real hi; \
             equation initialState(rest); transition(rest, work, rest.n >= 2); \
             transition(work, rest, true{arrow}); \
             hi = hold(work.out); \
             annotation(experiment(StopTime = 12, Interval = 1)); end M;"
        )
    };
    let where_it_sat = |arrow: &str| {
        let result = run(&source(arrow));
        let outer = result.columns.iter().position(|c| c == "$state0").unwrap();
        let mut seen = Vec::new();
        let mut out = String::new();
        for row in &result.rows {
            if seen.contains(&row[0].to_bits()) {
                continue;
            }
            seen.push(row[0].to_bits());
            out.push(if row[outer] == 0.0 { 'r' } else { 'w' });
        }
        out
    };
    // The return arrow's condition is simply `true`, so without waiting
    // the outer machine leaves after one tick, over and over.
    assert_eq!(where_it_sat(""), "rrrrwrrrwrrrw");
    // Waiting, it stays until the machine inside has reached a state no
    // arrow leaves - four ticks rather than one.
    assert_eq!(where_it_sat(", synchronize = true"), "rrrrwwwwrrrww");

    // And the machine inside holds still while the state holding it is
    // not the one in force: before the first arrival it is nowhere at
    // all, and after the outer leaves it keeps where it got to.
    let result = run(&source(", synchronize = true"));
    let inner = result.columns.iter().position(|c| c == "$state1").unwrap();
    let at = |t: f64| {
        result
            .rows
            .iter()
            .find(|row| row[0] == t)
            .map(|row| row[inner])
            .unwrap()
    };
    assert_eq!(
        at(0.0),
        -1.0,
        "nowhere before the state holding it is in force"
    );
    assert_eq!(at(12.0), 1.0, "kept where it got to after the outer left");
}

#[test]
fn a_body_nothing_could_inline_is_walked_by_the_run() {
    // Two things inlining cannot do. A function that leads back to
    // itself has no bottom to unroll to where what decides the
    // recursion comes from the run - `fact(5)` written out would be
    // unrolled here, so the count is what the run holds. `5!` is the
    // plainest example there is, and it comes out at 120.
    let factorial = run("model M function fact input Real n; output Real y; \
         algorithm if n <= 1 then y := 1; else y := n * fact(n - 1); end if; end fact; \
         Real n; Real y; equation n = 5 * time; y = fact(n); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;");
    let index =
        |result: &SimResult, name: &str| result.columns.iter().position(|c| c == name).unwrap();
    assert_eq!(
        factorial.rows.last().unwrap()[index(&factorial, "y")],
        120.0
    );

    // And a loop whose trip count the model decides rather than the
    // compiler: `u` counts up with time, and the body counts with it.
    let counted = run("model M function count input Real x; output Real y; \
         algorithm y := 0; while y < x loop y := y + 1; end while; end count; \
         Real u; Real y; equation u = time * 3; y = count(u); \
         annotation(experiment(StopTime = 2, Interval = 1)); end M;");
    let y = index(&counted, "y");
    // At t = 0 nothing to count to, at 1 up to 3, at 2 up to 6.
    assert_eq!(counted.rows[0][y], 0.0);
    assert_eq!(counted.rows.last().unwrap()[y], 6.0);

    // Everything a walked body may hold, in one body: a loop whose
    // range the model decides, a `break` out of it, a set written out,
    // a check, an early `return`, and the call that made it walked at
    // all. Worked out by hand: 0, 31, 64, 100, 136, 172.
    let broad = run(
        "model M function walkme input Real n; output Real y; protected Real acc; \
         algorithm acc := 0; \
         for i in 1:n loop if i > 3 then break; end if; acc := acc + i; end for; \
         for j in {10, 20} loop acc := acc + j; end for; \
         assert(acc > 0, \"the sum must be positive\"); \
         if n <= 0 then y := 0; return; end if; \
         y := acc + walkme(n - 1); end walkme; \
         Real n; Real y; equation n = 5 * time; y = walkme(n); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert_eq!(broad.rows.last().unwrap()[index(&broad, "y")], 172.0);

    let runaway = |source: &str| {
        compile(&parse_model(source).unwrap())
            .expect("builds")
            .simulate()
            .expect_err("does not end")
            .to_string()
    };

    // A walk that will not end is stopped and told about, rather than
    // left to run the stack or the clock out.
    assert!(runaway(
        "model M function loops input Real a; output Real b; algorithm b := loops(a); end loops; \
         Real y; equation y = loops(1); \
         annotation(experiment(StopTime = 0, Interval = 1)); end M;"
    )
    .contains("called itself 64 deep"));
    // A range with a step, worked out where it stands: 1 + 3 + 5 twice
    // over, once for each round of the recursion.
    let stepped = run(
        "model M function f input Real a; output Real b; protected Real acc; \
         algorithm acc := 0; for i in 1:2:5 loop acc := acc + i; end for; b := acc; \
         if a > 0 then b := b + f(a - 1); end if; end f; \
         Real n; Real y; equation n = time; y = f(n); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert_eq!(stepped.rows.last().unwrap()[index(&stepped, "y")], 18.0);

    // What a `for` in a walked body cannot run over: a range the body
    // was meant to read off an array, and a single value.
    let walked = |body: &str| {
        runaway(&format!(
            "model M function f input Real a; output Real b; protected Real acc; \
             algorithm acc := 0; {body} \
             if a > 0 then b := b + f(a - 1); end if; end f; \
             Real n; Real y; equation n = time; y = f(n); \
             annotation(experiment(StopTime = 0, Interval = 1)); end M;"
        ))
    };
    assert!(walked("for i loop acc := acc + i; end for; b := acc;")
        .contains("a walked body holds no arrays"));
    assert!(
        walked("for i in acc loop acc := acc + i; end for; b := acc;")
            .contains("runs over a range or a set written out")
    );
    // And a call given more than the body takes.
    assert!(runaway(
        "model M function f input Real a; output Real b; \
         algorithm b := a; if a > 0 then b := f(a - 1); end if; end f; \
         Real y; equation y = f(1, 2); \
         annotation(experiment(StopTime = 0, Interval = 1)); end M;"
    )
    .contains("takes 1 argument(s), given 2"));

    // A `when` has no meaning inside a call: there is no event there,
    // and the walk says so where it meets one.
    assert!(runaway(
        "model M function w input Real a; output Real b; \
         algorithm when a > 0 then b := 1; end when; b := b + w(a - 1); end w; \
         Real y; equation y = w(1); \
         annotation(experiment(StopTime = 0, Interval = 1)); end M;"
    )
    .contains("no event inside a call"));
    assert!(runaway(
        "model M function away input Real a; output Real b; \
         algorithm b := 0; while b < a loop b := b - 1; end while; end away; \
         Real u; Real y; equation u = time + 1; y = away(u); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    )
    .contains("without its condition turning false"));
}

#[test]
fn a_supplied_derivative_carries_a_model_the_compiler_could_not() {
    // `f(x) = |x| * 2`, which the differentiator cannot take apart. The
    // model needs it differentiated twice over: once to reduce the
    // index of the constraint, once for the Jacobian.
    let with_rule = "function f input Real x; output Real y; \
         algorithm y := abs(x) * 2; annotation(derivative = fd); end f; \
         function fd input Real x; input Real x_der; output Real y_der; \
         algorithm y_der := (if x >= 0 then 2 else -2) * x_der; end fd; ";

    // `|x| * 2 = 4 + t` with x positive is `x = 2 + t/2`, so the run
    // must reach 2.5 with a rate of a half - which is the answer the
    // same model gives with `abs` taken out.
    let result = run(&format!(
        "model M {with_rule} Real x(start = 2, fixed = true); Real v; \
         equation der(x) = v; f(x) = 4 + time; \
         annotation(experiment(StopTime = 1, Interval = 0.25)); end M;"
    ));
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    assert!(
        (last[index("x")] - 2.5).abs() < 1e-9,
        "{}",
        last[index("x")]
    );
    assert!(
        (last[index("v")] - 0.5).abs() < 1e-9,
        "{}",
        last[index("v")]
    );

    // The same call among the statements of a model, where what an
    // algorithm has assigned so far is substituted into it.
    let assigned = run(&format!(
        "model M {with_rule} Real x(start = 2, fixed = true); Real v; Real w; \
         algorithm w := f(x) + 1; \
         equation der(x) = v; f(x) = 4 + time; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;"
    ));
    let w = assigned.columns.iter().position(|c| c == "w").unwrap();
    let reached = assigned.rows.last().unwrap()[w];
    assert!((reached - 6.0).abs() < 1e-6, "{reached}");

    // Without the rule the same model is refused, which is what makes
    // the annotation worth having: the constraint has to be
    // differentiated to reduce the index, and `abs` is not something
    // the differentiator can take apart.
    let refused = compile_err(
        "model M function f input Real x; output Real y; algorithm y := abs(x) * 2; end f; \
         Real x(start = 2, fixed = true); Real v; \
         equation der(x) = v; f(x) = 4 + time; \
         annotation(experiment(StopTime = 1, Interval = 0.25)); end M;",
    );
    assert!(refused.contains("structurally singular"), "{refused}");

    // A rule of any shape survives the road to the differentiator, and
    // the call may stand anywhere an expression may - in a parameter
    // worked out before the run, in an equation, in a `when`.
    let all_over = run("model M function g input Real x; output Real y; \
         algorithm y := abs(x) + sqrt(x * x + 1); annotation(derivative = gd); end g; \
         function gd input Real x; input Real x_der; output Real y_der; \
         algorithm y_der := (if x >= 0 and not (x < 0) or false then 1 else -1) * x_der \
         + x / sqrt(x * x + 1) * x_der; end gd; \
         parameter Real p = g(2); Real x(start = 1, fixed = true); Real w; \
         discrete Real k(start = 0); \
         equation der(x) = 1; w = g(x) + p; \
         when time > 0.4 then k = g(x); end when; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;");
    let at = |name: &str| all_over.columns.iter().position(|c| c == name).unwrap();
    let g = |x: f64| x.abs() + (x * x + 1.0).sqrt();
    let end = all_over.rows.last().unwrap();
    assert!(
        (end[at("w")] - (g(2.0) + g(2.0))).abs() < 1e-9,
        "{}",
        end[at("w")]
    );
    // The `when` fired where the condition turned, so `k` is `g` of
    // where `x` was then rather than of where it ended.
    assert!((end[at("k")] - g(1.4)).abs() < 1e-6, "{}", end[at("k")]);

    // And the same rule serves the Jacobian: `der(x) = -f(x)/4` is
    // `der(x) = -x/2` for positive x, whose answer at t = 1 is
    // `exp(-1/2)`. The implicit solver is the one that needs it.
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf] {
        let decayed = run_on(
            &format!(
                "model M {with_rule} Real x(start = 1, fixed = true); \
                 equation der(x) = -f(x) / 4; \
                 annotation(experiment(StopTime = 1, Interval = 0.5)); end M;"
            ),
            method,
        )
        .expect("runs");
        let x = decayed.columns.iter().position(|c| c == "x").unwrap();
        let reached = decayed.rows.last().unwrap()[x];
        assert!(
            (reached - (-0.5f64).exp()).abs() < 1e-6,
            "{method:?} reached {reached}"
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

    // The same, for a model that does have something to integrate: the
    // stepping solvers read the relation off the step they interpolate
    // rather than by asking, and the awkward case is the same one.
    let stepped = |condition: &str, method: SolverMethod| {
        let result = run_on(
            &format!(
                "model M Real x(start = 1, fixed = true); discrete Real k(start = 0); \
                 equation der(x) = 1; \
                 when {condition} then k = x; end when; \
                 annotation(experiment(StopTime = 1, Interval = 0.5)); end M;"
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
        // `time > 0.5` turns exactly on an output point, where the
        // relation is still false and its indicator exactly zero.
        let found = stepped("time > 0.5", method);
        assert!(
            (found - 0.5).abs() < 1e-6,
            "{method:?} found {found}, not 0.5"
        );
        // And one that turns between two of them.
        let found = stepped("time > 0.7", method);
        assert!(
            (found - 0.7).abs() < 1e-6,
            "{method:?} found {found}, not 0.7"
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

/// An `initial algorithm` settles where the run starts, and a check
/// written inside an `if` branch fires only while that branch holds.
#[test]
fn an_initial_algorithm_and_a_guarded_check_run() {
    // `x` starts where the initial algorithm put it, gain * 2 = 6, and
    // rises at 1 a second: x(2) = 8.
    let result = run("model M parameter Real gain = 3; Real x; \
         initial algorithm x := gain * 2; \
         equation der(x) = 1; \
         annotation(experiment(StopTime = 2, Interval = 0.1, Tolerance = 1e-10)); end M;");
    let last = result.rows.last().unwrap();
    assert!((last[0] - 2.0).abs() < 1e-12);
    assert!((last[1] - 8.0).abs() < 1e-9, "x(2) = {}", last[1]);

    // The check of the second branch would fail at every moment the
    // first branch holds; guarded by the denial of the first
    // condition, it says nothing there and the run gets through.
    let result = run("model M Real y; equation \
         if time > 1 then assert(time > 0.5, \"late\"); y = 1; \
         else assert(time < 1.5, \"early\"); y = 2; end if; \
         annotation(experiment(StopTime = 2, Interval = 0.1, Tolerance = 1e-10)); end M;");
    assert!((result.rows.last().unwrap()[0] - 2.0).abs() < 1e-12);
}

/// A library states a relation the way physics does, `i = C * der(v)`,
/// and the solver needs the derivative on its own. The two are the
/// same equation, and getting from one to the other is undoing what
/// stands between them.
#[test]
fn a_derivative_is_got_out_of_the_equation_that_states_it() {
    // `i = C * der(v)` with `i` fixed: v rises at i/C = 2 a second, so
    // v(3) = 6.
    let result = run(
        "model M parameter Real c = 0.5; Real v(start = 0, fixed = true); Real i; \
         equation i = 1; i = c * der(v); \
         annotation(experiment(StopTime = 3, Interval = 0.1, Tolerance = 1e-10)); end M;",
    );
    let v = result.columns.iter().position(|n| n == "v").unwrap();
    assert!(
        (result.rows.last().unwrap()[v] - 6.0).abs() < 1e-9,
        "v(3) = {}",
        result.rows.last().unwrap()[v]
    );

    // Each way of writing it comes to the same thing. Every one of
    // these says the derivative is 2, and every one is a different
    // operation to undo: a sum, a difference either way round, a
    // product, a quotient either way round, and a negation.
    for equation in [
        "der(x) + 1 = 3",
        "1 + der(x) = 3",
        "der(x) - 1 = 1",
        "4 - der(x) = 2",
        "2 * der(x) = 4",
        "der(x) * 2 = 4",
        "der(x) / 2 = 1",
        "8 / der(x) = 4",
        "-der(x) = -2",
        "2 * (der(x) + 1) = 6",
    ] {
        let result = run(&format!(
            "model M Real x(start = 0, fixed = true); equation {equation}; \
             annotation(experiment(StopTime = 1, Interval = 0.5, Tolerance = 1e-10)); end M;"
        ));
        let last = result.rows.last().unwrap()[1];
        assert!((last - 2.0).abs() < 1e-9, "{equation} gave x(1) = {last}");
    }

    // A derivative another equation already defines is read where it
    // stands: `y = der(x) + 1` says what `y` is, and the derivative it
    // reads is the one the other equation stated.
    let result = run("model M Real x(start = 0, fixed = true); Real y; \
         equation der(x) = 3; y = der(x) + 1; \
         annotation(experiment(StopTime = 1, Interval = 1, Tolerance = 1e-10)); end M;");
    let y = result.columns.iter().position(|n| n == "y").unwrap();
    let last = result.rows.last().unwrap()[y];
    assert!((last - 4.0).abs() < 1e-9, "y = {last}");

    // The derivative it did not name is not one of the model's
    // variables: it was solved for, and it is not written out.
    assert!(
        !result.columns.iter().any(|name| name.starts_with("der(")),
        "{:?}",
        result.columns
    );

    // A shape no rearrangement gets a derivative out of is not
    // refused: the derivative is solved for like any other unknown.
    // Two of them stated together, where each equation says something
    // about both.
    let result = run(
        "model M Real x(start = 1, fixed = true); Real y(start = 0, fixed = true); \
         equation der(x) + der(y) = -x + 1; der(x) - der(y) = -x - 1; \
         annotation(experiment(StopTime = 1, Interval = 1, Tolerance = 1e-10)); end M;",
    );
    // Which is `der(x) = -x` and `der(y) = 1`, so `x = e^-t`, `y = t`.
    let last = result.rows.last().unwrap();
    let at = |name: &str| last[result.columns.iter().position(|n| n == name).unwrap()];
    assert!(
        (at("x") - (-1.0f64).exp()).abs() < 1e-8,
        "x(1) = {}",
        at("x")
    );
    assert!((at("y") - 1.0).abs() < 1e-8, "y(1) = {}", at("y"));

    // One inside a call has no operation to undo, and is solved for
    // instead: `abs(der(x)) = 2` from a positive guess is `der(x) = 2`.
    let result = run(
        "model M Real x(start = 1, fixed = true); equation abs(der(x)) = 2; \
         annotation(experiment(StopTime = 1, Interval = 1, Tolerance = 1e-10)); end M;",
    );
    let last = result.rows.last().unwrap()[1];
    assert!((last - 3.0).abs() < 1e-8, "x(1) = {last}");

    // One further down than the isolation looks is solved for the same
    // way, and comes to the same answer.
    let deep = "der(x) + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1";
    let result = run(&format!(
        "model M Real x(start = 0, fixed = true); equation {deep} = 11; \
         annotation(experiment(StopTime = 1, Interval = 1, Tolerance = 1e-10)); end M;"
    ));
    let last = result.rows.last().unwrap()[1];
    assert!((last - 2.0).abs() < 1e-9, "x(1) = {last}");

    // What a solved-for derivative may not be is undetermined. A
    // square has two roots, and which one a model meant is not a thing
    // to guess at.
    let said = refused(
        "model M Real x(start = 1, fixed = true); equation der(x)^2 = 4; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert!(said.contains("der(x)"), "{said}");
    // A derivative got out of an equation still has to be one of a
    // variable that moves.
    let said = refused("model M parameter Real p = 1; equation 2 * der(p) = 4; end M;");
    assert!(said.contains("not a continuous variable"), "{said}");
}

/// A call standing on its own inside a body the run walks: nothing
/// takes its outputs, so what it is there for is the checks it makes.
#[test]
fn a_call_on_its_own_is_walked_for_its_checks() {
    // `counted` cannot be unrolled - how many rounds it runs is the
    // model's to decide - so it stays a call and the run walks it. The
    // walk meets `guard(n);`, which nothing receives, and carries out
    // the check inside it.
    let source = "model M \
         function guard input Real u; output Real ok; \
         algorithm assert(u > -1, \"not too small\"); ok := u; end guard; \
         function counted input Real n; output Real y; \
         algorithm y := 0; guard(n); while y < n loop y := y + 1; end while; end counted; \
         Real y; equation y = counted(3 * time); \
         annotation(experiment(StopTime = 1, Interval = 0.5, Tolerance = 1e-10)); end M;";
    let result = run(source);
    let last = result.rows.last().unwrap();
    // At t = 1 the loop counts to 3.
    assert!((last[1] - 3.0).abs() < 1e-12, "y(1) = {}", last[1]);

    // The same check, failing: the walk stops the run and says what
    // the body said.
    let failing = source.replace("u > -1", "u < -1");
    let model = parse_model(&failing).unwrap();
    let error = compile(&model).unwrap().simulate().unwrap_err().to_string();
    assert!(error.contains("not too small"), "{error}");
}

/// `der(x)` written in an initial equation is the right-hand side the
/// model gives that state, wherever in the expression it stands.
#[test]
fn an_initial_equation_may_hold_a_derivative_anywhere_in_it() {
    // The condition is written the long way round on purpose: the
    // derivative has to be found under a call, a relation, a negation,
    // an `and`, an `or` and a choice, and it means the same in each.
    let result = run("model M Real x; Real y; \
         equation der(x) = 3 - x; y = x; \
         initial equation \
         0 = if abs(der(x)) > 100 or not (der(x) < 0) and true then der(x) else -der(x); \
         annotation(experiment(StopTime = 1, Interval = 0.5, Tolerance = 1e-10)); end M;");
    // A start where nothing moves: der(x) = 0 puts x at 3.
    let first = result.rows.first().unwrap();
    assert!((first[1] - 3.0).abs() < 1e-9, "x(0) = {}", first[1]);
    let last = result.rows.last().unwrap();
    assert!((last[1] - 3.0).abs() < 1e-9, "x(1) = {}", last[1]);
}

/// A bound is a check on the value, and the message names the bound
/// the way the model wrote it.
#[test]
fn a_bound_is_named_the_way_the_model_wrote_it() {
    // A bound written as a negative number, and one written as a
    // parameter: each is said back as it stands rather than as a
    // number the model never wrote.
    let error = run_err(
        "model M parameter Real ceiling = 1; Real x(start = 0, fixed = true, min = -0.5); \
         equation der(x) = -1; \
         annotation(experiment(StopTime = 2, Interval = 0.1, Tolerance = 1e-10)); end M;",
    );
    assert!(error.contains("-0.5"), "{error}");
    let error = run_err(
        "model M parameter Real ceiling = 1; Real x(start = 0, fixed = true, max = ceiling); \
         equation der(x) = 1; \
         annotation(experiment(StopTime = 2, Interval = 0.1, Tolerance = 1e-10)); end M;",
    );
    assert!(error.contains("ceiling"), "{error}");
}

/// Compile a model, run it, and give back what stopped it.
fn run_err(source: &str) -> String {
    let model = parse_model(source).expect("parses");
    compile(&model)
        .expect("compiles")
        .simulate()
        .expect_err("should have been stopped")
        .to_string()
}

/// A connector that is one value rather than a set of members, which
/// is how every signal in a block library is carried.
#[test]
fn a_connector_may_be_one_value() {
    // `connect(src.y, gain.u)` joins the values themselves: there is
    // no member to name on either side.
    let result = run(
        "connector RealInput = input Real; connector RealOutput = output Real; \
         block Source RealOutput y; equation y = 2 * time; end Source; \
         block Gain parameter Real k = 3; RealInput u; RealOutput y; \
         equation y = k * u; end Gain; \
         model M Source src; Gain gain(k = 5); Real out; \
         equation connect(src.y, gain.u); out = gain.y; \
         annotation(experiment(StopTime = 1, Interval = 0.5, Tolerance = 1e-10)); end M;",
    );
    let at = |name: &str| {
        let index = result.columns.iter().position(|c| c == name).unwrap();
        result.rows.last().unwrap()[index]
    };
    assert_eq!(at("src.y"), 2.0);
    // The connection carried it across.
    assert_eq!(at("gain.u"), 2.0);
    assert_eq!(at("out"), 10.0);

    // Three of them in one set all take the same value.
    let result = run("connector Signal = input Real; \
         block Source Signal y; equation y = time; end Source; \
         model M Source src; Signal a; Signal b; Real out; \
         equation connect(src.y, a); connect(a, b); out = b; \
         annotation(experiment(StopTime = 1, Interval = 0.5, Tolerance = 1e-10)); end M;");
    let index = result.columns.iter().position(|c| c == "out").unwrap();
    assert_eq!(result.rows.last().unwrap()[index], 1.0);
}

/// `Integer(e)` is the ordinal of an enumeration value, which is not
/// the same thing as `integer(x)` cutting a number down.
#[test]
fn the_ordinal_of_an_enumeration_is_not_a_truncation() {
    let result = run("model M type Resolution = enumeration(s, ms, us); \
         parameter Resolution pick = Resolution.ms; \
         constant Integer table[3] = {1, 1000, 1000000}; \
         parameter Integer factor = table[Integer(pick)]; \
         Real y; equation y = factor + integer(-1.5); \
         annotation(experiment(StopTime = 1, Interval = 1, Tolerance = 1e-10)); end M;");
    let index = result.columns.iter().position(|c| c == "y").unwrap();
    // The second literal is the 1000, and `integer(-1.5)` is -2.
    assert_eq!(result.rows.last().unwrap()[index], 998.0);
}

#[test]
fn a_walked_body_carries_arrays() {
    // A length worked out by a `while` whose rounds the model decides,
    // over an array handed in. Nothing here can be unrolled while
    // flattening, so the run walks the body - and now it may be handed
    // an array and hold one while it goes. Three and four make five.
    let result = run(
        "model M function norm input Real v[:]; input Real a; output Real n; \
         protected Real acc; Integer k; \
         algorithm acc := 0; k := 1; \
         while k <= size(v, 1) and a > 0 loop acc := acc + v[k] * v[k]; k := k + 1; end while; \
         n := sqrt(acc); end norm; \
         Real y; equation y = norm({3 * time, 4 * time}, time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let column = result.columns.iter().position(|c| c == "y").unwrap();
    let y = result.rows.last().unwrap()[column];
    assert!((y - 5.0).abs() < 1e-12, "y(1)={y}, expected 5");

    // The same over a scalar product and a fold, which the walk writes
    // out element by element rather than one number at a time.
    let result = run(
        "model M function power input Real v[:]; input Real i[size(v, 1)]; input Real a; \
         output Real p; \
         protected Integer rounds; \
         algorithm rounds := 0; \
         while rounds < 1 and a > 0 loop p := v * i; rounds := rounds + 1; end while; end power; \
         Real y; equation y = power({1, 2 * time}, {3, 4}, time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let column = result.columns.iter().position(|c| c == "y").unwrap();
    let y = result.rows.last().unwrap()[column];
    assert!((y - 11.0).abs() < 1e-12, "y(1)={y}, expected 1*3 + 2*4");

    // An array the body declares for itself: as long as what it was
    // handed, filled in a loop, then folded. Sum, product and the
    // smallest of them are all written out element by element.
    let result = run(
        "model M function shape input Real v[:]; input Real a; output Real p;          protected Real w[size(v, 1)]; Real doubled[2]; Integer k;          algorithm k := 1;          while k <= size(v, 1) and a > 0 loop w[k] := v[k] + 1; k := k + 1; end while;          doubled := 2 .* w;          p := sum(w) + product(w) + min(w) + max(w) + sum(doubled) + sum({a, 0}) - a; end shape;          Real y; equation y = shape({1 * time, 2 * time}, time);          annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let column = result.columns.iter().position(|c| c == "y").unwrap();
    let y = result.rows.last().unwrap()[column];
    // w = {2, 3}: 5 + 6 + 2 + 3, and twice w sums to 10.
    assert!((y - 26.0).abs() < 1e-12, "y(1)={y}, expected 26");
}

#[test]
fn a_walked_body_says_what_it_cannot_carry() {
    // A subscript the model decides, and decides badly: the loop runs
    // as long as `time` says, so nothing could have been settled while
    // flattening. What the walk cannot do it says, and the answer that
    // comes back is not a number.
    let model = parse_model(
        "model M function odd input Real v[:]; input Real a; output Real y; \
         protected Integer k; \
         algorithm k := 0; while k < a loop y := v[k]; k := k + 1; end while; end odd; \
         Real y; equation y = odd({1, 2}, time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("parses");
    let trouble = compile(&model)
        .expect("compiles")
        .simulate()
        .expect_err("a subscript the model decided badly");
    assert!(
        trouble.to_string().contains("a whole number from one"),
        "{trouble}"
    );

    // An array given a value of the wrong length.
    let model = parse_model(
        "model M function odd input Real v[:]; input Real a; output Real y; \
         protected Real w[3]; Integer k; \
         algorithm k := 0; while k < a loop w := 2 .* v; k := k + 1; end while; y := w[1]; \
         end odd; \
         Real y; equation y = odd({1, 2}, time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("parses");
    let trouble = compile(&model)
        .expect("compiles")
        .simulate()
        .expect_err("three long, given two");
    assert!(trouble.to_string().contains("was given 2"), "{trouble}");
}

#[test]
fn a_walked_body_lays_out_what_it_answers_with() {
    // What a body answers with is laid out before it runs, so an
    // element it never fills is nothing rather than a name with no
    // value - which is what the language says an unassigned local is.
    let result = run("model M function half input Real a; output Real w[2]; \
         protected Integer k; \
         algorithm k := 0; while k < a loop w[1] := a; k := k + 1; end while; end half; \
         Real y[2]; equation y = half(time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;");
    let last = result.rows.last().unwrap();
    let at = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    assert!((last[at("y[1]")] - 1.0).abs() < 1e-12);
    assert_eq!(last[at("y[2]")], 0.0);
}

#[test]
fn a_walked_body_decides_over_arrays() {
    // Conditions, choices and negation over what was handed in, all
    // written out element by element on the way to one answer.
    let result = run(
        "model M function pick input Real v[:]; input Real a; output Real y; \
         protected Integer k; Real best; \
         algorithm best := -1e30; k := 1; \
         while k <= size(v, 1) and a > 0 loop \
           if v[k] > best and not (v[k] > 100 or v[k] < -100) then best := v[k]; end if; \
           k := k + 1; end while; \
         y := if sum(v) > 0 then best else -best; end pick; \
         Real y; equation y = pick({1, 5 * time, 200}, time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let column = result.columns.iter().position(|c| c == "y").unwrap();
    let y = result.rows.last().unwrap()[column];
    // The two hundred is passed over, so the largest kept is five.
    assert!((y - 5.0).abs() < 1e-12, "y(1)={y}, expected 5");

    // Two arrays multiplied are their scalar product, and a single
    // number goes with every element whichever side it is written on.
    let result = run(
        "model M function paired input Real v[:]; input Real w[size(v, 1)]; input Real a; \
         output Real y; protected Real scaled[size(v, 1)]; Integer k; \
         algorithm k := 0; \
         while k < 1 and a > 0 loop scaled := v .* 2; y := scaled * w + sum(3 .* w); \
           k := k + 1; end while; end paired; \
         Real y; equation y = paired({1, 2 * time}, {3, 4}, time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let column = result.columns.iter().position(|c| c == "y").unwrap();
    let y = result.rows.last().unwrap()[column];
    // Twice {1, 2} against {3, 4} is 22, and three times {3, 4} sums
    // to 21.
    assert!((y - 43.0).abs() < 1e-12, "y(1)={y}, expected 43");
}

#[test]
fn a_walked_body_answers_with_several_numbers() {
    // Nothing here can be unrolled while flattening, so the run walks
    // the body - and it answers with three numbers rather than one.
    // The model takes them one at a time, by the subscript Modelica
    // would write. v = {1, 2, 3} scaled by position is {1, 4, 9}.
    let result = run(
        "model M function scaled input Real v[3]; input Real a; output Real w[3]; \
         protected Integer k; \
         algorithm k := 1; \
         while k <= 3 and a > 0 loop w[k] := v[k] * k; k := k + 1; end while; end scaled; \
         Real y[3]; Real z; equation y = scaled({1, 2, 3 * time}, time); z = y[3]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let last = result.rows.last().unwrap();
    for (name, want) in [("y[1]", 1.0), ("y[2]", 4.0), ("y[3]", 9.0), ("z", 9.0)] {
        assert!(result.columns.iter().any(|c| c == name), "{name} is there");
        let column = result.columns.iter().position(|c| c == name).unwrap();
        assert!(
            (last[column] - want).abs() < 1e-12,
            "{name} = {}, expected {want}",
            last[column]
        );
    }
}

#[test]
fn a_record_of_losses_reaches_the_component_that_reads_it() {
    // The shape every machine in the standard library is built in: a
    // data record naming a reference power and speed with the torques
    // worked out from them as `final` fields, typed by aliases rather
    // than by `Real`, handed down through one component into another.
    // Each of those three - the final fields, the aliases, and the
    // value naming a record of the class above - was enough on its own
    // to lose the reference speed in silence and leave the parameter
    // with nothing.
    let result = run(
        "type Power = Real(unit = \"W\"); type Speed = Real(unit = \"rad/s\"); \
         record F parameter Power PRef = 0; parameter Speed wRef; \
         final parameter Real tauRef = PRef; end F; \
         record Data parameter Speed wNominal = 5; \
         parameter F frictionParameters(wRef = wNominal); end Data; \
         model Friction parameter F frictionParameters; Real y; \
         equation y = frictionParameters.wRef * time; end Friction; \
         model Machine parameter F frictionParameters; \
         Friction friction(final frictionParameters = frictionParameters); end Machine; \
         model M parameter Data motorData; \
         Machine dcpm(frictionParameters = motorData.frictionParameters); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let y = result
        .columns
        .iter()
        .position(|c| c == "dcpm.friction.y")
        .expect("no dcpm.friction.y");
    // The reference speed arrived, so the reading at t = 1 is it.
    assert!((result.rows.last().unwrap()[y] - 5.0).abs() < 1e-9);
}

#[test]
fn a_record_given_to_a_function_whole_is_read_field_by_field() {
    // The shape a machine works its nominal voltage out in: a
    // function takes a record of brush parameters and reads `V` and
    // `ILinear` out of it, and the caller hands the record over by
    // name rather than writing its fields out. Binding the name alone
    // left the body reading fields nothing was bound to, and the
    // parameter it was working out went missing with no word said
    // about which name was wanting.
    let result = run(
        "record Brush parameter Real V = 2; parameter Real ILinear = 4; end Brush; \
         function drop input Brush brush; input Real i; output Real v; \
         algorithm v := if i > brush.ILinear then brush.V \
           else brush.V * i / brush.ILinear; end drop; \
         model M parameter Brush brushParameters; \
         parameter Real i = 2; \
         parameter Real v = drop(brushParameters, i); \
         Real y; equation y = v * time; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let y = result.columns.iter().position(|c| c == "y").expect("no y");
    // `2 * 2 / 4` is one, and a reading of one says both fields
    // arrived: a missing `V` or a missing `ILinear` refuses the model
    // outright rather than answering with the wrong number.
    assert!((result.rows.last().unwrap()[y] - 1.0).abs() < 1e-9);
}

#[test]
fn a_walked_body_answers_with_a_record() {
    // The shape the standard library's water is written in: a body
    // that cannot be unrolled fills one member of a record in one
    // branch and another in the other, and whoever reads a member
    // knows which branch it was. What no branch filled is nothing.
    let result = run(
        "package P record Props Real a; Real b; Real c; end Props; end P; \
         model M function boundary input Real p; input Real go; output P.Props pro; \
         protected Integer k; \
         algorithm k := 0; \
         while k < go loop \
           if p > 1 then pro.a := p; pro.b := 2 * p; \
           else pro.a := -p; pro.c := 3 * p; end if; \
           k := k + 1; end while; end boundary; \
         P.Props q; Real y; \
         equation q = boundary(2 * time, time); y = q.a + q.b + q.c; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let last = result.rows.last().unwrap();
    let at = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    // Two at the end, so the first branch: 2 and 4, and nothing for
    // the member the other branch would have filled.
    assert!((last[at("q.a")] - 2.0).abs() < 1e-12);
    assert!((last[at("q.b")] - 4.0).abs() < 1e-12);
    assert_eq!(last[at("q.c")], 0.0);
    assert!((last[at("y")] - 6.0).abs() < 1e-12);

    // The other branch, and with it the other member.
    let result = run(
        "package P record Props Real a; Real b; Real c; end Props; end P; \
         model M function boundary input Real p; input Real go; output P.Props pro; \
         protected Integer k; \
         algorithm k := 0; \
         while k < go loop \
           if p > 1 then pro.a := p; pro.b := 2 * p; \
           else pro.a := -p; pro.c := 3 * p; end if; \
           k := k + 1; end while; end boundary; \
         P.Props q; Real y; \
         equation q = boundary(0.5 * time, time); y = q.a + q.b + q.c; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let last = result.rows.last().unwrap();
    let at = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    assert!((last[at("q.a")] + 0.5).abs() < 1e-12);
    assert_eq!(last[at("q.b")], 0.0);
    assert!((last[at("q.c")] - 1.5).abs() < 1e-12);

    // A record of more than plain numbers is left as it was written:
    // a name and a subscript are not enough for it, and the model is
    // told what does not add up rather than given a guess.
    let model = parse_model(
        "package P record Deep Real a; Real v[2]; end Deep; end P; \
         model M function boundary input Real p; input Real go; output P.Deep pro; \
         protected Integer k; \
         algorithm k := 0; while k < go loop pro.a := p; k := k + 1; end while; \
         end boundary; \
         P.Deep q; Real y; equation q = boundary(time, time); y = q.a; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("parses");
    let trouble = compile(&model).expect_err("a record of more than numbers");
    assert!(trouble.to_string().contains("unbalanced"), "{trouble}");
}

#[test]
fn a_body_written_here_takes_the_numbers_it_was_written_for() {
    // Two scalars rather than one array: the body takes the numbers in
    // the order they were written, whichever way the declaration
    // grouped them.
    let result = run(
        "model M function draw input Integer low; input Integer high; output Real r; \
         output Integer nextLow; output Integer nextHigh; \
         external \"C\" ModelicaRandom_xorshift64star(low, high, r, nextLow, nextHigh); \
         end draw; \
         discrete Real r(start = 0, fixed = true); \
         equation when time > 0.5 then r = draw(126247697, 0); end when; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;",
    );
    let r = result.columns.iter().position(|c| c == "r").unwrap();
    let last = result.rows.last().unwrap()[r];
    assert!((last - 0.554353923013482).abs() < 1e-15, "{last}");

    // Handed more numbers than the body was written for, the compiler
    // says so rather than reading past the end of them.
    let refusal = parse_model(
        "model M function draw input Integer state[3]; output Real r; \
         output Integer nextState[2]; \
         external \"C\" ModelicaRandom_xorshift64star(state, nextState, r); end draw; \
         discrete Real r(start = 0, fixed = true); \
         equation when time > 0.5 then r = draw({1, 2, 3}); end when; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(
        refusal.contains("not for what it was handed: 1 argument(s) of 3 number(s)"),
        "{refusal}"
    );
}

/// A table block of the shape the standard library gives one whose
/// first column is time: the data in a handle, the value asked for by
/// a body written in C, and the corners asked for so the run can put
/// events there.
const TIME_TABLE: &str = "package Times \
     class Handle extends ExternalObject; \
       function constructor input String tableName; input String fileName; \
         input Real table[:, :]; input Real startTime; input Integer columns[:]; \
         input Integer smoothness; input Integer extrapolation; input Real shiftTime; \
         output Handle h; \
         external \"C\" h = ModelicaStandardTables_CombiTimeTable_init3(tableName, fileName, \
           table, startTime, columns, smoothness, extrapolation, shiftTime); \
         end constructor; \
       function destructor input Handle h; \
         external \"C\" ModelicaStandardTables_CombiTimeTable_close(h); end destructor; \
     end Handle; \
     function getValue input Handle h; input Integer column; input Real t; \
       input Real nextEvent; input Real preNextEvent; output Real y; \
       external \"C\" y = ModelicaStandardTables_CombiTimeTable_getValue(h, column, t, \
         nextEvent, preNextEvent); end getValue; \
     function nextEvent input Handle h; input Real t; output Real at; \
       external \"C\" at = ModelicaStandardTables_CombiTimeTable_nextTimeEvent(h, t); \
       end nextEvent; \
   end Times; ";

#[test]
fn a_time_table_follows_the_lines_it_was_written_as() {
    let result = run(&format!(
        "{TIME_TABLE} model M \
         Times.Handle h = Times.Handle(\"NoName\", \"NoName\", [0, 0; 1, 2; 2, 6], \
           0, {{2}}, 1, 2, 0); \
         Real y; Real turns; \
         equation y = Times.getValue(h, 1, time, 0, 0); turns = Times.nextEvent(h, time); \
         annotation(experiment(StopTime = 1.5, Interval = 0.25, Tolerance = 1e-10)); end M;"
    ));
    let column = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (y, turns) = (column("y"), column("turns"));
    let at = |when: f64, which: usize| {
        result
            .rows
            .iter()
            .rev()
            .find(|row| row[0] <= when + 1e-9)
            .unwrap()[which]
    };

    // Two straight lines: 2t up to one, then 2 + 4(t - 1) past it,
    // carried on beyond the last row. The room left is for the nudge
    // an event puts on the clock, not for the arithmetic.
    for when in [0.0, 0.25, 0.5, 0.75] {
        assert!(
            (at(when, y) - 2.0 * when).abs() < 1e-6,
            "at {when}: {}",
            at(when, y)
        );
    }
    for when in [1.0, 1.25, 1.5] {
        let closed = 2.0 + 4.0 * (when - 1.0);
        assert!(
            (at(when, y) - closed).abs() < 1e-6,
            "at {when}: {}",
            at(when, y)
        );
    }

    // The next corner the table turns, at every point of the run.
    assert_eq!(at(0.5, turns), 1.0);
    assert_eq!(at(1.25, turns), 2.0);

    // The corner at t = 1 is an event, so the run stops there rather
    // than stepping over it.
    let corners = result.rows.iter().filter(|row| (row[0] - 1.0).abs() < 1e-9);
    assert!(corners.count() >= 2, "no event at the corner");
}

#[test]
fn a_time_table_is_asked_for_its_corners_at_an_event() {
    // The shape the standard library's block has: the next corner is
    // read at an event rather than at every step, and how many outputs
    // the block has is the longest of two lists.
    let result = run(&format!(
        "{TIME_TABLE} model M \
         parameter Integer columns[:] = {{2}}; parameter Real offset[:] = {{0}}; \
         parameter Integer nout = max([size(columns, 1); size(offset, 1)]); \
         Times.Handle h = Times.Handle(\"NoName\", \"NoName\", [0, 0; 1, 2; 2, 6], \
           0, columns, 1, 2, 0); \
         Real y[nout]; discrete Real nextT(start = 0, fixed = true); \
         equation when {{time >= pre(nextT), initial()}} then \
           nextT = Times.nextEvent(h, time); end when; \
         for i in 1:nout loop y[i] = Times.getValue(h, i, time, nextT, pre(nextT)); end for; \
         annotation(experiment(StopTime = 1.5, Interval = 0.5, Tolerance = 1e-10)); end M;"
    ));
    let column = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (y, next) = (column("y[1]"), column("nextT"));
    let last = result.rows.last().unwrap();
    assert!((last[y] - 4.0).abs() < 1e-6, "{}", last[y]);
    // Past the last row of the table there is no corner left to turn.
    assert_eq!(last[next], 2.0);
}

#[test]
fn a_size_written_before_the_array_is_settled_once_every_shape_is_in() {
    // `extends MO(final nout = max([size(columns, 1); size(offset, 1)]))`
    // is how the standard library's table blocks count their outputs:
    // the lengths belong to declarations further down the same class,
    // so nothing could say what they were where the modifier was
    // written.
    let result = run(
        "package P partial block Base parameter Integer n = 1; Real y[n]; end Base; end P; \
         model M extends P.Base(final n = max([size(cols, 1); size(offs, 1)])); \
         parameter Integer cols[:] = {2, 3}; parameter Real offs[:] = {0}; \
         equation for i in 1:n loop y[i] = i * time; end for; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    // The run got as far as working its parameters out, which is what
    // settling the length buys: with `size` still standing in `n`,
    // there is nothing the run could make of it.
    let first = result.columns.iter().position(|c| c == "y[1]").unwrap();
    let last = result.rows.last().unwrap();
    assert!((last[first] - last[0]).abs() < 1e-12, "{}", last[first]);
}

#[test]
fn a_system_of_equations_is_solved_by_a_body_written_here() {
    // `Modelica.Math.Matrices.solve` is a declaration in Modelica and
    // LAPACK's `dgesv` underneath. The matrix arrives row by row and
    // the right-hand side after it; what comes back is the solution
    // and word of how it went.
    let result = run(
        "model M function solve input Real a[:, size(a, 1)]; input Real b[size(a, 1)]; \
         output Real x[size(a, 1)]; output Integer info; \
         external \"FORTRAN 77\" dgesv(a, b, x, info); end solve; \
         Real x[2]; Real ok; \
         equation (x, ok) = solve([2, 1; 1, 3], {3, 5}); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let column = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    // 2x + y = 3 and x + 3y = 5 hold at (0.8, 1.4).
    assert!((last[column("x[1]")] - 0.8).abs() < 1e-12, "{:?}", last);
    assert!((last[column("x[2]")] - 1.4).abs() < 1e-12, "{:?}", last);
    assert_eq!(last[column("ok")], 0.0);

    // A matrix that is not square is not a shape this body was written
    // for, and the refusal says so rather than reading past its end.
    let refusal = parse_model(
        "model M function solve input Real a[:, :]; input Real b[:]; \
         output Real x[2]; output Integer info; \
         external \"FORTRAN 77\" dgesv(a, b, x, info); end solve; \
         Real x[2]; Real ok; \
         equation (x, ok) = solve([1, 2, 3; 4, 5, 6], {1, 2}); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(refusal.contains("written here"), "{refusal}");
}

/// A parameter written `fixed = false` takes its value from the
/// initialisation rather than from its declaration.
#[test]
fn a_parameter_fixed_false_is_settled_by_an_initial_equation() {
    // The standard library catches the moment a simulation starts this
    // way - `parameter SI.Time t0(fixed=false)` with `t0 = time` among
    // the initial equations - and then reads it for the rest of the
    // run. Here the start is at zero, so `der(x)` is one throughout
    // and `x` reaches one.
    let result = run(
        "model M parameter Real t0(fixed = false); Real x(start = 0, fixed = true); \
         initial equation t0 = time; \
         equation der(x) = t0 + 1; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;",
    );
    let last = result.rows.last().expect("a final row");
    let at = |name: &str| {
        result
            .columns
            .iter()
            .position(|column| column == name)
            .map(|index| last[index])
    };
    assert!((at("x").expect("x") - 1.0).abs() < 1e-6, "{:?}", at("x"));

    // The equation that settled it is not counted twice: it did its
    // work among the parameters, and asking the initialisation for it
    // again would leave one equation more than there are unknowns.
    let started = run(
        "model M parameter Real t0(fixed = false); Real x(start = 3); \
         initial equation t0 = 2; x = t0; \
         equation der(x) = 0; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;",
    );
    let final_row = started.rows.last().expect("a final row");
    let index = started
        .columns
        .iter()
        .position(|column| column == "x")
        .expect("x");
    assert!((final_row[index] - 2.0).abs() < 1e-6, "{:?}", final_row);
}

/// What `fixed = false` on a parameter does and does not change.
#[test]
fn a_parameter_the_initialization_settles_is_read_the_way_the_language_says() {
    let final_value = |source: &str, wanted: &str| {
        let result = run(source);
        let last = result.rows.last().expect("a final row").clone();
        let at = result
            .columns
            .iter()
            .position(|column| column == wanted)
            .unwrap_or_else(|| panic!("{wanted} in {:?}", result.columns));
        last[at]
    };

    // A declaration that wrote a value keeps it: the language asks for
    // a warning where both are given, not for the value to be thrown
    // away. Here `p` is 3, `x` starts there and climbs by one.
    let both = final_value(
        "model M parameter Real p(fixed = false) = 3; Real x(start = 0, fixed = false); \
         initial equation x = p; equation der(x) = 1; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;",
        "x",
    );
    assert!((both - 4.0).abs() < 1e-6, "{both}");

    // Which equation settles the parameter is not the first one that
    // mentions it: `x = p` is about the state and cannot settle
    // anything, so `p = 7` is taken whichever order they stand in.
    for order in ["x = p; p = 7;", "p = 7; x = p;"] {
        let settled = final_value(
            &format!(
                "model M parameter Real p(fixed = false); Real x(start = 0, fixed = false); \
                 initial equation {order} equation der(x) = 1; \
                 annotation(experiment(StopTime = 1, Interval = 0.5)); end M;"
            ),
            "x",
        );
        assert!((settled - 8.0).abs() < 1e-6, "{order}: {settled}");
    }

    // A constant is settled by its declaration whatever it says about
    // `fixed`, since the language does not let one be an unknown.
    let held = final_value(
        "model M constant Real k(fixed = false) = 5; Real x(start = 0, fixed = true); \
         equation der(x) = k; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;",
        "x",
    );
    assert!((held - 5.0).abs() < 1e-6, "{held}");
}

/// `pre` reaches a Boolean that no `when` assigns.
///
/// The language calls a Boolean and an Integer discrete-valued by
/// their type, whatever writes them, so each has a value from before
/// the event whether or not a `when` clause is what set it. Reading
/// one only as a `when` target refused models the standard library is
/// full of, friction and thyristors among them.
#[test]
fn pre_reaches_a_boolean_that_no_when_assigns() {
    let result = run(
        "model M Boolean b; Integer n; Real x(start = 1, fixed = true); \
         equation b = x < 0.5; n = if b then 2 else 1; \
         der(x) = if pre(b) then -0.5 * pre(n) else -1; \
         annotation(experiment(StopTime=1.0)); end M;",
    );
    let b = result.columns.iter().position(|c| c == "b").unwrap();
    let x = result.columns.iter().position(|c| c == "x").unwrap();
    let first = result.rows.first().unwrap();
    assert!(first[b] < 0.5, "b starts false: {}", first[b]);
    let last = result.rows.last().unwrap();
    assert!(last[b] > 0.5, "b ends true: {}", last[b]);
    // It fell at 1 per second to 0.5, then at 1 per second again once
    // `pre(b)` caught up with `b` at the event: 0.5 at half a second,
    // and 0.5 more over the remaining half.
    assert!(last[x].abs() < 1e-6, "x ends at zero: {}", last[x]);
}

/// Inside a `when` body, `pre` reaches a moving state.
///
/// The body runs at the instant of the event, so the value the state
/// arrived with is one that exists, and it is the value a block
/// averaging over a period is after: the integral just before the
/// `reinit` that clears it for the next period. Outside a `when` the
/// same state is between events and moving, and the refusal stands.
#[test]
fn a_when_body_can_ask_what_a_state_arrived_with() {
    let result = run(
        "model M Real x(start = 0, fixed = true); discrete Real mean(start = 0); \
         equation der(x) = 2 * time; \
         when sample(1, 1) then mean = pre(x); reinit(x, 0); end when; \
         annotation(experiment(StopTime=1.5)); end M;",
    );
    let mean = result.columns.iter().position(|c| c == "mean").unwrap();
    // Over the first second der(x) = 2t integrates to exactly 1, and
    // that is what the event finds waiting for it.
    let last = result.rows.last().unwrap();
    assert!(
        (last[mean] - 1.0).abs() < 1e-6,
        "mean = {}, expected the integral of 2t over the period",
        last[mean]
    );
    // Outside a when body a state is moving and has no such value.
    assert!(refused(
        "model M Real x(start = 0, fixed = true); Real y; \
                 equation der(x) = 1; y = pre(x); end M;"
    )
    .contains("is not discrete"));
}

/// A value handed down an `extends` reaches the run as a number.
///
/// Flattening leaves the call inlined; what matters here is that the
/// parameters can then evaluate it, which is where a model whose
/// nominal voltage comes down from a base class used to stop.
#[test]
fn a_parameter_handed_a_call_down_an_extends_is_evaluated() {
    let result = run("package Top \
           function twice input Real i; output Real v; algorithm v := 2 * i; end twice; \
           partial model Base parameter Real k = 0; Real x(start = 1, fixed = true); \
             equation der(x) = -k; end Base; \
           model M extends Base(final k = twice(3)); \
             annotation(experiment(StopTime=1.0)); end M; \
         end Top;");
    let x = result.columns.iter().position(|c| c == "x").unwrap();
    let last = result.rows.last().unwrap();
    // k is 6, so x falls from 1 to -5 over the second.
    assert!(
        (last[x] + 5.0).abs() < 1e-6,
        "x = {}, expected the run to have used k = 6",
        last[x]
    );
}

/// A function body may leave a variable unset in one branch.
///
/// The language says an unassigned local or output of a function
/// starts where its type starts, and the standard library writes whole
/// property functions that way: the steam tables fill `cp` on one side
/// of a region boundary and `cv` on the other, each meant to be left
/// at zero where the other was set. Refusing that took thirty-two
/// models out of reach, the Fluid examples among them.
#[test]
fn a_function_may_leave_a_value_unset_in_one_branch() {
    let result = run("package Top \
           record Pair Real hot; Real cold; end Pair; \
           function pick input Real u; output Real y; \
           protected Pair p; \
           algorithm \
             if u > 0 then p.hot := u; else p.cold := -u; end if; \
             y := p.hot + p.cold; \
           end pick; \
           model M \
             Real warm = pick(time + 1); \
             annotation(experiment(StopTime=0.1)); \
           end M; \
         end Top;");
    let last = result.rows.last().expect("a final row");
    let warm = last[1];
    // The branch that ran set `hot` to 1.1; the branch that did not
    // leaves `cold` where its type starts, which is zero.
    assert!(
        (warm - 1.1).abs() < 1e-9,
        "the unset field should count as its own start of zero: {warm}"
    );
}

/// The same for a record handed on whole, and for a flag.
///
/// A body that sets a record in one branch and hands it on in both
/// needs the record itself to have a start, which is its fields'
/// starts gathered up. A `Boolean` starts at false rather than at
/// zero, and a field declared through a chain of unit aliases -
/// `SI.Temperature` is a `Real` by way of two other names - has to be
/// followed to the end to find that out.
#[test]
fn an_unset_record_starts_as_its_fields_do() {
    let result = run("package Top \
           type Temperature = Real; \
           record State Temperature warm; Boolean lit; end State; \
           function pick input Real u; output Real y; \
           protected State s; \
           algorithm \
             if u > 2 then s.warm := u; s.lit := true; end if; \
             y := s.warm + (if s.lit then 10 else 0); \
           end pick; \
           model M \
             Real cool = pick(time + 1); \
             annotation(experiment(StopTime=0.1)); \
           end M; \
         end Top;");
    let last = result.rows.last().expect("a final row");
    let cool = last[1];
    // Neither field was set: the number starts at zero and the flag at
    // false, so nothing is added for either.
    assert!(
        cool.abs() < 1e-9,
        "an unset record should start as its fields do: {cool}"
    );
}

/// A record set whole in one branch starts as its fields do.
///
/// The earlier test leaves the fields alone one by one; the steam
/// tables assign the record itself - `f := Basic.f3(bpro.d, bpro.T)`
/// inside the region 3 test - and read it back whichever way the test
/// went. The merge is then asked about the record's own name, and a
/// record has no start of its own to give: it starts as its fields
/// do, gathered in the order the record declares them. Without that
/// the whole body is turned down before anything else can be looked
/// at, and the Fluid fitting example that reaches the boiling curve
/// stops at that refusal rather than at whatever is really wrong
/// further in.
#[test]
fn a_record_assigned_whole_in_one_branch_starts_as_its_fields_do() {
    let source = "package Top \
           record Derivs Real d; Real t; end Derivs; \
           function f3 input Real u; output Derivs g; \
             algorithm g.d := u; g.t := 2*u; end f3; \
           function region input Real u; output Real y; \
           protected Derivs f; \
           algorithm \
             if u > 2 then f := f3(u); end if; \
             y := f.d + f.t; \
           end region; \
           model M \
             Real dry = region(time); \
             annotation(experiment(StopTime=0.1)); \
           end M; \
         end Top;";
    let said = match oxidelica_parser::parse_model(source) {
        Ok(_) => String::new(),
        Err(error) => error.message,
    };
    assert!(
        !said.contains("assigned in one branch only"),
        "the record should start at its fields rather than turn the body down: {said}"
    );
}

/// A flag left unset beside a run of numbers still starts at false.
///
/// Giving unset locals a start is held back where a branch writes an
/// array, because that lets a whole body be folded into the caller and
/// a quaternion's four elements of nested conditions cost the library
/// minutes. A flag is not that: it decides which branch runs rather
/// than being folded into arithmetic, so it can never grow the value.
/// The steam tables put one right beside an array - `hsubcrit` is set
/// once and read in both halves of the region test - and holding it
/// back kept thirty-two models from flattening at all.
#[test]
fn a_flag_beside_an_array_still_starts_at_false() {
    let result = run("package Top \
           function region input Real p; output Real r; \
           protected Boolean sub; Real[3] v; \
           algorithm \
             v := {0, 0, 0}; \
             if p > 1 then v := {p, 1, 2}; sub := true; end if; \
             r := v[1] + (if sub then 10 else 0); \
           end region; \
           model M \
             Real y = region(time); \
             annotation(experiment(StopTime=0.1)); \
           end M; \
         end Top;");
    let last = result.rows.last().expect("a final row");
    let y = last[1];
    // Time never passes 1, so the branch never runs: the array keeps
    // its zeros and the flag is false, adding nothing.
    assert!(y.abs() < 1e-9, "the unset flag should start at false: {y}");
}

/// A logical operator on things only the array pass can settle.
///
/// `anyTrue` is written as `size(b, 1) > 0 and max(b)`, and both sides
/// are questions about a vector that only this pass can answer: the
/// length of an input declared `[:]`, and the largest of its elements.
/// The pass had no rule for `and`, so the whole expression went off to
/// the scalar path, where the vector arrives whole and is turned down
/// for being an array. Seventeen models could not be flattened for
/// that, the state graphs among them - they ask whether any branch of
/// a split has been reset, and that is exactly this call.
#[test]
fn a_logical_operator_settles_what_only_shapes_can_answer() {
    let result = run("package Top \
           function anyTrue input Boolean b[:]; \
             output Boolean r = size(b, 1) > 0 and max(b); \
             algorithm end anyTrue; \
           model M \
             Boolean v[3] = {false, time > 0.5, false}; \
             Boolean q = anyTrue(v); \
             annotation(experiment(StopTime=1)); \
           end M; \
         end Top;");
    let last = result.rows.last().expect("a final row");
    let names = &result.columns;
    let q = last[names
        .iter()
        .position(|n| n == "q")
        .expect("q is written out")];
    // Past half a second the middle element is true, so any of them is.
    assert!(
        q > 0.5,
        "the vector's own questions should be answered before `and`: {q}"
    );
}

/// An `initial algorithm` that assigns a discrete variable says where
/// that variable starts, not what the states must satisfy: a periodic
/// source works out which period it is in before the first event ever
/// fires, and would otherwise be counted as a condition on states it
/// says nothing about.
#[test]
fn an_initial_algorithm_starts_the_discrete_variables_it_assigns() {
    let result = run(
        "model M parameter Real period = 1; Real T_start; Integer count; \
         initial algorithm count := integer((time + 2.5) / period); \
         T_start := count * period; \
         equation when time >= (pre(count) + 1) * period - 2.5 then \
         count = pre(count) + 1; T_start = time; end when; \
         annotation(experiment(StopTime = 0.4, Interval = 0.1, Tolerance = 1e-10)); end M;",
    );
    let last = result.rows.last().unwrap();
    let at = |name: &str| result.columns.iter().position(|n| n == name).unwrap();
    assert!(
        (last[at("count")] - 2.0).abs() < 1e-9,
        "the count the initial algorithm worked out stands: {}",
        last[at("count")]
    );
    assert!(
        (last[at("T_start")] - 2.0).abs() < 1e-9,
        "and what was worked out from it: {}",
        last[at("T_start")]
    );

    // The same when the value is one only the run knows: a count taken
    // off a state is still about where the count starts, not about
    // where the state does, and the state keeps its declared start.
    let result = run("model M Real x(start = 3); Integer count; \
         initial equation count = integer(x); \
         equation der(x) = 1; \
         when time >= 0.5 then count = pre(count) + 1; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.1, Tolerance = 1e-10)); end M;");
    let last = result.rows.last().unwrap();
    let at = |name: &str| result.columns.iter().position(|n| n == name).unwrap();
    assert!(
        (last[at("x")] - 4.0).abs() < 1e-9,
        "the state started where it was declared to: {}",
        last[at("x")]
    );

    // An initial equation that is about the states after all is left
    // where it was, and still says where the run begins.
    let result = run("model M Real x; Integer count; \
         initial equation 2 * x = 6; \
         equation der(x) = 1; \
         when time >= 0.5 then count = pre(count) + 1; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.1, Tolerance = 1e-10)); end M;");
    let last = result.rows.last().unwrap();
    let at = |name: &str| result.columns.iter().position(|n| n == name).unwrap();
    assert!(
        (last[at("x")] - 4.0).abs() < 1e-9,
        "the initial equation settled the state: {}",
        last[at("x")]
    );
}

/// A derivative another equation already states may be read anywhere
/// in a third: inside a comparison, either side of an `and` or an
/// `or`, under a negation, in any branch of a choice. Each of those
/// has to hand the derivative on by the name the run knows it by.
#[test]
fn a_derivative_is_named_wherever_it_is_read() {
    let result = run("model M Real x; Real y; equation der(x) = 3; \
         y = if (der(x) > 0 and not (der(x) < -1)) or der(x) == 0 then -der(x) else 2; \
         annotation(experiment(StopTime = 1, Interval = 0.5, Tolerance = 1e-10)); end M;");
    let last = result.rows.last().unwrap();
    let at = |name: &str| result.columns.iter().position(|n| n == name).unwrap();
    assert!(
        (last[at("y")] + 3.0).abs() < 1e-9,
        "the branch the derivative chose, and what it came to: {}",
        last[at("y")]
    );
}

#[test]
fn an_if_equation_asking_whether_the_run_has_begun_takes_the_running_branch() {
    // The hysteresis models write `if initial() then k = 0.01; else
    // ... end if` to give a state something to hold in the instant
    // before the run. Choosing a branch happens before the flag exists,
    // and what holds for every instant of the run is the other branch.
    let result = run("model M Real x(start = 0); Real k; \
         equation der(x) = k; \
         if initial() then k = 0; else k = 1; end if; \
         annotation(experiment(StopTime=1.0, Interval=0.1)); end M;");
    let last = result.rows.last().unwrap();
    let k = result.columns.iter().position(|n| n == "k").unwrap();
    assert!((last[k] - 1.0).abs() < 1e-9, "k = {}", last[k]);
}

#[test]
fn an_initial_equation_about_a_boolean_says_where_it_starts() {
    // The state graph writes `active = true` for the step a graph
    // begins in and `pre(newActive) = pre(localActive)` for the memory
    // behind it. Neither is a condition on the states - a Boolean keeps
    // its value between events whatever assigns it - so counting them
    // among the initial equations left the initialisation with ten
    // equations for no states at all.
    let result = run("model M Boolean on; Boolean seen; Real x(start = 0); \
         initial equation on = true; pre(seen) = true; \
         equation der(x) = 1; \
         when x > 0.5 then on = false; end when; \
         when x > 0.8 then seen = pre(on); end when; \
         annotation(experiment(StopTime=1.0, Interval=0.1)); end M;");
    let last = result.rows.last().unwrap();
    let on = result.columns.iter().position(|n| n == "on").unwrap();
    assert_eq!(last[on], 0.0);
    let first = result.rows.first().unwrap();
    assert_eq!(first[on], 1.0, "the initial equation says it starts true");
}

/// A check among the actions of a `when` is made when the event fires,
/// not at every step: the steady-state tests of the Fluid library are
/// written that way, and a run where it does not hold is wrong rather
/// than over.
#[test]
fn an_assert_at_an_event_fires_with_the_event() {
    // The check holds when the event comes, so the run finishes.
    let result = run("model M Real x; equation x = time; \
         when time > 1 then assert(x < 5, \"held\"); end when; \
         annotation (experiment(StopTime = 2)); end M;");
    assert!(result.rows.last().is_some_and(|row| row[0] > 1.0));

    // The same check that does not hold stops the run and says so, at
    // the time the event happened rather than at the first step where
    // the condition was already false.
    let model = parse_model(
        "model M Real x; equation x = time; \
         when time > 1 then assert(x < 0.5, \"too big at the event\"); end when; \
         annotation (experiment(StopTime = 2)); end M;",
    )
    .unwrap();
    let why = compile(&model).unwrap().simulate().unwrap_err().to_string();
    assert!(why.contains("too big at the event"), "{why}");
    assert!(why.contains("t = 1."), "at the event: {why}");
}

/// A run begins where the model asked it to. The force-stroke curves
/// of the flux tubes sweep a coil from `-4` millimetres, `time`
/// standing for the position rather than for a clock.
#[test]
fn a_run_begins_where_the_model_asked() {
    let result = run("model M Real x; equation x = time; \
         annotation (experiment(StartTime = -4, StopTime = 4)); end M;");
    let first = result.rows.first().unwrap();
    let last = result.rows.last().unwrap();
    assert!((first[0] + 4.0).abs() < 1e-9, "starts at -4: {first:?}");
    assert!((last[0] - 4.0).abs() < 1e-9, "ends at 4: {last:?}");
    // `time` is what it says it is on both ends.
    assert!((first[1] + 4.0).abs() < 1e-9, "{first:?}");
}

/// A call on its own among the actions of a `when`: nothing takes its
/// outputs, so what it was written for is what its body does. What the
/// compiler can take from it is the checks the body makes.
#[test]
fn a_call_at_an_event_carries_the_checks_of_its_body() {
    // The call stands, and the run reaches the end.
    let result = run("package P function note input Real x; output Real y; \
         algorithm y := x; end note; \
         model M Real t; equation t = time; \
         when terminal() then note(t); end when; end M; end P;");
    assert!(result.rows.last().is_some());

    // A check inside the called body is taken up by the model.
    let model = parse_model(
        "package P function guard input Real x; output Real y; \
         algorithm assert(x < 0.05, \"guard tripped\"); y := x; end guard; \
         model M Real t; equation t = time; \
         when terminal() then guard(t); end when; end M; end P;",
    )
    .unwrap();
    let why = compile(&model).unwrap().simulate().unwrap_err().to_string();
    assert!(why.contains("guard tripped"), "{why}");
}

/// A flexible `:` length is read from the value a component is given,
/// and a value handed down beats the one its declaration wrote. Two
/// instances of one block each measure their own.
#[test]
fn each_instance_measures_the_value_it_was_handed() {
    let result = run("package P block B \
         parameter Real v[:] = {0, 1}; \
         parameter Integer n = size(v, 1); \
         output Real y; \
         equation y = n; end B; \
         model M B a(v = {2, 4, 6, 8}); B b(v = {1, 3}); \
         Real p; Real q; equation p = a.y; q = b.y; end M; end P;");
    let last = result.rows.last().unwrap();
    let at = |name: &str| {
        result
            .columns
            .iter()
            .position(|c| c == name)
            .map(|i| last[i])
            .unwrap_or_else(|| panic!("no column {name} in {:?}", result.columns))
    };
    // Four and two, each from its own value rather than from the
    // declaration's default of two, and not from each other.
    assert_eq!(at("p"), 4.0, "{:?}", result.columns);
    assert_eq!(at("q"), 2.0, "{:?}", result.columns);

    // The same for a matrix and for a range read off one.
    let table = run("package P block B \
         parameter Real t[:,:] = [0, 1]; \
         parameter Integer cols[:] = 2:size(t, 2); \
         output Real y; \
         equation y = size(cols, 1); end B; \
         model M B b(t = [0, 10, 20; 1, 30, 40]); Real z; \
         equation z = b.y; end M; end P;");
    let last = table.rows.last().unwrap();
    let z = table.columns.iter().position(|c| c == "z").unwrap();
    assert_eq!(last[z], 2.0, "columns 2:3 is two");
}

/// A value handed to a base may ask how long an array of the class
/// handing it down is: `extends MO(final nout = size(columns, 1))` is
/// how the table blocks say how many outputs they have. The base is
/// instantiated next and has never heard of `columns`, so the question
/// is answered where the modifier was written.
#[test]
fn a_modifier_handed_to_a_base_is_worked_out_where_it_was_written() {
    let result = run("package P partial block MO parameter Integer n = 1; \
         output Real y[n]; end MO; \
         block Table extends MO(final n = size(cols, 1)); \
         parameter Real t[:,:] = [0, 1]; \
         parameter Integer cols[:] = 2:size(t, 2); \
         equation for i in 1:n loop y[i] = i; end for; end Table; \
         model M Table b(t = [0, 10, 20; 1, 30, 40]); \
         Real u; Real v; equation u = b.y[1]; v = b.y[2]; end M; end P;");
    let last = result.rows.last().unwrap();
    let at = |name: &str| {
        result
            .columns
            .iter()
            .position(|c| c == name)
            .map(|i| last[i])
            .unwrap_or_else(|| panic!("no column {name} in {:?}", result.columns))
    };
    // Two columns, so two outputs, and each is its own place.
    assert_eq!(at("u"), 1.0);
    assert_eq!(at("v"), 2.0);
}

/// `min` and `max` take two numbers or one array. A block takes the
/// longer of two lengths with `max([size(a, 1); size(b, 1)])`, which
/// is one argument holding both.
#[test]
fn min_and_max_fold_over_one_array_as_well_as_two_numbers() {
    let result = run("model M parameter Integer n = max([2; 1]); \
         parameter Integer m = min({5, 3, 4}); \
         Real y[n]; Real z; \
         equation for i in 1:n loop y[i] = i; end for; z = m; end M;");
    let last = result.rows.last().unwrap();
    let at = |name: &str| {
        result
            .columns
            .iter()
            .position(|c| c == name)
            .map(|i| last[i])
    };
    // `max([2; 1])` is two, so there is a second element to name.
    assert!(at("y[2]").is_some(), "{:?}", result.columns);
    assert_eq!(at("z"), Some(3.0));
}

/// A check among the actions of a `when` goes through the passes a
/// flat model is put through: its names are resolved, a string in it
/// is settled, and a table it asks after is rewritten.
#[test]
fn a_check_at_an_event_goes_through_the_flat_passes() {
    // A string constant compared inside the check: settled before the
    // run like every other string.
    let named = parse_model(
        "model M constant String mode = \"fast\"; Real x; \
         equation x = time; \
         when time > 1 then assert(mode == \"fast\", \"wrong mode\"); end when; \
         annotation (experiment(StopTime = 2)); end M;",
    )
    .unwrap();
    assert!(compile(&named).is_ok());

    // A check that holds lets the run finish; the same one failing
    // stops it at the event.
    let result = run("model M Real x; Real k; equation x = time; k = 2; \
         when time > 1 then assert(x < k * 10, \"held\"); end when; \
         annotation (experiment(StopTime = 2)); end M;");
    assert!(result.rows.last().is_some_and(|row| row[0] > 1.0));
}

/// A model whose components are conditional is compiled once per mode,
/// and a check among the actions of a `when` is carried into each of
/// them: the run has to be able to tell when the mode it was compiled
/// for has been left behind, and the check goes along.
#[test]
fn a_check_at_an_event_survives_a_conditional_component() {
    let result = run("model M parameter Boolean on = true; \
         Real x if on; Real y; \
         equation y = time; \
         if on then x = time; end if; \
         when time > 1 then assert(y < 5, \"held\"); end when; \
         annotation (experiment(StopTime = 2)); end M;");
    let last = result.rows.last().unwrap();
    assert!((last[0] - 2.0).abs() < 1e-9, "reached the stop time");

    // The same check failing stops the run at the event.
    let model = parse_model(
        "model M parameter Boolean on = true; \
         Real x if on; Real y; \
         equation y = time; \
         if on then x = time; end if; \
         when time > 1 then assert(y < 0.5, \"too big\"); end when; \
         annotation (experiment(StopTime = 2)); end M;",
    )
    .unwrap();
    let why = compile(&model).unwrap().simulate().unwrap_err().to_string();
    assert!(why.contains("too big"), "{why}");
}
