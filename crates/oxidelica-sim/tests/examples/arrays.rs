//! Models written over arrays: a discretized field, a chain of masses, a ladder of resistors.

use super::shared::*;
use oxidelica_parser::parse_model;
use oxidelica_sim::compile;

#[test]
fn discretized_heat_conduction_reaches_the_analytic_steady_state() {
    // The M5 pipeline end to end: an array of 40 nodes, a for
    // equation over the interior, and an inlined function.
    let compiled = compile(&parse_model(&example("heat_conduction.mo")).unwrap()).unwrap();
    assert_eq!(compiled.states.len(), 40, "one state per node");
    assert!(compiled.states.contains(&"T[20]".to_string()));

    let result = compiled.simulate().unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();

    // Held ends, so the rod relaxes to a straight line between them.
    for node in 1..=40 {
        let position = node as f64 / 41.0;
        let expected = 100.0 + (20.0 - 100.0) * position;
        let actual = last[index(&format!("T[{node}]"))];
        assert!(
            (actual - expected).abs() < 1e-2,
            "node {node}: {actual} vs {expected}"
        );
    }
    // The inlined steadyState() function measures the same thing.
    assert!(last[index("midError")].abs() < 1e-2);
    // Cold start: the rod begins uniform.
    assert!((result.rows[0][index("T[1]")] - 20.0).abs() < 1e-12);
}

#[test]
fn two_chains_of_different_length_share_their_functions() {
    // One `Chain` component, instantiated at three masses and at
    // five: the length is a parameter and the masses and starts
    // are handed over as whole arrays. `total` and `weighted` are
    // declared once with `[:]` inputs and measure each. Neither
    // chain is pushed from outside, so the physics has two exact
    // statements to make: momentum is constant, and the centre of
    // mass travels in a straight line.
    let result = compile(&with_library("mass_chains.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    for (chain, mass) in [("short", 6.0f64), ("long", 6.0f64)] {
        let (momentum, centre) = (
            index(&format!("{chain}.momentum")),
            index(&format!("{chain}.centre")),
        );
        let first = &result.rows[0];
        let speed = first[momentum] / mass;
        for row in &result.rows {
            assert!(
                (row[momentum] - first[momentum]).abs() < 1e-12,
                "chain {chain} at t = {}: momentum {} vs {}",
                row[0],
                row[momentum],
                first[momentum]
            );
            let wanted = first[centre] + speed * row[0];
            assert!(
                (row[centre] - wanted).abs() < 1e-12,
                "chain {chain} at t = {}: centre {} vs {wanted}",
                row[0],
                row[centre]
            );
        }
    }
    // The two chains really did start out differently.
    assert!(
        (result.rows[0][index("short.momentum")] + 0.5).abs() < 1e-12
            && result.rows[0][index("long.momentum")].abs() < 1e-12
    );
    // Each instance really did get its own length and its own
    // masses, handed over as whole arrays.
    assert!(result.columns.iter().any(|c| c == "long.x[5]"));
    assert!(!result.columns.iter().any(|c| c == "short.x[4]"));
}

#[test]
fn a_chain_of_masses_written_with_arrays_conserves_energy() {
    // Five bodies between two walls, everything about them arrays:
    // literals, fill, linspace, an array start, whole-array
    // equations and reductions. The check is physical, not textual:
    // the first body is pushed and the energy must stay put.
    let result = compile(&with_library("spring_chain.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let energy = index("energy");
    let first = result.rows[0][energy];
    // 0.5 * m[1] * push^2 with m[1] = 1 and push = 2.
    assert!((first - 2.0).abs() < 1e-9, "{first}");
    for row in &result.rows {
        assert!(
            (row[energy] - first).abs() < 1e-6,
            "drift at t = {}",
            row[0]
        );
    }
    // The bodies start on the linspace grid.
    assert!((result.rows[0][index("x[1]")] - 0.5).abs() < 1e-12);
    assert!((result.rows[0][index("x[5]")] - 2.5).abs() < 1e-12);
}

#[test]
fn a_ladder_of_resistors_wired_by_a_loop_divides_the_supply() {
    // One array declaration, `each R`, and the wiring written as a
    // loop of connects over the elements. Five equal resistors on
    // 10 V put exactly 8, 6, 4, 2, 0 volts on the taps.
    let result = compile(&with_library("resistor_ladder.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    for i in 1..=5 {
        let expected = 10.0 * (5 - i) as f64 / 5.0;
        let got = last[index(&format!("taps[{i}]"))];
        assert!(
            (got - expected).abs() < 1e-12,
            "tap {i}: {got} vs {expected}"
        );
    }
    // The same current runs through the whole chain.
    let current = last[index("r[1].i")];
    assert!((current - 10.0 / (5.0 * 220.0)).abs() < 1e-15);
    for i in 2..=5 {
        assert!((last[index(&format!("r[{i}].i"))] - current).abs() < 1e-15);
    }
}
