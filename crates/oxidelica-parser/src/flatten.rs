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
}

/// Summarize a class for tooling (the diagram editor).
pub fn class_info(classes: &[ClassDef], name: &str) -> Option<ClassInfo> {
    let registry: HashMap<&str, &ClassDef> = classes.iter().map(|c| (c.name.as_str(), c)).collect();
    let class = registry.get(name)?;
    let mut info = ClassInfo {
        description: class.description.clone(),
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

/// Flatten the class named `top` into a flat model.
pub fn flatten(classes: &[ClassDef], top: &str) -> Result<Model, String> {
    let registry: HashMap<&str, &ClassDef> = classes.iter().map(|c| (c.name.as_str(), c)).collect();
    let top_class = registry
        .get(top)
        .ok_or_else(|| format!("unknown class `{top}`"))?;

    let mut acc = Flat::default();
    let env = Env {
        overrides: &[],
        redeclares: &[],
        inners: &HashMap::new(),
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
                    });
                }
            } else if members.len() > 1 {
                // Potential equalities against the first member.
                for other in &members[1..] {
                    acc.equations.push(EquationItem {
                        lhs: Expr::Ref(var(other)),
                        rhs: Expr::Ref(var(members[0])),
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
        name: top_class.name.clone(),
        description: top_class.description.clone(),
        components: acc.components,
        equations: acc.equations,
        initial_equations: acc.initial_equations,
        asserts: acc.asserts,
        when_clauses: acc.when_clauses,
        conditional: Vec::new(),
        experiment: top_class.experiment.clone(),
    };
    for conditional in &acc.conditional {
        for branch in &conditional.branches {
            model.equations.extend(branch.iter().cloned());
        }
    }
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
    Ok(model)
}

/// Record an `if` equation whose condition only the run can decide.
///
/// The spec calls such an `if` balanced: every branch, `else`
/// included, contributes the same number of equations, so the model
/// has one equation per position however the condition falls. What a
/// branch may not do is change the structure - no `connect`, since a
/// connection is drawn once and for all.
fn push_conditional<R, E>(
    if_equation: &IfEquation,
    class_name: &str,
    resolve_here: R,
    expand_here: E,
    no_loop_vars: &HashMap<String, f64>,
    acc: &mut Flat,
) -> Result<(), String>
where
    R: Fn(&Expr) -> Result<Expr, String>,
    E: Fn(&Expr, &HashMap<String, f64>) -> Result<Value, String>,
{
    let mut conditions = Vec::new();
    let mut branches: Vec<Vec<EquationItem>> = Vec::new();
    for (position, branch) in if_equation.branches.iter().enumerate() {
        let last = position + 1 == if_equation.branches.len();
        match (&branch.condition, last) {
            (Some(condition), false) => conditions.push(resolve_here(condition)?),
            (None, true) => {}
            (Some(_), true) => {
                return Err(format!(
                    "an `if` equation in `{class_name}` has a condition the compiler cannot \
                     decide and no `else`, so the model would have a different number of \
                     equations depending on it"
                ))
            }
            (None, false) => unreachable!("an else branch is always last"),
        }
        if !branch.connects.is_empty() {
            return Err(format!(
                "a `connect` in `{class_name}` sits in an `if` branch whose condition is not \
                 known at compile time; connections are structural"
            ));
        }
        let mut scalars = Vec::new();
        for equation in &branch.equations {
            let lhs = expand_here(&equation.lhs, no_loop_vars)?;
            let rhs = expand_here(&equation.rhs, no_loop_vars)?;
            let (mut left, mut right) = (Vec::new(), Vec::new());
            lhs.flatten_into(&mut left);
            rhs.flatten_into(&mut right);
            if left.len() != right.len() {
                return Err(format!(
                    "an equation in `{class_name}` puts {} value(s) against {}",
                    left.len(),
                    right.len()
                ));
            }
            for (lhs, rhs) in left.into_iter().zip(right) {
                scalars.push(EquationItem { lhs, rhs });
            }
        }
        branches.push(scalars);
    }
    let wanted = branches[0].len();
    if let Some(odd) = branches.iter().position(|branch| branch.len() != wanted) {
        return Err(format!(
            "the branches of an `if` equation in `{class_name}` are not balanced: \
             {wanted} equation(s) in the first, {} in branch {}",
            branches[odd].len(),
            odd + 1
        ));
    }
    acc.conditional.push(ConditionalEquations {
        conditions,
        branches,
    });
    Ok(())
}

/// Give every expandable connector the members its connections name.
///
/// A bus declares nothing of its own: `connect(bus.speed, sensor.w)`
/// is what creates `bus.speed`, and it takes the type of the other
/// side. Buses connected to each other share one pool of members, so a
/// sub-bus carries everything its parent does and the matching members
/// are connected in turn.
fn expand_buses(registry: &HashMap<&str, &ClassDef>, acc: &mut Flat) -> Result<(), String> {
    let mut buses: Vec<String> = acc
        .connectors
        .iter()
        .filter(|(_, class_name)| registry[class_name.as_str()].expandable)
        .map(|(path, _)| path.clone())
        .collect();
    if buses.is_empty() {
        return Ok(());
    }
    buses.sort();
    let index: HashMap<&str, usize> = buses.iter().map(|p| p.as_str()).zip(0..).collect();

    // Buses joined directly share their members.
    let mut parent: Vec<usize> = (0..buses.len()).collect();
    fn root(parent: &mut Vec<usize>, i: usize) -> usize {
        if parent[i] != i {
            let found = root(parent, parent[i]);
            parent[i] = found;
        }
        parent[i]
    }
    for (a, b) in &acc.connects {
        if let (Some(&ia), Some(&ib)) = (index.get(a.as_str()), index.get(b.as_str())) {
            let (ra, rb) = (root(&mut parent, ia), root(&mut parent, ib));
            if ra != rb {
                parent[ra] = rb;
            }
        }
    }
    let mut groups: HashMap<usize, Vec<String>> = HashMap::new();
    for (i, bus) in buses.iter().enumerate() {
        let group = root(&mut parent, i);
        groups.entry(group).or_default().push(bus.clone());
    }

    // Every `<bus>.<member>` a connection names, with the other side
    // of that connection: the only place its type can come from.
    let mut pending: Vec<(usize, String, String)> = Vec::new();
    for (a, b) in &acc.connects {
        for (side, other) in [(a, b), (b, a)] {
            if acc.connectors.contains_key(side) {
                continue;
            }
            let Some((head, member)) = side.rsplit_once('.') else {
                continue;
            };
            let Some(&position) = index.get(head) else {
                continue;
            };
            pending.push((
                root(&mut parent, position),
                member.to_string(),
                other.clone(),
            ));
        }
    }

    let env = Env {
        overrides: &[],
        redeclares: &[],
        inners: &HashMap::new(),
    };
    let mut fresh: Vec<(String, String)> = Vec::new();
    loop {
        let mut progress = false;
        let mut waiting = Vec::new();
        for (group, member, other) in pending {
            let paths: Vec<String> = groups[&group]
                .iter()
                .map(|bus| format!("{bus}.{member}"))
                .collect();
            // Another connection may have created it already.
            if acc.connectors.contains_key(&paths[0]) {
                continue;
            }
            // The type comes from the other side, which must itself be
            // a connector by now - it may be a bus member created in an
            // earlier round.
            let Some(class_name) = acc.connectors.get(&other).cloned() else {
                waiting.push((group, member, other));
                continue;
            };
            let class = registry[class_name.as_str()];
            for path in &paths {
                instantiate(registry, class, &format!("{path}."), &env, acc, 0)?;
                acc.connectors.insert(path.clone(), class_name.clone());
            }
            // Matching members of joined buses are connected.
            for path in &paths[1..] {
                fresh.push((paths[0].clone(), path.clone()));
            }
            progress = true;
        }
        pending = waiting;
        if pending.is_empty() || !progress {
            break;
        }
    }
    if let Some((_, member, other)) = pending.first() {
        return Err(format!(
            "connect({other}, ...{member}): a bus member takes its type from the other \
             side of the connection, and `{other}` is not a connector"
        ));
    }
    acc.connects.append(&mut fresh);
    Ok(())
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
    /// Class definitions, for the member prefixes.
    registry: &'a HashMap<&'a str, &'a ClassDef>,
}

/// Replace `inStream(...)` and `actualStream(...)` with the mix the
/// connection set defines for them.
fn resolve_streams(expr: &Expr, context: &StreamContext) -> Result<Expr, String> {
    let recur = |e: &Expr| resolve_streams(e, context);
    Ok(match expr {
        Expr::Call(name, args) if name == "inStream" || name == "actualStream" => {
            let [Expr::Ref(target)] = args.as_slice() else {
                return Err(format!(
                    "`{name}` expects a single reference to a stream variable"
                ));
            };
            stream_mix(target, context, name == "actualStream")?
        }
        Expr::Call(name, args) => Expr::Call(
            name.clone(),
            args.iter().map(recur).collect::<Result<_, _>>()?,
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
        // Everything else is a leaf here, or an array form that never
        // survives to this point.
        _ => expr.clone(),
    })
}

/// The value flowing towards a port: what the other ports of its node
/// push out, weighted by how hard they push. `actual` wraps it into
/// `actualStream`: the incoming mix when the flow enters, the port's
/// own outflow when it leaves.
fn stream_mix(name: &str, context: &StreamContext, actual: bool) -> Result<Expr, String> {
    let Some((path, member)) = name.rsplit_once('.') else {
        return Err(format!(
            "`inStream` expects a connector's stream variable, got `{name}`"
        ));
    };
    let Some(class_name) = context.connectors.get(path) else {
        return Err(format!(
            "`{path}` is not a connector, so `inStream({name})` has no meaning"
        ));
    };
    let class = context.registry[class_name.as_str()];
    let Some(component) = class.components.iter().find(|c| c.name == member) else {
        return Err(format!("connector `{class_name}` has no member `{member}`"));
    };
    if !component.stream {
        return Err(format!(
            "`{name}` is not a stream variable; `inStream` reads only those"
        ));
    }
    let flow_name = class
        .components
        .iter()
        .find(|c| c.flow)
        .map(|c| c.name.clone())
        .expect("checked when the connection sets were built");
    let others: Vec<&String> = context.nodes[path]
        .iter()
        .filter(|other| other.as_str() != path)
        .collect();
    let in_stream = match others.as_slice() {
        // An unconnected port hears its own outflow back.
        [] => Expr::Ref(name.to_string()),
        // Two on a node: each hears exactly the other.
        [other] => Expr::Ref(format!("{other}.{member}")),
        // A junction: the mix of what the others push into the node,
        // weighted by their outbound flows, floored so the division
        // survives every flow going quiet.
        _ => {
            let weight = |other: &str| {
                Expr::Call(
                    "max".to_string(),
                    vec![
                        Expr::Neg(Box::new(Expr::Ref(format!("{other}.{flow_name}")))),
                        Expr::Number(STREAM_EPS),
                    ],
                )
            };
            let sum = |terms: Vec<Expr>| {
                terms
                    .into_iter()
                    .reduce(|a, b| Expr::Bin(BinOp::Add, Box::new(a), Box::new(b)))
                    .expect("at least two others")
            };
            let numerator = sum(others
                .iter()
                .map(|other| {
                    Expr::Bin(
                        BinOp::Mul,
                        Box::new(weight(other)),
                        Box::new(Expr::Ref(format!("{other}.{member}"))),
                    )
                })
                .collect());
            let denominator = sum(others.iter().map(|other| weight(other)).collect());
            Expr::Bin(BinOp::Div, Box::new(numerator), Box::new(denominator))
        }
    };
    Ok(if actual {
        Expr::If(
            Box::new(Expr::Rel(
                crate::ast::RelOp::Gt,
                Box::new(Expr::Ref(format!("{path}.{flow_name}"))),
                Box::new(Expr::Number(0.0)),
            )),
            Box::new(in_stream),
            Box::new(Expr::Ref(name.to_string())),
        )
    } else {
        in_stream
    })
}

/// Accumulated flat model contents.
#[derive(Default)]
struct Flat {
    components: Vec<Component>,
    equations: Vec<EquationItem>,
    when_clauses: Vec<WhenClause>,
    /// Equations of the `initial equation` sections, prefixed.
    initial_equations: Vec<EquationItem>,
    /// Assert conditions with their messages, prefixed.
    asserts: Vec<(Expr, String)>,
    /// Connector instance path -> connector class name.
    connectors: HashMap<String, String>,
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
}

/// Instantiate `class` under `prefix` with everything `env` carries.
fn instantiate(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    env: &Env,
    acc: &mut Flat,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "instantiation deeper than {MAX_DEPTH} levels at `{}` (recursive classes?)",
            class.name
        ));
    }

    let scope = class.name.as_str();

    // `inner` declarations of this class and of its bases own the
    // instances that `outer` declarations inside it refer to. They are
    // collected before anything is instantiated, so a base class may
    // refer to an `inner` of the class extending it and the other way
    // round.
    let mut inners = env.inners.clone();
    collect_inners(registry, class, prefix, &mut inners, 0);
    let outers = bind_outers(registry, class, &inners)?;

    // Redeclarations that reach this class: those written here as
    // `redeclare Type name;`, then the ones handed down.
    let mut redeclares = Vec::new();
    for component in class.components.iter().filter(|c| c.redeclaration) {
        redeclares.push(qualify_redeclare(
            &Redeclare {
                name: component.name.clone(),
                type_name: component.type_name.clone(),
                modifiers: component.modifiers.clone(),
                class_level: false,
            },
            registry,
            class,
            prefix,
            &outers,
        )?);
    }
    redeclares.extend(env.redeclares.iter().cloned());

    // Body-level class redeclarations replace aliases of the bases.
    for alias in class.class_aliases.iter().filter(|a| a.redeclaration) {
        let target = lookup(registry, &alias.target, scope, &class.imports)
            .ok_or_else(|| {
                format!(
                    "unknown class `{}` in the redeclaration of `{}`",
                    alias.target, alias.name
                )
            })?
            .name
            .clone();
        redeclares.push(Redeclare {
            name: alias.name.clone(),
            type_name: target,
            modifiers: Vec::new(),
            class_level: true,
        });
    }

    // The class's own aliases join its imports, with any redeclarations
    // from outside already applied.
    let imports = effective_imports(registry, class, scope, &redeclares)?;

    // Bases first, with their modifiers (already parent-scoped).
    for extend in &class.extends {
        let base = lookup(registry, &extend.base, scope, &imports)
            .ok_or_else(|| format!("unknown base class `{}`", extend.base))?;
        let mods: Vec<(String, Expr)> = extend
            .modifiers
            .iter()
            .map(|(n, e)| {
                let e = substitute_class_constants(e, registry, scope, &imports);
                (n.clone(), prefix_expr(&e, prefix, &outers))
            })
            .chain(env.overrides.iter().cloned())
            .collect();
        let mut base_redeclares = Vec::new();
        for redeclare in &extend.redeclares {
            base_redeclares.push(qualify_redeclare(
                redeclare, registry, class, prefix, &outers,
            )?);
        }
        base_redeclares.extend(redeclares.iter().cloned());
        let base_env = Env {
            overrides: &mods,
            redeclares: &base_redeclares,
            inners: &inners,
        };
        instantiate(registry, base, prefix, &base_env, acc, depth + 1)?;
    }
    let overrides = env.overrides;

    // Parameter values of this class, resolved to numbers where
    // possible: array dimensions and loop bounds are compile-time
    // constants and must come from here.
    let mut local_consts: HashMap<String, f64> = HashMap::new();
    loop {
        let mut progress = false;
        for component in &class.components {
            if !matches!(
                component.variability,
                Variability::Parameter | Variability::Constant
            ) || local_consts.contains_key(&component.name)
            {
                continue;
            }
            let binding = overrides
                .iter()
                .find(|(n, _)| n == &component.name)
                .map(|(_, e)| e.clone())
                .or_else(|| {
                    component
                        .binding
                        .as_ref()
                        .or(component.start.as_ref())
                        .map(|e| {
                            let e = substitute_class_constants(e, registry, scope, &imports);
                            prefix_expr(&e, prefix, &outers)
                        })
                });
            let Some(expr) = binding else { continue };
            let mut env = acc.const_values.clone();
            for (name, value) in &local_consts {
                env.insert(name.clone(), *value);
            }
            if let Some(value) = const_eval(&expr, &env) {
                local_consts.insert(component.name.clone(), value);
                acc.const_values
                    .insert(format!("{prefix}{}", component.name), value);
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }

    // What each array component of this class - and of its bases - is
    // shaped like, so a value may name one as a whole.
    let mut sizes: HashMap<String, Vec<i64>> = HashMap::new();
    collect_shapes(registry, class, &local_consts, &mut sizes, 0);
    let sizes_here = prefixed_sizes(&sizes, prefix);

    for component in &class.components {
        let flat_name = format!("{prefix}{}", component.name);

        // An `outer` declaration owns nothing: its references were bound
        // to the enclosing `inner` instance above. A `redeclare` in the
        // body replaced an inherited declaration instead of adding one.
        if component.scope == Scope::Outer || component.redeclaration {
            continue;
        }

        // `Support support if useSupport;` — a condition that does not
        // hold removes the component, and later the connections to it.
        if let Some(condition) = &component.condition {
            let mut env = acc.const_values.clone();
            env.extend(local_consts.iter().map(|(k, v)| (k.clone(), *v)));
            let value = const_eval(condition, &env).ok_or_else(|| {
                format!("condition of component `{flat_name}` is not a compile-time constant")
            })?;
            if value == 0.0 {
                acc.disabled.push(flat_name.clone());
                continue;
            }
        }

        // Array dimensions expand into scalar elements.
        let mut sizes = Vec::new();
        for dimension in &component.dimensions {
            let value = const_eval(dimension, &local_consts).ok_or_else(|| {
                format!("dimension of `{flat_name}` is not a compile-time constant")
            })?;
            if value.fract() != 0.0 || value < 0.0 {
                return Err(format!(
                    "dimension of `{flat_name}` must be a whole number that is not negative, \
                     got {value}"
                ));
            }
            sizes.push(value as i64);
        }
        // A dimension of zero is legal and means there is nothing
        // there: the declaration contributes no variables at all.
        let element_names: Vec<String> = if sizes.is_empty() {
            vec![component.name.clone()]
        } else {
            index_tuples(&sizes)
                .into_iter()
                .map(|indices| element_name(&component.name, &indices))
                .collect()
        };
        if !sizes.is_empty() && element_names.is_empty() {
            continue;
        }

        let mut component = component.clone();

        // A redeclaration from above replaces the type; its modifiers
        // come first so they win over the original declaration's.
        let mut extra_modifiers = Vec::new();
        let mut child_redeclares = Vec::new();
        if let Some(redeclare) = redeclares.iter().find(|r| r.name == component.name) {
            check_redeclare(registry, class, &component, redeclare)?;
            component.type_name = redeclare.type_name.clone();
            extra_modifiers.extend(redeclare.modifiers.iter().cloned());
        }
        // Redeclarations aimed at a component of this child travel on,
        // with the child's name stripped off the front.
        for redeclare in &redeclares {
            if let Some(rest) = redeclare
                .name
                .strip_prefix(&format!("{}.", component.name))
                .map(str::to_string)
            {
                child_redeclares.push(Redeclare {
                    name: rest,
                    ..redeclare.clone()
                });
            }
        }
        for redeclare in &component.redeclares {
            child_redeclares.push(qualify_redeclare(
                redeclare, registry, class, prefix, &outers,
            )?);
        }

        // A `type` alias stands for a primitive plus attribute defaults,
        // and an enumeration for an `Integer`; substitute before
        // instantiating.
        resolve_type(registry, &mut component, scope, &imports);

        let level = Level {
            prefix,
            outers: &outers,
            inners: &inners,
            overrides,
            consts: &local_consts,
            imports: &imports,
            scope,
        };
        // An array bound - or started - as a whole hands each element
        // its own value.
        let spread = |expr: &Expr, what: &str| -> Result<Vec<Expr>, String> {
            let shapes = Shapes {
                sizes: &sizes_here,
                loop_vars: &HashMap::new(),
                consts: &local_consts,
            };
            let expr = substitute_class_constants(expr, registry, scope, &imports);
            let value = expand(
                &prefix_expr(&expr, prefix, &outers),
                &shapes,
                registry,
                scope,
                &imports,
                0,
            )?;
            let mut items = Vec::new();
            value.flatten_into(&mut items);
            // A scalar start spreads over the whole array.
            if items.len() == 1 && element_names.len() > 1 {
                return Ok(vec![items[0].clone(); element_names.len()]);
            }
            if items.len() != element_names.len() {
                return Err(format!(
                    "`{}` has {} element(s) but its {what} has {}",
                    component.name,
                    element_names.len(),
                    items.len()
                ));
            }
            Ok(items)
        };
        let element_bindings: Option<Vec<Expr>> = match (&component.binding, sizes.is_empty()) {
            (Some(binding), false) => Some(spread(binding, "value")?),
            _ => None,
        };
        let element_starts: Option<Vec<Expr>> = match (&component.start, sizes.is_empty()) {
            (Some(start), false) => Some(spread(start, "start")?),
            _ => None,
        };

        for (position, local_name) in element_names.iter().enumerate() {
            let flat_name = format!("{prefix}{local_name}");
            let site = Site {
                component: &component,
                local_name,
                flat_name: &flat_name,
                extra_modifiers: &extra_modifiers,
                redeclares: &child_redeclares,
                binding: element_bindings.as_ref().map(|items| &items[position]),
                start: element_starts.as_ref().map(|items| &items[position]),
            };
            instantiate_one(registry, &site, &level, acc, depth)?;
        }
    }

    // Equations: arrays expanded, subscripts resolved, calls inlined.
    let resolve_here = |expr: &Expr| -> Result<Expr, String> {
        let expr = substitute_class_constants(expr, registry, scope, &imports);
        resolve(
            &prefix_expr(&expr, prefix, &outers),
            &HashMap::new(),
            &local_consts,
            registry,
            scope,
            &imports,
            0,
        )
    };
    let expand_here = |expr: &Expr, loop_vars: &HashMap<String, f64>| -> Result<Value, String> {
        let expr = substitute_class_constants(expr, registry, scope, &imports);
        let expr = prefix_expr(&expr, prefix, &outers);
        let shapes = Shapes {
            sizes: &sizes_here,
            loop_vars,
            consts: &local_consts,
        };
        expand(&expr, &shapes, registry, scope, &imports, 0)
    };
    let no_loop_vars = HashMap::new();
    for equation in &class.equations {
        // `(a, , c) = f(...)`: one call fills several targets. The
        // call is inlined once per output; a skipped slot costs its
        // computation nothing, since the expression is never used.
        if let Expr::Tuple(targets) = &equation.lhs {
            let rhs = substitute_class_constants(&equation.rhs, registry, scope, &imports);
            let rhs = prefix_expr(&rhs, prefix, &outers);
            let Expr::Call(name, raw_args) = &rhs else {
                return Err("the right side of a tuple equation must be a function call".into());
            };
            let function = lookup(registry, name, scope, &imports)
                .filter(|c| c.kind == ClassKind::Function)
                .ok_or_else(|| format!("`{name}` is not a function, so it cannot fill a tuple"))?;
            let shapes = Shapes {
                sizes: &sizes_here,
                loop_vars: &no_loop_vars,
                consts: &local_consts,
            };
            let values = raw_args
                .iter()
                .map(|arg| expand(arg, &shapes, registry, scope, &imports, 0))
                .collect::<Result<Vec<_>, String>>()?;
            let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
            let arguments: Vec<Expr> = values.into_iter().map(|value| value.into_expr()).collect();
            let outputs = inline_function_outputs(
                function,
                &arguments,
                &argument_shapes,
                &local_consts,
                registry,
                0,
            )?;
            if targets.len() > outputs.len() {
                return Err(format!(
                    "`{name}` has {} output(s) for {} target(s)",
                    outputs.len(),
                    targets.len()
                ));
            }
            for (slot, (_, value)) in targets.iter().zip(outputs) {
                let Some(target) = slot else { continue };
                // The target goes through the usual pipeline; the
                // inlined value is already resolved and only needs the
                // array layer, or a second prefix would corrupt it.
                let lhs = expand_here(target, &no_loop_vars)?;
                let rhs = expand(&value, &shapes, registry, scope, &imports, 0)?;
                push_equations(&lhs, &rhs, acc)?;
            }
            continue;
        }
        let lhs = expand_here(&equation.lhs, &no_loop_vars)?;
        let rhs = expand_here(&equation.rhs, &no_loop_vars)?;
        push_equations(&lhs, &rhs, acc)?;
    }

    for (condition, message) in &class.asserts {
        acc.asserts
            .push((resolve_here(condition)?, message.clone()));
    }

    for equation in &class.initial_equations {
        let (lhs, rhs) = (
            expand_here(&equation.lhs, &no_loop_vars)?,
            expand_here(&equation.rhs, &no_loop_vars)?,
        );
        let (mut left, mut right) = (Vec::new(), Vec::new());
        lhs.flatten_into(&mut left);
        rhs.flatten_into(&mut right);
        if left.len() != right.len() {
            return Err("an initial equation between shapes that do not match".to_string());
        }
        for (lhs, rhs) in left.into_iter().zip(right) {
            acc.initial_equations.push(EquationItem { lhs, rhs });
        }
    }

    // An `algorithm` section of a model is executed symbolically: what
    // comes out is one equation per variable it assigns, which is what
    // the rest of the pipeline understands.
    if class.kind != ClassKind::Function && !class.algorithm.is_empty() {
        let mut bindings: HashMap<String, Expr> = HashMap::new();
        let mut assigned: Vec<String> = Vec::new();
        match execute(
            &class.algorithm,
            &mut bindings,
            &mut assigned,
            &local_consts,
            &sizes,
            registry,
            scope,
            &imports,
            depth,
            false,
        )? {
            Flow::Normal => {}
            Flow::Break => return Err("`break` outside of a loop".to_string()),
            Flow::Return => {
                return Err("`return` belongs in a function, not a model algorithm".to_string())
            }
        }
        for name in assigned {
            let value = bindings
                .get(&name)
                .ok_or_else(|| format!("`{name}` is assigned by the algorithm but has no value"))?;
            // Both sides may be arrays: `w := v .* k` assigns a whole
            // one, and comes out as one equation per element.
            push_equations(
                &expand_here(&Expr::Ref(name.clone()), &no_loop_vars)?,
                &expand_here(value, &no_loop_vars)?,
                acc,
            )?;
        }
    }

    // `for` equations are unrolled: the loop variable is a constant.
    for loop_eq in &class.for_equations {
        unroll(
            loop_eq,
            &HashMap::new(),
            &local_consts,
            prefix,
            &outers,
            &sizes_here,
            registry,
            scope,
            &imports,
            acc,
        )?;
    }

    // `if` equations: the branch that holds contributes its equations,
    // the others contribute nothing. Conditions are structural, so they
    // must be constant at compile time.
    for if_equation in &class.if_equations {
        let mut env = acc.const_values.clone();
        env.extend(local_consts.iter().map(|(k, v)| (k.clone(), *v)));
        // A structural condition picks one branch and the model is
        // built from it. A condition only the run holds decides
        // nothing here, so every branch must contribute the same
        // number of equations and each position becomes one equation
        // that chooses its residual as it goes.
        let decidable = if_equation.branches.iter().all(|branch| {
            branch
                .condition
                .as_ref()
                .is_none_or(|condition| const_eval(condition, &env).is_some())
        });
        if !decidable {
            push_conditional(
                if_equation,
                &class.name,
                resolve_here,
                expand_here,
                &no_loop_vars,
                acc,
            )?;
            continue;
        }
        let mut chosen = None;
        for branch in &if_equation.branches {
            match &branch.condition {
                None => {
                    chosen = Some(branch);
                    break;
                }
                Some(condition) => {
                    let value = const_eval(condition, &env).ok_or_else(|| {
                        format!(
                            "condition of an `if` equation in `{}` is not a compile-time constant",
                            class.name
                        )
                    })?;
                    if value != 0.0 {
                        chosen = Some(branch);
                        break;
                    }
                }
            }
        }
        let Some(branch) = chosen else { continue };
        for equation in &branch.equations {
            push_equations(
                &expand_here(&equation.lhs, &no_loop_vars)?,
                &expand_here(&equation.rhs, &no_loop_vars)?,
                acc,
            )?;
        }
        for (a, b) in &branch.connects {
            let shapes = Shapes {
                sizes: &sizes_here,
                loop_vars: &no_loop_vars,
                consts: &local_consts,
            };
            push_connects(
                a, b, &shapes, prefix, &outers, registry, scope, &imports, acc,
            )?;
        }
    }

    for clause in &class.when_clauses {
        let mut branches = Vec::new();
        for branch in &clause.branches {
            let actions = branch
                .actions
                .iter()
                .map(|action| match action {
                    WhenAction::Reinit(state, value) => Ok(WhenAction::Reinit(
                        flat_name(state, prefix, &outers),
                        resolve_here(value)?,
                    )),
                    WhenAction::Assign(target, value) => Ok(WhenAction::Assign(
                        flat_name(target, prefix, &outers),
                        resolve_here(value)?,
                    )),
                    WhenAction::Terminate(message) => Ok(WhenAction::Terminate(message.clone())),
                })
                .collect::<Result<Vec<_>, String>>()?;
            branches.push(WhenBranch {
                condition: resolve_here(&branch.condition)?,
                actions,
            });
        }
        acc.when_clauses.push(WhenClause { branches });
    }
    // A connection to a component that a condition left out goes with
    // it: this is how the standard library switches a support flange
    // between an external connector and an internal ground.
    for (a, b) in &class.connects {
        let shapes = Shapes {
            sizes: &sizes_here,
            loop_vars: &no_loop_vars,
            consts: &local_consts,
        };
        push_connects(
            a, b, &shapes, prefix, &outers, registry, scope, &imports, acc,
        )?;
    }
    Ok(())
}

/// The imports a class resolves names through, with its class aliases
/// folded in as further entries.
///
/// `package Medium = Media.Water` makes `Medium.density` mean
/// `Media.Water.density` exactly the way `import Medium = Media.Water`
/// would, so an alias becomes an import entry. A redeclaration from the
/// environment swaps the target before that - checked against the
/// alias's `constrainedby` interface, since a replacement medium has to
/// honour the interface the component was written against.
fn effective_imports(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    scope: &str,
    redeclares: &[Redeclare],
) -> Result<Vec<(String, String)>, String> {
    let mut imports = class.imports.clone();
    for alias in &class.class_aliases {
        // A body-level `redeclare package X = ...` replaces an alias of
        // a base class; it is routed through the environment instead.
        if alias.redeclaration {
            continue;
        }
        let replacement = redeclares
            .iter()
            .find(|r| r.class_level && r.name == alias.name);
        let target = match replacement {
            Some(redeclare) => {
                if !alias.replaceable {
                    return Err(format!(
                        "class `{}` of `{}` is redeclared but not declared replaceable",
                        alias.name, class.name
                    ));
                }
                // Qualified already, at the site that wrote it.
                redeclare.type_name.clone()
            }
            None => lookup(registry, &alias.target, scope, &imports)
                .ok_or_else(|| {
                    format!(
                        "unknown class `{}` behind the alias `{}`",
                        alias.target, alias.name
                    )
                })?
                .name
                .clone(),
        };
        if let (Some(constraint), Some(_)) = (&alias.constrained_by, replacement) {
            let constraint = lookup(registry, constraint, scope, &imports).ok_or_else(|| {
                format!(
                    "unknown constraining class `{constraint}` of `{}`",
                    alias.name
                )
            })?;
            if !extends_class(registry, &target, &constraint.name, 0) {
                return Err(format!(
                    "`{target}` cannot replace `{}`: it does not extend `{}`",
                    alias.name, constraint.name
                ));
            }
        }
        imports.push((alias.name.clone(), target));
    }
    Ok(imports)
}

/// Collect the `inner` declarations of a class and of its bases.
fn collect_inners(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    out: &mut HashMap<String, InnerInstance>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let scope = class.name.as_str();
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, scope, &class.imports) {
            collect_inners(registry, base, prefix, out, depth + 1);
        }
    }
    for component in class.components.iter().filter(|c| c.scope == Scope::Inner) {
        if let Some(declared) = lookup(registry, &component.type_name, scope, &class.imports) {
            out.insert(
                component.name.clone(),
                InnerInstance {
                    path: format!("{prefix}{}", component.name),
                    class: declared.name.clone(),
                },
            );
        }
    }
}

