//! The bodies a flat model carries out to the run.
//!
//! Almost every function is written out where it is called. The ones
//! that cannot be - a loop the model decides, a recursion with no
//! bottom - are left standing, and this gathers what the run needs to
//! walk them: the bodies themselves, everything they call in turn, and
//! the refusals for what a walk cannot carry.

use super::inlining::*;
use super::*;

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
