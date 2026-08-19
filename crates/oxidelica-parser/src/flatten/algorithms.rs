//! Algorithm sections executed symbolically, and functions inlined
//! at the place they were called.

use super::*;

/// How a body says it cannot be unrolled.
///
/// The one place that can answer this differently - by leaving the call
/// standing for the run to walk - looks for these words, so they are
/// written once and matched against rather than repeated.
pub(super) const UNDECIDABLE_LOOP: &str = "the trip count of a loop is not settled here";

/// How an unrolling says it did not come to an end.
///
/// A body that calls itself unrolls only as far as what decides the
/// recursion is settled; an expression that nests deeper than the
/// compiler follows says the same thing about itself. Either way the
/// call is left standing and the run walks it, so the two are matched
/// against by these words rather than by where they came from.
pub(super) const NO_BOTTOM: &str = "did not come to an end here";

/// Whether an expression still holds a call to a function whose calls
/// lead back to itself. Such a call is what an unrolling that stopped
/// short leaves behind.
fn holds_unbounded_call(expr: &Expr, registry: &HashMap<&str, &ClassDef>) -> bool {
    // The names an unrolling leaves behind are already qualified, so
    // the registry answers to them directly.
    let mut called = Vec::new();
    gather_calls(expr, registry, "", &[], &mut called);
    called.iter().any(|name| {
        registry
            .get(name.as_str())
            .is_some_and(|class| class.kind == ClassKind::Function && recursive(class, registry))
    })
}

/// Whether a function's calls lead back to itself, directly or through
/// others. Such a call has no bottom to inline to.
pub(super) fn recursive(class: &ClassDef, registry: &HashMap<&str, &ClassDef>) -> bool {
    let mut seen: Vec<String> = Vec::new();
    let mut wanted: Vec<String> = Vec::new();
    gather_calls_in_statements(
        &class.algorithm,
        registry,
        &class.name,
        &class.imports,
        &mut wanted,
    );
    while let Some(name) = wanted.pop() {
        if name == class.name {
            return true;
        }
        if seen.contains(&name) {
            continue;
        }
        seen.push(name.clone());
        let called = registry[name.as_str()];
        gather_calls_in_statements(
            &called.algorithm,
            registry,
            &called.name,
            &called.imports,
            &mut wanted,
        );
    }
    false
}

/// Whether flow control could fire at this nesting level: a `break` or
/// `return` here or in an `if` here, or a `return` inside a loop here -
/// loops consume their own breaks but a return passes through them.
pub(super) fn has_flow_control(statements: &[Statement]) -> bool {
    statements.iter().any(|statement| match statement {
        Statement::Break | Statement::Return => true,
        Statement::If(branches) => branches.iter().any(|b| has_flow_control(&b.body)),
        Statement::For(_, _, body) | Statement::While(_, body) => has_return(body),
        _ => false,
    })
}

/// What `for i loop` runs over among statements: the size of the array
/// along the dimension `i` is used to subscript, wherever the body
/// first uses it that way.
pub(super) fn implied_statement_range(
    body: &[Statement],
    variable: &str,
    sizes: &HashMap<String, Vec<i64>>,
) -> Result<Vec<f64>, String> {
    fn look(body: &[Statement], variable: &str, sizes: &HashMap<String, Vec<i64>>) -> Option<i64> {
        body.iter().find_map(|statement| match statement {
            Statement::Assign(name, subscripts, value) => sizes
                .get(name)
                .and_then(|shape| {
                    subscripts
                        .iter()
                        .position(|s| matches!(s, Expr::Ref(used) if used == variable))
                        .and_then(|at| shape.get(at).copied())
                })
                .or_else(|| subscript_extent(value, variable, sizes)),
            Statement::TupleAssign(_, value) => subscript_extent(value, variable, sizes),
            Statement::If(branches) | Statement::When(branches) => branches
                .iter()
                .find_map(|branch| look(&branch.body, variable, sizes)),
            Statement::For(_, _, inner) | Statement::While(_, inner) => {
                look(inner, variable, sizes)
            }
            _ => None,
        })
    }
    match look(body, variable, sizes) {
        Some(extent) => Ok((1..=extent).map(|index| index as f64).collect()),
        None => Err(format!(
            "`for {variable} loop` leaves the range to the body, and nothing in the body \
             uses `{variable}` to subscript an array of a length the compiler knows"
        )),
    }
}

/// Whether a `return` hides anywhere below, loops included.
pub(super) fn has_return(statements: &[Statement]) -> bool {
    statements.iter().any(|statement| match statement {
        Statement::Return => true,
        Statement::If(branches) => branches.iter().any(|b| has_return(&b.body)),
        Statement::For(_, _, body) | Statement::While(_, body) => has_return(body),
        _ => false,
    })
}

