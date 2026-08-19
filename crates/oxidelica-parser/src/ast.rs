//! AST of the M0 Modelica slice. A flat model: declarations plus equations.

/// The local name an unqualified import is filed under. `import A.B.*;`
/// gives no name to anything, so it is filed under one no identifier
/// can spell, and name resolution treats it as a place to look rather
/// than as a name of its own.
pub const WILDCARD_IMPORT: &str = "*";

/// How a component's value may change over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variability {
    /// An ordinary continuous variable.
    Continuous,
    /// `parameter` — fixed for the duration of a simulation.
    Parameter,
    /// `constant`.
    Constant,
    /// `discrete` — a variable that keeps its value between events and
    /// changes only where a `when` clause assigns it.
    Discrete,
}

/// Where a component sits in the instance hierarchy.
///
/// An `inner` declaration owns a shared instance; an `outer` one is a
/// reference to the nearest enclosing `inner` of the same name and
/// creates no variables of its own (the `world` of a mechanical model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    /// An ordinary declaration, visible in its own class only.
    #[default]
    Local,
    /// `inner` — owns the instance others refer to.
    Inner,
    /// `outer` — refers to an enclosing `inner` instance.
    Outer,
}

/// A `redeclare <Type> <name>(<modifiers>)` entry of a modifier list:
/// it replaces the type of a `replaceable` declaration further down.
#[derive(Debug, Clone)]
pub struct Redeclare {
    /// Name of the declaration being replaced.
    pub name: String,
    /// New type; qualified in the scope where the redeclaration is written.
    pub type_name: String,
    /// Modifiers applied to the new type.
    pub modifiers: Vec<(String, Expr)>,
    /// Whether this replaces a class alias rather than a component:
    /// `redeclare package Medium = Oil` swaps what the name `Medium`
    /// stands for inside the class that declared it replaceable.
    pub class_level: bool,
}

/// A short class definition: `package Medium = Media.Water;` gives the
/// enclosing class a local name for another class. Marked `replaceable`
/// it is the hook the Fluid-style libraries hang their media on.
#[derive(Debug, Clone)]
pub struct ClassAlias {
    /// The local name.
    pub name: String,
    /// The class it stands for, as written; resolved where declared.
    pub target: String,
    /// Whether a redeclaration may replace the target.
    pub replaceable: bool,
    /// `redeclare package Medium = X;` in a class body: replaces the
    /// alias of a base class instead of introducing one.
    pub redeclaration: bool,
    /// The interface a replacement must extend, when given.
    pub constrained_by: Option<String>,
}

/// A component (variable) declaration.
#[derive(Debug, Clone)]
pub struct Component {
    /// Component name.
    pub name: String,
    /// Type name: `Real` or a user class (model/connector).
    pub type_name: String,
    /// Whether the component carries the `flow` prefix (connectors).
    pub flow: bool,
    /// Whether the component carries the `stream` prefix: a quantity
    /// transported by the flow, mixed by `inStream` at a connection.
    pub stream: bool,
    /// Array dimensions: `Real T[N, 3]` gives two of them. Empty for
    /// scalars; expanded into scalar components while flattening.
    pub dimensions: Vec<Expr>,
    /// `input` / `output` prefix (function arguments and results).
    pub causality: Causality,
    /// Modifiers for user-type components: `Resistor r(R = 100)`.
    pub modifiers: Vec<(String, Expr)>,
    /// Variability class of the component.
    pub variability: Variability,
    /// The `start` attribute from the modifier: `Real x(start = 1.0)`.
    pub start: Option<Expr>,
    /// The `fixed` attribute.
    pub fixed: Option<bool>,
    /// The `unit` attribute: `Real v(unit = "V")`, or inherited from a
    /// type alias. Feeds the dimensional check; `None` is unchecked.
    pub unit: Option<String>,
    /// The `min` and `max` attributes. Modelica calls these
    /// assertions on the value, so they become run-time checks rather
    /// than anything the solver is told about.
    pub min: Option<Expr>,
    /// See [`Component::min`].
    pub max: Option<Expr>,
    /// Declaration binding: `parameter Real a = 1.0`.
    pub binding: Option<Expr>,
    /// Optional description string.
    pub description: Option<String>,
    /// `inner` / `outer` prefix.
    pub scope: Scope,
    /// Whether the declaration may be redeclared from above.
    pub replaceable: bool,
    /// `constrainedby Interface` — the class a redeclaration must extend.
    pub constrained_by: Option<String>,
    /// `if <condition>` — the component exists only when the condition,
    /// a compile-time constant, holds.
    pub condition: Option<Expr>,
    /// Redeclarations written in this component's modifier list.
    pub redeclares: Vec<Redeclare>,
    /// `redeclare` prefix: the declaration replaces an inherited
    /// `replaceable` one instead of adding a component of its own.
    pub redeclaration: bool,
    /// `final` prefix: the declaration may not be modified or
    /// redeclared from an enclosing class.
    pub is_final: bool,
    /// Names of the modifiers written with `each`: on an array
    /// component, `each` spreads a value over every element rather than
    /// handing the elements the slices of it.
    pub each_modifiers: Vec<String>,
    /// What the declaration's annotation said: where a diagram puts it,
    /// which dialog group it belongs to, whether its value is worth
    /// writing down.
    pub annotations: Vec<Expr>,
}

