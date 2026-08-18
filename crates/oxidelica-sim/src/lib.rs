//! oxidelica-sim — compiles a flat model into an executable form and
//! integrates it (adaptive Dormand-Prince by default, RK4 optional).
//!
//! State equations must isolate `der(x)` on one side; algebraic
//! equations are general — a bipartite matcher pairs them with
//! unknowns, explicit assignments evaluate directly and simultaneous
//! blocks are solved by Newton iteration.

#![deny(missing_docs)]

use oxidelica_parser::{
    BinOp, EquationItem, Expr, Model, RelOp, Variability, WhenAction, WhenBranch, WhenClause,
};
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
    /// Start explicit and watch: a problem whose step size is limited by
    /// stability rather than accuracy is handed to the implicit solver
    /// instead (default).
    #[default]
    Auto,
    /// Adaptive Dormand-Prince 5(4) with error control.
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
            "auto" => Some(SolverMethod::Auto),
            "dopri" | "dopri45" => Some(SolverMethod::Dopri45),
            "rk4" => Some(SolverMethod::Rk4),
            "bdf" => Some(SolverMethod::Bdf),
            _ => None,
        }
    }

    /// Short name of the method.
    pub fn name(self) -> &'static str {
        match self {
            SolverMethod::Auto => "auto",
            SolverMethod::Dopri45 => "dopri45",
            SolverMethod::Rk4 => "rk4",
            SolverMethod::Bdf => "bdf",
        }
    }
}

