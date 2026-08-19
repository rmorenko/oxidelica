//! Algorithm sections executed symbolically, and functions inlined
//! at the place they were called.

use super::*;

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
        return Err("algorithm nested deeper than the instantiation limit".to_string());
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
                    let Some(target) = slot else { continue };
                    if !assigned.contains(target) {
                        assigned.push(target.clone());
                    }
                    bindings.insert(target.clone(), output);
                }
            }
            Statement::If(branches) => {
                // A branch that may `break` or `return` cannot be
                // merged symbolically - whether it fires must be known.
                // The conditions are decided and only the taken branch
                // runs, its flow passed on.
                if branches.iter().any(|b| has_flow_control(&b.body)) {
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
                        "a `while` condition must be decidable at compile time: algorithms \
                         are executed symbolically, so the trip count cannot depend on a \
                         simulated variable"
                            .to_string()
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
    let mut outputs = inline_function_outputs(class, args, shapes, consts, registry, depth)?;
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
    if depth > MAX_DEPTH {
        return Err(format!("recursive function `{}`", class.name));
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
    // An `assert` in a function body would have to travel out through
    // the expression the call becomes, and expressions have nowhere to
    // carry one; a model's own section is where it works.
    let mut inner_asserts = Vec::new();
    if execute(
        &class.algorithm,
        &mut bindings,
        &mut assigned,
        &mut inner_asserts,
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
    if !inner_asserts.is_empty() {
        return Err(format!(
            "`assert` in function `{}` has nowhere to go: a call becomes an expression, and \
             an expression carries no checks - one written among a model's own statements does",
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
