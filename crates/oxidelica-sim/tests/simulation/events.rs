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

/// A block that divides by what `when initial()` writes is solved
/// after the clause, not before it. `Blocks.Sources.TimeTable` gives
/// its line `y = a*time + b` that way, and before the clause fired
/// the table answered zero: the regularity check and the initial
/// event's first point both solved the block on the template, met
/// `1/0` and refused a model whose equations are fine - which is how
/// `TestSharpEdgedOrifice` was refused. The root is the one of
/// `x^3 + x - 1/b = 1e-5*x` at `b = 1.2e5`.
#[test]
fn a_block_reads_what_when_initial_writes() {
    let result = run("model M discrete Real b; Real x(start = 1); Real y; \
         equation y = x * x * x + x - 1 / b; y = 1e-5 * x; \
         when initial() then b = 1.2e5; end when; \
         annotation(experiment(StopTime = 0.2, Interval = 0.1)); end M;");
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let x = result.rows[0][index("x")];
    assert!((x - 8.333416666920152e-6).abs() < 1e-15, "x(0) = {x}");
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

/// A sample whose instant is also the instant of a crossing fires there,
/// and not at the end of whatever step the run takes next. The two
/// coincide by construction in a rectifier whose current turns on the
/// zero of its 50 Hz source, and a `Mean` sampled every 10 ms beside it
/// sampled at 0.0511 instead of 0.05 and read 102.02 for 99.035.
#[test]
fn a_sample_due_on_the_instant_of_a_crossing_fires_on_it() {
    let source = "model M Real x(start = 0, fixed = true); \
         discrete Real at(start = 0, fixed = true); Integer m(start = 0, fixed = true); \
         equation der(x) = 1; \
         when sample(0.01, 0.01) then at = time; end when; \
         when {sin(314.1592653589793*time) < 0, sin(314.1592653589793*time) > 0} then \
           m = pre(m) + 1; end when; \
         annotation(experiment(StopTime=0.1, Interval=0.0002, Tolerance=1e-6)); end M;";
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf] {
        let result = run_on(source, method).unwrap();
        let at = result.columns.iter().position(|c| c == "at").unwrap();
        for row in &result.rows {
            let ticks = row[at] / 0.01;
            assert!(
                (ticks - ticks.round()).abs() < 1e-6,
                "{method:?}: a sample fired at {} by t = {}",
                row[at],
                row[0]
            );
        }
    }
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
fn a_clock_written_in_the_condition_of_a_when_is_a_clock() {
    // `when Clock(0.1) then` writes its clock in place, with no `Clock`
    // declared anywhere. Asking the declarations alone, the pass that
    // reads clocked partitions gave up before it reached the clause, and
    // the run met `previous` as a function nobody had. The ticks are at
    // 0, 0.1, ..., 0.5, so `x` ends at 0.1 times the sum of `sin` over
    // them - the number the same clause gives under a declared clock.
    let result = run(
        "model M Real n = sin(time); discrete Real x(start = 0, fixed = true); \
         equation when Clock(0.1) then x = previous(x) + 0.1*n; end when; \
         annotation(experiment(StopTime = 0.5, Interval = 0.5)); end M;",
    );
    let x = result.columns.iter().position(|c| c == "x").unwrap();
    let expected: f64 = (0..=5).map(|k| 0.1 * (0.1 * k as f64).sin()).sum();
    assert!((result.rows.last().unwrap()[x] - expected).abs() < 1e-9);
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

#[test]
fn clocks_that_never_tick_together_owe_each_other_no_order() {
    // The clocked library's `BackSample` example: a clock shifted four
    // thirds of an interval late, and a `backSample` of it that lands
    // back on the root's grid. Each reads the other, which is a circle
    // on paper and none at any instant, since the two never fire at
    // once. The shifted clock ticks at 0.02/3 past each base tick from
    // 0.02667 on, and the one sampled back from it at 0.04, 0.06, 0.08.
    let result = run("model M Clock c = Clock(0.02); \
         Real x; Real y1(start = 7); Real y2; Real h1; Real h2; \
         equation x = sample(time, c); \
         y1 = shiftSample(x, 4, 3); \
         y2 = backSample(y1, 4, 3); \
         h1 = hold(y1); h2 = hold(y2); \
         annotation(experiment(StopTime = 0.09, Interval = 0.001)); end M;");
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let at = |time: f64, name: &str| {
        let row = result
            .rows
            .iter()
            .rev()
            .find(|row| row[0] <= time + 1e-12)
            .unwrap();
        row[index(name)]
    };
    // Before the shifted clock's first tick both hold the start; the
    // clock sampled back first fires at 0.04 on the value 0.02 the
    // shifted one took at 0.02667.
    assert_eq!(at(0.026, "h1"), 7.0);
    assert!((at(0.03, "h1") - 0.02).abs() < 1e-12);
    assert_eq!(at(0.039, "h2"), 7.0);
    assert!((at(0.041, "h2") - 0.02).abs() < 1e-12);
    assert!((at(0.09, "h2") - 0.06).abs() < 1e-12);
}

#[test]
fn partitions_reading_each_other_at_one_instant_are_still_refused() {
    let message = parse_model(
        "model M Clock c = Clock(0.1); Real a; Real b; \
         equation a = sample(time, c) + subSample(b, 2); \
         b = superSample(a, 2) + 1; \
         annotation(experiment(StopTime = 0.3, Interval = 0.1)); end M;",
    )
    .expect_err("a circle at one instant has no order")
    .message;
    assert!(
        message.contains("read each other's values within one tick"),
        "{message}"
    );
}

/// The actions of a `when` in an algorithm read one another in the
/// order they are written: `n := n + 1` is what the `if` below it
/// sees. The flattener had substituted `n` into the condition twice,
/// reading `(n + 1) + 1 == 2`, and the actions it handed out were
/// written in the world the body was entered with while the event
/// carried them out one after another, so the `if` saw the new `n`
/// and added one to it again. The run went on and answered `y = 0`.
#[test]
fn a_when_in_an_algorithm_reads_what_it_just_assigned() {
    let result = run(
        "model N Integer n(start = 0, fixed = true); Real y(start = 0, fixed = true); \
         algorithm when sample(0, 0.25) then n := n + 1; \
         if n == 2 then y := time; end if; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.25)); end N;",
    );
    let at = |what: &str| result.columns.iter().position(|c| c == what).expect(what);
    let (n, y) = (at("n"), at("y"));
    let last = result.rows.last().unwrap();
    assert_eq!(last[n], 5.0, "five ticks over [0, 1]");
    assert!(
        (last[y] - 0.25).abs() < 1e-12,
        "the second tick is at 0.25, y = {}",
        last[y]
    );
    // And not before it: at the first tick `n` comes to 1.
    let first = &result.rows[0];
    assert_eq!(first[y], 0.0, "y after the first tick");
}

/// The same with the value from before the event written out: the
/// condition was substituted into as `pre(pre(iTick) + 1)`, and the
/// run refused it as `pre()` of an expression.
#[test]
fn a_when_in_an_algorithm_reads_pre_once() {
    let result = run(
        "model L Integer iTick(start = 0, fixed = true); Real y(start = 0, fixed = true); \
         algorithm when sample(0, 0.25) then iTick := pre(iTick) + 1; \
         if iTick >= 3 then y := time; end if; end when; \
         annotation(experiment(StopTime = 0.6, Interval = 0.25)); end L;",
    );
    let y = result.columns.iter().position(|c| c == "y").unwrap();
    let last = result.rows.last().unwrap();
    assert!(
        (last[y] - 0.5).abs() < 1e-12,
        "the third tick is at 0.5, y = {}",
        last[y]
    );
}

/// A relation of `time` against a threshold known before the run is a
/// time event, not a crossing to be searched for. Searched for, the
/// ramp below ended short of one by what the solver's tolerance left
/// it - 0.999899 under dopri45 over a longer run - and under RK4 by a
/// twelfth, the last stage of the last step standing on the threshold
/// and reading the far side of the switch for the whole step.
#[test]
fn a_ramp_switched_off_by_time_ends_exactly_where_it_was_told() {
    for (stop, condition) in [
        (1.0, "time < 1.0"),
        (1.5, "time < 1.0"),
        (1.5, "1.0 > time"),
    ] {
        let source = format!(
            "model E Real s1(start = 0, fixed = true); \
             equation der(s1) = if {condition} then 1 else 0.0; \
             annotation(experiment(StopTime = {stop}, Interval = 0.5)); end E;"
        );
        for method in [SolverMethod::Dopri45, SolverMethod::Bdf, SolverMethod::Rk4] {
            let result = run_on(&source, method).expect("runs");
            let s1 = result.columns.iter().position(|c| c == "s1").unwrap();
            let last = result.rows.last().unwrap();
            assert!(
                (last[s1] - 1.0).abs() < 1e-6,
                "{method:?}, {condition}, stop {stop}: s1 = {}",
                last[s1]
            );
        }
    }
}

/// The same instant read by a `when`: it fires on the threshold, not
/// wherever a search for the crossing happened to settle past it.
#[test]
fn a_when_on_time_fires_on_the_threshold() {
    let result = run(
        "model W Real x(start = 0, fixed = true); discrete Real at(start = -1, fixed = true); \
         equation der(x) = 1; when time >= 0.3 then at = x; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.25)); end W;",
    );
    let at = result.columns.iter().position(|c| c == "at").unwrap();
    let last = result.rows.last().unwrap();
    assert!(
        (last[at] - 0.3).abs() < 1e-12,
        "fired with x = {}",
        last[at]
    );
}

/// A threshold between two grid points: RK4 splits its step there
/// rather than letting a stage read past it, and the adaptive solvers
/// step onto it the same way.
#[test]
fn a_threshold_between_grid_points_is_stepped_onto() {
    let source = "model E Real s1(start = 0, fixed = true); \
         equation der(s1) = if time < 0.7 then 1 else 0.0; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end E;";
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf, SolverMethod::Rk4] {
        let result = run_on(source, method).expect("runs");
        let s1 = result.columns.iter().position(|c| c == "s1").unwrap();
        let last = result.rows.last().unwrap();
        assert!(
            (last[s1] - 0.7).abs() < 1e-9,
            "{method:?}: s1 = {}",
            last[s1]
        );
    }
}

/// A run-time `if` equation decided by a relation on `time`, in a
/// model that also integrates: the mode is left at the scheduled
/// instant, and the run continues in the other one from there.
#[test]
fn a_mode_decided_by_time_changes_on_the_threshold() {
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf] {
        let result = run_on(
            "model X Real x(start = 0, fixed = true); Real a; Real b; \
             equation der(x) = 1; \
             if time < 0.5 then a = x; b = 2 * a; \
             else b = x; a = b / 2; end if; \
             annotation(experiment(StopTime = 1, Interval = 0.1)); end X;",
            method,
        )
        .expect("runs");
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let (a, b) = (index("a"), index("b"));
        let last = result.rows.last().unwrap();
        assert!(
            (last[a] - 0.5).abs() < 1e-6 && (last[b] - 1.0).abs() < 1e-6,
            "{method:?}: a = {}, b = {}",
            last[a],
            last[b]
        );
    }
}

