//! Clocked partitions and the state machines that run on them.

use super::*;

/// Split a model into its clocked partitions.
///
/// A clock is not a value the run carries: `Clock c = Clock(0.1)` says
/// when things happen, and the equations that happen then are lifted
/// into a `when` clause firing on that period. Inside one, the clock
/// conversions say what they always meant - `sample(u, c)` is reading
/// `u` at the tick, `previous(x)` is the value from the tick before,
/// `interval(c)` is the period - and the variables they define hold
/// their values in between, which is what `hold` asks for.
///
/// A model with no clocks in it passes through untouched.
pub(super) fn partition_clocks(model: &mut Model) -> Result<(), String> {
    let declared: Vec<String> = model
        .components
        .iter()
        .filter(|component| component.type_name == "Clock")
        .map(|component| component.name.clone())
        .collect();
    if declared.is_empty() {
        // A machine with no clock to run on still has to hear about
        // it, so it is asked before this pass gives up.
        return build_state_machines(model, &HashMap::new(), &mut HashMap::new());
    }
    // A clock says its interval either in its declaration or in an
    // equation of its own; a binding on a variable becomes the latter.
    let parameters: HashMap<String, f64> = model
        .components
        .iter()
        .filter_map(|component| {
            let value = component.binding.as_ref()?;
            const_eval(value, &HashMap::new()).map(|number| (component.name.clone(), number))
        })
        .collect();
    let interval_of = |name: &str, expr: &Expr| -> Option<f64> {
        match expr {
            Expr::Call(called, args) if called == "Clock" && args.len() == 1 => {
                let _ = name;
                const_eval(&args[0], &parameters)
            }
            _ => None,
        }
    };
    let mut clocks: HashMap<String, f64> = HashMap::new();
    for component in &model.components {
        if component.type_name != "Clock" {
            continue;
        }
        if let Some(binding) = &component.binding {
            if let Some(period) = interval_of(&component.name, binding) {
                clocks.insert(component.name.clone(), period);
            }
        }
    }
    model.equations.retain(|equation| {
        let Expr::Ref(target) = &equation.lhs else {
            return true;
        };
        if !declared.contains(target) {
            return true;
        }
        match interval_of(target, &equation.rhs) {
            Some(period) => {
                clocks.insert(target.clone(), period);
                false
            }
            None => true,
        }
    });
    for name in &declared {
        match clocks.get(name) {
            None => {
                return Err(format!(
                    "`{name}` is a Clock, so it needs an interval the compiler can see: \
                     `Clock {name} = Clock(0.1);`"
                ))
            }
            Some(period) if *period <= 0.0 => {
                return Err(format!("the interval of `{name}` must be positive"))
            }
            Some(_) => {}
        }
    }

    // Which variable belongs to which clock. A `sample(u, c)` puts the
    // equation it sits in on `c`, and from there it spreads to
    // whatever those variables define.
    let mut clock_of: HashMap<String, String> = HashMap::new();
    // A state machine is a clocked thing: it decides where it is at
    // each tick, and the equations of its states run only while their
    // state is the one it is in.
    build_state_machines(model, &clocks, &mut clock_of)?;
    for _ in 0..MAX_DEPTH {
        let mut settled = true;
        for equation in &model.equations {
            let Expr::Ref(target) = &equation.lhs else {
                continue;
            };
            if clock_of.contains_key(target) {
                continue;
            }
            if let Some(clock) = clock_touched(&equation.rhs, &clocks, &clock_of) {
                clock_of.insert(target.clone(), clock);
                settled = false;
            }
        }
        if settled {
            break;
        }
    }

    // A `previous` with no clock to hang on has nowhere to take its
    // value from.
    for equation in &model.equations {
        if let Expr::Ref(target) = &equation.lhs {
            if clock_of.contains_key(target) {
                continue;
            }
            if mentions_call(&equation.rhs, "previous") {
                return Err(format!(
                    "`{target}` uses `previous`, but nothing says which clock it is on"
                ));
            }
        }
    }

    // Lift the clocked equations into one `when` per clock, in the
    // order the clocks were declared so the result is settled.
    let mut names: Vec<&String> = clocks.keys().collect();
    names.sort();
    let mut kept = Vec::new();
    let mut lifted: HashMap<&String, Vec<(String, Expr)>> = HashMap::new();
    for equation in model.equations.drain(..) {
        let clock = match &equation.lhs {
            Expr::Ref(target) => clock_of.get(target).cloned(),
            _ => None,
        };
        match (clock, &equation.lhs) {
            (Some(clock), Expr::Ref(target)) => {
                let name = names
                    .iter()
                    .find(|candidate| ***candidate == clock)
                    .expect("the clock was found by name");
                lifted
                    .entry(name)
                    .or_default()
                    .push((target.clone(), at_the_tick(&equation.rhs, &clocks)));
            }
            _ => kept.push(equation),
        }
    }
    model.equations = kept;
    for name in &names {
        let Some(actions) = lifted.remove(name) else {
            continue;
        };
        // The equations of a partition are equations, in no order of
        // their own; what the tick needs is an order in which each is
        // ready when its turn comes. `previous` reaches back to the
        // tick before, so it is not a reason to wait.
        let actions = in_dependency_order(actions)?;
        model.when_clauses.push(WhenClause {
            branches: vec![WhenBranch {
                condition: Expr::Call(
                    "sample".to_string(),
                    vec![Expr::Number(0.0), Expr::Number(clocks[*name])],
                ),
                actions,
            }],
        });
    }

    // What is left of the continuous part may only reach a clocked
    // variable through `hold`, which the rewrite above has already
    // turned into the variable itself - so anything still naming one
    // here was written without it.
    for equation in &model.equations {
        for side in [&equation.lhs, &equation.rhs] {
            if let Some(clocked) = clocked_outside_hold(side, &clock_of) {
                return Err(format!(
                    "`{clocked}` is a clocked variable, so a continuous equation may only \
                     read it through `hold({clocked})`"
                ));
            }
        }
    }
    // With that settled, `hold` has nothing left to say: a clocked
    // variable holds its value between ticks by itself.
    for equation in &mut model.equations {
        equation.lhs = at_the_tick(&equation.lhs, &clocks);
        equation.rhs = at_the_tick(&equation.rhs, &clocks);
    }

    // The clocked variables keep their values between ticks, and the
    // clocks themselves are not variables at all.
    for component in &mut model.components {
        if clock_of.contains_key(&component.name) {
            component.variability = Variability::Discrete;
            if component.start.is_none() {
                component.start = Some(Expr::Number(0.0));
            }
        }
    }
    model
        .components
        .retain(|component| component.type_name != "Clock");
    Ok(())
}

