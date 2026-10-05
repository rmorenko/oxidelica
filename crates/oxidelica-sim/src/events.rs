//! Events: the built-ins that become flags, the state a run keeps
//! about them, and what happens when one fires.

use crate::*;

impl EventState {
    /// The next scheduled time event, if the model has any.
    pub(crate) fn next_time_event(&self) -> Option<f64> {
        self.next_sample
            .iter()
            .chain(&self.next_clock)
            .copied()
            .filter(|at| at.is_finite())
            .reduce(f64::min)
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

    /// Turn every relation on `time` whose threshold is `t`: from here
    /// on it holds the value it has past the threshold, and it is never
    /// due again. Says whether any turned.
    pub(crate) fn turn_clocks(
        &mut self,
        t: f64,
        clocks: &[(f64, bool, Slot)],
        values: &mut [f64],
    ) -> bool {
        let mut turned = false;
        for (index, &(_, turns_true, slot)) in clocks.iter().enumerate() {
            if self.next_clock[index] <= t + 1e-9 {
                values[slot] = truth(turns_true);
                self.next_clock[index] = f64::INFINITY;
                turned = true;
            }
        }
        turned
    }
}

impl CompiledModel {
    /// Everything scheduled for `t`: the `sample(...)` flags due here
    /// go up, and the relations on `time` whose threshold is `t` turn.
    /// Says whether a relation turned, which changes the right-hand
    /// side the integration was following: a history of past points
    /// kept for a multistep method no longer describes it.
    pub(crate) fn raise_time_events(
        &self,
        t: f64,
        state: &mut EventState,
        values: &mut [f64],
    ) -> bool {
        state.raise_samples(t, &self.samples, &self.sample_slots, values);
        state.turn_clocks(t, &self.clocks, values)
    }

    /// What the initial event's `when` clauses assign, written into
    /// `values` ahead of the block that reads it: every branch that
    /// holds now and has not fired gives its discrete values their
    /// first numbers, and is marked in `fired` so that the event does
    /// not fire it a second time. The caller raises `initial()`. Says
    /// whether any branch fired.
    ///
    /// A block solved before the clauses reads the discrete values at
    /// their template. A `TimeTable` holds `y = a*time + b` with `a`
    /// and `b` written by `when initial()`, so before the clause the
    /// table answers zero, and an orifice dividing by the pressure it
    /// reads refused a model whose equations are fine. The refusal was
    /// about the compiler's order rather than the model, so a block
    /// that refuses there is asked again once the clauses have fired.
    pub(crate) fn fire_initial_assignments(
        &self,
        t: f64,
        values: &mut [f64],
        before: &[Vec<bool>],
        fired: &mut [Vec<bool>],
    ) -> bool {
        let now = self.when_conditions(t, values);
        let mut any = false;
        for (index, clause) in self.when_clauses.iter().enumerate() {
            // The same edge the event fires on: true now, not true
            // before, and not fired already.
            let Some(branch) = (0..clause.branches.len())
                .find(|&b| now[index][b] && !before[index][b] && !fired[index][b])
            else {
                continue;
            };
            // A branch that also checks, stops or restarts something
            // is left to the event, where those are done in their
            // place: fired early, the assignments would go and the
            // rest be lost with the mark.
            let actions = &clause.branches[branch].actions;
            if !actions
                .iter()
                .all(|action| matches!(action, CompiledAction::Assign(..)))
            {
                continue;
            }
            fired[index][branch] = true;
            any = true;
            for action in actions {
                if let CompiledAction::Assign(discrete_index, code) = action {
                    values[self.discrete_slots[*discrete_index]] = code.run(values, t);
                }
            }
        }
        any
    }
}

/// Whether a block refused before the initial event's `when` clauses
/// have fired is refused outright, the order before the clauses were
/// put first. `OXIDELICA_NO_INITIAL_ORDER` keeps the old one, so that
/// one binary gives both numbers.
pub(crate) fn initial_order_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var_os("OXIDELICA_NO_INITIAL_ORDER").is_some())
}

