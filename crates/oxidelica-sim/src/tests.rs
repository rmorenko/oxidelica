//! The symbolic layer, which has no public face of its own: constant
//! folding, substitution, differentiation, and the compile-time
//! evaluator agreeing with the code the run uses.

use super::*;
use oxidelica_parser::parse_model;

fn expr_of(source_expr: &str) -> Expr {
    let model = parse_model(&format!(
        "model E Real a; Real b; Real q; equation q = {source_expr}; a = 1; b = 2; end E;"
    ))
    .unwrap();
    model
        .equations
        .iter()
        .find_map(|e| match (&e.lhs, &e.rhs) {
            (Expr::Ref(n), rhs) if n == "q" => Some(rhs.clone()),
            _ => None,
        })
        .unwrap()
}

/// Evaluate an expression with the given variable bindings.
fn value_of(expr: &Expr, bindings: &[(&str, f64)]) -> f64 {
    let vars: HashMap<String, f64> = bindings
        .iter()
        .map(|(n, v)| ((*n).to_string(), *v))
        .collect();
    eval(
        expr,
        &EvalCtx {
            vars: &vars,
            time: 0.0,
            programs: None,
            depth: 0,
        },
    )
    .unwrap()
}

#[test]
fn simplify_folds_constants_and_identities() {
    let cases = [
        ("2 * 3 + 1", 7.0),
        ("a * 0", 0.0),
        ("0 * a", 0.0),
        ("a * 1", 1.0),
        ("1 * a", 1.0),
        ("a + 0", 1.0),
        ("0 + a", 1.0),
        ("a - 0", 1.0),
        ("0 - a", -1.0),
        ("a / 1", 1.0),
        ("0 / a", 0.0),
        ("a ^ 1", 1.0),
        ("a ^ 0", 1.0),
        ("-(2)", -2.0),
    ];
    for (source, expected) in cases {
        let folded = simplify(&expr_of(source));
        assert_eq!(
            value_of(&folded, &[("a", 1.0), ("b", 2.0)]),
            expected,
            "{source} folded to {folded:?}"
        );
    }
    // Structure-preserving branches still simplify their children.
    let nested = simplify(&expr_of(
        "if a > 0 and b > 0 or not a > 0 then a * 1 else b + 0",
    ));
    assert_eq!(value_of(&nested, &[("a", 1.0), ("b", 2.0)]), 1.0);
    assert_eq!(
        value_of(&simplify(&expr_of("sin(a * 1)")), &[("a", 0.0)]),
        0.0
    );
}

#[test]
fn substitute_replaces_every_occurrence() {
    let expr = expr_of("if a > 0 and a < 5 or not a > 9 then sin(a) + (-a) else a / 2 ^ a");
    let substituted = substitute(&expr, "a", 0.0);
    let mut refs = Vec::new();
    substituted.collect_refs(&mut refs);
    assert!(!refs.contains(&"a"), "a survived: {substituted:?}");
    assert_eq!(value_of(&substituted, &[]), 0.0);
}

