//! Events: the built-ins that become flags, the state a run keeps
//! about them, and what happens when one fires.

use crate::*;

impl EventState {
    /// The next scheduled time event, if the model has any.
    pub(crate) fn next_time_event(&self) -> Option<f64> {
        self.next_sample.iter().copied().reduce(f64::min)
    }

    /// Raise the flag of every `sample(...)` occurring at `t` and move
    /// its schedule on.
    pub(crate) fn raise_samples(
        &mut self,
        t: f64,
        samples: &[(f64, f64)],
        flags: &[Slot],
        values: &mut [f64],
    ) {
        for (index, (_, interval)) in samples.iter().enumerate() {
            if self.next_sample[index] <= t + 1e-9 {
                values[flags[index]] = 1.0;
                self.next_sample[index] += interval.max(1e-12);
            }
        }
    }
}

impl EventRewrite<'_> {
    /// The `$pre.` reference of a discrete variable.
    pub(crate) fn pre_of(&self, arg: &Expr, builtin: &str) -> Result<Expr, SimError> {
        let Expr::Ref(name) = arg else {
            return err(format!("{builtin}() takes a variable, not an expression"));
        };
        if !self.discretes.iter().any(|d| d == name) {
            return err(format!(
                "{builtin}({name}): `{name}` is not discrete, so it has no value from before the event"
            ));
        }
        Ok(Expr::Ref(format!("$pre.{name}")))
    }

    /// Rewrite one expression.
    pub(crate) fn expr(&mut self, expr: &Expr) -> Result<Expr, SimError> {
        Ok(match expr {
            Expr::Str(_) => expr.clone(),
            Expr::Call(name, args) => match (name.as_str(), args.len()) {
                ("pre", 1) => self.pre_of(&args[0], "pre")?,
                // edge(b) is "b just became true", change(v) is "v just
                // took a different value".
                ("edge", 1) => Expr::And(
                    Box::new(Expr::Rel(
                        RelOp::Gt,
                        Box::new(args[0].clone()),
                        Box::new(Expr::Number(0.5)),
                    )),
                    Box::new(Expr::Rel(
                        RelOp::Lt,
                        Box::new(self.pre_of(&args[0], "edge")?),
                        Box::new(Expr::Number(0.5)),
                    )),
                ),
                ("change", 1) => Expr::Rel(
                    RelOp::Ne,
                    Box::new(args[0].clone()),
                    Box::new(self.pre_of(&args[0], "change")?),
                ),
                ("initial", 0) => Expr::Ref("$initial".to_string()),
                ("terminal", 0) => Expr::Ref("$terminal".to_string()),
                // `delay(u, T)` reads what `u` was `T` ago, which the
                // run remembers for it. The third argument, the
                // longest delay a variable one might reach, is not
                // needed here: this delay is a constant.
                ("delay", 2) | ("delay", 3) => {
                    let source = self.expr(&args[0])?;
                    let seconds = eval(
                        &args[1],
                        &EvalCtx {
                            vars: self.params,
                            time: 0.0,
                        },
                    )?;
                    if seconds <= 0.0 || seconds.is_nan() {
                        return err(format!("delay(..., {seconds}): the delay must be positive and known before the run"));
                    }
                    self.delays.push((source, seconds));
                    Expr::Ref(format!("$delay{}", self.delays.len() - 1))
                }
                ("sample", 2) => {
                    let ctx = EvalCtx {
                        vars: self.params,
                        time: 0.0,
                    };
                    let start = eval(&args[0], &ctx)?;
                    let interval = eval(&args[1], &ctx)?;
                    if interval <= 0.0 || interval.is_nan() {
                        return err(format!(
                            "sample(..., {interval}): the interval must be positive"
                        ));
                    }
                    let index = match self
                        .samples
                        .iter()
                        .position(|&(s, i)| s == start && i == interval)
                    {
                        Some(index) => index,
                        None => {
                            self.samples.push((start, interval));
                            self.samples.len() - 1
                        }
                    };
                    Expr::Ref(format!("$sample{index}"))
                }
                _ => Expr::Call(
                    name.clone(),
                    args.iter()
                        .map(|a| self.expr(a))
                        .collect::<Result<Vec<_>, SimError>>()?,
                ),
            },
            Expr::Neg(inner) => Expr::Neg(Box::new(self.expr(inner)?)),
            Expr::Not(inner) => Expr::Not(Box::new(self.expr(inner)?)),
            Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(self.expr(l)?), Box::new(self.expr(r)?)),
            Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(self.expr(l)?), Box::new(self.expr(r)?)),
            Expr::And(l, r) => Expr::And(Box::new(self.expr(l)?), Box::new(self.expr(r)?)),
            Expr::Or(l, r) => Expr::Or(Box::new(self.expr(l)?), Box::new(self.expr(r)?)),
            Expr::If(c, a, b) => Expr::If(
                Box::new(self.expr(c)?),
                Box::new(self.expr(a)?),
                Box::new(self.expr(b)?),
            ),
            Expr::Index(_, _)
            | Expr::Member(_, _)
            | Expr::Array(_)
            | Expr::Elementwise(_, _, _)
            | Expr::Range(_, _, _)
            | Expr::Comprehension(_, _, _)
            | Expr::ColonSubscript
            | Expr::EndSubscript
            | Expr::MatrixRows(_)
            | Expr::NamedArg(_, _)
            | Expr::Tuple(_) => {
                return err("subscripts and arrays survive flattening only as scalars".to_string())
            }
            Expr::Ref(_) | Expr::Number(_) | Expr::Bool(_) | Expr::Time => expr.clone(),
        })
    }
}

