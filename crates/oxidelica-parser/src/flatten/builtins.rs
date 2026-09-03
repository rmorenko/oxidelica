//! The array-shaped built-ins: `transpose`, `identity`, `diagonal`,
//! `cross`, `cat`, `linspace` and the folds over an array, worked out
//! where the shape is known.
//!
//! Moved out of `arrays.rs` whole, where they had been living inside
//! the expansion that calls them.

use super::arrays::*;
use super::*;

/// The builtins that build a shape or turn one about: `transpose`,
/// `identity`, `diagonal`, `cross`, `outerProduct`, `symmetric`,
/// `skew`, `cat` and `linspace`.
///
/// Moved out of `expand_call` unchanged.
#[allow(clippy::too_many_arguments)]
pub(super) fn shaped_by_a_builtin(
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
pub(super) fn folded_over_an_array(
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