/// One step of the algebraic evaluation plan, in the form the compiler
/// builds it: expressions with names.
#[derive(Debug)]
enum PlanStage {
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

/// One step of the plan as a run executes it: the same shape with the
/// names resolved to slots and the expressions compiled.
#[derive(Debug)]
enum AlgStage {
    /// An explicit assignment into a slot.
    Explicit {
        /// Index of the assigned variable in `algebraics`.
        var: usize,
        /// What to evaluate.
        code: Code,
    },
    /// A simultaneous block, iterated on the torn unknowns.
    Implicit {
        /// All unknowns of the block (indices into `algebraics`).
        vars: Vec<usize>,
        /// Torn unknowns Newton iterates on.
        torn: Vec<usize>,
        /// Inner explicit assignments in evaluation order.
        inner: Vec<(usize, Code)>,
        /// Residuals matched to the torn unknowns.
        residuals: Vec<(Code, Code)>,
    },
}

/// A `when` clause with its conditions and targets resolved.
#[derive(Debug)]
struct CompiledWhen {
    /// Branches in source order; `elsewhen` gives them priority.
    branches: Vec<CompiledBranch>,
}

/// One branch of a compiled `when`.
#[derive(Debug)]
struct CompiledBranch {
    /// Fires on the false-to-true edge of this.
    condition: Code,
    /// What it does when it fires.
    actions: Vec<CompiledAction>,
}

/// What a branch does, with names already turned into positions.
#[derive(Debug)]
enum CompiledAction {
    /// End the run with a message.
    Terminate(String),
    /// Restart a state, by index, from a new value.
    Reinit(usize, Code),
    /// Give a discrete variable, by index, a new value.
    Assign(usize, Code),
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
    /// Right-hand side of each state, compiled.
    derivatives: Vec<Code>,
    /// The value array as a run starts it: parameters in place, the
    /// rest zero.
    values_template: Vec<f64>,
    /// Slot of each state, algebraic and discrete variable, in the order
    /// of the lists that name them.
    state_slots: Vec<Slot>,
    /// See [`CompiledModel::state_slots`].
    algebraic_slots: Vec<Slot>,
    /// The algebraics worth reporting, with their slots: the dummy
    /// derivatives of demoted states are solver bookkeeping whose very
    /// names change with the selection, so they stay out of the rows.
    output_algebraics: Vec<(String, Slot)>,
    /// See [`CompiledModel::state_slots`].
    discrete_slots: Vec<Slot>,
    /// Slot holding the previous value of each discrete variable.
    pre_slots: Vec<Slot>,
    /// Slot of the flag raised during the initial event.
    initial_slot: Slot,
    /// Slot of the flag raised by each `sample(...)` source.
    sample_slots: Vec<Slot>,
    /// Algebraic variables in evaluation order.
    pub algebraics: Vec<String>,
    /// Initial Newton guesses for algebraic variables (start attributes).
    algebraic_start: Vec<f64>,
    /// Algebraic variables declared `fixed = true`: their start value is
    /// an initial condition, not a guess, so the solution must match it.
    fixed_starts: Vec<(String, usize, f64)>,
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
    /// Where integration begins. Zero for a fresh run; a continuation
    /// after a state re-selection starts where the last segment stopped.
    start_time: f64,
    /// Whether this is a continuation: the initial event has already
    /// happened and `when` conditions resume from their current truth.
    resume: bool,
    /// Whether index reduction demoted any state. Only then can a
    /// stalled run be rescued by re-selecting the states.
    reselectable: bool,
    /// Sensitivity of each reduced constraint to its chosen victim and
    /// to the alternatives; see [`CompiledModel::selection_sound`].
    selection_monitor: Vec<(Code, Vec<Code>)>,
    /// `assert` conditions with their messages: each must hold at every
    /// recorded point, or the run stops and says which one did not.
    asserts: Vec<(Code, String)>,
    /// The flat model this was compiled from, kept so a continuation
    /// can compile itself at a new point.
    flat: Model,
    /// Groups of state indices whose Jacobian columns can be probed
    /// together: no two columns of a group touch the same row, so one
    /// evaluation yields all of them. Empty when the structure was not
    /// worked out, which falls back to one column at a time.
    jacobian_groups: Vec<Vec<usize>>,
    /// Rows each column touches, in the same order as the states.
    jacobian_rows: Vec<Vec<usize>>,
    /// How far from the diagonal the Jacobian reaches, when that is
    /// little enough to be worth solving as a band.
    jacobian_band: Option<usize>,
    /// Discrete variables: they keep their value between events, so the
    /// continuous part sees them as knowns and only a `when` clause
    /// changes them.
    pub discretes: Vec<String>,
    /// `(start, interval)` of every `sample(...)` in the model. The
    /// solver steps exactly onto each occurrence and raises the matching
    /// `$sample` flag there.
    samples: Vec<(f64, f64)>,
    /// `when` clauses, compiled: conditions and the values they assign,
    /// with every target already resolved to its place.
    when_clauses: Vec<CompiledWhen>,
    /// Event indicators: expressions whose sign change marks an event.
    /// Built from every relation in the model, so switching branches of
    /// an `if` expression are located exactly, not stepped over.
    indicators: Vec<Code>,
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
    fn solve_initialization(
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

/// Replace every `der(x)` with the right-hand side of that state.
fn substitute_derivatives(
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
fn jacobian_structure(
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

/// What the adaptive solver came back with: either the finished run, or
/// the verdict that this problem belongs to the implicit solver.
enum AdaptiveOutcome {
    /// The run completed.
    Finished(SimResult),
    /// The step size was being held down by stability, not accuracy.
    Stiff,
    /// The run cannot move past a point, and the model's states were
    /// chosen by a pivot that may simply no longer fit: the caller may
    /// re-select and continue from here.
    Stalled(Stall),
}

/// Where a run stopped making progress, with everything a continuation
/// needs.
struct Stall {
    /// The rows produced up to the stall, still worth keeping.
    partial: SimResult,
    /// The time reached.
    time: f64,
    /// The value of every variable there, by name.
    values: HashMap<String, f64>,
}

impl CompiledModel {
    /// Whether the state selection is still the right one at this point.
    ///
    /// A constraint must determine its demoted victim, so the victim's
    /// sensitivity has to stay comparable to the best alternative. Once
    /// it falls below a fraction of one, the pivot that made the choice
    /// would choose differently now - and it is asked to, while the
    /// algebraic layer is still far from singular. The margin is why
    /// the switch happens in clean territory rather than at the wall.
    fn selection_sound(&self, values: &[f64], time: f64) -> bool {
        self.selection_monitor.iter().all(|(own, alternatives)| {
            let own = own.run(values, time).abs();
            let best = alternatives
                .iter()
                .map(|code| code.run(values, time).abs())
                .fold(0.0f64, f64::max);
            own >= 0.15 * best
        })
    }

    /// The stall a segment reports: the point it resumes from is the
    /// last row it *recorded*, never a re-evaluation at the breakdown -
    /// the algebraic layer is exactly what cannot be trusted there.
    fn stall_at_last_row(
        &self,
        columns: Vec<String>,
        rows: Vec<Vec<f64>>,
        method: SolverMethod,
    ) -> Result<AdaptiveOutcome, SimError> {
        let Some(last) = rows.last() else {
            return err("the run stalled before producing a single point".to_string());
        };
        let mut values: HashMap<String, f64> = self.parameters.iter().cloned().collect();
        for (name, value) in columns.iter().zip(last).skip(1) {
            values.insert(name.clone(), *value);
        }
        let time = last[0];
        Ok(AdaptiveOutcome::Stalled(Stall {
            partial: SimResult {
                columns,
                rows,
                parameters: self.parameters.clone(),
                terminated: None,
                method,
                reselections: 0,
            },
            time,
            values,
        }))
    }
}

/// Glue a continuation onto the rows already produced.
///
/// The variable set is identical between segments but the column order
/// is not: the states of each selection come first in its own rows, so
/// a continuation's rows are reordered into the first segment's layout
/// by name before they are appended.
fn append_segment(mut merged: SimResult, segment: SimResult) -> SimResult {
    let mapping: Vec<usize> = merged
        .columns
        .iter()
        .map(|name| {
            segment
                .columns
                .iter()
                .position(|other| other == name)
                .expect("both segments name the same variables")
        })
        .collect();
    for row in &segment.rows {
        merged
            .rows
            .push(mapping.iter().map(|&from| row[from]).collect());
    }
    merged.terminated = segment.terminated;
    merged.method = segment.method;
    merged
}

/// What the event machinery carries between events.
#[derive(Clone, Debug)]
struct EventState {
    /// Truth of every `when` branch as of the previous event.
    when_prev: Vec<Vec<bool>>,
    /// Next occurrence of each `sample(...)` source.
    next_sample: Vec<f64>,
}

impl EventState {
    /// The next scheduled time event, if the model has any.
    fn next_time_event(&self) -> Option<f64> {
        self.next_sample.iter().copied().reduce(f64::min)
    }

    /// Raise the flag of every `sample(...)` occurring at `t` and move
    /// its schedule on.
    fn raise_samples(
        &mut self,
        t: f64,
        samples: &[(f64, f64)],
        flags: &[Slot],
        values: &mut [f64],
    ) {
        for (index, (_, interval)) in samples.iter().enumerate() {
            if self.next_sample[index] <= t + 1e-9 {
                values[flags[index]] = 1.0;
                self.next_sample[index] += interval.max(1e-12);
            }
        }
    }
}

/// Rewrites the event built-ins into plain references the evaluator can
/// look up, collecting the `sample(...)` schedules on the way:
/// `pre(x)` becomes `$pre.x`, `initial()` the flag of the initial event,
/// `sample(s, i)` the flag of a scheduled one, and `edge`/`change` their
/// definitions in terms of `pre`.
struct EventRewrite<'a> {
    /// Names of the discrete variables, the only ones `pre` accepts.
    discretes: &'a [String],
    /// Parameter values: the arguments of `sample` must be constant.
    params: &'a HashMap<String, f64>,
    /// Schedules found so far, in flag order.
    samples: Vec<(f64, f64)>,
}

impl EventRewrite<'_> {
    /// The `$pre.` reference of a discrete variable.
    fn pre_of(&self, arg: &Expr, builtin: &str) -> Result<Expr, SimError> {
        let Expr::Ref(name) = arg else {
            return err(format!("{builtin}() takes a variable, not an expression"));
        };
        if !self.discretes.iter().any(|d| d == name) {
            return err(format!(
                "{builtin}({name}): `{name}` is not discrete, so it has no value from before the event"
            ));
        }
        Ok(Expr::Ref(format!("$pre.{name}")))
    }

    /// Rewrite one expression.
    fn expr(&mut self, expr: &Expr) -> Result<Expr, SimError> {
        Ok(match expr {
            Expr::Call(name, args) => match (name.as_str(), args.len()) {
                ("pre", 1) => self.pre_of(&args[0], "pre")?,
                // edge(b) is "b just became true", change(v) is "v just
                // took a different value".
                ("edge", 1) => Expr::And(
                    Box::new(Expr::Rel(
                        RelOp::Gt,
                        Box::new(args[0].clone()),
                        Box::new(Expr::Number(0.5)),
                    )),
                    Box::new(Expr::Rel(
                        RelOp::Lt,
                        Box::new(self.pre_of(&args[0], "edge")?),
                        Box::new(Expr::Number(0.5)),
                    )),
                ),
                ("change", 1) => Expr::Rel(
                    RelOp::Ne,
                    Box::new(args[0].clone()),
                    Box::new(self.pre_of(&args[0], "change")?),
                ),
                ("initial", 0) => Expr::Ref("$initial".to_string()),
                ("sample", 2) => {
                    let ctx = EvalCtx {
                        vars: self.params,
                        time: 0.0,
                    };
                    let start = eval(&args[0], &ctx)?;
                    let interval = eval(&args[1], &ctx)?;
                    if interval <= 0.0 || interval.is_nan() {
                        return err(format!(
                            "sample(..., {interval}): the interval must be positive"
                        ));
                    }
                    let index = match self
                        .samples
                        .iter()
                        .position(|&(s, i)| s == start && i == interval)
                    {
                        Some(index) => index,
                        None => {
                            self.samples.push((start, interval));
                            self.samples.len() - 1
                        }
                    };
                    Expr::Ref(format!("$sample{index}"))
                }
                _ => Expr::Call(
                    name.clone(),
                    args.iter()
                        .map(|a| self.expr(a))
                        .collect::<Result<Vec<_>, SimError>>()?,
                ),
            },
            Expr::Neg(inner) => Expr::Neg(Box::new(self.expr(inner)?)),
            Expr::Not(inner) => Expr::Not(Box::new(self.expr(inner)?)),
            Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(self.expr(l)?), Box::new(self.expr(r)?)),
            Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(self.expr(l)?), Box::new(self.expr(r)?)),
            Expr::And(l, r) => Expr::And(Box::new(self.expr(l)?), Box::new(self.expr(r)?)),
            Expr::Or(l, r) => Expr::Or(Box::new(self.expr(l)?), Box::new(self.expr(r)?)),
            Expr::If(c, a, b) => Expr::If(
                Box::new(self.expr(c)?),
                Box::new(self.expr(a)?),
                Box::new(self.expr(b)?),
            ),
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
            | Expr::Tuple(_) => {
                return err("subscripts and arrays survive flattening only as scalars".to_string())
            }
            Expr::Ref(_) | Expr::Number(_) | Expr::Bool(_) | Expr::Time => expr.clone(),
        })
    }
}

/// Compile a parsed flat model into an executable form.
pub fn compile(model: &Model) -> Result<CompiledModel, SimError> {
    compile_at(model, None)
}

/// A point a continuation starts from: the time reached and the value
/// of every variable there, by name.
struct ResumePoint<'a> {
    /// Where the previous segment stopped.
    time: f64,
    /// Values of every variable at that instant.
    values: &'a HashMap<String, f64>,
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
fn compile_at(model: &Model, resume: Option<ResumePoint>) -> Result<CompiledModel, SimError> {
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

    // The event built-ins become references the evaluator can look up.
    let mut rewrite = EventRewrite {
        discretes: &discretes,
        params: &params,
        samples: Vec::new(),
    };
    let equations: Vec<EquationItem> = model
        .equations
        .iter()
        .map(|equation| {
            Ok(EquationItem {
                lhs: rewrite.expr(&equation.lhs)?,
                rhs: rewrite.expr(&equation.rhs)?,
            })
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

    // 2. Split equations: explicit state derivatives vs general
    // algebraic equations (which need not be in assignment form).
    let continuous: Vec<&str> = model
        .components
        .iter()
        .filter(|c| c.variability == Variability::Continuous && !discretes.contains(&c.name))
        .map(|c| c.name.as_str())
        .collect();

    let mut state_rhs: HashMap<String, Expr> = HashMap::new();
    let mut algebraic_eqs: Vec<(Expr, Expr)> = Vec::new();

    for EquationItem { lhs, rhs } in &equations {
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
    let resumed = |name: &str| -> Option<f64> {
        resume
            .as_ref()
            .and_then(|point| point.values.get(name))
            .copied()
    };
    let start_env: HashMap<String, f64> = {
        let mut env = params.clone();
        for (name, value) in discretes.iter().zip(&discrete_start) {
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
                                    vars: &params,
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
    };

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
        | Expr::Tuple(_) => expr.clone(),
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
            // The staircase functions are flat almost everywhere.
            if matches!(name.as_str(), "ceil" | "floor" | "integer" | "sign") {
                return Ok(Expr::Number(0.0));
            }
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

/// Where a variable sits in the value array a run carries.
type Slot = usize;

/// The unary functions a model may call, resolved while compiling so
/// that evaluating one is a jump rather than a string comparison.
#[derive(Debug, Clone, Copy)]
enum Unary {
    Ceil,
    Floor,
    IntegerPart,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Exp,
    Log,
    Log10,
    Sqrt,
    Abs,
    Sign,
}

/// The two-argument ones.
#[derive(Debug, Clone, Copy)]
enum Binary {
    Atan2,
    Min,
    Max,
    Div,
    Mod,
    Rem,
}

/// An expression whose variables are already resolved to slots.
///
/// The same shape as [`Expr`] with the names taken out: evaluating it
/// indexes an array instead of hashing a string at every leaf, and the
/// function of a call is decided once instead of at every evaluation.
/// This is what the solvers run, thousands of times per second; `Expr`
/// stays the form the compiler reasons about.
#[derive(Debug, Clone)]
enum Code {
    /// A literal.
    Const(f64),
    /// The value in a slot.
    Slot(Slot),
    /// The independent variable.
    Time,
    /// Unary minus.
    Neg(Box<Code>),
    /// Logical negation.
    Not(Box<Code>),
    /// Arithmetic.
    Bin(BinOp, Box<Code>, Box<Code>),
    /// Comparison, giving 1.0 or 0.0.
    Rel(RelOp, Box<Code>, Box<Code>),
    /// Conjunction and disjunction, both short-circuiting.
    And(Box<Code>, Box<Code>),
    /// See [`Code::And`].
    Or(Box<Code>, Box<Code>),
    /// Conditional; only the branch taken is evaluated.
    If(Box<Code>, Box<Code>, Box<Code>),
    /// A one-argument built-in.
    Unary(Unary, Box<Code>),
    /// A two-argument built-in.
    Binary(Binary, Box<Code>, Box<Code>),
}

impl Code {
    /// Evaluate against the value array of a run. Nothing here can
    /// fail: unknown names and unknown functions were rejected while
    /// compiling, which is why this returns a number rather than a
    /// result.
    fn run(&self, values: &[f64], time: f64) -> f64 {
        match self {
            Code::Const(value) => *value,
            Code::Slot(slot) => values[*slot],
            Code::Time => time,
            Code::Neg(inner) => -inner.run(values, time),
            Code::Not(inner) => truth(inner.run(values, time) == 0.0),
            Code::Bin(op, l, r) => {
                let (a, b) = (l.run(values, time), r.run(values, time));
                match op {
                    BinOp::Add => a + b,
                    BinOp::Sub => a - b,
                    BinOp::Mul => a * b,
                    BinOp::Div => a / b,
                    BinOp::Pow => a.powf(b),
                }
            }
            Code::Rel(op, l, r) => {
                let (a, b) = (l.run(values, time), r.run(values, time));
                truth(match op {
                    RelOp::Lt => a < b,
                    RelOp::Le => a <= b,
                    RelOp::Gt => a > b,
                    RelOp::Ge => a >= b,
                    RelOp::Eq => a == b,
                    RelOp::Ne => a != b,
                })
            }
            Code::And(l, r) => truth(l.run(values, time) != 0.0 && r.run(values, time) != 0.0),
            Code::Or(l, r) => truth(l.run(values, time) != 0.0 || r.run(values, time) != 0.0),
            Code::If(condition, then, otherwise) => {
                if condition.run(values, time) != 0.0 {
                    then.run(values, time)
                } else {
                    otherwise.run(values, time)
                }
            }
            Code::Unary(function, argument) => {
                let x = argument.run(values, time);
                match function {
                    Unary::Ceil => x.ceil(),
                    Unary::Floor => x.floor(),
                    // integer(x) truncates toward negative infinity,
                    // like floor - the spec defines it that way.
                    Unary::IntegerPart => x.floor(),
                    Unary::Sin => x.sin(),
                    Unary::Cos => x.cos(),
                    Unary::Tan => x.tan(),
                    Unary::Asin => x.asin(),
                    Unary::Acos => x.acos(),
                    Unary::Atan => x.atan(),
                    Unary::Sinh => x.sinh(),
                    Unary::Cosh => x.cosh(),
                    Unary::Tanh => x.tanh(),
                    Unary::Exp => x.exp(),
                    Unary::Log => x.ln(),
                    Unary::Log10 => x.log10(),
                    Unary::Sqrt => x.sqrt(),
                    Unary::Abs => x.abs(),
                    Unary::Sign => x.signum(),
                }
            }
            Code::Binary(function, l, r) => {
                let (a, b) = (l.run(values, time), r.run(values, time));
                match function {
                    Binary::Atan2 => a.atan2(b),
                    Binary::Min => a.min(b),
                    Binary::Max => a.max(b),
                    // Integer division truncates toward zero; mod and
                    // rem follow their spec definitions from it.
                    Binary::Div => (a / b).trunc(),
                    Binary::Mod => a - (a / b).floor() * b,
                    Binary::Rem => a - (a / b).trunc() * b,
                }
            }
        }
    }
}

/// Booleans are carried as 1.0 and 0.0, like everywhere else here.
fn truth(yes: bool) -> f64 {
    if yes {
        1.0
    } else {
        0.0
    }
}

/// Names of the variables of a model, each with the slot it occupies.
struct SlotTable {
    /// Slot of every known name.
    index: HashMap<String, Slot>,
    /// The value array as a run starts it: parameters already in place,
    /// everything else zero.
    template: Vec<f64>,
}

impl SlotTable {
    /// An empty table.
    fn new() -> SlotTable {
        SlotTable {
            index: HashMap::new(),
            template: Vec::new(),
        }
    }

    /// Give a name a slot, or return the one it already has.
    fn slot(&mut self, name: &str) -> Slot {
        if let Some(slot) = self.index.get(name) {
            return *slot;
        }
        let slot = self.template.len();
        self.template.push(0.0);
        self.index.insert(name.to_string(), slot);
        slot
    }

    /// Give a name a slot holding a value that never changes.
    fn constant(&mut self, name: &str, value: f64) -> Slot {
        let slot = self.slot(name);
        self.template[slot] = value;
        slot
    }

    /// Resolve an expression into code, refusing anything that names a
    /// variable or a function the model does not have.
    fn compile(&self, expr: &Expr) -> Result<Code, SimError> {
        Ok(match expr {
            Expr::Number(value) => Code::Const(*value),
            Expr::Bool(value) => Code::Const(truth(*value)),
            Expr::Time => Code::Time,
            Expr::Ref(name) => match self.index.get(name) {
                Some(slot) => Code::Slot(*slot),
                None => return err(format!("unknown variable `{name}`")),
            },
            Expr::Neg(inner) => Code::Neg(Box::new(self.compile(inner)?)),
            Expr::Not(inner) => Code::Not(Box::new(self.compile(inner)?)),
            Expr::Bin(op, l, r) => {
                Code::Bin(*op, Box::new(self.compile(l)?), Box::new(self.compile(r)?))
            }
            Expr::Rel(op, l, r) => {
                Code::Rel(*op, Box::new(self.compile(l)?), Box::new(self.compile(r)?))
            }
            Expr::And(l, r) => Code::And(Box::new(self.compile(l)?), Box::new(self.compile(r)?)),
            Expr::Or(l, r) => Code::Or(Box::new(self.compile(l)?), Box::new(self.compile(r)?)),
            Expr::If(condition, then, otherwise) => Code::If(
                Box::new(self.compile(condition)?),
                Box::new(self.compile(then)?),
                Box::new(self.compile(otherwise)?),
            ),
            Expr::Call(name, args) => self.compile_call(name, args)?,
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
            | Expr::Tuple(_) => {
                return err("subscripts and arrays survive flattening only as scalars".to_string())
            }
        })
    }

    /// The built-in behind a call, with its arity checked here rather
    /// than on every evaluation.
    fn compile_call(&self, name: &str, args: &[Expr]) -> Result<Code, SimError> {
        let unary = match name {
            "ceil" => Some(Unary::Ceil),
            "floor" => Some(Unary::Floor),
            "integer" => Some(Unary::IntegerPart),
            "sin" => Some(Unary::Sin),
            "cos" => Some(Unary::Cos),
            "tan" => Some(Unary::Tan),
            "asin" => Some(Unary::Asin),
            "acos" => Some(Unary::Acos),
            "atan" => Some(Unary::Atan),
            "sinh" => Some(Unary::Sinh),
            "cosh" => Some(Unary::Cosh),
            "tanh" => Some(Unary::Tanh),
            "exp" => Some(Unary::Exp),
            "log" => Some(Unary::Log),
            "log10" => Some(Unary::Log10),
            "sqrt" => Some(Unary::Sqrt),
            "abs" => Some(Unary::Abs),
            "sign" => Some(Unary::Sign),
            _ => None,
        };
        if let Some(function) = unary {
            if args.len() != 1 {
                return err(format!("{name}: expects 1 argument, got {}", args.len()));
            }
            return Ok(Code::Unary(function, Box::new(self.compile(&args[0])?)));
        }
        let binary = match name {
            "atan2" => Some(Binary::Atan2),
            "min" => Some(Binary::Min),
            "max" => Some(Binary::Max),
            "div" => Some(Binary::Div),
            "mod" => Some(Binary::Mod),
            "rem" => Some(Binary::Rem),
            _ => None,
        };
        if let Some(function) = binary {
            if args.len() != 2 {
                return err(format!("{name}: expects 2 arguments, got {}", args.len()));
            }
            return Ok(Code::Binary(
                function,
                Box::new(self.compile(&args[0])?),
                Box::new(self.compile(&args[1])?),
            ));
        }
        if name == "der" {
            return err("der() outside a state equation is not supported".to_string());
        }
        err(format!("unknown function `{name}`"))
    }
}

/// Booleans are represented as 1.0 / 0.0 (proper typing is an M1+ task).
fn eval(expr: &Expr, ctx: &EvalCtx) -> Result<f64, SimError> {
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
                "ceil" => {
                    arity(1)?;
                    vals[0].ceil()
                }
                "floor" | "integer" => {
                    arity(1)?;
                    vals[0].floor()
                }
                "div" => {
                    arity(2)?;
                    (vals[0] / vals[1]).trunc()
                }
                "mod" => {
                    arity(2)?;
                    vals[0] - (vals[0] / vals[1]).floor() * vals[1]
                }
                "rem" => {
                    arity(2)?;
                    vals[0] - (vals[0] / vals[1]).trunc() * vals[1]
                }
                other => return err(format!("unknown function `{other}`")),
            }
        }
        // Arrays are expanded into scalars while flattening, so one
        // here would mean the compiler let something through.
        Expr::Array(_)
        | Expr::Elementwise(_, _, _)
        | Expr::Range(_, _, _)
        | Expr::Comprehension(_, _, _)
        | Expr::ColonSubscript
        | Expr::EndSubscript
        | Expr::MatrixRows(_)
        | Expr::NamedArg(_, _)
        | Expr::Tuple(_) => return err("an array reached the evaluator".to_string()),
    })
}

/// Outcome of handling an event.
#[derive(Default)]
struct EventOutcome {
    /// Set when a `terminate(...)` fired.
    terminated: Option<String>,
    /// Whether any state was reinitialized (the integrator restarts).
    reinitialized: bool,
    /// Whether the event changed anything at all: a state through
    /// `reinit`, or the value of a discrete variable.
    changed: bool,
}

impl CompiledModel {
    /// Values of the event indicators at a point already evaluated.
    fn indicator_values(&self, t: f64, values: &[f64]) -> Vec<f64> {
        self.indicators
            .iter()
            .map(|code| code.run(values, t))
            .collect()
    }

    /// Truth of every `when` branch, clause by clause.
    fn when_conditions(&self, t: f64, values: &[f64]) -> Vec<Vec<bool>> {
        self.when_clauses
            .iter()
            .map(|clause| {
                clause
                    .branches
                    .iter()
                    .map(|branch| branch.condition.run(values, t) != 0.0)
                    .collect()
            })
            .collect()
    }

    /// The state every run starts from: the initial-event flag raised
    /// and every `sample(...)` scheduled at its start. Discrete values
    /// live in the value array, already carrying their declared starts.
    fn event_state(&self) -> EventState {
        EventState {
            // Modelica treats every condition as false before the start,
            // so one that already holds at t = 0 fires immediately. A
            // continuation instead resumes from the current truth, which
            // the solver fills in after its first evaluation.
            when_prev: self
                .when_clauses
                .iter()
                .map(|clause| vec![false; clause.branches.len()])
                .collect(),
            // On a continuation each clock resumes at its first tick
            // after the point reached; ticks behind it already fired.
            next_sample: self
                .samples
                .iter()
                .map(|&(start, interval)| {
                    if !self.resume || start > self.start_time + 1e-9 {
                        start
                    } else {
                        let periods = ((self.start_time - start) / interval + 1e-9).floor() + 1.0;
                        start + periods * interval
                    }
                })
                .collect(),
        }
    }

    /// Handle an event at `t`: fire the `when` branches whose condition
    /// just became true, and keep firing while their assignments make
    /// further conditions true — the event iteration of the language.
    ///
    /// `pre(x)` keeps the value each discrete variable had when the event
    /// began, however many rounds the iteration takes.
    fn handle_event(
        &self,
        t: f64,
        y: &mut [f64],
        values: &mut [f64],
        alg_guess: &mut [f64],
        state: &mut EventState,
    ) -> Result<EventOutcome, SimError> {
        let mut outcome = EventOutcome::default();
        for (&slot, &pre) in self.discrete_slots.iter().zip(&self.pre_slots) {
            values[pre] = values[slot];
        }
        let before_event = state.when_prev.clone();
        // A `reinit` is collected rather than applied: the event
        // iteration works on the discrete variables, and the new value
        // of a state is what the integration resumes from afterwards.
        let mut pending_reinit: Vec<(usize, f64)> = Vec::new();
        let mut fired: Vec<Vec<bool>> = before_event
            .iter()
            .map(|branches| vec![false; branches.len()])
            .collect();
        let mut scratch = Vec::new();

        // Every branch fires at most once per event, so one round per
        // branch plus a final quiet one is all the iteration can need.
        let rounds = self
            .when_clauses
            .iter()
            .map(|clause| clause.branches.len())
            .sum::<usize>()
            + 1;
        for _ in 0..rounds {
            // The algebraic part follows the discrete values, so it is
            // re-evaluated before the conditions are tested again.
            self.eval_point(t, y, values, &mut scratch, alg_guess)?;
            let now = self.when_conditions(t, values);
            let mut acted = false;
            for (index, clause) in self.when_clauses.iter().enumerate() {
                // `elsewhen` is a priority list: the first branch that
                // just became true is the one that fires.
                let Some(branch) = (0..clause.branches.len())
                    .find(|&b| now[index][b] && !before_event[index][b] && !fired[index][b])
                else {
                    continue;
                };
                fired[index][branch] = true;
                acted = true;
                for action in &clause.branches[branch].actions {
                    match action {
                        CompiledAction::Terminate(message) => {
                            outcome.terminated =
                                Some(format!("terminated at t = {t:.6}: {message}"));
                        }
                        CompiledAction::Reinit(state_index, code) => {
                            pending_reinit.push((*state_index, code.run(values, t)));
                            outcome.reinitialized = true;
                            outcome.changed = true;
                        }
                        CompiledAction::Assign(discrete_index, code) => {
                            let new = code.run(values, t);
                            let slot = self.discrete_slots[*discrete_index];
                            if values[slot] != new {
                                outcome.changed = true;
                            }
                            // Later equations of the same branch see the
                            // new value, the way a simultaneous solution
                            // of a triangular system would.
                            values[slot] = new;
                        }
                    }
                }
            }
            if !acted {
                break;
            }
        }
        // The one-shot flags go down with the event, so the conditions
        // remembered for the next one do not see them still raised - a
        // `sample(...)` must be a fresh edge every period.
        values[self.initial_slot] = 0.0;
        for &slot in &self.sample_slots {
            values[slot] = 0.0;
        }
        self.eval_point(t, y, values, &mut scratch, alg_guess)?;
        state.when_prev = self.when_conditions(t, values);
        for (index, value) in pending_reinit {
            y[index] = value;
        }
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
/// Solve a banded system by elimination without pivoting.
///
/// `matrix[i][j - i + band]` holds the entry at row `i`, column `j`, so
/// each row is `2 * band + 1` wide. Skipping the pivot search is what
/// keeps the band narrow, and it is sound for the matrices this is used
/// on: `I - h*c*J` of a diffusion-like system has a diagonal that
/// dominates. A pivot that turns out too small to trust returns `None`
/// and the caller falls back to the dense path.
fn solve_banded(matrix: &mut [Vec<f64>], band: usize, rhs: &[f64]) -> Option<Vec<f64>> {
    let n = rhs.len();
    let width = 2 * band + 1;
    let mut x = rhs.to_vec();
    for i in 0..n {
        let pivot = matrix[i][band];
        if pivot.abs() < 1e-12 {
            return None;
        }
        for r in (i + 1)..(i + band + 1).min(n) {
            let offset = i + band - r;
            let factor = matrix[r][offset] / pivot;
            if factor == 0.0 {
                continue;
            }
            for column in i..(i + band + 1).min(n) {
                let source = matrix[i][column + band - i];
                let target = column + band - r;
                if target < width {
                    matrix[r][target] -= factor * source;
                }
            }
            x[r] -= factor * x[i];
        }
    }
    for i in (0..n).rev() {
        let mut sum = x[i];
        for column in (i + 1)..(i + band + 1).min(n) {
            sum -= matrix[i][column + band - i] * x[column];
        }
        x[i] = sum / matrix[i][band];
    }
    if x.iter().any(|value| !value.is_finite()) {
        return None;
    }
    Some(x)
}

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
    /// The method that produced these rows. With `Auto` this is the one
    /// the run settled on, never `Auto` itself.
    pub method: SolverMethod,
    /// How many times the run re-selected its states mid-way. Zero for
    /// every model whose selection holds; a Cartesian pendulum swinging
    /// through the horizontal counts one per crossing.
    pub reselections: usize,
}

impl SimResult {
    /// Render the result as CSV text.
    pub fn to_csv(&self) -> String {
        use std::fmt::Write;
        // A large result is mostly numbers, so this writes them straight
        // into one buffer: a `format!` per cell would allocate a string
        // for every value, which on a big model costs more than the
        // simulation that produced them. The values are written at full
        // precision - shortest text that reads back as the same double -
        // rather than padded to a fixed number of decimals.
        let mut out = String::with_capacity(
            self.columns.iter().map(|c| c.len() + 1).sum::<usize>()
                + self.rows.len() * self.columns.len() * 12,
        );
        // A column name may itself hold a comma - `mm[1,2]` - so names
        // are quoted the way CSV quotes things when they need it.
        for (index, name) in self.columns.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            if name.contains(',') || name.contains('"') {
                out.push('"');
                out.push_str(&name.replace('"', "\"\""));
                out.push('"');
            } else {
                out.push_str(name);
            }
        }
        out.push('\n');
        for row in &self.rows {
            for (index, value) in row.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{value}");
            }
            out.push('\n');
        }
        out
    }
}

impl CompiledModel {
    /// Evaluate algebraic variables and derivatives at point (t, y).
    /// `env` is reused between calls to avoid per-step allocation.
    /// Check every `assert` at an evaluated point; a violated one stops
    /// the run with its own message and the time.
    fn check_asserts(&self, t: f64, values: &[f64]) -> Result<(), SimError> {
        for (condition, message) in &self.asserts {
            if condition.run(values, t) == 0.0 {
                return err(format!("assertion failed at t = {t:.6}: {message}"));
            }
        }
        Ok(())
    }

