//! Turning a flat model into a plan: matching equations to unknowns,
//! reducing the index, tearing the loops and laying the result out as
//! stages a run can walk.

use crate::*;

/// Replace every `der(x)` with the right-hand side of that state.
pub(crate) fn substitute_derivatives(
    expr: &Expr,
    states: &[String],
    derivatives: &[Expr],
) -> Result<Expr, SimError> {
    if let Some(state) = expr.as_der_of() {
        let Some(index) = states.iter().position(|s| s == state) else {
            return err(format!(
                "der({state}): `{state}` is not a state of the model"
            ));
        };
        return Ok(derivatives[index].clone());
    }
    let recur = |e: &Expr| substitute_derivatives(e, states, derivatives);
    Ok(match expr {
        Expr::Call(name, args) => Expr::Call(
            name.clone(),
            args.iter()
                .map(recur)
                .collect::<Result<Vec<_>, SimError>>()?,
        ),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner)?)),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner)?)),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)?), Box::new(recur(r)?)),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)?), Box::new(recur(r)?)),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)?), Box::new(recur(r)?)),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)?), Box::new(recur(r)?)),
        Expr::If(c, a, b) => Expr::If(
            Box::new(recur(c)?),
            Box::new(recur(a)?),
            Box::new(recur(b)?),
        ),
        other => other.clone(),
    })
}

/// Which states each state's right-hand side depends on, and a grouping
/// of the columns that can be probed together.
///
/// The dependency runs through the algebraic plan: a derivative that
/// reads an algebraic variable depends on whatever that variable was
/// computed from. An implicit block is taken as a whole, since its
/// unknowns are solved together and each may move with any input of the
/// block.
///
/// Two columns may share a group when no row depends on both. Finding
/// the fewest groups is graph colouring, so this takes the greedy
/// answer, which for the banded Jacobians of a discretized field is
/// already the optimum: three groups for a tridiagonal one.
pub(crate) fn jacobian_structure(
    states: &[String],
    algebraics: &[String],
    derivatives: &[Expr],
    stages: &[PlanStage],
) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
    let index_of_state: HashMap<&str, usize> = states.iter().map(|s| s.as_str()).zip(0..).collect();
    let index_of_algebraic: HashMap<&str, usize> =
        algebraics.iter().map(|a| a.as_str()).zip(0..).collect();

    // What each algebraic variable depends on, in evaluation order.
    // `None` means "not worked out": such a variable is taken to depend
    // on every state, because a missing entry would quietly cost the
    // Jacobian a term while an extra one only costs an evaluation.
    let mut through_algebraic: Vec<Option<Vec<bool>>> = vec![None; algebraics.len()];
    let depends_on = |expr: &Expr, out: &mut Vec<bool>, through: &[Option<Vec<bool>>]| {
        let mut refs = Vec::new();
        expr.collect_refs(&mut refs);
        for name in refs {
            if let Some(&state) = index_of_state.get(name) {
                out[state] = true;
            } else if let Some(&algebraic) = index_of_algebraic.get(name) {
                match through.get(algebraic) {
                    Some(Some(inherited)) => {
                        for (state, &touched) in inherited.iter().enumerate() {
                            out[state] |= touched;
                        }
                    }
                    _ => out.iter_mut().for_each(|touched| *touched = true),
                }
            }
        }
    };

    for stage in stages {
        match stage {
            PlanStage::Explicit { var, expr } => {
                let mut row = vec![false; states.len()];
                depends_on(expr, &mut row, &through_algebraic);
                through_algebraic[*var] = Some(row);
            }
            PlanStage::Implicit {
                vars,
                inner,
                residuals,
                ..
            } => {
                // Everything the block reads reaches everything it solves.
                let mut row = vec![false; states.len()];
                for (_, expr) in inner {
                    depends_on(expr, &mut row, &through_algebraic);
                }
                for (lhs, rhs) in residuals {
                    depends_on(lhs, &mut row, &through_algebraic);
                    depends_on(rhs, &mut row, &through_algebraic);
                }
                for &var in vars {
                    through_algebraic[var] = Some(row.clone());
                }
            }
        }
    }

    // Rows of the Jacobian: the states each derivative depends on.
    let mut rows_of_column: Vec<Vec<usize>> = vec![Vec::new(); states.len()];
    for (row, expr) in derivatives.iter().enumerate() {
        let mut touched = vec![false; states.len()];
        depends_on(expr, &mut touched, &through_algebraic);
        for (column, &yes) in touched.iter().enumerate() {
            if yes {
                rows_of_column[column].push(row);
            }
        }
    }

    // Greedy colouring: a column joins the first group whose rows it
    // does not overlap.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut taken: Vec<Vec<bool>> = Vec::new();
    for (column, touched) in rows_of_column.iter().enumerate() {
        let mut placed = false;
        for (group, rows) in groups.iter_mut().zip(taken.iter_mut()) {
            if touched.iter().all(|&row| !rows[row]) {
                for &row in touched {
                    rows[row] = true;
                }
                group.push(column);
                placed = true;
                break;
            }
        }
        if !placed {
            let mut rows = vec![false; derivatives.len()];
            for &row in touched {
                rows[row] = true;
            }
            groups.push(vec![column]);
            taken.push(rows);
        }
    }
    (groups, rows_of_column)
}

/// Compile a parsed flat model into an executable form.
pub fn compile(model: &Model) -> Result<CompiledModel, SimError> {
    compile_at(model, None)
}

/// Sort the equations into the ones that give a state its derivative
/// and the ones that are algebraic.
///
/// `der(x)` has to stand alone on one side, which is what makes the
/// first kind recognisable; anything else is algebraic and need not be
/// in assignment form at all.
fn split_equations(
    equations: &[EquationItem],
    continuous: &[&str],
) -> Result<(HashMap<String, Expr>, Vec<(Expr, Expr)>), SimError> {
    let mut state_rhs: HashMap<String, Expr> = HashMap::new();
    let mut algebraic_eqs: Vec<(Expr, Expr)> = Vec::new();

    for EquationItem { lhs, rhs } in equations {
        // der(v) = expr  |  expr = der(v)
        let (target, value) = if let Some(v) = lhs.as_der_of() {
            (Some(v), rhs)
        } else if let Some(v) = rhs.as_der_of() {
            (Some(v), lhs)
        } else {
            (None, rhs)
        };
        if let Some(state) = target {
            if !continuous.contains(&state) {
                return err(format!(
                    "der({state}): {state} is not a continuous variable"
                ));
            }
            if value.contains_der() {
                return err("der() must appear alone on one side of an equation".to_string());
            }
            if state_rhs.insert(state.to_string(), value.clone()).is_some() {
                return err(format!("two equations for der({state})"));
            }
            continue;
        }
        if lhs.contains_der() || rhs.contains_der() {
            return err("der() must appear alone on one side of an equation".to_string());
        }
        algebraic_eqs.push((lhs.clone(), rhs.clone()));
    }
    Ok((state_rhs, algebraic_eqs))
}

