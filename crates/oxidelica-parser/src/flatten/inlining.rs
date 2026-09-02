//! Inlining a call: the body run in place of it, what it answers
//! with put where the call stood, and what it came to remembered for
//! the next asking.
//!
//! Carved out of `algorithms` unchanged.

use super::*;
use std::cell::RefCell;

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
    // A body is inlined into a body into a body: the media library
    // asks a property of a property of a state, and each step starts
    // its own count. What is nested this deep did not come to an end
    // by inlining, so the call is left standing and the run walks it.
    if algorithms::INLINING.with(|deep| deep.get()) > algorithms::MAX_NESTED_CALLS {
        return Ok(Expr::Call(class.name.clone(), args.to_vec()));
    }
    let _nested = algorithms::Nested::deeper();
    let standing = || Ok(Expr::Call(class.name.clone(), args.to_vec()));
    // Where a body leads back to itself the unrolling is a try rather
    // than a demand: the walk is waiting behind it, so anything the
    // inliner will not do - a loop it cannot unroll, a shape it cannot
    // carry, a recursion with no bottom - means the call stands and
    // the run walks it. For a body that does not lead back to itself
    // there is nothing behind, and a refusal is a refusal.
    let speculative = recursive(class, registry);
    // A call that ends up standing was not inlined, so whatever checks
    // the attempt set aside are checks of a body nobody ran.
    let mark = checks_mark();
    let standing = || {
        checks_rewind(mark);
        standing()
    };
    let attempt = inline_function_outputs(class, args, shapes, consts, registry, depth);
    let mut outputs = match attempt {
        Ok(outputs) => outputs,
        // A body that leaves by a `break` or a `return` the compiler
        // cannot decide is one it cannot write out at all: which
        // statements run is what the leaving decides. Walking it is
        // the answer, so the call stands - unless a walk could not
        // carry what the body takes or answers with, and then the
        // refusal is a refusal after all.
        Err(why)
            if why.starts_with(UNDECIDABLE_LOOP)
                || why.contains(NO_BOTTOM)
                || (why.contains(UNDECIDABLE_LEAVING) && walkable(class, registry).is_ok()) =>
        {
            return standing()
        }
        Err(_) if speculative => return standing(),
        Err(why) => {
            checks_rewind(mark);
            return Err(why);
        }
    };
    // An unrolling that still holds a call of its own cycle did not
    // reach the bottom: it stopped where the compiler stopped
    // following, and what it left behind is the same call under a pile
    // of conditions. The call is better off standing.
    if outputs
        .iter()
        .any(|(_, value)| algorithms::holds_unbounded_call(value, registry))
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
                    // A call on its own names a body to be walked
                    // the same way one inside an expression does.
                    WhenAction::Call(name, args) => {
                        // The call itself is named the way an
                        // expression's would be, so it goes through the
                        // same gathering rather than round it.
                        look(&Expr::Call(name.clone(), args.clone()));
                    }
                    // A check made at the event may call as freely as
                    // any other expression.
                    WhenAction::Assert(condition, _) => look(condition),
                    WhenAction::Terminate(_) => {}
                    // Taken apart while flattening, so neither a loop
                    // nor a choice is left.
                    WhenAction::Loop(_) | WhenAction::Choice(_) => {}
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
        // A specialized copy is not in the registry: it was made for
        // this model out of a function that was handed another
        // function, and it lives where such copies are kept.
        let made;
        let class = match registry.get(name.as_str()) {
            Some(held) => *held,
            None => match super::statements::specialization(&name) {
                Some(copy) => {
                    made = copy;
                    &made
                }
                None => continue,
            },
        };
        walkable(class, registry)?;
        // A body names what it calls the way it was written there; the
        // walk looks names up in one table, so they are made to agree.
        let mut carried = (*class).clone();
        // And what it inherits travels with it: the walk reads a
        // frame of names, and an input declared in a base is a name
        // like any other. A body written `extends partialScalarFunction`
        // reads `u` and writes `y`, and neither is declared here.
        carried.components = with_inherited_components(class, registry);
        // A local declared `constant Real eps = Modelica.Constants.eps`
        // is a name the walk's frame has never heard: it works out a
        // local's binding against its own frame, and a constant of
        // another package is nobody there. The lengths already get
        // this treatment; the bindings need it too.
        for held in &mut carried.components {
            for written in [&mut held.binding, &mut held.start].into_iter().flatten() {
                *written =
                    substitute_class_constants(written, registry, &class.name, &class.imports, &[]);
            }
        }
        let renamed = records_as_arrays(&mut carried, registry);
        carried.algorithm = qualified_calls(
            &class.algorithm,
            registry,
            &class.name,
            &class.imports,
            &renamed,
        );
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

/// Every record a body deals in written as an array of its members.
///
/// A walk carries numbers under names, and an array is those names
/// subscripted - `v[2]`. A record is the same thing under another
/// spelling, so it is given that spelling here: `bpro.cp` becomes
/// `bpro[7]`, in the order the record declared its members. Nothing in
/// the walk then has to know what a record is.
///
/// Only a record of plain numbers, though. One holding an array or
/// another record would need more than a name and a subscript, and is
/// left as it was for the walk to refuse by name.
fn records_as_arrays(
    class: &mut ClassDef,
    registry: &HashMap<&str, &ClassDef>,
) -> HashMap<String, Expr> {
    let mut renamed: HashMap<String, Expr> = HashMap::new();
    for component in &mut class.components {
        let Some(of) = lookup(registry, &component.type_name, &class.name, &class.imports)
            .filter(|of| of.kind == ClassKind::Record)
        else {
            continue;
        };
        let members = record_fields(of);
        let plain = of
            .components
            .iter()
            .all(|member| member.dimensions.is_empty() && is_primitive(&member.type_name));
        if !plain || members.is_empty() {
            continue;
        }
        for (index, member) in members.iter().enumerate() {
            renamed.insert(
                format!("{}.{member}", component.name),
                Expr::Ref(format!("{}[{}]", component.name, index + 1)),
            );
        }
        component.type_name = "Real".to_string();
        component.dimensions = vec![Expr::Number(members.len() as f64)];
    }
    renamed
}

