//! Symbolic work on expressions: folding, substitution, solving a
//! linear equation for one unknown, and differentiating by time.

use crate::*;

/// Constant folding and algebraic identities.
///
/// Symbolic derivatives are built structurally and carry a lot of dead
/// weight (`y * 0`, `x + 0`, `1 * u`). Folding it away matters twice
/// over: linearity detection asks whether a derivative still mentions
/// its variable, and differentiated constraints are evaluated at every
/// step.
pub(crate) fn simplify(expr: &Expr) -> Expr {
    use oxidelica_parser::BinOp::*;
    match expr {
        Expr::Neg(inner) => match simplify(inner) {
            Expr::Number(n) => Expr::Number(-n),
            other => Expr::Neg(Box::new(other)),
        },
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(simplify).collect()),
        Expr::Bin(op, l, r) => {
            let (l, r) = (simplify(l), simplify(r));
            if let (Expr::Number(a), Expr::Number(b)) = (&l, &r) {
                return Expr::Number(match op {
                    Add => a + b,
                    Sub => a - b,
                    Mul => a * b,
                    Div => a / b,
                    Pow => a.powf(*b),
                });
            }
            let is = |e: &Expr, v: f64| matches!(e, Expr::Number(n) if *n == v);
            match op {
                Add if is(&l, 0.0) => r,
                Add if is(&r, 0.0) => l,
                Sub if is(&r, 0.0) => l,
                Sub if is(&l, 0.0) => Expr::Neg(Box::new(r)),
                Mul if is(&l, 0.0) || is(&r, 0.0) => Expr::Number(0.0),
                Mul if is(&l, 1.0) => r,
                Mul if is(&r, 1.0) => l,
                Div if is(&l, 0.0) => Expr::Number(0.0),
                Div if is(&r, 1.0) => l,
                Pow if is(&r, 1.0) => l,
                Pow if is(&r, 0.0) => Expr::Number(1.0),
                _ => Expr::Bin(*op, Box::new(l), Box::new(r)),
            }
        }
        Expr::If(c, a, b) => Expr::If(
            Box::new(simplify(c)),
            Box::new(simplify(a)),
            Box::new(simplify(b)),
        ),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(simplify(l)), Box::new(simplify(r))),
        Expr::And(l, r) => Expr::And(Box::new(simplify(l)), Box::new(simplify(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(simplify(l)), Box::new(simplify(r))),
        Expr::Not(inner) => Expr::Not(Box::new(simplify(inner))),
        // Subscripts are resolved to scalar references while
        // flattening, so none can reach the compiler.
        Expr::Index(_, _)
        | Expr::Member(_, _)
        | Expr::Number(_)
        | Expr::Bool(_)
        | Expr::Ref(_)
        | Expr::Time => expr.clone(),
        // Arrays never reach here: flattening expands them.
        Expr::Array(_)
        | Expr::Elementwise(_, _, _)
        | Expr::Range(_, _, _)
        | Expr::Comprehension(_, _, _)
        | Expr::ColonSubscript
        | Expr::EndSubscript
        | Expr::MatrixRows(_)
        | Expr::NamedArg(_, _)
        | Expr::Tuple(_) => expr.clone(),
    }
}

/// Replace every reference to `var` with `value`.
pub(crate) fn substitute(expr: &Expr, var: &str, value: f64) -> Expr {
    match expr {
        Expr::Ref(name) if name == var => Expr::Number(value),
        Expr::Ref(_) | Expr::Number(_) | Expr::Bool(_) | Expr::Time => expr.clone(),
        Expr::Call(name, args) => Expr::Call(
            name.clone(),
            args.iter().map(|a| substitute(a, var, value)).collect(),
        ),
        Expr::Neg(inner) => Expr::Neg(Box::new(substitute(inner, var, value))),
        Expr::Not(inner) => Expr::Not(Box::new(substitute(inner, var, value))),
        Expr::Bin(op, l, r) => Expr::Bin(
            *op,
            Box::new(substitute(l, var, value)),
            Box::new(substitute(r, var, value)),
        ),
        Expr::Rel(op, l, r) => Expr::Rel(
            *op,
            Box::new(substitute(l, var, value)),
            Box::new(substitute(r, var, value)),
        ),
        Expr::And(l, r) => Expr::And(
            Box::new(substitute(l, var, value)),
            Box::new(substitute(r, var, value)),
        ),
        Expr::Or(l, r) => Expr::Or(
            Box::new(substitute(l, var, value)),
            Box::new(substitute(r, var, value)),
        ),
        Expr::If(c, a, b) => Expr::If(
            Box::new(substitute(c, var, value)),
            Box::new(substitute(a, var, value)),
            Box::new(substitute(b, var, value)),
        ),
        Expr::Index(_, _)
        | Expr::Member(_, _)
        | Expr::Array(_)
        | Expr::Elementwise(_, _, _)
        | Expr::Range(_, _, _)
        | Expr::Comprehension(_, _, _)
        | Expr::ColonSubscript
        | Expr::EndSubscript
        | Expr::MatrixRows(_)
        | Expr::NamedArg(_, _)
        | Expr::Tuple(_) => expr.clone(),
    }
}