/// The states, and the algebraic unknowns left over.
fn unknowns_of(
    continuous: &[&str],
    state_rhs: &HashMap<String, Expr>,
) -> (Vec<String>, Vec<String>) {
    let states = continuous
        .iter()
        .filter(|n| state_rhs.contains_key(**n))
        .map(|n| n.to_string())
        .collect();
    let unknowns = continuous
        .iter()
        .filter(|n| !state_rhs.contains_key(**n))
        .map(|n| n.to_string())
        .collect();
    (states, unknowns)
}

/// Every name the equations use must be one the model declared.
fn check_references(
    state_rhs: &HashMap<String, Expr>,
    algebraic_eqs: &[(Expr, Expr)],
    continuous: &[&str],
    params: &HashMap<String, f64>,
    discretes: &[String],
) -> Result<(), SimError> {
    let mut refs = Vec::new();
    for expr in state_rhs.values() {
        expr.collect_refs(&mut refs);
    }
    for (lhs, rhs) in algebraic_eqs {
        lhs.collect_refs(&mut refs);
        rhs.collect_refs(&mut refs);
    }
    if let Some(bad) = refs.iter().find(|r| {
        !continuous.contains(r)
            && !params.contains_key(**r)
            && !discretes.iter().any(|d| d == **r)
            // `$pre.x`, `$initial` and `$sampleN` are supplied by the
            // event machinery, not by the equations.
            && !r.starts_with('$')
    }) {
        return err(format!("unknown variable `{bad}` in equation"));
    }
    Ok(())
}

/// Which branch of each run-time `if` equation holds at this point.
///
/// A condition usually names a variable an ordinary equation defines -
/// `energised = sin(...) >= 0` - and start attributes say nothing
/// about those, so plain `name = expr` definitions are followed to a
/// fixpoint first. Then the conditions can be asked, before anything
/// has been matched or solved.
fn settle_modes(
    model: &Model,
    start_env: &HashMap<String, f64>,
    mode_time: f64,
    resuming: bool,
) -> Result<Vec<usize>, SimError> {
    let mut mode_env = start_env.clone();
    for _ in 0..MAX_DEFINITION_PASSES {
        let mut progress = false;
        for equation in &model.equations {
            let Expr::Ref(name) = &equation.lhs else {
                continue;
            };
            if resuming && mode_env.contains_key(name) {
                // A continuation already knows this from the run.
                continue;
            }
            if let Ok(value) = eval(
                &equation.rhs,
                &EvalCtx {
                    vars: &mode_env,
                    time: mode_time,
                },
            ) {
                if mode_env.insert(name.clone(), value) != Some(value) {
                    progress = true;
                }
            }
        }
        if !progress {
            break;
        }
    }
    let start_ctx = EvalCtx {
        vars: &mode_env,
        time: mode_time,
    };
    let mut modes: Vec<usize> = Vec::new();
    for conditional in &model.conditional {
        let mut taken = conditional.branches.len() - 1;
        for (index, condition) in conditional.conditions.iter().enumerate() {
            if eval(condition, &start_ctx)? != 0.0 {
                taken = index;
                break;
            }
        }
        modes.push(taken);
    }
    Ok(modes)
}

/// What every variable stands at, at the point being compiled for.
///
/// A fresh compilation reads the start attributes; a continuation
/// reads the run. The pivot that decides which states to demote works
/// from these numbers, so they have to be the numbers of *here*.
fn values_at_this_point(
    model: &Model,
    params: &HashMap<String, f64>,
    discretes: &[String],
    discrete_start: &[f64],
    resume: &Option<ResumePoint>,
) -> HashMap<String, f64> {
    let resumed = |name: &str| -> Option<f64> {
        resume
            .as_ref()
            .and_then(|point| point.values.get(name))
            .copied()
    };
    let mut env = params.clone();
    for (name, value) in discretes.iter().zip(discrete_start) {
        env.insert(name.clone(), resumed(name).unwrap_or(*value));
    }
    for component in &model.components {
        if component.variability == Variability::Continuous {
            let value = resumed(&component.name)
                .or_else(|| {
                    component.start.as_ref().and_then(|expr| {
                        eval(
                            expr,
                            &EvalCtx {
                                vars: params,
                                time: 0.0,
                            },
                        )
                        .ok()
                    })
                })
                .unwrap_or(0.0);
            env.insert(component.name.clone(), value);
        }
    }
    env
}

/// The discrete variables of a model, and the value each one starts
/// from.
///
/// A variable is discrete when it says so or when a `when` clause
/// assigns it: either way it keeps its value between events, so the
/// continuous part treats it as known. They come back in declaration
/// order, which is what keeps the result columns steady.
fn discrete_layer(
    model: &Model,
    params: &HashMap<String, f64>,
    resume: &Option<ResumePoint>,
) -> Result<(Vec<String>, Vec<f64>), SimError> {
    // 1b. The discrete layer. A variable is discrete when it says so or
    // when a `when` clause assigns it: either way it keeps its value
    // between events, so the continuous part treats it as known.
    let mut discrete_names: Vec<String> = Vec::new();
    for clause in &model.when_clauses {
        for branch in &clause.branches {
            for action in &branch.actions {
                if let WhenAction::Assign(target, _) = action {
                    if !discrete_names.contains(target) {
                        discrete_names.push(target.clone());
                    }
                }
            }
        }
    }
    for name in &discrete_names {
        if !model.components.iter().any(|c| &c.name == name) {
            return err(format!(
                "`{name}` is assigned by a when clause but never declared"
            ));
        }
    }
    for component in &model.components {
        if component.variability == Variability::Discrete
            && !discrete_names.contains(&component.name)
        {
            return err(format!(
                "discrete variable `{}` is never assigned by a when clause",
                component.name
            ));
        }
    }
    // Declaration order, so the result columns are stable.
    let discretes: Vec<String> = model
        .components
        .iter()
        .filter(|c| discrete_names.contains(&c.name))
        .map(|c| c.name.clone())
        .collect();
    let discrete_start: Vec<f64> = model
        .components
        .iter()
        .filter(|c| discrete_names.contains(&c.name))
        .map(|c| {
            resume
                .as_ref()
                .and_then(|point| point.values.get(&c.name))
                .copied()
                .or_else(|| {
                    c.start.as_ref().or(c.binding.as_ref()).and_then(|expr| {
                        eval(
                            expr,
                            &EvalCtx {
                                vars: &params,
                                time: 0.0,
                            },
                        )
                        .ok()
                    })
                })
                .unwrap_or(0.0)
        })
        .collect();
    Ok((discretes, discrete_start))
}

