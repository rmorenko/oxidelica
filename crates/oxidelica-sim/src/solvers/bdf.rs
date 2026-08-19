//! Variable-order BDF for stiff models.

use crate::*;

use super::{turned, Segment, SegmentStart};

impl CompiledModel {
    /// Variable-order (1..5), variable-step BDF with Newton iteration
    /// and a reused finite-difference Jacobian.
    ///
    /// Implicit and stable for stiff systems, where explicit methods
    /// are limited by stability rather than accuracy. Coefficients come
    /// from differentiating the Lagrange interpolant over the actual
    /// (non-uniform) step history, so no restart is needed after a step
    /// size change. Dense output on the `Interval` grid uses the same
    /// interpolant.
    pub fn simulate_bdf(&self) -> Result<SimResult, SimError> {
        match self.run_bdf()? {
            AdaptiveOutcome::Finished(result) => Ok(result),
            AdaptiveOutcome::Stalled(stall) => err(format!(
                "step size underflow at t = {:.6}: probable singularity",
                stall.time
            )),
            AdaptiveOutcome::Stiff => unreachable!("bdf never reports stiffness"),
        }
    }

    /// The BDF integration itself; a stall on a reselectable model
    /// comes back as an outcome instead of an error.
    pub(crate) fn run_bdf(&self) -> Result<AdaptiveOutcome, SimError> {
        const MAX_ORDER: usize = 5;
        const NEWTON_MAX: usize = 12;

        let n = self.states.len();
        let stop = self.stop_time;
        let out_step = self.step.max(1e-12);
        let rtol = self.tolerance;
        let atol = self.tolerance * 1e-3;

        let segment = match self.begin_segment(SolverMethod::Bdf)? {
            SegmentStart::Finished(result) => return Ok(AdaptiveOutcome::Finished(result)),
            SegmentStart::Running(segment) => *segment,
        };
        if n == 0 {
            return self.walk_without_states(segment, SolverMethod::Bdf);
        }
        let Segment {
            mut values,
            columns,
            mut rows,
            mut y,
            mut alg_guess,
            scratch: mut f_scratch,
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

        // History, newest first.
        let mut t_hist: Vec<f64> = vec![t0];
        let mut y_hist: Vec<Vec<f64>> = vec![y.clone()];
        let mut order = 1usize;
        let mut t = t0;
        let mut h = out_step.min(stop - t0).max(1e-12);
        let mut jac: Option<Vec<Vec<f64>>> = None;
        let mut consecutive_ok = 0usize;
        let mut steps: u64 = 0;

        let mut f_new = vec![0.0; n];
        let mut y_new = vec![0.0; n];
        let mut y_pred = vec![0.0; n];
        let mut f_last = Vec::new();
        self.eval_point(t0, &y, &mut values, &mut f_last, &mut alg_guess)?;

        /// See the macro of the adaptive solver: a failing evaluation on
        /// a chosen selection stalls the segment instead of ending the run.
        macro_rules! or_stall {
            ($attempt:expr) => {
                match $attempt {
                    Ok(value) => value,
                    Err(_) if self.reselectable => {
                        return self.stall_at_last_row(columns, rows, SolverMethod::Bdf, false);
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
            let k = order.min(t_hist.len());
            let t_new = t + h;

            // Lagrange nodes: the new point first, then the history.
            let mut nodes = Vec::with_capacity(k + 1);
            nodes.push(t_new);
            nodes.extend(t_hist.iter().take(k).copied());
            let coeffs = lagrange_derivative_coefficients(&nodes);

            // Predictor of the same order as the corrector: the
            // polynomial through the last k+1 points. Early on there is
            // no such history, so a derivative-based linear predictor
            // (y_n + h*f_n) stands in — otherwise the predictor-corrector
            // difference would measure the solution change, not the error.
            let predictor_points = (k + 1).min(t_hist.len());
            if predictor_points >= 2 {
                for (i, slot) in y_pred.iter_mut().enumerate() {
                    *slot = lagrange_extrapolate(
                        &t_hist[..predictor_points],
                        &y_hist[..predictor_points],
                        i,
                        t_new,
                    );
                }
            } else {
                for (i, slot) in y_pred.iter_mut().enumerate() {
                    *slot = y_hist[0][i] + h * f_last[i];
                }
            }
            y_new.copy_from_slice(&y_pred);

            // Constant part of the BDF residual.
            let mut hist_sum = vec![0.0; n];
            for (j, coeff) in coeffs.iter().enumerate().skip(1) {
                for (i, slot) in hist_sum.iter_mut().enumerate() {
                    *slot += coeff * y_hist[j - 1][i];
                }
            }
            let c0 = coeffs[0];

            // Newton iteration on c0*y + hist - f(t_new, y) = 0.
            let mut converged = false;
            let mut newton_failed = false;
            for iteration in 0..NEWTON_MAX {
                match self.eval_point(t_new, &y_new, &mut values, &mut f_new, &mut alg_guess) {
                    Ok(()) => {}
                    // The algebraic layer dying on a chosen selection is
                    // the selection failing: reject the step and let the
                    // step size fall toward the stall check.
                    Err(_) if self.reselectable => {
                        newton_failed = true;
                        break;
                    }
                    Err(error) => return Err(error),
                }
                steps += 1;
                if steps > 20_000_000 {
                    return err(format!(
                        "solver exceeded the evaluation budget at t = {t:.6}"
                    ));
                }
                let residual: Vec<f64> = (0..n)
                    .map(|i| c0 * y_new[i] + hist_sum[i] - f_new[i])
                    .collect();
                if residual.iter().any(|r| !r.is_finite()) {
                    newton_failed = true;
                    break;
                }

                if jac.is_none() || iteration == 4 {
                    jac = Some(self.jacobian(t_new, &y_new, &f_new, &mut values, &alg_guess)?);
                }
                // The iteration matrix only moves when the Jacobian is
                // refreshed or the step coefficient changes, so it is
                // eliminated once and its factors reused - both between
                // the iterations of a step and between steps.
                let jacobian = jac.as_ref().expect("jacobian present");
                // A banded structure is eliminated as a band; anything
                // else, and anything the band cannot pivot through,
                // goes through the dense path.
                let banded = self.jacobian_band.and_then(|band| {
                    let mut matrix: Vec<Vec<f64>> = (0..n)
                        .map(|i| {
                            (0..2 * band + 1)
                                .map(|offset| {
                                    let column = (i + offset).checked_sub(band)?;
                                    if column >= n {
                                        return None;
                                    }
                                    Some(if i == column {
                                        c0 - jacobian[i][column]
                                    } else {
                                        -jacobian[i][column]
                                    })
                                })
                                .map(|value| value.unwrap_or(0.0))
                                .collect()
                        })
                        .collect();
                    solve_banded(&mut matrix, band, &residual)
                });
                let delta = match banded {
                    Some(delta) => delta,
                    None => {
                        let mut matrix: Vec<Vec<f64>> = jacobian
                            .iter()
                            .enumerate()
                            .map(|(i, row)| {
                                row.iter()
                                    .enumerate()
                                    .map(|(j, value)| if i == j { c0 - value } else { -value })
                                    .collect()
                            })
                            .collect();
                        let Some(delta) = solve_linear(&mut matrix, &residual) else {
                            newton_failed = true;
                            break;
                        };
                        delta
                    }
                };
                let mut scaled = 0.0f64;
                for i in 0..n {
                    y_new[i] -= delta[i];
                    let scale = atol + rtol * y_new[i].abs().max(1.0);
                    scaled = scaled.max((delta[i] / scale).abs());
                }
                if !y_new.iter().all(|v| v.is_finite()) {
                    newton_failed = true;
                    break;
                }
                if scaled < 0.1 {
                    converged = true;
                    break;
                }
            }

            if newton_failed || !converged {
                // Shrink the step, drop to first order and refresh the
                // Jacobian: the linear model is clearly stale.
                h *= 0.25;
                order = 1;
                jac = None;
                if h < stop * 1e-14 || h < 1e-300 {
                    return err(format!(
                        "step size underflow at t = {t:.6}: Newton iteration does not converge"
                    ));
                }
                continue;
            }

            // Predictor-corrector difference estimates the local error.
            let mut err_norm = 0.0f64;
            for i in 0..n {
                let scale = atol + rtol * y_new[i].abs().max(y[i].abs());
                let e = (y_new[i] - y_pred[i]) / scale;
                err_norm += e * e;
            }
            err_norm = (err_norm / n as f64).sqrt();

            if err_norm.is_finite() && err_norm <= 1.0 {
                // Dense output on the grid via the step interpolant.
                let mut interp_nodes = Vec::with_capacity(k + 1);
                interp_nodes.push(t_new);
                interp_nodes.extend(t_hist.iter().take(k).copied());
                let mut interp_values: Vec<&[f64]> = Vec::with_capacity(k + 1);
                interp_values.push(&y_new);
                for slot in y_hist.iter().take(k) {
                    interp_values.push(slot);
                }
                let mut interp = vec![0.0; n];
                let sample = |at: f64, interp: &mut Vec<f64>| {
                    for (i, slot) in interp.iter_mut().enumerate() {
                        *slot = lagrange_value(&interp_nodes, &interp_values, i, at);
                    }
                };

                // Locate the earliest event indicator crossing, if any.
                self.eval_point(t_new, &y_new, &mut values, &mut f_scratch, &mut alg_guess)?;
                let indicators_new = self.indicator_values(t_new, &values);
                // A reading of exactly zero where the step begins is a
                // relation standing on its threshold: it turns as the
                // step leaves, and there is nothing to search for.
                let hair = t + 1e-9 * (t_new - t);
                let mut event_t: Option<f64> = None;
                for (index, (&before, &after)) in
                    indicators_prev.iter().zip(&indicators_new).enumerate()
                {
                    if !turned(before, after) {
                        continue;
                    }
                    if before == 0.0 {
                        // Unless this is the instant an event was just
                        // handled at: the reading there is zero because
                        // the crossing is behind, not ahead.
                        if t != handled_at {
                            event_t = Some(event_t.map_or(hair, |c: f64| c.min(hair)));
                        }
                        continue;
                    }
                    let (mut lo, mut hi) = (t, t_new);
                    for _ in 0..40 {
                        let mid = 0.5 * (lo + hi);
                        sample(mid, &mut interp);
                        self.eval_point(mid, &interp, &mut values, &mut f_scratch, &mut alg_guess)?;
                        if before * self.indicator_values(mid, &values)[index] <= 0.0 {
                            hi = mid;
                        } else {
                            lo = mid;
                        }
                    }
                    let crossed = (hi + 1e-9 * (t_new - t)).min(t_new);
                    event_t = Some(event_t.map_or(crossed, |c: f64| c.min(crossed)));
                }

                let horizon = event_t.unwrap_or(t_new);
                loop {
                    let out_t = out_i as f64 * out_step;
                    if out_t > horizon + 1e-12 || out_t > stop + 1e-12 {
                        break;
                    }
                    sample(out_t, &mut interp);
                    self.record_row(
                        out_t,
                        &interp,
                        &mut values,
                        &mut f_scratch,
                        &mut alg_guess,
                        &mut rows,
                    )?;
                    last_out_t = out_t;
                    out_i += 1;
                }

                if let Some(t_event) = event_t {
                    sample(t_event, &mut interp);
                    y.copy_from_slice(&interp);
                    self.eval_point(t_event, &y, &mut values, &mut f_scratch, &mut alg_guess)?;
                    let outcome = or_stall!(self.handle_event(
                        t_event,
                        &mut y,
                        &mut values,
                        &mut alg_guess,
                        &mut state,
                    ));
                    t = t_event;
                    // The history is meaningless across a discontinuity.
                    t_hist.clear();
                    y_hist.clear();
                    t_hist.push(t);
                    y_hist.push(y.clone());
                    order = 1;
                    consecutive_ok = 0;
                    jac = None;
                    self.eval_point(t, &y, &mut values, &mut f_last, &mut alg_guess)?;
                    indicators_prev = self.indicator_values(t, &values);
                    handled_at = t;
                    if let Some(message) = outcome.terminated {
                        self.record_row(
                            t,
                            &y,
                            &mut values,
                            &mut f_scratch,
                            &mut alg_guess,
                            &mut rows,
                        )?;
                        terminated = Some(message);
                        break;
                    }
                    // A state event that changed something is recorded at
                    // the instant it happened, so the jump is visible.
                    let mode_left = !self.mode_holds(&values, t);
                    if outcome.changed || mode_left {
                        self.record_row(
                            t,
                            &y,
                            &mut values,
                            &mut f_scratch,
                            &mut alg_guess,
                            &mut rows,
                        )?;
                    }
                    // A run-time `if` equation that changed branch at
                    // this very event wants a model built for the mode
                    // now in force, and it has to be rebuilt *here*
                    // rather than after the next step: the branch turns
                    // over exactly at the crossing, so a step taken
                    // first would find the mode gone and stall back at
                    // the instant this segment began, over and over.
                    // The row just written is only there to carry the
                    // point over; the continuation starts at this
                    // instant and writes it again with the values of
                    // the mode that now applies.
                    if mode_left {
                        let mut outcome =
                            self.stall_at_last_row(columns, rows, SolverMethod::Bdf, true)?;
                        if let AdaptiveOutcome::Stalled(stall) = &mut outcome {
                            stall.partial.rows.pop();
                        }
                        return Ok(outcome);
                    }
                    h = (h * 0.25).max(1e-12);
                    continue;
                }
                indicators_prev = indicators_new;

                t = t_new;
                y.copy_from_slice(&y_new);
                f_last.copy_from_slice(&f_new);
                t_hist.insert(0, t_new);
                y_hist.insert(0, y_new.clone());
                if t_hist.len() > MAX_ORDER + 1 {
                    t_hist.pop();
                    y_hist.pop();
                }
                consecutive_ok += 1;
                // See the adaptive solver: a selection that stopped
                // being the right one is re-made in clean territory.
                if self.reselectable && !self.selection_sound(&values, t) {
                    return self.stall_at_last_row(columns, rows, SolverMethod::Bdf, false);
                }
                // A run-time `if` equation that changed branch wants a
                // model built for the mode now in force: this one was
                // matched and torn for the other.
                if !self.mode_holds(&values, t) {
                    return self.stall_at_last_row(columns, rows, SolverMethod::Bdf, true);
                }
                // Raise the order once the history supports it. The
                // step controller keeps the error near its target, so
                // waiting for a *small* error would pin the order at 1;
                // a premature raise simply costs one rejected step.
                if consecutive_ok > order && order < MAX_ORDER && t_hist.len() > order {
                    order += 1;
                    consecutive_ok = 0;
                }

                // The step ended on a scheduled instant: the sources due
                // here raise their flags and the `when` clauses read them.
                if state.next_time_event().is_some_and(|next| next <= t + 1e-9) {
                    self.eval_point(t, &y, &mut values, &mut f_scratch, &mut alg_guess)?;
                    state.raise_samples(t, &self.samples, &self.sample_slots, &mut values);
                    let outcome =
                        self.handle_event(t, &mut y, &mut values, &mut alg_guess, &mut state)?;
                    self.eval_point(t, &y, &mut values, &mut f_last, &mut alg_guess)?;
                    indicators_prev = self.indicator_values(t, &values);
                    handled_at = t;
                    if outcome.changed {
                        // A jump the history cannot represent: restart
                        // from order one, and record both sides of it.
                        self.record_row(
                            t,
                            &y,
                            &mut values,
                            &mut f_scratch,
                            &mut alg_guess,
                            &mut rows,
                        )?;
                        y_hist[0].copy_from_slice(&y);
                        t_hist.truncate(1);
                        y_hist.truncate(1);
                        order = 1;
                        consecutive_ok = 0;
                    }
                    if let Some(message) = outcome.terminated {
                        terminated = Some(message);
                        break;
                    }
                }
            } else {
                consecutive_ok = 0;
                if order > 1 {
                    order -= 1;
                }
            }

            let factor = if !err_norm.is_finite() || err_norm == 0.0 {
                if err_norm == 0.0 {
                    2.0
                } else {
                    0.25
                }
            } else {
                (0.9 * err_norm.powf(-1.0 / (k as f64 + 1.0))).clamp(0.2, 4.0)
            };
            h *= factor;
            // A collapsing step on a model whose states were chosen by
            // a pivot: stall and let the caller choose again.
            if self.reselectable && h < stop * 5e-14 {
                return self.stall_at_last_row(columns, rows, SolverMethod::Bdf, false);
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
                scratch: f_scratch,
                state,
                indicators_prev,
                out_i,
                last_out_t,
                terminated,
            },
            SolverMethod::Bdf,
        )
    }
}
