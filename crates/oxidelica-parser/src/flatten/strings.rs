//! Strings, settled before the run.
//!
//! A `String` has no value a step can carry: the arrays a solver works
//! on hold numbers, and nothing here needs a string to change while a
//! model runs - a string names a medium, a file, a message. So every
//! one of them is worked out at the end of flattening, and what it
//! leaves behind is a Boolean where it was compared and nothing at all
//! where it was declared.
//!
//! What that buys is the way strings are actually used in Modelica:
//! `parameter String medium = "water"` chosen at the top of a model and
//! read by an `if` further down. What it rules out is a string that
//! only a run could produce.

use super::*;

/// What a pass over the strings settled, for whoever comes after: the
/// text of every `String` name, and what every number-valued name is
/// worth. The tables want both - a file name is a string and a
/// smoothness a number.
pub(super) struct Settled {
    pub texts: HashMap<String, String>,
    pub numbers: HashMap<String, f64>,
}

/// Work out every string in the model and take it out of the equations.
pub(super) fn resolve_strings(model: &mut Model) -> Result<Settled, String> {
    let named: Vec<String> = model
        .components
        .iter()
        .filter(|component| component.type_name == "String")
        .map(|component| component.name.clone())
        .collect();
    // What a number-valued name is worth, so `String(x)` of a constant
    // or a parameter folds to its digits rather than staying an
    // unresolved call.
    let numbers: HashMap<String, f64> = model
        .components
        .iter()
        .filter(|c| {
            matches!(
                c.variability,
                Variability::Constant | Variability::Parameter
            )
        })
        .filter_map(|c| {
            let value = c.binding.as_ref().or(c.start.as_ref())?;
            Some((c.name.clone(), const_eval(value, &HashMap::new())?))
        })
        .collect();
    // A record's fields arrive as equations rather than bindings, so a
    // plain `name = number` is a value too.
    let mut numbers = numbers;
    for equation in &model.equations {
        if let (Expr::Ref(name), Some(value)) =
            (&equation.lhs, const_eval(&equation.rhs, &HashMap::new()))
        {
            numbers.entry(name.clone()).or_insert(value);
        }
    }
    // A parameter built on another settles a round later: a table
    // block's `shiftTime = startTime` is one name standing for
    // another. A round that settles nothing new is the end of it.
    loop {
        let mut moved = false;
        for component in &model.components {
            if numbers.contains_key(&component.name) {
                continue;
            }
            let Some(value) = component.binding.as_ref().or(component.start.as_ref()) else {
                continue;
            };
            if let Some(number) = const_eval(value, &numbers) {
                numbers.insert(component.name.clone(), number);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    let values = settle(&named, model, &numbers)?;

    // A comparison of two strings is the one place a string reaches the
    // numbers, and it reaches them as a Boolean.
    for equation in model
        .equations
        .iter_mut()
        .chain(model.initial_equations.iter_mut())
    {
        equation.lhs = fold(&equation.lhs, &values, &numbers)?;
        equation.rhs = fold(&equation.rhs, &values, &numbers)?;
    }
    for clause in &mut model.when_clauses {
        for branch in &mut clause.branches {
            branch.condition = fold(&branch.condition, &values, &numbers)?;
            for action in &mut branch.actions {
                match action {
                    WhenAction::Assign(_, value)
                    | WhenAction::Reinit(_, value)
                    | WhenAction::TupleAssign(_, value) => {
                        *value = fold(value, &values, &numbers)?;
                    }
                    WhenAction::Terminate(_) => {}
                    // Taken apart while flattening, so neither a loop
                    // nor a choice is left.
                    WhenAction::Loop(_) | WhenAction::Choice(_) => {}
                }
            }
        }
    }
    for (condition, _) in &mut model.asserts {
        *condition = fold(condition, &values, &numbers)?;
    }
    // A declaration's value reaches the strings too: the table blocks
    // decide from a file name whether they read a file at all. What
    // will not fold is left as it was - a declaration is not an
    // equation, and something further on may still make sense of it.
    for component in &mut model.components {
        if component.type_name == "String" {
            continue;
        }
        if let Some(binding) = &component.binding {
            let taken = branch_taken(binding, &values, &numbers);
            if let Ok(said) = fold(&taken, &values, &numbers) {
                component.binding = Some(said);
            }
        }
        if let Some(start) = &component.start {
            let taken = branch_taken(start, &values, &numbers);
            if let Ok(said) = fold(&taken, &values, &numbers) {
                component.start = Some(said);
            }
        }
    }
    // A run-time `if` keeps its conditions and branches of its own,
    // and a string chooses a branch as readily as a number does.
    for conditional in &mut model.conditional {
        for condition in &mut conditional.conditions {
            *condition = fold(condition, &values, &numbers)?;
        }
        for branch in &mut conditional.branches {
            for equation in branch {
                equation.lhs = fold(&equation.lhs, &values, &numbers)?;
                equation.rhs = fold(&equation.rhs, &values, &numbers)?;
            }
        }
    }

    // The declarations and their bindings go: nothing downstream has a
    // place to put them.
    model
        .equations
        .retain(|equation| !defines_a_string(&equation.lhs, &values));
    model
        .components
        .retain(|component| component.type_name != "String");
    Ok(Settled {
        texts: values,
        numbers,
    })
}

/// The text of every `String` component, worked out in dependency
/// order: one may be built from another.
fn settle(
    named: &[String],
    model: &Model,
    numbers: &HashMap<String, f64>,
) -> Result<HashMap<String, String>, String> {
    // A parameter or a constant keeps its binding on the declaration;
    // a plain variable's became an equation on the way here.
    let mut bindings: HashMap<&str, &Expr> = HashMap::new();
    for component in &model.components {
        if let (true, Some(binding)) = (component.type_name == "String", &component.binding) {
            bindings.insert(component.name.as_str(), binding);
        }
    }
    for equation in &model.equations {
        if let Expr::Ref(name) = &equation.lhs {
            if named.iter().any(|s| s == name) {
                bindings.insert(name.as_str(), &equation.rhs);
            }
        }
    }
    // The name of what is being simulated, for `getInstanceName`, and
    // in place before anything is worked out, since a binding may ask
    // for it. A `$` name cannot collide with anything a model writes.
    let mut values: HashMap<String, String> = HashMap::new();
    values.insert("$model".to_string(), model.name.clone());
    for _ in 0..=named.len() {
        let before = values.len();
        for name in named {
            if values.contains_key(name) {
                continue;
            }
            let Some(binding) = bindings.get(name.as_str()) else {
                continue;
            };
            if let Some(text) = text_of(binding, &values, numbers) {
                values.insert(name.clone(), text);
            }
        }
        if values.len() == before {
            break;
        }
    }
    if let Some(missing) = named.iter().find(|name| !values.contains_key(*name)) {
        return Err(match bindings.get(missing.as_str()) {
            Some(_) => format!(
                "`{missing}` is a String whose value is not settled before the run; \
                 a string may only be built from literals and other strings"
            ),
            None => format!("`{missing}` is a String with no value"),
        });
    }
    Ok(values)
}

/// What an expression comes to as text, if it comes to text at all.
pub(super) fn text_of(
    expr: &Expr,
    values: &HashMap<String, String>,
    numbers: &HashMap<String, f64>,
) -> Option<String> {
    match expr {
        Expr::Str(text) => Some(text.clone()),
        Expr::Ref(name) => values.get(name).cloned(),
        // `+` joins two strings, which is what Modelica spells it as.
        Expr::Bin(BinOp::Add, a, b) => {
            Some(text_of(a, values, numbers)? + &text_of(b, values, numbers)?)
        }
        // `getInstanceName()` is the simulated model's name with the
        // path of the instance that asked appended to it.
        Expr::Call(name, args) if name == "getInstanceName" => {
            let root = values.get("$model")?;
            Some(match args.first() {
                None => root.clone(),
                Some(Expr::Str(path)) if path.is_empty() => root.clone(),
                Some(Expr::Str(path)) => format!("{root}.{path}"),
                _ => return None,
            })
        }
        // A body written outside Modelica that this compiler answers
        // for itself: the call was left standing by the inliner, and
        // this is where the string ones are worked out.
        Expr::Call(name, args) if name == "ModelicaStrings_substring" && args.len() == 3 => {
            Some(external::substring(
                &text_of(&args[0], values, numbers)?,
                const_eval(&args[1], numbers)?,
                const_eval(&args[2], numbers)?,
            ))
        }
        Expr::Call(name, args) if name == "String" && args.len() == 1 => {
            let number = const_eval(&args[0], numbers)?;
            Some(if number.fract() == 0.0 {
                format!("{number:.0}")
            } else {
                format!("{number}")
            })
        }
        _ => None,
    }
}

/// Take the branch of every `if` in a declaration's value that is
/// settled, and drop the others.
///
/// A branch nobody takes says nothing, strings included: a table block
/// asks about a file name only `if tableOnFile`, and where that is
/// false there is no file name to ask about - and nothing to refuse
/// for not being able to read one. Only a declaration's value is read
/// this way; an equation keeps the choice it wrote.
pub(super) fn branch_taken(
    expr: &Expr,
    values: &HashMap<String, String>,
    numbers: &HashMap<String, f64>,
) -> Expr {
    let recur = |e: &Expr| branch_taken(e, values, numbers);
    match expr {
        Expr::If(condition, then, otherwise) => {
            let settled = fold(&recur(condition), values, numbers)
                .ok()
                .and_then(|condition| const_eval(&condition, numbers));
            match settled {
                Some(truth) if truth != 0.0 => recur(then),
                Some(_) => recur(otherwise),
                None => Expr::If(
                    Box::new(recur(condition)),
                    Box::new(recur(then)),
                    Box::new(recur(otherwise)),
                ),
            }
        }
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        other => other.clone(),
    }
}

/// Replace a comparison of two strings with what it comes to, and
/// refuse a string that is standing anywhere else.
pub(super) fn fold(
    expr: &Expr,
    values: &HashMap<String, String>,
    numbers: &HashMap<String, f64>,
) -> Result<Expr, String> {
    if let Expr::Rel(op, a, b) = expr {
        if let (Some(left), Some(right)) =
            (text_of(a, values, numbers), text_of(b, values, numbers))
        {
            // Every relational operator is defined on strings, and
            // defined as C's strcmp against zero - which is a
            // comparison of the bytes, and so is Rust's.
            return Ok(Expr::Bool(match op {
                RelOp::Lt => left < right,
                RelOp::Le => left <= right,
                RelOp::Gt => left > right,
                RelOp::Ge => left >= right,
                RelOp::Eq => left == right,
                RelOp::Ne => left != right,
            }));
        }
    }
    if let Some(text) = text_of(expr, values, numbers) {
        return Err(format!(
            "`{text}` is a String, and a String has no value an equation can hold"
        ));
    }
    // The same, where what an outside call comes to is a number: the
    // length of a string, or how two of them compare.
    if let Expr::Call(name, args) = expr {
        if let Some(number) = external::number_of(name, args, values, numbers) {
            return Ok(Expr::Number(number));
        }
    }
    let recur = |e: &Expr| fold(e, values, numbers);
    Ok(match expr {
        Expr::Number(_) | Expr::Bool(_) | Expr::Ref(_) | Expr::Time => expr.clone(),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner)?)),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner)?)),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)?), Box::new(recur(r)?)),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)?), Box::new(recur(r)?)),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)?), Box::new(recur(r)?)),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)?), Box::new(recur(r)?)),
        Expr::If(c, a, b) => Expr::If(
            Box::new(recur(c)?),
            Box::new(recur(a)?),
            Box::new(recur(b)?),
        ),
        Expr::Call(name, args) => Expr::Call(
            name.clone(),
            args.iter().map(recur).collect::<Result<Vec<_>, _>>()?,
        ),
        Expr::Str(_) => unreachable!("a bare string was refused above"),
        // Everything else - arrays and their subscripts, ranges,
        // comprehensions, matrix rows, tuples, named arguments - is
        // resolved into scalars long before this pass runs, and a
        // string can only be scalar to begin with. Nothing inside one
        // of these can still be a string, so it is handed back whole.
        other => other.clone(),
    })
}

fn defines_a_string(lhs: &Expr, values: &HashMap<String, String>) -> bool {
    matches!(lhs, Expr::Ref(name) if values.contains_key(name))
}
