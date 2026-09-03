//! State machines: the states a model steps between, the arrows
//! joining them, and what their equations come to.
//!
//! A state machine is a clocked thing - it decides where it is at each
//! tick, and the equations of a state run only while that state is the
//! one it is in - so this was written inside the clock layer and moved
//! out whole when that file grew past reading.

use super::clocks::*;
use super::*;

/// What the states of a machine say about a variable declared outside
/// them: which machine, which of its states, and the value it gives.
type SaidInStates = Vec<(usize, usize, Expr)>;

/// One state machine: its states, its arrows, and where it sits.
#[derive(Clone)]
pub(super) struct Machine {
    /// The states, the one it starts in first.
    states: Vec<String>,
    /// The arrows joining them.
    arrows: Vec<Transition>,
    /// The machine this one is inside, and which of that machine's
    /// states holds it. A machine at the top of the model has none.
    inside: Option<(usize, usize)>,
}

/// Split what the model declared into machines.
///
/// A machine is a set of states joined by arrows, and nothing joins one
/// machine to another - so the arrows say where one ends and the next
/// begins. A machine whose states all live under one state of another
/// is inside it.
fn partition(model: &Model) -> Result<Vec<Machine>, String> {
    // Every state named anywhere, gathered into groups that arrows
    // reach across.
    let mut groups: Vec<Vec<String>> = Vec::new();
    let join = |a: &str, b: &str, groups: &mut Vec<Vec<String>>| {
        let at = |name: &str, groups: &Vec<Vec<String>>| {
            groups
                .iter()
                .position(|group| group.iter().any(|s| s == name))
        };
        match (at(a, groups), at(b, groups)) {
            (Some(one), Some(other)) if one != other => {
                let moved = groups.remove(one.max(other));
                groups[one.min(other)].extend(moved);
            }
            (Some(_), Some(_)) => {}
            (Some(one), None) => groups[one].push(b.to_string()),
            (None, Some(other)) => groups[other].push(a.to_string()),
            (None, None) => groups.push(vec![a.to_string(), b.to_string()]),
        }
    };
    for transition in &model.transitions {
        join(&transition.from, &transition.to, &mut groups);
    }
    // A machine of one state has no arrows to be found by.
    for state in &model.initial_states {
        if !groups.iter().any(|group| group.iter().any(|s| s == state)) {
            groups.push(vec![state.clone()]);
        }
    }

    let mut machines = Vec::new();
    for group in groups {
        let starts: Vec<&String> = model
            .initial_states
            .iter()
            .filter(|state| group.contains(state))
            .collect();
        let start = match starts.as_slice() {
            [one] => (*one).clone(),
            [] => {
                return Err(format!(
                    "the states {group:?} are joined by arrows and none of them is where \
                     the machine starts - one `initialState` to a machine"
                ))
            }
            several => {
                return Err(format!(
                    "the states {group:?} are one machine, and {} of them are named as \
                     where it starts",
                    several.len()
                ))
            }
        };
        // The one it starts in first, the rest in the order the arrows
        // named them.
        let mut states = vec![start];
        for transition in &model.transitions {
            for end in [&transition.from, &transition.to] {
                if group.contains(end) && !states.contains(end) {
                    states.push(end.clone());
                }
            }
        }
        let arrows = model
            .transitions
            .iter()
            .filter(|transition| group.contains(&transition.from))
            .cloned()
            .collect();
        machines.push(Machine {
            states,
            arrows,
            inside: None,
        });
    }

    // Which machine holds which: a machine sits inside the state whose
    // instance path every one of its own states is under, and inside
    // the innermost such state where there are several.
    for index in 0..machines.len() {
        let mut best: Option<(usize, usize, usize)> = None;
        for (outer, holding) in machines.iter().enumerate() {
            if outer == index {
                continue;
            }
            for (at, state) in holding.states.iter().enumerate() {
                let under = format!("{state}.");
                if machines[index]
                    .states
                    .iter()
                    .all(|inner| inner.starts_with(&under))
                    && best.is_none_or(|(_, _, deep)| state.len() > deep)
                {
                    best = Some((outer, at, state.len()));
                }
            }
        }
        machines[index].inside = best.map(|(outer, at, _)| (outer, at));
    }
    // Outermost first, so a machine can be built knowing the one that
    // holds it has been.
    let depth = |machine: &Machine, machines: &Vec<Machine>| {
        let mut deep = 0;
        let mut at = machine.inside;
        while let Some((outer, _)) = at {
            deep += 1;
            at = machines[outer].inside;
        }
        deep
    };
    let order: Vec<usize> = {
        let mut order: Vec<usize> = (0..machines.len()).collect();
        order.sort_by_key(|index| depth(&machines[*index], &machines));
        order
    };
    let mut sorted: Vec<Machine> = Vec::new();
    for index in &order {
        let mut machine = machines[*index].clone();
        machine.inside = machine
            .inside
            .map(|(outer, at)| (order.iter().position(|o| o == &outer).expect("kept"), at));
        sorted.push(machine);
    }
    Ok(sorted)
}

