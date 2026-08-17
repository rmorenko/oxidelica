//! oxidelica-sim — compiles a flat M0 model into an explicit ODE system
//! and integrates it with a fixed-step RK4 solver.
//!
//! M0 limitations (lifted in M1+): state equations must be explicit
//! (`der(x) = f(...)` or mirrored), algebraic equations must be
//! assignments (`y = g(...)`) without cyclic dependencies.

#![deny(missing_docs)]

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
    /// Integration step.
    pub step: f64,
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

// --- integration ---

/// Simulation output: a table of time, states and algebraic variables.
#[derive(Debug)]
pub struct SimResult {
    /// Column headers: time, states, algebraics.
    pub columns: Vec<String>,
    /// One row per output point.
    pub rows: Vec<Vec<f64>>,
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

    /// Classic fixed-step RK4 integration over `[0, stop_time]`.
    pub fn simulate(&self) -> Result<SimResult, SimError> {
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

        for i in 0..steps {
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
        }

        Ok(SimResult { columns, rows })
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
             annotation(experiment(StopTime=5.0, Interval=0.001)); end D;");
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
             annotation(experiment(StopTime=10.0, Interval=0.001)); end P;");
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
             annotation(experiment(StopTime=1.0, Interval=0.001)); end M;");
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
    fn time_is_available_in_equations() {
        let result = run("model T Real y; equation y = 2 * time; \
             annotation(experiment(StopTime=1.0, Interval=0.5)); end T;");
        let last = result.rows.last().unwrap();
        assert!((last[1] - 2.0).abs() < 1e-12);
    }
}
