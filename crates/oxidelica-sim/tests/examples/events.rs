//! Models that turn on an event: a ball bouncing, a diode blocking, a switch rectifying.

use super::shared::*;
use oxidelica_parser::parse_model;
use oxidelica_sim::{compile, SolverMethod};

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
fn an_event_that_never_settles_says_so() {
    // Two switches defined in terms of each other chase round the
    // event iteration for ever. The rounds are bounded, and what the
    // bound used to do was carry the last round's values forward as
    // though they had settled - a quiet wrong answer. It says so now,
    // and names what was still moving.
    let model = compile(
        &oxidelica_parser::parse_model(
            "model M Boolean a; Boolean b; Real x; \
             equation x = time + 1; a = not b and x > 0; b = a or x < 0; \
             annotation(experiment(StopTime = 0.01, Interval = 0.01)); end M;",
        )
        .unwrap(),
    );
    let why = match model {
        Ok(compiled) => match compiled.simulate() {
            Ok(_) => panic!("an event that never settles was allowed to pass"),
            Err(why) => why.to_string(),
        },
        Err(why) => why.to_string(),
    };
    assert!(why.contains("does not come to rest"), "{why}");
    assert!(why.contains('a') && why.contains('b'), "{why}");
}

/// The refusal names what kept moving, not every discrete name.
///
/// `creeps` defines itself and never comes to rest; `calm` settles on
/// the first round and is no part of the reason. Listing both is the
/// same fault as a refusal that quotes whichever parameter came
/// first - the reader cannot tell the cause from the company. In one
/// standard-library counter the list was seventy-nine names, all but
/// a few of them innocent.
#[test]
fn an_event_that_never_settles_names_only_what_moved() {
    let model = compile(
        &oxidelica_parser::parse_model(
            "model M Real x(start = 0, fixed = true); Integer creeps(start = 0); \
             Integer calm(start = 0); \
             equation der(x) = 1; creeps = if x > 0.5 then creeps + 1 else 0; \
             calm = if x > 0.5 then 7 else 0; \
             annotation(experiment(StopTime = 1.0, Interval = 0.1)); end M;",
        )
        .unwrap(),
    );
    let why = match model {
        Ok(compiled) => match compiled.simulate() {
            Ok(_) => panic!("an event that never settles was allowed to pass"),
            Err(why) => why.to_string(),
        },
        Err(why) => why.to_string(),
    };
    assert!(why.contains("does not come to rest"), "{why}");
    assert!(why.contains("creeps"), "{why}");
    assert!(!why.contains("calm"), "{why}");
}

/// A model that settles each event and immediately raises another a
/// hair further on. The bound inside one event cannot see it: every
/// event there comes to rest properly. What the run does instead is
/// creep forward by whatever the step size has fallen to, writing a
/// row apiece, and the only thing that ever stopped it was the memory
/// of the machine - a hundred gigabytes of rows, hours in, and a hand
/// on the process.
fn a_run_that_creeps(
    adjust: impl FnOnce(&mut oxidelica_sim::CompiledModel),
) -> oxidelica_sim::SimError {
    let mut compiled = compile(
        &oxidelica_parser::parse_model(
            "model Creep Real x(start = 0); discrete Real k(start = 0); \
             equation der(x) = 1; \
             when x > 1e-12 then reinit(x, 0); k = pre(k) + 1; end when; \
             annotation(experiment(StopTime = 1)); end Creep;",
        )
        .unwrap(),
    )
    .expect("the model compiles; it is the run that cannot end");
    adjust(&mut compiled);
    match compiled.simulate() {
        Ok(_) => panic!("a run that never advances was allowed to finish"),
        Err(why) => why,
    }
}

#[test]
fn a_run_that_never_advances_is_refused_by_its_events() {
    let why = a_run_that_creeps(|_| {}).to_string();
    // Named model and named instant: a message saying only that some
    // limit was reached leaves the reader with a corpus to search.
    assert!(why.contains("Creep"), "{why}");
    assert!(why.contains("events"), "{why}");
    assert!(why.contains("t = "), "{why}");
}

#[test]
fn a_run_that_writes_without_end_is_refused_by_its_rows() {
    // The same run with the event ceiling lifted out of the way, so
    // that the other ceiling is the one that answers. Both are needed:
    // a run can write without end while advancing in time perfectly
    // well, if what it was asked for is finer than any memory.
    let why = a_run_that_creeps(|model| {
        model.max_events_one_interval = usize::MAX;
        model.max_rows = 500;
    })
    .to_string();
    assert!(why.contains("Creep"), "{why}");
    assert!(why.contains("500 output rows"), "{why}");
    assert!(why.contains("t = "), "{why}");
}

#[test]
fn the_textbook_ideal_switch_rectifies_exactly() {
    // The switch's branches constrain different unknowns: blocking
    // is an equation on the current, conducting one on the voltage.
    // Each mode is compiled as its own model - matched and torn for
    // the equations actually in force - and compiled again at the
    // instant the switch flips. Nothing here is approximate: the
    // current is the clipped source to the last bit - on either
    // solver, since which branch is in force is the model's business
    // and not the stepper's.
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf] {
        let mut compiled = compile(&with_library("ideal_rectifier.mo")).unwrap();
        compiled.method = method;
        let result = compiled.simulate().unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let (mut blocking_rows, mut conducting_rows) = (0, 0);
        for row in &result.rows {
            assert_eq!(
                row[index("switch.i")],
                row[index("clipped")],
                "{method:?} at t = {}",
                row[0]
            );
            if row[index("switch.blocking")] > 0.5 {
                // The blocking branch is `i = 0`, and it holds exactly.
                assert_eq!(row[index("switch.i")], 0.0, "{method:?} at t = {}", row[0]);
                blocking_rows += 1;
            } else {
                // The conducting branch is `v = 0`, likewise.
                assert_eq!(row[index("switch.v")], 0.0, "{method:?} at t = {}", row[0]);
                conducting_rows += 1;
            }
        }
        // Two full periods: the switch really did work both ways.
        assert!(blocking_rows > 400 && conducting_rows > 400);
    }
}

