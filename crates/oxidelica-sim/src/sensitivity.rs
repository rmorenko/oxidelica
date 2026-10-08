//! How strongly a constraint holds a state it reaches only through the
//! definitions of algebraic unknowns.
//!
//! The pivot of index reduction weighs each candidate by the slope of
//! the constraint's residual in it, and a state the residual does not
//! name has a slope of zero there by construction: `y = sin(phi)` with
//! `phi` found from `x = cos(phi)` holds `x` as firmly as it holds `y`,
//! and reads as having nothing to do with it. That froze a pendulum at
//! its turning point.
//!
//! The weight is worked out numerically rather than symbolically. The
//! symbolic way - a derivative by time taken through every definition -
//! is exact and has no size of its own: on `PlanarFourbar` it grew past
//! nine gigabytes before the run began. Here the state is moved by a
//! small step, the definitions downstream of it are worked out again
//! in order, and the residual is read twice. The cost per candidate is
//! the number of definitions the state reaches, which the compiler
//! already holds, and nothing is built that outlives the reading.

use crate::*;

/// One definition on the way from a state to a residual, in the order
/// they have to be worked out.
#[derive(Debug, Clone)]
pub(crate) enum ConeDef {
    /// `name := expr`.
    Explicit(String, Expr),
    /// `name` is where `expr` is zero.
    Implicit(String, Expr),
}

impl ConeDef {
    fn name(&self) -> &str {
        match self {
            ConeDef::Explicit(name, _) | ConeDef::Implicit(name, _) => name,
        }
    }

    fn expr(&self) -> &Expr {
        match self {
            ConeDef::Explicit(_, expr) | ConeDef::Implicit(_, expr) => expr,
        }
    }
}

/// The definitions a residual reads, each after the ones it reads.
///
/// A name met again while its own definition is still being expanded
/// is read as it stands rather than followed, which is what keeps two
/// implicit equations naming each other from being walked for ever.
pub(crate) fn cone_of(
    residual: &Expr,
    alg_defs: &HashMap<String, Expr>,
    implicit_defs: &HashMap<String, (Expr, Expr)>,
) -> Vec<ConeDef> {
    let definition = |name: &str| -> Option<ConeDef> {
        if let Some(expr) = alg_defs.get(name) {
            Some(ConeDef::Explicit(name.to_string(), expr.clone()))
        } else {
            implicit_defs.get(name).map(|(l, r)| {
                ConeDef::Implicit(
                    name.to_string(),
                    Expr::Bin(
                        oxidelica_parser::BinOp::Sub,
                        Box::new(l.clone()),
                        Box::new(r.clone()),
                    ),
                )
            })
        }
    };
    let refs_of = |expr: &Expr| -> Vec<String> {
        let mut named = Vec::new();
        expr.collect_refs(&mut named);
        named.into_iter().map(str::to_string).collect()
    };
    let mut order = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Each entry is a definition and the names it reads that are still
    // to be looked at; it is written out once they have all been.
    let mut stack: Vec<(ConeDef, Vec<String>)> = Vec::new();
    for name in refs_of(residual) {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(def) = definition(&name) else {
            continue;
        };
        let reads = refs_of(def.expr());
        stack.push((def, reads));
        while let Some((_, reads)) = stack.last_mut() {
            if let Some(next) = reads.pop() {
                if seen.insert(next.clone()) {
                    if let Some(def) = definition(&next) {
                        let reads = refs_of(def.expr());
                        stack.push((def, reads));
                    }
                }
            } else if let Some((def, _)) = stack.pop() {
                order.push(def);
            }
        }
    }
    order
}

/// The definitions of a cone that move when `state` moves, in order.
fn downstream<'a>(cone: &'a [ConeDef], state: &str) -> Vec<&'a ConeDef> {
    let mut moving: std::collections::HashSet<&str> = std::collections::HashSet::new();
    moving.insert(state);
    let mut kept = Vec::new();
    for def in cone {
        let mut named = Vec::new();
        def.expr().collect_refs(&mut named);
        if named
            .iter()
            .any(|name| *name != def.name() && moving.contains(name))
        {
            moving.insert(def.name());
            kept.push(def);
        }
    }
    kept
}

