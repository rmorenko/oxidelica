//! Adaptive Dormand-Prince 5(4) with dense output.

use crate::*;

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

        let mut values = self.values_template.clone();
        let mut columns = vec!["time".to_string()];
        columns.extend(self.states.iter().cloned());
        columns.extend(self.output_algebraics.iter().map(|(name, _)| name.clone()));
        columns.extend(self.discretes.iter().cloned());
        let mut rows: Vec<Vec<f64>> = Vec::new();
        let mut derivatives_scratch = Vec::new();

        let mut record = |t: f64,
                          y: &[f64],
                          values: &mut [f64],
                          k: &mut Vec<f64>,
                          alg_guess: &mut [f64]|
         -> Result<(), SimError> {
            self.eval_point(t, y, values, k, alg_guess)?;
            // What is written down is also what the delays remember:
            // in order, and as close together as the model asked its
            // output to be.
            self.remember_delays(t, values);
            self.check_asserts(t, values)?;
            let mut row = Vec::with_capacity(1 + n + self.algebraics.len() + self.discretes.len());
            row.push(t);
            row.extend_from_slice(y);
            for &(_, slot) in &self.output_algebraics {
                row.push(values[slot]);
            }
            for &slot in &self.discrete_slots {
                row.push(values[slot]);
            }
            rows.push(row);
            Ok(())
        };

        let mut y = self.initial.clone();
        let mut alg_guess = self.algebraic_start.clone();
        // A segment remembers its own past, from its own beginning.
        self.history.borrow_mut().iter_mut().for_each(Vec::clear);
        let t0 = self.start_time;
        let mut last_out_t = t0;
        let mut terminated: Option<String> = None;
        let mut state = self.event_state();
        if self.resume {
            // Mid-run already: no initial event, and the `when`
            // conditions resume from what is true at this instant.
            self.eval_point(
                t0,
                &y,
                &mut values,
                &mut derivatives_scratch,
                &mut alg_guess,
            )?;
            state.when_prev = self.when_conditions(t0, &values);
        } else {
            values[self.initial_slot] = 1.0;
            // The initial event comes before the first output point: a
            // `when initial()` or a `sample(0, …)` has already fired by then.
            state.raise_samples(t0, &self.samples, &self.sample_slots, &mut values);
            let start_event =
                self.handle_event(t0, &mut y, &mut values, &mut alg_guess, &mut state)?;
            if let Some(message) = start_event.terminated {
                record(
                    t0,
                    &y,
                    &mut values,
                    &mut derivatives_scratch,
                    &mut alg_guess,
                )?;
                return Ok(AdaptiveOutcome::Finished(SimResult {
                    columns,
                    rows,
                    parameters: self.parameters.clone(),
                    terminated: Some(message),
                    method: SolverMethod::Dopri45,
                    reselections: 0,
                }));
            }
        }
        record(
            t0,
            &y,
            &mut values,
            &mut derivatives_scratch,
            &mut alg_guess,
        )?;
        self.remember_delays(t0, &values);
        let mut indicators_prev = self.indicator_values(t0, &values);
        // Pure-algebraic models: no ODE to integrate, only the grid.
        if n == 0 {
            // A continuation picks the grid up where it left off.
            let mut out_i = (t0 / out_step + 1e-9).floor() as usize + 1;
            loop {
                // Walk to whichever comes first: the next output point
                // or the next scheduled time event.
                let grid = out_i as f64 * out_step;
                let t = match state.next_time_event() {
                    Some(next) if next < grid - 1e-12 => next,
                    _ => grid,
                };
                if t > stop + 1e-12 {
                    break;
                }
                record(t, &y, &mut values, &mut derivatives_scratch, &mut alg_guess)?;
                // Nothing is integrated here, so a mode change is
                // noticed at the first grid point that sees it. The row
                // just written used the model on hand; the
                // continuation writes this instant again with the one
                // that now applies.
                if !self.mode_holds(&values, t) {
                    let mut outcome =
                        self.stall_at_last_row(columns, rows, SolverMethod::Dopri45, true)?;
                    if let AdaptiveOutcome::Stalled(stall) = &mut outcome {
                        stall.partial.rows.pop();
                    }
                    return Ok(outcome);
                }
                if (t - grid).abs() < 1e-12 {
                    last_out_t = t;
                    out_i += 1;
                }
                state.raise_samples(t, &self.samples, &self.sample_slots, &mut values);
                let outcome =
                    self.handle_event(t, &mut y, &mut values, &mut alg_guess, &mut state)?;
                if outcome.changed {
                    record(t, &y, &mut values, &mut derivatives_scratch, &mut alg_guess)?;
                }
                terminated = outcome.terminated;
                if terminated.is_some() {
                    break;
                }
            }
            if terminated.is_none() && last_out_t < stop - 1e-12 {
                record(
                    stop,
                    &y,
                    &mut values,
                    &mut derivatives_scratch,
                    &mut alg_guess,
                )?;
            }
            return Ok(AdaptiveOutcome::Finished(SimResult {
                columns,
                rows,
                parameters: self.parameters.clone(),
                terminated,
                method: SolverMethod::Dopri45,
                reselections: 0,
            }));
        }

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
        // The next output-grid index after where this segment starts.
        let mut out_i = (t0 / out_step + 1e-9).floor() as usize + 1;
        let mut evals: u64 = 0;

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
            let mut stage_failed = false;
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
                    Ok(()) => {}
                    // A dying algebraic loop on a model whose states were
                    // *chosen* is likely the choice failing, not the
                    // model: treat the step as rejected and let the step
                    // size fall toward the stall check below.
                    Err(_) if self.reselectable => {
                        stage_failed = true;
                        break;
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
            if accepted {
                // FSAL: the derivative at t+h.
                match self.eval_point(t + h, &y5, &mut values, &mut k[6], &mut alg_guess) {
                    Ok(()) => {}
                    // The point passed the error estimate but the
                    // algebraic layer will not hold there: on a chosen
                    // selection, reject the step and shrink instead.
                    Err(_) if self.reselectable => {
                        h *= 0.2;
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
                // An indicator sitting on zero (we just handled an event
                // there) is re-baselined instead of firing again.
                let indicators_new = self.indicator_values(t + h, &values);
                let mut event_theta: Option<f64> = None;
                for (index, (&before, &after)) in
                    indicators_prev.iter().zip(&indicators_new).enumerate()
                {
                    if before.abs() <= 1e-12 || before * after >= 0.0 {
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

                if let Some(theta) = event_theta {
                    let t_event = t + theta * h;
                    // Grid points before the event still come from the
                    // smooth part of the step.
                    loop {
                        let out_t = out_i as f64 * out_step;
                        if out_t > t_event + 1e-12 || out_t > stop + 1e-12 {
                            break;
                        }
                        interpolate(((out_t - t) / h).clamp(0.0, 1.0), &mut interp, &y, &k);
                        or_stall!(record(
                            out_t,
                            &interp,
                            &mut values,
                            &mut derivatives_scratch,
                            &mut alg_guess,
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
                    let outcome = or_stall!(self.handle_event(
                        t_event,
                        &mut y,
                        &mut values,
                        &mut alg_guess,
                        &mut state,
                    ));
                    t = t_event;
                    or_stall!(self.eval_point(t, &y, &mut values, &mut k[0], &mut alg_guess));
                    indicators_prev = self.indicator_values(t, &values);
                    if let Some(message) = outcome.terminated {
                        or_stall!(record(
                            t,
                            &y,
                            &mut values,
                            &mut derivatives_scratch,
                            &mut alg_guess
                        ));
                        terminated = Some(message);
                        break;
                    }
                    // A state event that changed something is recorded at
                    // the instant it happened, so the jump is visible.
                    let mode_left = !self.mode_holds(&values, t);
                    if outcome.changed || mode_left {
                        or_stall!(record(
                            t,
                            &y,
                            &mut values,
                            &mut derivatives_scratch,
                            &mut alg_guess
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

                // Dense output on the grid via cubic Hermite interpolation.
                loop {
                    let out_t = out_i as f64 * out_step;
                    if out_t > t + h + 1e-12 || out_t > stop + 1e-12 {
                        break;
                    }
                    interpolate(((out_t - t) / h).clamp(0.0, 1.0), &mut interp, &y, &k);
                    or_stall!(record(
                        out_t,
                        &interp,
                        &mut values,
                        &mut derivatives_scratch,
                        &mut alg_guess,
                    ));
                    last_out_t = out_t;
                    out_i += 1;
                }
                t += h;
                y.copy_from_slice(&y5);
                k.swap(0, 6);

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
                    state.raise_samples(t, &self.samples, &self.sample_slots, &mut values);
                    let outcome = or_stall!(self.handle_event(
                        t,
                        &mut y,
                        &mut values,
                        &mut alg_guess,
                        &mut state
                    ));
                    or_stall!(self.eval_point(t, &y, &mut values, &mut k[0], &mut alg_guess));
                    indicators_prev = self.indicator_values(t, &values);
                    if outcome.changed {
                        // The discrete values jumped here, so the point
                        // is recorded twice: before and after the event.
                        record(t, &y, &mut values, &mut derivatives_scratch, &mut alg_guess)?;
                    }
                    if let Some(message) = outcome.terminated {
                        terminated = Some(message);
                        break;
                    }
                }
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
        if terminated.is_none() && last_out_t < stop - 1e-12 {
            record(
                stop,
                &y,
                &mut values,
                &mut derivatives_scratch,
                &mut alg_guess,
            )?;
            terminated = self
                .handle_event(stop, &mut y, &mut values, &mut alg_guess, &mut state)?
                .terminated;
        }
        Ok(AdaptiveOutcome::Finished(SimResult {
            columns,
            rows,
            parameters: self.parameters.clone(),
            terminated,
            method: SolverMethod::Dopri45,
            reselections: 0,
        }))
    }
}