#[test]
fn a_chopped_supply_draws_the_exact_staircase() {
    // The supply's two branches are kept and merged into one
    // equation apiece, decided while the run goes; the relation
    // driving them is an event indicator, so the switching
    // instants land exactly. What comes out is an RC charging
    // towards the supply for half a period and towards zero for
    // the next, which has a closed form.
    //
    // Both adaptive solvers are held to it. The branch turns over at
    // the crossing itself, so a solver that waits until after its next
    // step to notice would have to rebuild at the instant it just
    // started from, and would never get past the first switch.
    let (supply, tau, half) = (10.0f64, 0.2f64, 0.5f64);
    for method in [SolverMethod::Dopri45, SolverMethod::Bdf] {
        let mut compiled = compile(&with_library("switched_rc.mo")).unwrap();
        compiled.method = method;
        let result = compiled.simulate().unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
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
                "{method:?} at t = {}: v = {} vs {wanted}",
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
fn a_delay_reads_its_own_expression_before_anything_is_remembered() {
    // Until `T` has passed, `delay(u, T)` is what `u` was at the start
    // time. The memory is empty there, and an empty memory used to
    // answer zero - a number no enumeration is worth.
    // `Electrical.Digital`'s transport delay indexes a table of logic
    // values by it, so the whole of the first delay came out NaN and
    // five flip-flops wrote nothing but NaN to their stop time.
    let result = compile(
        &parse_model(
            "model D Integer x(start = 3, fixed = true); Real xr; \
             Integer y; constant Integer tab[3] = {10, 20, 30}; \
             equation x = 3; xr = Integer(pre(x)); \
             y = tab[integer(delay(xr, 0.2))]; end D;",
        )
        .unwrap(),
    )
    .unwrap()
    .simulate()
    .unwrap();
    let y = result.columns.iter().position(|c| c == "y").unwrap();
    let first = result.rows.first().expect("a row at the start time");
    assert!(
        (first[y] - 30.0).abs() < 1e-9,
        "the delay reads `xr` at the start, so the table gives 30, not {}",
        first[y]
    );
}

#[test]
fn a_switch_defined_by_a_delayed_signal_finds_its_slot() {
    // A flip-flop is written as a delayed signal compared against a
    // threshold, which makes a Boolean's definition read the slot the
    // run fills from the history it keeps. That slot has to exist
    // before the definition is compiled, exactly as `$initial` does.
    let result = compile(
        &parse_model(
            "model D Real u; Boolean b; equation u = time; \
             b = delay(u, 0.1) > 0.5; end D;",
        )
        .unwrap(),
    )
    .unwrap()
    .simulate()
    .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (time, u, b) = (index("time"), index("u"), index("b"));
    for row in &result.rows {
        // `u` is time itself, so the delayed value is `t - 0.1`, and
        // the switch is on from six tenths of a second.
        let expected = f64::from(row[time] - 0.1 > 0.5);
        if (row[u] - 0.6).abs() > 1e-6 {
            assert!(
                (row[b] - expected).abs() < 0.5,
                "at t = {}: switch {} wanted {expected}",
                row[time],
                row[b]
            );
        }
    }
}

/// A `when initial()` fires after the definitions have settled among
/// themselves, not while one of them still holds the value it had
/// before the event.
///
/// A discrete definition reads the algebraic part, and a definition
/// whose input is another definition's output is an ordering: asked
/// once each, in whatever order the compiler holds them, the second
/// answers from what the first was worth beforehand. For a definition
/// that is harmless, because it is asked again next round. For a
/// `when initial()` it is not: the clause fires exactly once, and
/// what it stored on that round is what the model carries for the
/// rest of the run.
///
/// Here `late` is the downstream definition, worth NaN before the
/// event and 3 after it, and `kept` is what the clause stored. A
/// `when initial()` that fires too early keeps the NaN, and the
/// model then never comes to rest at all - NaN differs from itself,
/// so the definition holding it reports a change on every round for
/// ever. `Modelica.Electrical.Digital` is thirteen models of this.
#[test]
fn a_when_at_the_start_sees_the_definitions_settled() {
    let model = compile(
        &oxidelica_parser::parse_model(
            "model M Real x(start = 0, fixed = true); \
             Integer late; Integer early; Integer kept(start = 0, fixed = true); \
             Real relay; discrete Real t_next; \
             algorithm \
             when {initial(), time >= t_next} then \
               t_next := time + 1; kept := late; \
             end when; \
             equation der(x) = 1; \
             late = if relay > 2.5 then 3 else 0/0; \
             relay = early; \
             early = if x >= 0 then 3 else 0; \
             annotation(experiment(StopTime = 0.5, Interval = 0.1)); end M;",
        )
        .unwrap(),
    );
    let run = model.expect("the model compiles").simulate();
    let out = run.expect("the event comes to rest");
    let kept = out
        .columns
        .iter()
        .position(|c| c == "kept")
        .expect("`kept` is in the output");
    let last = out.rows.last().expect("the run has a row");
    assert_eq!(last[kept], 3.0, "the clause stored a half-built value");
}
