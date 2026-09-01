//! The equations of a class, put into the flat model: what it wrote
//! itself, what its algorithm sections come to, what its `when`
//! clauses say, and which branch of an `if` the compiler could settle.
//!
//! Carved out of `instantiate` unchanged.

use super::*;

/// One branch of an `if` at an event: its condition, where it has one,
/// and what each variable it names is given.
pub(super) type GivenBranch = (Option<Expr>, Vec<(String, Expr)>);

/// Record one `spatialDistribution` and give the equation section the
/// two boundary values in its place.
///
/// The arguments are checked here rather than at the run, since every
/// one of them but the two inputs is settled before it: a profile that
/// does not span the coordinate, or a pair of arrays of different
/// lengths, is a mistake in the model rather than in the arithmetic.
pub(super) fn spatial_transport(
    targets: &[Option<Expr>],
    arguments: &[Expr],
    prefix: &str,
    outers: &HashMap<String, String>,
    consts: &HashMap<String, f64>,
    acc: &mut Flat,
) -> Result<(), String> {
    if arguments.len() != 6 {
        return Err(format!(
            "spatialDistribution takes in0, in1, x, positiveVelocity, \
             initialPoints and initialValues, but got {} arguments",
            arguments.len()
        ));
    }
    if targets.len() != 2 {
        return Err("spatialDistribution fills two values, `(out0, out1)`".to_string());
    }
    let numbers = |expr: &Expr, what: &str| -> Result<Vec<f64>, String> {
        let Expr::Array(items) = expr else {
            return Err(format!("spatialDistribution needs an array for {what}"));
        };
        items
            .iter()
            .map(|item| {
                const_eval(item, consts).ok_or_else(|| {
                    format!("{what} of spatialDistribution must be known before the run")
                })
            })
            .collect()
    };
    let initial_points = numbers(&arguments[4], "initialPoints")?;
    let initial_values = numbers(&arguments[5], "initialValues")?;
    if initial_points.len() != initial_values.len() {
        return Err(format!(
            "spatialDistribution has {} initialPoints against {} initialValues",
            initial_points.len(),
            initial_values.len()
        ));
    }
    if initial_points.len() < 2
        || initial_points.first() != Some(&0.0)
        || initial_points.last() != Some(&1.0)
    {
        return Err("initialPoints of spatialDistribution must span 0 to 1".to_string());
    }
    if initial_points.windows(2).any(|pair| pair[1] < pair[0]) {
        return Err("initialPoints of spatialDistribution must not decrease".to_string());
    }
    let named = |target: &Option<Expr>, which: &str| -> Result<String, String> {
        match target {
            Some(Expr::Ref(name)) => Ok(name.clone()),
            _ => Err(format!(
                "the {which} of spatialDistribution must be a variable"
            )),
        }
    };
    acc.transports.push(SpatialTransport {
        out0: named(&targets[0], "first output")?,
        out1: named(&targets[1], "second output")?,
        in0: prefix_expr(&arguments[0], prefix, outers),
        in1: prefix_expr(&arguments[1], prefix, outers),
        x: prefix_expr(&arguments[2], prefix, outers),
        positive: prefix_expr(&arguments[3], prefix, outers),
        initial_points,
        initial_values,
    });
    Ok(())
}