/// Symbolically execute an algorithm section.
///
/// `bindings` maps every variable the section has written to the
/// expression it now holds; reading a variable substitutes that
/// expression, which is what turns a sequence of assignments into one
/// expression per assigned variable. `assigned` collects the targets in
/// the order they were first written, so the equations a model gets out
/// of the section are in source order.
///
/// An `if` runs both ways: each branch is executed on its own copy of
/// the bindings and the results are merged into one `if` expression per
/// variable, with the value from before the statement as the fallback -
/// unless a branch holds `break` or `return`, in which case the
/// conditions must be decidable and only the taken branch runs.
/// A `for` is unrolled, its variable being a compile-time constant.
/// A `while` runs for real: its condition must be decidable each round,
/// and `fold` collapses the loop's assignments to numbers so the
/// expressions do not grow with the iteration count.
#[allow(clippy::too_many_arguments)]
pub(super) fn execute(
    statements: &[Statement],
    bindings: &mut HashMap<String, Expr>,
    assigned: &mut Vec<String>,
    asserts: &mut Vec<(Expr, String)>,
    consts: &HashMap<String, f64>,
    sizes: &HashMap<String, Vec<i64>>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
    fold: bool,
) -> Result<Flow, String> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "an algorithm {NO_BOTTOM}, nested deeper than the compiler follows"
        ));
    }
    for statement in statements {
        match statement {
            Statement::Assign(target, subscripts, value) => {
                // A body may name a constant of a package the way an
                // equation may, and it is resolved where it was
                // written - in this class, not at the call site.
                let value = substitute_class_constants(value, registry, scope, imports, &[]);
                let value = substitute_refs(&value, bindings);
                // Through the array layer, so `c := a .* b` binds a whole
                // array and a scalar stays a scalar.
                let no_loop_vars = HashMap::new();
                let shapes = Shapes {
                    sizes,
                    loop_vars: &no_loop_vars,
                    consts,
                    records: no_records(),
                };
                let value =
                    expand(&value, &shapes, registry, scope, imports, depth + 1)?.into_expr();
                // Expansion turns `p[i - 1]` into the element's own name,
                // which may itself be bound by an earlier statement - so
                // the bindings are applied once more.
                let value = substitute_refs(&value, bindings);
                // `c[i] := ...` lands on the element's own name.
                let target = if subscripts.is_empty() {
                    target.clone()
                } else {
                    let indices = subscripts
                        .iter()
                        .map(|subscript| {
                            let subscript = substitute_refs(subscript, bindings);
                            const_eval(&subscript, consts)
                                .filter(|v| v.fract() == 0.0 && *v >= 1.0)
                                .map(|v| v as i64)
                                .ok_or_else(|| {
                                    format!(
                                        "the subscript of `{target}` must be a whole number the compiler can see"
                                    )
                                })
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    element_name(target, &indices)
                };
                if !assigned.contains(&target) {
                    assigned.push(target.clone());
                }
                // Inside a `while`, a value that folds to a number is
                // stored as one, or the expressions would double in
                // size with every round.
                let value = match const_eval(&value, consts) {
                    Some(number) if fold => Expr::Number(number),
                    _ => value,
                };
                bindings.insert(target, value);
            }
            Statement::TupleAssign(targets, value) => {
                let value = substitute_refs(value, bindings);
                let Expr::Call(name, raw_args) = &value else {
                    return Err(
                        "the right side of a tuple assignment must be a function call".into(),
                    );
                };
                let function = lookup(registry, name, scope, imports)
                    .filter(|c| c.kind == ClassKind::Function)
                    .ok_or_else(|| {
                        format!("`{name}` is not a function, so it cannot fill a tuple")
                    })?;
                let no_loop_vars = HashMap::new();
                let shapes = Shapes {
                    sizes,
                    loop_vars: &no_loop_vars,
                    consts,
                    records: no_records(),
                };
                let values = raw_args
                    .iter()
                    .map(|arg| expand(arg, &shapes, registry, scope, imports, depth + 1))
                    .collect::<Result<Vec<_>, String>>()?;
                let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
                let arguments: Vec<Expr> = values
                    .into_iter()
                    .map(|value| substitute_refs(&value.into_expr(), bindings))
                    .collect();
                let outputs = inline_function_outputs(
                    function,
                    &arguments,
                    &argument_shapes,
                    consts,
                    registry,
                    depth + 1,
                )?;
                if targets.len() > outputs.len() {
                    return Err(format!(
                        "`{name}` has {} output(s) for {} target(s)",
                        outputs.len(),
                        targets.len()
                    ));
                }
                for (slot, (_, output)) in targets.iter().zip(outputs) {
                    let Some((target, subscripts)) = slot else { continue };
                    if !subscripts.is_empty() {
                        return Err(format!(
                            "`{target}` takes part of an array from a call filling several \
                             targets, which is more than this compiler does"
                        ));
                    }
                    if !assigned.contains(target) {
                        assigned.push(target.clone());
                    }
                    bindings.insert(target.clone(), output);
                }
            }
            Statement::If(branches) => {
                // A condition the compiler can decide picks one branch,
                // and only that one runs. Merging both would be the
                // same answer written at greater length - and where a
                // body calls itself, it would be no answer at all: the
                // branch that ends the recursion cannot end it if the
                // branch that continues it is taken as well.
                let decidable = branches.iter().all(|branch| {
                    branch.condition.as_ref().is_none_or(|condition| {
                        const_eval(&substitute_refs(condition, bindings), consts).is_some()
                    })
                });
                // A branch that may `break` or `return` cannot be
                // merged symbolically either - whether it fires must be
                // known. The conditions are decided and only the taken
                // branch runs, its flow passed on.
                if decidable || branches.iter().any(|b| has_flow_control(&b.body)) {
                    let mut taken = None;
                    for branch in branches {
                        match &branch.condition {
                            None => {
                                taken = Some(&branch.body);
                                break;
                            }
                            Some(condition) => {
                                let condition = substitute_refs(condition, bindings);
                                let value = const_eval(&condition, consts).ok_or_else(|| {
                                    "a branch holding `break` or `return` needs a condition \
                                     the compiler can decide"
                                        .to_string()
                                })?;
                                if value != 0.0 {
                                    taken = Some(&branch.body);
                                    break;
                                }
                            }
                        }
                    }
                    if let Some(body) = taken {
                        let flow = execute(
                            body,
                            bindings,
                            assigned,
                            asserts,
                            consts,
                            sizes,
                            registry,
                            scope,
                            imports,
                            depth + 1,
                            fold,
                        )?;
                        if flow != Flow::Normal {
                            return Ok(flow);
                        }
                    }
                    continue;
                }
                let before = bindings.clone();
                let mut outcomes: Vec<(Option<Expr>, HashMap<String, Expr>)> = Vec::new();
                for branch in branches {
                    let mut local = before.clone();
                    execute(
                        &branch.body,
                        &mut local,
                        assigned,
                        asserts,
                        consts,
                        sizes,
                        registry,
                        scope,
                        imports,
                        depth + 1,
                        fold,
                    )?;
                    let condition = branch
                        .condition
                        .as_ref()
                        .map(|c| {
                            let c = substitute_class_constants(c, registry, scope, imports, &[]);
                            let c = substitute_refs(&c, &before);
                            resolve(
                                &c,
                                &HashMap::new(),
                                consts,
                                sizes,
                                registry,
                                scope,
                                imports,
                                depth + 1,
                            )
                        })
                        .transpose()?;
                    outcomes.push((condition, local));
                }
                // Every variable any branch wrote gets one merged value.
                let mut touched: Vec<String> = Vec::new();
                for (_, local) in &outcomes {
                    for name in local.keys() {
                        if before.get(name) != local.get(name) && !touched.contains(name) {
                            touched.push(name.clone());
                        }
                    }
                }
                touched.sort();
                for name in touched {
                    let fallback = before.get(&name).cloned();
                    let mut value = match outcomes.last() {
                        // A trailing `else` supplies the last value.
                        Some((None, local)) => local.get(&name).cloned().or(fallback.clone()),
                        _ => fallback.clone(),
                    };
                    for (condition, local) in outcomes.iter().rev() {
                        let Some(condition) = condition else { continue };
                        let taken = local.get(&name).cloned().or_else(|| fallback.clone());
                        match (taken, value) {
                            (Some(taken), Some(otherwise)) => {
                                value = Some(Expr::If(
                                    Box::new(condition.clone()),
                                    Box::new(taken),
                                    Box::new(otherwise),
                                ));
                            }
                            _ => {
                                return Err(format!(
                                    "`{name}` is assigned in one branch only and has no value before the `if`"
                                ))
                            }
                        }
                    }
                    let Some(value) = value else {
                        return Err(format!(
                            "`{name}` is assigned in one branch only and has no value before the `if`"
                        ));
                    };
                    bindings.insert(name, value);
                }
            }
            // A check written where the statements are, carried out
            // to the model with whatever the section has assigned so
            // far already substituted into it.
            Statement::Assert(condition, message) => {
                asserts.push((substitute_refs(condition, bindings), message.clone()));
            }
            // A call on its own: nothing takes its outputs, so what it
            // was written for is the checks its body makes, and those
            // join the section's.
            Statement::Call(name, args) => {
                let called = lookup(registry, name, scope, imports)
                    .filter(|c| c.kind == ClassKind::Function)
                    .ok_or_else(|| format!("`{name}` is not a function"))?;
                let no_loop_vars = HashMap::new();
                let shapes = Shapes {
                    sizes,
                    loop_vars: &no_loop_vars,
                    consts,
                    records: no_records(),
                };
                let values = args
                    .iter()
                    .map(|arg| expand(arg, &shapes, registry, scope, imports, depth + 1))
                    .collect::<Result<Vec<_>, String>>()?;
                let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
                let arguments: Vec<Expr> = values
                    .into_iter()
                    .map(|value| substitute_refs(&value.into_expr(), bindings))
                    .collect();
                let checks = inline_function_checks(
                    called,
                    &arguments,
                    &argument_shapes,
                    consts,
                    registry,
                    depth + 1,
                )?;
                asserts.extend(checks);
            }
            Statement::For(variable, range, body) => {
                let values = match range {
                    Some(range) => {
                        let expr = substitute_refs(range, bindings);
                        // Through the array layer first, so a range
                        // written `1:size(v, 1)` is a list of numbers by
                        // the time it is asked to be constant - and so
                        // that `{1, 3, 5}` and the name of an array come
                        // out as the same kind of list.
                        let no_loop_vars = HashMap::new();
                        let spread = expand(
                            &expr,
                            &Shapes {
                                sizes,
                                loop_vars: &no_loop_vars,
                                consts,
                                records: no_records(),
                            },
                            registry,
                            scope,
                            imports,
                            depth + 1,
                        )?;
                        loop_values(&spread, consts, variable)?
                    }
                    None => implied_statement_range(body, variable, sizes)?,
                };
                for index in values {
                    bindings.insert(variable.clone(), Expr::Number(index));
                    let flow = execute(
                        body,
                        bindings,
                        assigned,
                        asserts,
                        consts,
                        sizes,
                        registry,
                        scope,
                        imports,
                        depth + 1,
                        fold,
                    )?;
                    match flow {
                        Flow::Normal => {}
                        Flow::Break => break,
                        Flow::Return => {
                            bindings.remove(variable);
                            assigned.retain(|name| name != variable);
                            return Ok(Flow::Return);
                        }
                    }
                }
                bindings.remove(variable);
                assigned.retain(|name| name != variable);
            }
            Statement::While(condition, body) => {
                let mut rounds = 0;
                loop {
                    let now = substitute_refs(condition, bindings);
                    let truth = const_eval(&now, consts).ok_or_else(|| {
                        format!(
                            "{UNDECIDABLE_LOOP}: a `while` here is unrolled, so the trip \
                             count cannot depend on a simulated variable"
                        )
                    })?;
                    if truth == 0.0 {
                        break;
                    }
                    rounds += 1;
                    if rounds > MAX_WHILE_ROUNDS {
                        return Err(format!(
                            "a `while` did not finish within {MAX_WHILE_ROUNDS} rounds"
                        ));
                    }
                    let flow = execute(
                        body,
                        bindings,
                        assigned,
                        asserts,
                        consts,
                        sizes,
                        registry,
                        scope,
                        imports,
                        depth + 1,
                        true,
                    )?;
                    match flow {
                        Flow::Normal => {}
                        Flow::Break => break,
                        Flow::Return => return Ok(Flow::Return),
                    }
                }
            }
            Statement::Break => return Ok(Flow::Break),
            Statement::Return => return Ok(Flow::Return),
            // A `when` is lifted out of the section before the rest of
            // it is executed, so nothing should arrive here.
            Statement::When(_) => {
                return Err("a `when` may sit at the top of a model's algorithm section, not inside an `if`, a loop or a function".to_string())
            }
        }
    }
    Ok(Flow::Normal)
}

/// Inline a function call: arguments are bound to the inputs, the
/// algorithm's assignments are substituted in order, and the output
/// expression replaces the call.
/// Inline a call in an expression: the value is the first output. A
/// function with several outputs may still be called this way; the
/// rest are computed for nothing and dropped, as the spec allows.
pub(super) fn inline_function(
    class: &ClassDef,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
) -> Result<Expr, String> {
    // A call nothing can inline is left standing, and the run walks the
    // body for itself. Two things cannot be inlined: a loop whose trip
    // count the model decides rather than the compiler, and a
    // recursion that does not come to an end here.
    //
    // Neither is known by looking. A function that leads back to
    // itself unrolls perfectly well when what decides the recursion is
    // settled - the standard library builds an m-phase winding by
    // halving m until it is odd, and with m a parameter every step of
    // that is decidable. So the unrolling is tried, and the depth
    // guard inside is what says it will not come to an end.
    let standing = || Ok(Expr::Call(class.name.clone(), args.to_vec()));
    // Where a body leads back to itself the unrolling is a try rather
    // than a demand: the walk is waiting behind it, so anything the
    // inliner will not do - a loop it cannot unroll, a shape it cannot
    // carry, a recursion with no bottom - means the call stands and
    // the run walks it. For a body that does not lead back to itself
    // there is nothing behind, and a refusal is a refusal.
    let speculative = recursive(class, registry);
    let attempt = inline_function_outputs(class, args, shapes, consts, registry, depth);
    let mut outputs = match attempt {
        Ok(outputs) => outputs,
        Err(why) if why.starts_with(UNDECIDABLE_LOOP) || why.contains(NO_BOTTOM) => {
            return standing()
        }
        Err(_) if speculative => return standing(),
        Err(why) => return Err(why),
    };
    // An unrolling that still holds a call of its own cycle did not
    // reach the bottom: it stopped where the compiler stopped
    // following, and what it left behind is the same call under a pile
    // of conditions. The call is better off standing.
    if outputs
        .iter()
        .any(|(_, value)| holds_unbounded_call(value, registry))
    {
        return standing();
    }
    let value = outputs.remove(0).1;
    // What a function says about its own inverse is checked and then
    // set aside. The nonlinear corrector already solves `f(x) = u` for
    // `x` where an inverse would have said the answer outright, so
    // reaching for one would save work rather than make anything
    // possible - and an annotation nobody reads is still worth
    // refusing when it names something that is not there.
    check_inverse(class, registry)?;
    // A function that said how to differentiate itself is inlined for
    // its value like any other, and keeps its rule beside it - so a
    // body the differentiator could not read is still differentiable.
    match &class.derivative {
        None => Ok(value),
        Some(named) => {
            let rule = derivative_rule(class, named, args, shapes, consts, registry, depth)?;
            Ok(Expr::WithDerivative(
                Box::new(value),
                Box::new(rule.0),
                rule.1,
            ))
        }
    }
}

/// The functions a flat model still calls, and everything they call in
/// turn.
///
/// A call standing in a flat model is one nothing could inline, so its
/// body has to travel with the model for the run to walk. What such a
/// body may hold is narrower than what an inlined one may: the run
/// carries numbers and nothing else, so an array or a string inside one
/// is refused here rather than at the first step of a simulation.
pub(super) fn programs_used(
    model: &Model,
    registry: &HashMap<&str, &ClassDef>,
) -> Result<Vec<ClassDef>, String> {
    let mut wanted: Vec<String> = Vec::new();
    // What the flat model itself calls is named the way the registry
    // knows it: flattening qualified it on the way out.
    let mut look = |expr: &Expr| gather_calls(expr, registry, "", &[], &mut wanted);
    for equation in model.equations.iter().chain(&model.initial_equations) {
        look(&equation.lhs);
        look(&equation.rhs);
    }
    for (condition, _) in &model.asserts {
        look(condition);
    }
    for clause in &model.when_clauses {
        for branch in &clause.branches {
            look(&branch.condition);
            for action in &branch.actions {
                match action {
                    WhenAction::Assign(_, value)
                    | WhenAction::Reinit(_, value)
                    | WhenAction::TupleAssign(_, value) => look(value),
                    WhenAction::Terminate(_) => {}
                    // Unrolled while flattening, so no loop is left.
                    WhenAction::Loop(_) => {}
                }
            }
        }
    }
    // Everything those call, and everything that calls in turn.
    let mut out: Vec<ClassDef> = Vec::new();
    while let Some(name) = wanted.pop() {
        if out.iter().any(|already| already.name == name) {
            continue;
        }
        let class = registry[name.as_str()];
        walkable(class)?;
        // A body names what it calls the way it was written there; the
        // walk looks names up in one table, so they are made to agree.
        let mut carried = (*class).clone();
        carried.algorithm =
            qualified_calls(&class.algorithm, registry, &class.name, &class.imports);
        out.push(carried);
        gather_calls_in_statements(
            &class.algorithm,
            registry,
            &class.name,
            &class.imports,
            &mut wanted,
        );
    }
    Ok(out)
}

/// The same statements with every call to a user function named the
/// way the registry knows it.
fn qualified_calls(
    body: &[Statement],
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
) -> Vec<Statement> {
    let inner = |body: &[Statement]| qualified_calls(body, registry, scope, imports);
    let expr = |e: &Expr| qualified_in(e, registry, scope, imports);
    body.iter()
        .map(|statement| match statement {
            Statement::Assign(target, subscripts, value) => Statement::Assign(
                target.clone(),
                subscripts.iter().map(&expr).collect(),
                expr(value),
            ),
            Statement::TupleAssign(targets, value) => {
                Statement::TupleAssign(targets.clone(), expr(value))
            }
            Statement::Assert(condition, message) => {
                Statement::Assert(expr(condition), message.clone())
            }
            Statement::Call(name, args) => Statement::Call(
                lookup(registry, name, scope, imports)
                    .map(|class| class.name.clone())
                    .unwrap_or_else(|| name.clone()),
                args.iter().map(&expr).collect(),
            ),
            Statement::If(branches) => Statement::If(rebranch(branches, &expr, &inner)),
            Statement::When(branches) => Statement::When(rebranch(branches, &expr, &inner)),
            Statement::For(variable, range, body) => {
                Statement::For(variable.clone(), range.as_ref().map(&expr), inner(body))
            }
            Statement::While(condition, body) => Statement::While(expr(condition), inner(body)),
            Statement::Break => Statement::Break,
            Statement::Return => Statement::Return,
        })
        .collect()
}

/// The branches of an `if` or a `when`, rebuilt through the same two
/// rewrites.
fn rebranch(
    branches: &[StatementBranch],
    expr: &impl Fn(&Expr) -> Expr,
    inner: &impl Fn(&[Statement]) -> Vec<Statement>,
) -> Vec<StatementBranch> {
    branches
        .iter()
        .map(|branch| StatementBranch {
            condition: branch.condition.as_ref().map(expr),
            body: inner(&branch.body),
        })
        .collect()
}

/// The same expression with every call to a user function named the way
/// the registry knows it.
fn qualified_in(
    expr: &Expr,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
) -> Expr {
    let recur = |inner: &Expr| qualified_in(inner, registry, scope, imports);
    match expr {
        Expr::Call(name, args) => {
            let named = lookup(registry, name, scope, imports)
                .filter(|class| class.kind == ClassKind::Function)
                .map(|class| class.name.clone())
                .unwrap_or_else(|| name.clone());
            Expr::Call(named, args.iter().map(recur).collect())
        }
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::If(c, a, b) => Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b))),
        Expr::Range(a, step, b) => Expr::Range(
            Box::new(recur(a)),
            step.as_ref().map(|s| Box::new(recur(s))),
            Box::new(recur(b)),
        ),
        Expr::Array(items) => Expr::Array(items.iter().map(recur).collect()),
        _ => expr.clone(),
    }
}