/// Work out the value of every parameter and constant.
///
/// One may be written in terms of another, in any order, so they are
/// evaluated in passes: each pass settles whatever it can, and a pass
/// that settles nothing means what is left refers to itself or to
/// something that is not there.
fn evaluate_parameters(model: &Model) -> Result<HashMap<String, f64>, SimError> {
    let mut params: HashMap<String, f64> = HashMap::new();
    let mut pending: Vec<(&str, &Expr)> = Vec::new();
    for c in &model.components {
        if matches!(
            c.variability,
            Variability::Parameter | Variability::Constant
        ) {
            let binding = c.binding.as_ref().or(c.start.as_ref());
            match binding {
                Some(expr) => pending.push((&c.name, expr)),
                None => return err(format!("parameter {} has no value", c.name)),
            }
        }
    }
    loop {
        let before = pending.len();
        pending.retain(|(name, expr)| {
            match eval(
                expr,
                &EvalCtx {
                    vars: &params,
                    time: 0.0,
                },
            ) {
                Ok(v) => {
                    params.insert((*name).to_string(), v);
                    false
                }
                Err(_) => true,
            }
        });
        if pending.is_empty() {
            break;
        }
        if pending.len() == before {
            let names: Vec<_> = pending.iter().map(|(n, _)| *n).collect();
            return err(format!(
                "cannot evaluate parameters {names:?}: cycle or unknown reference"
            ));
        }
    }
    Ok(params)
}

