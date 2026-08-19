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
