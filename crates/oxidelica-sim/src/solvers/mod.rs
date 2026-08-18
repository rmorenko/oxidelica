//! Integration: the point evaluation every method shares, the
//! implicit blocks, and the choice of which method runs.

use crate::*;

mod bdf;
mod dopri;
mod rk4;

/// Everything a segment carries that has nothing to do with how the
/// stepping is done: the run's own bookkeeping.
///
/// Both solvers set this up the same way, walk the same grid when
/// there is nothing to integrate, and close it out the same way. Only
/// what happens between the two - explicit stages against a Newton
/// iteration - is theirs alone. Keeping the rest here is not tidiness:
/// the two copies it replaces had drifted apart twice, and each time
/// the implicit solver was the one missing a step.
pub(crate) struct Segment {
    pub(crate) values: Vec<f64>,
    pub(crate) columns: Vec<String>,
    pub(crate) rows: Vec<Vec<f64>>,
    pub(crate) y: Vec<f64>,
    pub(crate) alg_guess: Vec<f64>,
    pub(crate) scratch: Vec<f64>,
    pub(crate) state: EventState,
    pub(crate) indicators_prev: Vec<f64>,
    /// Index of the next output point on the `Interval` grid.
    pub(crate) out_i: usize,
    pub(crate) last_out_t: f64,
    pub(crate) terminated: Option<String>,
}

/// What starting a segment produced: something to integrate, or a run
/// that was over before its first step.
pub(crate) enum SegmentStart {
    Running(Box<Segment>),
    Finished(SimResult),
}

impl CompiledModel {
    /// Write one output row, and let the delays and the asserts see the
    /// point while it is evaluated.
    pub(crate) fn record_row(
        &self,
        t: f64,
        y: &[f64],
        values: &mut [f64],
        scratch: &mut Vec<f64>,
        alg_guess: &mut [f64],
        rows: &mut Vec<Vec<f64>>,
    ) -> Result<(), SimError> {
        self.eval_point(t, y, values, scratch, alg_guess)?;
        // What is written down is also what the delays remember: in
        // order, and as close together as the model asked its output to
        // be.
        self.remember_delays(t, values);
        self.check_asserts(t, values)?;
        let mut row = Vec::with_capacity(
            1 + self.states.len() + self.algebraics.len() + self.discretes.len(),
        );
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
    }

    /// Set a segment up to its first output point: the initial event if
    /// this is the beginning, the resumed truth if it is not.
    pub(crate) fn begin_segment(&self, method: SolverMethod) -> Result<SegmentStart, SimError> {
        let mut values = self.values_template.clone();
        let mut columns = vec!["time".to_string()];
        columns.extend(self.states.iter().cloned());
        columns.extend(self.output_algebraics.iter().map(|(name, _)| name.clone()));
        columns.extend(self.discretes.iter().cloned());
        let mut rows: Vec<Vec<f64>> = Vec::new();
        let mut y = self.initial.clone();
        let mut alg_guess = self.algebraic_start.clone();
        let mut scratch = Vec::new();

        // A segment remembers its own past, from its own beginning.
        self.history.borrow_mut().iter_mut().for_each(Vec::clear);
        let t0 = self.start_time;
        let mut state = self.event_state();
        if self.resume {
            // Mid-run already: no initial event, and the `when`
            // conditions resume from what is true at this instant.
            self.eval_point(t0, &y, &mut values, &mut scratch, &mut alg_guess)?;
            state.when_prev = self.when_conditions(t0, &values);
        } else {
            values[self.initial_slot] = 1.0;
            // The initial event comes before the first output point: a
            // `when initial()` or a `sample(0, …)` has already fired by then.
            state.raise_samples(t0, &self.samples, &self.sample_slots, &mut values);
            let start_event =
                self.handle_event(t0, &mut y, &mut values, &mut alg_guess, &mut state)?;
            if let Some(message) = start_event.terminated {
                self.record_row(t0, &y, &mut values, &mut scratch, &mut alg_guess, &mut rows)?;
                return Ok(SegmentStart::Finished(SimResult {
                    columns,
                    rows,
                    parameters: self.parameters.clone(),
                    terminated: Some(message),
                    method,
                    reselections: 0,
                }));
            }
        }
        self.record_row(t0, &y, &mut values, &mut scratch, &mut alg_guess, &mut rows)?;
        self.remember_delays(t0, &values);
        let indicators_prev = self.indicator_values(t0, &values);
        // A continuation picks the grid up where it left off.
        let out_i = (t0 / self.step.max(1e-12) + 1e-9).floor() as usize + 1;
        Ok(SegmentStart::Running(Box::new(Segment {
            values,
            columns,
            rows,
            y,
            alg_guess,
            scratch,
            state,
            indicators_prev,
            out_i,
            last_out_t: t0,
            terminated: None,
        })))
    }