/// Compile a model, either from its declared start (`resume` absent) or
/// as a continuation from mid-run.
///
/// A continuation matters for one reason: the choice of which states to
/// demote during index reduction is a numerical pivot at a point, and a
/// choice that was right at the start can become singular later - a
/// pendulum in Cartesian coordinates crossing the horizontal. Compiling
/// again at the current point re-makes the choice with the sensitivities
/// of *now*, and everything downstream (matching, tearing, slots) simply
/// follows.
pub(crate) fn compile_at(
    model: &Model,
    resume: Option<ResumePoint>,
) -> Result<CompiledModel, SimError> {
    // 1. Parameters and constants, in whatever order they depend on
    // each other.
    let params = evaluate_parameters(model)?;

    // 1b. The discrete layer: what changes only at an event, and what
    // each of those starts at.
    let (discretes, discrete_start) = discrete_layer(model, &params, &resume)?;

    // Where everything stands at the point being compiled for: the
    // pivot that chooses which states to demote reads it, and so does
    // the question of which branch of a run-time `if` holds here.
    let resumed = |name: &str| -> Option<f64> {
        resume
            .as_ref()
            .and_then(|point| point.values.get(name))
            .copied()
    };
    let start_env = values_at_this_point(model, &params, &discretes, &discrete_start, &resume);

    // The event built-ins become references the evaluator can look up.
    let mut rewrite = EventRewrite {
        discretes: &discretes,
        params: &params,
        samples: Vec::new(),
        delays: Vec::new(),
    };
    // An `if` equation the compiler could not decide is settled here:
    // whichever branch holds at this point joins the model, and this
    // mode is then matched, torn and solved as its own set of
    // equations. The run watches the conditions and asks for a fresh
    // compilation when one of them flips.
    let mode_time = resume.as_ref().map_or(0.0, |point| point.time);
    let modes = settle_modes(model, &start_env, mode_time, resume.is_some())?;
    let equations: Vec<EquationItem> = model
        .equations
        .iter()
        .chain(
            model
                .conditional
                .iter()
                .zip(&modes)
                .flat_map(|(conditional, &taken)| &conditional.branches[taken]),
        )
        .map(|equation| {
            Ok(EquationItem {
                lhs: rewrite.expr(&equation.lhs)?,
                rhs: rewrite.expr(&equation.rhs)?,
            })
        })
        .collect::<Result<Vec<_>, SimError>>()?;
    // The conditions, rewritten the same way, so the run can tell when
    // the mode it was compiled for has been left behind.
    let mode_conditions: Vec<Vec<Expr>> = model
        .conditional
        .iter()
        .map(|conditional| {
            conditional
                .conditions
                .iter()
                .map(|condition| rewrite.expr(condition))
                .collect::<Result<Vec<_>, SimError>>()
        })
        .collect::<Result<Vec<_>, SimError>>()?;
    let mut when_clauses: Vec<WhenClause> = Vec::new();
    for clause in &model.when_clauses {
        let mut branches = Vec::new();
        for branch in &clause.branches {
            let mut actions = Vec::new();
            for action in &branch.actions {
                actions.push(match action {
                    WhenAction::Assign(target, value) => {
                        WhenAction::Assign(target.clone(), rewrite.expr(value)?)
                    }
                    WhenAction::Reinit(state, value) => {
                        WhenAction::Reinit(state.clone(), rewrite.expr(value)?)
                    }
                    WhenAction::Terminate(message) => WhenAction::Terminate(message.clone()),
                });
            }
            branches.push(WhenBranch {
                condition: rewrite.expr(&branch.condition)?,
                actions,
            });
        }
        when_clauses.push(WhenClause { branches });
    }
    let initial_equations: Vec<EquationItem> = model
        .initial_equations
        .iter()
        .map(|equation| {
            Ok(EquationItem {
                lhs: rewrite.expr(&equation.lhs)?,
                rhs: rewrite.expr(&equation.rhs)?,
            })
        })
        .collect::<Result<Vec<_>, SimError>>()?;
    let samples = rewrite.samples;
    let delayed = rewrite.delays;

    // 2. Which equations give a state its derivative, and which are
    // algebraic.
    let continuous: Vec<&str> = model
        .components
        .iter()
        .filter(|c| c.variability == Variability::Continuous && !discretes.contains(&c.name))
        .map(|c| c.name.as_str())
        .collect();
    let (mut state_rhs, mut algebraic_eqs) = split_equations(&equations, &continuous)?;

    // 3. What is left to solve for, and whether every name in the
    // equations is one the model knows.
    let (mut states, mut unknowns) = unknowns_of(&continuous, &state_rhs);
    check_references(&state_rhs, &algebraic_eqs, &continuous, &params, &discretes)?;
    if algebraic_eqs.len() != unknowns.len() {
        return err(format!(
            "unbalanced model: {} algebraic equation(s) for {} unknown(s) {:?}",
            algebraic_eqs.len(),
            unknowns.len(),
            unknowns
        ));
    }

    // 4. Structural analysis: match equations to unknowns. An equation
    // that cannot be matched is a DAE constraint, and index reduction
    // takes one step (Pantelides with dummy derivatives):
    //
    //   * differentiate the constraint symbolically and *add* the
    //     result as a new equation (the original stays, so it keeps
    //     holding exactly - no drift, no stabilization term);
    //   * demote one state appearing in the constraint to an algebraic
    //     unknown. Its former state equation becomes an ordinary
    //     equation and its derivative becomes a dummy unknown, which
    //     restores the equation/unknown balance.
    const MAX_INDEX_REDUCTIONS: usize = 16;

    // Augmenting-path maximum matching.
    fn try_match(
        eq: usize,
        eq_vars: &[Vec<usize>],
        matched_eq: &mut [Option<usize>],
        visited: &mut [bool],
    ) -> bool {
        for &v in &eq_vars[eq] {
            if !visited[v] {
                visited[v] = true;
                if matched_eq[v].is_none()
                    || try_match(matched_eq[v].unwrap(), eq_vars, matched_eq, visited)
                {
                    matched_eq[v] = Some(eq);
                    return true;
                }
            }
        }
        false
    }

    let mut dummies: HashMap<String, String> = HashMap::new();
    // States named in the right-hand side of an already-demoted state:
    // when `y` goes, its `der(y) = vy` marks `vy` as the companion the
    // next differentiation level should demote. Preferring companions
    // keeps each level of a chain of constraints demoting at its own
    // level - a velocity constraint takes a velocity, not a position
    // that happens to be numerically larger at this instant.
    let mut companions: Vec<String> = Vec::new();
    // Per reduction: the constraint residual, the demoted victim and
    // the states that were candidates - the runtime monitor watches the
    // victim's sensitivity against the alternatives and asks for a
    // re-selection while the numbers are still healthy.
    let mut selection_records: Vec<(Expr, String, Vec<String>)> = Vec::new();
    let mut reductions = 0usize;
    let (matched_eq, eq_vars, n_alg) = loop {
        let var_index: HashMap<&str, usize> = unknowns
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();
        let eq_vars: Vec<Vec<usize>> = algebraic_eqs
            .iter()
            .map(|(lhs, rhs)| {
                let mut refs = Vec::new();
                lhs.collect_refs(&mut refs);
                rhs.collect_refs(&mut refs);
                let mut vars: Vec<usize> = refs
                    .iter()
                    .filter_map(|r| var_index.get(r).copied())
                    .collect();
                vars.sort_unstable();
                vars.dedup();
                vars
            })
            .collect();

        let n_alg = unknowns.len();
        let mut matched_eq: Vec<Option<usize>> = vec![None; n_alg];
        let mut failed = None;
        for eq in 0..algebraic_eqs.len() {
            let mut visited = vec![false; n_alg];
            if !try_match(eq, &eq_vars, &mut matched_eq, &mut visited) {
                failed = Some(eq);
                break;
            }
        }
        let Some(eq) = failed else {
            break (matched_eq, eq_vars, n_alg);
        };

        let (lhs, rhs) = algebraic_eqs[eq].clone();
        if reductions >= MAX_INDEX_REDUCTIONS {
            return err(format!(
                "structurally singular model: equation {lhs:?} = {rhs:?} still cannot be matched after {MAX_INDEX_REDUCTIONS} index reductions"
            ));
        }
        reductions += 1;

        // Explicit definitions let differentiation reach through
        // algebraic unknowns.
        // Definitions to differentiate through, built to a fixpoint so
        // the graph is acyclic and grounds out in states and parameters.
        // Explicit forms (`u = 2*x`) come first; an unknown that only
        // appears inside a linear equation (`phi_rel = b - a` pins `a`)
        // is defined by solving for it. A definition is accepted only
        // once everything it references is itself grounded, which is
        // what keeps `a := b` and `b := a` from chasing each other.
        let alg_defs: HashMap<String, Expr> = {
            let mut candidates: Vec<(String, Expr)> = Vec::new();
            for (index, (l, r)) in algebraic_eqs.iter().enumerate() {
                // The equation under reduction cannot define its own
                // way out: `u = 3` must be read through `u = 2*x`.
                if index == eq {
                    continue;
                }
                if let (Expr::Ref(name), other) | (other, Expr::Ref(name)) = (l, r) {
                    if unknowns.contains(name) {
                        candidates.push((name.clone(), other.clone()));
                    }
                }
                let mut named = Vec::new();
                l.collect_refs(&mut named);
                r.collect_refs(&mut named);
                named.sort_unstable();
                named.dedup();
                for name in named {
                    if !unknowns.iter().any(|u| u == name) {
                        continue;
                    }
                    if let Some(solved) = solve_linear_for(l, r, name) {
                        candidates.push((name.to_string(), solved));
                    }
                }
            }
            let mut accepted: HashMap<String, Expr> = HashMap::new();
            loop {
                let mut progress = false;
                for (name, expr) in &candidates {
                    if accepted.contains_key(name) {
                        continue;
                    }
                    let mut refs = Vec::new();
                    expr.collect_refs(&mut refs);
                    let grounded = refs.iter().all(|r| {
                        *r != name
                            && (!unknowns.iter().any(|u| u == *r) || accepted.contains_key(*r))
                    });
                    if grounded {
                        accepted.insert(name.clone(), expr.clone());
                        progress = true;
                    }
                }
                if !progress {
                    break;
                }
            }
            accepted
        };

        let residual = Expr::Bin(
            oxidelica_parser::BinOp::Sub,
            Box::new(lhs.clone()),
            Box::new(rhs.clone()),
        );
        let derivative = match differentiate(
            &residual,
            &DiffTarget::Time {
                state_rhs: &state_rhs,
                params: &params,
                dummies: &dummies,
                alg_defs: &alg_defs,
            },
        ) {
            Ok(d) => simplify(&d),
            Err(reason) => {
                return err(format!(
                    "structurally singular model: equation {lhs:?} = {rhs:?} cannot be matched to an unknown ({reason})"
                ))
            }
        };

        // Demote a state the constraint actually constrains. The
        // choice is a pivot: the constraint has to *determine* the
        // demoted variable, so prefer the state with the largest
        // sensitivity at the start point. (The selection is static;
        // models that need it to change mid-run - a pendulum swinging
        // full circle - are the known limit of this implementation.)
        // Reachable states: the constraint may pin a state only
        // indirectly, through the definition of an algebraic unknown
        // (`u = 3` with `u = 2*x` constrains x).
        let mut reachable: Vec<String> = Vec::new();
        {
            let mut queue: Vec<String> = Vec::new();
            let mut direct = Vec::new();
            residual.collect_refs(&mut direct);
            queue.extend(direct.into_iter().map(str::to_string));
            let mut seen: Vec<String> = Vec::new();
            while let Some(name) = queue.pop() {
                if seen.contains(&name) {
                    continue;
                }
                seen.push(name.clone());
                if states.iter().any(|s| s == &name) {
                    reachable.push(name.clone());
                } else if let Some(definition) = alg_defs.get(&name) {
                    let mut more = Vec::new();
                    definition.collect_refs(&mut more);
                    queue.extend(more.into_iter().map(str::to_string));
                }
            }
        }
        let sensitivity = |name: &str| -> f64 {
            differentiate(&residual, &DiffTarget::Variable(name))
                .ok()
                .map(|d| simplify(&d))
                .and_then(|d| {
                    eval(
                        &d,
                        &EvalCtx {
                            vars: &start_env,
                            time: resume.as_ref().map_or(0.0, |point| point.time),
                        },
                    )
                    .ok()
                })
                .map(f64::abs)
                .unwrap_or(0.0)
        };
        // Companions of earlier victims first, by sensitivity; anything
        // else only when no companion is constrained here at all.
        let favoured: Vec<String> = reachable
            .iter()
            .filter(|name| companions.contains(name) && sensitivity(name) > 0.0)
            .cloned()
            .collect();
        let candidates = if favoured.is_empty() {
            reachable
        } else {
            favoured
        };
        // The runtime monitor compares the victim against exactly the
        // set the pivot weighed - alternatives of another derivative
        // level would make a healthy selection look wrong.
        let all_candidates = candidates.clone();
        let Some(victim) = candidates.into_iter().max_by(|a, b| {
            sensitivity(a)
                .partial_cmp(&sensitivity(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        }) else {
            return err(format!(
                "structurally singular model: equation {lhs:?} = {rhs:?} constrains no state, so index reduction cannot help"
            ));
        };

        selection_records.push((residual.clone(), victim.clone(), all_candidates));
        let dummy = format!("der({victim})");
        let victim_rhs = state_rhs
            .remove(&victim)
            .expect("a state has a defining derivative");
        {
            let mut named = Vec::new();
            victim_rhs.collect_refs(&mut named);
            companions.extend(
                named
                    .into_iter()
                    .filter(|name| states.iter().any(|s| s == name))
                    .map(str::to_string),
            );
        }
        states.retain(|s| s != &victim);
        unknowns.push(victim.clone());
        unknowns.push(dummy.clone());
        dummies.insert(victim.clone(), dummy.clone());
        // The former state equation `der(v) = rhs` now determines the
        // dummy, and the differentiated constraint joins the system.
        algebraic_eqs.push((Expr::Ref(dummy), victim_rhs));
        algebraic_eqs.push((derivative, Expr::Number(0.0)));
    };

    let mut matched_var: Vec<usize> = vec![0; n_alg];
    for (v, eq) in matched_eq.iter().enumerate() {
        matched_var[eq.expect("maximum matching covers every unknown")] = v;
    }

    // Initial values of the states that survived demotion.
    let ctx0 = EvalCtx {
        vars: &params,
        time: 0.0,
    };
    let mut initial = Vec::new();
    for s in &states {
        if let Some(value) = resumed(s) {
            initial.push(value);
            continue;
        }
        let comp = model
            .components
            .iter()
            .find(|c| &c.name == s)
            .expect("states come from declared components");
        let value = match &comp.start {
            Some(expr) => eval(expr, &ctx0).map_err(|e| SimError(format!("start of {s}: {e}")))?,
            None => 0.0,
        };
        initial.push(value);
    }

    // Kahn topological order over equations.
    let producer: Vec<usize> = {
        let mut p = vec![0; n_alg];
        for (eq, &v) in matched_var.iter().enumerate() {
            p[v] = eq;
        }
        p
    };
    let mut indegree = vec![0usize; n_alg];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n_alg];
    for eq in 0..n_alg {
        for &v in &eq_vars[eq] {
            if v != matched_var[eq] {
                dependents[producer[v]].push(eq);
                indegree[eq] += 1;
            }
        }
    }
    let mut queue: Vec<usize> = (0..n_alg).filter(|&e| indegree[e] == 0).collect();
    let mut emitted = Vec::new();
    let mut done = vec![false; n_alg];
    while let Some(eq) = queue.pop() {
        if done[eq] {
            continue;
        }
        done[eq] = true;
        emitted.push(eq);
        for &dep in &dependents[eq] {
            indegree[dep] = indegree[dep].saturating_sub(1);
            if indegree[dep] == 0 && !done[dep] {
                queue.push(dep);
            }
        }
    }

    let mentions = |expr: &Expr, name: &str| -> bool {
        let mut refs = Vec::new();
        expr.collect_refs(&mut refs);
        refs.contains(&name)
    };
    let mut ordered_algs: Vec<String> = Vec::new();
    let mut stages: Vec<PlanStage> = Vec::new();
    for &eq in &emitted {
        let var_name = unknowns[matched_var[eq]].clone();
        let index = ordered_algs.len();
        let (lhs, rhs) = &algebraic_eqs[eq];
        let stage = if matches!(lhs, Expr::Ref(n) if n == &var_name) && !mentions(rhs, &var_name) {
            PlanStage::Explicit {
                var: index,
                expr: rhs.clone(),
            }
        } else if matches!(rhs, Expr::Ref(n) if n == &var_name) && !mentions(lhs, &var_name) {
            PlanStage::Explicit {
                var: index,
                expr: lhs.clone(),
            }
        } else if let Some(expr) = solve_linear_for(lhs, rhs, &var_name) {
            // Linear in its unknown: solved symbolically, no iteration.
            PlanStage::Explicit { var: index, expr }
        } else {
            PlanStage::Implicit {
                vars: vec![index],
                torn: vec![index],
                inner: Vec::new(),
                residuals: vec![(lhs.clone(), rhs.clone())],
            }
        };
        ordered_algs.push(var_name);
        stages.push(stage);
    }

    // The cyclic remainder becomes one torn block: equations that can
    // be solved explicitly for their unknown are evaluated in
    // dependency order, and Newton iterates only on the tearing
    // variables needed to break the remaining cycles.
    let remainder: Vec<usize> = (0..n_alg).filter(|&e| !done[e]).collect();
    if !remainder.is_empty() {
        let base = ordered_algs.len();
        let mut index_of: HashMap<usize, usize> = HashMap::new();
        for (offset, &eq) in remainder.iter().enumerate() {
            let var = matched_var[eq];
            index_of.insert(var, base + offset);
            ordered_algs.push(unknowns[var].clone());
        }
        let vars: Vec<usize> = (base..ordered_algs.len()).collect();

        // Which equations can be solved explicitly for their unknown?
        let solvable: HashMap<usize, Expr> = remainder
            .iter()
            .filter_map(|&eq| {
                let name = &unknowns[matched_var[eq]];
                let (lhs, rhs) = &algebraic_eqs[eq];
                if matches!(lhs, Expr::Ref(n) if n == name) && !mentions(rhs, name) {
                    Some((eq, rhs.clone()))
                } else if matches!(rhs, Expr::Ref(n) if n == name) && !mentions(lhs, name) {
                    Some((eq, lhs.clone()))
                } else {
                    solve_linear_for(lhs, rhs, name).map(|expr| (eq, expr))
                }
            })
            .collect();

        // Equations that resist explicit solution force their unknown
        // into the tearing set; then tear greedily until the rest sorts
        // topologically, preferring the most-referenced unknowns.
        let mut torn_eqs: Vec<usize> = remainder
            .iter()
            .copied()
            .filter(|eq| !solvable.contains_key(eq))
            .collect();
        let uses: HashMap<usize, usize> = {
            let mut counts: HashMap<usize, usize> = HashMap::new();
            for &eq in &remainder {
                for &v in &eq_vars[eq] {
                    if index_of.contains_key(&v) {
                        *counts.entry(v).or_default() += 1;
                    }
                }
            }
            counts
        };
        let inner_order = loop {
            let torn_vars: Vec<usize> = torn_eqs.iter().map(|&eq| matched_var[eq]).collect();
            let pending: Vec<usize> = remainder
                .iter()
                .copied()
                .filter(|eq| !torn_eqs.contains(eq))
                .collect();
            // Topological pass over the untorn equations.
            let mut placed: Vec<usize> = Vec::new();
            let mut available = torn_vars.clone();
            let mut left = pending.clone();
            loop {
                let before = left.len();
                left.retain(|&eq| {
                    let ready = eq_vars[eq].iter().all(|v| {
                        !index_of.contains_key(v) || *v == matched_var[eq] || available.contains(v)
                    });
                    if ready {
                        placed.push(eq);
                        available.push(matched_var[eq]);
                        false
                    } else {
                        true
                    }
                });
                if left.is_empty() || left.len() == before {
                    break;
                }
            }
            if left.is_empty() {
                break placed;
            }
            // Still cyclic: tear the most-referenced unknown left.
            let victim = *left
                .iter()
                .max_by_key(|&&eq| uses.get(&matched_var[eq]).copied().unwrap_or(0))
                .expect("non-empty cycle");
            torn_eqs.push(victim);
        };

        let inner: Vec<(usize, Expr)> = inner_order
            .iter()
            .map(|eq| (index_of[&matched_var[*eq]], solvable[eq].clone()))
            .collect();
        let torn: Vec<usize> = torn_eqs
            .iter()
            .map(|&eq| index_of[&matched_var[eq]])
            .collect();
        let residuals: Vec<(Expr, Expr)> = torn_eqs
            .iter()
            .map(|&eq| algebraic_eqs[eq].clone())
            .collect();
        stages.push(PlanStage::Implicit {
            vars,
            torn,
            inner,
            residuals,
        });
    }

    let ctx = ctx0;
    let derivatives: Vec<Expr> = states.iter().map(|s| state_rhs[s].clone()).collect();
    let algebraic_start: Vec<f64> = ordered_algs
        .iter()
        .map(|name| {
            resumed(name)
                .or_else(|| {
                    model
                        .components
                        .iter()
                        .find(|c| &c.name == name)
                        .and_then(|c| c.start.as_ref())
                        .and_then(|expr| eval(expr, &ctx).ok())
                })
                .unwrap_or(0.0)
        })
        .collect();

    // Event indicators: one per relation anywhere in the model.
    let indicators: Vec<Expr> = {
        let mut out = Vec::new();
        let mut collect = |expr: &Expr| collect_relations(expr, &mut out);
        for (lhs, rhs) in &algebraic_eqs {
            collect(lhs);
            collect(rhs);
        }
        for expr in state_rhs.values() {
            collect(expr);
        }
        for clause in &when_clauses {
            for branch in &clause.branches {
                collect(&branch.condition);
            }
        }
        // The condition of a run-time `if` equation belongs here even
        // though the branch it chose may not mention it: the step has
        // to land on the switch, or the mode would change somewhere
        // inside a step that was taken under the old one.
        for conditions in &mode_conditions {
            for condition in conditions {
                collect(condition);
            }
        }
        out
    };

    let fixed_starts: Vec<(String, usize, f64)> = ordered_algs
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            let component = model.components.iter().find(|c| &c.name == name)?;
            if component.fixed != Some(true) {
                return None;
            }
            let value = eval(component.start.as_ref()?, &ctx).ok()?;
            Some((name.clone(), index, value))
        })
        .collect();

    let (jacobian_groups, jacobian_rows) =
        jacobian_structure(&states, &ordered_algs, &derivatives, &stages);
    // A discretized field gives a Jacobian that only reaches a step or
    // two from the diagonal. Eliminating such a matrix as a band costs
    // n*b^2 instead of n^3, which on a rod of a few hundred nodes is the
    // difference between seconds and milliseconds.
    let jacobian_band = jacobian_rows
        .iter()
        .enumerate()
        .flat_map(|(column, rows)| rows.iter().map(move |&row| row.abs_diff(column)))
        .max()
        .filter(|band| 4 * (band + 1) < states.len());

    let mut parameters: Vec<(String, f64)> = params.into_iter().collect();
    parameters.sort_by(|a, b| a.0.cmp(&b.0));

    // Everything a run reads gets a place in one array, and every
    // expression is resolved against it: after this point the solvers
    // never look a variable up by name.
    let mut table = SlotTable::new();
    for (name, value) in &parameters {
        table.constant(name, *value);
    }
    let state_slots: Vec<Slot> = states.iter().map(|name| table.slot(name)).collect();
    let algebraic_slots: Vec<Slot> = ordered_algs.iter().map(|name| table.slot(name)).collect();
    let discrete_slots: Vec<Slot> = discretes.iter().map(|name| table.slot(name)).collect();
    let pre_slots: Vec<Slot> = discretes
        .iter()
        .map(|name| table.slot(&format!("$pre.{name}")))
        .collect();
    let initial_slot = table.slot("$initial");
    let sample_slots: Vec<Slot> = (0..samples.len())
        .map(|index| table.slot(&format!("$sample{index}")))
        .collect();
    let delay_slots: Vec<Slot> = (0..delayed.len())
        .map(|index| table.slot(&format!("$delay{index}")))
        .collect();
    for (name, value) in discretes.iter().zip(&discrete_start) {
        let slot = table.slot(name);
        table.template[slot] = *value;
        let pre = table.slot(&format!("$pre.{name}"));
        table.template[pre] = *value;
    }

    let compiled_stages: Vec<AlgStage> = stages
        .iter()
        .map(|stage| match stage {
            PlanStage::Explicit { var, expr } => Ok(AlgStage::Explicit {
                var: *var,
                code: table.compile(expr)?,
            }),
            PlanStage::Implicit {
                vars,
                torn,
                inner,
                residuals,
            } => Ok(AlgStage::Implicit {
                vars: vars.clone(),
                torn: torn.clone(),
                inner: inner
                    .iter()
                    .map(|(var, expr)| Ok((*var, table.compile(expr)?)))
                    .collect::<Result<Vec<_>, SimError>>()?,
                residuals: residuals
                    .iter()
                    .map(|(lhs, rhs)| Ok((table.compile(lhs)?, table.compile(rhs)?)))
                    .collect::<Result<Vec<_>, SimError>>()?,
            }),
        })
        .collect::<Result<Vec<_>, SimError>>()?;

    let compiled_derivatives: Vec<Code> = derivatives
        .iter()
        .map(|expr| table.compile(expr))
        .collect::<Result<Vec<_>, SimError>>()?;
    let compiled_indicators: Vec<Code> = indicators
        .iter()
        .map(|expr| table.compile(expr))
        .collect::<Result<Vec<_>, SimError>>()?;

    let mut compiled_whens: Vec<CompiledWhen> = Vec::new();
    for clause in &when_clauses {
        let mut branches = Vec::new();
        for branch in &clause.branches {
            let mut actions = Vec::new();
            for action in &branch.actions {
                actions.push(match action {
                    WhenAction::Terminate(message) => CompiledAction::Terminate(message.clone()),
                    WhenAction::Reinit(name, value) => {
                        let Some(index) = states.iter().position(|state| state == name) else {
                            return err(format!(
                                "reinit({name}, ...): `{name}` is not a state of the flattened model"
                            ));
                        };
                        CompiledAction::Reinit(index, table.compile(value)?)
                    }
                    WhenAction::Assign(name, value) => {
                        let index = discretes
                            .iter()
                            .position(|discrete| discrete == name)
                            .expect("when targets were collected from these");
                        CompiledAction::Assign(index, table.compile(value)?)
                    }
                });
            }
            branches.push(CompiledBranch {
                condition: table.compile(&branch.condition)?,
                actions,
            });
        }
        compiled_whens.push(CompiledWhen { branches });
    }

    // The selection monitor: for every reduction, the sensitivity of
    // the constraint to the chosen victim and to each alternative, as
    // runnable code. Watching their ratio after every accepted step
    // asks for a re-selection while the current one is still sound.
    let mut selection_monitor: Vec<(Code, Vec<Code>)> = Vec::new();
    for (residual, victim, candidates) in &selection_records {
        let sensitivity_of = |name: &str| -> Result<Code, SimError> {
            let derivative =
                differentiate(residual, &DiffTarget::Variable(name)).map_err(SimError)?;
            table.compile(&simplify(&derivative))
        };
        let own = sensitivity_of(victim)?;
        let alternatives = candidates
            .iter()
            .filter(|name| *name != victim)
            .map(|name| sensitivity_of(name))
            .collect::<Result<Vec<_>, SimError>>()?;
        selection_monitor.push((own, alternatives));
    }

    let output_algebraics: Vec<(String, Slot)> = ordered_algs
        .iter()
        .zip(&algebraic_slots)
        .filter(|(name, _)| !dummies.values().any(|dummy| dummy == *name))
        .map(|(name, &slot)| (name.clone(), slot))
        .collect();

    let mut compiled = CompiledModel {
        name: model.name.clone(),
        parameters,
        states,
        initial,
        derivatives: compiled_derivatives,
        values_template: table.template.clone(),
        state_slots,
        algebraic_slots,
        discrete_slots,
        pre_slots,
        initial_slot,
        sample_slots,
        delays: delayed
            .iter()
            .zip(&delay_slots)
            .map(|((source, seconds), slot)| {
                Ok(CompiledDelay {
                    slot: *slot,
                    source: table.compile(source)?,
                    seconds: *seconds,
                })
            })
            .collect::<Result<Vec<_>, SimError>>()?,
        history: std::cell::RefCell::new(vec![Vec::new(); delayed.len()]),
        algebraics: ordered_algs,
        algebraic_start,
        fixed_starts,
        stages: compiled_stages,
        stop_time: model.experiment.stop_time.unwrap_or(1.0),
        step: model.experiment.interval.unwrap_or(1e-3),
        tolerance: model.experiment.tolerance.unwrap_or(1e-6),
        method: SolverMethod::default(),
        output_algebraics,
        start_time: resume.as_ref().map_or(0.0, |point| point.time),
        resume: resume.is_some(),
        reselectable: !dummies.is_empty(),
        selection_monitor,
        mode_monitor: mode_conditions
            .iter()
            .zip(&modes)
            .map(|(conditions, &taken)| {
                Ok((
                    conditions
                        .iter()
                        .map(|condition| table.compile(condition))
                        .collect::<Result<Vec<_>, SimError>>()?,
                    taken,
                ))
            })
            .collect::<Result<Vec<_>, SimError>>()?,
        asserts: model
            .asserts
            .iter()
            .map(|(condition, message)| Ok((table.compile(condition)?, message.clone())))
            .collect::<Result<Vec<_>, SimError>>()?,
        flat: model.clone(),
        jacobian_groups,
        jacobian_rows,
        jacobian_band,
        discretes,
        samples,
        when_clauses: compiled_whens,
        indicators: compiled_indicators,
    };
    // States the model pins down itself: `fixed = true` says the start
    // value is an initial condition, not a guess Newton may move.
    let fixed_states: Vec<bool> = compiled
        .states
        .iter()
        .map(|name| {
            model
                .components
                .iter()
                .find(|c| &c.name == name)
                .and_then(|c| c.fixed)
                .unwrap_or(false)
        })
        .collect();
    if resume.is_none() {
        compiled.solve_initialization(&initial_equations, &fixed_states, &derivatives, &table)?;
        compiled.check_block_regularity()?;
    }
    Ok(compiled)
}

