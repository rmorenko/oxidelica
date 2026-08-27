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
use std::collections::{HashMap, HashSet};

/// Maximum instantiation depth (guards against recursive classes).
mod algorithms;
mod arrays;
mod clocks;
mod components;
mod connections;
mod constants;
mod equations;
mod extents;
mod external;
mod grids;
mod inheritance;
mod instantiate;
mod lookup;
mod names;
mod operators;
mod restrictions;
mod scoping;
mod statements;
mod strings;
mod table_files;
mod tables;
#[cfg(test)]
mod tests;

pub use lookup::{counts as name_counts, Trail};
/// Working an expression of this class out where it stands: the array
/// layer, with the class's names, shapes and loop variables in view.
type ExpandHere<'a> = dyn Fn(&Expr, &HashMap<String, f64>) -> Result<Value, String> + 'a;

pub(crate) use names::const_eval;
pub use table_files::table_in_file as read_table_file;

use algorithms::*;
use arrays::*;
use clocks::*;
use connections::*;
use constants::*;
use extents::*;
use inheritance::*;
use instantiate::*;
use lookup::*;
use names::*;
use operators::*;
use scoping::*;
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
        instantiable: !class.partial && (class.kind.is_model() || class.kind == ClassKind::Record),
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
/// The classes the language defines itself.
///
/// `StateSelect` is the enumeration a model uses to say which
/// variables it would rather the compiler integrated. Its literals are
/// ordered from the least to the most insistent, which is the order
/// the specification gives them and the order a comparison reads.
fn built_in_classes() -> Vec<ClassDef> {
    vec![
        ClassDef {
            kind: ClassKind::Type,
            name: "StateSelect".to_string(),
            enumeration: ["never", "avoid", "default", "prefer", "always"]
                .iter()
                .map(|literal| literal.to_string())
                .collect(),
            ..ClassDef::empty()
        },
        // `ExternalObject` is the handle a library keeps outside
        // Modelica: a class extending it says how to make one and how
        // to let it go, both in another language. It holds nothing of
        // its own, so a component of such a class is no variables at
        // all - and what is done with the handle is done by calls this
        // compiler refuses where they are made.
        ClassDef {
            kind: ClassKind::Model,
            name: "ExternalObject".to_string(),
            partial: true,
            ..ClassDef::empty()
        },
    ]
}

/// Flatten a model and everything it holds into one system of
/// equations, with every name written out in full.
pub fn flatten(classes: &[ClassDef], top: &str) -> Result<Model, String> {
    // The types the language supplies rather than a library: they are
    // written as ordinary classes so that everything downstream reads
    // them the way it reads any other, and they are put in first so a
    // library that declares its own of that name wins.
    let built_in = built_in_classes();
    let registry: HashMap<&str, &ClassDef> = built_in
        .iter()
        .map(|c| (c.name.as_str(), c))
        .chain(classes.iter().map(|c| (c.name.as_str(), c)))
        .collect();
    // The registry stands for as long as this flattening lasts, so
    // what a name is found to mean in it is worth remembering: the
    // walk out of the enclosing packages is asked the same question
    // thousands of times over.
    let _standing = lookup::StandingNames::open();
    let top_class = registry
        .get(top)
        .ok_or_else(|| format!("unknown class `{top}`"))?;

    what_a_class_may_hold(classes, &registry)?;

    let mut acc = build_the_model(&registry, top_class)?;

    // An expandable connector holds whatever the connections to it
    // name, so its members exist only once every `connect` is in.
    expand_buses(&registry, &mut acc)?;

    // What the connections come to: the sets they draw, and the
    // equations those sets stand for.
    join_the_connections(&registry, &mut acc)?;

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
    // What the connections can still be asked, now that they are all
    // in: which ports are roots of the overconstrained graph, and how
    // many connections named each.
    let GraphAnswers { roots, connected } = what_the_graph_answers(
        &acc.connection_graph,
        &acc.connects,
        &acc.roots,
        &acc.connect_rules,
        &model,
    )?;

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

    // Those answers put wherever the model asked the question.
    say_what_the_graph_answered(&mut model, &mut acc.conditional, &roots, &connected);

    // Clocked equations are lifted out before anything is checked:
    // what they leave behind is a `when` clause per clock, which the
    // rest of the pipeline already understands.
    // The branches of an undecided `if` were appended to the model so
    // that it could be counted whole; they come off again before
    // anything moves them about. Lifting the clocked equations out
    // first left the tail of the list something else - a `when` per
    // clock rather than the branches - and the truncation then took
    // equations the model still needed.
    for conditional in &acc.conditional {
        for branch in &conditional.branches {
            model
                .equations
                .truncate(model.equations.len() - branch.len());
        }
    }
    partition_clocks(&mut model)?;
    crate::check::verify(&model)?;
    // Each branch is still checked as it was written, against the
    // model it belongs to: a mistake inside a branch the run may take
    // is a mistake whether or not the run takes it.
    if !acc.conditional.is_empty() {
        let mut with_branches = model.clone();
        for conditional in &acc.conditional {
            for branch in &conditional.branches {
                with_branches.equations.extend(branch.iter().cloned());
            }
        }
        crate::check::verify(&with_branches)?;
    }
    // The branches themselves travel to the compiler, which settles
    // which one applies and compiles that mode as its own model.
    model.conditional = acc.conditional;
    // Strings are settled last, once every branch that could hold one
    // is in the model: what they leave behind is a Boolean where one
    // was compared, and nothing where one was declared.
    // A `size` written where the array had not been declared yet -
    // `extends SIMO(final nout = size(columns, 1))`, with `columns`
    // further down the same class - could not be settled there. Every
    // shape is in hand now, so it is settled here.
    settle_member_slices(&mut model, &acc.sizes);
    settle_sizes(&mut model, &acc.sizes);
    let settled = resolve_strings(&mut model)?;
    // A table the model wrote as a matrix is written out here, where
    // everything that could stand in it is settled - a file name is a
    // string, a smoothness a number - and what it leaves behind is
    // arithmetic with nothing outside Modelica left to run.
    tables::resolve_tables(&mut model, &acc.handles, &settled)?;
    restrictions::every_input_is_given_a_value(&acc.unsupplied, &model)?;
    external::nothing_left_unanswered(&model)?;
    // Whatever calls are still standing in the flat model are calls
    // nothing could inline. The bodies behind them travel with the
    // model, so the run can walk them for itself.
    model.functions = programs_used(&model, &registry)?;
    Ok(model)
}