/// Bind the `outer` declarations of a class to the visible `inner`
/// instances, yielding the name-to-path map references go through.
fn bind_outers(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    inners: &HashMap<String, InnerInstance>,
) -> Result<HashMap<String, String>, String> {
    let scope = class.name.as_str();
    let mut outers = HashMap::new();
    for component in class.components.iter().filter(|c| c.scope == Scope::Outer) {
        let inner = inners.get(&component.name).ok_or_else(|| {
            format!(
                "`outer {} {}` in `{}` has no `inner` declaration above it",
                component.type_name, component.name, class.name
            )
        })?;
        let declared =
            lookup(registry, &component.type_name, scope, &class.imports).ok_or_else(|| {
                format!(
                    "unknown type `{}` of outer component `{}`",
                    component.type_name, component.name
                )
            })?;
        if !extends_class(registry, &inner.class, &declared.name, 0) {
            return Err(format!(
                "`outer {} {}` does not match the `inner` instance, which is a `{}`",
                component.type_name, component.name, inner.class
            ));
        }
        outers.insert(component.name.clone(), inner.path.clone());
    }
    Ok(outers)
}

/// Whether `candidate` is `target` or extends it, directly or not.
fn extends_class(
    registry: &HashMap<&str, &ClassDef>,
    candidate: &str,
    target: &str,
    depth: usize,
) -> bool {
    if candidate == target {
        return true;
    }
    if depth > MAX_DEPTH {
        return false;
    }
    let Some(class) = registry.get(candidate) else {
        return false;
    };
    let scope = class.name.as_str();
    class.extends.iter().any(|extend| {
        lookup(registry, &extend.base, scope, &class.imports)
            .is_some_and(|base| extends_class(registry, &base.name, target, depth + 1))
    })
}

/// Prepare a redeclaration for use further down: its type is resolved in
/// the scope where the redeclaration is written, and its modifier
/// expressions are prefixed with the instance path they belong to.
fn qualify_redeclare(
    redeclare: &Redeclare,
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    outers: &HashMap<String, String>,
) -> Result<Redeclare, String> {
    let scope = class.name.as_str();
    let target =
        lookup(registry, &redeclare.type_name, scope, &class.imports).ok_or_else(|| {
            format!(
                "unknown type `{}` in the redeclaration of `{}`",
                redeclare.type_name, redeclare.name
            )
        })?;
    Ok(Redeclare {
        name: redeclare.name.clone(),
        type_name: target.name.clone(),
        class_level: redeclare.class_level,
        modifiers: redeclare
            .modifiers
            .iter()
            .map(|(n, e)| {
                let e = substitute_class_constants(e, registry, scope, &class.imports);
                (n.clone(), prefix_expr(&e, prefix, outers))
            })
            .collect(),
    })
}

/// Check that a declaration may be replaced by a redeclaration: it must
/// be `replaceable`, and the new type must meet the `constrainedby`
/// interface where one is given.
fn check_redeclare(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    component: &Component,
    redeclare: &Redeclare,
) -> Result<(), String> {
    if !component.replaceable {
        return Err(format!(
            "`{}` of `{}` is redeclared but not declared replaceable",
            component.name, class.name
        ));
    }
    let Some(constraint) = &component.constrained_by else {
        return Ok(());
    };
    let scope = class.name.as_str();
    let constraint = lookup(registry, constraint, scope, &class.imports).ok_or_else(|| {
        format!(
            "unknown constraining class `{constraint}` of `{}`",
            component.name
        )
    })?;
    if !extends_class(registry, &redeclare.type_name, &constraint.name, 0) {
        return Err(format!(
            "`{}` cannot replace `{}`: it does not extend `{}`",
            redeclare.type_name, component.name, constraint.name
        ));
    }
    Ok(())
}

/// Resolve a declared type down to a primitive: `type` aliases chain,
/// each contributing its attribute defaults, and an enumeration is
/// carried as an `Integer` holding the position of its literal.
fn resolve_type(
    registry: &HashMap<&str, &ClassDef>,
    component: &mut Component,
    scope: &str,
    imports: &[(String, String)],
) {
    // A declaration typed by an alias writes its attributes the
    // modifier way - `Units.AngularVelocity w(start = w0)` parses into
    // modifiers, not into the attribute fields. They mean exactly what
    // the attribute form means, and they belong to the declaration, so
    // they take precedence over anything an alias contributes below.
    if lookup(registry, &component.type_name, scope, imports)
        .is_some_and(|class| class.alias_of.is_some() || !class.enumeration.is_empty())
    {
        component
            .modifiers
            .retain(|(name, value)| match name.as_str() {
                "start" => {
                    if component.start.is_none() {
                        component.start = Some(value.clone());
                    }
                    false
                }
                "fixed" => {
                    if component.fixed.is_none() {
                        component.fixed = Some(matches!(value, Expr::Bool(true)));
                    }
                    false
                }
                _ => true,
            });
    }
    let mut scope = scope.to_string();
    let mut imports = imports.to_vec();
    for _ in 0..MAX_DEPTH {
        if is_primitive(&component.type_name) {
            return;
        }
        let Some(class) = lookup(registry, &component.type_name, &scope, &imports) else {
            return;
        };
        if !class.enumeration.is_empty() {
            component.type_name = "Integer".to_string();
            return;
        }
        let Some((base, attributes)) = class.alias_of.clone() else {
            return;
        };
        component.type_name = base;
        if component.unit.is_none() {
            component.unit = class.alias_unit.clone();
        }
        for (name, value) in attributes {
            match name.as_str() {
                "start" if component.start.is_none() => component.start = Some(value),
                "fixed" if component.fixed.is_none() => {
                    component.fixed = Some(matches!(value, Expr::Bool(true)))
                }
                _ => {}
            }
        }
        // The next alias in the chain resolves where it was written.
        scope = class.name.clone();
        imports = class.imports.clone();
    }
}

/// Unroll a `for` equation, recursing into nested loops. The loop
/// variable is a compile-time constant, so the body is emitted once per
/// value with every subscript already resolved.
#[allow(clippy::too_many_arguments)]
fn unroll(
    loop_eq: &ForEquation,
    outer_vars: &HashMap<String, f64>,
    consts: &HashMap<String, f64>,
    prefix: &str,
    outers: &HashMap<String, String>,
    sizes: &HashMap<String, Vec<i64>>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    acc: &mut Flat,
) -> Result<(), String> {
    // Everything in the loop is prefixed before it is folded, so a
    // parameter of the class the loop is written in - the `n` of `for i
    // in 1:n` or of a guard `if i < n` - has to be findable under its
    // instance path as well as under its plain name.
    let consts: HashMap<String, f64> = consts
        .iter()
        .map(|(name, value)| (name.clone(), *value))
        .chain(
            consts
                .iter()
                .map(|(name, value)| (format!("{prefix}{name}"), *value)),
        )
        .collect();
    let consts = &consts;
    let bound = |expr: &Expr| -> Result<i64, String> {
        let mut env = consts.clone();
        env.extend(outer_vars.iter().map(|(k, v)| (k.clone(), *v)));
        // A bound may ask an array how long it is, so it goes through
        // the same expansion as everything else before being folded.
        let shapes = Shapes {
            sizes,
            loop_vars: outer_vars,
            consts,
        };
        let expr = &expand(
            &prefix_expr(expr, prefix, outers),
            &shapes,
            registry,
            scope,
            imports,
            0,
        )?
        .scalar()?;
        let value = const_eval(expr, &env)
            .ok_or_else(|| format!("loop bound is not a compile-time constant: {expr:?}"))?;
        if value.fract() != 0.0 {
            return Err(format!("loop bound must be a whole number, got {value}"));
        }
        Ok(value as i64)
    };
    let (lower, upper) = (bound(&loop_eq.range.0)?, bound(&loop_eq.range.1)?);
    for index in lower..=upper {
        let mut loop_vars = outer_vars.clone();
        loop_vars.insert(loop_eq.variable.clone(), index as f64);
        // The loop variable is a compile-time number, not a component,
        // and it is folded in before anything is prefixed: prefixing
        // reaches into subscripts too, and `x[i]` inside a component
        // would otherwise be asking for `a.i`.
        let folded: HashMap<String, Expr> = loop_vars
            .iter()
            .map(|(name, value)| (name.clone(), Expr::Number(*value)))
            .collect();
        for item in &loop_eq.body {
            match item {
                ForBody::Equation(equation) => {
                    let shapes = Shapes {
                        sizes,
                        loop_vars: &loop_vars,
                        consts,
                    };
                    let side = |expr: &Expr| -> Result<Value, String> {
                        let expr = substitute_refs(expr, &folded);
                        let expr = substitute_class_constants(&expr, registry, scope, imports);
                        expand(
                            &prefix_expr(&expr, prefix, outers),
                            &shapes,
                            registry,
                            scope,
                            imports,
                            0,
                        )
                    };
                    push_equations(&side(&equation.lhs)?, &side(&equation.rhs)?, acc)?;
                }
                ForBody::Connect(a, b) => {
                    let shapes = Shapes {
                        sizes,
                        loop_vars: &loop_vars,
                        consts,
                    };
                    let (a, b) = (substitute_refs(a, &folded), substitute_refs(b, &folded));
                    push_connects(
                        &a, &b, &shapes, prefix, outers, registry, scope, imports, acc,
                    )?;
                }
                ForBody::Nested(inner) => unroll(
                    inner, &loop_vars, consts, prefix, outers, sizes, registry, scope, imports, acc,
                )?,
            }
        }
    }
    Ok(())
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

/// Instantiate one component element (a scalar, or one element of an
/// array).
fn instantiate_one(
    registry: &HashMap<&str, &ClassDef>,
    site: &Site,
    level: &Level,
    acc: &mut Flat,
    depth: usize,
) -> Result<(), String> {
    let Site {
        component,
        local_name,
        flat_name,
        extra_modifiers,
        redeclares,
        binding: _,
        start: _,
    } = *site;
    let Level {
        prefix,
        outers,
        inners,
        overrides,
        consts: local_consts,
        imports,
        scope,
    } = *level;
    {
        if is_primitive(&component.type_name) {
            let mut flat = component.clone();
            flat.name = flat_name.to_string();
            flat.dimensions = Vec::new();
            let resolve_value = |e: &Expr| -> Result<Expr, String> {
                let e = substitute_class_constants(e, registry, scope, imports);
                resolve(
                    &prefix_expr(&e, prefix, outers),
                    &HashMap::new(),
                    local_consts,
                    registry,
                    scope,
                    imports,
                    0,
                )
            };
            flat.start = match site.start {
                Some(expr) => Some(expr.clone()),
                None => flat.start.as_ref().map(&resolve_value).transpose()?,
            };
            flat.binding = match site.binding {
                // Already expanded from the array the declaration bound.
                Some(expr) => Some(expr.clone()),
                None => flat.binding.as_ref().map(&resolve_value).transpose()?,
            };
            // A parent modifier `name = expr` overrides the binding, and
            // a nested one - `phi(start = 1)` - the attribute.
            let modifier = |target: &str| {
                extra_modifiers
                    .iter()
                    .chain(overrides.iter())
                    .find(|(n, _)| n == target)
                    .map(|(_, e)| e.clone())
            };
            if let Some(value) = modifier(local_name) {
                flat.binding = Some(value);
            }
            if let Some(value) = modifier(&format!("{}.start", component.name)) {
                flat.start = Some(value);
            }
            if let Some(value) = modifier(&format!("{}.fixed", component.name)) {
                flat.fixed = Some(!matches!(value, Expr::Bool(false) | Expr::Number(0.0)));
            }
            // On a variable rather than a parameter, a binding is a
            // declaration equation: `Support support(tau = -flange.tau)`
            // in the standard library ties a connector to its component.
            if flat.variability == Variability::Continuous {
                if let Some(value) = flat.binding.take() {
                    acc.equations.push(EquationItem {
                        lhs: Expr::Ref(flat.name.clone()),
                        rhs: value,
                    });
                }
            }
            acc.components.push(flat);
        } else {
            let child =
                lookup(registry, &component.type_name, scope, imports).ok_or_else(|| {
                    format!(
                        "unknown type `{}` of component `{flat_name}`",
                        component.type_name
                    )
                })?;
            if child.partial {
                return Err(format!(
                    "`{}` is partial and cannot be instantiated as `{flat_name}`",
                    child.name
                ));
            }
            if matches!(child.kind, ClassKind::Package | ClassKind::Function) {
                return Err(format!(
                    "`{}` is a {} and cannot be a component type",
                    child.name,
                    if child.kind == ClassKind::Package {
                        "package"
                    } else {
                        "function"
                    }
                ));
            }
            if child.kind == ClassKind::Connector {
                acc.connectors
                    .insert(flat_name.to_string(), child.name.clone());
            }
            // Child modifiers, outermost first so they win: dotted
            // overrides handed down, then a redeclaration's, then the
            // ones written on this declaration.
            let inherited = overrides.iter().filter_map(|(name, value)| {
                name.strip_prefix(&format!("{local_name}."))
                    .map(|rest| (rest.to_string(), value.clone()))
            });
            let mods: Vec<(String, Expr)> = inherited
                .chain(extra_modifiers.iter().cloned())
                .chain(component.modifiers.iter().map(|(n, e)| {
                    let e = substitute_class_constants(e, registry, scope, imports);
                    (n.clone(), prefix_expr(&e, prefix, outers))
                }))
                .collect();
            let child_prefix = format!("{flat_name}.");
            let child_env = Env {
                overrides: &mods,
                redeclares,
                inners,
            };
            instantiate(registry, child, &child_prefix, &child_env, acc, depth + 1)?;
        }
    }
    Ok(())
}

/// Flat name of a reference written inside a class: an `outer`
/// declaration points at the enclosing `inner` instance, everything else
/// gets the instance prefix.
fn flat_name(name: &str, prefix: &str, outers: &HashMap<String, String>) -> String {
    let (head, rest) = match name.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (name, None),
    };
    if let Some(path) = outers.get(head) {
        return match rest {
            Some(rest) => format!("{path}.{rest}"),
            None => path.clone(),
        };
    }
    format!("{prefix}{name}")
}

