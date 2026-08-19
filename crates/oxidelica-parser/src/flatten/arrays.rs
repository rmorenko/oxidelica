//! The array layer: values with a shape, and what may be done to
//! them before everything drops to scalars.

use super::*;

/// Expand an expression into scalars, keeping the array structure while
/// it is needed and dropping to the scalar path for everything else.
#[allow(clippy::too_many_arguments)]
pub(super) fn expand(
    expr: &Expr,
    shapes: &Shapes,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "an expression {NO_BOTTOM}, nested deeper than the compiler follows: {}",
            sketch(expr)
        ));
    }
    let recur = |e: &Expr| expand(e, shapes, registry, scope, imports, depth + 1);
    let scalar = |e: &Expr| -> Result<Value, String> {
        Ok(Value::Scalar(resolve(
            e,
            shapes.loop_vars,
            shapes.consts,
            shapes.sizes,
            registry,
            scope,
            imports,
            depth + 1,
        )?))
    };

    let constant_here = |e: &Expr| -> Option<f64> {
        let mut env = shapes.consts.clone();
        env.extend(shapes.loop_vars.iter().map(|(k, v)| (k.clone(), *v)));
        const_eval(e, &env)
    };
    Ok(match expr {
        Expr::Array(items) => Value::Array(
            items
                .iter()
                .map(&recur)
                .collect::<Result<Vec<_>, String>>()?,
        ),
        // A range is a vector whose bounds the compiler can see.
        Expr::Range(a, step, b) => {
            let scalar_of = |e: &Expr| -> Result<f64, String> {
                let resolved = recur(e)?.scalar()?;
                constant_here(&resolved).ok_or_else(|| {
                    format!(
                        "{UNDECIDABLE_LOOP}: a range needs bounds the compiler can see, \
                         and {} is not one",
                        crate::flatten::names::sketch(&resolved)
                    )
                })
            };
            let (from, to) = (scalar_of(a)?, scalar_of(b)?);
            let step = match step {
                Some(step) => scalar_of(step)?,
                None => 1.0,
            };
            if step == 0.0 {
                return Err("a range cannot step by zero".to_string());
            }
            let count = ((to - from) / step + 1e-9).floor() as i64 + 1;
            Value::Array(
                (0..count.max(0))
                    .map(|i| Value::Scalar(Expr::Number(from + i as f64 * step)))
                    .collect(),
            )
        }
        // `{expr for i in range}` unrolls with the iterator bound.
        Expr::Comprehension(body, variable, range) => {
            let Value::Array(items) = recur(range)? else {
                return Err(format!("`{variable}` needs an array to iterate over"));
            };
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let value = constant_here(&item.scalar()?).ok_or_else(|| {
                    format!("the range of `{variable}` must be constant at compile time")
                })?;
                let mut loop_vars = shapes.loop_vars.clone();
                loop_vars.insert(variable.clone(), value);
                let inner = Shapes {
                    sizes: shapes.sizes,
                    loop_vars: &loop_vars,
                    consts: shapes.consts,
                    records: no_records(),
                };
                out.push(expand(body, &inner, registry, scope, imports, depth + 1)?);
            }
            Value::Array(out)
        }
        // `[a, b; c, d]`: every part is a matrix, and they are joined
        // side by side within a row and one row under another. A
        // scalar is one by one; a vector of n is n rows of one - a
        // column, which is what makes `[v; 0]` a vector one longer
        // rather than two rows of different widths.
        Expr::MatrixRows(rows) => {
            let mut out_rows: Vec<Vec<Expr>> = Vec::new();
            for row in rows {
                let mut blocks: Vec<Vec<Vec<Expr>>> = Vec::new();
                for item in row {
                    blocks.push(as_block(recur(item)?)?);
                }
                let height = blocks.first().map_or(0, |block| block.len());
                if blocks.iter().any(|block| block.len() != height) {
                    return Err("the parts of one row of a matrix must be equally tall".to_string());
                }
                for line in 0..height {
                    let mut cells = Vec::new();
                    for block in &blocks {
                        cells.extend(block[line].iter().cloned());
                    }
                    out_rows.push(cells);
                }
            }
            let width = out_rows.first().map_or(0, |row| row.len());
            if out_rows.iter().any(|row| row.len() != width) {
                return Err("the rows of a matrix must be equally wide".to_string());
            }
            Value::Array(
                out_rows
                    .into_iter()
                    .map(|row| Value::Array(row.into_iter().map(Value::Scalar).collect()))
                    .collect(),
            )
        }
        Expr::ColonSubscript | Expr::EndSubscript => {
            return Err("`:` and `end` make sense only inside a subscript".to_string())
        }
        // A name that was declared with dimensions stands for all of its
        // elements at once.
        Expr::Ref(name) if shapes.sizes.contains_key(name) => {
            elements_of(name, &shapes.sizes[name])
        }
        // `plug.pin.v` where `pin` is an array of connectors is the
        // array of their `v`. The name of the array is a prefix of the
        // name written, so the prefixes are tried longest first: with
        // arrays inside arrays, the innermost one is the one whose
        // subscript goes nearest the member.
        Expr::Ref(name) if member_of_array(name, shapes.sizes).is_some() => {
            let (array, member) = member_of_array(name, shapes.sizes).expect("just matched");
            let elements = elements_of(array, &shapes.sizes[array]);
            map_value(&elements, &|element| match element {
                Expr::Ref(each) => Expr::Ref(format!("{each}.{member}")),
                other => other,
            })
        }
        // A record instance stands for its fields, in the order they
        // were declared: that is what an operator works on.
        Expr::Ref(name) if shapes.records.contains_key(name) => {
            let of = registry
                .get(shapes.records[name].as_str())
                .ok_or_else(|| format!("`{name}` is a record of a class that is not here"))?;
            Value::Array(
                record_fields(of)
                    .into_iter()
                    .map(|field| Value::Scalar(Expr::Ref(format!("{name}.{field}"))))
                    .collect(),
            )
        }
        Expr::Neg(inner) => {
            if let Some(record) = record_class_of(inner, shapes, registry, scope, imports) {
                return apply_operator(
                    &record,
                    "-",
                    std::slice::from_ref(inner.as_ref()),
                    shapes,
                    registry,
                    scope,
                    imports,
                    depth,
                );
            }
            map_value(&recur(inner)?, &|e| Expr::Neg(Box::new(e)))
        }
        Expr::Bin(op, l, r) | Expr::Elementwise(op, l, r) => {
            // An operator on records is whatever the record says it
            // is; anything else combines element by element.
            if let Some(record) = record_class_of(expr, shapes, registry, scope, imports) {
                return apply_operator(
                    &record,
                    operator_symbol(*op),
                    &[l.as_ref().clone(), r.as_ref().clone()],
                    shapes,
                    registry,
                    scope,
                    imports,
                    depth,
                );
            }
            let elementwise = matches!(expr, Expr::Elementwise(_, _, _));
            combine(*op, &recur(l)?, &recur(r)?, elementwise)?
        }
        // A comparison of records is whatever the record's relational
        // operator says; a comparison of numbers is left for the run.
        Expr::Rel(op, l, r) => {
            if let Some(record) = record_class_of(l, shapes, registry, scope, imports)
                .or_else(|| record_class_of(r, shapes, registry, scope, imports))
            {
                return apply_operator(
                    &record,
                    relation_symbol(*op),
                    &[l.as_ref().clone(), r.as_ref().clone()],
                    shapes,
                    registry,
                    scope,
                    imports,
                    depth,
                );
            }
            Value::Scalar(Expr::Rel(
                *op,
                Box::new(recur(l)?.scalar()?),
                Box::new(recur(r)?.scalar()?),
            ))
        }
        Expr::If(condition, then, otherwise) => {
            let condition = recur(condition)?.scalar()?;
            // A guard on the loop variable takes its branch and
            // leaves the other alone: at the first element of a loop
            // over neighbours, `if i > 1 then x[i - 1] else 0` must
            // not go looking for `x[0]`. A condition that does not
            // mention the loop stays as it was written, parameters
            // included - folding those would nail down a value the
            // model is meant to be re-run with.
            // Inside a loop being unrolled, everything the compiler
            // can decide is part of the structure being built; outside
            // one it is a value the model may be re-run with.
            if shapes.loop_vars.is_empty() {
                let settled = constant_here(&condition);
                // The branch that stands is expanded first, so that
                // its own trouble is what gets said rather than the
                // other's.
                let stands = settled.map(|truth| truth != 0.0);
                let (first, second) = match stands {
                    Some(false) => (otherwise, then),
                    _ => (then, otherwise),
                };
                let first = recur(first)?;
                let second = match (recur(second), settled) {
                    (Ok(value), _) => value,
                    // Where the compiler settles the condition, the
                    // branch it does not take need not be buildable at
                    // all: the standard library asks the length of a
                    // file name only `if tableOnFile`, and that length
                    // has a body written in C. Nothing is lost - the
                    // branch is not part of this model - though a
                    // mistake in it will go unmentioned until a run
                    // that takes it.
                    (Err(_), Some(_)) => return Ok(first),
                    (Err(trouble), None) => return Err(trouble),
                };
                let (taken, left) = match stands {
                    Some(false) => (second, first),
                    _ => (first, second),
                };
                // Two branches of the same shape are one value chosen
                // as the run goes. Two of different shapes are not a
                // value at all but a structure - the standard library
                // builds a table one way or another way depending on
                // whether there is anything in it - and a structure
                // has to be settled here.
                if taken.shape() == left.shape() {
                    return zip_values(&taken, &left, &|a, b| {
                        Expr::If(
                            Box::new(condition.clone()),
                            Box::new(a.clone()),
                            Box::new(b.clone()),
                        )
                    });
                }
                let Some(truth) = settled else {
                    return Err(format!(
                        "an `if` whose branches are of shapes {:?} and {:?} decides the \
                         shape of what it stands for, so its condition has to be one the \
                         compiler can settle: {}",
                        taken.shape(),
                        left.shape(),
                        crate::flatten::names::sketch(&condition)
                    ));
                };
                return Ok(if truth != 0.0 { taken } else { left });
            }
            if let Some(truth) = constant_here(&condition) {
                return if truth != 0.0 {
                    recur(then)
                } else {
                    recur(otherwise)
                };
            }
            let (then, otherwise) = (recur(then)?, recur(otherwise)?);
            zip_values(&then, &otherwise, &|a, b| {
                Expr::If(
                    Box::new(condition.clone()),
                    Box::new(a.clone()),
                    Box::new(b.clone()),
                )
            })?
        }
        Expr::Call(name, args) => expand_call(name, args, shapes, registry, scope, imports, depth)?,
        // Indexing something that expands to an array picks the element:
        // this is how `a[i]` works inside a function whose `a` was bound
        // to an array literal.
        Expr::Index(base, subscripts) => {
            let base_value = recur(base)?;
            match base_value {
                Value::Array(_) => index_into(
                    base_value, subscripts, shapes, registry, scope, imports, depth,
                )?,
                _ => scalar(expr)?,
            }
        }
        // `ac.pin[:].v` - a member read off each of the connectors a
        // slice kept. The slice is an array of names, and the member
        // goes on every one of them.
        Expr::Member(base, path) => match recur(base)? {
            array @ Value::Array(_) => map_value(&array, &|element| match element {
                Expr::Ref(name) => Expr::Ref(format!("{name}.{path}")),
                other => Expr::Member(Box::new(other), path.clone()),
            }),
            _ => scalar(expr)?,
        },
        other => scalar(other)?,
    })
}