/// A crossing the solver finds in the same step as a scheduled
/// threshold, a hair past it: the pulse's `T_start + T_width` turns at
/// 0.3 + 1e-9 while `time < 0.3` is due at 0.3. The state event is
/// handled first and the step goes round again, so the threshold has
/// to turn there too - left for the next step, it turned one step late
/// and the integral of the step came to 1.36 where it is 1.4.
#[test]
fn a_threshold_due_with_a_crossing_turns_with_it() {
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf] {
        let result = run_on(
            "model C parameter Real startTime = 0.1, period = 0.4, width = 50; \
             Real T_width = period*width/100; \
             discrete Integer count; discrete Real T_start; Real p; \
             Real x(start = 0, fixed = true); \
             initial algorithm count := integer((time - startTime)/period); \
             T_start := startTime + count*period; \
             equation when time >= (pre(count) + 1)*period + startTime then \
             count = pre(count) + 1; T_start = time; end when; \
             p = if time < startTime then 0 else if time < T_start + T_width then 1 else 0; \
             der(x) = if time < 0.3 then 0 else 2; \
             annotation(experiment(StopTime = 1, Interval = 0.25)); end C;",
            method,
        )
        .expect("runs");
        let x = result.columns.iter().position(|c| c == "x").unwrap();
        let last = result.rows.last().unwrap();
        assert!((last[x] - 1.4).abs() < 1e-6, "{method:?}: x = {}", last[x]);
    }
}