/// The operator a call names, with the leading dot of a global
/// reference taken off.
///
/// `.asin(u)` is the language's own `asin` looked up from the top of
/// the tree - which is how a library that writes its own `asin` reaches
/// past it. Once nothing in the tree has answered to the name, the dot
/// has said all it had to say and the two spellings are one operator.
pub fn operator_name(name: &str) -> &str {
    name.strip_prefix('.').unwrap_or(name)
}

/// A single equation `lhs = rhs`.
#[derive(Debug, Clone)]
pub struct EquationItem {
    /// Left-hand side expression.
    pub lhs: Expr,
    /// Right-hand side expression.
    pub rhs: Expr,
    /// The instance this equation was written inside, by its flat path.
    /// Empty for one the compiler made up, and for the top-level model
    /// itself. It is what tells a state's own equations apart when what
    /// they define lives outside the state.
    pub origin: String,
}

impl EquationItem {
    /// An equation belonging to nothing in particular.
    pub fn new(lhs: Expr, rhs: Expr) -> EquationItem {
        EquationItem {
            lhs,
            rhs,
            origin: String::new(),
        }
    }
}

/// Direction of a function argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Causality {
    /// An ordinary variable.
    #[default]
    None,
    /// A function argument.
    Input,
    /// A function result.
    Output,
}

/// One branch of an `if` statement inside an algorithm.
#[derive(Debug, Clone)]
pub struct StatementBranch {
    /// The condition; `None` marks the `else` branch.
    pub condition: Option<Expr>,
    /// Statements of the branch.
    pub body: Vec<Statement>,
}

/// One statement of an algorithm section.
#[derive(Debug, Clone)]
pub enum Statement {
    /// `target := value;` — the subscripts are empty for a plain
    /// target, and hold the indices of `c[i] := value;`.
    Assign(String, Vec<Expr>, Expr),
    /// `(a, , c) := f(...);` — one function call fills several
    /// targets, `None` where an output is skipped. A target may carry
    /// subscripts, as `(v[1], info) := f(...)` does, where only part
    /// of an array is being filled.
    TupleAssign(Vec<Option<(String, Vec<Expr>)>>, Expr),
    /// `if c then … elseif … else … end if;`
    If(Vec<StatementBranch>),
    /// `for i in lo:hi loop … end for;` — the range is whatever the
    /// values are, and `None` where the loop left it to the body.
    For(String, Option<Expr>, Vec<Statement>),
    /// `while c loop … end while;` — executed at compile time, so the
    /// condition must be decidable there, each round.
    While(Expr, Vec<Statement>),
    /// `break;` — leave the innermost `for` or `while`.
    Break,
    /// `return;` — leave the function, outputs as they stand.
    Return,
    /// `when c then … elsewhen … end when;` among the statements: what
    /// it holds happens at an event and nowhere else.
    When(Vec<StatementBranch>),
    /// `assert(condition, "message");` — a check written where the
    /// statements are rather than among the equations.
    Assert(Expr, String),
    /// `f(x);` — a call standing on its own. Nothing receives its
    /// outputs, so what is left of it is the checks its body makes.
    Call(String, Vec<Expr>),
}

/// The kind of a class definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassKind {
    /// `model` — a component or the top-level system.
    Model,
    /// `connector` — an interface with potential and flow variables.
    Connector,
    /// `record` — a plain bundle of variables.
    Record,
    /// `function` — inputs, one output and an algorithm of assignments.
    Function,
    /// `package` — a namespace of classes.
    Package,
    /// `type` — a named alias of a primitive with attribute defaults.
    Type,
}

/// An `extends Base(mod = expr, ...);` clause.
#[derive(Debug, Clone)]
pub struct Extend {
    /// Name of the base class.
    pub base: String,
    /// Modifier overrides applied to the base.
    pub modifiers: Vec<(String, Expr)>,
    /// Redeclarations applied to the base.
    pub redeclares: Vec<Redeclare>,
    /// Selective extension: elements of the base to leave out.
    pub broken: Vec<Deselect>,
}