/// Prefix every component reference in an expression, resolving `outer`
/// references to the instance that owns them.
fn prefix_expr(expr: &Expr, prefix: &str, outers: &HashMap<String, String>) -> Expr {
    if prefix.is_empty() && outers.is_empty() {
        return expr.clone();
    }
    let recur = |e: &Expr| prefix_expr(e, prefix, outers);
    match expr {
        Expr::Ref(name) => Expr::Ref(flat_name(name, prefix, outers)),
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::If(c, a, b) => Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b))),
        Expr::Index(base, subscripts) => Expr::Index(
            Box::new(recur(base)),
            subscripts.iter().map(recur).collect(),
        ),
        Expr::Member(base, path) => Expr::Member(Box::new(recur(base)), path.clone()),
        Expr::Array(items) => Expr::Array(items.iter().map(recur).collect()),
        Expr::Elementwise(op, l, r) => {
            Expr::Elementwise(*op, Box::new(recur(l)), Box::new(recur(r)))
        }
        Expr::Range(a, step, b) => Expr::Range(
            Box::new(recur(a)),
            step.as_ref().map(|s| Box::new(recur(s))),
            Box::new(recur(b)),
        ),
        // The iterator variable is not a component; a shadowing rename
        // is not needed because expansion resolves it before prefixing
        // could see it again.
        Expr::Comprehension(body, var, range) => {
            Expr::Comprehension(Box::new(recur(body)), var.clone(), Box::new(recur(range)))
        }
        Expr::MatrixRows(rows) => Expr::MatrixRows(
            rows.iter()
                .map(|row| row.iter().map(recur).collect())
                .collect(),
        ),
        Expr::ColonSubscript | Expr::EndSubscript => expr.clone(),
        Expr::Number(_) | Expr::Bool(_) | Expr::Time => expr.clone(),
        // The keyword names an input of the function, not a component.
        Expr::NamedArg(keyword, value) => Expr::NamedArg(keyword.clone(), Box::new(recur(value))),
        Expr::Tuple(targets) => Expr::Tuple(
            targets
                .iter()
                .map(|slot| slot.as_ref().map(recur))
                .collect(),
        ),
    }
}

/// Evaluate a compile-time constant expression (array dimensions, loop
/// bounds, subscripts). Only the arithmetic that can appear there is
/// supported; anything else means the value is not constant.
fn const_eval(expr: &Expr, env: &HashMap<String, f64>) -> Option<f64> {
    use crate::ast::BinOp::*;
    use crate::ast::RelOp;
    // Truth is carried as 1.0 and 0.0, the way the flat model carries
    // every Boolean.
    let truth = |yes: bool| if yes { 1.0 } else { 0.0 };
    Some(match expr {
        Expr::Number(n) => *n,
        Expr::Bool(b) => truth(*b),
        Expr::Ref(name) => *env.get(name)?,
        Expr::Neg(inner) => -const_eval(inner, env)?,
        Expr::Not(inner) => truth(const_eval(inner, env)? == 0.0),
        Expr::And(l, r) => truth(const_eval(l, env)? != 0.0 && const_eval(r, env)? != 0.0),
        Expr::Or(l, r) => truth(const_eval(l, env)? != 0.0 || const_eval(r, env)? != 0.0),
        Expr::Rel(op, l, r) => {
            let (a, b) = (const_eval(l, env)?, const_eval(r, env)?);
            truth(match op {
                RelOp::Lt => a < b,
                RelOp::Le => a <= b,
                RelOp::Gt => a > b,
                RelOp::Ge => a >= b,
                RelOp::Eq => a == b,
                RelOp::Ne => a != b,
            })
        }
        Expr::If(c, a, b) => {
            if const_eval(c, env)? != 0.0 {
                const_eval(a, env)?
            } else {
                const_eval(b, env)?
            }
        }
        Expr::Bin(op, l, r) => {
            let (a, b) = (const_eval(l, env)?, const_eval(r, env)?);
            match op {
                Add => a + b,
                Sub => a - b,
                Mul => a * b,
                Div => a / b,
                Pow => a.powf(b),
            }
        }
        // The numeric builtins fold too: a `while` iterating Newton's
        // method or an AGM lives on `abs` and `sqrt` in its condition.
        Expr::Call(name, args) => {
            let one = || -> Option<f64> { const_eval(args.first()?, env) };
            let two = || -> Option<(f64, f64)> {
                Some((
                    const_eval(args.first()?, env)?,
                    const_eval(args.get(1)?, env)?,
                ))
            };
            match name.as_str() {
                "abs" => one()?.abs(),
                "sqrt" => one()?.sqrt(),
                "exp" => one()?.exp(),
                "log" => one()?.ln(),
                "log10" => one()?.log10(),
                "sin" => one()?.sin(),
                "cos" => one()?.cos(),
                "tan" => one()?.tan(),
                "asin" => one()?.asin(),
                "acos" => one()?.acos(),
                "atan" => one()?.atan(),
                "sinh" => one()?.sinh(),
                "cosh" => one()?.cosh(),
                "tanh" => one()?.tanh(),
                "floor" => one()?.floor(),
                "ceil" => one()?.ceil(),
                "integer" => one()?.floor(),
                "atan2" => {
                    let (a, b) = two()?;
                    a.atan2(b)
                }
                "min" => {
                    let (a, b) = two()?;
                    a.min(b)
                }
                "max" => {
                    let (a, b) = two()?;
                    a.max(b)
                }
                "div" => {
                    let (a, b) = two()?;
                    (a / b).trunc()
                }
                "mod" => {
                    let (a, b) = two()?;
                    a - (a / b).floor() * b
                }
                "rem" => {
                    let (a, b) = two()?;
                    a - (a / b).trunc() * b
                }
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// Replace references according to a substitution map (function
/// inlining and loop-variable substitution).
fn substitute_refs(expr: &Expr, map: &HashMap<String, Expr>) -> Expr {
    match expr {
        Expr::Ref(name) => map.get(name).cloned().unwrap_or_else(|| expr.clone()),
        Expr::Number(_) | Expr::Bool(_) | Expr::Time => expr.clone(),
        Expr::Call(name, args) => Expr::Call(
            name.clone(),
            args.iter().map(|a| substitute_refs(a, map)).collect(),
        ),
        Expr::Neg(inner) => Expr::Neg(Box::new(substitute_refs(inner, map))),
        Expr::Not(inner) => Expr::Not(Box::new(substitute_refs(inner, map))),
        Expr::Bin(op, l, r) => Expr::Bin(
            *op,
            Box::new(substitute_refs(l, map)),
            Box::new(substitute_refs(r, map)),
        ),
        Expr::Rel(op, l, r) => Expr::Rel(
            *op,
            Box::new(substitute_refs(l, map)),
            Box::new(substitute_refs(r, map)),
        ),
        Expr::And(l, r) => Expr::And(
            Box::new(substitute_refs(l, map)),
            Box::new(substitute_refs(r, map)),
        ),
        Expr::Or(l, r) => Expr::Or(
            Box::new(substitute_refs(l, map)),
            Box::new(substitute_refs(r, map)),
        ),
        Expr::If(c, a, b) => Expr::If(
            Box::new(substitute_refs(c, map)),
            Box::new(substitute_refs(a, map)),
            Box::new(substitute_refs(b, map)),
        ),
        Expr::Index(base, subscripts) => Expr::Index(
            Box::new(substitute_refs(base, map)),
            subscripts.iter().map(|s| substitute_refs(s, map)).collect(),
        ),
        Expr::Member(base, path) => {
            Expr::Member(Box::new(substitute_refs(base, map)), path.clone())
        }
        Expr::Array(items) => Expr::Array(
            items
                .iter()
                .map(|item| substitute_refs(item, map))
                .collect(),
        ),
        Expr::Elementwise(op, l, r) => Expr::Elementwise(
            *op,
            Box::new(substitute_refs(l, map)),
            Box::new(substitute_refs(r, map)),
        ),
        Expr::Range(a, step, b) => Expr::Range(
            Box::new(substitute_refs(a, map)),
            step.as_ref().map(|s| Box::new(substitute_refs(s, map))),
            Box::new(substitute_refs(b, map)),
        ),
        Expr::Comprehension(body, var, range) => {
            // The iterator shadows any outer binding of the same name.
            let mut inner = map.clone();
            inner.remove(var);
            Expr::Comprehension(
                Box::new(substitute_refs(body, &inner)),
                var.clone(),
                Box::new(substitute_refs(range, map)),
            )
        }
        Expr::MatrixRows(rows) => Expr::MatrixRows(
            rows.iter()
                .map(|row| row.iter().map(|item| substitute_refs(item, map)).collect())
                .collect(),
        ),
        Expr::ColonSubscript | Expr::EndSubscript => expr.clone(),
        Expr::NamedArg(keyword, value) => {
            Expr::NamedArg(keyword.clone(), Box::new(substitute_refs(value, map)))
        }
        Expr::Tuple(targets) => Expr::Tuple(
            targets
                .iter()
                .map(|slot| slot.as_ref().map(|target| substitute_refs(target, map)))
                .collect(),
        ),
    }
}

/// Value of a constant declared inside a class: `Constants.pi`.
///
/// Package constants are compile-time values, so a reference to one is
/// replaced by its number before any prefixing happens - otherwise the
/// dotted name would be mistaken for a component of the instance.
fn class_constant(
    registry: &HashMap<&str, &ClassDef>,
    name: &str,
    scope: &str,
    imports: &[(String, String)],
) -> Option<f64> {
    let (class_path, member) = name.rsplit_once('.')?;
    let class = lookup(registry, class_path, scope, imports)?;
    // An enumeration literal is the position it was declared at.
    if let Some(index) = class.enumeration.iter().position(|l| l == member) {
        return Some(index as f64 + 1.0);
    }
    if !class.components.iter().any(|c| {
        c.name == member
            && matches!(
                c.variability,
                Variability::Constant | Variability::Parameter
            )
    }) {
        return None;
    }
    // Constants of one package may build on each other, so resolve the
    // whole set to a fixpoint before reading the one asked for.
    let mut values: HashMap<String, f64> = HashMap::new();
    loop {
        let mut progress = false;
        for component in &class.components {
            if !matches!(
                component.variability,
                Variability::Constant | Variability::Parameter
            ) || values.contains_key(&component.name)
            {
                continue;
            }
            let binding = component.binding.as_ref().or(component.start.as_ref());
            if let Some(value) = binding.and_then(|expr| const_eval(expr, &values)) {
                values.insert(component.name.clone(), value);
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
    values.get(member).copied()
}

/// Replace every reference to a class constant with its value.
fn substitute_class_constants(
    expr: &Expr,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
) -> Expr {
    let recur = |e: &Expr| substitute_class_constants(e, registry, scope, imports);
    match expr {
        Expr::Ref(name) if name.contains('.') => {
            match class_constant(registry, name, scope, imports) {
                Some(value) => Expr::Number(value),
                None => expr.clone(),
            }
        }
        Expr::Ref(_) | Expr::Number(_) | Expr::Bool(_) | Expr::Time => expr.clone(),
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::If(c, a, b) => Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b))),
        Expr::Index(base, subscripts) => Expr::Index(
            Box::new(recur(base)),
            subscripts.iter().map(recur).collect(),
        ),
        Expr::Member(base, path) => Expr::Member(Box::new(recur(base)), path.clone()),
        Expr::Array(items) => Expr::Array(items.iter().map(recur).collect()),
        Expr::Elementwise(op, l, r) => {
            Expr::Elementwise(*op, Box::new(recur(l)), Box::new(recur(r)))
        }
        Expr::Range(a, step, b) => Expr::Range(
            Box::new(recur(a)),
            step.as_ref().map(|s| Box::new(recur(s))),
            Box::new(recur(b)),
        ),
        Expr::Comprehension(body, var, range) => {
            Expr::Comprehension(Box::new(recur(body)), var.clone(), Box::new(recur(range)))
        }
        Expr::MatrixRows(rows) => Expr::MatrixRows(
            rows.iter()
                .map(|row| row.iter().map(recur).collect())
                .collect(),
        ),
        Expr::ColonSubscript | Expr::EndSubscript => expr.clone(),
        Expr::NamedArg(keyword, value) => Expr::NamedArg(keyword.clone(), Box::new(recur(value))),
        Expr::Tuple(targets) => Expr::Tuple(
            targets
                .iter()
                .map(|slot| slot.as_ref().map(recur))
                .collect(),
        ),
    }
}

/// Resolve a class name the way Modelica scoping does: an import
/// alias first, then the class's own nested classes, then the
/// enclosing packages from the inside out, then the global name.
///
/// `scope` is the qualified name of the class doing the looking - not
/// its parent - so that `connector Pin` declared inside `model Bus` is
/// found by components of `Bus` itself.
fn lookup<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    name: &str,
    scope: &str,
    imports: &[(String, String)],
) -> Option<&'a ClassDef> {
    // `import Basic = Electrical.Analog.Basic;` then `Basic.Resistor`.
    let (head, rest) = match name.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (name, None),
    };
    if let Some((_, target)) = imports.iter().find(|(local, _)| local == head) {
        let qualified = match rest {
            Some(rest) => format!("{target}.{rest}"),
            None => target.clone(),
        };
        if let Some(class) = registry.get(qualified.as_str()) {
            return Some(class);
        }
    }
    // Walk out of the enclosing packages: A.B.C -> A.B -> A -> global.
    let mut prefix = scope.to_string();
    loop {
        let candidate = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        };
        if let Some(class) = registry.get(candidate.as_str()) {
            return Some(class);
        }
        match prefix.rfind('.') {
            Some(cut) => prefix.truncate(cut),
            None if prefix.is_empty() => return None,
            None => prefix.clear(),
        }
    }
}

/// Built-in scalar types. `Integer` and `Boolean` are carried as
/// numbers, like everything else in the flat model.
fn is_primitive(type_name: &str) -> bool {
    matches!(type_name, "Real" | "Integer" | "Boolean")
}

/// Flat scalar name of one array element: `T[2]`, `A[1,3]`.
fn element_name(base: &str, subscripts: &[i64]) -> String {
    let list: Vec<String> = subscripts.iter().map(|i| i.to_string()).collect();
    format!("{base}[{}]", list.join(","))
}

/// Every index tuple of an array with the given dimensions, in row-major
/// order: `[2, 3]` yields (1,1), (1,2), (1,3), (2,1), ...
fn index_tuples(dimensions: &[i64]) -> Vec<Vec<i64>> {
    let mut out = vec![Vec::new()];
    for &size in dimensions {
        let mut next = Vec::new();
        for prefix in &out {
            for i in 1..=size {
                let mut extended = prefix.clone();
                extended.push(i);
                next.push(extended);
            }
        }
        out = next;
    }
    out
}

/// Resolve subscripts and inline function calls, turning `T[i+1]` into
/// the scalar reference `T[3]`.
#[allow(clippy::too_many_arguments)]
fn resolve(
    expr: &Expr,
    loop_vars: &HashMap<String, f64>,
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
) -> Result<Expr, String> {
    if depth > MAX_DEPTH {
        return Err("expression nested deeper than the instantiation limit".to_string());
    }
    let recur = |e: &Expr| resolve(e, loop_vars, consts, registry, scope, imports, depth + 1);
    Ok(match expr {
        Expr::Index(base, subscripts) => {
            let Expr::Ref(name) = base.as_ref() else {
                return Err(format!("only variables can be subscripted, found {base:?}"));
            };
            // Subscripts see both loop variables and parameters: they
            // must be constant at compile time.
            let mut subscript_env = consts.clone();
            subscript_env.extend(loop_vars.iter().map(|(k, v)| (k.clone(), *v)));
            let mut indices = Vec::new();
            for subscript in subscripts {
                let resolved = recur(subscript)?;
                let value = const_eval(&resolved, &subscript_env).ok_or_else(|| {
                    format!("subscript of `{name}` is not constant: {subscript:?}")
                })?;
                if value.fract() != 0.0 || value < 1.0 {
                    return Err(format!(
                        "subscript of `{name}` must be a positive whole number, got {value}"
                    ));
                }
                indices.push(value as i64);
            }
            Expr::Ref(element_name(name, &indices))
        }
        // A loop variable is a compile-time constant, not a model
        // variable: it is folded into the unrolled equations. Parameters
        // stay symbolic so they remain tunable.
        Expr::Ref(name) => match loop_vars.get(name) {
            Some(value) => Expr::Number(*value),
            None => expr.clone(),
        },
        Expr::Call(name, args) => {
            let args: Result<Vec<Expr>, String> = args.iter().map(&recur).collect();
            let args = args?;
            match lookup(registry, name, scope, imports) {
                Some(class) if class.kind == ClassKind::Function => {
                    inline_function(class, &args, &[], consts, registry, depth + 1)?
                }
                // `noEvent(x)` and `smooth(n, x)` are hints about event
                // generation and continuity; the value is the argument.
                _ if name == "noEvent" && args.len() == 1 => args[0].clone(),
                _ if name == "smooth" && args.len() == 2 => args[1].clone(),
                _ if args.iter().any(|a| matches!(a, Expr::NamedArg(_, _))) => {
                    return Err(format!(
                        "`{name}` is not a function, so it cannot take named arguments"
                    ))
                }
                _ => Expr::Call(name.clone(), args),
            }
        }
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
        Expr::Member(base, path) => {
            let Expr::Ref(name) = recur(base)? else {
                return Err(format!("`{path}` must follow a subscripted component"));
            };
            Expr::Ref(format!("{name}.{path}"))
        }
        Expr::Array(_)
        | Expr::Elementwise(_, _, _)
        | Expr::Range(_, _, _)
        | Expr::Comprehension(_, _, _)
        | Expr::MatrixRows(_) => {
            return Err("an array value cannot be used where a scalar is expected".to_string())
        }
        Expr::ColonSubscript | Expr::EndSubscript => {
            return Err("`:` and `end` make sense only inside a subscript".to_string())
        }
        Expr::Number(_) | Expr::Bool(_) | Expr::Time => expr.clone(),
        Expr::NamedArg(keyword, value) => Expr::NamedArg(keyword.clone(), Box::new(recur(value)?)),
        Expr::Tuple(_) => {
            return Err("a tuple may only stand on the left of `=` or `:=`".to_string())
        }
    })
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

/// Whether flow control could fire at this nesting level: a `break` or
/// `return` here or in an `if` here, or a `return` inside a loop here -
/// loops consume their own breaks but a return passes through them.
fn has_flow_control(statements: &[Statement]) -> bool {
    statements.iter().any(|statement| match statement {
        Statement::Break | Statement::Return => true,
        Statement::If(branches) => branches.iter().any(|b| has_flow_control(&b.body)),
        Statement::For(_, _, body) | Statement::While(_, body) => has_return(body),
        _ => false,
    })
}

/// Whether a `return` hides anywhere below, loops included.
fn has_return(statements: &[Statement]) -> bool {
    statements.iter().any(|statement| match statement {
        Statement::Return => true,
        Statement::If(branches) => branches.iter().any(|b| has_return(&b.body)),
        Statement::For(_, _, body) | Statement::While(_, body) => has_return(body),
        _ => false,
    })
}

/// The most rounds a `while` may take before the compiler assumes it
/// will never finish. Iterative algorithms converge in tens of rounds;
/// this is a backstop, not a budget.
const MAX_WHILE_ROUNDS: usize = 100_000;

/// Symbolically execute an algorithm section.
///
/// `bindings` maps every variable the section has written to the
/// expression it now holds; reading a variable substitutes that
/// expression, which is what turns a sequence of assignments into one
/// expression per assigned variable. `assigned` collects the targets in
/// the order they were first written, so the equations a model gets out
/// of the section are in source order.
///
/// An `if` runs both ways: each branch is executed on its own copy of
/// the bindings and the results are merged into one `if` expression per
/// variable, with the value from before the statement as the fallback -
/// unless a branch holds `break` or `return`, in which case the
/// conditions must be decidable and only the taken branch runs.
/// A `for` is unrolled, its variable being a compile-time constant.
/// A `while` runs for real: its condition must be decidable each round,
/// and `fold` collapses the loop's assignments to numbers so the
/// expressions do not grow with the iteration count.
#[allow(clippy::too_many_arguments)]
fn execute(
    statements: &[Statement],
    bindings: &mut HashMap<String, Expr>,
    assigned: &mut Vec<String>,
    consts: &HashMap<String, f64>,
    sizes: &HashMap<String, Vec<i64>>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
    fold: bool,
) -> Result<Flow, String> {
    if depth > MAX_DEPTH {
        return Err("algorithm nested deeper than the instantiation limit".to_string());
    }
    for statement in statements {
        match statement {
            Statement::Assign(target, subscripts, value) => {
                let value = substitute_refs(value, bindings);
                // Through the array layer, so `c := a .* b` binds a whole
                // array and a scalar stays a scalar.
                let no_loop_vars = HashMap::new();
                let shapes = Shapes {
                    sizes,
                    loop_vars: &no_loop_vars,
                    consts,
                };
                let value =
                    expand(&value, &shapes, registry, scope, imports, depth + 1)?.into_expr();
                // Expansion turns `p[i - 1]` into the element's own name,
                // which may itself be bound by an earlier statement - so
                // the bindings are applied once more.
                let value = substitute_refs(&value, bindings);
                // `c[i] := ...` lands on the element's own name.
                let target = if subscripts.is_empty() {
                    target.clone()
                } else {
                    let indices = subscripts
                        .iter()
                        .map(|subscript| {
                            let subscript = substitute_refs(subscript, bindings);
                            const_eval(&subscript, consts)
                                .filter(|v| v.fract() == 0.0 && *v >= 1.0)
                                .map(|v| v as i64)
                                .ok_or_else(|| {
                                    format!(
                                        "the subscript of `{target}` must be a whole number the compiler can see"
                                    )
                                })
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    element_name(target, &indices)
                };
                if !assigned.contains(&target) {
                    assigned.push(target.clone());
                }
                // Inside a `while`, a value that folds to a number is
                // stored as one, or the expressions would double in
                // size with every round.
                let value = match const_eval(&value, consts) {
                    Some(number) if fold => Expr::Number(number),
                    _ => value,
                };
                bindings.insert(target, value);
            }
            Statement::TupleAssign(targets, value) => {
                let value = substitute_refs(value, bindings);
                let Expr::Call(name, raw_args) = &value else {
                    return Err(
                        "the right side of a tuple assignment must be a function call".into(),
                    );
                };
                let function = lookup(registry, name, scope, imports)
                    .filter(|c| c.kind == ClassKind::Function)
                    .ok_or_else(|| {
                        format!("`{name}` is not a function, so it cannot fill a tuple")
                    })?;
                let no_loop_vars = HashMap::new();
                let shapes = Shapes {
                    sizes,
                    loop_vars: &no_loop_vars,
                    consts,
                };
                let values = raw_args
                    .iter()
                    .map(|arg| expand(arg, &shapes, registry, scope, imports, depth + 1))
                    .collect::<Result<Vec<_>, String>>()?;
                let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
                let arguments: Vec<Expr> = values
                    .into_iter()
                    .map(|value| substitute_refs(&value.into_expr(), bindings))
                    .collect();
                let outputs = inline_function_outputs(
                    function,
                    &arguments,
                    &argument_shapes,
                    consts,
                    registry,
                    depth + 1,
                )?;
                if targets.len() > outputs.len() {
                    return Err(format!(
                        "`{name}` has {} output(s) for {} target(s)",
                        outputs.len(),
                        targets.len()
                    ));
                }
                for (slot, (_, output)) in targets.iter().zip(outputs) {
                    let Some(target) = slot else { continue };
                    if !assigned.contains(target) {
                        assigned.push(target.clone());
                    }
                    bindings.insert(target.clone(), output);
                }
            }
            Statement::If(branches) => {
                // A branch that may `break` or `return` cannot be
                // merged symbolically - whether it fires must be known.
                // The conditions are decided and only the taken branch
                // runs, its flow passed on.
                if branches.iter().any(|b| has_flow_control(&b.body)) {
                    let mut taken = None;
                    for branch in branches {
                        match &branch.condition {
                            None => {
                                taken = Some(&branch.body);
                                break;
                            }
                            Some(condition) => {
                                let condition = substitute_refs(condition, bindings);
                                let value = const_eval(&condition, consts).ok_or_else(|| {
                                    "a branch holding `break` or `return` needs a condition \
                                     the compiler can decide"
                                        .to_string()
                                })?;
                                if value != 0.0 {
                                    taken = Some(&branch.body);
                                    break;
                                }
                            }
                        }
                    }
                    if let Some(body) = taken {
                        let flow = execute(
                            body,
                            bindings,
                            assigned,
                            consts,
                            sizes,
                            registry,
                            scope,
                            imports,
                            depth + 1,
                            fold,
                        )?;
                        if flow != Flow::Normal {
                            return Ok(flow);
                        }
                    }
                    continue;
                }
                let before = bindings.clone();
                let mut outcomes: Vec<(Option<Expr>, HashMap<String, Expr>)> = Vec::new();
                for branch in branches {
                    let mut local = before.clone();
                    execute(
                        &branch.body,
                        &mut local,
                        assigned,
                        consts,
                        sizes,
                        registry,
                        scope,
                        imports,
                        depth + 1,
                        fold,
                    )?;
                    let condition = branch
                        .condition
                        .as_ref()
                        .map(|c| {
                            let c = substitute_refs(c, &before);
                            resolve(
                                &c,
                                &HashMap::new(),
                                consts,
                                registry,
                                scope,
                                imports,
                                depth + 1,
                            )
                        })
                        .transpose()?;
                    outcomes.push((condition, local));
                }
                // Every variable any branch wrote gets one merged value.
                let mut touched: Vec<String> = Vec::new();
                for (_, local) in &outcomes {
                    for name in local.keys() {
                        if before.get(name) != local.get(name) && !touched.contains(name) {
                            touched.push(name.clone());
                        }
                    }
                }
                touched.sort();
                for name in touched {
                    let fallback = before.get(&name).cloned();
                    let mut value = match outcomes.last() {
                        // A trailing `else` supplies the last value.
                        Some((None, local)) => local.get(&name).cloned().or(fallback.clone()),
                        _ => fallback.clone(),
                    };
                    for (condition, local) in outcomes.iter().rev() {
                        let Some(condition) = condition else { continue };
                        let taken = local.get(&name).cloned().or_else(|| fallback.clone());
                        match (taken, value) {
                            (Some(taken), Some(otherwise)) => {
                                value = Some(Expr::If(
                                    Box::new(condition.clone()),
                                    Box::new(taken),
                                    Box::new(otherwise),
                                ));
                            }
                            _ => {
                                return Err(format!(
                                    "`{name}` is assigned in one branch only and has no value before the `if`"
                                ))
                            }
                        }
                    }
                    let Some(value) = value else {
                        return Err(format!(
                            "`{name}` is assigned in one branch only and has no value before the `if`"
                        ));
                    };
                    bindings.insert(name, value);
                }
            }
            Statement::For(variable, (lower, upper), body) => {
                let bound = |expr: &Expr| -> Result<i64, String> {
                    let expr = substitute_refs(expr, bindings);
                    // Through the array layer first, so a bound written
                    // `size(v, 1)` is a number by the time it is asked
                    // to be constant.
                    let no_loop_vars = HashMap::new();
                    let measured = expand(
                        &expr,
                        &Shapes {
                            sizes,
                            loop_vars: &no_loop_vars,
                            consts,
                        },
                        registry,
                        scope,
                        imports,
                        depth + 1,
                    )
                    .map(Value::into_expr)
                    .unwrap_or_else(|_| expr.clone());
                    let value = const_eval(&measured, consts).ok_or_else(|| {
                        format!("loop bound of `{variable}` is not a compile-time constant")
                    })?;
                    if value.fract() != 0.0 {
                        return Err(format!("loop bound must be a whole number, got {value}"));
                    }
                    Ok(value as i64)
                };
                let (lower, upper) = (bound(lower)?, bound(upper)?);
                for index in lower..=upper {
                    bindings.insert(variable.clone(), Expr::Number(index as f64));
                    let flow = execute(
                        body,
                        bindings,
                        assigned,
                        consts,
                        sizes,
                        registry,
                        scope,
                        imports,
                        depth + 1,
                        fold,
                    )?;
                    match flow {
                        Flow::Normal => {}
                        Flow::Break => break,
                        Flow::Return => {
                            bindings.remove(variable);
                            assigned.retain(|name| name != variable);
                            return Ok(Flow::Return);
                        }
                    }
                }
                bindings.remove(variable);
                assigned.retain(|name| name != variable);
            }
            Statement::While(condition, body) => {
                let mut rounds = 0;
                loop {
                    let now = substitute_refs(condition, bindings);
                    let truth = const_eval(&now, consts).ok_or_else(|| {
                        "a `while` condition must be decidable at compile time: algorithms \
                         are executed symbolically, so the trip count cannot depend on a \
                         simulated variable"
                            .to_string()
                    })?;
                    if truth == 0.0 {
                        break;
                    }
                    rounds += 1;
                    if rounds > MAX_WHILE_ROUNDS {
                        return Err(format!(
                            "a `while` did not finish within {MAX_WHILE_ROUNDS} rounds"
                        ));
                    }
                    let flow = execute(
                        body,
                        bindings,
                        assigned,
                        consts,
                        sizes,
                        registry,
                        scope,
                        imports,
                        depth + 1,
                        true,
                    )?;
                    match flow {
                        Flow::Normal => {}
                        Flow::Break => break,
                        Flow::Return => return Ok(Flow::Return),
                    }
                }
            }
            Statement::Break => return Ok(Flow::Break),
            Statement::Return => return Ok(Flow::Return),
        }
    }
    Ok(Flow::Normal)
}

/// Resolve both sides of a `connect` into instance paths and pair them.
///
/// A subscripted reference folds to one path; a whole array of
/// connectors pairs element by element with the other side, which must
/// then have the same length. Connections to components a condition
/// left out are dropped, like everywhere else.
#[allow(clippy::too_many_arguments)]
fn push_connects(
    a: &Expr,
    b: &Expr,
    shapes: &Shapes,
    prefix: &str,
    outers: &HashMap<String, String>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    acc: &mut Flat,
) -> Result<(), String> {
    let side = |expr: &Expr| -> Result<Vec<String>, String> {
        let value = expand(
            &prefix_expr(expr, prefix, outers),
            shapes,
            registry,
            scope,
            imports,
            0,
        )?;
        let mut items = Vec::new();
        value.flatten_into(&mut items);
        items
            .into_iter()
            .map(|item| match item {
                Expr::Ref(path) => Ok(path),
                other => Err(format!(
                    "a side of connect must reference connectors, found {other:?}"
                )),
            })
            .collect()
    };
    let (left, right) = (side(a)?, side(b)?);
    if left.len() != right.len() {
        return Err(format!(
            "connect between {} and {} connector(s)",
            left.len(),
            right.len()
        ));
    }
    for (a, b) in left.into_iter().zip(right) {
        if acc.is_disabled(&a) || acc.is_disabled(&b) {
            continue;
        }
        acc.connects.push((a, b));
    }
    Ok(())
}

/// Dimensions of every array component of a class and of its bases.
fn collect_shapes(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    consts: &HashMap<String, f64>,
    out: &mut HashMap<String, Vec<i64>>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let scope = class.name.as_str();
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, scope, &class.imports) {
            collect_shapes(registry, base, consts, out, depth + 1);
        }
    }
    for component in &class.components {
        if component.dimensions.is_empty() {
            continue;
        }
        // Declarations are visited in source order, so a length
        // written as `size(v, 1)` can look up a `v` already measured -
        // which is how a function's result takes the shape of its
        // argument.
        let sizes: Option<Vec<i64>> = component
            .dimensions
            .iter()
            .map(|dimension| dimension_value(dimension, consts, out))
            .collect();
        if let Some(sizes) = sizes {
            out.insert(component.name.clone(), sizes);
        }
    }
}

/// The shape of a value, as the dimension tables spell it.
fn shape_i64(value: &Value) -> Vec<i64> {
    value
        .shape()
        .into_iter()
        .map(|length| length as i64)
        .collect()
}

/// One array dimension as a number, or `None` when it cannot be told
/// here - a colon waiting for a call site, or a length that depends on
/// something not yet known.
fn dimension_value(
    expr: &Expr,
    consts: &HashMap<String, f64>,
    sizes: &HashMap<String, Vec<i64>>,
) -> Option<i64> {
    // `size(v)` and `size(v, k)` of something already measured.
    if let Expr::Call(name, args) = expr {
        if name == "size" && !args.is_empty() {
            if let Expr::Ref(of) = &args[0] {
                let shape = sizes.get(of)?;
                let index = match args.get(1) {
                    None => 0,
                    Some(dimension) => const_eval(dimension, consts)? as usize - 1,
                };
                return shape.get(index).copied();
            }
        }
    }
    let value = const_eval(expr, consts)?;
    (value.fract() == 0.0 && value >= 0.0).then_some(value as i64)
}

/// The same shapes under the instance path, since equations are
/// prefixed before they are expanded.
fn prefixed_sizes(sizes: &HashMap<String, Vec<i64>>, prefix: &str) -> HashMap<String, Vec<i64>> {
    sizes
        .iter()
        .map(|(name, dimensions)| (format!("{prefix}{name}"), dimensions.clone()))
        .collect()
}

/// Emit one equation per element, refusing sides that do not match.
fn push_equations(lhs: &Value, rhs: &Value, acc: &mut Flat) -> Result<(), String> {
    let (left_shape, right_shape) = (lhs.shape(), rhs.shape());
    if left_shape != right_shape {
        return Err(format!(
            "an equation between shapes {left_shape:?} and {right_shape:?}"
        ));
    }
    let (mut left, mut right) = (Vec::new(), Vec::new());
    lhs.flatten_into(&mut left);
    rhs.flatten_into(&mut right);
    for (lhs, rhs) in left.into_iter().zip(right) {
        acc.equations.push(EquationItem { lhs, rhs });
    }
    Ok(())
}

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
}

/// Expand an expression into scalars, keeping the array structure while
/// it is needed and dropping to the scalar path for everything else.
#[allow(clippy::too_many_arguments)]
fn expand(
    expr: &Expr,
    shapes: &Shapes,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return Err("expression nested deeper than the instantiation limit".to_string());
    }
    let recur = |e: &Expr| expand(e, shapes, registry, scope, imports, depth + 1);
    let scalar = |e: &Expr| -> Result<Value, String> {
        Ok(Value::Scalar(resolve(
            e,
            shapes.loop_vars,
            shapes.consts,
            registry,
            scope,
            imports,
            depth + 1,
        )?))
    };

    let constant_here = |e: &Expr| -> Option<f64> {
        let mut env = shapes.consts.clone();
        env.extend(shapes.loop_vars.iter().map(|(k, v)| (k.clone(), *v)));
        const_eval(e, &env)
    };
    Ok(match expr {
        Expr::Array(items) => Value::Array(
            items
                .iter()
                .map(&recur)
                .collect::<Result<Vec<_>, String>>()?,
        ),
        // A range is a vector whose bounds the compiler can see.
        Expr::Range(a, step, b) => {
            let scalar_of = |e: &Expr| -> Result<f64, String> {
                let resolved = recur(e)?.scalar()?;
                constant_here(&resolved)
                    .ok_or_else(|| "a range needs bounds the compiler can see".to_string())
            };
            let (from, to) = (scalar_of(a)?, scalar_of(b)?);
            let step = match step {
                Some(step) => scalar_of(step)?,
                None => 1.0,
            };
            if step == 0.0 {
                return Err("a range cannot step by zero".to_string());
            }
            let count = ((to - from) / step + 1e-9).floor() as i64 + 1;
            Value::Array(
                (0..count.max(0))
                    .map(|i| Value::Scalar(Expr::Number(from + i as f64 * step)))
                    .collect(),
            )
        }
        // `{expr for i in range}` unrolls with the iterator bound.
        Expr::Comprehension(body, variable, range) => {
            let Value::Array(items) = recur(range)? else {
                return Err(format!("`{variable}` needs an array to iterate over"));
            };
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let value = constant_here(&item.scalar()?).ok_or_else(|| {
                    format!("the range of `{variable}` must be constant at compile time")
                })?;
                let mut loop_vars = shapes.loop_vars.clone();
                loop_vars.insert(variable.clone(), value);
                let inner = Shapes {
                    sizes: shapes.sizes,
                    loop_vars: &loop_vars,
                    consts: shapes.consts,
                };
                out.push(expand(body, &inner, registry, scope, imports, depth + 1)?);
            }
            Value::Array(out)
        }
        // `[a, b; c, d]`: rows concatenated along the first dimension,
        // the elements of a row along the second. A scalar element is a
        // 1x1 block; vector elements lie along the row.
        Expr::MatrixRows(rows) => {
            let mut out_rows: Vec<Value> = Vec::new();
            for row in rows {
                let mut cells: Vec<Value> = Vec::new();
                for item in row {
                    match recur(item)? {
                        Value::Scalar(expr) => cells.push(Value::Scalar(expr)),
                        Value::Array(items) => cells.extend(items),
                    }
                }
                out_rows.push(Value::Array(cells));
            }
            let width = out_rows
                .first()
                .map(|row| row.shape().first().copied().unwrap_or(0))
                .unwrap_or(0);
            for row in &out_rows {
                if row.shape().first().copied().unwrap_or(0) != width {
                    return Err("the rows of a matrix must be equally wide".to_string());
                }
            }
            Value::Array(out_rows)
        }
        Expr::ColonSubscript | Expr::EndSubscript => {
            return Err("`:` and `end` make sense only inside a subscript".to_string())
        }
        // A name that was declared with dimensions stands for all of its
        // elements at once.
        Expr::Ref(name) if shapes.sizes.contains_key(name) => {
            elements_of(name, &shapes.sizes[name])
        }
        Expr::Neg(inner) => map_value(&recur(inner)?, &|e| Expr::Neg(Box::new(e))),
        Expr::Bin(op, l, r) => combine(*op, &recur(l)?, &recur(r)?, false)?,
        Expr::Elementwise(op, l, r) => combine(*op, &recur(l)?, &recur(r)?, true)?,
        Expr::If(condition, then, otherwise) => {
            let condition = recur(condition)?.scalar()?;
            // A guard on the loop variable takes its branch and
            // leaves the other alone: at the first element of a loop
            // over neighbours, `if i > 1 then x[i - 1] else 0` must
            // not go looking for `x[0]`. A condition that does not
            // mention the loop stays as it was written, parameters
            // included - folding those would nail down a value the
            // model is meant to be re-run with.
            // Inside a loop being unrolled, everything the compiler
            // can decide is part of the structure being built; outside
            // one it is a value the model may be re-run with.
            if shapes.loop_vars.is_empty() {
                let (then, otherwise) = (recur(then)?, recur(otherwise)?);
                return zip_values(&then, &otherwise, &|a, b| {
                    Expr::If(
                        Box::new(condition.clone()),
                        Box::new(a.clone()),
                        Box::new(b.clone()),
                    )
                });
            }
            if let Some(truth) = constant_here(&condition) {
                return if truth != 0.0 {
                    recur(then)
                } else {
                    recur(otherwise)
                };
            }
            let (then, otherwise) = (recur(then)?, recur(otherwise)?);
            zip_values(&then, &otherwise, &|a, b| {
                Expr::If(
                    Box::new(condition.clone()),
                    Box::new(a.clone()),
                    Box::new(b.clone()),
                )
            })?
        }
        Expr::Call(name, args) => expand_call(name, args, shapes, registry, scope, imports, depth)?,
        // Indexing something that expands to an array picks the element:
        // this is how `a[i]` works inside a function whose `a` was bound
        // to an array literal.
        Expr::Index(base, subscripts) => {
            let base_value = recur(base)?;
            match base_value {
                Value::Array(_) => {
                    // A subscript picks one element; a range, a `:` or a
                    // vector of indices takes a slice; `end` stands for
                    // this dimension's length.
                    let mut current = base_value;
                    for subscript in subscripts {
                        let Value::Array(items) = current else {
                            return Err("more subscripts than dimensions".to_string());
                        };
                        let length = items.len();
                        let one = |index: f64| -> Result<Value, String> {
                            if index.fract() != 0.0 || index < 1.0 || index as usize > length {
                                return Err(format!(
                                    "subscript {index} is outside an array of {length}"
                                ));
                            }
                            Ok(items[index as usize - 1].clone())
                        };
                        let with_end = substitute_end(subscript, length as f64);
                        current = match &with_end {
                            Expr::ColonSubscript => Value::Array(items.clone()),
                            _ => match expand(
                                &with_end,
                                shapes,
                                registry,
                                scope,
                                imports,
                                depth + 1,
                            )? {
                                Value::Scalar(index) => {
                                    let index = constant_here(&index).ok_or_else(|| {
                                        "a subscript into an array value must be a                                          compile-time constant"
                                            .to_string()
                                    })?;
                                    one(index)?
                                }
                                Value::Array(picks) => {
                                    // A vector subscript selects a slice.
                                    let mut out = Vec::with_capacity(picks.len());
                                    for pick in picks {
                                        let index =
                                            constant_here(&pick.scalar()?).ok_or_else(|| {
                                                "a slicing subscript must be constant at                                                  compile time"
                                                    .to_string()
                                            })?;
                                        out.push(one(index)?);
                                    }
                                    Value::Array(out)
                                }
                            },
                        };
                    }
                    current
                }
                _ => scalar(expr)?,
            }
        }
        other => scalar(other)?,
    })
}