/// The same statements with every call to a user function named the
/// way the registry knows it.
fn qualified_calls(
    body: &[Statement],
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    renamed: &HashMap<String, Expr>,
) -> Vec<Statement> {
    let inner = |body: &[Statement]| qualified_calls(body, registry, scope, imports, renamed);
    let expr = |e: &Expr| substitute_refs(&qualified_in(e, registry, scope, imports), renamed);
    // A member of a record is written as an element of an array, and a
    // statement may be filling one.
    let target = |name: &String| match renamed.get(name) {
        Some(Expr::Ref(instead)) => instead.clone(),
        _ => name.clone(),
    };
    body.iter()
        .map(|statement| match statement {
            Statement::Assign(name, subscripts, value) => Statement::Assign(
                target(name),
                subscripts.iter().map(&expr).collect(),
                expr(value),
            ),
            Statement::TupleAssign(targets, value) => Statement::TupleAssign(
                targets
                    .iter()
                    .map(|slot| {
                        slot.as_ref()
                            .map(|(name, subs)| (target(name), subs.clone()))
                    })
                    .collect(),
                expr(value),
            ),
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
pub(super) fn gather_calls(
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
        // A specialized copy is named after what went into it and is
        // not in the registry: it was made for this model out of a
        // function handed another function.
        } else if super::statements::specialization(name).is_some() {
            out.push(name.clone());
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
        // `f(x)[2]` - a call answering with several numbers, asked for
        // one of them. The call is under the subscript.
        Expr::Index(base, _) => gather_calls(base, registry, scope, imports, out),
        _ => {}
    }
}

/// Every user function the statements of a body call.
pub(super) fn gather_calls_in_statements(
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
                    // A body with no outputs answers nothing, so a
                    // walk has nothing to do with it: the standard
                    // library shouts through `Streams.error` and
                    // `print`, and those take a String and give back
                    // nothing at all. Carried along, they fail the
                    // walkability check and take the whole model with
                    // them - for a branch that may never be taken.
                    // Inlining already treats such a body as nothing
                    // (see `an external body with no outputs`); this
                    // is the same rule where bodies are carried.
                    let answers = class
                        .components
                        .iter()
                        .any(|held| held.causality == Causality::Output);
                    if answers {
                        out.push(class.name.clone());
                    }
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
pub(super) fn walkable(
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
) -> Result<(), String> {
    for component in &class.components {
        // An array goes in, is held while the walk runs, and may come
        // back: a body answering with several numbers is asked once for
        // each of them. Only a length the compiler can see, though -
        // the model has to name every element it takes.
        if component.causality == Causality::Output && !component.dimensions.is_empty() {
            // A length written as a constant of the package the
            // function belongs to counts as one the compiler can see:
            // a random generator answers with `state[nState]`, and
            // `nState` is a number the package states outright.
            let settled = |dimension: &Expr| -> bool {
                let named = substitute_class_constants(
                    dimension,
                    registry,
                    &class.name,
                    &class.imports,
                    &[],
                );
                const_eval(&named, &HashMap::new()).is_some()
            };
            let settled_length = match component.dimensions.as_slice() {
                [only] => settled(only),
                _ => false,
            };
            let [Expr::Number(_)] = component.dimensions.as_slice() else {
                if settled_length {
                    continue;
                }
                return Err(format!(
                    "`{}` is called where nothing could inline it, so the run walks its body \
                     - and it answers with `{}`, whose length is not one the compiler can \
                     see",
                    class.name, component.name
                ));
            };
        }
        if component.type_name == "String" {
            return Err(format!(
                "`{}` is called where nothing could inline it, so the run walks its body - \
                 and `{}` is a String, which no step carries",
                class.name, component.name
            ));
        }
    }
    // A body may answer with several numbers, and the call asks for
    // the one it wants: `(d, T) := dTofph(...)` takes both, `f(x)[2]`
    // the second. What the run cannot carry is a mixture of shapes -
    // the numbers of the answer are laid out one after another, and
    // an array among them would need a length at every call site to
    // say where the next one starts.
    // What a function declares, its bases included: the standard
    // library writes `extends partialScalarFunction` and gets its `u`
    // and its `y` from there, declaring only the extra inputs it
    // wants. Looking at the class's own components alone, such a
    // function answers with nothing and is refused for it.
    let held = with_inherited_components(class, registry);
    let outputs: Vec<&Component> = held
        .iter()
        .filter(|c| c.causality == Causality::Output)
        .collect();
    match outputs.len() {
        0 => {
            return Err(format!(
                "`{}` is called where nothing could inline it, so the run walks its body - \
                 and a body walked at run time answers with something, not nothing",
                class.name
            ))
        }
        1 => {}
        several => {
            if let Some(spread) = outputs.iter().find(|c| !c.dimensions.is_empty()) {
                return Err(format!(
                    "`{}` is called where nothing could inline it, so the run walks its \
                     body - and it answers with {several} things, of which `{}` is an \
                     array: the run lays the answers end to end, and cannot say where \
                     one of unknown length leaves off",
                    class.name, spread.name
                ));
            }
        }
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
    // What a function takes may be written in a base of it:
    // `redeclare function extends saturationPressure` says only what
    // the body is, and the base says what goes in. Counting a class's
    // own declarations alone makes such a function take nothing, and
    // its derivative is then refused for taking too much.
    let inputs = |class: &ClassDef| {
        function_components(registry, class, 0)
            .iter()
            .filter(|component| component.causality == Causality::Input)
            .count()
    };
    // Only what can be differentiated gets a derivative handed to it.
    // A table is asked for a value by `(tableID, column, u)`, and
    // neither the table nor the column has a rate of change: the
    // derivative function takes the three and then `der_u` alone.
    let differentiable: Vec<bool> = function_components(registry, class, 0)
        .iter()
        .filter(|component| component.causality == Causality::Input)
        .map(|component| is_real(registry, component, &class.name, &class.imports))
        .collect();
    let seeded = differentiable.iter().filter(|real| **real).count();
    let (given, wanted) = (inputs(of), args.len() + seeded);
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
        .filter(|(index, _)| differentiable.get(*index).copied().unwrap_or(true))
        .map(|(index, argument)| (format!("$seed{index}"), argument.clone()))
        .collect();
    let handed: Vec<Expr> = args
        .iter()
        .cloned()
        .chain(seeds.iter().map(|(name, _)| Expr::Ref(name.clone())))
        .collect();
    let seeded_shapes: Vec<Vec<i64>> = args
        .iter()
        .enumerate()
        .filter(|(index, _)| differentiable.get(*index).copied().unwrap_or(true))
        .filter_map(|(index, _)| shapes.get(index).cloned())
        .collect();
    let shapes: Vec<Vec<i64>> = shapes.iter().cloned().chain(seeded_shapes).collect();
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
    if outputs.is_empty() {
        return Err(format!("function `{}` declares no output", class.name));
    }
    // What the body answers with is written in the terms of the
    // package it was written in: `reference_h + (T - reference_T)*
    // cp_const` names constants of the medium, and inside the medium a
    // bare name is the right way to say it. Inlined into the model
    // nothing encloses them any more and the walk outwards has no
    // medium to walk to - so they travel with the body, substituted
    // here, where the package is still known.
    //
    // Only while a parameter is being settled, and only where a name
    // is left standing at all. A constant carries a unit and the
    // number replacing it does not, so folding one into an equation
    // reads kelvin against joules per kilogram and refuses a sound
    // model; a parameter wants the digit and has no such reader.
    let outputs = match constants::SETTLING_PARAMETER.with(|on| on.get()) {
        false => outputs,
        true => outputs
            .into_iter()
            .map(|(name, value)| {
                let value = match holds_a_name(&value) {
                    false => value,
                    true => constants::substitute_class_constants(
                        &value,
                        registry,
                        &class.name,
                        &class.imports,
                        &[],
                    ),
                };
                (name, value)
            })
            .collect(),
    };
    // An `assert` in a function body cannot travel out through the
    // expression the call becomes, so it is set aside for the model
    // being built to take up.
    if !checks.is_empty() {
        algorithms::SET_ASIDE.with(|aside| aside.borrow_mut().extend(checks));
    }
    Ok(outputs)
}

/// Whether an expression still names anything, rather than being
/// arithmetic over numbers alone.
fn holds_a_name(expr: &Expr) -> bool {
    let mut found = false;
    expr.for_each(&mut |inner| {
        if matches!(inner, Expr::Ref(_)) {
            found = true;
        }
    });
    found
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

/// What to say about a function whose body is written outside
/// Modelica and which nobody here answers for.
///
/// The declaration says what is called and what it is handed, so the
/// refusal says it too: a name to look for is worth more than the fact
/// that there is one.
pub(super) fn outside_this_language(class: &ClassDef) -> String {
    let Some(call) = &class.external_call else {
        return format!(
            "`{}` has a body written outside Modelica, which this compiler cannot run",
            class.name
        );
    };
    // An argument is nearly always a name, and a name reads better as
    // itself than as the shape it is held in.
    let handed: Vec<String> = call
        .arguments
        .iter()
        .map(|argument| match argument {
            Expr::Ref(name) => name.clone(),
            other => names::sketch(other),
        })
        .collect();
    format!(
        "`{}` is `{}({})`{}, written outside Modelica; this compiler has none of its own for \
         that name, and no outside library was given",
        class.name,
        call.called,
        handed.join(", "),
        match &call.language {
            Some(language) => format!(" in {language}"),
            None => String::new(),
        }
    )
}

/// How many numbers a value written out holds. A matrix arrives as an
/// array of its rows, and what a body written here is handed is the
/// numbers themselves.
fn numbers_in(expr: &Expr) -> usize {
    match expr {
        Expr::Array(items) => items.iter().map(numbers_in).sum(),
        _ => 1,
    }
}

/// The outputs of a body written here, each taking its own place of
/// what the call answers with.
///
/// The answer is one flat list of numbers, in the order the outputs
/// were declared and each written out: an output of two numbers takes
/// two places. A scalar output is one place, and a call asked for one
/// place is written `f(...)[k]` - the same shape a walked body's
/// answer takes, so nothing downstream needs a second rule.
#[allow(clippy::too_many_arguments)]
fn numbered_outputs(
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
    consts: &HashMap<String, f64>,
    given_shapes: &HashMap<String, Vec<i64>>,
    made: &Expr,
    answers: usize,
) -> Result<Vec<(String, Expr)>, String> {
    let place = |which: usize| {
        Expr::Index(
            Box::new(made.clone()),
            vec![Expr::Number(which as f64 + 1.0)],
        )
    };
    let mut outputs = Vec::new();
    let mut taken = 0;
    for output in function_components(registry, class, 0)
        .iter()
        .filter(|c| c.causality == Causality::Output)
    {
        // A length is nearly always a constant of the package the
        // function is written in - `stateOut[nState]` of a generator -
        // or a length the call handed over - `x[size(A, 1)]` of a
        // solver - and neither is a name an environment holds.
        let length = match output.dimensions.as_slice() {
            [] => None,
            [dimension] => {
                let named = substitute_class_constants(
                    dimension,
                    registry,
                    &class.name,
                    &class.imports,
                    &[],
                );
                Some(
                    const_eval(&named, consts)
                        .map(|length| length as i64)
                        .or_else(|| dimension_value(&named, consts, given_shapes))
                        .ok_or_else(|| {
                            format!(
                                "`{}` answers with `{}`, whose length this compiler cannot see",
                                class.name, output.name
                            )
                        })? as usize,
                )
            }
            _ => {
                return Err(format!(
                    "`{}` answers with `{}`, whose shape this compiler cannot see",
                    class.name, output.name
                ))
            }
        };
        let said = match length {
            None => place(taken),
            Some(length) => Expr::Array((0..length).map(|step| place(taken + step)).collect()),
        };
        taken += length.unwrap_or(1);
        outputs.push((output.name.clone(), said));
    }
    if taken != answers {
        return Err(format!(
            "`{}` answers with {taken} number(s), and the body written here answers with \
             {answers}",
            class.name
        ));
    }
    Ok(outputs)
}

/// Whether a declaration is of `Real`, following the aliases a library
/// wraps it in: `SI.Voltage` is a `Real` and so is `Modelica.Units.SI
/// .Time`, while an `Integer`, a `Boolean` or an `ExternalObject` is
/// not. What this decides is which inputs of a function have a rate of
/// change to be handed alongside them.
fn is_real(
    registry: &HashMap<&str, &ClassDef>,
    component: &Component,
    scope: &str,
    imports: &[(String, String)],
) -> bool {
    let mut named = component.type_name.clone();
    // Each step of the chain is a name written inside the class that
    // came before it: `SI.Temperature` is `ThermodynamicTemperature`
    // as `Modelica.Units.SI` spells it, and looking that up from where
    // the declaration stands finds nothing. So the place to look moves
    // along with the name.
    let mut scope = scope.to_string();
    let mut imports = imports.to_vec();
    for _ in 0..MAX_DEPTH {
        if named == "Real" {
            return true;
        }
        let Some(of) = lookup(registry, &named, &scope, &imports) else {
            return false;
        };
        let Some(alias) = &of.alias_of else {
            return false;
        };
        named = alias.0.clone();
        scope = of.name.clone();
        imports = of.imports.clone();
    }
    false
}

/// Every declaration of a function, its bases' first.
///
/// A function may say only what it does - `redeclare function extends
/// bubbleEnthalpy` - and leave what it takes and answers with to the
/// one it extends. The base's declarations come first, since that is
/// the order the arguments are given in.
/// The body a function runs: its own, or the one it inherited.
///
/// A function may say what it takes and answers with in one class and
/// how it works in another - `loadResource` of the standard library
/// extends the declaration from one base and the algorithm from a
/// second. A class that writes an algorithm of its own means that
/// one; a class that writes none runs the first it inherits.
fn function_body<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    class: &'a ClassDef,
    depth: usize,
) -> &'a [Statement] {
    // A function the base declared and left for another to write is
    // written in the class it was asked under: `PartialMedium` states
    // `dynamicViscosity` and every medium redeclares it with a body.
    // Asked under the medium, the medium's own is the one that runs.
    // A function left deliberately unwritten is looked for in the
    // class it was asked under: `partial` is how a base says the body
    // belongs to whoever redeclares it, and `PartialMedium` states
    // `dynamicViscosity` for every medium to write. The redeclaration
    // may write it outright or take it from a base of its own, which
    // is what `redeclare function extends` says.
    if class.algorithm.is_empty() && class.partial && depth == 0 {
        let under = asked_under(class);
        if under != class.name {
            if let Some(tail) = class.name.rsplit_once('.').map(|(_, tail)| tail) {
                // Written by the class asked under, or inherited by
                // it: a medium says `redeclare function extends
                // setState_pTX` in a package of its own, and the one
                // that reaches the medium is the one that runs.
                let theirs = lookup(registry, tail, &under, &class.imports);
                if let Some(theirs) = theirs.filter(|found| found.name != class.name) {
                    let body = function_body(registry, theirs, depth + 1);
                    if !body.is_empty() {
                        return body;
                    }
                }
            }
        }
    }
    if !class.algorithm.is_empty() || depth > MAX_DEPTH {
        return &class.algorithm;
    }
    for extend in &class.extends {
        let base = match extend.from_base {
            true => inherited_class(registry, class, &extend.base, 0),
            false => lookup(registry, &extend.base, &class.name, &class.imports),
        };
        if let Some(base) = base {
            let body = function_body(registry, base, depth + 1);
            if !body.is_empty() {
                return body;
            }
        }
    }
    &class.algorithm
}

pub(super) fn function_components(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    depth: usize,
) -> Vec<Component> {
    let mut out = Vec::new();
    if depth > MAX_DEPTH {
        return out;
    }
    for extend in &class.extends {
        let base = match extend.from_base {
            true => inherited_class(registry, class, &extend.base, 0),
            false => lookup(registry, &extend.base, &class.name, &class.imports),
        };
        if let Some(base) = base {
            out.extend(function_components(registry, base, depth + 1));
        }
    }
    for component in &class.components {
        // What the class writes for itself replaces what it inherited
        // of that name rather than joining it.
        out.retain(|kept: &Component| kept.name != component.name);
        out.push(component.clone());
    }
    out
}

/// What a body came to last time it was handed exactly this, for as
/// long as one class is being instantiated.
///
/// A model asks the same question of the same body over and over: the
/// transistor bodies of `Spice3` are written out four million times
/// between them, and a hundred thousand of those askings are
/// different. What a body answers depends on what it was handed, on
/// the shapes it was handed, and on the parameter values in view -
/// and the last of those is what stands still while one class is
/// instantiated, which is why the remembering lives exactly that long.
type Remembered = Result<(Vec<(String, Expr)>, Vec<(Expr, String)>), String>;

thread_local! {
    static INLINED: std::cell::RefCell<HashMap<String, Remembered>> =
        std::cell::RefCell::new(HashMap::new());
    static REMEMBERING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Remember what bodies come to while this stands, and forget it when
/// it falls: one class is being instantiated, and its parameter values
/// do not move while it is.
pub(super) struct Inlined(HashMap<String, Remembered>, bool);

impl Inlined {
    pub(super) fn open() -> Self {
        // The expansions of the class above are answers to questions
        // asked under its numbers, not this one's.
        super::arrays::EXPANDED.with(|held| held.borrow_mut().clear());
        let held = INLINED.with(|held| std::mem::take(&mut *held.borrow_mut()));
        let before = REMEMBERING.with(|on| on.replace(true));
        Inlined(held, before)
    }
}

impl Inlined {
    /// Forget what was remembered, because what a body would fold with
    /// has moved: a parameter this class holds has just been settled,
    /// and a body asked before it was settled may answer differently
    /// now.
    pub(super) fn forget() {
        INLINED.with(|held| held.borrow_mut().clear());
        super::arrays::EXPANDED.with(|held| held.borrow_mut().clear());
    }
}

impl Drop for Inlined {
    fn drop(&mut self) {
        INLINED.with(|held| *held.borrow_mut() = std::mem::take(&mut self.0));
        REMEMBERING.with(|on| on.set(self.1));
    }
}

/// Every name a function body mentions, gathered once and remembered.
///
/// What a body folds with is part of what it comes to, so the answers
/// remembered for one call are only the answers for another where
/// these are worth the same. A model's table of numbers holds
/// thousands and a body reads a handful, so the handful is what is
/// looked up rather than the table compared.
pub(super) fn names_read(class: &ClassDef) -> std::rc::Rc<Vec<String>> {
    thread_local! {
        static READ: std::cell::RefCell<HashMap<String, std::rc::Rc<Vec<String>>>> =
            std::cell::RefCell::new(HashMap::new());
    }
    if let Some(known) = READ.with(|read| read.borrow().get(&class.name).cloned()) {
        return known;
    }
    let mut names: Vec<String> = Vec::new();
    let mut note = |expr: &Expr| {
        algorithms::walk_expr(expr, &mut |node| {
            if let Expr::Ref(name) = node {
                names.push(name.clone());
            }
        });
    };
    for component in &class.components {
        for expr in component.binding.iter().chain(component.start.iter()) {
            note(expr);
        }
        for dimension in &component.dimensions {
            note(dimension);
        }
    }
    for statement in &class.algorithm {
        statement_expressions(statement, &mut note);
    }
    names.sort();
    names.dedup();
    let names = std::rc::Rc::new(names);
    READ.with(|read| read.borrow_mut().insert(class.name.clone(), names.clone()));
    names
}

/// Every expression a statement holds, its branches and loops among
/// them.
fn statement_expressions(statement: &Statement, note: &mut impl FnMut(&Expr)) {
    fn branches(branches: &[StatementBranch], note: &mut impl FnMut(&Expr)) {
        for branch in branches {
            if let Some(condition) = &branch.condition {
                note(condition);
            }
            for statement in &branch.body {
                statement_expressions(statement, note);
            }
        }
    }
    match statement {
        Statement::Assign(_, subscripts, value) => {
            for subscript in subscripts {
                note(subscript);
            }
            note(value);
        }
        Statement::TupleAssign(targets, value) => {
            for subscript in targets.iter().flatten().flat_map(|(_, at)| at) {
                note(subscript);
            }
            note(value);
        }
        Statement::If(inner) | Statement::When(inner) => branches(inner, note),
        Statement::For(_, over, body) => {
            if let Some(over) = over {
                note(over);
            }
            for statement in body {
                statement_expressions(statement, note);
            }
        }
        Statement::While(condition, body) => {
            note(condition);
            for statement in body {
                statement_expressions(statement, note);
            }
        }
        Statement::Assert(condition, _) => note(condition),
        Statement::Call(_, args) => {
            for arg in args {
                note(arg);
            }
        }
        Statement::Break | Statement::Return => {}
    }
}

thread_local! {
    /// The name a body was asked for, and the scope it was asked
    /// from: the pair the class alone cannot say.
    ///
    /// A media function is written once in `PartialMedium` and called
    /// as `Medium.prandtlNumber`, where `Medium` is the medium at
    /// hand. Its body then asks for `ThermodynamicState`, and asking
    /// under the class that wrote it lands on the empty record of the
    /// base rather than on the medium's own. Asking under the name it
    /// was called by lands on the medium - which is what the language
    /// means by a redeclaration reaching the functions that use it.
    static ASKED_AS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Let a body be worked out under the name it was asked for, and take
/// that name away again after.
pub(super) struct AskedAs;

impl AskedAs {
    /// The scope the caller worked out the head of the name to mean.
    pub(super) fn under(scope: &str) -> Option<AskedAs> {
        ASKED_AS.with(|held| held.borrow_mut().push(scope.to_string()));
        Some(AskedAs)
    }

    /// The name a written path was asked as, where resolving it walked
    /// out of the class that was named into a base of it.
    ///
    /// `Medium.BaseProperties` resolves to the base that declares
    /// `BaseProperties`, and the medium the model named - the one
    /// place its own functions can be found - is dropped on the way.
    /// This holds on to it, so that a body written in the base can ask
    /// again under the name it was reached by.
    pub(super) fn resolving<'a>(
        written: &str,
        found: &ClassDef,
        registry: &HashMap<&'a str, &'a ClassDef>,
        scope: &str,
        imports: &[(String, String)],
    ) -> Option<AskedAs> {
        let (head, _) = written.rsplit_once('.')?;
        let named = super::lookup::lookup(registry, head, scope, imports)?;
        // Tested against the *resolved* head, not the written one. A
        // component written `Machines.ControlledPump pump` has a head
        // that the found class's full name never starts with, so a
        // test on the written head never fires and the component's own
        // package is pushed - standing over the whole instantiation
        // where a medium is expected.
        if found.name == named.name {
            return None;
        }
        // A class found *inside* the head is not a reason to forget
        // the head. Resolution dropped nothing, true - but what comes
        // one step later does: `MoistAir.BaseProperties` extends the
        // interface's, and walking that `extends` re-enters
        // `PartialMedium.BaseProperties` as a class in its own right,
        // where the medium is gone and every constant answers the
        // interface's default. The head is the package those bases
        // are about to climb out of, so it is remembered here.
        AskedAs::under(&named.name)
    }
}

impl Drop for AskedAs {
    fn drop(&mut self) {
        ASKED_AS.with(|held| {
            held.borrow_mut().pop();
        });
    }
}

/// The record a class keeps a place for, as the class asked under
/// redeclared it.
///
/// `PartialMedium` declares `ThermodynamicState` with no fields, and
/// every medium redeclares it with the ones it uses. A body of the
/// base building that record has to build the medium's, or it builds
/// a record with nothing in it.
pub(super) fn record_asked_under<'a>(
    record: &'a ClassDef,
    registry: &HashMap<&'a str, &'a ClassDef>,
) -> &'a ClassDef {
    // A place kept for another to fill: a record with no fields at
    // all, or one written as `redeclare record extends` - which adds
    // to whatever the class below it says rather than stating the
    // whole. `PartialTwoPhaseMedium` adds `phase` that way, and the
    // medium under it adds the four fields a state is really made
    // of; built as the middle one, a state comes out with the phase
    // and none of the rest.
    let adds_to_another = record.extends.iter().any(|extend| extend.from_base);
    let kept_for_another =
        record.kind == ClassKind::Record && (record.components.is_empty() || adds_to_another);
    let under = asked_under(record);
    let theirs = kept_for_another
        .then(|| record.name.rsplit_once('.'))
        .flatten()
        .filter(|_| under != record.name)
        .and_then(|(_, tail)| lookup(registry, tail, &under, &record.imports))
        .filter(|found| found.kind == ClassKind::Record && found.name != record.name);
    theirs.unwrap_or(record)
}

/// The scope a body is being worked out under: what it was asked as,
/// where that is known, and the class that wrote it otherwise.
/// Whether one class reaches another by extending it, however many
/// steps away.
///
/// The bases are resolved plainly - walking out of the scope rather
/// than through the whole machinery of names - because this is a
/// question about the shape of the registry, and asking it the long
/// way would colour the counts of every model that has no media in
/// it. What is found is remembered for as long as the registry stands.
fn descends_from(registry: &HashMap<&str, &ClassDef>, from: &str, wanted: &str) -> bool {
    super::lookup::kindred_remembers(from, wanted, || reaches(registry, from, wanted, 0))
}

fn reaches(registry: &HashMap<&str, &ClassDef>, from: &str, wanted: &str, depth: usize) -> bool {
    if from == wanted {
        return true;
    }
    if depth > MAX_DEPTH {
        return false;
    }
    let Some(class) = registry.get(from) else {
        return false;
    };
    class.extends.iter().any(|extend| {
        super::lookup::plain_lookup(registry, &extend.base, &class.name)
            .is_some_and(|base| reaches(registry, &base.name, wanted, depth + 1))
    })
}

/// The function a call really means, where the name it was asked
/// under redeclared what the class it resolved to declares.
///
/// A medium redeclares `specificEnthalpy_pT` with a `region` its base
/// never had, and a call written against the medium hands one over.
/// Bound against the base's declaration that argument is an input
/// nothing has; asked under the medium, the medium's own function is
/// what the call means, and everything about it - the inputs, the
/// body - is read from that one class.
pub(super) fn function_asked_under<'a>(
    class: &'a ClassDef,
    registry: &HashMap<&'a str, &'a ClassDef>,
) -> &'a ClassDef {
    if class.kind != ClassKind::Function {
        return class;
    }
    let Some(under) = ASKED_AS.with(|held| held.borrow().last().cloned()) else {
        return class;
    };
    let Some((wrote, tail)) = class.name.rsplit_once('.') else {
        return class;
    };
    if under == wrote || !descends_from(registry, &under, wrote) {
        return class;
    }
    super::lookup::lookup(registry, tail, &under, &class.imports)
        .filter(|found| found.kind == ClassKind::Function && found.name != class.name)
        .unwrap_or(class)
}