/// Every user function an expression calls.
fn gather_calls(
    expr: &Expr,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    out: &mut Vec<String>,
) {
    // A body names what it calls the way it was written there, so the
    // name is resolved where it was written before it is filed under
    // the one the registry knows it by.
    if let Expr::Call(name, _) = expr {
        if let Some(class) = lookup(registry, name, scope, imports) {
            if class.kind == ClassKind::Function {
                out.push(class.name.clone());
            }
        }
    }
    match expr {
        Expr::Call(_, args) => args
            .iter()
            .for_each(|arg| gather_calls(arg, registry, scope, imports, out)),
        Expr::WithDerivative(value, rule, seeds) => {
            gather_calls(value, registry, scope, imports, out);
            gather_calls(rule, registry, scope, imports, out);
            seeds
                .iter()
                .for_each(|(_, arg)| gather_calls(arg, registry, scope, imports, out));
        }
        Expr::Neg(inner) | Expr::Not(inner) => gather_calls(inner, registry, scope, imports, out),
        Expr::Bin(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r)
        | Expr::Elementwise(_, l, r) => {
            gather_calls(l, registry, scope, imports, out);
            gather_calls(r, registry, scope, imports, out);
        }
        Expr::If(c, a, b) => {
            gather_calls(c, registry, scope, imports, out);
            gather_calls(a, registry, scope, imports, out);
            gather_calls(b, registry, scope, imports, out);
        }
        _ => {}
    }
}

