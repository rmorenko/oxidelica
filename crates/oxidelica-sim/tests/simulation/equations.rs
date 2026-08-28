//! What a system of equations comes to when it is run.

use super::shared::*;
use oxidelica_parser::parse_model;
use oxidelica_sim::{compile, SimResult};

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
    // A call nothing worked out is the other cause, and it waits on
    // nobody: a parameter written as a function of literals names no
    // free variable at all, so calling that a cycle names a shape the
    // model does not have. The call is what is said instead.
    let standing =
        compile_err("model M parameter Real a = nowhere(1, 2); Real x; equation x = 1; end M;");
    assert!(standing.contains("nothing works out"), "{standing}");
    assert!(standing.contains("`nowhere`"), "{standing}");
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

    // The refusal names what nothing determines, not every unknown of
    // the model. The counts alone stand for many different illnesses,
    // and a survey of a library cannot tell them apart until the
    // refusal says which name is missing its equation.
    let said = compile_err("model M Real y; Real z; equation y = 1; end M;");
    assert!(said.contains("nothing determines z"), "{said}");
    // Too many equations instead: the equation with nothing left to
    // solve for is the one to name. Two equations on one unknown are
    // caught earlier, as a model whose extra equation constrains no
    // state, so this asks for a model that is unbalanced the other way.
    let said = compile_err("model M Real y; equation y = 1; y + 1 = 2; end M;");
    assert!(said.contains("nothing is left for"), "{said}");

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
fn time_is_available_in_equations() {
    let result = run("model T Real y; equation y = 2 * time; \
         annotation(experiment(StopTime=1.0, Interval=0.5)); end T;");
    let last = result.rows.last().unwrap();
    assert!((last[1] - 2.0).abs() < 1e-12);
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
fn one_phase_of_a_port_stands_where_its_port_stands() {
    // A class whose ports are plugs may join one phase of each:
    // `connect(a.pin[1], b.pin[1])` names two ports of the class
    // itself, not of its components, and both ends are outside. Read
    // as inside - which is what looking for a dot in the whole path
    // does - the two halves of every boundary grow together into one
    // node, and the flow through each plug loses its equation.
    const P: &str = "connector Pin Real v; flow Real i; end Pin; \
         connector Plug Pin pin[1]; end Plug; \
         model Box Plug a; Plug b; equation connect(a.pin[1], b.pin[1]); end Box; ";
    let result = run(&format!(
        "{P} model M Plug src; Box box; Plug snk; \
         equation connect(src, box.a); connect(box.b, snk); \
         src.pin[1].v = 10; snk.pin[1].i = 1; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    ));
    // What is pushed in at one end comes out at the other, and the
    // current through the box is the same current all the way.
    let at = |name: &str| {
        let index = result.columns.iter().position(|c| c == name).unwrap();
        result.rows.last().unwrap()[index]
    };
    assert!(
        (at("box.a.pin[1].i") - 1.0).abs() < 1e-9,
        "{}",
        at("box.a.pin[1].i")
    );
    assert!(
        (at("src.pin[1].i") + 1.0).abs() < 1e-9,
        "{}",
        at("src.pin[1].i")
    );
}

#[test]
fn an_ideal_diode_defines_its_switch_outside_any_when() {
    // `off = s < 0` is how the standard library writes an ideal
    // semiconductor: a Boolean, defined by a relation, outside any
    // `when`. Read as one more continuous unknown it joins the loop
    // that solves for `s`, where the residual of a relation is a step
    // and its finite difference is zero either side of the knee - the
    // Jacobian says nothing, and the model is refused for a matrix
    // that is mute rather than for anything being wrong. The pair
    // belongs to the event layer: the switch is asked once a round of
    // the event iteration, and between events it stands still while
    // the indicator watches for the crossing.
    let result = run("connector Pin Real v; flow Real i; end Pin; \
         model Diode Pin p; Pin n; Real v; Real i; Real s(start = 0); \
         Boolean off(start = true); parameter Real Ron = 1e-5; \
         parameter Real Goff = 1e-5; \
         equation v = p.v - n.v; i = p.i; p.i + n.i = 0; off = s < 0; \
         v = s * (if off then 1 else Ron); \
         i = s * (if off then Goff else 1); end Diode; \
         model Src Pin p; Pin n; \
         equation p.v - n.v = sin(time * 314); p.i + n.i = 0; end Src; \
         model Res Pin p; Pin n; parameter Real R = 1; \
         equation p.v - n.v = R * p.i; p.i + n.i = 0; end Res; \
         model M Diode dp1; Diode dp2; Diode dn1; Diode dn2; \
         Src src; Res load(R = 1); Pin g; \
         equation connect(src.p, dp1.p); connect(dp1.n, load.p); \
         connect(src.n, dp2.p); connect(dp2.n, load.p); \
         connect(load.n, dn1.p); connect(dn1.n, src.p); \
         connect(load.n, dn2.p); connect(dn2.n, src.n); \
         connect(load.n, g); g.v = 0; \
         annotation(experiment(StopTime = 0.04, Interval = 0.0002)); end M;");
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (off, current) = (index("dp1.off"), index("dp1.i"));
    // Half-wave rectification: current one way only, and none at all
    // while the diode blocks. Two periods of a 50 Hz source, so the
    // switch has to have flipped.
    let mut flips = 0;
    for (before, row) in result.rows.iter().zip(result.rows.iter().skip(1)) {
        if (before[off] > 0.5) != (row[off] > 0.5) {
            flips += 1;
        }
        // A blocking ideal diode still leaks `Goff` times the voltage
        // across it - a hundredth of a milliamp here - so the bound is
        // the leak rather than zero.
        assert!(
            row[current] > -2e-5,
            "current ran backwards: {}",
            row[current]
        );
        if row[off] > 0.5 {
            assert!(row[current].abs() < 2e-5, "blocking, yet {}", row[current]);
        }
    }
    assert!(flips >= 2, "the switch never moved: {flips} flips");
}

#[test]
fn a_medium_redeclaring_a_function_is_the_one_the_call_means() {
    // The water tables redeclare `specificEnthalpy_pT` with a
    // `region` their base never had, and the body of the base's own
    // BaseProperties calls it with that argument by name. Bound
    // against the base's declaration - which is what the written name
    // resolves to - `region` is an input nothing has, and forty-five
    // models were refused for it. Asked under the medium the model
    // named, the medium's own function is what the call means.
    let result = run("package Base partial package Two \
         replaceable function enthalpy input Real p; input Real T; \
         input Integer phase = 0; output Real h; \
         algorithm h := p + T; end enthalpy; \
         replaceable model BaseProperties input Real p; input Real T; Real h; \
         equation h = enthalpy(p, T, region = 3); end BaseProperties; \
         end Two; end Base; \
         package Water extends Base.Two; \
         redeclare function enthalpy input Real p; input Real T; \
         input Integer phase = 0; input Integer region = 0; output Real h; \
         algorithm h := 2 * p + T + region; end enthalpy; end Water; \
         model M package Medium = Water; Medium.BaseProperties props; \
         equation props.p = 1; props.T = 2; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;");
    let index = result.columns.iter().position(|c| c == "props.h").unwrap();
    // The redeclaration: 2*1 + 2 + 3. The base would give 1 + 2, and
    // could not take `region` at all.
    assert!((result.rows.last().unwrap()[index] - 7.0).abs() < 1e-9);
}

#[test]
fn a_walked_body_may_answer_with_several_numbers() {
    // `dofpt3` gives a density and an error code; `regFun3` a value
    // and a flag. A body the run walks used to be held to one output,
    // which refused four models of the standard library outright. The
    // numbers of the answer are laid out in the order the outputs are
    // declared, and a call takes the one it asked for - including
    // through a tuple assignment inside another walked body, which is
    // how the inverse functions of the water tables are written.
    //
    // The trip count comes from a variable, so nothing can unroll the
    // loop and the run really does walk the body: inlined instead,
    // this would pass without the fix.
    let result = run("function pair input Real x; output Real d; output Real e; \
         protected Real acc; \
         algorithm acc := 0; for k in 1:3 loop acc := acc + x; end for; \
         d := acc; e := acc * 2; end pair; \
         function total input Real x; input Integer n; output Real y; \
         protected Real d; Real e; \
         algorithm y := 0; \
         for k in 1:n loop (d, e) := pair(x); y := y + d + e; end for; \
         end total; \
         model M Real y; Real u; Integer n; \
         equation u = time + 1; n = integer(2 + 0 * time); y = total(u, n); \
         annotation(experiment(StopTime = 0.5, Interval = 0.5)); end M;");
    let index = result.columns.iter().position(|c| c == "y").unwrap();
    // u is 1.5, so d is 4.5 and e is 9; twice round the loop is 27.
    let last = result.rows.last().unwrap()[index];
    assert!((last - 27.0).abs() < 1e-9, "{last}");
}

#[test]
fn a_bound_over_a_whole_array_is_not_an_assertion_per_element() {
    // `Ron[m](final min = zeros(m))` bounds the array as a whole. The
    // flattener leaves such a bound standing rather than refusing it,
    // since it does not come to one value, and each element of the
    // array is its own component by the time the run is built. Turned
    // into an assertion per element it would compare one number
    // against a call to `zeros`, which the code generator knows
    // nothing about, and the model would be refused for a function
    // the language supplies.
    let result = run("model D parameter Integer m = 3; \
         parameter Real Ron[m](final min = zeros(m)) = fill(1e-5, m); \
         Real y[m]; equation for k in 1:m loop y[k] = Ron[k]; end for; end D; \
         model M D d(m = 3); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;");
    let index = result.columns.iter().position(|c| c == "d.y[1]").unwrap();
    assert!((result.rows.last().unwrap()[index] - 1e-5).abs() < 1e-12);
}

#[test]
fn a_port_joined_only_from_inside_carries_nothing() {
    // A machine's internal thermal port is joined by the windings
    // inside it, and the model above it is free to leave it alone -
    // the ambient it feeds is a component of the machine, switched on
    // by `useThermalPort = false`. Nothing outside such a port is
    // there to take the flow, so the flow through it is zero: "not
    // connected from the outside". Without that equation the port has
    // one half of a seam where it should have two, and the flow it
    // carries is a variable nothing determines.
    let result = run("connector HP Real T; flow Real Q_flow; end HP; \
         model Amb HP port; parameter Real Ta = 300; equation port.T = Ta; end Amb; \
         model Src HP heatPort; parameter Real P = 5; \
         equation heatPort.Q_flow = -P; end Src; \
         model Machine HP internalPort; Amb ambient(Ta = 300); Src core(P = 5); \
         equation connect(core.heatPort, internalPort); \
         connect(ambient.port, internalPort); end Machine; \
         model M Machine m; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;");
    let at = |name: &str| {
        let index = result.columns.iter().position(|c| c == name).unwrap();
        result.rows.last().unwrap()[index]
    };
    // The heat the core gives off goes to the ambient inside, and the
    // port to the outside world carries none of it.
    assert!((at("m.internalPort.Q_flow")).abs() < 1e-9);
    assert!((at("m.ambient.port.Q_flow") - 5.0).abs() < 1e-9);
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
    // b pushes 20 with weight 1, and the outside port brings nothing:
    // a flow variable passes through a port rather than being summed
    // against it, so what the port carries is what the inside sends
    // out - both components push 1 out, the port carries 2 out - and a
    // port carrying flow outwards has nothing to push in. Read the
    // other way round, as it was while the two sides of a port were
    // summed into one node, the port carried 2 inwards at the same
    // time as the components it feeds were emptying into it.
    assert!(
        (result.rows.last().unwrap()[index] - 20.0).abs() < 1e-6,
        "heard {}",
        result.rows.last().unwrap()[index]
    );
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
    // What the standard library's own C answers for this seed:
    // `x*INVM64 + 0.5` of the word the state moved to, the word read
    // as signed. Checked against a run of that C rather than worked
    // out here, because agreeing with it is the whole point.
    assert!((last - 0.054353923013481964).abs() < 1e-15, "{last}");

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

/// A value handed to a component may name a constant array of the
/// class handing it down: a table block is given one of three curves
/// the class keeps. A constant has no elements to build, so it never
/// reached the table of shapes the component is instantiated with, and
/// the length could not be read from it.
#[test]
fn a_component_may_be_handed_a_constant_array_of_the_class_above() {
    let result = run("package P type Kind = enumeration(A, B); \
         block Sink parameter Real tab[:,:] = [0, 1]; output Real y; \
         equation y = size(tab, 1); end Sink; \
         block Pick parameter Kind k = Kind.A; \
         Sink s(final tab = if k == Kind.A then One else Two); \
         output Real y; \
         protected constant Real One[:,2] = [0,1; 1,0]; \
         constant Real Two[:,2] = [0,1; 0.5,0.5; 1,0]; \
         equation y = s.y; end Pick; \
         model M Pick a(k = Kind.A); Pick b(k = Kind.B); \
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
    // Two rows in the first curve, three in the second, and each
    // instance reads the one it was given.
    assert_eq!(at("p"), 2.0);
    assert_eq!(at("q"), 3.0);
}

/// A matrix written with `[ ; ]` stacks what it is given rather than
/// laying it out: `[v, w]` where `v` is a vector of four is four rows,
/// not one. Counting the rows as written answers four times too small,
/// and a shape said wrongly is worse than none.
#[test]
fn a_matrix_of_vectors_is_measured_by_what_it_stacks() {
    let result = run("package P block Sink parameter Real tab[:,:] = [0, 1]; \
         output Real y; equation y = size(tab, 1); end Sink; \
         block Src parameter Real v[4] = {1, 2, 3, 4}; \
         parameter Real w[4] = {5, 6, 7, 8}; \
         Sink s(final tab = [v, w]); output Real y; \
         equation y = s.y; end Src; \
         model M Src a; Real z; equation z = a.y; end M; end P;");
    let last = result.rows.last().unwrap();
    let z = result.columns.iter().position(|c| c == "z").unwrap();
    // Four rows of two, not one row of two.
    assert_eq!(last[z], 4.0, "{:?}", result.columns);

    // A matrix of plain numbers still says its shape by how it is
    // written, which is the case this measurement was added for.
    let plain = run("package P block Sink parameter Real tab[:,:] = [0, 1]; \
         output Real y; equation y = size(tab, 1); end Sink; \
         model M Sink s(tab = [0, 10; 1, 20; 2, 30]); Real z; \
         equation z = s.y; end M; end P;");
    let last = plain.rows.last().unwrap();
    let z = plain.columns.iter().position(|c| c == "z").unwrap();
    assert_eq!(last[z], 3.0);
}

/// A record may take its fields from a base rather than write them:
/// `redeclare record extends ThermodynamicState` is how a medium says
/// its state is the one it inherits. A function taking that state was
/// told it takes nothing at all.
#[test]
fn a_record_that_extends_carries_the_fields_it_inherits() {
    let result = run("package P partial package Base \
         replaceable record ThermodynamicState Real p; Real T; \
         end ThermodynamicState; \
         replaceable partial function density \
         input ThermodynamicState state; output Real d; end density; \
         end Base; \
         package Simple extends Base; \
         redeclare record extends ThermodynamicState end ThermodynamicState; \
         redeclare function extends density \
         algorithm d := state.p / state.T; end density; end Simple; \
         model M Simple.ThermodynamicState st; Real y; \
         equation st.p = 100; st.T = 4; y = Simple.density(st); end M; end P;");
    let last = result.rows.last().unwrap();
    let y = result.columns.iter().position(|c| c == "y").unwrap();
    // A hundred over four: the state carried both its inherited fields.
    assert_eq!(last[y], 25.0, "{:?}", result.columns);
}

/// A bus member nobody connects to stands at zero.
#[test]
fn a_declared_bus_member_nothing_connects_to_stands_at_zero() {
    // An expandable connector usually declares nothing and takes the
    // members its connections name. The standard library's control bus
    // declares the five it expects as well, and a declared member no
    // `connect` reaches has no equation at all - so the model came out
    // with one unknown more than it had equations. 9.1.3 says what it
    // is worth: a potential variable with no connections is zero.
    let result = run(
        "package P expandable connector Bus Real signal1; Real signal2; \
         Boolean flag; end Bus; \
         connector Signal = output Real; \
         model Source Signal y; equation y = time; end Source; \
         model M Bus bus; Source src; Real y; \
         equation connect(src.y, bus.signal1); y = bus.signal1; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M; end P;",
    );
    let at = |name: &str| {
        let column = result
            .columns
            .iter()
            .position(|held| held == name)
            .unwrap_or_else(|| panic!("{name} is not a column: {:?}", result.columns));
        result.rows[result.rows.len() - 1][column]
    };
    assert_eq!(at("bus.signal2"), 0.0);
    // A Boolean member stands at false, which is zero carried as a
    // truth rather than a number equated to one.
    assert_eq!(at("bus.flag"), 0.0);
}

/// A table asked to repeat may be differentiated.
///
/// A periodic table wraps its abscissa with `mod`, and the derivative
/// of the block that reads it could not be taken: `mod` was not a
/// call the differentiator knew, so sixteen models came out
/// structurally singular. What the wrap comes to between its steps is
/// a straight line, so the derivative is the one the table would have
/// had were it never wrapped.
#[test]
fn a_repeating_time_table_may_be_differentiated() {
    // `0, 2, 6` against `0, 1, 2`, repeating: a slope of 2 over the
    // first second of each period and 4 over the second.
    let source = |extrapolation: u32| {
        format!(
            "{TIME_TABLE} model M \
             Times.Handle h = Times.Handle(\"NoName\", \"NoName\", [0, 0; 1, 2; 2, 6], \
               0, {{2}}, 1, {extrapolation}, 0); \
             Real y; Real slope; \
             equation y = Times.getValue(h, 1, time, 0, 0); slope = der(y); \
             annotation(experiment(StopTime = 0.75, Interval = 0.25, \
               Tolerance = 1e-10)); end M;"
        )
    };
    let slope_of = |extrapolation: u32| {
        let result = run(&source(extrapolation));
        let which = result.columns.iter().position(|c| c == "slope").unwrap();
        result.rows.last().unwrap()[which]
    };
    // Periodic is 3; carrying the last line on is 2. Inside the first
    // period the two tables are the same table, so their slopes are
    // the same slope - which is the whole of what the rule claims.
    let repeating = slope_of(3);
    assert!((repeating - 2.0).abs() < 1e-6, "{repeating}");
    assert!(
        (repeating - slope_of(2)).abs() < 1e-9,
        "wrapped {repeating} against unwrapped {}",
        slope_of(2)
    );
}

/// A body written here may answer several numbers to a parameter.
///
/// A random generator gives a value and the state it moved to, and
/// the standard library builds a generator's first state by drawing
/// ten numbers from a seed - so the whole nest of calls has to come
/// to a number before the run starts, not during it.
#[test]
fn a_body_written_here_answers_a_parameter_too() {
    let result = run("model M \
         function draw input Integer low; input Integer high; output Real r; \
           protected Integer s[2]; \
           algorithm (r, s) := random({low, high}); end draw; \
         function random input Integer state[2]; output Real r; output Integer out[2]; \
           external \"C\" ModelicaRandom_xorshift64star(state, out, r); end random; \
         parameter Real drawn = draw(126247697, 0); \
         Real y; equation y = drawn; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;");
    let y = result.columns.iter().position(|c| c == "y").unwrap();
    let value = result.rows.last().unwrap()[y];
    // The same number the run would have drawn, worked out before it.
    assert!((value - 0.054353923013481964).abs() < 1e-15, "{value}");
}

/// A run of elements given one place of an answer takes the places
/// after it.
///
/// A body written here answers with several numbers at once, and an
/// array output of it arrives as one place of that answer. Counted as
/// a single value against the elements, it was refused for the count;
/// each element now takes the place after the one before it.
#[test]
fn a_run_of_elements_takes_the_places_after_the_first() {
    let result = run("model M \
         function random input Integer state[2]; output Real r; output Integer out[2]; \
           external \"C\" ModelicaRandom_xorshift64star(state, out, r); end random; \
         function first input Integer seed; output Integer state[2]; protected Real r; \
           algorithm (r, state) := random({seed, 0}); state[1:2] := state; end first; \
         parameter Integer s[2] = first(126247697); \
         Real y; equation y = s[1] + s[2]; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;");
    let y = result.columns.iter().position(|c| c == "y").unwrap();
    let value = result.rows.last().unwrap()[y];
    // Both halves of the state the generator moved to, added: what
    // matters is that each element is a number of its own rather than
    // the whole answer twice.
    assert!(value.is_finite(), "{value}");
    let first = result.columns.iter().position(|c| c == "s[1]");
    let second = result.columns.iter().position(|c| c == "s[2]");
    if let (Some(first), Some(second)) = (first, second) {
        let row = result.rows.last().unwrap();
        assert!(
            row[first] != row[second],
            "{:?} {:?}",
            row[first],
            row[second]
        );
    }
}
