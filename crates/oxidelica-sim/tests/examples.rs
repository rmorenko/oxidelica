//! The examples in `examples/`, run and held to what physics says.
//!
//! Every one of these compiles a model that ships with the project and
//! checks the answer against a closed form or an independent
//! formulation, so a change that quietly breaks an example is caught
//! here rather than by a reader.

use oxidelica_parser::parse_model;
use oxidelica_sim::{compile, SimResult, SolverMethod};

fn example(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name),
    )
    .unwrap()
}

fn with_library(name: &str) -> oxidelica_parser::Model {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let library = std::fs::read_to_string(root.join("lib/Oxidelica.mo")).unwrap();
    let source = std::fs::read_to_string(root.join("examples").join(name)).unwrap();
    oxidelica_parser::parse_model_with_libraries(&[library], &source).unwrap()
}

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
