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

/// Replace every reference the table names, in one walk.
///
/// The same work as calling `substitute` once per name, at the price
/// of one traversal rather than one per name. The difference is not a
/// constant: a slope carrying a thousand references was walked a
/// thousand times over, which is quadratic in a tree that index
/// reduction is perfectly capable of growing to that size.
fn substitute_all(expr: &Expr, table: &HashMap<&str, f64>) -> Expr {
    match expr {
        Expr::Ref(name) => match table.get(name.as_str()) {
            Some(value) => Expr::Number(*value),
            None => expr.clone(),
        },
        _ => expr.map_children(&mut |child| substitute_all(child, table)),
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
    //
    // And the cheap question comes before any substitution at all: a
    // slope naming nothing the table knows is folded into itself, and
    // the walk that discovers this costs one lookup per name against a
    // whole rebuild of the tree. Index reduction hands slopes with
    // thousands of references here, and the two faults compounded -
    // one walk per name over a tree that size is quadratic, and the
    // reduction on `RollingWheel` never came out of it.
    let wanted: HashMap<&str, f64> = refs
        .iter()
        .filter_map(|name| known.get(*name).map(|value| (*name, *value)))
        .collect();
    let judged = if wanted.is_empty() {
        slope.clone()
    } else {
        simplify(&substitute_all(&slope, &wanted))
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

/// Solve `lhs = rhs` for a variable that stands only under a division.
///
/// A reluctance is written `R_m = 1/G_m`, and no amount of linear
/// solving reaches `G_m`: the equation is linear in the *reciprocal*
/// and nothing else. With no explicit solution the equation joins the
/// tearing set, and Newton is handed a one-dimensional block whose
/// derivative is `-1/G_m^2` - which is enormous beside the pole and
/// flat away from it. Started off the zero it walks outward, the slope
/// dies away faster than the residual does, and what comes back is
/// `singular Jacobian` about an equation that has one plain answer.
///
/// So the reciprocal is solved for instead, and the answer inverted:
/// with `u = 1/var`, an equation linear in `u` gives `u` in closed
/// form, and `var = 1/u`. This is exact rather than a guess at a
/// starting point, which is what makes it worth doing at the plan's
/// layer rather than the solver's.
///
/// The substitution has to be honest about where `var` occurs. If the
/// variable appears anywhere except as a whole divisor - multiplied in
/// as well, say, or inside a call - then `1/u` is not what stands
/// there, and the equation is quadratic rather than linear in either
/// unknown. Those are left alone, so this widens what can be solved
/// without claiming anything the substitution does not support.
pub(crate) fn solve_reciprocal_known(
    lhs: &Expr,
    rhs: &Expr,
    var: &str,
    known: &HashMap<String, f64>,
) -> Option<Expr> {
    // A name no equation of this model could have written, so that
    // the reciprocal cannot collide with something the model calls
    // its own. The substituted equation is solved and thrown away;
    // only the inverted answer, in the model's own names, is kept.
    let fresh = format!("$recip${var}");
    let left = as_reciprocal(lhs, var, &fresh)?;
    let right = as_reciprocal(rhs, var, &fresh)?;
    // Both sides free of the divisor shape means the variable never
    // appeared at all, and this is not the equation for it.
    if !mentions_name(&left, &fresh) && !mentions_name(&right, &fresh) {
        return None;
    }
    let solved = solve_linear_known(&left, &right, &fresh, known)?;
    // The reciprocal's own solution must not mention the variable it
    // replaced, or the inversion would be circular.
    if mentions_name(&solved, var) || mentions_name(&solved, &fresh) {
        return None;
    }
    // A reciprocal of zero is the equation saying it has no solution,
    // not a value to hand back. `1/x = 0` is satisfied by no `x` at
    // all, and inverted without asking it yields an infinity that the
    // run would carry as though it were a number - the guessing this
    // compiler owes a refusal instead of. Left alone, the equation
    // reaches the layer that says an algebraic loop has come apart,
    // which is the true thing to say about it.
    if matches!(simplify(&solved), Expr::Number(x) if x == 0.0) {
        return None;
    }
    Some(simplify(&Expr::Bin(
        oxidelica_parser::BinOp::Div,
        Box::new(Expr::Number(1.0)),
        Box::new(solved),
    )))
}

/// Rewrite `a / var` as `a * u`, refusing any other occurrence of `var`.
///
/// The refusal is the point. A variable that also stands on its own
/// makes the equation quadratic once the reciprocal is introduced, and
/// handing back a linear-looking expression there would give a wrong
/// number where a refusal is owed.
fn as_reciprocal(expr: &Expr, var: &str, fresh: &str) -> Option<Expr> {
    match expr {
        Expr::Bin(oxidelica_parser::BinOp::Div, dividend, divisor) if matches!(divisor.as_ref(), Expr::Ref(name) if name == var) =>
        {
            // The dividend still has to be clean: `var / var` is one,
            // not something to solve.
            if mentions_name(dividend, var) {
                return None;
            }
            Some(Expr::Bin(
                oxidelica_parser::BinOp::Mul,
                Box::new(dividend.as_ref().clone()),
                Box::new(Expr::Ref(fresh.to_string())),
            ))
        }
        Expr::Ref(name) if name == var => None,
        _ => expr
            .try_map_children(&mut |child| as_reciprocal(child, var, fresh).ok_or(()))
            .ok(),
    }
}

/// Whether an expression mentions a name.
fn mentions_name(expr: &Expr, var: &str) -> bool {
    let mut refs = Vec::new();
    expr.collect_refs(&mut refs);
    refs.contains(&var)
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

/// Whether an expression divides by any of the given names without
/// having guarded against that name being zero.
///
/// An inner assignment of a torn block is evaluated before Newton has
/// moved anything, at whatever the block's unknowns happen to start
/// from - and a block unknown starts at its declared `start`, which is
/// zero unless a declaration said otherwise. So a divisor that is
/// itself such an unknown is a division by zero on the first
/// evaluation: `v = R_actual*i` solved as `R_actual := v/i` reads `i`
/// at its start of zero, and the whole chain below it comes out NaN
/// before a single step is taken. The refusal that follows names the
/// solver, which is the one place nothing is wrong.
///
/// A division the model itself guarded is not one of these, and the
/// distinction is worth six models. The standard library writes
/// `L_stat = if abs(i) > eps then Psi/i else L_nominal`, which is a
/// division that cannot be reached where it would fail; refusing it
/// along with the rest grows the Newton system until the magnetic
/// examples' blocks come back singular. So a division under a
/// conditional that tests the divisor is left alone, and only a
/// division nothing stands between counts.
///
/// Only the divisor is walked, and only for the names handed in: a
/// parameter in a denominator is a number the plan can trust, and a
/// name settled before the block runs is evaluated before it.
///
/// And a divisor that *mentions* such a name is not therefore zero.
/// `mu_r = 1 + (mu_i - 1 + c_a*B_N)/(1 + c_b*B_N + B_N^n)` divides by
/// a sum that reads exactly one where `B_N` starts at nothing, so the
/// assignment is perfectly safe - and refused on the mention alone it
/// pushed `mu_r` into the tearing set, where Newton started it at zero
/// and `G_m = mu_0*mu_r*A/l` came out zero, and `R_m = 1/G_m` came out
/// infinite before the first step. The test that matters is what the
/// divisor *comes to* at the starts, so the zero-start names are put
/// at zero and whatever else is settled at its value: a divisor that
/// folds to a number other than zero divides by nothing that vanishes.
/// A divisor that will not fold at all keeps the old answer, because
/// what cannot be worked out cannot be trusted.
pub(crate) fn divides_by_any(expr: &Expr, names: &[&str], settled: &HashMap<String, f64>) -> bool {
    // The names a conditional tests on the way down, so that a
    // division below it can tell whether its own divisor was the
    // thing asked about.
    fn walk(expr: &Expr, names: &[&str], guarded: &[&str], settled: &HashMap<String, f64>) -> bool {
        match expr {
            Expr::If(condition, then, otherwise) => {
                let mut tested = Vec::new();
                condition.collect_refs(&mut tested);
                let mut deeper: Vec<&str> = guarded.to_vec();
                deeper.extend(tested.iter().copied());
                walk(condition, names, guarded, settled)
                    || walk(then, names, &deeper, settled)
                    || walk(otherwise, names, &deeper, settled)
            }
            Expr::Bin(oxidelica_parser::BinOp::Div, dividend, divisor) => {
                let mut refs = Vec::new();
                divisor.collect_refs(&mut refs);
                let unguarded = refs
                    .iter()
                    .any(|name| names.contains(name) && !guarded.contains(name))
                    && !divisor_is_nonzero_at_starts(divisor, names, settled);
                // The other way a first evaluation divides by zero,
                // and the one the block's own unknowns cannot show: a
                // divisor built entirely of names that are settled
                // already - a state at its start, a parameter - which
                // comes to exactly zero. `sin(angles[3])` with the
                // angle starting at zero is that divisor, and the
                // rolling wheel's inner assignment for the second
                // Euler rate divides by it. Nothing about the block is
                // wrong; the plan chose to assign through a quotient
                // that has no value at the point it is first asked
                // for, so the equation belongs in the tearing set with
                // the rest.
                let settled_zero = !refs.is_empty()
                    && refs.iter().all(|name| !guarded.contains(name))
                    && divisor_is_zero_at_starts(divisor, settled);
                unguarded
                    || settled_zero
                    || walk(dividend, names, guarded, settled)
                    || walk(divisor, names, guarded, settled)
            }
            other => {
                let mut found = false;
                other.map_children(&mut |child| {
                    found |= walk(child, names, guarded, settled);
                    child.clone()
                });
                found
            }
        }
    }
    walk(expr, names, &[], settled)
}

/// Whether a divisor works out to something other than zero at the
/// values the block starts from: the zero-start unknowns at zero, and
/// everything already settled at what it settled on.
fn divisor_is_nonzero_at_starts(
    divisor: &Expr,
    names: &[&str],
    settled: &HashMap<String, f64>,
) -> bool {
    let mut refs = Vec::new();
    divisor.collect_refs(&mut refs);
    let mut table: HashMap<&str, f64> = HashMap::new();
    for name in refs {
        if let Some(value) = settled.get(name) {
            table.insert(name, *value);
        } else if names.contains(&name) {
            table.insert(name, 0.0);
        } else {
            // A name that is neither settled nor known to start at
            // zero: the divisor cannot be worked out, so nothing is
            // claimed about it.
            return false;
        }
    }
    matches!(simplify(&substitute_all(divisor, &table)), Expr::Number(value) if value != 0.0)
}

/// Whether a divisor comes to exactly zero at the values the run
/// starts from, using only names whose starting value is already
/// settled.
///
/// Stricter than its neighbour above in the one way that matters: a
/// name that is not settled makes the answer no. Nothing is guessed
/// at zero here, so what this reports is a division that certainly
/// cannot be done rather than one that might not be.
fn divisor_is_zero_at_starts(divisor: &Expr, settled: &HashMap<String, f64>) -> bool {
    let mut refs = Vec::new();
    divisor.collect_refs(&mut refs);
    let mut table: HashMap<String, f64> = HashMap::new();
    for name in refs {
        match settled.get(name) {
            Some(value) => {
                table.insert(name.to_string(), *value);
            }
            None => return false,
        }
    }
    // Worked out rather than simplified. `sin(angles[3])` at an angle
    // of zero is zero, and the simplifier says nothing about a call -
    // it is an arrangement of terms, not an arithmetic. Reading a
    // divisor of a call through the simplifier alone is how this test
    // first came back saying nothing at all about the rolling wheel,
    // whose divisor is exactly such a call.
    matches!(crate::eval_at_starts(divisor, &table), Some(value) if value == 0.0)
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
            // A constant nobody folded is still a constant. The
            // exponent arrives here as the flat model wrote it, and a
            // library writes `2/3` rather than the number it comes
            // to - `Bin(Pow, Number(9.01e-5), Bin(Div, Number(2.0),
            // Number(3.0)))` in `DryAirNasa`, all literals, and
            // `Neg(Number(0.14874))` beside it in the same equation.
            // Asked as a literal, both are non-constant exponents and
            // the derivative is refused over arithmetic that could
            // have been done at any time. So the question is put to
            // the instrument that answers it, the same way the
            // divisor one case up is put to it: a live exponent, the
            // `a^b` of `DifferenceAmplifier`, does not fold and goes
            // on being refused in the same words.
            let Expr::Number(c) = simplify(exponent) else {
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
        // `min(a, b)` is `if a < b then a else b` written as a call,
        // and the branch already has a rule: each side differentiated
        // under the condition that selected it. Not a new rule, then,
        // but the definition written out - which is why it is exact
        // everywhere the two arguments differ, and says nothing at
        // all about where they cross. That is the same standing the
        // branch itself has, and the crossing is an event indicator
        // the solver stops at rather than integrates across.
        //
        // Two arguments only. `min(v)` over an array picks by a
        // search rather than by a comparison of two names, and there
        // is no branch to write it as; asked for one, the catch-all
        // below still refuses it by name.
        Expr::Call(name, args) if matches!(name.as_str(), "min" | "max") && args.len() == 2 => {
            let (a, b) = (&args[0], &args[1]);
            let op = if name == "min" {
                oxidelica_parser::RelOp::Lt
            } else {
                oxidelica_parser::RelOp::Gt
            };
            Expr::If(
                Box::new(Expr::Rel(op, Box::new(a.clone()), Box::new(b.clone()))),
                Box::new(d(a)?),
                Box::new(d(b)?),
            )
        }
        // `atan2(y, x)` is the angle of a point, and its derivative is
        // the one in every table: `(x*dy - y*dx) / (x^2 + y^2)`. Unlike
        // `atan`, whose rule is written out among the one-argument
        // functions above, this one needs both arguments at once - the
        // denominator is the squared radius rather than anything the
        // chain rule would build from a single input, and a rule taken
        // from `atan(y/x)` would be right only where `x` never moves.
        //
        // It says nothing about the origin, where the angle is not
        // defined at all and the denominator is nothing; that is the
        // same standing `atan2` itself has there.
        //
        // The name is matched the way the one-argument table matches
        // its own: bare, or with the package path resolved away to
        // nothing in front of it. A name with a path still on it is
        // somebody's own `atan2` and gets no rule of ours.
        Expr::Call(name, args)
            if matches!(name.as_str(), "atan2" | ".atan2") && args.len() == 2 =>
        {
            let (y, x) = (&args[0], &args[1]);
            bin(
                Div,
                bin(Sub, bin(Mul, x.clone(), d(y)?), bin(Mul, y.clone(), d(x)?)),
                bin(
                    Add,
                    bin(Mul, x.clone(), x.clone()),
                    bin(Mul, y.clone(), y.clone()),
                ),
            )
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
        // Everything the rules above do not reach. A refusal names
        // what it refused, and here that is two things rather than
        // one: which construction was met, because that is what says
        // whether the work is a missing rule or a shape that has no
        // derivative at all, and the expression as it was written, so
        // that the model it came from can be found. The catch-all this
        // replaces said neither, which is why ten different models
        // shared one line of the register.
        //
        // Matched by name and not swept up, so a variant added to
        // `Expr` has to be decided about here rather than quietly
        // joining the refusal.
        Expr::Str(_)
        | Expr::Rel(..)
        | Expr::And(..)
        | Expr::Or(..)
        | Expr::Not(_)
        | Expr::Call(..)
        | Expr::Index(..)
        | Expr::Member(..)
        | Expr::Array(_)
        | Expr::Elementwise(..)
        | Expr::Range(..)
        | Expr::Comprehension(..)
        | Expr::ColonSubscript
        | Expr::EndSubscript
        | Expr::MatrixRows(_)
        | Expr::NamedArg(..)
        | Expr::Tuple(_) => {
            return Err(format!(
                "cannot differentiate {}: `{}`",
                undifferentiable_kind(expr),
                expr.describe()
            ));
        }
    })
}

/// Which construction the differentiator met and had no rule for.
///
/// Not the expression itself, which the refusal quotes beside this:
/// this is the family, and it is what says whether the missing work is
/// a rule to write or a shape that should never have survived
/// flattening. A subscript that reached here is the second sort - the
/// arrays were meant to be taken apart long before - while a call of
/// several arguments is the first.
fn undifferentiable_kind(expr: &Expr) -> &'static str {
    match expr {
        Expr::Str(_) => "a string",
        Expr::Rel(..) => "a comparison",
        Expr::And(..) | Expr::Or(..) | Expr::Not(_) => "a Boolean operation",
        Expr::Call(..) => "a call of several arguments",
        Expr::Index(..) => "a subscript that survived flattening",
        Expr::Member(..) => "a field of a record",
        Expr::Array(_) | Expr::MatrixRows(_) => "an array written out",
        Expr::Elementwise(..) => "an elementwise operation",
        Expr::Range(..) => "a range",
        Expr::Comprehension(..) => "a comprehension",
        Expr::ColonSubscript | Expr::EndSubscript => "a subscript with no value",
        Expr::NamedArg(..) => "a named argument",
        Expr::Tuple(_) => "a tuple",
        // The differentiator has rules for these, so reaching here
        // with one means the fault is elsewhere; they are named rather
        // than swept up for the same reason as above.
        Expr::Number(_)
        | Expr::Bool(_)
        | Expr::Ref(_)
        | Expr::Time
        | Expr::Neg(_)
        | Expr::Bin(..)
        | Expr::If(..)
        | Expr::WithDerivative(..) => "an expression",
    }
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