/// `y = pre(u)` is how `Blocks.Logical.Pre` breaks a loop of switches,
/// and the event iteration of the language goes on until every
/// discrete value equals its `pre`, so `y` takes the new `u` within
/// the instant `u` changes. Copying `pre` only when the event began
/// left `y` false until some later event came along, and in this model
/// none does: the integral stayed at zero for the whole run. In a
/// thyristor bridge the same lag was a third of a millisecond of
/// firing delay and a load current ten percent short.
#[test]
fn a_value_read_through_pre_catches_up_within_the_event() {
    let result = run(
        "model P Boolean u = time > 0.1; Boolean y; Real x(start = 0, fixed = true); \
         equation y = pre(u); der(x) = if y then 1 else 0; \
         annotation(experiment(StopTime=1.0, Interval=0.1)); end P;",
    );
    let x = result.columns.iter().position(|c| c == "x").unwrap();
    let last = result.rows.last().expect("a row")[x];
    assert!((last - 0.9).abs() < 1e-6, "x(1) = {last}");
}

/// A value defined against its own `pre` that flips on every pass is
/// an event that never comes to rest, and the iteration that goes on
/// until `pre(v) = v` must stop and say so, naming the value, rather
/// than carry whichever pass it gave up on into the run.
#[test]
fn a_value_that_never_equals_its_pre_is_refused_by_name() {
    let message = run_err(
        "model F Boolean u = time > 0.1; Boolean y(start = false, fixed = true); \
         equation y = if u then not pre(y) else false; \
         annotation(experiment(StopTime=1.0, Interval=0.1)); end F;",
    );
    assert!(
        message.contains("differs from its `pre`") && message.contains("\"y\""),
        "{message}"
    );
}