/// Turn the arrows of a state machine into equations on its clock.
///
/// A machine keeps one variable of its own, the state it is in. At
/// each tick it looks at the arrows leaving that state, in priority
/// order, and takes the first whose condition holds - judged on the
/// values from the tick before, since this tick's are decided by where
/// the machine goes. The states' own equations then run guarded: the
/// one that is in force computes, and the others hold what they had.
pub(super) fn build_state_machines(
    model: &mut Model,
    clocks: &Clocks,
    clock_of: &mut HashMap<String, usize>,
) -> Result<(), String> {
    if model.transitions.is_empty() && model.initial_states.is_empty() {
        return Ok(());
    }
    let Some(clock) = clocks.only_named() else {
        return Err(format!(
            "a state machine runs on a clock, and this model declares {} of them",
            clocks.count()
        ));
    };
    // `timeInState` is the tick count times the period, and an event
    // clock has no period to multiply by: how long a state has been
    // held is then a question only the run can answer.
    let period = clocks.spec(clock).interval();
    if period.is_none()
        && model
            .equations
            .iter()
            .any(|equation| mentions_call(&equation.rhs, "timeInState"))
    {
        return Err(
            "`timeInState` counts periods, and this machine's clock ticks on an event \
             rather than on a period - `ticksInState` is what it can answer"
                .to_string(),
        );
    }
    // Never read where there is none: the check above saw to that.
    let period = period.unwrap_or(0.0);
    let machines = partition(model)?;

    // A state is an instance with equations of its own; a plain
    // variable cannot be one.
    for machine in &machines {
        for state in &machine.states {
            let under = format!("{state}.");
            if !model
                .components
                .iter()
                .any(|component| component.name.starts_with(&under))
            {
                return Err(format!(
                    "`{state}` is named as a state but is not a component with anything in it"
                ));
            }
        }
        // Two arrows out of one state must say which goes first, and
        // the specification asks that they never say the same thing.
        for (index, one) in machine.arrows.iter().enumerate() {
            for other in machine.arrows.iter().skip(index + 1) {
                if one.from == other.from && one.priority == other.priority {
                    return Err(format!(
                        "two arrows leave `{}` with priority {}, and the one to take is then \
                         nobody's decision",
                        one.from, one.priority
                    ));
                }
            }
        }
    }

    // Which machine and which of its states every variable belongs to,
    // by the instance path it was flattened under - the innermost state
    // that holds it, since a state inside a state holds both paths. A
    // parameter is not one of these: it does not change, so it has no
    // value from before to reach back to.
    let varying: Vec<String> = model
        .components
        .iter()
        .filter(|component| {
            !matches!(
                component.variability,
                Variability::Parameter | Variability::Constant
            )
        })
        .map(|component| component.name.clone())
        .collect();
    let owner = |name: &str| -> Option<(usize, usize)> {
        if !varying.iter().any(|known| known == name) {
            return None;
        }
        let mut best: Option<(usize, usize, usize)> = None;
        for (tag, machine) in machines.iter().enumerate() {
            for (at, state) in machine.states.iter().enumerate() {
                if name.starts_with(&format!("{state}."))
                    && best.is_none_or(|(_, _, deep)| state.len() > deep)
                {
                    best = Some((tag, at, state.len()));
                }
            }
        }
        best.map(|(tag, at, _)| (tag, at))
    };
    let in_a_state = |name: &str| owner(name).map(|(_, at)| at);

    let state_var = |tag: usize| format!("$state{tag}");
    let ticks_var = |tag: usize| format!("$ticks{tag}");
    let reset_var = |tag: usize| format!("$reset{tag}");
    let previous_of = |name: &str| Expr::Call("previous".to_string(), vec![Expr::Ref(name.into())]);
    let is = |name: &str, index: usize| {
        Expr::Rel(
            crate::ast::RelOp::Eq,
            Box::new(Expr::Ref(name.to_string())),
            Box::new(Expr::Number(index as f64)),
        )
    };
    let was = |name: &str, index: usize| {
        Expr::Rel(
            crate::ast::RelOp::Eq,
            Box::new(previous_of(name)),
            Box::new(Expr::Number(index as f64)),
        )
    };

    // A machine sitting inside a state runs only while that state is in
    // force, and starts over where the arrow that reached it asked.
    // This is 17.3.3's `active` input, and it is the whole of what makes
    // a machine hierarchical.
    let held_by = |machine: &Machine| -> Option<(Expr, Expr)> {
        let (outer, at) = machine.inside?;
        let alive = is(&state_var(outer), at);
        let arrived = Expr::And(
            Box::new(Expr::Not(Box::new(was(&state_var(outer), at)))),
            Box::new(Expr::Rel(
                crate::ast::RelOp::Gt,
                Box::new(Expr::Ref(reset_var(outer))),
                Box::new(Expr::Number(0.5)),
            )),
        );
        Some((alive.clone(), Expr::And(Box::new(alive), Box::new(arrived))))
    };

    // The states a machine may stop in: those no arrow leaves. A
    // `synchronize` arrow waits for every machine inside the state it
    // leaves to have reached one - and asks about the tick before, the
    // way every other condition here does. Asking about this one would
    // be asking the machine inside about a tick whose answer waits on
    // the machine outside, which waits on it.
    let settled = |tag: usize| -> Option<Expr> {
        let machine = &machines[tag];
        let mut resting: Option<Expr> = None;
        for (at, state) in machine.states.iter().enumerate() {
            if machine.arrows.iter().any(|arrow| &arrow.from == state) {
                continue;
            }
            let here = was(&state_var(tag), at);
            resting = Some(match resting {
                Some(so_far) => Expr::Or(Box::new(so_far), Box::new(here)),
                None => here,
            });
        }
        resting
    };

    let mut bookkeeping: Vec<(String, &str, Expr)> = Vec::new();
    let mut machine_equations: Vec<(String, Expr)> = Vec::new();
    for (tag, machine) in machines.iter().enumerate() {
        let (active, ticks, resetting) = (state_var(tag), ticks_var(tag), reset_var(tag));
        let index_of = |state: &str| {
            machine
                .states
                .iter()
                .position(|candidate| candidate == state)
                .expect("the states were gathered from the arrows") as f64
        };

        let mut arrows: Vec<(usize, &Transition)> = machine.arrows.iter().enumerate().collect();
        arrows.sort_by_key(|(_, transition)| (transition.priority, transition.from.clone()));
        // An arrow is ready when the machine is where it leaves from and
        // its condition holds. An immediate arrow is taken on that at
        // once; a delayed one keeps the answer for a tick and is taken
        // on what it kept, which is the whole of `immediate = false`.
        let mut armed: Vec<(String, Expr)> = Vec::new();
        let mut guards: Vec<(Expr, &Transition)> = Vec::new();
        for (index, transition) in &arrows {
            let mut ready = Expr::And(
                Box::new(was(&active, index_of(&transition.from) as usize)),
                Box::new(look_back(&transition.condition, &in_a_state)),
            );
            // A `synchronize` arrow waits for the machines inside the
            // state it leaves to have finished.
            if transition.synchronize {
                let under = format!("{}.", transition.from);
                let mut inside = machines
                    .iter()
                    .enumerate()
                    .filter(|(_, held)| held.states.iter().all(|s| s.starts_with(&under)))
                    .filter_map(|(held, _)| settled(held));
                let Some(first) = inside.next() else {
                    return Err(format!(
                        "the arrow leaving `{}` waits for the machines inside it to finish, \
                         and there are none there to wait for",
                        transition.from
                    ));
                };
                let finished = inside.fold(first, |so_far, one| {
                    Expr::And(Box::new(so_far), Box::new(one))
                });
                ready = Expr::And(Box::new(ready), Box::new(finished));
            }
            let guard = if transition.immediate {
                ready
            } else {
                let kept = format!("$arm{tag}_{index}");
                armed.push((kept.clone(), ready));
                previous_of(&kept)
            };
            guards.push((guard, transition));
        }
        let mut next = previous_of(&active);
        let mut resets_now = Expr::Number(0.0);
        for (guard, transition) in guards.iter().rev() {
            next = Expr::If(
                Box::new(guard.clone()),
                Box::new(Expr::Number(index_of(&transition.to))),
                Box::new(next),
            );
            // Only the arrow taken decides whether what it arrives at is
            // put back to its start values.
            resets_now = Expr::If(
                Box::new(guard.clone()),
                Box::new(Expr::Number(if transition.reset { 1.0 } else { 0.0 })),
                Box::new(resets_now),
            );
        }

        let nowhere = -1.0;
        let first_tick = Expr::Rel(
            crate::ast::RelOp::Lt,
            Box::new(previous_of(&active)),
            Box::new(Expr::Number(0.0)),
        );
        // Before the first tick the machine is nowhere, so the first
        // tick is an arrival at the state it starts in like any other -
        // which is what makes that state's variables start from their
        // start values.
        let mut next = Expr::If(
            Box::new(first_tick.clone()),
            Box::new(Expr::Number(0.0)),
            Box::new(next),
        );
        let mut resets_now = Expr::If(
            Box::new(first_tick),
            Box::new(Expr::Number(1.0)),
            Box::new(resets_now),
        );
        let mut ticks_now = Expr::If(
            Box::new(Expr::Rel(
                crate::ast::RelOp::Eq,
                Box::new(Expr::Ref(active.clone())),
                Box::new(previous_of(&active)),
            )),
            Box::new(Expr::Bin(
                BinOp::Add,
                Box::new(previous_of(&ticks)),
                Box::new(Expr::Number(1.0)),
            )),
            Box::new(Expr::Number(0.0)),
        );
        // A machine inside a state holds still while that state is not
        // the one in force, and starts over where it was entered afresh.
        if let Some((alive, restart)) = held_by(machine) {
            next = Expr::If(
                Box::new(restart),
                Box::new(Expr::Number(0.0)),
                Box::new(Expr::If(
                    Box::new(alive.clone()),
                    Box::new(next),
                    Box::new(previous_of(&active)),
                )),
            );
            resets_now = Expr::If(
                Box::new(alive.clone()),
                Box::new(resets_now),
                Box::new(Expr::Number(0.0)),
            );
            ticks_now = Expr::If(
                Box::new(alive.clone()),
                Box::new(ticks_now),
                Box::new(previous_of(&ticks)),
            );
            for (_, value) in &mut armed {
                *value = Expr::And(Box::new(alive.clone()), Box::new(value.clone()));
            }
        }

        machine_equations.push((active.clone(), next));
        machine_equations.push((resetting.clone(), resets_now));
        machine_equations.push((ticks.clone(), ticks_now));
        machine_equations.extend(armed.iter().cloned());
        bookkeeping.push((active, "Real", Expr::Number(nowhere)));
        bookkeeping.push((ticks, "Real", Expr::Number(0.0)));
        bookkeeping.push((resetting, "Real", Expr::Number(0.0)));
        // What a delayed arrow keeps for a tick is a truth, not a number.
        bookkeeping.extend(
            armed
                .iter()
                .map(|(name, _)| (name.clone(), "Boolean", Expr::Bool(false))),
        );
    }
    for (name, of, start) in bookkeeping {
        model.components.push(Component {
            name: name.clone(),
            type_name: of.to_string(),
            variability: Variability::Discrete,
            start: Some(start),
            description: Some("state machine bookkeeping".to_string()),
            ..blank_component()
        });
        clock_of.insert(name, clock);
    }

    // The states' equations, guarded by the state being in force.
    let starts: HashMap<String, Expr> = model
        .components
        .iter()
        .filter_map(|component| Some((component.name.clone(), component.start.clone()?)))
        .collect();
    // Which state an equation was written inside, for one whose target
    // lives outside every state: `outer output v` written by several of
    // them is one definition of `v`, merged here.
    let written_in = |origin: &str| -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize, usize)> = None;
        for (tag, machine) in machines.iter().enumerate() {
            for (at, state) in machine.states.iter().enumerate() {
                if (origin == state || origin.starts_with(&format!("{state}.")))
                    && best.is_none_or(|(_, _, deep)| state.len() > deep)
                {
                    best = Some((tag, at, state.len()));
                }
            }
        }
        best.map(|(tag, at, _)| (tag, at))
    };
    let mut shared: Vec<(String, SaidInStates)> = Vec::new();
    let mut kept = Vec::new();
    for equation in model.equations.drain(..) {
        let Expr::Ref(target) = &equation.lhs else {
            kept.push(equation);
            continue;
        };
        let Some((tag, state)) = owner(target) else {
            // Not one of a state's own variables. Written inside one,
            // it is that state's say in what the variable holds, and
            // the says are merged into one definition below.
            match written_in(&equation.origin) {
                Some((tag, at)) => {
                    let name = target.clone();
                    let rhs = equation.rhs.clone();
                    match shared.iter_mut().find(|(known, _)| known == &name) {
                        Some((_, says)) => says.push((tag, at, rhs)),
                        None => shared.push((name, vec![(tag, at, rhs)])),
                    }
                }
                None => kept.push(equation),
            }
            continue;
        };
        let (active, resetting) = (state_var(tag), reset_var(tag));
        let in_force = is(&active, state);
        let holding = previous_of(target);
        let mut value = Expr::If(
            Box::new(in_force.clone()),
            Box::new(equation.rhs),
            Box::new(holding),
        );
        // Arriving here puts this state's variables back to their start
        // values where the arrow taken asked for it - the arrow, not
        // the state: one leading here may ask and another not.
        let entered = Expr::And(
            Box::new(in_force),
            Box::new(Expr::Not(Box::new(was(&active, state)))),
        );
        let asked = Expr::And(
            Box::new(entered),
            Box::new(Expr::Rel(
                crate::ast::RelOp::Gt,
                Box::new(Expr::Ref(resetting)),
                Box::new(Expr::Number(0.5)),
            )),
        );
        let back_to = starts.get(target).cloned().unwrap_or(Expr::Number(0.0));
        value = Expr::If(Box::new(asked), Box::new(back_to), Box::new(value));
        clock_of.insert(target.clone(), clock);
        kept.push(EquationItem {
            lhs: equation.lhs,
            rhs: value,
            origin: String::new(),
        });
    }
    model.equations = kept;
    // What several states say about one variable is one definition of
    // it: whichever state is in force has its say, and where none does
    // the variable keeps what it held. That is 17.3.5, with `last` here
    // being simply the value from the tick before.
    for (target, says) in shared {
        if model
            .equations
            .iter()
            .any(|equation| equation.lhs == Expr::Ref(target.clone()))
        {
            return Err(format!(
                "`{target}` is written both inside a state and outside every state, and \
                 a variable has one definition"
            ));
        }
        let mut value = previous_of(&target);
        for (tag, at, rhs) in says.iter().rev() {
            value = Expr::If(
                Box::new(is(&state_var(*tag), *at)),
                Box::new(rhs.clone()),
                Box::new(value),
            );
        }
        clock_of.insert(target.clone(), clock);
        model.equations.push(EquationItem {
            lhs: Expr::Ref(target),
            rhs: value,
            origin: String::new(),
        });
    }

    // `activeState`, `ticksInState` and `timeInState` say what they
    // mean once the machines have variables to say it with. Which
    // machine a question is about is the one whose state it was asked
    // inside; asked outside every state, only a model with one machine
    // can answer.
    let named: HashMap<String, (usize, usize)> = machines
        .iter()
        .enumerate()
        .flat_map(|(tag, machine)| {
            machine
                .states
                .iter()
                .enumerate()
                .map(move |(at, state)| (state.clone(), (tag, at)))
        })
        .collect();
    let mut asked_outside = false;
    for equation in &mut model.equations {
        let here = match &equation.lhs {
            Expr::Ref(target) => owner(target).map(|(tag, _)| tag),
            _ => None,
        };
        if here.is_none()
            && machines.len() > 1
            && (mentions_call(&equation.rhs, "ticksInState")
                || mentions_call(&equation.rhs, "timeInState"))
        {
            asked_outside = true;
        }
        let tag = here.unwrap_or(0);
        equation.rhs = machine_queries(&equation.rhs, &named, &state_var, &ticks_var(tag), period);
    }
    if asked_outside {
        return Err(
            "`ticksInState` and `timeInState` are about the machine they are asked inside, \
             and this model has more than one - ask them among a state's own equations"
                .to_string(),
        );
    }
    for (target, value) in machine_equations {
        let tag = owner(&target).map(|(tag, _)| tag).unwrap_or(0);
        let value = machine_queries(&value, &named, &state_var, &ticks_var(tag), period);
        model.equations.push(EquationItem {
            lhs: Expr::Ref(target),
            rhs: value,
            origin: String::new(),
        });
    }
    model.transitions.clear();
    model.initial_states.clear();
    Ok(())
}

