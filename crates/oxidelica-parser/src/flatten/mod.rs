//! Flattening: instantiate a hierarchical class tree into a flat
//! [`Model`] of `Real` components and equations.
//!
//! - Components of user types (models/connectors) expand recursively;
//!   flat names are dotted paths (`r1.p.v`).
//! - Modifiers (`Resistor r(R = 100)`) override the binding of the
//!   named child component; modifier expressions are evaluated in the
//!   parent scope.
//! - `extends Base(...)` merges the base class into the current one.
//! - `connect(a, b)` joins connector instances into connection sets:
//!   potential variables become equalities, `flow` variables sum to
//!   zero; unconnected flow variables are forced to zero.
//! - `redeclare` replaces the type of a `replaceable` declaration
//!   further down, checked against its `constrainedby` interface.
//! - `outer` declarations create no variables: their references point at
//!   the nearest enclosing `inner` instance of the same name.
//! - A component with a false `if` condition is left out, and so are the
//!   `connect` statements that mention it.

use crate::ast::*;
use std::collections::HashMap;

/// Maximum instantiation depth (guards against recursive classes).
mod algorithms;
mod arrays;
mod clocks;
mod connections;
mod instantiate;
mod names;
mod operators;
mod strings;
#[cfg(test)]
mod tests;

pub(crate) use names::const_eval;

use algorithms::*;
use arrays::*;
use clocks::*;
use connections::*;
use instantiate::*;
use names::*;
use operators::*;
use strings::*;

/// Maximum instantiation depth (guards against recursive classes).
const MAX_DEPTH: usize = 32;

/// What tooling needs to know about a class to draw it: its connector
/// ports and its parameters, inherited members included.
#[derive(Debug, Clone, Default)]
pub struct ClassInfo {
    /// Names of the connector components, in declaration order.
    pub ports: Vec<String>,
    /// Parameters with their default expressions, where declared.
    pub parameters: Vec<(String, Option<Expr>)>,
    /// The class description string.
    pub description: Option<String>,
    /// Whether the class can be instantiated as a component.
    pub instantiable: bool,
    /// What the class annotation said, as written: the drawing of its
    /// `Icon`, its `Documentation`, whatever else a tool reads.
    pub annotations: Vec<Expr>,
}

/// Summarize a class for tooling (the diagram editor).
pub fn class_info(classes: &[ClassDef], name: &str) -> Option<ClassInfo> {
    let registry: HashMap<&str, &ClassDef> = classes.iter().map(|c| (c.name.as_str(), c)).collect();
    let class = registry.get(name)?;
    let mut info = ClassInfo {
        description: class.description.clone(),
        annotations: class.annotations.clone(),
        instantiable: !class.partial && matches!(class.kind, ClassKind::Model | ClassKind::Record),
        ..ClassInfo::default()
    };
    collect_members(&registry, class, &mut info, 0);
    Some(info)
}

/// Walk a class and its bases, gathering ports and parameters.
fn collect_members(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    info: &mut ClassInfo,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let scope = class.name.as_str();
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, scope, &class.imports) {
            collect_members(registry, base, info, depth + 1);
        }
    }
    for component in &class.components {
        // `outer` declarations are references to an instance owned
        // elsewhere: they are neither ports nor parameters of this class.
        if component.scope == Scope::Outer {
            continue;
        }
        match component.variability {
            Variability::Parameter | Variability::Constant => info
                .parameters
                .push((component.name.clone(), component.binding.clone())),
            Variability::Continuous | Variability::Discrete => {
                let is_connector = lookup(registry, &component.type_name, scope, &class.imports)
                    .is_some_and(|c| c.kind == ClassKind::Connector);
                if is_connector {
                    info.ports.push(component.name.clone());
                }
            }
        }
    }
}

/// What an annotation said under a given name, where it said anything:
/// the string it carries, or an empty one where it is a bare word or
/// carries something else.
fn annotation_says(said: &[Expr], wanted: &str) -> Option<String> {
    said.iter().find_map(|entry| match entry {
        Expr::NamedArg(name, value) if name == wanted => Some(match value.as_ref() {
            Expr::Str(text) => text.clone(),
            _ => String::new(),
        }),
        Expr::Ref(name) if name == wanted => Some(String::new()),
        _ => None,
    })
}