/// A name that could not be found where it was written, looked for
/// once more under the name the body was reached by.
///
/// The body of a partial medium calls `specificEnthalpy_pT`, which a
/// package extending it declares; in the base there is no such
/// function, and in the medium the model named there is. Only the one
/// name already in hand is tried - nothing is searched for.
pub(super) fn found_under_the_asked_name<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    name: &str,
    scope: &str,
    imports: &[(String, String)],
) -> Option<&'a ClassDef> {
    let under = ASKED_AS.with(|held| held.borrow().last().cloned())?;
    if under == scope {
        return None;
    }
    // Only where the name the body was reached by is a relative of
    // the class the body belongs to. A medium extends the package
    // whose body is being worked out, and what it declares is what
    // that body meant; a name that happens to stand under some
    // unrelated class is not an answer to this call, and guessing one
    // is worse than refusing.
    let package = scope.rsplit_once('.').map(|(head, _)| head)?;
    if !descends_from(registry, &under, package) {
        return None;
    }
    super::lookup::lookup(registry, name, &under, imports)
}

pub(super) fn asked_under(class: &ClassDef) -> String {
    ASKED_AS.with(|held| {
        held.borrow()
            .last()
            .cloned()
            .unwrap_or_else(|| class.name.clone())
    })
}

/// The name a body is being worked out under, or nothing where none
/// was pushed. What a cache of answers has to hold in its key: the
/// same class asked under two media answers two things.
pub(super) fn asked_as_mark() -> String {
    ASKED_AS.with(|held| held.borrow().last().cloned().unwrap_or_default())
}