/// Wrap the state machine's own variables in `previous`, so a
/// condition is judged on the values from the tick before.
pub(super) fn look_back(expr: &Expr, owner: &impl Fn(&str) -> Option<usize>) -> Expr {
    let recur = |e: &Expr| look_back(e, owner);
    match expr {
        Expr::Ref(name) if owner(name).is_some() => {
            Expr::Call("previous".to_string(), vec![expr.clone()])
        }
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::If(c, a, b) => Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b))),
        _ => expr.clone(),
    }
}

/// Answer what a model asks about the machine it declared.
pub(super) fn machine_queries(
    expr: &Expr,
    named: &HashMap<String, (usize, usize)>,
    state_var: &impl Fn(usize) -> String,
    ticks: &str,
    period: f64,
) -> Expr {
    let recur = |e: &Expr| machine_queries(e, named, state_var, ticks, period);
    match expr {
        Expr::Call(name, args) if name == "activeState" && args.len() == 1 => {
            let wanted = match &args[0] {
                Expr::Ref(state) => named.get(state),
                _ => None,
            };
            match wanted {
                Some((tag, at)) => Expr::Rel(
                    crate::ast::RelOp::Eq,
                    Box::new(Expr::Ref(state_var(*tag))),
                    Box::new(Expr::Number(*at as f64)),
                ),
                None => Expr::Call(name.clone(), args.iter().map(recur).collect()),
            }
        }
        Expr::Call(name, args) if name == "ticksInState" && args.is_empty() => {
            Expr::Ref(ticks.to_string())
        }
        Expr::Call(name, args) if name == "timeInState" && args.is_empty() => Expr::Bin(
            BinOp::Mul,
            Box::new(Expr::Ref(ticks.to_string())),
            Box::new(Expr::Number(period)),
        ),
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::If(c, a, b) => Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b))),
        _ => expr.clone(),
    }
}

/// A component with nothing said about it, for the ones the compiler
/// makes up for its own bookkeeping.
pub(super) fn blank_component() -> Component {
    Component {
        name: String::new(),
        type_name: "Real".to_string(),
        flow: false,
        stream: false,
        dimensions: Vec::new(),
        causality: Causality::None,
        modifiers: Vec::new(),
        variability: Variability::Continuous,
        start: None,
        fixed: None,
        unit: None,
        min: None,
        max: None,
        binding: None,
        description: None,
        scope: Scope::Local,
        replaceable: false,
        constrained_by: None,
        condition: None,
        redeclares: Vec::new(),
        redeclaration: false,
        is_final: false,
        protected: false,
        each_modifiers: Vec::new(),
        annotations: Vec::new(),
    }
}