#[test]
fn differentiates_every_elementary_function() {
    // d/da of f(a) at a = 0.7, compared with a central difference.
    for name in [
        "sin", "cos", "tan", "exp", "log", "sqrt", "atan", "sinh", "cosh", "tanh",
    ] {
        let expr = expr_of(&format!("{name}(a)"));
        let derivative = simplify(&differentiate(&expr, &DiffTarget::Variable("a")).unwrap());
        let (point, step) = (0.7f64, 1e-6);
        let numeric = (value_of(&expr_of(&format!("{name}(a)")), &[("a", point + step)])
            - value_of(&expr_of(&format!("{name}(a)")), &[("a", point - step)]))
            / (2.0 * step);
        let symbolic = value_of(&derivative, &[("a", point)]);
        assert!(
            (symbolic - numeric).abs() < 1e-5,
            "{name}: symbolic {symbolic} vs numeric {numeric}"
        );
    }
    // Products, quotients, powers and if-expressions.
    let d = |source: &str| {
        simplify(&differentiate(&expr_of(source), &DiffTarget::Variable("a")).unwrap())
    };
    // `mod` and `rem` are a straight line with a staircase taken off
    // it: between the steps the derivative is the argument's own, and
    // a table asked to repeat wraps its abscissa exactly this way.
    assert_eq!(value_of(&d("mod(a, 2)"), &[("a", 0.7)]), 1.0);
    assert_eq!(value_of(&d("mod(a, 2)"), &[("a", 3.4)]), 1.0);
    assert_eq!(value_of(&d("rem(a, 2)"), &[("a", 3.4)]), 1.0);
    // Where the period moves too, the staircase counts: at `a = 3.4`
    // and `b = 2` the wrap has happened once, so a period growing by
    // one takes one off what `mod` comes to.
    assert_eq!(value_of(&d("mod(3.4, a)"), &[("a", 2.0)]), -1.0);
    assert_eq!(value_of(&d("a * b"), &[("a", 3.0), ("b", 2.0)]), 2.0);
    assert_eq!(value_of(&d("a / b"), &[("a", 3.0), ("b", 2.0)]), 0.5);
    assert_eq!(value_of(&d("a ^ 3"), &[("a", 2.0)]), 12.0);
    assert_eq!(value_of(&d("-a"), &[("a", 2.0)]), -1.0);
    assert_eq!(
        value_of(&d("if b > 0 then a * 2 else a"), &[("a", 1.0), ("b", 1.0)]),
        2.0
    );
    // Refusals: unknown function, non-constant exponent, time target.
    assert!(differentiate(&expr_of("atan2(a, b)"), &DiffTarget::Variable("a")).is_err());
    assert!(differentiate(&expr_of("a ^ b"), &DiffTarget::Variable("a")).is_err());
    assert_eq!(
        value_of(
            &differentiate(&expr_of("time"), &DiffTarget::Variable("a")).unwrap(),
            &[]
        ),
        0.0
    );
}

#[test]
fn a_call_carrying_its_own_rule_is_worked_on_through_the_value() {
    // `f(a)` worth `a * a`, with the model's own rule for its
    // derivative: `2 * a` times whatever `a`'s derivative is. Nothing
    // here could have been worked out from the value - that is the
    // point of a rule - so the answers below can only come from it.
    let node = |argument: Expr| {
        Expr::WithDerivative(
            Box::new(Expr::Bin(
                oxidelica_parser::BinOp::Mul,
                Box::new(argument.clone()),
                Box::new(argument.clone()),
            )),
            Box::new(expr_of(
                "(if a >= 0 and not (a < 0) or false then 1 else -1) * 2 * a * seed0",
            )),
            vec![("seed0".to_string(), argument)],
        )
    };
    let call = node(expr_of("a"));

    // Differentiating by `a` seeds the rule with `da/da`, which is one.
    let by_a = simplify(&differentiate(&call, &DiffTarget::Variable("a")).unwrap());
    assert_eq!(value_of(&by_a, &[("a", 3.0)]), 6.0);
    // By anything else the seed is zero, and the rule multiplies out.
    let by_b = simplify(&differentiate(&call, &DiffTarget::Variable("b")).unwrap());
    assert_eq!(value_of(&by_b, &[("a", 3.0), ("b", 1.0)]), 0.0);

    // Folding reaches inside without losing the rule, and so does
    // putting a number in the place of a variable.
    let folded = simplify(&node(expr_of("a * 1")));
    assert!(matches!(folded, Expr::WithDerivative(..)));
    assert_eq!(value_of(&folded, &[("a", 4.0)]), 16.0);
    let pinned = substitute(&call, "a", 5.0);
    assert_eq!(value_of(&pinned, &[]), 25.0);
    assert_eq!(
        value_of(
            &simplify(&differentiate(&pinned, &DiffTarget::Variable("a")).unwrap()),
            &[]
        ),
        0.0
    );

    // A rule of an inner call is left alone where an outer one is
    // seeded: the two functions' parameter names mean nothing to each
    // other, and `seed0` in one is not `seed0` in the other.
    let nested = node(call.clone());
    let outer = simplify(&differentiate(&nested, &DiffTarget::Variable("a")).unwrap());
    // At a = 2 the inner rule gives 2a = 4, and the outer one takes
    // that as its seed: 2a * 4 = 16.
    assert_eq!(value_of(&outer, &[("a", 2.0)]), 16.0);
}