impl EventRewrite<'_> {
    /// `time < C` with `C` known before the run, as the flag that
    /// stands for it.
    ///
    /// Such a relation turns at an instant the compiler already knows,
    /// and watched as a crossing it is found only as well as the solver
    /// can find it: a ramp `der(s) = if time < 1 then 1 else 0` meant
    /// to end on 1 ended on 0.999899 under dopri45 and on 0.916667
    /// under RK4, whose last stage stood on the threshold and read the
    /// far side of the switch for the whole step. As a flag it holds
    /// its value between events - which is what a relation does - and
    /// the run steps exactly onto the threshold and turns it there.
    ///
    /// Only `time` alone against a side that names nothing but
    /// parameters qualifies. A threshold at or before the start is
    /// left to the relation, which has already turned or turns on the
    /// very first instant.
    fn clock_of(&mut self, op: RelOp, l: &Expr, r: &Expr) -> Option<Expr> {
        if !self.clocks_open {
            return None;
        }
        let (op, other) = match (l, r) {
            (Expr::Time, other) => (op, other),
            (other, Expr::Time) => (
                match op {
                    RelOp::Lt => RelOp::Gt,
                    RelOp::Le => RelOp::Ge,
                    RelOp::Gt => RelOp::Lt,
                    RelOp::Ge => RelOp::Le,
                    same @ (RelOp::Eq | RelOp::Ne) => same,
                },
                other,
            ),
            _ => return None,
        };
        let turns_true = match op {
            RelOp::Lt | RelOp::Le => false,
            RelOp::Gt | RelOp::Ge => true,
            RelOp::Eq | RelOp::Ne => return None,
        };
        if names_time(other) {
            return None;
        }
        let threshold = eval(
            other,
            &EvalCtx {
                vars: self.params,
                time: f64::NAN,
                programs: None,
                depth: 0,
            },
        )
        .ok()?;
        if !threshold.is_finite() || threshold <= self.clock_after + 1e-9 {
            return None;
        }
        let index = match self
            .clocks
            .iter()
            .position(|&(at, up)| at == threshold && up == turns_true)
        {
            Some(index) => index,
            None => {
                self.clocks.push((threshold, turns_true));
                self.clocks.len() - 1
            }
        };
        Some(Expr::Ref(format!("$clock{index}")))
    }

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
            Expr::Rel(op, l, r) => match self.clock_of(*op, l, r) {
                Some(flag) => flag,
                None => Expr::Rel(*op, Box::new(self.expr(l)?), Box::new(self.expr(r)?)),
            },
            Expr::And(l, r) => Expr::And(Box::new(self.expr(l)?), Box::new(self.expr(r)?)),
            Expr::Or(l, r) => Expr::Or(Box::new(self.expr(l)?), Box::new(self.expr(r)?)),
            Expr::If(c, a, b) => Expr::If(
                Box::new(self.expr(c)?),
                Box::new(self.expr(a)?),
                Box::new(self.expr(b)?),
            ),
            // `{a, b, c}[2]`: a list written out and read at a number
            // is the element at that number, whatever the list is. A
            // record handed over as its fields and read by one of them
            // arrives this way - the reference air's Helmholtz record,
            // eleven fields, read as `f.tau` - and nothing before the
            // run had taken the element out.
            Expr::Index(base, subscripts)
                if literal_index_open() && matches!(base.as_ref(), Expr::Array(_)) =>
            {
                match picked(base, subscripts) {
                    Some(element) => self.expr(element)?,
                    None => Expr::Index(Box::new(self.expr(base)?), subscripts.clone()),
                }
            }
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
            next_clock: self.clocks.iter().map(|&(at, _, _)| at).collect(),
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
        let mut before_event = state.when_prev.clone();
        // The language's event iteration goes on until every discrete
        // value equals its `pre`: `y = pre(u)` in `Blocks.Logical.Pre`
        // is how a library breaks a loop of switches, and it is meant
        // to take the new `u` within the same instant. Copying `pre`
        // once, when the event began, left `y` at the old value until
        // whatever event came next - in a thyristor bridge, the next
        // output point, a third of a millisecond of firing delay that
        // nothing in the model asked for. So each pass that ends with
        // a discrete value away from its `pre` takes the values as the
        // new `pre` and goes round again; the bound is one pass per
        // discrete value and one more, the same reasoning as the rounds
        // inside a pass.
        // The initial event is the exception: there the start of a
        // discrete-valued name is read as `pre(v) = start` (MLS 8.6),
        // a condition of the initial problem rather than a value the
        // iteration may move, and `y = pre(aux)` must show the start.
        let passes = if pre_iteration_off() || values[self.initial_slot] != 0.0 {
            1
        } else {
            self.pre_slots.len() + 2
        };
        // A `reinit` is collected rather than applied: the event
        // iteration works on the discrete variables, and the new value
        // of a state is what the integration resumes from afterwards.
        let mut pending_reinit: Vec<(usize, f64)> = Vec::new();
        let mut scratch = Vec::new();
        let mut pass = 0;
        loop {
            pass += 1;
            let mut fired: Vec<Vec<bool>> = before_event
                .iter()
                .map(|branches| vec![false; branches.len()])
                .collect();

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
                    // A block refused before the initial event's clauses
                    // have fired may be refused for what they had not yet
                    // written - a table answering zero until `when
                    // initial()` gives it its line. Those clauses are
                    // fired then, and the point is asked once more; a
                    // block that refuses with them fired refuses for
                    // the model's own reasons.
                    if let Err(why) = self.eval_point(t, y, values, &mut scratch, alg_guess) {
                        if initial_order_off()
                            || values[self.initial_slot] == 0.0
                            || !self.fire_initial_assignments(t, values, &before_event, &mut fired)
                        {
                            return Err(why);
                        }
                        acted = true;
                        outcome.changed = true;
                        self.eval_point(t, y, values, &mut scratch, alg_guess)?;
                    }
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
                return Err(SimError::from(format!(
                    "the event at t = {t} does not come to rest after {rounds} round(s): \
                 what changes on every round is among {names:?}"
                )));
            }
            let lagging: Vec<usize> = self
                .pre_slots
                .iter()
                .enumerate()
                .filter(|(_, &(slot, pre))| {
                    values[slot] != values[pre] && !(values[slot].is_nan() && values[pre].is_nan())
                })
                .map(|(at, _)| at)
                .collect();
            if lagging.is_empty() || pass >= passes {
                if !lagging.is_empty() && passes > 1 {
                    let names: Vec<String> = lagging
                        .iter()
                        .filter_map(|&at| self.pre_names.get(at).cloned())
                        .collect();
                    return Err(SimError::from(format!(
                        "the event at t = {t} does not come to rest after {passes} pass(es): \
                     a discrete value still differs from its `pre` among {names:?}"
                    )));
                }
                break;
            }
            for &(slot, pre) in &self.pre_slots {
                values[pre] = values[slot];
            }
            // A condition that held at the end of the last pass is not a
            // fresh edge on the next: a `when` fires on what changed.
            self.eval_point(t, y, values, &mut scratch, alg_guess)?;
            before_event = self.when_conditions(t, values);
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

