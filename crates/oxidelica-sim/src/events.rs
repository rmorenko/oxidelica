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
    /// The `$pre.` reference of a variable that has a value from
    /// before the event.
    ///
    /// A `when` target has one because it only ever changes at an
    /// event. So does a Boolean or an Integer, whatever assigns it:
    /// the language calls those discrete-valued, and a continuous
    /// equation writing one still only lets it change when a relation
    /// inside it flips, which is an event.
    ///
    /// Inside a `when` body any variable has one, including a moving
    /// state: the body runs at the instant of the event, so the value
    /// the state arrived with is a value that exists. That is what a
    /// block averaging over a period asks for, `pre(x)` being the
    /// integral just before the `reinit` that clears it.
    pub(crate) fn pre_of(&mut self, arg: &Expr, builtin: &str) -> Result<Expr, SimError> {
        let Expr::Ref(name) = arg else {
            return err(format!("{builtin}() takes a variable, not an expression"));
        };
        if !self.discretes.iter().any(|d| d == name) {
            let by_type = self.discrete_valued.iter().any(|d| d == name);
            let at_an_event = self.inside_a_when && self.declared.iter().any(|d| d == name);
            if !by_type && !at_an_event {
                return err(format!(
                    "{builtin}({name}): `{name}` is not discrete, so it has no value from before the event"
                ));
            }
            if !self.pre_wanted.iter().any(|d| d == name) {
                self.pre_wanted.push(name.clone());
            }
        }
        Ok(Expr::Ref(format!("$pre.{name}")))
    }

    /// The same, where an argument may be an array and stays one: a
    /// body written outside Modelica takes what it was handed as it
    /// was handed it, however deep it goes.
    fn whole(&mut self, expr: &Expr) -> Result<Expr, SimError> {
        match expr {
            Expr::Array(items) => Ok(Expr::Array(
                items
                    .iter()
                    .map(|item| self.whole(item))
                    .collect::<Result<Vec<_>, SimError>>()?,
            )),
            one => self.expr(one),
        }
    }

    /// Rewrite one expression.
    pub(crate) fn expr(&mut self, expr: &Expr) -> Result<Expr, SimError> {
        Ok(match expr {
            Expr::WithDerivative(value, rule, seeds) => Expr::WithDerivative(
                Box::new(self.expr(value)?),
                Box::new(self.expr(rule)?),
                seeds
                    .iter()
                    .map(|(name, argument)| Ok((name.clone(), self.expr(argument)?)))
                    .collect::<Result<Vec<_>, SimError>>()?,
            ),
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
                            programs: None,
                            depth: 0,
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
                        programs: None,
                        depth: 0,
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
                // A call the run walks is handed arrays written out;
                // the elements are ordinary expressions and are looked
                // through, but the array around them stays as it is.
                _ => Expr::Call(
                    name.clone(),
                    args.iter()
                        .map(|arg| self.whole(arg))
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
            // `f(x)[2]` of a body the run walks: the call is looked
            // through and the subscript kept, since it says which
            // number of the answer this is.
            Expr::Index(base, subscripts) => {
                Expr::Index(Box::new(self.expr(base)?), subscripts.clone())
            }
            Expr::Member(_, _)
            | Expr::Array(_)
            | Expr::Elementwise(_, _, _)
            | Expr::Range(_, _, _)
            | Expr::Comprehension(_, _, _)
            | Expr::ColonSubscript
            | Expr::EndSubscript
            | Expr::MatrixRows(_)
            | Expr::NamedArg(_, _)
            | Expr::Tuple(_) => {
                return err(format!(
                    "subscripts and arrays survive flattening only as scalars: {}",
                    crate::code::shape_of(expr)
                ))
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
            last_event_t: f64::NAN,
            events_here: 0,
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
        // An event that settles and is immediately followed by another
        // at the same instant is the iteration one storey up, and
        // nothing inside a single event can see it: each one comes to
        // rest, and the run makes no progress in time while writing a
        // row apiece. Counting them where the instant is remembered is
        // what turns an unbounded run into a refusal that names the
        // model and the point of time it stuck at.
        // A chattering model rarely stands on one number exactly: it
        // creeps forward by the smallest step the solver will take,
        // which is progress no epsilon can be chosen against - the
        // creep is whatever the step size happens to have fallen to.
        // What the model asked for instead is a fixed measure: one
        // output interval is a stretch of time the run is meant to
        // cross in a handful of steps, so a great many events inside
        // one is chatter whatever the spacing between them.
        let window = self.step.max(1e-12);
        // A fresh window when the run has left the last one behind -
        // and the state before any event, where the instant is not a
        // number at all, is left behind by everything.
        if !matches!(
            (t - state.last_event_t).partial_cmp(&window),
            Some(std::cmp::Ordering::Less)
        ) {
            state.last_event_t = t;
            state.events_here = 0;
        }
        state.events_here += 1;
        let most = self.max_events_one_interval;
        if state.events_here > most {
            return crate::err(format!(
                "`{}` handled more than {most} events between t = {} and t = {t}, \
                 one output interval: the model's switches raise another event \
                 however many have settled",
                self.name, state.last_event_t
            ));
        }
        for &(slot, pre) in &self.pre_slots {
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
        // Each branch may fire once, and each discrete definition may
        // settle once more after it: that many rounds are enough for
        // an event that comes to rest, and one more is the round that
        // proves it has.
        let rounds = self
            .when_clauses
            .iter()
            .map(|clause| clause.branches.len())
            .sum::<usize>()
            + self.discrete_definitions.len()
            + 1;
        let mut settled = false;
        // Which discrete-valued names moved on the last round. A name
        // that settled early is not what stopped the event coming to
        // rest, and listing it beside the ones that did is the same
        // fault as a refusal that quotes whichever parameter came
        // first: the reader cannot tell the cause from the company it
        // keeps. `Counter`'s list went from seventy-nine names to
        // forty - four triggers of ten names apiece, which is the
        // ring that actually turns rather than every discrete value
        // the model holds.
        let mut moved: Vec<usize> = Vec::new();
        for _ in 0..rounds {
            let mut acted = false;
            moved.clear();
            // What a discrete-valued name is worth now. Unlike the
            // body of a `when`, which fires on an edge, these hold at
            // every moment of the event, so they are asked every round
            // rather than when something just became true. A value
            // that moves is a reason to go round again: the algebraic
            // part is solved with the switches held still, and a
            // switch that flips changes the system it was solved in.
            //
            // They are settled among themselves before any `when` is
            // allowed to fire, and the reason is the whole of what a
            // `when initial()` is for. A definition reads the
            // algebraic part, and the algebraic part is only
            // re-evaluated at the top of a pass, so a definition whose
            // input is another definition's output reads the value
            // from before the event on the first pass through. That is
            // harmless for a definition, which is asked again next
            // pass - but a `when initial()` fires exactly once, and if
            // it fires on that pass it writes what it read from a
            // half-built point. In `Modelica.Electrical.Digital` the
            // half-built point is `bUF3S.yy = NaN`, the delay's body
            // stores the NaN, and because NaN is equal to nothing at
            // all - not even itself - the definition that holds it
            // reports a change on every pass for ever and the event
            // never comes to rest.
            for _ in 0..=self.discrete_definitions.len() {
                self.eval_point(t, y, values, &mut scratch, alg_guess)?;
                let mut again = false;
                for (at, (slot, code)) in self.discrete_definitions.iter().enumerate() {
                    let new = code.run(values, t);
                    // A value that is NaN twice running has not moved.
                    // The comparison below is the one the language
                    // means - a discrete value holds until something
                    // assigns it another - and IEEE's answer that NaN
                    // differs from itself is about arithmetic rather
                    // than about whether an assignment happened.
                    if values[*slot] != new && !(values[*slot].is_nan() && new.is_nan()) {
                        values[*slot] = new;
                        outcome.changed = true;
                        acted = true;
                        again = true;
                        if !moved.contains(&at) {
                            moved.push(at);
                        }
                    }
                }
                if !again {
                    break;
                }
            }
            // A probe rather than a feature: what the event iteration
            // held after each round, so that a refusal naming three
            // names that keep moving can be read as the ring they
            // actually turn in. `OXIDELICA_EVENT_TRAIL=1` runs it.
            if std::env::var_os("OXIDELICA_EVENT_TRAIL").is_some() {
                let held: Vec<String> = self
                    .discrete_definitions
                    .iter()
                    .filter_map(|(slot, _)| {
                        let at = self.discrete_slots.iter().position(|held| held == slot)?;
                        Some(format!("{} = {}", self.discretes.get(at)?, values[*slot]))
                    })
                    .collect();
                eprintln!("event t = {t}: {held:?}");
            }
            let now = self.when_conditions(t, values);
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
                        // A check made when the event fires: what a
                        // model means by writing it here is that the
                        // thing must hold at that moment, and a run
                        // where it does not is wrong rather than over.
                        CompiledAction::Assert(condition, message) => {
                            if condition.run(values, t) == 0.0 {
                                return crate::err(format!(
                                    "assertion failed at t = {t:.6}: {message}"
                                ));
                            }
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
                settled = true;
                break;
            }
        }
        // An event that never comes to rest is a model whose switches
        // chase each other: saying so with the names of what was still
        // moving is worth more than a step that quietly carries the
        // last round's values forward as though they had settled.
        if !settled {
            // The discrete-valued names, which is where a definition
            // that keeps moving has to be: the slots run alongside.
            let names: Vec<String> = moved
                .iter()
                .filter_map(|&which| {
                    let (slot, _) = self.discrete_definitions.get(which)?;
                    let at = self.discrete_slots.iter().position(|held| held == slot)?;
                    let name = self.discretes.get(at)?;
                    Some(format!("{name} = {}", values[*slot]))
                })
                .collect();
            // The list cannot come out empty, and the reason is worth
            // writing down rather than guarded against: a refusal that
            // named `among []` would tell the reader less than the long
            // one this replaced, so the question is real. But a `when`
            // branch fires at most once an event - `fired` sees to
            // that - and the bound is the branch count plus the
            // definition count plus one, so by the round the loop runs
            // out, every branch that could fire has, and the only thing
            // that can still set `acted` is a definition that moved.
            // Whatever the loop gave up on, it gave up on with a name.
            return Err(SimError(format!(
                "the event at t = {t} does not come to rest after {rounds} round(s): \
                 what changes on every round is among {names:?}"
            )));
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
        // A model whose discrete value goes to NaN runs to its stop
        // time and says nothing: the integration never touches a
        // discrete slot, so nothing downstream is in a position to
        // notice. Asked here, once the event has come to rest, the
        // question is about what the event decided rather than about a
        // value some round of the iteration merely passed through - a
        // half-built point inside a round is allowed to hold NaN, and
        // catching it there would refuse models that settle perfectly
        // well.
        //
        // Measured over the corpus from one binary with the check and
        // without it, the whole cost is six models, and all six are
        // the `Digital` family the check was built to look at. Nothing
        // outside it was answering with NaN, so the check is on by
        // default; `OXIDELICA_DISCRETE_NAN_GUARD=0` takes it off for
        // anyone who would rather have the wrong number.
        if std::env::var("OXIDELICA_DISCRETE_NAN_GUARD").as_deref() != Ok("0") {
            for (at, &slot) in self.discrete_slots.iter().enumerate() {
                if values[slot].is_nan() {
                    let name = self
                        .discretes
                        .get(at)
                        .map_or_else(|| format!("discrete #{at}"), Clone::clone);
                    return crate::err(format!(
                        "`{name}` is not a number after the event at t = {t}: \
                         a discrete value that reaches NaN is carried to the \
                         stop time unchanged, so every row written from here \
                         on is wrong",
                    ));
                }
            }
        }
        Ok(outcome)
    }
}