#[test]
fn nonlinear_equations_are_not_solved_symbolically() {
    // x * x = 4 is not linear in x, so no closed form is offered.
    let expr = expr_of("a * a");
    assert!(solve_linear_for(&expr, &Expr::Number(4.0), "a").is_none());
    // ... but 3 * x - 6 = 0 is.
    let linear = expr_of("3 * a - 6");
    let solution = solve_linear_for(&linear, &Expr::Number(0.0), "a").unwrap();
    assert!((value_of(&solution, &[]) - 2.0).abs() < 1e-12);
}

#[test]
fn an_equation_whose_slope_is_zero_is_not_solved_for_that_unknown() {
    // `R = 100 * (1 + alpha * (T - 293.15))` with `alpha = 0` says
    // nothing whatever about `T`. Divided through anyway it hands back
    // an infinity, and the run reports it as a residual that could not
    // be evaluated - which names the solver, the one place nothing is
    // wrong. Zero written outright is caught with no parameters at all.
    let lhs = expr_of("r");
    let rhs = expr_of("100 * (1 + 0 * (a - 293.15))");
    assert!(solve_linear_for(&lhs, &rhs, "a").is_none());

    // The same equation with the coefficient behind a name. Judged
    // without the parameter values the slope looks live, so this is the
    // half that needs them in view; and with `alpha` nonzero the
    // equation is solved as it always was.
    let rhs = expr_of("100 * (1 + alpha * (a - 293.15))");
    let mut known = HashMap::new();
    known.insert("alpha".to_string(), 0.0);
    assert!(solve_linear_known(&lhs, &rhs, "a", &known).is_none());
    known.insert("alpha".to_string(), 1e-3);
    assert!(solve_linear_known(&lhs, &rhs, "a", &known).is_some());
}

#[test]
fn the_banded_solver_agrees_with_the_dense_one() {
    // A tridiagonal system with a dominant diagonal, the shape a
    // discretized field gives: both paths must land on the same
    // answer, and it must satisfy the equations.
    let n = 12usize;
    let band = 1usize;
    let dense: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|j| match i.abs_diff(j) {
                    0 => 4.0 + i as f64 * 0.1,
                    1 => -1.0,
                    _ => 0.0,
                })
                .collect()
        })
        .collect();
    let rhs: Vec<f64> = (0..n).map(|i| (i as f64 * 0.7).sin()).collect();

    let packed: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            (0..2 * band + 1)
                .map(|offset| match (i + offset).checked_sub(band) {
                    Some(column) if column < n => dense[i][column],
                    _ => 0.0,
                })
                .collect()
        })
        .collect();
    let banded = solve_banded(&mut packed.clone(), band, &rhs).expect("diagonally dominant");
    let plain = solve_linear(&mut dense.clone(), &rhs).expect("nonsingular");
    for (a, b) in banded.iter().zip(&plain) {
        assert!((a - b).abs() < 1e-12, "{a} vs {b}");
    }
    // And the answer really solves the system.
    for (i, row) in dense.iter().enumerate() {
        let value: f64 = row.iter().zip(&banded).map(|(a, x)| a * x).sum();
        assert!((value - rhs[i]).abs() < 1e-12);
    }

    // Without a diagonal to pivot on it declines instead of dividing
    // by nothing, and the caller falls back to the dense path.
    let mut hollow = vec![vec![0.0, 0.0, 1.0], vec![1.0, 0.0, 0.0]];
    assert!(solve_banded(&mut hollow, 1, &[1.0, 1.0]).is_none());
}