/// Symbolic time-differentiation of an expression.
///
/// `d(state)/dt` substitutes the state's defining right-hand side;
/// parameters and literals differentiate to zero; `time` to one.
/// Differentiating through an algebraic unknown or a non-smooth
/// function is reported as an error (dummy derivatives arrive with the
/// full M3).
/// Collect `lhs - rhs` for every relation in an expression: these are
/// the functions whose sign changes mark an event.
pub(crate) fn collect_relations(expr: &Expr, out: &mut Vec<Expr>) {
    match expr {
        Expr::Rel(_, l, r) => {
            out.push(simplify(&Expr::Bin(
                oxidelica_parser::BinOp::Sub,
                Box::new((**l).clone()),
                Box::new((**r).clone()),
            )));
            collect_relations(l, out);
            collect_relations(r, out);
        }
        Expr::Bin(_, l, r) | Expr::And(l, r) | Expr::Or(l, r) => {
            collect_relations(l, out);
            collect_relations(r, out);
        }
        Expr::Neg(inner) | Expr::Not(inner) => collect_relations(inner, out),
        Expr::If(c, a, b) => {
            collect_relations(c, out);
            collect_relations(a, out);
            collect_relations(b, out);
        }
        Expr::Call(_, args) => args.iter().for_each(|a| collect_relations(a, out)),
        // Arrays are expanded into scalars before any of this runs.
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
        | Expr::Tuple(_)
        | Expr::Number(_)
        | Expr::Bool(_)
        | Expr::Ref(_)
        | Expr::Time => {}
    }
}