/// One `break` of a selective `extends`: an element of the base that
/// the extending class leaves out.
#[derive(Debug, Clone)]
pub enum Deselect {
    /// `break f` — the component `f` and every connection to it.
    Component(String),
    /// `break connect(a, b)` — that one connection.
    Connection(String, String),
}

/// One class definition in a file.
#[derive(Debug, Clone)]
pub struct ClassDef {
    /// Model or connector.
    pub kind: ClassKind,
    /// Class name, qualified with its enclosing packages.
    pub name: String,
    /// `partial` classes are bases only: they cannot be instantiated.
    pub partial: bool,
    /// `encapsulated` — a class that does not see its enclosing scope:
    /// a simple name inside it is looked up in the class and its
    /// imports and nowhere further out.
    pub encapsulated: bool,
    /// `expandable connector` — a bus whose members are whatever the
    /// connections to it name, rather than what it declares.
    pub expandable: bool,
    /// For `type` aliases: the primitive being named, plus attribute
    /// defaults (`type Voltage = Real(start = 0)`).
    pub alias_of: Option<(String, Vec<(String, Expr)>)>,
    /// The `unit` attribute of a type alias:
    /// `type Voltage = Real(unit = "V")`.
    pub alias_unit: Option<String>,
    /// Literals of an enumeration type, in declaration order. A
    /// reference `Init.SteadyState` is their 1-based position.
    pub enumeration: Vec<String>,
    /// Classes declared inside a package, by qualified name.
    pub nested: Vec<ClassDef>,
    /// Short class definitions: local names for other classes.
    pub class_aliases: Vec<ClassAlias>,
    /// `import` clauses: (local name, qualified target).
    pub imports: Vec<(String, String)>,
    /// Optional description string.
    pub description: Option<String>,
    /// A body written outside Modelica: `external "C" ...`. The class
    /// is read so that the file holding it loads, and a call to it is
    /// refused where the call is made.
    pub external: bool,
    /// `external "builtin" y = asin(u);` — the function is the
    /// language's own operator of that name, spelled out so that a
    /// library can give it a place in its tree. The name is kept and a
    /// call becomes a call to the operator.
    pub builtin: Option<String>,
    /// Component declarations.
    pub components: Vec<Component>,
    /// `extends` clauses.
    pub extends: Vec<Extend>,
    /// Ordinary equations.
    pub equations: Vec<EquationItem>,
    /// Equations of the `initial equation` section: they hold at the
    /// start and decide the state the simulation begins from.
    pub initial_equations: Vec<EquationItem>,
    /// `assert(condition, "message")` checks of the equation section.
    pub asserts: Vec<(Expr, String)>,
    /// The arrows of a state machine declared in this class.
    pub transitions: Vec<Transition>,
    /// The state a machine starts in, from `initialState(s)`.
    pub initial_state: Option<String>,
    /// `Connections.root`, `potentialRoot` and `branch` clauses.
    pub connection_graph: Vec<GraphClause>,
    /// `for` equations, unrolled while flattening.
    pub for_equations: Vec<ForEquation>,
    /// `if` equations, resolved while flattening.
    pub if_equations: Vec<IfEquation>,
    /// Statements of an `algorithm` section: the body of a function, or
    /// a block of a model that is executed into equations.
    pub algorithm: Vec<Statement>,
    /// Statements of an `initial algorithm` section: they run once,
    /// before the simulation starts, and what they assign belongs to
    /// the initial system rather than holding throughout.
    pub initial_algorithm: Vec<Statement>,
    /// `connect(a, b);` statements. Each side is an expression so that
    /// a reference may carry subscripts (`pins[i]`, `a[2].p`) or name a
    /// whole array of connectors; flattening resolves both to instance
    /// paths.
    pub connects: Vec<(Expr, Expr)>,
    /// `when` clauses (events).
    pub when_clauses: Vec<WhenClause>,
    /// Experiment settings.
    pub experiment: Experiment,
    /// `annotation(derivative = f_der)`: the function that gives this
    /// one's derivative, so a tool need not work it out from the body.
    pub derivative: Option<String>,
    /// `annotation(inverse(x = f_inv(y, z)))`: which input this class
    /// can be solved for, by which function, given which arguments.
    pub inverse: Vec<(String, String, Vec<String>)>,
    /// Everything else the class annotation said, as written: the
    /// drawing of an `Icon`, a `Documentation`, a `version`. None of it
    /// changes what a run does, and all of it is what a tool around the
    /// run reads.
    pub annotations: Vec<Expr>,
}