/// Every user function the statements of a body call.
fn gather_calls_in_statements(
    body: &[Statement],
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    out: &mut Vec<String>,
) {
    for statement in body {
        match statement {
            Statement::Assign(_, subscripts, value) => {
                subscripts
                    .iter()
                    .for_each(|s| gather_calls(s, registry, scope, imports, out));
                gather_calls(value, registry, scope, imports, out);
            }
            Statement::TupleAssign(_, value) => gather_calls(value, registry, scope, imports, out),
            Statement::Assert(condition, _) => {
                gather_calls(condition, registry, scope, imports, out)
            }
            Statement::Call(name, args) => {
                if let Some(class) = lookup(registry, name, scope, imports) {
                    out.push(class.name.clone());
                }
                args.iter()
                    .for_each(|arg| gather_calls(arg, registry, scope, imports, out));
            }
            Statement::If(branches) | Statement::When(branches) => {
                for branch in branches {
                    if let Some(condition) = &branch.condition {
                        gather_calls(condition, registry, scope, imports, out);
                    }
                    gather_calls_in_statements(&branch.body, registry, scope, imports, out);
                }
            }
            Statement::For(_, range, inner) => {
                if let Some(range) = range {
                    gather_calls(range, registry, scope, imports, out);
                }
                gather_calls_in_statements(inner, registry, scope, imports, out);
            }
            Statement::While(condition, inner) => {
                gather_calls(condition, registry, scope, imports, out);
                gather_calls_in_statements(inner, registry, scope, imports, out);
            }
            Statement::Break | Statement::Return => {}
        }
    }
}

