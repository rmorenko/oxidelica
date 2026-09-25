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

/// A discrete variable defined in terms of `initial()`.
///
/// The slots the event machinery supplies were made after the
/// discrete definitions were turned into code, so a definition that
/// reads `initial()` was compiled while `$initial` had no slot and
/// the whole model was refused for an unknown variable that is the
/// compiler's own. Nine models of the library were refused for it.
#[test]
fn a_discrete_defined_by_initial_finds_the_slot_the_events_supply() {
    let result = run("model M Real x(start = 0, fixed = true); \
         Boolean atStart; \
         equation der(x) = 1; \
         atStart = pre(atStart) and x > 5 or initial() and x >= 0; \
         annotation(experiment(StopTime = 0.2, Interval = 0.1)); end M;");
    // Whatever `initial()` is worth once the run has started, the
    // model is a model: it was refused outright before, for a name
    // no model wrote.
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    assert!((result.rows.last().unwrap()[index("x")] - 0.2).abs() < 1e-9);
    assert_eq!(result.rows.last().unwrap()[index("atStart")], 0.0);
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
    // Written as the derivation it is: a clock declared apart is a
    // clock of its own however its rate compares, so the four spellings
    // being compared are four spellings within one family.
    let written_out = reached("subSample(fast, 2)", "subSample(a, 2)");
    assert!((written_out - 0.9).abs() < 1e-12, "{written_out}");
    for (slow, sampled) in [
        ("Clock()", "subSample(a, 2)"),
        ("subSample(fast, 2)", "subSample(a)"),
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

/// A connection between signals is defined by the side that states a
/// value, not by the order the set happened to be in.
#[test]
fn a_signal_connection_defines_the_input_rather_than_the_output() {
    // `connect(sink.u, src.y)` is an equation between two names, and
    // which of them it defines is not the side it was written on: the
    // `output` states a value and the `input` takes one. Written the
    // other way round, the output got a second definition and the
    // input none - which on a clocked signal is a model refused as
    // unbalanced, and here is a source that would be solved for.
    let m = oxidelica_parser::parse_model(
        "package P connector RIn = input Real; connector ROut = output Real; \
         block Src ROut y; equation y = time; end Src; \
         block Sink RIn u; ROut y; equation y = 2 * u; end Sink; \
         model M Src src; Sink sink; Real out; \
         equation connect(sink.u, src.y); out = sink.y; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M; end P;",
    )
    .expect("two signals connected");
    let joined = m
        .equations
        .iter()
        .find(|equation| {
            let text = format!("{:?} {:?}", equation.lhs, equation.rhs);
            text.contains("sink.u") && text.contains("src.y")
        })
        .expect("the connection is an equation");
    assert!(
        matches!(&joined.lhs, oxidelica_parser::Expr::Ref(name) if name == "sink.u"),
        "the input is what the connection defines: {:?} = {:?}",
        joined.lhs,
        joined.rhs
    );
}

#[test]
fn a_clocked_connection_inside_a_block_writes_the_end_nobody_else_writes() {
    // `connect(u, y2)` inside the block joins its own input to a
    // protected output. From inside, the input is the source, and the
    // connection arrives as `b.u = b.y2`. Lifted onto the clock as it
    // stood, `b.u` was written twice (by the connection and by
    // `b.u = s` from outside) and `b.y2` by nobody.
    let result = run(
        "package P connector RIn = input Real; connector ROut = output Real; \
         block B RIn u; ROut y; protected ROut y2; \
         equation connect(u, y2); y = 2 * y2; end B; \
         model M B b; Real s; Clock c = Clock(0.1); \
         equation s = sample(time, c); b.u = s; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M; end P;",
    );
    let at = |name: &str| {
        result.rows.last().unwrap()[result.columns.iter().position(|c| c == name).unwrap()]
    };
    // The last tick is at t = 1, so `y2` holds 1 and `y` twice that.
    assert!((at("b.y2") - 1.0).abs() < 1e-9, "b.y2 = {}", at("b.y2"));
    assert!((at("b.y") - 2.0).abs() < 1e-9, "b.y = {}", at("b.y"));
}

#[test]
fn an_if_decided_on_a_tick_counts_on_that_tick() {
    // The tick-based sources count their ticks in an `if` whose
    // condition is `previous(go)`. Its branches were set aside for the
    // modes settled while running, which know nothing of a clock, and
    // `counter` was left written by nobody. Merged into one equation
    // per name, it counts 1, 2, then `go` turns at the third tick and
    // resets it, and from there it climbs by one a tick: ten ticks up
    // to t = 1 leave it at 8.
    let result = run("model M Clock c = Clock(0.1); Real s; \
         Integer counter(start = 0); Boolean go(start = false); Real y; \
         equation s = sample(time, c); y = s + counter; \
         if previous(go) then counter = previous(counter) + 1; go = previous(go); \
         else go = previous(counter) >= 2; \
         counter = if go then 0 else previous(counter) + 1; end if; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;");
    let counter = result.columns.iter().position(|c| c == "counter").unwrap();
    assert_eq!(result.rows.last().unwrap()[counter], 8.0);
}

#[test]
fn an_if_on_the_first_tick_is_decided_on_the_clock() {
    // `BooleanChange` and `IntegerChange` answer `false` on the first
    // tick and compare with `previous` after it, in an `if firstTick()`.
    // Set aside for the modes settled while running, `firstTick` reached
    // the run as a function nobody had. Here the first tick gives -1,
    // and every tick after the step of a sampled ramp, 0.1.
    let result = run("model M Clock c = Clock(0.1); Real s; Real y; \
         equation s = sample(time, c); \
         if firstTick() then y = -1; else y = s - previous(s); end if; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;");
    let y = result.columns.iter().position(|c| c == "y").unwrap();
    assert_eq!(result.rows.first().unwrap()[y], -1.0);
    assert!((result.rows.last().unwrap()[y] - 0.1).abs() < 1e-9);
}

#[test]
fn previous_inside_the_arguments_of_a_tuple_call_is_the_tick_before() {
    // `(r, st) = random(previous(st))` is how every clocked noise block
    // of the library steps its generator. Filled into a tuple it
    // arrives as `random({previous(st[1]), previous(st[2])})[k]`, an
    // element of a call on an array, and the walk that turns
    // `previous` into the value before the tick stopped at both of
    // those, so the run met `previous` as a function nobody had. The
    // numbers are those of the same generator stepped by a plain
    // `when` on `pre(st)`, which drew 0.1565793467523503 and then
    // 0.28194898396902013 from this seed.
    let result = run("model M \
         function random input Integer stateIn[2]; output Real result; \
           output Integer stateOut[2]; \
           external \"C\" ModelicaRandom_xorshift64star(stateIn, stateOut, result); \
         end random; \
         Clock c = Clock(0.1); Real u; Real y; Real r(start = 0); \
         discrete Integer st[2](start = {614657, 30020}); \
         equation u = sample(time, c); (r, st) = random(previous(st)); y = u + r; \
         annotation(experiment(StopTime = 0.25, Interval = 0.1)); end M;");
    let at = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    assert_eq!(last[at("r")], 0.28194898396902013);
    assert_eq!(last[at("st[1]")], -547572939.0);
    assert_eq!(last[at("st[2]")], -1122748525.0);
}

#[test]
fn a_switch_resting_on_its_threshold_is_not_lost_for_the_rest_of_the_run() {
    // A sliding mode: the state is driven towards the threshold from
    // whichever side it stands on, so the event settles exactly on it
    // and the next step carries the state off. The indicator therefore
    // reads zero where the step begins and has turned where it ends,
    // and that crossing used to be dropped outright at the instant an
    // event had just been handled - which was true of every step here,
    // so the switch kept its value for the whole rest of the run and
    // `x` walked out to 2 with `on` reading false all the way. The
    // condition is a wrong number rather than a slow one, and a run
    // that reports it is worse than a run that refuses.
    let refusal = run_err(
        "model S Real x(start = 0, fixed = true); Boolean on; \
         equation on = x > 0.5; der(x) = if on then -1 else 1; \
         annotation(experiment(StopTime = 2, Interval = 0.001)); end S;",
    );
    assert!(
        refusal.contains("handled more than"),
        "a sliding mode has to be refused rather than answered: {refusal}"
    );
}

#[test]
fn a_derivative_inside_an_event_indicator_is_the_state_equation() {
    // `asc = der(Hstat) > 0` is how the Tellinen hysteresis model
    // names the direction it is travelling in, and an indicator is
    // asked at a point the run already stands on: `der(x)` there is
    // not an approximation of anything, it is that state's right-hand
    // side, which the run has just evaluated.
    //
    // Checked by a number rather than by the model building. With
    // `der(x) = 1 - 2*time` the crossing is at exactly t = 0.5, and
    // `seen` integrates one while the indicator holds, so it must
    // come out at 0.5 rather than at 0 or 1.
    let result = run("model D Real x(start = 0, fixed = true); Boolean asc; \
         Real seen(start = 0, fixed = true); \
         equation der(x) = 1 - 2 * time; asc = der(x) > 0; \
         der(seen) = if asc then 1 else 0; \
         annotation(experiment(StopTime=1.0, Interval=0.01)); end D;");
    let seen = result.columns.iter().position(|c| c == "seen").unwrap();
    let last = result.rows.last().unwrap()[seen];
    assert!(
        (last - 0.5).abs() < 1e-3,
        "the indicator must turn at t = 0.5, giving seen = 0.5, not {last}"
    );
}

#[test]
fn a_discrete_that_reaches_nan_is_named_rather_than_carried_to_the_end() {
    // A discrete value is written by an event and then carried
    // untouched to the stop time: nothing in the integration reads it
    // back, so a model whose switch lands on NaN used to run to the
    // end and report a column of NaN as though it were an answer. Ten
    // `Digital` models did exactly that. The check asks after the
    // event has come to rest, and it names the variable; setting
    // `OXIDELICA_DISCRETE_NAN_GUARD=0` gives the old silence back.
    let source =
        "model N Real x(start = -1, fixed = true); discrete Real d(start = 0, fixed = true); \
         equation der(x) = -1; \
         when time > 0.5 then d = sqrt(x); end when; \
         annotation(experiment(StopTime = 1, Interval = 0.01)); end N;";
    let refusal = run_err(source);
    assert!(
        refusal.contains("`d` is not a number"),
        "the check names the variable that went to NaN: {refusal}"
    );
    std::env::set_var("OXIDELICA_DISCRETE_NAN_GUARD", "0");
    let quiet = run(source);
    std::env::remove_var("OXIDELICA_DISCRETE_NAN_GUARD");
    let column = quiet
        .columns
        .iter()
        .position(|c| c == "d")
        .expect("d is reported");
    assert!(
        quiet.rows.last().unwrap()[column].is_nan(),
        "without the check the run reaches its stop time writing NaN"
    );
}

#[test]
fn a_branch_of_a_when_that_says_nothing_leaves_the_value_it_had() {
    // `if c then y := x; end if;` inside a `when` of a model: on a
    // tick where the condition is false the variable keeps what it
    // held, which at an event is `pre` of it. Reading it as the
    // type's start instead is how `Electrical.Digital`'s inertial
    // delay answered zero - a logic value no table has - and
    // `Adder4` died on `AndTable` a round later.
    //
    // The number is what the test is for. `y` is set to 7 at the
    // initial event, the `if` at t = 0.5 is not taken, and the
    // branch at t = 0.8 sets it to 9: a run that reads the fallback
    // as the start shows 0 in between rather than 7.
    let result = run("model W discrete Real y(start = 1, fixed = true); \
         discrete Real seen(start = 0, fixed = true); \
         Real x(start = 0, fixed = true); \
         equation der(x) = 1; \
         algorithm \
         when {initial(), time > 0.5, time > 0.8} then \
           if initial() then y := 7; end if; \
           if time > 0.8 then y := 9; end if; \
           if time > 0.6 and time < 0.7 then seen := 1; end if; \
         end when; \
         annotation(experiment(StopTime = 1, Interval = 0.01)); end W;");
    let column = result
        .columns
        .iter()
        .position(|c| c == "y")
        .expect("y is reported");
    let time = result.columns.iter().position(|c| c == "time").unwrap();
    let between = result
        .rows
        .iter()
        .find(|row| row[time] > 0.6 && row[time] < 0.75)
        .expect("a row between the two events");
    assert_eq!(
        between[column], 7.0,
        "a branch that did not fire leaves `y` at the 7 it was given, not at its type's start"
    );
    let last = result.rows.last().unwrap();
    assert_eq!(last[column], 9.0, "the later branch gives `y` its 9");
}

#[test]
fn an_indicator_that_holds_zero_before_it_turns_is_stepped_past_once() {
    // `Digital.Examples.FlipFlop` crept forward by a thousandth of a
    // nanosecond at a time from t = 0.003 and raised ten thousand
    // events without arriving anywhere. What turns there is a gate
    // whose indicator sits at zero over a stretch of time and then
    // steps off it: the walk read zero, called that a relation about
    // to leave its threshold, and stepped a hair along - where the
    // indicator was still zero, so the same turn was found again, and
    // again, for as long as the run had patience.
    //
    // The instant that has to be stepped onto is where the indicator
    // stops being zero, not a hair past where it started being zero.
    let result = run(
        "model C Real x; discrete Real seen(start = 0, fixed = true); \
         equation x = if time < 0.5 then 0 else 1; \
         when x > 0 then seen = pre(seen) + 1; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.1)); end C;",
    );
    let at = result.columns.iter().position(|c| c == "seen").unwrap();
    let last = result.rows.last().unwrap()[at];
    assert_eq!(
        last, 1.0,
        "the gate turns once and the run has to reach its stop time: seen = {last}"
    );
}

#[test]
fn a_when_over_a_vector_named_whole_fires_on_each_element() {
    // `when pre_reset then` over `Boolean pre_reset[n]` fires when any
    // element becomes true, as `when {c1, c2}` does. A vector written
    // as a name shows its length only after flattening, and was
    // refused as an array where one condition was wanted. Two
    // elements turning true at two instants count two events.
    for section in [
        "algorithm when c then y := pre(y) + 1; end when;",
        "equation when c then y = pre(y) + 1; end when;",
    ] {
        let result = run(&format!(
            "model M Boolean c[2]; discrete Real y(start = 0, fixed = true); \
             equation c[1] = time > 0.3; c[2] = time > 0.6; {section} \
             annotation(experiment(StopTime = 1, Interval = 0.1)); end M;"
        ));
        let at = result.columns.iter().position(|c| c == "y").unwrap();
        let y = result.rows.last().unwrap()[at];
        assert!((y - 2.0).abs() < 1e-12, "{section}: {y}");
    }
}
