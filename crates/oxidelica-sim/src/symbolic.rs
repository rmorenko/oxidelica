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
            // Two of them cancel. A connection set builds its signs by
            // moving terms across, so `-(-i)` is the ordinary shape of
            // a current that was on the other side, and left standing
            // it hides a plain name from everything downstream.
            Expr::Neg(twice) => *twice,
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
                // Minus one is a sign, not a coefficient: solving a
                // connection equation for one of its currents divides
                // by the sign it was written with, and `x/-1` left as
                // a division is a multiplication nobody meant.
                Mul if is(&l, -1.0) => simplify(&Expr::Neg(Box::new(r))),
                Mul if is(&r, -1.0) => simplify(&Expr::Neg(Box::new(l))),
                Div if is(&l, 0.0) => Expr::Number(0.0),
                Div if is(&r, 1.0) => l,
                Div if is(&r, -1.0) => simplify(&Expr::Neg(Box::new(l))),
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
    solve_linear_known(lhs, rhs, var, &HashMap::new())
}

/// The same, with the parameter values in view.
///
/// The slope decides whether an equation may be divided through, and a
/// slope written `alpha*R` is a number the moment `alpha` is known. A
/// zero there is not a small coefficient: it is the equation declining
/// to mention its unknown, and `Resistor` with `alpha = 0` writes
/// exactly that. Judged without the parameters the slope looks like a
/// live coefficient, the division goes through, and the run reports an
/// infinite residual against the solver rather than a refusal against
/// the model.
pub(crate) fn solve_linear_known(
    lhs: &Expr,
    rhs: &Expr,
    var: &str,
    known: &HashMap<String, f64>,
) -> Option<Expr> {
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
    // A slope of zero is not a small coefficient, it is the equation
    // saying it does not mention its unknown at all: `R = 100*(1 +
    // alpha*(T - T_ref))` with `alpha = 0` constrains `R` and says
    // nothing whatever about `T`. Divided through anyway it yields an
    // infinity, and what the run then reports is a residual that could
    // not be evaluated - which names the solver, the one place nothing
    // is wrong. Refusing here leaves the equation for the tearing set,
    // where an honest singular Jacobian can be raised about the thing
    // that is actually undetermined.
    //
    // Judged with the parameters folded in, but kept out of the answer:
    // what is being asked is whether the coefficient is zero, and the
    // expression handed back stays in the model's own names so the
    // arithmetic is the same as it always was.
    //
    // The names walked are the slope's own, not the table's. A model
    // carries thousands of parameters and a slope mentions two, so the
    // cheap direction is to ask the slope what it needs; the other way
    // round pays the whole table on every equation.
    let judged = {
        let mut folded = slope.clone();
        for name in &refs {
            if let Some(value) = known.get(*name) {
                folded = substitute(&folded, name, *value);
            }
        }
        simplify(&folded)
    };
    if matches!(judged, Expr::Number(x) if x == 0.0) {
        return None;
    }
    // The same refusal for a coefficient the model itself writes as
    // sometimes zero. `semiLinear(m, h_a, h_b)` is `if m >= 0 then
    // h_a*m else h_b*m`, so an equation for the upstream enthalpy has
    // the slope `if m >= 0 then m else 0`: on the branch the flow runs
    // the other way the equation does not mention that enthalpy at
    // all. Judged by one branch the slope looks live, the division
    // goes through, and the first residual of the block is infinite
    // before Newton has taken a step. Left alone the equation joins
    // the tearing set, where the connection equality that does
    // determine the enthalpy is the residual that gets solved.
    if branch_is_zero(&judged) && std::env::var_os("OXIDELICA_NO_BRANCHED_SLOPE").is_none() {
        return None;
    }
    let intercept = simplify(&substitute(&residual, var, 0.0));
    Some(simplify(&Expr::Bin(
        oxidelica_parser::BinOp::Div,
        Box::new(Expr::Neg(Box::new(intercept))),
        Box::new(slope),
    )))
}