/// What a body the run walks may be made of. The run carries numbers,
/// so anything shaped otherwise is refused here rather than left to
/// fail at the first step.
fn walkable(class: &ClassDef) -> Result<(), String> {
    for component in &class.components {
        if !component.dimensions.is_empty() {
            return Err(format!(
                "`{}` is called where nothing could inline it, so the run walks its body - \
                 and `{}` is an array, which a body walked at run time cannot hold",
                class.name, component.name
            ));
        }
        if component.type_name == "String" {
            return Err(format!(
                "`{}` is called where nothing could inline it, so the run walks its body - \
                 and `{}` is a String, which no step carries",
                class.name, component.name
            ));
        }
    }
    if class
        .components
        .iter()
        .filter(|c| c.causality == Causality::Output)
        .count()
        != 1
    {
        return Err(format!(
            "`{}` is called where nothing could inline it, so the run walks its body - \
             and a body walked at run time gives one output, not {}",
            class.name,
            class
                .components
                .iter()
                .filter(|c| c.causality == Causality::Output)
                .count()
        ));
    }
    Ok(())
}

/// Check what a function says about its own inverse: the function it
/// names has to be there, the input it claims to solve for has to be
/// one of its own, and what it hands the inverse has to be something it
/// has to hand.
fn check_inverse(class: &ClassDef, registry: &HashMap<&str, &ClassDef>) -> Result<(), String> {
    let named = |wanted: &str, causality: Causality| {
        class
            .components
            .iter()
            .any(|component| component.name == wanted && component.causality == causality)
    };
    for (solved_for, called, arguments) in &class.inverse {
        if lookup(registry, called, &class.name, &class.imports).is_none() {
            return Err(format!(
                "`{}` says `{called}` inverts it, and there is no such function",
                class.name
            ));
        }
        if !named(solved_for, Causality::Input) {
            return Err(format!(
                "`{}` says its inverse solves for `{solved_for}`, which is not one of its \
                 inputs",
                class.name
            ));
        }
        for argument in arguments {
            if !named(argument, Causality::Input) && !named(argument, Causality::Output) {
                return Err(format!(
                    "the inverse of `{}` is handed `{argument}`, which `{}` neither takes \
                     nor gives",
                    class.name, class.name
                ));
            }
        }
    }
    Ok(())
}