/// Replace `end` inside a subscript with the dimension's length.
fn substitute_end(expr: &Expr, length: f64) -> Expr {
    match expr {
        Expr::EndSubscript => Expr::Number(length),
        Expr::Bin(op, l, r) => Expr::Bin(
            *op,
            Box::new(substitute_end(l, length)),
            Box::new(substitute_end(r, length)),
        ),
        Expr::Neg(inner) => Expr::Neg(Box::new(substitute_end(inner, length))),
        Expr::Range(a, step, b) => Expr::Range(
            Box::new(substitute_end(a, length)),
            step.as_ref().map(|s| Box::new(substitute_end(s, length))),
            Box::new(substitute_end(b, length)),
        ),
        other => other.clone(),
    }
}

/// The scalar references an array name stands for: `v` of `Real v[3]`
/// is `{v[1], v[2], v[3]}`.
fn elements_of(name: &str, sizes: &[i64]) -> Value {
    match sizes.split_first() {
        None => Value::Scalar(Expr::Ref(name.to_string())),
        Some((&length, rest)) => Value::Array(
            (1..=length)
                .map(|index| {
                    if rest.is_empty() {
                        Value::Scalar(Expr::Ref(element_name(name, &[index])))
                    } else {
                        // Deeper dimensions keep the subscripts together,
                        // which is how the flat names are written.
                        let inner = elements_of(name, rest);
                        prefix_subscript(&inner, name, index)
                    }
                })
                .collect(),
        ),
    }
}

/// Put an outer subscript in front of the ones a nested value carries.
fn prefix_subscript(value: &Value, name: &str, index: i64) -> Value {
    match value {
        Value::Scalar(Expr::Ref(inner)) => {
            let subscripts = inner
                .strip_prefix(name)
                .and_then(|rest| rest.strip_prefix('['))
                .and_then(|rest| rest.strip_suffix(']'))
                .unwrap_or_default();
            Value::Scalar(Expr::Ref(format!("{name}[{index},{subscripts}]")))
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| prefix_subscript(item, name, index))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Apply a scalar operation to every element of a value.
fn map_value(value: &Value, f: &dyn Fn(Expr) -> Expr) -> Value {
    match value {
        Value::Scalar(expr) => Value::Scalar(f(expr.clone())),
        Value::Array(items) => Value::Array(items.iter().map(|item| map_value(item, f)).collect()),
    }
}

/// Pair two values element by element, broadcasting a scalar over an
/// array. Arrays of different shapes are an error, not a guess.
fn zip_values(
    left: &Value,
    right: &Value,
    f: &dyn Fn(&Expr, &Expr) -> Expr,
) -> Result<Value, String> {
    Ok(match (left, right) {
        (Value::Scalar(a), Value::Scalar(b)) => Value::Scalar(f(a, b)),
        (Value::Array(items), Value::Scalar(b)) => Value::Array(
            items
                .iter()
                .map(|item| zip_values(item, &Value::Scalar(b.clone()), f))
                .collect::<Result<Vec<_>, String>>()?,
        ),
        (Value::Scalar(a), Value::Array(items)) => Value::Array(
            items
                .iter()
                .map(|item| zip_values(&Value::Scalar(a.clone()), item, f))
                .collect::<Result<Vec<_>, String>>()?,
        ),
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Err(format!(
                    "arrays of {} and {} elements do not fit together",
                    a.len(),
                    b.len()
                ));
            }
            Value::Array(
                a.iter()
                    .zip(b)
                    .map(|(x, y)| zip_values(x, y, f))
                    .collect::<Result<Vec<_>, String>>()?,
            )
        }
    })
}

