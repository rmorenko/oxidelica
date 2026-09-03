//! Partitions: lifting a model's clocked equations into the `when`
//! clauses that fire on each clock.
//!
//! What a clock *is* lives in `clocks.rs`; this is what a model made
//! of them becomes - which equation belongs to which partition, and
//! the counters and last-tick markers the machinery needs beside it.

use super::clocks::*;
use super::machines::{blank_component, build_state_machines};
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
/// Whether the equation that gives a name its value leaves the
/// clocked world.
///
/// A name is asked about by where it comes from, not only by where it
/// stands: `y = u` says nothing about `u` on its face, and `u = hold(z)`
/// a line below says `u` is continuous - so a clock reaching `y` must
/// stop rather than travel on.
///
/// Which side of the boundary the name lands on is the whole of it.
/// `hold` and `noClock` leave a clock, so what they give is
/// continuous and takes no clock from elsewhere. `sample` enters one,
/// so what it gives is clocked and a clock reaching it is right.
fn left_the_clocked_world(name: &str, model: &Model) -> bool {
    model.equations.iter().any(|other| {
        matches!(&other.lhs, Expr::Ref(target) if target == name)
            && ["hold", "noClock"]
                .iter()
                .any(|edge| mentions_call(&other.rhs, edge))
    })
}

/// The variables an equation puts on its own clock: everything it
/// reads, save what sits under an operator that changes the rate or
/// leaves the clocked world. `y = k[1]*u[1] + k[2]*u[2]` says all of
/// them tick together, and that is how a clock settled downstream
/// reaches back to what feeds it.
///
/// `hold`, `noClock` and `sample` are the ways out of a clock, and
/// the sub-clock conversions are the ways across to another one; the
/// boundary node is seen but not entered, so `superSample(u)` on the
/// right of a clocked equation says nothing about `u`.
fn on_the_same_clock(expr: &Expr, into: &mut Vec<String>) {
    let boundary = |node: &Expr| {
        matches!(node, Expr::Call(name, _)
            if matches!(name.as_str(), "hold" | "noClock" | "sample"
                | "subSample" | "superSample" | "shiftSample" | "backSample"))
    };
    super::algorithms::walk_pruned(expr, &boundary, &mut |node| {
        if let Expr::Ref(name) = node {
            into.push(name.clone());
        }
    });
}

