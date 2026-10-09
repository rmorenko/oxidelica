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
fn an_auxiliary_the_caller_worked_out_is_handed_on_without_a_derivative() {
    // `derivative(noDerivative = aux) = f_der` says the rule is handed
    // the auxiliary record as it stands: the caller worked it out from
    // the other arguments, so its rate of change is already accounted
    // for by theirs, and a record has no derivative anybody could form.
    // That is how the water library differentiates a property read -
    // `rho_props_ph(p, h, waterBaseProp_ph(p, h, ...))` - without ever
    // differentiating the property table.
    //
    // The producer here has a loop the inliner cannot unroll, so the
    // call stands and the differentiator meets `makeAux(x)[1]`, which
    // is exactly the shape the water models refuse on.
    let library = "record Aux Real a; end Aux; \
         function makeAux input Real p; output Aux aux; protected Real w; \
         algorithm w := p; while w > 1.0 loop w := w - 1.0; end while; \
         aux.a := 2.0 * p; end makeAux; ";
    let with_rule = "function f input Real p; input Aux aux; output Real y; \
         algorithm y := aux.a; \
         annotation(derivative(noDerivative = aux) = f_der); end f; \
         function f_der input Real p; input Aux aux; input Real p_der; \
         output Real y_der; algorithm y_der := 2.0 * p_der; end f_der; ";

    // `f(x, makeAux(x))` is `2x`, held at 2, so `x` is 1 and its rate
    // is zero: the constraint had to be differentiated to get there,
    // and only the annotation could differentiate it.
    let result = run(&format!(
        "model M {library} {with_rule} Real x(start = 1, fixed = true); Real v; \
         equation der(x) = v; f(x, makeAux(x)) = 2.0; \
         annotation(experiment(StopTime = 0.5, Interval = 0.25)); end M;"
    ));
    let at = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let end = result.rows.last().unwrap();
    assert!((end[at("x")] - 1.0).abs() < 1e-9, "{}", end[at("x")]);
    assert!((end[at("v")]).abs() < 1e-9, "{}", end[at("v")]);

    // A rule that moves with time, so the number says the seed reached
    // it rather than that a constant came out right: `2x = 2 + t`
    // gives `x = 1 + t/2` and a rate of a half.
    let moving = run(&format!(
        "model M {library} {with_rule} Real x(start = 1, fixed = true); Real v; \
         equation der(x) = v; f(x, makeAux(x)) = 2.0 + time; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;"
    ));
    let at = |name: &str| moving.columns.iter().position(|c| c == name).unwrap();
    let end = moving.rows.last().unwrap();
    assert!((end[at("x")] - 1.5).abs() < 1e-9, "{}", end[at("x")]);
    assert!((end[at("v")] - 0.5).abs() < 1e-9, "{}", end[at("v")]);

    // Without the annotation the same model is refused, and the words
    // are the ones the water models met: the call stands, the
    // subscript on it reaches the differentiator, and nothing there
    // has a rule for it.
    let refused = compile_err(&format!(
        "model M {library} function f input Real p; input Aux aux; output Real y; \
         algorithm y := aux.a; end f; Real x(start = 1, fixed = true); Real v; \
         equation der(x) = v; f(x, makeAux(x)) = 2.0; \
         annotation(experiment(StopTime = 0.5, Interval = 0.25)); end M;"
    ));
    assert!(
        refused.contains("cannot differentiate a subscript that survived flattening"),
        "{refused}"
    );
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
fn a_pendulum_written_through_its_angle_does_not_freeze_at_the_turning_point() {
    // The position is a state and the angle an algebraic unknown found
    // from `x = cos(phi)`, so the constraint on `y` reaches `x` only
    // through that equation. Weighed by its slope in the residual
    // alone, `x` counted for nothing: the pivot kept `y` demoted, and
    // at phi = 0 - where `x = 1` and the angle stops being found from
    // it - every column stood still to the last digit until the stop
    // time, with no refusal. The reference is the same pendulum
    // integrated in its angle by a fixed-step RK4 at h = 1e-5.
    const P3: &str = "model P3 parameter Real r[2] = {1, 0}; parameter Real m = 1; \
         Real x(start = 0.5, fixed = true); Real y; \
         Real vx(start = 0, fixed = true); Real vy; Real phi(start = 0.5); Real T; \
         equation der(x) = vx; der(y) = vy; m*der(vx) = -T*x; m*der(vy) = -T*y - m*9.81; \
         x = cos(phi)*r[1] - sin(phi)*r[2]; y = sin(phi)*r[1] + cos(phi)*r[2]; \
         annotation(experiment(StopTime = 1)); end P3;";
    let result = run_on(P3, SolverMethod::Dopri45).expect("runs");
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    assert!((last[0] - 1.0).abs() < 1e-9);
    let (x, y) = (last[index("x")], last[index("y")]);
    assert!(
        (x - -0.635_181_7).abs() < 1e-3 && (y - -0.772_362_7).abs() < 1e-3,
        "the pendulum ended at x = {x}, y = {y}, not at (-0.6352, -0.7724)"
    );
}

#[test]
fn a_constant_ratio_between_victim_and_alternative_is_no_reason_to_choose_again() {
    // The constraint `p = q` reaches `a` through `p = sinh(a)` and `b`
    // through `q = sinh(0.1*b)`: a gear of one to ten, the same ratio
    // at every instant. The first build weighs both at zero and demotes
    // `b` by order; the monitor, weighing through the definitions, read
    // 0.1 against 1.0 below its level of 0.15 and asked for a rebuild
    // at the first step - which in `GearConstraint` built a model whose
    // algebra was NaN. A ratio that does not fall is not a selection
    // going bad.
    const GEAR: &str = "model G Real a(start = 0, fixed = true); Real b; Real p; Real q; \
         Real tau; equation der(a) = 1 - tau; der(b) = tau; \
         p = sinh(a); q = sinh(0.1*b); p = q; \
         annotation(experiment(StopTime = 1)); end G;";
    let result = run_on(GEAR, SolverMethod::Dopri45).expect("runs");
    assert_eq!(
        result.reselections, 0,
        "a constant ratio asked for a rebuild"
    );
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    // a = 0.1 b and a + b = 1.
    let (a, b) = (last[index("a")], last[index("b")]);
    assert!(
        (a - 1.0 / 11.0).abs() < 1e-6 && (b - 10.0 / 11.0).abs() < 1e-6,
        "the gear ended at a = {a}, b = {b}"
    );
}

#[test]
fn a_reselected_run_stops_where_the_caller_said_and_not_where_the_model_did() {
    // The spinning pendulum below re-selects its states every quarter
    // turn, and every re-selection compiles the run afresh. The stop
    // time and output step set on the compiled model after it was
    // built - which is what `simulate --stop` and the ten steps of
    // `library check` both do - must survive that, rather than the
    // fresh build reading them from the annotation again and running
    // on to t = 3 at the annotation's step.
    const SPIN: &str = "model P parameter Real g = 9.81; \
         Real x(start = 0, fixed = true); Real y(start = -1, fixed = true); \
         Real vx(start = 8, fixed = true); Real vy(start = 0, fixed = true); Real lam; \
         equation der(x) = vx; der(y) = vy; der(vx) = lam * x; der(vy) = lam * y - g; \
         x * x + y * y = 1; \
         annotation(experiment(StopTime = 3, Interval = 0.002, Tolerance = 1e-9)); end P;";
    let mut compiled = compile(&parse_model(SPIN).unwrap()).unwrap();
    compiled.stop_time = 0.5;
    compiled.step = 0.01;
    let result = compiled.simulate().expect("runs");
    assert!(result.reselections >= 1, "the run never re-selected");
    let last = result.rows.last().unwrap()[0];
    assert!((last - 0.5).abs() < 1e-9, "the run stopped at t = {last}");
    // Every output point lies on the caller's grid of 0.01; a point an
    // event placed between them is allowed, a grid of 0.002 is not.
    let on_fine_grid = result
        .rows
        .iter()
        .filter(|row| {
            let k = row[0] / 0.002;
            (k - k.round()).abs() < 1e-6 && ((row[0] / 0.01) - (row[0] / 0.01).round()).abs() > 1e-6
        })
        .count();
    assert!(
        on_fine_grid < 5,
        "{on_fine_grid} rows on the annotation's grid of 0.002"
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

#[test]
fn a_saturating_amplifier_does_not_send_newton_between_the_rails() {
    // An idealised operational amplifier: a gain of 15000 clipped to
    // plus or minus fifteen volts, with a resistive divider from the
    // output back to the inverting input. The whole of it is one
    // algebraic loop, and the solution sits on the sloped part
    // between the rails.
    //
    // Started from zero, the first residual puts the iterate on the
    // flat of the limiter, where the Jacobian claims a step across to
    // the other rail clears the residual. It does not: the same
    // argument sends the next step back, and undamped Newton spends
    // its fifty iterations swinging between plus and minus fifteen.
    // Shortening the step until the residual falls walks off the flat
    // and onto the slope, where Newton converges as it should.
    let result = run("model OpAmp \
         parameter Real V0 = 15000; parameter Real R1 = 1000; parameter Real R2 = 1000; \
         Real in_p_v; Real in_n_v; Real out_v; Real r_i; Real mid_v; \
         equation \
         in_p_v = time; \
         out_v = smooth(0, noEvent(if V0*(in_p_v - in_n_v) > 15 then 15 \
             else if V0*(in_p_v - in_n_v) < -15 then -15 else V0*(in_p_v - in_n_v))); \
         out_v - mid_v = R2*r_i; mid_v = R1*r_i; in_n_v = mid_v; \
         annotation(experiment(StopTime = 1, Interval = 0.25, Tolerance = 1e-8)); end OpAmp;");
    // A divider of equal resistors makes this a buffer of gain two,
    // so the output follows twice the input until the rails cut in -
    // and at t = 1 the ideal 2 V is well inside them.
    let out = result.columns.iter().position(|c| c == "out_v").unwrap();
    let last = result.rows.last().unwrap();
    assert!(
        (last[out] - 2.0).abs() < 1e-3,
        "out_v = {}, wanted 2",
        last[out]
    );
}

#[test]
fn a_newton_step_over_the_edge_of_a_domain_is_shortened_rather_than_called_divergence() {
    // `1/sqrt(x - 3)` is the shape of a medium's property function:
    // finite and well behaved over its domain, and nothing at all
    // beyond the edge of it. Newton's full step from `x = 4` lands at
    // minus fourteen, where the square root is not a number, and the
    // solver used to read that value and call the block diverged -
    // naming the iteration for a fault of the domain, with the root
    // sitting at 3.01 a short way from where it started.
    //
    // This is what the sixteen `algebraic loop diverged` models of the
    // corpus do. `SeriesPipes2`, the smallest, steps a pressure from
    // five atmospheres to two hundred, where water's IF97 formulation
    // answers NaN, and refuses on its second iteration with a finite
    // residual behind it.
    let result = run("model D Real x(start = 4.0); equation \
                      1.0 / sqrt(x - 3.0) = 10.0 + time; \
                      annotation(experiment(StopTime=0.001, Interval=0.001)); end D;");
    let x = result.rows.last().unwrap()[1];
    // At `t = 0.001` the right side is 10.001, so `x - 3` is its
    // inverse square: the answer is a value and not the mere fact that
    // the block came back.
    let expected = 3.0 + 1.0 / (10.001f64 * 10.001);
    assert!((x - expected).abs() < 1e-9, "x={x}, expected {expected}");
}

#[test]
fn a_column_small_in_its_own_unit_is_not_a_dead_column() {
    // An enthalpy in a cooling circuit is carried by a mass flow, and
    // at rest that flow is zero: `PumpAndValve` hands the solver a
    // block whose five enthalpy columns sit at 1e-24 beside a volume
    // flow at 1e-4. How small a column's entries are is the unit its
    // unknown is measured in, and no honest test of whether a block
    // determines a step may notice that - but `solve_linear` judges
    // its pivots against 1e-14 flat, so the enthalpy columns read as
    // though nothing in the block moved when they did, and the refusal
    // came back as a singular Jacobian about a block with one plain
    // answer.
    //
    // Divided each column through by its own largest entry the block
    // is invertible, and the step comes back in the scaled unknowns to
    // be divided out again. The same argument `equilibrate_columns`
    // was written for, one path over: the check that a *converged*
    // block is determined already scaled, and the step that has to get
    // there did not.
    let source = "model C parameter Real m = 1e-24; \
                  Real h(start = 288.0); Real q(start = 0.1); equation \
                  q*abs(q) + m*h = 0.25 + 293.15*m; \
                  m*h*abs(h) = m*293.4*abs(293.4) + q*1e-24 - 0.5e-24; \
                  annotation(experiment(StopTime=0.001, Interval=0.001)); end C;";
    let result = run(source);
    let row = result.rows.last().unwrap();
    let names = &result.columns;
    let at = |what: &str| row[names.iter().position(|n| n == what).expect(what)];
    // The first equation fixes the flow at a half, and the second then
    // fixes the enthalpy at 293.4 - the values, not the mere fact that
    // the block came back with something.
    assert!((at("q") - 0.5).abs() < 1e-9, "q={}", at("q"));
    assert!((at("h") - 293.4).abs() < 1e-6, "h={}", at("h"));
}

#[test]
fn a_direction_the_residual_does_not_fall_along_is_said_so_rather_than_walked() {
    // `abs(x) + 1 = 0` has no root, and its Jacobian is a perfectly
    // ordinary plus or minus one: nothing about the matrix is wrong.
    // What is wrong is that the direction it hands back does not take
    // the residual down, and no fraction of it does either - the line
    // search halves twenty times and every trial is as large as where
    // it started.
    //
    // Taken anyway, which is what happened before, the iteration
    // creeps by a millionth of a step at a time until the arithmetic
    // hands back a value that is not a number, and the Jacobian built
    // at that point is all NaN and reported as singular. Seven models
    // of the corpus refused that way; `BranchingPipes2` is the trail
    // that showed it, twelve iterations with the residual rising from
    // 1.6249e6 to 1.6251e6 and then NaN.
    let refusal = refused(
        "model D Real x(start = 1.0); equation \
                           abs(x) + 1.0 = 0.0; \
                           annotation(experiment(StopTime=0.001, Interval=0.001)); end D;",
    );
    assert!(
        refusal.contains("does not reduce the residual"),
        "{refusal}"
    );
    // And it names what was measured rather than the solver: how large
    // the residual was when the halving began.
    assert!(refusal.contains("steps running"), "{refusal}");
    assert!(!refusal.contains("singular"), "{refusal}");
}

#[test]
fn a_fall_bought_only_by_cutting_the_step_to_a_sliver_is_not_a_step() {
    // The guard above asks whether the line search found a smaller
    // residual, and that is not the whole question: a block can go on
    // finding one for as long as the step is cut small enough, and
    // then it is not travelling, it is creeping. Each such iteration
    // reports descent and resets the count of stuck steps, so the
    // guard never reaches three and the block spends its whole budget.
    //
    // This is what `BranchingPipes1` does. The trail in
    // /tmp/m214/bp1b.txt has thirteen steps whose accepted fraction
    // falls from 6.25e-2 to 9.5e-7 while the residual goes from
    // 1.06078e6 to 1.06076e6 - two parts in a hundred thousand for the
    // whole crawl - and it ends by stepping over the edge of the water
    // formulation, where the residual is NaN and the Jacobian built
    // there is four rows of NaN, reported as a singular matrix. The
    // matrix again is not what is wrong.
    //
    // Here the crawl ends in the budget rather than in a NaN, so
    // without the rule the refusal blames fifty iterations, which
    // names a count and not a cause.
    let refusal = refused(
        "model E Real x(start = 0.0); Real y(start = 1.0); equation \
                       y = abs(x) + 1e-9*x + sqrt(1.0 - x); \
                       y*1e6 + x = -1e6; \
                       annotation(experiment(StopTime=0.001, Interval=0.001)); end E;",
    );
    assert!(
        refusal.contains("does not reduce the residual"),
        "{refusal}"
    );
    // And it says what was paid for the fall, which is the fact the
    // guard could not see before.
    assert!(refusal.contains("only below"), "{refusal}");
    assert!(!refusal.contains("50 Newton iterations"), "{refusal}");
}

#[test]
fn a_step_over_the_edge_of_a_domain_says_so_rather_than_naming_the_matrix() {
    // `sqrt(x) + 1 = 0` has no root, and every Newton step from a
    // positive start goes left, past zero, into the half line where
    // the square root is not a number. The retreat that exists for
    // exactly this - halve the step back towards the last finite
    // point - runs out after twenty halvings, and the question is
    // what is said then.
    //
    // What was said before was about somewhere else. The walk went
    // on from the point whose residual is NaN, the Jacobian was
    // built there by finite differences and came back all NaN, and
    // the refusal named a singular matrix or a divergence. Both
    // send the reader to the solver, which is the one place nothing
    // is wrong: the matrix is NaN because the point is, and the
    // point is over an edge.
    //
    // Three models of the corpus refuse this way for real -
    // `BranchingPipes1`, `BranchingPipes12` and `BranchingPipes14`,
    // whose water formulation answers NaN above 1e7 pascals - and
    // none of them is small enough to write down here.
    let refusal = refused(
        "model I Real x(start = 1e-14); equation \
         sqrt(x) + 1.0 = 0.0; \
         annotation(experiment(StopTime=0.001, Interval=0.001)); end I;",
    );
    assert!(
        refusal.contains("stepped outside the domain of its own equations"),
        "{refusal}"
    );
    // And it names the equation that stopped being a number, not
    // only the fact that one did.
    assert!(refusal.contains("sqrt(x)"), "{refusal}");
    // The two refusals it replaces name the solver. Neither may
    // come back.
    assert!(!refusal.contains("singular"), "{refusal}");
    assert!(!refusal.contains("diverged"), "{refusal}");
}

#[test]
fn a_residual_is_judged_against_what_the_inner_unknowns_it_reads_cancelled() {
    // The cancellation of the test below, moved one storey away: `y`
    // is an inner unknown of the torn block, assigned from the
    // difference of two numbers of 2^22, and the row that is left
    // reads only `z` and `x`, both near one. The rounding of the big
    // numbers reaches the row through `y` and `z`, and the row by
    // itself never met anything loud - so judged against its own
    // numbers the ulp of 2^22 it carries was a distance from the
    // solution, and the block was refused. `IMS_Start` stands on this
    // edge with its node voltages behind currents of 3.8e6.
    let source = "model L Real x(start = 1.0); Real y; Real z; \
         parameter Real big = 4.194304e6; equation \
         y = (big + x^3) - (big + x); z = y + x; z - x = 5e-10 + 1e-30*sin(z); \
         annotation(experiment(StopTime=0.001, Interval=0.001)); end L;";
    let result = run(source);
    let column = result.columns.iter().position(|c| c == "x").expect("x");
    let x = result.rows.last().expect("a row")[column];
    assert!((x - 1.0).abs() < 1e-6, "x = {x}");
}

#[test]
fn a_residual_that_cancelled_inside_one_side_is_judged_against_what_it_cancelled() {
    // How large a residual has to be before it means anything is set
    // by the numbers it was subtracted from, and the convergence test
    // asks that of the two sides of the equation. Which is right
    // where the cancellation is between them, and blind where a side
    // cancels within itself: then both sides are small, the floor
    // taken from them is smaller still, and what is left is one ulp
    // of numbers neither side remembers.
    //
    // Here both sides carry 2^22 and the difference being chased is
    // 5e-10, which is under an ulp of it. Newton walks to within one
    // and can go no further - there is no double in between - and the
    // floor from the sides is 4e-22, a million times below where the
    // iteration has to stop. Judged against the loudest number the
    // row met on the way, 4194305, it is under 4 eps of it and the
    // block is solved.
    //
    // `Modelica.Electrical.Analog.Examples.Rectifier` is the same
    // eight lines with four million amperes in place of 2^22: its
    // seventh row reads lhs = 9.313e-10, rhs = 0.
    let source = "model C Real x(start = 1.0); \
         parameter Real big = 4.194304e6; equation \
         (big + x^3) - (big + x) = 5e-10; \
         annotation(experiment(StopTime=0.001, Interval=0.001)); end C;";
    let result = run(source);
    // And the answer is the root, to what the arithmetic can hold:
    // x^3 - x = 5e-10 near 1 has no solution the doubles can name, so
    // what is accepted is the point one ulp away from where the two
    // sides meet.
    let column = result
        .columns
        .iter()
        .position(|c| c == "x")
        .expect("x is recorded");
    let x = result.rows.last().expect("a row")[column];
    assert!((x - 1.0).abs() < 1e-6, "x = {x}");
}

/// A step the explicit solver throws away leaves nothing behind. The
/// cubic `y^3 - 3 y = x` has three roots for `x` near one, and the
/// run starts on the lowest of them, at -1.38 for x = 1.5. The state
/// falls to one in a few microseconds, so the first step tried is far
/// too long for it: its stages put `x` at 50 and beyond, where the
/// cubic has a single root on the upper branch, and the block solves
/// there. Before this was repaired the rejected step handed that root
/// on as the start of every shorter try, the block went on being
/// solved on the upper branch, and the run reported y = 1.879 - a
/// root of the equation, and not the one a continuous `y` reaches
/// from where it began. Along the run `y` follows `x` down its own
/// branch to -1.532, the lowest root of `y^3 - 3 y = 1`.
#[test]
fn a_rejected_step_does_not_hand_its_roots_to_the_next_try() {
    let result = run(
        "model W Real x(start = 1.5, fixed = true); Real y(start = -2); \
         equation der(x) = -1e5*(x - 1); y^3 - 3*y = x; \
         annotation(experiment(StopTime = 0.0005, Interval = 0.0001)); end W;",
    );
    let column = result.columns.iter().position(|c| c == "y").expect("y");
    for row in &result.rows {
        assert!(
            row[column] < 0.0,
            "y left its branch at t = {}: {}",
            row[0],
            row[column]
        );
    }
    let y = result.rows.last().expect("a row")[column];
    // y = 2 cos(theta) turns the cubic into cos(3 theta) = 1/2.
    let lowest = 2.0 * (7.0 * std::f64::consts::PI / 9.0).cos();
    assert!((y - lowest).abs() < 1e-5, "y = {y}, lowest root {lowest}");
}

/// The same for the implicit solver. Started on the upper root of
/// `y^3 - 3 y = x`, 1.942 for x = 1.5, the run follows `x` down to one
/// along the upper branch and ends on its root of 1.879. The first step
/// BDF tries is far too long for the stiff `x`: its predictor puts `x`
/// near -48, where the cubic has a single root of about -3.9 on the
/// lower branch, and the block solves there. Before this was repaired
/// the rejected step handed that root on to every shorter try, and the
/// run ended on the lower branch at -1.532 without a word.
#[test]
fn a_rejected_implicit_step_does_not_hand_its_roots_to_the_next_try() {
    let result = run_on(
        "model W Real x(start = 1.5, fixed = true); Real y(start = 2); \
         equation der(x) = -1e5*(x - 1); y^3 - 3*y = x; \
         annotation(experiment(StopTime = 0.01, Interval = 0.001)); end W;",
        SolverMethod::Bdf,
    )
    .expect("runs");
    let column = result.columns.iter().position(|c| c == "y").expect("y");
    for row in &result.rows {
        assert!(
            row[column] > 0.0,
            "y left its branch at t = {}: {}",
            row[0],
            row[column]
        );
    }
    let y = result.rows.last().expect("a row")[column];
    // y = 2 cos(theta) turns the cubic into cos(3 theta) = 1/2.
    let upper = 2.0 * (std::f64::consts::PI / 9.0).cos();
    assert!((y - upper).abs() < 1e-5, "y = {y}, upper root {upper}");
}

/// The volume of the small tee of m321, written out flat: the energy
/// balance of a litre of dry air held at 1e5 Pa, with the enthalpy of
/// the NASA polynomial the library's `DryAirNasa` gives it. With the
/// pressure fixed the mass is not free, so the block that is left is
/// one unknown, `der(h)`, solved from `der(U) = Hb` where `U = m u`
/// and the terms of the balance stand at 7.4e9 while `der(h)` decays
/// toward zero. A slope read with the textbook step `1e-8 (1 + |v|)`
/// moves that sum by less than its rounding, Newton is handed the
/// rounding for a slope and the trail jumps without settling until
/// it runs out of steps. The column is asked again from further away
/// when its difference sits inside the rounding of the loudest term
/// its row met, and the volume settles at p V / (R T) for 300 K.
///
/// `TJ2` in the standard library's own components stands on the same
/// wall, but a test cannot read the library, so the flow and the
/// start are chosen where the flat copy meets it too: at the flow of
/// the full tee, 1.363 kg/s, and a start of 293.15 K, Newton happens
/// to land, and a sweep over flow and start found this corner
/// (`m_in` 0.45, 310 to 360 K) refused without the ladder at every
/// point. The tolerance is not to the bit: the number is the
/// solver's, and 1e-9 relative is three orders below what a wrong
/// slope would leave and well above what the step control moves.
#[test]
fn a_slope_lost_in_the_rounding_of_an_energy_balance_is_read_from_further_away() {
    let enthalpy = |t: &str| {
        "R*((-10099.5016) + X*(((-176.796731) + ((-196.827561)*log(X))) + X*(5.00915511 \
         + X*(0.5*(-0.00576101373) + X*((1/3)*0.0000106685993 + X*(0.25*(-0.00000000794029797) \
         + 0.2*0.00000000000218523191*X))))))/X + 4333.833858403446 + 298609.6803431054"
            .replace('X', t)
    };
    let source = format!(
        "model W parameter Real V = 1e-3; parameter Real R = 8.31451/0.0289651159; \
         parameter Real T_in = 300; \
         Real p(start = 1e5, stateSelect = StateSelect.prefer); \
         Real T(start = 293.15, stateSelect = StateSelect.prefer); \
         Real d; Real h; Real u; Real m; Real U; Real m_in; Real m_out; Real h_in; \
         Real m2; Real Hb; \
         initial equation T = 330; \
         equation h = {}; h_in = {}; u = h - p/d; d = p/(R*T); m = V*d; U = m*u; \
         p = 1e5; m_in = 0.45; der(m) = m_in + m_out + m2; der(U) = Hb; \
         Hb = m_in*h_in + m_out*h + m2*h; m2 = 0; \
         annotation(experiment(StopTime = 1)); end W;",
        enthalpy("T"),
        enthalpy("T_in")
    );
    let result = run(&source);
    let column = result.columns.iter().position(|c| c == "m").expect("m");
    let m = result.rows.last().expect("a row")[column];
    let expected = 1e5 * 1e-3 / (8.31451 / 0.0289651159 * 300.0);
    assert!(
        ((m - expected) / expected).abs() < 1e-9,
        "m = {m}, p V / (R T) = {expected}"
    );
}

/// A three-phase diode bridge whose source star is grounded through a
/// megohm. The star's current moves the last row by a millionth of the
/// step, under the rounding of the diode currents it is summed with, so
/// the textbook step reads that rounding for a slope and the base
/// chattered through ten thousand events at the commutation near
/// t = 0.045. The zero ladder now reads a move inside a row's rounding
/// as no move and asks from further away. The load current is the one
/// the same bridge carries at a thousand ohms, where the column stood
/// clear of the rounding all along: 13.4721801090857 A.
#[test]
fn a_star_grounded_through_a_megohm_is_read_past_the_rounding_of_its_row() {
    let source = "model S parameter Real R = 1e6; parameter Real Ron = 1e-5; \
         parameter Real Goff = 1e-5; parameter Real V = sqrt(2)*110; \
         parameter Real pi = 3.141592653589793; \
         Real va[3]; Real vs; Real vp; Real vn; \
         Real sp[3](each start = 0); Real sn[3](each start = 0); \
         Real ip[3]; Real in_[3]; Real ia[3]; Real iload; \
         equation for k in 1:3 loop \
         va[k] - vs = V*sin(2*pi*50*time - (k - 1)*2*pi/3); \
         va[k] - vp = sp[k]*(if sp[k] < 0 then 1 else Ron); \
         ip[k] = sp[k]*(if sp[k] < 0 then Goff else 1); \
         vn - va[k] = sn[k]*(if sn[k] < 0 then 1 else Ron); \
         in_[k] = sn[k]*(if sn[k] < 0 then Goff else 1); \
         ia[k] = ip[k] - in_[k]; end for; \
         vp - vn = 20*iload; sum(ip) = iload; sum(in_) = iload; sum(ia) = vs/R; \
         annotation(experiment(StopTime = 0.1, Interval = 0.0002)); end S;";
    let result = run(source);
    let column = result
        .columns
        .iter()
        .position(|c| c == "iload")
        .expect("iload");
    let last = result.rows.last().expect("a row");
    assert!((last[0] - 0.1).abs() < 1e-9, "stopped at t = {}", last[0]);
    let iload = last[column];
    assert!(
        (iload - 13.4721801090857).abs() < 1e-9 * 13.5,
        "iload = {iload}"
    );
}

/// A loop that is singular only where an overlong implicit step
/// predicts its state. `x` decays at 1e4 per second; the first BDF
/// step's predictor overshoots it past -0.5, where the second equation
/// turns into a copy of the first and the loop's matrix loses its
/// rank. At every point the solution passes through, the loop is
/// regular and `a = b = cbrt((2 + x) / 2)`. Taking the singular matrix
/// for the end of the run refused a model that a shorter step solves;
/// the step is rejected instead, as for any other refusal a shorter
/// step mends.
#[test]
fn a_loop_singular_only_where_an_overlong_step_predicts_rejects_the_step() {
    let source = "model P Real x(start = 1, fixed = true); Real s; \
         Real a(start = 1); Real b(start = 1); \
         equation der(x) = -1e4*x; s = if x < -0.5 then 1 else 0; \
         a^3 + b^3 = 2 + x; \
         (1 - s)*(a^3 - b^3) + s*(a^3 + b^3 - 2 - x) = 0; \
         annotation(experiment(StopTime = 0.01)); end P;";
    let result = run_on(source, SolverMethod::Bdf).expect("runs");
    let column = |name: &str| result.columns.iter().position(|c| c == name).expect(name);
    let (x, a) = (column("x"), column("a"));
    let last = result.rows.last().expect("a row");
    assert!((last[0] - 0.01).abs() < 1e-12, "stopped at t = {}", last[0]);
    for row in &result.rows {
        let exact = ((2.0 + row[x]) / 2.0).cbrt();
        assert!(
            (row[a] - exact).abs() < 1e-9,
            "a = {} at t = {}, cbrt((2 + x) / 2) = {exact}",
            row[a],
            row[0]
        );
    }
}

/// A loop that runs out of Newton iterations only where an overlong
/// step predicts its state. `x` decays at 1e4 per second from 1, and
/// the loop `a*a + b*b = 2 + x, a = b` has `a = sqrt((2 + x) / 2)` at
/// every point the solution passes through. The first step's predictor
/// puts `x` far below -2, where the loop has no real root and the
/// iteration wanders for fifty steps. That ended the run, in the
/// implicit solver and in the explicit one before it, as a model
/// nobody could solve; the step is rejected instead, as a singular
/// loop's is, and under `Auto` the explicit solver hands the run to
/// the implicit one rather than ending it.
#[test]
fn a_loop_unconverged_only_where_an_overlong_step_predicts_rejects_the_step() {
    let source = "model U Real x(start = 1, fixed = true); \
         Real a(start = 1); Real b(start = 1); \
         equation der(x) = -1e4*x; a*a + b*b = 2 + x; a - b = 0; \
         annotation(experiment(StopTime = 0.01)); end U;";
    for method in [SolverMethod::Bdf, SolverMethod::Auto] {
        let result = run_on(source, method).expect("runs");
        let column = |name: &str| result.columns.iter().position(|c| c == name).expect(name);
        let (x, a) = (column("x"), column("a"));
        let last = result.rows.last().expect("a row");
        assert!((last[0] - 0.01).abs() < 1e-12, "stopped at t = {}", last[0]);
        for row in &result.rows {
            let exact = ((2.0 + row[x]) / 2.0).sqrt();
            assert!(
                (row[a] - exact).abs() < 1e-9,
                "a = {} at t = {}, sqrt((2 + x) / 2) = {exact}",
                row[a],
                row[0]
            );
        }
    }
}

/// A voltage read through the reciprocal of a nanohenry. The loop's
/// one row carries the rounding of `u` - an ulp of a thousand volts -
/// times 1e9, so the residual cannot fall below some 1e-4 however the
/// iteration steps, while the loudest number the row meets is 1e6 and
/// the loudness floor sits at 1e-9. The base refused it for a Newton
/// direction that does not descend. The floor now also asks how far
/// the row's coefficients carry the rounding of its unknowns, and the
/// block is solved: `u` to the last digits a double holds, and `w` to
/// what an ulp of `u` over a nanohenry leaves of it.
#[test]
fn a_row_reading_a_voltage_through_a_nanohenry_stops_at_the_rounding_it_carries() {
    let source = "model D3 parameter Real L = 1e-9; parameter Real V = 1000; \
         Real u(start = 1000); Real w; \
         equation w = 2 + 1e-3*sin(u); (u - V*cos(time))/L + 1e-9*u^2 = w; \
         annotation(experiment(StopTime = 0.01, Interval = 0.001)); end D3;";
    let result = run(source);
    let column = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().expect("a row");
    assert!((last[0] - 0.01).abs() < 1e-12, "stopped at t = {}", last[0]);
    let u = last[column("u")];
    assert!((u - 999.950000418665).abs() < 1e-9, "u = {u}");
    let w = last[column("w")];
    assert!((w - 2.0007977391698297).abs() < 1e-4, "w = {w}");
}
