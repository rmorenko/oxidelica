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
fn a_product_equation_is_matched_where_it_multiplies() {
    // The inverter's shape. `a = b * c` may be given to `a`, which
    // multiplies, or to `b`, which divides by `c` - and `c` is zero at
    // the start, so the second reading hands Newton a NaN residual
    // before it has taken a step. The equation mentions both, and the
    // matching used to take whichever came first.
    let result = run("model P Real c; Real a; Real b; \
         equation c = time; a = b * c; b = 1 - a; \
         annotation(experiment(StopTime=0.2, Interval=0.1)); end P;");
    let value = |name: &str| {
        let index = result.columns.iter().position(|c| c == name).unwrap();
        result.rows.last().unwrap()[index]
    };
    // a = c/(1 + c) at c = 0.2, and b = 1 - a.
    assert!((value("a") - 0.2 / 1.2).abs() < 1e-9, "a = {}", value("a"));
    assert!((value("b") - 1.0 / 1.2).abs() < 1e-9, "b = {}", value("b"));
}

#[test]
fn a_slope_worth_exactly_zero_is_not_the_cheap_pairing() {
    // The air gap's shape, in eleven lines. A mutual inductance matrix
    // whose off-diagonal a library sets to zero says the two windings
    // do not couple, so `psi2 = L12*i1 + L11*i2` determines `i2` and
    // says nothing whatever about `i1`. The matching ranked a slope
    // naming no other unknown as the cheapest pairing without asking
    // what that slope is worth, so this equation was handed `i1`, and
    // what solving it for `i1` needs is a division by `L12`.
    //
    // Judged with the parameter table in view the slope is the number
    // zero, which is the equation declining to mention the name at
    // all - the dearest pairing there is, and the matching then falls
    // on the one that exists. The check is the answer and not the
    // running: `i1 = sin(time)` is given, so `psi1 = L11*i1` exactly.
    let result = run("model Z parameter Real L11 = 2; parameter Real L12 = 0; \
         Real psi1(start = 1); Real psi2(start = 0); Real i1; Real i2; Real u; \
         equation psi1 = L11 * i1 + L12 * i2; psi2 = L12 * i1 + L11 * i2; \
         der(psi1) = u - i1; der(psi2) = -i2; i1 = sin(time); \
         annotation(experiment(StopTime=0.1, Interval=0.05)); end Z;");
    let value = |name: &str| {
        let index = result.columns.iter().position(|c| c == name).unwrap();
        result.rows.last().unwrap()[index]
    };
    let t = 0.1_f64;
    assert!((value("i1") - t.sin()).abs() < 1e-9, "i1 = {}", value("i1"));
    // The windings do not couple, so the second flux stays where the
    // second current is: at nothing.
    assert!((value("i2")).abs() < 1e-9, "i2 = {}", value("i2"));
    assert!(
        (value("psi1") - 2.0 * t.sin()).abs() < 1e-7,
        "psi1 = {}",
        value("psi1")
    );
}

#[test]
fn a_torn_block_does_not_divide_by_its_own_unknown() {
    // The heated resistor's shape, and the reason a whole family of
    // library models would not start. The loop is `v = R*i` with `R`
    // depending on a temperature the dissipated power drives, so the
    // block holds `i`, `R` and `T` together. Solved explicitly for
    // `R` the assignment reads `R := v/i`, and `i` begins at zero
    // because nothing has told it otherwise - so the whole inner
    // chain is NaN before Newton has taken a step, and the refusal
    // names the solver rather than the plan that divided.
    //
    // Carried in the tearing set instead, the same equations have an
    // answer, and this checks the answer rather than the running:
    // every one of the loop's equations has to hold at the values
    // reported, which is what a guessed start would not give.
    let result = run(
        "model H Real i(start = 0); Real v; Real R_actual; Real T(start = 288.15); Real Q; \
         parameter Real R = 100; parameter Real alpha = 1e-3; \
         parameter Real T_ref = 293.15; parameter Real G = 50; \
         equation v = 220 * sin(6.2831853 * time); v = R_actual * i; \
         R_actual = R * (1 + alpha * (T - T_ref)); \
         Q = v * i; Q = G * (T - 293.15); \
         annotation(experiment(StopTime=0.2, Interval=0.1)); end H;",
    );
    let value = |name: &str, row: usize| {
        let index = result.columns.iter().position(|c| c == name).unwrap();
        result.rows[row][index]
    };
    for row in [0, 1, 2] {
        let (v, i, r, t, q) = (
            value("v", row),
            value("i", row),
            value("R_actual", row),
            value("T", row),
            value("Q", row),
        );
        assert!((v - r * i).abs() < 1e-6, "v = {v}, R*i = {}", r * i);
        assert!(
            (r - 100.0 * (1.0 + 1e-3 * (t - 293.15))).abs() < 1e-6,
            "R = {r}, T = {t}"
        );
        assert!((q - v * i).abs() < 1e-6, "Q = {q}, v*i = {}", v * i);
        assert!(
            (q - 50.0 * (t - 293.15)).abs() < 1e-6,
            "Q = {q}, G*dT = {}",
            50.0 * (t - 293.15)
        );
    }
}