/// The package on the mark, where it descends from the one that
/// declared what is being asked.
///
/// The guard is the same `descends_from` every other reader of the
/// mark uses: a mark left standing by an unrelated caller says nothing
/// about a name of this package, and answering from it would be a
/// guess dressed as an answer.
pub(super) fn asked_as_package(registry: &HashMap<&str, &ClassDef>, owner: &str) -> Option<String> {
    let under = ASKED_AS.with(|held| held.borrow().last().cloned())?;
    if under == owner || !descends_from(registry, &under, owner) {
        return None;
    }
    registry
        .get(under.as_str())
        .filter(|found| found.kind == ClassKind::Package)?;
    Some(under)
}

/// Run a function body and give back what each output came to, with
/// the checks the body made collected into `checks`.
///
/// Asked the same thing twice, it answers the second from the first.
fn inline_body(
    class: &ClassDef,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
    checks: &mut Vec<(Expr, String)>,
) -> Result<Vec<(String, Expr)>, String> {
    if !REMEMBERING.with(|on| on.get()) {
        return worked_body(class, args, shapes, consts, registry, depth, checks);
    }
    // How deep the asking is belongs to the question: a body that
    // will not come to an end is refused at a depth rather than by
    // what it was handed, and the same asking higher up may be
    // answered.
    //
    // So do the values the body folds with. Counting them was not
    // enough: two instances of the same class hand the same arguments
    // to the same body with the same number of values in view and
    // different values among them, and the second was answered with
    // the first's. Only the names the body actually reads are looked
    // up, since a model's table holds thousands and a body reads a
    // handful.
    let read = names_read(class);
    let mut folded: Vec<(&str, f64)> = read
        .iter()
        .filter_map(|name| Some((name.as_str(), *consts.get(name)?)))
        .collect();
    folded.sort_by_key(|(name, _)| *name);
    // And the name the body was asked under, where it says something
    // the class does not: the body's own records and its `partial`
    // functions are found through that name, so two media sharing one
    // wrapper of their base ask the same class the same arguments and
    // must not be given each other's answer.
    let under = asked_under(class);
    let under = match under == class.name {
        true => String::new(),
        false => under,
    };
    let asked = format!(
        "{}|{under}|{depth}|{folded:?}|{args:?}|{shapes:?}",
        class.name
    );
    if let Some(told) = INLINED.with(|held| held.borrow().get(&asked).cloned()) {
        let (outputs, said) = told?;
        checks.extend(said);
        return Ok(outputs);
    }
    let mut said = Vec::new();
    let answer = worked_body(class, args, shapes, consts, registry, depth, &mut said);
    let told: Remembered = match &answer {
        Ok(outputs) => Ok((outputs.clone(), said.clone())),
        Err(why) => Err(why.clone()),
    };
    INLINED.with(|held| held.borrow_mut().insert(asked, told));
    checks.extend(said);
    answer
}