impl ClassDef {
    /// A class with nothing in it, to be filled in field by field. The
    /// short forms - a type alias, an enumeration - are a name and one
    /// or two fields against two dozen empty ones, and this is what
    /// spares them from spelling every one of those out.
    pub fn empty() -> Self {
        ClassDef {
            kind: ClassKind::Model,
            name: String::new(),
            partial: false,
            encapsulated: false,
            expandable: false,
            alias_of: None,
            alias_unit: None,
            enumeration: Vec::new(),
            nested: Vec::new(),
            class_aliases: Vec::new(),
            imports: Vec::new(),
            description: None,
            external: false,
            builtin: None,
            components: Vec::new(),
            extends: Vec::new(),
            equations: Vec::new(),
            initial_equations: Vec::new(),
            asserts: Vec::new(),
            transitions: Vec::new(),
            initial_state: None,
            connection_graph: Vec::new(),
            for_equations: Vec::new(),
            if_equations: Vec::new(),
            algorithm: Vec::new(),
            initial_algorithm: Vec::new(),
            connects: Vec::new(),
            when_clauses: Vec::new(),
            experiment: Experiment::default(),
            derivative: None,
            inverse: Vec::new(),
            annotations: Vec::new(),
        }
    }
}

/// What a `when` clause does when its condition becomes true.
#[derive(Debug, Clone)]
pub enum WhenAction {
    /// `reinit(state, expr)` — restart integration of `state` from a
    /// new value (a bouncing ball reversing its velocity).
    Reinit(String, Expr),
    /// `terminate("message")` — end the simulation.
    Terminate(String),
    /// `x = expr` — the new value of a discrete variable. It holds that
    /// value until another event assigns it.
    Assign(String, Expr),
    /// `(a, , c) = f(...);` — one call fills several targets at the
    /// event, `None` where an output is skipped. Flattening inlines
    /// the call and hands each target its own assignment, so nothing
    /// downstream meets this form.
    TupleAssign(Vec<Option<String>>, Expr),
}

/// One branch of a `when` clause: `when c1 then … elsewhen c2 then …`.
#[derive(Debug, Clone)]
pub struct WhenBranch {
    /// The condition; the branch fires on its false-to-true edge.
    pub condition: Expr,
    /// Actions performed when it fires.
    pub actions: Vec<WhenAction>,
}

/// One item of a `for` loop body.
#[derive(Debug, Clone)]
pub enum ForBody {
    /// An ordinary equation.
    Equation(EquationItem),
    /// A `connect` between references that may use the loop variable.
    Connect(Expr, Expr),
    /// A nested loop.
    Nested(ForEquation),
}

/// A `for <var> in <lo>:<hi> loop <body> end for;` clause.
#[derive(Debug, Clone)]
pub struct ForEquation {
    /// The loop variable, visible inside the body.
    pub variable: String,
    /// What the variable runs over: a range, a set, an array - anything
    /// the compiler can work out the values of. `None` where the loop
    /// left it for the body to say.
    pub range: Option<Expr>,
    /// Equations and nested loops of the body.
    pub body: Vec<ForBody>,
}

/// One branch of an `if` equation.
#[derive(Debug, Clone)]
pub struct IfBranch {
    /// The condition; `None` marks the `else` branch.
    pub condition: Option<Expr>,
    /// Equations the branch contributes.
    pub equations: Vec<EquationItem>,
    /// `connect` statements the branch contributes.
    pub connects: Vec<(Expr, Expr)>,
    /// `assert(condition, "message")` checks written in the branch.
    /// They hold only while the branch is the one taken, so what
    /// flattening emits for them carries that guard.
    pub asserts: Vec<(Expr, String)>,
    /// `for` equations written in the branch. A loop unrolls into
    /// equations, so it can only be part of a branch the compiler
    /// picks: how many equations it makes is settled before the run.
    pub loops: Vec<ForEquation>,
}

/// A statement about the overconstrained connection graph.
#[derive(Debug, Clone)]
pub enum GraphClause {
    /// `Connections.root(a)` — this node is a root and stays one.
    Root(String),
    /// `Connections.potentialRoot(a, priority)` — a root where one is
    /// needed; lower priority is preferred.
    PotentialRoot(String, i64),
    /// `Connections.branch(a, b)` — the two are joined in the graph
    /// whether or not they are connected in the ordinary way.
    Branch(String, String),
}