    fn eval_point(
        &self,
        t: f64,
        y: &[f64],
        values: &mut [f64],
        derivatives_out: &mut Vec<f64>,
        alg_guess: &mut [f64],
    ) -> Result<(), SimError> {
        // Parameters sit in the array from the start and discrete values
        // are written there by the event machinery, so a point only has
        // to place the states and run the plan.
        for (&slot, value) in self.state_slots.iter().zip(y) {
            values[slot] = *value;
        }
        for stage in &self.stages {
            match stage {
                AlgStage::Explicit { var, code } => {
                    values[self.algebraic_slots[*var]] = code.run(values, t);
                }
                stage @ AlgStage::Implicit { .. } => {
                    self.solve_implicit_block(t, values, stage, alg_guess, false)?;
                }
            }
        }
        derivatives_out.clear();
        for code in &self.derivatives {
            derivatives_out.push(code.run(values, t));
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
        values: &mut [f64],
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

        let residual = |values: &mut [f64], v: &[f64]| -> Vec<f64> {
            for (j, &index) in block.iter().enumerate() {
                values[self.algebraic_slots[index]] = v[j];
            }
            // Torn values fixed: the inner unknowns follow explicitly.
            for (var, code) in inner {
                values[self.algebraic_slots[*var]] = code.run(values, t);
            }
            residuals
                .iter()
                .map(|(lhs, rhs)| lhs.run(values, t) - rhs.run(values, t))
                .collect()
        };
        let block_names =
            || -> Vec<&str> { block.iter().map(|&i| self.algebraics[i].as_str()).collect() };

        for _ in 0..50 {
            let f = residual(values, &v);
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
                        let fp = residual(values, &perturbed);
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
                let fp = residual(values, &perturbed);
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

    /// Integrate over `[start_time, stop_time]` with the selected
    /// method, re-selecting the states and continuing whenever the
    /// current selection stalls the run.
    pub fn simulate(&self) -> Result<SimResult, SimError> {
        let mut outcome = self.run_segment()?;
        let mut merged: Option<SimResult> = None;
        let mut reselections = 0usize;
        let mut last_stall = f64::NEG_INFINITY;
        loop {
            match outcome {
                AdaptiveOutcome::Finished(result) => {
                    let mut result = match merged {
                        Some(merged) => append_segment(merged, result),
                        None => result,
                    };
                    result.reselections = reselections;
                    return Ok(result);
                }
                AdaptiveOutcome::Stiff => {
                    unreachable!("run_segment resolves the stiffness switch itself")
                }
                AdaptiveOutcome::Stalled(stall) => {
                    // A stall that made no ground since the last one is
                    // not a wrong selection but a genuine singularity.
                    if stall.time <= last_stall + 1e-12 || reselections >= 200 {
                        return err(format!(
                            "step size underflow at t = {:.6}: probable singularity                              (state re-selection did not help)",
                            stall.time
                        ));
                    }
                    last_stall = stall.time;
                    reselections += 1;
                    // Compile again at the point reached: the pivot now
                    // sees the sensitivities of this instant and picks
                    // the states that fit here.
                    let mut next = compile_at(
                        &self.flat,
                        Some(ResumePoint {
                            time: stall.time,
                            values: &stall.values,
                        }),
                    )?;
                    next.method = self.method;
                    merged = Some(match merged {
                        Some(merged) => append_segment(merged, stall.partial),
                        None => stall.partial,
                    });
                    outcome = next.run_segment()?;
                }
            }
        }
    }

    /// One integration attempt with the selected method; a stall comes
    /// back to `simulate` for a re-selection.
    ///
    /// With `Auto` the explicit solver goes first and watches the
    /// product of the step size and the dominant eigenvalue of the
    /// Jacobian, which it gets for free from two stages it has already
    /// evaluated. A step size held down by stability rather than by
    /// accuracy is what "stiff" means, and once that is clear the run
    /// starts again with the implicit solver.
    fn run_segment(&self) -> Result<AdaptiveOutcome, SimError> {
        match self.method {
            SolverMethod::Auto => match self.adaptive(true)? {
                AdaptiveOutcome::Stiff => self.run_bdf(),
                outcome => Ok(outcome),
            },
            SolverMethod::Dopri45 => match self.adaptive(false)? {
                AdaptiveOutcome::Stiff => err("the stiffness watch fired without being asked"),
                outcome => Ok(outcome),
            },
            SolverMethod::Rk4 => self.simulate_rk4().map(AdaptiveOutcome::Finished),
            SolverMethod::Bdf => self.run_bdf(),
        }
    }

    /// Adaptive Dormand-Prince 5(4) integration with dense output.
    pub fn simulate_adaptive(&self) -> Result<SimResult, SimError> {
        match self.adaptive(false)? {
            AdaptiveOutcome::Finished(result) => Ok(result),
            AdaptiveOutcome::Stiff => err("the stiffness watch fired without being asked"),
            AdaptiveOutcome::Stalled(stall) => err(format!(
                "step size underflow at t = {:.6}: probable singularity",
                stall.time
            )),
        }
    }

    /// Adaptive Dormand-Prince 5(4) integration with dense output on
    /// the `step` grid. The step size shrinks automatically near sharp
    /// dynamics (close encounters, kinks) and grows on smooth stretches;
    /// a persistent step-size underflow is reported as a probable
    /// singularity instead of returning garbage.
    ///
    /// With `watch_stiffness` the run gives up as soon as the step size
    /// is clearly limited by stability rather than accuracy, so that the
    /// caller can hand the problem to the implicit solver.
    fn adaptive(&self, watch_stiffness: bool) -> Result<AdaptiveOutcome, SimError> {
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

        let mut values = self.values_template.clone();
        let mut columns = vec!["time".to_string()];
        columns.extend(self.states.iter().cloned());
        columns.extend(self.output_algebraics.iter().map(|(name, _)| name.clone()));
        columns.extend(self.discretes.iter().cloned());
        let mut rows: Vec<Vec<f64>> = Vec::new();
        let mut derivatives_scratch = Vec::new();

        let mut record = |t: f64,
                          y: &[f64],
                          values: &mut [f64],
                          k: &mut Vec<f64>,
                          alg_guess: &mut [f64]|
         -> Result<(), SimError> {
            self.eval_point(t, y, values, k, alg_guess)?;
            self.check_asserts(t, values)?;
            let mut row = Vec::with_capacity(1 + n + self.algebraics.len() + self.discretes.len());
            row.push(t);
            row.extend_from_slice(y);
            for &(_, slot) in &self.output_algebraics {
                row.push(values[slot]);
            }
            for &slot in &self.discrete_slots {
                row.push(values[slot]);
            }
            rows.push(row);
            Ok(())
        };

        let mut y = self.initial.clone();
        let mut alg_guess = self.algebraic_start.clone();
        let t0 = self.start_time;
        let mut last_out_t = t0;
        let mut terminated: Option<String> = None;
        let mut state = self.event_state();
        if self.resume {
            // Mid-run already: no initial event, and the `when`
            // conditions resume from what is true at this instant.
            self.eval_point(
                t0,
                &y,
                &mut values,
                &mut derivatives_scratch,
                &mut alg_guess,
            )?;
            state.when_prev = self.when_conditions(t0, &values);
        } else {
            values[self.initial_slot] = 1.0;
            // The initial event comes before the first output point: a
            // `when initial()` or a `sample(0, …)` has already fired by then.
            state.raise_samples(t0, &self.samples, &self.sample_slots, &mut values);
            let start_event =
                self.handle_event(t0, &mut y, &mut values, &mut alg_guess, &mut state)?;
            if let Some(message) = start_event.terminated {
                record(
                    t0,
                    &y,
                    &mut values,
                    &mut derivatives_scratch,
                    &mut alg_guess,
                )?;
                return Ok(AdaptiveOutcome::Finished(SimResult {
                    columns,
                    rows,
                    parameters: self.parameters.clone(),
                    terminated: Some(message),
                    method: SolverMethod::Dopri45,
                    reselections: 0,
                }));
            }
        }
        record(
            t0,
            &y,
            &mut values,
            &mut derivatives_scratch,
            &mut alg_guess,
        )?;
        let mut indicators_prev = self.indicator_values(t0, &values);
        // Pure-algebraic models: no ODE to integrate, only the grid.
        if n == 0 {
            let mut out_i = 1usize;
            loop {
                // Walk to whichever comes first: the next output point
                // or the next scheduled time event.
                let grid = out_i as f64 * out_step;
                let t = match state.next_time_event() {
                    Some(next) if next < grid - 1e-12 => next,
                    _ => grid,
                };
                if t > stop + 1e-12 {
                    break;
                }
                record(t, &y, &mut values, &mut derivatives_scratch, &mut alg_guess)?;
                if (t - grid).abs() < 1e-12 {
                    last_out_t = t;
                    out_i += 1;
                }
                state.raise_samples(t, &self.samples, &self.sample_slots, &mut values);
                let outcome =
                    self.handle_event(t, &mut y, &mut values, &mut alg_guess, &mut state)?;
                if outcome.changed {
                    record(t, &y, &mut values, &mut derivatives_scratch, &mut alg_guess)?;
                }
                terminated = outcome.terminated;
                if terminated.is_some() {
                    break;
                }
            }
            if terminated.is_none() && last_out_t < stop - 1e-12 {
                record(
                    stop,
                    &y,
                    &mut values,
                    &mut derivatives_scratch,
                    &mut alg_guess,
                )?;
            }
            return Ok(AdaptiveOutcome::Finished(SimResult {
                columns,
                rows,
                parameters: self.parameters.clone(),
                terminated,
                method: SolverMethod::Dopri45,
                reselections: 0,
            }));
        }

        let mut k: Vec<Vec<f64>> = vec![vec![0.0; n]; 7];
        let mut stage = vec![0.0; n];
        let mut penultimate = vec![0.0; n];
        // Steps that looked stiff, and the calm ones that clear them.
        // A single step over the boundary is noise - a kink, an event, a
        // bad guess; a run of them is the method saying it is held back
        // by stability rather than accuracy.
        let (mut stiff_steps, mut calm_steps) = (0usize, 0usize);
        let mut y5 = vec![0.0; n];
        let mut interp = vec![0.0; n];
        let mut t = t0;
        let mut h = out_step.min(stop - t0).max(1e-9);
        // The next output-grid index after where this segment starts.
        let mut out_i = (t0 / out_step + 1e-9).floor() as usize + 1;
        let mut evals: u64 = 0;

        self.eval_point(t, &y, &mut values, &mut k[0], &mut alg_guess)?;

        /// On a model whose states were chosen by a pivot, a failing
        /// evaluation is the choice failing: stop here and let the
        /// caller choose again. Everything already recorded stays.
        macro_rules! or_stall {
            ($attempt:expr) => {
                match $attempt {
                    Ok(value) => value,
                    Err(_) if self.reselectable => {
                        return self.stall_at_last_row(columns, rows, SolverMethod::Dopri45);
                    }
                    Err(error) => return Err(error),
                }
            };
        }

        while t < stop - 1e-12 {
            h = h.min(stop - t);
            // A scheduled time event is not something to step over: the
            // step ends exactly on it.
            if let Some(next) = state.next_time_event() {
                if next > t + 1e-12 && next <= stop + 1e-12 {
                    h = h.min(next - t);
                }
            }
            // Stages 2..7 (stage 1 is FSAL from the previous step).
            let mut stage_failed = false;
            for s in 1..7 {
                for j in 0..n {
                    let mut acc = 0.0;
                    for (q, k_q) in k.iter().enumerate().take(s) {
                        acc += A[s][q] * k_q[j];
                    }
                    stage[j] = y[j] + h * acc;
                }
                if s == 5 {
                    // The sixth stage sits at t + h, like the solution
                    // itself but at a different point; the gap between
                    // the two, and between their derivatives, is what
                    // the stiffness estimate is made of.
                    penultimate.copy_from_slice(&stage);
                }
                let (head, tail) = k.split_at_mut(s);
                let _ = head;
                match self.eval_point(
                    t + C[s] * h,
                    &stage,
                    &mut values,
                    &mut tail[0],
                    &mut alg_guess,
                ) {
                    Ok(()) => {}
                    // A dying algebraic loop on a model whose states were
                    // *chosen* is likely the choice failing, not the
                    // model: treat the step as rejected and let the step
                    // size fall toward the stall check below.
                    Err(_) if self.reselectable => {
                        stage_failed = true;
                        break;
                    }
                    Err(error) => return Err(error),
                }
            }
            evals += 6;
            if evals > 20_000_000 {
                return err(format!(
                    "solver exceeded the evaluation budget at t = {t:.6}"
                ));
            }

            // 5th-order solution and the embedded error estimate.
            let mut err_norm = if stage_failed { f64::INFINITY } else { 0.0f64 };
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
                match self.eval_point(t + h, &y5, &mut values, &mut k[6], &mut alg_guess) {
                    Ok(()) => {}
                    // The point passed the error estimate but the
                    // algebraic layer will not hold there: on a chosen
                    // selection, reject the step and shrink instead.
                    Err(_) if self.reselectable => {
                        h *= 0.2;
                        if h < stop * 5e-14 {
                            return self.stall_at_last_row(columns, rows, SolverMethod::Dopri45);
                        }
                        continue;
                    }
                    Err(error) => return Err(error),
                }
                evals += 1;

                if watch_stiffness {
                    // h times the dominant eigenvalue of the Jacobian,
                    // read off two points at t + h the step has already
                    // produced. Past the stability limit of the method
                    // (3.25 for Dormand-Prince) the step size is no
                    // longer about accuracy.
                    let (mut numerator, mut denominator) = (0.0f64, 0.0f64);
                    for j in 0..n {
                        numerator += (k[6][j] - k[5][j]).powi(2);
                        denominator += (y5[j] - penultimate[j]).powi(2);
                    }
                    if denominator > 0.0 && h * (numerator / denominator).sqrt() > 3.25 {
                        calm_steps = 0;
                        stiff_steps += 1;
                        if stiff_steps >= 15 {
                            return Ok(AdaptiveOutcome::Stiff);
                        }
                    } else {
                        // The estimate hovers around the boundary on a
                        // stiff problem, so a single step below it means
                        // nothing; only a calm stretch clears the count.
                        calm_steps += 1;
                        if calm_steps >= 6 {
                            stiff_steps = 0;
                        }
                    }
                }

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
                let indicators_new = self.indicator_values(t + h, &values);
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
                        or_stall!(self.eval_point(
                            t + mid * h,
                            &interp,
                            &mut values,
                            &mut derivatives_scratch,
                            &mut alg_guess,
                        ));
                        if before * self.indicator_values(t + mid * h, &values)[index] <= 0.0 {
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
                        or_stall!(record(
                            out_t,
                            &interp,
                            &mut values,
                            &mut derivatives_scratch,
                            &mut alg_guess,
                        ));
                        last_out_t = out_t;
                        out_i += 1;
                    }
                    interpolate(theta, &mut interp, &y, &k);
                    y.copy_from_slice(&interp);
                    or_stall!(self.eval_point(
                        t_event,
                        &y,
                        &mut values,
                        &mut derivatives_scratch,
                        &mut alg_guess,
                    ));
                    let outcome = or_stall!(self.handle_event(
                        t_event,
                        &mut y,
                        &mut values,
                        &mut alg_guess,
                        &mut state,
                    ));
                    t = t_event;
                    or_stall!(self.eval_point(t, &y, &mut values, &mut k[0], &mut alg_guess));
                    indicators_prev = self.indicator_values(t, &values);
                    if let Some(message) = outcome.terminated {
                        or_stall!(record(
                            t,
                            &y,
                            &mut values,
                            &mut derivatives_scratch,
                            &mut alg_guess
                        ));
                        terminated = Some(message);
                        break;
                    }
                    // A state event that changed something is recorded at
                    // the instant it happened, so the jump is visible.
                    if outcome.changed {
                        or_stall!(record(
                            t,
                            &y,
                            &mut values,
                            &mut derivatives_scratch,
                            &mut alg_guess
                        ));
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
                    or_stall!(record(
                        out_t,
                        &interp,
                        &mut values,
                        &mut derivatives_scratch,
                        &mut alg_guess,
                    ));
                    last_out_t = out_t;
                    out_i += 1;
                }
                t += h;
                y.copy_from_slice(&y5);
                k.swap(0, 6);

                // The pivot that chose the states would choose otherwise
                // here: hand the run back for a re-selection while the
                // algebra is still sound.
                if self.reselectable && !self.selection_sound(&values, t) {
                    return self.stall_at_last_row(columns, rows, SolverMethod::Dopri45);
                }

                // The step ended on a scheduled instant: raise the flags
                // of the `sample(...)` sources due here and let the
                // `when` clauses read them.
                if state.next_time_event().is_some_and(|next| next <= t + 1e-9) {
                    or_stall!(self.eval_point(
                        t,
                        &y,
                        &mut values,
                        &mut derivatives_scratch,
                        &mut alg_guess
                    ));
                    state.raise_samples(t, &self.samples, &self.sample_slots, &mut values);
                    let outcome = or_stall!(self.handle_event(
                        t,
                        &mut y,
                        &mut values,
                        &mut alg_guess,
                        &mut state
                    ));
                    or_stall!(self.eval_point(t, &y, &mut values, &mut k[0], &mut alg_guess));
                    indicators_prev = self.indicator_values(t, &values);
                    if outcome.changed {
                        // The discrete values jumped here, so the point
                        // is recorded twice: before and after the event.
                        record(t, &y, &mut values, &mut derivatives_scratch, &mut alg_guess)?;
                    }
                    if let Some(message) = outcome.terminated {
                        terminated = Some(message);
                        break;
                    }
                }
            }

            let factor = if !err_norm.is_finite() {
                0.2
            } else if err_norm == 0.0 {
                5.0
            } else {
                (0.9 * err_norm.powf(-0.2)).clamp(0.2, 5.0)
            };
            h *= factor;
            // On a model whose states were chosen by a pivot, a step
            // collapse is the cue to choose again - well before the
            // hard underflow that would end the run.
            if self.reselectable && h < stop * 5e-14 {
                // The snapshot comes from the last accepted point; the
                // evaluation there succeeded when it was accepted.
                return self.stall_at_last_row(columns, rows, SolverMethod::Dopri45);
            }
            if h < stop * 1e-14 || h < 1e-300 {
                return err(format!(
                    "step size underflow at t = {t:.6}: probable singularity"
                ));
            }
        }
        if terminated.is_none() && last_out_t < stop - 1e-12 {
            record(
                stop,
                &y,
                &mut values,
                &mut derivatives_scratch,
                &mut alg_guess,
            )?;
            terminated = self
                .handle_event(stop, &mut y, &mut values, &mut alg_guess, &mut state)?
                .terminated;
        }
        Ok(AdaptiveOutcome::Finished(SimResult {
            columns,
            rows,
            parameters: self.parameters.clone(),
            terminated,
            method: SolverMethod::Dopri45,
            reselections: 0,
        }))
    }

    /// Finite-difference Jacobian `df/dy` of the state right-hand side
    /// at `(t, y)`. Algebraic warm starts are kept on a scratch copy so
    /// probing does not disturb the accepted solution.
    fn jacobian(
        &self,
        t: f64,
        y: &[f64],
        f0: &[f64],
        values: &mut [f64],
        alg_guess: &[f64],
    ) -> Result<Vec<Vec<f64>>, SimError> {
        let n = y.len();
        let mut jac = vec![vec![0.0; n]; n];
        let mut probe = y.to_vec();
        let mut scratch = alg_guess.to_vec();
        let mut f = Vec::with_capacity(n);
        let mut deltas = vec![0.0f64; n];
        for group in &self.jacobian_groups {
            // Every column of a group is perturbed at once: they share
            // no row, so each difference belongs to exactly one of them.
            for &j in group {
                deltas[j] = 1e-7 * (1.0 + y[j].abs());
                probe[j] = y[j] + deltas[j];
            }
            self.eval_point(t, &probe, values, &mut f, &mut scratch)?;
            for &j in group {
                probe[j] = y[j];
                for &i in &self.jacobian_rows[j] {
                    jac[i][j] = (f[i] - f0[i]) / deltas[j];
                }
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
        match self.run_bdf()? {
            AdaptiveOutcome::Finished(result) => Ok(result),
            AdaptiveOutcome::Stalled(stall) => err(format!(
                "step size underflow at t = {:.6}: probable singularity",
                stall.time
            )),
            AdaptiveOutcome::Stiff => unreachable!("bdf never reports stiffness"),
        }
    }

    /// The BDF integration itself; a stall on a reselectable model
    /// comes back as an outcome instead of an error.
    fn run_bdf(&self) -> Result<AdaptiveOutcome, SimError> {
        const MAX_ORDER: usize = 5;
        const NEWTON_MAX: usize = 12;

        let n = self.states.len();
        let stop = self.stop_time;
        let out_step = self.step.max(1e-12);
        let rtol = self.tolerance;
        let atol = self.tolerance * 1e-3;

        let mut values = self.values_template.clone();
        let mut columns = vec!["time".to_string()];
        columns.extend(self.states.iter().cloned());
        columns.extend(self.output_algebraics.iter().map(|(name, _)| name.clone()));
        columns.extend(self.discretes.iter().cloned());
        let mut rows: Vec<Vec<f64>> = Vec::new();
        let mut alg_guess = self.algebraic_start.clone();
        let mut f_scratch = Vec::new();

        let mut record = |t: f64,
                          y: &[f64],
                          values: &mut [f64],
                          k: &mut Vec<f64>,
                          alg_guess: &mut [f64]|
         -> Result<(), SimError> {
            self.eval_point(t, y, values, k, alg_guess)?;
            self.check_asserts(t, values)?;
            let mut row = Vec::with_capacity(1 + n + self.algebraics.len() + self.discretes.len());
            row.push(t);
            row.extend_from_slice(y);
            for &(_, slot) in &self.output_algebraics {
                row.push(values[slot]);
            }
            for &slot in &self.discrete_slots {
                row.push(values[slot]);
            }
            rows.push(row);
            Ok(())
        };

        let mut y = self.initial.clone();
        let mut terminated: Option<String> = None;
        let t0 = self.start_time;
        let mut state = self.event_state();
        if self.resume {
            // Mid-run already: no initial event, conditions resume from
            // their current truth.
            self.eval_point(t0, &y, &mut values, &mut f_scratch, &mut alg_guess)?;
            state.when_prev = self.when_conditions(t0, &values);
        } else {
            values[self.initial_slot] = 1.0;
            // The initial event comes before the first output point: a
            // `when initial()` or a `sample(0, …)` has already fired by then.
            state.raise_samples(t0, &self.samples, &self.sample_slots, &mut values);
            let start_event =
                self.handle_event(t0, &mut y, &mut values, &mut alg_guess, &mut state)?;
            if let Some(message) = start_event.terminated {
                record(t0, &y, &mut values, &mut f_scratch, &mut alg_guess)?;
                return Ok(AdaptiveOutcome::Finished(SimResult {
                    columns,
                    rows,
                    parameters: self.parameters.clone(),
                    terminated: Some(message),
                    method: SolverMethod::Bdf,
                    reselections: 0,
                }));
            }
        }
        record(t0, &y, &mut values, &mut f_scratch, &mut alg_guess)?;
        let mut indicators_prev = self.indicator_values(t0, &values);

        // Pure-algebraic models: nothing to integrate, walk the grid.
        let mut out_i = (t0 / out_step + 1e-9).floor() as usize + 1;
        let mut last_out_t = t0;
        if n == 0 {
            loop {
                // Walk to whichever comes first: the next output point
                // or the next scheduled time event.
                let grid = out_i as f64 * out_step;
                let t = match state.next_time_event() {
                    Some(next) if next < grid - 1e-12 => next,
                    _ => grid,
                };
                if t > stop + 1e-12 {
                    break;
                }
                record(t, &y, &mut values, &mut f_scratch, &mut alg_guess)?;
                if (t - grid).abs() < 1e-12 {
                    last_out_t = t;
                    out_i += 1;
                }
                state.raise_samples(t, &self.samples, &self.sample_slots, &mut values);
                let outcome =
                    self.handle_event(t, &mut y, &mut values, &mut alg_guess, &mut state)?;
                if outcome.changed {
                    record(t, &y, &mut values, &mut f_scratch, &mut alg_guess)?;
                }
                terminated = outcome.terminated;
                if terminated.is_some() {
                    break;
                }
            }
            if terminated.is_none() && last_out_t < stop - 1e-12 {
                record(stop, &y, &mut values, &mut f_scratch, &mut alg_guess)?;
            }
            return Ok(AdaptiveOutcome::Finished(SimResult {
                columns,
                rows,
                parameters: self.parameters.clone(),
                terminated,
                method: SolverMethod::Bdf,
                reselections: 0,
            }));
        }

        // History, newest first.
        let mut t_hist: Vec<f64> = vec![t0];
        let mut y_hist: Vec<Vec<f64>> = vec![y.clone()];
        let mut order = 1usize;
        let mut t = t0;
        let mut h = out_step.min(stop - t0).max(1e-12);
        let mut jac: Option<Vec<Vec<f64>>> = None;
        let mut consecutive_ok = 0usize;
        let mut steps: u64 = 0;

        let mut f_new = vec![0.0; n];
        let mut y_new = vec![0.0; n];
        let mut y_pred = vec![0.0; n];
        let mut f_last = Vec::new();
        self.eval_point(t0, &y, &mut values, &mut f_last, &mut alg_guess)?;

        /// See the macro of the adaptive solver: a failing evaluation on
        /// a chosen selection stalls the segment instead of ending the run.
        macro_rules! or_stall {
            ($attempt:expr) => {
                match $attempt {
                    Ok(value) => value,
                    Err(_) if self.reselectable => {
                        return self.stall_at_last_row(columns, rows, SolverMethod::Bdf);
                    }
                    Err(error) => return Err(error),
                }
            };
        }

        while t < stop - 1e-12 {
            h = h.min(stop - t);
            // A scheduled time event is not something to step over: the
            // step ends exactly on it.
            if let Some(next) = state.next_time_event() {
                if next > t + 1e-12 && next <= stop + 1e-12 {
                    h = h.min(next - t);
                }
            }
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
                match self.eval_point(t_new, &y_new, &mut values, &mut f_new, &mut alg_guess) {
                    Ok(()) => {}
                    // The algebraic layer dying on a chosen selection is
                    // the selection failing: reject the step and let the
                    // step size fall toward the stall check.
                    Err(_) if self.reselectable => {
                        newton_failed = true;
                        break;
                    }
                    Err(error) => return Err(error),
                }
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
                    jac = Some(self.jacobian(t_new, &y_new, &f_new, &mut values, &alg_guess)?);
                }
                // The iteration matrix only moves when the Jacobian is
                // refreshed or the step coefficient changes, so it is
                // eliminated once and its factors reused - both between
                // the iterations of a step and between steps.
                let jacobian = jac.as_ref().expect("jacobian present");
                // A banded structure is eliminated as a band; anything
                // else, and anything the band cannot pivot through,
                // goes through the dense path.
                let banded = self.jacobian_band.and_then(|band| {
                    let mut matrix: Vec<Vec<f64>> = (0..n)
                        .map(|i| {
                            (0..2 * band + 1)
                                .map(|offset| {
                                    let column = (i + offset).checked_sub(band)?;
                                    if column >= n {
                                        return None;
                                    }
                                    Some(if i == column {
                                        c0 - jacobian[i][column]
                                    } else {
                                        -jacobian[i][column]
                                    })
                                })
                                .map(|value| value.unwrap_or(0.0))
                                .collect()
                        })
                        .collect();
                    solve_banded(&mut matrix, band, &residual)
                });
                let delta = match banded {
                    Some(delta) => delta,
                    None => {
                        let mut matrix: Vec<Vec<f64>> = jacobian
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
                        delta
                    }
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
                self.eval_point(t_new, &y_new, &mut values, &mut f_scratch, &mut alg_guess)?;
                let indicators_new = self.indicator_values(t_new, &values);
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
                        self.eval_point(mid, &interp, &mut values, &mut f_scratch, &mut alg_guess)?;
                        if before * self.indicator_values(mid, &values)[index] <= 0.0 {
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
                    record(out_t, &interp, &mut values, &mut f_scratch, &mut alg_guess)?;
                    last_out_t = out_t;
                    out_i += 1;
                }

                if let Some(t_event) = event_t {
                    sample(t_event, &mut interp);
                    y.copy_from_slice(&interp);
                    self.eval_point(t_event, &y, &mut values, &mut f_scratch, &mut alg_guess)?;
                    let outcome = or_stall!(self.handle_event(
                        t_event,
                        &mut y,
                        &mut values,
                        &mut alg_guess,
                        &mut state,
                    ));
                    t = t_event;
                    // The history is meaningless across a discontinuity.
                    t_hist.clear();
                    y_hist.clear();
                    t_hist.push(t);
                    y_hist.push(y.clone());
                    order = 1;
                    consecutive_ok = 0;
                    jac = None;
                    self.eval_point(t, &y, &mut values, &mut f_last, &mut alg_guess)?;
                    indicators_prev = self.indicator_values(t, &values);
                    if let Some(message) = outcome.terminated {
                        record(t, &y, &mut values, &mut f_scratch, &mut alg_guess)?;
                        terminated = Some(message);
                        break;
                    }
                    // A state event that changed something is recorded at
                    // the instant it happened, so the jump is visible.
                    if outcome.changed {
                        record(t, &y, &mut values, &mut f_scratch, &mut alg_guess)?;
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
                // See the adaptive solver: a selection that stopped
                // being the right one is re-made in clean territory.
                if self.reselectable && !self.selection_sound(&values, t) {
                    return self.stall_at_last_row(columns, rows, SolverMethod::Bdf);
                }
                // Raise the order once the history supports it. The
                // step controller keeps the error near its target, so
                // waiting for a *small* error would pin the order at 1;
                // a premature raise simply costs one rejected step.
                if consecutive_ok > order && order < MAX_ORDER && t_hist.len() > order {
                    order += 1;
                    consecutive_ok = 0;
                }

                // The step ended on a scheduled instant: the sources due
                // here raise their flags and the `when` clauses read them.
                if state.next_time_event().is_some_and(|next| next <= t + 1e-9) {
                    self.eval_point(t, &y, &mut values, &mut f_scratch, &mut alg_guess)?;
                    state.raise_samples(t, &self.samples, &self.sample_slots, &mut values);
                    let outcome =
                        self.handle_event(t, &mut y, &mut values, &mut alg_guess, &mut state)?;
                    self.eval_point(t, &y, &mut values, &mut f_last, &mut alg_guess)?;
                    indicators_prev = self.indicator_values(t, &values);
                    if outcome.changed {
                        // A jump the history cannot represent: restart
                        // from order one, and record both sides of it.
                        record(t, &y, &mut values, &mut f_scratch, &mut alg_guess)?;
                        y_hist[0].copy_from_slice(&y);
                        t_hist.truncate(1);
                        y_hist.truncate(1);
                        order = 1;
                        consecutive_ok = 0;
                    }
                    if let Some(message) = outcome.terminated {
                        terminated = Some(message);
                        break;
                    }
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
            // A collapsing step on a model whose states were chosen by
            // a pivot: stall and let the caller choose again.
            if self.reselectable && h < stop * 5e-14 {
                return self.stall_at_last_row(columns, rows, SolverMethod::Bdf);
            }
            if h < stop * 1e-14 || h < 1e-300 {
                return err(format!(
                    "step size underflow at t = {t:.6}: probable singularity"
                ));
            }
        }

        if terminated.is_none() && last_out_t < stop - 1e-12 {
            record(stop, &y, &mut values, &mut f_scratch, &mut alg_guess)?;
            terminated = self
                .handle_event(stop, &mut y, &mut values, &mut alg_guess, &mut state)?
                .terminated;
        }
        Ok(AdaptiveOutcome::Finished(SimResult {
            columns,
            rows,
            parameters: self.parameters.clone(),
            terminated,
            method: SolverMethod::Bdf,
            reselections: 0,
        }))
    }

    /// Classic fixed-step RK4 integration over `[0, stop_time]`.
    pub fn simulate_rk4(&self) -> Result<SimResult, SimError> {
        let n = self.states.len();
        let steps = (self.stop_time / self.step).ceil() as usize;
        let mut y = self.initial.clone();
        let mut alg_guess = self.algebraic_start.clone();
        let mut values = self.values_template.clone();
        let (mut k1, mut k2, mut k3, mut k4) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut scratch = vec![0.0; n];

        let mut columns = vec!["time".to_string()];
        columns.extend(self.states.iter().cloned());
        columns.extend(self.output_algebraics.iter().map(|(name, _)| name.clone()));
        columns.extend(self.discretes.iter().cloned());
        let mut rows = Vec::with_capacity(steps + 1);

        let mut record = |t: f64,
                          y: &[f64],
                          values: &mut [f64],
                          k: &mut Vec<f64>,
                          this: &CompiledModel,
                          alg_guess: &mut [f64]|
         -> Result<(), SimError> {
            this.eval_point(t, y, values, k, alg_guess)?;
            this.check_asserts(t, values)?;
            let mut row = Vec::with_capacity(1 + this.states.len() + this.algebraics.len());
            row.push(t);
            row.extend_from_slice(y);
            for &(_, slot) in &this.output_algebraics {
                row.push(values[slot]);
            }
            for &slot in &this.discrete_slots {
                row.push(values[slot]);
            }
            rows.push(row);
            Ok(())
        };

        // A model with time events needs a solver that can step onto
        // them; the fixed grid of RK4 cannot.
        if !self.samples.is_empty() {
            return err(
                "sample() needs a solver that steps onto the event: use dopri45 or bdf".to_string(),
            );
        }
        let mut state = self.event_state();
        values[self.initial_slot] = 1.0;
        record(0.0, &y, &mut values, &mut k1, self, &mut alg_guess)?;
        let mut terminated = self
            .handle_event(0.0, &mut y, &mut values, &mut alg_guess, &mut state)?
            .terminated;

        for i in 0..steps {
            if terminated.is_some() {
                break;
            }
            let t = i as f64 * self.step;
            let h = (self.stop_time - t).min(self.step);

            self.eval_point(t, &y, &mut values, &mut k1, &mut alg_guess)?;
            for j in 0..n {
                scratch[j] = y[j] + 0.5 * h * k1[j];
            }
            self.eval_point(t + 0.5 * h, &scratch, &mut values, &mut k2, &mut alg_guess)?;
            for j in 0..n {
                scratch[j] = y[j] + 0.5 * h * k2[j];
            }
            self.eval_point(t + 0.5 * h, &scratch, &mut values, &mut k3, &mut alg_guess)?;
            for j in 0..n {
                scratch[j] = y[j] + h * k3[j];
            }
            self.eval_point(t + h, &scratch, &mut values, &mut k4, &mut alg_guess)?;
            for j in 0..n {
                y[j] += h / 6.0 * (k1[j] + 2.0 * k2[j] + 2.0 * k3[j] + k4[j]);
            }
            record(t + h, &y, &mut values, &mut k1, self, &mut alg_guess)?;
            terminated = self
                .handle_event(t + h, &mut y, &mut values, &mut alg_guess, &mut state)?
                .terminated;
        }

        Ok(SimResult {
            columns,
            rows,
            parameters: self.parameters.clone(),
            terminated,
            method: SolverMethod::Rk4,
            reselections: 0,
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
    fn expressions_are_checked_before_anything_runs() {
        // Names and functions are resolved while compiling, so none of
        // these can reach a solver: an unknown variable, an unknown
        // function, and a built-in given the wrong number of arguments.
        assert!(
            compile_err("model M Real y; equation y = z + 1; end M;").contains("unknown variable")
        );
        assert!(compile_err("model M Real y; equation y = frob(1); end M;")
            .contains("unknown function"));
        assert!(compile_err("model M Real y; equation y = sin(1, 2); end M;").contains("argument"));
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
        // Values are written at full precision in the shortest text that
        // reads back as the same double, so a round number is round.
        assert_eq!(lines.next().unwrap(), "0,1");
        assert_eq!(csv.lines().count(), result.rows.len() + 1);
        // And a value that is not round keeps every digit it needs.
        let last = csv.lines().last().unwrap();
        let value: f64 = last.split(',').nth(1).unwrap().parse().unwrap();
        assert!((value - result.rows.last().unwrap()[1]).abs() == 0.0);
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
        // Caught while compiling: the target of a reinit is resolved to a
        // place in the state vector, so a name that is not one cannot
        // wait until the run to be noticed.
        let error = compile(&model).unwrap_err();
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
    fn a_while_loop_computes_the_exact_large_swing_period() {
        // The function runs an arithmetic-geometric mean to convergence
        // at compile time; the simulated pendulum must come back to its
        // amplitude exactly one such period later.
        let result = compile(&with_library("pendulum_period.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();

        let (length, gravity, amplitude) = (1.2f64, 9.81f64, 1.0f64);
        let (mut a, mut b) = (1.0f64, (amplitude / 2.0).cos());
        while (a - b).abs() > 1e-15 {
            let mean = 0.5 * (a + b);
            b = (a * b).sqrt();
            a = mean;
        }
        let period = 2.0 * std::f64::consts::PI * (length / gravity).sqrt() / a;
        assert!(
            (result.rows[0][index("period")] - period).abs() < 1e-12,
            "period {} vs {period}",
            result.rows[0][index("period")]
        );
        // Small-angle theory would say 2.1972 s; the true period at a
        // 1 rad swing is 2.3430 s, and the trajectory knows it.
        let row = result
            .rows
            .iter()
            .min_by(|p, q| {
                let (dp, dq) = ((p[0] - period).abs(), (q[0] - period).abs());
                dp.partial_cmp(&dq).unwrap()
            })
            .unwrap();
        assert!(
            (row[index("theta")] - amplitude).abs() < 1e-3,
            "theta {} after one period",
            row[index("theta")]
        );
        assert!(
            row[index("w")].abs() < 0.05,
            "w {} at the turn",
            row[index("w")]
        );
    }

    #[test]
    fn the_flight_plan_and_the_flown_trajectory_agree() {
        // `(planned_range, planned_duration) = flight(v0, angle)` fills
        // both targets from one call, with gravity defaulted inside the
        // function; the integrated throw must land where it says.
        let result = compile(&with_library("ballistic_range.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let last = result.rows.last().unwrap();

        let (v0, angle, g) = (12.0f64, 0.6f64, 9.81);
        let range = v0 * v0 * (2.0 * angle).sin() / g;
        let duration = 2.0 * v0 * angle.sin() / g;
        // The planned values are constants over the whole run.
        assert!((result.rows[0][index("planned_range")] - range).abs() < 1e-12);
        assert!((last[index("planned_range")] - range).abs() < 1e-12);
        assert!((last[index("planned_duration")] - duration).abs() < 1e-12);
        // The run stops within a hair of the planned landing, so the
        // ball is at the planned range and back on the ground.
        assert!(
            (last[index("x")] - range).abs() < 1e-3,
            "landed at {} instead of {range}",
            last[index("x")]
        );
        assert!(
            last[index("y")].abs() < 1e-2,
            "still {} up",
            last[index("y")]
        );
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
    fn a_world_shared_through_inner_outer_drives_a_projectile() {
        // The point mass reads gravity from the `inner World` of the top
        // model; the trajectory is a polynomial the solver integrates
        // exactly.
        let result = compile(&with_library("projectile.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let (x, y, vy) = (index("ball.x"), index("ball.y"), index("ball.vy"));
        for row in &result.rows {
            let t = row[0];
            assert!((row[x] - 12.0 * t).abs() < 1e-12, "x at {t}: {}", row[x]);
            let height = 16.0 * t - 0.5 * 9.81 * t * t;
            assert!((row[y] - height).abs() < 1e-12, "y at {t}: {}", row[y]);
            assert!((row[vy] - (16.0 - 9.81 * t)).abs() < 1e-12);
        }
    }

    #[test]
    fn a_conditional_support_carries_the_reaction_torque() {
        // Two identical drives: one reacting on its internal housing,
        // one on an exposed support flange. The shafts must not be able
        // to tell the difference.
        let result = compile(&with_library("torque_support.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let last = result.rows.last().unwrap();
        for row in &result.rows {
            assert!(row[index("difference")].abs() < 1e-12);
        }
        // phi = tau / (2 J) t^2 with tau = 2, J = 0.5, t = 4.
        assert!((last[index("shaftA.phi")] - 32.0).abs() < 1e-9);
        // The exposed support takes the reaction; the internal one hides it.
        assert!((last[index("driveB.support.tau")] - 2.0).abs() < 1e-12);
        assert!(!result.columns.iter().any(|c| c == "driveA.support.tau"));
    }

    #[test]
    fn a_redeclared_controller_changes_the_steady_state() {
        // The example file ships a proportional drive and a derived model
        // that redeclares the controller as a PI. Loading the file as a
        // library reaches both.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let library = std::fs::read_to_string(root.join("lib/Oxidelica.mo")).unwrap();
        let example = std::fs::read_to_string(root.join("examples/pi_drive.mo")).unwrap();
        let run_top = |source: &str| {
            let model = oxidelica_parser::parse_model_with_libraries(
                &[library.clone(), example.clone()],
                source,
            )
            .unwrap();
            compile(&model).unwrap().simulate().unwrap()
        };

        // Proportional only: a gain of 5 on a unit-gain plant leaves
        // 1/(1+5) of the setpoint as a standing error.
        let plain = run_top(
            "model Plain ProportionalDrive drive; Real y; equation y = drive.y; \
             annotation(experiment(StopTime = 6.0, Interval = 0.01, Tolerance = 1e-9)); end Plain;",
        );
        let y = plain.columns.iter().position(|c| c == "y").unwrap();
        assert!((plain.rows.last().unwrap()[y] - 5.0 / 6.0).abs() < 1e-6);

        // With the PI redeclared in, the offset is gone.
        let tuned = run_top(
            "model Tuned PIDrive drive; Real y; equation y = drive.y; \
             annotation(experiment(StopTime = 6.0, Interval = 0.01, Tolerance = 1e-9)); end Tuned;",
        );
        let y = tuned.columns.iter().position(|c| c == "y").unwrap();
        assert!((tuned.rows.last().unwrap()[y] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn an_enumeration_selects_the_shape_of_a_waveform() {
        // Square wave through a first-order lag: on the first half
        // period the answer is the analytic step response, and the jump
        // at the half period is an event the solver stops at.
        let result = compile(&with_library("waveform.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let (u, y) = (index("u"), index("y"));
        for row in &result.rows {
            let t = row[0];
            assert!(row[u].abs() == 1.0, "square wave value {} at {t}", row[u]);
            if t <= 1.0 - 1e-9 {
                let expected = 1.0 - (-t / 0.3f64).exp();
                assert!((row[y] - expected).abs() < 1e-6, "y at {t}: {}", row[y]);
            }
        }

        // The triangle shape of the same source is asin of a sine.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let library = std::fs::read_to_string(root.join("lib/Oxidelica.mo")).unwrap();
        let model = oxidelica_parser::parse_model_with_libraries(
            &[library],
            "model T Oxidelica.Blocks.Sources.Waveform source(\
               kind = Oxidelica.Types.WaveformKind.Triangle, f = 0.5); \
             Real u; equation u = source.y; \
             annotation(experiment(StopTime = 3.0, Interval = 0.01)); end T;",
        )
        .unwrap();
        let triangle = compile(&model).unwrap().simulate().unwrap();
        let u = triangle.columns.iter().position(|c| c == "u").unwrap();
        for row in &triangle.rows {
            let expected =
                2.0 * (std::f64::consts::PI * row[0]).sin().asin() / std::f64::consts::PI;
            assert!((row[u] - expected).abs() < 1e-12);
        }
    }

    #[test]
    fn a_sampled_controller_holds_its_output_between_ticks() {
        let result = compile(&with_library("sampled_control.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let (u, y) = (index("u"), index("y"));
        let (period, plant_time) = (0.1f64, 0.5f64);

        // The control signal changes only on the clock, and the ticks
        // land on the period exactly rather than on the output grid.
        let mut ticks = Vec::new();
        for pair in result.rows.windows(2) {
            if pair[0][u] != pair[1][u] {
                ticks.push(pair[1][0]);
            }
        }
        assert_eq!(ticks.len(), 50, "one tick per period over five seconds");
        for t in &ticks {
            let off = t / period - (t / period).round();
            assert!(off.abs() < 1e-9, "tick off the clock at t = {t}");
        }

        // Between two ticks the plant is a first-order lag relaxing
        // toward the held value, which has a closed form.
        let mut worst = 0.0f64;
        for pair in result.rows.windows(2) {
            let (t0, t1) = (pair[0][0], pair[1][0]);
            if t1 <= t0 || pair[0][u] != pair[1][u] {
                continue;
            }
            let held = pair[1][u];
            let expected = held + (pair[0][y] - held) * (-(t1 - t0) / plant_time).exp();
            worst = worst.max((pair[1][y] - expected).abs());
        }
        assert!(worst < 1e-8, "hold response off by {worst}");

        // Integral action still lands on the setpoint.
        assert!((result.rows.last().unwrap()[y] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn hysteresis_switches_exactly_on_its_band() {
        let result = compile(&with_library("thermostat.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let (temperature, heating, switches) = (index("T"), index("heating"), index("switches"));

        let mut switch_on = Vec::new();
        let mut switch_off = Vec::new();
        for pair in result.rows.windows(2) {
            if pair[0][heating] == pair[1][heating] {
                continue;
            }
            if pair[1][heating] > 0.5 {
                switch_on.push((pair[1][0], pair[1][temperature]));
            } else {
                switch_off.push((pair[1][0], pair[1][temperature]));
            }
        }
        // The heater switches on the band edges, located to the same
        // tolerance as any other event.
        for (_, t) in &switch_off {
            assert!((t - 21.0).abs() < 1e-6, "switched off at {t}");
        }
        for (_, t) in &switch_on {
            assert!((t - 19.0).abs() < 1e-6, "switched on at {t}");
        }

        // Heating from 19 to 21 and cooling back is a closed form: the
        // room chases 29 with the heater on and 5 with it off, both with
        // the time constant C / G = 200 s.
        let expected = 200.0 * (10.0f64 / 8.0).ln() + 200.0 * (16.0f64 / 14.0).ln();
        for pair in switch_on.windows(2) {
            let period = pair[1].0 - pair[0].0;
            assert!(
                (period - expected).abs() < 1e-3,
                "cycle {period} vs {expected}"
            );
        }
        // The counter counted the switch-ons, and only those.
        assert_eq!(
            result.rows.last().unwrap()[switches] as i64,
            switch_on.len() as i64 + 1,
            "the heater starts on, so the count leads the switch-ons by one"
        );
    }

    #[test]
    fn the_discrete_library_blocks_run_on_the_clock() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let library = std::fs::read_to_string(root.join("lib/Oxidelica.mo")).unwrap();
        let run_top = |source: &str| {
            let model = oxidelica_parser::parse_model_with_libraries(
                std::slice::from_ref(&library),
                source,
            )
            .unwrap();
            compile(&model).unwrap().simulate().unwrap()
        };

        // A unit delay carries the previous tick's value, so against a
        // ramp its output trails the input by exactly one period.
        let delayed = run_top(
            "model D Oxidelica.Blocks.Discrete.UnitDelay delay(samplePeriod = 0.25); \
             Real ramp; equation ramp = time; delay.u = ramp; \
             annotation(experiment(StopTime = 2.0, Interval = 0.05)); end D;",
        );
        let index =
            |result: &SimResult, name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let (y, held) = (index(&delayed, "delay.y"), index(&delayed, "delay.held"));
        for row in &delayed.rows {
            let t = row[0];
            // A tick instant carries two rows, before and after the
            // event; the value between ticks is the one to check.
            if t < 0.5 || (t / 0.25 - (t / 0.25).round()).abs() < 1e-9 {
                continue;
            }
            // At time t the output is the input from one tick earlier.
            let tick = (t / 0.25).floor() * 0.25;
            let expected = (tick - 0.25).max(0.0);
            assert!(
                (row[y] - expected).abs() < 1e-9,
                "t = {t}: delayed {} vs {expected}",
                row[y]
            );
            assert!((row[held] - tick).abs() < 1e-9);
        }

        // The library controller reproduces the hand-written one of the
        // example, tick for tick.
        let library_pi = run_top(
            "model L Oxidelica.Blocks.Discrete.PI controller(samplePeriod = 0.1, k = 2.0, Ti = 0.5); \
             Real y(start = 0, fixed = true); equation controller.u = 1.0 - y; \
             der(y) = (controller.y - y) / 0.5; \
             annotation(experiment(StopTime = 5.0, Interval = 0.002, Tolerance = 1e-9)); end L;",
        );
        let by_hand = compile(&with_library("sampled_control.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let (a, b) = (index(&library_pi, "y"), index(&by_hand, "y"));
        assert_eq!(library_pi.rows.len(), by_hand.rows.len());
        for (left, right) in library_pi.rows.iter().zip(&by_hand.rows) {
            assert!((left[a] - right[b]).abs() < 1e-12);
        }
    }

    #[test]
    fn event_iteration_chains_the_clauses_of_one_event() {
        // `initial()` fires before the first output point; the edge of a
        // discrete variable and the change of a counter are seen inside
        // the same event that produced them.
        let result = run(
            "model M Real x(start = 0, fixed = true); Boolean started(start = false); \
             Boolean on(start = false); Integer rises(start = 0); Integer changes(start = 0); \
             equation der(x) = 1; \
             when initial() then started = true; end when; \
             when x > 0.5 then on = true; end when; \
             when edge(on) then rises = pre(rises) + 1; end when; \
             when change(rises) then changes = pre(changes) + 1; end when; \
             annotation(experiment(StopTime = 1.0, Interval = 0.05)); end M;",
        );
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let first = &result.rows[0];
        assert_eq!(first[index("started")], 1.0, "initial() fires at t = 0");
        assert_eq!(first[index("rises")], 0.0);

        let last = result.rows.last().unwrap();
        assert_eq!(last[index("on")], 1.0);
        assert_eq!(last[index("rises")], 1.0);
        assert_eq!(last[index("changes")], 1.0);
        // Everything happened in the single event at x = 0.5.
        let switch = result
            .rows
            .iter()
            .find(|row| row[index("rises")] > 0.5)
            .expect("the chain fires");
        assert!((switch[index("x")] - 0.5).abs() < 1e-6);
        assert_eq!(switch[index("changes")], 1.0);
    }

    #[test]
    fn the_discrete_layer_reports_its_error_paths() {
        let error = |source: &str| {
            let model = parse_model(source).unwrap();
            compile(&model).unwrap_err().to_string()
        };
        // `pre` needs a variable that has a value from before the event.
        assert!(error(
            "model M Real x(start = 0, fixed = true); Real y; equation der(x) = 1; y = pre(x); end M;"
        )
        .contains("not discrete"));
        // A clock with no period.
        assert!(error(
            "model M Real x(start = 0, fixed = true); Real u; equation der(x) = 1; \
             when sample(0, 0) then u = x; end when; end M;"
        )
        .contains("interval must be positive"));
        // A `when` assigning something that was never declared.
        assert!(error(
            "model M Real x(start = 0, fixed = true); equation der(x) = 1; \
             when x > 0.5 then u = 1; end when; end M;"
        )
        .contains("never declared"));
        // A discrete variable nothing ever assigns.
        assert!(error(
            "model M discrete Real u; Real x(start = 0, fixed = true); equation der(x) = 1; end M;"
        )
        .contains("never assigned"));
        // `pre` of an expression rather than a variable.
        assert!(error(
            "model M Real x(start = 0, fixed = true); Real u; equation der(x) = 1; \
             when x > 0.5 then u = pre(x + 1); end when; end M;"
        )
        .contains("takes a variable"));

        // The fixed grid of RK4 cannot step onto a time event.
        let model = parse_model(
            "model M Real x(start = 0, fixed = true); Real u; equation der(x) = 1; \
             when sample(0, 0.1) then u = x; end when; \
             annotation(experiment(StopTime = 1.0, Interval = 0.05)); end M;",
        )
        .unwrap();
        let mut compiled = compile(&model).unwrap();
        compiled.method = SolverMethod::Rk4;
        assert!(compiled
            .simulate()
            .unwrap_err()
            .to_string()
            .contains("dopri45 or bdf"));
        // The stiff solver steps onto them like the adaptive one does.
        compiled.method = SolverMethod::Bdf;
        let stiff = compiled.simulate().unwrap();
        let u = stiff.columns.iter().position(|c| c == "u").unwrap();
        assert!((stiff.rows.last().unwrap()[u] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn an_algorithm_converts_a_held_sample_into_a_staircase() {
        let result = compile(&with_library("quantizer.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let (sample, quantized, error) = (index("adc.y"), index("quantized"), index("error"));
        let (step, levels, period) = (0.25f64, 4, 0.05f64);

        // The staircase the algorithm describes, evaluated the same way
        // it is written: strictly above a level to reach it.
        let staircase = |value: f64| {
            let mut out = 0.0;
            for i in 1..=levels {
                if value > i as f64 * step {
                    out = i as f64 * step;
                }
                if value < -(i as f64) * step {
                    out = -(i as f64) * step;
                }
            }
            out
        };
        for row in &result.rows {
            assert_eq!(row[quantized], staircase(row[sample]));
            assert!(row[error].abs() <= step + 1e-12);
            // Between ticks the converter holds the signal it sampled.
            let t = row[0];
            if (t / period - (t / period).round()).abs() < 1e-9 {
                continue;
            }
            let tick = (t / period).floor() * period;
            let held = (2.0 * std::f64::consts::PI * tick).sin();
            assert!(
                (row[sample] - held).abs() < 1e-8,
                "t = {t}: held {} vs {held}",
                row[sample]
            );
        }
    }

    #[test]
    fn an_initial_equation_section_starts_the_model_in_equilibrium() {
        let result = compile(&with_library("steady_start.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let (temperature, x, v) = (index("T"), index("x"), index("v"));
        let first = &result.rows[0];

        // Heater against losses, and the spring against gravity.
        assert!((first[temperature] - (5.0 + 3000.0 / 250.0)).abs() < 1e-9);
        assert!((first[x] - (-2.0 * 9.81 / 40.0)).abs() < 1e-9);
        assert!(first[v].abs() < 1e-12);
        // Started at the balance point, nothing moves for the whole run.
        for row in &result.rows {
            assert!((row[temperature] - first[temperature]).abs() < 1e-9);
            assert!((row[x] - first[x]).abs() < 1e-9);
            assert!(row[v].abs() < 1e-9);
        }
    }

    #[test]
    fn initialization_mixes_fixed_starts_with_initial_equations() {
        // One state is pinned by `fixed = true`, the other follows from
        // an initial equation that ties the two together.
        let result = run(
            "model M Real a(start = 2, fixed = true); Real b(start = 0); \
             equation der(a) = -a; der(b) = a - b; \
             initial equation b = 3 * a; \
             annotation(experiment(StopTime = 0.1, Interval = 0.05)); end M;",
        );
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let first = &result.rows[0];
        assert!((first[index("a")] - 2.0).abs() < 1e-12);
        assert!((first[index("b")] - 6.0).abs() < 1e-9);
    }

    #[test]
    fn initialization_reports_what_it_cannot_solve() {
        let error = |source: &str| {
            let model = parse_model(source).unwrap();
            compile(&model).unwrap_err().to_string()
        };
        // Two initial equations for one free state.
        assert!(error(
            "model M Real a(start = 1); equation der(a) = -a; \
             initial equation a = 1; der(a) = 0; end M;"
        )
        .contains("not square"));
        // An initial equation that says nothing about the state.
        assert!(error(
            "model M Real a(start = 1); Real b; equation der(a) = -a; b = 2 * a; \
             initial equation b = 2 * a; end M;"
        )
        .contains("singular"));
        // `der` of something that is not a state.
        assert!(error(
            "model M Real a(start = 1); Real b; equation der(a) = -a; b = a; \
             initial equation der(b) = 0; end M;"
        )
        .contains("is not a state"));
    }

    #[test]
    fn the_solver_picks_itself() {
        // A decay with a time constant of a microsecond over a second of
        // simulated time: the explicit method could only crawl through
        // it, so the run should end up on the implicit one - and land on
        // the analytic answer all the same.
        let stiff = run(
            "model S Real x(start = 1, fixed = true); Real slow(start = 1, fixed = true); \
             equation der(x) = -1e6 * (x - slow); der(slow) = -slow; \
             annotation(experiment(StopTime = 1.0, Interval = 0.05, Tolerance = 1e-6)); end S;",
        );
        assert_eq!(stiff.method, SolverMethod::Bdf, "a stiff model needs bdf");
        let slow = stiff.columns.iter().position(|c| c == "slow").unwrap();
        let last = stiff.rows.last().unwrap();
        assert!(
            (last[slow] - (-1.0f64).exp()).abs() < 1e-5,
            "{}",
            last[slow]
        );
        // The fast state follows the slow one, which is the point of the
        // stiff pair.
        let x = stiff.columns.iter().position(|c| c == "x").unwrap();
        assert!((last[x] - last[slow]).abs() < 1e-5);

        // An ordinary model stays where it started, and says so.
        let gentle = run(
            "model G Real x(start = 1, fixed = true); equation der(x) = -x; \
             annotation(experiment(StopTime = 1.0, Interval = 0.1)); end G;",
        );
        assert_eq!(gentle.method, SolverMethod::Dopri45);
        // The default tolerance is 1e-6, so that is what to expect of it.
        assert!((gentle.rows.last().unwrap()[1] - (-1.0f64).exp()).abs() < 1e-6);

        // Asking for a method by name still overrides the choice.
        assert_eq!(SolverMethod::from_name("auto"), Some(SolverMethod::Auto));
        let model = parse_model(
            "model G Real x(start = 1, fixed = true); equation der(x) = -x; \
             annotation(experiment(StopTime = 1.0, Interval = 0.1)); end G;",
        )
        .unwrap();
        let mut compiled = compile(&model).unwrap();
        compiled.method = SolverMethod::Bdf;
        assert_eq!(compiled.simulate().unwrap().method, SolverMethod::Bdf);
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

    #[test]
    fn a_chain_of_masses_written_with_arrays_conserves_energy() {
        // Five bodies between two walls, everything about them arrays:
        // literals, fill, linspace, an array start, whole-array
        // equations and reductions. The check is physical, not textual:
        // the first body is pushed and the energy must stay put.
        let result = compile(&with_library("spring_chain.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let energy = index("energy");
        let first = result.rows[0][energy];
        // 0.5 * m[1] * push^2 with m[1] = 1 and push = 2.
        assert!((first - 2.0).abs() < 1e-9, "{first}");
        for row in &result.rows {
            assert!(
                (row[energy] - first).abs() < 1e-6,
                "drift at t = {}",
                row[0]
            );
        }
        // The bodies start on the linspace grid.
        assert!((result.rows[0][index("x[1]")] - 0.5).abs() < 1e-12);
        assert!((result.rows[0][index("x[5]")] - 2.5).abs() < 1e-12);
    }

    #[test]
    fn a_ladder_of_resistors_wired_by_a_loop_divides_the_supply() {
        // One array declaration, `each R`, and the wiring written as a
        // loop of connects over the elements. Five equal resistors on
        // 10 V put exactly 8, 6, 4, 2, 0 volts on the taps.
        let result = compile(&with_library("resistor_ladder.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let last = result.rows.last().unwrap();
        for i in 1..=5 {
            let expected = 10.0 * (5 - i) as f64 / 5.0;
            let got = last[index(&format!("taps[{i}]"))];
            assert!(
                (got - expected).abs() < 1e-12,
                "tap {i}: {got} vs {expected}"
            );
        }
        // The same current runs through the whole chain.
        let current = last[index("r[1].i")];
        assert!((current - 10.0 / (5.0 * 220.0)).abs() < 1e-15);
        for i in 2..=5 {
            assert!((last[index(&format!("r[{i}].i"))] - current).abs() < 1e-15);
        }
    }

    #[test]
    fn a_pendulum_over_the_top_reselects_its_states() {
        // The known limit of a static selection, now closed: enough
        // speed to rotate fully, so the length constraint has to swap
        // which coordinate it defines every quarter turn.
        let result = compile(&with_library("spinning_pendulum.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        assert!(
            result.reselections >= 4,
            "a full rotation needs several re-selections, saw {}",
            result.reselections
        );
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let (x, y, vx, vy) = (index("x"), index("y"), index("vx"), index("vy"));

        let first = &result.rows[0];
        let energy = |row: &Vec<f64>| 0.5 * (row[vx] * row[vx] + row[vy] * row[vy]) + 9.81 * row[y];
        let e0 = energy(first);
        let mut revolutions = 0;
        for pair in result.rows.windows(2) {
            // The rod length holds exactly through every switch.
            let constraint = pair[1][x] * pair[1][x] + pair[1][y] * pair[1][y] - 1.0;
            assert!(constraint.abs() < 1e-6, "constraint {constraint}");
            // And so does the energy - a wrong branch after a switch
            // (the bug this test was written against) zeroes it.
            assert!(
                (energy(&pair[1]) - e0).abs() < 1e-3,
                "energy drifted to {} from {e0}",
                energy(&pair[1])
            );
            if pair[0][y] < 0.0 && pair[1][y] >= 0.0 && pair[1][x] > 0.0 {
                revolutions += 1;
            }
        }
        assert!(revolutions >= 3, "kept rotating, saw {revolutions}");

        // The whole trajectory agrees with the angle form of the same
        // pendulum - an independent formulation of the same physics.
        let reference = run(
            "model SpinAngle Real th(start = 0, fixed = true); Real w(start = 8, fixed = true); \
             Real x; Real y; equation der(th) = w; der(w) = -9.81 * sin(th); \
             x = sin(th); y = -cos(th); \
             annotation(experiment(StopTime = 3.0, Interval = 0.002, Tolerance = 1e-9)); \
             end SpinAngle;",
        );
        let rx = reference.columns.iter().position(|c| c == "x").unwrap();
        let mut worst = 0.0f64;
        let mut checked = 0;
        for row in &result.rows {
            let Some(matching) = reference
                .rows
                .iter()
                .find(|other| (other[0] - row[0]).abs() < 1e-9)
            else {
                continue;
            };
            worst = worst.max((row[x] - matching[rx]).abs());
            checked += 1;
        }
        assert!(checked > 1000, "grids barely overlap: {checked}");
        assert!(worst < 1e-4, "cartesian vs angle form: {worst}");
    }

    #[test]
    fn a_damper_straight_onto_a_fixed_flange_works() {
        // The former known limit: the damper's relative angle is
        // redundant with the shaft angle, and reducing the index means
        // differentiating a connection equality through connector
        // potentials no equation defines explicitly - they are pinned
        // only linearly. J = 0.5, d = 0.4: the shaft speed must decay
        // as 5*exp(-0.8 t).
        let result = compile(&with_library("damper_on_fixed.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let (w, phi, phi_rel) = (
            index("shaft.w"),
            index("shaft.phi"),
            index("damper.phi_rel"),
        );
        for row in &result.rows {
            let expected = 5.0 * (-0.8 * row[0]).exp();
            assert!(
                (row[w] - expected).abs() < 1e-6,
                "w at {}: {} vs {expected}",
                row[0],
                row[w]
            );
            // The relative angle mirrors the shaft, holding the
            // redundant pair consistent through the whole run.
            assert!((row[phi_rel] + row[phi]).abs() < 1e-9);
        }
    }

    #[test]
    fn a_start_written_through_a_type_alias_is_kept() {
        // `Units.AngularVelocity w(start = w0)` parses its parenthesis
        // as modifiers, not attributes; the initial condition used to
        // vanish without a sound. Found because the damper test above
        // started from rest instead of 5 rad/s.
        let result = run(
            "package Units type Speed = Real(unit = \"m/s\"); end Units; \
             model M parameter Real w0 = 5; parameter Real tau(unit = \"s\") = 1; \
             Units.Speed w(start = w0); \
             equation der(w) = -w / tau; \
             annotation(experiment(StopTime = 1.0, Interval = 0.1)); end M;",
        );
        assert!(
            (result.rows[0][1] - 5.0).abs() < 1e-12,
            "{}",
            result.rows[0][1]
        );
        // And the declaration's own start wins over an alias default.
        let overridden = run("package Units type Speed = Real(start = 7); end Units; \
             model M Units.Speed w(start = 5); equation der(w) = -w; \
             annotation(experiment(StopTime = 1.0, Interval = 0.1)); end M;");
        assert!((overridden.rows[0][1] - 5.0).abs() < 1e-12);
    }

    #[test]
    fn a_replaceable_medium_changes_what_the_tank_holds() {
        // The example file ends with the oil variant, so that is the
        // entry point: heating follows oil's density and heat capacity.
        let oil = compile(&with_library("replaceable_medium.mo"))
            .unwrap()
            .simulate()
            .unwrap();
        let index =
            |result: &SimResult, name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let temperature = index(&oil, "T");
        let last = oil.rows.last().unwrap();
        let expected_oil = 20.0 + 600.0 * 50000.0 / (0.2 * 900.0 * 1900.0);
        assert!(
            (last[temperature] - expected_oil).abs() < 1e-6,
            "oil: {} vs {expected_oil}",
            last[temperature]
        );
        // And the viscosity comes from oil's own function.
        let viscosity = index(&oil, "mu");
        let expected_mu = 0.1 * (-0.05f64 * (expected_oil - 20.0)).exp();
        assert!((last[viscosity] - expected_mu).abs() < 1e-6);

        // The same tank with its default medium heats like water.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = std::fs::read_to_string(root.join("examples/replaceable_medium.mo")).unwrap();
        let water_only = source
            .replace("model OilTank", "partial model OilTank")
            .replace(
                "end OilTank;",
                "end OilTank; model WaterTank extends HeatedTank; \
                 annotation(experiment(StopTime = 600.0, Interval = 1.0)); end WaterTank;",
            );
        let water = compile(&oxidelica_parser::parse_model(&water_only).unwrap())
            .unwrap()
            .simulate()
            .unwrap();
        let temperature = index(&water, "T");
        let expected_water = 20.0 + 600.0 * 50000.0 / (0.2 * 1000.0 * 4186.0);
        let last = water.rows.last().unwrap();
        assert!(
            (last[temperature] - expected_water).abs() < 1e-6,
            "water: {} vs {expected_water}",
            last[temperature]
        );
    }

    #[test]
    fn the_numeric_builtins_follow_their_definitions() {
        // ceil/floor/integer, div/mod/rem - checked at runtime and in
        // the compile-time path, both against the spec definitions.
        let result = run(
            "model N Real u; Real a; Real b; Real c; Real d; Real e; Real f; \
             equation u = 3 * sin(time); \
             a = ceil(u); b = floor(u); c = integer(u); \
             d = div(u, 2.0); e = mod(u, 2.0); f = rem(u, 2.0); \
             annotation(experiment(StopTime = 3.0, Interval = 0.05)); end N;",
        );
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        for row in &result.rows {
            let u = row[index("u")];
            assert_eq!(row[index("a")], u.ceil());
            assert_eq!(row[index("b")], u.floor());
            assert_eq!(row[index("c")], u.floor());
            assert_eq!(row[index("d")], (u / 2.0).trunc());
            assert_eq!(row[index("e")], u - (u / 2.0).floor() * 2.0);
            assert_eq!(row[index("f")], u - (u / 2.0).trunc() * 2.0);
        }
        // Their derivatives are flat almost everywhere: a staircase as
        // a state source integrates without complaint.
        let stepped = run("model S Real x(start = 0, fixed = true); \
             equation der(x) = floor(time) - floor(time); \
             annotation(experiment(StopTime = 1.0, Interval = 0.1)); end S;");
        assert!(stepped.rows.last().unwrap()[1].abs() < 1e-12);
    }

    #[test]
    fn asserts_stop_the_run_with_their_message() {
        // Holding: the run completes.
        let fine = run("model A Real u; equation u = sin(time); \
             assert(u < 2.0, \"cannot happen\"); \
             annotation(experiment(StopTime = 1.0, Interval = 0.1)); end A;");
        assert!(fine.terminated.is_none());

        // Violated: the run stops at the crossing and names the check.
        let model = parse_model(
            "model B Real u; equation u = 3 * sin(time); \
             assert(u < 2.0, \"the input left its window\", AssertionLevel.error); \
             annotation(experiment(StopTime = 3.0, Interval = 0.01)); end B;",
        )
        .unwrap();
        let error = compile(&model).unwrap().simulate().unwrap_err().to_string();
        assert!(error.contains("the input left its window"), "{error}");
        assert!(error.contains("assertion failed at t = 0.73"), "{error}");

        // `block` is a class kind now.
        let block = run("block G Real y; equation y = 2 * time; \
             annotation(experiment(StopTime = 1.0, Interval = 0.5)); end G;");
        assert!((block.rows.last().unwrap()[1] - 2.0).abs() < 1e-12);
    }

    #[test]
    fn matrix_results_survive_the_csv_round_trip() {
        // A 2-D name holds a comma; the CSV quotes it and the value
        // reads back intact.
        let result = run(
            "model M parameter Real A[2, 2] = [1, 2; 3, 4]; Real mm[2, 2]; \
             equation mm = A * transpose(A); \
             annotation(experiment(StopTime = 1.0, Interval = 0.5)); end M;",
        );
        let csv = result.to_csv();
        assert!(csv.contains("\"mm[1,2]\""), "{csv}");
        let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
        let last = result.rows.last().unwrap();
        assert_eq!(last[index("mm[1,1]")], 5.0);
        assert_eq!(last[index("mm[1,2]")], 11.0);
        assert_eq!(last[index("mm[2,2]")], 25.0);
    }

    #[test]
    fn the_compile_time_eval_matches_the_runtime_code() {
        // The same builtins exist twice: in `eval` for compile-time
        // folding and in `Code::run` for the solvers. Feeding them
        // through parameter bindings exercises the eval side; the
        // equations above covered the code side.
        let model = parse_model(
            "model M \
             parameter Real a = ceil(2.3) + floor(2.7) + integer(-1.5); \
             parameter Real b = div(-7, 2) + mod(-7, 2) + rem(-7, 2); \
             parameter Real c = abs(-3) + sign(-2) + sqrt(16) + min(1, 2) + max(1, 2); \
             parameter Real d = atan2(1, 1) + log10(100) + sinh(0) + cosh(0) + tanh(0); \
             parameter Real e = asin(1) + acos(1) + atan(0) + tan(0) + exp(0) + log(1); \
             Real x; equation x = a + b + c + d + e; end M;",
        )
        .unwrap();
        let compiled = compile(&model).unwrap();
        let value = |name: &str| {
            compiled
                .parameters
                .iter()
                .find(|(n, _)| n == name)
                .unwrap()
                .1
        };
        assert_eq!(value("a"), 3.0 + 2.0 + (-2.0));
        // div truncates toward zero, mod follows the floor, rem the
        // truncation: -3 + 1 + (-1).
        assert_eq!(value("b"), -3.0);
        assert_eq!(value("c"), 3.0 - 1.0 + 4.0 + 1.0 + 2.0);
        assert!((value("d") - (std::f64::consts::FRAC_PI_4 + 2.0 + 1.0)).abs() < 1e-12);
        assert!((value("e") - (std::f64::consts::FRAC_PI_2 + 1.0)).abs() < 1e-12);
    }

    #[test]
    fn relations_and_logic_fold_at_compile_time() {
        // The comparison and boolean arms of the constant folder.
        let model = parse_model(
            "model M \
             parameter Boolean q = 1 < 2 and 2 <= 2 and 3 > 2 and 2 >= 2 \
               and 1 == 1 and 1 <> 2 or false; \
             parameter Real k = if q and not false then 7 else 9; \
             Real x; equation x = k; end M;",
        )
        .unwrap();
        let compiled = compile(&model).unwrap();
        let k = compiled
            .parameters
            .iter()
            .find(|(n, _)| n == "k")
            .unwrap()
            .1;
        assert_eq!(k, 7.0);
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
