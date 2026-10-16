//! Adaptive Dormand-Prince 5(4) with dense output.

use crate::*;

use super::{reject_keeps_guess, turned, Segment, SegmentStart};

/// Whether a refusal only the implicit solver mends ends an `Auto` run
/// in the explicit one, as it did before such a run was handed over.
/// `OXIDELICA_NO_IMPLICIT_HANDOFF` keeps the old reading, so that one
/// binary gives both numbers.
fn implicit_handoff_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var_os("OXIDELICA_NO_IMPLICIT_HANDOFF").is_some())
}

/// Whether a step found to hold a crossing inside it is interpolated up
/// to the crossing, as it was before such a step was taken again
/// shorter. `OXIDELICA_NO_EVENT_RETAKE` keeps the old reading, so that
/// one binary gives both numbers.
fn event_retake_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var_os("OXIDELICA_NO_EVENT_RETAKE").is_some())
}

/// Whether a step holding a crossing is left uncut when the crossing is
/// in its first tenth, or the step follows a cut one, even with an
/// output point before the crossing, as it was before.
/// `OXIDELICA_NO_CUT_BEFORE_ROW` keeps the old reading, so that one
/// binary gives both numbers.
fn cut_before_row_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var_os("OXIDELICA_NO_CUT_BEFORE_ROW").is_some())
}

/// Whether the event indicators are read only at the two ends of a
/// step, as they were before the stages inside it were read too.
/// `OXIDELICA_NO_STAGE_CROSSINGS` keeps the old reading, so that one
/// binary gives both numbers.
fn stage_crossings_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var_os("OXIDELICA_NO_STAGE_CROSSINGS").is_some())
}

/// Where inside a step an event indicator that reads the same sign at
/// both ends nonetheless turned, as a fraction of the step, read off the
/// values it took at the stages. `None` when nothing between the ends
/// says it turned.
///
/// Two things say so. A stage reading the other sign from the start
/// is the plain one, and the step is to end there. The other is a pair
/// of crossings that falls between two stages, so that every reading
/// keeps its sign: the readings then come close to the threshold and go
/// away again, and the parabola through the three around the closest
/// one reaches past it. The step is to end where that parabola turns,
/// which is where the indicator stands farthest on the other side if it
/// went there at all; if it did not, the shorter step costs one step and
/// finds nothing.
///
/// A reading counts as on the other side only past `floor`: an
/// indicator resting on zero - the velocity of a stuck friction element -
/// changes sign on rounding alone, at 1e-16 and below, and reading that
/// as a turn cut `Translational.Examples.Friction` into eleven times its
/// points.
fn hidden_crossing(
    before: &[f64],
    stages: &[(f64, Vec<f64>)],
    after: &[f64],
    floor: f64,
) -> Option<f64> {
    let mut earliest: Option<f64> = None;
    let mut take = |theta: f64| {
        if theta > 0.0 && theta < 1.0 {
            earliest = Some(earliest.map_or(theta, |e: f64| e.min(theta)));
        }
    };
    for (index, (&start, &end)) in before.iter().zip(after).enumerate() {
        // Ends that differ are the search's to find, and a zero at either
        // end is a relation on its threshold, which the search reads too.
        if start * end <= 0.0 {
            continue;
        }
        let side = start.signum();
        // The readings in the order they stand inside the step, each
        // measured as how far it is on the starting side.
        let mut points: Vec<(f64, f64)> = Vec::with_capacity(stages.len() + 2);
        points.push((0.0, start * side));
        points.extend(
            stages
                .iter()
                .map(|(c, reading)| (*c, reading[index] * side)),
        );
        points.push((1.0, end * side));
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        if let Some(&(c, _)) = points.iter().find(|&&(_, v)| v < -floor) {
            take(c);
            continue;
        }
        for window in points.windows(3) {
            let [(x0, v0), (x1, v1), (x2, v2)] = [window[0], window[1], window[2]];
            if !(v1 <= v0 && v1 <= v2) || x0 == x1 || x1 == x2 {
                continue;
            }
            // The parabola through the three, by its divided differences.
            let d01 = (v1 - v0) / (x1 - x0);
            let d12 = (v2 - v1) / (x2 - x1);
            let curvature = (d12 - d01) / (x2 - x0);
            if curvature <= 0.0 {
                continue;
            }
            let vertex = 0.5 * (x0 + x1) - d01 / (2.0 * curvature);
            let lowest = v1 + d01 * (vertex - x1) + curvature * (vertex - x0) * (vertex - x1);
            if lowest < -floor && vertex > x0 && vertex < x2 {
                take(vertex);
            }
        }
    }
    earliest
}