#[test]
fn a_divisor_that_reads_one_at_the_start_stays_an_explicit_assignment() {
    // The flux tubes' shape, and the third reading of the same rule.
    // `mu_r = 1 + (mu_i - 1 + c_a*B_N)/(1 + c_b*B_N + B_N^2)` divides
    // by a sum that *mentions* `B_N`, which is a block unknown with
    // no start and therefore one that reads zero - and the sum itself
    // reads exactly one there, so the assignment is perfectly safe.
    // Refused on the mention alone, `mu_r` went into the tearing set,
    // Newton started it at zero, `G_m = mu_0*mu_r*A/l` came out zero
    // and `R_m = 1/G_m` came out infinite before the first step.
    //
    // The answer is checked and not the running: every equation of
    // the chain has to hold at the values reported, and `mu_r` has to
    // be the saturation curve's value rather than whatever a start
    // happened to leave behind.
    let result = run(
        "model FluxShape Real Phi(start = 1e-6); Real B; Real B_N; Real mu_r; Real G_m; \
         Real R_m; Real V_m; Real y(start = 0); \
         parameter Real A = 1e-4; parameter Real l = 0.1; \
         parameter Real mu_i = 1210; parameter Real c_a = 3.5; \
         parameter Real c_b = 6.0; parameter Real B_max = 1.6; \
         equation B = Phi/A; B_N = abs(B/B_max); \
         mu_r = 1 + (mu_i - 1 + c_a*B_N)/(1 + c_b*B_N + B_N^2); \
         G_m = 1.25663706212e-6*mu_r*A/l; R_m = 1/G_m; V_m = Phi*R_m; \
         V_m = 10 + 5*sin(6.2831853*time); der(y) = Phi; \
         annotation(experiment(StopTime=0.2, Interval=0.1)); end FluxShape;",
    );
    let value = |name: &str, row: usize| {
        let index = result.columns.iter().position(|c| c == name).unwrap();
        result.rows[row][index]
    };
    for row in [0, 1, 2] {
        let (phi, b, b_n, mu_r, g_m, r_m, v_m) = (
            value("Phi", row),
            value("B", row),
            value("B_N", row),
            value("mu_r", row),
            value("G_m", row),
            value("R_m", row),
            value("V_m", row),
        );
        assert!((b - phi / 1e-4).abs() < 1e-6 * b.abs().max(1.0), "B = {b}");
        assert!((b_n - (b / 1.6).abs()).abs() < 1e-9, "B_N = {b_n}");
        let curve = 1.0 + (1209.0 + 3.5 * b_n) / (1.0 + 6.0 * b_n + b_n * b_n);
        assert!(
            (mu_r - curve).abs() < 1e-6 * curve,
            "mu_r = {mu_r}, curve = {curve}"
        );
        assert!(mu_r > 0.0, "mu_r = {mu_r} is not a permeability");
        assert!(
            (g_m - 1.25663706212e-6 * mu_r * 1e-4 / 0.1).abs() < 1e-15,
            "G_m = {g_m}"
        );
        assert!(
            (r_m - 1.0 / g_m).abs() < 1e-6 * r_m.abs(),
            "R_m = {r_m}, 1/G_m = {}",
            1.0 / g_m
        );
        assert!(
            (v_m - phi * r_m).abs() < 1e-6 * v_m.abs().max(1.0),
            "V_m = {v_m}, Phi*R_m = {}",
            phi * r_m
        );
    }
    // And the permeability is a permeability throughout: on the old
    // reading `mu_r` was torn and started at zero, which made `G_m`
    // zero and `R_m` infinite before any of this could be asked.
    assert!(
        value("mu_r", 0) > 1.0 && value("mu_r", 0) < 1210.0,
        "mu_r at t = 0 is {}",
        value("mu_r", 0)
    );
}