/// Inline the function a `derivative` annotation names, leaving a name
/// where each argument's own derivative belongs.
///
/// The derivative function takes what the original takes and then one
/// more for each of those, in the same order - so the second half of
/// its inputs is what the rule is a rule about.
#[allow(clippy::too_many_arguments)]
fn derivative_rule(
    class: &ClassDef,
    named: &str,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
) -> Result<(Expr, Vec<(String, Expr)>), String> {
    let of = lookup(registry, named, &class.name, &class.imports).ok_or_else(|| {
        format!(
            "`{}` says its derivative is `{named}`, and there is no such function",
            class.name
        )
    })?;
    let inputs = |class: &ClassDef| {
        class
            .components
            .iter()
            .filter(|component| component.causality == Causality::Input)
            .count()
    };
    let (given, wanted) = (inputs(of), 2 * args.len());
    if given != wanted {
        return Err(format!(
            "`{named}` is the derivative of `{}`, so it takes {wanted} inputs - what `{}` \
             takes, and then one derivative for each - but it takes {given}",
            class.name, class.name
        ));
    }
    // The names standing in for the derivatives are the compiler's own,
    // so nothing a model can write collides with them.
    let seeds: Vec<(String, Expr)> = args
        .iter()
        .enumerate()
        .map(|(index, argument)| (format!("$seed{index}"), argument.clone()))
        .collect();
    let handed: Vec<Expr> = args
        .iter()
        .cloned()
        .chain(seeds.iter().map(|(name, _)| Expr::Ref(name.clone())))
        .collect();
    let shapes: Vec<Vec<i64>> = shapes.iter().chain(shapes.iter()).cloned().collect();
    let rule = inline_function(of, &handed, &shapes, consts, registry, depth + 1)?;
    Ok((rule, seeds))
}

