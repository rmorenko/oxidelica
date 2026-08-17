//! oxidelica-sim — compiles a flat model into an executable form and
//! integrates it (adaptive Dormand-Prince by default, RK4 optional).
//!
//! State equations must isolate `der(x)` on one side; algebraic
//! equations are general — a bipartite matcher pairs them with
//! unknowns, explicit assignments evaluate directly and simultaneous
//! blocks are solved by Newton iteration.

#![deny(missing_docs)]

use oxidelica_parser::ast::{WhenAction, WhenClause};
use oxidelica_parser::{EquationItem, Expr, Model, Variability};
use std::collections::HashMap;
use std::fmt;

/// A compilation or simulation error.
#[derive(Debug)]
pub struct SimError(pub String);

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SimError {}

fn err<T>(message: impl Into<String>) -> Result<T, SimError> {
    Err(SimError(message.into()))
}

/// Integration method used by [`CompiledModel::simulate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SolverMethod {
    /// Adaptive Dormand-Prince 5(4) with error control (default).
    #[default]
    Dopri45,
    /// Classic fixed-step RK4.
    Rk4,
    /// Variable-order variable-step BDF: implicit, for stiff systems.
    Bdf,
}

impl SolverMethod {
    /// Parse a method name (CLI and IDE selectors).
    pub fn from_name(name: &str) -> Option<SolverMethod> {
        match name {
            "dopri" | "dopri45" => Some(SolverMethod::Dopri45),
            "rk4" => Some(SolverMethod::Rk4),
            "bdf" => Some(SolverMethod::Bdf),
            _ => None,
        }
    }

    /// Short name of the method.
    pub fn name(self) -> &'static str {
        match self {
            SolverMethod::Dopri45 => "dopri45",
            SolverMethod::Rk4 => "rk4",
            SolverMethod::Bdf => "bdf",
        }
    }
}

/// One step of the algebraic evaluation plan.
#[derive(Debug)]
enum AlgStage {
    /// An explicit assignment: `algebraics[var] := expr`.
    Explicit {
        /// Index of the assigned variable in `algebraics`.
        var: usize,
        /// The assigned expression.
        expr: Expr,
    },
    /// A simultaneous block. Newton iterates only on the torn
    /// unknowns; the rest are recovered by explicit assignments in
    /// dependency order, which keeps the Jacobian small.
    Implicit {
        /// All unknowns of the block (indices into `algebraics`).
        vars: Vec<usize>,
        /// Torn unknowns Newton actually iterates on.
        torn: Vec<usize>,
        /// Inner explicit assignments in evaluation order.
        inner: Vec<(usize, Expr)>,
        /// Residual equations matched to the torn unknowns.
        residuals: Vec<(Expr, Expr)>,
    },
}

/// A model reduced to "states plus ordered algebraic assignments".
#[derive(Debug)]
pub struct CompiledModel {
    /// Model name.
    pub name: String,
    /// Evaluated parameter and constant values, sorted by name.
    pub parameters: Vec<(String, f64)>,
    /// State names (defines the order of the state vector `y`).
    pub states: Vec<String>,
    /// Initial state values.
    pub initial: Vec<f64>,
    /// Right-hand side expression for each state.
    derivatives: Vec<Expr>,
    /// Algebraic variables in evaluation order.
    pub algebraics: Vec<String>,
    /// Initial Newton guesses for algebraic variables (start attributes).
    algebraic_start: Vec<f64>,
    /// Algebraic variables declared `fixed = true`: their start value is
    /// an initial condition, not a guess, so the solution must match it.
    fixed_starts: Vec<(String, f64)>,
    /// Evaluation plan: explicit assignments and implicit Newton blocks.
    stages: Vec<AlgStage>,
    /// Simulation end time.
    pub stop_time: f64,
    /// Integration step (fixed step for RK4, output grid for Dopri45).
    pub step: f64,
    /// Relative tolerance for the adaptive solver.
    pub tolerance: f64,
    /// Selected integration method.
    pub method: SolverMethod,
    /// `when` clauses; their actions fire on a false-to-true edge.
    when_clauses: Vec<WhenClause>,
    /// Event indicators: expressions whose sign change marks an event.
    /// Built from every relation in the model, so switching branches of
    /// an `if` expression are located exactly, not stepped over.
    indicators: Vec<Expr>,
}