/// A model's `algorithm` and `initial algorithm` sections, executed
/// symbolically: what comes out is one equation per variable assigned,
/// which is what the rest of the pipeline understands.
///
/// Moved out of `instantiate` unchanged.
#[allow(clippy::too_many_arguments)]
fn run_algorithm_sections(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    acc: &mut Flat,
    depth: usize,
    imports: &[(String, String)],
    outers: &HashMap<String, String>,
    sizes: &HashMap<String, Vec<i64>>,
    local_consts: &HashMap<String, f64>,
    expand_here: &ExpandHere<'_>,
) -> Result<(), String> {
    let scope = class.name.as_str();
    let no_loop_vars = HashMap::new();
    let resolve_here =
        |expr: &Expr| -> Result<Expr, String> { expand_here(expr, &HashMap::new())?.scalar() };
    // An `algorithm` section of a model is executed symbolically: what
    // comes out is one equation per variable it assigns, which is what
    // the rest of the pipeline understands.
    if class.kind != ClassKind::Function && !class.algorithm.is_empty() {
        // A `when` written among the statements is an event, not a
        // step of the algorithm: it becomes a clause of its own and
        // the rest of the section runs without it.
        let mut plain = Vec::new();
        for statement in &class.algorithm {
            let Statement::When(branches) = statement else {
                plain.push(statement.clone());
                continue;
            };
            let mut lifted = Vec::new();
            for branch in branches {
                // The body of a `when` is an algorithm like any other:
                // it may hold an `if`, a loop, or a write to one
                // element. Running it is what says which names it
                // leaves changed and what each of them is worth, and
                // that is exactly what an event does.
                let mut written: HashMap<String, Expr> = HashMap::new();
                let mut order: Vec<String> = Vec::new();
                let mut checked: Vec<(Expr, String)> = Vec::new();
                match statements::execute(
                    &branch.body,
                    &mut written,
                    &mut order,
                    &mut checked,
                    local_consts,
                    sizes,
                    registry,
                    scope,
                    imports,
                    depth,
                    false,
                )? {
                    Flow::Normal => {}
                    Flow::Break => {
                        return Err("`break` outside of a loop, inside a `when`".to_string())
                    }
                    Flow::Return => {
                        return Err(
                            "`return` belongs in a function, not a `when` of a model".to_string()
                        )
                    }
                }
                for (condition, message) in checked {
                    acc.asserts.push((resolve_here(&condition)?, message));
                }
                let mut actions = Vec::new();
                for target in &order {
                    // Every name the run put in the order it also gave
                    // a value; there is nothing to say about one that
                    // is not there.
                    let Some(value) = written.get(target) else {
                        continue;
                    };
                    actions.push(WhenAction::Assign(
                        flat_name(target, prefix, outers),
                        resolve_here(value)?,
                    ));
                }
                let condition = branch
                    .condition
                    .as_ref()
                    .ok_or_else(|| "a `when` has no `else`".to_string())?;
                lifted.push(WhenBranch {
                    condition: resolve_here(condition)?,
                    actions,
                });
            }
            acc.when_clauses.push(WhenClause {
                branches: lifted,
                origin: acc.origin.clone(),
            });
        }
        let mut bindings: HashMap<String, Expr> = HashMap::new();
        let mut assigned: Vec<String> = Vec::new();
        let mut section_asserts: Vec<(Expr, String)> = Vec::new();
        match statements::execute(
            &plain,
            &mut bindings,
            &mut assigned,
            &mut section_asserts,
            local_consts,
            sizes,
            registry,
            scope,
            imports,
            depth,
            false,
        )? {
            Flow::Normal => {}
            Flow::Break => return Err("`break` outside of a loop".to_string()),
            Flow::Return => {
                return Err("`return` belongs in a function, not a model algorithm".to_string())
            }
        }
        // Checks written among the statements are checks of the model,
        // and carry its prefix like everything else it says.
        for (condition, message) in section_asserts {
            acc.asserts.push((resolve_here(&condition)?, message));
        }
        for name in assigned {
            let value = bindings
                .get(&name)
                .ok_or_else(|| format!("`{name}` is assigned by the algorithm but has no value"))?;
            // Both sides may be arrays: `w := v .* k` assigns a whole
            // one, and comes out as one equation per element.
            push_equations(
                &expand_here(&Expr::Ref(name.clone()), &no_loop_vars)?,
                &expand_here(value, &no_loop_vars)?,
                acc,
            )?;
        }
    }

    // `initial algorithm` runs once, before the simulation starts. It
    // is executed the same way, and what it assigns is an equation of
    // the initial system: the statements are what decide where the
    // variables begin rather than how they move.
    if class.kind != ClassKind::Function && !class.initial_algorithm.is_empty() {
        if class
            .initial_algorithm
            .iter()
            .any(|s| matches!(s, Statement::When(_)))
        {
            return Err(
                "a `when` belongs in an algorithm that runs, not an initial one".to_string(),
            );
        }
        let mut bindings: HashMap<String, Expr> = HashMap::new();
        let mut assigned: Vec<String> = Vec::new();
        let mut section_asserts: Vec<(Expr, String)> = Vec::new();
        match statements::execute(
            &class.initial_algorithm,
            &mut bindings,
            &mut assigned,
            &mut section_asserts,
            local_consts,
            sizes,
            registry,
            scope,
            imports,
            depth,
            false,
        )? {
            Flow::Normal => {}
            Flow::Break => return Err("`break` outside of a loop".to_string()),
            Flow::Return => {
                return Err("`return` belongs in a function, not a model algorithm".to_string())
            }
        }
        for (condition, message) in section_asserts {
            acc.asserts.push((resolve_here(&condition)?, message));
        }
        // `push_equations` writes where ordinary equations go; what it
        // wrote here is the initial system, so it is moved across.
        let boundary = acc.equations.len();
        for name in assigned {
            let value = bindings.get(&name).ok_or_else(|| {
                format!("`{name}` is assigned by the initial algorithm but has no value")
            })?;
            push_equations(
                &expand_here(&Expr::Ref(name.clone()), &no_loop_vars)?,
                &expand_here(value, &no_loop_vars)?,
                acc,
            )?;
        }
        let written: Vec<EquationItem> = acc.equations.drain(boundary..).collect();
        acc.initial_equations.extend(written);
    }

    Ok(())
}