/// A clamp held until a crossing, and a step long enough to reach past
/// it. The step's far end and its slope there belong to the branch
/// after the crossing, so the cubic through the two ends is not the
/// solution before it: interpolated up to the crossing, it bowed the
/// held value up by 6.9e-6 before the switch, which in `CCCV_Stack`
/// carried a charge held at its limit past the model's own assert. The
/// step is taken again ending at the crossing, and the held value is
/// held to the last digit until the switch.
#[test]
fn a_held_value_is_not_bowed_by_a_step_reaching_past_the_switch() {
    let source = "model K Real y(start = 1, fixed = true); Real u = 1 - time / 10; \
         equation der(y) = if u > 0 then 0 else u; \
         annotation(experiment(StopTime = 12, Interval = 0.001)); end K;";
    let result = run_on(source, SolverMethod::Dopri45).expect("runs");
    let y = result.columns.iter().position(|c| c == "y").unwrap();
    let highest = result
        .rows
        .iter()
        .map(|row| row[y])
        .fold(f64::MIN, f64::max);
    assert!(highest <= 1.0, "y rose to {highest}");
    let last = result.rows.last().unwrap()[y];
    assert!((last - 0.8).abs() < 1e-6, "y(12) = {last}");
}

/// The same clamp read on an output interval shorter than the steps.
/// A step reaching past the switch only a few hundredths in, or right
/// after a cut one, was not cut, and every row before the switch was
/// read off the cubic through the far branch: the held value printed
/// 4.2e-6 over its limit, and `CCCV_Stack` on its own interval of
/// 0.1 s stopped on its overcharge assert at t = 741.5. A row lying
/// before the crossing is now always written from a step cut short of
/// it. What is left, 4.2e-10, is the step that starts on the switch
/// itself, and stays under the battery's tolerance of 1e-9.
#[test]
fn a_row_before_a_switch_is_not_read_off_the_far_branch() {
    let source = "model K3 parameter Real ymax = 1 - 1e-9; Real u = 1 - time/60; \
         Real y(start = 0.9, fixed = true); \
         equation der(y) = if y > ymax and u > 0 then 0 else 0.01*u; \
         annotation(experiment(StopTime = 100, Interval = 0.1)); end K3;";
    let result = run_on(source, SolverMethod::Dopri45).expect("runs");
    let y = result.columns.iter().position(|c| c == "y").unwrap();
    let highest = result
        .rows
        .iter()
        .map(|row| row[y])
        .fold(f64::MIN, f64::max);
    assert!(highest - (1.0 - 1e-9) < 1e-9, "y rose to {highest}");
}

/// A clamp let go when its input turns, the shape of a battery cell
/// held at its limit. A step cut short of the crossing it found can
/// end with the relation reading exactly zero, on its threshold; taken
/// as a plain step, every step after it started from that zero, found
/// the turn a hair in, and the run chattered at t = 5.1676 past ten
/// thousand events, and past two hundred thousand when allowed. A cut
/// step ending on the threshold now ends in the event there.
#[test]
fn a_step_cut_short_of_a_switch_does_not_creep_up_to_it() {
    let source = "model Z2 parameter Real outMax = 1 - 1e-9; \
         Real u = 1e-6*(1 - time/5); Real y(start = 1 - 1e-9 - 1e-6, fixed = true); \
         equation der(y) = if y > outMax and u > 0 then 0 else u; \
         annotation(experiment(StopTime = 10, Interval = 0.01)); end Z2;";
    let result = run_on(source, SolverMethod::Dopri45).expect("runs");
    let y = result.columns.iter().position(|c| c == "y").unwrap();
    let last = result.rows.last().unwrap()[y];
    assert!((last - 0.999998).abs() < 1e-6, "y(10) = {last}");
}

