//! Lengths, ranges and the loops read off them: how long a
//! declaration is, what a `for` runs over, and what an equation
//! written over an array becomes one element at a time.
//!
//! Carved out of `instantiate` unchanged.

use super::*;

/// Answer every `size(v, k)` an expression asks about an array whose
/// shape is known, leaving the rest of it alone.
///
/// A modifier handed to a base is worked out in the terms of the class
/// that wrote it: `extends MO(final nout = size(columns, 1))` asks
/// about a declaration of the block, and the base it is handed to has
/// never heard of that name. Answering it here is what lets the base
/// be built with a length rather than with a question.
pub(super) fn measured_sizes(
    expr: &Expr,
    sizes: &HashMap<String, Vec<i64>>,
    consts: &HashMap<String, f64>,
) -> Expr {
    if let Expr::Call(name, args) = expr {
        if name == "size" && !args.is_empty() && args.len() <= 2 {
            if let Some(length) = dimension_value(expr, consts, sizes) {
                return Expr::Number(length as f64);
            }
        }
    }
    let recur = |e: &Expr| measured_sizes(e, sizes, consts);
    match expr {
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::If(c, a, b) => Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b))),
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        Expr::Array(items) => Expr::Array(items.iter().map(recur).collect()),
        // `max([size(a, 1); size(b, 1)])` is how a block takes the
        // longer of two, and the questions are inside the matrix.
        Expr::MatrixRows(rows) => Expr::MatrixRows(
            rows.iter()
                .map(|row| row.iter().map(recur).collect())
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The length of a value along one axis, for a flexible `:` size. Only
/// a value written out says its length by being written out: a list,
/// or a matrix of rows. Anything that has to be worked out first -
/// a range, a list scaled by a factor - is measured where there is an
/// environment to work it out against.
pub(super) fn flexible_size(
    binding: &Expr,
    axis: usize,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
) -> Option<i64> {
    // `[a, b; c, d]` says its shape by how it is written: as many rows
    // as there are semicolons, as many columns as a row holds. Rows of
    // different widths are no shape at all, and are left unmeasured
    // rather than guessed at.
    if let Expr::MatrixRows(rows) = binding {
        // A matrix written with `[ ; ]` stacks what it is given rather
        // than laying it out: `[v, w]` where `v` is a vector of four
        // is four rows, not one. Counting the rows as written would
        // answer four times too small, and a shape that is wrong is
        // worse than none - so only a matrix whose every cell is
        // plainly one number is measured here. Anything else is left
        // for the pass that knows what the names stand for.
        let plainly_one = |cell: &Expr| {
            matches!(cell, Expr::Number(_) | Expr::Bool(_))
                || matches!(cell, Expr::Neg(inner) if matches!(inner.as_ref(), Expr::Number(_)))
        };
        if !rows.iter().flatten().all(plainly_one) {
            return None;
        }
        let width = rows.first()?.len();
        if rows.iter().any(|row| row.len() != width) {
            return None;
        }
        return match axis {
            0 => Some(rows.len() as i64),
            1 => Some(width as i64),
            _ => None,
        };
    }
    // `fill(0, 0, 2)` is a table with no rows and two columns, which
    // is how the table blocks say they were given nothing yet. Written
    // out it is an empty list, and an empty list has no second
    // dimension to read - so the lengths are taken from the call,
    // which states them whether or not any of them is zero. `zeros`
    // and `ones` say theirs the same way, with no filler in front.
    //
    // The three are the language's own, and a package may write a
    // function of the same name: `Utilities.fill` is whatever its
    // writer meant and says nothing about how long it is. Where the
    // name resolves to a class in view, it is that class's and not
    // this.
    if let Expr::Call(name, args) = binding {
        let plainly = name
            .rsplit_once('.')
            .map_or(name.as_str(), |(_, tail)| tail);
        let lengths = match lookup(registry, name, scope, imports).is_none() {
            false => None,
            true => match plainly {
                "fill" => args.get(1..),
                "zeros" | "ones" => args.get(..),
                _ => None,
            },
        };
        if let Some(lengths) = lengths {
            if let Some(Expr::Number(length)) = lengths.get(axis) {
                return Some(*length as i64);
            }
        }
    }
    let mut here = binding;
    for _ in 0..axis {
        here = match here {
            Expr::Array(items) => items.first()?,
            _ => return None,
        };
    }
    match here {
        Expr::Array(items) => Some(items.len() as i64),
        _ => None,
    }
}

/// One element's slice of a modifier value spread over an array.
///
/// A value written as a list of the right length is handed out one
/// entry per element - `items[3](w = {1, 2, 3})` gives each its own -
/// while anything else, a scalar most of all, reaches every element
/// whole.
#[allow(clippy::too_many_arguments)]
pub(super) fn array_element(
    value: &Expr,
    position: usize,
    count: usize,
    sizes: &HashMap<String, Vec<i64>>,
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
) -> Expr {
    // A literal says its length outright; a name has to be measured,
    // which is how `cells(k = ks)` hands `cells[i].k` the value
    // `ks[i]`. Anything that does not come to one value per element is
    // handed over whole: a scalar spreads, and a modifier reaching
    // into a member of the element is not an array at this level.
    if let Expr::Array(items) = value {
        if items.len() == count {
            return items[position].clone();
        }
    }
    let shapes = Shapes {
        sizes,
        loop_vars: &HashMap::new(),
        consts,
        records: no_records(),
    };
    let mark = checks_mark();
    let measured = expand(value, &shapes, registry, scope, imports, 0);
    // Slicing a modifier is a measurement rather than the model asking
    // for the value, and the value itself is expanded again where it
    // lands; keeping the checks here would count them twice.
    checks_rewind(mark);
    let Ok(measured) = measured else {
        return value.clone();
    };
    // The slice is taken along the outermost dimension: an element of
    // `cylinders[2]` gets one of the two values, and that value may
    // itself be a vector of three.
    match measured {
        Value::Array(items) if items.len() == count => items[position].clone().into_expr(),
        _ => value.clone(),
    }
}

/// The values a loop variable takes, from whatever the range expanded
/// to. A range, a set and an array all expand to the same thing - the
/// values, in order - so there is nothing to tell apart here.
pub(super) fn loop_values(
    spread: &Value,
    env: &HashMap<String, f64>,
    variable: &str,
) -> Result<Vec<f64>, String> {
    let Value::Array(items) = spread else {
        return Err(format!(
            "`{variable}` needs something to run over - a range, a set or an array - and \
             a single value is not one"
        ));
    };
    items
        .iter()
        .map(|item| {
            let expr = item.clone().scalar()?;
            const_eval(&expr, env).ok_or_else(|| {
                format!("`{variable}` runs over values the compiler cannot work out: {expr:?}")
            })
        })
        .collect()
}

/// What `for i loop` runs over, which the body has to say: the size of
/// the array along the dimension `i` is used to subscript.
pub(super) fn implied_range(
    body: &[ForBody],
    variable: &str,
    prefix: &str,
    outers: &HashMap<String, String>,
    sizes: &HashMap<String, Vec<i64>>,
) -> Result<Vec<f64>, String> {
    let mut found = None;
    for item in body {
        let mut look = |expr: &Expr| {
            if found.is_none() {
                found = subscript_extent(&prefix_expr(expr, prefix, outers), variable, sizes);
            }
        };
        match item {
            ForBody::Equation(equation) => {
                look(&equation.lhs);
                look(&equation.rhs);
            }
            ForBody::Connect(a, b) => {
                look(a);
                look(b);
            }
            // A branch says what it holds the same way the loop does.
            ForBody::Branch(if_equation) => {
                for branch in &if_equation.branches {
                    for equation in &branch.equations {
                        look(&equation.lhs);
                        look(&equation.rhs);
                    }
                }
            }
            ForBody::Nested(inner) => {
                if found.is_none() {
                    found = implied_range(&inner.body, variable, prefix, outers, sizes)
                        .ok()
                        .map(|values| values.len() as i64);
                }
            }
            ForBody::Assert(condition, _) => look(condition),
        }
    }
    let Some(extent) = found else {
        return Err(format!(
            "`for {variable} loop` leaves the range to the body, and nothing in the body \
             uses `{variable}` to subscript an array of a length the compiler knows"
        ));
    };
    Ok((1..=extent).map(|index| index as f64).collect())
}

/// How long an array is along the dimension a name is used to subscript
/// it by, looking through a whole expression for the first such use.
pub(super) fn subscript_extent(
    expr: &Expr,
    variable: &str,
    sizes: &HashMap<String, Vec<i64>>,
) -> Option<i64> {
    let recur = |inner: &Expr| subscript_extent(inner, variable, sizes);
    match expr {
        Expr::Index(base, subscripts) => {
            if let Expr::Ref(name) = base.as_ref() {
                if let Some(shape) = sizes.get(name) {
                    let along = subscripts.iter().position(
                        |subscript| matches!(subscript, Expr::Ref(used) if used == variable),
                    );
                    if let Some(dimension) = along.and_then(|at| shape.get(at)) {
                        return Some(*dimension);
                    }
                }
            }
            recur(base).or_else(|| subscripts.iter().find_map(recur))
        }
        Expr::Call(_, args) | Expr::Array(args) => args.iter().find_map(recur),
        Expr::Neg(inner) | Expr::Not(inner) | Expr::Member(inner, _) => recur(inner),
        Expr::Bin(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r)
        | Expr::Elementwise(_, l, r) => recur(l).or_else(|| recur(r)),
        Expr::If(c, t, e) => recur(c).or_else(|| recur(t)).or_else(|| recur(e)),
        Expr::MatrixRows(rows) => rows.iter().find_map(|row| row.iter().find_map(recur)),
        _ => None,
    }
}

/// Unroll a `for` equation, recursing into nested loops. The loop
/// variable is a compile-time constant, so the body is emitted once per
/// value with every subscript already resolved.
#[allow(clippy::too_many_arguments)]
pub(super) fn unroll(
    loop_eq: &ForEquation,
    outer_vars: &HashMap<String, f64>,
    consts: &HashMap<String, f64>,
    prefix: &str,
    outers: &HashMap<String, String>,
    sizes: &HashMap<String, Vec<i64>>,
    records: &HashMap<String, String>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    acc: &mut Flat,
) -> Result<(), String> {
    // Everything in the loop is prefixed before it is folded, so a
    // parameter of the class the loop is written in - the `n` of `for i
    // in 1:n` or of a guard `if i < n` - has to be findable under its
    // instance path as well as under its plain name.
    let consts: HashMap<String, f64> = consts
        .iter()
        .map(|(name, value)| (name.clone(), *value))
        .chain(
            consts
                .iter()
                .map(|(name, value)| (format!("{prefix}{name}"), *value)),
        )
        .collect();
    let consts = &consts;
    let values = match &loop_eq.range {
        Some(range) => {
            let mut env = consts.clone();
            env.extend(outer_vars.iter().map(|(k, v)| (k.clone(), *v)));
            // A range may ask an array how long it is, so it goes
            // through the same expansion as everything else - which is
            // also what turns `1:n`, `{1, 3, 5}` and the name of an
            // array into one thing: the values, in order.
            let shapes = Shapes {
                sizes,
                loop_vars: outer_vars,
                consts,
                records: no_records(),
            };
            // The bounds may be constants of a package the class is
            // written inside, and those are put in before the class's
            // own prefix goes on: `1:nX` counts a medium's substances.
            let range = substitute_class_constants(range, registry, scope, imports, &[]);
            let spread = expand(
                &prefix_expr(&range, prefix, outers),
                &shapes,
                registry,
                scope,
                imports,
                0,
            )?;
            loop_values(&spread, &env, &loop_eq.variable)?
        }
        None => implied_range(&loop_eq.body, &loop_eq.variable, prefix, outers, sizes)?,
    };
    for index in values {
        let mut loop_vars = outer_vars.clone();
        loop_vars.insert(loop_eq.variable.clone(), index);
        // The loop variable is a compile-time number, not a component,
        // and it is folded in before anything is prefixed: prefixing
        // reaches into subscripts too, and `x[i]` inside a component
        // would otherwise be asking for `a.i`.
        let folded: HashMap<String, Expr> = loop_vars
            .iter()
            .map(|(name, value)| (name.clone(), Expr::Number(*value)))
            .collect();
        for item in &loop_eq.body {
            match item {
                ForBody::Equation(equation) => {
                    let shapes = Shapes {
                        sizes,
                        loop_vars: &loop_vars,
                        consts,
                        records,
                    };
                    let side = |expr: &Expr| -> Result<Value, String> {
                        let expr = substitute_refs(expr, &folded);
                        let expr = substitute_class_constants(&expr, registry, scope, imports, &[]);
                        let value = expand(
                            &prefix_expr(&expr, prefix, outers),
                            &shapes,
                            registry,
                            scope,
                            imports,
                            0,
                        )?;
                        records_written_out(value, &shapes, registry, &|e| {
                            expand(e, &shapes, registry, scope, imports, 0)
                        })
                    };
                    push_equations(&side(&equation.lhs)?, &side(&equation.rhs)?, acc)?;
                }
                ForBody::Connect(a, b) => {
                    let shapes = Shapes {
                        sizes,
                        loop_vars: &loop_vars,
                        consts,
                        records,
                    };
                    let (a, b) = (substitute_refs(a, &folded), substitute_refs(b, &folded));
                    push_connects(
                        &a, &b, &shapes, prefix, outers, registry, scope, imports, acc,
                    )?;
                }
                ForBody::Nested(inner) => unroll(
                    inner, &loop_vars, consts, prefix, outers, sizes, records, registry, scope,
                    imports, acc,
                )?,
                // An `if` inside a loop: the branch that holds gives
                // its equations to the round, and the others give
                // nothing. What decides it is settled here, as it is
                // for an `if` written among the equations of a class.
                ForBody::Branch(if_equation) => {
                    let shapes = Shapes {
                        sizes,
                        loop_vars: &loop_vars,
                        consts,
                        records,
                    };
                    // Everything inside the loop is read with this
                    // round's value of the loop variable already put
                    // in, so what comes out is one round's worth of
                    // this branch and nothing about any other round.
                    let side = |expr: &Expr| -> Result<Value, String> {
                        let expr = substitute_refs(expr, &folded);
                        let expr = substitute_class_constants(&expr, registry, scope, imports, &[]);
                        let value = expand(
                            &prefix_expr(&expr, prefix, outers),
                            &shapes,
                            registry,
                            scope,
                            imports,
                            0,
                        )?;
                        records_written_out(value, &shapes, registry, &|e| {
                            expand(e, &shapes, registry, scope, imports, 0)
                        })
                    };
                    let mut settling = consts.clone();
                    settling.extend(loop_vars.iter().map(|(k, v)| (k.clone(), *v)));
                    // A condition that asks the connections is read
                    // the long way round: the array layer has to run
                    // first, since `cardinality(port[i])` is a
                    // question about one port of an array and it is
                    // the array layer that names it.
                    let (roots, counts, answered) =
                        (acc.roots.clone(), acc.counts.clone(), acc.answered);
                    let settle = |condition: &Expr| {
                        let plain = substitute_refs(condition, &folded);
                        let plain =
                            substitute_class_constants(&plain, registry, scope, imports, &[]);
                        if let Some(value) = const_eval(&plain, &settling) {
                            return Some(value);
                        }
                        if !answered {
                            return None;
                        }
                        let asked = side(condition).ok()?.scalar().ok()?;
                        let told = answer_graph_queries(&asked, &roots, &counts);
                        const_eval(&told, &settling)
                    };
                    let decidable = if_equation.branches.iter().all(|branch| {
                        branch
                            .condition
                            .as_ref()
                            .is_none_or(|condition| settle(condition).is_some())
                    });
                    // Undecidable only because nothing has looked at
                    // the connections yet: the whole model is built
                    // again once one pass has gathered them.
                    if !decidable
                        && !answered
                        && if_equation.branches.iter().any(|branch| {
                            branch.condition.as_ref().is_some_and(asks_the_connections)
                        })
                    {
                        acc.graph_asked = true;
                        continue;
                    }
                    // A condition the run decides makes the same `if`
                    // it would make among the equations of a class -
                    // one equation per position, choosing its residual
                    // as it goes - only written once per round.
                    if !decidable {
                        push_conditional(
                            if_equation,
                            scope,
                            |expr: &Expr| side(expr)?.scalar(),
                            |expr: &Expr, _: &HashMap<String, f64>| side(expr),
                            &HashMap::new(),
                            acc,
                        )?;
                        continue;
                    }
                    let mut chosen = None;
                    for branch in &if_equation.branches {
                        let Some(condition) = &branch.condition else {
                            chosen = Some(branch);
                            break;
                        };
                        if settle(condition) != Some(0.0) {
                            chosen = Some(branch);
                            break;
                        }
                    }
                    let Some(branch) = chosen else { continue };
                    if !branch.whens.is_empty()
                        || !branch.calls.is_empty()
                        || !branch.graph.is_empty()
                    {
                        return Err(format!(
                            "a `when`, a call on its own or a `Connections` clause sits in an \
                             `if` inside a `for` in `{scope}`, and this compiler reads none of \
                             them there"
                        ));
                    }
                    for (a, b) in &branch.connects {
                        let (a, b) = (substitute_refs(a, &folded), substitute_refs(b, &folded));
                        push_connects(
                            &a, &b, &shapes, prefix, outers, registry, scope, imports, acc,
                        )?;
                    }
                    for inner in &branch.loops {
                        unroll(
                            inner, &loop_vars, consts, prefix, outers, sizes, records, registry,
                            scope, imports, acc,
                        )?;
                    }
                    for equation in &branch.equations {
                        push_equations(&side(&equation.lhs)?, &side(&equation.rhs)?, acc)?;
                    }
                    for (condition, message) in &branch.asserts {
                        acc.asserts
                            .push((side(condition)?.scalar()?, message.clone()));
                    }
                }
                ForBody::Assert(condition, message) => {
                    let condition = substitute_refs(condition, &folded);
                    let condition =
                        substitute_class_constants(&condition, registry, scope, imports, &[]);
                    acc.asserts
                        .push((prefix_expr(&condition, prefix, outers), message.clone()));
                }
            }
        }
    }
    Ok(())
}

/// The values that lie `depth` levels down inside one, in the order a
/// row-major walk meets them.
///
/// A value nests once per dimension and then once more for a record's
/// fields, and telling those apart by counting alone goes wrong as soon
/// as the two counts agree. The declaration says how many levels are
/// dimensions, so that is what is followed.
pub(super) fn levels_down(value: &Value, depth: usize, out: &mut Vec<Value>) {
    match (depth, value) {
        (0, _) => out.push(value.clone()),
        (_, Value::Array(items)) => items
            .iter()
            .for_each(|item| levels_down(item, depth - 1, out)),
        _ => {}
    }
}

/// How many numbers one record of this class holds: its fields, each
/// as many times over as its own dimensions say, and a field that is
/// itself a record counted the same way.
///
/// `None` where a dimension is not a length written as a number, or
/// where a field's class is one this cannot look into. Then nothing
/// here can say what a value's shape means, and whoever asked is left
/// to decide by other means rather than by a count that might be
/// wrong.
pub(super) fn numbers_of_one(
    registry: &HashMap<&str, &ClassDef>,
    of: &ClassDef,
    depth: usize,
) -> Option<usize> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut all = 0;
    for field in &of.components {
        let mut many = 1;
        for dimension in &field.dimensions {
            let length = const_eval(dimension, &HashMap::new())?;
            if length < 0.0 || length.fract() != 0.0 {
                return None;
            }
            many *= length as usize;
        }
        let each = match is_primitive(&field.type_name) {
            true => 1,
            false => {
                let inside = lookup(registry, &field.type_name, &of.name, &of.imports)?;
                // A type alias is a name for a primitive with
                // attributes attached - `type Power = Real(unit =
                // "W")` - and holds one number, not the none its
                // empty component list would suggest. Counting it as
                // none made a record of aliases look emptier than it
                // is, so a value handed to it matched neither reading
                // and was dropped, which is what left the machines'
                // friction records without their reference speed.
                match inside.alias_of.is_some() || !inside.enumeration.is_empty() {
                    true => 1,
                    false => numbers_of_one(registry, inside, depth + 1)?,
                }
            }
        };
        all += many * each;
    }
    Some(all)
}