/// One arrow of a state machine: `transition(from, to, condition, …)`.
#[derive(Debug, Clone)]
pub struct Transition {
    /// The state it leaves.
    pub from: String,
    /// The state it arrives at.
    pub to: String,
    /// What has to hold for it to be taken.
    pub condition: Expr,
    /// Whether the arrival's variables go back to their start values.
    pub reset: bool,
    /// Whether the arrow is taken on this tick's condition, which is
    /// the default, or waits a tick and is taken on the one before.
    pub immediate: bool,
    /// Whether the arrow waits for the machines inside the state it
    /// leaves to have reached a state no arrow leaves.
    pub synchronize: bool,
    /// Which arrow wins when several could be taken; lower goes first.
    pub priority: i64,
}

/// An `if <cond> then … elseif … else … end if;` in an equation section.
///
/// The conditions are compile-time constants: the first branch that
/// holds contributes its equations and the others contribute nothing.
/// This is how the standard library gives a component different
/// equations depending on a structural parameter.
#[derive(Debug, Clone)]
pub struct IfEquation {
    /// Branches in source order, the `else` one last where present.
    pub branches: Vec<IfBranch>,
}

/// A `when <condition> then <actions> [elsewhen …] end when;` clause.
///
/// At an event the first branch whose condition just became true fires,
/// and the others stay silent — the priority `elsewhen` gives.
#[derive(Debug, Clone)]
pub struct WhenClause {
    /// Branches in source order.
    pub branches: Vec<WhenBranch>,
}

/// Simulation settings from `annotation(experiment(...))`.
#[derive(Debug, Clone, Default)]
pub struct Experiment {
    /// `StopTime` — simulation end time.
    pub stop_time: Option<f64>,
    /// `Interval` — output/integration step.
    pub interval: Option<f64>,
    /// `Tolerance` — solver tolerance (reserved for adaptive solvers).
    pub tolerance: Option<f64>,
}

/// An `if` equation whose condition only the run can decide.
///
/// Every branch holds the same number of equations, so the model is
/// square whichever one applies. Which one that is gets settled when
/// the model is compiled, and the run asks for a fresh compilation
/// when a condition flips: that way each mode is matched, torn and
/// solved as the equations of that mode, rather than as one blurred
/// residual standing for all of them.
#[derive(Debug, Clone)]
pub struct ConditionalEquations {
    /// One condition per branch except the final `else`.
    pub conditions: Vec<Expr>,
    /// The equations of each branch, scalar and in source order.
    pub branches: Vec<Vec<EquationItem>>,
}

/// One `spatialDistribution(...)`: a quantity carried along a
/// coordinate rather than held for a time.
///
/// The profile lives on ξ ∈ [0, 1] and moves with the velocity whose
/// integral is `x`. What enters at one end leaves at the other once
/// `x` has moved by one, so the memory is indexed by position rather
/// than by the clock - which is the whole difference from `delay`.
#[derive(Debug, Clone)]
pub struct SpatialTransport {
    /// Where `z(0, t)` is written.
    pub out0: String,
    /// Where `z(1, t)` is written.
    pub out1: String,
    /// What enters at ξ = 0 while the velocity is positive.
    pub in0: Expr,
    /// What enters at ξ = 1 while it is negative.
    pub in1: Expr,
    /// The integral of the velocity.
    pub x: Expr,
    /// Which way the transport is going.
    pub positive: Expr,
    /// The profile the run starts from: positions in [0, 1], sorted,
    /// with the value carried at each.
    pub initial_points: Vec<f64>,
    /// See [`SpatialTransport::initial_points`].
    pub initial_values: Vec<f64>,
}

/// A parsed flat model.
#[derive(Debug, Clone)]
pub struct Model {
    /// Model name.
    pub name: String,
    /// Optional description string after the model name.
    pub description: Option<String>,
    /// Component declarations in source order.
    pub components: Vec<Component>,
    /// Equations in source order.
    pub equations: Vec<EquationItem>,
    /// Equations that hold only at the start.
    pub initial_equations: Vec<EquationItem>,
    /// Conditions that must hold at every evaluated point, with the
    /// message reported when one does not.
    pub asserts: Vec<(Expr, String)>,
    /// `when` clauses (events).
    pub when_clauses: Vec<WhenClause>,
    /// `if` equations settled while running rather than while
    /// compiling.
    pub conditional: Vec<ConditionalEquations>,
    /// The arrows of the state machines, with the states named by
    /// their instance paths.
    pub transitions: Vec<Transition>,
    /// Every `spatialDistribution` of the model.
    pub transports: Vec<SpatialTransport>,
    /// Where each machine starts, by instance path.
    pub initial_states: Vec<String>,
    /// Graph clauses with nodes named by instance path.
    pub connection_graph: Vec<GraphClause>,
    /// Experiment settings (defaults when absent).
    pub experiment: Experiment,
    /// The functions the run has to walk for itself, because inlining
    /// them was not possible: a recursive one has no bottom to inline
    /// to, and a loop whose trip count the model decides has no length.
    /// Everything they call is here too.
    pub functions: Vec<ClassDef>,
}