/// A `when` on a condition that rises four times in the run fires four
/// times beside a continuous state. With nothing but `der(x) = 1` to
/// hold it back the step grows past half the period of `sin(20 t)`, and
/// a crossing that turns and turns back inside one step reads the same
/// at both ends: the adaptive solver fired once.
#[test]
fn a_condition_that_rises_four_times_fires_four_times() {
    let source = "model W Real x(start = 0, fixed = true); \
         discrete Real m(start = 0, fixed = true); Real z; \
         equation der(x) = 1; z = sin(20 * time); \
         when z > 0.5 then m = pre(m) + 1; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.01)); end W;";
    for method in [SolverMethod::Dopri45, SolverMethod::Auto] {
        let result = run_on(source, method).expect("runs");
        let at = result.columns.iter().position(|c| c == "m").unwrap();
        let m = result.rows.last().unwrap()[at];
        assert_eq!(
            m, 4.0,
            "sin(20 t) rises through 0.5 at 0.026, 0.340, 0.654 and 0.969"
        );
    }
}

/// The same over a longer run, where the step grows wider still and a
/// pair of crossings falls between two stages, so that no reading inside
/// the step is on the other side. `sin(10 t) > 0.5` rises seven times
/// in four seconds; the adaptive solver fired once, and with only the
/// stages' signs read, six times.
#[test]
fn a_pair_of_crossings_between_two_stages_is_not_lost() {
    let source = "model W10 Real x(start = 0, fixed = true); \
         discrete Real m(start = 0, fixed = true); \
         equation der(x) = 1; \
         when sin(10 * time) > 0.5 then m = pre(m) + 1; end when; \
         annotation(experiment(StopTime = 4, Interval = 0.01)); end W10;";
    let result = run_on(source, SolverMethod::Dopri45).expect("runs");
    let at = result.columns.iter().position(|c| c == "m").unwrap();
    assert_eq!(result.rows.last().unwrap()[at], 7.0);
}

/// A Boolean relation on a smooth signal, integrated: what the
/// integrator gathers is the time the relation held, which is exact.
/// `sin(20 t) > 0.5` holds on [pi/120, 5 pi/120] and every pi/10 after,
/// three whole stretches of pi/30 and the start of a fourth from 0.9687
/// to 1, so 0.34550. The relation stuck at true after its first rise
/// integrated to 0.974; this is what `Sine` through `GreaterThreshold`
/// and `BooleanToReal` into an `Integrator` printed too.
#[test]
fn a_relation_beside_a_state_turns_back_after_its_first_rise() {
    let source = "model E Real x(start = 0, fixed = true); Boolean b; \
         Real c(start = 0, fixed = true); \
         equation der(x) = 1; b = sin(20 * time) > 0.5; der(c) = if b then 1 else 0; \
         annotation(experiment(StopTime = 1, Interval = 0.01)); end E;";
    let pi = std::f64::consts::PI;
    // Rises at (pi/6 + 2 pi k)/20, falls at (5 pi/6 + 2 pi k)/20.
    let owed = 3.0 * (4.0 * pi / 120.0) + (1.0 - (pi / 6.0 + 6.0 * pi) / 20.0);
    let result = run_on(source, SolverMethod::Dopri45).expect("runs");
    let at = result.columns.iter().position(|c| c == "c").unwrap();
    let c = result.rows.last().unwrap()[at];
    assert!((c - owed).abs() < 1e-6, "c(1) = {c}, owed {owed}");
}

