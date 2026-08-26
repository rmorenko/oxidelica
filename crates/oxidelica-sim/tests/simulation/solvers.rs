//! The solvers: which one is chosen, what it costs, and what it does with a step it cannot take.

use super::shared::*;
use oxidelica_parser::parse_model;
use oxidelica_sim::{compile, SolverMethod};

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