impl CompiledModel {
    /// Evaluate the plan once at the initial point, verifying that every
    /// implicit block is regular there. Catches models that are
    /// structurally fine but numerically underdetermined.
    pub(crate) fn check_block_regularity(&self) -> Result<(), SimError> {
        if self.fixed_starts.is_empty()
            && !self
                .stages
                .iter()
                .any(|s| matches!(s, AlgStage::Implicit { .. }))
        {
            return Ok(());
        }
        let mut values = self.values_template.clone();
        for (&slot, value) in self.state_slots.iter().zip(&self.initial) {
            values[slot] = *value;
        }
        let mut alg_guess = self.algebraic_start.clone();
        for stage in &self.stages {
            match stage {
                AlgStage::Explicit { var, code } => {
                    values[self.algebraic_slots[*var]] = code.run(&values, 0.0);
                }
                stage @ AlgStage::Implicit { .. } => {
                    self.solve_implicit_block(0.0, &mut values, stage, &mut alg_guess, true)?;
                }
            }
        }
        // A variable demoted by index reduction is solved from the
        // constraints; if it was declared `fixed = true`, that solution
        // has to agree with the declared initial condition.
        for (name, index, expected) in &self.fixed_starts {
            let actual = values[self.algebraic_slots[*index]];
            if (actual - expected).abs() > 1e-6 {
                return err(format!(
                    "initial value of `{name}` is fixed at {expected} but the constraints require {actual}"
                ));
            }
        }
        Ok(())
    }