/// One equation that fills several targets at once, where it is one:
/// `(r, a, b, ku) = lowPass(cr, c0, c1, f_cut)` names four variables
/// and one call, and a skipped slot costs its output nothing since the
/// expression is never used.
///
/// `false` where the equation was not a tuple after all, and the
/// caller reads it the ordinary way.
///
/// Moved out of `flatten_equations` unchanged.
#[allow(clippy::too_many_arguments)]
fn one_tuple_equation(
    equation: &EquationItem,
    acc: &mut Flat,
    _class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    prefix: &str,
    imports: &[(String, String)],
    shadow: &[&str],
    outers: &HashMap<String, String>,
    sizes_here: &HashMap<String, Vec<i64>>,
    local_consts: &HashMap<String, f64>,
    records_here: &HashMap<String, String>,
    expand_here: &ExpandHere<'_>,
) -> Result<bool, String> {
    let no_loop_vars = HashMap::new();
    let Expr::Tuple(targets) = &equation.lhs else {
        return Ok(false);
    };
    let rhs = substitute_class_constants(&equation.rhs, registry, scope, imports, shadow);
    let rhs = prefix_expr(&rhs, prefix, outers);
    let Expr::Call(name, raw_args) = &rhs else {
        return Err("the right side of a tuple equation must be a function call".into());
    };
    // `spatialDistribution` fills a pair the way a function
    // does, but there is no body to inline: what it stands for
    // is a profile the run carries, so it is recorded here and
    // the equation becomes the two boundary values.
    if name == "spatialDistribution" {
        let shapes = Shapes {
            sizes: sizes_here,
            loop_vars: &no_loop_vars,
            consts: local_consts,
            records: records_here,
        };
        let arguments = raw_args
            .iter()
            .map(|arg| Ok(expand(arg, &shapes, registry, scope, imports, 0)?.into_expr()))
            .collect::<Result<Vec<Expr>, String>>()?;
        // The targets go through the usual pipeline, so the
        // names recorded are the flat ones.
        let mut named = Vec::new();
        for target in targets {
            let Some(target) = target else {
                named.push(None);
                continue;
            };
            named.push(Some(expand_here(target, &no_loop_vars)?.into_expr()));
        }
        spatial_transport(&named, &arguments, prefix, outers, local_consts, acc)?;
        return Ok(true);
    }
    let function = lookup(registry, name, scope, imports)
        .filter(|c| c.kind == ClassKind::Function)
        .ok_or_else(|| format!("`{name}` is not a function, so it cannot fill a tuple"))?;
    let shapes = Shapes {
        sizes: sizes_here,
        loop_vars: &no_loop_vars,
        consts: local_consts,
        records: records_here,
    };
    let values = raw_args
        .iter()
        .map(|arg| expand(arg, &shapes, registry, scope, imports, 0))
        .collect::<Result<Vec<_>, String>>()?;
    let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
    let arguments: Vec<Expr> = values.into_iter().map(|value| value.into_expr()).collect();
    let outputs = inlining::inline_function_outputs(
        function,
        &arguments,
        &argument_shapes,
        local_consts,
        registry,
        0,
    )?;
    if targets.len() > outputs.len() {
        return Err(format!(
            "`{name}` has {} output(s) for {} target(s)",
            outputs.len(),
            targets.len()
        ));
    }
    for (slot, (_, value)) in targets.iter().zip(outputs) {
        let Some(target) = slot else { continue };
        // The target goes through the usual pipeline; the
        // inlined value is already resolved and only needs the
        // array layer, or a second prefix would corrupt it.
        let lhs = expand_here(target, &no_loop_vars)?;
        let rhs = expand(&value, &shapes, registry, scope, imports, 0)?;
        push_equations(&lhs, &rhs, acc)?;
    }
    Ok(true)
}

/// What the pass before this one gathered about the connections: the
/// roots of the overconstrained graph and how many `connect` equations
/// named each port. `answered` is what says a pass has been made - a
/// model with no overconstrained loop has no roots in earnest, so
/// emptiness says nothing.
pub(super) struct Graph<'a> {
    pub(super) known_roots: &'a HashMap<String, bool>,
    pub(super) known_counts: &'a HashMap<String, f64>,
    pub(super) answered: bool,
}