/// The shapes a connection equation leaves behind. Solving `-p.i +
/// r.p.i = 0` for one of its currents gives `-(-r.n.i)/-1`: the signs
/// it was moved across and the coefficient it was divided by are all
/// still written, and read literally that is not a name. Everything
/// downstream that asks "is this just a name" said no, so a current
/// that a connection set pins was never taken as a definition.
///
/// Folded here rather than read through later, which is one place
/// instead of every place that looks.
#[test]
fn the_wrappers_a_connection_set_leaves_are_folded_away() {
    let cases = [
        ("-(-a)", "a"),
        ("-(-a)/(-1)", "-a"),
        ("a/1", "a"),
        ("a*1", "a"),
        ("-a/(-1)", "a"),
        ("(-1)*(-a)", "a"),
    ];
    for (written, meant) in cases {
        assert_eq!(
            format!("{:?}", simplify(&expr_of(written))),
            format!("{:?}", simplify(&expr_of(meant))),
            "{written}"
        );
    }
}

#[test]
fn two_implicit_unknowns_determining_each_other_are_refused() {
    // Two unknowns, each determined by an equation no rearrangement
    // solves, and each equation naming the other name. Reaching the
    // second one from inside the first one's `dg/dt at x fixed` means
    // neither can be differentiated alone - that wants a linear system
    // and not a quotient, and the honest answer is a refusal.
    //
    // The mutual guard on `Ref` was written for exactly this and could
    // not be reached: `does_not_move` answered true for every name in
    // the chain, so a name from the middle was silently folded to zero
    // and the derivative came out as a number with a term missing. A
    // wrong number where a refusal was owed is the worst thing this
    // compiler can do, so the test is on the refusal and not on a
    // model flattening.
    let state_rhs = HashMap::new();
    let params: HashMap<String, f64> = HashMap::from([("k".to_string(), 0.3)]);
    let dummies = HashMap::new();
    let alg_defs = HashMap::new();
    // g1: Psi1 = 0.1*a + k*a*a*b, determining `a` and naming `b`.
    // g2: Psi2 = 0.1*b + k*b*b*a, determining `b` and naming `a`.
    let implicit_defs: HashMap<String, (Expr, Expr)> = HashMap::from([
        (
            "a".to_string(),
            (expr_of("0.1 * a + k * a * a * b"), Expr::Time),
        ),
        (
            "b".to_string(),
            (expr_of("0.1 * b + k * b * b * a"), Expr::Time),
        ),
    ]);
    let target = DiffTarget::Time {
        state_rhs: &state_rhs,
        params: &params,
        dummies: &dummies,
        alg_defs: &alg_defs,
        implicit_defs: &implicit_defs,
        holding: &[],
    };
    let outcome = differentiate(&Expr::Ref("a".to_string()), &target);
    let reason = outcome.expect_err("mutually determined unknowns cannot be differentiated");
    assert!(
        reason.contains("depend on each other"),
        "expected the mutual refusal, got: {reason}"
    );
}

/// Differentiating by a still denominator must not grow the expression.
///
/// Index reduction differentiates its own output, so any residue the
/// quotient rule leaves behind is raised to a power once per reduction.
/// A division by a parameter is the common shape - `tau/J` in every
/// rotational model - and the general rule answers it with
/// `(a'*J - a*0)/J^2`, whose `J^2` becomes `(J^2)^2` the next time
/// round. On CurrentControlledDCPM that ran to 111M characters by the
/// twentieth reduction and was killed for memory.
///
/// Measured as repeated differentiation, because one pass looks
/// harmless: it is the growth and not the first answer that is the bug.
#[test]
fn a_still_denominator_does_not_grow_under_repeated_differentiation() {
    let state_rhs: HashMap<String, Expr> = HashMap::from([("w".to_string(), expr_of("tau"))]);
    let params: HashMap<String, f64> = HashMap::from([("j".to_string(), 0.5)]);
    let dummies = HashMap::new();
    let mut alg_defs: HashMap<String, Expr> = HashMap::from([("tau".to_string(), expr_of("w"))]);
    let implicit_defs = HashMap::new();

    let mut expr = simplify(&expr_of("w / j"));
    let first = format!("{expr:?}").len();
    for round in 0..6 {
        let taken = {
            let target = DiffTarget::Time {
                state_rhs: &state_rhs,
                params: &params,
                dummies: &dummies,
                alg_defs: &alg_defs,
                implicit_defs: &implicit_defs,
                holding: &[],
            };
            differentiate(&expr, &target).expect("a quotient by a parameter")
        };
        // The derivative of a definition is a name now, and the name
        // needs its equation before the next round can reach through
        // it - which is what the reduction loop does with these.
        for (minted, value) in take_minted_derivatives() {
            alg_defs.entry(minted).or_insert_with(|| simplify(&value));
        }
        expr = simplify(&taken);
        let size = format!("{expr:?}").len();
        assert!(
            size <= first * 3,
            "round {round}: {size} characters against {first} at the start - \
             the denominator is squaring itself"
        );
    }
}