/// Whether the event iteration stops after one pass even where a
/// discrete value still differs from its `pre`, the behaviour before
/// it went on. `OXIDELICA_NO_PRE_ITERATION` keeps the old one, so that
/// one binary gives both numbers.
fn pre_iteration_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var_os("OXIDELICA_NO_PRE_ITERATION").is_some())
}

/// Whether a list written out and read at a number is taken as the
/// element there. `OXIDELICA_NO_LITERAL_INDEX` leaves it as written, so
/// that one binary gives both numbers.
fn literal_index_open() -> bool {
    std::env::var_os("OXIDELICA_NO_LITERAL_INDEX").is_none()
}

/// The element a list written out is read at, where every subscript is
/// a whole number inside the list. Anything else - a subscript the run
/// decides, one past the end - is `None`, and the list stays for the
/// refusal that names it.
fn picked<'a>(base: &'a Expr, subscripts: &[Expr]) -> Option<&'a Expr> {
    let mut at = base;
    for subscript in subscripts {
        let Expr::Array(items) = at else {
            return None;
        };
        let Expr::Number(index) = subscript else {
            return None;
        };
        if index.fract() != 0.0 || *index < 1.0 {
            return None;
        }
        at = items.get(*index as usize - 1)?;
    }
    Some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A list written out and read at a whole number gives the element
    /// there, a table read at two gives the element of the row, and a
    /// subscript outside the list or not a number gives nothing - so the
    /// list stays for the refusal that names it.
    #[test]
    fn a_list_read_at_a_number_is_its_element() {
        let n = |x: f64| Expr::Number(x);
        let list = Expr::Array(vec![Expr::Ref("a".into()), Expr::Ref("b".into())]);
        assert_eq!(picked(&list, &[n(2.0)]), Some(&Expr::Ref("b".into())));
        assert_eq!(picked(&list, &[n(3.0)]), None);
        assert_eq!(picked(&list, &[n(1.5)]), None);
        assert_eq!(picked(&list, &[Expr::Ref("i".into())]), None);
        let table = Expr::Array(vec![list.clone(), Expr::Array(vec![n(5.0), n(6.0)])]);
        assert_eq!(picked(&table, &[n(2.0), n(1.0)]), Some(&n(5.0)));
    }
}