impl CompiledModel {
    /// Values of the event indicators at a point already evaluated.
    pub(crate) fn indicator_values(&self, t: f64, values: &[f64]) -> Vec<f64> {
        self.indicators
            .iter()
            .map(|code| code.run(values, t))
            .collect()
    }

    /// Truth of every `when` branch, clause by clause.
    pub(crate) fn when_conditions(&self, t: f64, values: &[f64]) -> Vec<Vec<bool>> {
        self.when_clauses
            .iter()
            .map(|clause| {
                clause
                    .branches
                    .iter()
                    .map(|branch| branch.condition.run(values, t) != 0.0)
                    .collect()
            })
            .collect()
    }

    /// The state every run starts from: the initial-event flag raised
    /// and every `sample(...)` scheduled at its start. Discrete values
    /// live in the value array, already carrying their declared starts.
    pub(crate) fn event_state(&self) -> EventState {
        EventState {
            // Modelica treats every condition as false before the start,
            // so one that already holds at t = 0 fires immediately. A
            // continuation instead resumes from the current truth, which
            // the solver fills in after its first evaluation.
            when_prev: self
                .when_clauses
                .iter()
                .map(|clause| vec![false; clause.branches.len()])
                .collect(),
            // On a continuation each clock resumes at its first tick
            // after the point reached; ticks behind it already fired.
            next_sample: self
                .samples
                .iter()
                .map(|&(start, interval)| {
                    if !self.resume || start > self.start_time + 1e-9 {
                        start
                    } else {
                        let periods = ((self.start_time - start) / interval + 1e-9).floor() + 1.0;
                        start + periods * interval
                    }
                })
                .collect(),
        }
    }

    /// Handle an event at `t`: fire the `when` branches whose condition
    /// just became true, and keep firing while their assignments make
    /// further conditions true — the event iteration of the language.
    ///
    /// `pre(x)` keeps the value each discrete variable had when the event
    /// began, however many rounds the iteration takes.
    pub(crate) fn handle_event(
        &self,
        t: f64,
        y: &mut [f64],
        values: &mut [f64],
        alg_guess: &mut [f64],
        state: &mut EventState,
    ) -> Result<EventOutcome, SimError> {
        let mut outcome = EventOutcome::default();
        for (&slot, &pre) in self.discrete_slots.iter().zip(&self.pre_slots) {
            values[pre] = values[slot];
        }
        let before_event = state.when_prev.clone();
        // A `reinit` is collected rather than applied: the event
        // iteration works on the discrete variables, and the new value
        // of a state is what the integration resumes from afterwards.
        let mut pending_reinit: Vec<(usize, f64)> = Vec::new();
        let mut fired: Vec<Vec<bool>> = before_event
            .iter()
            .map(|branches| vec![false; branches.len()])
            .collect();
        let mut scratch = Vec::new();

        // Every branch fires at most once per event, so one round per
        // branch plus a final quiet one is all the iteration can need.
        let rounds = self
            .when_clauses
            .iter()
            .map(|clause| clause.branches.len())
            .sum::<usize>()
            + 1;
        for _ in 0..rounds {
            // The algebraic part follows the discrete values, so it is
            // re-evaluated before the conditions are tested again.
            self.eval_point(t, y, values, &mut scratch, alg_guess)?;
            let now = self.when_conditions(t, values);
            let mut acted = false;
            for (index, clause) in self.when_clauses.iter().enumerate() {
                // `elsewhen` is a priority list: the first branch that
                // just became true is the one that fires.
                let Some(branch) = (0..clause.branches.len())
                    .find(|&b| now[index][b] && !before_event[index][b] && !fired[index][b])
                else {
                    continue;
                };
                fired[index][branch] = true;
                acted = true;
                for action in &clause.branches[branch].actions {
                    match action {
                        CompiledAction::Terminate(message) => {
                            outcome.terminated =
                                Some(format!("terminated at t = {t:.6}: {message}"));
                        }
                        CompiledAction::Reinit(state_index, code) => {
                            pending_reinit.push((*state_index, code.run(values, t)));
                            outcome.reinitialized = true;
                            outcome.changed = true;
                        }
                        CompiledAction::Assign(discrete_index, code) => {
                            let new = code.run(values, t);
                            let slot = self.discrete_slots[*discrete_index];
                            if values[slot] != new {
                                outcome.changed = true;
                            }
                            // Later equations of the same branch see the
                            // new value, the way a simultaneous solution
                            // of a triangular system would.
                            values[slot] = new;
                        }
                    }
                }
            }
            if !acted {
                break;
            }
        }
        // The one-shot flags go down with the event, so the conditions
        // remembered for the next one do not see them still raised - a
        // `sample(...)` must be a fresh edge every period.
        values[self.initial_slot] = 0.0;
        for &slot in &self.sample_slots {
            values[slot] = 0.0;
        }
        // The states restarted by `reinit` take their new values now
        // that the round of events is over - and before the point is
        // evaluated again, so what the conditions are remembered as is
        // what they are once the jump has happened. Remembering them
        // from before it would leave a condition that fired standing
        // true for ever, and it would never fire again.
        for (index, value) in pending_reinit {
            y[index] = value;
        }
        self.eval_point(t, y, values, &mut scratch, alg_guess)?;
        state.when_prev = self.when_conditions(t, values);
        Ok(outcome)
    }
}
