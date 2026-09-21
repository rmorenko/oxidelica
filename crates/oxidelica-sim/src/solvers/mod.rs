//! Integration: the point evaluation every method shares, the
//! implicit blocks, and the choice of which method runs.

use crate::*;

/// Whether to print the Newton iteration of every algebraic block.
///
/// What a block does before it refuses is the only thing that says
/// which refusal it is owed - a step into a domain's edge, a swing
/// between two points and a residual sitting on the floor of the
/// arithmetic all end at the same message. Read once: the answer
/// cannot change during a run, and the question is asked inside the
/// solver's innermost loop.
fn newton_trail() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("OXIDELICA_NEWTON_TRAIL").is_some())
}

/// Whether to take a Newton step whose direction the line search
/// could not make descend. Off by default; the switch exists so that
/// the two halves of a measurement come from one binary.
fn descent_guard_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var_os("OXIDELICA_NO_DESCENT_GUARD").is_some())
}

mod bdf;
mod dopri;
mod rk4;

/// Whether a relation turned between two readings of its indicator.
///
/// A reading of exactly zero is a relation standing on its threshold,
/// and it has turned if the next reading is off it. The product alone
/// would call that no crossing, which is how a condition turning
/// exactly on an output point - `time > 0.5` on a grid of halves - used
/// to be missed until the grid noticed it a whole step later.
pub(crate) fn turned(was: f64, now: f64) -> bool {
    was * now < 0.0 || (was == 0.0 && now != 0.0)
}

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
        // A run that cannot finish writes rows at the speed it can
        // evaluate, and nothing else in the run half has a size. The
        // memory of the machine is not a ceiling anybody can plan
        // against: it is reached quietly, hours in, and takes the
        // whole measurement with it.
        let most = self.max_rows;
        if rows.len() >= most {
            return crate::err(format!(
                "`{}` wrote more than {most} output rows, the last at t = {t}: \
                 the run is producing points faster than it advances in time",
                self.name
            ));
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

    /// Where the earliest relation turns between two instants, if one
    /// does.
    ///
    /// This is for the walk that has nothing to integrate. There, every
    /// value is a function of the time it is asked for, so an instant
    /// in between can simply be evaluated - where a solver would have
    /// to interpolate the step it just took. What comes back is the
    /// first instant at which the sign has changed, so the relation has
    /// definitely turned by then and the `when` watching it will see it.
    #[allow(clippy::too_many_arguments)]
    fn first_crossing(
        &self,
        from: f64,
        to: f64,
        before: &[f64],
        y: &[f64],
        values: &mut [f64],
        scratch: &mut Vec<f64>,
        alg_guess: &mut [f64],
    ) -> Result<Option<f64>, SimError> {
        if self.indicators.is_empty() || to <= from + 1e-12 {
            return Ok(None);
        }
        // A reading of exactly zero where the walk stands is a relation
        // standing on its threshold: it turns as the walk leaves, and
        // there is nothing to search for. Reading it as an indicator to
        // leave alone is what used to lose a crossing that landed on an
        // output point.
        let hair = from + (to - from) * 1e-9;
        self.eval_point(to, y, values, scratch, alg_guess)?;
        let after = self.indicator_values(to, values);
        let mut earliest: Option<f64> = None;
        for (index, (&start, &end)) in before.iter().zip(&after).enumerate() {
            if !turned(start, end) {
                continue;
            }
            if start == 0.0 {
                earliest = Some(earliest.map_or(hair, |so_far: f64| so_far.min(hair)));
                continue;
            }
            let (mut lo, mut hi) = (from, to);
            for _ in 0..40 {
                let mid = 0.5 * (lo + hi);
                self.eval_point(mid, y, values, scratch, alg_guess)?;
                if start * self.indicator_values(mid, values)[index] <= 0.0 {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            earliest = Some(earliest.map_or(hi, |so_far: f64| so_far.min(hi)));
        }
        Ok(earliest)
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
            mut indicators_prev,
            mut out_i,
            mut last_out_t,
            mut terminated,
            ..
        } = segment;
        let (stop, out_step) = (self.stop_time, self.step.max(1e-12));
        // The last instant this walk has been to, which is where the
        // search for a crossing starts from.
        let mut walked = last_out_t;
        loop {
            // Walk to whichever comes first: the next output point or
            // the next scheduled time event.
            let grid = out_i as f64 * out_step;
            let mut t = match state.next_time_event() {
                Some(next) if next < grid - 1e-12 => next,
                _ => grid,
            };
            if t > stop + 1e-12 {
                break;
            }
            // A relation may turn between the last instant walked to and
            // this one, and the turn is where the event belongs - not at
            // whichever grid point first happens to see it. Nothing is
            // integrated here, so every value is a function of the time
            // asked for and the crossing is found by asking, rather than
            // by interpolating a step the way the solvers do.
            if let Some(crossing) = self.first_crossing(
                walked,
                t,
                &indicators_prev,
                &y,
                &mut values,
                &mut scratch,
                &mut alg_guess,
            )? {
                t = crossing;
            }
            self.record_row(t, &y, &mut values, &mut scratch, &mut alg_guess, &mut rows)?;
            // The row just written used the model on hand; the
            // continuation writes this instant again with the one that
            // now applies.
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
            indicators_prev = self.indicator_values(t, &values);
            walked = t;
            terminated = outcome.terminated;
            if terminated.is_some() {
                break;
            }
        }
        if terminated.is_none() {
            if last_out_t < stop - 1e-12 {
                self.record_row(
                    stop,
                    &y,
                    &mut values,
                    &mut scratch,
                    &mut alg_guess,
                    &mut rows,
                )?;
            }
            // See `finish_segment`: the end of the run is an event.
            values[self.terminal_slot] = 1.0;
            let outcome =
                self.handle_event(stop, &mut y, &mut values, &mut alg_guess, &mut state)?;
            if outcome.changed {
                self.record_row(
                    stop,
                    &y,
                    &mut values,
                    &mut scratch,
                    &mut alg_guess,
                    &mut rows,
                )?;
            }
            terminated = outcome.terminated;
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
        if terminated.is_none() {
            if last_out_t < stop - 1e-12 {
                self.record_row(
                    stop,
                    &y,
                    &mut values,
                    &mut scratch,
                    &mut alg_guess,
                    &mut rows,
                )?;
            }
            // Reaching the stop time is itself an event: `terminal()`
            // becomes true and a `when` watching it fires once, here,
            // with everything the run arrived at still in place. A run
            // the model stopped itself never gets here, which is the
            // difference between an analysis that ended and one that
            // succeeded.
            values[self.terminal_slot] = 1.0;
            let outcome =
                self.handle_event(stop, &mut y, &mut values, &mut alg_guess, &mut state)?;
            if outcome.changed {
                self.record_row(
                    stop,
                    &y,
                    &mut values,
                    &mut scratch,
                    &mut alg_guess,
                    &mut rows,
                )?;
            }
            terminated = outcome.terminated;
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
        // A carried profile is read at a coordinate that is itself a
        // state, so this comes after they are in place.
        self.fill_transports(t, values);
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
        // A body the run walks answers with a number, so a walk that
        // failed left its reason behind rather than raising one. This is
        // where it is read back out.
        match self.walked.complaint() {
            Some(why) => Err(SimError(format!("at t = {t:.6}: {why}"))),
            None => Ok(()),
        }
    }

    /// Solve one implicit algebraic block by damped Newton iteration
    /// with a finite-difference Jacobian. `alg_guess` supplies warm
    /// starts (the previous evaluation point) and receives the
    /// solution.
    ///
    /// The step is the full Newton one until the iteration is caught
    /// walking in a circle, and shortened to a descent from then on.
    /// The circle is what a saturating amplifier does to an undamped
    /// step: on the flat of a limiter the Jacobian says the residual
    /// can be cleared by a step landing on the opposite rail, the full
    /// step takes it, and the same argument sends the next one back.
    /// Damping from the start instead would be worse than either - a
    /// magnetic curve entering saturation raises its residual once and
    /// then converges, and a rule demanding descent every time
    /// shortens the step that was about to work.
    /// A block whose residual is not a number at the point it starts
    /// from has not diverged: it was asked a question arithmetic
    /// cannot answer, and nearly always the question is a reciprocal
    /// of an unknown standing at the zero its declaration left it.
    /// `R_m = 1/G_m` is the whole of it - nothing about the model is
    /// wrong, and the plan cannot avoid it either, because the
    /// division is written in the equation rather than in an
    /// assignment the plan chose. What is wrong is only the point the
    /// iteration was handed.
    ///
    /// So the block is tried again from off the zero rather than
    /// given up on. Which value to move to is not a free choice and
    /// no single one serves: near the pole the slope is enormous and
    /// Newton creeps, far from it the slope is flat and the Jacobian
    /// reads as singular, and the distance at which a reciprocal
    /// turns from one into the other is the model's own scale, which
    /// this layer does not know. So several are tried in turn, and
    /// the first that solves is the answer - which is then checked
    /// exactly as any other solution of the block is.
    ///
    /// A block whose residual is a number where it stands is left
    /// alone entirely: the first attempt is the start it was handed,
    /// and the rest of the list is never reached.
    pub(crate) fn solve_implicit_block(
        &self,
        t: f64,
        values: &mut [f64],
        stage: &AlgStage,
        alg_guess: &mut [f64],
        validate: bool,
    ) -> Result<(), SimError> {
        let first = self.solve_implicit_block_from(t, values, stage, alg_guess, validate, None);
        // Only the one refusal is retried, and it has to be named
        // rather than taken as "anything that failed". A block that
        // did not converge, or converged on a solution it cannot call
        // unique, has been evaluated and has something to say about
        // the model; started again from elsewhere it would say the
        // same thing more slowly, and `der(x)^2 = 4` - which has two
        // roots and must be refused - would come back with whichever
        // root the retry happened to land on. What is retried is the
        // block that was never evaluated at all.
        //
        // Following the flux tubes past the first wall, the same cure
        // was tried on a Jacobian whose column is dead at the zero the
        // block was handed - and it is not safe, which the tests said
        // at once. `der(x)^2 = 4` has a dead column at zero for the
        // same arithmetic reason, and retried from elsewhere it comes
        // back with whichever of its two roots the retry landed on: a
        // wrong number presented as a right one, in place of the
        // refusal that was owed. A dead column is a statement about
        // the block, not about the point, and it stays refused.
        let unevaluated = matches!(&first, Err(e) if e.0.contains("before any Newton step"));
        if !unevaluated || std::env::var_os("OXIDELICA_NO_ZERO_STEP").is_some() {
            return first;
        }
        let AlgStage::Implicit { torn: block, .. } = stage else {
            return first;
        };
        if !block.iter().any(|&i| alg_guess[i] == 0.0) {
            return first;
        }
        for magnitude in [1e-6, 1e-3, 1.0, 1e3] {
            let start: Vec<f64> = block
                .iter()
                .map(|&i| {
                    if alg_guess[i] == 0.0 {
                        magnitude
                    } else {
                        alg_guess[i]
                    }
                })
                .collect();
            let again =
                self.solve_implicit_block_from(t, values, stage, alg_guess, validate, Some(&start));
            if again.is_ok() {
                return again;
            }
        }
        first
    }

    fn solve_implicit_block_from(
        &self,
        t: f64,
        values: &mut [f64],
        stage: &AlgStage,
        alg_guess: &mut [f64],
        validate: bool,
        start: Option<&[f64]>,
    ) -> Result<(), SimError> {
        let AlgStage::Implicit {
            torn: block,
            inner,
            residuals,
            residual_sources,
            inner_sources,
            residual_reads,
            ..
        } = stage
        else {
            return Ok(());
        };
        let n = block.len();
        let mut v: Vec<f64> = match start {
            Some(given) => given.to_vec(),
            None => block.iter().map(|&i| alg_guess[i]).collect(),
        };

        // Both halves of each equation are kept, not only their
        // difference: how large a residual has to be before it means
        // anything is a question about the numbers it was subtracted
        // from, and the difference alone no longer remembers them.
        let residual_parts = |values: &mut [f64], v: &[f64]| -> Vec<(f64, f64)> {
            for (j, &index) in block.iter().enumerate() {
                values[self.algebraic_slots[index]] = v[j];
            }
            // Torn values fixed: the inner unknowns follow explicitly.
            for (var, code) in inner {
                values[self.algebraic_slots[*var]] = code.run(values, t);
            }
            residuals
                .iter()
                .map(|(lhs, rhs)| (lhs.run(values, t), rhs.run(values, t)))
                .collect()
        };
        let residual = |values: &mut [f64], v: &[f64]| -> Vec<f64> {
            residual_parts(values, v)
                .into_iter()
                .map(|(lhs, rhs)| lhs - rhs)
                .collect()
        };
        let block_names =
            || -> Vec<&str> { block.iter().map(|&i| self.algebraics[i].as_str()).collect() };

        let mut seen: Vec<Vec<f64>> = Vec::new();
        let mut damped = false;
        // How many steps in a row the line search could not make
        // descend. One such step is ordinary - `PumpAndValve` takes
        // one on its way and converges afterwards - so what the guard
        // below watches for is a run of them.
        let mut stuck = 0usize;
        // The step that brought the iteration to where it now stands:
        // where it came from, which way it went and how much of that
        // way it took. Kept so that a step over the edge of a domain
        // can be taken again, shorter, from the footing it left.
        let mut footing: Option<(Vec<f64>, Vec<f64>, f64)> = None;
        for iteration in 0..50 {
            let parts = residual_parts(values, &v);
            let f: Vec<f64> = parts.iter().map(|(lhs, rhs)| lhs - rhs).collect();
            if newton_trail() {
                let norm = f.iter().map(|x| x * x).sum::<f64>().sqrt();
                eprintln!("newton {iteration} t={t} |f|={norm:e} v={v:?} f={f:?}");
            }
            // A step that lands where the residual is not a number has
            // not diverged: the iteration was going the right way and
            // overshot the edge of a domain. Water is where this shows
            // - the trail of `SeriesPipes2` steps a pressure from
            // 4.97e5 to 2.08e7, where the IF97 formulation answers NaN,
            // and the block was reported as diverged on its second
            // iteration with a perfectly finite residual behind it.
            // Nothing at the point stepped from gives the edge away, so
            // the only thing to do is go back and take less of the same
            // step. Divergence is then what is left when even a step of
            // a millionth of the way still cannot be evaluated.
            if !f.iter().all(|x| x.is_finite()) {
                if let Some((from, dv, lambda)) = footing.take() {
                    let lambda = lambda / 2.0;
                    if lambda > 1e-6 {
                        v = (0..n).map(|j| from[j] - lambda * dv[j]).collect();
                        footing = Some((from, dv, lambda));
                        // A block that has once been over the edge
                        // keeps the shortened step for the rest of
                        // this solve. Without that it walks back to
                        // the same edge at the next full step and
                        // spends its whole budget going over and
                        // coming back, which is the trail the small
                        // model printed before this line was added.
                        damped = true;
                        continue;
                    }
                }
            }
            // A residual that is not a number is not a step away from
            // the solution: the block never had a finite one to step
            // from. Reported as divergence it names a thing that did
            // not happen and sends the reader to the solver, when
            // what went wrong is upstream - an explicit assignment
            // whose divisor vanished in the branch the model starts
            // in. Say which residual, and say what it was.
            if iteration == 0 {
                if let Some((i, bad)) = f.iter().enumerate().find(|(_, x)| !x.is_finite()) {
                    // Which of the block's own values is not a number
                    // says where the fault entered: a residual built
                    // from finite inputs cannot come out NaN, so the
                    // first unknown that is one is the assignment to
                    // look at. Named, this refusal points at a line;
                    // unnamed, it points at the solver, which is the
                    // one place the fault is not.
                    let mut bad_names: Vec<String> = Vec::new();
                    for (k, (var, _)) in inner.iter().enumerate() {
                        let value = values[self.algebraic_slots[*var]];
                        if !value.is_finite() {
                            // The assignment, not only the name it
                            // wrote. A torn block recovers its inner
                            // unknowns from explicit expressions, and
                            // the one that came out NaN is the whole
                            // of the fault; the name alone sends the
                            // reader looking for an equation the plan
                            // no longer holds under that spelling.
                            match inner_sources.get(k) {
                                Some(source) => bad_names.push(format!(
                                    "{} = {source} = {value}",
                                    self.algebraics[*var]
                                )),
                                None => {
                                    bad_names.push(format!("{} = {value}", self.algebraics[*var]));
                                }
                            }
                        }
                    }
                    let entered = if bad_names.is_empty() {
                        // Nothing of the block's own is at fault, and
                        // a block with no inner assignments has
                        // nothing of its own at all. Then the fault
                        // came in from outside, through one of the
                        // names the residual reads, and every one of
                        // those was settled before the block was
                        // reached. Naming them is the difference
                        // between a refusal that points at a page of
                        // arithmetic and one that points at a value.
                        let read: Vec<String> = residual_reads
                            .get(i)
                            .map(|names| {
                                names
                                    .iter()
                                    .filter(|(_, slot)| !values[*slot].is_finite())
                                    .map(|(name, slot)| format!("{name} = {}", values[*slot]))
                                    .collect()
                            })
                            .unwrap_or_default();
                        if read.is_empty() {
                            // A silence here is indistinguishable from
                            // a check that never ran, and the negative
                            // finding then lives in the code rather
                            // than in what the instrument printed. Say
                            // the positive half: the fault is in the
                            // arithmetic itself, not in anything
                            // handed to it.
                            "; and every value the residual reads is a finite number".to_string()
                        } else {
                            format!("; it reads values that are not numbers: {read:?}")
                        }
                    } else {
                        format!("; the block's own values are not numbers: {bad_names:?}")
                    };
                    let which = residual_sources
                        .get(i)
                        .map_or_else(|| format!("residual {i}"), |source| format!("`{source}`"));
                    // A body the run walks cannot raise: a walk that
                    // fails answers with a number that is not one and
                    // leaves its reason behind. If one of those is
                    // what made this residual NaN, the reason is the
                    // whole answer, and the block around it is the
                    // place the fault was noticed rather than the
                    // place it happened. Read it out before the
                    // refusal is written, or it is dropped when the
                    // block returns and the reader is sent to the
                    // solver for a fault in a function body.
                    if let Some(why) = self.walked.complaint() {
                        return err(format!(
                            "{which} of algebraic loop {:?} is {bad} at t = {t}, \
                             before any Newton step, because a function it calls \
                             could not be walked: {why}",
                            block_names()
                        ));
                    }
                    return err(format!(
                        "{which} of algebraic loop {:?} is {bad} at t = {t}, \
                         before any Newton step: the equations cannot be evaluated \
                         at the values the block starts from{entered}",
                        block_names()
                    ));
                }
            }
            // An equation is solved when its two sides agree, and
            // what "agree" means is set by how large those sides are.
            // Judged against the unknown alone, a diode's exponential
            // is asked to cancel a current of a thousand million
            // against a volt: the two sides agree to every digit
            // double precision holds, their difference sits at 3e-7
            // because that is where the rounding of 1e9 lands, and
            // the test demands 1e-10. Newton then steps by nothing,
            // gets the same residual back, and spends its remaining
            // thirty-five iterations reproducing it - the trail in
            // /tmp/m202/trail.txt shows all thirty-five identical.
            // The second half of the test asks the question the
            // first cannot: a difference below the rounding noise of
            // the numbers it came from is not a distance from the
            // solution, it is the floor of the arithmetic, and no
            // iteration can go under it.
            let converged = f.iter().zip(&v).zip(&parts).all(|((fi, vi), (lhs, rhs))| {
                fi.abs() <= 1e-10 * (1.0 + vi.abs()) || fi.abs() <= 1e-12 * (lhs.abs() + rhs.abs())
            });
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
                    // Judged on a matrix whose rows have each been
                    // divided through by their own largest entry. The
                    // spread between one equation of a block and
                    // another is a matter of the units they are
                    // written in, and the comparison below is against
                    // the whole matrix's largest entry, so without
                    // taking that spread out the test asks about the
                    // units as much as about the block. A magnetic
                    // circuit is where this bites: a permeance near
                    // `mu_0` and a reluctance near its reciprocal are
                    // in one block by construction, and read unscaled
                    // every such block is called underdetermined
                    // while being plainly invertible.
                    if std::env::var_os("OXIDELICA_NO_ROW_SCALING").is_none() {
                        // Columns as well as rows, and for the same
                        // reason read along the other axis: a row's
                        // scale is the unit its equation is written
                        // in, and a column's is the unit its unknown
                        // is measured in. Neither is a fact about
                        // whether the block determines a solution, so
                        // a test that notices either is asking about
                        // the units. The flux tubes are where the
                        // column half bites - a reluctance near 1e6
                        // beside a flux near 1e-5 - and unscaled the
                        // flux's column read as dead.
                        if std::env::var_os("OXIDELICA_DIVISOR_BY_MENTION").is_none() {
                            crate::linear::equilibrate_columns(&mut jac);
                        }
                        equilibrate_rows(&mut jac);
                    }
                    let probe = vec![1.0; n];
                    // Judged against the Jacobian's own scale, not
                    // against zero. The matrix here is built by finite
                    // differences, and a column that cancels exactly in
                    // exact arithmetic comes back as noise of order
                    // 1e-8 rather than as a zero: `x = y + 1` beside
                    // `y = x - 1` is the same equation twice, and its
                    // Jacobian read absolutely looks invertible. What
                    // makes a block underdetermined is a direction the
                    // residual barely moves along *compared with the
                    // rest of the block*, which is what this asks.
                    let scale = jac
                        .iter()
                        .flat_map(|row| row.iter())
                        .fold(0.0f64, |m, x| m.max(x.abs()));
                    let singular = solve_linear(&mut jac.clone(), &probe).is_none()
                        || smallest_pivot(&mut jac.clone()) <= 1e-7 * scale.max(1.0);
                    if singular {
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
            // A column's entries are small because of the unit its
            // unknown is measured in, and no honest test may notice
            // that. The enthalpies of a cooling circuit are the case:
            // `PumpAndValve` hands a block whose five enthalpy columns
            // sit at 1e-24 beside a volume flow at 1e-4, because an
            // enthalpy is carried by a mass flow that is zero at rest,
            // and `solve_linear` judges its pivots against 1e-14 flat.
            // Divided each column through by its own largest entry the
            // block is plainly invertible; the step comes back in the
            // scaled unknowns and is divided back out again. The
            // argument is the one `equilibrate_columns` was written
            // for, one path over: the check that a *converged* block
            // is determined already scales, and the step that has to
            // get there did not.
            let mut rescued = false;
            let step = solve_linear(&mut jac.clone(), &f).or_else(|| {
                if std::env::var_os("OXIDELICA_NO_COLUMN_UNITS").is_some() {
                    return None;
                }
                let mut scaled = jac.clone();
                let units: Vec<f64> = (0..n)
                    .map(|j| scaled.iter().fold(0.0f64, |m, row| m.max(row[j].abs())))
                    .collect();
                crate::linear::equilibrate_columns(&mut scaled);
                let dv = solve_linear(&mut scaled, &f)?;
                let dv: Vec<f64> = (0..n).map(|j| dv[j] / units[j]).collect();
                rescued = dv.iter().all(|x| x.is_finite());
                rescued.then_some(dv)
            });
            let Some(dv) = step else {
                // A column that is exactly zero is not a matrix that
                // happened to come out ill conditioned: it is the
                // block saying that nothing in it moves when that
                // unknown moves, so no arithmetic on the matrix can
                // find a step for it. Reported as a singular Jacobian
                // the refusal names the solver, and the solver is the
                // one place nothing is wrong - the fault is upstream,
                // where the unknown was paired with an equation whose
                // coefficient on it is zero at these values. Say
                // which unknown, and say that the equations do not
                // mention it.
                //
                // Measured on the library, /tmp/m200/census2.txt: the
                // thirty-four models that said `singular Jacobian`
                // came apart into twenty-two that say this and twelve
                // that still say the other, so two thirds of the row
                // were a dead column wearing the words of an ill
                // conditioned matrix. No model moved - a refusal
                // renamed is a wall named, not a wall removed - and
                // 868/526 stood before and after. The smallest of the
                // twenty-two are one unknown apiece and show the two
                // ways in: `T1.irc` of the Spice3 `Oscillator`, whose
                // `irc * m_collectorResist = ...` has a collector
                // resistance the model card leaves at zero, and
                // `bearingFriction.sa` of `GearType2`, which every
                // branch of the friction `if` drops where the bearing
                // is locked. A parameter that is zero and a branch
                // that does not mention it come to the same column.
                let dead: Vec<&str> = (0..n)
                    .filter(|&j| jac.iter().all(|row| row[j] == 0.0))
                    .map(|j| block_names()[j])
                    .collect();
                if dead.is_empty() {
                    if newton_trail() {
                        for (i, row) in jac.iter().enumerate() {
                            eprintln!("jac row {i}: {row:?}");
                        }
                        eprintln!("names: {:?}", block_names());
                    }
                    return err(format!(
                        "singular Jacobian in algebraic loop {:?}",
                        block_names()
                    ));
                }
                return err(format!(
                    "the equations of algebraic loop {:?} do not mention {dead:?} at t = {t}: \
                     nothing in the block changes when it does, so no step determines it",
                    block_names()
                ));
            };
            let full: Vec<f64> = (0..n).map(|j| v[j] - dv[j]).collect();
            // Newton's full step is right unless it walks in a circle.
            // On the flat of a limiter the Jacobian says the residual
            // can be cleared by a step onto the opposite rail; taken,
            // the same argument sends the next step back, and the
            // iteration swings between two points until the budget is
            // out. Nothing about the residual gives this away - it is
            // as large at one rail as at the other - so what is watched
            // for is the return itself.
            let circling = seen.iter().any(|old: &Vec<f64>| {
                old.iter()
                    .zip(&full)
                    .all(|(a, b)| (a - b).abs() <= 1e-6 * (1.0 + a.abs()))
            });
            seen.push(v.clone());
            if seen.len() > 4 {
                seen.remove(0);
            }
            // Once a block has walked in a circle, every later step of
            // this solve is shortened until the residual falls. Before
            // that it is not: insisting on descent from the start
            // shortens steps that were about to work, and a magnetic
            // curve entering saturation rises once before it converges.
            //
            // A step that only exists because the columns were put in
            // their own units is shortened from its first use, and for
            // the same reason read the other way round: the pivot that
            // was too small to solve against unscaled says the block is
            // nearly flat along that unknown, so the full step is a
            // huge one across a direction the linear model barely
            // describes. `PumpAndValve` walks its enthalpies to 1e19
            // on the first such step and never comes back; shortened
            // until the residual falls, it runs.
            damped |= circling || rescued;
            let mut next = full;
            let mut taken = 1.0f64;
            if damped {
                let norm = |r: &[f64]| r.iter().map(|x| x * x).sum::<f64>().sqrt();
                let before = norm(&f);
                let mut lambda = 1.0f64;
                let mut descended = false;
                for _ in 0..20 {
                    if next.iter().all(|value| value.is_finite())
                        && norm(&residual(values, &next)) < before
                    {
                        descended = true;
                        break;
                    }
                    lambda /= 2.0;
                    next = (0..n).map(|j| v[j] - lambda * dv[j]).collect();
                }
                // Twenty halvings that never brought the residual down
                // say the direction is not one the residual falls
                // along, and a millionth of such a step is no better
                // than the whole of it. Taken anyway - which is what
                // happened before this - the iteration walks uphill by
                // a millionth a time until the arithmetic hands back a
                // value that is not a number, and the Jacobian built
                // at that point is reported as singular. That refusal
                // names the matrix, and the matrix is not what is
                // wrong: `BranchingPipes2` crawls twelve iterations
                // with its residual rising from 1.6249e6 to 1.6251e6
                // and then goes NaN (/tmp/m213/bp2.txt), and every
                // column of the Jacobian printed there is NaN rather
                // than dependent. Say the thing that was measured.
                if !descended && !descent_guard_off() {
                    stuck += 1;
                    // One such step is not a verdict: `PumpAndValve`
                    // takes one on its way and converges afterwards,
                    // and a guard that fired on the first cost it.
                    // Three in a row is a block that is not going
                    // anywhere - `BranchingPipes2` has twelve.
                    if stuck >= 3 {
                        return err(format!(
                            "the Newton direction of algebraic loop {:?} does not reduce the \
                             residual at t = {t}: halved twenty times from |f| = {before:e} on \
                             {stuck} steps running and no trial was smaller, so the block has no \
                             step to take from here",
                            block_names()
                        ));
                    }
                } else {
                    stuck = 0;
                }
                taken = lambda;
            }
            footing = Some((v.clone(), dv, taken));
            v = next;
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