/// Flatten the class named `top` into a flat model.
pub fn flatten(classes: &[ClassDef], top: &str) -> Result<Model, String> {
    let registry: HashMap<&str, &ClassDef> = classes.iter().map(|c| (c.name.as_str(), c)).collect();
    let top_class = registry
        .get(top)
        .ok_or_else(|| format!("unknown class `{top}`"))?;

    // A package holds classes and constants and nothing else: the
    // specification forbids a parameter or a variable in one, since a
    // package has no instance for a value to belong to.
    for class in classes.iter().filter(|c| c.kind == ClassKind::Package) {
        if let Some(loose) = class
            .components
            .iter()
            .find(|c| !matches!(c.variability, Variability::Constant))
        {
            return Err(format!(
                "`{}.{}` is a {} in a package; a package may hold only classes and constants",
                class.name,
                loose.name,
                match loose.variability {
                    Variability::Parameter => "parameter",
                    Variability::Discrete => "discrete variable",
                    _ => "variable",
                }
            ));
        }
    }

    let mut acc = Flat::default();
    let env = Env {
        overrides: &[],
        redeclares: &[],
        inners: &HashMap::new(),
        broken: &[],
    };
    instantiate(&registry, top_class, "", &env, &mut acc, 0)?;

    // An expandable connector holds whatever the connections to it
    // name, so its members exist only once every `connect` is in.
    expand_buses(&registry, &mut acc)?;

    // Connection sets via union-find over connector instance paths.
    let paths: Vec<String> = acc.connectors.keys().cloned().collect();
    let index: HashMap<&str, usize> = paths.iter().map(|p| p.as_str()).zip(0..).collect();
    let mut parent: Vec<usize> = (0..paths.len()).collect();
    fn find(parent: &mut Vec<usize>, i: usize) -> usize {
        if parent[i] != i {
            let root = find(parent, parent[i]);
            parent[i] = root;
        }
        parent[i]
    }
    for (a, b) in &acc.connects {
        let (&ia, &ib) = match (index.get(a.as_str()), index.get(b.as_str())) {
            (Some(ia), Some(ib)) => (ia, ib),
            _ => {
                return Err(format!(
                    "connect({a}, {b}): both sides must be connector instances"
                ))
            }
        };
        let (ra, rb) = (find(&mut parent, ia), find(&mut parent, ib));
        if ra != rb {
            parent[ra] = rb;
        }
    }
    let mut sets: HashMap<usize, Vec<&str>> = HashMap::new();
    for (i, path) in paths.iter().enumerate() {
        sets.entry(find(&mut parent, i)).or_default().push(path);
    }

    for members in sets.values_mut() {
        members.sort();
        // Connectors in one set must match in shape, not in name: a
        // signal output and a signal input are different classes with
        // the same members, and connecting them is the whole point.
        let class_name = acc.connectors[members[0]].clone();
        let class = registry[class_name.as_str()];
        let shape = |class: &ClassDef| -> Vec<(String, bool, bool)> {
            let mut members: Vec<(String, bool, bool)> = class
                .components
                .iter()
                .map(|c| (c.name.clone(), c.flow, c.stream))
                .collect();
            members.sort();
            members
        };
        let wanted = shape(class);
        for member in members.iter() {
            let other = registry[acc.connectors[*member].as_str()];
            if shape(other) != wanted {
                return Err(format!(
                    "connection set {members:?} joins `{class_name}` to `{}`, \
                     which have different members",
                    other.name
                ));
            }
        }
        // A stream variable rides on the one flow variable of its
        // connector; without exactly one, `inStream` has no weights.
        if class.components.iter().any(|c| c.stream) {
            let flows = class.components.iter().filter(|c| c.flow).count();
            if flows != 1 {
                return Err(format!(
                    "connector `{class_name}` carries stream variables, so it needs \
                     exactly one flow variable, found {flows}"
                ));
            }
        }
        for member_component in &class.components {
            let var = |path: &str| format!("{path}.{}", member_component.name);
            if member_component.stream {
                // A stream variable gets no equation from the
                // connection: each side's outflow is set by its own
                // component, and `inStream` reads the others' below.
                continue;
            }
            if member_component.flow {
                if members.len() == 1 {
                    // Unconnected connector: flow forced to zero.
                    acc.equations.push(EquationItem {
                        lhs: Expr::Ref(var(members[0])),
                        rhs: Expr::Number(0.0),
                        origin: String::new(),
                    });
                } else {
                    // Kirchhoff sum over the set.
                    let sum = members
                        .iter()
                        .map(|m| Expr::Ref(var(m)))
                        .reduce(|a, b| Expr::Bin(BinOp::Add, Box::new(a), Box::new(b)))
                        .expect("non-empty set");
                    acc.equations.push(EquationItem {
                        lhs: sum,
                        rhs: Expr::Number(0.0),
                        origin: String::new(),
                    });
                }
            } else if members.len() > 1 {
                // Potential equalities against the first member.
                for other in &members[1..] {
                    acc.equations.push(EquationItem {
                        lhs: Expr::Ref(var(other)),
                        rhs: Expr::Ref(var(members[0])),
                        origin: String::new(),
                    });
                }
            }
        }
    }

    // `inStream` and `actualStream` are functions of the connection
    // set, so only now, with the sets known, do they have a value.
    let any_streams = acc.connectors.values().any(|class_name| {
        registry[class_name.as_str()]
            .components
            .iter()
            .any(|c| c.stream)
    });
    if any_streams {
        let mut node_of: HashMap<String, Vec<String>> = HashMap::new();
        for members in sets.values() {
            for member in members.iter() {
                node_of.insert(
                    (*member).to_string(),
                    members.iter().map(|m| (*m).to_string()).collect(),
                );
            }
        }
        let context = StreamContext {
            nodes: node_of,
            connectors: &acc.connectors,
            outside: &acc.outside,
            registry: &registry,
        };
        for equation in &mut acc.equations {
            equation.lhs = resolve_streams(&equation.lhs, &context)?;
            equation.rhs = resolve_streams(&equation.rhs, &context)?;
        }
        for equation in &mut acc.initial_equations {
            equation.lhs = resolve_streams(&equation.lhs, &context)?;
            equation.rhs = resolve_streams(&equation.rhs, &context)?;
        }
        for clause in &mut acc.when_clauses {
            for branch in &mut clause.branches {
                branch.condition = resolve_streams(&branch.condition, &context)?;
                for action in &mut branch.actions {
                    match action {
                        WhenAction::Assign(_, value) | WhenAction::Reinit(_, value) => {
                            *value = resolve_streams(value, &context)?;
                        }
                        WhenAction::Terminate(_) => {}
                    }
                }
            }
        }
        for (condition, _) in &mut acc.asserts {
            *condition = resolve_streams(condition, &context)?;
        }
        for conditional in &mut acc.conditional {
            for condition in &mut conditional.conditions {
                *condition = resolve_streams(condition, &context)?;
            }
            for branch in &mut conditional.branches {
                for equation in branch {
                    equation.lhs = resolve_streams(&equation.lhs, &context)?;
                    equation.rhs = resolve_streams(&equation.rhs, &context)?;
                }
            }
        }
    }

    // The branches of a run-time `if` are checked one equation at a
    // time, the way they were written: merged into residuals they
    // would look like a volt equated to an ampere, which is exactly
    // what an ideal switch is and not a mistake.
    let mut model = Model {
        transports: acc.transports.clone(),
        name: top_class.name.clone(),
        description: top_class.description.clone(),
        components: acc.components,
        equations: acc.equations,
        initial_equations: acc.initial_equations,
        asserts: acc.asserts,
        transitions: acc.transitions,
        initial_states: acc.initial_states,
        connection_graph: acc.connection_graph.clone(),
        when_clauses: acc.when_clauses,
        conditional: Vec::new(),
        experiment: top_class.experiment.clone(),
        functions: Vec::new(),
    };
    for conditional in &acc.conditional {
        for branch in &conditional.branches {
            model.equations.extend(branch.iter().cloned());
        }
    }
    // An overconstrained graph is broken open before anything else
    // looks at it, and `Connections.isRoot` is answered from what that
    // came to. `cardinality` is answered from the same place: how many
    // `connect` equations named a port. Both are questions about the
    // connections, and this is the last moment the answers are known.
    let roots = choose_roots(&acc.connection_graph, &acc.connects)?;
    let mut connected: HashMap<String, f64> = HashMap::new();
    for (a, b) in &acc.connects {
        for port in [a, b] {
            *connected.entry(port.clone()).or_insert(0.0) += 1.0;
        }
    }
    // What a connector's declaration asked of the connections to it.
    // The chapter says these make it an error rather than leaving it to
    // the tool, so they are checked here, where how often each port was
    // named is already known.
    for (port, said) in &acc.connect_rules {
        let times = connected.get(port).copied().unwrap_or(0.0);
        if let Some(why) = annotation_says(said, "mustBeConnected") {
            if times == 0.0 {
                return Err(match why.is_empty() {
                    true => format!("`{port}` must be connected, and nothing connects to it"),
                    false => format!("`{port}` must be connected: {why}"),
                });
            }
        }
        if let Some(why) = annotation_says(said, "mayOnlyConnectOnce") {
            if times > 1.0 {
                return Err(match why.is_empty() {
                    true => format!(
                        "`{port}` may only be connected once, and {times} \
                                     connections name it"
                    ),
                    false => format!("`{port}` may only be connected once: {why}"),
                });
            }
        }
    }

    // `Evaluate = true` says the parameter has to be one the compiler
    // settles rather than one the run carries. Where it cannot be
    // settled the declaration is asking for something that did not
    // happen, and saying so beats letting it pass as though it had.
    for component in &model.components {
        if !matches!(component.variability, Variability::Parameter)
            || annotation_says(&component.annotations, "Evaluate").is_none()
        {
            continue;
        }
        let asked = component.annotations.iter().any(|entry| {
            matches!(entry, Expr::NamedArg(name, value)
                if name == "Evaluate" && matches!(value.as_ref(), Expr::Bool(true)))
        });
        if asked && !acc.const_values.contains_key(&component.name) {
            return Err(format!(
                "`{}` asks to be evaluated before the run, and its value is not one the \
                 compiler can work out",
                component.name
            ));
        }
    }

    let answer = |expr: &Expr| answer_graph_queries(expr, &roots, &connected);
    for equation in model
        .equations
        .iter_mut()
        .chain(model.initial_equations.iter_mut())
    {
        equation.lhs = answer(&equation.lhs);
        equation.rhs = answer(&equation.rhs);
    }
    // An unconnected port is what `cardinality` is usually asked about,
    // and what it is usually asked about it in is an assertion.
    for (condition, _) in &mut model.asserts {
        *condition = answer(condition);
    }
    for clause in &mut model.when_clauses {
        for branch in &mut clause.branches {
            branch.condition = answer(&branch.condition);
            for action in &mut branch.actions {
                match action {
                    WhenAction::Assign(_, value) | WhenAction::Reinit(_, value) => {
                        *value = answer(value);
                    }
                    WhenAction::Terminate(_) => {}
                }
            }
        }
    }

    // Clocked equations are lifted out before anything is checked:
    // what they leave behind is a `when` clause per clock, which the
    // rest of the pipeline already understands.
    partition_clocks(&mut model)?;
    crate::check::verify(&model)?;
    for conditional in &acc.conditional {
        for branch in &conditional.branches {
            model
                .equations
                .truncate(model.equations.len() - branch.len());
        }
    }
    // The branches themselves travel to the compiler, which settles
    // which one applies and compiles that mode as its own model.
    model.conditional = acc.conditional;
    // Strings are settled last, once every branch that could hold one
    // is in the model: what they leave behind is a Boolean where one
    // was compared, and nothing where one was declared.
    resolve_strings(&mut model)?;
    // Whatever calls are still standing in the flat model are calls
    // nothing could inline. The bodies behind them travel with the
    // model, so the run can walk them for itself.
    model.functions = programs_used(&model, &registry)?;
    Ok(model)
}