/// Binary arithmetic operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `^`
    Pow,
}

/// Relational operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelOp {
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `==`
    Eq,
    /// `<>`
    Ne,
}

/// An expression tree node.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A string literal. Strings are settled before the run - there is
    /// no place for one in the numeric arrays a step works on - so a
    /// `String` that survives into the equations is refused.
    Str(String),
    /// Numeric literal.
    Number(f64),
    /// Boolean literal.
    Bool(bool),
    /// Component reference; in a flat M0 model just a name
    /// (`x`; dotted `body.m` arrives with M2).
    Ref(String),
    /// The built-in `time` variable.
    Time,
    /// A call to a function that said how to differentiate itself: what
    /// it works out to, the rule its `derivative` annotation gives, and
    /// a name standing in for each argument's own derivative.
    ///
    /// The value is what a run computes; the rule is what
    /// differentiation reaches for instead of taking the body apart.
    /// Keeping both is what lets a body the differentiator cannot read -
    /// one with `abs` in it, say - still be differentiated.
    WithDerivative(Box<Expr>, Box<Expr>, Vec<(String, Expr)>),
    /// Function call, including `der(x)`.
    Call(String, Vec<Expr>),
    /// Unary minus.
    Neg(Box<Expr>),
    /// Binary arithmetic.
    Bin(BinOp, Box<Expr>, Box<Expr>),
    /// Relational comparison.
    Rel(RelOp, Box<Expr>, Box<Expr>),
    /// Logical `and`.
    And(Box<Expr>, Box<Expr>),
    /// Logical `or`.
    Or(Box<Expr>, Box<Expr>),
    /// Logical `not`.
    Not(Box<Expr>),
    /// `if c then a else b` (`elseif` chains become nested `If`s).
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    /// Array subscript `name[i, j]`; resolved to a scalar reference
    /// while flattening.
    Index(Box<Expr>, Vec<Expr>),
    /// Member of a subscripted component: `points[i].x`.
    Member(Box<Expr>, String),
    /// An array written out: `{1, 2, 3}`, or `{{1, 2}, {3, 4}}` for a
    /// matrix. Arrays are a compile-time shape here: flattening turns
    /// them into the scalars underneath.
    Array(Vec<Expr>),
    /// An operator written with a dot - `.*`, `./`, `.^` - which works
    /// element by element even where the plain one would not.
    Elementwise(BinOp, Box<Expr>, Box<Expr>),
    /// A range `a:b` or `a:step:b`, a vector value whose bounds are
    /// compile-time constants.
    Range(Box<Expr>, Option<Box<Expr>>, Box<Expr>),
    /// `{expr for i in range}` - an array built by iterating.
    Comprehension(Box<Expr>, String, Box<Expr>),
    /// A bare `:` as a subscript: the whole of that dimension.
    ColonSubscript,
    /// `end` inside a subscript: the length of that dimension.
    EndSubscript,
    /// `[a, b; c, d]` - rows of a matrix, elements within a row
    /// concatenated along the second dimension, rows along the first.
    MatrixRows(Vec<Vec<Expr>>),
    /// `gain = 2` inside a call's argument list; matched against the
    /// function's inputs by name when the call is inlined.
    NamedArg(String, Box<Expr>),
    /// `(a, , c)` on the left of an equation: targets for a function
    /// with several outputs, `None` where one is skipped.
    Tuple(Vec<Option<Expr>>),
}

impl Expr {
    /// If the expression is exactly `der(<name>)`, return the state name.
    pub fn as_der_of(&self) -> Option<&str> {
        if let Expr::Call(name, args) = self {
            if name == "der" && args.len() == 1 {
                if let Expr::Ref(var) = &args[0] {
                    return Some(var);
                }
            }
        }
        None
    }