impl CompiledModel {
    /// Evaluate the plan once at the initial point, verifying that every
    /// implicit block is regular there. Catches models that are
    /// structurally fine but numerically underdetermined.
    fn check_block_regularity(&self) -> Result<(), SimError> {
        if self.fixed_starts.is_empty()
            && !self
                .stages
                .iter()
                .any(|s| matches!(s, AlgStage::Implicit { .. }))
        {
            return Ok(());
        }
        let mut env: HashMap<String, f64> = self.parameters.iter().cloned().collect();
        for (name, value) in self.states.iter().zip(&self.initial) {
            env.insert(name.clone(), *value);
        }
        let mut alg_guess = self.algebraic_start.clone();
        for stage in &self.stages {
            match stage {
                AlgStage::Explicit { var, expr } => {
                    let ctx = EvalCtx {
                        vars: &env,
                        time: 0.0,
                    };
                    let Ok(value) = eval(expr, &ctx) else {
                        return Ok(()); // Not evaluable at t = 0; leave it to runtime.
                    };
                    env.insert(self.algebraics[*var].clone(), value);
                }
                stage @ AlgStage::Implicit { .. } => {
                    self.solve_implicit_block(0.0, &mut env, stage, &mut alg_guess, true)?;
                }
            }
        }
        // A variable demoted by index reduction is solved from the
        // constraints; if it was declared `fixed = true`, that solution
        // has to agree with the declared initial condition.
        for (name, expected) in &self.fixed_starts {
            if let Some(actual) = env.get(name) {
                if (actual - expected).abs() > 1e-6 {
                    return err(format!(
                        "initial value of `{name}` is fixed at {expected} but the constraints require {actual}"
                    ));
                }
            }
        }
        Ok(())
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

/// Compile a parsed flat model into an executable form.
pub fn compile(model: &Model) -> Result<CompiledModel, SimError> {
    // 1. Parameters and constants: multi-pass dependency evaluation.
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

    // 2. Split equations: explicit state derivatives vs general
    // algebraic equations (which need not be in assignment form).
    let continuous: Vec<&str> = model
        .components
        .iter()
        .filter(|c| c.variability == Variability::Continuous)
        .map(|c| c.name.as_str())
        .collect();

    let mut state_rhs: HashMap<String, Expr> = HashMap::new();
    let mut algebraic_eqs: Vec<(Expr, Expr)> = Vec::new();

    for EquationItem { lhs, rhs } in &model.equations {
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

    // 3. Unknowns and reference validation.
    let mut states: Vec<String> = continuous
        .iter()
        .filter(|n| state_rhs.contains_key(**n))
        .map(|n| n.to_string())
        .collect();
    let mut unknowns: Vec<String> = continuous
        .iter()
        .filter(|n| !state_rhs.contains_key(**n))
        .map(|n| n.to_string())
        .collect();

    {
        let mut refs = Vec::new();
        for expr in state_rhs.values() {
            expr.collect_refs(&mut refs);
        }
        for (lhs, rhs) in &algebraic_eqs {
            lhs.collect_refs(&mut refs);
            rhs.collect_refs(&mut refs);
        }
        if let Some(bad) = refs
            .iter()
            .find(|r| !continuous.contains(r) && !params.contains_key(**r))
        {
            return err(format!("unknown variable `{bad}` in equation"));
        }
    }

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

    // Start values of every continuous variable: used to pick the
    // demotion victim by numerical pivoting.
    let start_env: HashMap<String, f64> = {
        let mut env = params.clone();
        for component in &model.components {
            if component.variability == Variability::Continuous {
                let value = component
                    .start
                    .as_ref()
                    .and_then(|expr| {
                        eval(
                            expr,
                            &EvalCtx {
                                vars: &params,
                                time: 0.0,
                            },
                        )
                        .ok()
                    })
                    .unwrap_or(0.0);
                env.insert(component.name.clone(), value);
            }
        }
        env
    };

    let mut dummies: HashMap<String, String> = HashMap::new();
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
        let alg_defs: HashMap<String, Expr> = algebraic_eqs
            .iter()
            .enumerate()
            // The equation under reduction cannot define its own way
            // out: `u = 3` must be read through `u = 2*x`, not itself.
            .filter(|(index, _)| *index != eq)
            .map(|(_, pair)| pair)
            .filter_map(|(l, r)| match (l, r) {
                (Expr::Ref(name), other) | (other, Expr::Ref(name)) if unknowns.contains(name) => {
                    let mut refs = Vec::new();
                    other.collect_refs(&mut refs);
                    if refs.contains(&name.as_str()) {
                        None
                    } else {
                        Some((name.clone(), other.clone()))
                    }
                }
                _ => None,
            })
            .collect();

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
                            time: 0.0,
                        },
                    )
                    .ok()
                })
                .map(f64::abs)
                .unwrap_or(0.0)
        };
        let Some(victim) = reachable.into_iter().max_by(|a, b| {
            sensitivity(a)
                .partial_cmp(&sensitivity(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        }) else {
            return err(format!(
                "structurally singular model: equation {lhs:?} = {rhs:?} constrains no state, so index reduction cannot help"
            ));
        };

        let dummy = format!("der({victim})");
        let victim_rhs = state_rhs
            .remove(&victim)
            .expect("a state has a defining derivative");
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
    let mut stages: Vec<AlgStage> = Vec::new();
    for &eq in &emitted {
        let var_name = unknowns[matched_var[eq]].clone();
        let index = ordered_algs.len();
        let (lhs, rhs) = &algebraic_eqs[eq];
        let stage = if matches!(lhs, Expr::Ref(n) if n == &var_name) && !mentions(rhs, &var_name) {
            AlgStage::Explicit {
                var: index,
                expr: rhs.clone(),
            }
        } else if matches!(rhs, Expr::Ref(n) if n == &var_name) && !mentions(lhs, &var_name) {
            AlgStage::Explicit {
                var: index,
                expr: lhs.clone(),
            }
        } else if let Some(expr) = solve_linear_for(lhs, rhs, &var_name) {
            // Linear in its unknown: solved symbolically, no iteration.
            AlgStage::Explicit { var: index, expr }
        } else {
            AlgStage::Implicit {
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
        stages.push(AlgStage::Implicit {
            vars,
            torn,
            inner,
            residuals,
        });
    }

    let ctx = ctx0;
    let derivatives = states.iter().map(|s| state_rhs[s].clone()).collect();
    let algebraic_start: Vec<f64> = ordered_algs
        .iter()
        .map(|name| {
            model
                .components
                .iter()
                .find(|c| &c.name == name)
                .and_then(|c| c.start.as_ref())
                .and_then(|expr| eval(expr, &ctx).ok())
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
        for clause in &model.when_clauses {
            collect(&clause.condition);
        }
        out
    };

    let fixed_starts: Vec<(String, f64)> = ordered_algs
        .iter()
        .filter_map(|name| {
            let component = model.components.iter().find(|c| &c.name == name)?;
            if component.fixed != Some(true) {
                return None;
            }
            let value = eval(component.start.as_ref()?, &ctx).ok()?;
            Some((name.clone(), value))
        })
        .collect();

    let mut parameters: Vec<(String, f64)> = params.into_iter().collect();
    parameters.sort_by(|a, b| a.0.cmp(&b.0));

    let compiled = CompiledModel {
        name: model.name.clone(),
        parameters,
        states,
        initial,
        derivatives,
        algebraics: ordered_algs,
        algebraic_start,
        fixed_starts,
        stages,
        stop_time: model.experiment.stop_time.unwrap_or(1.0),
        step: model.experiment.interval.unwrap_or(1e-3),
        tolerance: model.experiment.tolerance.unwrap_or(1e-6),
        method: SolverMethod::default(),
        when_clauses: model.when_clauses.clone(),
        indicators,
    };
    compiled.check_block_regularity()?;
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
fn collect_relations(expr: &Expr, out: &mut Vec<Expr>) {
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
        Expr::Index(_, _)
        | Expr::Member(_, _)
        | Expr::Number(_)
        | Expr::Bool(_)
        | Expr::Ref(_)
        | Expr::Time => {}
    }
}

/// Constant folding and algebraic identities.
///
/// Symbolic derivatives are built structurally and carry a lot of dead
/// weight (`y * 0`, `x + 0`, `1 * u`). Folding it away matters twice
/// over: linearity detection asks whether a derivative still mentions
/// its variable, and differentiated constraints are evaluated at every
/// step.
fn simplify(expr: &Expr) -> Expr {
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
    }
}

/// Replace every reference to `var` with `value`.
fn substitute(expr: &Expr, var: &str, value: f64) -> Expr {
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
        Expr::Index(_, _) | Expr::Member(_, _) => expr.clone(),
    }
}

/// Solve `lhs = rhs` symbolically for `var` when the equation is linear
/// in it: with residual `r = a*var + b`, the solution is `-b/a`, where
/// `a` is the (var-free) derivative and `b` is `r` at `var = 0`.
fn solve_linear_for(lhs: &Expr, rhs: &Expr, var: &str) -> Option<Expr> {
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

enum DiffTarget<'a> {
    /// Differentiate with respect to time.
    Time {
        /// Defining right-hand sides of the states.
        state_rhs: &'a HashMap<String, Expr>,
        /// Known parameters (constant in time).
        params: &'a HashMap<String, f64>,
        /// Demoted states and the dummy derivative standing in for
        /// their `der(...)`.
        dummies: &'a HashMap<String, String>,
        /// Explicit definitions of algebraic unknowns, differentiated
        /// recursively when their derivative is needed.
        alg_defs: &'a HashMap<String, Expr>,
    },
    /// Differentiate with respect to one variable, all else constant.
    Variable(&'a str),
}

fn differentiate(expr: &Expr, target: &DiffTarget) -> Result<Expr, String> {
    differentiate_at(expr, target, 0)
}

/// Guards against cyclic algebraic definitions while differentiating.
const MAX_DIFF_DEPTH: usize = 32;

fn differentiate_at(expr: &Expr, target: &DiffTarget, depth: usize) -> Result<Expr, String> {
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

// --- expression evaluation ---

struct EvalCtx<'a> {
    vars: &'a HashMap<String, f64>,
    time: f64,
}

/// Booleans are represented as 1.0 / 0.0 (proper typing is an M1+ task).
fn eval(expr: &Expr, ctx: &EvalCtx) -> Result<f64, SimError> {
    use oxidelica_parser::ast::RelOp;
    use oxidelica_parser::BinOp::*;
    Ok(match expr {
        Expr::Number(n) => *n,
        Expr::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Expr::Time => ctx.time,
        Expr::Ref(name) => match ctx.vars.get(name) {
            Some(v) => *v,
            None => return err(format!("unknown variable `{name}`")),
        },
        Expr::Neg(inner) => -eval(inner, ctx)?,
        Expr::Not(inner) => {
            if eval(inner, ctx)? == 0.0 {
                1.0
            } else {
                0.0
            }
        }
        Expr::And(l, r) => {
            if eval(l, ctx)? != 0.0 && eval(r, ctx)? != 0.0 {
                1.0
            } else {
                0.0
            }
        }
        Expr::Or(l, r) => {
            if eval(l, ctx)? != 0.0 || eval(r, ctx)? != 0.0 {
                1.0
            } else {
                0.0
            }
        }
        Expr::Rel(op, l, r) => {
            let a = eval(l, ctx)?;
            let b = eval(r, ctx)?;
            let holds = match op {
                RelOp::Lt => a < b,
                RelOp::Le => a <= b,
                RelOp::Gt => a > b,
                RelOp::Ge => a >= b,
                RelOp::Eq => a == b,
                RelOp::Ne => a != b,
            };
            if holds {
                1.0
            } else {
                0.0
            }
        }
        Expr::If(cond, then_branch, else_branch) => {
            if eval(cond, ctx)? != 0.0 {
                eval(then_branch, ctx)?
            } else {
                eval(else_branch, ctx)?
            }
        }
        Expr::Bin(op, l, r) => {
            let a = eval(l, ctx)?;
            let b = eval(r, ctx)?;
            match op {
                Add => a + b,
                Sub => a - b,
                Mul => a * b,
                Div => a / b,
                Pow => a.powf(b),
            }
        }
        Expr::Index(base, _) | Expr::Member(base, _) => {
            return err(format!(
                "unresolved array subscript on {base:?}: flattening should have expanded it"
            ))
        }
        Expr::Call(name, args) => {
            let vals: Result<Vec<f64>, SimError> = args.iter().map(|a| eval(a, ctx)).collect();
            let vals = vals?;
            let arity = |n: usize| -> Result<(), SimError> {
                if vals.len() == n {
                    Ok(())
                } else {
                    err(format!(
                        "{name}: expects {n} argument(s), got {}",
                        vals.len()
                    ))
                }
            };
            match name.as_str() {
                "der" => return err("der() outside a state equation is not supported in M0"),
                "sin" => {
                    arity(1)?;
                    vals[0].sin()
                }
                "cos" => {
                    arity(1)?;
                    vals[0].cos()
                }
                "tan" => {
                    arity(1)?;
                    vals[0].tan()
                }
                "asin" => {
                    arity(1)?;
                    vals[0].asin()
                }
                "acos" => {
                    arity(1)?;
                    vals[0].acos()
                }
                "atan" => {
                    arity(1)?;
                    vals[0].atan()
                }
                "atan2" => {
                    arity(2)?;
                    vals[0].atan2(vals[1])
                }
                "sinh" => {
                    arity(1)?;
                    vals[0].sinh()
                }
                "cosh" => {
                    arity(1)?;
                    vals[0].cosh()
                }
                "tanh" => {
                    arity(1)?;
                    vals[0].tanh()
                }
                "exp" => {
                    arity(1)?;
                    vals[0].exp()
                }
                "log" => {
                    arity(1)?;
                    vals[0].ln()
                }
                "log10" => {
                    arity(1)?;
                    vals[0].log10()
                }
                "sqrt" => {
                    arity(1)?;
                    vals[0].sqrt()
                }
                "abs" => {
                    arity(1)?;
                    vals[0].abs()
                }
                "sign" => {
                    arity(1)?;
                    vals[0].signum()
                }
                "min" => {
                    arity(2)?;
                    vals[0].min(vals[1])
                }
                "max" => {
                    arity(2)?;
                    vals[0].max(vals[1])
                }
                other => return err(format!("unknown function `{other}`")),
            }
        }
    })
}

/// Outcome of handling an event.
#[derive(Default)]
struct EventOutcome {
    /// Set when a `terminate(...)` fired.
    terminated: Option<String>,
    /// Whether any state was reinitialized (the integrator restarts).
    reinitialized: bool,
}

impl CompiledModel {
    /// Values of the event indicators at a point already evaluated into
    /// `env`.
    fn indicator_values(&self, t: f64, env: &HashMap<String, f64>) -> Result<Vec<f64>, SimError> {
        self.indicators
            .iter()
            .map(|expr| eval(expr, &EvalCtx { vars: env, time: t }))
            .collect()
    }

    /// Current truth value of every `when` condition.
    fn when_conditions(&self, t: f64, env: &HashMap<String, f64>) -> Result<Vec<bool>, SimError> {
        self.when_clauses
            .iter()
            .map(|clause| Ok(eval(&clause.condition, &EvalCtx { vars: env, time: t })? != 0.0))
            .collect()
    }

    /// Fire the `when` clauses whose condition just became true,
    /// applying their actions to the state vector.
    fn handle_event(
        &self,
        t: f64,
        env: &HashMap<String, f64>,
        y: &mut [f64],
        previous: &mut Vec<bool>,
    ) -> Result<EventOutcome, SimError> {
        let now = self.when_conditions(t, env)?;
        let mut outcome = EventOutcome::default();
        for (index, clause) in self.when_clauses.iter().enumerate() {
            let was = previous.get(index).copied().unwrap_or(false);
            if !now[index] || was {
                continue;
            }
            for action in &clause.actions {
                match action {
                    WhenAction::Terminate(message) => {
                        outcome.terminated = Some(format!("terminated at t = {t:.6}: {message}"));
                    }
                    WhenAction::Reinit(name, value) => {
                        let Some(slot) = self.states.iter().position(|s| s == name) else {
                            return err(format!(
                                "reinit({name}, ...): `{name}` is not a state of the flattened model"
                            ));
                        };
                        y[slot] = eval(value, &EvalCtx { vars: env, time: t })?;
                        outcome.reinitialized = true;
                    }
                }
            }
        }
        *previous = now;
        Ok(outcome)
    }
}

/// Derivatives of the Lagrange basis polynomials at the first node.
///
/// For nodes `[t0, t1, ...]` the result `c` satisfies
/// `P'(t0) = sum_j c[j] * y_j` for the interpolant `P` through them —
/// exactly the coefficients of a non-uniform BDF formula.
fn lagrange_derivative_coefficients(nodes: &[f64]) -> Vec<f64> {
    let count = nodes.len();
    let mut coefficients = vec![0.0; count];
    // j = 0: sum of reciprocal distances to the other nodes.
    coefficients[0] = nodes[1..].iter().map(|t| 1.0 / (nodes[0] - t)).sum();
    for j in 1..count {
        let mut numerator = 1.0;
        for (m, node) in nodes.iter().enumerate() {
            if m != j && m != 0 {
                numerator *= nodes[0] - node;
            }
        }
        let mut denominator = 1.0;
        for (m, node) in nodes.iter().enumerate() {
            if m != j {
                denominator *= nodes[j] - node;
            }
        }
        coefficients[j] = numerator / denominator;
    }
    coefficients
}

/// Value at `at` of the Lagrange interpolant through `nodes` for
/// component `i` of the stored vectors.
fn lagrange_value(nodes: &[f64], values: &[&[f64]], i: usize, at: f64) -> f64 {
    let mut sum = 0.0;
    for (j, &node) in nodes.iter().enumerate() {
        let mut basis = 1.0;
        for (m, &other) in nodes.iter().enumerate() {
            if m != j {
                basis *= (at - other) / (node - other);
            }
        }
        sum += basis * values[j][i];
    }
    sum
}

/// Extrapolate the history polynomial (component `i`) to `at`.
fn lagrange_extrapolate(nodes: &[f64], values: &[Vec<f64>], i: usize, at: f64) -> f64 {
    let borrowed: Vec<&[f64]> = values.iter().map(|v| v.as_slice()).collect();
    lagrange_value(nodes, &borrowed, i, at)
}

/// Solve `a * x = b` in place by Gaussian elimination with partial
/// pivoting; `None` on a (numerically) singular matrix.
fn solve_linear(a: &mut [Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    let n = b.len();
    let mut x = b.to_vec();
    for col in 0..n {
        let pivot_row = (col..n).max_by(|&r1, &r2| {
            a[r1][col]
                .abs()
                .partial_cmp(&a[r2][col].abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        if a[pivot_row][col].abs() < 1e-14 {
            return None;
        }
        a.swap(col, pivot_row);
        x.swap(col, pivot_row);
        for row in (col + 1)..n {
            let factor = a[row][col] / a[col][col];
            let (upper, lower) = a.split_at_mut(row);
            for (k, value) in lower[0].iter_mut().enumerate().take(n).skip(col) {
                *value -= factor * upper[col][k];
            }
            x[row] -= factor * x[col];
        }
    }
    for col in (0..n).rev() {
        for k in (col + 1)..n {
            let prev = x[k];
            x[col] -= a[col][k] * prev;
        }
        x[col] /= a[col][col];
    }
    Some(x)
}

// --- integration ---

/// Simulation output: a table of time, states and algebraic variables.
#[derive(Debug)]
pub struct SimResult {
    /// Column headers: time, states, algebraics.
    pub columns: Vec<String>,
    /// One row per output point.
    pub rows: Vec<Vec<f64>>,
    /// Parameter values of the run, so consumers (the 3D view) can read
    /// sizes and colours that never vary in time.
    pub parameters: Vec<(String, f64)>,
    /// Set when a `when ... then terminate(...)` clause fired; contains
    /// a human-readable "terminated at t = ...: message" line.
    pub terminated: Option<String>,
}

impl SimResult {
    /// Render the result as CSV text.
    pub fn to_csv(&self) -> String {
        let mut out = self.columns.join(",");
        out.push('\n');
        for row in &self.rows {
            let line: Vec<String> = row.iter().map(|v| format!("{v:.9}")).collect();
            out.push_str(&line.join(","));
            out.push('\n');
        }
        out
    }
}

impl CompiledModel {
    /// Evaluate algebraic variables and derivatives at point (t, y).
    /// `env` is reused between calls to avoid per-step allocation.
    fn eval_point(
        &self,
        t: f64,
        y: &[f64],
        env: &mut HashMap<String, f64>,
        derivatives_out: &mut Vec<f64>,
        alg_guess: &mut [f64],
    ) -> Result<(), SimError> {
        env.clear();
        for (name, value) in &self.parameters {
            env.insert(name.clone(), *value);
        }
        for (name, value) in self.states.iter().zip(y) {
            env.insert(name.clone(), *value);
        }
        for stage in &self.stages {
            match stage {
                AlgStage::Explicit { var, expr } => {
                    let value = eval(expr, &EvalCtx { vars: env, time: t })?;
                    env.insert(self.algebraics[*var].clone(), value);
                }
                stage @ AlgStage::Implicit { .. } => {
                    self.solve_implicit_block(t, env, stage, alg_guess, false)?;
                }
            }
        }
        derivatives_out.clear();
        for expr in &self.derivatives {
            derivatives_out.push(eval(expr, &EvalCtx { vars: env, time: t })?);
        }
        Ok(())
    }

    /// Solve one implicit algebraic block by damped-free Newton
    /// iteration with a finite-difference Jacobian. `alg_guess` supplies
    /// warm starts (the previous evaluation point) and receives the
    /// solution.
    fn solve_implicit_block(
        &self,
        t: f64,
        env: &mut HashMap<String, f64>,
        stage: &AlgStage,
        alg_guess: &mut [f64],
        validate: bool,
    ) -> Result<(), SimError> {
        let AlgStage::Implicit {
            torn: block,
            inner,
            residuals,
            ..
        } = stage
        else {
            return Ok(());
        };
        let n = block.len();
        let mut v: Vec<f64> = block.iter().map(|&i| alg_guess[i]).collect();

        let residual = |env: &mut HashMap<String, f64>, v: &[f64]| -> Result<Vec<f64>, SimError> {
            for (j, &index) in block.iter().enumerate() {
                env.insert(self.algebraics[index].clone(), v[j]);
            }
            // Torn values fixed: the inner unknowns follow explicitly.
            for (var, expr) in inner {
                let value = eval(expr, &EvalCtx { vars: env, time: t })?;
                env.insert(self.algebraics[*var].clone(), value);
            }
            let mut f = Vec::with_capacity(n);
            for (lhs, rhs) in residuals {
                let l = eval(lhs, &EvalCtx { vars: env, time: t })?;
                let r = eval(rhs, &EvalCtx { vars: env, time: t })?;
                f.push(l - r);
            }
            Ok(f)
        };
        let block_names =
            || -> Vec<&str> { block.iter().map(|&i| self.algebraics[i].as_str()).collect() };

        for _ in 0..50 {
            let f = residual(env, &v)?;
            let converged = f
                .iter()
                .zip(&v)
                .all(|(fi, vi)| fi.abs() <= 1e-10 * (1.0 + vi.abs()));
            if converged {
                for (j, &index) in block.iter().enumerate() {
                    alg_guess[index] = v[j];
                }
                if validate {
                    // A converged block still has to be *determined*:
                    // a singular Jacobian means the loop admits a whole
                    // family of solutions, and the one we landed on is
                    // an artifact of the initial guess.
                    let mut jac = vec![vec![0.0f64; n]; n];
                    for j in 0..n {
                        let h = 1e-8 * (1.0 + v[j].abs());
                        let mut perturbed = v.clone();
                        perturbed[j] += h;
                        let fp = residual(env, &perturbed)?;
                        for (i, row) in jac.iter_mut().enumerate() {
                            row[j] = (fp[i] - f[i]) / h;
                        }
                    }
                    let probe = vec![1.0; n];
                    if solve_linear(&mut jac, &probe).is_none() {
                        return err(format!(
                            "underdetermined algebraic loop {:?}: the equations do not determine a unique solution",
                            block_names()
                        ));
                    }
                }
                return Ok(());
            }
            // Finite-difference Jacobian of the residual.
            let mut jac = vec![vec![0.0f64; n]; n];
            for j in 0..n {
                let h = 1e-8 * (1.0 + v[j].abs());
                let mut perturbed = v.clone();
                perturbed[j] += h;
                let fp = residual(env, &perturbed)?;
                for (i, row) in jac.iter_mut().enumerate() {
                    row[j] = (fp[i] - f[i]) / h;
                }
            }
            let Some(dv) = solve_linear(&mut jac, &f) else {
                return err(format!(
                    "singular Jacobian in algebraic loop {:?}",
                    block_names()
                ));
            };
            for j in 0..n {
                v[j] -= dv[j];
            }
            if v.iter().any(|value| !value.is_finite()) {
                return err(format!("algebraic loop diverged: {:?}", block_names()));
            }
        }
        err(format!(
            "algebraic loop did not converge in 50 Newton iterations: {:?}",
            block_names()
        ))
    }

    /// Integrate over `[0, stop_time]` with the selected method.
    pub fn simulate(&self) -> Result<SimResult, SimError> {
        match self.method {
            SolverMethod::Dopri45 => self.simulate_adaptive(),
            SolverMethod::Rk4 => self.simulate_rk4(),
            SolverMethod::Bdf => self.simulate_bdf(),
        }
    }

    /// Adaptive Dormand-Prince 5(4) integration with dense output on
    /// the `step` grid. The step size shrinks automatically near sharp
    /// dynamics (close encounters, kinks) and grows on smooth stretches;
    /// a persistent step-size underflow is reported as a probable
    /// singularity instead of returning garbage.
    pub fn simulate_adaptive(&self) -> Result<SimResult, SimError> {
        // Dormand-Prince Butcher tableau.
        const C: [f64; 7] = [0.0, 0.2, 0.3, 0.8, 8.0 / 9.0, 1.0, 1.0];
        const A: [[f64; 6]; 7] = [
            [0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            [0.2, 0.0, 0.0, 0.0, 0.0, 0.0],
            [3.0 / 40.0, 9.0 / 40.0, 0.0, 0.0, 0.0, 0.0],
            [44.0 / 45.0, -56.0 / 15.0, 32.0 / 9.0, 0.0, 0.0, 0.0],
            [
                19372.0 / 6561.0,
                -25360.0 / 2187.0,
                64448.0 / 6561.0,
                -212.0 / 729.0,
                0.0,
                0.0,
            ],
            [
                9017.0 / 3168.0,
                -355.0 / 33.0,
                46732.0 / 5247.0,
                49.0 / 176.0,
                -5103.0 / 18656.0,
                0.0,
            ],
            [
                35.0 / 384.0,
                0.0,
                500.0 / 1113.0,
                125.0 / 192.0,
                -2187.0 / 6784.0,
                11.0 / 84.0,
            ],
        ];
        /// 5th-order weights (the last tableau row, FSAL).
        const B5: [f64; 7] = [
            35.0 / 384.0,
            0.0,
            500.0 / 1113.0,
            125.0 / 192.0,
            -2187.0 / 6784.0,
            11.0 / 84.0,
            0.0,
        ];
        /// 4th-order weights of the embedded method.
        const B4: [f64; 7] = [
            5179.0 / 57600.0,
            0.0,
            7571.0 / 16695.0,
            393.0 / 640.0,
            -92097.0 / 339200.0,
            187.0 / 2100.0,
            1.0 / 40.0,
        ];

        let n = self.states.len();
        let stop = self.stop_time;
        let out_step = self.step.max(1e-12);
        let rtol = self.tolerance;
        let atol = self.tolerance * 1e-3;

        let mut env = HashMap::new();
        let mut columns = vec!["time".to_string()];
        columns.extend(self.states.iter().cloned());
        columns.extend(self.algebraics.iter().cloned());
        let mut rows: Vec<Vec<f64>> = Vec::new();
        let mut derivatives_scratch = Vec::new();

        let mut record = |t: f64,
                          y: &[f64],
                          env: &mut HashMap<String, f64>,
                          k: &mut Vec<f64>,
                          alg_guess: &mut [f64]|
         -> Result<(), SimError> {
            self.eval_point(t, y, env, k, alg_guess)?;
            let mut row = Vec::with_capacity(1 + n + self.algebraics.len());
            row.push(t);
            row.extend_from_slice(y);
            for name in &self.algebraics {
                row.push(env[name]);
            }
            rows.push(row);
            Ok(())
        };

        let mut y = self.initial.clone();
        let mut alg_guess = self.algebraic_start.clone();
        let mut last_out_t = 0.0f64;
        let mut terminated: Option<String> = None;
        // Modelica treats `when` conditions as false before the start,
        // so one that already holds at t = 0 fires immediately.
        let mut when_prev = vec![false; self.when_clauses.len()];
        record(0.0, &y, &mut env, &mut derivatives_scratch, &mut alg_guess)?;
        let start_event = self.handle_event(0.0, &env, &mut y, &mut when_prev)?;
        let mut indicators_prev = self.indicator_values(0.0, &env)?;
        if let Some(message) = start_event.terminated {
            return Ok(SimResult {
                columns,
                rows,
                parameters: self.parameters.clone(),
                terminated: Some(message),
            });
        }

        // Pure-algebraic models: no ODE to integrate, only the grid.
        if n == 0 {
            let mut out_i = 1usize;
            loop {
                let t = out_i as f64 * out_step;
                if t > stop + 1e-12 {
                    break;
                }
                record(t, &y, &mut env, &mut derivatives_scratch, &mut alg_guess)?;
                last_out_t = t;
                out_i += 1;
                terminated = self
                    .handle_event(t, &env, &mut y, &mut when_prev)?
                    .terminated;
                if terminated.is_some() {
                    break;
                }
            }
            if terminated.is_none() && last_out_t < stop - 1e-12 {
                record(stop, &y, &mut env, &mut derivatives_scratch, &mut alg_guess)?;
            }
            return Ok(SimResult {
                columns,
                rows,
                parameters: self.parameters.clone(),
                terminated,
            });
        }

        let mut k: Vec<Vec<f64>> = vec![vec![0.0; n]; 7];
        let mut stage = vec![0.0; n];
        let mut y5 = vec![0.0; n];
        let mut interp = vec![0.0; n];
        let mut t = 0.0f64;
        let mut h = out_step.min(stop).max(1e-9);
        let mut out_i = 1usize;
        let mut evals: u64 = 0;

        self.eval_point(t, &y, &mut env, &mut k[0], &mut alg_guess)?;

        while t < stop - 1e-12 {
            h = h.min(stop - t);
            // Stages 2..7 (stage 1 is FSAL from the previous step).
            for s in 1..7 {
                for j in 0..n {
                    let mut acc = 0.0;
                    for (q, k_q) in k.iter().enumerate().take(s) {
                        acc += A[s][q] * k_q[j];
                    }
                    stage[j] = y[j] + h * acc;
                }
                let (head, tail) = k.split_at_mut(s);
                let _ = head;
                self.eval_point(t + C[s] * h, &stage, &mut env, &mut tail[0], &mut alg_guess)?;
            }
            evals += 6;
            if evals > 20_000_000 {
                return err(format!(
                    "solver exceeded the evaluation budget at t = {t:.6}"
                ));
            }

            // 5th-order solution and the embedded error estimate.
            let mut err_norm = 0.0f64;
            for j in 0..n {
                let mut acc5 = 0.0;
                let mut acc4 = 0.0;
                for q in 0..7 {
                    acc5 += B5[q] * k[q][j];
                    acc4 += B4[q] * k[q][j];
                }
                let y5j = y[j] + h * acc5;
                let y4j = y[j] + h * acc4;
                let scale = atol + rtol * y[j].abs().max(y5j.abs());
                let e = (y5j - y4j) / scale;
                err_norm += e * e;
                y5[j] = y5j;
            }
            err_norm = (err_norm / n as f64).sqrt();

            let accepted = err_norm.is_finite() && err_norm <= 1.0;
            if accepted {
                // FSAL: the derivative at t+h.
                self.eval_point(t + h, &y5, &mut env, &mut k[6], &mut alg_guess)?;
                evals += 1;

                // Cubic Hermite interpolation across the accepted step.
                let interpolate = |theta: f64, out: &mut Vec<f64>, y: &[f64], k: &[Vec<f64>]| {
                    out.clear();
                    for j in 0..n {
                        let (y0, y1) = (y[j], y5[j]);
                        let (f0, f1) = (k[0][j], k[6][j]);
                        out.push(
                            (1.0 - theta) * y0
                                + theta * y1
                                + theta
                                    * (theta - 1.0)
                                    * ((1.0 - 2.0 * theta) * (y1 - y0)
                                        + (theta - 1.0) * h * f0
                                        + theta * h * f1),
                        );
                    }
                };

                // Did any event indicator change sign across the step?
                // An indicator sitting on zero (we just handled an event
                // there) is re-baselined instead of firing again.
                let indicators_new = self.indicator_values(t + h, &env)?;
                let mut event_theta: Option<f64> = None;
                for (index, (&before, &after)) in
                    indicators_prev.iter().zip(&indicators_new).enumerate()
                {
                    if before.abs() <= 1e-12 || before * after >= 0.0 {
                        continue;
                    }
                    let (mut lo, mut hi) = (0.0f64, 1.0f64);
                    for _ in 0..40 {
                        let mid = 0.5 * (lo + hi);
                        interpolate(mid, &mut interp, &y, &k);
                        self.eval_point(
                            t + mid * h,
                            &interp,
                            &mut env,
                            &mut derivatives_scratch,
                            &mut alg_guess,
                        )?;
                        if before * self.indicator_values(t + mid * h, &env)?[index] <= 0.0 {
                            hi = mid;
                        } else {
                            lo = mid;
                        }
                    }
                    // A hair past the crossing, so the relation has
                    // definitely switched when the condition is tested.
                    let crossed = (hi + 1e-9).min(1.0);
                    event_theta = Some(event_theta.map_or(crossed, |c: f64| c.min(crossed)));
                }

                if let Some(theta) = event_theta {
                    let t_event = t + theta * h;
                    // Grid points before the event still come from the
                    // smooth part of the step.
                    loop {
                        let out_t = out_i as f64 * out_step;
                        if out_t > t_event + 1e-12 || out_t > stop + 1e-12 {
                            break;
                        }
                        interpolate(((out_t - t) / h).clamp(0.0, 1.0), &mut interp, &y, &k);
                        record(
                            out_t,
                            &interp,
                            &mut env,
                            &mut derivatives_scratch,
                            &mut alg_guess,
                        )?;
                        last_out_t = out_t;
                        out_i += 1;
                    }
                    interpolate(theta, &mut interp, &y, &k);
                    y.copy_from_slice(&interp);
                    self.eval_point(
                        t_event,
                        &y,
                        &mut env,
                        &mut derivatives_scratch,
                        &mut alg_guess,
                    )?;
                    let outcome = self.handle_event(t_event, &env, &mut y, &mut when_prev)?;
                    t = t_event;
                    self.eval_point(t, &y, &mut env, &mut k[0], &mut alg_guess)?;
                    indicators_prev = self.indicator_values(t, &env)?;
                    if let Some(message) = outcome.terminated {
                        record(t, &y, &mut env, &mut derivatives_scratch, &mut alg_guess)?;
                        terminated = Some(message);
                        break;
                    }
                    if outcome.reinitialized {
                        record(t, &y, &mut env, &mut derivatives_scratch, &mut alg_guess)?;
                    }
                    h = (h * theta.max(0.1)).max(1e-12);
                    continue;
                }
                indicators_prev = indicators_new;

                // Dense output on the grid via cubic Hermite interpolation.
                loop {
                    let out_t = out_i as f64 * out_step;
                    if out_t > t + h + 1e-12 || out_t > stop + 1e-12 {
                        break;
                    }
                    interpolate(((out_t - t) / h).clamp(0.0, 1.0), &mut interp, &y, &k);
                    record(
                        out_t,
                        &interp,
                        &mut env,
                        &mut derivatives_scratch,
                        &mut alg_guess,
                    )?;
                    last_out_t = out_t;
                    out_i += 1;
                }
                t += h;
                y.copy_from_slice(&y5);
                k.swap(0, 6);
            }

            let factor = if !err_norm.is_finite() {
                0.2
            } else if err_norm == 0.0 {
                5.0
            } else {
                (0.9 * err_norm.powf(-0.2)).clamp(0.2, 5.0)
            };
            h *= factor;
            if h < stop * 1e-14 || h < 1e-300 {
                return err(format!(
                    "step size underflow at t = {t:.6}: probable singularity"
                ));
            }
        }
        if terminated.is_none() && last_out_t < stop - 1e-12 {
            record(stop, &y, &mut env, &mut derivatives_scratch, &mut alg_guess)?;
            terminated = self
                .handle_event(stop, &env, &mut y, &mut when_prev)?
                .terminated;
        }
        Ok(SimResult {
            columns,
            rows,
            parameters: self.parameters.clone(),
            terminated,
        })
    }

    /// Finite-difference Jacobian `df/dy` of the state right-hand side
    /// at `(t, y)`. Algebraic warm starts are kept on a scratch copy so
    /// probing does not disturb the accepted solution.
    fn jacobian(
        &self,
        t: f64,
        y: &[f64],
        f0: &[f64],
        env: &mut HashMap<String, f64>,
        alg_guess: &[f64],
    ) -> Result<Vec<Vec<f64>>, SimError> {
        let n = y.len();
        let mut jac = vec![vec![0.0; n]; n];
        let mut probe = y.to_vec();
        let mut scratch = alg_guess.to_vec();
        let mut f = Vec::with_capacity(n);
        for j in 0..n {
            let delta = 1e-7 * (1.0 + y[j].abs());
            probe[j] = y[j] + delta;
            self.eval_point(t, &probe, env, &mut f, &mut scratch)?;
            probe[j] = y[j];
            for (i, row) in jac.iter_mut().enumerate() {
                row[j] = (f[i] - f0[i]) / delta;
            }
        }
        Ok(jac)
    }

    /// Variable-order (1..5), variable-step BDF with Newton iteration
    /// and a reused finite-difference Jacobian.
    ///
    /// Implicit and stable for stiff systems, where explicit methods
    /// are limited by stability rather than accuracy. Coefficients come
    /// from differentiating the Lagrange interpolant over the actual
    /// (non-uniform) step history, so no restart is needed after a step
    /// size change. Dense output on the `Interval` grid uses the same
    /// interpolant.
    pub fn simulate_bdf(&self) -> Result<SimResult, SimError> {
        const MAX_ORDER: usize = 5;
        const NEWTON_MAX: usize = 12;

        let n = self.states.len();
        let stop = self.stop_time;
        let out_step = self.step.max(1e-12);
        let rtol = self.tolerance;
        let atol = self.tolerance * 1e-3;

        let mut env = HashMap::new();
        let mut columns = vec!["time".to_string()];
        columns.extend(self.states.iter().cloned());
        columns.extend(self.algebraics.iter().cloned());
        let mut rows: Vec<Vec<f64>> = Vec::new();
        let mut alg_guess = self.algebraic_start.clone();
        let mut f_scratch = Vec::new();

        let mut record = |t: f64,
                          y: &[f64],
                          env: &mut HashMap<String, f64>,
                          k: &mut Vec<f64>,
                          alg_guess: &mut [f64]|
         -> Result<(), SimError> {
            self.eval_point(t, y, env, k, alg_guess)?;
            let mut row = Vec::with_capacity(1 + n + self.algebraics.len());
            row.push(t);
            row.extend_from_slice(y);
            for name in &self.algebraics {
                row.push(env[name]);
            }
            rows.push(row);
            Ok(())
        };

        let mut y = self.initial.clone();
        let mut terminated: Option<String> = None;
        let mut when_prev = vec![false; self.when_clauses.len()];
        record(0.0, &y, &mut env, &mut f_scratch, &mut alg_guess)?;
        let start_event = self.handle_event(0.0, &env, &mut y, &mut when_prev)?;
        let mut indicators_prev = self.indicator_values(0.0, &env)?;
        if let Some(message) = start_event.terminated {
            return Ok(SimResult {
                columns,
                rows,
                parameters: self.parameters.clone(),
                terminated: Some(message),
            });
        }

        // Pure-algebraic models: nothing to integrate, walk the grid.
        let mut out_i = 1usize;
        let mut last_out_t = 0.0f64;
        if n == 0 {
            loop {
                let t = out_i as f64 * out_step;
                if t > stop + 1e-12 {
                    break;
                }
                record(t, &y, &mut env, &mut f_scratch, &mut alg_guess)?;
                last_out_t = t;
                out_i += 1;
                terminated = self
                    .handle_event(t, &env, &mut y, &mut when_prev)?
                    .terminated;
                if terminated.is_some() {
                    break;
                }
            }
            if terminated.is_none() && last_out_t < stop - 1e-12 {
                record(stop, &y, &mut env, &mut f_scratch, &mut alg_guess)?;
            }
            return Ok(SimResult {
                columns,
                rows,
                parameters: self.parameters.clone(),
                terminated,
            });
        }

        // History, newest first.
        let mut t_hist: Vec<f64> = vec![0.0];
        let mut y_hist: Vec<Vec<f64>> = vec![y.clone()];
        let mut order = 1usize;
        let mut t = 0.0f64;
        let mut h = out_step.min(stop).max(1e-12);
        let mut jac: Option<Vec<Vec<f64>>> = None;
        let mut consecutive_ok = 0usize;
        let mut steps: u64 = 0;

        let mut f_new = vec![0.0; n];
        let mut y_new = vec![0.0; n];
        let mut y_pred = vec![0.0; n];
        let mut f_last = Vec::new();
        self.eval_point(0.0, &y, &mut env, &mut f_last, &mut alg_guess)?;

        while t < stop - 1e-12 {
            h = h.min(stop - t);
            let k = order.min(t_hist.len());
            let t_new = t + h;

            // Lagrange nodes: the new point first, then the history.
            let mut nodes = Vec::with_capacity(k + 1);
            nodes.push(t_new);
            nodes.extend(t_hist.iter().take(k).copied());
            let coeffs = lagrange_derivative_coefficients(&nodes);

            // Predictor of the same order as the corrector: the
            // polynomial through the last k+1 points. Early on there is
            // no such history, so a derivative-based linear predictor
            // (y_n + h*f_n) stands in — otherwise the predictor-corrector
            // difference would measure the solution change, not the error.
            let predictor_points = (k + 1).min(t_hist.len());
            if predictor_points >= 2 {
                for (i, slot) in y_pred.iter_mut().enumerate() {
                    *slot = lagrange_extrapolate(
                        &t_hist[..predictor_points],
                        &y_hist[..predictor_points],
                        i,
                        t_new,
                    );
                }
            } else {
                for (i, slot) in y_pred.iter_mut().enumerate() {
                    *slot = y_hist[0][i] + h * f_last[i];
                }
            }
            y_new.copy_from_slice(&y_pred);

            // Constant part of the BDF residual.
            let mut hist_sum = vec![0.0; n];
            for (j, coeff) in coeffs.iter().enumerate().skip(1) {
                for (i, slot) in hist_sum.iter_mut().enumerate() {
                    *slot += coeff * y_hist[j - 1][i];
                }
            }
            let c0 = coeffs[0];

            // Newton iteration on c0*y + hist - f(t_new, y) = 0.
            let mut converged = false;
            let mut newton_failed = false;
            for iteration in 0..NEWTON_MAX {
                self.eval_point(t_new, &y_new, &mut env, &mut f_new, &mut alg_guess)?;
                steps += 1;
                if steps > 20_000_000 {
                    return err(format!(
                        "solver exceeded the evaluation budget at t = {t:.6}"
                    ));
                }
                let residual: Vec<f64> = (0..n)
                    .map(|i| c0 * y_new[i] + hist_sum[i] - f_new[i])
                    .collect();
                if residual.iter().any(|r| !r.is_finite()) {
                    newton_failed = true;
                    break;
                }

                if jac.is_none() || iteration == 4 {
                    jac = Some(self.jacobian(t_new, &y_new, &f_new, &mut env, &alg_guess)?);
                }
                let mut matrix: Vec<Vec<f64>> = jac
                    .as_ref()
                    .expect("jacobian present")
                    .iter()
                    .enumerate()
                    .map(|(i, row)| {
                        row.iter()
                            .enumerate()
                            .map(|(j, value)| if i == j { c0 - value } else { -value })
                            .collect()
                    })
                    .collect();
                let Some(delta) = solve_linear(&mut matrix, &residual) else {
                    newton_failed = true;
                    break;
                };
                let mut scaled = 0.0f64;
                for i in 0..n {
                    y_new[i] -= delta[i];
                    let scale = atol + rtol * y_new[i].abs().max(1.0);
                    scaled = scaled.max((delta[i] / scale).abs());
                }
                if !y_new.iter().all(|v| v.is_finite()) {
                    newton_failed = true;
                    break;
                }
                if scaled < 0.1 {
                    converged = true;
                    break;
                }
            }

            if newton_failed || !converged {
                // Shrink the step, drop to first order and refresh the
                // Jacobian: the linear model is clearly stale.
                h *= 0.25;
                order = 1;
                jac = None;
                if h < stop * 1e-14 || h < 1e-300 {
                    return err(format!(
                        "step size underflow at t = {t:.6}: Newton iteration does not converge"
                    ));
                }
                continue;
            }

            // Predictor-corrector difference estimates the local error.
            let mut err_norm = 0.0f64;
            for i in 0..n {
                let scale = atol + rtol * y_new[i].abs().max(y[i].abs());
                let e = (y_new[i] - y_pred[i]) / scale;
                err_norm += e * e;
            }
            err_norm = (err_norm / n as f64).sqrt();

            if err_norm.is_finite() && err_norm <= 1.0 {
                // Dense output on the grid via the step interpolant.
                let mut interp_nodes = Vec::with_capacity(k + 1);
                interp_nodes.push(t_new);
                interp_nodes.extend(t_hist.iter().take(k).copied());
                let mut interp_values: Vec<&[f64]> = Vec::with_capacity(k + 1);
                interp_values.push(&y_new);
                for slot in y_hist.iter().take(k) {
                    interp_values.push(slot);
                }
                let mut interp = vec![0.0; n];
                let sample = |at: f64, interp: &mut Vec<f64>| {
                    for (i, slot) in interp.iter_mut().enumerate() {
                        *slot = lagrange_value(&interp_nodes, &interp_values, i, at);
                    }
                };

                // Locate the earliest event indicator crossing, if any.
                self.eval_point(t_new, &y_new, &mut env, &mut f_scratch, &mut alg_guess)?;
                let indicators_new = self.indicator_values(t_new, &env)?;
                let mut event_t: Option<f64> = None;
                for (index, (&before, &after)) in
                    indicators_prev.iter().zip(&indicators_new).enumerate()
                {
                    if before.abs() <= 1e-12 || before * after >= 0.0 {
                        continue;
                    }
                    let (mut lo, mut hi) = (t, t_new);
                    for _ in 0..40 {
                        let mid = 0.5 * (lo + hi);
                        sample(mid, &mut interp);
                        self.eval_point(mid, &interp, &mut env, &mut f_scratch, &mut alg_guess)?;
                        if before * self.indicator_values(mid, &env)?[index] <= 0.0 {
                            hi = mid;
                        } else {
                            lo = mid;
                        }
                    }
                    let crossed = (hi + 1e-9 * (t_new - t)).min(t_new);
                    event_t = Some(event_t.map_or(crossed, |c: f64| c.min(crossed)));
                }

                let horizon = event_t.unwrap_or(t_new);
                loop {
                    let out_t = out_i as f64 * out_step;
                    if out_t > horizon + 1e-12 || out_t > stop + 1e-12 {
                        break;
                    }
                    sample(out_t, &mut interp);
                    record(out_t, &interp, &mut env, &mut f_scratch, &mut alg_guess)?;
                    last_out_t = out_t;
                    out_i += 1;
                }

                if let Some(t_event) = event_t {
                    sample(t_event, &mut interp);
                    y.copy_from_slice(&interp);
                    self.eval_point(t_event, &y, &mut env, &mut f_scratch, &mut alg_guess)?;
                    let outcome = self.handle_event(t_event, &env, &mut y, &mut when_prev)?;
                    t = t_event;
                    // The history is meaningless across a discontinuity.
                    t_hist.clear();
                    y_hist.clear();
                    t_hist.push(t);
                    y_hist.push(y.clone());
                    order = 1;
                    consecutive_ok = 0;
                    jac = None;
                    self.eval_point(t, &y, &mut env, &mut f_last, &mut alg_guess)?;
                    indicators_prev = self.indicator_values(t, &env)?;
                    if let Some(message) = outcome.terminated {
                        record(t, &y, &mut env, &mut f_scratch, &mut alg_guess)?;
                        terminated = Some(message);
                        break;
                    }
                    if outcome.reinitialized {
                        record(t, &y, &mut env, &mut f_scratch, &mut alg_guess)?;
                    }
                    h = (h * 0.25).max(1e-12);
                    continue;
                }
                indicators_prev = indicators_new;

                t = t_new;
                y.copy_from_slice(&y_new);
                f_last.copy_from_slice(&f_new);
                t_hist.insert(0, t_new);
                y_hist.insert(0, y_new.clone());
                if t_hist.len() > MAX_ORDER + 1 {
                    t_hist.pop();
                    y_hist.pop();
                }
                consecutive_ok += 1;
                // Raise the order once the history supports it. The
                // step controller keeps the error near its target, so
                // waiting for a *small* error would pin the order at 1;
                // a premature raise simply costs one rejected step.
                if consecutive_ok > order && order < MAX_ORDER && t_hist.len() > order {
                    order += 1;
                    consecutive_ok = 0;
                }
            } else {
                consecutive_ok = 0;
                if order > 1 {
                    order -= 1;
                }
            }

            let factor = if !err_norm.is_finite() || err_norm == 0.0 {
                if err_norm == 0.0 {
                    2.0
                } else {
                    0.25
                }
            } else {
                (0.9 * err_norm.powf(-1.0 / (k as f64 + 1.0))).clamp(0.2, 4.0)
            };
            h *= factor;
            if h < stop * 1e-14 || h < 1e-300 {
                return err(format!(
                    "step size underflow at t = {t:.6}: probable singularity"
                ));
            }
        }

        if terminated.is_none() && last_out_t < stop - 1e-12 {
            record(stop, &y, &mut env, &mut f_scratch, &mut alg_guess)?;
            terminated = self
                .handle_event(stop, &env, &mut y, &mut when_prev)?
                .terminated;
        }
        Ok(SimResult {
            columns,
            rows,
            parameters: self.parameters.clone(),
            terminated,
        })
    }

    /// Classic fixed-step RK4 integration over `[0, stop_time]`.
    pub fn simulate_rk4(&self) -> Result<SimResult, SimError> {
        let n = self.states.len();
        let steps = (self.stop_time / self.step).ceil() as usize;
        let mut y = self.initial.clone();
        let mut alg_guess = self.algebraic_start.clone();
        let mut env = HashMap::new();
        let (mut k1, mut k2, mut k3, mut k4) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut scratch = vec![0.0; n];

        let mut columns = vec!["time".to_string()];
        columns.extend(self.states.iter().cloned());
        columns.extend(self.algebraics.iter().cloned());
        let mut rows = Vec::with_capacity(steps + 1);

        let mut record = |t: f64,
                          y: &[f64],
                          env: &mut HashMap<String, f64>,
                          k: &mut Vec<f64>,
                          this: &CompiledModel,
                          alg_guess: &mut [f64]|
         -> Result<(), SimError> {
            this.eval_point(t, y, env, k, alg_guess)?;
            let mut row = Vec::with_capacity(1 + this.states.len() + this.algebraics.len());
            row.push(t);
            row.extend_from_slice(y);
            for name in &this.algebraics {
                row.push(env[name]);
            }
            rows.push(row);
            Ok(())
        };

        record(0.0, &y, &mut env, &mut k1, self, &mut alg_guess)?;
        let mut when_prev = vec![false; self.when_clauses.len()];
        let mut terminated = self
            .handle_event(0.0, &env, &mut y, &mut when_prev)?
            .terminated;

        for i in 0..steps {
            if terminated.is_some() {
                break;
            }
            let t = i as f64 * self.step;
            let h = (self.stop_time - t).min(self.step);

            self.eval_point(t, &y, &mut env, &mut k1, &mut alg_guess)?;
            for j in 0..n {
                scratch[j] = y[j] + 0.5 * h * k1[j];
            }
            self.eval_point(t + 0.5 * h, &scratch, &mut env, &mut k2, &mut alg_guess)?;
            for j in 0..n {
                scratch[j] = y[j] + 0.5 * h * k2[j];
            }
            self.eval_point(t + 0.5 * h, &scratch, &mut env, &mut k3, &mut alg_guess)?;
            for j in 0..n {
                scratch[j] = y[j] + h * k3[j];
            }
            self.eval_point(t + h, &scratch, &mut env, &mut k4, &mut alg_guess)?;
            for j in 0..n {
                y[j] += h / 6.0 * (k1[j] + 2.0 * k2[j] + 2.0 * k3[j] + k4[j]);
            }
            record(t + h, &y, &mut env, &mut k1, self, &mut alg_guess)?;
            terminated = self
                .handle_event(t + h, &env, &mut y, &mut when_prev)?
                .terminated;
        }

        Ok(SimResult {
            columns,
            rows,
            parameters: self.parameters.clone(),
            terminated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxidelica_parser::parse_model;

    fn run(source: &str) -> SimResult {
        let model = parse_model(source).unwrap();
        compile(&model).unwrap().simulate().unwrap()
    }

    #[test]
    fn decay_matches_analytic() {
        let result = run("model D parameter Real a = 1.0; Real x(start = 1.0); \
             equation der(x) = -a*x; \
             annotation(experiment(StopTime=5.0, Interval=0.001, Tolerance=1e-12)); end D;");
        let last = result.rows.last().unwrap();
        let t = last[0];
        let x = last[1];
        assert!((t - 5.0).abs() < 1e-12);
        assert!(
            (x - (-5.0f64).exp()).abs() < 1e-9,
            "x(5)={x}, expected e^-5"
        );
    }

    #[test]
    fn pendulum_conserves_energy() {
        let result = run("model P parameter Real g = 9.81; parameter Real L = 1.0; \
             Real phi(start = 0.7); Real w(start = 0.0); \
             equation der(phi) = w; der(w) = -(g/L)*sin(phi); \
             annotation(experiment(StopTime=10.0, Interval=0.001, Tolerance=1e-12)); end P;");
        let energy = |row: &Vec<f64>| {
            let (phi, w) = (row[1], row[2]);
            0.5 * w * w + 9.81 * (1.0 - phi.cos())
        };
        let e0 = energy(&result.rows[0]);
        let e_end = energy(result.rows.last().unwrap());
        assert!(
            ((e_end - e0) / e0).abs() < 1e-9,
            "energy drifted: {e0} -> {e_end}"
        );
    }

    #[test]
    fn algebraic_chain_is_ordered() {
        // y depends on x, x on the state; declaration order is reversed.
        let result = run("model A Real s(start = 1.0); Real y; Real x; \
             equation der(s) = -s; y = 2*x; x = s + 1; \
             annotation(experiment(StopTime=1.0, Interval=0.01)); end A;");
        let first = &result.rows[0];
        // Columns: time, s, x, y (algebraics in evaluation order).
        assert_eq!(result.columns, vec!["time", "s", "x", "y"]);
        assert!((first[2] - 2.0).abs() < 1e-12); // x = s+1 = 2
        assert!((first[3] - 4.0).abs() < 1e-12); // y = 2x = 4
    }

    #[test]
    fn reports_missing_equation() {
        let model = parse_model("model B Real x; Real y; equation x = 1; end B;").unwrap();
        let error = compile(&model).unwrap_err();
        assert!(error.0.contains("unbalanced"), "{}", error.0);
    }

    #[test]
    fn solves_implicit_linear_system() {
        // x + y = 2 and x - y = 0 are not assignments - the matcher
        // pairs them with x and y and Newton solves the block.
        let result = run("model I Real x; Real y; equation x + y = 2; x - y = 0; \
             annotation(experiment(StopTime=0.01, Interval=0.01)); end I;");
        let x_idx = result.columns.iter().position(|c| c == "x").unwrap();
        let y_idx = result.columns.iter().position(|c| c == "y").unwrap();
        assert!((result.rows[0][x_idx] - 1.0).abs() < 1e-9);
        assert!((result.rows[0][y_idx] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn if_expression_saturates() {
        // Saturation: y = clamp(x, -1, 1); x grows linearly from 0 to 2.
        let result = run("model S Real x(start = 0.0); Real y; \
             equation der(x) = 1; y = if x > 1 then 1 elseif x < -1 then -1 else x; \
             annotation(experiment(StopTime=2.0, Interval=0.01)); end S;");
        let mid = &result.rows[result.rows.len() / 2]; // t=1: y == x == 1
        let last = result.rows.last().unwrap(); // t=2: x=2, y=1
        assert!((mid[2] - 1.0).abs() < 1e-6, "y(1)={}", mid[2]);
        assert!((last[1] - 2.0).abs() < 1e-9, "x(2)={}", last[1]);
        assert!((last[2] - 1.0).abs() < 1e-12, "y(2)={}", last[2]);
    }

    #[test]
    fn degenerate_algebraic_cycle_is_rejected_at_compile_time() {
        // x = y + 1 and y = x - 1 are the same equation twice: the loop
        // is structurally sound but has a whole family of solutions, so
        // the regularity check rejects it before any stepping happens.
        let model =
            parse_model("model C Real x; Real y; equation x = y + 1; y = x - 1; end C;").unwrap();
        let error = compile(&model).unwrap_err();
        assert!(error.0.contains("underdetermined"), "{}", error.0);
    }

    #[test]
    fn solves_linear_algebraic_loop() {
        // x = y/2 + 1, y = x/2 + 1  ->  x = y = 2.
        let result = run(
            "model L Real x; Real y; equation x = y / 2 + 1; y = x / 2 + 1; \
             annotation(experiment(StopTime=0.01, Interval=0.01)); end L;",
        );
        let x_idx = result.columns.iter().position(|c| c == "x").unwrap();
        let y_idx = result.columns.iter().position(|c| c == "y").unwrap();
        assert!((result.rows[0][x_idx] - 2.0).abs() < 1e-9);
        assert!((result.rows[0][y_idx] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn solves_nonlinear_self_reference() {
        // x = cos(x): the Dottie number 0.739085...
        let result = run("model D Real x(start = 1); equation x = cos(x); \
             annotation(experiment(StopTime=0.01, Interval=0.01)); end D;");
        assert!(
            (result.rows[0][1] - 0.739_085_133_2).abs() < 1e-8,
            "{}",
            result.rows[0][1]
        );
    }

    #[test]
    fn algebraic_loop_follows_a_state() {
        // The loop depends on a state: x = y/2 + s, y = x/2, so
        // x = (2/3) s ... wait: x = y/2 + s and y = x/2 -> x = x/4 + s
        // -> x = (4/3) s, y = (2/3) s.
        let result = run("model F Real s(start = 3.0); Real x; Real y; equation \
             der(s) = 0; x = y / 2 + s; y = x / 2; \
             annotation(experiment(StopTime=0.01, Interval=0.01)); end F;");
        let x_idx = result.columns.iter().position(|c| c == "x").unwrap();
        let y_idx = result.columns.iter().position(|c| c == "y").unwrap();
        assert!(
            (result.rows[0][x_idx] - 4.0).abs() < 1e-9,
            "{}",
            result.rows[0][x_idx]
        );
        assert!(
            (result.rows[0][y_idx] - 2.0).abs() < 1e-9,
            "{}",
            result.rows[0][y_idx]
        );
    }

    fn compile_err(source: &str) -> String {
        compile(&parse_model(source).unwrap())
            .unwrap_err()
            .to_string()
    }

    fn simulate_err(source: &str) -> String {
        compile(&parse_model(source).unwrap())
            .unwrap()
            .simulate()
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn evaluates_every_builtin_function() {
        let result = run("model F Real y; equation \
             y = sin(1) + cos(1) + tan(1) + asin(0.5) + acos(0.5) + atan(1) \
               + atan2(1, 2) + sinh(1) + cosh(1) + tanh(1) + exp(1) + log(2) \
               + log10(100) + sqrt(4) + abs(-3) + sign(-2) + min(1, 2) + max(1, 5) + 2 ^ 10; \
             annotation(experiment(StopTime=0.01, Interval=0.01)); end F;");
        let expected = 1f64.sin()
            + 1f64.cos()
            + 1f64.tan()
            + 0.5f64.asin()
            + 0.5f64.acos()
            + 1f64.atan()
            + 1f64.atan2(2.0)
            + 1f64.sinh()
            + 1f64.cosh()
            + 1f64.tanh()
            + 1f64.exp()
            + 2f64.ln()
            + 100f64.log10()
            + 2.0
            + 3.0
            + (-1.0)
            + 1.0
            + 5.0
            + 1024.0;
        assert!((result.rows[0][1] - expected).abs() < 1e-12);
    }

    #[test]
    fn evaluates_booleans_and_relations() {
        let result = run("model B Real y; Real r; equation \
             y = if true and not false or false then 1 else 0; \
             r = (if 1 < 2 then 1 else 0) + (if 2 <= 2 then 1 else 0) \
               + (if 3 > 2 then 1 else 0) + (if 2 >= 3 then 0 else 1) \
               + (if 1 == 1 then 1 else 0) + (if 1 <> 2 then 1 else 0); \
             annotation(experiment(StopTime=0.01, Interval=0.01)); end B;");
        let names = &result.columns;
        let y_idx = names.iter().position(|n| n == "y").unwrap();
        let r_idx = names.iter().position(|n| n == "r").unwrap();
        assert_eq!(result.rows[0][y_idx], 1.0);
        assert_eq!(result.rows[0][r_idx], 6.0);
    }

    #[test]
    fn compile_error_paths() {
        // Parameter without a value.
        assert!(
            compile_err("model M parameter Real p; Real x; equation x = 1; end M;")
                .contains("has no value")
        );
        // Parameter cycle.
        assert!(compile_err(
            "model M parameter Real a = b; parameter Real b = a; Real x; equation x = 1; end M;"
        )
        .contains("cycle"));
        // der of a parameter.
        assert!(
            compile_err("model M parameter Real p = 1; equation der(p) = 1; end M;")
                .contains("continuous")
        );
        // der on both sides.
        assert!(
            compile_err("model M Real x; Real y; equation der(x) = der(y); y = 1; end M;")
                .contains("must appear alone")
        );
        // Two equations for one state.
        assert!(
            compile_err("model M Real x; equation der(x) = 1; der(x) = 2; end M;")
                .contains("two equations")
        );
        // Two equations for one algebraic unknown: unbalanced.
        assert!(compile_err("model M Real y; equation y = 1; y = 2; end M;").contains("unbalanced"));
        // A state with an extra algebraic equation: unbalanced.
        assert!(
            compile_err("model M Real x; equation der(x) = 1; x = 2; end M;")
                .contains("unbalanced")
        );

        // der inside an algebraic expression.
        assert!(
            compile_err("model M Real x; Real y; equation der(x) = 1; y = der(x) + 1; end M;")
                .contains("appear alone")
        );
        // Reference to an undeclared variable.
        assert!(compile_err("model M Real x; equation x = 1; q = 2; end M;")
            .contains("unknown variable"));
        // Error in a start expression.
        assert!(
            compile_err("model M Real x(start = q); equation der(x) = 1; end M;")
                .contains("start of x")
        );
    }

    #[test]
    fn runtime_error_paths() {
        // An unknown variable is now a compile-time error.
        assert!(
            compile_err("model M Real y; equation y = z + 1; end M;").contains("unknown variable")
        );
        // Unknown function.
        assert!(simulate_err("model M Real y; equation y = frob(1); end M;")
            .contains("unknown function"));
        // Wrong arity.
        assert!(
            simulate_err("model M Real y; equation y = sin(1, 2); end M;").contains("argument")
        );
    }

    #[test]
    fn csv_output_and_defaults() {
        let model = parse_model("model M Real x(start=1); equation der(x) = -x; end M;").unwrap();
        let compiled = compile(&model).unwrap();
        // Defaults apply without an annotation.
        assert_eq!(compiled.stop_time, 1.0);
        assert_eq!(compiled.step, 1e-3);
        let result = compiled.simulate().unwrap();
        let csv = result.to_csv();
        let mut lines = csv.lines();
        assert_eq!(lines.next().unwrap(), "time,x");
        assert!(lines.next().unwrap().starts_with("0.000000000,1.000000000"));
        assert_eq!(csv.lines().count(), result.rows.len() + 1);
    }

    #[test]
    fn parameter_uses_start_as_fallback_value() {
        let model =
            parse_model("model M parameter Real p(start = 3); Real x; equation x = p; end M;")
                .unwrap();
        let compiled = compile(&model).unwrap();
        assert_eq!(compiled.parameters, vec![("p".to_string(), 3.0)]);
    }

    #[test]
    fn mirrored_equation_forms_and_default_start() {
        // expr = der(v), expr = v, a state without start (zero-initialized),
        // and subtraction.
        let result = run("model M Real x; Real y; equation \
             -x - 1 = der(x); 2 + time = y; \
             annotation(experiment(StopTime=1.0, Interval=0.001, Tolerance=1e-12)); end M;");
        let first = &result.rows[0];
        assert_eq!(first[1], 0.0); // x(0) = 0 by default
        assert!((first[2] - 2.0).abs() < 1e-12); // y(0) = 2
                                                 // der(x) = -x - 1, x(0)=0 -> x(t) = e^{-t} - 1
        let last = result.rows.last().unwrap();
        assert!(
            (last[1] - ((-1.0f64).exp() - 1.0)).abs() < 1e-9,
            "x(1)={}",
            last[1]
        );
    }

    #[test]
    fn false_branches_of_relations_and_logic() {
        let result = run("model B Real r; Real y; equation \
             r = (if 2 < 1 then 1 else 0) + (if 3 <= 2 then 1 else 0) \
               + (if 2 > 3 then 1 else 0) + (if 3 >= 2 then 1 else 0) \
               + (if 1 == 2 then 1 else 0) + (if 1 <> 1 then 1 else 0); \
             y = (if false or false then 1 else 0) + (if false and true then 1 else 0) \
               + (if not true then 1 else 0) + (if false then 1 else 0); \
             annotation(experiment(StopTime=0.01, Interval=0.01)); end B;");
        let r_idx = result.columns.iter().position(|n| n == "r").unwrap();
        let y_idx = result.columns.iter().position(|n| n == "y").unwrap();
        assert_eq!(result.rows[0][r_idx], 1.0); // only >= holds
        assert_eq!(result.rows[0][y_idx], 0.0); // all false branches
    }

    #[test]
    fn adaptive_respects_tolerance() {
        let source = |tol: &str| {
            format!(
                "model D Real x(start = 1.0); equation der(x) = -x; \
                 annotation(experiment(StopTime=5.0, Interval=0.1, Tolerance={tol})); end D;"
            )
        };
        let error_at = |tol: &str| {
            let result = run(&source(tol));
            (result.rows.last().unwrap()[1] - (-5.0f64).exp()).abs()
        };
        let loose = error_at("1e-3");
        let tight = error_at("1e-10");
        assert!(tight < loose, "tight={tight}, loose={loose}");
        assert!(tight < 1e-8, "tight={tight}");
    }

    #[test]
    fn singularity_reports_step_underflow() {
        // x' = -1/x reaches x = 0 at t = 0.5: a genuine singularity.
        let model = parse_model(
            "model S Real x(start = 1.0); equation der(x) = -1/x; \
             annotation(experiment(StopTime=1.0, Interval=0.01)); end S;",
        )
        .unwrap();
        let error = compile(&model).unwrap().simulate().unwrap_err();
        assert!(
            error.0.contains("step size underflow") || error.0.contains("budget"),
            "{}",
            error.0
        );
    }

    #[test]
    fn rk4_method_is_still_available() {
        let model = parse_model(
            "model D Real x(start = 1.0); equation der(x) = -x; \
             annotation(experiment(StopTime=1.0, Interval=0.001)); end D;",
        )
        .unwrap();
        let mut compiled = compile(&model).unwrap();
        compiled.method = SolverMethod::Rk4;
        let result = compiled.simulate().unwrap();
        let x = result.rows.last().unwrap()[1];
        assert!((x - (-1.0f64).exp()).abs() < 1e-9, "x(1)={x}");
    }

    #[test]
    fn when_terminate_stops_simulation() {
        let result = run("model W Real x(start = 0.0); equation der(x) = 1; \
             when x > 0.5 then terminate(\"threshold reached\"); end when; \
             annotation(experiment(StopTime=2.0, Interval=0.01)); end W;");
        let message = result.terminated.expect("must terminate");
        assert!(message.contains("threshold reached"), "{message}");
        let last_t = result.rows.last().unwrap()[0];
        assert!(
            (0.5..=0.55).contains(&last_t),
            "stopped at t = {last_t}, expected just past 0.5"
        );
    }

    #[test]
    fn when_terminate_can_fire_at_start() {
        let result = run("model W Real x(start = 5.0); equation der(x) = 1; \
             when x > 1 then terminate(\"already past\"); end when; \
             annotation(experiment(StopTime=1.0, Interval=0.01)); end W;");
        assert!(result.terminated.is_some());
        assert_eq!(result.rows.len(), 1); // only the initial point
    }

    #[test]
    fn normal_runs_do_not_terminate() {
        let result = run("model N Real x(start = 0.0); equation der(x) = 1; \
             when x > 100 then terminate(\"never\"); end when; \
             annotation(experiment(StopTime=1.0, Interval=0.1)); end N;");
        assert!(result.terminated.is_none());
        assert!((result.rows.last().unwrap()[0] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn rc_circuit_from_components_matches_analytic() {
        // The full M2 pipeline: connectors, extends, connect, flattening,
        // matching. c.v(t) = V * (1 - e^(-t / (R*C))).
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/rc_circuit.mo"),
        )
        .unwrap();
        let result = compile(&parse_model(&source).unwrap())
            .unwrap()
            .simulate()
            .unwrap();
        let cv = result.columns.iter().position(|c| c == "c.v").unwrap();
        let last = result.rows.last().unwrap();
        let analytic = 1.0 - (-last[0] / (100.0 * 0.001)).exp();
        assert!(
            (last[cv] - analytic).abs() < 1e-9,
            "c.v = {}, analytic = {analytic}",
            last[cv]
        );
    }

    #[test]
    fn index2_dae_tracks_the_constraint() {
        // q = time^2 with der(q) = z: the constraint must be
        // differentiated once (Pantelides) to expose z; z = 2t follows.
        let result = run(
            "model D Real z; Real q(start = 0); equation der(q) = z; q = time ^ 2; \
             annotation(experiment(StopTime=1.0, Interval=0.01, Tolerance=1e-10)); end D;",
        );
        let last = result.rows.last().unwrap();
        let q_idx = result.columns.iter().position(|c| c == "q").unwrap();
        let z_idx = result.columns.iter().position(|c| c == "z").unwrap();
        assert!((last[q_idx] - 1.0).abs() < 1e-6, "q(1) = {}", last[q_idx]);
        assert!((last[z_idx] - 2.0).abs() < 1e-4, "z(1) = {}", last[z_idx]);
    }

    #[test]
    fn index3_cartesian_pendulum_matches_angle_form() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/cartesian_pendulum.mo"),
        )
        .unwrap();
        let cart = compile(&parse_model(&source).unwrap())
            .unwrap()
            .simulate()
            .unwrap();
        let x_idx = cart.columns.iter().position(|c| c == "x").unwrap();
        let y_idx = cart.columns.iter().position(|c| c == "y").unwrap();
        // The length constraint holds throughout.
        let worst = cart
            .rows
            .iter()
            .map(|r| (r[x_idx] * r[x_idx] + r[y_idx] * r[y_idx] - 1.0).abs())
            .fold(0.0f64, f64::max);
        assert!(worst < 1e-6, "constraint violation {worst}");
        // Cross-check against the angle formulation of the same pendulum.
        let angle = run("model P parameter Real g = 9.81; parameter Real L = 1.0; \
             Real phi(start = 0.7); Real w(start = 0.0); Real x; Real y; \
             equation der(phi) = w; der(w) = -(g/L)*sin(phi); \
             x = L * sin(phi); y = -L * cos(phi); \
             annotation(experiment(StopTime=10.0, Interval=0.001, Tolerance=1e-10)); end P;");
        let ax = angle.columns.iter().position(|c| c == "x").unwrap();
        let ay = angle.columns.iter().position(|c| c == "y").unwrap();
        let (cl, al) = (cart.rows.last().unwrap(), angle.rows.last().unwrap());
        assert!(
            (cl[x_idx] - al[ax]).abs() < 1e-4 && (cl[y_idx] - al[ay]).abs() < 1e-4,
            "cartesian ({}, {}) vs angle ({}, {})",
            cl[x_idx],
            cl[y_idx],
            al[ax],
            al[ay]
        );
    }

    #[test]
    fn a_plain_start_is_a_guess_but_fixed_is_an_initial_condition() {
        // q is demoted by index reduction and solved from q = time^2,
        // so a plain start is only a Newton guess: q(0) = 0 wins.
        let result = run(
            "model D Real z; Real q(start = 1); equation der(q) = z; q = time ^ 2; \
             annotation(experiment(StopTime=1.0, Interval=0.01)); end D;",
        );
        let q_idx = result.columns.iter().position(|c| c == "q").unwrap();
        assert!(
            result.rows[0][q_idx].abs() < 1e-9,
            "q(0) = {}",
            result.rows[0][q_idx]
        );
        // Declaring it fixed turns the contradiction into an error.
        let model = parse_model(
            "model D Real z; Real q(start = 1, fixed = true); \
             equation der(q) = z; q = time ^ 2; \
             annotation(experiment(StopTime=1.0, Interval=0.01)); end D;",
        )
        .unwrap();
        let error = compile(&model).unwrap_err();
        assert!(error.0.contains("is fixed at"), "{}", error.0);
    }

    #[test]
    fn truly_singular_system_is_still_rejected() {
        // Two equations for `a`, none for `b`; differentiation cannot
        // help because b never appears.
        let error = compile_err("model M Real a; Real b; equation a = 1; a = 2; end M;");
        assert!(
            error.contains("structurally singular") && error.contains("constrains no state"),
            "{error}"
        );
    }

    #[test]
    fn bdf_handles_a_stiff_system_that_starves_explicit_methods() {
        // der(x) = -1e6 * (x - cos t): the explicit method is limited by
        // stability, the implicit one by accuracy only.
        let source = "model S Real x(start = 0.0); \
             equation der(x) = -1000000.0 * (x - cos(time)); \
             annotation(experiment(StopTime=5.0, Interval=0.01, Tolerance=1e-6)); end S;";
        let mut compiled = compile(&parse_model(source).unwrap()).unwrap();
        compiled.method = SolverMethod::Bdf;
        let result = compiled.simulate().unwrap();
        let x = result.rows.last().unwrap()[1];
        // After the transient the solution tracks the quasi-steady cos t.
        assert!(
            (x - 5.0f64.cos()).abs() < 1e-5,
            "x(5) = {x}, expected ~{}",
            5.0f64.cos()
        );
    }

    #[test]
    fn bdf_and_dopri_agree_on_a_non_stiff_model() {
        let source = "model P parameter Real g = 9.81; Real phi(start = 0.7); Real w(start = 0); \
             equation der(phi) = w; der(w) = -g * sin(phi); \
             annotation(experiment(StopTime=2.0, Interval=0.01, Tolerance=1e-10)); end P;";
        let model = parse_model(source).unwrap();
        let dopri = compile(&model).unwrap().simulate().unwrap();
        let mut stiff_solver = compile(&model).unwrap();
        stiff_solver.method = SolverMethod::Bdf;
        let bdf = stiff_solver.simulate().unwrap();
        let (a, b) = (dopri.rows.last().unwrap(), bdf.rows.last().unwrap());
        assert!((a[1] - b[1]).abs() < 1e-6, "phi: {} vs {}", a[1], b[1]);
        assert!((a[2] - b[2]).abs() < 1e-6, "w: {} vs {}", a[2], b[2]);
    }

    #[test]
    fn tearing_shrinks_the_newton_system() {
        // A two-variable algebraic loop: one variable is torn, the other
        // follows from an explicit assignment.
        let model =
            parse_model("model L Real x; Real y; equation x = y / 2 + 1; y = x / 2 + 1; end L;")
                .unwrap();
        let compiled = compile(&model).unwrap();
        let plan = compiled.plan_summary();
        let block = plan
            .iter()
            .find(|line| line.contains("implicit block of 2"))
            .expect("a two-variable block");
        assert!(block.contains("iterating on 1"), "{block}");
        // And it still gets the right answer: x = y = 2.
        let result = compiled.simulate().unwrap();
        assert!((result.rows[0][1] - 2.0).abs() < 1e-9);
        assert!((result.rows[0][2] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn solver_names_round_trip() {
        for method in [SolverMethod::Dopri45, SolverMethod::Rk4, SolverMethod::Bdf] {
            assert_eq!(SolverMethod::from_name(method.name()), Some(method));
        }
        assert_eq!(SolverMethod::from_name("nope"), None);
    }

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
    fn bdf_covers_termination_and_algebraic_only_models() {
        // Termination inside the BDF loop.
        let mut compiled = compile(
            &parse_model(
                "model W Real x(start = 0.0); equation der(x) = 1; \
                 when x > 0.5 then terminate(\"done\"); end when; \
                 annotation(experiment(StopTime=2.0, Interval=0.01)); end W;",
            )
            .unwrap(),
        )
        .unwrap();
        compiled.method = SolverMethod::Bdf;
        assert!(compiled.simulate().unwrap().terminated.is_some());

        // A model without states: the solver just walks the grid.
        let mut algebraic_only = compile(
            &parse_model(
                "model A Real y; equation y = 2 * time; \
                 annotation(experiment(StopTime=1.0, Interval=0.25)); end A;",
            )
            .unwrap(),
        )
        .unwrap();
        algebraic_only.method = SolverMethod::Bdf;
        let result = algebraic_only.simulate().unwrap();
        assert_eq!(result.rows.len(), 5);
        assert!((result.rows.last().unwrap()[1] - 2.0).abs() < 1e-12);

        // Terminating at t = 0 short-circuits before any stepping.
        let mut immediate = compile(
            &parse_model(
                "model I Real x(start = 5.0); equation der(x) = 1; \
                 when x > 1 then terminate(\"already\"); end when; end I;",
            )
            .unwrap(),
        )
        .unwrap();
        immediate.method = SolverMethod::Bdf;
        let result = immediate.simulate().unwrap();
        assert!(result.terminated.is_some());
        assert_eq!(result.rows.len(), 1);
    }

    #[test]
    fn bdf_reports_a_singularity_instead_of_guessing() {
        // x' = -1/x runs into x = 0 at t = 0.5.
        let mut compiled = compile(
            &parse_model(
                "model S Real x(start = 1.0); equation der(x) = -1/x; \
                 annotation(experiment(StopTime=1.0, Interval=0.01)); end S;",
            )
            .unwrap(),
        )
        .unwrap();
        compiled.method = SolverMethod::Bdf;
        let error = compiled.simulate().unwrap_err();
        assert!(
            error.0.contains("underflow") || error.0.contains("budget"),
            "{}",
            error.0
        );
    }

    #[test]
    fn plan_summary_describes_stages() {
        let compiled = compile(
            &parse_model("model P Real x; Real y; equation x = 2; y = x + 1; end P;").unwrap(),
        )
        .unwrap();
        let plan = compiled.plan_summary();
        assert_eq!(plan.len(), 2);
        assert!(plan.iter().all(|line| line.starts_with("explicit")));
    }

    #[test]
    fn dummy_derivatives_demote_a_state_and_keep_the_constraint_exact() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/cartesian_pendulum.mo"),
        )
        .unwrap();
        let compiled = compile(&parse_model(&source).unwrap()).unwrap();
        // Index reduction demoted one position and one velocity: four
        // states became two, and their derivatives became dummies.
        assert_eq!(compiled.states, vec!["x", "vx"]);
        assert!(compiled.algebraics.contains(&"der(y)".to_string()));
        assert!(compiled.algebraics.contains(&"der(vy)".to_string()));

        // The constraint is solved, not stabilized: its residual stays
        // at solver tolerance instead of drifting with time.
        let result = compiled.simulate().unwrap();
        let x = result.columns.iter().position(|c| c == "x").unwrap();
        let y = result.columns.iter().position(|c| c == "y").unwrap();
        let violation = |row: &Vec<f64>| (row[x] * row[x] + row[y] * row[y] - 1.0).abs();
        let early = result.rows[result.rows.len() / 10..result.rows.len() / 5]
            .iter()
            .map(violation)
            .fold(0.0f64, f64::max);
        let late = result.rows[result.rows.len() * 4 / 5..]
            .iter()
            .map(violation)
            .fold(0.0f64, f64::max);
        assert!(late < 1e-8, "late violation {late}");
        assert!(late < 100.0 * early.max(1e-12), "drift: {early} -> {late}");
    }

    #[test]
    fn index_reduction_reaches_states_through_algebraic_definitions() {
        // `u = 3` names no state, but `u = 2*x` ties it to one: x is
        // pinned at 1.5 and its velocity has to vanish.
        let result = run("model N Real x(start = 1.0); Real v; Real u; \
             equation der(x) = v; u = 2 * x; u = 3; \
             annotation(experiment(StopTime=1.0, Interval=0.5)); end N;");
        let value = |name: &str| {
            let index = result.columns.iter().position(|c| c == name).unwrap();
            result.rows.last().unwrap()[index]
        };
        assert!((value("x") - 1.5).abs() < 1e-9, "x = {}", value("x"));
        assert!(value("v").abs() < 1e-9, "v = {}", value("v"));
        assert!((value("u") - 3.0).abs() < 1e-9, "u = {}", value("u"));
    }

    fn example(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn bouncing_ball_reinits_at_every_impact() {
        let result = compile(&parse_model(&example("bouncing_ball.mo")).unwrap())
            .unwrap()
            .simulate()
            .unwrap();
        let h = result.columns.iter().position(|c| c == "h").unwrap();
        let v = result.columns.iter().position(|c| c == "v").unwrap();

        // The floor is never breached beyond event-location tolerance.
        let deepest = result
            .rows
            .iter()
            .map(|row| row[h])
            .fold(f64::INFINITY, f64::min);
        assert!(deepest > -1e-6, "ball fell through the floor: {deepest}");

        // First impact: free fall from 1 m, rebound at 0.8 of the
        // impact speed.
        let first = result
            .rows
            .windows(2)
            .find(|w| w[0][v] < 0.0 && w[1][v] > 0.0)
            .expect("at least one bounce");
        let expected_t = (2.0f64 / 9.81).sqrt();
        let expected_v = 0.8 * (2.0 * 9.81f64).sqrt();
        assert!(
            (first[1][0] - expected_t).abs() < 1e-4,
            "t = {}",
            first[1][0]
        );
        assert!(
            (first[1][v] - expected_v).abs() < 1e-3,
            "v = {}",
            first[1][v]
        );

        // Impacts crowd toward the Zeno limit, where terminate fires.
        let message = result.terminated.expect("terminates at rest");
        assert!(message.contains("come to rest"), "{message}");
    }

    #[test]
    fn ideal_diode_never_conducts_while_blocking() {
        let result = compile(&parse_model(&example("rectifier.mo")).unwrap())
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let (vs, vc, id) = (index("vs"), index("vc"), index("id"));
        for row in &result.rows {
            if row[vs] - row[vc] < -1e-9 {
                assert!(row[id].abs() < 1e-12, "blocking diode carried {}", row[id]);
            }
        }
        // The load charges toward the source amplitude.
        let peak = result.rows.iter().map(|r| r[vc]).fold(0.0f64, f64::max);
        assert!((0.8..1.0).contains(&peak), "load peaked at {peak}");
    }

    #[test]
    fn events_are_located_rather_than_stepped_over() {
        // A coarse output grid must not blunt the event: the impact is
        // found to solver tolerance even between grid points.
        let result = run(
            "model B parameter Real g = 9.81; Real h(start = 1.0); Real v(start = 0.0); \
             equation der(h) = v; der(v) = -g; \
             when h < 0 then reinit(v, -v); end when; \
             annotation(experiment(StopTime=1.0, Interval=0.25, Tolerance=1e-9)); end B;",
        );
        let v = result.columns.iter().position(|c| c == "v").unwrap();
        let h = result.columns.iter().position(|c| c == "h").unwrap();
        // Perfectly elastic: after the bounce the ball returns to 1 m.
        let peak_after = result
            .rows
            .iter()
            .filter(|row| row[0] > 0.46)
            .map(|row| row[h])
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(peak_after > 0.9, "rebound reached only {peak_after}");
        assert!(result.rows.iter().any(|row| row[v] > 0.0), "never bounced");
    }

    #[test]
    fn reinit_targets_must_be_states() {
        let model = parse_model(
            "model R Real x(start = 1.0); Real y; equation der(x) = -2; y = 2 * x; \
             when x < 0 then reinit(y, 0); end when; \
             annotation(experiment(StopTime=1.0, Interval=0.01)); end R;",
        )
        .unwrap();
        let error = compile(&model).unwrap().simulate().unwrap_err();
        assert!(error.0.contains("is not a state"), "{}", error.0);
    }

    #[test]
    fn when_clauses_fire_on_the_rising_edge_only() {
        // A single terminate that stays true must not fire twice, and a
        // condition true at t = 0 fires immediately.
        let result = run("model E Real x(start = 0.0); equation der(x) = 1; \
             when x > 0.25 then terminate(\"crossed\"); end when; \
             annotation(experiment(StopTime=1.0, Interval=0.1)); end E;");
        let message = result.terminated.unwrap();
        assert!(message.contains("crossed"), "{message}");
        let last_t = result.rows.last().unwrap()[0];
        assert!((last_t - 0.25).abs() < 1e-6, "stopped at {last_t}");
    }

    #[test]
    fn discretized_heat_conduction_reaches_the_analytic_steady_state() {
        // The M5 pipeline end to end: an array of 40 nodes, a for
        // equation over the interior, and an inlined function.
        let compiled = compile(&parse_model(&example("heat_conduction.mo")).unwrap()).unwrap();
        assert_eq!(compiled.states.len(), 40, "one state per node");
        assert!(compiled.states.contains(&"T[20]".to_string()));

        let result = compiled.simulate().unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let last = result.rows.last().unwrap();

        // Held ends, so the rod relaxes to a straight line between them.
        for node in 1..=40 {
            let position = node as f64 / 41.0;
            let expected = 100.0 + (20.0 - 100.0) * position;
            let actual = last[index(&format!("T[{node}]"))];
            assert!(
                (actual - expected).abs() < 1e-2,
                "node {node}: {actual} vs {expected}"
            );
        }
        // The inlined steadyState() function measures the same thing.
        assert!(last[index("midError")].abs() < 1e-2);
        // Cold start: the rod begins uniform.
        assert!((result.rows[0][index("T[1]")] - 20.0).abs() < 1e-12);
    }

    fn with_library(name: &str) -> oxidelica_parser::Model {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let library = std::fs::read_to_string(root.join("lib/Oxidelica.mo")).unwrap();
        let source = std::fs::read_to_string(root.join("examples").join(name)).unwrap();
        oxidelica_parser::parse_model_with_libraries(&[library], &source).unwrap()
    }

    #[test]
    fn dc_motor_from_library_components_matches_theory() {
        // Three domains at once: an electrical circuit, the EMF
        // coupling and a rotational load, all from library packages.
        let result = compile(&with_library("dc_motor.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let last = result.rows.last().unwrap();

        // Steady state: k*i = d*w and V = i*R + k*w.
        let (v, r, k, d) = (24.0, 0.5, 0.3, 0.02);
        let speed = v * k / (k * k + d * r);
        assert!(
            (last[index("speed")] - speed).abs() < 1e-2,
            "speed {} vs {speed}",
            last[index("speed")]
        );
        assert!((last[index("current")] - d * speed / k).abs() < 1e-3);

        // The supply steps at t = 0.1, so nothing moves before that.
        let early = result
            .rows
            .iter()
            .find(|row| row[0] >= 0.05)
            .expect("a sample before the step");
        assert!(early[index("speed")].abs() < 1e-9);
    }

    #[test]
    fn pi_control_loop_from_library_blocks_settles_on_the_setpoint() {
        let result = compile(&with_library("control_loop.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let last = result.rows.last().unwrap();
        // Integral action removes the steady-state error.
        assert!(
            last[index("e")].abs() < 1e-4,
            "residual error {}",
            last[index("e")]
        );
        assert!((last[index("y")] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn results_carry_the_parameter_values() {
        // Consumers such as the 3D view read sizes and colours from
        // here, since those never appear as columns.
        let result = run(
            "model P parameter Real k = 2.5; parameter Real m = 4; Real y; \
             equation y = k * m * time; \
             annotation(experiment(StopTime=1.0, Interval=0.5)); end P;",
        );
        assert_eq!(
            result.parameters,
            vec![("k".to_string(), 2.5), ("m".to_string(), 4.0)]
        );
        assert!((result.rows.last().unwrap()[1] - 10.0).abs() < 1e-12);
    }

    #[test]
    fn time_is_available_in_equations() {
        let result = run("model T Real y; equation y = 2 * time; \
             annotation(experiment(StopTime=1.0, Interval=0.5)); end T;");
        let last = result.rows.last().unwrap();
        assert!((last[1] - 2.0).abs() < 1e-12);
    }
}