/// The `if` equations of a class: the branch that holds contributes its
/// equations, the others contribute nothing. Conditions are structural,
/// so they must be constant at compile time.
///
/// The `when` clauses and graph clauses inside a branch the compiler
/// picked are gathered as they are met: they belong to the class as
/// much as the ones it wrote outright.
///
/// Moved out of `flatten_equations` unchanged.
#[allow(clippy::too_many_arguments)]
fn flatten_if_equations<'a>(
    class: &'a ClassDef,
    acc: &mut Flat,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    prefix: &str,
    imports: &[(String, String)],
    outers: &HashMap<String, String>,
    sizes_here: &HashMap<String, Vec<i64>>,
    local_consts: &HashMap<String, f64>,
    records_here: &HashMap<String, String>,
    expand_here: &ExpandHere<'_>,
    resolve_here: &dyn Fn(&Expr) -> Result<Expr, String>,
    take_checks: &dyn Fn(&Expr, &mut Flat) -> Result<(), String>,
    tuple_equation: &dyn Fn(&EquationItem, &mut Flat) -> Result<bool, String>,
    graph: &Graph<'_>,
    whens_from_branches: &mut Vec<&'a WhenClause>,
    graph_from_branches: &mut Vec<&'a GraphClause>,
) -> Result<(), String> {
    let no_loop_vars = HashMap::new();
    let Graph {
        known_roots,
        known_counts,
        answered,
    } = *graph;
    for if_equation in &class.if_equations {
        let mut env = acc.const_values.clone();
        env.extend(local_consts.iter().map(|(k, v)| (k.clone(), *v)));
        // A structural condition picks one branch and the model is
        // built from it. A condition only the run holds decides
        // nothing here, so every branch must contribute the same
        // number of equations and each position becomes one equation
        // that chooses its residual as it goes.
        // A condition is read with the constants of the classes it
        // names put in first: `smoothness == Smoothness.LinearSegments`
        // compares a parameter against an enumeration literal, and
        // neither is a name the environment holds on its own. A
        // question about the connection graph is answered from the
        // roots, which are in hand on the pass that follows the one
        // that drew them.
        let settle = |condition: &Expr| {
            let named = substitute_class_constants(condition, registry, scope, imports, &[]);
            if let Some(value) = const_eval(&named, &env) {
                return Some(value);
            }
            if !answered {
                return None;
            }
            let asked = prefix_expr(&named, prefix, outers);
            let told = answer_graph_queries(&asked, known_roots, known_counts);
            const_eval(&told, &env)
        };
        let decidable = if_equation.branches.iter().all(|branch| {
            branch
                .condition
                .as_ref()
                .is_none_or(|condition| settle(condition).is_some())
        });
        // A condition that asks the graph where the graph has not been
        // drawn cannot be answered here, and the branches of such an
        // `if` are not balanced - a body that is a root carries states
        // and one that is not carries none. So it is set aside, and
        // the whole model is built again once the graph is in.
        if !decidable && !answered && asks_the_graph(if_equation) {
            acc.graph_asked = true;
            continue;
        }
        if !decidable {
            push_conditional(
                if_equation,
                &class.name,
                resolve_here,
                expand_here,
                &no_loop_vars,
                acc,
            )?;
            continue;
        }
        let mut chosen = None;
        for branch in &if_equation.branches {
            match &branch.condition {
                None => {
                    chosen = Some(branch);
                    break;
                }
                Some(condition) => {
                    let value = settle(condition).ok_or_else(|| {
                        format!(
                            "condition of an `if` equation in `{}` is not a compile-time constant",
                            class.name
                        )
                    })?;
                    if value != 0.0 {
                        chosen = Some(branch);
                        break;
                    }
                }
            }
        }
        let Some(branch) = chosen else { continue };
        // The branch is the one taken, so its checks hold outright.
        for (condition, message) in &branch.asserts {
            acc.asserts
                .push((resolve_here(condition)?, message.clone()));
        }
        whens_from_branches.extend(branch.whens.iter());
        graph_from_branches.extend(branch.graph.iter());
        for call in &branch.calls {
            take_checks(call, acc)?;
        }
        for loop_eq in &branch.loops {
            unroll(
                loop_eq,
                &HashMap::new(),
                local_consts,
                prefix,
                outers,
                sizes_here,
                records_here,
                registry,
                scope,
                imports,
                acc,
            )?;
        }
        for equation in &branch.equations {
            if tuple_equation(equation, acc)? {
                continue;
            }
            // An `if` written in an `initial equation` section says
            // where the run begins, so what its branch holds joins the
            // initial equations rather than the running ones - the
            // integrator's `if initType == ... then y = y_start` is
            // one, and read as an ordinary equation it left the model
            // with one equation more than it had unknowns.
            let boundary = acc.equations.len();
            push_equations(
                &expand_here(&equation.lhs, &no_loop_vars)?,
                &expand_here(&equation.rhs, &no_loop_vars)?,
                acc,
            )?;
            if if_equation.initial {
                let written: Vec<EquationItem> = acc.equations.drain(boundary..).collect();
                acc.initial_equations.extend(written);
            }
        }
        for (a, b) in &branch.connects {
            let shapes = Shapes {
                sizes: sizes_here,
                loop_vars: &no_loop_vars,
                consts: local_consts,
                records: records_here,
            };
            push_connects(a, b, &shapes, prefix, outers, registry, scope, imports, acc)?;
        }
    }

    Ok(())
}