/// Solve `lhs = rhs` symbolically for `var` when the equation is linear
/// in it: with residual `r = a*var + b`, the solution is `-b/a`, where
/// `a` is the (var-free) derivative and `b` is `r` at `var = 0`.
pub(crate) fn solve_linear_for(lhs: &Expr, rhs: &Expr, var: &str) -> Option<Expr> {
    let residual = Expr::Bin(
        oxidelica_parser::BinOp::Sub,
        Box::new(lhs.clone()),
        Box::new(rhs.clone()),
    );
    let slope = simplify(&differentiate(&residual, &DiffTarget::Variable(var)).ok()?);
    let mut refs = Vec::new();
    slope.collect_refs(&mut refs);
    if refs.contains(&var) {
        return None;
    }
    let intercept = simplify(&substitute(&residual, var, 0.0));
    Some(simplify(&Expr::Bin(
        oxidelica_parser::BinOp::Div,
        Box::new(Expr::Neg(Box::new(intercept))),
        Box::new(slope),
    )))
}

pub(crate) fn differentiate(expr: &Expr, target: &DiffTarget) -> Result<Expr, String> {
    differentiate_at(expr, target, 0)
}

pub(crate) fn differentiate_at(
    expr: &Expr,
    target: &DiffTarget,
    depth: usize,
) -> Result<Expr, String> {
    if depth > MAX_DIFF_DEPTH {
        return Err("differentiation recursed through a cyclic definition".to_string());
    }
    use oxidelica_parser::BinOp::*;
    fn bin(op: oxidelica_parser::BinOp, a: Expr, b: Expr) -> Expr {
        Expr::Bin(op, Box::new(a), Box::new(b))
    }
    fn call(name: &str, arg: Expr) -> Expr {
        Expr::Call(name.to_string(), vec![arg])
    }
    let d = |e: &Expr| differentiate_at(e, target, depth + 1);
    Ok(match expr {
        Expr::Number(_) | Expr::Bool(_) => Expr::Number(0.0),
        Expr::Time => match target {
            DiffTarget::Time { .. } => Expr::Number(1.0),
            DiffTarget::Variable(_) => Expr::Number(0.0),
        },
        Expr::Ref(name) => match target {
            DiffTarget::Time {
                state_rhs,
                params,
                dummies,
                alg_defs,
            } => {
                if let Some(rhs) = state_rhs.get(name) {
                    rhs.clone()
                } else if params.contains_key(name) {
                    Expr::Number(0.0)
                } else if let Some(dummy) = dummies.get(name) {
                    // A demoted state: its derivative is the dummy.
                    Expr::Ref(dummy.clone())
                } else if let Some(definition) = alg_defs.get(name) {
                    // An algebraic unknown with an explicit definition:
                    // differentiate the definition instead (Pantelides
                    // reaches the derivative through the equation that
                    // determines the variable).
                    d(definition)?
                } else {
                    return Err(format!(
                        "cannot differentiate through algebraic variable `{name}`"
                    ));
                }
            }
            DiffTarget::Variable(var) => {
                if name == var {
                    Expr::Number(1.0)
                } else {
                    Expr::Number(0.0)
                }
            }
        },
        Expr::Neg(inner) => Expr::Neg(Box::new(d(inner)?)),
        Expr::Bin(Add, a, b) => bin(Add, d(a)?, d(b)?),
        Expr::Bin(Sub, a, b) => bin(Sub, d(a)?, d(b)?),
        Expr::Bin(Mul, a, b) => bin(
            Add,
            bin(Mul, d(a)?, (**b).clone()),
            bin(Mul, (**a).clone(), d(b)?),
        ),
        Expr::Bin(Div, a, b) => bin(
            Div,
            bin(
                Sub,
                bin(Mul, d(a)?, (**b).clone()),
                bin(Mul, (**a).clone(), d(b)?),
            ),
            bin(Pow, (**b).clone(), Expr::Number(2.0)),
        ),
        Expr::Bin(Pow, base, exponent) => {
            let Expr::Number(c) = **exponent else {
                return Err("cannot differentiate a non-constant exponent".to_string());
            };
            bin(
                Mul,
                bin(
                    Mul,
                    Expr::Number(c),
                    bin(Pow, (**base).clone(), Expr::Number(c - 1.0)),
                ),
                d(base)?,
            )
        }
        Expr::Call(name, args) if args.len() == 1 => {
            // The staircase functions are flat almost everywhere.
            if matches!(name.as_str(), "ceil" | "floor" | "integer" | "sign") {
                return Ok(Expr::Number(0.0));
            }
            let u = &args[0];
            let du = d(u)?;
            let outer = match name.as_str() {
                "sin" => call("cos", u.clone()),
                "cos" => Expr::Neg(Box::new(call("sin", u.clone()))),
                "tan" => bin(
                    Div,
                    Expr::Number(1.0),
                    bin(Pow, call("cos", u.clone()), Expr::Number(2.0)),
                ),
                "exp" => call("exp", u.clone()),
                "log" => bin(Div, Expr::Number(1.0), u.clone()),
                "sqrt" => bin(Div, Expr::Number(0.5), call("sqrt", u.clone())),
                "atan" => bin(
                    Div,
                    Expr::Number(1.0),
                    bin(
                        Add,
                        Expr::Number(1.0),
                        bin(Pow, u.clone(), Expr::Number(2.0)),
                    ),
                ),
                "sinh" => call("cosh", u.clone()),
                "cosh" => call("sinh", u.clone()),
                "tanh" => bin(
                    Div,
                    Expr::Number(1.0),
                    bin(Pow, call("cosh", u.clone()), Expr::Number(2.0)),
                ),
                other => return Err(format!("cannot differentiate function `{other}`")),
            };
            bin(Mul, outer, du)
        }
        Expr::If(cond, then_branch, else_branch) => Expr::If(
            cond.clone(),
            Box::new(d(then_branch)?),
            Box::new(d(else_branch)?),
        ),
        _ => return Err("cannot differentiate this expression".to_string()),
    })
}