/// The clock an expression belongs to, if it names one.
pub(super) fn clock_touched(
    expr: &Expr,
    clocks: &HashMap<String, f64>,
    clock_of: &HashMap<String, String>,
) -> Option<String> {
    match expr {
        Expr::Call(name, args) if name == "hold" => {
            // `hold` is where a clock stops: what comes out of it is
            // continuous however clocked its argument was.
            let _ = args;
            None
        }
        // `sample(u, c)` and `interval(c)` name their clock outright;
        // the value being sampled may carry one of its own.
        Expr::Call(name, args) if name == "sample" || name == "interval" => {
            args.iter().find_map(|arg| match arg {
                Expr::Ref(clock) if clocks.contains_key(clock) => Some(clock.clone()),
                _ => clock_touched(arg, clocks, clock_of),
            })
        }
        Expr::Ref(name) => clock_of.get(name).cloned(),
        Expr::Call(_, args) => args
            .iter()
            .find_map(|arg| clock_touched(arg, clocks, clock_of)),
        Expr::Neg(inner) | Expr::Not(inner) => clock_touched(inner, clocks, clock_of),
        Expr::Bin(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r)
        | Expr::Elementwise(_, l, r) => {
            clock_touched(l, clocks, clock_of).or_else(|| clock_touched(r, clocks, clock_of))
        }
        Expr::If(c, a, b) => clock_touched(c, clocks, clock_of)
            .or_else(|| clock_touched(a, clocks, clock_of))
            .or_else(|| clock_touched(b, clocks, clock_of)),
        _ => None,
    }
}