/// The regularisation floor of a stream mix: a port whose flow points
/// out of the node still contributes this much weight, so the mix
/// stays defined when every other flow vanishes.
const STREAM_EPS: f64 = 1e-10;

/// What `inStream` needs to know: who shares the node with a
/// connector, and what its class calls the stream and flow members.
struct StreamContext<'a> {
    /// Connector instance path -> every path on the same node, sorted.
    nodes: HashMap<String, Vec<String>>,
    /// Connector instance path -> connector class name.
    connectors: &'a HashMap<String, String>,
    /// The connectors that were a class's own port: their flow enters
    /// the node where an inside connector's leaves it.
    outside: &'a [String],
    /// Class definitions, for the member prefixes.
    registry: &'a HashMap<&'a str, &'a ClassDef>,
}

/// Accumulated flat model contents.
#[derive(Default)]
struct Flat {
    components: Vec<Component>,
    /// The instance whose equations are being gathered, by flat path.
    /// Every equation that goes in is stamped with it, which is what
    /// tells a state's own equations apart later on.
    origin: String,
    /// Connectors that were a class's own port where a `connect` named
    /// them - "outside" connectors, whose flow points into the node
    /// rather than out of it.
    outside: Vec<String>,
    transports: Vec<SpatialTransport>,
    equations: Vec<EquationItem>,
    when_clauses: Vec<WhenClause>,
    /// Equations of the `initial equation` sections, prefixed.
    initial_equations: Vec<EquationItem>,
    /// Assert conditions with their messages, prefixed.
    asserts: Vec<(Expr, String)>,
    /// State machine arrows, with the states named by instance path.
    transitions: Vec<Transition>,
    /// Where each machine starts.
    initial_states: Vec<String>,
    /// Graph clauses with nodes named by instance path.
    connection_graph: Vec<GraphClause>,
    /// Connector instance path -> connector class name.
    connectors: HashMap<String, String>,
    /// What a connector's declaration said about how it must be
    /// connected: `mustBeConnected` and `mayOnlyConnectOnce` of 18.8,
    /// by the port's flat path, with the message each carries.
    connect_rules: Vec<(String, Vec<Expr>)>,
    /// Connect statements with fully prefixed paths.
    connects: Vec<(String, String)>,
    /// Values of parameters already instantiated, by flat name: array
    /// dimensions and loop bounds are resolved against them.
    const_values: HashMap<String, f64>,
    /// Instance paths of components a false condition left out.
    disabled: Vec<String>,
    /// `if` equations whose condition is only known while running.
    conditional: Vec<ConditionalEquations>,
}