/// The same, worked out rather than remembered.
fn worked_body(
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
    // A body the language or this compiler already answers for is
    // answered there rather than walked.
    if let Some(answer) = body_written_elsewhere(class, args, shapes, consts, registry)? {
        return Ok(answer);
    }

    // What a function takes and answers with may be written in a base
    // of it: `redeclare function extends bubbleEnthalpy` says only what
    // the body is, and the base says what goes in and comes out.
    let declared = function_components(registry, class, 0);
    let inputs: Vec<&Component> = declared
        .iter()
        .filter(|c| c.causality == Causality::Input)
        .collect();
    let outputs: Vec<&Component> = declared
        .iter()
        .filter(|c| c.causality == Causality::Output)
        .collect();
    let mut bindings: HashMap<String, Expr> = HashMap::new();
    let mut given_shapes: HashMap<String, Vec<i64>> = HashMap::new();
    bind_the_arguments(
        class,
        args,
        shapes,
        &inputs,
        registry,
        &mut bindings,
        &mut given_shapes,
    )?;
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
            // A redeclaration may have filled this one in:
            // `redeclare function f = g(a = 1)` is partial
            // application written where a declaration goes, and the
            // pumps of the standard library ask for their
            // characteristics that way.
            let filled = super::statements::filled_inputs(&class.name).and_then(|held| {
                held.iter()
                    .find(|(name, _)| name == &input.name)
                    .map(|(_, value)| value.clone())
            });
            if let Some(value) = filled {
                bindings.insert(input.name.clone(), value);
                continue;
            }
            let Some(default) = &input.binding else {
                return Err(format!(
                    "function `{}` is missing its argument `{}`",
                    class.name, input.name
                ));
            };
            // A default may name a constant of a class - `input Real
            // eps = 100*Modelica.Constants.eps` is how the standard
            // library keeps a normalization off zero - and the name
            // means what it meant where it was written, not where the
            // call is.
            let default =
                substitute_class_constants(default, registry, &class.name, &class.imports, &[]);
            let default = substitute_refs(&default, &bindings);
            bindings.insert(input.name.clone(), default);
        }
    }
    // What the call handed in, which a local's value may name: the
    // multibody world builds an orientation from an axis vector that
    // way - `Real e_x[3] = if length(n_x) < 1e-10 then {1, 0, 0} else
    // normalize(n_x)` - and left unbound the body carried `n_x` out
    // with it, a name meaning nothing outside the function. A local
    // naming another local is left alone: the array layer reads an
    // element off a name and cannot read one off the list written in
    // its place.
    // The lengths the call decided go in first: a declared dimension
    // that is a colon measures nothing on its own, and a result sized
    // `size(v, 1)` reads its length back out of here.
    let mut sizes: HashMap<String, Vec<i64>> = given_shapes;
    // What the call handed over as numbers: a result declared
    // `Integer[nState]` takes its length from the `nState` it was
    // given, and nowhere else says what that is.
    let given: HashMap<String, f64> = bindings
        .iter()
        .filter_map(|(name, value)| Some((name.clone(), const_eval(value, consts)?)))
        .collect();
    collect_shapes(registry, class, consts, &given, &mut sizes, 0);
    // A length may be a call rather than arithmetic: the polyphase
    // functions size their result
    // `[numberOfSymmetricBaseSystems(m)*(integer(m/...) - 1)]`, where
    // `m` is an input. Arithmetic alone cannot decide that, and the
    // shape it leaves unmeasured is what a `:` in the caller then has
    // nothing to read. Only the declarations the round above could not
    // measure are asked, and only against what the call handed in, so
    // a body whose lengths are plain numbers pays nothing.
    // Substituting the bindings rewrites the body's names into the
    // caller's - `m` becomes `conv.m` - so the numbers a length is read
    // against have to be the caller's, with what the call decided on
    // top of them.
    //
    // Copied only where there is a length left to measure. Nearly
    // every body says its lengths in plain numbers and the round above
    // measures them all, and every one of those was paying for a copy
    // of every number the model had settled.
    let unmeasured = || {
        class
            .components
            .iter()
            .any(|c| !c.dimensions.is_empty() && !sizes.contains_key(&c.name))
    };
    let caller_numbers = match unmeasured() {
        false => HashMap::new(),
        true => {
            let mut numbers = consts.clone();
            numbers.extend(given.iter().map(|(name, value)| (name.clone(), *value)));
            numbers
        }
    };
    for component in &class.components {
        if component.dimensions.is_empty() || sizes.contains_key(&component.name) {
            continue;
        }
        let measured: Option<Vec<i64>> = component
            .dimensions
            .iter()
            .map(|dimension| {
                settled_in_body(
                    dimension,
                    &bindings,
                    &caller_numbers,
                    &sizes,
                    registry,
                    &class.name,
                    &class.imports,
                    depth,
                )
                .filter(|length| length.is_finite() && *length >= 0.0)
                .map(|length| length as i64)
            })
            .collect();
        if let Some(measured) = measured {
            sizes.insert(component.name.clone(), measured);
        }
    }
    let no_loop_vars = HashMap::new();
    let local_shapes = Shapes {
        sizes: &sizes,
        loop_vars: &no_loop_vars,
        consts,
        records: no_records(),
    };
    let mut handed: HashMap<String, Expr> = bindings.clone();
    for component in &class.components {
        if component.causality == Causality::None {
            if let Some(binding) = &component.binding {
                // A local that is one number is read here rather than
                // where it is used: `m = size(x, 1)` of a space-phasor
                // transform is written in terms of an input, and the
                // inputs are bound by now. Left to be read later it
                // would carry the input's name out of the body, and
                // out there the name means nothing.
                //
                // A local that is an array keeps its name, because the
                // array layer reads an element off a name and cannot
                // read one off a list written in its place.
                // A declaration value may name a constant of a class
                // the way a statement may - `Real Tlim = min(T,
                // data.TCRIT)` reads a constant of a record beside the
                // function - and it is read here, where the names mean
                // what they meant to whoever wrote them.
                let binding = &substitute_class_constants(
                    binding,
                    registry,
                    &class.name,
                    &class.imports,
                    &[],
                );
                let bound = match component.dimensions.is_empty() {
                    true => substitute_refs(binding, &bindings),
                    false => substitute_refs(binding, &handed),
                };
                // Where it comes to a number, it is stored as one: the
                // number of base systems of an m-phase winding is a
                // call, and a call is not something arithmetic alone
                // can decide an `if` by. Worked out once here, it is a
                // digit everywhere it is used.
                let bound = match settled_in_body(
                    &bound,
                    &HashMap::new(),
                    consts,
                    &sizes,
                    registry,
                    &class.name,
                    &class.imports,
                    depth,
                ) {
                    Some(number) if component.dimensions.is_empty() => Expr::Number(number),
                    _ => bound,
                };
                // A local array may be read by a later local, so it
                // joins what the next value is written against - but
                // only where its value is a name rather than a list
                // written out, since the array layer reads an element
                // off a name and cannot read one off a list.
                // A local array written out as a list is also bound
                // element by element - `Real e[3] = n; Real z[3] = e`
                // of a body that then reads `z[2]` comes through the
                // array layer as the name `e[2]`, and only an element
                // name answers that.
                // The value may be a call the array layer works out -
                // `Real e_z_aux[3] = cross(e_x, n_y_aux)` of the
                // multibody frames - and until it is worked out there
                // is no element to read.
                let worked = expand(
                    &bound,
                    &local_shapes,
                    registry,
                    &class.name,
                    &class.imports,
                    depth + 1,
                );
                let bound = match (component.dimensions.is_empty(), worked) {
                    (false, Ok(value)) => substitute_refs(&value.into_expr(), &bindings),
                    _ => bound,
                };
                if let Expr::Array(_) = &bound {
                    let mut elements = HashMap::new();
                    by_element(&component.name, &bound, &mut Vec::new(), &mut elements);
                    handed.extend(elements.clone());
                    bindings.extend(elements);
                }
                bindings.insert(component.name.clone(), bound);
            }
        }
    }
    let mut assigned = Vec::new();
    // `Return` is simply an early landing here; the outputs are read
    // out the same way. A `break` with no loop has nowhere to go.
    if statements::execute(
        function_body(registry, class, 0),
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
                // An answer of more than one dimension is an array of
                // arrays, not a flat list: `den2[:, 2]` is read
                // `den2[i, 2]` by whoever was handed it, and a second
                // subscript has nowhere to go in a flat list.
                fn gathered(
                    name: &str,
                    dimensions: &[i64],
                    so_far: &mut Vec<i64>,
                    bindings: &HashMap<String, Expr>,
                    class: &str,
                ) -> Result<Expr, String> {
                    let Some((&length, rest)) = dimensions.split_first() else {
                        let element = element_name(name, so_far);
                        return bindings.get(&element).cloned().ok_or_else(|| {
                            format!("function `{class}` never assigns `{element}` of its output")
                        });
                    };
                    let mut items = Vec::new();
                    for index in 1..=length {
                        so_far.push(index);
                        items.push(gathered(name, rest, so_far, bindings, class)?);
                        so_far.pop();
                    }
                    Ok(Expr::Array(items))
                }
                let mut so_far = Vec::new();
                let items = gathered(name, dimensions, &mut so_far, &bindings, &class.name)?;
                return Ok((name.clone(), items));
            }
            // A record-typed output built up field by field - `v.x :=`,
            // `v.y :=`, as an operator record's constructor does - is
            // gathered into the record value its fields make.
            // Asked under the name the body was called by, the way an
            // input's record is: a medium's `setState_pTX` answers
            // with the state that medium redeclared, not with the
            // empty one its base kept a place for.
            let under = asked_under(class);
            let found = match under == class.name {
                true => lookup(registry, &output.type_name, &class.name, &class.imports),
                false => lookup(registry, &output.type_name, &under, &class.imports)
                    .or_else(|| lookup(registry, &output.type_name, &class.name, &class.imports)),
            };
            if let Some(record) = found.filter(|c| c.kind == ClassKind::Record) {
                // A field the body never assigned may have been given
                // on the declaration instead: `output Complex result(re
                // = re, im = im)` is how a record says what it is made
                // of without an algorithm at all.
                let declared = |field: &str| -> Option<Expr> {
                    let (_, value) = output.modifiers.iter().find(|(given, _)| given == field)?;
                    let value = substitute_class_constants(
                        value,
                        registry,
                        &class.name,
                        &class.imports,
                        &[],
                    );
                    Some(substitute_refs(&value, &bindings))
                };
                let held = record_fields_of(registry, record, 0);
                // A record with no fields at all is a place kept for
                // another to fill: `PartialMedium` declares its state
                // empty on purpose, and a medium redeclares it whole.
                // Answering with an empty list would hand the caller a
                // guess dressed as an answer - and the caller, finding
                // nothing to bind, would say the argument was missing,
                // which names a symptom two steps from its cause.
                if held.is_empty() {
                    return Err(format!(
                        "`{}` answers with `{}`, which declares no fields: it is a record \
                         kept for another to redeclare, and the redeclaration did not \
                         reach here{}",
                        class.name,
                        output.type_name,
                        statements::where_the_names_landed()
                    ));
                }
                let fields = held
                    .into_iter()
                    .map(|field| {
                        let member = format!("{name}.{field}");
                        bindings
                            .get(&member)
                            .cloned()
                            .or_else(|| declared(&field))
                            .ok_or_else(|| {
                                format!(
                                    "function `{}` never assigns `{member}` of its output, and \
                                     its declaration says nothing about it either",
                                    class.name
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                return Ok((name.clone(), Expr::Array(fields)));
            }
            // The one value a declaration may give outright.
            if let Some(value) = &output.binding {
                let value =
                    substitute_class_constants(value, registry, &class.name, &class.imports, &[]);
                return Ok((name.clone(), substitute_refs(&value, &bindings)));
            }
            Err(format!(
                "function `{}` never assigns its output `{name}`",
                class.name
            ))
        })
        .collect()
}

/// A function whose body is not Modelica: `external "builtin"` names
/// an operator the language already has, and `external "C"` names
/// something this compiler either answers for itself or refuses by
/// name.
///
/// `None` where the body is ordinary and the caller walks it.
///
/// Moved out of `worked_body` unchanged.
fn body_written_elsewhere(
    class: &ClassDef,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
) -> Result<Option<Vec<(String, Expr)>>, String> {
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
        return Ok(Some(vec![(
            output.name.clone(),
            Expr::Call(builtin.clone(), args.to_vec()),
        )]));
    }
    // A function whose body is written outside Modelica is read as far
    // as its declaration and no further. Where this compiler answers
    // for the name itself, the call is written as that name and left
    // standing for whoever can work it out; where nobody answers, the
    // refusal says which name was wanted.
    if class.external {
        let Some(call) = class.external_call.as_ref().filter(|call| {
            external::answered_here(&call.called) || crate::outside::written_here(&call.called)
        }) else {
            return Err(outside_this_language(class));
        };
        let made = Expr::Call(call.called.clone(), args.to_vec());
        // A body written here in Rust answers with numbers rather than
        // with a string, and may answer with several: the generators
        // give a value and the state they moved to. Each output takes
        // its own place of that answer, the way a walked body's does.
        let handed: Vec<usize> = args.iter().map(numbers_in).collect();
        if let Some((_, answers)) = crate::outside::shape(&call.called, &handed) {
            // The shapes the call handed over, under the names the
            // declaration knows them by: an answer as long as `size(A,
            // 1)` reads its length back out of here.
            let mut given_shapes: HashMap<String, Vec<i64>> = HashMap::new();
            for (input, shape) in function_components(registry, class, 0)
                .iter()
                .filter(|c| c.causality == Causality::Input)
                .zip(shapes)
            {
                given_shapes.insert(input.name.clone(), shape.clone());
            }
            return numbered_outputs(class, registry, consts, &given_shapes, &made, answers)
                .map(Some);
        }
        if crate::outside::written_here(&call.called) {
            return Err(format!(
                "`{}` is written here, and not for what it was handed: {} argument(s) of {} \
                 number(s) in all",
                call.called,
                handed.len(),
                handed.iter().sum::<usize>()
            ));
        }
        let output = class
            .components
            .iter()
            .find(|c| c.causality == Causality::Output)
            .ok_or_else(|| format!("function `{}` declares no output", class.name))?;
        // An answer declared an array is taken a place at a time: the
        // two-dimensional tables give both ends of their grid at once
        // - `output Real uMin[2]` - and the model holds each end as a
        // parameter of its own. Bound to the call whole, both ends
        // were the whole pair and neither could be worked out.
        if let [Expr::Number(length)] = output.dimensions.as_slice() {
            return Ok(Some(vec![(
                output.name.clone(),
                Expr::Array(
                    (1..=*length as i64)
                        .map(|index| {
                            Expr::Index(Box::new(made.clone()), vec![Expr::Number(index as f64)])
                        })
                        .collect(),
                ),
            )]));
        }
        return Ok(Some(vec![(output.name.clone(), made)]));
    }

    Ok(None)
}

/// What the call handed over, under the names the body knows them by.
///
/// An argument may be given in order or by name; a record arrives as
/// its fields, since the body reads them by name; and a `[:]` input is
/// as long as whatever was handed to it.
///
/// Moved out of `worked_body` unchanged.
#[allow(clippy::too_many_arguments)]
fn bind_the_arguments(
    class: &ClassDef,
    args: &[Expr],
    shapes: &[Vec<i64>],
    inputs: &[&Component],
    registry: &HashMap<&str, &ClassDef>,
    bindings: &mut HashMap<String, Expr>,
    given_shapes: &mut HashMap<String, Vec<i64>>,
) -> Result<(), String> {
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
            //
            // An input declared an array of records is not that: the
            // quasi-RMS of a polyphase system takes `Complex u[:]`, and
            // what arrives is three phasors, not the two fields of one.
            // Taken for fields, three phasors were refused for being
            // three where two were wanted; left whole, the body reads
            // `u[k].re` off them, which is what it was written to do.
            if let Some(fields) = record_fields::record_input_fields(registry, class, input)
                .filter(|_| input.dimensions.is_empty())
            {
                if let Expr::Array(items) = arg {
                    if items.len() != fields.len() {
                        return Err(format!(
                            "function `{}` wants {} field(s) for `{}`, got {}{}",
                            class.name,
                            fields.len(),
                            input.name,
                            items.len(),
                            statements::where_the_names_landed()
                        ));
                    }
                    for (field, value) in fields.iter().zip(items) {
                        let here = format!("{}.{field}", input.name);
                        // A field that is itself an array is bound
                        // element by element as well: the body of an
                        // orientation function reads `R.T[1, 1]`, and
                        // the list bound to `R.T` whole is not
                        // something a name with a subscript can be read
                        // off.
                        by_element(&here, value, &mut Vec::new(), bindings);
                        bindings.insert(here, value.clone());
                    }
                    position += 1;
                    continue;
                }
                // A record handed over by name rather than written out
                // is the commoner way of it: the caller has the record
                // as a variable and passes it whole. Flattening has
                // already taken that variable apart, so its fields are
                // there to be named one by one - and binding the name
                // alone would leave the body reading `p.V` with
                // nothing bound to it, which is a value gone missing
                // rather than a refusal.
                //
                // The name itself is bound too, below: a body may hand
                // the record on to another function whole, and that
                // call wants the record and not its fields.
                //
                // Only the fields that are single numbers. A field
                // with dimensions of its own - an orientation carries
                // a three by three - has a shape the caller knows and
                // this does not, and binding a bare name to it loses
                // the shape and refuses the model further along.
                if let Expr::Ref(given) = arg {
                    for field in record_fields::scalar_record_fields(registry, class, input) {
                        bindings.insert(
                            format!("{}.{field}", input.name),
                            Expr::Ref(format!("{given}.{field}")),
                        );
                    }
                    // A field with dimensions of its own - an
                    // orientation carries a three by three - is bound
                    // element by element as well as whole: a body
                    // reading `R.T[1, 1]` has to find the caller's own
                    // `R1.T[1, 1]` under it, and a name alone is not
                    // something a subscript can be read off here.
                    for (field, shape) in
                        record_fields::shaped_record_fields(registry, class, input)
                    {
                        let here = format!("{}.{field}", input.name);
                        let there = format!("{given}.{field}");
                        for indices in index_tuples(&shape) {
                            let source = Expr::Ref(element_name(&there, &indices));
                            bindings.insert(element_name(&here, &indices), source);
                        }
                        bindings.insert(here, spread_out(&there, &shape, &mut Vec::new()));
                    }
                }
            }
            bindings.insert(input.name.clone(), arg.clone());
            position += 1;
        }
    }

    Ok(())
}

