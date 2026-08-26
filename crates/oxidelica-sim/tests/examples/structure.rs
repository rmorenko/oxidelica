//! Models whose structure is the point: index reduction, initial equations, streams, transport.

use super::shared::*;
use oxidelica_parser::parse_model;
use oxidelica_sim::compile;

#[test]
fn a_delayed_wave_arrives_unchanged_but_later() {
    // What comes out of the pipe is what went in, a transit time
    // ago and a little smaller. The shape is exact; the shift is
    // as exact as the output grid, which is what a straight line
    // between two remembered points can manage.
    let result = compile(&with_library("transport_delay.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (transit, loss) = (0.53f64, 0.15f64);
    for row in &result.rows {
        let (t, seen) = (row[0], row[index("outlet")]);
        let wanted = if t >= transit {
            (1.0 - loss) * (3.0 * (t - transit)).sin()
        } else {
            0.0
        };
        assert!(
            (seen - wanted).abs() < 1e-5,
            "t = {t}: outlet {seen} vs {wanted}"
        );
    }
    // Before the fluid has crossed, the far end holds what the
    // inlet started at.
    assert_eq!(result.rows[0][index("outlet")], 0.0);
    // And the vessel it pours into really did fill.
    assert!(result
        .rows
        .iter()
        .any(|row| row[index("vessel")].abs() > 0.2));
}

#[test]
fn a_stream_junction_mixes_and_the_tank_relaxes_to_it() {
    // Two sources push 1 kg/s at h=100 and 3 kg/s at h=20 into a
    // three-way node; the junction hands the tank their
    // flow-weighted mix and the tank's contents approach it as a
    // first-order lag with time constant mass / m_flow = 2 s.
    let result = compile(&with_library("stream_mixer.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let mix = (1.0 * 100.0 + 3.0 * 20.0) / 4.0;
    let last = result.rows.last().unwrap();
    assert!((last[index("h_supplied")] - mix).abs() < 1e-9);
    for row in &result.rows {
        let expected = mix * (1.0 - (-row[0] / 2.0).exp());
        assert!(
            (row[index("tank.h")] - expected).abs() < 1e-5,
            "t = {}: h = {} vs {expected}",
            row[0],
            row[index("tank.h")]
        );
    }
}

#[test]
fn the_flight_plan_and_the_flown_trajectory_agree() {
    // `(planned_range, planned_duration) = flight(v0, angle)` fills
    // both targets from one call, with gravity defaulted inside the
    // function; the integrated throw must land where it says.
    let result = compile(&with_library("ballistic_range.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();

    let (v0, angle, g) = (12.0f64, 0.6f64, 9.81);
    let range = v0 * v0 * (2.0 * angle).sin() / g;
    let duration = 2.0 * v0 * angle.sin() / g;
    // The planned values are constants over the whole run.
    assert!((result.rows[0][index("planned_range")] - range).abs() < 1e-12);
    assert!((last[index("planned_range")] - range).abs() < 1e-12);
    assert!((last[index("planned_duration")] - duration).abs() < 1e-12);
    // The run stops within a hair of the planned landing, so the
    // ball is at the planned range and back on the ground.
    assert!(
        (last[index("x")] - range).abs() < 1e-3,
        "landed at {} instead of {range}",
        last[index("x")]
    );
    assert!(
        last[index("y")].abs() < 1e-2,
        "still {} up",
        last[index("y")]
    );
}

#[test]
fn an_initial_equation_section_starts_the_model_in_equilibrium() {
    let result = compile(&with_library("steady_start.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (temperature, x, v) = (index("T"), index("x"), index("v"));
    let first = &result.rows[0];

    // Heater against losses, and the spring against gravity.
    assert!((first[temperature] - (5.0 + 3000.0 / 250.0)).abs() < 1e-9);
    assert!((first[x] - (-2.0 * 9.81 / 40.0)).abs() < 1e-9);
    assert!(first[v].abs() < 1e-12);
    // Started at the balance point, nothing moves for the whole run.
    for row in &result.rows {
        assert!((row[temperature] - first[temperature]).abs() < 1e-9);
        assert!((row[x] - first[x]).abs() < 1e-9);
        assert!(row[v].abs() < 1e-9);
    }
}

#[test]
fn a_pendulum_over_the_top_reselects_its_states() {
    // The known limit of a static selection, now closed: enough
    // speed to rotate fully, so the length constraint has to swap
    // which coordinate it defines every quarter turn.
    let result = compile(&with_library("spinning_pendulum.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    assert!(
        result.reselections >= 4,
        "a full rotation needs several re-selections, saw {}",
        result.reselections
    );
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (x, y, vx, vy) = (index("x"), index("y"), index("vx"), index("vy"));

    let first = &result.rows[0];
    let energy = |row: &Vec<f64>| 0.5 * (row[vx] * row[vx] + row[vy] * row[vy]) + 9.81 * row[y];
    let e0 = energy(first);
    let mut revolutions = 0;
    for pair in result.rows.windows(2) {
        // The rod length holds exactly through every switch.
        let constraint = pair[1][x] * pair[1][x] + pair[1][y] * pair[1][y] - 1.0;
        assert!(constraint.abs() < 1e-6, "constraint {constraint}");
        // And so does the energy - a wrong branch after a switch
        // (the bug this test was written against) zeroes it.
        assert!(
            (energy(&pair[1]) - e0).abs() < 1e-3,
            "energy drifted to {} from {e0}",
            energy(&pair[1])
        );
        if pair[0][y] < 0.0 && pair[1][y] >= 0.0 && pair[1][x] > 0.0 {
            revolutions += 1;
        }
    }
    assert!(revolutions >= 3, "kept rotating, saw {revolutions}");

    // The whole trajectory agrees with the angle form of the same
    // pendulum - an independent formulation of the same physics.
    let angle_form = parse_model(
        "model SpinAngle Real th(start = 0, fixed = true); Real w(start = 8, fixed = true); \
         Real x; Real y; equation der(th) = w; der(w) = -9.81 * sin(th); \
         x = sin(th); y = -cos(th); \
         annotation(experiment(StopTime = 3.0, Interval = 0.002, Tolerance = 1e-9)); \
         end SpinAngle;",
    )
    .unwrap();
    let reference = compile(&angle_form).unwrap().simulate().unwrap();
    let rx = reference.columns.iter().position(|c| c == "x").unwrap();
    let mut worst = 0.0f64;
    let mut checked = 0;
    for row in &result.rows {
        let Some(matching) = reference
            .rows
            .iter()
            .find(|other| (other[0] - row[0]).abs() < 1e-9)
        else {
            continue;
        };
        worst = worst.max((row[x] - matching[rx]).abs());
        checked += 1;
    }
    assert!(checked > 1000, "grids barely overlap: {checked}");
    assert!(worst < 1e-4, "cartesian vs angle form: {worst}");
}