impl Flat {
    /// Whether a path names a component left out by its condition, or
    /// anything inside one.
    fn is_disabled(&self, path: &str) -> bool {
        self.disabled
            .iter()
            .any(|left_out| path == left_out || path.starts_with(&format!("{left_out}.")))
    }
}

/// An `inner` instance that `outer` declarations bind to.
#[derive(Clone)]
struct InnerInstance {
    /// Flat path of the instance (`world`).
    path: String,
    /// Qualified name of its class.
    class: String,
}

/// What an instantiation inherits from the level above it.
struct Env<'a> {
    /// Modifier overrides; a dotted name targets a member of a child.
    overrides: &'a [(String, Expr)],
    /// Redeclarations of `replaceable` declarations below.
    redeclares: &'a [Redeclare],
    /// `inner` instances visible here, by declaration name.
    inners: &'a HashMap<String, InnerInstance>,
    /// A selective `extends` leaves these elements of this class out.
    broken: &'a [Deselect],
}

/// One component element about to be instantiated: the declaration, the
/// names it gets and what the level above contributed to it.
struct Site<'a> {
    /// The declaration, with its type already resolved.
    component: &'a Component,
    /// Name inside the enclosing class (`v[2]` for an array element).
    local_name: &'a str,
    /// Full instance path.
    flat_name: &'a str,
    /// Modifiers from a redeclaration, already prefixed.
    extra_modifiers: &'a [(String, Expr)],
    /// This element's own modifiers, already substituted and prefixed.
    /// On an array component each element holds its slice of an
    /// array-valued modifier that was not written `each`.
    modifiers: &'a [(String, Expr)],
    /// Redeclarations aimed at this component's own members.
    redeclares: &'a [Redeclare],
    /// The binding of this element, when the declaration bound the whole
    /// array at once: `parameter Real k[3] = {2, 4, 6}`.
    binding: Option<&'a Expr>,
    /// The start of this element, from an array-valued start attribute.
    start: Option<&'a Expr>,
}

