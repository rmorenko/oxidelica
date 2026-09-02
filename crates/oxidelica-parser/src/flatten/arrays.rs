//! The array layer: values with a shape, and what may be done to
//! them before everything drops to scalars.

use super::*;

/// Expand an expression into scalars, keeping the array structure while
/// it is needed and dropping to the scalar path for everything else.
#[allow(clippy::too_many_arguments)]
thread_local! {
    /// What an expression came to while one class is being
    /// instantiated. Held for exactly that long: see `expand`.
    pub(super) static EXPANDED: std::cell::RefCell<HashMap<String, Result<Value, String>>> =
        std::cell::RefCell::new(HashMap::new());
}

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
    // What an expression comes to is asked over and over: a multibody
    // model builds an orientation from the same handful of names in
    // every equation of a body, and walks it whole each time.
    // Measured on `DoublePendulum`: forty million expansions, and
    // within one class's instantiation only 641 distinct questions -
    // the other thirty-five million are the same question again.
    //
    // The bracket is what makes the key simple. While one class is
    // being instantiated its parameters do not move, which is the
    // same invariant `Inlined::open` was written for, so the table
    // lives exactly that long and is cleared on the same beat. Inside
    // it an expression and a scope are the whole of the question -
    // the shapes and numbers in view belong to the class being built,
    // and the class being built is what the bracket holds still.
    //
    // Not at the top: a caller may hand different shapes in per call
    // (`loop_vars` above all), and those callers are the ones asking
    // at depth nothing.
    // The mark belongs to the key beside the scope: one expression
    // asked under two media is two answers, and the mark is what
    // tells them apart everywhere else in this flattener.
    // Only for an expression worth remembering. A name, a number or
    // a subscript is answered in a few instructions, and building a
    // key for one costs more than the answer - it is the built-up
    // arithmetic of an orientation that is asked a hundred thousand
    // times and walked whole each time.
    let worth_it = matches!(
        expr,
        Expr::Bin(..) | Expr::Call(..) | Expr::If(..) | Expr::Array(_) | Expr::Elementwise(..)
    );
    let remembering = depth > 0 && worth_it && shapes.loop_vars.is_empty();
    let key = match remembering {
        false => None,
        true => {
            // And the shapes of the names this expression writes: a
            // value worked out inside another class - `Root r(s =
            // anyT(suspend.reset))` - reads a run of ports the class
            // above knows about and this one does not, so the same
            // written expression under two tables is two answers.
            // Only the names in hand, never the table itself: hashing
            // thousands of entries per asking is the cost this is
            // meant to remove.
            let mut key = format!("{scope}|{}|{expr:?}", super::inlining::asked_as_mark());
            let mut named: Vec<&str> = Vec::new();
            expr.for_each(&mut |inner| {
                if let Expr::Ref(name) = inner {
                    named.push(name.as_str());
                }
            });
            named.sort_unstable();
            named.dedup();
            for name in named {
                if let Some(shape) = shapes.sizes.get(name) {
                    key.push_str(&format!("|{name}{shape:?}"));
                }
                // A member walked off an array - `suspend.reset`
                // where `suspend` is the run - is not in the table
                // under the name written, so the array it belongs to
                // is asked for as well.
                else if let Some((array, _)) = member_of_array(name, shapes.sizes) {
                    key.push_str(&format!("|{name}@{:?}", shapes.sizes[array]));
                }
                if let Some(record) = shapes.records.get(name) {
                    key.push_str(&format!("|{name}:{record}"));
                }
            }
            Some(key)
        }
    };
    if let Some(key) = &key {
        if let Some(held) = EXPANDED.with(|held| held.borrow().get(key).cloned()) {
            return held;
        }
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

    let constant_here = |e: &Expr| -> Option<f64> { settled_by(e, shapes) };
    let answer = (|| -> Result<Value, String> {
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
                        return Err(
                            "the parts of one row of a matrix must be equally tall".to_string()
                        );
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
                // A field may be an array of its own - a rotation is a
                // three by three and a rate of three - so each is worked
                // out rather than named.
                Value::Array(
                    record_fields_of(registry, of, 0)
                        .into_iter()
                        .map(|field| recur(&Expr::Ref(format!("{name}.{field}"))))
                        .collect::<Result<Vec<_>, String>>()?,
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
            // A logical operator is worked out here rather than left for
            // the scalar path, because its sides may still be things only
            // this pass can settle: `size(b, 1) > 0 and max(b)` asks the
            // length and the largest of a vector, and both answers are
            // scalars once they have been looked at with the shapes to
            // hand. Left to the scalar path the vector arrives whole and
            // is refused for being an array.
            Expr::Not(inner) => Value::Scalar(Expr::Not(Box::new(recur(inner)?.scalar()?))),
            Expr::And(l, r) => Value::Scalar(Expr::And(
                Box::new(recur(l)?.scalar()?),
                Box::new(recur(r)?.scalar()?),
            )),
            Expr::Or(l, r) => Value::Scalar(Expr::Or(
                Box::new(recur(l)?.scalar()?),
                Box::new(recur(r)?.scalar()?),
            )),
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
                    // A check a branch makes holds only when that branch
                    // is the one taken.
                    let before_first = checks_mark();
                    let first = recur(first)?;
                    checks_guarded(before_first, &condition, stands != Some(false));
                    // A branch that comes to nothing leaves no checks
                    // behind either.
                    let mark = checks_mark();
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
                        (Err(_), Some(_)) => {
                            checks_rewind(mark);
                            return Ok(first);
                        }
                        (Err(trouble), None) => {
                            checks_rewind(mark);
                            return Err(trouble);
                        }
                    };
                    checks_guarded(mark, &condition, stands == Some(false));
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
                    // One branch stands and the other is dropped, so the
                    // checks it made go with it.
                    checks_rewind(mark);
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
            Expr::Call(name, args) => {
                expand_call(name, args, shapes, registry, scope, imports, depth)?
            }
            // Indexing something that expands to an array picks the element:
            // this is how `a[i]` works inside a function whose `a` was bound
            // to an array literal.
            Expr::Index(base, subscripts) => {
                let base_value = recur(base)?;
                match base_value {
                    Value::Array(_) => index_into(
                        base_value, subscripts, shapes, registry, scope, imports, depth,
                    )
                    .map_err(|why| match why.starts_with("subscript ") {
                        // A refusal about a subscript is worth little
                        // without the name it was written on: `subscript 1
                        // is outside an array of 0` said which model was
                        // refused and nothing about where to look in it.
                        true => format!(
                            "{why}, reading `{}`",
                            names::sketch(&Expr::Index(base.clone(), subscripts.clone()))
                        ),
                        false => why,
                    })?,
                    // A name subscripted by a range is a slice of it, and
                    // an empty range slices nothing: `X_default[1:nXi]`
                    // with no trace substances is the empty array, not a
                    // scalar. The name's own shape is not needed to say
                    // so - the range says it - and without this the whole
                    // `Index` went off to be resolved as a scalar, where
                    // a range is refused for being an array.
                    _ if empty_range_subscript(subscripts, shapes) => Value::Array(Vec::new()),
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
            // A named argument is its value under a name, and the value
            // may be an array: `actual = f(dps_fg, ...)` is how the
            // library writes a homotopy, and the whole thing used to fall
            // through to the scalar path - taking the call inside it
            // along, where a function is inlined whole and an array is
            // bound to an input that takes one number. Expanded here, the
            // call inside reaches the hand-out that applies a scalar
            // function element by element, which is what the language
            // says it means.
            Expr::NamedArg(name, value) => map_value(&recur(value)?, &|element| {
                Expr::NamedArg(name.clone(), Box::new(element.clone()))
            }),
            other => scalar(other)?,
        })
    })();
    if let Some(key) = key {
        EXPANDED.with(|held| held.borrow_mut().insert(key, answer.clone()));
    }
    answer
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
    let constant_here = |e: &Expr| -> Option<f64> { settled_by(e, shapes) };
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
            let Some(value) = constant_here(&index) else {
                // A subscript that is not settled until the run - the
                // logic tables read `NotTable[x]` off a signal - picks
                // its element by asking. Every element is a candidate,
                // and what comes out is the one whose place the index
                // names: a chain of `if index == k then a[k]`. Every
                // place is asked, the last one included, so an index
                // outside the array falls through to a value that is
                // no number rather than quietly taking the end.
                let mut picked = Vec::with_capacity(items.len());
                for item in items {
                    picked.push(inner(item)?.scalar()?);
                }
                let mut chosen = Expr::Number(f64::NAN);
                for (place, candidate) in picked.into_iter().enumerate().rev() {
                    // A Boolean index names its place by being `false`
                    // or `true` rather than by counting: comparing one
                    // against a number is comparing two different
                    // kinds, which the checker refuses outright.
                    let names = match is_boolean(&with_end) {
                        true => Expr::Bool(place != 0),
                        false => Expr::Number(place as f64 + 1.0),
                    };
                    chosen = Expr::If(
                        Box::new(Expr::Rel(
                            crate::ast::RelOp::Eq,
                            Box::new(index.clone()),
                            Box::new(names),
                        )),
                        Box::new(candidate),
                        Box::new(chosen),
                    );
                }
                return Ok(Value::Scalar(chosen));
            };
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

/// A call left standing for the run to walk, as the value that stands
/// in for it.
///
/// The walk answers with numbers, and how many is what the function
/// declares: one for a scalar, as many as the length for an array, as
/// many as the members for a record. The model takes them one at a
/// time, by the subscript Modelica would write. A shape said wrongly
/// here is a shape said wrongly everywhere below, so a length that
/// cannot be seen answers as the one number a walk always gives.
fn standing_call(
    call: Expr,
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
    imports: &[(String, String)],
) -> Value {
    let inherited = inlining::function_components(registry, class, 0);
    let answer = inherited.iter().find(|c| c.causality == Causality::Output);
    let length = answer.and_then(|answer| match answer.dimensions.as_slice() {
        [Expr::Number(length)] => Some(*length as i64),
        // A record answers with its members, in the order it declared
        // them: the walk is handed it written that way.
        [] => lookup(registry, &answer.type_name, &class.name, imports)
            .filter(|of| of.kind == ClassKind::Record)
            .map(|of| record_fields_of(registry, of, 0).len() as i64)
            .filter(|members| *members > 0),
        _ => None,
    });
    match length {
        None => Value::Scalar(call),
        Some(length) => Value::Array(
            (1..=length)
                .map(|index| {
                    Value::Scalar(Expr::Index(
                        Box::new(call.clone()),
                        vec![Expr::Number(index as f64)],
                    ))
                })
                .collect(),
        ),
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
        let value = settled_by(&value, shapes).ok_or_else(|| {
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
        // A body written here in Rust takes what it was handed as it
        // was handed it - an array stays an array, so the run knows
        // how many numbers went into which argument - and answers with
        // one flat list, which a subscript takes a place of. Nothing
        // spreads over the elements: the body was written for the
        // whole.
        // A call the run walks is the same case: what it was handed
        // travels whole, and the answer is one flat list a subscript
        // takes a place of.
        (_, _)
            if crate::outside::written_here(name)
                || super::names::stands_for_the_run_here(name, scope) =>
        {
            let handed = args
                .iter()
                .map(|arg| Ok(recur(arg)?.into_expr()))
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Value::Scalar(Expr::Call(name.to_string(), handed)))
        }
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
            // A table block declares `table` empty and fills it from a
            // file: asked how wide it is before the run, the shape of
            // the declaration says nothing, and the file says
            // everything. The same reading the sizes were measured by.
            // The declaration of a table on a file is `fill(0.0, 0,
            // 2)`: it has both dimensions, and the first of them is a
            // zero that says nothing about the numbers waiting in the
            // file. So the file answers wherever it can, not only
            // where the declaration is short of a dimension.
            let empty_here = shape
                .get((dimension - 1).max(0) as usize)
                .is_none_or(|length| *length == 0);
            if empty_here {
                let in_view = super::statements::texts_in_view();
                let text_of = |wanted: &str| in_view.get(wanted).cloned();
                let truth_of = |wanted: &str| shapes.consts.get(wanted).map(|value| *value != 0.0);
                let asks_a_table = matches!(&args[0], Expr::Ref(name)
                    if name == "table" || name.ends_with(".table"));
                if asks_a_table && truth_of("tableOnFile") == Some(true) {
                    if let (Some(file), Some(named)) = (text_of("fileName"), text_of("tableName")) {
                        if let Ok(rows) = super::table_files::table_in_file(&file, &named) {
                            let length = match dimension {
                                1 => Some(rows.len()),
                                2 => rows.first().map(|row| row.len()),
                                _ => None,
                            };
                            if let Some(length) = length {
                                return Ok(Value::Scalar(Expr::Number(length as f64)));
                            }
                        }
                    }
                }
            }
            let length = shape.get((dimension - 1).max(0) as usize).ok_or_else(|| {
                if std::env::var("OXSZ").is_ok() {
                    let mut named: Vec<&String> = shapes.sizes.keys().collect();
                    named.sort();
                    eprintln!(
                        "SZ {:?} scope={scope} known={:?}\n{}",
                        args[0],
                        &named[..named.len().min(12)],
                        std::backtrace::Backtrace::force_capture()
                    );
                }
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
        // `sum`, `product`, `min`, `max` and `vector`: one value
        // read off a whole array.
        ("sum" | "product" | "min" | "max" | "vector", 1) => {
            folded_over_an_array(name, &args[0], shapes, registry, scope, imports, depth)
        }
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
            let filler = recur(&args[0])?;
            let lengths = args[1..]
                .iter()
                .map(&constant)
                .collect::<Result<Vec<_>, String>>()?;
            Ok(nested_value(&lengths, &filler))
        }
        ("fill", 2) => {
            let filler = recur(&args[0])?;
            let length = constant(&args[1])?;
            Ok(nested_value(&[length], &filler))
        }
        // How many rows and columns a matrix on a file has, and the
        // matrix itself. Written in C in the standard library, and
        // this compiler opens those files already - it is what every
        // table block reads its numbers from.
        (
            "ModelicaIO_readMatrixSizes"
            | "ModelicaIO_readRealMatrix"
            | "Modelica.Utilities.Streams.readMatrixSize"
            | "Modelica.Utilities.Streams.readRealMatrix",
            _,
        ) => {
            let in_view = super::statements::texts_in_view();
            let text_of = |at: usize| match args.get(at) {
                Some(Expr::Str(held)) => Some(held.clone()),
                Some(Expr::Ref(name)) => in_view.get(name).cloned(),
                _ => None,
            };
            let (Some(file), Some(named)) = (text_of(0), text_of(1)) else {
                return Err(format!(
                    "`{name}` needs to know the file and the matrix by name before the run"
                ));
            };
            let rows = super::table_files::table_in_file(&file, &named)?;
            let width = rows.first().map_or(0, |row| row.len());
            match name.ends_with("Sizes") || name.ends_with("readMatrixSize") {
                true => Ok(Value::Array(vec![
                    Value::Scalar(Expr::Number(rows.len() as f64)),
                    Value::Scalar(Expr::Number(width as f64)),
                ])),
                false => Ok(Value::Array(
                    rows.iter()
                        .map(|row| {
                            Value::Array(
                                row.iter()
                                    .map(|held| Value::Scalar(Expr::Number(*held)))
                                    .collect(),
                            )
                        })
                        .collect(),
                )),
            }
        }
        // The array constructors and rearrangers: a shape built or a
        // shape turned about.
        (
            "transpose" | "identity" | "diagonal" | "cross" | "outerProduct" | "symmetric" | "skew"
            | "cat" | "linspace",
            _,
        ) => shaped_by_a_builtin(name, args, shapes, registry, scope, imports, depth),
        (PARTIAL_CALL, _) => Err(format!(
            "a function is given as an argument with some of what it takes already filled \
             in - {} - and there is nothing here to pass a function around in",
            args.first()
                .map_or(PARTIAL_CALL.to_string(), crate::flatten::names::sketch)
        )),
        _ if name.starts_with("Connections.") => {
            // `Connections.rooted(frame_a.R)` asks the connection graph
            // about a node, and the node is the name itself rather than
            // the values under it. The question is answered once the
            // graph is drawn, so the call stands as it was written.
            Ok(Value::Scalar(Expr::Call(name.to_string(), args.to_vec())))
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
                // A handle to something outside Modelica is built,
                // not called: what it is handed says what it stands
                // for, and nothing spreads over an array of it.
                if descends_from_external_object(registry, class, 0) {
                    return Ok(Value::Scalar(Expr::Call(
                        class.name.clone(),
                        handle_arguments(class, args, registry, scope, imports)?
                            .iter()
                            .map(|arg| Ok(recur(arg)?.into_expr()))
                            .collect::<Result<Vec<_>, String>>()?,
                    )));
                }
                if class.kind == ClassKind::Record {
                    // A record a class keeps a place for is built as
                    // the one the class asked under redeclared: a
                    // medium's `ThermodynamicState` has the fields
                    // that medium uses, and the base's has none at
                    // all. Where nothing was asked under, or the two
                    // are the same, this is the record it was.
                    let class = super::inlining::record_asked_under(class, registry);
                    // Given in order rather than by name, a record is
                    // built from exactly its fields.
                    let named = args.iter().any(|a| matches!(a, Expr::NamedArg(..)));
                    let fields = record_fields_of(registry, class, 0);
                    if !named && fields.len() != args.len() {
                        return Err(format!(
                            "`{}` is built from {} field(s), {} given",
                            class.name,
                            fields.len(),
                            args.len()
                        ));
                    }
                    return record_written_out(class, args, registry, &recur);
                }
            }
            // A function handed over as an argument, with some of its
            // inputs already filled in. There is nowhere to put a
            // function value and nowhere it would survive to - the
            // walk takes numbers - so the receiving function is
            // specialized instead: a copy of it with the function
            // input replaced by ordinary numeric ones, and every call
            // to that input rewritten into a direct call of the
            // target. The language allows a function value in exactly
            // one place, an argument, so every giver and taker is
            // visible right here.
            if args
                .iter()
                .any(|arg| matches!(arg, Expr::Call(head, _) if head == PARTIAL_CALL))
            {
                if let Some(class) = lookup(registry, name, scope, imports) {
                    if class.kind == ClassKind::Function {
                        let (copy, rest) = specialized(class, args, registry, scope, imports)?;
                        let copy_name = copy.name.clone();
                        super::statements::remember_specialization(copy);
                        return expand_call(
                            &copy_name, &rest, shapes, registry, scope, imports, depth,
                        );
                    }
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
                    // A function written for one record handed a whole
                    // array of them is called once per element, which
                    // is the vectorization the language gives every
                    // function. It has to be seen before the body is
                    // written out, because the body reads the fields by
                    // name and there is no name for the fields of three
                    // records at once.
                    // A function whose inputs are all single numbers is
                    // inlined here only because it answers with a
                    // record, and handed arrays it is the plain
                    // vectorization after all: `fromPolar` over three
                    // amplitudes and three angles is three phasors. Run
                    // whole, the record's fields flattened into the row
                    // and the equation came out between a three-by-two
                    // and a two.
                    if let Some(spread) = spread_of_scalar_inputs(class, registry, &values) {
                        let elements = (0..spread)
                            .map(|index| {
                                let args = values
                                    .iter()
                                    .map(|value| match value {
                                        Value::Array(items) => items[index].clone().scalar(),
                                        Value::Scalar(expr) => Ok(expr.clone()),
                                    })
                                    .collect::<Result<Vec<_>, String>>()?;
                                expand(
                                    &Expr::Call(name.to_string(), args),
                                    shapes,
                                    registry,
                                    scope,
                                    imports,
                                    depth + 1,
                                )
                            })
                            .collect::<Result<Vec<_>, String>>()?;
                        return Ok(Value::Array(elements));
                    }
                    if let Some(spread) = spread_of_records(class, registry, shapes, &values) {
                        let elements = (0..spread)
                            .map(|index| {
                                let args =
                                    one_of_each(&values, spread, index, shapes, registry, &recur)?;
                                expand(
                                    &Expr::Call(name.to_string(), args),
                                    shapes,
                                    registry,
                                    scope,
                                    imports,
                                    depth + 1,
                                )
                            })
                            .collect::<Result<Vec<_>, String>>()?;
                        return Ok(Value::Array(elements));
                    }
                    // The shape each argument turned out to have is
                    // what a `[:]` input takes its length from.
                    let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
                    let arguments: Vec<Expr> =
                        values.into_iter().map(|value| value.into_expr()).collect();
                    // Worked out under the name the call wrote: a
                    // media function called as `Medium.density` has
                    // to find the medium's own records, not the ones
                    // of the base that wrote it.
                    // The name as the call wrote it, and where the
                    // site could resolve its head: `Medium.density`
                    // is written against a package the model named,
                    // and the body has never heard that name.
                    // Worked out under the name the call wrote, with
                    // the head of that name put through the same
                    // lookup as any class: `Medium.density` names a
                    // package the model called `Medium`, and the body
                    // has never heard that name. A call written with
                    // no path at all - one body calling another of
                    // the same base - keeps whatever scope it was
                    // already being worked out under, which is how a
                    // medium's own functions are reached from a body
                    // of the base.
                    // Only where the body might want it: a function
                    // that takes a record of a kind its own class
                    // leaves for another to fill is the one this is
                    // for, and asking for anything else is work for
                    // nothing. The head of the name is put through
                    // the same lookup as any class, since a call may
                    // be written against a package the model named.
                    // The head of the name put through the same
                    // lookup as any class, since a call may be
                    // written against a package the model named:
                    // `Medium.density` where `Medium` is whatever
                    // this model said it was.
                    // Only where the class was reached through a name
                    // of its own rather than by its full path: a
                    // media function is called as `Medium.density`,
                    // where `Medium` is whatever this model said it
                    // was, and that is the one case the body's own
                    // names may mean something else. A call written
                    // by the path the class actually has says nothing
                    // new, and nothing is asked about it.
                    let _asked = name.rsplit_once('.').and_then(|(head, _)| {
                        (!class.name.starts_with(head)).then(|| {
                            let package = lookup(registry, head, scope, imports)?;
                            super::inlining::AskedAs::under(&package.name)
                        })?
                    });
                    // The function the call really means: the medium
                    // this was asked under may have redeclared it
                    // with inputs its base never had.
                    let class = inlining::function_asked_under(class, registry);
                    let result = inlining::inline_function(
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
                    // A call that could not be inlined comes back as
                    // itself, and the run walks it. The walk takes
                    // arrays and answers with one number, so a call
                    // standing here is a number - and one that would
                    // answer otherwise is refused where the body is
                    // gathered, by name.
                    if matches!(&result, Expr::Call(called, _) if called == &class.name) {
                        // A call left standing is walked, and a walk
                        // may answer with several numbers. The model
                        // takes them one at a time, by the subscript
                        // Modelica would write.
                        // A `redeclare function extends density`
                        // writes a body and nothing else: what it
                        // answers with is its base's declaration, not
                        // its own. Reading the components it wrote
                        // found none, and a function with no output
                        // was taken for one answering with a record of
                        // as many members as the input had - so a
                        // density came back as a state of two.
                        return Ok(standing_call(result, class, registry, imports));
                    }
                    // What the inlining built has to be read once more,
                    // since a body written in arrays answers with one.
                    // That reading is a walk of whatever was built, and
                    // what a steam table builds is enormous: the water
                    // properties fold a property of a property of a
                    // state, and the tree runs past the depth this
                    // compiler follows.
                    //
                    // Failing there was fatal, and it need not be. A
                    // call that could not be built at all is left
                    // standing for the run to walk, and one that was
                    // built but cannot be carried means the same
                    // thing: the run will walk it. So the refusal is
                    // caught and answered the way the other is, which
                    // also spares the reading that was going to fail.
                    let carried = expand(&result, shapes, registry, scope, imports, depth + 1);
                    return match carried {
                        Err(why) if why.contains(crate::flatten::algorithms::NO_BOTTOM) => {
                            // Left standing the same way, shape and
                            // all: a body answering with several
                            // numbers is taken one at a time, and
                            // saying it is a scalar would be a shape
                            // that lies.
                            let standing = Expr::Call(class.name.clone(), arguments);
                            Ok(standing_call(standing, class, registry, imports))
                        }
                        answered => answered,
                    };
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
                    let call = Expr::Call(name.to_string(), args);
                    let element = resolve(
                        &call,
                        shapes.loop_vars,
                        shapes.consts,
                        shapes.sizes,
                        registry,
                        scope,
                        imports,
                        depth + 1,
                    )?;
                    // A scalar function answering with a record answers
                    // with a record per element: `fromPolar` over three
                    // amplitudes and three angles is three phasors, not
                    // one array of six numbers. Read as a scalar the two
                    // fields flattened into the row and the equation was
                    // between a three-by-two and a two.
                    match &element {
                        Expr::Array(_) => {
                            expand(&element, shapes, registry, scope, imports, depth + 1)
                        }
                        _ => Ok(Value::Scalar(element)),
                    }
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Value::Array(elements))
        }
    }
}

/// The builtins that build a shape or turn one about: `transpose`,
/// `identity`, `diagonal`, `cross`, `outerProduct`, `symmetric`,
/// `skew`, `cat` and `linspace`.
///
/// Moved out of `expand_call` unchanged.
#[allow(clippy::too_many_arguments)]
fn shaped_by_a_builtin(
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
        let value = settled_by(&value, shapes).ok_or_else(|| {
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
        _ => Err(format!(
            "`{name}` is not one this compiler builds shapes with"
        )),
    }
}

/// One value read off a whole array: what `sum`, `product`, `min`,
/// `max` and `vector` come to.
///
/// Moved out of `expand_call` unchanged.
#[allow(clippy::too_many_arguments)]
fn folded_over_an_array(
    name: &str,
    arg: &Expr,
    shapes: &Shapes,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
) -> Result<Value, String> {
    let recur = |e: &Expr| expand(e, shapes, registry, scope, imports, depth + 1);
    let args = std::slice::from_ref(arg);
    match (name, args.len()) {
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
                    let fields = record_fields_of(registry, of, 0);
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
            // A name that reads a member off an array of components -
            // `sum(rs.resistor.LossPower)` on an `extends` - is read
            // where the array it names has not been built yet, so
            // nothing here knows it is one and it looks like a
            // scalar. Summing it now would come to the name itself.
            // Left standing, it is written out and summed once every
            // shape is in hand.
            if let Expr::Ref(named) = &args[0] {
                if named.contains('.') && !shapes.sizes.contains_key(named) {
                    return Ok(Value::Scalar(Expr::Call(
                        "sum".to_string(),
                        vec![args[0].clone()],
                    )));
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
            // The same name that a sum cannot read yet, for the same
            // reason: the array it belongs to is not built.
            if let Expr::Ref(named) = &args[0] {
                if named.contains('.') && !shapes.sizes.contains_key(named) {
                    return Ok(Value::Scalar(Expr::Call(
                        name.to_string(),
                        vec![args[0].clone()],
                    )));
                }
            }
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
        _ => unreachable!("only the folds reach here"),
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

/// A record written out: `Complex(1, 2)`, `Orientation(T = ..., w =
/// ...)`.
///
/// A record instance already comes out of the array layer as its
/// members in the order they were declared, so one written out comes
/// out the same way and the equation between them lines up member by
/// member. Members may be given in order or by name, and one nobody
/// gave stands on the value its declaration gives it.
fn record_written_out(
    class: &ClassDef,
    args: &[Expr],
    registry: &HashMap<&str, &ClassDef>,
    recur: &dyn Fn(&Expr) -> Result<Value, String>,
) -> Result<Value, String> {
    // Every member the record has, its bases' among them and in their
    // order: `record S extends B; Real p; end S` is `B`'s fields and
    // then `p`, which is the order everything else reads a record in.
    //
    // Reading only what the record wrote itself built a value shorter
    // than the record - the water states are four fields over a fifth
    // inherited from the two-phase medium - and the caller measuring
    // the same record honestly then said it wanted five and got four.
    // A constant of a record is not one of its members: the battery
    // parameter records inherit a `constant String CellType` that says
    // what kind of cell they describe, and it belongs to the class
    // rather than to any value of it. Counted among the members, one
    // record written out came to five things where the declaration
    // held four.
    let held: Vec<Component> = record_fields::record_components(registry, class, 0)
        .into_iter()
        .filter(|member| member.variability != Variability::Constant)
        .collect();
    let mut members = Vec::new();
    let mut position = 0;
    // A name that is nobody's member is a value going nowhere: the
    // states were built with `phase = 1` and the `phase` was dropped
    // without a word, which is a guess dressed as an answer.
    for arg in args {
        if let Expr::NamedArg(name, _) = arg {
            if !held.iter().any(|member| &member.name == name) {
                return Err(format!(
                    "`{}` written out is given `{name}`, and the record has no such member{}",
                    class.name,
                    statements::where_the_names_landed()
                ));
            }
        }
    }
    for member in &held {
        let given = args.iter().find_map(|arg| match arg {
            Expr::NamedArg(name, value) if name == &member.name => Some((**value).clone()),
            _ => None,
        });
        let given = given.or_else(|| {
            let taken = args
                .get(position)
                .filter(|arg| !matches!(arg, Expr::NamedArg(..)))
                .cloned();
            if taken.is_some() {
                position += 1;
            }
            taken
        });
        let value = given.or_else(|| member.binding.clone()).ok_or_else(|| {
            format!(
                "`{}` written out says nothing about its member `{}`, and the declaration \
                 gives it no value either",
                class.name, member.name
            )
        })?;
        members.push(recur(&value)?);
    }
    Ok(Value::Array(members))
}

/// Whether any argument or answer of a function is more than one value.
///
/// The dimensions may be written on the declaration - `input Real
/// u[3]` - or come with its type, as they do for `input Orientation T`
/// where `type Orientation = Real[3, 3]`. A record counts too: it
/// comes out of this layer as its members, which is more than one
/// value however each of them is shaped. All of them mean the same
/// thing here - the function is inlined with what it deals in intact,
/// rather than applied to one element at a time.
/// What a handle was handed, in the order its constructor declares.
///
/// A handle is built the way a function is called: by position, by
/// name, or not at all where the declaration gives a value of its own.
/// The standard library's gears build a table handle entirely by name,
/// and what is behind the handle can only be read once the names are
/// back in their places.
fn handle_arguments(
    class: &ClassDef,
    args: &[Expr],
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
) -> Result<Vec<Expr>, String> {
    let made = lookup(
        registry,
        &format!("{}.constructor", class.name),
        scope,
        imports,
    );
    let inputs: Vec<Component> = made
        .map(|made| {
            inlining::function_components(registry, made, 0)
                .into_iter()
                .filter(|c| c.causality == Causality::Input)
                .collect()
        })
        .unwrap_or_default();
    // No constructor to read the order from: the arguments stand as
    // they were written, which is what a handle built by position
    // comes to anyway.
    if inputs.is_empty() {
        return Ok(args.to_vec());
    }
    let mut placed: Vec<Option<Expr>> = vec![None; inputs.len()];
    let mut position = 0;
    for arg in args {
        match arg {
            Expr::NamedArg(name, value) => {
                let at = inputs
                    .iter()
                    .position(|input| &input.name == name)
                    .ok_or_else(|| {
                        format!("`{}` is built with no argument named `{name}`", class.name)
                    })?;
                placed[at] = Some((**value).clone());
            }
            one => {
                if position < placed.len() {
                    placed[position] = Some(one.clone());
                }
                position += 1;
            }
        }
    }
    let made = made.expect("inputs came from a constructor");
    Ok(inputs
        .iter()
        .zip(placed)
        .map(|(input, given)| match given {
            Some(given) => given,
            // What the constructor's own declaration gives, where the
            // call left it out. A name it holds is the constructor's,
            // so it is read there.
            None => input.binding.as_ref().map_or(Expr::Number(0.0), |value| {
                substitute_class_constants(value, registry, &made.name, &made.imports, &[])
            }),
        })
        .collect())
}

/// One round of a spreading call: the arguments with each spreading
/// array taken down to its `index`th record, written out as fields,
/// and everything else travelling unchanged.
pub(super) fn one_of_each(
    values: &[Value],
    spread: usize,
    index: usize,
    shapes: &Shapes,
    registry: &HashMap<&str, &ClassDef>,
    recur: &impl Fn(&Expr) -> Result<Value, String>,
) -> Result<Vec<Expr>, String> {
    values
        .iter()
        .map(|value| match value {
            Value::Array(items) if items.len() == spread => {
                let one = items[index].clone();
                match written_out(&one, shapes, registry, recur)? {
                    Some(fields) => Ok(fields.into_expr()),
                    None => Ok(one.into_expr()),
                }
            }
            other => Ok(other.clone().into_expr()),
        })
        .collect()
}

/// The class of the record a value stands for, where it stands for a
/// whole one rather than a field of one.
///
/// One element of an array of records is named rather than written
/// out: `v[1]` where `v` is `Complex[3]`. What says it is a record is
/// the array it is an element of.
fn whole_record<'a>(
    value: &Value,
    shapes: &Shapes,
    registry: &HashMap<&str, &'a ClassDef>,
) -> Option<&'a ClassDef> {
    let Value::Scalar(Expr::Ref(path)) = value else {
        return None;
    };
    // A subscript is what says this is one of an array rather than the
    // array itself: `v` of a record type has already been written out
    // as its fields by the time anything looks here.
    if !path.contains('[') {
        return None;
    }
    // A declaration is written once however many of it there are, so
    // the table is keyed without the subscripts of what is asked for -
    // but the path an instance sits at may carry subscripts of its
    // own, and those are part of the key. So the subscripts come off
    // from the right, and the first name the table knows is the one.
    let mut shortened = path.to_string();
    while let Some(open) = shortened.rfind('[') {
        let close = shortened[open..].find(']')?;
        shortened = format!("{}{}", &shortened[..open], &shortened[open + close + 1..]);
        if let Some(of) = shapes.records.get(&shortened) {
            return registry.get(of.as_str()).copied();
        }
    }
    None
}

/// Every record an array names written out as its fields.
///
/// One element of an array of records is a name - `v[1]` of a
/// `Complex[3]` - and a name is not something an equation can stand
/// between: what the model holds is `v[1].re` and `v[1].im`. So an
/// equation side is written out before the two are put together, and
/// an array of records and an array of their fields become the same
/// thing.
pub(super) fn records_written_out(
    value: Value,
    shapes: &Shapes,
    registry: &HashMap<&str, &ClassDef>,
    recur: &impl Fn(&Expr) -> Result<Value, String>,
) -> Result<Value, String> {
    Ok(match value {
        Value::Scalar(_) => match written_out(&value, shapes, registry, recur)? {
            Some(fields) => fields,
            None => value,
        },
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| records_written_out(item, shapes, registry, recur))
                .collect::<Result<Vec<_>, String>>()?,
        ),
    })
}

/// A value that names a whole record, written out as its fields in the
/// order the record declared them, each worked out where it stands.
/// `None` where the value does not name one.
fn written_out(
    value: &Value,
    shapes: &Shapes,
    registry: &HashMap<&str, &ClassDef>,
    recur: &impl Fn(&Expr) -> Result<Value, String>,
) -> Result<Option<Value>, String> {
    let Some(of) = whole_record(value, shapes, registry) else {
        return Ok(None);
    };
    let Value::Scalar(Expr::Ref(path)) = value else {
        return Ok(None);
    };
    Ok(Some(Value::Array(
        record_fields_of(registry, of, 0)
            .into_iter()
            .map(|field| recur(&Expr::Ref(format!("{path}.{field}"))))
            .collect::<Result<Vec<_>, String>>()?,
    )))
}

/// How many times a call spreads, where an input written for one
/// record was handed an array of them.
///
/// A record arrives written as an array of its fields, so one
/// `Complex` and three of them are both arrays. What tells them apart
/// is depth: the fields of one record are values, and three records
/// are three arrays. Nothing spreads unless an input asked for a
/// record and got that, and every spreading argument has to spread the
/// same number of times.
pub(super) fn spread_of_records(
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
    shapes: &Shapes,
    values: &[Value],
) -> Option<usize> {
    let inputs: Vec<Component> = inlining::function_components(registry, class, 0)
        .into_iter()
        .filter(|c| c.causality == Causality::Input)
        .collect();
    let mut spread = None;
    for (input, value) in inputs.iter().zip(values) {
        if !input.dimensions.is_empty()
            || record_fields::record_input_fields(registry, class, input).is_none()
        {
            continue;
        }
        let Value::Array(items) = value else { continue };
        // One record of an array is named rather than written out, so
        // an array of names is an array of records and an array of
        // values is the fields of one.
        if items.is_empty()
            || !items
                .iter()
                .all(|item| whole_record(item, shapes, registry).is_some())
        {
            continue;
        }
        if spread.is_some_and(|already| already != items.len()) {
            return None;
        }
        spread = Some(items.len());
    }
    spread
}

/// How many elements a call spreads over where every input the
/// function declares is a single number and an argument is an array.
///
/// Such a function is inlined whole only because it answers with a
/// record - `fromPolar` gives a `Complex` - and handed arrays it wants
/// the vectorization every scalar function gets, one call per element.
/// Where any input is declared an array, or takes a record, nothing
/// spreads: the body was written for the whole thing.
fn spread_of_scalar_inputs(
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
    values: &[Value],
) -> Option<usize> {
    let inputs: Vec<Component> = inlining::function_components(registry, class, 0)
        .into_iter()
        .filter(|c| c.causality == Causality::Input)
        .collect();
    let mut spread = None;
    for (input, value) in inputs.iter().zip(values) {
        let mut input = input.clone();
        resolve_type(registry, &mut input, &class.name, &class.imports);
        if !input.dimensions.is_empty()
            || record_fields::record_input_fields(registry, class, &input).is_some()
        {
            return None;
        }
        let Value::Array(items) = value else { continue };
        if spread.is_some_and(|already| already != items.len()) {
            return None;
        }
        spread = Some(items.len());
    }
    spread
}

fn takes_or_gives_an_array(class: &ClassDef, registry: &HashMap<&str, &ClassDef>) -> bool {
    // A `redeclare function extends density` writes a body and nothing
    // else: what it takes and answers with belongs to the function it
    // extends. Reading only what it wrote for itself found no
    // declarations at all, so a function taking a record was taken for
    // one taking nothing, and the call was spread over the record's
    // fields as if they were elements - a density of one number came
    // back shaped like the state of two it was asked about.
    inlining::function_components(registry, class, 0)
        .iter()
        .any(|component| {
            if component.causality == Causality::None {
                return false;
            }
            if lookup(registry, &component.type_name, &class.name, &class.imports)
                .is_some_and(|of| of.kind == ClassKind::Record)
            {
                return true;
            }
            let mut component = component.clone();
            resolve_type(registry, &mut component, &class.name, &class.imports);
            !component.dimensions.is_empty()
        })
}

/// What an expression comes to, read against the parameters and the
/// loop variables in view.
///
/// Where there are no loop variables - which is nearly always - the
/// parameters are read as they stand. Copying them to add nothing to
/// them is what a model with a thousand of them cannot afford, and
/// this is asked once per subscript, length and condition.
fn settled_by(expr: &Expr, shapes: &Shapes) -> Option<f64> {
    if shapes.loop_vars.is_empty() {
        return const_eval(expr, shapes.consts);
    }
    let mut env = shapes.consts.clone();
    env.extend(shapes.loop_vars.iter().map(|(k, v)| (k.clone(), *v)));
    const_eval(expr, &env)
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
    given: &HashMap<String, f64>,
    out: &mut HashMap<String, Vec<i64>>,
    depth: usize,
) {
    collect_shapes_under(registry, class, "", consts, given, &[], out, depth)
}

/// The same, told what the class was handed.
///
/// A flexible `:` length is read from the value a component is given,
/// and a value handed down beats the one the declaration wrote: a
/// table block declares `table[:] = {0, 1}` and is handed
/// `{2, 4, 6, 8}`, so its length is four rather than two. Measuring
/// the declaration would put a number in the table that the model has
/// already overruled, and a wrong number there is worse than none: a
/// parameter settled from it is settled for good, while a name with no
/// shape is simply asked again later.
pub(super) fn collect_shapes_given(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    consts: &HashMap<String, f64>,
    given: &HashMap<String, f64>,
    handed: &[(String, Expr)],
    out: &mut HashMap<String, Vec<i64>>,
    depth: usize,
) {
    collect_shapes_under(registry, class, "", consts, given, handed, out, depth)
}

/// The same, for the members of a record below a name.
#[allow(clippy::too_many_arguments)]
fn collect_shapes_under(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    consts: &HashMap<String, f64>,
    given: &HashMap<String, f64>,
    handed: &[(String, Expr)],
    out: &mut HashMap<String, Vec<i64>>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let scope = class.name.as_str();
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, scope, &class.imports) {
            collect_shapes_under(
                registry,
                base,
                prefix,
                consts,
                given,
                handed,
                out,
                depth + 1,
            );
        }
    }
    for component in &class.components {
        // A type may be an array of its own - `type Axis = Real[3]` -
        // and then the declaration is that shape whether or not it
        // wrote any dimensions itself.
        let mut component = component.clone();
        resolve_type(registry, &mut component, scope, &class.imports);
        let component = &component;
        // A record's members are shaped too, and a body handed one -
        // `input Orientation R` - reads `R.T` off it.
        if let Some(of) = lookup(registry, &component.type_name, scope, &class.imports)
            .filter(|of| of.kind == ClassKind::Record)
        {
            let below = format!("{prefix}{}.", component.name);
            collect_shapes_under(registry, of, &below, consts, given, &[], out, depth + 1);
        }
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
            .map(|(axis, dimension)| {
                // A length may be a constant of the class around the
                // declaration, named plainly: a random generator says
                // `output Integer state[nState]` and `nState` is a
                // constant of the package holding the function. The
                // table of constants knows it by its full name, so the
                // declaration is read the way its writer meant it
                // before it is measured.
                let _settling = super::constants::SettlingParameter::now();
                let named = super::constants::substitute_class_constants(
                    dimension,
                    registry,
                    scope,
                    &class.imports,
                    &[],
                );
                let dimension = &named;
                match dimension {
                    Expr::Ref(name) if name == "Boolean" => Some(2),
                    // A length the call decided: `output Integer[nState]
                    // state` of a function whose `nState` is an input. The
                    // caller's number is what says how long it is, and it
                    // is only ever a handful of names, so it is asked
                    // first and costs nothing.
                    _ if !given.is_empty() && const_eval(dimension, given).is_some() => {
                        const_eval(dimension, given).map(|length| length as i64)
                    }
                    Expr::Ref(name) => lookup(registry, name, scope, &class.imports)
                        .filter(|c| !c.enumeration.is_empty())
                        .map(|c| c.enumeration.len() as i64)
                        .or_else(|| dimension_value(dimension, consts, out)),
                    Expr::ColonSubscript => {
                        // What the model handed this component beats
                        // what its declaration wrote: the declaration
                        // is the default, and the default was
                        // overruled. Where the handed value cannot be
                        // measured, nothing is written rather than the
                        // losing number - a name with no shape is
                        // asked again, a wrong shape is settled for
                        // good.
                        let value = handed
                            .iter()
                            .find(|(name, _)| name == &component.name)
                            .map(|(_, given)| given)
                            .or(component.binding.as_ref());
                        value.and_then(|binding| {
                            // A table told where to find its numbers
                            // knows how wide it is only once the file
                            // is read, and the file is named among
                            // the same values that were handed down.
                            let handed_text = |wanted: &str| {
                                handed.iter().find_map(|(name, given)| match given {
                                    Expr::Str(text) if name == wanted => Some(text.clone()),
                                    _ => None,
                                })
                            };
                            let handed_truth = |wanted: &str| {
                                handed.iter().find_map(|(name, given)| match given {
                                    Expr::Bool(yes) if name == wanted => Some(*yes),
                                    _ => None,
                                })
                            };
                            if let Some(length) = super::extents::size_of_a_table_in_a_file(
                                binding,
                                axis,
                                handed_text,
                                handed_truth,
                            ) {
                                return Some(length);
                            }
                            // A value written out says its length outright.
                            if let Some(length) =
                                flexible_size(binding, axis, registry, scope, &class.imports)
                            {
                                return Some(length);
                            }
                            // A range says it by its bounds: the table
                            // blocks write `columns[:] = 2:size(table, 2)`,
                            // and how many columns that is depends on the
                            // table. The bounds are read against the
                            // numbers in view and the arrays measured so
                            // far, both of which are here.
                            range_length(binding, axis, consts, out)
                        })
                    }
                    _ => dimension_value(dimension, consts, out),
                }
            })
            .collect();
        if let Some(sizes) = sizes {
            out.insert(format!("{prefix}{}", component.name), sizes);
        }
    }
}

/// How long a range comes to, for a flexible `:` size read from one.
///
/// `2:size(table, 2)` is how the table blocks say which columns they
/// take, and the length is the count of steps from one bound to the
/// other. The bounds may themselves ask after an array already
/// measured, so both tables are read. Nothing that cannot be settled
/// here is guessed at: an unmeasurable range comes back unmeasured.
pub(super) fn range_length(
    binding: &Expr,
    axis: usize,
    consts: &HashMap<String, f64>,
    sizes: &HashMap<String, Vec<i64>>,
) -> Option<i64> {
    if axis != 0 {
        return None;
    }
    let Expr::Range(from, step, to) = binding else {
        return None;
    };
    let settle = |e: &Expr| {
        const_eval(e, consts).or_else(|| dimension_value(e, consts, sizes).map(|n| n as f64))
    };
    let (from, to) = (settle(from)?, settle(to)?);
    let step = match step {
        None => 1.0,
        Some(step) => settle(step)?,
    };
    if step == 0.0 {
        return None;
    }
    let count = ((to - from) / step).floor() + 1.0;
    (count.is_finite() && count >= 0.0).then_some(count as i64)
}

/// An array of the given shape, every element the same.
fn nested(lengths: &[i64], value: &Expr) -> Value {
    nested_value(lengths, &Value::Scalar(value.clone()))
}

/// The same, where what is repeated is a whole value: `fill(Complex(0),
/// m)` fills with a record, and a record is its fields.
fn nested_value(lengths: &[i64], value: &Value) -> Value {
    match lengths.split_first() {
        None => value.clone(),
        Some((&length, rest)) => {
            Value::Array((0..length).map(|_| nested_value(rest, value)).collect())
        }
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
    // And `size(a, 1) - 1` is a length too: a transfer function is
    // one state shorter than its denominator has coefficients, and
    // the library writes exactly that. The measured sizes are not in
    // the constants, so a `size` standing anywhere inside an
    // expression is answered first and the arithmetic done after.
    let answered = measured_sizes(expr, consts, sizes);
    let value = const_eval(&answered, consts)?;
    (value.fract() == 0.0 && value >= 0.0).then_some(value as i64)
}

/// Every `size(...)` of an already measured name, replaced by what it
/// measures.
fn measured_sizes(
    expr: &Expr,
    consts: &HashMap<String, f64>,
    sizes: &HashMap<String, Vec<i64>>,
) -> Expr {
    if let Expr::Call(name, args) = expr {
        if name == "size" && !args.is_empty() {
            if let Expr::Ref(of) = &args[0] {
                let held = sizes.get(of).and_then(|shape| {
                    let index = match args.get(1) {
                        None => Some(0),
                        Some(dimension) => {
                            const_eval(dimension, consts).map(|held| held as usize - 1)
                        }
                    }?;
                    shape.get(index).copied()
                });
                if let Some(length) = held {
                    return Expr::Number(length as f64);
                }
            }
        }
    }
    expr.map_children(&mut |held| measured_sizes(held, consts, sizes))
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
        // Two sides that hold no values at all are the same equation
        // however their shapes are written. A medium with no mass
        // fractions gives every port `Xi_outflow[0]`, and the model
        // says `ports[i].Xi_outflow = medium.Xi` about it: an array of
        // nothing against a name that was never given a shape, and
        // A medium with no mass fractions gives every port
        // `Xi_outflow[0]`, and the model says `ports[i].Xi_outflow =
        // medium.Xi` about it: an array of nothing against a name that
        // never got a shape of its own. The two are written
        // differently and hold the same nothing between them, and
        // refusing them refused the whole of the Fluid library for
        // saying something empty twice.
        //
        // What settles it is how many values each side really holds
        // along the axes it shares. A shape with a nought in it holds
        // none whatever else it says, and a name with no shape stands
        // for one place that nothing fills. Where both come to nothing
        // there is no equation to write and nothing to refuse.
        // A shape with a nought in it holds nothing whatever else it
        // says; the side written as names got one place per name, and
        // those places are the same nothing. What has to agree is the
        // lengths they share - four ports each carrying nothing is
        // nothing, four against three is a model saying two different
        // things.
        let empty_shape = |shape: &[usize]| shape.contains(&0);
        // The dimensions before the empty one are what has to agree:
        // a run of two states each carrying nothing is `[1, 2]`
        // against `[1, 0]`, and comparing the last pair asks two to
        // equal nothing. What the library writes is `statesFM =
        // fill(Medium.setState_phX(...), 0)` where the medium has no
        // trace substances - the outer run is real, the inner is the
        // nothing both sides agree on.
        let outer_of = |shape: &[usize]| -> Vec<usize> {
            shape
                .iter()
                .take_while(|length| **length != 0)
                .copied()
                .collect()
        };
        let (left_outer, right_outer) = (outer_of(&left_shape), outer_of(&right_shape));
        let shared = left_outer.len().min(right_outer.len());
        let same_outer = left_outer[..shared] == right_outer[..shared];
        if (empty_shape(&left_shape) || empty_shape(&right_shape)) && same_outer {
            return Ok(());
        }
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
        // A subscript the run settles reads its element by asking
        // which place the index names, and what that builds is a
        // choice among values. A choice can be read but not assigned:
        // standing on the left it names no variable, and the equation
        // it makes says nothing a solver can hold to. Refused here
        // rather than left to come out as a number no one asked for.
        if matches!(lhs, Expr::If(..)) {
            return Err(format!(
                "the left of an equation must name a variable, and a subscript \
                 the run settles gives a choice among them instead: {}",
                crate::flatten::names::sketch(&lhs)
            ));
        }
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

/// Whether a subscript list holds a range that picks nothing.
///
/// `1:0` is how the standard library writes "none of them" when a
/// count is zero, and a slice by it is the empty array whatever the
/// name it is written on holds.
fn empty_range_subscript(subscripts: &[Expr], shapes: &Shapes) -> bool {
    subscripts.iter().any(|subscript| {
        let Expr::Range(from, step, to) = subscript else {
            return false;
        };
        // A stepped range is left alone: the count is the same
        // arithmetic, but nothing in the library writes an empty one
        // with a step, and guessing wrong here would swallow a slice
        // that means something.
        if step.is_some() {
            return false;
        }
        let (Some(from), Some(to)) = (settled_by(from, shapes), settled_by(to, shapes)) else {
            return false;
        };
        to < from
    })
}

/// A copy of a function that was handed another function, with the
/// function input gone.
///
/// The copy takes ordinary numbers where the target's filled-in
/// inputs were, and every call to the vanished input is rewritten
/// into a direct call of the target: the free arguments where the
/// body wrote them, the filled ones from the new inputs. What comes
/// back is the copy and the arguments the outer call should now be
/// made with - the partial argument dropped, the filled-in
/// expressions appended in the order the copy declares them.
///
/// The new inputs are named after the input they replace - `f.A` for
/// what `f`'s target called `A` - because a body of any size has
/// locals of its own, and Brent's method has an `s` exactly where a
/// caller writes `s=-y_zero`. A frame is a map of strings, so a dot
/// in the name is nothing to it and everything to the collision.
fn specialized(
    class: &ClassDef,
    args: &[Expr],
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
) -> Result<(ClassDef, Vec<Expr>), String> {
    // Which argument is the function, and which input it lands on.
    let at = args
        .iter()
        .position(|arg| matches!(arg, Expr::Call(head, _) if head == PARTIAL_CALL))
        .ok_or("no function was handed over")?;
    let Expr::Call(_, partial) = &args[at] else {
        return Err("no function was handed over".to_string());
    };
    let (target, filled) = partial
        .split_first()
        .ok_or("a handed-over function with no name")?;
    let Expr::Ref(target) = target else {
        return Err(format!(
            "a handed-over function has to be named, found {target:?}"
        ));
    };
    // The target is resolved where the call was written: the model
    // wrote that name in its own scope, and the receiving function
    // never heard of it.
    let target = lookup(registry, target, scope, imports)
        .ok_or_else(|| format!("`{target}` is handed over and is not a function here"))?;
    let inputs: Vec<&Component> = class
        .components
        .iter()
        .filter(|held| held.causality == Causality::Input)
        .collect();
    let replaced = inputs
        .get(at)
        .ok_or_else(|| format!("`{}` takes no argument in that place", class.name))?
        .name
        .clone();
    // What the target takes, and which of those the call filled in.
    let said = |wanted: &str| -> Option<Expr> {
        filled.iter().find_map(|arg| match arg {
            Expr::NamedArg(name, value) if name == wanted => Some((**value).clone()),
            _ => None,
        })
    };
    // The target's own inputs, in order. The first is the free one -
    // what the receiver calls the function with - and the rest are
    // either filled in at the call or left at their defaults.
    // The target's inputs, its bases included: the standard library
    // writes `extends partialScalarFunction` and inherits the very
    // input the receiver will call it with.
    let target_held = super::inlining::with_inherited_components(target, registry);
    let target_inputs: Vec<&Component> = target_held
        .iter()
        .filter(|held| held.causality == Causality::Input)
        .collect();
    let (free, bound) = target_inputs
        .split_first()
        .ok_or_else(|| format!("`{}` takes nothing to be solved for", target.name))?;
    let mut copy = class.clone();
    let mut extra: Vec<Component> = Vec::new();
    let mut appended: Vec<Expr> = Vec::new();
    for held in bound {
        let Some(value) = said(&held.name) else {
            continue;
        };
        let mut carried = (*held).clone();
        carried.name = format!("{replaced}.{}", held.name);
        carried.binding = None;
        extra.push(carried);
        appended.push(value);
    }
    // The call the body writes as `f(x)` becomes the target called
    // with everything, positionally, in the order it declares them.
    let rewritten = |written: &[Expr]| -> Expr {
        let mut all: Vec<Expr> = written.to_vec();
        all.truncate(1);
        if all.is_empty() {
            all.push(Expr::Ref(free.name.clone()));
        }
        for held in bound {
            all.push(match said(&held.name) {
                Some(_) => Expr::Ref(format!("{replaced}.{}", held.name)),
                None => held
                    .binding
                    .clone()
                    .or_else(|| held.start.clone())
                    .unwrap_or(Expr::Number(0.0)),
            });
        }
        Expr::Call(target.name.clone(), all)
    };
    copy.components.retain(|held| held.name != replaced);
    copy.components.extend(extra);
    copy.algorithm = calls_rewritten(&class.algorithm, &replaced, &rewritten);
    // A name of its own, worked out from what went into it, so the
    // same pair is specialized once however many models ask for it.
    copy.name = format!("{}${}", class.name, target.name.replace('.', "_"));
    let rest: Vec<Expr> = args
        .iter()
        .enumerate()
        .filter(|(which, _)| *which != at)
        .map(|(_, arg)| arg.clone())
        .chain(appended)
        .collect();
    Ok((copy, rest))
}

/// Every call of one name in a body, rewritten.
fn calls_rewritten(
    body: &[Statement],
    named: &str,
    into: &impl Fn(&[Expr]) -> Expr,
) -> Vec<Statement> {
    let expr = |e: &Expr| call_rewritten(e, named, into);
    let inner = |body: &[Statement]| calls_rewritten(body, named, into);
    let rebranch = |branches: &[StatementBranch]| -> Vec<StatementBranch> {
        branches
            .iter()
            .map(|branch| StatementBranch {
                condition: branch.condition.as_ref().map(&expr),
                body: inner(&branch.body),
            })
            .collect()
    };
    body.iter()
        .map(|statement| match statement {
            Statement::Assign(name, subscripts, value) => Statement::Assign(
                name.clone(),
                subscripts.iter().map(&expr).collect(),
                expr(value),
            ),
            Statement::TupleAssign(targets, value) => {
                Statement::TupleAssign(targets.clone(), expr(value))
            }
            Statement::Assert(condition, message) => {
                Statement::Assert(expr(condition), message.clone())
            }
            Statement::Call(name, args) => {
                Statement::Call(name.clone(), args.iter().map(&expr).collect())
            }
            Statement::If(branches) => Statement::If(rebranch(branches)),
            Statement::When(branches) => Statement::When(rebranch(branches)),
            Statement::For(variable, range, body) => {
                Statement::For(variable.clone(), range.as_ref().map(&expr), inner(body))
            }
            Statement::While(condition, body) => Statement::While(expr(condition), inner(body)),
            Statement::Break => Statement::Break,
            Statement::Return => Statement::Return,
        })
        .collect()
}

/// The same rewrite inside one expression.
fn call_rewritten(expr: &Expr, named: &str, into: &impl Fn(&[Expr]) -> Expr) -> Expr {
    if let Expr::Call(head, args) = expr {
        if head == named {
            return into(args);
        }
    }
    expr.map_children(&mut |held| call_rewritten(held, named, into))
}
