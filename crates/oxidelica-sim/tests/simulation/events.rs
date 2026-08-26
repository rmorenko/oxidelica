//! Events: when a `when` fires, what `reinit` does, and where a crossing lands.

use super::shared::*;
use oxidelica_parser::parse_model;
use oxidelica_sim::{compile, SolverMethod};

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
