//! Shapes: how long each array of a class is, measured before
//! anything is expanded.
//!
//! The expansion in `arrays.rs` reads these tables and never builds
//! them; they are gathered here, from declarations and from what a
//! caller handed in.

use super::*;

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
                            if let Some(length) = range_length(binding, axis, consts, out) {
                                return Some(length);
                            }
                            // A value that is neither written out nor a
                            // range still has a length once it is built:
                            // a table block is handed `table = {{...}
                            // for j in 1:size(Ptable, 1)}`, a
                            // comprehension whose count nothing here
                            // reads outright. Building it says how long
                            // it is, and the length is what the
                            // declarations after it are written against
                            // - `columns[:] = 2:size(table, 2)`, and
                            // `nout = size(columns, 1)` behind that.
                            //
                            // Asked last, after every cheaper reading
                            // has failed, because building a value is
                            // dearer than measuring one. On failure
                            // nothing is written: a wrong number here is
                            // settled for good, where a missing one is
                            // asked again later.
                            let no_loop_vars = HashMap::new();
                            let shapes = Shapes {
                                sizes: out,
                                loop_vars: &no_loop_vars,
                                consts,
                                records: super::arrays::no_records(),
                            };
                            let mark = super::algorithms::checks_mark();
                            let built = super::arrays::expand(
                                binding,
                                &shapes,
                                registry,
                                scope,
                                &class.imports,
                                0,
                            );
                            super::algorithms::checks_rewind(mark);
                            built.ok()?.shape().get(axis).map(|length| *length as i64)
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
pub(super) fn nested(lengths: &[i64], value: &Expr) -> Value {
    nested_value(lengths, &Value::Scalar(value.clone()))
}

/// The same, where what is repeated is a whole value: `fill(Complex(0),
/// m)` fills with a record, and a record is its fields.
pub(super) fn nested_value(lengths: &[i64], value: &Value) -> Value {
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

/// What an expression comes to where the lengths of things are part
/// of the answer: `size(u, 1) > 0` is a condition an `if` equation
/// may be written on, and by the time equations are read the shapes
/// are settled.
///
/// Arithmetic and comparison over lengths and numbers, and nothing
/// else - a condition that reads a variable is one the run decides,
/// and this must not pretend otherwise.
pub(super) fn settled_by_shape(
    expr: &Expr,
    consts: &HashMap<String, f64>,
    sizes: &HashMap<String, Vec<i64>>,
) -> Option<f64> {
    if let Some(length) = dimension_value(expr, consts, sizes) {
        return Some(length as f64);
    }
    let recur = |inner: &Expr| settled_by_shape(inner, consts, sizes);
    match expr {
        Expr::Number(value) => Some(*value),
        Expr::Bool(value) => Some(*value as i64 as f64),
        Expr::Neg(inner) => recur(inner).map(|value| -value),
        Expr::Bin(op, a, b) => {
            let (a, b) = (recur(a)?, recur(b)?);
            Some(match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div => a / b,
                BinOp::Pow => a.powf(b),
            })
        }
        Expr::Rel(op, a, b) => {
            let (a, b) = (recur(a)?, recur(b)?);
            let held = match op {
                RelOp::Lt => a < b,
                RelOp::Le => a <= b,
                RelOp::Gt => a > b,
                RelOp::Ge => a >= b,
                RelOp::Eq => a == b,
                RelOp::Ne => a != b,
            };
            Some(held as i64 as f64)
        }
        Expr::And(a, b) => Some(((recur(a)? != 0.0) && (recur(b)? != 0.0)) as i64 as f64),
        Expr::Or(a, b) => Some(((recur(a)? != 0.0) || (recur(b)? != 0.0)) as i64 as f64),
        Expr::Not(inner) => Some((recur(inner)? == 0.0) as i64 as f64),
        // An `if` inside a condition: settled where all three parts
        // are, which the exhaustiveness check is what found - it was
        // being dropped silently before.
        Expr::If(condition, then, otherwise) => {
            if recur(condition)? != 0.0 {
                recur(then)
            } else {
                recur(otherwise)
            }
        }
        // A name is a value the run carries, not a length: whatever
        // `dimension_value` could tell about one it has told already.
        Expr::Ref(_) | Expr::Member(..) | Expr::Index(..) => None,
        // A call that is not a question about shape is a question for
        // the run - `size` and `ndims` were answered above.
        Expr::Call(..) => None,
        // Arrays, ranges and the rest are not numbers, and a condition
        // an `if` equation stands on has to come to one.
        Expr::Array(_)
        | Expr::MatrixRows(_)
        | Expr::Range(..)
        | Expr::Comprehension(..)
        | Expr::Tuple(_)
        | Expr::NamedArg(..)
        | Expr::Str(_)
        | Expr::Elementwise(..)
        | Expr::WithDerivative(..) => None,
        // Time moves, and a subscript standing alone is part of an
        // index rather than a value.
        Expr::Time | Expr::ColonSubscript | Expr::EndSubscript => None,
    }
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