/// The `when` clauses of a class: what it says outright, and what the
/// branches the compiler picked said as well.
///
/// A `when` is an event and what to do at it - assign, reinitialize,
/// check - and each of those is read the way an equation is, with the
/// arrays expanded and the names of this class put on.
///
/// Moved out of `flatten_equations` unchanged.
#[allow(clippy::too_many_arguments)]
fn flatten_when_clauses(
    class: &ClassDef,
    acc: &mut Flat,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    prefix: &str,
    imports: &[(String, String)],
    shadow: &[&str],
    outers: &HashMap<String, String>,
    sizes_here: &HashMap<String, Vec<i64>>,
    local_consts: &HashMap<String, f64>,
    records_here: &HashMap<String, String>,
    expand_here: &ExpandHere<'_>,
    resolve_here: &dyn Fn(&Expr) -> Result<Expr, String>,
    take_checks: &dyn Fn(&Expr, &mut Flat) -> Result<(), String>,
    whens_from_branches: &[&WhenClause],
) -> Result<(), String> {
    let no_loop_vars = HashMap::new();
    for clause in class
        .when_clauses
        .iter()
        .chain(whens_from_branches.iter().copied())
    {
        let mut branches = Vec::new();
        for branch in &clause.branches {
            let mut actions = Vec::new();
            for action in &branch.actions {
                match action {
                    WhenAction::Reinit(state, value) => actions.push(WhenAction::Reinit(
                        flat_name(state, prefix, outers),
                        resolve_here(value)?,
                    )),
                    // A `when` may give a whole array at once -
                    // `y = u` between two vectors is how the clocked
                    // samplers pass a bus through - and an event
                    // assigns one variable, so it is taken apart the
                    // way an equation between arrays is: one
                    // assignment per element, refusing sides that do
                    // not have the same shape. A scalar target goes
                    // the short way, which is every other `when` in
                    // the library.
                    WhenAction::Assign(target, value) => {
                        let named = flat_name(target, prefix, outers);
                        let given = expand_here(value, &HashMap::new())?;
                        // The shapes are filed under the full path, and
                        // the target is written as this class named it,
                        // so it is the flat name that finds one.
                        match sizes_here.get(&named).filter(|shape| !shape.is_empty()) {
                            None => actions.push(WhenAction::Assign(named, given.scalar()?)),
                            Some(shape) => {
                                let mut elements = Vec::new();
                                given.flatten_into(&mut elements);
                                let wanted: usize =
                                    shape.iter().map(|length| *length as usize).product();
                                if elements.len() != wanted {
                                    return Err(format!(
                                        "`{named}` is given {} value(s) at an event and holds                                          {wanted}",
                                        elements.len()
                                    ));
                                }
                                for (indices, one) in index_tuples(shape).into_iter().zip(elements)
                                {
                                    actions.push(WhenAction::Assign(
                                        element_name(&named, &indices),
                                        one,
                                    ));
                                }
                            }
                        }
                    }
                    WhenAction::Terminate(message) => {
                        actions.push(WhenAction::Terminate(message.clone()))
                    }
                    // A call on its own at an event: nothing takes its
                    // outputs, so what the compiler can have of it is
                    // the checks its body makes - the same as a call
                    // standing among the equations. The effect itself,
                    // closing a file at `terminal()`, is one this
                    // compiler has no way to have.
                    WhenAction::Call(name, args) => {
                        take_checks(&Expr::Call(name.clone(), args.clone()), acc)?;
                    }
                    // A check made when the event fires: the names it
                    // was written with are this class's, so it is
                    // resolved here like any other expression.
                    WhenAction::Assert(condition, message) => actions.push(WhenAction::Assert(
                        expand_here(&resolve_here(condition)?, &HashMap::new())?.scalar()?,
                        message.clone(),
                    )),
                    // `if c then x = a; else x = b; end if;` at an
                    // event: what `x` is given depends on the
                    // condition, so it gets one assignment whose value
                    // is the choice. A branch that says nothing about
                    // a variable leaves it what it had, which is what
                    // `pre` of it is.
                    WhenAction::Choice(chosen) => {
                        let mut targets: Vec<String> = Vec::new();
                        let mut branches: Vec<GivenBranch> = Vec::new();
                        for branch in &chosen.branches {
                            // A connection is drawn once and for all,
                            // and a check has nowhere to go from here;
                            // what an `if` at an event holds is values.
                            if !branch.connects.is_empty()
                                || !branch.loops.is_empty()
                                || !branch.asserts.is_empty()
                                || !branch.whens.is_empty()
                                || !branch.calls.is_empty()
                                || !branch.graph.is_empty()
                            {
                                return Err(
                                    "an `if` inside `when` gives values to variables".to_string()
                                );
                            }
                            let mut given = Vec::new();
                            for equation in &branch.equations {
                                let Expr::Ref(target) = &equation.lhs else {
                                    return Err("an `if` inside `when` gives values to variables"
                                        .to_string());
                                };
                                let target = flat_name(target, prefix, outers);
                                if !targets.contains(&target) {
                                    targets.push(target.clone());
                                }
                                given.push((target, resolve_here(&equation.rhs)?));
                            }
                            let condition =
                                branch.condition.as_ref().map(&resolve_here).transpose()?;
                            branches.push((condition, given));
                        }
                        for target in targets {
                            // Built from the last branch back, so the
                            // conditions are tested in the order they
                            // were written.
                            let mut value =
                                Expr::Call("pre".to_string(), vec![Expr::Ref(target.clone())]);
                            for (condition, given) in branches.iter().rev() {
                                let Some(chosen) = given.iter().find(|(name, _)| name == &target)
                                else {
                                    continue;
                                };
                                value = match condition {
                                    None => chosen.1.clone(),
                                    Some(condition) => Expr::If(
                                        Box::new(condition.clone()),
                                        Box::new(chosen.1.clone()),
                                        Box::new(value),
                                    ),
                                };
                            }
                            actions.push(WhenAction::Assign(target, value));
                        }
                    }
                    // `for i in 1:n loop k[i] = ...; end for;` at an
                    // event: the loop is unrolled the way one among
                    // the equations is, and each round's equation
                    // becomes an assignment of its own. It is unrolled
                    // into the equations and taken straight back out,
                    // there being one unroller and no reason for two.
                    WhenAction::Loop(loop_eq) => {
                        let boundary = acc.equations.len();
                        let drawn = acc.connects.len();
                        unroll(
                            loop_eq,
                            &HashMap::new(),
                            local_consts,
                            prefix,
                            outers,
                            sizes_here,
                            records_here,
                            registry,
                            scope,
                            imports,
                            acc,
                        )?;
                        if acc.connects.len() != drawn {
                            return Err("a loop inside `when` assigns variables, one per round; \
                                        a connection is drawn once and for all, not at an event"
                                .to_string());
                        }
                        for round in acc.equations.drain(boundary..).collect::<Vec<_>>() {
                            let Expr::Ref(target) = round.lhs else {
                                return Err(
                                    "a loop inside `when` assigns variables, one per round"
                                        .to_string(),
                                );
                            };
                            actions.push(WhenAction::Assign(target, round.rhs));
                        }
                    }
                    // `(a, b) = f(x)` at an event: the call is inlined
                    // once per output, and each target gets an
                    // assignment of its own. A skipped slot costs its
                    // output nothing, since it is never used.
                    WhenAction::TupleAssign(targets, value) => {
                        // Not through `resolve_here`: that would inline
                        // the call into the one value an expression can
                        // carry, and what is wanted here is the call
                        // itself, to be inlined once per target.
                        let value =
                            substitute_class_constants(value, registry, scope, imports, shadow);
                        let value = prefix_expr(&value, prefix, outers);
                        let Expr::Call(name, raw_args) = &value else {
                            return Err(
                                "the right side of a tuple inside `when` must be a function call"
                                    .to_string(),
                            );
                        };
                        let function = lookup(registry, name, scope, imports)
                            .filter(|c| c.kind == ClassKind::Function)
                            .ok_or_else(|| {
                                format!("`{name}` is not a function, so it cannot fill a tuple")
                            })?;
                        let shapes = Shapes {
                            sizes: sizes_here,
                            loop_vars: &no_loop_vars,
                            consts: local_consts,
                            records: records_here,
                        };
                        let values = raw_args
                            .iter()
                            .map(|arg| expand(arg, &shapes, registry, scope, imports, 0))
                            .collect::<Result<Vec<_>, String>>()?;
                        let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
                        let arguments: Vec<Expr> =
                            values.into_iter().map(|value| value.into_expr()).collect();
                        let attempt = inlining::inline_function_outputs(
                            function,
                            &arguments,
                            &argument_shapes,
                            local_consts,
                            registry,
                            0,
                        );
                        // A body nothing could write out is walked by
                        // the run instead, and a tuple takes what it
                        // answers with the way an expression does:
                        // `f(...)[k]` for the k-th output. The table
                        // sources of the library are here - a `while`
                        // hunting the next row of a table cannot be
                        // unrolled, because which row is next is what
                        // the run decides.
                        let outputs = match attempt {
                            Ok(outputs) => outputs,
                            Err(why)
                                if (why.starts_with(algorithms::UNDECIDABLE_LOOP)
                                    || why.contains(algorithms::NO_BOTTOM)
                                    || why.contains(algorithms::UNDECIDABLE_LEAVING))
                                    && inlining::walkable(function, registry).is_ok()
                                    && inlining::carried_by_the_run(&arguments) =>
                            {
                                let standing = Expr::Call(name.clone(), arguments.clone());
                                inlining::declared_outputs(function, registry)
                                    .into_iter()
                                    .enumerate()
                                    .map(|(at, named)| {
                                        (
                                            named,
                                            Expr::Index(
                                                Box::new(standing.clone()),
                                                vec![Expr::Number(at as f64 + 1.0)],
                                            ),
                                        )
                                    })
                                    .collect()
                            }
                            Err(why) => return Err(why),
                        };
                        if outputs.len() < targets.len() {
                            return Err(format!(
                                "`{name}` has {} output(s) and the tuple asks for {}",
                                outputs.len(),
                                targets.len()
                            ));
                        }
                        for (target, (_, worth)) in targets.iter().zip(outputs) {
                            let Some(target) = target else { continue };
                            let target = flat_name(target, prefix, outers);
                            // An output of several numbers lands on
                            // several names: a generator answers with
                            // the state it moved to, and the model
                            // holds one number per name.
                            let placed = expand(
                                &Expr::Ref(target.clone()),
                                &shapes,
                                registry,
                                scope,
                                imports,
                                0,
                            )?;
                            let (mut names, mut worths) = (Vec::new(), Vec::new());
                            placed.flatten_into(&mut names);
                            expand(&worth, &shapes, registry, scope, imports, 0)?
                                .flatten_into(&mut worths);
                            if names.len() != worths.len() {
                                return Err(format!(
                                    "`{target}` is {} name(s) and what it is given at the \
                                     event is {} value(s)",
                                    names.len(),
                                    worths.len()
                                ));
                            }
                            for (name, worth) in names.into_iter().zip(worths) {
                                let Expr::Ref(name) = name else {
                                    return Err(format!(
                                        "`{target}` is not a name an event can assign"
                                    ));
                                };
                                actions.push(WhenAction::Assign(name, worth));
                            }
                        }
                    }
                }
            }
            branches.push(WhenBranch {
                condition: resolve_here(&branch.condition)?,
                actions,
            });
        }
        acc.when_clauses.push(WhenClause {
            branches,
            origin: acc.origin.clone(),
        });
    }

    Ok(())
}