/// One part of a `[ ]` as the matrix it stands for: a scalar is one by
/// one, a vector of n is n rows of one, a matrix is itself.
fn as_block(value: Value) -> Result<Vec<Vec<Expr>>, String> {
    Ok(match value {
        Value::Scalar(expr) => vec![vec![expr]],
        Value::Array(items) => {
            let mut rows = Vec::new();
            for item in items {
                match item {
                    Value::Scalar(expr) => rows.push(vec![expr]),
                    Value::Array(cells) => {
                        let mut line = Vec::new();
                        for cell in cells {
                            line.push(cell.scalar()?);
                        }
                        rows.push(line);
                    }
                }
            }
            rows
        }
    })
}

/// Read subscripts into an array value.
///
/// A subscript picks one element; a range, a `:` or a vector of indices
/// takes several. What matters is that the subscripts after a slice
/// apply *inside* each element it kept, not to the slice as a whole:
/// `a[:, 3]` is the third of every row, and reading the two in turn
/// would take the third row instead. `end` stands for the length of
/// the dimension it is written in.
#[allow(clippy::too_many_arguments)]
fn index_into(
    value: Value,
    subscripts: &[Expr],
    shapes: &Shapes,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return Err(format!("subscripts {NO_BOTTOM}"));
    }
    let Some((subscript, rest)) = subscripts.split_first() else {
        return Ok(value);
    };
    let Value::Array(items) = value else {
        return Err("more subscripts than dimensions".to_string());
    };
    let length = items.len();
    let constant_here = |e: &Expr| -> Option<f64> {
        let mut env = shapes.consts.clone();
        env.extend(shapes.loop_vars.iter().map(|(k, v)| (k.clone(), *v)));
        const_eval(e, &env)
    };
    let inner = |item: Value| index_into(item, rest, shapes, registry, scope, imports, depth + 1);
    let one = |index: f64| -> Result<Value, String> {
        if index.fract() != 0.0 || index < 1.0 || index as usize > length {
            return Err(format!("subscript {index} is outside an array of {length}"));
        }
        inner(items[index as usize - 1].clone())
    };
    let with_end = substitute_end(subscript, length as f64);
    if let Expr::ColonSubscript = &with_end {
        return Ok(Value::Array(
            items
                .into_iter()
                .map(inner)
                .collect::<Result<Vec<_>, String>>()?,
        ));
    }
    match expand(&with_end, shapes, registry, scope, imports, depth + 1)? {
        Value::Scalar(index) => {
            let value = constant_here(&index).ok_or_else(|| {
                "a subscript into an array value must be a compile-time constant".to_string()
            })?;
            // A Boolean subscript indexes off its `false` lower bound:
            // `false` is the first element, `true` the second.
            let value = if is_boolean(&with_end) {
                value + 1.0
            } else {
                value
            };
            one(value)
        }
        Value::Array(picks) => {
            // A vector subscript takes the elements it names, and what
            // follows applies inside each of them.
            let mut out = Vec::with_capacity(picks.len());
            for pick in picks {
                let index = constant_here(&pick.scalar()?).ok_or_else(|| {
                    "a slicing subscript must be constant at compile time".to_string()
                })?;
                out.push(one(index)?);
            }
            Ok(Value::Array(out))
        }
    }
}