/// Execute a function body symbolically and return every output, in
/// declaration order, as `(name, expression)`. Arguments are matched
/// positionally, then by name (`f(x, precision = 6)`); an input left
/// unmatched falls back to its declared default.
pub(super) fn inline_function_outputs(
    class: &ClassDef,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
) -> Result<Vec<(String, Expr)>, String> {
    let mut checks = Vec::new();
    let outputs = inline_body(class, args, shapes, consts, registry, depth, &mut checks)?;
    // An `assert` in a function body would have to travel out through
    // the expression the call becomes, and expressions have nowhere to
    // carry one.
    if !checks.is_empty() {
        return Err(format!(
            "`assert` in function `{}` has nowhere to go: a call becomes an expression, and \
             an expression carries no checks - one written among a model's own statements does",
            class.name
        ));
    }
    Ok(outputs)
}

/// The checks a call makes, for a call that stands on its own as a
/// statement. Nothing receives its outputs, so what is left of it is
/// what its body asserted - and here, unlike in an expression, there
/// is somewhere for that to go.
pub(super) fn inline_function_checks(
    class: &ClassDef,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
) -> Result<Vec<(Expr, String)>, String> {
    // A body written outside Modelica that answers with nothing is
    // there for what it does rather than for what it says:
    // `Streams.print(...)` writes a line on the terminal. There is no
    // terminal here and no value to miss, so the call does nothing and
    // the run is the same run. A body that does answer is another
    // matter - its value is wanted, and it is refused below.
    if class.external
        && !class
            .components
            .iter()
            .any(|c| c.causality == Causality::Output)
    {
        return Ok(Vec::new());
    }
    let mut checks = Vec::new();
    inline_body(class, args, shapes, consts, registry, depth, &mut checks)?;
    Ok(checks)
}