impl CompiledModel {
    /// Adaptive Dormand-Prince 5(4) integration with dense output.
    pub fn simulate_adaptive(&self) -> Result<SimResult, SimError> {
        match self.adaptive(false)? {
            AdaptiveOutcome::Finished(result) => Ok(result),
            AdaptiveOutcome::Stiff => err("the stiffness watch fired without being asked"),
            AdaptiveOutcome::Stalled(stall) => err(format!(
                "step size underflow at t = {:.6}: probable singularity",
                stall.time
            )),
        }
    }

    /// Adaptive Dormand-Prince 5(4) integration with dense output on
    /// the `step` grid. The step size shrinks automatically near sharp
    /// dynamics (close encounters, kinks) and grows on smooth stretches;
    /// a persistent step-size underflow is reported as a probable
    /// singularity instead of returning garbage.
    ///
    /// With `watch_stiffness` the run gives up as soon as the step size
    /// is clearly limited by stability rather than accuracy, so that the
    /// caller can hand the problem to the implicit solver.
    pub(crate) fn adaptive(&self, watch_stiffness: bool) -> Result<AdaptiveOutcome, SimError> {
        // Dormand-Prince Butcher tableau.
        const C: [f64; 7] = [0.0, 0.2, 0.3, 0.8, 8.0 / 9.0, 1.0, 1.0];
        const A: [[f64; 6]; 7] = [
            [0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            [0.2, 0.0, 0.0, 0.0, 0.0, 0.0],
            [3.0 / 40.0, 9.0 / 40.0, 0.0, 0.0, 0.0, 0.0],
            [44.0 / 45.0, -56.0 / 15.0, 32.0 / 9.0, 0.0, 0.0, 0.0],
            [
                19372.0 / 6561.0,
                -25360.0 / 2187.0,
                64448.0 / 6561.0,
                -212.0 / 729.0,
                0.0,
                0.0,
            ],
            [
                9017.0 / 3168.0,
                -355.0 / 33.0,
                46732.0 / 5247.0,
                49.0 / 176.0,
                -5103.0 / 18656.0,
                0.0,
            ],
            [
                35.0 / 384.0,
                0.0,
                500.0 / 1113.0,
                125.0 / 192.0,
                -2187.0 / 6784.0,
                11.0 / 84.0,
            ],
        ];
        /// 5th-order weights (the last tableau row, FSAL).
        const B5: [f64; 7] = [
            35.0 / 384.0,
            0.0,
            500.0 / 1113.0,
            125.0 / 192.0,
            -2187.0 / 6784.0,
            11.0 / 84.0,
            0.0,
        ];
        /// 4th-order weights of the embedded method.
        const B4: [f64; 7] = [
            5179.0 / 57600.0,
            0.0,
            7571.0 / 16695.0,
            393.0 / 640.0,
            -92097.0 / 339200.0,
            187.0 / 2100.0,
            1.0 / 40.0,
        ];

        let n = self.states.len();
        let stop = self.stop_time;
        let out_step = self.step.max(1e-12);
        let rtol = self.tolerance;
        let atol = self.tolerance * 1e-3;

        let segment = match self.begin_segment(SolverMethod::Dopri45)? {
            SegmentStart::Finished(result) => return Ok(AdaptiveOutcome::Finished(result)),
            SegmentStart::Running(segment) => *segment,
        };
        if n == 0 {
            return self.walk_without_states(segment, SolverMethod::Dopri45);
        }
        let Segment {
            mut values,
            columns,
            mut rows,
            mut y,
            mut alg_guess,
            scratch: mut derivatives_scratch,
            mut state,
            mut indicators_prev,
            mut out_i,
            mut last_out_t,
            mut terminated,
        } = segment;
        // Where the last event was handled, so a relation left standing
        // on its threshold there is not taken for one turning here.
        let mut handled_at = f64::NAN;
        let t0 = self.start_time;

        let mut k: Vec<Vec<f64>> = vec![vec![0.0; n]; 7];
        let mut stage = vec![0.0; n];
        let mut penultimate = vec![0.0; n];
        // Steps that looked stiff, and the calm ones that clear them.
        // A single step over the boundary is noise - a kink, an event, a
        // bad guess; a run of them is the method saying it is held back
        // by stability rather than accuracy.
        let (mut stiff_steps, mut calm_steps) = (0usize, 0usize);
        let mut y5 = vec![0.0; n];
        let mut interp = vec![0.0; n];
        let mut t = t0;
        let mut h = out_step.min(stop - t0).max(1e-9);
        let mut evals: u64 = 0;
        // Whether the step about to be taken was cut to end at a crossing
        // the longer one found inside it. A cut step that finds the
        // crossing again, still inside it - its stages see a slightly
        // different curve - is not cut a second time, or the run would
        // creep up to the crossing by ever shorter steps; it is near
        // enough by then for the cubic to be read. A cut step that ends
        // short of the crossing is accepted like any other, and the flag
        // clears with it: kept until the next event, it let a step much
        // later interpolate straight through a kink of its own
        // (`CCCV_Stack` on a 755 s horizon, cut at 750.21, then bowed
        // past its limit by 753.85).
        let mut retaken = false;
        // Whether the step about to be taken is the one right after a
        // cut step accepted short of its crossing. That one step is not
        // cut again: the cut lands a little short, the next step finds
        // the same crossing a few hundredths in, and cutting it too is
        // how `ComparisonQuasiStatic`, whose relations sit within a
        // rounding unit of zero from the start, was cut eight thousand
        // times on the way to t = 1e-6 and refused with a step size
        // underflow. It holds for that one step only; the one after is
        // free to be cut.
        let mut spared = false;
        // Whether the step about to be taken was already shortened for a
        // crossing its stages saw and its ends did not.
        let mut stage_cut = false;
        // Where the step cut short of a crossing would have ended, and
        // what the indicators read there, until a step has passed it.
        let mut long_end: Option<(f64, Vec<f64>)> = None;

        self.eval_point(t, &y, &mut values, &mut k[0], &mut alg_guess)?;

        /// On a model whose states were chosen by a pivot, a failing
        /// evaluation is the choice failing: stop here and let the
        /// caller choose again. Everything already recorded stays.
        macro_rules! or_stall {
            ($attempt:expr) => {
                match $attempt {
                    Ok(value) => value,
                    Err(_) if self.reselectable => {
                        return self.stall_at_last_row(columns, rows, SolverMethod::Dopri45, false);
                    }
                    Err(error) => return Err(error),
                }
            };
        }

        while t < stop - 1e-12 {
            h = h.min(stop - t).min(self.delay_step_limit());
            // A scheduled time event is not something to step over: the
            // step ends exactly on it.
            if let Some(next) = state.next_time_event() {
                if next > t + 1e-12 && next <= stop + 1e-12 {
                    h = h.min(next - t);
                }
            }
            // Stages 2..7 (stage 1 is FSAL from the previous step).
            // What the algebraic blocks start from at the point this
            // step leaves, kept so that a step thrown away does not
            // hand its stages' roots on as the start of the next try.
            // A stage at t + h/5 of a step too long for a stiff state
            // stands where the state never goes, and a block with more
            // than one root - a saturating iron, `y^3 - 3 y = x` -
            // converges there onto a root the run never reaches.
            // Every shorter try then starts its blocks from that root
            // and is solved on it: `ComparisonQuasiStatic` at a
            // difference step of 1.1e-8 halved its step from 2e-4 down
            // to 1e-15 with each try starting the magnetic circuit from
            // 7e24, and was refused at t = 0. Rejected, the step leaves
            // nothing behind.
            let footing_guess = alg_guess.clone();
            let mut stage_failed = false;
            // What the event indicators read at the stages inside the
            // step, by the fraction of the step each stands at.
            let mut stage_readings: Vec<(f64, Vec<f64>)> = Vec::new();
            let read_stages = !self.indicators.is_empty() && !stage_crossings_off();
            for s in 1..7 {
                for j in 0..n {
                    let mut acc = 0.0;
                    for (q, k_q) in k.iter().enumerate().take(s) {
                        acc += A[s][q] * k_q[j];
                    }
                    stage[j] = y[j] + h * acc;
                }
                if s == 5 {
                    // The sixth stage sits at t + h, like the solution
                    // itself but at a different point; the gap between
                    // the two, and between their derivatives, is what
                    // the stiffness estimate is made of.
                    penultimate.copy_from_slice(&stage);
                }
                let (head, tail) = k.split_at_mut(s);
                let _ = head;
                match self.eval_point(
                    t + C[s] * h,
                    &stage,
                    &mut values,
                    &mut tail[0],
                    &mut alg_guess,
                ) {
                    Ok(()) => {
                        if read_stages && C[s] < 1.0 {
                            stage_readings
                                .push((C[s], self.indicator_values(t + C[s] * h, &values)));
                        }
                    }
                    // A dying algebraic loop on a model whose states were
                    // *chosen* is likely the choice failing, not the
                    // model: treat the step as rejected and let the step
                    // size fall toward the stall check below. So is a
                    // refusal of the kind a stage too long provokes and a
                    // shorter step does not (see `Refusal`), on any model.
                    Err(ref error) if self.reselectable || error.smaller_step_mends() => {
                        stage_failed = true;
                        break;
                    }
                    // A refusal only the implicit solver takes for a step
                    // too long is not this solver's to end the run on
                    // when the implicit one is standing by: under `Auto`
                    // the run is handed over, as it is for stiffness.
                    Err(ref error)
                        if watch_stiffness
                            && !implicit_handoff_off()
                            && error.hands_to_implicit(super::edge_handoff().takes(t > t0)) =>
                    {
                        return Ok(AdaptiveOutcome::Stiff);
                    }
                    Err(error) => return Err(error),
                }
            }
            evals += 6;
            if evals > 20_000_000 {
                return err(format!(
                    "solver exceeded the evaluation budget at t = {t:.6}"
                ));
            }

            // 5th-order solution and the embedded error estimate.
            let mut err_norm = if stage_failed { f64::INFINITY } else { 0.0f64 };
            for j in 0..n {
                let mut acc5 = 0.0;
                let mut acc4 = 0.0;
                for q in 0..7 {
                    acc5 += B5[q] * k[q][j];
                    acc4 += B4[q] * k[q][j];
                }
                let y5j = y[j] + h * acc5;
                let y4j = y[j] + h * acc4;
                let scale = atol + rtol * y[j].abs().max(y5j.abs());
                let e = (y5j - y4j) / scale;
                err_norm += e * e;
                y5[j] = y5j;
            }
            err_norm = (err_norm / n as f64).sqrt();

            let accepted = err_norm.is_finite() && err_norm <= 1.0;
            // A relation that turns and turns back inside one step reads
            // the same at both ends, and a search that only compares the
            // ends never learns it turned. On a smooth state the step
            // grows past half the period of whatever the relation reads,
            // and from then on whole pairs of crossings fall inside
            // steps: `der(x) = 1` beside `when sin(20*time) > 0.5` fired
            // once in a second where it rises four times, and a Boolean
            // read through `BooleanToReal` into an integrator stuck at
            // true and integrated to 0.974 where the answer is 0.340.
            // The stages inside the step already stand at points between
            // its ends, and `hidden_crossing` reads them for a turn the
            // ends cannot see; the step is then taken again ending where
            // the indicator reads the other sign, so that the two ends
            // differ and the search below finds the crossing. Once per
            // step: the shorter try is accepted whatever its stages say,
            // or a curve that only grazes its threshold would be chased
            // by ever shorter steps.
            if accepted && !stage_readings.is_empty() && !stage_cut {
                let indicators_end = self.indicator_values(t + h, &values);
                let mut retake =
                    hidden_crossing(&indicators_prev, &stage_readings, &indicators_end, atol)
                        .map(|theta| h * theta);
                // A step cut short of a crossing ends two hairs before
                // it, and the step after starts there with the crossing a
                // hair in. Grown fivefold, its first stage can stand past
                // the whole of a dip that begins there: `cos(20*time) <
                // -0.5` beside `der(x) = 1` was cut 3e-9 short of its
                // second fall, the step after grew to 0.58 with its first
                // stage at 0.12, and the dip, 0.10 long, lay before it.
                // The long step that was cut saw the crossing at its far
                // end; a step that passes that end without seeing it is
                // taken again ending there, where the long step saw it.
                // Holding every step after a cut to the cut one's length
                // instead cost `ActuatorWithNoise` eight times its points.
                if retake.is_none() {
                    if let Some((t_end, ends)) = &long_end {
                        let missed = t + h > *t_end + 1e-12
                            && *t_end > t
                            && indicators_prev.iter().zip(ends).zip(&indicators_end).any(
                                |((&now, &there), &after)| {
                                    now * there < 0.0 && there.abs() > atol && !turned(now, after)
                                },
                            );
                        if missed {
                            retake = Some(*t_end - t);
                        }
                    }
                }
                if let Some(shorter) = retake {
                    stage_cut = true;
                    h = shorter;
                    if !reject_keeps_guess() {
                        alg_guess.copy_from_slice(&footing_guess);
                    }
                    continue;
                }
            }
            if accepted {
                stage_cut = false;
            }
            if accepted {
                // FSAL: the derivative at t+h.
                match self.eval_point(t + h, &y5, &mut values, &mut k[6], &mut alg_guess) {
                    Ok(()) => {}
                    // The point passed the error estimate but the
                    // algebraic layer will not hold there: on a chosen
                    // selection, reject the step and shrink instead.
                    Err(_) if self.reselectable => {
                        h *= 0.2;
                        if !reject_keeps_guess() {
                            alg_guess.copy_from_slice(&footing_guess);
                        }
                        if h < stop * 5e-14 {
                            return self.stall_at_last_row(
                                columns,
                                rows,
                                SolverMethod::Dopri45,
                                false,
                            );
                        }
                        continue;
                    }
                    Err(error) => return Err(error),
                }
                evals += 1;

                if watch_stiffness {
                    // h times the dominant eigenvalue of the Jacobian,
                    // read off two points at t + h the step has already
                    // produced. Past the stability limit of the method
                    // (3.25 for Dormand-Prince) the step size is no
                    // longer about accuracy.
                    let (mut numerator, mut denominator) = (0.0f64, 0.0f64);
                    for j in 0..n {
                        numerator += (k[6][j] - k[5][j]).powi(2);
                        denominator += (y5[j] - penultimate[j]).powi(2);
                    }
                    if denominator > 0.0 && h * (numerator / denominator).sqrt() > 3.25 {
                        calm_steps = 0;
                        stiff_steps += 1;
                        if stiff_steps >= 15 {
                            return Ok(AdaptiveOutcome::Stiff);
                        }
                    } else {
                        // The estimate hovers around the boundary on a
                        // stiff problem, so a single step below it means
                        // nothing; only a calm stretch clears the count.
                        calm_steps += 1;
                        if calm_steps >= 6 {
                            stiff_steps = 0;
                        }
                    }
                }

                // Cubic Hermite interpolation across the accepted step.
                let interpolate = |theta: f64, out: &mut Vec<f64>, y: &[f64], k: &[Vec<f64>]| {
                    out.clear();
                    for j in 0..n {
                        let (y0, y1) = (y[j], y5[j]);
                        let (f0, f1) = (k[0][j], k[6][j]);
                        out.push(
                            (1.0 - theta) * y0
                                + theta * y1
                                + theta
                                    * (theta - 1.0)
                                    * ((1.0 - 2.0 * theta) * (y1 - y0)
                                        + (theta - 1.0) * h * f0
                                        + theta * h * f1),
                        );
                    }
                };

                // Did any event indicator change sign across the step?
                let indicators_new = self.indicator_values(t + h, &values);
                // A reading of exactly zero where the step begins is a
                // relation standing on its threshold: it turns as the
                // step leaves, and there is nothing to search for.
                // Reading it as an indicator to leave alone is what used
                // to lose a condition turning on an output point.
                const HAIR: f64 = 1e-9;
                let mut event_theta: Option<f64> = None;
                for (index, (&before, &after)) in
                    indicators_prev.iter().zip(&indicators_new).enumerate()
                {
                    if !turned(before, after) {
                        continue;
                    }
                    if before == 0.0 {
                        // Unless this is the instant an event was just
                        // handled at: the reading there is zero because
                        // the crossing is behind, not ahead, and taking
                        // it at the instant itself would leave the run
                        // standing still.
                        //
                        // Standing still is what had to be avoided, and
                        // dropping the crossing is not the only way to
                        // avoid it - it is the way that loses the
                        // crossing for good. A relation resting exactly
                        // on its threshold and leaving it over the step
                        // is the shape a sliding mode makes: the event
                        // settles on the threshold, the step carries
                        // the state off it, and the reading at the far
                        // end has turned. Skipped, the switch keeps the
                        // value it had, and it keeps it for the whole
                        // rest of the run: `der(x) = if x > 0.5 then -1
                        // else 1` walks x out to 2 with its own
                        // condition reading false the entire way. That
                        // is not a slow answer, it is a wrong one, and
                        // a wrong number is the worst thing this
                        // compiler can do.
                        //
                        // So the event is raised at the far end of the
                        // step rather than at the instant. The run
                        // advances by a whole step, which is what the
                        // guard was protecting, and the switch is
                        // tested where it has actually turned.
                        let theta = if t == handled_at { 1.0 } else { HAIR };
                        event_theta = Some(event_theta.map_or(theta, |c: f64| c.min(theta)));
                        continue;
                    }
                    let (mut lo, mut hi) = (0.0f64, 1.0f64);
                    for _ in 0..40 {
                        let mid = 0.5 * (lo + hi);
                        interpolate(mid, &mut interp, &y, &k);
                        or_stall!(self.eval_point(
                            t + mid * h,
                            &interp,
                            &mut values,
                            &mut derivatives_scratch,
                            &mut alg_guess,
                        ));
                        if before * self.indicator_values(t + mid * h, &values)[index] <= 0.0 {
                            hi = mid;
                        } else {
                            lo = mid;
                        }
                    }
                    // A hair past the crossing, so the relation has
                    // definitely switched when the condition is tested.
                    let crossed = (hi + 1e-9).min(1.0);
                    event_theta = Some(event_theta.map_or(crossed, |c: f64| c.min(crossed)));
                }

                // A cut step whose far end reads a relation exactly on
                // its threshold has arrived at the crossing it was cut
                // for, and ends in the event there. Taken as a plain
                // step, the next one starts from a reading of zero and
                // finds the turn a hair in, again and again: `Z2`, a
                // clamp let go when its input turns, raised ten thousand
                // events at t = 5.1676. Only on an exact zero: a cut
                // step that ends short with the relation unturned is no
                // place for an event, and raising one there anyway kept
                // `DiodeBridge2mPulse` handling events at t = 0.00167
                // without the switch ever moving.
                let landed_on_threshold = indicators_prev
                    .iter()
                    .zip(&indicators_new)
                    .any(|(&before, &after)| before != 0.0 && after == 0.0);
                if retaken && event_theta.is_none() && landed_on_threshold && !event_retake_off() {
                    event_theta = Some(1.0);
                }
                if let Some(theta) = event_theta {
                    // A crossing inside the step means the stages
                    // straddled the switch, and the step's far end and its
                    // slope there belong to the other branch. The cubic
                    // through the two ends is then not the solution on
                    // this side of the crossing: across the kink of a
                    // clamped integrator, `der(y) = if y > outMax and
                    // k*u > 0 then 0 else k*u`, it bowed a held battery
                    // charge up past its limit before the switch was
                    // reached (`CCCV_Stack`, a step of 14 s carrying
                    // the cell to 1 + 1.06e-9 where the model's own
                    // assert stops at 1 + 1e-9). So the step is taken
                    // again, once, ending at the crossing, with every
                    // stage on the side the run is on. What that shorter
                    // step finds is then used as before. A crossing in
                    // the first tenth of the step is not cut for: the bow
                    // grows with the square of how far into the step the
                    // cubic is read, so there it is a hundredth of what it
                    // is at the middle. And cutting there is what never
                    // ends - a cut step lands a little short of the
                    // crossing, the next one finds it a few hundredths
                    // in, is cut again, and the step falls tenfold a
                    // round until it underflows (`CCCV_Stack` at
                    // t = 679.629, twelve cuts from 4 s to 2e-13 s).
                    //
                    // Neither exemption holds where an output point lies
                    // before the crossing. That row is read off the
                    // cubic whatever theta is, and its error is not a
                    // hundredth of anything: it is the bow at wherever
                    // the row falls, and a step of 27 s crossing 1.6 s
                    // in left fifteen rows of a held charge to be read
                    // off a curve whose far end belongs to the other
                    // branch (`CCCV_Stack` on its own interval of 0.1 s,
                    // the cell at 1 + 2.2e-9 and the model's assert at
                    // t = 741.5; `K3`, eight lines, 4.2e-6 over its
                    // limit). Nor can it run away: the row is asked to
                    // lie inside the cut step, which writes it, so the
                    // next step has it behind and there are no more such
                    // cuts than there are rows.
                    let row_before = !cut_before_row_off() && {
                        let next_out = out_i as f64 * out_step;
                        next_out > t + 1e-12 && next_out < t + (theta - 2.0 * HAIR) * h
                    };
                    if !retaken
                        && (row_before || (!spared && theta > 0.1))
                        && theta < 1.0
                        && !event_retake_off()
                    {
                        retaken = true;
                        long_end = Some((t + h, indicators_new.clone()));
                        // The search put `theta` a hair past the
                        // crossing; the cut step ends a hair before it,
                        // so that none of its stages, nor its far end,
                        // stands on the other branch. Ending on the
                        // crossing itself still let the cubic bow by
                        // 1.6e-13 over a value held at its limit (`K`).
                        h *= theta - 2.0 * HAIR;
                        if !reject_keeps_guess() {
                            alg_guess.copy_from_slice(&footing_guess);
                        }
                        continue;
                    }
                    retaken = false;
                    spared = false;
                    let t_event = t + theta * h;
                    // Grid points before the event still come from the
                    // smooth part of the step.
                    loop {
                        let out_t = out_i as f64 * out_step;
                        if out_t > t_event + 1e-12 || out_t > stop + 1e-12 {
                            break;
                        }
                        interpolate(((out_t - t) / h).clamp(0.0, 1.0), &mut interp, &y, &k);
                        or_stall!(self.record_row(
                            out_t,
                            &interp,
                            &mut values,
                            &mut derivatives_scratch,
                            &mut alg_guess,
                            &mut rows,
                        ));
                        last_out_t = out_t;
                        out_i += 1;
                    }
                    interpolate(theta, &mut interp, &y, &k);
                    y.copy_from_slice(&interp);
                    or_stall!(self.eval_point(
                        t_event,
                        &y,
                        &mut values,
                        &mut derivatives_scratch,
                        &mut alg_guess,
                    ));
                    // A step that ends on a scheduled threshold may also
                    // hold a crossing, and this branch goes round the
                    // loop without reaching the one below: the relation
                    // on `time` is turned here, or the next step would
                    // start past its threshold and turn it a step late.
                    state.turn_clocks(t_event, &self.clocks, &mut values);
                    let outcome = or_stall!(self.handle_event(
                        t_event,
                        &mut y,
                        &mut values,
                        &mut alg_guess,
                        &mut state,
                    ));
                    t = t_event;
                    long_end = None;
                    or_stall!(self.eval_point(t, &y, &mut values, &mut k[0], &mut alg_guess));
                    indicators_prev = self.indicator_values(t, &values);
                    handled_at = t;
                    if let Some(message) = outcome.terminated {
                        or_stall!(self.record_row(
                            t,
                            &y,
                            &mut values,
                            &mut derivatives_scratch,
                            &mut alg_guess,
                            &mut rows
                        ));
                        terminated = Some(message);
                        break;
                    }
                    // A state event that changed something is recorded at
                    // the instant it happened, so the jump is visible.
                    let mode_left = !self.mode_holds(&values, t);
                    if outcome.changed || mode_left {
                        or_stall!(self.record_row(
                            t,
                            &y,
                            &mut values,
                            &mut derivatives_scratch,
                            &mut alg_guess,
                            &mut rows
                        ));
                    }
                    // A run-time `if` equation that changed branch at
                    // this very event wants a model built for the mode
                    // now in force. The row just written is only there
                    // to carry the point over; the continuation starts
                    // at this instant and writes it again with the
                    // values of the mode that now applies.
                    if mode_left {
                        let mut outcome =
                            self.stall_at_last_row(columns, rows, SolverMethod::Dopri45, true)?;
                        if let AdaptiveOutcome::Stalled(stall) = &mut outcome {
                            stall.partial.rows.pop();
                        }
                        return Ok(outcome);
                    }
                    h = (h * theta.max(0.1)).max(1e-12);
                    continue;
                }
                indicators_prev = indicators_new;
                spared = retaken;
                retaken = false;

                // Dense output on the grid via cubic Hermite interpolation.
                loop {
                    let out_t = out_i as f64 * out_step;
                    if out_t > t + h + 1e-12 || out_t > stop + 1e-12 {
                        break;
                    }
                    interpolate(((out_t - t) / h).clamp(0.0, 1.0), &mut interp, &y, &k);
                    or_stall!(self.record_row(
                        out_t,
                        &interp,
                        &mut values,
                        &mut derivatives_scratch,
                        &mut alg_guess,
                        &mut rows,
                    ));
                    last_out_t = out_t;
                    out_i += 1;
                }
                t += h;
                y.copy_from_slice(&y5);
                k.swap(0, 6);
                if long_end
                    .as_ref()
                    .is_some_and(|(t_end, _)| *t_end <= t + 1e-12)
                {
                    long_end = None;
                }

                // The pivot that chose the states would choose otherwise
                // here: hand the run back for a re-selection while the
                // algebra is still sound.
                if self.reselectable && !self.selection_sound(&values, t) {
                    return self.stall_at_last_row(columns, rows, SolverMethod::Dopri45, false);
                }
                // A run-time `if` equation that changed branch wants a
                // model built for the mode now in force: this one was
                // matched and torn for the other.
                if !self.mode_holds(&values, t) {
                    return self.stall_at_last_row(columns, rows, SolverMethod::Dopri45, true);
                }

                // The step ended on a scheduled instant: raise the flags
                // of the `sample(...)` sources due here and let the
                // `when` clauses read them.
                if state.next_time_event().is_some_and(|next| next <= t + 1e-9) {
                    or_stall!(self.eval_point(
                        t,
                        &y,
                        &mut values,
                        &mut derivatives_scratch,
                        &mut alg_guess
                    ));
                    let turned = self.raise_time_events(t, &mut state, &mut values);
                    let outcome = or_stall!(self.handle_event(
                        t,
                        &mut y,
                        &mut values,
                        &mut alg_guess,
                        &mut state
                    ));
                    or_stall!(self.eval_point(t, &y, &mut values, &mut k[0], &mut alg_guess));
                    indicators_prev = self.indicator_values(t, &values);
                    handled_at = t;
                    // A relation on `time` that turned here is a jump
                    // like any other, and may have left the mode this
                    // model was built for.
                    let mode_left = turned && !self.mode_holds(&values, t);
                    if outcome.changed || turned {
                        // The discrete values jumped here, so the point
                        // is recorded twice: before and after the event.
                        self.record_row(
                            t,
                            &y,
                            &mut values,
                            &mut derivatives_scratch,
                            &mut alg_guess,
                            &mut rows,
                        )?;
                    }
                    if let Some(message) = outcome.terminated {
                        terminated = Some(message);
                        break;
                    }
                    if mode_left {
                        let mut outcome =
                            self.stall_at_last_row(columns, rows, SolverMethod::Dopri45, true)?;
                        if let AdaptiveOutcome::Stalled(stall) = &mut outcome {
                            stall.partial.rows.pop();
                        }
                        return Ok(outcome);
                    }
                }
            }

            if !accepted && !reject_keeps_guess() {
                alg_guess.copy_from_slice(&footing_guess);
            }
            let factor = if !err_norm.is_finite() {
                0.2
            } else if err_norm == 0.0 {
                5.0
            } else {
                (0.9 * err_norm.powf(-0.2)).clamp(0.2, 5.0)
            };
            h *= factor;
            // On a model whose states were chosen by a pivot, a step
            // collapse is the cue to choose again - well before the
            // hard underflow that would end the run.
            if self.reselectable && h < stop * 5e-14 {
                // The snapshot comes from the last accepted point; the
                // evaluation there succeeded when it was accepted.
                return self.stall_at_last_row(columns, rows, SolverMethod::Dopri45, false);
            }
            if h < stop * 1e-14 || h < 1e-300 {
                return err(format!(
                    "step size underflow at t = {t:.6}: probable singularity"
                ));
            }
        }
        self.finish_segment(
            Segment {
                values,
                columns,
                rows,
                y,
                alg_guess,
                scratch: derivatives_scratch,
                state,
                indicators_prev,
                out_i,
                last_out_t,
                terminated,
            },
            SolverMethod::Dopri45,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::hidden_crossing;

    fn stages(readings: &[(f64, f64)]) -> Vec<(f64, Vec<f64>)> {
        readings.iter().map(|&(c, v)| (c, vec![v])).collect()
    }

    #[test]
    fn a_stage_on_the_other_side_ends_the_step_there() {
        let inside = stages(&[(0.2, 0.5), (0.3, -0.1), (0.8, -0.4), (8.0 / 9.0, 0.2)]);
        assert_eq!(hidden_crossing(&[1.0], &inside, &[1.0], 0.0), Some(0.3));
    }

    #[test]
    fn a_dip_between_two_stages_is_found_by_its_parabola() {
        // (x - 0.55)^2 - 0.01: below zero on 0.45..0.65, no stage there.
        let f = |x: f64| (x - 0.55f64).powi(2) - 0.01;
        let inside = stages(&[
            (0.2, f(0.2)),
            (0.3, f(0.3)),
            (0.8, f(0.8)),
            (8.0 / 9.0, f(8.0 / 9.0)),
        ]);
        let theta = hidden_crossing(&[f(0.0)], &inside, &[f(1.0)], 0.0).unwrap();
        assert!((theta - 0.55).abs() < 1e-9, "{theta}");
    }

    #[test]
    fn a_curve_that_only_comes_near_says_nothing() {
        let f = |x: f64| (x - 0.55f64).powi(2) + 0.01;
        let inside = stages(&[
            (0.2, f(0.2)),
            (0.3, f(0.3)),
            (0.8, f(0.8)),
            (8.0 / 9.0, f(8.0 / 9.0)),
        ]);
        assert_eq!(hidden_crossing(&[f(0.0)], &inside, &[f(1.0)], 0.0), None);
        // Ends that differ are the end search's to find.
        assert_eq!(hidden_crossing(&[1.0], &inside, &[-1.0], 0.0), None);
    }

    #[test]
    fn a_sign_turned_by_rounding_alone_is_not_a_crossing() {
        // The velocity of a stuck friction element, at rest on zero.
        let inside = stages(&[
            (0.2, -3.6e-18),
            (0.3, -7.9e-18),
            (0.8, 3.0e-16),
            (8.0 / 9.0, 9.1e-16),
        ]);
        assert_eq!(
            hidden_crossing(&[-3.6e-18], &inside, &[-3.6e-17], 1e-9),
            None
        );
        assert_eq!(
            hidden_crossing(&[-3.6e-18], &inside, &[-3.6e-17], 0.0),
            Some(0.8)
        );
    }
}