/// Combine two values with an arithmetic operator.
///
/// Written with a dot the operator always works element by element.
/// Written plainly, `+` and `-` do too, while `*` between two vectors is
/// their scalar product and `/` only divides by a scalar - which is what
/// the language means by them.
fn combine(op: BinOp, left: &Value, right: &Value, elementwise: bool) -> Result<Value, String> {
    let apply = |a: &Expr, b: &Expr| Expr::Bin(op, Box::new(a.clone()), Box::new(b.clone()));
    if elementwise || matches!(op, BinOp::Add | BinOp::Sub) {
        return zip_values(left, right, &apply);
    }
    match (op, left, right) {
        (BinOp::Mul, Value::Array(a), Value::Array(b)) => {
            let (left_shape, right_shape) = (left.shape(), right.shape());
            match (left_shape.len(), right_shape.len()) {
                // Vector times vector is their scalar product.
                (1, 1) => {
                    if a.len() != b.len() {
                        return Err(format!(
                            "a scalar product needs equal lengths, got {} and {}",
                            a.len(),
                            b.len()
                        ));
                    }
                    let products = zip_values(left, right, &apply)?;
                    let mut terms = Vec::new();
                    products.flatten_into(&mut terms);
                    Ok(Value::Scalar(sum_of(terms)))
                }
                // Matrix times vector, vector times matrix, and matrix
                // times matrix follow the usual row-by-column rule.
                (2, 1) => a
                    .iter()
                    .map(|row| combine(BinOp::Mul, row, right, false))
                    .collect::<Result<Vec<_>, String>>()
                    .map(Value::Array),
                (1, 2) => {
                    let columns = right_shape[1];
                    (0..columns)
                        .map(|column| {
                            let column = pick_column(b, column)?;
                            combine(BinOp::Mul, left, &column, false)
                        })
                        .collect::<Result<Vec<_>, String>>()
                        .map(Value::Array)
                }
                (2, 2) => a
                    .iter()
                    .map(|row| combine(BinOp::Mul, row, right, false))
                    .collect::<Result<Vec<_>, String>>()
                    .map(Value::Array),
                _ => Err("`*` between arrays deeper than matrices".to_string()),
            }
        }
        (BinOp::Div, _, Value::Array(_)) => {
            Err("an array cannot be a divisor; use `./` for element by element".to_string())
        }
        _ => zip_values(left, right, &apply),
    }
}

/// One column of a matrix given by rows, as a vector value.
fn pick_column(rows: &[Value], column: usize) -> Result<Value, String> {
    rows.iter()
        .map(|row| {
            let Value::Array(cells) = row else {
                return Err("a matrix row must be an array".to_string());
            };
            cells
                .get(column)
                .cloned()
                .ok_or_else(|| "the rows of a matrix must be equally wide".to_string())
        })
        .collect::<Result<Vec<_>, String>>()
        .map(Value::Array)
}

/// The sum of a list of expressions, or zero when it is empty.
fn sum_of(terms: Vec<Expr>) -> Expr {
    terms
        .into_iter()
        .reduce(|a, b| Expr::Bin(BinOp::Add, Box::new(a), Box::new(b)))
        .unwrap_or(Expr::Number(0.0))
}