/// What the language asks of a class however it is used, checked over
/// the classes rather than the instances so that a `partial` one
/// others are built on is checked too.
///
/// Moved out of `flatten` unchanged.
fn what_a_class_may_hold(
    classes: &[ClassDef],
    registry: &HashMap<&str, &ClassDef>,
) -> Result<(), String> {
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

    // A `block` is a model whose connectors all have a direction.
    for class in classes.iter().filter(|c| c.kind == ClassKind::Block) {
        restrictions::every_connector_of_a_block_is_causal(registry, class)?;
        restrictions::every_output_of_a_block_is_settled_inside(registry, class)?;
    }
    Ok(())
}

/// The whole model built from the top class down, twice where it has
/// to be.
///
/// `Connections.isRoot(frame_a.R)` is a question about the model as a
/// whole: a class asking it cannot be built until the graph is drawn,
/// and the graph is drawn from what building gathers. So the first
/// pass sets those `if` equations aside and is kept only for its
/// graph, and everything is built again with the answers in hand.
///
/// Moved out of `flatten` unchanged.
fn build_the_model(
    registry: &HashMap<&str, &ClassDef>,
    top_class: &ClassDef,
) -> Result<Flat, String> {
    let mut acc = Flat::default();
    let env = Env {
        outer_sizes: &HashMap::new(),
        overrides: &[],
        redeclares: &[],
        inners: &HashMap::new(),
        broken: &[],
        handed_shapes: &HashMap::new(),
        inside_a_parameter: false,
    };
    instantiate(registry, top_class, "", &env, &mut acc, 0)?;

    // `Connections.isRoot(frame_a.R)` is a question about the model as
    // a whole: a multibody body has states when nothing else settles
    // its orientation, and which body that is comes out of the graph
    // the connections draw. A class asking it cannot be built until
    // the graph is drawn, and the graph is drawn from what building
    // gathers - so the first pass sets those `if` equations aside and
    // is kept only for its graph, and everything is built again with
    // the answers in hand.
    if acc.graph_asked {
        let roots = choose_roots(&acc.connection_graph, &acc.connects)?;
        let counts = tally(&acc.connects);
        acc = Flat {
            roots,
            counts,
            answered: true,
            ..Flat::default()
        };
        instantiate(registry, top_class, "", &env, &mut acc, 0)?;
    }

    Ok(acc)
}