    /// A model with no state to integrate: there is no step to take, so
    /// the run walks from one scheduled instant to the next output
    /// point and lets the discrete layer do the rest.
    pub(crate) fn walk_without_states(
        &self,
        segment: Segment,
        method: SolverMethod,
    ) -> Result<AdaptiveOutcome, SimError> {
        let Segment {
            mut values,
            columns,
            mut rows,
            mut y,
            mut alg_guess,
            mut scratch,
            mut state,
            mut out_i,
            mut last_out_t,
            mut terminated,
            ..
        } = segment;
        let (stop, out_step) = (self.stop_time, self.step.max(1e-12));
        loop {
            // Walk to whichever comes first: the next output point or
            // the next scheduled time event.
            let grid = out_i as f64 * out_step;
            let t = match state.next_time_event() {
                Some(next) if next < grid - 1e-12 => next,
                _ => grid,
            };
            if t > stop + 1e-12 {
                break;
            }
            self.record_row(t, &y, &mut values, &mut scratch, &mut alg_guess, &mut rows)?;
            // Nothing is integrated here, so a mode change is noticed at
            // the first grid point that sees it. The row just written
            // used the model on hand; the continuation writes this
            // instant again with the one that now applies.
            if !self.mode_holds(&values, t) {
                let mut outcome = self.stall_at_last_row(columns, rows, method, true)?;
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
            let outcome = self.handle_event(t, &mut y, &mut values, &mut alg_guess, &mut state)?;
            if outcome.changed {
                self.record_row(t, &y, &mut values, &mut scratch, &mut alg_guess, &mut rows)?;
            }
            terminated = outcome.terminated;
            if terminated.is_some() {
                break;
            }
        }
        if terminated.is_none() && last_out_t < stop - 1e-12 {
            self.record_row(
                stop,
                &y,
                &mut values,
                &mut scratch,
                &mut alg_guess,
                &mut rows,
            )?;
        }
        Ok(AdaptiveOutcome::Finished(SimResult {
            columns,
            rows,
            parameters: self.parameters.clone(),
            terminated,
            method,
            reselections: 0,
        }))
    }

    /// Close a segment out: if the stepping stopped short of the stop
    /// time, that instant is still an output point and still an event.
    pub(crate) fn finish_segment(
        &self,
        segment: Segment,
        method: SolverMethod,
    ) -> Result<AdaptiveOutcome, SimError> {
        let Segment {
            mut values,
            columns,
            mut rows,
            mut y,
            mut alg_guess,
            mut scratch,
            mut state,
            last_out_t,
            mut terminated,
            ..
        } = segment;
        let stop = self.stop_time;
        if terminated.is_none() && last_out_t < stop - 1e-12 {
            self.record_row(
                stop,
                &y,
                &mut values,
                &mut scratch,
                &mut alg_guess,
                &mut rows,
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
            method,
            reselections: 0,
        }))
    }
    /// Evaluate algebraic variables and derivatives at point (t, y).
    /// `env` is reused between calls to avoid per-step allocation.
    /// Check every `assert` at an evaluated point; a violated one stops
    /// the run with its own message and the time.
    pub(crate) fn check_asserts(&self, t: f64, values: &[f64]) -> Result<(), SimError> {
        for (condition, message) in &self.asserts {
            if condition.run(values, t) == 0.0 {
                return err(format!("assertion failed at t = {t:.6}: {message}"));
            }
        }
        Ok(())
    }

    pub(crate) fn eval_point(
        &self,
        t: f64,
        y: &[f64],
        values: &mut [f64],
        derivatives_out: &mut Vec<f64>,
        alg_guess: &mut [f64],
    ) -> Result<(), SimError> {
        // Parameters sit in the array from the start and discrete values
        // are written there by the event machinery, so a point only has
        // to place the states, look up what was delayed, and run the
        // plan.
        self.fill_delays(t, values);
        for (&slot, value) in self.state_slots.iter().zip(y) {
            values[slot] = *value;
        }
        for stage in &self.stages {
            match stage {
                AlgStage::Explicit { var, code } => {
                    values[self.algebraic_slots[*var]] = code.run(values, t);
                }
                stage @ AlgStage::Implicit { .. } => {
                    self.solve_implicit_block(t, values, stage, alg_guess, false)?;
                }
            }
        }
        derivatives_out.clear();
        for code in &self.derivatives {
            derivatives_out.push(code.run(values, t));
        }
        Ok(())
    }