/// The array built-ins, and the ordinary ones applied to every element.
#[allow(clippy::too_many_arguments)]
fn expand_call(
    name: &str,
    args: &[Expr],
    shapes: &Shapes,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
) -> Result<Value, String> {
    let recur = |e: &Expr| expand(e, shapes, registry, scope, imports, depth + 1);
    let constant = |e: &Expr| -> Result<i64, String> {
        let value = recur(e)?.scalar()?;
        let mut env = shapes.consts.clone();
        env.extend(shapes.loop_vars.iter().map(|(k, v)| (k.clone(), *v)));
        let value = const_eval(&value, &env)
            .ok_or_else(|| format!("`{name}` needs a length the compiler can see"))?;
        if value.fract() != 0.0 || value < 0.0 {
            return Err(format!(
                "`{name}`: a length must be a whole number, got {value}"
            ));
        }
        Ok(value as i64)
    };

    match (name, args.len()) {
        // How long an array is, which is a compile-time number.
        ("size", 1) => {
            let shape = recur(&args[0])?.shape();
            Ok(Value::Array(
                shape
                    .into_iter()
                    .map(|length| Value::Scalar(Expr::Number(length as f64)))
                    .collect(),
            ))
        }
        ("size", 2) => {
            let shape = recur(&args[0])?.shape();
            let dimension = constant(&args[1])?;
            let length = shape
                .get((dimension - 1).max(0) as usize)
                .ok_or_else(|| format!("size(..., {dimension}): there is no such dimension"))?;
            Ok(Value::Scalar(Expr::Number(*length as f64)))
        }
        // Reductions.
        ("sum", 1) | ("product", 1) => {
            let mut terms = Vec::new();
            recur(&args[0])?.flatten_into(&mut terms);
            Ok(Value::Scalar(match name {
                "sum" => sum_of(terms),
                _ => terms
                    .into_iter()
                    .reduce(|a, b| Expr::Bin(BinOp::Mul, Box::new(a), Box::new(b)))
                    .unwrap_or(Expr::Number(1.0)),
            }))
        }
        ("min", 1) | ("max", 1) => {
            let mut terms = Vec::new();
            recur(&args[0])?.flatten_into(&mut terms);
            let reduced = terms
                .into_iter()
                .reduce(|a, b| Expr::Call(name.to_string(), vec![a, b]))
                .ok_or_else(|| format!("`{name}` of an empty array"))?;
            Ok(Value::Scalar(reduced))
        }
        // Constructors.
        ("zeros", 1) | ("ones", 1) => {
            let length = constant(&args[0])?;
            let value = if name == "ones" { 1.0 } else { 0.0 };
            Ok(Value::Array(
                (0..length)
                    .map(|_| Value::Scalar(Expr::Number(value)))
                    .collect(),
            ))
        }
        ("fill", 2) => {
            let filler = recur(&args[0])?.scalar()?;
            let length = constant(&args[1])?;
            Ok(Value::Array(
                (0..length).map(|_| Value::Scalar(filler.clone())).collect(),
            ))
        }
        ("transpose", 1) => {
            let value = recur(&args[0])?;
            let shape = value.shape();
            if shape.len() != 2 {
                return Err("transpose works on a matrix".to_string());
            }
            let Value::Array(rows) = &value else {
                return Err("transpose works on a matrix".to_string());
            };
            (0..shape[1])
                .map(|column| pick_column(rows, column))
                .collect::<Result<Vec<_>, String>>()
                .map(Value::Array)
        }
        ("identity", 1) => {
            let n = constant(&args[0])?;
            Ok(Value::Array(
                (1..=n)
                    .map(|i| {
                        Value::Array(
                            (1..=n)
                                .map(|j| {
                                    Value::Scalar(Expr::Number(if i == j { 1.0 } else { 0.0 }))
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            ))
        }
        ("diagonal", 1) => {
            let Value::Array(items) = recur(&args[0])? else {
                return Err("diagonal takes a vector".to_string());
            };
            let n = items.len();
            Ok(Value::Array(
                (0..n)
                    .map(|i| {
                        Value::Array(
                            (0..n)
                                .map(|j| {
                                    if i == j {
                                        items[i].clone()
                                    } else {
                                        Value::Scalar(Expr::Number(0.0))
                                    }
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            ))
        }
        ("cross", 2) => {
            let (a, b) = (recur(&args[0])?, recur(&args[1])?);
            let (Value::Array(a), Value::Array(b)) = (&a, &b) else {
                return Err("cross takes two 3-vectors".to_string());
            };
            if a.len() != 3 || b.len() != 3 {
                return Err("cross takes two 3-vectors".to_string());
            }
            let term = |i: usize, j: usize| -> Result<Expr, String> {
                Ok(Expr::Bin(
                    BinOp::Mul,
                    Box::new(a[i].clone().scalar()?),
                    Box::new(b[j].clone().scalar()?),
                ))
            };
            let minus = |p: Expr, q: Expr| Expr::Bin(BinOp::Sub, Box::new(p), Box::new(q));
            Ok(Value::Array(vec![
                Value::Scalar(minus(term(1, 2)?, term(2, 1)?)),
                Value::Scalar(minus(term(2, 0)?, term(0, 2)?)),
                Value::Scalar(minus(term(0, 1)?, term(1, 0)?)),
            ]))
        }
        // cat(1, ...) stacks along the first dimension; cat(2, ...)
        // joins along the second.
        ("cat", n) if n >= 2 => {
            let along = constant(&args[0])?;
            let values = args[1..]
                .iter()
                .map(&recur)
                .collect::<Result<Vec<_>, String>>()?;
            match along {
                1 => {
                    let mut out = Vec::new();
                    for value in values {
                        match value {
                            Value::Array(items) => out.extend(items),
                            scalar => out.push(scalar),
                        }
                    }
                    Ok(Value::Array(out))
                }
                2 => {
                    let rows = values
                        .first()
                        .map(|value| value.shape().first().copied().unwrap_or(0))
                        .unwrap_or(0);
                    (0..rows)
                        .map(|row| {
                            let mut cells = Vec::new();
                            for value in &values {
                                let Value::Array(these) = value else {
                                    return Err("cat(2, ...) takes matrices".to_string());
                                };
                                let Some(Value::Array(row_cells)) = these.get(row) else {
                                    return Err("cat(2, ...) needs equal row counts".to_string());
                                };
                                cells.extend(row_cells.iter().cloned());
                            }
                            Ok(Value::Array(cells))
                        })
                        .collect::<Result<Vec<_>, String>>()
                        .map(Value::Array)
                }
                other => Err(format!("cat along dimension {other} is not supported")),
            }
        }
        ("linspace", 3) => {
            let (from, to) = (recur(&args[0])?.scalar()?, recur(&args[1])?.scalar()?);
            let length = constant(&args[2])?;
            if length < 2 {
                return Err("linspace needs at least two points".to_string());
            }
            Ok(Value::Array(
                (0..length)
                    .map(|index| {
                        let fraction = index as f64 / (length - 1) as f64;
                        // from + (to - from) * fraction
                        Value::Scalar(Expr::Bin(
                            BinOp::Add,
                            Box::new(from.clone()),
                            Box::new(Expr::Bin(
                                BinOp::Mul,
                                Box::new(Expr::Bin(
                                    BinOp::Sub,
                                    Box::new(to.clone()),
                                    Box::new(from.clone()),
                                )),
                                Box::new(Expr::Number(fraction)),
                            )),
                        ))
                    })
                    .collect(),
            ))
        }
        _ => {
            // A user function that takes or returns an array is inlined
            // with the arrays intact - vectorizing it element by element
            // would compute something else entirely.
            if let Some(class) = lookup(registry, name, scope, imports) {
                if class.kind == ClassKind::Function
                    && class
                        .components
                        .iter()
                        .any(|c| c.causality != Causality::None && !c.dimensions.is_empty())
                {
                    let values = args
                        .iter()
                        .map(&recur)
                        .collect::<Result<Vec<_>, String>>()?;
                    // The shape each argument turned out to have is
                    // what a `[:]` input takes its length from.
                    let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
                    let arguments: Vec<Expr> =
                        values.into_iter().map(|value| value.into_expr()).collect();
                    let result = inline_function(
                        class,
                        &arguments,
                        &argument_shapes,
                        shapes.consts,
                        registry,
                        depth + 1,
                    )?;
                    return expand(&result, shapes, registry, scope, imports, depth + 1);
                }
            }
            // Anything else: an ordinary call, applied to every element
            // when an argument turns out to be an array.
            let values = args
                .iter()
                .map(&recur)
                .collect::<Result<Vec<_>, String>>()?;
            let arrayed = values.iter().any(|value| matches!(value, Value::Array(_)));
            if !arrayed {
                let scalars = values
                    .into_iter()
                    .map(Value::scalar)
                    .collect::<Result<Vec<_>, String>>()?;
                return resolve(
                    &Expr::Call(name.to_string(), scalars),
                    shapes.loop_vars,
                    shapes.consts,
                    registry,
                    scope,
                    imports,
                    depth + 1,
                )
                .map(Value::Scalar);
            }
            // The call spreads over the elements, a scalar argument
            // travelling unchanged to every one - the vectorization the
            // language gives every scalar function.
            let length = values
                .iter()
                .filter_map(|value| match value {
                    Value::Array(items) => Some(items.len()),
                    Value::Scalar(_) => None,
                })
                .max()
                .expect("at least one argument is an array");
            if values
                .iter()
                .any(|value| matches!(value, Value::Array(items) if items.len() != length))
            {
                return Err(format!(
                    "`{name}`: its array arguments must have one length"
                ));
            }
            let elements = (0..length)
                .map(|index| {
                    let args = values
                        .iter()
                        .map(|value| match value {
                            Value::Array(items) => items[index].clone().scalar(),
                            Value::Scalar(expr) => Ok(expr.clone()),
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    // Through `resolve`, so a user function still
                    // inlines per element instead of surviving as a call.
                    resolve(
                        &Expr::Call(name.to_string(), args),
                        shapes.loop_vars,
                        shapes.consts,
                        registry,
                        scope,
                        imports,
                        depth + 1,
                    )
                    .map(Value::Scalar)
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Value::Array(elements))
        }
    }
}

/// Inline a function call: arguments are bound to the inputs, the
/// algorithm's assignments are substituted in order, and the output
/// expression replaces the call.
/// Inline a call in an expression: the value is the first output. A
/// function with several outputs may still be called this way; the
/// rest are computed for nothing and dropped, as the spec allows.
fn inline_function(
    class: &ClassDef,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
) -> Result<Expr, String> {
    let mut outputs = inline_function_outputs(class, args, shapes, consts, registry, depth)?;
    Ok(outputs.remove(0).1)
}

/// Execute a function body symbolically and return every output, in
/// declaration order, as `(name, expression)`. Arguments are matched
/// positionally, then by name (`f(x, precision = 6)`); an input left
/// unmatched falls back to its declared default.
fn inline_function_outputs(
    class: &ClassDef,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
) -> Result<Vec<(String, Expr)>, String> {
    if depth > MAX_DEPTH {
        return Err(format!("recursive function `{}`", class.name));
    }
    let inputs: Vec<&Component> = class
        .components
        .iter()
        .filter(|c| c.causality == Causality::Input)
        .collect();
    let outputs: Vec<&Component> = class
        .components
        .iter()
        .filter(|c| c.causality == Causality::Output)
        .collect();
    if outputs.is_empty() {
        return Err(format!("function `{}` declares no output", class.name));
    }
    let mut bindings: HashMap<String, Expr> = HashMap::new();
    let mut given_shapes: HashMap<String, Vec<i64>> = HashMap::new();
    let mut position = 0;
    for (index, arg) in args.iter().enumerate() {
        if let Expr::NamedArg(name, value) = arg {
            if !inputs.iter().any(|input| &input.name == name) {
                return Err(format!(
                    "function `{}` has no input named `{name}`",
                    class.name
                ));
            }
            if bindings.insert(name.clone(), (**value).clone()).is_some() {
                return Err(format!(
                    "argument `{name}` of function `{}` is given twice",
                    class.name
                ));
            }
        } else {
            if bindings.len() > position {
                return Err(format!(
                    "function `{}`: positional arguments must come before named ones",
                    class.name
                ));
            }
            let Some(input) = inputs.get(position) else {
                return Err(format!(
                    "function `{}` expects {} argument(s), got more",
                    class.name,
                    inputs.len()
                ));
            };
            // A `[:]` input is as long as whatever was handed to it.
            if !input.dimensions.is_empty() {
                if let Some(shape) = shapes.get(index) {
                    if !shape.is_empty() {
                        given_shapes.insert(input.name.clone(), shape.clone());
                    }
                }
            }
            bindings.insert(input.name.clone(), arg.clone());
            position += 1;
        }
    }
    // Whatever the call left unsaid falls back to the input's own
    // default. Defaults may name earlier inputs, so they are resolved
    // against what is already bound.
    for input in &inputs {
        if !bindings.contains_key(&input.name) {
            let Some(default) = &input.binding else {
                return Err(format!(
                    "function `{}` is missing its argument `{}`",
                    class.name, input.name
                ));
            };
            let default = substitute_refs(default, &bindings);
            bindings.insert(input.name.clone(), default);
        }
    }
    for component in &class.components {
        if component.causality == Causality::None {
            if let Some(binding) = &component.binding {
                bindings.insert(component.name.clone(), binding.clone());
            }
        }
    }
    let mut assigned = Vec::new();
    // The lengths the call decided go in first: a declared dimension
    // that is a colon measures nothing on its own, and a result sized
    // `size(v, 1)` reads its length back out of here.
    let mut sizes: HashMap<String, Vec<i64>> = given_shapes;
    collect_shapes(registry, class, consts, &mut sizes, 0);
    // `Return` is simply an early landing here; the outputs are read
    // out the same way. A `break` with no loop has nowhere to go.
    if execute(
        &class.algorithm,
        &mut bindings,
        &mut assigned,
        consts,
        &sizes,
        registry,
        &class.name,
        &class.imports,
        depth + 1,
        false,
    )? == Flow::Break
    {
        return Err(format!(
            "`break` outside of a loop in function `{}`",
            class.name
        ));
    }
    outputs
        .iter()
        .map(|output| {
            let name = &output.name;
            // A whole-array assignment bound the name itself;
            // per-element assignments bound `c[1]`, `c[2]`, ... -
            // gather them in order.
            if let Some(expr) = bindings.get(name) {
                return Ok((name.clone(), expr.clone()));
            }
            if let Some(dimensions) = sizes.get(name) {
                let items = index_tuples(dimensions)
                    .into_iter()
                    .map(|indices| {
                        let element = element_name(name, &indices);
                        bindings.get(&element).cloned().ok_or_else(|| {
                            format!(
                                "function `{}` never assigns `{element}` of its output",
                                class.name
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                return Ok((name.clone(), Expr::Array(items)));
            }
            Err(format!(
                "function `{}` never assigns its output `{name}`",
                class.name
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::ast::Expr;
    use crate::parser::parse_model;

    #[test]
    fn flat_model_passes_through_unchanged() {
        let m = parse_model("model M Real x(start = 1); equation der(x) = -x; end M;").unwrap();
        assert_eq!(m.name, "M");
        assert_eq!(m.components.len(), 1);
        assert_eq!(m.components[0].name, "x");
    }

    #[test]
    fn instantiates_components_with_prefixes_and_modifiers() {
        let m = parse_model(
            "model Gain parameter Real k = 1; Real u; Real y; equation y = k * u; end Gain;\
             model Top Gain g1(k = 3); Gain g2; Real s; equation \
             g1.u = time; g2.u = g1.y; s = g2.y; end Top;",
        )
        .unwrap();
        assert_eq!(m.name, "Top");
        let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"g1.k"));
        assert!(names.contains(&"g2.y"));
        // g1.k binding overridden to 3.
        let g1k = m.components.iter().find(|c| c.name == "g1.k").unwrap();
        assert_eq!(g1k.binding, Some(crate::ast::Expr::Number(3.0)));
    }

    #[test]
    fn extends_merges_base_with_modifiers() {
        let m = parse_model(
            "model Base parameter Real k = 1; Real y; equation y = k * time; end Base;             model Top extends Base(k = 5); end Top;",
        )
        .unwrap();
        let k = m.components.iter().find(|c| c.name == "k").unwrap();
        assert_eq!(k.binding, Some(crate::ast::Expr::Number(5.0)));
        assert!(m.components.iter().any(|c| c.name == "y"));
    }

    #[test]
    fn flatten_error_paths() {
        // Unknown component type.
        assert!(parse_model("model M Widget w; end M;")
            .unwrap_err()
            .to_string()
            .contains("unknown type"));
        // Unknown base class.
        assert!(parse_model("model M extends Missing; end M;")
            .unwrap_err()
            .to_string()
            .contains("unknown base class"));
        // connect of non-connectors.
        assert!(parse_model(
            "model A Real x; equation x = 1; end A;             model M A a; A b; equation connect(a, b); end M;"
        )
        .unwrap_err()
        .to_string()
        .contains("connector instances"));
        // Recursive instantiation.
        assert!(parse_model("model M M m; end M;")
            .unwrap_err()
            .to_string()
            .contains("recursive"));
        // Connectors whose members do not line up.
        assert!(parse_model(
            "connector A Real v; flow Real i; end A;             connector B Real v; flow Real q; end B;             model U A p; end U; model W B p; end W;             model M U u; W w; equation connect(u.p, w.p); end M;"
        )
        .unwrap_err()
        .to_string()
        .contains("different members"));
        // Two names for the same shape connect happily: a signal
        // output and a signal input are exactly that case.
        parse_model(
            "connector Out output Real y; end Out; connector In input Real y; end In; \
             model U Out p; equation p.y = 1; end U; model W In p; end W; \
             model M U u; W w; equation connect(u.p, w.p); end M;",
        )
        .unwrap();
        // A file with no model class.
        assert!(parse_model("connector Pin Real v; flow Real i; end Pin;")
            .unwrap_err()
            .to_string()
            .contains("no model class"));
    }

    #[test]
    fn arrays_expand_into_scalars_and_loops_unroll() {
        let m = parse_model(
            "model A parameter Integer n = 3; Real v[n]; Real s; \
             equation for i in 1:n loop v[i] = i * time; end for; s = v[2]; end A;",
        )
        .unwrap();
        let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"v[1]") && names.contains(&"v[3]"));
        assert!(!names.contains(&"v"), "the array itself must not survive");
        // Three unrolled loop equations plus the scalar one.
        assert_eq!(m.equations.len(), 4);
        // Subscripts became plain references.
        assert!(m
            .equations
            .iter()
            .all(|e| !matches!(e.lhs, crate::ast::Expr::Index(_, _))));

        // Two-dimensional arrays expand in row-major order.
        let grid = parse_model(
            "model G Real a[2, 3]; equation for i in 1:2 loop for j in 1:3 loop \
             a[i, j] = i + j; end for; end for; end G;",
        )
        .unwrap();
        let names: Vec<&str> = grid.components.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["a[1,1]", "a[1,2]", "a[1,3]", "a[2,1]", "a[2,2]", "a[2,3]"]
        );
    }

    #[test]
    fn records_and_component_arrays_expand() {
        let m = parse_model(
            "record P Real x; Real y; end P;\
             model M P points[2]; equation for i in 1:2 loop \
             points[i].x = i * time; points[i].y = 0; end for; end M;",
        )
        .unwrap();
        let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["points[1].x", "points[1].y", "points[2].x", "points[2].y"]
        );
    }

    #[test]
    fn functions_are_inlined_at_the_call_site() {
        let m = parse_model(
            "function scale input Real a; input Real b; output Real c; \
             algorithm c := a * b; end scale;\
             model M Real y; equation y = scale(time, 3); end M;",
        )
        .unwrap();
        // The call is gone: what remains is the substituted body.
        let mut refs = Vec::new();
        m.equations[0].rhs.collect_refs(&mut refs);
        assert!(refs.is_empty(), "unexpected references {refs:?}");
        assert!(!format!("{:?}", m.equations[0].rhs).contains("Call"));

        // A body of several assignments folds into one expression.
        let chained = parse_model(
            "function poly input Real x; output Real y; \
             algorithm y := x * x; y := y + x; end poly;\
             model M Real z; equation z = poly(3); end M;",
        )
        .unwrap();
        assert!(!format!("{:?}", chained.equations[0].rhs).contains("Call"));
    }

    #[test]
    fn array_and_function_error_paths() {
        let err = |source: &str| parse_model(source).unwrap_err().to_string();
        // A dimension that is not a compile-time constant.
        assert!(err("model M Real x; Real v[x]; equation x = 1; end M;")
            .contains("not a compile-time constant"));
        // A dimension of zero is an empty array, not a mistake; a
        // negative one still is.
        assert!(parse_model("model M Real v[0]; end M;")
            .unwrap()
            .components
            .is_empty());
        assert!(err("model M Real v[-1]; end M;").contains("not negative"));
        // A subscript that cannot be folded.
        assert!(
            err("model M Real v[2]; Real k; equation k = 1; v[1] = 0; v[2] = v[k]; end M;")
                .contains("compile-time constant")
        );
        // A subscript out of range names the bound it broke.
        assert!(
            err("model M Real v[2]; equation v[1] = 0; v[2] = v[0]; end M;")
                .contains("outside an array of 2")
        );
        // A loop bound that is not constant.
        assert!(
            err("model M Real x; equation x = 1; for i in 1:x loop x = i; end for; end M;")
                .contains("not a compile-time constant")
        );
        // Functions: wrong arity, missing output, output never assigned.
        assert!(err(
            "function f input Real a; output Real b; algorithm b := a; end f;\
             model M Real y; equation y = f(1, 2); end M;"
        )
        .contains("expects 1 argument"));
        assert!(err("function f input Real a; algorithm a := 1; end f;\
             model M Real y; equation y = f(1); end M;")
        .contains("declares no output"));
        // Every output must be assigned, even one the caller ignores.
        assert!(err(
            "function f input Real a; output Real b; output Real c; algorithm b := a; end f;\
             model M Real y; equation y = f(1); end M;"
        )
        .contains("never assigns its output `c`"));
    }

    #[test]
    fn a_function_fills_a_tuple_of_targets() {
        const TWO: &str = "function two input Real a; output Real b; output Real c; \
             algorithm b := a + 1; c := a + 2; end two;";

        // Both outputs of one call, in one equation each.
        let m = parse_model(&format!(
            "{TWO} model M Real p; Real q; equation (p, q) = two(3); end M;"
        ))
        .unwrap();
        assert_eq!(m.equations.len(), 2);
        let text = format!("{:?}", m.equations);
        assert!(
            !text.contains("Call(\"two\""),
            "the call must inline: {text}"
        );

        // A skipped slot drops that output on the floor.
        let m = parse_model(&format!(
            "{TWO} model M Real q; equation (, q) = two(3); end M;"
        ))
        .unwrap();
        assert_eq!(m.equations.len(), 1);
        assert_eq!(format!("{:?}", m.equations[0].lhs), "Ref(\"q\")");

        // An expression context quietly takes the first output.
        let m = parse_model(&format!(
            "{TWO} model M Real y; equation y = two(3) * 10; end M;"
        ))
        .unwrap();
        let text = format!("{:?}", m.equations[0].rhs);
        assert!(text.contains("1.0"), "b = a + 1 is the value: {text}");
        assert!(!text.contains("2.0"), "c must not leak: {text}");

        // The same tuple inside an algorithm.
        let m = parse_model(&format!(
            "{TWO} model M Real p; Real q; algorithm (p, q) := two(3); end M;"
        ))
        .unwrap();
        assert_eq!(m.equations.len(), 2);

        // A parenthesised left side is still an ordinary equation.
        let m =
            parse_model("model M Real x; Real y; equation (x) = 2 * y; y = time; end M;").unwrap();
        assert_eq!(m.equations.len(), 2);
    }

    #[test]
    fn named_arguments_and_defaults_fill_the_inputs() {
        const LINE: &str = "function line input Real x; input Real k = 2; input Real b = 10; \
             output Real y; algorithm y := k * x + b; end line;";

        // A named argument out of order; the untouched input defaults.
        let m = parse_model(&format!(
            "{LINE} model M Real y; equation y = line(5, b = 1); end M;"
        ))
        .unwrap();
        let text = format!("{:?}", m.equations[0].rhs);
        assert!(text.contains("2.0") && text.contains("5.0") && text.contains("1.0"));
        assert!(
            !text.contains("10.0"),
            "the default for b must lose: {text}"
        );

        // A default may lean on an earlier input.
        let m = parse_model(
            "function f input Real a; input Real half = a / 2; output Real y; \
             algorithm y := half; end f; \
             model M Real y; equation y = f(8); end M;",
        )
        .unwrap();
        assert!(format!("{:?}", m.equations[0].rhs).contains("8.0"));

        // The whole family of mistakes, each named.
        let err = |source: &str| parse_model(source).unwrap_err().to_string();
        let bad = |call: &str| {
            err(&format!(
                "{LINE} model M Real y; equation y = {call}; end M;"
            ))
        };
        assert!(bad("line(5, q = 1)").contains("no input named"));
        assert!(bad("line(5, x = 1)").contains("given twice"));
        assert!(bad("line(k = 2, 5)").contains("positional arguments must come before"));
        assert!(bad("line()").contains("missing its argument `x`"));
        assert!(err("model M Real y; equation y = sin(x = 1); end M;")
            .contains("cannot take named arguments"));
        assert!(err("model M Real p; Real q; equation (p, q) = 5; end M;")
            .contains("must be a function call"));
        assert!(err("model M Real p; Real q; algorithm (p, q) := 5; end M;")
            .contains("must be a function call"));
        assert!(err(
            "function f input Real a; output Real b; algorithm b := a; end f;\
             model M Real p; Real q; equation (p, q) = f(1); end M;"
        )
        .contains("1 output(s) for 2 target(s)"));
    }

    #[test]
    fn while_break_and_return_run_at_compile_time() {
        // Euclid's algorithm: a `while` folding its state each round.
        let m = parse_model(
            "function gcd input Real a; input Real b; output Real g; \
             protected Real x; Real y; Real t; \
             algorithm x := a; y := b; \
             while y > 0.5 loop t := y; y := mod(x, y); x := t; end while; \
             g := x; end gcd;\
             model M Real r; equation r = gcd(48, 18); end M;",
        )
        .unwrap();
        assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(6.0)");

        // Newton's square root converges onto the analytic one.
        let m = parse_model(
            "function newton_sqrt input Real a; output Real r; \
             algorithm r := a; \
             while abs(r * r - a) > 1e-12 loop r := 0.5 * (r + a / r); end while; \
             end newton_sqrt;\
             model M Real y; equation y = newton_sqrt(2); end M;",
        )
        .unwrap();
        let Expr::Number(value) = &m.equations[0].rhs else {
            panic!("the loop must fold to a number: {:?}", m.equations[0].rhs);
        };
        assert!((value - 2.0f64.sqrt()).abs() < 1e-9, "{value}");

        // `break` ends a search as soon as it succeeds.
        let m = parse_model(
            "function first_square_above input Real limit; output Real k; \
             algorithm k := 0; \
             for i in 1:100 loop if i * i > limit then k := i; break; end if; end for; \
             end first_square_above;\
             model M Real y; equation y = first_square_above(20); end M;",
        )
        .unwrap();
        assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(5.0)");

        // `return` leaves early on one path and not the other.
        let m = parse_model(
            "function clipped input Real u; output Real y; \
             algorithm y := u; if u > 1 then y := 1; return; end if; y := y * 2; \
             end clipped;\
             model M Real a; Real b; equation a = clipped(3); b = clipped(0.25); end M;",
        )
        .unwrap();
        assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(1.0)");
        assert!(format!("{:?}", m.equations[1].rhs).contains("0.25"));

        // `break` inside a `while`, behind a decided `if`.
        let m = parse_model(
            "function capped output Real r; protected Real i; \
             algorithm i := 0; r := 0; \
             while 1 > 0 loop i := i + 1; \
             if i > 4.5 then break; end if; r := r + i; end while; \
             end capped;\
             model M Real y; equation y = capped(); end M;",
        )
        .unwrap();
        assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(10.0)");

        // `return` rides out of a `for` loop.
        let m = parse_model(
            "function findfirst input Real limit; output Real k; \
             algorithm k := 0; \
             for i in 1:100 loop if i * i > limit then k := i; return; end if; end for; \
             end findfirst;\
             model M Real y; equation y = findfirst(20); end M;",
        )
        .unwrap();
        assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(5.0)");

        // A tuple equation and a named argument inside a component:
        // both go through the prefixing walker.
        let m = parse_model(
            "function two input Real a; output Real b; output Real c; \
             algorithm b := a + 1; c := a * 2; end two;\
             model Sub parameter Real k = 3; Real p; Real q; \
             equation (p, q) = two(a = k); end Sub;\
             model M Sub s; Real y; equation y = s.p + s.q; end M;",
        )
        .unwrap();
        let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"s.p") && names.contains(&"s.q"));
        let text = format!("{:?}", m.equations);
        assert!(
            text.contains("Ref(\"s.k\")"),
            "the argument must prefix: {text}"
        );
    }

    #[test]
    fn a_function_measures_the_array_it_is_handed() {
        const FUNCTIONS: &str = "function total input Real v[:]; output Real s; \
             algorithm s := 0; for i in 1:size(v, 1) loop s := s + v[i]; end for; end total;\
             function scaled input Real v[:]; input Real k; output Real w[size(v, 1)]; \
             algorithm for i in 1:size(v, 1) loop w[i] := k * v[i]; end for; end scaled;";

        // One function, two lengths, in the same model.
        let m = parse_model(&format!(
            "{FUNCTIONS} model M parameter Real a[3] = {{1, 2, 3}}; \
             parameter Real b[5] = {{1, 1, 1, 1, 1}}; Real p; Real q; \
             equation p = total(a); q = total(b); end M;"
        ))
        .unwrap();
        let rhs = |index: usize| format!("{:?}", m.equations[index].rhs);
        assert!(
            rhs(0).contains("Number(6.0)") || rhs(0).contains("a[3]"),
            "{}",
            rhs(0)
        );

        // The result takes the shape of the argument, so a whole-array
        // equation against it balances.
        let m = parse_model(&format!(
            "{FUNCTIONS} model M parameter Real a[3] = {{1, 2, 3}}; Real w[3]; \
             equation w = scaled(a, 2); end M;"
        ))
        .unwrap();
        assert_eq!(m.equations.len(), 3);

        // An empty array is a value like any other: a declaration of
        // length zero contributes nothing and its sum is zero.
        let m = parse_model(
            "model M parameter Integer n = 0; parameter Real nothing[n] = zeros(n); \
             Real s; Real t; equation s = sum(nothing); t = sum({}); end M;",
        )
        .unwrap();
        assert!(m.components.iter().all(|c| !c.name.starts_with("nothing[")));
        assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(0.0)");
        assert_eq!(format!("{:?}", m.equations[1].rhs), "Number(0.0)");
    }

    #[test]
    fn a_loop_inside_a_component_counts_in_its_own_terms() {
        // Prefixing reaches into subscripts, so the loop variable is
        // folded to a number before it can be mistaken for a component
        // of the instance. The bound and the guards read the
        // component's own parameters, under whatever path it sits at.
        let m = parse_model(
            "model Sub parameter Integer n = 3; Real x[n]; \
             equation for i in 1:n loop \
             x[i] = (if i > 1 then x[i - 1] else 0) + i; end for; end Sub;\
             model M Sub a(n = 3); end M;",
        )
        .unwrap();
        let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a.n", "a.x[1]", "a.x[2]", "a.x[3]"]);
        assert_eq!(m.equations.len(), 3);
        // The guard at the first element left no reference to `x[0]`.
        let text = format!("{:?}", m.equations);
        assert!(!text.contains("x[0]"), "{text}");
        assert!(!text.contains("Ref(\"a.i\")"), "{text}");
    }

    #[test]
    fn a_run_time_if_equation_keeps_every_branch() {
        // Two equations per branch, kept side by side for the
        // compiler to choose between.
        let m = parse_model(
            "model M Real gate; Real a; Real b; equation gate = time; \
             if gate > 1 then a = 1; b = 2; else a = 3; b = 4; end if; end M;",
        )
        .unwrap();
        assert_eq!(m.equations.len(), 1);
        let conditional = &m.conditional[0];
        assert_eq!(conditional.branches.len(), 2);
        assert!(conditional.branches.iter().all(|branch| branch.len() == 2));
        assert_eq!(
            format!("{:?}", conditional.branches[1][1].rhs),
            "Number(4.0)"
        );

        // An `elseif` chain keeps a condition for every branch but the
        // last, which is the `else`.
        let m = parse_model(
            "model M Real gate; Real y; equation gate = time; \
             if gate > 2 then y = 1; elseif gate > 1 then y = 2; else y = 3; end if; end M;",
        )
        .unwrap();
        assert_eq!(m.conditional[0].conditions.len(), 2);
        assert_eq!(m.conditional[0].branches.len(), 3);

        // Whole-array equations count by their scalars, not by the
        // lines they were written on.
        let m = parse_model(
            "model M Real gate; Real v[2]; equation gate = time; \
             if gate > 1 then v = {1, 2}; else v[1] = 3; v[2] = 4; end if; end M;",
        )
        .unwrap();
        assert!(m.conditional[0].branches.iter().all(|b| b.len() == 2));

        // Each branch is checked as it was written, so a mistake
        // inside one is still caught.
        let error = parse_model(
            "model M Real gate; Boolean flag; Real y; equation gate = time; flag = true; \
             if gate > 1 then y = flag; else y = 3; end if; end M;",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("type mismatch"), "{error}");

        // A branch may equate a volt to a volt and the other an
        // ampere to an ampere: they are separate equations, and only
        // the merge puts them in one slot.
        parse_model(
            "model M Real gate; Real v(unit = \"V\"); Real i(unit = \"A\"); \
             equation gate = time; \
             if gate > 1 then v = 1; i = 0; else v = 0; i = 1; end if; end M;",
        )
        .unwrap();
    }

    #[test]
    fn run_time_if_equation_error_paths() {
        let err = |source: &str| parse_model(source).unwrap_err().to_string();
        // Unbalanced branches.
        assert!(
            err("model M Real gate; Real a; Real b; equation gate = time; \
             if gate > 1 then a = 1; b = 2; else a = 3; end if; b = 0; end M;")
            .contains("not balanced")
        );
        // No `else`, so the equation count would depend on the run.
        assert!(err("model M Real gate; Real a; equation gate = time; \
             if gate > 1 then a = 1; end if; end M;")
        .contains("no `else`"));
        // A connection cannot be drawn conditionally at run time.
        assert!(err("connector Pin Real v; flow Real i; end Pin; \
             model U Pin p; end U; \
             model M Real gate; U a; U b; equation gate = time; \
             if gate > 1 then connect(a.p, b.p); else connect(a.p, b.p); end if; end M;")
        .contains("connections are structural"));
    }

    #[test]
    fn an_expandable_connector_holds_what_is_connected_to_it() {
        const SIGNALS: &str = "connector Out output Real y; end Out; \
             connector In input Real y; end In; \
             expandable connector Bus end Bus;";

        // The member exists because a connection named it, and it
        // takes the type of the other side.
        let m = parse_model(&format!(
            "{SIGNALS} model Src Out port; equation port.y = 5; end Src;\
             model Snk In port; end Snk;\
             model M Bus bus; Src src; Snk snk; \
             equation connect(src.port, bus.speed); connect(bus.speed, snk.port); end M;"
        ))
        .unwrap();
        let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"bus.speed.y"),
            "the bus member must exist: {names:?}"
        );
        // Source, bus and sink all carry the same signal.
        let text = format!("{:?}", m.equations);
        assert!(text.contains("bus.speed.y"), "{text}");

        // Joined buses share one pool: the sub-bus gets the member
        // too, and the two are connected.
        let m = parse_model(&format!(
            "{SIGNALS} model Src Out port; equation port.y = 5; end Src;\
             model Snk In port; end Snk;\
             model M Bus bus; Bus sub; Src src; Snk snk; \
             equation connect(bus, sub); connect(src.port, bus.speed); \
             connect(sub.speed, snk.port); end M;"
        ))
        .unwrap();
        let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"bus.speed.y") && names.contains(&"sub.speed.y"),
            "both buses carry the member: {names:?}"
        );
        // Everything ends up equal to the source, through both buses.
        let text = format!("{:?}", m.equations);
        assert!(text.contains("sub.speed.y"), "{text}");

        // A bus nobody writes to is simply empty.
        let m = parse_model(&format!("{SIGNALS} model M Bus bus; end M;")).unwrap();
        assert!(m.components.is_empty(), "{:?}", m.components);
    }

    #[test]
    fn expandable_connector_error_paths() {
        let err = |source: &str| parse_model(source).unwrap_err().to_string();
        // Two bus members connected to each other and to nothing else:
        // there is no side to take a type from.
        assert!(err("expandable connector Bus end Bus; \
             model M Bus a; Bus b; equation connect(a.speed, b.rate); end M;")
        .contains("not a connector"));
    }

    #[test]
    fn streams_mix_by_their_connection_set() {
        const PORT: &str = "connector Port Real p; flow Real m; stream Real h; end Port;";

        // Unconnected: a port hears its own outflow back.
        let m = parse_model(&format!(
            "{PORT} model M Port port; Real y; \
             equation port.h = 7; y = inStream(port.h); end M;"
        ))
        .unwrap();
        let text = format!("{:?}", m.equations);
        assert!(
            text.contains("Ref(\"y\"), rhs: Ref(\"port.h\")"),
            "an unconnected inStream is the own outflow: {text}"
        );

        // Two on a node: each hears exactly the other.
        let m = parse_model(&format!(
            "{PORT} model A Port port; Real y; \
             equation port.h = 1; port.m = 0; y = inStream(port.h); end A;\
             model B Port port; equation port.h = 2; end B;\
             model M A a; B b; equation connect(a.port, b.port); end M;"
        ))
        .unwrap();
        let text = format!("{:?}", m.equations);
        assert!(
            text.contains("Ref(\"a.y\"), rhs: Ref(\"b.port.h\")"),
            "a pair hears each other: {text}"
        );

        // Three on a node: the flow-weighted mix of the others.
        let m = parse_model(&format!(
            "{PORT} model E Port port; Real y; \
             equation port.h = 1; port.m = 0; y = inStream(port.h); end E;\
             model M E e1; E e2; E e3; \
             equation connect(e1.port, e2.port); connect(e2.port, e3.port); end M;"
        ))
        .unwrap();
        let text = format!("{:?}", m.equations);
        assert!(
            text.contains("Call(\"max\""),
            "a junction needs weights: {text}"
        );
        // Each mix reads the two other ports, never its own.
        assert!(
            !text.contains("Ref(\"e1.y\"), rhs: Ref(\"e1.port.h\")"),
            "a junction mix is not an echo: {text}"
        );

        // The connection itself writes no equation for a stream
        // variable: the two outflow definitions above are the only
        // ones naming them on the left.
        let stream_lhs = m
            .equations
            .iter()
            .filter(|eq| format!("{:?}", eq.lhs).contains(".port.h"))
            .count();
        assert_eq!(stream_lhs, 3, "one outflow definition per component");
    }

    #[test]
    fn streams_reach_conditions_whens_asserts_and_initials() {
        const PORT: &str = "connector Port Real p; flow Real m; stream Real h; end Port;";
        let m = parse_model(&format!(
            "{PORT} model M Port port; Real y; discrete Real d(start = 0); \
             Real z(start = 1, fixed = false); \
             equation port.h = 7; port.m = 0; der(z) = -z; \
             y = if time > 0.5 then inStream(port.h) else -inStream(port.h); \
             when time > 1 then d = inStream(port.h); end when; \
             assert(not (inStream(port.h) < 0) or inStream(port.h) > 100, \"mixed\"); \
             initial equation z = inStream(port.h); end M;"
        ))
        .unwrap();
        // Every corner rewrote its call away.
        let everything = format!(
            "{:?} {:?} {:?} {:?}",
            m.equations, m.initial_equations, m.when_clauses, m.asserts
        );
        assert!(
            !everything.contains("inStream"),
            "a call survived: {everything}"
        );
    }

    #[test]
    fn stream_error_paths() {
        let err = |source: &str| parse_model(source).unwrap_err().to_string();
        const PORT: &str = "connector Port Real p; flow Real m; stream Real h; end Port;";
        // The argument must be a single reference to a stream member.
        assert!(err(&format!(
            "{PORT} model M Port port; Real y; \
             equation port.h = 1; y = inStream(1 + 2); end M;"
        ))
        .contains("single reference"));
        assert!(err(&format!(
            "{PORT} model M Port port; Real x; Real y; \
             equation port.h = 1; x = 1; y = inStream(x); end M;"
        ))
        .contains("stream variable"));
        assert!(err(&format!(
            "{PORT} model M Port port; Real y; \
             equation port.h = 1; y = inStream(port.nope); end M;"
        ))
        .contains("no member"));
        // A stream connector must carry exactly one flow variable.
        assert!(err("connector Port Real p; stream Real h; end Port; \
             model M Port port; equation port.h = 1; end M;")
        .contains("exactly one flow variable"));
        // `inStream` of something that is not a stream variable.
        assert!(err(
            "connector Port Real p; flow Real m; stream Real h; end Port; \
             model M Port port; Real y; \
             equation port.h = 1; y = inStream(port.p); end M;"
        )
        .contains("not a stream variable"));
        // `inStream` of something that is not a connector member.
        assert!(err(
            "connector Port Real p; flow Real m; stream Real h; end Port; \
             model Sub Real x; end Sub; \
             model M Port port; Sub sub; Real y; \
             equation port.h = 1; sub.x = 1; y = inStream(sub.x); end M;"
        )
        .contains("is not a connector"));
    }

    #[test]
    fn the_compile_time_folder_knows_the_numeric_builtins() {
        // One `while` round folds every builtin the folder knows; the
        // pi-flavoured results sum to exactly pi.
        let m = parse_model(
            "function burst output Real y; protected Real go; \
             algorithm go := 1; \
             while go > 0 loop \
             y := abs(-2) + sqrt(4) + exp(0) + log(1) + log10(10) \
                + sin(0) + cos(0) + tan(0) + asin(1) + acos(1) + atan(1) \
                + sinh(0) + cosh(0) + tanh(0) \
                + floor(1.5) + ceil(1.5) + integer(2.7) \
                + atan2(1, 1) + min(1, 2) + max(1, 2) \
                + div(7, 2) + mod(7, 4) + rem(7, 4); \
             go := 0; \
             end while; end burst;\
             model M Real y; equation y = burst(); end M;",
        )
        .unwrap();
        let Expr::Number(value) = &m.equations[0].rhs else {
            panic!("the burst must fold: {:?}", m.equations[0].rhs);
        };
        let expected = 25.0 + std::f64::consts::PI;
        assert!((value - expected).abs() < 1e-12, "{value} vs {expected}");
    }

    #[test]
    fn flow_control_error_paths() {
        let err = |source: &str| parse_model(source).unwrap_err().to_string();
        // A loop that can never finish hits the backstop.
        assert!(err("model M Real y; \
             algorithm y := 0; while y < 1e9 loop y := y + 1; end while; end M;")
        .contains("did not finish"));
        // A `break` guarded by a condition the compiler cannot decide.
        assert!(
            err("function f input Real u; output Real y; algorithm y := 0; \
             for i in 1:3 loop if u > 0 then break; end if; y := y + 1; end for; end f;\
             model M Real u; Real y; equation u = time; y = f(u); end M;")
            .contains("compiler can decide")
        );
        // `return` is for functions, `break` for loops.
        assert!(err("model M Real y; algorithm y := 1; return; end M;")
            .contains("belongs in a function"));
        assert!(err(
            "function f input Real a; output Real b; algorithm b := a; break; end f;\
             model M Real y; equation y = f(1); end M;"
        )
        .contains("`break` outside of a loop in function"));
        assert!(err("model M Real y; algorithm y := 1; break; end M;")
            .contains("`break` outside of a loop"));
    }

    #[test]
    fn member_access_and_nested_loops() {
        // A record array with a nested loop over two dimensions.
        let m = parse_model(
            "record P Real x; Real y; end P;\
             model M P g[2]; Real s[2, 2]; equation \
             for i in 1:2 loop g[i].x = i; g[i].y = 0; \
             for j in 1:2 loop s[i, j] = i * j; end for; end for; end M;",
        )
        .unwrap();
        assert_eq!(m.equations.len(), 8);
        let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"g[2].y") && names.contains(&"s[2,2]"));
        // Member access on something that is not a component is refused.
        assert!(
            parse_model("model M Real v[2]; equation v[1] = 0; v[2] = (v[1] + 1).x; end M;")
                .is_err()
        );
    }

    #[test]
    fn functions_reach_through_expressions_and_defaults() {
        // Local variables with bindings act as defaults inside a body.
        let m = parse_model(
            "function offset input Real a; output Real b; Real bias = 10; \
             algorithm b := a + bias; end offset;\
             model M Real y; equation y = offset(5) + offset(1); end M;",
        )
        .unwrap();
        let text = format!("{:?}", m.equations[0].rhs);
        assert!(!text.contains("Call"), "calls survived: {text}");

        // Nested calls inline from the inside out.
        let nested = parse_model(
            "function twice input Real a; output Real b; algorithm b := 2 * a; end twice;\
             model M Real y; equation y = twice(twice(3)); end M;",
        )
        .unwrap();
        assert!(!format!("{:?}", nested.equations[0].rhs).contains("Call"));

        // Built-ins are not shadowed by the registry lookup.
        let builtin = parse_model("model M Real y; equation y = sin(time); end M;").unwrap();
        assert!(format!("{:?}", builtin.equations[0].rhs).contains("Call(\"sin\""));
    }

    #[test]
    fn classes_nested_inside_a_model_are_visible_to_it() {
        // A connector and a function declared in the model itself, not
        // in an enclosing package: resolution starts at the class doing
        // the looking, so both are found.
        let m = parse_model(
            "model Bus                connector Pin Real v; flow Real i; end Pin;                function double input Real a; output Real b;                algorithm b := 2 * a; end double;                Pin left; Pin right; Real y;              equation left.v = 3; right.i = 0.5;                connect(left, right); y = double(left.v); end Bus;",
        )
        .unwrap();
        assert!(m.components.iter().any(|c| c.name == "left.v"));
        // The call inlined and the connection generated its equations.
        let text = format!("{:?}", m.equations);
        assert!(!text.contains("Call"), "{text}");
        assert!(text.contains("right.v"), "{text}");

        // The inner name wins over an outer one of the same spelling.
        let shadowed = parse_model(
            "package Kit model Gain parameter Real k = 100; Real u; Real y;              equation y = k * u; end Gain; end Kit;              model M                model Gain parameter Real k = 2; Real u; Real y;                equation y = k * u; end Gain;                Gain g; Real out;              equation g.u = 1; out = g.y; end M;",
        )
        .unwrap();
        let k = shadowed
            .components
            .iter()
            .find(|c| c.name == "g.k")
            .unwrap();
        assert_eq!(k.binding, Some(Expr::Number(2.0)));
    }

    #[test]
    fn packages_qualify_names_and_scoping_walks_outwards() {
        let m = parse_model(
            "package P \
               constant Real two = 2; \
               package Inner \
                 model Gain parameter Real k = two; Real u; Real y; \
                 equation y = k * u; end Gain; \
               end Inner; \
             end P; \
             model M P.Inner.Gain g; Real s; equation g.u = time; s = g.y; end M;",
        )
        .unwrap();
        // The nested class resolved, and `two` was found by walking out
        // of the enclosing packages.
        let k = m.components.iter().find(|c| c.name == "g.k").unwrap();
        assert!(k.binding.is_some());
        assert!(m.components.iter().any(|c| c.name == "g.y"));
    }

    #[test]
    fn imports_type_aliases_and_partial_classes() {
        let m = parse_model(
            "package Lib \
               type Voltage = Real(unit = \"V\", start = 7); \
               partial model Base Real x; end Base; \
               model Source extends Base; Voltage v; \
               equation x = time; v = 2 * x; end Source; \
             end Lib; \
             model M import Lib.Source; Source s; end M;",
        )
        .unwrap();
        // The alias contributed its start attribute; the unit string was
        // ignored.
        let v = m.components.iter().find(|c| c.name == "s.v").unwrap();
        assert_eq!(v.start, Some(crate::ast::Expr::Number(7.0)));

        // A partial class may be extended but not instantiated.
        let error = parse_model(
            "partial model Base Real x; end Base; \
             model M Base b; end M;",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("partial"), "{error}");

        // Packages are not component types either.
        let error = parse_model(
            "package P model Q Real x; equation x = 1; end Q; end P; \
             model M P p; end M;",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("package"), "{error}");
    }

    #[test]
    fn msl_style_syntax_is_accepted() {
        // A component written the way the Modelica Standard Library
        // writes them: a `within` header, dotted names, an attribute
        // modifier, assert(), noEvent() and a graphical annotation full
        // of braces.
        let m = parse_model(
            "within Modelica.Electrical.Analog.Basic; \
             model Resistor \"Ideal linear electrical resistor\" \
               parameter Real R(start = 1) \"Resistance\"; \
               Real v; Real i; \
             equation \
               assert(R > 0, \"Resistance must be positive\"); \
               v = noEvent(R * i); \
               i = smooth(0, time); \
               annotation (Icon(coordinateSystem(extent = {{-100, -100}, {100, 100}}), \
                 graphics = {Rectangle(extent = {{-70, 30}, {70, -30}})})); \
             end Resistor;",
        )
        .unwrap();
        assert_eq!(m.name, "Resistor");
        assert_eq!(
            m.equations.len(),
            2,
            "assert is skipped, two equations remain"
        );
        // noEvent and smooth collapse to their value argument.
        assert!(!format!("{:?}", m.equations).contains("noEvent"));
        assert!(!format!("{:?}", m.equations).contains("smooth"));
    }

    #[test]
    fn class_info_reports_ports_and_parameters_including_inherited() {
        use super::class_info;
        let classes = crate::parser::parse_file(
            "package Lib \
               connector Pin Real v; flow Real i; end Pin; \
               partial model OnePort Pin p; Pin n; Real v; Real i; \
               equation v = p.v - n.v; p.i = i; n.i = -i; end OnePort; \
               model Resistor extends OnePort; parameter Real R = 1; \
               equation v = R * i; end Resistor; \
               model Gain parameter Real k = 1; Real u; Real y; \
               equation y = k * u; end Gain; \
             end Lib;",
        )
        .unwrap();

        // Ports and parameters come from the base as well as the class.
        let resistor = class_info(&classes, "Lib.Resistor").unwrap();
        assert_eq!(resistor.ports, vec!["p", "n"]);
        assert_eq!(resistor.parameters.len(), 1);
        assert_eq!(resistor.parameters[0].0, "R");
        assert!(resistor.instantiable);

        // A partial base is described but not instantiable.
        let base = class_info(&classes, "Lib.OnePort").unwrap();
        assert_eq!(base.ports, vec!["p", "n"]);
        assert!(!base.instantiable);

        // A class without connectors has no ports.
        let gain = class_info(&classes, "Lib.Gain").unwrap();
        assert!(gain.ports.is_empty());
        assert!(gain.instantiable);

        // Connectors themselves are not instantiable as components.
        assert!(!class_info(&classes, "Lib.Pin").unwrap().instantiable);
        assert!(class_info(&classes, "Lib.Missing").is_none());
    }

    #[test]
    fn package_constants_are_substituted_by_value() {
        // A constant of a package, reached from a model and from inside
        // a library class - the latter is what makes a sine source work.
        let m = parse_model(
            "package Lib \
               constant Real two = 2; \
               constant Real four = 2 * two; \
               model Doubler Real y; equation y = Lib.four * time; end Doubler; \
             end Lib; \
             model M Lib.Doubler d; Real z; equation z = Lib.two * time; end M;",
        )
        .unwrap();
        let text = format!("{:?}", m.equations);
        // Both references became numbers, and the constant built on
        // another constant resolved too.
        assert!(text.contains("Number(4.0)"), "{text}");
        assert!(text.contains("Number(2.0)"), "{text}");
        assert!(!text.contains("Lib.two"), "a reference survived: {text}");

        // Dotted names that are not class constants keep their meaning:
        // this one is a connector variable of a component.
        let circuit = parse_model(
            "connector Pin Real v; flow Real i; end Pin; \
             model Probe Pin p; Real reading; equation reading = p.v; p.i = 0; end Probe; \
             model M Probe probe; equation probe.p.v = time; end M;",
        )
        .unwrap();
        assert!(format!("{:?}", circuit.equations).contains("probe.p.v"));
    }

    /// Sources that share the shape of a standard-library package: a
    /// replaceable component with an interface, a conditional one and a
    /// world shared through `inner`/`outer`.
    const LIB: &str = "package Lib \
           connector Pin Real v; flow Real i; end Pin; \
           partial model SISO Real u; Real y; end SISO; \
           model Gain extends SISO; parameter Real k = 1; equation y = k * u; end Gain; \
           model Doubler extends SISO; equation y = 2 * u; end Doubler; \
           model Loose Real y; equation y = 0; end Loose; \
           model World parameter Real g = 9.81; end World; \
           model Falling outer World world; Real a; equation a = -world.g; end Falling; \
         end Lib;";

    fn with_lib(source: &str) -> Result<crate::ast::Model, String> {
        let mut classes = crate::parser::parse_file(LIB).unwrap();
        classes.extend(crate::parser::parse_file(source).unwrap());
        let top = classes
            .iter()
            .rev()
            .find(|c| c.kind == crate::ast::ClassKind::Model && !c.partial)
            .unwrap()
            .name
            .clone();
        super::flatten(&classes, &top)
    }

    #[test]
    fn a_replaceable_package_swaps_constants_and_functions() {
        let media = "package Media              partial package PartialMedium constant Real rho = 0;                function f input Real x; output Real y; algorithm y := 0; end f;              end PartialMedium;              package Water extends PartialMedium; constant Real rho = 1000;                function f input Real x; output Real y; algorithm y := 2 * x; end f;              end Water;              package Oil extends PartialMedium; constant Real rho = 900;                function f input Real x; output Real y; algorithm y := 3 * x; end f;              end Oil;              package Rogue constant Real rho = 1; end Rogue;            end Media;            model Tank              replaceable package Medium = Media.Water constrainedby Media.PartialMedium;              Real a; Real b; equation a = Medium.rho; b = Medium.f(4); end Tank; ";

        // The default alias: water's constant and water's function.
        let plain = parse_model(&format!("{media} model M Tank tank; end M;")).unwrap();
        let text = format!("{:?}", plain.equations);
        assert!(text.contains("Number(1000.0)"), "{text}");
        // The function inlined with its own factor; folding constants
        // is the simulator's business, not the flattener's.
        assert!(text.contains("Number(2.0)"), "{text}");

        // Redeclared in an extends modifier: everything becomes oil.
        let swapped = parse_model(&format!(
            "{media} model OilTank extends Tank(redeclare package Medium = Media.Oil); end OilTank;"
        ))
        .unwrap();
        let text = format!("{:?}", swapped.equations);
        assert!(text.contains("Number(900.0)"), "{text}");
        assert!(text.contains("Number(3.0)"), "{text}");

        // Redeclared in a component's modifier list.
        let component = parse_model(&format!(
            "{media} model M Tank tank(redeclare package Medium = Media.Oil); end M;"
        ))
        .unwrap();
        assert!(format!("{:?}", component.equations).contains("Number(900.0)"));

        // Redeclared in the body of a derived class.
        let body = parse_model(&format!(
            "{media} model OilTank extends Tank; redeclare package Medium = Media.Oil; end OilTank;"
        ))
        .unwrap();
        assert!(format!("{:?}", body.equations).contains("Number(900.0)"));

        // A replacement outside the constraining interface is refused.
        let error = parse_model(&format!(
            "{media} model Bad extends Tank(redeclare package Medium = Media.Rogue); end Bad;"
        ))
        .unwrap_err()
        .to_string();
        assert!(error.contains("does not extend"), "{error}");

        // And so is replacing an alias never marked replaceable.
        let error = parse_model(&format!(
            "{media} model Fixed package Medium = Media.Water; Real a;              equation a = Medium.rho; end Fixed;              model Bad extends Fixed(redeclare package Medium = Media.Oil); end Bad;"
        ))
        .unwrap_err()
        .to_string();
        assert!(error.contains("not declared replaceable"), "{error}");
    }

    #[test]
    fn redeclare_replaces_the_type_of_a_replaceable_component() {
        // The base declares a Gain, the derived model swaps in a Doubler
        // and the equations follow the new type.
        let m = with_lib(
            "model Base replaceable Lib.Gain block1(k = 3) constrainedby Lib.SISO; \
               Real y; equation block1.u = time; y = block1.y; end Base; \
             model Derived extends Base(redeclare Lib.Doubler block1); end Derived;",
        )
        .unwrap();
        let text = format!("{:?}", m.equations);
        assert!(text.contains("Number(2.0)"), "{text}");
        // The Gain's parameter is gone with the Gain.
        assert!(!m.components.iter().any(|c| c.name == "block1.k"));

        // A redeclaration written in the body of the derived class does
        // the same thing.
        let in_body = with_lib(
            "model Base2 replaceable Lib.Gain block1 constrainedby Lib.SISO; \
               Real y; equation block1.u = time; y = block1.y; end Base2; \
             model Derived2 extends Base2; redeclare Lib.Doubler block1; end Derived2;",
        )
        .unwrap();
        assert!(format!("{:?}", in_body.equations).contains("Number(2.0)"));
    }

    #[test]
    fn redeclare_error_paths() {
        // Not replaceable.
        let error = with_lib(
            "model Base Lib.Gain block1; Real y; equation block1.u = time; y = block1.y; end Base; \
             model Derived extends Base(redeclare Lib.Doubler block1); end Derived;",
        )
        .unwrap_err();
        assert!(error.contains("not declared replaceable"), "{error}");

        // The replacement does not meet the constraining interface.
        let error = with_lib(
            "model Base replaceable Lib.Gain block1 constrainedby Lib.SISO; \
               Real y; equation block1.u = time; y = block1.y; end Base; \
             model Derived extends Base(redeclare Lib.Loose block1); end Derived;",
        )
        .unwrap_err();
        assert!(error.contains("does not extend"), "{error}");

        // An unknown type in the redeclaration.
        let error = with_lib(
            "model Base replaceable Lib.Gain block1 constrainedby Lib.SISO; \
               Real y; equation block1.u = time; y = block1.y; end Base; \
             model Derived extends Base(redeclare Lib.Missing block1); end Derived;",
        )
        .unwrap_err();
        assert!(error.contains("unknown type"), "{error}");
    }

    #[test]
    fn outer_components_reach_the_inner_instance() {
        let m = with_lib(
            "model Top inner Lib.World world(g = 2); Lib.Falling ball; \
             Real a; equation a = ball.a; end Top;",
        )
        .unwrap();
        // `world.g` inside the component resolved to the shared
        // instance, not to a variable of its own.
        assert!(format!("{:?}", m.equations).contains("world.g"));
        assert!(!m.components.iter().any(|c| c.name == "ball.world.g"));
        let g = m.components.iter().find(|c| c.name == "world.g").unwrap();
        assert_eq!(g.binding, Some(Expr::Number(2.0)));
    }

    #[test]
    fn outer_without_inner_is_refused() {
        let error = with_lib("model Top Lib.Falling ball; end Top;").unwrap_err();
        assert!(error.contains("no `inner` declaration"), "{error}");

        // An `outer` of a type the `inner` instance is not.
        let error =
            with_lib("model Top inner Lib.Gain world; Lib.Falling ball; end Top;").unwrap_err();
        assert!(
            error.contains("does not match the `inner` instance"),
            "{error}"
        );
    }

    #[test]
    fn a_false_condition_removes_a_component_and_its_connections() {
        let source = "connector Pin Real v; flow Real i; end Pin; \
             model Probe Pin p; Real reading; equation reading = p.v; p.i = 0; end Probe; \
             model Top parameter Boolean measure = false; \
               Probe probe if measure; Pin node; \
             equation node.v = time; connect(probe.p, node); end Top;";
        let m = crate::parser::parse_model(source).unwrap();
        assert!(
            !m.components.iter().any(|c| c.name.starts_with("probe")),
            "the component survived its condition"
        );
        // With the probe gone, the node is the only member of its set and
        // its flow is forced to zero rather than joined to a missing one.
        let text = format!("{:?}", m.equations);
        assert!(!text.contains("probe"), "{text}");
        assert!(text.contains("node.i"), "{text}");

        // The same model with the condition true keeps both.
        let kept = crate::parser::parse_model(&source.replace("measure = false", "measure = true"))
            .unwrap();
        assert!(kept.components.iter().any(|c| c.name == "probe.reading"));
        assert!(format!("{:?}", kept.equations).contains("probe.p.v"));
    }

    #[test]
    fn a_condition_must_be_constant() {
        let error = crate::parser::parse_model(
            "model Inner Real x; equation x = 1; end Inner; \
             model Top Real gate; Inner part if gate > 0; equation gate = time; end Top;",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("not a compile-time constant"), "{error}");
    }

    #[test]
    fn enumeration_literals_are_their_position() {
        let m = crate::parser::parse_model(
            "package Types type Kind = enumeration(First, Second \"the second one\", Third); \
             end Types; \
             model M parameter Types.Kind kind = Types.Kind.Second; Real y; \
             equation y = if kind == Types.Kind.Third then 30 else 20; end M;",
        )
        .unwrap();
        // The parameter is carried as an Integer holding the position.
        let kind = m.components.iter().find(|c| c.name == "kind").unwrap();
        assert_eq!(kind.type_name, "Integer");
        assert_eq!(kind.binding, Some(Expr::Number(2.0)));
        assert!(format!("{:?}", m.equations).contains("Number(3.0)"));
    }

    #[test]
    fn nested_modifiers_reach_children_and_attributes() {
        let m = crate::parser::parse_model(
            "model Leaf parameter Real k = 1; Real x(start = 0); \
             equation der(x) = k; end Leaf; \
             model Middle Leaf leaf; end Middle; \
             model Top Middle mid(leaf(k = 5, x(start = 7))); end Top;",
        )
        .unwrap();
        let k = m
            .components
            .iter()
            .find(|c| c.name == "mid.leaf.k")
            .unwrap();
        assert_eq!(k.binding, Some(Expr::Number(5.0)));
        let x = m
            .components
            .iter()
            .find(|c| c.name == "mid.leaf.x")
            .unwrap();
        assert_eq!(x.start, Some(Expr::Number(7.0)));

        // `fixed` travels the same way.
        let fixed = crate::parser::parse_model(
            "model Leaf Real x(start = 0); equation der(x) = 1; end Leaf; \
             model Top Leaf leaf(x(fixed = true)); end Top;",
        )
        .unwrap();
        assert_eq!(
            fixed
                .components
                .iter()
                .find(|c| c.name == "leaf.x")
                .unwrap()
                .fixed,
            Some(true)
        );
    }

    #[test]
    fn if_equations_pick_a_branch_at_compile_time() {
        let template = "model M parameter Boolean fast = SETTING; Real y; \
             equation if fast then y = 2 * time; else y = time / 2; end if; end M;";
        let fast = crate::parser::parse_model(&template.replace("SETTING", "true")).unwrap();
        let slow = crate::parser::parse_model(&template.replace("SETTING", "false")).unwrap();
        // Both models have exactly one equation: the other branch is gone.
        assert_eq!(fast.equations.len(), 1);
        assert_eq!(slow.equations.len(), 1);
        assert!(matches!(
            fast.equations[0].rhs,
            Expr::Bin(crate::ast::BinOp::Mul, _, _)
        ));
        assert!(matches!(
            slow.equations[0].rhs,
            Expr::Bin(crate::ast::BinOp::Div, _, _)
        ));

        // An elseif chain, and a chain where nothing holds.
        let chain = "model M parameter Integer mode = SETTING; Real y; equation \
             if mode == 1 then y = time; elseif mode == 2 then y = 2 * time; end if; \
             end M;";
        assert_eq!(
            crate::parser::parse_model(&chain.replace("SETTING", "2"))
                .unwrap()
                .equations
                .len(),
            1
        );
        assert!(crate::parser::parse_model(&chain.replace("SETTING", "3"))
            .unwrap()
            .equations
            .is_empty());

        // A condition the run decides keeps every branch instead, for
        // the compiler to settle where the run has got to.
        let m = crate::parser::parse_model(
            "model M Real gate; Real y; equation gate = time; \
             if gate > 0 then y = 1; else y = 2; end if; end M;",
        )
        .unwrap();
        assert_eq!(m.equations.len(), 1);
        assert_eq!(m.conditional.len(), 1);
        let conditional = &m.conditional[0];
        assert_eq!(conditional.conditions.len(), 1);
        assert_eq!(conditional.branches.len(), 2);
        assert_eq!(
            format!("{:?}", conditional.branches[0][0].rhs),
            "Number(1.0)"
        );
        assert_eq!(
            format!("{:?}", conditional.branches[1][0].rhs),
            "Number(2.0)"
        );
    }

    #[test]
    fn if_equations_can_hold_connections() {
        let source = "connector Pin Real v; flow Real i; end Pin; \
             model Top parameter Boolean joined = SETTING; Pin a; Pin b; \
             equation a.v = time; if joined then connect(a, b); end if; end Top;";
        let joined = crate::parser::parse_model(&source.replace("SETTING", "true")).unwrap();
        // Joined: one potential equality and one flow sum.
        let text = format!("{:?}", joined.equations);
        assert!(text.contains("b.v"), "{text}");
        let apart = crate::parser::parse_model(&source.replace("SETTING", "false")).unwrap();
        // Apart: each connector carries its own zero flow.
        let text = format!("{:?}", apart.equations);
        assert!(text.contains("a.i") && text.contains("b.i"), "{text}");
    }

    #[test]
    fn a_binding_on_a_variable_is_an_equation() {
        let m =
            crate::parser::parse_model("model M Real x; Real y = 2 * x; equation x = time; end M;")
                .unwrap();
        assert_eq!(m.equations.len(), 2);
        // The declaration equation survived as an equation, not as a
        // binding that the solver would ignore.
        assert!(m
            .components
            .iter()
            .find(|c| c.name == "y")
            .unwrap()
            .binding
            .is_none());
        assert!(format!("{:?}", m.equations).contains("Ref(\"y\")"));
    }

    #[test]
    fn chained_type_aliases_resolve_to_a_primitive() {
        let m = crate::parser::parse_model(
            "package SI type Angle = Real(unit = \"rad\", start = 3); end SI; \
             package Units type Turn = SI.Angle; end Units; \
             model M Units.Turn phi; equation der(phi) = 1; end M;",
        )
        .unwrap();
        let phi = m.components.iter().find(|c| c.name == "phi").unwrap();
        assert_eq!(phi.type_name, "Real");
        assert_eq!(phi.start, Some(Expr::Number(3.0)));
    }

    #[test]
    fn redeclarations_reach_through_a_nested_component() {
        // `mid(redeclare Doubler leaf)` names a component one level down.
        let m = with_lib(
            "model Middle replaceable Lib.Gain leaf(k = 3) constrainedby Lib.SISO(); \
               Real y; equation leaf.u = time; y = leaf.y; end Middle; \
             model Top Middle mid(redeclare Lib.Doubler leaf); Real z; \
               equation z = mid.y; end Top;",
        )
        .unwrap();
        assert!(format!("{:?}", m.equations).contains("Number(2.0)"));
        assert!(!m.components.iter().any(|c| c.name == "mid.leaf.k"));
    }

    #[test]
    fn structural_conditions_use_the_whole_boolean_language() {
        // Comparisons, `and`, `or` and `not` all fold at compile time.
        let m = crate::parser::parse_model(
            "model Part Real x; equation x = 1; end Part; \
             model M parameter Integer n = 3; parameter Boolean on = true; \
               Part a if n >= 3 and on; \
               Part b if n < 3 or not on; \
               Part c if n <> 3; \
             end M;",
        )
        .unwrap();
        let parts: Vec<&str> = m
            .components
            .iter()
            .map(|c| c.name.as_str())
            .filter(|name| name.ends_with(".x"))
            .collect();
        assert_eq!(parts, vec!["a.x"], "kept the wrong components: {parts:?}");
    }

    #[test]
    fn an_unknown_constraining_class_is_reported() {
        let error = with_lib(
            "model Base replaceable Lib.Gain block1 constrainedby Lib.Nothing; \
               Real y; equation block1.u = time; y = block1.y; end Base; \
             model Derived extends Base(redeclare Lib.Doubler block1); end Derived;",
        )
        .unwrap_err();
        assert!(error.contains("unknown constraining class"), "{error}");
    }

    #[test]
    fn an_alias_contributes_its_fixed_attribute() {
        let m = crate::parser::parse_model(
            "package Units type Held = Real(start = 2, fixed = true); end Units; \
             model M Units.Held x; equation der(x) = 1; end M;",
        )
        .unwrap();
        let x = m.components.iter().find(|c| c.name == "x").unwrap();
        assert_eq!(x.fixed, Some(true));
        assert_eq!(x.start, Some(Expr::Number(2.0)));
    }

    #[test]
    fn an_algorithm_section_of_a_model_becomes_equations() {
        let m = parse_model(
            "model M parameter Real limit = 1.5; Real u; Real y; Real gain; Real total; \
             equation u = 2 * time; \
             algorithm \
               gain := 1.0; \
               if u > limit then y := limit; gain := limit / u; \
               elseif u < -limit then y := -limit; gain := -limit / u; \
               else y := u; end if; \
               total := 0.0; \
               for i in 1:3 loop total := total + i * u; end for; \
             end M;",
        )
        .unwrap();
        // One equation per assigned variable, in the order the algorithm
        // writes them, plus the one from the equation section.
        let assigned: Vec<String> = m
            .equations
            .iter()
            .skip(1)
            .map(|e| format!("{:?}", e.lhs))
            .collect();
        assert_eq!(
            assigned,
            vec![
                "Ref(\"gain\")".to_string(),
                "Ref(\"y\")".to_string(),
                "Ref(\"total\")".to_string()
            ]
        );
        // The branch became one if-expression, and the loop unrolled
        // into 1*u + 2*u + 3*u rather than staying a loop.
        let gain = &m.equations[1].rhs;
        assert!(matches!(gain, Expr::If(_, _, _)), "{gain:?}");
        let total = format!("{:?}", m.equations[3].rhs);
        assert!(!total.contains("Ref(\"i\")"), "the loop variable survived");
        assert_eq!(total.matches("Ref(\"u\")").count(), 3);
    }

    #[test]
    fn algorithm_error_paths() {
        let err = |source: &str| parse_model(source).unwrap_err().to_string();
        // A variable written in one branch only, with nothing before it.
        assert!(err("model M Real u; Real y; equation u = time; \
             algorithm if u > 1 then y := 1; end if; end M;")
        .contains("assigned in one branch only"));
        // A loop whose bounds are not constant.
        assert!(err("model M Real u; Real y; equation u = time; \
             algorithm y := 0; for i in 1:u loop y := y + i; end for; end M;")
        .contains("not a compile-time constant"));
        // A `while` whose trip count depends on a simulated variable.
        assert!(err("model M Real u; Real y; equation u = time; \
             algorithm y := 0; while y < u loop y := y + 1; end while; end M;")
        .contains("decidable at compile time"));
        // An empty loop body.
        assert!(
            err("model M Real y; algorithm for i in 1:2 loop end for; end M;").contains("no body")
        );
    }

    #[test]
    fn arrays_are_values() {
        // A whole-array equation, a literal, the elementwise operators,
        // reductions, sizes and constructors - each expanded into the
        // scalars underneath at compile time.
        let m = parse_model(
            "model A parameter Real k[3] = {2, 4, 6}; Real v[3]; Real w[3];              Real total; Real dot;              equation v = {1, 2, 3}; w = 2 * v .* k;              total = sum(v) + size(v, 1) + max(k); dot = v * k; end A;",
        )
        .unwrap();
        let text = format!("{:?}", m.equations);
        // Three scalar equations came out of `w = 2 * v .* k`.
        assert_eq!(
            m.equations
                .iter()
                .filter(|e| format!("{:?}", e.lhs).contains("w["))
                .count(),
            3
        );
        // The scalar product is a sum of products, not a vector.
        assert!(text.contains("dot"), "{text}");

        // Constructors: fill, zeros, linspace; and an array start.
        let chain = parse_model(
            "model C parameter Integer n = 4;              parameter Real k[n] = fill(7.0, n);              parameter Real grid[n] = linspace(0.0, 3.0, n);              Real x[n](start = grid);              equation der(x) = zeros(n) .+ k; end C;",
        )
        .unwrap();
        let k2 = chain.components.iter().find(|c| c.name == "k[2]").unwrap();
        assert_eq!(k2.binding, Some(Expr::Number(7.0)));
        // Each element starts from its own element of the grid; the
        // number itself is the simulator's to look up.
        let x3 = chain.components.iter().find(|c| c.name == "x[3]").unwrap();
        assert_eq!(x3.start, Some(Expr::Ref("grid[3]".to_string())));
    }

    #[test]
    fn matrices_conditionals_and_the_remaining_array_corners() {
        // A matrix: a two-dimensional literal against a declared shape,
        // sized in both dimensions, summed and taken apart.
        let m = parse_model(
            "model M Real a[2, 3]; Real s; Real rows;              equation a = {{1, 2, 3}, {4, 5, 6}};              s = sum(a) + product({1, 2, 3}); rows = size(a, 1); end M;",
        )
        .unwrap();
        let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"a[2,3]"), "{names:?}");
        assert_eq!(m.equations.len(), 8, "six elements plus two scalars");

        // An if-expression over whole arrays, with a scalar condition.
        let picked = parse_model(
            "model P parameter Boolean top = true; Real v[2];              equation v = if top then {1, 2} else {3, 4}; end P;",
        )
        .unwrap();
        assert_eq!(picked.equations.len(), 2);

        // An elementwise op inside a function argument spreads the call.
        let spread = parse_model(
            "model S Real v[2]; Real w[2];              equation v = {0.1, 0.2}; w = sin(v ./ {2, 4}); end S;",
        )
        .unwrap();
        let text = format!("{:?}", spread.equations);
        assert_eq!(text.matches("Call(\"sin\"").count(), 2, "{text}");

        // `sum` folds the full array expression, `.+` broadcasts, and a
        // whole-array equation may live inside a for body.
        let looped = parse_model(
            "model L parameter Integer n = 2; Real g[n]; Real acc[n];              equation g = fill(3.0, n);              for i in 1:1 loop acc = g .+ 1; end for; end L;",
        )
        .unwrap();
        assert_eq!(looped.equations.len(), 4);
    }

    #[test]
    fn the_expression_walkers_cover_the_long_tail() {
        // One model that drives the rarely-taken arms: logic and
        // conditionals inside class-constant substitution, subscripted
        // references under prefixing, `size(v)` without a dimension,
        // and a boolean fold inside const_eval.
        let m = parse_model(
            "package P constant Real lim = 2; end P;              model W parameter Boolean wide = true;              parameter Real q = if wide and not (P.lim > 3) then 1 else 0;              Real v[2]; Real n[1]; Real s;              equation v = {if wide then P.lim else 0, P.lim};              n = size(v) .+ 0; s = v[1] + q; end W;",
        )
        .unwrap();
        let q = m.components.iter().find(|c| c.name == "q").unwrap();
        // The condition folded: wide and the package constant are known.
        assert!(format!("{:?}", q.binding).contains("If"), "{:?}", q.binding);
        assert_eq!(m.equations.len(), 4);

        // Functions with array-aware bodies still inline per element,
        // and substitution reaches through every operator on the way.
        let inlined = parse_model(
            "function pick input Real a; input Real b; output Real c;              algorithm c := if a > b or a < -b then a else -b; end pick;             model F Real v[2]; Real w[2];              equation v = {1, -3}; w = pick(v .* {1, 1}, fill(2.0, 2)); end F;",
        )
        .unwrap();
        assert!(
            !format!("{:?}", inlined.equations).contains("Call(\"pick\""),
            "the call must inline"
        );
    }

    #[test]
    fn arrays_refuse_what_does_not_fit() {
        let err = |source: &str| parse_model(source).unwrap_err().to_string();
        // Shapes that do not match.
        assert!(
            err("model M Real v[3]; Real w[2]; equation v = {1, 2, 3}; w = v; end M;")
                .contains("shapes")
        );
        // An array where a scalar belongs.
        assert!(
            err("model M Real v[2]; Real s; equation v = {1, 2}; s = sin(v) + 1; end M;")
                .contains("scalar")
                || err("model M Real v[2]; Real s; equation v = {1, 2}; s = sin(v) + 1; end M;")
                    .contains("shapes")
        );
        // Dividing by an array.
        assert!(
            err("model M Real v[2]; Real w[2]; equation v = {1, 2}; w = 1 / v; end M;")
                .contains("divisor")
        );
        // A binding of the wrong length.
        assert!(
            err("model M parameter Real k[3] = {1, 2}; Real x; equation x = k[1]; end M;")
                .contains("element")
        );
        // size of a missing dimension.
        assert!(
            err("model M Real v[2]; Real s; equation v = {1, 2}; s = size(v, 3); end M;")
                .contains("no such dimension")
        );
        // Elementwise between lengths that differ.
        assert!(err(
            "model M Real v[3]; Real w[3]; equation v = {1, 2, 3}; w = v .* {1, 2}; end M;"
        )
        .contains("do not fit together"));
        // A scalar product between different lengths.
        assert!(
            err("model M Real v[3]; Real s; equation v = {1, 2, 3}; s = v * {1, 2}; end M;")
                .contains("equal lengths")
        );
        // A start attribute of the wrong length.
        assert!(
            err("model M Real x[3](start = {1, 2}); equation der(x) = zeros(3); end M;")
                .contains("start has 2")
        );
        // linspace without enough points, a bad fill length.
        assert!(
            err("model M Real v[1]; equation v = linspace(0, 1, 1); end M;")
                .contains("at least two")
        );
        assert!(
            err("model M Real v[2]; Real q; equation q = 1; v = fill(1.0, q); end M;")
                .contains("compiler can see")
        );
    }

    #[test]
    fn the_new_expression_forms_travel_through_every_walker() {
        // Ranges, comprehensions, matrices and `end` written where each
        // walker touches them: inside a component's binding (prefixing
        // and substitution), behind a class constant, and with logical
        // operators around them.
        let m = parse_model(
            "package K constant Real width = 3; end K;              model Inner parameter Real n = 3;              parameter Real edge_case[3] = {i + K.width for i in 1:3};              Real gate;              equation gate = if time > 0.5 and not time > 2.0 or time < -1                then edge_case[end] else edge_case[1]; end Inner;              model M Inner part; end M;",
        )
        .unwrap();
        let binding = m
            .components
            .iter()
            .find(|c| c.name == "part.edge_case[2]")
            .and_then(|c| c.binding.clone())
            .unwrap();
        // 2 + K.width with the constant substituted.
        assert!(format!("{binding:?}").contains("Number(3.0)"));
        let gate = m
            .equations
            .iter()
            .find(|e| format!("{:?}", e.lhs).contains("gate"))
            .unwrap();
        assert!(format!("{:?}", gate.rhs).contains("edge_case[3]"));

        // A class alias at the top level of a file is refused.
        let error = crate::parser::parse_file("package P = Q;")
            .unwrap_err()
            .to_string();
        assert!(error.contains("full definition"), "{error}");

        // A range with a step, and one in a for loop with a step.
        let stepped = parse_model("model M Real v[3]; equation v = 1:3:7; end M;").unwrap();
        let text = format!("{:?}", stepped.equations);
        assert!(text.contains("Number(7.0)"), "{text}");
        assert!(
            parse_model("model M Real x; equation for i in 1:2:9 loop x = i; end for; end M;")
                .unwrap_err()
                .to_string()
                .contains("step is not supported")
        );

        // `fixed` through an alias-typed declaration's modifier list.
        let held = parse_model(
            "package U type V = Real(unit = \"m\"); end U;              model M U.V x(start = 2, fixed = true); equation der(x) = 1; end M;",
        )
        .unwrap();
        assert_eq!(
            held.components
                .iter()
                .find(|c| c.name == "x")
                .unwrap()
                .fixed,
            Some(true)
        );
    }

    #[test]
    fn alias_and_redeclare_spellings_with_all_the_trimmings() {
        // Modifiers on an alias target, `constrainedby` with its own
        // modifier list, a class redeclaration carrying both, and a
        // string description - every optional trailing piece at once.
        let m = parse_model(
            "package Media                partial package Base constant Real rho = 0; end Base;                package Water extends Base; constant Real rho = 1000; end Water;                package Oil extends Base; constant Real rho = 900; end Oil;              end Media;              model Tank                replaceable package Medium = Media.Water(rho = 1)                  constrainedby Media.Base(rho = 2);                Real a; equation a = Medium.rho; end Tank;              model M extends Tank(redeclare package Medium =                Media.Oil(rho = 3) constrainedby Media.Base); end M;",
        )
        .unwrap();
        assert!(format!("{:?}", m.equations).contains("Number(900.0)"));

        // `end` outside a subscript is refused with its own words.
        let error = parse_model("model M Real v[2]; Real x; equation v = 1:2; x = 1 + (v[1]);              x = v[end]; end M;");
        assert!(error.is_ok(), "end inside a subscript is fine");
        // Colon outside a subscript context in a scalar position.
        let error =
            parse_model("model M Real v[2]; Real x; equation v = 1:2; x = v[:] * {1, 1}; end M;")
                .unwrap();
        assert!(format!("{:?}", error.equations).contains("v[2]"));
    }

    #[test]
    fn more_matrix_builtins_and_their_error_paths() {
        let m = parse_model(
            "model M              parameter Real I3[3, 3] = identity(3);              parameter Real D[3, 3] = diagonal({7, 8, 9});              parameter Real W[2, 4] = cat(2, [1, 2; 3, 4], [5, 6; 7, 8]);              parameter Real S[4, 2] = cat(1, [1, 2; 3, 4], [5, 6; 7, 8]);              Real vm[2];              equation vm = {1.0, 0.0} * [1, 2; 3, 4]; end M;",
        )
        .unwrap();
        let binding_of = |name: &str| {
            m.components
                .iter()
                .find(|c| c.name == name)
                .and_then(|c| c.binding.clone())
                .unwrap_or_else(|| panic!("no binding for {name}"))
        };
        assert_eq!(binding_of("I3[2,2]"), Expr::Number(1.0));
        assert_eq!(binding_of("I3[2,3]"), Expr::Number(0.0));
        assert_eq!(binding_of("D[3,3]"), Expr::Number(9.0));
        assert_eq!(binding_of("D[1,2]"), Expr::Number(0.0));
        assert_eq!(binding_of("W[1,3]"), Expr::Number(5.0));
        assert_eq!(binding_of("S[3,1]"), Expr::Number(5.0));
        // Vector times matrix picks columns.
        let vm = m
            .equations
            .iter()
            .find(|e| format!("{:?}", e.lhs) == "Ref(\"vm[2]\")")
            .unwrap();
        assert!(format!("{:?}", vm.rhs).contains("Number(2.0)"));

        let err = |source: &str| parse_model(source).unwrap_err().to_string();
        assert!(err("model M Real x; equation x = transpose({1, 2}); end M;").contains("matrix"));
        assert!(
            err("model M Real v[3]; equation v = cross({1, 0}, {0, 1, 0}); end M;")
                .contains("3-vectors")
        );
        assert!(
            err("model M parameter Real W[2, 4] = cat(3, [1, 2; 3, 4], [5, 6; 7, 8]); end M;")
                .contains("dimension 3")
        );
        assert!(
            err("model M Real x[2]; equation x = {1, 2} * [1, 2; 3, 4] * {1}; end M;")
                .contains("equal")
        );
    }

    #[test]
    fn ranges_slices_comprehensions_and_matrices() {
        let m = parse_model(
            "model M parameter Integer n = 4;              parameter Real A[2, 2] = [1, 2; 3, 4];              Real v[4]; Real evens[2]; Real tail[2]; Real squares[4];              Real rotated[2]; Real mm[2, 2]; Real crossed[3]; Real total;              equation v = 1:4; evens = v[{2, 4}]; tail = v[end - 1:end];              squares = {i * i for i in 1:n};              rotated = A * {1.0, 0.0}; mm = A * transpose(A);              crossed = cross({1, 0, 0}, {0, 1, 0});              total = sum(i * 2 for i in 1:n); end M;",
        )
        .unwrap();
        let equation_for = |name: &str| {
            m.equations
                .iter()
                .find(|e| format!("{:?}", e.lhs) == format!("Ref({name:?})"))
                .unwrap_or_else(|| panic!("no equation for {name}"))
        };
        // The range unrolled into literals, the slices picked the right
        // elements, the comprehension squared its index.
        assert_eq!(equation_for("v[3]").rhs, Expr::Number(3.0));
        assert_eq!(equation_for("evens[2]").rhs, Expr::Ref("v[4]".into()));
        assert_eq!(equation_for("tail[1]").rhs, Expr::Ref("v[3]".into()));
        // The comprehension bound its index; folding 4*4 is the
        // simulator's business.
        let squares = format!("{:?}", equation_for("squares[4]").rhs);
        assert_eq!(squares, "Bin(Mul, Number(4.0), Number(4.0))");
        // cross of the first two axes is the third.
        let text = format!("{:?}", equation_for("crossed[3]").rhs);
        assert!(text.contains("Mul"), "{text}");

        // Error paths: a subscript outside the array, uneven matrix
        // rows, a zero-step range.
        let err = |source: &str| parse_model(source).unwrap_err().to_string();
        assert!(
            err("model M Real v[2]; Real x; equation v = 1:2; x = v[3]; end M;")
                .contains("outside an array")
        );
        assert!(err("model M parameter Real A[2, 2] = [1, 2; 3]; end M;").contains("equally wide"));
        assert!(err("model M Real v[2]; equation v = 1:0:2; end M;").contains("step by zero"));
    }

    #[test]
    fn functions_take_and_return_arrays() {
        // Reversal per element, a whole-array body, calls by qualified
        // name, and the result flowing on into a scalar product.
        let m = parse_model(
            "package Lib                function reverse input Real a[3]; output Real b[3];                algorithm for i in 1:3 loop b[i] := a[4 - i]; end for; end reverse;                function axpy input Real a; input Real x[3]; input Real y[3];                output Real z[3]; algorithm z := a * x .+ y; end axpy;              end Lib;              model M Real v[3]; Real r[3]; Real w[3]; Real check;              equation v = {1, 2, 3}; r = Lib.reverse(v);              w = Lib.axpy(10, v, r); check = w * {1, 1, 1}; end M;",
        )
        .unwrap();
        // Everything inlined: no calls survive into the flat model.
        let text = format!("{:?}", m.equations);
        assert!(!text.contains("Call"), "{text}");
        // r[1] is the last element of v: the function body reversed the
        // references, and v stays a variable rather than being folded.
        let r1 = m
            .equations
            .iter()
            .find(|e| format!("{:?}", e.lhs) == "Ref(\"r[1]\")")
            .unwrap();
        assert_eq!(r1.rhs, Expr::Ref("v[3]".to_string()));

        // An output never fully assigned is named element by element.
        let error = parse_model(
            "package Lib function half input Real a[2]; output Real b[2];              algorithm b[1] := a[1]; end half; end Lib;              model M Real v[2]; Real w[2];              equation v = {1, 2}; w = Lib.half(v); end M;",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("b[2]"), "{error}");

        // A subscripted target with a subscript nothing can fold.
        let error = parse_model(
            "package Lib function bad input Real a; output Real b[2];              algorithm b[a] := 1; b[1] := 1; end bad; end Lib;              model M Real q; Real w[2]; equation q = 1; w = Lib.bad(q); end M;",
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("whole number the compiler can see"),
            "{error}"
        );
    }

    #[test]
    fn connects_take_subscripts_loops_and_whole_arrays() {
        // A chain wired inside a for loop, with subscripted references.
        let chain = parse_model(
            "connector Pin Real v; flow Real i; end Pin;              model Two Pin p; Pin n; equation p.i + n.i = 0; p.v - n.v = p.i; end Two;              model Ground Pin p; equation p.v = 0; end Ground;              model Chain Two r[3]; Ground ground;              equation for i in 1:2 loop connect(r[i].n, r[i + 1].p); end for;              connect(r[3].n, ground.p); r[1].p.v = 6; r[1].p.i + 0 = 0; end Chain;",
        )
        .unwrap();
        // Two joints of the loop plus the ground joint: the potentials
        // of neighbouring pins are equal in the flat model.
        let text = format!("{:?}", chain.equations);
        assert!(text.contains("r[2].p.v"), "{text}");

        // Two whole arrays pair element by element.
        let bus = parse_model(
            "connector Pin Real v; flow Real i; end Pin;              model Bus Pin left[2]; Pin right[2];              equation left[1].v = 1; left[2].v = 2;              right[1].i = 0.1; right[2].i = 0.2;              connect(left, right); end Bus;",
        )
        .unwrap();
        let text = format!("{:?}", bus.equations);
        assert!(text.contains("right[2].v"), "{text}");

        // Arrays of different lengths are refused with the counts.
        let error = parse_model(
            "connector Pin Real v; flow Real i; end Pin;              model B Pin a[2]; Pin b[3]; equation connect(a, b); end B;",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("2 and 3"), "{error}");
    }

    #[test]
    fn connects_generate_kirchhoff_equations() {
        let source = "connector Pin Real v; flow Real i; end Pin;\
             model Ground Pin p; equation p.v = 0; end Ground;\
             model Two Pin p; Pin n; equation p.i + n.i = 0; p.v - n.v = p.i; end Two;\
             model Top Two a; Ground g; equation connect(a.p, g.p); end Top;";
        let m = parse_model(source).unwrap();
        // a.n is unconnected: its flow is forced to zero.
        let has_zero_flow = m.equations.iter().any(|e| {
            format!("{:?}", e.lhs).contains("a.n.i") && format!("{:?}", e.rhs).contains("0.0")
        });
        assert!(has_zero_flow, "unconnected flow must be zeroed");
    }
}