/// Everything the class states rather than declares: its equations,
/// the `if` branches and `when` clauses among them, the values of its
/// record-valued declarations, its connections and its algorithm
/// sections.
///
/// Moved out of `instantiate` unchanged.
#[allow(clippy::too_many_arguments)]
pub(super) fn flatten_equations(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    env: &Env,
    acc: &mut Flat,
    depth: usize,
    imports: &[(String, String)],
    shadow: &[&str],
    outers: &HashMap<String, String>,
    sizes: &HashMap<String, Vec<i64>>,
    sizes_here: &HashMap<String, Vec<i64>>,
    local_consts: &HashMap<String, f64>,
    records_here: &HashMap<String, String>,
    record_values: &[(String, Expr, bool)],
    broke_something: &mut [bool],
) -> Result<(), String> {
    let scope = class.name.as_str();
    let broken = env.broken;
    // Equations: arrays expanded, subscripts resolved, calls inlined.
    let expand_here = |expr: &Expr, loop_vars: &HashMap<String, f64>| -> Result<Value, String> {
        let expr = substitute_class_constants(expr, registry, scope, imports, shadow);
        let expr = prefix_expr(&expr, prefix, outers);
        let shapes = Shapes {
            sizes: sizes_here,
            loop_vars,
            consts: local_consts,
            records: records_here,
        };
        let value = expand(&expr, &shapes, registry, scope, imports, 0)?;
        records_written_out(value, &shapes, registry, &|e| {
            expand(e, &shapes, registry, scope, imports, 0)
        })
    };
    // What has to come to one value - a condition, what a `when` gives
    // a variable - still goes through the array layer to get there:
    // `min({pre(y), u, pre(u)})` is one value made out of three, and
    // only the array layer knows how to read it.
    let resolve_here =
        |expr: &Expr| -> Result<Expr, String> { expand_here(expr, &HashMap::new())?.scalar() };
    let no_loop_vars = HashMap::new();
    // A record-valued variable's value, said now that both sides can
    // be written out as fields. It is an equation because that is what
    // a variable's declaration value is: `Complex vs[m] = plug.pin.v`
    // holds for the whole run.
    // A modifier arrives written in the terms of the class that
    // supplied it - `Shape s(R = mine)` names a record of the class
    // holding `s` - so those have to be in view as well as this
    // class's own. It is put together only where there is a record
    // value to say it of, since every class would otherwise pay for a
    // copy of every record the model holds.
    let records_wider = record_values
        .iter()
        .any(|(_, _, prefixed)| *prefixed)
        .then(|| {
            let mut all = acc.records.clone();
            all.extend(records_here.iter().map(|(k, v)| (k.clone(), v.clone())));
            all
        });
    for (name, value, prefixed) in record_values {
        let lhs = expand_here(&Expr::Ref(name.clone()), &no_loop_vars)?;
        let rhs = match prefixed {
            // A modifier arrives already written in the terms of the
            // class that supplied it.
            true => {
                let shapes = Shapes {
                    sizes: sizes_here,
                    loop_vars: &no_loop_vars,
                    consts: local_consts,
                    records: records_wider.as_ref().unwrap_or(records_here),
                };
                let worked = expand(value, &shapes, registry, scope, imports, 0)?;
                records_written_out(worked, &shapes, registry, &|e| {
                    expand(e, &shapes, registry, scope, imports, 0)
                })?
            }
            false => expand_here(value, &no_loop_vars)?,
        };
        // Where the value cannot be written out as fields - a record
        // named in a class this one knows nothing about - the two
        // sides do not line up, and the value is left where it was
        // rather than half-applied. That is what this compiler did
        // with every record value until now, so nothing is lost by
        // leaving it there.
        let held = acc.equations.len();
        if push_equations(&lhs, &rhs, acc).is_err() {
            acc.equations.truncate(held);
        }
    }
    // `(r, a, b, ku) = lowPass(cr, c0, c1, f_cut)`: one call fills
    // several targets, and a skipped slot costs its output nothing
    // since the expression is never used. A tuple may stand at the top
    // of a class or inside a branch the compiler settles, and it is
    // read the same way in both.
    let tuple_equation = |equation: &EquationItem, acc: &mut Flat| -> Result<bool, String> {
        one_tuple_equation(
            equation,
            acc,
            class,
            registry,
            scope,
            prefix,
            imports,
            shadow,
            outers,
            sizes_here,
            local_consts,
            records_here,
            &expand_here,
        )
    };
    for equation in &class.equations {
        // `(a, , c) = f(...)`: one call fills several targets. The
        // call is inlined once per output; a skipped slot costs its
        // computation nothing, since the expression is never used.
        if tuple_equation(equation, acc)? {
            continue;
        }
        let lhs = expand_here(&equation.lhs, &no_loop_vars)?;
        let rhs = expand_here(&equation.rhs, &no_loop_vars)?;
        push_equations(&lhs, &rhs, acc)?;
    }

    for (condition, message) in &class.asserts {
        // A check may be written over arrays - `assert(length(n) >
        // 0, ...)` of an axis of three - so it goes through the array
        // layer like an equation, and what comes out is the one truth
        // it has to be.
        let condition = expand_here(condition, &no_loop_vars)?.scalar()?;
        acc.asserts.push((condition, message.clone()));
    }

    // The arrows of a state machine name instances of this class, so
    // they carry its prefix like everything else.
    for transition in &class.transitions {
        acc.transitions.push(Transition {
            from: flat_name(&transition.from, prefix, outers),
            to: flat_name(&transition.to, prefix, outers),
            condition: resolve_here(&transition.condition)?,
            reset: transition.reset,
            immediate: transition.immediate,
            synchronize: transition.synchronize,
            priority: transition.priority,
        });
    }
    if let Some(state) = &class.initial_state {
        acc.initial_states.push(flat_name(state, prefix, outers));
    }
    for equation in &class.initial_equations {
        let (lhs, rhs) = (
            expand_here(&equation.lhs, &no_loop_vars)?,
            expand_here(&equation.rhs, &no_loop_vars)?,
        );
        let (mut left, mut right) = (Vec::new(), Vec::new());
        lhs.flatten_into(&mut left);
        rhs.flatten_into(&mut right);
        if left.len() != right.len() {
            return Err("an initial equation between shapes that do not match".to_string());
        }
        for (lhs, rhs) in left.into_iter().zip(right) {
            acc.initial_equations.push(EquationItem {
                lhs,
                rhs,
                origin: String::new(),
            });
        }
    }

    // The `algorithm` and `initial algorithm` sections, executed
    // symbolically into equations.
    run_algorithm_sections(
        registry,
        class,
        prefix,
        acc,
        depth,
        imports,
        outers,
        sizes,
        local_consts,
        &expand_here,
    )?;

    // A call written among the equations takes nothing back from what
    // it calls, so what it is there for is the checks the body makes.
    // They become the model's, carrying this instance's prefix like
    // everything else it says.
    let take_checks = |call: &Expr, acc: &mut Flat| -> Result<(), String> {
        let call = substitute_class_constants(call, registry, scope, imports, shadow);
        let call = prefix_expr(&call, prefix, outers);
        let Expr::Call(name, args) = &call else {
            return Err("a line of an equation section that is not an equation is a call".into());
        };
        let called = lookup(registry, name, scope, imports)
            .filter(|c| c.kind == ClassKind::Function)
            .ok_or_else(|| format!("`{name}` is not a function"))?;
        let shapes = Shapes {
            sizes: sizes_here,
            loop_vars: &no_loop_vars,
            consts: local_consts,
            records: records_here,
        };
        let values = args
            .iter()
            .map(|arg| expand(arg, &shapes, registry, scope, imports, 0))
            .collect::<Result<Vec<_>, String>>()?;
        let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
        let arguments: Vec<Expr> = values.into_iter().map(|value| value.into_expr()).collect();
        let checks = inlining::inline_function_checks(
            called,
            &arguments,
            &argument_shapes,
            local_consts,
            registry,
            0,
        )?;
        acc.asserts.extend(checks);
        Ok(())
    };
    for call in &class.calls {
        take_checks(call, acc)?;
    }

    // `for` equations are unrolled: the loop variable is a constant.
    for loop_eq in &class.for_equations {
        unroll(
            loop_eq,
            &HashMap::new(),
            local_consts,
            prefix,
            outers,
            sizes_here,
            records_here,
            registry,
            scope,
            imports,
            acc,
        )?;
    }

    // What a branch the compiler picked says about events joins what
    // the class says outright: `if use_reset then when reset then
    // reinit(y, y_start); end when; end if;` is how the standard
    // library gives a block a reset it can be built without.
    // What the pass before this one gathered about the connections:
    // the roots of the overconstrained graph, and how many `connect`
    // equations named each port. `answered` is what says a pass has
    // been made - a model with no overconstrained loop has no roots in
    // earnest, so emptiness says nothing.
    let known_roots = acc.roots.clone();
    let known_counts = acc.counts.clone();
    let answered = acc.answered;
    let mut whens_from_branches: Vec<&WhenClause> = Vec::new();
    let mut graph_from_branches: Vec<&GraphClause> = Vec::new();
    // `if` equations: the branch that holds contributes its equations,
    // the others contribute nothing. Conditions are structural, so they
    // must be constant at compile time.
    flatten_if_equations(
        class,
        acc,
        registry,
        scope,
        prefix,
        imports,
        outers,
        sizes_here,
        local_consts,
        records_here,
        &expand_here,
        &resolve_here,
        &take_checks,
        &tuple_equation,
        &Graph {
            known_roots: &known_roots,
            known_counts: &known_counts,
            answered,
        },
        &mut whens_from_branches,
        &mut graph_from_branches,
    )?;

    // What the class says about the overconstrained graph, and what
    // the branches the compiler picked said about it.
    for clause in class
        .connection_graph
        .iter()
        .chain(graph_from_branches.iter().copied())
    {
        acc.connection_graph.push(match clause {
            GraphClause::Root(node) => GraphClause::Root(flat_name(node, prefix, outers)),
            GraphClause::PotentialRoot(node, priority) => {
                GraphClause::PotentialRoot(flat_name(node, prefix, outers), *priority)
            }
            GraphClause::Branch(a, b) => {
                GraphClause::Branch(flat_name(a, prefix, outers), flat_name(b, prefix, outers))
            }
        });
    }

    flatten_when_clauses(
        class,
        acc,
        registry,
        scope,
        prefix,
        imports,
        shadow,
        outers,
        sizes_here,
        local_consts,
        records_here,
        &expand_here,
        &resolve_here,
        &take_checks,
        &whens_from_branches,
    )?;
    // A connection to a component that a condition left out goes with
    // it: this is how the standard library switches a support flange
    // between an external connector and an internal ground.
    for (a, b) in &class.connects {
        // `break connect(a, b)` drops this exact connection, in either
        // order, before it becomes a set.
        let (na, nb) = (connect_side_name(a), connect_side_name(b));
        if let Some(index) = broken.iter().position(|item| {
            matches!(item, Deselect::Connection(x, y)
                if (Some(x) == na.as_ref() && Some(y) == nb.as_ref())
                    || (Some(x) == nb.as_ref() && Some(y) == na.as_ref()))
        }) {
            broke_something[index] = true;
            continue;
        }
        let shapes = Shapes {
            sizes: sizes_here,
            loop_vars: &no_loop_vars,
            consts: local_consts,
            records: no_records(),
        };
        push_connects(a, b, &shapes, prefix, outers, registry, scope, imports, acc)?;
    }

    Ok(())
}

/// Whether a condition asks the connections a question.
///
/// `Connections.isRoot(frame_a.R)` and `Connections.rooted(...)` are
/// answered from the roots the graph was broken open at, and
/// `cardinality(port)` from how many `connect` equations named the
/// port. Both are gathered by building the model, so until one pass
/// has been made there is nothing to answer with.
pub(super) fn asks_the_connections(condition: &Expr) -> bool {
    let mut found = false;
    walk_calls(condition, &mut |name| {
        if name == "Connections.isRoot" || name == "Connections.rooted" || name == "cardinality" {
            found = true;
        }
    });
    found
}

/// Whether an `if` equation asks the connections a question.
pub(super) fn asks_the_graph(if_equation: &IfEquation) -> bool {
    if_equation
        .branches
        .iter()
        .any(|branch| branch.condition.as_ref().is_some_and(asks_the_connections))
}