    /// Solve one implicit algebraic block by damped-free Newton
    /// iteration with a finite-difference Jacobian. `alg_guess` supplies
    /// warm starts (the previous evaluation point) and receives the
    /// solution.
    pub(crate) fn solve_implicit_block(
        &self,
        t: f64,
        values: &mut [f64],
        stage: &AlgStage,
        alg_guess: &mut [f64],
        validate: bool,
    ) -> Result<(), SimError> {
        let AlgStage::Implicit {
            torn: block,
            inner,
            residuals,
            ..
        } = stage
        else {
            return Ok(());
        };
        let n = block.len();
        let mut v: Vec<f64> = block.iter().map(|&i| alg_guess[i]).collect();

        let residual = |values: &mut [f64], v: &[f64]| -> Vec<f64> {
            for (j, &index) in block.iter().enumerate() {
                values[self.algebraic_slots[index]] = v[j];
            }
            // Torn values fixed: the inner unknowns follow explicitly.
            for (var, code) in inner {
                values[self.algebraic_slots[*var]] = code.run(values, t);
            }
            residuals
                .iter()
                .map(|(lhs, rhs)| lhs.run(values, t) - rhs.run(values, t))
                .collect()
        };
        let block_names =
            || -> Vec<&str> { block.iter().map(|&i| self.algebraics[i].as_str()).collect() };

        for _ in 0..50 {
            let f = residual(values, &v);
            let converged = f
                .iter()
                .zip(&v)
                .all(|(fi, vi)| fi.abs() <= 1e-10 * (1.0 + vi.abs()));
            if converged {
                for (j, &index) in block.iter().enumerate() {
                    alg_guess[index] = v[j];
                }
                if validate {
                    // A converged block still has to be *determined*:
                    // a singular Jacobian means the loop admits a whole
                    // family of solutions, and the one we landed on is
                    // an artifact of the initial guess.
                    let mut jac = vec![vec![0.0f64; n]; n];
                    for j in 0..n {
                        let h = 1e-8 * (1.0 + v[j].abs());
                        let mut perturbed = v.clone();
                        perturbed[j] += h;
                        let fp = residual(values, &perturbed);
                        for (i, row) in jac.iter_mut().enumerate() {
                            row[j] = (fp[i] - f[i]) / h;
                        }
                    }
                    let probe = vec![1.0; n];
                    if solve_linear(&mut jac, &probe).is_none() {
                        return err(format!(
                            "underdetermined algebraic loop {:?}: the equations do not determine a unique solution",
                            block_names()
                        ));
                    }
                }
                return Ok(());
            }
            // Finite-difference Jacobian of the residual.
            let mut jac = vec![vec![0.0f64; n]; n];
            for j in 0..n {
                let h = 1e-8 * (1.0 + v[j].abs());
                let mut perturbed = v.clone();
                perturbed[j] += h;
                let fp = residual(values, &perturbed);
                for (i, row) in jac.iter_mut().enumerate() {
                    row[j] = (fp[i] - f[i]) / h;
                }
            }
            let Some(dv) = solve_linear(&mut jac, &f) else {
                return err(format!(
                    "singular Jacobian in algebraic loop {:?}",
                    block_names()
                ));
            };
            for j in 0..n {
                v[j] -= dv[j];
            }
            if v.iter().any(|value| !value.is_finite()) {
                return err(format!("algebraic loop diverged: {:?}", block_names()));
            }
        }
        err(format!(
            "algebraic loop did not converge in 50 Newton iterations: {:?}",
            block_names()
        ))
    }