/// What every `connect` of the model comes to.
///
/// The connectors joined by connections fall into sets, and a set is
/// what the equations are written about: the potentials across it are
/// equal and the flows into it sum to nothing. Stream variables are
/// answered from the same sets, since `inStream` is a question about
/// one.
///
/// Moved out of `flatten` unchanged.
fn join_the_connections(registry: &HashMap<&str, &ClassDef>, acc: &mut Flat) -> Result<(), String> {
    // Connection sets via union-find over connector instance paths.
    // The paths are put in order first: what a hash map hands back
    // comes out differently in every process, and everything below
    // here - which connector a set is named after, which equation is
    // written first, and so which one the index reduction finds left
    // over - would come out differently with it.
    // A connector that holds connectors - the thermal port of a
    // machine holds a heat port per winding - is joined by them: a
    // `connect` of two such ports joins the heat ports in pairs, and
    // each pair carries the temperature and the heat flow. Written
    // against the ports themselves the equations would name something
    // no component of the flat model is called.
    let mut inside: HashMap<&str, Vec<&str>> = HashMap::new();
    for path in acc.connectors.keys() {
        if let Some(cut) = path.rfind('.') {
            let (outer, member) = (&path[..cut], &path[cut + 1..]);
            if acc.connectors.contains_key(outer) {
                inside.entry(outer).or_default().push(member);
            }
        }
    }
    for members in inside.values_mut() {
        members.sort_unstable();
    }
    let mut joined = Vec::new();
    for (a, b) in &acc.connects {
        // A side that names no connector at all is somebody else's
        // complaint to make, a few lines below.
        let of = |path: &String| inside.get(path.as_str()).cloned().unwrap_or_default();
        let (inside_a, inside_b) = (of(a), of(b));
        match inside_a.is_empty() || inside_a != inside_b {
            true => joined.push((a.clone(), b.clone())),
            false => joined.extend(
                inside_a
                    .into_iter()
                    .map(|member| (format!("{a}.{member}"), format!("{b}.{member}"))),
            ),
        }
    }
    let mut paths: Vec<String> = acc.connectors.keys().cloned().collect();
    paths.sort();
    let index: HashMap<&str, usize> = paths.iter().map(|p| p.as_str()).zip(0..).collect();
    let mut parent: Vec<usize> = (0..paths.len()).collect();
    fn find(parent: &mut Vec<usize>, i: usize) -> usize {
        if parent[i] != i {
            let root = find(parent, parent[i]);
            parent[i] = root;
        }
        parent[i]
    }
    for (a, b) in &joined {
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

    // And the sets themselves in order, by the first connector each
    // holds. Sorting inside a set is not enough: the sequence of sets
    // is what decides the order the equations are written in.
    let mut sets: Vec<Vec<&str>> = sets
        .into_values()
        .map(|mut members| {
            members.sort();
            members
        })
        .collect();
    sets.sort();
    for members in sets.iter_mut() {
        // Connectors in one set must match in shape, not in name: a
        // signal output and a signal input are different classes with
        // the same members, and connecting them is the whole point.
        let class_name = acc.connectors[members[0]].clone();
        let class = registry[class_name.as_str()];
        // A connector may say what it holds through a base class: the
        // multibody frames are one `Frame` with the position, the
        // orientation and the two flows, and `Frame_a` and `Frame_b`
        // add nothing to it but an icon. Read from the class alone
        // those two hold nothing at all, and a flow nobody sums is a
        // variable no equation ever names.
        let members_of = |class: &ClassDef| -> Vec<Component> {
            let mut out = Vec::new();
            fn gather(
                registry: &HashMap<&str, &ClassDef>,
                class: &ClassDef,
                out: &mut Vec<Component>,
                depth: usize,
            ) {
                if depth > MAX_DEPTH {
                    return;
                }
                for extend in &class.extends {
                    if let Some(base) = lookup(registry, &extend.base, &class.name, &class.imports)
                    {
                        gather(registry, base, out, depth + 1);
                    }
                }
                for component in &class.components {
                    // A member that is a record is not one variable
                    // but the fields it holds: the magnetic ports of
                    // the fundamental-wave machines carry a complex
                    // potential and a complex flux, and flattening
                    // knows those by `V_m.re` and `V_m.im`. Equating
                    // the record's own name would name a variable the
                    // flat model does not have. A field with
                    // dimensions of its own is left whole, since the
                    // name it would take is not one this knows.
                    let held = lookup(registry, &component.type_name, &class.name, &class.imports)
                        .filter(|of| of.kind == ClassKind::Record)
                        .filter(|_| component.dimensions.is_empty());
                    match held {
                        Some(record) => {
                            let mut fields = Vec::new();
                            gather(registry, record, &mut fields, depth + 1);
                            for mut field in fields {
                                field.name = format!("{}.{}", component.name, field.name);
                                field.flow = component.flow;
                                field.stream = component.stream;
                                field.variability = component.variability;
                                out.push(field);
                            }
                        }
                        None => out.push(component.clone()),
                    }
                }
            }
            gather(registry, class, &mut out, 0);
            out
        };
        let held = members_of(class);
        let shape = |class: &ClassDef| -> Vec<(String, bool, bool)> {
            let mut members: Vec<(String, bool, bool)> = members_of(class)
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
        if held.iter().any(|c| c.stream) {
            let flows = held.iter().filter(|c| c.flow).count();
            if flows != 1 {
                return Err(format!(
                    "connector `{class_name}` carries stream variables, so it needs \
                     exactly one flow variable, found {flows}"
                ));
            }
        }
        // A connector that is one value rather than a set of members -
        // `connector RealInput = input Real` - joins on itself: there
        // is no member to name, so the paths are the variables.
        if held.is_empty() && class.alias_of.is_some() {
            // Which side the equation defines is not the order the
            // set happened to be in: an `input` takes its value from
            // whatever it was connected to, and an `output` states
            // one. Written the other way round, a signal that already
            // had a definition of its own got a second and the input
            // got none - and on a clocked signal that is a model
            // refused as unbalanced.
            let states_it = |path: &str| {
                acc.connectors
                    .get(path)
                    .and_then(|of| registry.get(of.as_str()))
                    .is_some_and(|of| of.alias_causality == Causality::Output)
            };
            let source = members
                .iter()
                .find(|path| states_it(path))
                .unwrap_or(&members[0]);
            for other in members.iter() {
                if other == source {
                    continue;
                }
                acc.equations.push(EquationItem {
                    lhs: Expr::Ref((*other).to_string()),
                    rhs: Expr::Ref((*source).to_string()),
                    origin: String::new(),
                });
            }
            continue;
        }
        for member_component in &held {
            let var = |path: &str| format!("{path}.{}", member_component.name);
            // A parameter of a connector is not a variable the
            // connection solves for: the fluid ports of the heat-flow
            // library each carry the medium they are filled with, and
            // joining two of them says the media must agree, not that
            // one is computed from the other. Equating them would ask
            // the run to solve for something settled before it began.
            if matches!(
                member_component.variability,
                Variability::Parameter | Variability::Constant
            ) {
                continue;
            }
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
        for members in sets.iter() {
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
            registry,
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
                        WhenAction::Assign(_, value)
                        | WhenAction::Reinit(_, value)
                        | WhenAction::TupleAssign(_, value) => {
                            *value = resolve_streams(value, &context)?;
                        }
                        // A check made at the event may ask after a
                        // stream the same way an assignment may.
                        WhenAction::Assert(condition, _) => {
                            *condition = resolve_streams(condition, &context)?;
                        }
                        // A call on its own is taken apart while
                        // flattening, which keeps the checks its body
                        // makes and nothing of the call.
                        WhenAction::Terminate(_) | WhenAction::Call(..) => {}
                        // Taken apart while flattening, so neither a
                        // loop nor a choice is left.
                        WhenAction::Loop(_) | WhenAction::Choice(_) => {}
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

    Ok(())
}

/// What the connections answer once they are all in: which ports are
/// roots of the overconstrained graph, and how many connections named
/// each port.
struct GraphAnswers {
    roots: HashMap<String, bool>,
    connected: HashMap<String, f64>,
}

/// The two questions a model may ask about its own connections, and
/// the checks a connector's declaration asked for.
///
/// An overconstrained graph is broken open before anything else looks
/// at it, and `Connections.isRoot` is answered from what that came to.
/// `cardinality` is answered from the same place: how many `connect`
/// equations named a port. This is the last moment either answer is
/// known.
///
/// Moved out of `flatten` unchanged.
fn what_the_graph_answers(
    connection_graph: &[GraphClause],
    connects: &[(String, String)],
    already: &HashMap<String, bool>,
    connect_rules: &[(String, Vec<Expr>)],
    _model: &Model,
) -> Result<GraphAnswers, String> {
    // An overconstrained graph is broken open before anything else
    // looks at it, and `Connections.isRoot` is answered from what that
    // came to. `cardinality` is answered from the same place: how many
    // `connect` equations named a port. Both are questions about the
    // connections, and this is the last moment the answers are known.
    let roots = choose_roots(connection_graph, connects)?;
    // The second pass was built on the roots the first pass's graph
    // gave. If building on them drew a different graph, the model asks
    // the graph a question whose answer changes the graph, and there is
    // no answer to give.
    if !already.is_empty() && &roots != already {
        return Err(
            "the model asks `Connections.isRoot` where the answer changes the graph it is \
             asked about"
                .to_string(),
        );
    }
    let connected = tally(connects);
    // What a connector's declaration asked of the connections to it.
    // The chapter says these make it an error rather than leaving it to
    // the tool, so they are checked here, where how often each port was
    // named is already known.
    for (port, said) in connect_rules {
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

    Ok(GraphAnswers { roots, connected })
}

/// The graph's answers put wherever the model asked the question:
/// among the equations, inside the branches of an `if` that travel to
/// the compiler on their own, and in the assertions and `when` clauses
/// where `cardinality` is usually asked about an unconnected port.
///
/// Moved out of `flatten` unchanged.
fn say_what_the_graph_answered(
    model: &mut Model,
    conditional: &mut [ConditionalEquations],
    roots: &HashMap<String, bool>,
    connected: &HashMap<String, f64>,
) {
    let answer = |expr: &Expr| answer_graph_queries(expr, roots, connected);
    for equation in model
        .equations
        .iter_mut()
        .chain(model.initial_equations.iter_mut())
    {
        equation.lhs = answer(&equation.lhs);
        equation.rhs = answer(&equation.rhs);
    }
    // The branches of an `if` equation travel to the compiler apart
    // from the equations, and copies of them were what went through
    // the loop above. So they are answered where they live: the state
    // graph asks `if cardinality(inPort[i]) == 0` to decide what an
    // unconnected port stands at, and nothing later knows the count.
    for conditional in conditional.iter_mut() {
        for condition in &mut conditional.conditions {
            *condition = answer(condition);
        }
        for branch in &mut conditional.branches {
            for equation in branch.iter_mut() {
                equation.lhs = answer(&equation.lhs);
                equation.rhs = answer(&equation.rhs);
            }
        }
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
                    WhenAction::Assign(_, value)
                    | WhenAction::Reinit(_, value)
                    | WhenAction::TupleAssign(_, value) => {
                        *value = answer(value);
                    }
                    WhenAction::Assert(condition, _) => *condition = answer(condition),
                    // Taken apart while flattening, so none of these
                    // reaches a flat model.
                    WhenAction::Terminate(_)
                    | WhenAction::Call(..)
                    | WhenAction::Loop(_)
                    | WhenAction::Choice(_) => {}
                }
            }
        }
    }
}

/// Write out every `a.b.c` still standing that names a member of an
/// array of components.
///
/// A value written on an `extends` may read one member off every
/// element of an array that belongs to a neighbour: a machine sums
/// `rs.resistor.LossPower` over its phases. Where that value is read,
/// the base is being built and the neighbour has not been instantiated
/// yet, so nothing there knows `rs.resistor` is an array and the name
/// travels out whole. By now everything is measured, so it is written
/// out here into the array of names it always meant.
fn settle_member_slices(model: &mut Model, shapes: &[(String, Vec<i64>)]) {
    let known: HashMap<&str, &Vec<i64>> = shapes
        .iter()
        .map(|(name, shape)| (name.as_str(), shape))
        .collect();
    if known.is_empty() {
        return;
    }
    fn answer(expr: &Expr, known: &HashMap<&str, &Vec<i64>>, under_reduction: bool) -> Expr {
        if let Expr::Ref(name) = expr {
            // A name that is already an element - `rs.resistor[1].R` -
            // was written out by whoever knew the shape, and saying so
            // again would subscript it twice.
            if !name.contains('[') && under_reduction {
                // The name may be the array itself - `sum(gap.i_ss)`
                // - or a member read off one - `sum(rs.resistor.P)`.
                let spread = match known.get(name.as_str()) {
                    Some(shape) => Some(((*shape).clone(), name.clone(), None)),
                    None => member_of(name, known).map(|(array, member)| {
                        (known[array.as_str()].clone(), array, Some(member))
                    }),
                };
                if let Some((shape, array, member)) = spread {
                    let items: Vec<Expr> = index_tuples(&shape)
                        .into_iter()
                        .map(|at| {
                            let element = element_name(&array, &at);
                            Expr::Ref(match &member {
                                Some(member) => format!("{element}.{member}"),
                                None => element,
                            })
                        })
                        .collect();
                    if !items.is_empty() {
                        return Expr::Array(items);
                    }
                }
            }
        }
        let recur = |e: &Expr| answer(e, known, false);
        // The operators that take an array and come to one number.
        // Nothing else may hold an array by the time flattening is
        // over, so only what a written-out slice can stand inside is
        // followed at all.
        if let Expr::Call(name, args) = expr {
            if let ("sum" | "max" | "min", [inside]) = (name.as_str(), args.as_slice()) {
                if let Expr::Array(items) = answer(inside, known, true) {
                    let mut over = items.into_iter();
                    if let Some(first) = over.next() {
                        return over.fold(first, |so_far, item| match name.as_str() {
                            "sum" => Expr::Bin(BinOp::Add, Box::new(so_far), Box::new(item)),
                            other => Expr::Call(other.to_string(), vec![so_far, item]),
                        });
                    }
                }
            }
        }
        match expr {
            Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
            Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
            Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
            other => other.clone(),
        }
    }
    // An equation between two whole arrays that nothing knew the
    // shape of when it was written - a connection between two
    // connectors holding a space phasor `Real v_[2]` - is one
    // equation per element, so it is taken apart rather than
    // rewritten in place.
    for equations in [&mut model.equations, &mut model.initial_equations] {
        let mut written = Vec::with_capacity(equations.len());
        for equation in equations.drain(..) {
            let whole = whole_shape(&equation.lhs, &known)
                .into_iter()
                .chain(whole_shape(&equation.rhs, &known))
                .try_fold(None, |so_far: Option<Vec<i64>>, shape| match so_far {
                    Some(first) if first != shape => Err(()),
                    _ => Ok(Some(shape)),
                })
                .ok()
                .flatten()
                .map(|shape| index_tuples(&shape));
            match whole {
                Some(indices) if !indices.is_empty() => {
                    written.extend(indices.into_iter().map(|at| EquationItem {
                        lhs: per_element(&equation.lhs, &known, &at),
                        rhs: per_element(&equation.rhs, &known, &at),
                        origin: equation.origin.clone(),
                    }));
                }
                _ => written.push(equation),
            }
        }
        *equations = written;
    }
    for equation in model
        .equations
        .iter_mut()
        .chain(model.initial_equations.iter_mut())
    {
        // An equation already written out one element at a time -
        // `vs[2] = plug_sp.pin.v - plug_sn.pin.v`, where the left is
        // an array of this class and the right reads a member off an
        // array of components - wants the element of the same index
        // on both sides rather than the whole slice. The subscript
        // the left carries says which element that is.
        if let Some(at) = element_read(&equation.lhs, &known) {
            let (lhs, rhs) = (
                per_element(&equation.lhs, &known, &at),
                per_element(&equation.rhs, &known, &at),
            );
            equation.lhs = lhs;
            equation.rhs = rhs;
            continue;
        }
        equation.lhs = answer(&equation.lhs, &known, false);
        equation.rhs = answer(&equation.rhs, &known, false);
    }
}

/// The array a name reads a member off, longest prefix first: with
/// arrays inside arrays the innermost one owns the subscript.
fn member_of(name: &str, known: &HashMap<&str, &Vec<i64>>) -> Option<(String, String)> {
    let mut cut = name.rfind('.')?;
    loop {
        let (array, member) = (&name[..cut], &name[cut + 1..]);
        if known.contains_key(array) {
            return Some((array.to_string(), member.to_string()));
        }
        cut = array.rfind('.')?;
    }
}

/// The shape every whole array standing in `expr` has, or nothing
/// where there is none or where two disagree. What stands under a
/// reduction is left out: `sum(rs.resistor.LossPower)` is one number
/// however long the array inside it is.
fn whole_shape(expr: &Expr, known: &HashMap<&str, &Vec<i64>>) -> Vec<Vec<i64>> {
    let gather = |e: &Expr| whole_shape(e, known);
    match expr {
        Expr::Ref(name) if !name.contains('[') => known
            .get(name.as_str())
            .map(|shape| (*shape).clone())
            .or_else(|| {
                let (array, _) = member_of(name, known)?;
                Some(known[array.as_str()].clone())
            })
            .into_iter()
            .collect(),
        Expr::Call(name, _) if matches!(name.as_str(), "sum" | "max" | "min") => Vec::new(),
        Expr::Call(_, args) => args.iter().flat_map(gather).collect(),
        Expr::Neg(inner) => gather(inner),
        Expr::Bin(_, l, r) => gather(l).into_iter().chain(gather(r)).collect(),
        _ => Vec::new(),
    }
}

/// The subscripts of `expr`, where it is one element of an array whose
/// shape is known, and nothing for anything else.
fn element_read(expr: &Expr, known: &HashMap<&str, &Vec<i64>>) -> Option<Vec<i64>> {
    let Expr::Ref(name) = expr else {
        return None;
    };
    let (base, rest) = name.split_once('[')?;
    let shape = known.get(base)?;
    let at: Vec<i64> = rest
        .trim_end_matches(']')
        .split(',')
        .filter_map(|part| part.trim().parse::<i64>().ok())
        .collect();
    match at.len() == shape.len() {
        true => Some(at),
        false => None,
    }
}

/// Write every whole member slice in `expr` as the one element at
/// `at`, where the array it reads off has an element there to give.
fn per_element(expr: &Expr, known: &HashMap<&str, &Vec<i64>>, at: &[i64]) -> Expr {
    let recur = |e: &Expr| per_element(e, known, at);
    if let Expr::Ref(name) = expr {
        if !name.contains('[') {
            // The name may be an array of the same shape - a machine
            // writes `idq_ss = airGap.i_ss`, and the air gap was not
            // built when that was read - or a member read off one.
            let here = match known.get(name.as_str()) {
                Some(shape) => Some(((*shape).clone(), name.clone(), None)),
                None => member_of(name, known)
                    .map(|(array, member)| (known[array.as_str()].clone(), array, Some(member))),
            };
            if let Some((shape, array, member)) = here {
                let fits = shape.len() == at.len()
                    && shape.iter().zip(at).all(|(&size, &index)| index <= size);
                if fits {
                    let element = element_name(&array, at);
                    return Expr::Ref(match &member {
                        Some(member) => format!("{element}.{member}"),
                        None => element,
                    });
                }
            }
        }
    }
    match expr {
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        other => other.clone(),
    }
}

/// Answer every `size` still standing in the model from the shapes
/// flattening gathered.
fn settle_sizes(model: &mut Model, shapes: &[(String, Vec<i64>)]) {
    let known: HashMap<&str, &Vec<i64>> = shapes
        .iter()
        .map(|(name, shape)| (name.as_str(), shape))
        .collect();
    if known.is_empty() {
        return;
    }
    /// Every number in a value written out, in order, or `None` where
    /// something in it is not one.
    fn all_numbers(expr: &Expr) -> Option<Vec<f64>> {
        match expr {
            Expr::Number(value) => Some(vec![*value]),
            Expr::Array(items) => items.iter().try_fold(Vec::new(), |mut all, item| {
                all.extend(all_numbers(item)?);
                Some(all)
            }),
            Expr::MatrixRows(rows) => {
                rows.iter()
                    .flatten()
                    .try_fold(Vec::new(), |mut all: Vec<f64>, cell| {
                        all.extend(all_numbers(cell)?);
                        Some(all)
                    })
            }
            _ => None,
        }
    }

    fn answer(expr: &Expr, known: &HashMap<&str, &Vec<i64>>) -> Expr {
        // The largest of a column of lengths, which is how a block
        // with several outputs counts them: `nout = max([size(columns,
        // 1); size(offset, 1)])`. Once the lengths are in, that is a
        // number rather than a question about arrays.
        if let Expr::Call(name, args) = expr {
            if let ("max" | "min", [inside]) = (name.as_str(), args.as_slice()) {
                if let Some(numbers) = all_numbers(&answer(inside, known)) {
                    if let Some((first, rest)) = numbers.split_first() {
                        return Expr::Number(rest.iter().fold(
                            *first,
                            |a, b| match name.as_str() {
                                "max" => a.max(*b),
                                _ => a.min(*b),
                            },
                        ));
                    }
                }
            }
        }
        if let Expr::Call(name, args) = expr {
            if name == "size" && args.len() == 2 {
                if let (Expr::Ref(of), Expr::Number(axis)) = (&args[0], &args[1]) {
                    if let Some(length) = known
                        .get(of.as_str())
                        .and_then(|shape| shape.get(*axis as usize - 1))
                    {
                        return Expr::Number(*length as f64);
                    }
                }
            }
        }
        let recur = |e: &Expr| answer(e, known);
        match expr {
            Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
            Expr::Array(items) => Expr::Array(items.iter().map(recur).collect()),
            Expr::MatrixRows(rows) => Expr::MatrixRows(
                rows.iter()
                    .map(|row| row.iter().map(recur).collect())
                    .collect(),
            ),
            Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
            Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
            Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
            Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
            Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
            Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
            Expr::If(c, a, b) => {
                Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b)))
            }
            other => other.clone(),
        }
    }
    for component in &mut model.components {
        if let Some(binding) = &component.binding {
            component.binding = Some(answer(binding, &known));
        }
    }
    for equation in model
        .equations
        .iter_mut()
        .chain(model.initial_equations.iter_mut())
    {
        equation.lhs = answer(&equation.lhs, &known);
        equation.rhs = answer(&equation.rhs, &known);
    }
    // A check is where a `size` is most often written: the table
    // blocks assert that their matrix is not empty.
    for (condition, _) in &mut model.asserts {
        *condition = answer(condition, &known);
    }
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
    /// How long every array instantiated so far is, by flat path, in
    /// the order they were instantiated. A class's own declarations
    /// are measured as it goes, so by the time an expression names
    /// `plug.pin.v` the length of `plug.pin` is here - which no table
    /// built from the class's own text could say, since `pin` belongs
    /// to another class. The order is what lets a reader take only
    /// what is new to it rather than sweeping the whole list again.
    sizes: Vec<(String, Vec<i64>)>,
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
    /// The same numbers in the order they arrived, so that a class
    /// part-way through its declarations can take up what is new
    /// without walking everything it already has.
    numbers: Vec<(String, f64)>,
    /// Instance paths of components a false condition left out.
    disabled: Vec<String>,
    /// `if` equations whose condition is only known while running.
    conditional: Vec<ConditionalEquations>,
    /// Which nodes of the overconstrained graph turned out to be
    /// roots. Empty while the graph has not been drawn - which is to
    /// say, on the first pass.
    roots: HashMap<String, bool>,
    /// How many `connect` equations named each connector, as the pass
    /// before gathered them. This is what `cardinality` is answered
    /// from where the answer decides whether an equation exists at
    /// all. Empty on the first pass, along with `roots`.
    counts: HashMap<String, f64>,
    /// Whether the roots and the counts above are the pass before's
    /// rather than nothing at all. `roots` may be empty in earnest -
    /// a model with no overconstrained loop has none - so emptiness
    /// is not what says which pass this is.
    answered: bool,
    /// Whether an `if` equation was set aside because its condition
    /// asks the connections a question the first pass cannot answer.
    graph_asked: bool,
    /// What each handle to something outside Modelica was built from:
    /// the instance path of the declaration, and the constructor call
    /// with everything it was handed worked out. A table block keeps
    /// its data behind one of these, and this is where what is behind
    /// it can still be reached.
    handles: HashMap<String, Expr>,
    /// Classes already looked over for a reach into what a component
    /// keeps to itself. The answer is about the class alone, so it is
    /// worked out once however many instances of it a model holds.
    restrictions_checked: HashSet<String>,
    /// Every instance that is a class in its own right, by its path,
    /// and what that class is called. A record is not one of these:
    /// its fields are the holder's, and a count that asked otherwise
    /// would find every record short of two equations.
    instances: HashMap<String, String>,
    /// The `input` declarations of a model or block that nothing
    /// settled: not the declaration itself, and not whoever holds it.
    /// Written down here because after flattening a value that was
    /// given and a value that was never written look alike.
    unsupplied: Vec<(String, String)>,
    /// Which base classes have been merged into which instance. A
    /// diamond names one of them by two paths, and the specification
    /// says such an element is included once.
    extended: HashSet<(String, String)>,
    /// Every instance known to be a record, by its flat path, and
    /// what record it is. A modifier is written in the terms of the
    /// class that supplied it, so what one class knows has to still
    /// be in view while the class it hands the value to is built.
    records: HashMap<String, String>,
}