/// Run a function body and give back what each output came to, with
/// the checks the body made collected into `checks`.
fn inline_body(
    class: &ClassDef,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
    checks: &mut Vec<(Expr, String)>,
) -> Result<Vec<(String, Expr)>, String> {
    if depth > MAX_DEPTH {
        return Err(format!("`{}` {NO_BOTTOM}", class.name));
    }
    // `external "builtin" y = asin(u)` says the function is the
    // operator the language already has, given a place in a library's
    // tree. The call becomes a call to the operator, arguments in the
    // order they were written.
    if let Some(builtin) = &class.builtin {
        let output = class
            .components
            .iter()
            .find(|c| c.causality == Causality::Output)
            .ok_or_else(|| format!("function `{}` declares no output", class.name))?;
        return Ok(vec![(
            output.name.clone(),
            Expr::Call(builtin.clone(), args.to_vec()),
        )]);
    }
    // A function whose body is written in C is read as far as its
    // declaration and no further: there is nothing here to inline, and
    // nothing to walk at run time either.
    if class.external {
        return Err(format!(
            "`{}` has a body written outside Modelica, which this compiler cannot run",
            class.name
        ));
    }
    let inputs: Vec<&Component> = class
        .components
        .iter()
        .filter(|c| c.causality == Causality::Input)
        .collect();
    let outputs: Vec<&Component> = class
        .components
        .iter()
        .filter(|c| c.causality == Causality::Output)
        .collect();
    if outputs.is_empty() {
        return Err(format!("function `{}` declares no output", class.name));
    }
    let mut bindings: HashMap<String, Expr> = HashMap::new();
    let mut given_shapes: HashMap<String, Vec<i64>> = HashMap::new();
    let mut named_seen = false;
    let mut position = 0;
    for (index, arg) in args.iter().enumerate() {
        if let Expr::NamedArg(name, value) = arg {
            if !inputs.iter().any(|input| &input.name == name) {
                return Err(format!(
                    "function `{}` has no input named `{name}`",
                    class.name
                ));
            }
            if bindings.insert(name.clone(), (**value).clone()).is_some() {
                return Err(format!(
                    "argument `{name}` of function `{}` is given twice",
                    class.name
                ));
            }
            named_seen = true;
        } else {
            if named_seen {
                return Err(format!(
                    "function `{}`: positional arguments must come before named ones",
                    class.name
                ));
            }
            let Some(input) = inputs.get(position) else {
                return Err(format!(
                    "function `{}` expects {} argument(s), got more",
                    class.name,
                    inputs.len()
                ));
            };
            // A `[:]` input is as long as whatever was handed to it.
            if !input.dimensions.is_empty() {
                if let Some(shape) = shapes.get(index) {
                    if !shape.is_empty() {
                        given_shapes.insert(input.name.clone(), shape.clone());
                        // The body reads the argument by the caller's
                        // name once the binding is substituted in, so
                        // `size(x, 1)` becomes `size(s.i, 1)` and has
                        // to find the length under that name too.
                        if let Expr::Ref(given) = arg {
                            given_shapes.insert(given.clone(), shape.clone());
                        }
                    }
                }
            }
            // A record input arrives as its fields, and the body reads
            // them by name: `c1.re` has to be bound, not `c1`.
            if let Some(fields) = record_input_fields(registry, class, input) {
                if let Expr::Array(items) = arg {
                    if items.len() != fields.len() {
                        return Err(format!(
                            "function `{}` wants {} field(s) for `{}`, got {}",
                            class.name,
                            fields.len(),
                            input.name,
                            items.len()
                        ));
                    }
                    for (field, value) in fields.iter().zip(items) {
                        bindings.insert(format!("{}.{field}", input.name), value.clone());
                    }
                    position += 1;
                    continue;
                }
            }
            bindings.insert(input.name.clone(), arg.clone());
            position += 1;
        }
    }
    // Whatever the call left unsaid falls back to the input's own
    // default. Defaults may name earlier inputs, so they are resolved
    // against what is already bound.
    for input in &inputs {
        // A record was bound field by field, so its own name is not
        // among the bindings and it is not missing either.
        let field_prefix = format!("{}.", input.name);
        let bound = bindings.contains_key(&input.name)
            || bindings.keys().any(|name| name.starts_with(&field_prefix));
        if !bound {
            let Some(default) = &input.binding else {
                return Err(format!(
                    "function `{}` is missing its argument `{}`",
                    class.name, input.name
                ));
            };
            let default = substitute_refs(default, &bindings);
            bindings.insert(input.name.clone(), default);
        }
    }
    for component in &class.components {
        if component.causality == Causality::None {
            if let Some(binding) = &component.binding {
                bindings.insert(component.name.clone(), binding.clone());
            }
        }
    }
    let mut assigned = Vec::new();
    // The lengths the call decided go in first: a declared dimension
    // that is a colon measures nothing on its own, and a result sized
    // `size(v, 1)` reads its length back out of here.
    let mut sizes: HashMap<String, Vec<i64>> = given_shapes;
    collect_shapes(registry, class, consts, &mut sizes, 0);
    // `Return` is simply an early landing here; the outputs are read
    // out the same way. A `break` with no loop has nowhere to go.
    if execute(
        &class.algorithm,
        &mut bindings,
        &mut assigned,
        checks,
        consts,
        &sizes,
        registry,
        &class.name,
        &class.imports,
        depth + 1,
        false,
    )? == Flow::Break
    {
        return Err(format!(
            "`break` outside of a loop in function `{}`",
            class.name
        ));
    }
    outputs
        .iter()
        .map(|output| {
            let name = &output.name;
            // A whole-array assignment bound the name itself;
            // per-element assignments bound `c[1]`, `c[2]`, ... -
            // gather them in order.
            if let Some(expr) = bindings.get(name) {
                return Ok((name.clone(), expr.clone()));
            }
            if let Some(dimensions) = sizes.get(name) {
                let items = index_tuples(dimensions)
                    .into_iter()
                    .map(|indices| {
                        let element = element_name(name, &indices);
                        bindings.get(&element).cloned().ok_or_else(|| {
                            format!(
                                "function `{}` never assigns `{element}` of its output",
                                class.name
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                return Ok((name.clone(), Expr::Array(items)));
            }
            // A record-typed output built up field by field - `v.x :=`,
            // `v.y :=`, as an operator record's constructor does - is
            // gathered into the record value its fields make.
            if let Some(record) = lookup(registry, &output.type_name, &class.name, &class.imports)
                .filter(|c| c.kind == ClassKind::Record)
            {
                let fields = record_fields(record)
                    .into_iter()
                    .map(|field| {
                        let member = format!("{name}.{field}");
                        bindings.get(&member).cloned().ok_or_else(|| {
                            format!(
                                "function `{}` never assigns `{member}` of its output",
                                class.name
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                return Ok((name.clone(), Expr::Array(fields)));
            }
            Err(format!(
                "function `{}` never assigns its output `{name}`",
                class.name
            ))
        })
        .collect()
}

/// The fields of a record-typed argument of a function, when it is one.
pub(super) fn record_input_fields(
    registry: &HashMap<&str, &ClassDef>,
    function: &ClassDef,
    input: &Component,
) -> Option<Vec<String>> {
    let of = lookup(
        registry,
        &input.type_name,
        &function.name,
        &function.imports,
    )?;
    (of.kind == ClassKind::Record).then(|| record_fields(of))
}