/// How far a state is moved to read its weight, relative to its size.
///
/// Chosen on the pendulum by measurement: see the test beside the
/// monitor. The implicit definitions are settled far tighter than
/// this, so the difference reads the constraint and not the solve.
fn step_for(value: f64) -> f64 {
    1e-6 * (1.0 + value.abs())
}

/// Settle `g(v) = 0` by Newton from `start`, with the slope taken by a
/// difference. `None` where the slope vanishes or the iteration does
/// not settle: the weight is then not known, and nothing is guessed.
///
/// Settled means the step has come down to the arithmetic's floor, not
/// below a fixed fraction of the value. A definition that has converged
/// to the last digit goes on rocking by a unit or two in the last place
/// of its residual, and a step of that size divided by a small slope
/// can stand far above any fixed fraction of a small value: a current of
/// 1.9e-4 rocked by 1.4e-14 against a bar of 1e-14, a pressure of 1e5
/// by 6.6e-7 against 1e-9. The weight came out not known, the monitor
/// read that as a selection gone bad, and three models were rebuilt in
/// mid-run for nothing. So a step that is already small and has stopped
/// halving is the floor, and there is no constant in it to be wrong
/// about the size of the value.
fn settle_one(start: f64, mut g: impl FnMut(f64) -> Option<f64>) -> Option<f64> {
    let mut v = start;
    let mut last_step = f64::INFINITY;
    for _ in 0..40 {
        let here = g(v)?;
        let h = 1e-7 * (1.0 + v.abs());
        let slope = (g(v + h)? - here) / h;
        if slope == 0.0 || !slope.is_finite() {
            return None;
        }
        let step = here / slope;
        v -= step;
        if !v.is_finite() {
            return None;
        }
        if step.abs() <= 1e-14 * (1.0 + v.abs()) {
            return Some(v);
        }
        // Newton roughly squares the error while it is still gaining;
        // a small step no smaller than half the last one is noise.
        if settle_at_the_floor()
            && step.abs() <= 1e-8 * (1.0 + v.abs())
            && step.abs() >= 0.5 * last_step
        {
            return Some(v);
        }
        last_step = step.abs();
    }
    None
}

/// Whether a Newton rocking at the floor of the arithmetic counts as
/// settled. On by default; `OXIDELICA_PARTIAL_SENSITIVITY=1` keeps the
/// old weighing whole, this with it.
fn settle_at_the_floor() -> bool {
    !crate::compile::partial_sensitivity()
}

/// The weight at the start point, for the pivot: the residual's change
/// per unit change of `state` with every definition it reaches worked
/// out again. The definitions are first settled at the start point
/// itself, since a start value of an algebraic unknown is a guess and
/// not a solution.
pub(crate) fn weigh_at_start(
    residual: &Expr,
    state: &str,
    cone: &[ConeDef],
    env: &HashMap<String, f64>,
    time: f64,
    programs: &HashMap<String, ClassDef>,
) -> Option<f64> {
    let chain = downstream(cone, state);
    if chain.is_empty() {
        return Some(0.0);
    }
    let mut vars = env.clone();
    let x = *vars.get(state)?;
    let read = |expr: &Expr, vars: &HashMap<String, f64>| -> Option<f64> {
        eval(
            expr,
            &EvalCtx {
                vars,
                time,
                programs: Some(programs),
                depth: 0,
            },
        )
        .ok()
        .filter(|value| value.is_finite())
    };
    let work = |vars: &mut HashMap<String, f64>| -> Option<f64> {
        for def in &chain {
            let value = match def {
                ConeDef::Explicit(_, expr) => read(expr, vars)?,
                ConeDef::Implicit(name, expr) => {
                    let start = *vars.get(name)?;
                    let mut trial = vars.clone();
                    settle_one(start, |v| {
                        trial.insert(name.clone(), v);
                        read(expr, &trial)
                    })?
                }
            };
            vars.insert(def.name().to_string(), value);
        }
        read(residual, vars)
    };
    let before = work(&mut vars)?;
    let h = step_for(x);
    vars.insert(state.to_string(), x + h);
    let after = work(&mut vars)?;
    Some((after - before) / h)
}