/// The same identity through the door `solve_linear_for` uses.
///
/// A slope taken against one name has every parameter beside it
/// standing still, so `(a/L)` differentiated by `a` is `1/L` and not
/// `(1*L - a*0)/L^2`. This is not a second rule but the same one seen
/// from the other side, and it had to be measured separately: guarding
/// only the time walk left CurrentControlledDCPM growing through the
/// candidate definitions this mints - 10k, 64k, 3.4M, 110M characters.
#[test]
fn a_slope_by_variable_does_not_square_its_denominator() {
    let solved = solve_linear_for(&expr_of("q"), &expr_of("a / l"), "a")
        .expect("a quotient linear in its numerator");
    let printed = format!("{solved:?}");
    assert!(
        !printed.contains("Pow"),
        "the slope squared its denominator: {printed}"
    );
}

/// A definition met k times costs one copy of its derivative, not k.
///
/// A definition is written out afresh wherever the name it defines
/// occurs, and index reduction differentiates its own output: on
/// `CurrentControlledDCPM` eighty-three definitions were inlined five
/// million times in a single reduction, and the constraint reached
/// five gigabytes. Memoising the work does not help, because a
/// remembered tree is cloned into each occurrence exactly as a freshly
/// worked one is - measured, the sizes came out identical to the
/// digit. Only a *name* in place of the subtree makes the answer flat
/// in k.
///
/// What is measured is the whole system the walk leaves behind - the
/// differentiated expression and the definitions minted for it -
/// because a rule that hides the body somewhere else has not made it
/// smaller. k references are genuinely linear in k and that is not
/// the fault; the fault is the *body* appearing k times, so the
/// witness grows the body at a fixed k and asks what that costs.
#[test]
fn a_definition_met_many_times_is_referenced_and_not_copied() {
    // The rule is parked behind a switch, so the witness turns it on
    // for this thread alone: what is measured is the rule, and the
    // tests running beside it keep the default.
    share_derivatives_here();
    let system_size = |k: usize, terms: usize| -> usize {
        let state_rhs: HashMap<String, Expr> = HashMap::from([("x".to_string(), expr_of("u"))]);
        let params: HashMap<String, f64> = HashMap::new();
        let dummies = HashMap::new();
        let body = vec!["x * x"; terms].join(" + ");
        let alg_defs: HashMap<String, Expr> = HashMap::from([("u".to_string(), expr_of(&body))]);
        let implicit_defs = HashMap::new();
        let target = DiffTarget::Time {
            state_rhs: &state_rhs,
            params: &params,
            dummies: &dummies,
            alg_defs: &alg_defs,
            implicit_defs: &implicit_defs,
            holding: &[],
        };
        let sum = vec!["u"; k].join(" + ");
        let taken = differentiate(&expr_of(&sum), &target).expect("a definition");
        let minted: usize = take_minted_derivatives()
            .iter()
            .map(|(name, value)| name.len() + format!("{value:?}").len())
            .sum();
        format!("{taken:?}").len() + minted
    };
    // One occurrence pays for the body once, whatever else happens.
    let once = system_size(1, 12);
    // Twelve occurrences of the same fat definition must not pay
    // for it twelve times: what they add is eleven names.
    let many = system_size(12, 12);
    assert!(
        many < once * 2,
        "the definition is copied rather than referenced: {once} characters for one \
         occurrence against {many} for twelve"
    );
}