pub(super) fn partition_clocks(model: &mut Model) -> Result<(), String> {
    // What every clock of the model ticks at, read off the
    // declarations and the equations that name them. `None` where the
    // model declares no clock at all, and the machines have been asked
    // about it already.
    let Some(DeclaredClocks {
        mut clocks,
        parameters,
        declared,
    }) = clocks_of_the_model(model)?
    else {
        return Ok(());
    };

    // A `when Clock() then ... end when` is a clocked partition
    // written out by hand, which is how the standard library's
    // samplers say that a handful of equations share one tick. The
    // actions are equations on that clock, so they are read out as
    // equations here and the clock they were written under is
    // remembered for each: a clause that named its clock hands it
    // straight over, and one that left it open - `Clock()` - has it
    // settled by whatever else the same equations touch, the whole
    // clause moving together because every target points at the one
    // place waiting for a clock.
    let mut grouped: HashMap<String, usize> = HashMap::new();
    let mut kept_clauses = Vec::new();
    for clause in model.when_clauses.drain(..) {
        let plain = clause.branches.len() == 1
            && clause.branches[0]
                .actions
                .iter()
                .all(|action| matches!(action, WhenAction::Assign(..)));
        let clock = match plain {
            true => clock_expr(&clause.branches[0].condition, &mut clocks, &parameters)?,
            false => None,
        };
        let Some(clock) = clock else {
            kept_clauses.push(clause);
            continue;
        };
        for action in &clause.branches[0].actions {
            let WhenAction::Assign(target, value) = action else {
                unreachable!("every action was checked to be an assignment")
            };
            grouped.insert(target.clone(), clock);
            model
                .equations
                .push(EquationItem::new(Expr::Ref(target.clone()), value.clone()));
        }
    }
    model.when_clauses = kept_clauses;

    // Which variable belongs to which clock. A `sample(u, c)` puts the
    // equation it sits in on `c`, and from there it spreads to
    // whatever those variables define.
    let mut clock_of: HashMap<String, usize> = HashMap::new();
    // A state machine is a clocked thing: it decides where it is at
    // each tick, and the equations of its states run only while their
    // state is the one it is in.
    build_state_machines(model, &clocks, &mut clock_of)?;
    // Only what varies with time can be on a clock: a parameter or a
    // constant holds its value across every tick and belongs to no
    // partition, so `y = k[1]*u[1]` hands its clock to `u[1]` alone.
    let known_variables: std::collections::HashSet<String> = model
        .components
        .iter()
        .filter(|c| {
            !matches!(
                c.variability,
                Variability::Parameter | Variability::Constant
            )
        })
        .map(|c| c.name.clone())
        .collect();
    for _ in 0..MAX_DEPTH {
        let mut settled = true;
        let mut found = Vec::new();
        for equation in &model.equations {
            let Some((target, is_rate)) = assigned_by(equation) else {
                continue;
            };
            // The reverse feed: an equation whose target already has a
            // clock is not finished with - it may hold a conversion
            // whose factor was left out, and this equation is the only
            // constraint on it. `y = if b_super <> previous(b_super)
            // then u_super else 0` is where an up-sampler's bare
            // `superSample` learns its rate, from the clock the sum
            // downstream gave `y`.
            //
            // Only forced steps: the target's clock goes into the same
            // list the right side's clocks go into, and `one_clock`
            // hands whatever is waiting to `work_out`, which solves a
            // factor and then proves it by deriving the goal back. A
            // factor that does not come back exactly is refused, so
            // nothing here can settle what the constraints leave free.
            if clock_of.contains_key(&target) {
                let known = clock_of[&target];
                found.clear();
                found.push(known);
                clocks_touched(
                    &equation.rhs,
                    &mut clocks,
                    &clock_of,
                    &parameters,
                    &mut found,
                )?;
                let waiting = found
                    .iter()
                    .any(|clock| clocks.spec(*clock).waiting().is_some());
                if waiting && one_clock(&found, &mut clocks, &target)?.is_some() {
                    settled = false;
                }
                // The same clock also travels the plain way: an
                // equation with no rate change on it holds all its
                // variables on one clock, so a target that has one
                // hands it to whatever it reads that has none.
                let mut reads = Vec::new();
                on_the_same_clock(&equation.rhs, &mut reads);
                for name in reads {
                    // The boundary a name stands on counts here as
                    // it does below: a clock that reaches `s.y` has
                    // nothing to say about the `s.u` a `sample` reads.
                    if !known_variables.contains(&name) || left_the_clocked_world(&name, model) {
                        continue;
                    }
                    match clock_of.get(&name) {
                        None => {
                            clock_of.insert(name, known);
                            settled = false;
                        }
                        // A variable two equations put on two clocks
                        // is refused, not decided: taking whichever
                        // was read first would make the meaning of a
                        // model depend on the order its equations
                        // happen to be written in.
                        Some(already) if !clocks.spec(*already).same(clocks.spec(known)) => {
                            return Err(super::clocks::two_clocks_at_once(
                                &name,
                                clocks.spec(*already),
                                clocks.spec(known),
                            ));
                        }
                        Some(_) => {}
                    }
                }
                continue;
            }
            found.clear();
            if let Some(clock) = grouped.get(&target) {
                found.push(*clock);
            }
            clocks_touched(
                &equation.rhs,
                &mut clocks,
                &clock_of,
                &parameters,
                &mut found,
            )?;
            if let Some(clock) = one_clock(&found, &mut clocks, &target)? {
                // A derivative joins a clock only where the clock says
                // how to step it across a tick. On any other it stays
                // continuous, and reading a clocked value from it is
                // the mistake the check further down names.
                if is_rate && clocks.spec(clock).solver.is_none() {
                    continue;
                }
                clock_of.insert(target, clock);
                settled = false;
            }
        }
        if settled {
            break;
        }
    }

    // A clock left for the compiler to work out has to have met a known
    // one by now. Letting an unsettled one through would be worse than
    // refusing it: nothing would be lifted onto it, and the equations
    // that were meant to tick would quietly stay continuous.
    for name in &declared {
        let index = clocks.by_name(name).expect("every one was checked above");
        if clocks.spec(index).waiting().is_some() {
            return Err(format!(
                "nothing in this model says how often `{name}` ticks - a clock written as \
                 `Clock()` takes its rate from an equation where it meets a clock that has \
                 one"
            ));
        }
    }

    // A name a clocked equation reads is on that clock too, where its
    // own equation cannot stand off one. `counter = if
    // previous(counter) < startTick then ...` says nothing on its own
    // - `previous` asks for a clock rather than giving one - and the
    // clock stands one equation away, on the `y` the block was
    // written to answer with, or across the `connect` that gave the
    // block its clock in the first place.
    //
    // Only to a name whose own equation writes `previous` or its kin:
    // those cannot stand off a clock at all, so joining one is the
    // only reading. A name that asks for none - what a `sample` reads
    // - is continuous on purpose, and pulling it in would lift
    // equations that were meant to stay.
    for _ in 0..MAX_DEPTH {
        let asks_for_a_clock = |name: &str| {
            model.equations.iter().any(|other| {
                matches!(&other.lhs, Expr::Ref(target) if target == name)
                    && ["previous", "firstTick", "subSample", "superSample"]
                        .iter()
                        .any(|asked| mentions_call(&other.rhs, asked))
            })
        };
        let mut joined = Vec::new();
        for equation in &model.equations {
            let Expr::Ref(target) = &equation.lhs else {
                continue;
            };
            let Some(clock) = clock_of.get(target).copied() else {
                continue;
            };
            let mut named = Vec::new();
            named_within_the_partition(&equation.rhs, &mut named);
            for name in named {
                if clock_of.contains_key(&name)
                    || !model.components.iter().any(|held| held.name == name)
                {
                    continue;
                }
                // Either the name cannot stand off a clock itself, or
                // it is one end of a plain equality with something
                // that cannot: `assignClock1.y = assignClock1.u` and
                // `assignClock1.u = step.y` are how a `connect`
                // arrives, and the clock a model assigns has to cross
                // them to reach the block that wrote nothing about
                // one. An equality is the whole equation and holds no
                // boundary, so nothing continuous rides over.
                // A plain equality carries the clock only where the
                // name it reaches does not stand on a boundary of its
                // own. `s.y = sample(s.u)` is a sampler: `s.y` is on
                // the clock and `s.u` is the continuous signal it
                // reads, so the equality between them is exactly
                // where a clock must stop.
                let crosses = model.equations.iter().any(|other| {
                    matches!(&other.lhs, Expr::Ref(target) if target == &name)
                        && ["sample", "hold", "noClock"]
                            .iter()
                            .any(|edge| mentions_call(&other.rhs, edge))
                });
                let plain = matches!(&equation.rhs, Expr::Ref(_)) && !crosses;
                if (asks_for_a_clock(&name) || plain) && known_variables.contains(&name) {
                    joined.push((name, clock));
                }
            }
        }
        if joined.is_empty() {
            break;
        }
        for (name, clock) in joined {
            clock_of.insert(name, clock);
        }
    }

    // An operator that only makes sense on a clock has to be on one.
    for equation in &model.equations {
        if let Expr::Ref(target) = &equation.lhs {
            if clock_of.contains_key(target) {
                continue;
            }
            for asked in [
                "previous",
                "firstTick",
                "subSample",
                "superSample",
                "noClock",
            ] {
                if mentions_call(&equation.rhs, asked) {
                    return Err(format!(
                        "`{target}` uses `{asked}`, but nothing says which clock it is on"
                    ));
                }
            }
        }
    }

    // Lift the clocked equations into one `when` per clock.
    let mut kept = Vec::new();
    let mut lifted: HashMap<usize, Vec<(String, Expr)>> = HashMap::new();
    let mut rates: HashMap<usize, Vec<(String, Expr)>> = HashMap::new();
    for equation in model.equations.drain(..) {
        let clock = assigned_by(&equation)
            .and_then(|(target, is_rate)| Some((target.clone(), is_rate, *clock_of.get(&target)?)));
        match clock {
            Some((target, is_rate, clock)) => {
                let value = at_the_tick(&equation.rhs, &clocks, &clock_of, Some(clock));
                let into = if is_rate { &mut rates } else { &mut lifted };
                into.entry(clock).or_default().push((target, value));
            }
            None => kept.push(equation),
        }
    }
    model.equations = kept;
    let mut bookkeeping: Vec<(String, usize, f64)> = Vec::new();

    // A clock carrying derivatives steps them across its tick with the
    // method it was given, which turns each into an assignment like any
    // other. It happens before the partitions are ordered, so what the
    // step reads counts towards that order.
    let mut clocks_with_rates: Vec<usize> = rates.keys().copied().collect();
    clocks_with_rates.sort_unstable();
    for clock in clocks_with_rates {
        let mut states = rates.remove(&clock).expect("just listed");
        states.sort_by(|left, right| left.0.cmp(&right.0));
        let spec = clocks.spec(clock).clone();
        // A clock reached by a `der` without being told how to step
        // is a model the compiler cannot run, not a mistake in the
        // compiler: `Clock(0.1)` says when to tick and says nothing
        // about integrating, and a state can land on one by the clock
        // travelling along the equations that feed it.
        let Some(solver) = spec.solver else {
            let named = states
                .iter()
                .map(|(target, _)| format!("`{target}`"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "{named} changes by a derivative on a clock ticking {}, which was not \
                 given a way to step it - a clocked state asks for a solver method, as in \
                 `Clock(0.1, solverMethod = \"ExplicitEuler\")`",
                spec.describe()
            ));
        };
        // The step just taken is one the run can measure. The step
        // about to be taken is not, on an event clock, and a method
        // with more than one stage has to guess where the state will be
        // partway through it - so those want a clock that says in
        // advance how long its ticks are.
        let step = match spec.interval() {
            Some(seconds) => Expr::Number(seconds),
            None if solver.weights.len() == 1 => elapsed_since_last_tick(&spec, clock),
            None => {
                return Err(format!(
                    "`{}` works out where the state will be partway through a step, and an \
                     event clock does not know how long its next step is - `ExplicitEuler` \
                     is what a clock ticking on a condition can be stepped with",
                    solver.name
                ))
            }
        };
        let stepped = one_step(solver, clock, &states, &step);
        for (target, _) in &stepped {
            if !states.iter().any(|(name, _)| name == target) {
                bookkeeping.push((target.clone(), clock, 0.0));
            }
        }
        lifted.entry(clock).or_default().extend(stepped);
    }
    for (name, clock, _) in &bookkeeping {
        clock_of.insert(name.clone(), *clock);
    }
    for clock in in_partition_order(&lifted)? {
        let mut actions = lifted.remove(&clock).expect("the order names each once");
        let spec = clocks.spec(clock).clone();
        let counter = counter_name(clock);
        let last = last_tick_name(clock);

        // `firstTick` needs the partition to count its own ticks, and
        // nothing but a counter will do it: a clock has no other way of
        // telling its first activation from its hundredth. An event
        // clock's `interval` reads the same counter to know whether
        // there was a tick before to measure back to.
        let asks_when = actions.iter().any(|(_, value)| mentions_ref(value, &last));
        if asks_when
            || actions
                .iter()
                .any(|(_, value)| mentions_call(value, "firstTick"))
        {
            for (_, value) in &mut actions {
                *value = answer_first_tick(value, &counter);
            }
            actions.push((counter.clone(), after(&counter, Expr::Number(1.0))));
            bookkeeping.push((counter.clone(), clock, 0.0));
        }
        if asks_when {
            actions.push((last.clone(), Expr::Time));
            bookkeeping.push((last, clock, 0.0));
        }

        // An event clock sub-sampled by n fires on every n-th rising
        // edge, but the edge itself arrives every time, so the
        // partition counts the ones it skips and holds what it had
        // through them. A periodic clock needs none of this: its rate
        // is already in the interval it ticks on.
        let condition = match &spec.root {
            Root::Every(..) => Expr::Call(
                "sample".to_string(),
                vec![
                    Expr::Number(spec.first().expect("a periodic clock has a first tick")),
                    Expr::Number(spec.interval().expect("and an interval")),
                ],
            ),
            Root::When(condition, ..) => condition.clone(),
            Root::Waiting { .. } => {
                return Err(format!(
                    "nothing in this model says how often `{}` ticks - a clock left for the \
                     compiler to work out has to meet a known one somewhere in an equation",
                    actions
                        .first()
                        .map(|(target, _)| target.as_str())
                        .unwrap_or("it")
                ))
            }
        };
        if spec.interval().is_none() && spec.every_nth() > 1 {
            let skipped = format!("$every{clock}");
            let due = Expr::Rel(
                crate::ast::RelOp::Ge,
                Box::new(after(&skipped, Expr::Number(1.0))),
                Box::new(Expr::Number(spec.every_nth() as f64)),
            );
            for (target, value) in &mut actions {
                *value = Expr::If(
                    Box::new(Expr::Rel(
                        crate::ast::RelOp::Lt,
                        Box::new(Expr::Ref(skipped.clone())),
                        Box::new(Expr::Number(0.5)),
                    )),
                    Box::new(value.clone()),
                    Box::new(Expr::Call(
                        "pre".to_string(),
                        vec![Expr::Ref(target.clone())],
                    )),
                );
            }
            actions.push((
                skipped.clone(),
                Expr::If(
                    Box::new(due),
                    Box::new(Expr::Number(0.0)),
                    Box::new(after(&skipped, Expr::Number(1.0))),
                ),
            ));
            // Counting from one short of the factor makes the first
            // edge a firing one, as 16.5 asks: the sub-sampled clock's
            // first activation is its argument's first activation.
            bookkeeping.push((skipped, clock, spec.every_nth() as f64 - 1.0));
        }

        // The equations of a partition are equations, in no order of
        // their own; what the tick needs is an order in which each is
        // ready when its turn comes. `previous` reaches back to the
        // tick before, so it is not a reason to wait.
        let actions = in_dependency_order(actions)?;
        // What an event clock waits for happens in continuous time, so
        // its condition is written in continuous time too: a clocked
        // variable only changes at a tick, and a clock waiting on one of
        // its own would be waiting on itself.
        if let Some(clocked) = clocked_outside_hold(&condition, &clock_of) {
            return Err(format!(
                "an event clock waits on something the run varies between ticks, and \
                 `{clocked}` is clocked - `hold({clocked})` is how a clocked value is \
                 read in continuous time"
            ));
        }
        model.when_clauses.push(WhenClause {
            branches: vec![WhenBranch { condition, actions }],
            origin: String::new(),
        });
    }
    for (name, clock, start) in bookkeeping {
        model.components.push(Component {
            name: name.clone(),
            variability: Variability::Discrete,
            start: Some(Expr::Number(start)),
            description: Some("clock bookkeeping".to_string()),
            ..blank_component()
        });
        clock_of.insert(name, clock);
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
        equation.lhs = at_the_tick(&equation.lhs, &clocks, &clock_of, None);
        equation.rhs = at_the_tick(&equation.rhs, &clocks, &clock_of, None);
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

/// The clocks a model declares, what they tick at, and the numbers
/// they were read against.
struct DeclaredClocks {
    clocks: Clocks,
    parameters: HashMap<String, f64>,
    declared: Vec<String>,
}

/// What every clock the model declares ticks at.
///
/// A clock says what it is either in its declaration or in an equation
/// naming it, and either may be written in terms of parameters built
/// on other parameters - the standard library's exact clock reads its
/// factor out of a table of constants - so the numbers are settled
/// first and the clocks read against them.
///
/// `None` where the model declares no clock: a machine with no clock
/// to run on still has to hear about it, and hears here.
///
/// Moved out of `partition_clocks` unchanged.
fn clocks_of_the_model(model: &mut Model) -> Result<Option<DeclaredClocks>, String> {
    let declared: Vec<String> = model
        .components
        .iter()
        .filter(|component| component.type_name == "Clock")
        .map(|component| component.name.clone())
        .collect();
    if declared.is_empty() {
        // A machine with no clock to run on still has to hear about
        // it, so it is asked before this pass gives up.
        build_state_machines(model, &Clocks::default(), &mut HashMap::new())?;
        return Ok(None);
    }
    // Parameters may be built on one another - the standard library's
    // exact clock reads its factor out of a table of constants - so
    // they are worked out until nothing new settles rather than in one
    // pass against nothing.
    let mut parameters: HashMap<String, f64> = HashMap::new();
    loop {
        let before = parameters.len();
        for component in &model.components {
            if parameters.contains_key(&component.name) {
                continue;
            }
            let Some(value) = component
                .binding
                .as_ref()
                .and_then(|value| const_eval(value, &parameters))
            else {
                continue;
            };
            parameters.insert(component.name.clone(), value);
        }
        if parameters.len() == before {
            break;
        }
    }

    // A clock says what it is either in its declaration or in an
    // equation of its own, and it may say it in terms of another -
    // `Clock fast = superSample(slow, 3)` - so the definitions are
    // gathered first and worked out until nothing new settles.
    let mut definitions: Vec<(String, Expr)> = model
        .components
        .iter()
        .filter(|component| component.type_name == "Clock")
        .filter_map(|component| Some((component.name.clone(), component.binding.clone()?)))
        .collect();
    let mut kept = Vec::new();
    for equation in model.equations.drain(..) {
        // Either side may be the clock being said: a connection
        // between two of them - a clock signal drawn from one block to
        // another - comes out with whichever name sorts first on the
        // left, and that one may be the one already known.
        let spoken_for = |name: &String| definitions.iter().any(|(known, _)| known == name);
        let said = match (&equation.lhs, &equation.rhs) {
            // Where both are clocks - a clock signal drawn from one
            // block to another is exactly that - the one being said is
            // the one nothing has said yet.
            (Expr::Ref(left), Expr::Ref(right))
                if declared.contains(left) && declared.contains(right) =>
            {
                match spoken_for(left) {
                    true => Some((right.clone(), equation.lhs.clone())),
                    false => Some((left.clone(), equation.rhs.clone())),
                }
            }
            (Expr::Ref(target), _) if declared.contains(target) => {
                Some((target.clone(), equation.rhs.clone()))
            }
            (_, Expr::Ref(target)) if declared.contains(target) => {
                Some((target.clone(), equation.lhs.clone()))
            }
            _ => None,
        };
        match said {
            Some(said) => definitions.push(said),
            None => kept.push(equation),
        }
    }
    model.equations = kept;

    let mut clocks = Clocks::default();
    for _ in 0..MAX_DEPTH {
        let mut settled = true;
        for (name, value) in &definitions {
            if clocks.by_name(name).is_some() {
                continue;
            }
            if let Some(index) = clock_expr(value, &mut clocks, &parameters)? {
                clocks.named.insert(name.clone(), index);
                settled = false;
            }
        }
        if settled {
            break;
        }
    }
    for name in &declared {
        let Some(index) = clocks.by_name(name) else {
            return Err(format!(
                "`{name}` is a Clock, so it needs an interval the compiler can see: \
                 `Clock {name} = Clock(0.1);`"
            ));
        };
        if clocks
            .spec(index)
            .interval()
            .is_some_and(|seconds| seconds <= 0.0)
        {
            return Err(format!("the interval of `{name}` must be positive"));
        }
    }

    Ok(Some(DeclaredClocks {
        clocks,
        parameters,
        declared,
    }))
}