/// The fields of a record-typed argument that are single numbers.
///
/// A field with dimensions is left out: what a caller handed over
/// knows its shape and a bare name written in its place does not, so
/// binding one would turn a matrix into something of no shape at all.
/// Those are still reached through the record's own name.
/// Every element of a value written out as a list, under the name it
/// is bound to: `e` bound to `{a, b}` also binds `e[1]` and `e[2]`,
/// which is how a name with a subscript is read.
fn by_element(name: &str, value: &Expr, so_far: &mut Vec<i64>, out: &mut HashMap<String, Expr>) {
    match value {
        Expr::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                so_far.push(index as i64 + 1);
                by_element(name, item, so_far, out);
                so_far.pop();
            }
        }
        one => {
            out.insert(element_name(name, so_far), one.clone());
        }
    }
}

/// A name of a given shape written out as the list of its elements:
/// `T` of three by three is three rows of three names. Written whole a
/// matrix is a list of lists, the way `T[1, :]` reads a row.
fn spread_out(name: &str, shape: &[i64], so_far: &mut Vec<i64>) -> Expr {
    let Some((&length, rest)) = shape.split_first() else {
        return Expr::Ref(element_name(name, so_far));
    };
    let mut items = Vec::new();
    for index in 1..=length {
        so_far.push(index);
        items.push(spread_out(name, rest, so_far));
        so_far.pop();
    }
    Expr::Array(items)
}