#[test]
fn a_guarded_division_stays_an_explicit_assignment() {
    // The other half of the rule above, and the half that is worth
    // six models. The standard library writes its static inductance
    // as `if abs(i) > eps then Psi/i else L_nominal`: a division by a
    // block unknown that begins at zero, and one that cannot be
    // reached where it would fail, because the model tested the
    // divisor itself. Refused along with the unguarded ones, the
    // Newton system grows until the magnetic examples' blocks come
    // back singular - so a guarded division is left where it was.
    //
    // Checked by the answer rather than by the running: on the branch
    // the guard sends the start to, `L` is the nominal value exactly,
    // and once current flows it is the ratio the division says.
    let result = run("model G Real i(start = 0); Real L; Real v; \
         parameter Real eps = 1e-6; parameter Real L_nom = 7; \
         equation v = 2 * time; i * 3 = v; \
         L = if abs(i) > eps then v / i else L_nom; \
         annotation(experiment(StopTime=0.2, Interval=0.1)); end G;");
    let value = |name: &str, row: usize| {
        let index = result.columns.iter().position(|c| c == name).unwrap();
        result.rows[row][index]
    };
    assert!(
        (value("L", 0) - 7.0).abs() < 1e-9,
        "at the start the guard holds: L = {}",
        value("L", 0)
    );
    // `i*3 = v` makes the ratio three wherever current flows.
    assert!(
        (value("L", 2) - 3.0).abs() < 1e-7,
        "L = {}, i = {}",
        value("L", 2),
        value("i", 2)
    );
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
fn index_reduction_differentiates_min_and_max_as_the_branch_they_are() {
    // `min(a, b)` is `if a < b then a else b`, and a constraint
    // holding one of them against a moving right-hand side has to be
    // differentiated through whichever side was actually selected.
    //
    // The number is the point rather than the flattening. With x
    // positive, `max(2*x, 3*x)` is `3*x`, so `3*x = t + 1` pins the
    // velocity at a third; a derivative taken from the wrong side of
    // the branch would give a half, and one taken as though the call
    // were flat would give a model with no solution at all. The `min`
    // half is asked for the same third from the other position in
    // the argument list: `min(3*x, 4*x)` is the first argument where
    // `max(2*x, 3*x)` was the second.
    let velocity = |body: &str, start: f64| {
        let result = run(&format!(
            "model B Real x(start = {start}); Real v; \
             equation der(x) = v; {body} = time + 1; \
             annotation(experiment(StopTime = 1.0, Interval = 0.5)); end B;"
        ));
        let index = result.columns.iter().position(|c| c == "v").unwrap();
        result.rows.last().unwrap()[index]
    };
    let by_max = velocity("max(2 * x, 3 * x)", 1.0 / 3.0);
    assert!(
        (by_max - 1.0 / 3.0).abs() < 1e-7,
        "max selects 3*x, so v = 1/3, not {by_max}"
    );
    let by_min = velocity("min(3 * x, 4 * x)", 1.0 / 3.0);
    assert!(
        (by_min - 1.0 / 3.0).abs() < 1e-7,
        "min selects 3*x, so v = 1/3, not {by_min}"
    );
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
    // it, so the refusal names the equation it could not evaluate and
    // its value instead. The equation, not the residual's number: a
    // number sends the reader counting through a block of eighteen
    // unknowns, and the text says outright what was divided by.
    assert_eq!(
        refused(
            "model D Real x; Real s(start = 0, fixed = true); \
             equation 1 / x = 0; der(s) = x; \
             annotation(experiment(StopTime = 1, Interval = 0.1)); end D;"
        ),
        "`1 / x = 0` of algebraic loop [\"x\"] is inf at t = 0, before any \
         Newton step: the equations cannot be evaluated at the values the \
         block starts from; and every value the residual reads is a finite number"
    );

    // The other way a loop fails: `x^2 * y = 1` where y is sin(t),
    // which is exactly zero at the start. The residual does not move
    // when x does, so there is no direction to step in - and that is a
    // different complaint from walking off to infinity.
    //
    // Named for what it is. A column of the Jacobian that is exactly
    // zero is not a matrix that came out ill conditioned; it is the
    // block saying it does not mention that unknown at all, and no
    // arithmetic on the matrix will find a step for it. "Singular
    // Jacobian" sent the reader to the solver, which is the one place
    // nothing was wrong, where the fault is the pairing upstream.
    assert_eq!(
        refused(
            "model S Real x; Real y; Real s(start = 0, fixed = true); \
             equation y = sin(time); x * x * y = 1; der(s) = x; \
             annotation(experiment(StopTime = 1, Interval = 0.1)); end S;"
        ),
        "the equations of algebraic loop [\"x\"] do not mention [\"x\"] at t = 0: \
         nothing in the block changes when it does, so no step determines it"
    );

    // And a column that reads dead is not thereby dead. A residual of
    // the order of 1e4 whose true coefficient on an unknown is 1e-9
    // moves by 1e-13 under the textbook step of 1e-8 - a twentieth of
    // an ulp of the number it is added to - and the subtraction gives
    // that back as an exact zero. The unknown is determined; what is
    // not determined is the difference, at that step. Asked again
    // from further away the coefficient is there, and the block
    // solves. This is what the switching loops of the library stand
    // on: an ideal diode's off conductance is exactly this ratio.
    let out = run("model T Real s(start = 0); Real y; \
         equation y = 14127.0 + 1e-9 * s; \
         y * y = (14127.0 + 2e-9) * (14127.0 + 1e-9 * s); \
         annotation(experiment(StopTime = 0.1, Interval = 0.1)); end T;");
    let y = out.rows.last().unwrap()[1];
    assert!((y - 14127.0).abs() < 1e-3, "y = {y}");

    // A block whose divisor is one of its own unknowns is not given
    // that division to do: `u = y/(y - x)` divides by a difference of
    // two unknowns that both begin at zero, so the plan carries the
    // equation in the tearing set rather than assigning through it.
    // The model still has no solution - `y = x` makes the divisor
    // identically zero, whatever the values - so the refusal stands;
    // what changed is that it names the whole block it could not
    // evaluate instead of naming an inner assignment that no longer
    // exists.
    assert_eq!(
        refused(
            "model N Real x; Real u; Real y; Real s(start = 0, fixed = true); \
             equation u = y / (y - x); y = x; u * x = 1; der(s) = x; \
             annotation(experiment(StopTime = 1, Interval = 0.1)); end N;"
        ),
        "`u = y / (y - x)` of algebraic loop [\"u\", \"x\"] is NaN at t = 0, before any \
         Newton step: the equations cannot be evaluated at the values the \
         block starts from; and every value the residual reads is a finite number"
    );
}

#[test]
fn a_column_flat_at_the_point_is_not_a_column_the_equations_lack_nor_an_answer() {
    // Two blocks the Jacobian cannot tell apart: in both, every entry
    // of one column is exactly zero at the values the iteration
    // starts from. Only one of them is a fact about the equations.
    //
    // `dp = m^2/2` with `dp` pinned at 2 has a slope of zero in `m`
    // at `m = 0` and nowhere else, so the tangent is simply blind
    // there and a secant over a whole unit sees the curvature. But
    // the equation is answered by `m = +-2`, and the two secants say
    // so: the one walked upward and the one walked downward do not
    // agree, so the column is not taken and the block is refused.
    // This is the price of refusing to guess - the same square that
    // makes `der(x)^2 = 4` undetermined makes this block
    // undetermined, and it costs this model its run. What the
    // refusal must not do is name the solver, because nothing is
    // wrong in the solver: it names the unknown that is answered
    // both ways.
    assert_eq!(
        refused(
            "model Q Real m(start = 0); Real dp; \
             equation dp = 0.5 * m * m; dp = 2.0 + 0.0 * time; \
             annotation(experiment(StopTime = 0.01, Interval = 0.01)); end Q;"
        ),
        "algebraic loop [\"m\"] has a solution on either side of [\"m\"] at t = 0: \
         the equations are answered both ways and do not say which was meant"
    );

    // The other kind, unmoved by the same repair. `i * R = v` with
    // the model card's `R` at zero has lost the unknown from the
    // residual altogether, and no distance brings it back, so the
    // block is refused in the words it was always refused in. This is
    // the half the secant must not swallow: a wrong number here would
    // be worse than the refusal.
    assert_eq!(
        refused(
            "model Z parameter Real R = 0; Real i; Real v; Real s(start = 0, fixed = true); \
             equation i * R = v; v = 1.0; der(s) = i; \
             annotation(experiment(StopTime = 1, Interval = 0.1)); end Z;"
        ),
        "the equations of algebraic loop [\"i\"] do not mention [\"i\"] at t = 0: \
         nothing in the block changes when it does, so no step determines it"
    );
}

#[test]
fn a_block_does_not_divide_by_a_state_that_starts_at_zero() {
    // `u*sin(a) = x - 1` solved explicitly for `u` divides by
    // `sin(a)`, and `a` starts at zero: the assignment runs before
    // Newton has moved anything, so it divides by exactly zero on its
    // first evaluation and the whole block comes out as a non-number.
    // The divisor guard saw only the block's own unknowns, and a
    // state is not one of those - it is a name that already holds a
    // value, which is why nothing was claimed about it and the
    // division went ahead. Read at the state's start the divisor is a
    // number, and a zero, so the equation joins the tearing set where
    // Newton carries it.
    //
    // Checked on the answer rather than on the model merely running:
    // at `a = 0` the equation reads `0 = x - 1`, so `x` is one and
    // `u` is `3 - 1`.
    let result = run(
        "model StateDiv Real a(start = 0, fixed = true); Real x; Real u; \
         equation der(a) = 1; u * sin(a) = x - 1; u + x * x = 3; \
         annotation(experiment(StopTime = 1, Interval = 0.1)); end StateDiv;",
    );
    let x = result.columns.iter().position(|c| c == "x").unwrap();
    let u = result.columns.iter().position(|c| c == "u").unwrap();
    let first = &result.rows[0];
    assert!(
        (first[x] - 1.0).abs() < 1e-9,
        "x starts at one, not {}",
        first[x]
    );
    assert!(
        (first[u] - 2.0).abs() < 1e-9,
        "u starts at two, not {}",
        first[u]
    );
}

#[test]
fn a_block_says_which_value_it_reads_is_not_a_number() {
    // A block whose unknowns are all torn has no inner assignment to
    // blame, and the refusal then said only that the residual could
    // not be evaluated - which sends the reader through a page of
    // arithmetic looking for the one term that is not a number. The
    // sharp-edged orifice's whole complaint is that page. But a
    // residual built from finite values cannot come out NaN, so
    // something it reads is one, and everything it reads was settled
    // before the block was reached. Named, the refusal points at a
    // value instead of at an expression.
    assert_eq!(
        refused(
            "model R Real q; Real x; Real s(start = 0, fixed = true); \
             equation q = sqrt(-1 - time); x * x + q = 4; der(s) = x; \
             annotation(experiment(StopTime = 1, Interval = 0.1)); end R;"
        ),
        "`(x * x) + q = 4` of algebraic loop [\"x\"] is NaN at t = 0, before any \
         Newton step: the equations cannot be evaluated at the values the block \
         starts from; it reads values that are not numbers: [\"q = NaN\"]"
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

#[test]
fn an_unconnected_input_of_a_connector_stands_at_its_start() {
    // A connector carrying an `input` that no `connect` names is
    // supplied from outside the model, exactly as an unconnected
    // `flow` carries nothing. The compiler counted it as an unknown
    // and refused the whole model as unbalanced - a value that was
    // never the model's to find. Fifteen models of the standard
    // library stood at this, the smallest of them
    // `StateGraph.Examples.Utilities.Source`.
    let result = run("package P \
           connector Outflow output Real out; input Real open(start = 3); end Outflow; \
           model M Outflow outflow; \
           equation outflow.out = 2*outflow.open; \
             annotation(experiment(StopTime = 1)); end M; \
         end P;");
    // The input stands at the start its declaration gave it, and the
    // output the model does state is computed from it.
    let out = result
        .columns
        .iter()
        .position(|c| c == "outflow.out")
        .unwrap();
    assert!(result.rows.len() > 1, "the model ran");
    assert_eq!(result.rows[0][out], 6.0, "outflow.out = 2 * 3");
}

#[test]
fn an_unconnected_causal_connector_stands_at_its_start() {
    // The same for a connector that is one value rather than a set of
    // members - `connector RealInput = input Real`, which is how every
    // signal of the standard library is written. A block whose input
    // the example never wires is waiting on its environment, not short
    // of an equation.
    let result = run("package P \
           connector RealInput = input Real; \
           model M RealInput u(start = 4); Real y; \
           equation y = u + 1; \
             annotation(experiment(StopTime = 1)); end M; \
         end P;");
    let y = result.columns.iter().position(|c| c == "y").unwrap();
    assert!(result.rows.len() > 1, "the model ran");
    assert_eq!(result.rows[0][y], 5.0, "y = 4 + 1");
}

#[test]
fn an_input_of_the_model_itself_stands_at_its_start() {
    // A block asked for on its own, rather than as a component of an
    // example, is the whole of the run: its own `input` is a value
    // handed down from a level above that is not there. The connection
    // joining it inward was read the other way round, so the top-level
    // name was left with nothing writing it and the whole block was
    // refused as unbalanced - the wall `LimitedPI` of the controlled DC
    // drives stood at, and three more machine utilities behind it.
    let result = run("package P \
           connector RealInput = input Real; \
           connector RealOutput = output Real; \
           block Gain RealInput u; RealOutput y; parameter Real k = 2; \
           equation y = k*u; end Gain; \
           block Wrap RealInput u(start = 3); RealOutput y; Gain gain; \
           equation connect(u, gain.u); connect(gain.y, y); \
             annotation(experiment(StopTime = 1)); end Wrap; \
         end P;");
    let at = |name: &str| {
        let index = result.columns.iter().position(|c| c == name).unwrap();
        result.rows[0][index]
    };
    // The input stands at the start its declaration gave it, and the
    // value travels through the component and out again.
    assert_eq!(at("u"), 3.0);
    assert_eq!(at("gain.y"), 6.0, "the gain of two over a start of three");
    assert_eq!(at("y"), 6.0, "and out through the block's own output");
}

#[test]
fn a_stream_carrying_nothing_still_says_which_enthalpies_are_equal() {
    // `semiLinear(m, h_port, h)` is `h_port*m` one way and `h*m` the
    // other, so at `m = 0` it is zero whichever way it is read and
    // says nothing at all about `h_port`. A component connected to
    // one whose flow has stopped therefore has an enthalpy nothing
    // determines, and the block it sits in is underdetermined - which
    // is what nine models of the standard library were refused for.
    // The language says what the missing equation is: where the flow
    // is zero the two enthalpies are equal.
    let result = run("model S Real m; Real h_port; Real h; Real H; \
         equation m = 0; h = 300; H = semiLinear(m, h_port, h); H = 0; \
         annotation(experiment(StopTime=0.01, Interval=0.01)); end S;");
    let port = result.columns.iter().position(|c| c == "h_port").unwrap();
    // The number, not the fact that it compiled: with nothing
    // flowing, the port carries the enthalpy of what it is joined to.
    assert!(
        (result.rows[0][port] - 300.0).abs() < 1e-9,
        "{}",
        result.rows[0][port]
    );
}

#[test]
fn a_block_that_starts_where_it_cannot_be_evaluated_is_tried_from_elsewhere() {
    // `R_m = 1/G_m` names a block whose unknown starts at the zero its
    // declaration left it, and a reciprocal has nothing to say there.
    // That is not a fault of the model and not one the plan can route
    // around - the division is in the equation, not in an assignment
    // the plan chose - so the block is started again from off the
    // zero. The number is the check: `G_m` is the reciprocal of the
    // reluctance the rest of the model fixes.
    let result = run(
        "model S Real G_m; Real R_m; Real x(start = 1, fixed = true); \
         equation der(x) = 0; R_m = 2 + x; R_m = 1 / G_m; \
         annotation(experiment(StopTime = 0.01, Interval = 0.01)); end S;",
    );
    let at = |name: &str| result.rows[0][result.columns.iter().position(|c| c == name).unwrap()];
    assert!((at("R_m") - 3.0).abs() < 1e-9, "R_m = {}", at("R_m"));
    assert!((at("G_m") - 1.0 / 3.0).abs() < 1e-9, "G_m = {}", at("G_m"));
}

#[test]
fn an_outer_with_no_inner_above_it_gets_one_at_the_top() {
    // A helper of a library is written to sit inside a model that
    // holds the shared instance, and says `outer System system` or
    // `outer World world` on that understanding. Checked on its own -
    // which is what a library check does to every class it finds -
    // it has nothing above it at all, and the declaration answers to
    // nobody. The language says to declare the missing `inner` at the
    // top with the class's own defaults and say so (MLS 5.4); read as
    // an error instead, thirteen `Utilities` and `BaseClasses` models
    // of the standard library were refused outright.
    let result = run("package P \
           model Seed parameter Real id = 3; end Seed; \
           model Helper outer Seed s; Real y; equation y = s.id; end Helper; \
           model Top Helper h; annotation(experiment(StopTime = 1)); end Top; \
         end P;");
    let at = |name: &str| result.rows[0][result.columns.iter().position(|c| c == name).unwrap()];
    // The minted instance carries the defaults its class declares, and
    // the helper reads them through the `outer` name.
    assert_eq!(at("h.y"), 3.0, "the default of the minted `inner Seed`");
}

#[test]
fn a_residual_at_the_rounding_floor_of_its_own_equation_is_solved() {
    // An equation whose two sides are near a thousand million agree
    // to every digit double precision holds when their difference is
    // around 1e-7: that is where the rounding of 1e9 lands, and no
    // iteration can go under it. Judged against the unknown alone -
    // a volt - the test demanded 1e-10 and was never going to be
    // met, so Newton stepped by nothing and spent its whole budget
    // reproducing one residual before refusing to converge.
    //
    // This is the shape of the Zener diode in
    // `Modelica.Electrical.Analog.Examples.OvervoltageProtection`,
    // reduced to a single equation: an exponential in millivolts
    // against a current of a thousand million.
    let result = run("model Z Real v(start = 0); Real i; \
         equation i = 0.7*exp(-(v + 5.1)/(0.74*0.04)); \
           i = (v + 5.7)*1e9 + 0*time; \
         annotation(experiment(StopTime = 0.001, Interval = 0.001)); end Z;");
    let v = result.columns.iter().position(|c| c == "v").unwrap();
    // The solution is where the exponential meets the line, a little
    // above -5.7 volts; what the test is about is that the block was
    // solved at all, so the value is checked for being the root
    // rather than for a digit that the arithmetic sets.
    let value = result.rows[0][v];
    assert!(
        (value + 5.64).abs() < 0.01,
        "the loop was solved at v = {value}"
    );
}

/// A body the run walks cannot raise: a walk that fails answers with a
/// number that is not one and leaves its reason behind for whoever
/// evaluated the point. Inside an algebraic block that reader is the
/// block itself, and it used to refuse before reading - so a model
/// whose own `assert` had fired was reported as a set of equations
/// that "cannot be evaluated", with the sentence the model wrote to
/// explain itself dropped on the floor.
///
/// This is the shape of the whole `NaN before any Newton step` row in
/// the fluid libraries: `solveOneNonlinearEquation` says outright that
/// the bracket it was handed does not contain a root, and the compiler
/// answered with the solver's name instead of the library's sentence.
#[test]
fn a_function_that_refused_inside_a_loop_says_why() {
    // The loop in the body is what keeps the call standing: a body
    // simple enough to inline is evaluated where the equation is, and
    // its assert raises in the ordinary way. Only a call the run walks
    // can lose its reason, which is why the model needs one.
    let refused = compile(
        &parse_model(
            "model W \
               function guard \
                 input Real u; output Real y; \
                 protected Real a; \
                 algorithm \
                   assert(u > 1.0, \"guard: u must exceed one\"); \
                   a := u; \
                   while a < 10.0 loop a := a + 1.0; end while; \
                   y := u * u - 4.0; \
               end guard; \
               Real x; Real s(start = 0, fixed = true); \
             equation guard(x) + x + 3.0 = 0; der(s) = x; \
             annotation(experiment(StopTime = 1, Interval = 0.5)); end W;",
        )
        .unwrap(),
    )
    .expect_err("the assert fires at the point the block starts from")
    .to_string();
    assert_eq!(
        refused,
        "`(W.guard(x) + x) + 3 = 0` of algebraic loop [\"x\"] is NaN at t = 0, \
         before any Newton step, because a function it calls could not be \
         walked: guard: u must exceed one"
    );
}

/// A model refusing in prose has its whole sentence carried, and the
/// numbers in it filled in.
///
/// `Modelica.Utilities.Streams.error(text)` is how the standard
/// library shouts, and the text is built by joining literals to
/// `String(x)` of what the run knows. A call standing on its own is
/// read for its value, so the first literal piece of the sentence was
/// refused as "a String has no value a step can carry" and the reason
/// the library wrote was replaced by a complaint about its spelling.
/// With the call taken as the assert it is, the sentence arrives
/// whole - and because the walk is standing in the frame that holds
/// the values, `String(u)` comes out as the number rather than as the
/// `?` a reading off the run can only give.
#[test]
fn a_model_that_refuses_in_prose_is_quoted_whole_with_its_numbers() {
    let refused = compile(
        &parse_model(
            "model P \
               function shout \
                 input Real u; output Real y; \
                 protected Real a; \
                 algorithm \
                   a := u; \
                   while a < 10.0 loop a := a + 1.0; end while; \
                   if u < 1.0 then \
                     Modelica.Utilities.Streams.error(\
                       \"u is \" + String(u) + \", which is below one\"); \
                   end if; \
                   y := u * u - 4.0; \
               end shout; \
               Real x; Real s(start = 0, fixed = true); \
             equation shout(x) + x + 3.0 = 0; der(s) = x; \
             annotation(experiment(StopTime = 1, Interval = 0.5)); end P;",
        )
        .unwrap(),
    )
    .expect_err("the shout fires at the point the block starts from")
    .to_string();
    assert_eq!(
        refused,
        "`(P.shout(x) + x) + 3 = 0` of algebraic loop [\"x\"] is NaN at t = 0, \
         before any Newton step, because a function it calls could not be \
         walked: u is 0, which is below one"
    );
}

#[test]
fn index_reduction_differentiates_atan2_by_both_of_its_arguments() {
    // A constraint written with `atan2` had no derivative rule and the
    // model was refused as structurally singular. The rule is the one
    // in every table - `(x*dy - y*dx) / (x^2 + y^2)` - and the point
    // of the test is the number rather than the flattening.
    //
    // `atan2(x, 2) = time / 2` with `der(x) = v` makes the velocity
    // come out of the differentiated constraint alone: the second
    // argument is a number, so `2*v / (x^2 + 4) = 1/2`, which is
    // `v = (x^2 + 4) / 4`. The path is `x = 2*tan(time/2)`, so at one
    // second the velocity is a quarter of `4*tan(0.5)^2 + 4`. Taken
    // by the first argument only - dropping the `x*dy` half, or
    // differentiating as though the call were `atan(x/2)` with a
    // constant denominator - the number comes out elsewhere.
    let result = run("model B Real x(start = 0); Real v; \
         equation der(x) = v; atan2(x, 2) = time / 2; \
         annotation(experiment(StopTime = 1.0, Interval = 0.5)); end B;");
    let index = result.columns.iter().position(|c| c == "v").unwrap();
    let v = result.rows.last().unwrap()[index];
    let x = 2.0 * (0.5f64).tan();
    let expected = (x * x + 4.0) / 4.0;
    assert!(
        (v - expected).abs() < 1e-5,
        "atan2 pins v to {expected}, not {v}"
    );
}

#[test]
fn a_branch_that_settles_a_sign_differentiates_the_abs_inside_it() {
    // `regRoot2` writes `if x <= -x_small then -sqrt(abs(x))`, and the
    // whole Fluid family of derivative tests was refused because `abs`
    // has no derivative rule. It needs none inside that branch: the
    // branch is reached only when `x <= -x_small`, and `x_small` is a
    // parameter with a positive number, so `x` is strictly negative
    // there and `abs(x)` is `-x`.
    //
    // The number is the point. At `time = 0.02` the path is
    // `x = -1.98`, so `y = -sqrt(1.98)` and its derivative is
    // `0.5/sqrt(1.98) = 0.35533452725935...`, both to every digit
    // below. A rule that guessed `sign(x)*der(x)` would agree here and
    // be wrong at zero, which is why nothing of the sort is used: the
    // sign comes from the condition, not from the value.
    //
    // The parameter form is the one that matters. Written with the
    // literal `-0.01` it passed while `-x_small` still refused, because
    // the proof was available to one of the compiler's two
    // differentiation targets and not to the other.
    let result = run(
        "model A parameter Real x_small = 0.01; Real x; Real y; Real yd; \
         equation x = time - 2.0; \
         y = if x >= x_small then sqrt(x) else if x <= -x_small then -sqrt(abs(x)) else 0.0; \
         yd = der(y); \
         annotation(experiment(StopTime = 0.02, Interval = 0.01)); end A;",
    );
    let at = |name: &str| {
        let index = result.columns.iter().position(|c| c == name).unwrap();
        result.rows.last().unwrap()[index]
    };
    let x = -1.98f64;
    assert!((at("y") - -x.abs().sqrt()).abs() < 1e-9, "y = {}", at("y"));
    assert!(
        (at("yd") - 0.5 / x.abs().sqrt()).abs() < 1e-6,
        "yd = {}",
        at("yd")
    );
}