/// What a clocked expression says once the tick has arrived.
pub(super) fn at_the_tick(expr: &Expr, clocks: &HashMap<String, f64>) -> Expr {
    let recur = |e: &Expr| at_the_tick(e, clocks);
    match expr {
        // Sampling is reading, at the instant of the tick.
        Expr::Call(name, args)
            if name == "sample" && args.len() == 2 && names_a_clock(&args[1], clocks) =>
        {
            recur(&args[0])
        }
        // The value from the tick before is the value from before this
        // event, which is what `pre` has always meant here.
        Expr::Call(name, args) if name == "previous" && args.len() == 1 => {
            Expr::Call("pre".to_string(), vec![recur(&args[0])])
        }
        Expr::Call(name, args)
            if name == "interval" && args.len() == 1 && names_a_clock(&args[0], clocks) =>
        {
            let Expr::Ref(clock) = &args[0] else {
                unreachable!("the guard just checked it")
            };
            Expr::Number(clocks[clock])
        }
        // `hold` asks for the value a clocked variable keeps between
        // ticks, which is the variable.
        Expr::Call(name, args) if name == "hold" && args.len() == 1 => recur(&args[0]),
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Elementwise(op, l, r) => {
            Expr::Elementwise(*op, Box::new(recur(l)), Box::new(recur(r)))
        }
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::If(c, a, b) => Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b))),
        _ => expr.clone(),
    }
}

/// Whether an argument is the name of a declared clock.
pub(super) fn names_a_clock(expr: &Expr, clocks: &HashMap<String, f64>) -> bool {
    matches!(expr, Expr::Ref(name) if clocks.contains_key(name))
}

/// A clocked variable read by a continuous equation without asking for
/// the value it holds - which is the one thing that is not allowed.
pub(super) fn clocked_outside_hold(
    expr: &Expr,
    clock_of: &HashMap<String, String>,
) -> Option<String> {
    let recur = |e: &Expr| clocked_outside_hold(e, clock_of);
    match expr {
        // Inside `hold` is exactly where a clocked variable may be.
        Expr::Call(name, _) if name == "hold" => None,
        Expr::Ref(name) => clock_of.contains_key(name).then(|| name.clone()),
        Expr::Call(_, args) => args.iter().find_map(recur),
        Expr::Neg(inner) | Expr::Not(inner) => recur(inner),
        Expr::Bin(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r)
        | Expr::Elementwise(_, l, r) => recur(l).or_else(|| recur(r)),
        Expr::If(c, a, b) => recur(c).or_else(|| recur(a)).or_else(|| recur(b)),
        _ => None,
    }
}

/// Whether a call by this name appears anywhere in an expression.
pub(super) fn mentions_call(expr: &Expr, wanted: &str) -> bool {
    let mut found = false;
    walk_calls(expr, &mut |name| {
        if name == wanted {
            found = true;
        }
    });
    found
}

/// Visit the name of every call in an expression.
pub(super) fn walk_calls(expr: &Expr, seen: &mut impl FnMut(&str)) {
    match expr {
        Expr::Call(name, args) => {
            seen(name);
            for arg in args {
                walk_calls(arg, seen);
            }
        }
        Expr::Neg(inner) | Expr::Not(inner) => walk_calls(inner, seen),
        Expr::Bin(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r)
        | Expr::Elementwise(_, l, r) => {
            walk_calls(l, seen);
            walk_calls(r, seen);
        }
        Expr::If(c, a, b) => {
            walk_calls(c, seen);
            walk_calls(a, seen);
            walk_calls(b, seen);
        }
        _ => {}
    }
}

/// Put the assignments of one tick in an order where each is ready
/// when its turn comes.
pub(super) fn in_dependency_order(actions: Vec<(String, Expr)>) -> Result<Vec<WhenAction>, String> {
    let targets: Vec<String> = actions.iter().map(|(target, _)| target.clone()).collect();
    let mut placed = vec![false; actions.len()];
    let mut order = Vec::new();
    for _ in 0..actions.len() {
        let next = (0..actions.len()).find(|&index| {
            if placed[index] {
                return false;
            }
            // What the value reads, other than through `previous`.
            let mut named = Vec::new();
            collect_immediate_refs(&actions[index].1, &mut named);
            !named.iter().any(|name| {
                targets
                    .iter()
                    .enumerate()
                    .any(|(other, target)| other != index && !placed[other] && target == name)
            })
        });
        match next {
            Some(index) => {
                placed[index] = true;
                order.push(index);
            }
            None => {
                let stuck: Vec<&str> = (0..actions.len())
                    .filter(|index| !placed[*index])
                    .map(|index| targets[index].as_str())
                    .collect();
                return Err(format!(
                    "the equations on one clock depend on each other in a circle: {stuck:?}"
                ));
            }
        }
    }
    let mut actions: Vec<Option<(String, Expr)>> = actions.into_iter().map(Some).collect();
    Ok(order
        .into_iter()
        .map(|index| {
            let (target, value) = actions[index].take().expect("each placed once");
            WhenAction::Assign(target, value)
        })
        .collect())
}