    /// Integrate over `[start_time, stop_time]` with the selected
    /// method, re-selecting the states and continuing whenever the
    /// current selection stalls the run.
    pub fn simulate(&self) -> Result<SimResult, SimError> {
        let mut outcome = self.run_segment()?;
        let mut merged: Option<SimResult> = None;
        let mut reselections = 0usize;
        let mut mode_changes = 0usize;
        let mut last_stall = f64::NEG_INFINITY;
        loop {
            match outcome {
                AdaptiveOutcome::Finished(result) => {
                    let mut result = match merged {
                        Some(merged) => append_segment(merged, result),
                        None => result,
                    };
                    result.reselections = reselections;
                    return Ok(result);
                }
                AdaptiveOutcome::Stiff => {
                    unreachable!("run_segment resolves the stiffness switch itself")
                }
                AdaptiveOutcome::Stalled(stall) => {
                    // A mode change is ordinary business - a chopper
                    // switches as often as it likes - so it is not held
                    // to the rule below. What it is held to is telling
                    // the truth: the compilation that follows must
                    // settle on a different branch, or the run would
                    // rebuild the same model at the same instant for
                    // ever.
                    if stall.mode_change {
                        if mode_changes >= 100_000 {
                            return err(format!(
                                "the model kept changing mode at t = {:.6}",
                                stall.time
                            ));
                        }
                        mode_changes += 1;
                    } else if stall.time <= last_stall + 1e-12 || reselections >= 200 {
                        // A stall that made no ground since the last one
                        // is not a wrong selection but a genuine
                        // singularity.
                        return err(format!(
                            "step size underflow at t = {:.6}: probable singularity                              (state re-selection did not help)",
                            stall.time
                        ));
                    } else {
                        last_stall = stall.time;
                        reselections += 1;
                    }
                    // Compile again at the point reached: the pivot now
                    // sees the sensitivities of this instant and picks
                    // the states that fit here.
                    let mut next = compile_at(
                        &self.flat,
                        Some(ResumePoint {
                            time: stall.time,
                            values: &stall.values,
                        }),
                    )?;
                    next.method = self.method;
                    merged = Some(match merged {
                        Some(merged) => append_segment(merged, stall.partial),
                        None => stall.partial,
                    });
                    outcome = next.run_segment()?;
                }
            }
        }
    }

    /// One integration attempt with the selected method; a stall comes
    /// back to `simulate` for a re-selection.
    ///
    /// With `Auto` the explicit solver goes first and watches the
    /// product of the step size and the dominant eigenvalue of the
    /// Jacobian, which it gets for free from two stages it has already
    /// evaluated. A step size held down by stability rather than by
    /// accuracy is what "stiff" means, and once that is clear the run
    /// starts again with the implicit solver.
    pub(crate) fn run_segment(&self) -> Result<AdaptiveOutcome, SimError> {
        match self.method {
            SolverMethod::Auto => match self.adaptive(true)? {
                AdaptiveOutcome::Stiff => self.run_bdf(),
                outcome => Ok(outcome),
            },
            SolverMethod::Dopri45 => match self.adaptive(false)? {
                AdaptiveOutcome::Stiff => err("the stiffness watch fired without being asked"),
                outcome => Ok(outcome),
            },
            SolverMethod::Rk4 => self.simulate_rk4().map(AdaptiveOutcome::Finished),
            SolverMethod::Bdf => self.run_bdf(),
        }
    }

    /// Finite-difference Jacobian `df/dy` of the state right-hand side
    /// at `(t, y)`. Algebraic warm starts are kept on a scratch copy so
    /// probing does not disturb the accepted solution.
    pub(crate) fn jacobian(
        &self,
        t: f64,
        y: &[f64],
        f0: &[f64],
        values: &mut [f64],
        alg_guess: &[f64],
    ) -> Result<Vec<Vec<f64>>, SimError> {
        let n = y.len();
        let mut jac = vec![vec![0.0; n]; n];
        let mut probe = y.to_vec();
        let mut scratch = alg_guess.to_vec();
        let mut f = Vec::with_capacity(n);
        let mut deltas = vec![0.0f64; n];
        for group in &self.jacobian_groups {
            // Every column of a group is perturbed at once: they share
            // no row, so each difference belongs to exactly one of them.
            for &j in group {
                deltas[j] = 1e-7 * (1.0 + y[j].abs());
                probe[j] = y[j] + deltas[j];
            }
            self.eval_point(t, &probe, values, &mut f, &mut scratch)?;
            for &j in group {
                probe[j] = y[j];
                for &i in &self.jacobian_rows[j] {
                    jac[i][j] = (f[i] - f0[i]) / deltas[j];
                }
            }
        }
        Ok(jac)
    }
}
