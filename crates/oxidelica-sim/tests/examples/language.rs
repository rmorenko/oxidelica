//! Models exercising the language itself: loops, enumerations, `inner`/`outer`, complex arithmetic.

use super::shared::*;
use oxidelica_parser::parse_model;
use oxidelica_sim::compile;

#[test]
fn a_phasor_written_in_complex_arithmetic_predicts_the_circuit() {
    // The impedance is written `R + j * X` and worked out by the
    // record's own operators; the circuit is then integrated from
    // rest with none of that in sight. After the transient has
    // died the two must agree, in amplitude and in phase.
    let result = compile(&with_library("complex_impedance.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let first = &result.rows[0];
    let (amplitude, phase) = (
        first[index("predicted_amplitude")],
        first[index("predicted_phase")],
    );

    // The phasor is exact: complex division of 10 by 2 - 0.5j.
    let (r, l, c, w, v) = (2.0f64, 0.5f64, 0.1f64, 4.0f64, 10.0f64);
    let reactance = w * l - 1.0 / (w * c);
    assert!((amplitude - v / r.hypot(reactance)).abs() < 1e-12);
    assert!((phase - (-reactance).atan2(r)).abs() < 1e-12);

    // And the circuit settles onto exactly that sine.
    for row in result.rows.iter().filter(|row| row[0] >= 10.0) {
        let wanted = amplitude * (w * row[0] + phase).sin();
        assert!(
            (row[index("i")] - wanted).abs() < 1e-6,
            "t = {}: i = {} vs {wanted}",
            row[0],
            row[index("i")]
        );
    }
}

#[test]
fn a_while_loop_computes_the_exact_large_swing_period() {
    // The function runs an arithmetic-geometric mean to convergence
    // at compile time; the simulated pendulum must come back to its
    // amplitude exactly one such period later.
    let result = compile(&with_library("pendulum_period.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();

    let (length, gravity, amplitude) = (1.2f64, 9.81f64, 1.0f64);
    let (mut a, mut b) = (1.0f64, (amplitude / 2.0).cos());
    while (a - b).abs() > 1e-15 {
        let mean = 0.5 * (a + b);
        b = (a * b).sqrt();
        a = mean;
    }
    let period = 2.0 * std::f64::consts::PI * (length / gravity).sqrt() / a;
    assert!(
        (result.rows[0][index("period")] - period).abs() < 1e-12,
        "period {} vs {period}",
        result.rows[0][index("period")]
    );
    // Small-angle theory would say 2.1972 s; the true period at a
    // 1 rad swing is 2.3430 s, and the trajectory knows it.
    let row = result
        .rows
        .iter()
        .min_by(|p, q| {
            let (dp, dq) = ((p[0] - period).abs(), (q[0] - period).abs());
            dp.partial_cmp(&dq).unwrap()
        })
        .unwrap();
    assert!(
        (row[index("theta")] - amplitude).abs() < 1e-3,
        "theta {} after one period",
        row[index("theta")]
    );
    assert!(
        row[index("w")].abs() < 0.05,
        "w {} at the turn",
        row[index("w")]
    );
}

#[test]
fn a_world_shared_through_inner_outer_drives_a_projectile() {
    // The point mass reads gravity from the `inner World` of the top
    // model; the trajectory is a polynomial the solver integrates
    // exactly.
    let result = compile(&with_library("projectile.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (x, y, vy) = (index("ball.x"), index("ball.y"), index("ball.vy"));
    for row in &result.rows {
        let t = row[0];
        assert!((row[x] - 12.0 * t).abs() < 1e-12, "x at {t}: {}", row[x]);
        let height = 16.0 * t - 0.5 * 9.81 * t * t;
        assert!((row[y] - height).abs() < 1e-12, "y at {t}: {}", row[y]);
        assert!((row[vy] - (16.0 - 9.81 * t)).abs() < 1e-12);
    }
}

#[test]
fn an_enumeration_selects_the_shape_of_a_waveform() {
    // Square wave through a first-order lag: on the first half
    // period the answer is the analytic step response, and the jump
    // at the half period is an event the solver stops at.
    let result = compile(&with_library("waveform.mo"))
        .unwrap()
        .simulate()
        .unwrap();
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (u, y) = (index("u"), index("y"));
    for row in &result.rows {
        let t = row[0];
        assert!(row[u].abs() == 1.0, "square wave value {} at {t}", row[u]);
        if t <= 1.0 - 1e-9 {
            let expected = 1.0 - (-t / 0.3f64).exp();
            assert!((row[y] - expected).abs() < 1e-6, "y at {t}: {}", row[y]);
        }
    }

    // The triangle shape of the same source is asin of a sine.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let library = std::fs::read_to_string(root.join("lib/Oxidelica.mo")).unwrap();
    let model = oxidelica_parser::parse_model_with_libraries(
        &[library],
        "model T Oxidelica.Blocks.Sources.Waveform source(\
           kind = Oxidelica.Types.WaveformKind.Triangle, f = 0.5); \
         Real u; equation u = source.y; \
         annotation(experiment(StopTime = 3.0, Interval = 0.01)); end T;",
    )
    .unwrap();
    let triangle = compile(&model).unwrap().simulate().unwrap();
    let u = triangle.columns.iter().position(|c| c == "u").unwrap();
    for row in &triangle.rows {
        let expected = 2.0 * (std::f64::consts::PI * row[0]).sin().asin() / std::f64::consts::PI;
        assert!((row[u] - expected).abs() < 1e-12);
    }
}

#[test]
fn random_draw_draws_the_stream_the_algorithm_defines() {
    let result = compile(&parse_model(&example("random_draw.mo")).unwrap())
        .unwrap()
        .simulate()
        .unwrap();
    let column = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let (r, low, high) = (column("r"), column("state[1]"), column("state[2]"));

    // The same stream, worked out here from the algorithm's own
    // definition rather than from this compiler's copy of it: three
    // shifts and three exclusive-ors move the state, and the number is
    // the state times a fixed odd multiplier, scaled into (0, 1].
    let mut state: u64 = 126247697;
    let mut drawn = Vec::new();
    for _ in 0..4 {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        // The word is read as signed and moved half a step up, which
        // is what the standard library's own C does: `x*INVM64 + 0.5`.
        let value =
            state.wrapping_mul(2685821657736338717) as i64 as f64 * 5.421_010_862_427_522e-20 + 0.5;
        let half = |value: u64| ((value & 0xffff_ffff) as u32 as i32) as f64;
        drawn.push((value, half(state), half(state >> 32)));
    }

    // Each draw lands at its sample and holds until the next.
    for (step, (value, low_half, high_half)) in drawn.iter().enumerate() {
        // The first sample is at zero, so draw `k` lands at `0.25k`.
        let at = 0.25 * step as f64;
        let row = result
            .rows
            .iter()
            .rev()
            .find(|row| row[0] <= at + 1e-9)
            .unwrap();
        assert!(
            (row[r] - value).abs() < 1e-12,
            "at {at}: {} vs {value}",
            row[r]
        );
        assert_eq!(row[low], *low_half, "at {at}");
        assert_eq!(row[high], *high_half, "at {at}");
    }

    // Every number a generator of this kind draws is in (0, 1].
    for row in &result.rows {
        assert!(row[r] >= 0.0 && row[r] <= 1.0, "{}", row[r]);
    }
}
