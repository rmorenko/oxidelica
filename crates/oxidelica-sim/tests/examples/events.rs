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