/// Whether a slope written as a conditional takes the value zero on
/// one of its branches.
///
/// A plan is made once and holds for the whole run, so a coefficient
/// that is zero on a branch the model will enter is a division that
/// will produce an infinity at some point during it - which the run
/// reports against the solver rather than against the equation. Only
/// a branch that is *outright* zero counts: a branch whose value is
/// some other expression may well be zero at a moment, but so may any
/// coefficient, and refusing on that would leave nothing solvable.
fn branch_is_zero(slope: &Expr) -> bool {
    match slope {
        Expr::If(_, a, b) => {
            matches!(a.as_ref(), Expr::Number(x) if *x == 0.0)
                || matches!(b.as_ref(), Expr::Number(x) if *x == 0.0)
                || branch_is_zero(a)
                || branch_is_zero(b)
        }
        // A conditional under an arithmetic node carries its zero
        // upward: `2 * (if c then m else 0)` is zero on that branch
        // as surely as the conditional itself is. A sum does not -
        // the other term may be what makes it live - so only the
        // shapes where a zero factor decides the whole are followed.
        Expr::Neg(inner) => branch_is_zero(inner),
        Expr::Bin(oxidelica_parser::BinOp::Mul, l, r) => branch_is_zero(l) || branch_is_zero(r),
        Expr::Bin(oxidelica_parser::BinOp::Div, l, _) => branch_is_zero(l),
        _ => false,
    }
}

pub(crate) fn differentiate(expr: &Expr, target: &DiffTarget) -> Result<Expr, String> {
    MINTED.with(|c| c.borrow_mut().clear());
    NEEDED.with(|c| c.borrow_mut().clear());
    differentiate_at(expr, target, 0)
}