/// The names an expression reads at this tick: what sits inside
/// `previous` came from the tick before and does not count.
pub(super) fn collect_immediate_refs<'a>(expr: &'a Expr, out: &mut Vec<&'a str>) {
    match expr {
        Expr::Call(name, _) if name == "pre" || name == "previous" => {}
        Expr::Ref(name) => out.push(name),
        Expr::Call(_, args) => args.iter().for_each(|arg| collect_immediate_refs(arg, out)),
        Expr::Neg(inner) | Expr::Not(inner) => collect_immediate_refs(inner, out),
        Expr::Bin(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r)
        | Expr::Elementwise(_, l, r) => {
            collect_immediate_refs(l, out);
            collect_immediate_refs(r, out);
        }
        Expr::If(c, a, b) => {
            collect_immediate_refs(c, out);
            collect_immediate_refs(a, out);
            collect_immediate_refs(b, out);
        }
        _ => {}
    }
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
    clocks: &HashMap<String, f64>,
    clock_of: &mut HashMap<String, String>,
) -> Result<(), String> {
    if model.transitions.is_empty() && model.initial_states.is_empty() {
        return Ok(());
    }
    let (clock, period) = match clocks.len() {
        1 => {
            let name = clocks.keys().next().expect("just counted");
            (name.clone(), clocks[name])
        }
        found => {
            return Err(format!(
                "a state machine runs on a clock, and this model declares {found} of them"
            ))
        }
    };
    let Some(start) = model.initial_states.first().cloned() else {
        return Err("a state machine needs `initialState(...)` to say where it starts".to_string());
    };
    if model.initial_states.len() > 1 {
        return Err("only one state machine to a model, for now".to_string());
    }

    // The states, numbered: the one it starts in first, the rest in
    // the order the arrows name them.
    let mut states = vec![start.clone()];
    for transition in &model.transitions {
        for end in [&transition.from, &transition.to] {
            if !states.contains(end) {
                states.push(end.clone());
            }
        }
    }
    let number = |state: &str| {
        states
            .iter()
            .position(|candidate| candidate == state)
            .map(|index| index as f64)
    };
    let index_of = |state: &str| number(state).expect("the states were gathered from the arrows");

    // A state is an instance with equations of its own; a plain
    // variable cannot be one.
    for state in &states {
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

    // Which state each variable of the model belongs to, by the
    // instance path it was flattened under. A parameter of a state is
    // not one of these: it does not change, so it has no value from
    // before to reach back to.
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
    let owner = |name: &str| -> Option<usize> {
        if !varying.iter().any(|known| known == name) {
            return None;
        }
        states
            .iter()
            .position(|state| name.starts_with(&format!("{state}.")))
    };

    let active = "$state".to_string();
    let ticks = "$ticks".to_string();
    let previous_of = |name: &str| Expr::Call("previous".to_string(), vec![Expr::Ref(name.into())]);

    // Where the machine goes next, arrows in priority order.
    let mut arrows: Vec<&Transition> = model.transitions.iter().collect();
    arrows.sort_by_key(|transition| (transition.priority, transition.from.clone()));
    // Before the first tick the machine is nowhere, so the first tick
    // is an arrival at the initial state like any other - which is
    // what makes its variables start from their start values.
    let mut next = previous_of(&active);
    for transition in arrows.iter().rev() {
        let (from, to) = (index_of(&transition.from), index_of(&transition.to));
        // The condition is judged on the values from the tick before:
        // this tick's belong to whichever state the machine settles
        // on, which is what is being decided.
        let condition = Expr::And(
            Box::new(Expr::Rel(
                crate::ast::RelOp::Eq,
                Box::new(previous_of(&active)),
                Box::new(Expr::Number(from)),
            )),
            Box::new(look_back(&transition.condition, &owner)),
        );
        next = Expr::If(
            Box::new(condition),
            Box::new(Expr::Number(to)),
            Box::new(next),
        );
    }

    // The machine's own variables, and the arrival counter behind
    // `ticksInState` and `timeInState`.
    let nowhere = -1.0;
    let next = Expr::If(
        Box::new(Expr::Rel(
            crate::ast::RelOp::Lt,
            Box::new(previous_of(&active)),
            Box::new(Expr::Number(0.0)),
        )),
        Box::new(Expr::Number(number(&start).unwrap_or(0.0))),
        Box::new(next),
    );
    let mut machine = vec![
        (active.clone(), next),
        (
            ticks.clone(),
            Expr::If(
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
            ),
        ),
    ];
    for (name, start) in [(&active, nowhere), (&ticks, 0.0)] {
        model.components.push(Component {
            name: name.clone(),
            type_name: "Real".to_string(),
            variability: Variability::Discrete,
            start: Some(Expr::Number(start)),
            description: Some("state machine bookkeeping".to_string()),
            ..blank_component()
        });
        clock_of.insert(name.clone(), clock.clone());
    }

    // Which states are entered with their variables put back to their
    // start values, as `reset = true` asks.
    let resets: Vec<bool> = states
        .iter()
        .map(|state| {
            model
                .transitions
                .iter()
                .any(|transition| &transition.to == state && transition.reset)
        })
        .collect();

    // The states' equations, guarded by the state being in force.
    let starts: HashMap<String, Expr> = model
        .components
        .iter()
        .filter_map(|component| Some((component.name.clone(), component.start.clone()?)))
        .collect();
    let mut kept = Vec::new();
    for equation in model.equations.drain(..) {
        let Expr::Ref(target) = &equation.lhs else {
            kept.push(equation);
            continue;
        };
        let Some(state) = owner(target) else {
            kept.push(equation);
            continue;
        };
        let in_force = Expr::Rel(
            crate::ast::RelOp::Eq,
            Box::new(Expr::Ref(active.clone())),
            Box::new(Expr::Number(state as f64)),
        );
        let holding = previous_of(target);
        let mut value = Expr::If(
            Box::new(in_force.clone()),
            Box::new(equation.rhs),
            Box::new(holding),
        );
        if resets[state] {
            let entered = Expr::And(
                Box::new(in_force),
                Box::new(Expr::Not(Box::new(Expr::Rel(
                    crate::ast::RelOp::Eq,
                    Box::new(previous_of(&active)),
                    Box::new(Expr::Number(state as f64)),
                )))),
            );
            let back_to = starts.get(target).cloned().unwrap_or(Expr::Number(0.0));
            value = Expr::If(Box::new(entered), Box::new(back_to), Box::new(value));
        }
        clock_of.insert(target.clone(), clock.clone());
        kept.push(EquationItem {
            lhs: equation.lhs,
            rhs: value,
        });
    }
    model.equations = kept;

    // `activeState`, `ticksInState` and `timeInState` say what they
    // mean once the machine has a variable to say it with.
    for equation in &mut model.equations {
        equation.rhs = machine_queries(&equation.rhs, &states, &active, &ticks, period);
    }
    for (_, value) in &mut machine {
        *value = machine_queries(value, &states, &active, &ticks, period);
    }
    for (target, value) in machine {
        model.equations.push(EquationItem {
            lhs: Expr::Ref(target),
            rhs: value,
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
    states: &[String],
    active: &str,
    ticks: &str,
    period: f64,
) -> Expr {
    let recur = |e: &Expr| machine_queries(e, states, active, ticks, period);
    match expr {
        Expr::Call(name, args) if name == "activeState" && args.len() == 1 => {
            let wanted = match &args[0] {
                Expr::Ref(state) => states.iter().position(|s| s == state),
                _ => None,
            };
            match wanted {
                Some(index) => Expr::Rel(
                    crate::ast::RelOp::Eq,
                    Box::new(Expr::Ref(active.to_string())),
                    Box::new(Expr::Number(index as f64)),
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
        binding: None,
        description: None,
        scope: Scope::Local,
        replaceable: false,
        constrained_by: None,
        condition: None,
        redeclares: Vec::new(),
        redeclaration: false,
    }
}