    /// Whether the expression contains a `der(...)` call anywhere.
    pub fn contains_der(&self) -> bool {
        match self {
            Expr::Call(name, args) => name == "der" || args.iter().any(Expr::contains_der),
            Expr::Neg(inner) | Expr::Not(inner) => inner.contains_der(),
            Expr::Bin(_, l, r) | Expr::Rel(_, l, r) | Expr::And(l, r) | Expr::Or(l, r) => {
                l.contains_der() || r.contains_der()
            }
            Expr::If(c, t, e) => c.contains_der() || t.contains_der() || e.contains_der(),
            Expr::Index(base, subscripts) => {
                base.contains_der() || subscripts.iter().any(Expr::contains_der)
            }
            // The rule is not part of what the expression is worth, so
            // a `der` can only be in the value or in an argument.
            Expr::WithDerivative(value, _, seeds) => {
                value.contains_der() || seeds.iter().any(|(_, arg)| arg.contains_der())
            }
            Expr::Member(base, _) => base.contains_der(),
            Expr::Array(items) => items.iter().any(Expr::contains_der),
            Expr::Elementwise(_, l, r) => l.contains_der() || r.contains_der(),
            Expr::Range(a, step, b) => {
                a.contains_der()
                    || step.as_ref().is_some_and(|s| s.contains_der())
                    || b.contains_der()
            }
            Expr::Comprehension(body, _, range) => body.contains_der() || range.contains_der(),
            Expr::ColonSubscript | Expr::EndSubscript => false,
            Expr::MatrixRows(rows) => rows.iter().any(|row| row.iter().any(Expr::contains_der)),
            Expr::NamedArg(_, value) => value.contains_der(),
            Expr::Tuple(targets) => targets.iter().flatten().any(Expr::contains_der),
            _ => false,
        }
    }