/// The hysteresis of `Modelica.Blocks.Logical.Hysteresis`, written out,
/// on `sin(20 t)` between -0.5 and 0.5 and integrated. It is on from
/// each rise through 0.5 to the next fall through -0.5; the adaptive
/// solver kept it on from the first rise and printed 0.974 for 0.503.
#[test]
fn a_hysteresis_on_a_smooth_signal_lets_go_each_time() {
    let source = "model H Real x(start = 0, fixed = true); Real u; \
         Boolean y(start = false, fixed = true); Real c(start = 0, fixed = true); \
         equation der(x) = 1; u = sin(20 * time); \
         y = not pre(y) and u > 0.5 or pre(y) and u >= -0.5; \
         der(c) = if y then 1 else 0; \
         annotation(experiment(StopTime = 1, Interval = 0.01)); end H;";
    let pi = std::f64::consts::PI;
    // On from (pi/6 + 2 pi k)/20 to (7 pi/6 + 2 pi k)/20: half a period
    // of pi/10 each, three whole ones, then from the fourth rise to 1.
    let owed = 3.0 * pi / 20.0 + (1.0 - (pi / 6.0 + 6.0 * pi) / 20.0);
    let result = run_on(source, SolverMethod::Dopri45).expect("runs");
    let at = result.columns.iter().position(|c| c == "c").unwrap();
    let c = result.rows.last().unwrap()[at];
    assert!((c - owed).abs() < 1e-6, "c(1) = {c}, owed {owed}");
}

/// A relation that dips past its threshold just after a crossing the
/// step was cut short of. The step after a cut one grew fivefold, and
/// `cos(20 t) < -0.5`, below its threshold for a tenth of a second every
/// third of one, fell wholly before that step's first stage: one fall
/// found of three, where the indicators read at the ends found none.
#[test]
fn a_dip_right_after_a_cut_step_is_not_stepped_over() {
    let source = "model CS Real x(start = 0, fixed = true); \
         discrete Real m(start = 0, fixed = true); \
         equation der(x) = 1; when cos(20 * time) < -0.5 then m = pre(m) + 1; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.01)); end CS;";
    let result = run_on(source, SolverMethod::Dopri45).expect("runs");
    let at = result.columns.iter().position(|c| c == "m").unwrap();
    assert_eq!(
        result.rows.last().unwrap()[at],
        3.0,
        "cos(20 t) falls through -0.5 at 0.105, 0.419 and 0.733"
    );
}

/// The count of events that changed something is the model's and not
/// the solver's: `rk4` raises an event at every grid point and finds
/// nothing moved at most of them, and those are not counted. The
/// relation `sin(20 t) > 0.5` beside a state rises four times and
/// falls three times in a second, and a `when` on it fires on each
/// rise, so both solvers count four - the number a lost crossing
/// shows as a missing unit, which is what comparing two solvers
/// leans on.
#[test]
fn events_that_changed_something_are_counted_alike_on_two_solvers() {
    let source = "model W Real x(start = 0, fixed = true); \
         discrete Real m(start = 0, fixed = true); \
         equation der(x) = 1; when sin(20 * time) > 0.5 then m = pre(m) + 1; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.01)); end W;";
    for method in [SolverMethod::Dopri45, SolverMethod::Rk4] {
        let result = run_on(source, method).expect("runs");
        assert_eq!(result.events, 4, "{method:?}");
    }
    // A model with no event at all counts none, the initial event that
    // moves nothing included.
    let quiet = "model Q Real x(start = 0, fixed = true); equation der(x) = 1; \
         annotation(experiment(StopTime = 1, Interval = 0.01)); end Q;";
    assert_eq!(run_on(quiet, SolverMethod::Rk4).expect("runs").events, 0);
}

#[test]
fn a_clock_handed_through_an_input_reaches_the_block_inside() {
    // A clock connected into a block and on to a block inside it comes
    // out as two equations between clocks, the inner one first. Read
    // as a definition of whichever side was not yet spoken for, the
    // inner one took the input's name and the innermost clock was
    // defined by nobody, even for a periodic clock. Ticking every 0.1 s
    // over a second, the count is 11.
    let result = run("model M \
           connector CO = output Clock; connector CI = input Clock; \
           block Tick CI u; Integer n(start = 0); \
             equation when u then n = previous(n) + 1; end when; end Tick; \
           block Sink CI u; Tick t; equation connect(u, t.u); end Sink; \
           Sink sink; CO y = Clock(0.1); \
           equation connect(y, sink.u); \
           annotation(experiment(StopTime = 1, Interval = 0.1)); \
         end M;");
    let last = result.rows.len() - 1;
    let n = result.rows[last][result.columns.iter().position(|c| c == "sink.t.n").unwrap()];
    assert_eq!(n, 11.0, "sink.t.n = {n}");
}

