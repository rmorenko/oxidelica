//! oxidelica-sim — compiles a flat M0 model into an explicit ODE system
//! and integrates it with a fixed-step RK4 solver.
//!
//! M0 limitations (lifted in M1+): state equations must be explicit
//! (`der(x) = f(...)` or mirrored), algebraic equations must be
//! assignments (`y = g(...)`) without cyclic dependencies.

#![deny(missing_docs)]

use oxidelica_parser::ast::Termination;
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
    algebraic_exprs: Vec<Expr>,
    /// Simulation end time.
    pub stop_time: f64,
    /// Integration step (fixed step for RK4, output grid for Dopri45).
    pub step: f64,
    /// Relative tolerance for the adaptive solver.
    pub tolerance: f64,
    /// Selected integration method.
    pub method: SolverMethod,
    /// Termination clauses checked at every output point.
    pub terminations: Vec<Termination>,
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

    // 2. Equation classification: states vs algebraic assignments.
    let continuous: Vec<&str> = model
        .components
        .iter()
        .filter(|c| c.variability == Variability::Continuous)
        .map(|c| c.name.as_str())
        .collect();

    let mut state_rhs: HashMap<String, Expr> = HashMap::new();
    let mut alg_rhs: HashMap<String, Expr> = HashMap::new();

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
                return err("M0: der() must appear alone on one side of an equation".to_string());
            }
            if state_rhs.insert(state.to_string(), value.clone()).is_some() {
                return err(format!("two equations for der({state})"));
            }
            continue;
        }

        // Algebraic: v = expr | expr = v
        let (var, expr) = match (lhs, rhs) {
            (Expr::Ref(v), e) => (v, e),
            (e, Expr::Ref(v)) => (v, e),
            _ => {
                return err(format!(
                    "M0 requires explicit equations (v = ... or der(v) = ...): {lhs:?} = {rhs:?}"
                ))
            }
        };
        if expr.contains_der() {
            return err("M0: der() in an algebraic equation is not supported".to_string());
        }
        if alg_rhs.insert(var.clone(), expr.clone()).is_some() {
            return err(format!("two equations for {var}"));
        }
    }

    // 3. Every continuous variable must be determined in exactly one way.
    let mut states: Vec<String> = Vec::new();
    let mut algebraic_names: Vec<String> = Vec::new();
    for name in &continuous {
        let is_state = state_rhs.contains_key(*name);
        let is_alg = alg_rhs.contains_key(*name);
        match (is_state, is_alg) {
            (true, true) => return err(format!("{name}: both a state and an algebraic variable")),
            (true, false) => states.push((*name).to_string()),
            (false, true) => algebraic_names.push((*name).to_string()),
            (false, false) => return err(format!("no equation for variable {name}")),
        }
    }
    let unknown_eq: Vec<&String> = state_rhs
        .keys()
        .chain(alg_rhs.keys())
        .filter(|k| !continuous.contains(&k.as_str()))
        .collect();
    if !unknown_eq.is_empty() {
        return err(format!(
            "equations for undeclared variables: {unknown_eq:?}"
        ));
    }

    // 4. Topological sort of algebraic assignments.
    let ordered_algs = topo_sort(&algebraic_names, &alg_rhs)?;

    // 5. Initial state values.
    let ctx = EvalCtx {
        vars: &params,
        time: 0.0,
    };
    let mut initial = Vec::new();
    for s in &states {
        let comp = model.components.iter().find(|c| &c.name == s).unwrap();
        let value = match &comp.start {
            Some(expr) => eval(expr, &ctx).map_err(|e| SimError(format!("start of {s}: {e}")))?,
            None => 0.0,
        };
        initial.push(value);
    }

    let derivatives = states.iter().map(|s| state_rhs[s].clone()).collect();
    let algebraic_exprs = ordered_algs.iter().map(|a| alg_rhs[a].clone()).collect();

    let mut parameters: Vec<(String, f64)> = params.into_iter().collect();
    parameters.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(CompiledModel {
        name: model.name.clone(),
        parameters,
        states,
        initial,
        derivatives,
        algebraics: ordered_algs,
        algebraic_exprs,
        stop_time: model.experiment.stop_time.unwrap_or(1.0),
        step: model.experiment.interval.unwrap_or(1e-3),
        tolerance: model.experiment.tolerance.unwrap_or(1e-6),
        method: SolverMethod::default(),
        terminations: model.terminations.clone(),
    })
}