/// The class-level context a component is instantiated in.
struct Level<'a> {
    /// Instance path prefix of the enclosing class.
    prefix: &'a str,
    /// `outer` declaration name -> flat path of the `inner` instance.
    outers: &'a HashMap<String, String>,
    /// `inner` instances the components below can bind to.
    inners: &'a HashMap<String, InnerInstance>,
    /// Modifier overrides the enclosing class received.
    overrides: &'a [(String, Expr)],
    /// Parameter values of the enclosing class, by local name.
    consts: &'a HashMap<String, f64>,
    /// Imports of the enclosing class.
    imports: &'a [(String, String)],
    /// Package scope of the enclosing class.
    scope: &'a str,
}

/// How a statement list finished: fell off the end, hit a `break`, or
/// hit a `return`. Loops consume `Break`; `Return` rides all the way
/// out of the function body.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
    /// Ran to the end.
    Normal,
    /// A `break` is looking for its loop.
    Break,
    /// A `return` is looking for its function.
    Return,
}

/// The most rounds a `while` may take before the compiler assumes it
/// will never finish. Iterative algorithms converge in tens of rounds;
/// this is a backstop, not a budget.
const MAX_WHILE_ROUNDS: usize = 100_000;

/// What an expression is worth once the shapes are known: one scalar,
/// or an array of them.
///
/// Arrays live only at compile time here. A model may say `v` where `v`
/// was declared `Real v[3]`, add two arrays, take their sum - and what
/// reaches the solver is scalars, because the dimensions are constants
/// the compiler can see.
#[derive(Debug, Clone)]
enum Value {
    /// A single expression.
    Scalar(Expr),
    /// An array, given by its elements; nested for more dimensions.
    Array(Vec<Value>),
}

