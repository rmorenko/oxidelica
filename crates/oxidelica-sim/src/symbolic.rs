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
        Expr::WithDerivative(value, rule, seeds) => Expr::WithDerivative(
            Box::new(simplify(value)),
            Box::new(simplify(rule)),
            seeds
                .iter()
                .map(|(name, argument)| (name.clone(), simplify(argument)))
                .collect(),
        ),
        // Nothing to fold: a string is already as simple as it gets.
        Expr::Str(_) => expr.clone(),
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

/// Put each argument's derivative where a rule left a name for it.
///
/// The search stops at any call that carries a rule of its own: that
/// rule speaks about its own arguments, and the names two functions
/// happened to give their parameters mean nothing to each other.
fn seeded(rule: &Expr, given: &HashMap<String, Expr>) -> Expr {
    let recur = |inner: &Expr| seeded(inner, given);
    match rule {
        Expr::WithDerivative(..) => rule.clone(),
        Expr::Ref(name) => given.get(name).cloned().unwrap_or_else(|| rule.clone()),
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::If(c, a, b) => Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b))),
        _ => rule.clone(),
    }
}

/// Replace every reference to `var` with `value`.
pub(crate) fn substitute(expr: &Expr, var: &str, value: f64) -> Expr {
    match expr {
        // The one case this is about; everything else is the same
        // expression with its children substituted.
        Expr::Ref(name) if name == var => Expr::Number(value),
        _ => expr.map_children(&mut |child| substitute(child, var, value)),
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
        return Err("an expression too deep to differentiate".to_string());
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
            // The library writes `Modelica.Math.sin`, and what arrives
            // here is what the name resolved to - which for a function
            // the compiler answers itself is the last part of it, with
            // whatever package path it was written under left in
            // front. Matched whole, `.sin` was a function with no
            // derivative rule, and every model with a sine source was
            // refused as structurally singular.
            let name = name.rsplit('.').next().unwrap_or(name);
            // The staircase functions are flat almost everywhere.
            if matches!(name, "ceil" | "floor" | "integer" | "sign") {
                return Ok(Expr::Number(0.0));
            }
            let u = &args[0];
            let du = d(u)?;
            let outer = match name {
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
        // `mod(a, b)` is `a - floor(a / b) * b` and is differentiated
        // as that: the staircase is flat wherever it is defined, so
        // what is left is `da - floor(a / b) * db`. A table asked to
        // repeat wraps its abscissa this way and the period is a
        // number, so `db` is nothing and the derivative comes out as
        // the derivative of the abscissa - which is what the table
        // would have been differentiated to had it never been wrapped.
        //
        // At the instant the wrap happens there is no derivative at
        // all. Nothing is claimed about it: the conditions of the
        // chain the table is written as are event indicators, and the
        // solver stops at each of them rather than integrating across.
        Expr::Call(name, args) if name == "mod" && args.len() == 2 => {
            let (a, b) = (&args[0], &args[1]);
            let steps = call("floor", bin(Div, a.clone(), b.clone()));
            bin(Sub, d(a)?, bin(Mul, steps, d(b)?))
        }
        // `rem(a, b)` is the same with the staircase rounded towards
        // nothing rather than downwards, and differentiates alike.
        Expr::Call(name, args) if name == "rem" && args.len() == 2 => {
            let (a, b) = (&args[0], &args[1]);
            let steps = call("integer", bin(Div, a.clone(), b.clone()));
            bin(Sub, d(a)?, bin(Mul, steps, d(b)?))
        }
        Expr::If(cond, then_branch, else_branch) => Expr::If(
            cond.clone(),
            Box::new(d(then_branch)?),
            Box::new(d(else_branch)?),
        ),
        // A call that said how to differentiate itself: the rule takes
        // the place of taking the body apart, with each argument's own
        // derivative put where the rule left a name for it. The chain
        // rule is already in the rule - that is what the annotation
        // means - so there is nothing to multiply by here.
        Expr::WithDerivative(_, rule, seeds) => {
            let mut given = HashMap::new();
            for (name, argument) in seeds {
                given.insert(name.clone(), differentiate_at(argument, target, depth + 1)?);
            }
            return Ok(seeded(rule, &given));
        }
        _ => return Err("cannot differentiate this expression".to_string()),
    })
}
