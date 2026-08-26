//! Models built out of standard-library components, checked against what theory says they do.

use super::shared::*;
use oxidelica_sim::{compile, SimResult};

#[test]
fn a_control_loop_wired_through_a_bus_closes() {
    // Nothing in the model wires the plant to the controller
    // directly: both talk to an expandable bus, and a sub-bus
    // carries the same members because it is joined to it. The
    // loop that comes out settles at k*r/(1+k) with time constant
    // T/(1+k).
    let result = compile(&with_library("signal_bus.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (k, r, t_plant) = (4.0f64, 1.0f64, 0.5f64);
    let settled = k * r / (1.0 + k);
    let tau = t_plant / (1.0 + k);
    for row in &result.rows {
        let expected = settled * (1.0 - (-row[0] / tau).exp());
        assert!(
            (row[index("plant.x")] - expected).abs() < 1e-6,
            "t = {}: x = {} vs {expected}",
            row[0],
            row[index("plant.x")]
        );
    }
    // The bus and the sub-bus really do carry the same signal.
    let last = result.rows.last().unwrap();
    assert_eq!(
        last[index("bus.measurement.y")],
        last[index("subbus.measurement.y")]
    );
    assert_eq!(
        last[index("bus.command.y")],
        last[index("subbus.command.y")]
    );
    // What the plant is driven with is the law applied to what the
    // controller heard, both of which travelled through the bus.
    let heard = last[index("controller.measurement.y")];
    assert!((last[index("plant.u.y")] - k * (r - heard)).abs() < 1e-12);
    assert!((heard - settled).abs() < 1e-4, "not settled: {heard}");
}

#[test]
fn dc_motor_from_library_components_matches_theory() {
    // Three domains at once: an electrical circuit, the EMF
    // coupling and a rotational load, all from library packages.
    let result = compile(&with_library("dc_motor.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();

    // Steady state: k*i = d*w and V = i*R + k*w.
    let (v, r, k, d) = (24.0, 0.5, 0.3, 0.02);
    let speed = v * k / (k * k + d * r);
    assert!(
        (last[index("speed")] - speed).abs() < 1e-2,
        "speed {} vs {speed}",
        last[index("speed")]
    );
    assert!((last[index("current")] - d * speed / k).abs() < 1e-3);

    // The supply steps at t = 0.1, so nothing moves before that.
    let early = result
        .rows
        .iter()
        .find(|row| row[0] >= 0.05)
        .expect("a sample before the step");
    assert!(early[index("speed")].abs() < 1e-9);
}

#[test]
fn pi_control_loop_from_library_blocks_settles_on_the_setpoint() {
    let result = compile(&with_library("control_loop.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    // Integral action removes the steady-state error.
    assert!(
        last[index("e")].abs() < 1e-4,
        "residual error {}",
        last[index("e")]
    );
    assert!((last[index("y")] - 1.0).abs() < 1e-4);
}

#[test]
fn a_conditional_support_carries_the_reaction_torque() {
    // Two identical drives: one reacting on its internal housing,
    // one on an exposed support flange. The shafts must not be able
    // to tell the difference.
    let result = compile(&with_library("torque_support.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    for row in &result.rows {
        assert!(row[index("difference")].abs() < 1e-12);
    }
    // phi = tau / (2 J) t^2 with tau = 2, J = 0.5, t = 4.
    assert!((last[index("shaftA.phi")] - 32.0).abs() < 1e-9);
    // The exposed support takes the reaction; the internal one hides it.
    assert!((last[index("driveB.support.tau")] - 2.0).abs() < 1e-12);
    assert!(!result.columns.iter().any(|c| c == "driveA.support.tau"));
}

#[test]
fn a_damper_straight_onto_a_fixed_flange_works() {
    // The former known limit: the damper's relative angle is
    // redundant with the shaft angle, and reducing the index means
    // differentiating a connection equality through connector
    // potentials no equation defines explicitly - they are pinned
    // only linearly. J = 0.5, d = 0.4: the shaft speed must decay
    // as 5*exp(-0.8 t).
    let result = compile(&with_library("damper_on_fixed.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (w, phi, phi_rel) = (
        index("shaft.w"),
        index("shaft.phi"),
        index("damper.phi_rel"),
    );
    for row in &result.rows {
        let expected = 5.0 * (-0.8 * row[0]).exp();
        assert!(
            (row[w] - expected).abs() < 1e-6,
            "w at {}: {} vs {expected}",
            row[0],
            row[w]
        );
        // The relative angle mirrors the shaft, holding the
        // redundant pair consistent through the whole run.
        assert!((row[phi_rel] + row[phi]).abs() < 1e-9);
    }
}

#[test]
fn a_replaceable_medium_changes_what_the_tank_holds() {
    // The example file ends with the oil variant, so that is the
    // entry point: heating follows oil's density and heat capacity.
    let oil = compile(&with_library("replaceable_medium.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index =
        |result: &SimResult, name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let temperature = index(&oil, "T");
    let last = oil.rows.last().unwrap();
    let expected_oil = 20.0 + 600.0 * 50000.0 / (0.2 * 900.0 * 1900.0);
    assert!(
        (last[temperature] - expected_oil).abs() < 1e-6,
        "oil: {} vs {expected_oil}",
        last[temperature]
    );
    // And the viscosity comes from oil's own function.
    let viscosity = index(&oil, "mu");
    let expected_mu = 0.1 * (-0.05f64 * (expected_oil - 20.0)).exp();
    assert!((last[viscosity] - expected_mu).abs() < 1e-6);

    // The same tank with its default medium heats like water.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = std::fs::read_to_string(root.join("examples/replaceable_medium.mo")).unwrap();
    let water_only = source
        .replace("model OilTank", "partial model OilTank")
        .replace(
            "end OilTank;",
            "end OilTank; model WaterTank extends HeatedTank; \
             annotation(experiment(StopTime = 600.0, Interval = 1.0)); end WaterTank;",
        );
    let water = compile(&oxidelica_parser::parse_model(&water_only).unwrap())
        .unwrap()
        .simulate()
        .unwrap();
    let temperature = index(&water, "T");
    let expected_water = 20.0 + 600.0 * 50000.0 / (0.2 * 1000.0 * 4186.0);
    let last = water.rows.last().unwrap();
    assert!(
        (last[temperature] - expected_water).abs() < 1e-6,
        "water: {} vs {expected_water}",
        last[temperature]
    );
}
