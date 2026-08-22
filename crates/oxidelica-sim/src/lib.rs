//! oxidelica-sim — compiles a flat model into an executable form and
//! integrates it (adaptive Dormand-Prince by default, RK4 optional).
//!
//! State equations must isolate `der(x)` on one side; algebraic
//! equations are general — a bipartite matcher pairs them with
//! unknowns, explicit assignments evaluate directly and simultaneous
//! blocks are solved by Newton iteration.

#![deny(missing_docs)]

use oxidelica_parser::{
    operator_name, BinOp, Causality, ClassDef, Component, EquationItem, Expr, Model, RelOp,
    Statement, Variability, WhenAction, WhenBranch, WhenClause,
};
use std::collections::HashMap;
use std::fmt;

mod code;
mod compile;
mod continuation;
mod events;
mod linear;
mod result;
mod solvers;
mod symbolic;
#[cfg(test)]
mod tests;
mod walk;

pub use compile::compile;

use code::*;
use compile::*;
use continuation::*;
use linear::*;
use symbolic::*;

/// A compilation or simulation error.
#[derive(Debug)]
pub struct SimError(pub String);

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SimError {}

/// Where a refusal was made, when the run was asked for it.
///
/// A message says what was wrong with the model; it does not say which
/// of the several places that could have raised it did. Finding that
/// out meant a debugger, and a debugger on this codebase reads `String`
/// and `Expr` as addresses - it answers "where" well and "what" not at
/// all, which is the wrong half. `OXIDELICA_WHERE=1` puts the file and
/// line on the message and answers the half a debugger was wanted for,
/// on every platform and without one.
#[track_caller]
fn err<T>(message: impl Into<String>) -> Result<T, SimError> {
    let message = message.into();
    let message = match std::env::var_os("OXIDELICA_WHERE").is_some() {
        true => {
            let at = std::panic::Location::caller();
            format!("{message} [{}:{}]", at.file(), at.line())
        }
        false => message,
    };
    Err(SimError(message))
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

/// One `delay(u, T)`: where its value is read from, what to remember,
/// and how far back to look.
#[derive(Debug)]
struct CompiledDelay {
    /// The slot the delayed value is written into before each point.
    slot: Slot,
    /// The expression whose past is being kept.
    source: Code,
    /// How far back to look.
    seconds: f64,
}

/// One `spatialDistribution(...)`: a profile carried along a
/// coordinate, and the two slots the run reads it from.
///
/// The memory is the same shape as a delay's - a rising sequence of
/// (key, value) - but the key is a position rather than a time. The
/// key that makes both directions the same shape is `k = x - ξ`: a
/// value keeps its key while the coordinate moves, which is precisely
/// what being carried along means. The pipe is then the window
/// `k ∈ [x - 1, x]`, its two ends are the two slots, and whichever end
/// the flow enters by is the one being written.
#[derive(Debug)]
struct CompiledTransport {
    /// The profile at ξ = 0, which is `k = x`.
    at_zero_slot: Slot,
    /// The profile at ξ = 1, which is `k = x - 1`.
    at_one_slot: Slot,
    /// What enters at ξ = 0, and at ξ = 1.
    in0: Code,
    /// See [`CompiledTransport::in0`].
    in1: Code,
    /// The integral of the velocity.
    x: Code,
    /// Which way it is going, as 1.0 or 0.0.
    positive: Code,
    /// The profile the run starts from, as (position, value) pairs
    /// already turned into entry coordinates once `x` is known.
    initial: Vec<(f64, f64)>,
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
    /// Slot of `$terminal`, raised once the run reaches its stop time.
    terminal_slot: Slot,
    /// The delayed expressions, each with the slot its value is read
    /// from and how far back it looks.
    delays: Vec<CompiledDelay>,
    /// What each delayed expression has been, kept as the run goes.
    /// Only the run touches it, and a run has the model to itself.
    history: std::cell::RefCell<Vec<Vec<(f64, f64)>>>,
    /// The `spatialDistribution` operators, and the profile each one
    /// carries. Kept beside the delays, and for the same reason.
    transports: Vec<CompiledTransport>,
    /// See [`CompiledModel::transports`].
    profiles: std::cell::RefCell<Vec<Vec<(f64, f64)>>>,
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
    /// One entry per run-time `if` equation: the conditions and the
    /// branch this compilation was made for. A branch that no longer
    /// holds means the model on hand is the wrong one for where the
    /// run has got to.
    mode_monitor: Vec<(Vec<Code>, usize)>,
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
    /// The bodies this run walks, and where a walk leaves its reason
    /// when one fails.
    walked: std::sync::Arc<Walked>,
    /// Event indicators: expressions whose sign change marks an event.
    /// Built from every relation in the model, so switching branches of
    /// an `if` expression are located exactly, not stepped over.
    indicators: Vec<Code>,
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
    /// Whether a run-time `if` equation changed branch, rather than
    /// the state selection going bad: a mode change is ordinary
    /// business and may happen as often as the model switches.
    mode_change: bool,
}

/// What the event machinery carries between events.
#[derive(Clone, Debug)]
struct EventState {
    /// Truth of every `when` branch as of the previous event.
    when_prev: Vec<Vec<bool>>,
    /// Next occurrence of each `sample(...)` source.
    next_sample: Vec<f64>,
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
    /// Delayed expressions found so far, with how far back each looks.
    delays: Vec<(Expr, f64)>,
}

/// A point a continuation starts from: the time reached and the value
/// of every variable there, by name.
struct ResumePoint<'a> {
    /// Where the previous segment stopped.
    time: f64,
    /// Values of every variable at that instant.
    values: &'a HashMap<String, f64>,
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

/// Guards against cyclic algebraic definitions while differentiating.
const MAX_DIFF_DEPTH: usize = 32;

/// How many times to follow plain `name = expr` definitions while
/// working out which branch of a run-time `if` equation holds at the
/// point being compiled. A chain longer than this is not worth
/// chasing: the branch falls back to the `else`.
const MAX_DEFINITION_PASSES: usize = 16;

// --- expression evaluation ---

struct EvalCtx<'a> {
    vars: &'a HashMap<String, f64>,
    time: f64,
    /// The bodies the run walks for itself, where there are any: a call
    /// to one of these is answered by walking it rather than by any
    /// built-in rule.
    programs: Option<&'a HashMap<String, ClassDef>>,
    /// How deep the walking has gone, so a function calling itself for
    /// ever is stopped rather than running the stack out.
    depth: usize,
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
    /// `Integer(e)` — the ordinal of an enumeration value.
    Ordinal,
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
    NthRoot,
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
    /// A call to a body the run walks, by the name the model knows it
    /// by, with the arguments to work out first and how many numbers
    /// each of them is: zero for one number, otherwise the length of
    /// the array it was written out as.
    /// The last field is which number of the answer this stands for:
    /// a body answering with an array is asked once for each.
    Program(std::sync::Arc<Walked>, String, Vec<Code>, Vec<usize>, usize),
    /// A call to a body written outside Modelica and answered here in
    /// Rust: the name it is called by outside, the arguments to work
    /// out first, and which number of the answer this stands for.
    /// Nothing can go wrong that is not a mistake in this compiler, so
    /// unlike a walk there is no reason to leave behind.
    Outside(String, Vec<Code>, usize),
    /// A one-argument built-in.
    Unary(Unary, Box<Code>),
    /// A two-argument built-in.
    Binary(Binary, Box<Code>, Box<Code>),
}

/// Names of the variables of a model, each with the slot it occupies.
/// The bodies a run walks, and the first thing that went wrong while
/// walking one.
///
/// `Code::run` answers with a number and has no way to say "this went
/// wrong", so a walk that fails leaves its reason here and answers with
/// a number that is not one. Whoever evaluated the point reads the
/// reason back out and stops the run with it.
#[derive(Debug)]
pub(crate) struct Walked {
    pub(crate) programs: HashMap<String, ClassDef>,
    pub(crate) trouble: std::sync::Mutex<Option<String>>,
}

impl Walked {
    /// Take the reason a walk failed, if one did.
    pub(crate) fn complaint(&self) -> Option<String> {
        self.trouble.lock().ok().and_then(|mut held| held.take())
    }
}

struct SlotTable {
    /// Slot of every known name.
    index: HashMap<String, Slot>,
    /// The bodies the run walks, shared with every call compiled.
    walked: std::sync::Arc<Walked>,
    /// The value array as a run starts it: parameters already in place,
    /// everything else zero.
    template: Vec<f64>,
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