impl Value {
    /// The scalar inside, or an error naming what went wrong.
    fn scalar(self) -> Result<Expr, String> {
        match self {
            Value::Scalar(expr) => Ok(expr),
            Value::Array(_) => Err("an array is used where a scalar is expected".to_string()),
        }
    }

    /// Every scalar of the value, in row-major order.
    fn flatten_into(&self, out: &mut Vec<Expr>) {
        match self {
            Value::Scalar(expr) => out.push(expr.clone()),
            Value::Array(items) => items.iter().for_each(|item| item.flatten_into(out)),
        }
    }

    /// The expression form of the value: a scalar as itself, an array
    /// as a literal - which is how one travels through the bindings of
    /// an algorithm.
    fn into_expr(self) -> Expr {
        match self {
            Value::Scalar(expr) => expr,
            Value::Array(items) => Expr::Array(items.into_iter().map(Value::into_expr).collect()),
        }
    }

    /// Its shape, as the length of each dimension.
    fn shape(&self) -> Vec<usize> {
        match self {
            Value::Scalar(_) => Vec::new(),
            Value::Array(items) => {
                let mut shape = vec![items.len()];
                if let Some(first) = items.first() {
                    shape.extend(first.shape());
                }
                shape
            }
        }
    }
}

/// Everything the array layer needs to know about the class it is
/// working in.
struct Shapes<'a> {
    /// Dimensions of every array component visible here, by name.
    sizes: &'a HashMap<String, Vec<i64>>,
    /// Values of the loop variables in scope.
    loop_vars: &'a HashMap<String, f64>,
    /// Parameter values, for subscripts and lengths.
    consts: &'a HashMap<String, f64>,
    /// Record instances in scope, by name, with the class each one is
    /// of: what tells an overloaded operator which record it is for.
    records: &'a HashMap<String, String>,
}
