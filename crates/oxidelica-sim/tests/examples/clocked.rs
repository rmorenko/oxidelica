//! Models that run on a clock or a state machine rather than continuously.

use super::shared::*;
use oxidelica_sim::{compile, SimResult};

#[test]
fn a_state_machine_holds_up_a_queue_of_cars() {
    // The states are blocks, the arrows are declared, and the
    // machine ticks once a second. Underneath it the queue knows
    // nothing about any of that: it grows at a steady rate and
    // drains only while the light is green, so it is a sawtooth
    // whose corners fall where the machine says they do.
    let result = compile(&with_library("traffic_light.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();

    // One colour per second: red for four, green for five, amber
    // for two, over and over.
    let mut colours = String::new();
    for second in 0..=22 {
        let at = second as f64;
        let row = result
            .rows
            .iter()
            .rev()
            .find(|row| (row[0] - at).abs() < 1e-9)
            .unwrap_or_else(|| panic!("no row at t = {at}"));
        colours.push(match row[index("lamp")] as i64 {
            0 => 'r',
            1 => 'g',
            _ => 'a',
        });
    }
    assert_eq!(colours, "rrrrgggggaarrrrgggggaar");

    // And the queue is exactly the sawtooth that follows from it.
    let (arrivals, departures) = (2.0f64, 5.0f64);
    let (mut queue, mut at, mut green) = (0.0f64, 0.0f64, false);
    for row in &result.rows {
        let step = row[0] - at;
        queue += (arrivals - if green { departures } else { 0.0 }) * step;
        assert!(
            (row[index("queue")] - queue).abs() < 1e-9,
            "t = {}: queue {} vs {queue}",
            row[0],
            row[index("queue")]
        );
        green = row[index("lamp")] == 1.0;
        at = row[0];
    }
    // Four seconds of red and two of amber at two cars a second.
    assert!(
        (result
            .rows
            .iter()
            .map(|row| row[index("queue")])
            .fold(0.0, f64::max)
            - 8.0)
            .abs()
            < 1e-9
    );
}

#[test]
fn a_clocked_controller_follows_its_own_recurrence() {
    // The clock is declared and the equations that belong to it are
    // found rather than marked. What comes out is a sampled-data
    // loop whose every tick can be written by hand: the control is
    // constant between ticks, so the plant relaxes towards it, and
    // the integral advances by one period at a time.
    let result = compile(&with_library("clocked_control.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (period, plant, kp, ki, setpoint) = (0.05f64, 0.4f64, 1.6f64, 4.0f64, 1.0f64);
    let decay = (-period / plant).exp();

    let (mut state, mut integral) = (0.0f64, 0.0f64);
    let mut tick = 0;
    while tick as f64 * period <= 3.0 + 1e-12 {
        let at = tick as f64 * period;
        let error = setpoint - state;
        integral += error * period;
        let command = kp * error + ki * integral;
        // The row a tick leaves behind is the one after the event.
        let row = result
            .rows
            .iter()
            .rev()
            .find(|row| (row[0] - at).abs() < 1e-9)
            .unwrap_or_else(|| panic!("no row at t = {at}"));
        assert!(
            (row[index("u")] - command).abs() < 1e-9,
            "tick {tick}: u = {} vs {command}",
            row[index("u")]
        );
        assert!((row[index("x")] - state).abs() < 1e-9, "tick {tick}");
        state = command + (state - command) * decay;
        tick += 1;
    }
    assert_eq!(tick, 61);
    // And it did settle on the setpoint.
    assert!((result.rows.last().unwrap()[index("x")] - setpoint).abs() < 1e-3);
}

#[test]
fn a_sampled_controller_holds_its_output_between_ticks() {
    let result = compile(&with_library("sampled_control.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (u, y) = (index("u"), index("y"));
    let (period, plant_time) = (0.1f64, 0.5f64);

    // The control signal changes only on the clock, and the ticks
    // land on the period exactly rather than on the output grid.
    let mut ticks = Vec::new();
    for pair in result.rows.windows(2) {
        if pair[0][u] != pair[1][u] {
            ticks.push(pair[1][0]);
        }
    }
    assert_eq!(ticks.len(), 50, "one tick per period over five seconds");
    for t in &ticks {
        let off = t / period - (t / period).round();
        assert!(off.abs() < 1e-9, "tick off the clock at t = {t}");
    }

    // Between two ticks the plant is a first-order lag relaxing
    // toward the held value, which has a closed form.
    let mut worst = 0.0f64;
    for pair in result.rows.windows(2) {
        let (t0, t1) = (pair[0][0], pair[1][0]);
        if t1 <= t0 || pair[0][u] != pair[1][u] {
            continue;
        }
        let held = pair[1][u];
        let expected = held + (pair[0][y] - held) * (-(t1 - t0) / plant_time).exp();
        worst = worst.max((pair[1][y] - expected).abs());
    }
    assert!(worst < 1e-8, "hold response off by {worst}");

    // Integral action still lands on the setpoint.
    assert!((result.rows.last().unwrap()[y] - 1.0).abs() < 1e-3);
}

#[test]
fn the_discrete_library_blocks_run_on_the_clock() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let library = std::fs::read_to_string(root.join("lib/Oxidelica.mo")).unwrap();
    let run_top = |source: &str| {
        let model =
            oxidelica_parser::parse_model_with_libraries(std::slice::from_ref(&library), source)
                .unwrap();
        compile(&model).unwrap().simulate().unwrap()
    };

    // A unit delay carries the previous tick's value, so against a
    // ramp its output trails the input by exactly one period.
    let delayed = run_top(
        "model D Oxidelica.Blocks.Discrete.UnitDelay delay(samplePeriod = 0.25); \
         Real ramp; equation ramp = time; delay.u = ramp; \
         annotation(experiment(StopTime = 2.0, Interval = 0.05)); end D;",
    );
    let index =
        |result: &SimResult, name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (y, held) = (index(&delayed, "delay.y"), index(&delayed, "delay.held"));
    for row in &delayed.rows {
        let t = row[0];
        // A tick instant carries two rows, before and after the
        // event; the value between ticks is the one to check.
        if t < 0.5 || (t / 0.25 - (t / 0.25).round()).abs() < 1e-9 {
            continue;
        }
        // At time t the output is the input from one tick earlier.
        let tick = (t / 0.25).floor() * 0.25;
        let expected = (tick - 0.25).max(0.0);
        assert!(
            (row[y] - expected).abs() < 1e-9,
            "t = {t}: delayed {} vs {expected}",
            row[y]
        );
        assert!((row[held] - tick).abs() < 1e-9);
    }

    // The library controller reproduces the hand-written one of the
    // example, tick for tick.
    let library_pi = run_top(
        "model L Oxidelica.Blocks.Discrete.PI controller(samplePeriod = 0.1, k = 2.0, Ti = 0.5); \
         Real y(start = 0, fixed = true); equation controller.u = 1.0 - y; \
         der(y) = (controller.y - y) / 0.5; \
         annotation(experiment(StopTime = 5.0, Interval = 0.002, Tolerance = 1e-9)); end L;",
    );
    let by_hand = compile(&with_library("sampled_control.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let (a, b) = (index(&library_pi, "y"), index(&by_hand, "y"));
    assert_eq!(library_pi.rows.len(), by_hand.rows.len());
    for (left, right) in library_pi.rows.iter().zip(&by_hand.rows) {
        assert!((left[a] - right[b]).abs() < 1e-12);
    }
}

#[test]
fn an_algorithm_converts_a_held_sample_into_a_staircase() {
    let result = compile(&with_library("quantizer.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (sample, quantized, error) = (index("adc.y"), index("quantized"), index("error"));
    let (step, levels, period) = (0.25f64, 4, 0.05f64);

    // The staircase the algorithm describes, evaluated the same way
    // it is written: strictly above a level to reach it.
    let staircase = |value: f64| {
        let mut out = 0.0;
        for i in 1..=levels {
            if value > i as f64 * step {
                out = i as f64 * step;
            }
            if value < -(i as f64) * step {
                out = -(i as f64) * step;
            }
        }
        out
    };
    for row in &result.rows {
        assert_eq!(row[quantized], staircase(row[sample]));
        assert!(row[error].abs() <= step + 1e-12);
        // Between ticks the converter holds the signal it sampled.
        let t = row[0];
        if (t / period - (t / period).round()).abs() < 1e-9 {
            continue;
        }
        let tick = (t / period).floor() * period;
        let held = (2.0 * std::f64::consts::PI * tick).sin();
        assert!(
            (row[sample] - held).abs() < 1e-8,
            "t = {t}: held {} vs {held}",
            row[sample]
        );
    }
}