/// A function's components together with the ones it inherits.
///
/// `extends partialScalarFunction` is how the standard library says
/// "this takes a `u` and answers with a `y`", and a function written
/// that way declares only what it adds. The bases come first, in the
/// order they are extended, which is the order the language gives
/// their arguments.
/// Whether a call left standing is one the run can actually make.
///
/// The walk is handed a flat run of numbers and one length per
/// argument, so a scalar or a list of numbers arrives whole and a
/// matrix has nowhere to say where each row starts. Left standing all
/// the same, such a call flattens and then refuses a storey lower,
/// which is worse than the refusal it replaced.
pub(super) fn carried_by_the_run(arguments: &[Expr]) -> bool {
    arguments.iter().all(|argument| match argument {
        // A list of numbers, or rows of equal length: both are laid
        // out one element after another and both dimensions travel
        // with them.
        Expr::Array(items) => items.iter().all(|item| match item {
            Expr::Array(row) => match items.first() {
                Some(Expr::Array(first)) => {
                    row.len() == first.len() && row.iter().all(|one| !matches!(one, Expr::Array(_)))
                }
                _ => false,
            },
            _ => true,
        }),
        Expr::MatrixRows(_) | Expr::Range(..) | Expr::Comprehension(..) => false,
        _ => true,
    })
}