/// `not` over a Boolean vector is the vector of the negations. The
/// space vector modulation writes `fire_n = not fire_p` with `fire_p`
/// three wide, and the compiler refused it for an array standing where
/// a scalar was expected.
#[test]
fn not_over_a_boolean_vector_negates_each_element() {
    let source = "model A Boolean p[3]; Boolean n[3]; \
         equation p = {time > 0.25, time > 0.5, time > 0.75}; n = not p; \
         annotation(experiment(StopTime = 1, Interval = 0.01)); end A;";
    let result = run(source);
    let column = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let at = result
        .rows
        .iter()
        .position(|row| (row[0] - 0.6).abs() < 1e-9)
        .unwrap();
    let row = &result.rows[at];
    assert_eq!(
        [
            row[column("n[1]")],
            row[column("n[2]")],
            row[column("n[3]")]
        ],
        [0.0, 0.0, 1.0],
        "n at t = 0.6"
    );
}

/// A row of a constant table picked by an index known only during the
/// run, as the modulation picks `fire[ka + 1, :]` by the sector it
/// worked out. Each element is asked by the index, and the row comes out
/// whole; before, the pick was taken for one scalar and the row refused.
#[test]
fn a_row_of_a_table_is_picked_by_an_index_of_the_run() {
    let source = "model B constant Real c[3, 2] = [1, 2; 3, 4; 5, 6]; \
         Integer k(start = 1, fixed = true); Real p[2]; \
         algorithm when sample(0, 0.1) then k := if time > 0.65 then 3 else if time > 0.35 then 2 else 1; end when; \
         equation p = c[k, :]; \
         annotation(experiment(StopTime = 1, Interval = 0.01)); end B;";
    let result = run(source);
    let column = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let value_at = |t: f64, name: &str| {
        let at = result
            .rows
            .iter()
            .position(|row| (row[0] - t).abs() < 1e-9)
            .unwrap();
        result.rows[at][column(name)]
    };
    assert_eq!([value_at(0.2, "p[1]"), value_at(0.2, "p[2]")], [1.0, 2.0]);
    assert_eq!([value_at(0.5, "p[1]"), value_at(0.5, "p[2]")], [3.0, 4.0]);
    assert_eq!([value_at(0.9, "p[1]"), value_at(0.9, "p[2]")], [5.0, 6.0]);
}

/// A vector assigned whole inside a `when` of an algorithm, as the
/// random generators hand back their state from `initialState`. It is
/// one action per element, the same as the equation `st = {...}` in a
/// `when` already was; before, it was refused for an array standing
/// where a scalar was expected.
#[test]
fn a_vector_assigned_whole_in_a_when_of_an_algorithm_lands_on_each_element() {
    let source = "model W discrete Real st[2](each start = 0, each fixed = true); \
         algorithm when sample(0, 0.25) then st := pre(st) + {1, 10}; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.01)); end W;";
    let result = run(source);
    let column = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    // Ticks at 0, 0.25, 0.5, 0.75 and 1.
    assert_eq!([last[column("st[1]")], last[column("st[2]")]], [5.0, 50.0]);
}

/// An `if` inside a `when` that gives a whole array in each branch, as
/// the Preisach hysteresis keeps its turning points. Each element is a
/// choice of its own, and the branch not taken keeps `pre`; before, the
/// array was taken for one scalar and refused.
#[test]
fn an_if_at_an_event_gives_an_array_element_by_element() {
    let source = "model P discrete Real s[3](each start = 0, each fixed = true); \
         Boolean a = time > 0.3; Boolean b = time > 0.6; \
         equation when {a, b} then \
           if a and not b then s = cat(1, {1}, fill(2, 2)); else s = pre(s) + {1, 1, 1}; end if; \
         end when; \
         annotation(experiment(StopTime = 1, Interval = 0.01)); end P;";
    let result = run(source);
    let column = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    assert_eq!(
        [
            last[column("s[1]")],
            last[column("s[2]")],
            last[column("s[3]")]
        ],
        [2.0, 3.0, 3.0]
    );
}

/// The value of one column of the last row of a result.
fn last_of(result: &oxidelica_sim::SimResult, name: &str) -> f64 {
    let at = result
        .columns
        .iter()
        .position(|c| c == name)
        .unwrap_or_else(|| panic!("no column {name} in {:?}", result.columns));
    result.rows.last().expect("a final row")[at]
}