/// How many `connect` equations name each connector.
fn tally(connects: &[(String, String)]) -> HashMap<String, f64> {
    let mut counted: HashMap<String, f64> = HashMap::new();
    for (a, b) in connects {
        for port in [a, b] {
            *counted.entry(port.clone()).or_insert(0.0) += 1.0;
        }
    }
    counted
}

impl Flat {
    /// Whether a path names a component left out by its condition, or
    /// anything inside one.
    fn is_disabled(&self, path: &str) -> bool {
        self.disabled.iter().any(|left_out| {
            // An array of components is left out by the name of the
            // array, and what a `connect` names is an element of it:
            // `filter[1].u` where the whole of `filter` is not there.
            path == left_out
                || path.starts_with(&format!("{left_out}."))
                || path.starts_with(&format!("{left_out}["))
        })
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
    /// Whether this instance sits inside a parameter: the fields of a
    /// `parameter` record are parameters themselves, however the
    /// record declares them, and a field left continuous turns its
    /// value into an equation the parameters cannot read.
    inside_a_parameter: bool,
    /// Shapes of the arrays named by the values in `overrides`, which
    /// are written in the terms of the class above and are not names
    /// here at all. Without them a value like `T = T_ref` looks like a
    /// scalar and spreads whole over every element it lands on.
    handed_shapes: &'a HashMap<String, Vec<i64>>,
    /// The arrays of the class above, whose names a modifier it wrote
    /// still uses even though it is read down here.
    outer_sizes: &'a HashMap<String, Vec<i64>>,
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
    /// The connector class this was declared with, where that class is
    /// one holding a single value rather than members: `connector
    /// RealInput = input Real`. Resolving the type leaves a primitive
    /// behind and the connector-ness with it, so it is carried here.
    value_connector: Option<&'a str>,
}

/// The class-level context a component is instantiated in.
struct Level<'a> {
    /// Instance path prefix of the enclosing class.
    prefix: &'a str,
    /// How long the arrays in view are, by the name an expression
    /// written here would use: a declaration's value may ask.
    sizes: &'a HashMap<String, Vec<i64>>,
    /// The same for the class above, whose names a modifier handed
    /// down still uses. In view for reading such a value and nowhere
    /// else.
    outer_sizes: &'a HashMap<String, Vec<i64>>,
    /// `outer` declaration name -> flat path of the `inner` instance.
    outers: &'a HashMap<String, String>,
    /// `inner` instances the components below can bind to.
    inners: &'a HashMap<String, InnerInstance>,
    /// Modifier overrides the enclosing class received.
    overrides: &'a [(String, Expr)],
    /// Parameter values of the enclosing class, by local name.
    consts: &'a HashMap<String, f64>,
    /// What the `String` parameters of this class are worth: a value
    /// worked out by a function may be handed one, and a name is
    /// nothing a body can measure.
    texts: &'a HashMap<String, String>,
    /// Imports of the enclosing class.
    imports: &'a [(String, String)],
    /// Package scope of the enclosing class.
    scope: &'a str,
    /// Whether the enclosing class is itself part of a parameter.
    inside_a_parameter: bool,
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
            Value::Array(_) => {
                let shape = self.shape();
                let mut items = Vec::new();
                self.flatten_into(&mut items);
                Err(format!(
                    "an array of shape {shape:?} is used where a scalar is expected, \
                     beginning {:?}",
                    items.first()
                ))
            }
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