/// The array built-ins, and the ordinary ones applied to every element.
#[allow(clippy::too_many_arguments)]
pub(super) fn expand_call(
    name: &str,
    args: &[Expr],
    shapes: &Shapes,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
) -> Result<Value, String> {
    let recur = |e: &Expr| expand(e, shapes, registry, scope, imports, depth + 1);
    let constant = |e: &Expr| -> Result<i64, String> {
        let value = recur(e)?.scalar()?;
        let mut env = shapes.consts.clone();
        env.extend(shapes.loop_vars.iter().map(|(k, v)| (k.clone(), *v)));
        let value = const_eval(&value, &env).ok_or_else(|| {
            format!(
                "`{name}` needs a length the compiler can see: {}",
                crate::flatten::names::sketch(&value)
            )
        })?;
        if value.fract() != 0.0 || value < 0.0 {
            return Err(format!(
                "`{name}`: a length must be a whole number, got {value}"
            ));
        }
        Ok(value as i64)
    };

    match (name, args.len()) {
        // `String(a, ...)` on a record is what the record's `'String'`
        // operator makes of it; on a number it stays for the string
        // pass to fold.
        ("String", _) if !args.is_empty() => {
            if let Some(record) = record_class_of(&args[0], shapes, registry, scope, imports) {
                if operator_function(registry, &record, "String", args.len()).is_some() {
                    return apply_operator(
                        &record, "String", args, shapes, registry, scope, imports, depth,
                    );
                }
            }
            let folded = args
                .iter()
                .map(|a| Ok(recur(a)?.into_expr()))
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Value::Scalar(Expr::Call(name.to_string(), folded)))
        }
        // How long an array is, which is a compile-time number.
        ("size", 1) => {
            let shape = recur(&args[0])?.shape();
            Ok(Value::Array(
                shape
                    .into_iter()
                    .map(|length| Value::Scalar(Expr::Number(length as f64)))
                    .collect(),
            ))
        }
        ("size", 2) => {
            let shape = recur(&args[0])?.shape();
            let dimension = constant(&args[1])?;
            let length = shape.get((dimension - 1).max(0) as usize).ok_or_else(|| {
                format!(
                    "size(..., {dimension}): {} is of shape {shape:?}",
                    crate::flatten::names::sketch(&args[0])
                )
            })?;
            Ok(Value::Scalar(Expr::Number(*length as f64)))
        }
        // Reductions.
        // Summing an array of records adds them with the record's own
        // `'+'`, starting from its `'0'` - which is what that operator
        // is for. Without a `'0'` the first element starts it off, and
        // an empty array has nothing to start from at all.
        ("sum", 1) => {
            if let Some(record) = element_record_of(&args[0], shapes) {
                if operator_function(registry, &record, "+", 2).is_some() {
                    // Each element goes in as the array of its fields,
                    // which is the shape a record argument arrives in.
                    let Expr::Ref(array) = &args[0] else {
                        return Err(format!("`sum` of a `{record}` that is not an array"));
                    };
                    let dimensions = shapes
                        .sizes
                        .get(array)
                        .ok_or_else(|| format!("`{array}` has no shape"))?;
                    let of = registry
                        .get(record.as_str())
                        .ok_or_else(|| format!("`{record}` is not here"))?;
                    let fields = record_fields(of);
                    let elements: Vec<Expr> = index_tuples(dimensions)
                        .into_iter()
                        .map(|indices| {
                            let element = element_name(array, &indices);
                            Expr::Array(
                                fields
                                    .iter()
                                    .map(|field| Expr::Ref(format!("{element}.{field}")))
                                    .collect(),
                            )
                        })
                        .collect();
                    let zero = operator_function(registry, &record, "0", 0).is_some();
                    if elements.is_empty() && !zero {
                        return Err(format!(
                            "`sum` of an empty array of `{record}`, which declares no `'0'`"
                        ));
                    }
                    let mut total = if zero {
                        apply_operator(&record, "0", &[], shapes, registry, scope, imports, depth)?
                            .into_expr()
                    } else {
                        elements[0].clone()
                    };
                    let rest = if zero { &elements[..] } else { &elements[1..] };
                    for item in rest {
                        total = apply_operator(
                            &record,
                            "+",
                            &[total, item.clone()],
                            shapes,
                            registry,
                            scope,
                            imports,
                            depth,
                        )?
                        .into_expr();
                    }
                    return recur(&total);
                }
            }
            let mut terms = Vec::new();
            recur(&args[0])?.flatten_into(&mut terms);
            Ok(Value::Scalar(sum_of(terms)))
        }
        ("product", 1) => {
            let mut terms = Vec::new();
            recur(&args[0])?.flatten_into(&mut terms);
            Ok(Value::Scalar(match name {
                "sum" => sum_of(terms),
                _ => terms
                    .into_iter()
                    .reduce(|a, b| Expr::Bin(BinOp::Mul, Box::new(a), Box::new(b)))
                    .unwrap_or(Expr::Number(1.0)),
            }))
        }
        ("min", 1) | ("max", 1) => {
            let mut terms = Vec::new();
            recur(&args[0])?.flatten_into(&mut terms);
            let reduced = terms
                .into_iter()
                .reduce(|a, b| Expr::Call(name.to_string(), vec![a, b]))
                .ok_or_else(|| format!("`{name}` of an empty array"))?;
            Ok(Value::Scalar(reduced))
        }
        // Constructors.
        // `vector(A)` reads an array with at most one dimension worth
        // more than one as the values along it: `[v; 0]` is a column,
        // and `vector` of it is the vector again, one longer.
        ("vector", 1) => {
            let value = recur(&args[0])?;
            let shape = value.shape();
            if shape.iter().filter(|length| **length > 1).count() > 1 {
                return Err(format!(
                    "`vector` reads an array with one dimension worth more than one, \
                     and this is of shape {shape:?}"
                ));
            }
            let mut items = Vec::new();
            value.flatten_into(&mut items);
            Ok(Value::Array(items.into_iter().map(Value::Scalar).collect()))
        }
        // `zeros(n)`, `zeros(n, m)`, `zeros(n, m, k)` - as many
        // dimensions as it is given, and the same for `ones`.
        ("zeros", _) | ("ones", _) if !args.is_empty() => {
            let value = if name == "ones" { 1.0 } else { 0.0 };
            let lengths = args
                .iter()
                .map(&constant)
                .collect::<Result<Vec<_>, String>>()?;
            Ok(nested(&lengths, &Expr::Number(value)))
        }
        // `fill(v, n, m, ...)` - the value, then the dimensions.
        ("fill", _) if args.len() > 2 => {
            let filler = recur(&args[0])?.scalar()?;
            let lengths = args[1..]
                .iter()
                .map(&constant)
                .collect::<Result<Vec<_>, String>>()?;
            Ok(nested(&lengths, &filler))
        }
        ("fill", 2) => {
            let filler = recur(&args[0])?.scalar()?;
            let length = constant(&args[1])?;
            Ok(Value::Array(
                (0..length).map(|_| Value::Scalar(filler.clone())).collect(),
            ))
        }
        ("transpose", 1) => {
            let value = recur(&args[0])?;
            let shape = value.shape();
            if shape.len() != 2 {
                return Err(format!(
                    "transpose works on a matrix, and {} is of shape {shape:?}",
                    crate::flatten::names::sketch(&args[0])
                ));
            }
            let Value::Array(rows) = &value else {
                return Err("transpose works on a matrix".to_string());
            };
            (0..shape[1])
                .map(|column| pick_column(rows, column))
                .collect::<Result<Vec<_>, String>>()
                .map(Value::Array)
        }
        ("identity", 1) => {
            let n = constant(&args[0])?;
            Ok(Value::Array(
                (1..=n)
                    .map(|i| {
                        Value::Array(
                            (1..=n)
                                .map(|j| {
                                    Value::Scalar(Expr::Number(if i == j { 1.0 } else { 0.0 }))
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            ))
        }
        ("diagonal", 1) => {
            let Value::Array(items) = recur(&args[0])? else {
                return Err("diagonal takes a vector".to_string());
            };
            let n = items.len();
            Ok(Value::Array(
                (0..n)
                    .map(|i| {
                        Value::Array(
                            (0..n)
                                .map(|j| {
                                    if i == j {
                                        items[i].clone()
                                    } else {
                                        Value::Scalar(Expr::Number(0.0))
                                    }
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            ))
        }
        ("cross", 2) => {
            let (a, b) = (recur(&args[0])?, recur(&args[1])?);
            let (Value::Array(a), Value::Array(b)) = (&a, &b) else {
                return Err("cross takes two 3-vectors".to_string());
            };
            if a.len() != 3 || b.len() != 3 {
                return Err("cross takes two 3-vectors".to_string());
            }
            let term = |i: usize, j: usize| -> Result<Expr, String> {
                Ok(Expr::Bin(
                    BinOp::Mul,
                    Box::new(a[i].clone().scalar()?),
                    Box::new(b[j].clone().scalar()?),
                ))
            };
            let minus = |p: Expr, q: Expr| Expr::Bin(BinOp::Sub, Box::new(p), Box::new(q));
            Ok(Value::Array(vec![
                Value::Scalar(minus(term(1, 2)?, term(2, 1)?)),
                Value::Scalar(minus(term(2, 0)?, term(0, 2)?)),
                Value::Scalar(minus(term(0, 1)?, term(1, 0)?)),
            ]))
        }
        // outerProduct(x, y)[i, j] = x[i] * y[j].
        ("outerProduct", 2) => {
            let (Value::Array(x), Value::Array(y)) = (recur(&args[0])?, recur(&args[1])?) else {
                return Err("outerProduct takes two vectors".to_string());
            };
            let rows = x
                .iter()
                .map(|xi| {
                    let xi = xi.clone().scalar()?;
                    let row = y
                        .iter()
                        .map(|yj| {
                            Ok(Value::Scalar(Expr::Bin(
                                BinOp::Mul,
                                Box::new(xi.clone()),
                                Box::new(yj.clone().scalar()?),
                            )))
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    Ok(Value::Array(row))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Value::Array(rows))
        }
        // symmetric(A) keeps A on and above the diagonal and mirrors it
        // below: B[i, j] = A[i, j] for i <= j, else A[j, i].
        ("symmetric", 1) => {
            let value = recur(&args[0])?;
            let shape = value.shape();
            let Value::Array(rows) = &value else {
                return Err("symmetric takes a square matrix".to_string());
            };
            if shape.len() != 2 || shape[0] != shape[1] {
                return Err("symmetric takes a square matrix".to_string());
            }
            let at = |i: usize, j: usize| -> Result<Value, String> {
                let Value::Array(row) = &rows[i] else {
                    return Err("symmetric takes a square matrix".to_string());
                };
                Ok(row[j].clone())
            };
            let n = shape[0];
            let out = (0..n)
                .map(|i| {
                    let row = (0..n)
                        .map(|j| if i <= j { at(i, j) } else { at(j, i) })
                        .collect::<Result<Vec<_>, String>>()?;
                    Ok(Value::Array(row))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Value::Array(out))
        }
        // skew(x) is the 3x3 matrix with skew(x) * y = cross(x, y):
        // [0, -x3, x2; x3, 0, -x1; -x2, x1, 0].
        ("skew", 1) => {
            let Value::Array(x) = recur(&args[0])? else {
                return Err("skew takes a 3-vector".to_string());
            };
            if x.len() != 3 {
                return Err("skew takes a 3-vector".to_string());
            }
            let e = |k: usize| x[k].clone().scalar();
            let zero = || Expr::Number(0.0);
            let neg = |v: Expr| Expr::Neg(Box::new(v));
            let s = |v: Expr| Value::Scalar(v);
            Ok(Value::Array(vec![
                Value::Array(vec![s(zero()), s(neg(e(2)?)), s(e(1)?)]),
                Value::Array(vec![s(e(2)?), s(zero()), s(neg(e(0)?))]),
                Value::Array(vec![s(neg(e(1)?)), s(e(0)?), s(zero())]),
            ]))
        }
        // cat(1, ...) stacks along the first dimension; cat(2, ...)
        // joins along the second.
        ("cat", n) if n >= 2 => {
            let along = constant(&args[0])?;
            let values = args[1..]
                .iter()
                .map(&recur)
                .collect::<Result<Vec<_>, String>>()?;
            match along {
                1 => {
                    let mut out = Vec::new();
                    for value in values {
                        match value {
                            Value::Array(items) => out.extend(items),
                            scalar => out.push(scalar),
                        }
                    }
                    Ok(Value::Array(out))
                }
                2 => {
                    let rows = values
                        .first()
                        .map(|value| value.shape().first().copied().unwrap_or(0))
                        .unwrap_or(0);
                    (0..rows)
                        .map(|row| {
                            let mut cells = Vec::new();
                            for value in &values {
                                let Value::Array(these) = value else {
                                    return Err("cat(2, ...) takes matrices".to_string());
                                };
                                let Some(Value::Array(row_cells)) = these.get(row) else {
                                    return Err("cat(2, ...) needs equal row counts".to_string());
                                };
                                cells.extend(row_cells.iter().cloned());
                            }
                            Ok(Value::Array(cells))
                        })
                        .collect::<Result<Vec<_>, String>>()
                        .map(Value::Array)
                }
                other => Err(format!("cat along dimension {other} is not supported")),
            }
        }
        ("linspace", 3) => {
            let (from, to) = (recur(&args[0])?.scalar()?, recur(&args[1])?.scalar()?);
            let length = constant(&args[2])?;
            if length < 2 {
                return Err("linspace needs at least two points".to_string());
            }
            Ok(Value::Array(
                (0..length)
                    .map(|index| {
                        let fraction = index as f64 / (length - 1) as f64;
                        // from + (to - from) * fraction
                        Value::Scalar(Expr::Bin(
                            BinOp::Add,
                            Box::new(from.clone()),
                            Box::new(Expr::Bin(
                                BinOp::Mul,
                                Box::new(Expr::Bin(
                                    BinOp::Sub,
                                    Box::new(to.clone()),
                                    Box::new(from.clone()),
                                )),
                                Box::new(Expr::Number(fraction)),
                            )),
                        ))
                    })
                    .collect(),
            ))
        }
        _ => {
            // `Complex(1, 2)` builds a record. A declared
            // `'constructor'` is called if the record has one; failing
            // that, the fields are taken in the order they were
            // declared.
            if let Some(class) = lookup(registry, name, scope, imports) {
                if class.kind == ClassKind::Record
                    && operator_function(registry, &class.name, "constructor", args.len()).is_some()
                {
                    return apply_operator(
                        &class.name.clone(),
                        "constructor",
                        args,
                        shapes,
                        registry,
                        scope,
                        imports,
                        depth,
                    );
                }
                if class.kind == ClassKind::Record {
                    let fields = record_fields(class);
                    if fields.len() != args.len() {
                        return Err(format!(
                            "`{}` is built from {} field(s), {} given",
                            class.name,
                            fields.len(),
                            args.len()
                        ));
                    }
                    return Ok(Value::Array(args.iter().map(&recur).collect::<Result<
                        Vec<_>,
                        String,
                    >>(
                    )?));
                }
            }
            // A user function that takes or returns an array is inlined
            // with the arrays intact - vectorizing it element by element
            // would compute something else entirely.
            if let Some(class) = lookup(registry, name, scope, imports) {
                if class.kind == ClassKind::Function && takes_or_gives_an_array(class, registry) {
                    let values = args
                        .iter()
                        .map(&recur)
                        .collect::<Result<Vec<_>, String>>()?;
                    // The shape each argument turned out to have is
                    // what a `[:]` input takes its length from.
                    let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
                    let arguments: Vec<Expr> =
                        values.into_iter().map(|value| value.into_expr()).collect();
                    let result = inline_function(
                        class,
                        &arguments,
                        &argument_shapes,
                        shapes.consts,
                        registry,
                        depth + 1,
                    )?;
                    // A call that could not be inlined comes back as
                    // itself. Expanding it again would ask the same
                    // question and get the same answer, for ever; and
                    // there is nothing else to do with it here, since
                    // what stands in for an un-inlined call is a walk,
                    // and a walk carries numbers rather than arrays.
                    if matches!(&result, Expr::Call(called, _) if called == &class.name) {
                        return Err(format!(
                            "`{}` gives back an array and cannot be worked out here: a call \
                             left standing is walked at run time, and a walk carries numbers",
                            class.name
                        ));
                    }
                    return expand(&result, shapes, registry, scope, imports, depth + 1);
                }
            }
            // Anything else: an ordinary call, applied to every element
            // when an argument turns out to be an array.
            let values = args
                .iter()
                .map(&recur)
                .collect::<Result<Vec<_>, String>>()?;
            let arrayed = values.iter().any(|value| matches!(value, Value::Array(_)));
            if !arrayed {
                let scalars = values
                    .into_iter()
                    .map(Value::scalar)
                    .collect::<Result<Vec<_>, String>>()?;
                return resolve(
                    &Expr::Call(name.to_string(), scalars),
                    shapes.loop_vars,
                    shapes.consts,
                    shapes.sizes,
                    registry,
                    scope,
                    imports,
                    depth + 1,
                )
                .map(Value::Scalar);
            }
            // The call spreads over the elements, a scalar argument
            // travelling unchanged to every one - the vectorization the
            // language gives every scalar function.
            let length = values
                .iter()
                .filter_map(|value| match value {
                    Value::Array(items) => Some(items.len()),
                    Value::Scalar(_) => None,
                })
                .max()
                .expect("at least one argument is an array");
            if values
                .iter()
                .any(|value| matches!(value, Value::Array(items) if items.len() != length))
            {
                return Err(format!(
                    "`{name}`: its array arguments must have one length"
                ));
            }
            let elements = (0..length)
                .map(|index| {
                    let args = values
                        .iter()
                        .map(|value| match value {
                            Value::Array(items) => items[index].clone().scalar(),
                            Value::Scalar(expr) => Ok(expr.clone()),
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    // Through `resolve`, so a user function still
                    // inlines per element instead of surviving as a call.
                    resolve(
                        &Expr::Call(name.to_string(), args),
                        shapes.loop_vars,
                        shapes.consts,
                        shapes.sizes,
                        registry,
                        scope,
                        imports,
                        depth + 1,
                    )
                    .map(Value::Scalar)
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Value::Array(elements))
        }
    }
}

/// Combine two values with an arithmetic operator.
///
/// Written with a dot the operator always works element by element.
/// Written plainly, `+` and `-` do too, while `*` between two vectors is
/// their scalar product and `/` only divides by a scalar - which is what
/// the language means by them.
pub(super) fn combine(
    op: BinOp,
    left: &Value,
    right: &Value,
    elementwise: bool,
) -> Result<Value, String> {
    let apply = |a: &Expr, b: &Expr| Expr::Bin(op, Box::new(a.clone()), Box::new(b.clone()));
    if elementwise || matches!(op, BinOp::Add | BinOp::Sub) {
        return zip_values(left, right, &apply);
    }
    match (op, left, right) {
        (BinOp::Mul, Value::Array(a), Value::Array(b)) => {
            let (left_shape, right_shape) = (left.shape(), right.shape());
            match (left_shape.len(), right_shape.len()) {
                // Vector times vector is their scalar product.
                (1, 1) => {
                    if a.len() != b.len() {
                        let (mut left_items, mut right_items) = (Vec::new(), Vec::new());
                        left.flatten_into(&mut left_items);
                        right.flatten_into(&mut right_items);
                        return Err(format!(
                            "a scalar product needs equal lengths, got {} and {}: \
                             {:?} against {:?}",
                            a.len(),
                            b.len(),
                            left_items.first(),
                            right_items.first()
                        ));
                    }
                    let products = zip_values(left, right, &apply)?;
                    let mut terms = Vec::new();
                    products.flatten_into(&mut terms);
                    Ok(Value::Scalar(sum_of(terms)))
                }
                // Matrix times vector, vector times matrix, and matrix
                // times matrix follow the usual row-by-column rule.
                (2, 1) => a
                    .iter()
                    .map(|row| combine(BinOp::Mul, row, right, false))
                    .collect::<Result<Vec<_>, String>>()
                    .map(Value::Array),
                (1, 2) => {
                    let columns = right_shape[1];
                    (0..columns)
                        .map(|column| {
                            let column = pick_column(b, column)?;
                            combine(BinOp::Mul, left, &column, false)
                        })
                        .collect::<Result<Vec<_>, String>>()
                        .map(Value::Array)
                }
                (2, 2) => a
                    .iter()
                    .map(|row| combine(BinOp::Mul, row, right, false))
                    .collect::<Result<Vec<_>, String>>()
                    .map(Value::Array),
                _ => Err("`*` between arrays deeper than matrices".to_string()),
            }
        }
        (BinOp::Div, _, Value::Array(_)) => {
            Err("an array cannot be a divisor; use `./` for element by element".to_string())
        }
        _ => zip_values(left, right, &apply),
    }
}

/// Whether any argument or answer of a function is an array.
///
/// The dimensions may be written on the declaration - `input Real
/// u[3]` - or come with its type, as they do for `input Orientation T`
/// where `type Orientation = Real[3, 3]`. Both count: a function that
/// deals in arrays is inlined with them intact rather than applied to
/// one element at a time.
fn takes_or_gives_an_array(class: &ClassDef, registry: &HashMap<&str, &ClassDef>) -> bool {
    class.components.iter().any(|component| {
        if component.causality == Causality::None {
            return false;
        }
        let mut component = component.clone();
        resolve_type(registry, &mut component, &class.name, &class.imports);
        !component.dimensions.is_empty()
    })
}

/// One column of a matrix given by rows, as a vector value.
pub(super) fn pick_column(rows: &[Value], column: usize) -> Result<Value, String> {
    rows.iter()
        .map(|row| {
            let Value::Array(cells) = row else {
                return Err("a matrix row must be an array".to_string());
            };
            cells
                .get(column)
                .cloned()
                .ok_or_else(|| "the rows of a matrix must be equally wide".to_string())
        })
        .collect::<Result<Vec<_>, String>>()
        .map(Value::Array)
}

/// The sum of a list of expressions, or zero when it is empty.
pub(super) fn sum_of(terms: Vec<Expr>) -> Expr {
    terms
        .into_iter()
        .reduce(|a, b| Expr::Bin(BinOp::Add, Box::new(a), Box::new(b)))
        .unwrap_or(Expr::Number(0.0))
}

/// Apply a scalar operation to every element of a value.
pub(super) fn map_value(value: &Value, f: &dyn Fn(Expr) -> Expr) -> Value {
    match value {
        Value::Scalar(expr) => Value::Scalar(f(expr.clone())),
        Value::Array(items) => Value::Array(items.iter().map(|item| map_value(item, f)).collect()),
    }
}

/// Pair two values element by element, broadcasting a scalar over an
/// array. Arrays of different shapes are an error, not a guess.
pub(super) fn zip_values(
    left: &Value,
    right: &Value,
    f: &dyn Fn(&Expr, &Expr) -> Expr,
) -> Result<Value, String> {
    Ok(match (left, right) {
        (Value::Scalar(a), Value::Scalar(b)) => Value::Scalar(f(a, b)),
        (Value::Array(items), Value::Scalar(b)) => Value::Array(
            items
                .iter()
                .map(|item| zip_values(item, &Value::Scalar(b.clone()), f))
                .collect::<Result<Vec<_>, String>>()?,
        ),
        (Value::Scalar(a), Value::Array(items)) => Value::Array(
            items
                .iter()
                .map(|item| zip_values(&Value::Scalar(a.clone()), item, f))
                .collect::<Result<Vec<_>, String>>()?,
        ),
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Err(format!(
                    "arrays of {} and {} elements do not fit together",
                    a.len(),
                    b.len()
                ));
            }
            Value::Array(
                a.iter()
                    .zip(b)
                    .map(|(x, y)| zip_values(x, y, f))
                    .collect::<Result<Vec<_>, String>>()?,
            )
        }
    })
}

/// The scalar references an array name stands for: `v` of `Real v[3]`
/// is `{v[1], v[2], v[3]}`.
pub(super) fn elements_of(name: &str, sizes: &[i64]) -> Value {
    match sizes.split_first() {
        None => Value::Scalar(Expr::Ref(name.to_string())),
        Some((&length, rest)) => Value::Array(
            (1..=length)
                .map(|index| {
                    if rest.is_empty() {
                        Value::Scalar(Expr::Ref(element_name(name, &[index])))
                    } else {
                        // Deeper dimensions keep the subscripts together,
                        // which is how the flat names are written.
                        let inner = elements_of(name, rest);
                        prefix_subscript(&inner, name, index)
                    }
                })
                .collect(),
        ),
    }
}

/// Split `plug.pin.v` into the array `plug.pin` and the member `v`,
/// where some prefix of the name is an array this table has measured.
/// The longest prefix wins: `a.b.c` with both `a` and `a.b` measured is
/// the member `c` of the array `a.b`.
fn member_of_array<'a>(
    name: &'a str,
    sizes: &HashMap<String, Vec<i64>>,
) -> Option<(&'a str, &'a str)> {
    let mut cut = name.rfind('.')?;
    loop {
        let (array, member) = (&name[..cut], &name[cut + 1..]);
        if sizes.contains_key(array) {
            return Some((array, member));
        }
        cut = array.rfind('.')?;
    }
}

/// Put an outer subscript in front of the ones a nested value carries.
pub(super) fn prefix_subscript(value: &Value, name: &str, index: i64) -> Value {
    match value {
        Value::Scalar(Expr::Ref(inner)) => {
            let subscripts = inner
                .strip_prefix(name)
                .and_then(|rest| rest.strip_prefix('['))
                .and_then(|rest| rest.strip_suffix(']'))
                .unwrap_or_default();
            Value::Scalar(Expr::Ref(format!("{name}[{index},{subscripts}]")))
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| prefix_subscript(item, name, index))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Dimensions of every array component of a class and of its bases.
pub(super) fn collect_shapes(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    consts: &HashMap<String, f64>,
    out: &mut HashMap<String, Vec<i64>>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let scope = class.name.as_str();
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, scope, &class.imports) {
            collect_shapes(registry, base, consts, out, depth + 1);
        }
    }
    for component in &class.components {
        // A type may be an array of its own - `type Axis = Real[3]` -
        // and then the declaration is that shape whether or not it
        // wrote any dimensions itself.
        let mut component = component.clone();
        resolve_type(registry, &mut component, scope, &class.imports);
        let component = &component;
        if component.dimensions.is_empty() {
            continue;
        }
        // Declarations are visited in source order, so a length
        // written as `size(v, 1)` can look up a `v` already measured -
        // which is how a function's result takes the shape of its
        // argument. A type dimension (`Boolean`, an enumeration) or a
        // flexible `:` read from the declaration's value is measured
        // the same way it is when the component is instantiated.
        let sizes: Option<Vec<i64>> = component
            .dimensions
            .iter()
            .enumerate()
            .map(|(axis, dimension)| match dimension {
                Expr::Ref(name) if name == "Boolean" => Some(2),
                Expr::Ref(name) => lookup(registry, name, scope, &class.imports)
                    .filter(|c| !c.enumeration.is_empty())
                    .map(|c| c.enumeration.len() as i64)
                    .or_else(|| dimension_value(dimension, consts, out)),
                Expr::ColonSubscript => component
                    .binding
                    .as_ref()
                    .and_then(|binding| flexible_size(binding, axis)),
                _ => dimension_value(dimension, consts, out),
            })
            .collect();
        if let Some(sizes) = sizes {
            out.insert(component.name.clone(), sizes);
        }
    }
}

/// An array of the given shape, every element the same.
fn nested(lengths: &[i64], value: &Expr) -> Value {
    match lengths.split_first() {
        None => Value::Scalar(value.clone()),
        Some((&length, rest)) => Value::Array((0..length).map(|_| nested(rest, value)).collect()),
    }
}

/// The shape of a value, as the dimension tables spell it.
pub(super) fn shape_i64(value: &Value) -> Vec<i64> {
    value
        .shape()
        .into_iter()
        .map(|length| length as i64)
        .collect()
}

/// One array dimension as a number, or `None` when it cannot be told
/// here - a colon waiting for a call site, or a length that depends on
/// something not yet known.
pub(super) fn dimension_value(
    expr: &Expr,
    consts: &HashMap<String, f64>,
    sizes: &HashMap<String, Vec<i64>>,
) -> Option<i64> {
    // `size(v)` and `size(v, k)` of something already measured.
    if let Expr::Call(name, args) = expr {
        if name == "size" && !args.is_empty() {
            if let Expr::Ref(of) = &args[0] {
                let shape = sizes.get(of)?;
                let index = match args.get(1) {
                    None => 0,
                    Some(dimension) => const_eval(dimension, consts)? as usize - 1,
                };
                return shape.get(index).copied();
            }
        }
    }
    let value = const_eval(expr, consts)?;
    (value.fract() == 0.0 && value >= 0.0).then_some(value as i64)
}

/// The same shapes under the instance path, since equations are
/// prefixed before they are expanded.
pub(super) fn prefixed_sizes(
    sizes: &HashMap<String, Vec<i64>>,
    prefix: &str,
) -> HashMap<String, Vec<i64>> {
    sizes
        .iter()
        .map(|(name, dimensions)| (format!("{prefix}{name}"), dimensions.clone()))
        .collect()
}

/// Emit one equation per element, refusing sides that do not match.
pub(super) fn push_equations(lhs: &Value, rhs: &Value, acc: &mut Flat) -> Result<(), String> {
    let (left_shape, right_shape) = (lhs.shape(), rhs.shape());
    if left_shape != right_shape {
        let (mut left, mut right) = (Vec::new(), Vec::new());
        lhs.flatten_into(&mut left);
        rhs.flatten_into(&mut right);
        return Err(format!(
            "an equation between shapes {left_shape:?} and {right_shape:?}: {} = {}",
            left.first().map_or("()".to_string(), |e| format!("{e:?}")),
            right.first().map_or("()".to_string(), |e| format!("{e:?}")),
        ));
    }
    let (mut left, mut right) = (Vec::new(), Vec::new());
    lhs.flatten_into(&mut left);
    rhs.flatten_into(&mut right);
    for (lhs, rhs) in left.into_iter().zip(right) {
        let origin = acc.origin.clone();
        acc.equations.push(EquationItem { lhs, rhs, origin });
    }
    Ok(())
}

/// No record instances are in scope here.
pub(super) fn no_records() -> &'static HashMap<String, String> {
    static EMPTY: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}