fn topo_sort(names: &[String], exprs: &HashMap<String, Expr>) -> Result<Vec<String>, SimError> {
    let mut ordered = Vec::new();
    let mut done: Vec<&str> = Vec::new();
    let mut remaining: Vec<&String> = names.iter().collect();
    while !remaining.is_empty() {
        let before = remaining.len();
        remaining.retain(|name| {
            let mut refs = Vec::new();
            exprs[*name].collect_refs(&mut refs);
            let ready = refs
                .iter()
                .all(|r| done.contains(r) || !names.iter().any(|n| n == r));
            if ready {
                ordered.push((*name).clone());
                done.push(name.as_str());
                false
            } else {
                true
            }
        });
        if remaining.len() == before {
            let cycle: Vec<_> = remaining.iter().map(|s| s.as_str()).collect();
            return Err(SimError(format!(
                "cyclic dependency among algebraic variables {cycle:?} (M1: implicit systems)"
            )));
        }
    }
    Ok(ordered)
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

impl CompiledModel {
    /// Evaluate termination clauses at an output point; the `env` must
    /// already hold every variable (as after `eval_point`). Returns the
    /// report line of the first clause that holds.
    fn check_terminations(
        &self,
        t: f64,
        env: &HashMap<String, f64>,
    ) -> Result<Option<String>, SimError> {
        for clause in &self.terminations {
            let ctx = EvalCtx { vars: env, time: t };
            if eval(&clause.condition, &ctx)? != 0.0 {
                return Ok(Some(format!(
                    "terminated at t = {t:.6}: {}",
                    clause.message
                )));
            }
        }
        Ok(None)
    }
}

// --- integration ---

/// Simulation output: a table of time, states and algebraic variables.
#[derive(Debug)]
pub struct SimResult {
    /// Column headers: time, states, algebraics.
    pub columns: Vec<String>,
    /// One row per output point.
    pub rows: Vec<Vec<f64>>,
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
    ) -> Result<(), SimError> {
        env.clear();
        for (name, value) in &self.parameters {
            env.insert(name.clone(), *value);
        }
        for (name, value) in self.states.iter().zip(y) {
            env.insert(name.clone(), *value);
        }
        for (name, expr) in self.algebraics.iter().zip(&self.algebraic_exprs) {
            let value = eval(expr, &EvalCtx { vars: env, time: t })?;
            env.insert(name.clone(), value);
        }
        derivatives_out.clear();
        for expr in &self.derivatives {
            derivatives_out.push(eval(expr, &EvalCtx { vars: env, time: t })?);
        }
        Ok(())
    }

    /// Integrate over `[0, stop_time]` with the selected method.
    pub fn simulate(&self) -> Result<SimResult, SimError> {
        match self.method {
            SolverMethod::Dopri45 => self.simulate_adaptive(),
            SolverMethod::Rk4 => self.simulate_rk4(),
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
                          k: &mut Vec<f64>|
         -> Result<(), SimError> {
            self.eval_point(t, y, env, k)?;
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
        let mut last_out_t = 0.0f64;
        let mut terminated: Option<String> = None;
        record(0.0, &y, &mut env, &mut derivatives_scratch)?;
        if let Some(message) = self.check_terminations(0.0, &env)? {
            return Ok(SimResult {
                columns,
                rows,
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
                record(t, &y, &mut env, &mut derivatives_scratch)?;
                last_out_t = t;
                out_i += 1;
                terminated = self.check_terminations(t, &env)?;
                if terminated.is_some() {
                    break;
                }
            }
            if terminated.is_none() && last_out_t < stop - 1e-12 {
                record(stop, &y, &mut env, &mut derivatives_scratch)?;
            }
            return Ok(SimResult {
                columns,
                rows,
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

        self.eval_point(t, &y, &mut env, &mut k[0])?;

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
                self.eval_point(t + C[s] * h, &stage, &mut env, &mut tail[0])?;
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
                self.eval_point(t + h, &y5, &mut env, &mut k[6])?;
                evals += 1;
                // Dense output on the grid via cubic Hermite interpolation.
                loop {
                    let out_t = out_i as f64 * out_step;
                    if out_t > t + h + 1e-12 || out_t > stop + 1e-12 {
                        break;
                    }
                    let theta = ((out_t - t) / h).clamp(0.0, 1.0);
                    for j in 0..n {
                        let (y0, y1) = (y[j], y5[j]);
                        let (f0, f1) = (k[0][j], k[6][j]);
                        interp[j] = (1.0 - theta) * y0
                            + theta * y1
                            + theta
                                * (theta - 1.0)
                                * ((1.0 - 2.0 * theta) * (y1 - y0)
                                    + (theta - 1.0) * h * f0
                                    + theta * h * f1);
                    }
                    record(out_t, &interp, &mut env, &mut derivatives_scratch)?;
                    last_out_t = out_t;
                    out_i += 1;
                    terminated = self.check_terminations(out_t, &env)?;
                    if terminated.is_some() {
                        break;
                    }
                }
                if terminated.is_some() {
                    break;
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
            record(stop, &y, &mut env, &mut derivatives_scratch)?;
            terminated = self.check_terminations(stop, &env)?;
        }
        Ok(SimResult {
            columns,
            rows,
            terminated,
        })
    }

    /// Classic fixed-step RK4 integration over `[0, stop_time]`.
    pub fn simulate_rk4(&self) -> Result<SimResult, SimError> {
        let n = self.states.len();
        let steps = (self.stop_time / self.step).ceil() as usize;
        let mut y = self.initial.clone();
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
                          this: &CompiledModel|
         -> Result<(), SimError> {
            this.eval_point(t, y, env, k)?;
            let mut row = Vec::with_capacity(1 + this.states.len() + this.algebraics.len());
            row.push(t);
            row.extend_from_slice(y);
            for name in &this.algebraics {
                row.push(env[name]);
            }
            rows.push(row);
            Ok(())
        };

        record(0.0, &y, &mut env, &mut k1, self)?;
        let mut terminated = self.check_terminations(0.0, &env)?;

        for i in 0..steps {
            if terminated.is_some() {
                break;
            }
            let t = i as f64 * self.step;
            let h = (self.stop_time - t).min(self.step);

            self.eval_point(t, &y, &mut env, &mut k1)?;
            for j in 0..n {
                scratch[j] = y[j] + 0.5 * h * k1[j];
            }
            self.eval_point(t + 0.5 * h, &scratch, &mut env, &mut k2)?;
            for j in 0..n {
                scratch[j] = y[j] + 0.5 * h * k2[j];
            }
            self.eval_point(t + 0.5 * h, &scratch, &mut env, &mut k3)?;
            for j in 0..n {
                scratch[j] = y[j] + h * k3[j];
            }
            self.eval_point(t + h, &scratch, &mut env, &mut k4)?;
            for j in 0..n {
                y[j] += h / 6.0 * (k1[j] + 2.0 * k2[j] + 2.0 * k3[j] + k4[j]);
            }
            record(t + h, &y, &mut env, &mut k1, self)?;
            terminated = self.check_terminations(t + h, &env)?;
        }

        Ok(SimResult {
            columns,
            rows,
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
        assert!(error.0.contains("no equation"), "{}", error.0);
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
    fn reports_algebraic_cycle() {
        let model =
            parse_model("model C Real x; Real y; equation x = y + 1; y = x - 1; end C;").unwrap();
        let error = compile(&model).unwrap_err();
        assert!(error.0.contains("cyclic"), "{}", error.0);
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
        // Two equations for an algebraic variable.
        assert!(
            compile_err("model M Real y; equation y = 1; y = 2; end M;").contains("two equations")
        );
        // Both a state and an algebraic variable.
        assert!(
            compile_err("model M Real x; equation der(x) = 1; x = 2; end M;")
                .contains("both a state and an algebraic")
        );
        // Implicit equation.
        assert!(
            compile_err("model M Real x; Real y; equation x + y = 2; y = 1; end M;")
                .contains("explicit equations")
        );
        // der inside an algebraic expression.
        assert!(
            compile_err("model M Real x; Real y; equation der(x) = 1; y = der(x) + 1; end M;")
                .contains("algebraic")
        );
        // Equation for an undeclared variable.
        assert!(compile_err("model M Real x; equation x = 1; q = 2; end M;").contains("undeclared"));
        // Error in a start expression.
        assert!(
            compile_err("model M Real x(start = q); equation der(x) = 1; end M;")
                .contains("start of x")
        );
    }

    #[test]
    fn runtime_error_paths() {
        // Unknown variable in an expression.
        assert!(
            simulate_err("model M Real y; equation y = z + 1; end M;").contains("unknown variable")
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
    fn time_is_available_in_equations() {
        let result = run("model T Real y; equation y = 2 * time; \
             annotation(experiment(StopTime=1.0, Interval=0.5)); end T;");
        let last = result.rows.last().unwrap();
        assert!((last[1] - 2.0).abs() < 1e-12);
    }
}