/// A buffer filled at a place only the run knows.
///
/// `buf[iTick] := u` in the `when` of a model is how the FFT blocks of
/// the standard library sample their input, and it was refused: "the
/// subscript of `buf` must be a whole number the compiler can see".
/// Written out as one guarded write per element, each element takes
/// the value at the tick that names it and keeps it after, so after
/// four ticks the buffer holds 10, 20, 30, 40 and its sum is 100.
#[test]
fn a_buffer_is_filled_at_a_place_the_run_settles() {
    let result = run("model A \
           parameter Integer n = 4; \
           Integer k(start = 0, fixed = true); \
           Real buf[n](each start = 0, each fixed = true); \
           Real s; \
         algorithm \
           when sample(0, 0.1) then \
             k := pre(k) + 1; \
             if k >= 1 and k <= n then buf[k] := 10*k; end if; \
           end when; \
         equation s = sum(buf); \
           annotation(experiment(StopTime = 1)); \
         end A;");
    let got: Vec<f64> = ["buf[1]", "buf[2]", "buf[3]", "buf[4]", "s"]
        .iter()
        .map(|name| last_of(&result, name))
        .collect();
    assert_eq!(got, vec![10.0, 20.0, 30.0, 40.0, 100.0]);
}

/// A discrete matrix written a row at a time, at a row only the run
/// knows and outside any `when`, the way the digital memories write
/// `mem[addr, 1:n] := word` under a write enable. The rows the address
/// does not name keep what they held: at the end the address has gone
/// round 1, 2, 3, 4 and only rows 3 and 4 were written, so rows 1 and
/// 2 still hold their start of zero.
#[test]
fn a_discrete_matrix_keeps_the_rows_an_address_does_not_name() {
    let result = run("model F1 \
           Integer mem[4,2](each start = 0); \
           Integer a(start = 1); \
         algorithm \
           when sample(0, 0.1) then a := mod(pre(a), 4) + 1; end when; \
           if a > 2 then mem[a, 1:2] := {a, 10*a}; end if; \
           annotation(experiment(StopTime = 0.45)); \
         end F1;");
    let got: Vec<f64> = [
        "mem[1,1]", "mem[1,2]", "mem[2,1]", "mem[2,2]", "mem[3,1]", "mem[3,2]", "mem[4,1]",
        "mem[4,2]",
    ]
    .iter()
    .map(|name| last_of(&result, name))
    .collect();
    assert_eq!(got, vec![0.0, 0.0, 0.0, 0.0, 3.0, 30.0, 4.0, 40.0]);
}

/// `pre` of a matrix is a matrix of `pre`s. It was asked for each row
/// as one scalar and refused: "an array of shape [2] is used where a
/// scalar is expected".
#[test]
fn pre_of_a_matrix_is_taken_element_by_element() {
    let result = run("model I7 \
           Integer mem[2,2](each start = 0); \
           Integer y[2,2]; \
         equation \
           y = pre(mem); \
           mem = {{1, 2}, {3, 4}}; \
           annotation(experiment(StopTime = 0.5)); \
         end I7;");
    let got: Vec<f64> = ["y[1,1]", "y[1,2]", "y[2,1]", "y[2,2]"]
        .iter()
        .map(|name| last_of(&result, name))
        .collect();
    assert_eq!(got, vec![1.0, 2.0, 3.0, 4.0]);
}

/// A value loaded once by `if initial()` in an algorithm keeps it.
///
/// `m := 7` under `if initial()` flattens to `m = if initial() then 7
/// else pre(m)`, which is right. Kept among the continuous unknowns it
/// was solved where `initial()` reads false, so `m` held its start of
/// zero for the whole run, and `x`, which integrates it, stayed at zero
/// too. Taken by the event iteration, the initial event raises
/// `initial()`: `m` is 7 throughout and `x` reaches 7 * 0.6 = 4.2.
#[test]
fn a_value_loaded_at_the_initial_event_is_kept() {
    let result = run("model J4 \
           Integer m(start = 0); \
           Real x(start = 0, fixed = true); \
         equation der(x) = m; \
         algorithm if initial() then m := 7; end if; \
           annotation(experiment(StopTime = 0.6)); \
         end J4;");
    let m = result.columns.iter().position(|c| c == "m").expect("m");
    assert_eq!(result.rows.first().map(|row| row[m]), Some(7.0));
    assert_eq!(last_of(&result, "m"), 7.0);
    assert!(
        (last_of(&result, "x") - 4.2).abs() < 1e-9,
        "{}",
        last_of(&result, "x")
    );
}