/// The residual's change per unit change of `state` with every
/// definition of the cone settled together, by one Newton over all of
/// them, every other state held where it stands.
///
/// `weigh_at_start` works the definitions out one at a time in order,
/// each with the ones after it held, which is exact for a chain and
/// not for definitions that hold one another. A machine's stator writes
/// its three phase currents as three implicit equations that each read
/// all three: worked one at a time, an open star's `starpoint.i = 0`
/// weighed the space-phasor current `lssigma.i_[1]` at 3.0, and settled
/// together it weighs exactly zero, which is what the physics says -
/// the star holds the zero sequence and nothing else. `None` where the
/// cone will not settle or its own Jacobian is singular: then nothing
/// is known, and nothing is guessed.
pub(crate) fn weigh_jointly(
    residual: &Expr,
    state: &str,
    cone: &[ConeDef],
    env: &HashMap<String, f64>,
    time: f64,
    programs: &HashMap<String, ClassDef>,
) -> Option<f64> {
    let names: Vec<&str> = cone.iter().map(ConeDef::name).collect();
    let read = |expr: &Expr, vars: &HashMap<String, f64>| -> Option<f64> {
        eval(
            expr,
            &EvalCtx {
                vars,
                time,
                programs: Some(programs),
                depth: 0,
            },
        )
        .ok()
        .filter(|value| value.is_finite())
    };
    // An explicit definition is the residual `name - expr`, so that one
    // Newton settles both kinds alike.
    let gaps = |vars: &HashMap<String, f64>| -> Option<Vec<f64>> {
        cone.iter()
            .map(|def| match def {
                ConeDef::Explicit(name, expr) => Some(*vars.get(name)? - read(expr, vars)?),
                ConeDef::Implicit(_, expr) => read(expr, vars),
            })
            .collect()
    };
    let settle = |vars: &mut HashMap<String, f64>| -> Option<()> {
        for _ in 0..30 {
            let here = gaps(vars)?;
            let mut columns = Vec::with_capacity(names.len());
            for name in &names {
                let v = *vars.get(*name)?;
                let h = 1e-7 * (1.0 + v.abs());
                vars.insert(name.to_string(), v + h);
                let moved = gaps(vars)?;
                vars.insert(name.to_string(), v);
                columns.push(
                    moved
                        .iter()
                        .zip(&here)
                        .map(|(m, g)| (m - g) / h)
                        .collect::<Vec<f64>>(),
                );
            }
            let step = solve_dense(&columns, &here)?;
            for (name, delta) in names.iter().zip(&step) {
                let v = vars[*name] - delta;
                if !v.is_finite() {
                    return None;
                }
                vars.insert(name.to_string(), v);
            }
            let size = step.iter().fold(0.0f64, |m, v| m.max(v.abs()));
            let gap = here.iter().fold(0.0f64, |m, v| m.max(v.abs()));
            if size <= 1e-13 && gap <= 1e-9 {
                return Some(());
            }
        }
        None
    };
    let mut vars = env.clone();
    let x = *vars.get(state)?;
    settle(&mut vars)?;
    let before = read(residual, &vars)?;
    let h = step_for(x);
    vars.insert(state.to_string(), x + h);
    settle(&mut vars)?;
    let after = read(residual, &vars)?;
    Some((after - before) / h)
}