    /// Collect the names of all component references in the expression.
    pub fn collect_refs<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Expr::Ref(name) => out.push(name),
            Expr::Call(_, args) => args.iter().for_each(|a| a.collect_refs(out)),
            Expr::Neg(inner) | Expr::Not(inner) => inner.collect_refs(out),
            Expr::Bin(_, l, r) | Expr::Rel(_, l, r) | Expr::And(l, r) | Expr::Or(l, r) => {
                l.collect_refs(out);
                r.collect_refs(out);
            }
            Expr::If(c, t, e) => {
                c.collect_refs(out);
                t.collect_refs(out);
                e.collect_refs(out);
            }
            Expr::Index(base, subscripts) => {
                base.collect_refs(out);
                subscripts.iter().for_each(|s| s.collect_refs(out));
            }
            // What the call reads is in its value and its arguments;
            // the rule names the same variables and, besides those, only
            // the compiler's own stand-ins for their derivatives.
            Expr::WithDerivative(value, _, seeds) => {
                value.collect_refs(out);
                seeds.iter().for_each(|(_, arg)| arg.collect_refs(out));
            }
            Expr::Member(base, _) => base.collect_refs(out),
            Expr::Array(items) => items.iter().for_each(|item| item.collect_refs(out)),
            Expr::Elementwise(_, l, r) => {
                l.collect_refs(out);
                r.collect_refs(out);
            }
            Expr::Range(a, step, b) => {
                a.collect_refs(out);
                if let Some(step) = step {
                    step.collect_refs(out);
                }
                b.collect_refs(out);
            }
            Expr::Comprehension(body, _, range) => {
                body.collect_refs(out);
                range.collect_refs(out);
            }
            Expr::ColonSubscript | Expr::EndSubscript => {}
            Expr::MatrixRows(rows) => rows
                .iter()
                .for_each(|row| row.iter().for_each(|item| item.collect_refs(out))),
            Expr::NamedArg(_, value) => value.collect_refs(out),
            Expr::Tuple(targets) => targets
                .iter()
                .flatten()
                .for_each(|target| target.collect_refs(out)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(name: &str) -> Expr {
        Expr::Ref(name.into())
    }

    fn der(name: &str) -> Expr {
        Expr::Call("der".into(), vec![r(name)])
    }

    #[test]
    fn as_der_of_only_matches_exact_shape() {
        assert_eq!(der("x").as_der_of(), Some("x"));
        // Two-argument der, der of an expression and non-der do not match.
        assert_eq!(
            Expr::Call("der".into(), vec![r("x"), r("y")]).as_der_of(),
            None
        );
        assert_eq!(
            Expr::Call("der".into(), vec![Expr::Number(1.0)]).as_der_of(),
            None
        );
        assert_eq!(Expr::Call("sin".into(), vec![r("x")]).as_der_of(), None);
        assert_eq!(r("x").as_der_of(), None);
    }

    #[test]
    fn contains_der_walks_every_variant() {
        let deep = Expr::If(
            Box::new(Expr::Rel(
                RelOp::Lt,
                Box::new(r("a")),
                Box::new(Expr::Number(0.0)),
            )),
            Box::new(Expr::And(
                Box::new(Expr::Bool(true)),
                Box::new(Expr::Not(Box::new(r("b")))),
            )),
            Box::new(Expr::Or(
                Box::new(Expr::Neg(Box::new(der("x")))),
                Box::new(Expr::Time),
            )),
        );
        assert!(deep.contains_der());
        assert!(!r("x").contains_der());
        assert!(!Expr::Time.contains_der());

        // The array forms carry it too, wherever it hides in them.
        let hides_in = |expr: Expr| expr.contains_der();
        assert!(hides_in(Expr::Index(
            Box::new(r("v")),
            vec![Expr::Number(1.0), der("i")]
        )));
        assert!(hides_in(Expr::Member(Box::new(der("p")), "x".into())));
        assert!(hides_in(Expr::Array(vec![Expr::Number(0.0), der("x")])));
        assert!(hides_in(Expr::Elementwise(
            BinOp::Mul,
            Box::new(r("a")),
            Box::new(der("x"))
        )));
        assert!(hides_in(Expr::Range(
            Box::new(Expr::Number(1.0)),
            Some(Box::new(der("s"))),
            Box::new(Expr::Number(9.0))
        )));
        assert!(hides_in(Expr::Range(
            Box::new(Expr::Number(1.0)),
            None,
            Box::new(der("n"))
        )));
        assert!(hides_in(Expr::Comprehension(
            Box::new(der("x")),
            "i".into(),
            Box::new(Expr::Number(3.0))
        )));
        assert!(hides_in(Expr::MatrixRows(vec![
            vec![Expr::Number(1.0)],
            vec![der("x")]
        ])));
        assert!(hides_in(Expr::NamedArg("k".into(), Box::new(der("x")))));
        assert!(hides_in(Expr::Tuple(vec![None, Some(der("x"))])));
        // And where it does not hide, they say so.
        assert!(!Expr::ColonSubscript.contains_der());
        assert!(!Expr::EndSubscript.contains_der());
        assert!(!Expr::Array(vec![Expr::Number(1.0)]).contains_der());
        assert!(!Expr::Tuple(vec![None]).contains_der());
    }

    #[test]
    fn collect_refs_walks_every_variant() {
        let deep = Expr::If(
            Box::new(Expr::Rel(
                RelOp::Ge,
                Box::new(r("a")),
                Box::new(Expr::Number(1.0)),
            )),
            Box::new(Expr::Bin(
                BinOp::Add,
                Box::new(Expr::Not(Box::new(r("b")))),
                Box::new(Expr::Call("sin".into(), vec![r("c")])),
            )),
            Box::new(Expr::Neg(Box::new(Expr::Bool(false)))),
        );
        let mut refs = Vec::new();
        deep.collect_refs(&mut refs);
        assert_eq!(refs, vec!["a", "b", "c"]);

        // Every array form hands its pieces over in turn.
        let named = |expr: Expr| {
            let mut refs = Vec::new();
            expr.collect_refs(&mut refs);
            refs.join(",")
        };
        assert_eq!(
            named(Expr::Index(
                Box::new(r("v")),
                vec![r("i"), Expr::ColonSubscript]
            )),
            "v,i"
        );
        assert_eq!(named(Expr::Member(Box::new(r("p")), "x".into())), "p");
        assert_eq!(named(Expr::Array(vec![r("a"), r("b")])), "a,b");
        assert_eq!(
            named(Expr::Elementwise(
                BinOp::Div,
                Box::new(r("a")),
                Box::new(r("b"))
            )),
            "a,b"
        );
        assert_eq!(
            named(Expr::Range(
                Box::new(r("lo")),
                Some(Box::new(r("step"))),
                Box::new(r("hi"))
            )),
            "lo,step,hi"
        );
        assert_eq!(
            named(Expr::Range(Box::new(r("lo")), None, Box::new(r("hi")))),
            "lo,hi"
        );
        assert_eq!(
            named(Expr::Comprehension(
                Box::new(r("body")),
                "i".into(),
                Box::new(r("range"))
            )),
            "body,range"
        );
        assert_eq!(
            named(Expr::MatrixRows(vec![vec![r("a"), r("b")], vec![r("c")]])),
            "a,b,c"
        );
        assert_eq!(named(Expr::NamedArg("k".into(), Box::new(r("v")))), "v");
        assert_eq!(
            named(Expr::Tuple(vec![Some(r("p")), None, Some(r("q"))])),
            "p,q"
        );
        assert_eq!(named(Expr::EndSubscript), "");
    }
}