thread_local! {
    /// Derivatives of definitions given a name of their own in this
    /// call: the minted name against what it stands for. Only names
    /// reached with nothing held still are minted - inside an implicit
    /// derivative the chain of held names changes the answer, and a
    /// name shared between two chains would carry the wrong one.
    static MINTED: std::cell::RefCell<HashMap<String, Expr>> =
        std::cell::RefCell::new(HashMap::new());
    /// The rule turned on for this thread alone. A test measuring a
    /// parked rule must not turn it on for the tests beside it, and an
    /// environment variable in a test binary is shared by every thread
    /// in it.
    static SHARE_HERE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Names whose derivative was answered with a bare `der(x)` that
    /// nothing here can work out: the walk had no definition, no
    /// dummy and no implicit equation for `x`, and the equation that
    /// determines it is known to the matching rather than to this
    /// module. The caller supplies each of these with its matched
    /// equation, differentiated.
    static NEEDED: std::cell::RefCell<Vec<String>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

/// The names minted with no value during the current top-level
/// `differentiate`, for the caller to supply equations for.
pub(crate) fn take_needed_derivatives() -> Vec<String> {
    NEEDED.with(|n| {
        let mut needed = std::mem::take(&mut *n.borrow_mut());
        needed.sort();
        needed.dedup();
        needed
    })
}

/// Whether the derivative of a definition becomes a name of its own.
///
/// A definition is written out afresh at every occurrence of the name
/// it defines, and a machine model meets the same name many times in
/// one differentiated constraint: `CurrentControlledDCPM` inlines
/// eighty-three definitions five million times over in a single index
/// reduction, and the expression goes from a kilobyte to five
/// gigabytes over twenty of them. A table of answers does not help -
/// measured, the sizes came out identical to the digit, because a
/// cached tree is cloned into every occurrence just as a freshly
/// worked one is. The cost is the shape and not the work.
///
/// So the derivative of a definition is given a name, `der(x)`, and
/// the definition of that name joins the system once. The same move
/// Pantelides makes for a demoted state, made for an algebraic one:
/// k occurrences become k references.
///
/// Off by default, and the reason is measured rather than guessed.
/// Inlining a definition is also what flattens an algebraic loop: with
/// the name kept, `Translational.Examples.Brake` becomes a loop the
/// tearing cannot plan, and three more models refuse alongside it,
/// against two won. Four for two is not a fix, it is a different
/// compiler, and the choice of which one to be is not this switch's to
/// make quietly. `OXIDELICA_SHARED_DERIVATIVES=1` runs it.
fn shared_derivatives() -> bool {
    SHARE_HERE.with(|forced| forced.get())
        || std::env::var_os("OXIDELICA_SHARED_DERIVATIVES").is_some()
}

/// Derivatives minted for definitions during the current top-level
/// `differentiate`, waiting for the caller to put them in the system.
pub(crate) fn take_minted_derivatives() -> Vec<(String, Expr)> {
    MINTED.with(|m| {
        let mut minted: Vec<(String, Expr)> = m.borrow_mut().drain().collect();
        minted.sort_by(|a, b| a.0.cmp(&b.0));
        minted
    })
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
        // Whatever does not move has a derivative of zero, whichever
        // function stands around it. `KinematicPTP` writes
        // `1/max(abs(aux1))` where `aux1` is a quotient of two
        // parameters: nothing in it changes with time, so the answer
        // is zero and no rule for `abs` is needed to say so. Asked
        // structurally instead, eleven models were refused for a
        // function whose argument never moves.
        //
        // A rule for `abs` itself would be `sign(x)*der(x)`, which is
        // what other tools do and is wrong at exactly zero - and zero
        // is where physical models live rather than a corner they
        // avoid: a relay switching, friction breaking away, a gap
        // closing, a flow reversing. This says nothing about `abs`
        // instead, and what it does say is true everywhere.
        _ if matches!(target, DiffTarget::Time { .. }) && does_not_move(expr, target) => {
            Expr::Number(0.0)
        }
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
                implicit_defs,
                holding,
            } => {
                if holding.last() == Some(&name.as_str()) {
                    // The name this implicit derivative is being taken
                    // for: held still by construction, which is what
                    // `dg/dt at x fixed` means.
                    Expr::Number(0.0)
                } else if holding.contains(&name.as_str()) {
                    // Reached again through a *different* equation, so
                    // the two determine each other and neither can be
                    // differentiated alone - that wants a linear system
                    // and not a quotient. Refused rather than held
                    // still, which would be a wrong number where the
                    // honest answer is that this is not implemented.
                    return Err(format!(
                        "`{name}` and the unknown determining it depend on each other"
                    ));
                } else if let Some(rhs) = state_rhs.get(name) {
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
                    if shared_derivatives() && holding.is_empty() {
                        // Written out here, the definition is written
                        // out again at the next occurrence of the same
                        // name, and index reduction differentiates its
                        // own output. A name instead, defined once.
                        let minted = crate::derivative_name(name);
                        if !MINTED.with(|m| m.borrow().contains_key(&minted)) {
                            // Claimed before the work, so a definition
                            // reaching itself through another finds a
                            // name rather than recurring for ever.
                            MINTED.with(|m| {
                                m.borrow_mut().insert(minted.clone(), Expr::Number(0.0));
                            });
                            match d(definition) {
                                Ok(worked) => MINTED.with(|m| {
                                    m.borrow_mut().insert(minted.clone(), worked);
                                }),
                                Err(reason) => {
                                    MINTED.with(|m| {
                                        m.borrow_mut().remove(&minted);
                                    });
                                    return Err(reason);
                                }
                            }
                        }
                        Expr::Ref(minted)
                    } else {
                        d(definition)?
                    }
                } else if let Some((l, r)) = implicit_defs.get(name) {
                    // An unknown its equation cannot be solved for.
                    // `Psi = Linf*i + c*atan(i/Ipar)` determines the
                    // current from the flux, and no rearrangement puts
                    // `i` alone on a side - but the derivative does not
                    // need the solution, only the implicit function
                    // theorem. With residual `g(t, x) = 0`,
                    //
                    //     dx/dt = -(dg/dt at x fixed) / (dg/dx).
                    //
                    // Both halves are derivatives this module already
                    // takes: the numerator with `x` held still, the
                    // denominator with respect to `x`. A slope of zero
                    // is a genuine singularity and is refused rather
                    // than divided by, since a wrong number is worse
                    // than a refusal.
                    let residual = Expr::Bin(Sub, Box::new(l.clone()), Box::new(r.clone()));
                    let slope = simplify(&differentiate_at(
                        &residual,
                        &DiffTarget::Variable(name),
                        depth + 1,
                    )?);
                    if matches!(slope, Expr::Number(s) if s == 0.0) {
                        return Err(format!(
                            "the equation determining `{name}` does not depend on it"
                        ));
                    }
                    let mut chain: Vec<&str> = holding.to_vec();
                    chain.push(name.as_str());
                    let held = DiffTarget::Time {
                        state_rhs,
                        params,
                        dummies,
                        alg_defs,
                        implicit_defs,
                        holding: &chain,
                    };
                    let motion = simplify(&differentiate_at(&residual, &held, depth + 1)?);
                    bin(Div, Expr::Neg(Box::new(motion)), slope)
                } else if holding.is_empty() {
                    // Nothing here determines `name`, and substitution
                    // is the wrong instrument for what does: a current
                    // in a circuit is named by a Kirchhoff node that
                    // rewrites into its neighbour for ever. The
                    // matching knows which single equation determines
                    // this unknown, so the derivative takes a name of
                    // its own and the caller brings the equation.
                    //
                    // Only with nothing held still: inside an implicit
                    // derivative the chain of held names changes the
                    // answer, and a name shared between two chains
                    // would carry the wrong one.
                    let minted = crate::derivative_name(name);
                    NEEDED.with(|n| n.borrow_mut().push(name.clone()));
                    Expr::Ref(minted)
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
        Expr::Bin(Div, a, b) => {
            // A denominator whose derivative is zero divides the
            // derivative and nothing else: `(a/J)' = a'/J` exactly,
            // and no quotient rule is wanted. Written the general way
            // the answer carries `(a'*J - a*0)/J^2`, whose zero folds
            // while the `J/J^2` does not - nothing here cancels a name
            // against its own square.
            //
            // Harmless once and fatal in a loop. Index reduction
            // differentiates its own output, so a model demoting a
            // state per reduction squares the denominator again each
            // time. On ControlledDCDrives.CurrentControlledDCPM the
            // reduction number stood still at twenty while the
            // expression went 14k, 90k, 5M, 111M characters - a factor
            // of twenty-two a reduction, killed for memory rather than
            // looping. Every division there is by a parameter; it is
            // the shape of the answer that grows.
            //
            // Asked of the derivative rather than of the denominator,
            // which is what makes it cover both doors. Time is one:
            // `J` is a parameter and does not move. The other is
            // `solve_linear_for`, which takes a slope against a single
            // name with every parameter beside it standing still, and
            // it mints the candidate definitions the fixpoint then
            // hands back to the walk. Guarding only the first left the
            // growth in place through the second - 10k, 64k, 3.4M,
            // 110M over the same reductions - which is how the two
            // doors were found to be one identity.
            let da = d(a)?;
            let db = d(b)?;
            if matches!(simplify(&db), Expr::Number(n) if n == 0.0) {
                bin(Div, da, (**b).clone())
            } else {
                bin(
                    Div,
                    bin(
                        Sub,
                        bin(Mul, da, (**b).clone()),
                        bin(Mul, (**a).clone(), db),
                    ),
                    bin(Pow, (**b).clone(), Expr::Number(2.0)),
                )
            }
        }
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
            // here is what the name resolved to: a function the
            // compiler answers itself keeps its own name with the
            // package path resolved away to *nothing* in front of it -
            // `.sin`, an empty head and a bare dot. Matched whole,
            // that was a function with no derivative rule, and every
            // model with a sine source was refused as structurally
            // singular.
            //
            // Only that shape. A name with a path still on it is
            // somebody's own function - a package may write its own
            // `sin`, and giving it the built-in's derivative would be
            // a wrong number rather than a refusal. Shortening a name
            // to its tail is a guess; an empty head is a resolution.
            let name = match name.strip_prefix('.') {
                Some(bare) if !bare.contains('.') => bare,
                _ => name.as_str(),
            };
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

/// Whether nothing in an expression changes as time passes.
///
/// Strictly: every leaf is a literal, a parameter or a constant. Not
/// `time`, not a state, not a discrete, not an algebraic unknown, and
/// not a call whose arguments move - a call on things that do not move
/// does not move either, whatever the function does inside.
///
/// The strictness is the point: a looser reading, anything along the
/// lines of "probably constant", would be a guess, and a guess in
/// differentiation is a wrong number rather than a refusal.
fn does_not_move(expr: &Expr, target: &DiffTarget) -> bool {
    let DiffTarget::Time {
        state_rhs,
        params,
        dummies,
        alg_defs,
        implicit_defs,
        holding,
    } = target
    else {
        return false;
    };
    match expr {
        Expr::Number(_) | Expr::Bool(_) | Expr::Str(_) => true,
        Expr::Time => false,
        Expr::Ref(name) => {
            // Only the innermost held name stands still. `dg/dt at x
            // fixed` holds `x`, and nothing else in the chain: a name
            // from further up is reached through a *different*
            // equation, which makes the two determine each other, and
            // that is refused by the walk itself rather than answered.
            // Read as "everything in the chain is constant", this
            // folded such a name to zero and returned a derivative with
            // a term missing - a wrong number where the refusal on
            // `Ref` was owed, and the refusal was unreachable for as
            // long as it stood.
            if holding.last() == Some(&name.as_str()) {
                return true;
            }
            if holding.contains(&name.as_str()) {
                return false;
            }
            if state_rhs.contains_key(name)
                || dummies.contains_key(name)
                || implicit_defs.contains_key(name)
            {
                return false;
            }
            if params.contains_key(name) {
                return true;
            }
            // An unknown whose definition does not move does not move
            // either: `KinematicPTP` writes `aux1[i] = p_deltaq[i]/
            // p_qd_max[i]` and then `1/max(abs(aux1))`, so `aux1` is
            // an algebraic name standing for a quotient of two
            // parameters. Followed one step at a time and never
            // through itself, which is what keeps a definition that
            // mentions its own name from being read as constant.
            match alg_defs.get(name) {
                Some(definition) => {
                    let mut refs = Vec::new();
                    definition.collect_refs(&mut refs);
                    !refs.iter().any(|r| *r == name) && does_not_move(definition, target)
                }
                None => false,
            }
        }
        Expr::Neg(inner) | Expr::Not(inner) => does_not_move(inner, target),
        Expr::Bin(_, l, r)
        | Expr::Elementwise(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r) => does_not_move(l, target) && does_not_move(r, target),
        Expr::If(c, a, b) => {
            does_not_move(c, target) && does_not_move(a, target) && does_not_move(b, target)
        }
        Expr::Call(_, args) | Expr::Array(args) => {
            args.iter().all(|arg| does_not_move(arg, target))
        }
        Expr::Index(base, subscripts) => {
            does_not_move(base, target) && subscripts.iter().all(|s| does_not_move(s, target))
        }
        _ => false,
    }
}

/// Turn the shared-derivative rule on for this thread, for a test that
/// measures it.
#[cfg(test)]
pub(crate) fn share_derivatives_here() {
    SHARE_HERE.with(|forced| forced.set(true));
}