/// `x` with `A x = b`, the matrix given by its columns, by elimination
/// with partial pivoting. `None` where a pivot is zero.
fn solve_dense(columns: &[Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    let n = b.len();
    let mut a: Vec<Vec<f64>> = (0..n)
        .map(|i| columns.iter().map(|c| c[i]).collect())
        .collect();
    let mut b = b.to_vec();
    for k in 0..n {
        let pivot = (k..n).max_by(|&i, &j| a[i][k].abs().total_cmp(&a[j][k].abs()))?;
        if a[pivot][k] == 0.0 {
            return None;
        }
        a.swap(k, pivot);
        b.swap(k, pivot);
        let (above, below) = a.split_at_mut(k + 1);
        let pivot_row = &above[k];
        for (offset, row) in below.iter_mut().enumerate() {
            let factor = row[k] / pivot_row[k];
            for (cell, top) in row[k..].iter_mut().zip(&pivot_row[k..]) {
                *cell -= factor * top;
            }
            b[k + 1 + offset] -= factor * b[k];
        }
    }
    let mut x = vec![0.0; n];
    for k in (0..n).rev() {
        let known: f64 = (k + 1..n).map(|j| a[k][j] * x[j]).sum();
        x[k] = (b[k] - known) / a[k][k];
    }
    Some(x)
}

/// The same weight as run-time code, for the monitor that asks after
/// every accepted step whether the pivot would still choose the same.
#[derive(Debug, Clone)]
pub(crate) struct ConeCode {
    state: Slot,
    /// Each definition's slot, whether it is implicit, and its code.
    steps: Vec<(Slot, bool, Code)>,
    residual: Code,
}

impl ConeCode {
    /// Lay out a cone against the run's slots. `None` where a name the
    /// cone reads has no place in the run: the caller then keeps the
    /// slope the monitor always had.
    pub(crate) fn build(
        residual: &Expr,
        state: &str,
        cone: &[ConeDef],
        table: &SlotTable,
    ) -> Option<ConeCode> {
        let state_slot = table.existing_slot(state)?;
        let mut steps = Vec::new();
        for def in downstream(cone, state) {
            let slot = table.existing_slot(def.name())?;
            let code = table.compile(def.expr()).ok()?;
            steps.push((slot, matches!(def, ConeDef::Implicit(..)), code));
        }
        Some(ConeCode {
            state: state_slot,
            steps,
            residual: table.compile(residual).ok()?,
        })
    }

    /// The weight at the point `scratch` holds, which is a point of the
    /// run and so already consistent. Every slot moved is put back.
    /// `NaN` where an implicit definition will not settle - which the
    /// monitor's maximum passes over rather than reading as a zero.
    pub(crate) fn run(&self, scratch: &mut [f64], time: f64) -> f64 {
        if self.steps.is_empty() {
            return 0.0;
        }
        let before = self.residual.run(scratch, time);
        let saved: Vec<f64> = self.steps.iter().map(|(slot, ..)| scratch[*slot]).collect();
        let x = scratch[self.state];
        let h = step_for(x);
        scratch[self.state] = x + h;
        let mut settled = true;
        for (slot, implicit, code) in &self.steps {
            let value = if *implicit {
                let start = scratch[*slot];
                settle_one(start, |v| {
                    scratch[*slot] = v;
                    Some(code.run(scratch, time)).filter(|g| g.is_finite())
                })
            } else {
                Some(code.run(scratch, time))
            };
            match value {
                Some(value) => scratch[*slot] = value,
                None => {
                    settled = false;
                    break;
                }
            }
        }
        let after = self.residual.run(scratch, time);
        scratch[self.state] = x;
        for ((slot, ..), value) in self.steps.iter().zip(saved) {
            scratch[*slot] = value;
        }
        if settled {
            (after - before) / h
        } else {
            f64::NAN
        }
    }
}

/// One entry of the selection monitor: the residual's own slope in a
/// name, or the weight worked through the definitions.
#[derive(Debug, Clone)]
pub(crate) enum Weigh {
    Slope(Code),
    Through(ConeCode),
}

impl Weigh {
    pub(crate) fn run(&self, scratch: &mut [f64], time: f64) -> f64 {
        match self {
            Weigh::Slope(code) => code.run(scratch, time),
            Weigh::Through(cone) => cone.run(scratch, time),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(name: &str) -> Expr {
        Expr::Ref(name.into())
    }

    #[test]
    fn a_state_behind_an_implicit_definition_is_weighed_through_it() {
        // y - sin(phi), with phi where x - cos(phi) is zero: at x = 0.5
        // phi = pi/3, and d/dx sin(acos(x)) = -x / sqrt(1 - x^2).
        let residual = Expr::Bin(
            oxidelica_parser::BinOp::Sub,
            Box::new(r("y")),
            Box::new(Expr::Call(".sin".into(), vec![r("phi")])),
        );
        let mut implicit = HashMap::new();
        implicit.insert(
            "phi".to_string(),
            (r("x"), Expr::Call(".cos".into(), vec![r("phi")])),
        );
        let cone = cone_of(&residual, &HashMap::new(), &implicit);
        assert_eq!(cone.len(), 1);
        let env: HashMap<String, f64> = [("x", 0.5), ("y", 0.8), ("phi", 0.9)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let weight =
            weigh_at_start(&residual, "x", &cone, &env, 0.0, &HashMap::new()).expect("weighed");
        let exact = 0.5 / (1.0f64 - 0.25).sqrt();
        assert!((weight - exact).abs() < 1e-5, "{weight} against {exact}");
        // What the residual names itself is not in the cone's reach.
        assert_eq!(
            weigh_at_start(&residual, "y", &cone, &env, 0.0, &HashMap::new()),
            Some(0.0)
        );
    }

    #[test]
    fn a_definition_that_will_not_settle_is_not_a_weight() {
        // x - phi^2 has no root at x = -1.
        let residual = r("phi");
        let mut implicit = HashMap::new();
        implicit.insert(
            "phi".to_string(),
            (
                r("x"),
                Expr::Bin(
                    oxidelica_parser::BinOp::Mul,
                    Box::new(r("phi")),
                    Box::new(r("phi")),
                ),
            ),
        );
        let cone = cone_of(&residual, &HashMap::new(), &implicit);
        let env: HashMap<String, f64> = [("x", -1.0), ("phi", 0.5)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        assert_eq!(
            weigh_at_start(&residual, "x", &cone, &env, 0.0, &HashMap::new()),
            None
        );
    }

    #[test]
    fn definitions_that_hold_one_another_are_weighed_settled_together() {
        // p and q hold each other: p - (a + q) = 0 and q - (b - p) = 0,
        // so p = (a + b)/2 and q = (b - a)/2. The residual p + q = b
        // reads nothing of `a`, though worked one definition at a time
        // it would.
        let sub =
            |l: Expr, r: Expr| Expr::Bin(oxidelica_parser::BinOp::Sub, Box::new(l), Box::new(r));
        let add =
            |l: Expr, r: Expr| Expr::Bin(oxidelica_parser::BinOp::Add, Box::new(l), Box::new(r));
        let residual = add(r("p"), r("q"));
        let mut implicit = HashMap::new();
        implicit.insert("p".to_string(), (r("p"), add(r("a"), r("q"))));
        implicit.insert("q".to_string(), (r("q"), sub(r("b"), r("p"))));
        let cone = cone_of(&residual, &HashMap::new(), &implicit);
        let env: HashMap<String, f64> = [("a", 1.0), ("b", 2.0), ("p", 0.0), ("q", 0.0)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let none = HashMap::new();
        let a = weigh_jointly(&residual, "a", &cone, &env, 0.0, &none).expect("weighed");
        let b = weigh_jointly(&residual, "b", &cone, &env, 0.0, &none).expect("weighed");
        assert!(a.abs() < 1e-6, "a weighs {a}");
        assert!((b - 1.0).abs() < 1e-6, "b weighs {b}");
    }

    #[test]
    fn a_cone_with_no_unique_solution_is_not_a_weight() {
        // p - (a + q) = 0 twice over: p and q are not determined.
        let add =
            |l: Expr, r: Expr| Expr::Bin(oxidelica_parser::BinOp::Add, Box::new(l), Box::new(r));
        let residual = r("p");
        let mut implicit = HashMap::new();
        implicit.insert("p".to_string(), (r("p"), add(r("a"), r("q"))));
        implicit.insert("q".to_string(), (r("p"), add(r("a"), r("q"))));
        let cone = cone_of(&residual, &HashMap::new(), &implicit);
        let env: HashMap<String, f64> = [("a", 1.0), ("p", 0.0), ("q", 0.0)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        assert_eq!(
            weigh_jointly(&residual, "a", &cone, &env, 0.0, &HashMap::new()),
            None
        );
    }

    #[test]
    fn a_definition_rocking_at_the_last_digit_has_settled() {
        // g(v) = v - r with noise of two parts in 1e14 whose sign turns
        // each iteration: Newton lands on the root to the noise and then
        // steps back and forth by twice it for ever.
        let root = 1.9e-4;
        let mut calls = 0usize;
        let settled = settle_one(1e-3, |v| {
            // Two readings per iteration, one for the slope.
            let iteration = calls / 2;
            calls += 1;
            let noise = if iteration.is_multiple_of(2) {
                2e-14
            } else {
                -2e-14
            };
            Some(v - root + noise)
        });
        let v = settled.expect("a converged definition is settled");
        assert!((v - root).abs() < 1e-12, "{v} against {root}");
    }
}