/// The names a function answers with, in the order it declared them:
/// what a call left standing has to be indexed by, since the run lays
/// the answer out in that order and nothing else says where each one
/// is.
pub(super) fn declared_outputs(
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
) -> Vec<String> {
    with_inherited_components(class, registry)
        .into_iter()
        .filter(|held| held.causality == Causality::Output)
        .map(|held| held.name)
        .collect()
}

pub(super) fn with_inherited_components(
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
) -> Vec<Component> {
    fn gather(
        class: &ClassDef,
        registry: &HashMap<&str, &ClassDef>,
        depth: usize,
        out: &mut Vec<Component>,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        for extend in &class.extends {
            // A `redeclare function extends dewDensity` names its base
            // by the same name it has itself, and the base is a
            // neighbour of the package rather than of the function:
            // asked from inside, the search finds this very function
            // and gathers nothing. So the enclosing class answers
            // first, and only then the function's own scope.
            let base = super::inheritance::inherited_class(registry, class, &extend.base, 0)
                .or_else(|| lookup(registry, &extend.base, &class.name, &class.imports))
                .filter(|found| found.name != class.name);
            if let Some(base) = base {
                gather(base, registry, depth + 1, out);
            }
        }
        for held in &class.components {
            out.retain(|already| already.name != held.name);
            out.push(held.clone());
        }
    }
    let mut out = Vec::new();
    gather(class, registry, 0, &mut out);
    out
}