    /// Solve the initialization problem: the state vector a run starts
    /// from is the one satisfying the `initial equation` section
    /// together with every state declared `fixed = true`.
    ///
    /// Without such a section nothing changes and the declared `start`
    /// values are the initial state. With one they become what they mean
    /// in the language once a model says something else about its start:
    /// the guess Newton begins from.
    pub(crate) fn solve_initialization(
        &mut self,
        initial_equations: &[EquationItem],
        fixed: &[bool],
        derivative_exprs: &[Expr],
        table: &SlotTable,
    ) -> Result<(), SimError> {
        if initial_equations.is_empty() {
            return Ok(());
        }
        let n = self.states.len();
        let pinned = fixed.iter().filter(|f| **f).count();
        if initial_equations.len() + pinned != n {
            return err(format!(
                "initialization is not square: {} initial equation(s) and {pinned} fixed start(s) for {n} state(s)",
                initial_equations.len()
            ));
        }

        // `der(x)` in an initial equation is the right-hand side the
        // model gives that state, so a steady start reads `der(x) = 0`.
        let substituted: Vec<(Code, Code)> = initial_equations
            .iter()
            .map(|equation| {
                let lhs = substitute_derivatives(&equation.lhs, &self.states, derivative_exprs)?;
                let rhs = substitute_derivatives(&equation.rhs, &self.states, derivative_exprs)?;
                Ok((table.compile(&lhs)?, table.compile(&rhs)?))
            })
            .collect::<Result<Vec<_>, SimError>>()?;

        let guess = self.initial.clone();
        let mut values = self.values_template.clone();
        let mut derivatives = Vec::new();
        let mut alg_guess = self.algebraic_start.clone();
        let mut y = guess.clone();

        let residual = |y: &[f64],
                        values: &mut Vec<f64>,
                        derivatives: &mut Vec<f64>,
                        alg_guess: &mut Vec<f64>|
         -> Result<Vec<f64>, SimError> {
            self.eval_point(0.0, y, values, derivatives, alg_guess)?;
            let mut out = Vec::with_capacity(n);
            for (lhs, rhs) in &substituted {
                out.push(lhs.run(values, 0.0) - rhs.run(values, 0.0));
            }
            for (index, pinned) in fixed.iter().enumerate() {
                if *pinned {
                    out.push(y[index] - guess[index]);
                }
            }
            Ok(out)
        };

        for _ in 0..50 {
            let f = residual(&y, &mut values, &mut derivatives, &mut alg_guess)?;
            let solved = f.iter().all(|r| r.abs() < 1e-10);
            let mut jac = vec![vec![0.0; n]; n];
            for j in 0..n {
                let h = 1e-7 * (1.0 + y[j].abs());
                let mut probe = y.clone();
                probe[j] += h;
                let fp = residual(&probe, &mut values, &mut derivatives, &mut alg_guess)?;
                for (i, row) in jac.iter_mut().enumerate() {
                    row[j] = (fp[i] - f[i]) / h;
                }
            }
            if solved {
                // Satisfied is not the same as determined: a singular
                // Jacobian means the equations leave a whole family of
                // starting points and this one is just the guess.
                let probe = vec![1.0; n];
                if solve_linear(&mut jac.clone(), &probe).is_none() {
                    return err(
                        "the initialization problem is singular: its equations do not pin the states down"
                            .to_string(),
                    );
                }
                self.initial = y;
                return Ok(());
            }
            let Some(step) = solve_linear(&mut jac, &f) else {
                return err(
                    "the initialization problem is singular: its equations do not pin the states down"
                        .to_string(),
                );
            };
            for j in 0..n {
                y[j] -= step[j];
            }
            if y.iter().any(|value| !value.is_finite()) {
                return err("initialization diverged".to_string());
            }
        }
        err("initialization did not converge in 50 Newton iterations".to_string())
    }
}

impl CompiledModel {
    /// How many evaluations one Jacobian costs: the number of column
    /// groups, which for a banded structure is far below the number of
    /// states. Reported by the CLI, and what the colouring is for.
    pub fn jacobian_cost(&self) -> usize {
        self.jacobian_groups.len()
    }

    /// Human-readable summary of the algebraic evaluation plan:
    /// one line per stage (explicit assignment or implicit block).
    pub fn plan_summary(&self) -> Vec<String> {
        self.stages
            .iter()
            .map(|stage| match stage {
                AlgStage::Explicit { var, .. } => {
                    format!("explicit: {}", self.algebraics[*var])
                }
                AlgStage::Implicit { vars, torn, .. } => {
                    let names: Vec<&str> =
                        vars.iter().map(|&i| self.algebraics[i].as_str()).collect();
                    format!(
                        "implicit block of {} (iterating on {}): {}",
                        vars.len(),
                        torn.len(),
                        names.join(", ")
                    )
                }
            })
            .collect()
    }
}
