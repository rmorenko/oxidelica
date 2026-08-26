//! Connections: the sets they make, the buses they fill, what a
//! stream carries, and the graph they must not close.

use super::*;

/// Resolve both sides of a `connect` into instance paths and pair them.
///
/// A subscripted reference folds to one path; a whole array of
/// connectors pairs element by element with the other side, which must
/// then have the same length. Connections to components a condition
/// left out are dropped, like everywhere else.
#[allow(clippy::too_many_arguments)]
pub(super) fn push_connects(
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
    // A connector named here is "outside" when it is a port of the very
    // class the `connect` is written in, rather than of one of its
    // components: `connect(inner.c, c)` in Sub makes `c` outside and
    // `inner.c` inside, and the names differ by exactly that - an
    // outside port sits directly under the class prefix.
    //
    // The top-level model is nobody's component, so nothing there is
    // outside; an empty prefix would otherwise call every connector
    // outside and flip the sign of the whole node.
    let is_outside = |path: &str| {
        !prefix.is_empty()
            && path
                .strip_prefix(prefix)
                .is_some_and(|rest| !rest.contains('.'))
    };
    for (a, b) in left.into_iter().zip(right) {
        if acc.is_disabled(&a) || acc.is_disabled(&b) {
            continue;
        }
        for side in [&a, &b] {
            if is_outside(side) && !acc.outside.contains(side) {
                acc.outside.push(side.clone());
            }
        }
        acc.connects.push((a, b));
    }
    Ok(())
}

/// Give every expandable connector the members its connections name.
///
/// A bus declares nothing of its own: `connect(bus.speed, sensor.w)`
/// is what creates `bus.speed`, and it takes the type of the other
/// side. Buses connected to each other share one pool of members, so a
/// sub-bus carries everything its parent does and the matching members
/// are connected in turn.
pub(super) fn expand_buses(
    registry: &HashMap<&str, &ClassDef>,
    acc: &mut Flat,
) -> Result<(), String> {
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
        outer_sizes: &HashMap::new(),
        overrides: &[],
        redeclares: &[],
        inners: &HashMap::new(),
        broken: &[],
        handed_shapes: &HashMap::new(),
        inside_a_parameter: false,
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
    // A bus may declare members of its own as well as taking the ones
    // its connections name - the standard library's control bus writes
    // out the five signals it expects - and a declared member nothing
    // connects to has no equation at all. 9.1.3 says what it is worth:
    // a potential variable with no connections is zero, the same as
    // any unconnected connector variable. Left without one it was an
    // unknown more than the model had equations, which is what two
    // hundred and twenty-three models were refused for.
    for bus in &buses {
        let Some(class_name) = acc.connectors.get(bus) else {
            continue;
        };
        let class = registry[class_name.as_str()];
        for member in &class.components {
            let path = format!("{bus}.{}", member.name);
            let stated = acc
                .equations
                .iter()
                .any(|equation| names_it(&equation.lhs, &path) || names_it(&equation.rhs, &path));
            if stated || acc.connectors.contains_key(&path) {
                continue;
            }
            if !acc.components.iter().any(|c| c.name == path) {
                continue;
            }
            // What "zero" is depends on the type: a Boolean signal
            // stands at false, and equating it to a number is a type
            // error rather than a value.
            let rhs = match member.type_name.as_str() {
                "Boolean" => Expr::Bool(false),
                _ => Expr::Number(0.0),
            };
            acc.equations.push(EquationItem {
                lhs: Expr::Ref(path),
                rhs,
                origin: acc.origin.clone(),
            });
        }
    }
    Ok(())
}

/// Whether an expression names exactly this variable.
fn names_it(expr: &Expr, path: &str) -> bool {
    matches!(expr, Expr::Ref(name) if name == path)
}

/// Replace `inStream(...)` and `actualStream(...)` with the mix the
/// connection set defines for them.
pub(super) fn resolve_streams(expr: &Expr, context: &StreamContext) -> Result<Expr, String> {
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
pub(super) fn stream_mix(
    name: &str,
    context: &StreamContext,
    actual: bool,
) -> Result<Expr, String> {
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
            // The specification weighs each port by `max(-m, 0)`: a port
            // pushing nothing into the node has no say in what the node
            // holds. Only the divisor is regularised - that is what
            // `positiveMax` is - so the mix survives every flow going
            // quiet without a silent port tugging it towards its own
            // value.
            // Which way a port's flow points into the node depends on
            // which side of its class it is: an inside connector pushes
            // when its flow is negative, an outside one - a port of the
            // class the connection was written in - when it is
            // positive. That is the sign convention of 9.1.2, and it is
            // the whole of the inside/outside distinction here.
            let inflow = |other: &str| {
                let flow = Expr::Ref(format!("{other}.{flow_name}"));
                if context.outside.iter().any(|path| path == other) {
                    flow
                } else {
                    Expr::Neg(Box::new(flow))
                }
            };
            let weight =
                |other: &str| Expr::Call("max".to_string(), vec![inflow(other), Expr::Number(0.0)]);
            let guarded = |other: &str| {
                Expr::Call(
                    "max".to_string(),
                    vec![inflow(other), Expr::Number(STREAM_EPS)],
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
            let denominator = sum(others.iter().map(|other| guarded(other)).collect());
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

/// Break an overconstrained connection graph open, and say which nodes
/// ended up as roots.
///
/// Every part of the graph needs exactly one root: with none there is
/// nothing for the rest of it to be measured against, and with two the
/// equations that hold a loop closed would be stated twice. A declared
/// root is taken as given; a potential one serves where a part has
/// none, lowest priority first and by name after that, so the same
/// model always breaks the same way.
pub(super) fn choose_roots(
    clauses: &[GraphClause],
    connects: &[(String, String)],
) -> Result<HashMap<String, bool>, String> {
    if clauses.is_empty() {
        return Ok(HashMap::new());
    }
    let mut nodes: Vec<String> = Vec::new();
    let remember = |name: &str, nodes: &mut Vec<String>| {
        if !nodes.iter().any(|known| known == name) {
            nodes.push(name.to_string());
        }
    };
    for clause in clauses {
        match clause {
            GraphClause::Root(node) | GraphClause::PotentialRoot(node, _) => {
                remember(node, &mut nodes)
            }
            GraphClause::Branch(a, b) => {
                remember(a, &mut nodes);
                remember(b, &mut nodes);
            }
        }
    }
    // A `connect` between two nodes of the graph is a branch of it as
    // surely as one written out.
    let mut edges: Vec<(String, String)> = clauses
        .iter()
        .filter_map(|clause| match clause {
            GraphClause::Branch(a, b) => Some((a.clone(), b.clone())),
            _ => None,
        })
        .collect();
    // Two multibody frames are connected as frames, and it is the
    // orientation inside each that the graph is drawn over: a
    // `connect` between two connectors holding one is a branch between
    // the two of them. The member is read off the nodes the clauses
    // named - `frame_a.R` says the member is `R` - and a connection
    // brings its other side into the graph only where one side is
    // already there, so a connection between signals never invents a
    // node of its own. That spreads a step at a time, so it goes round
    // until nothing more joins.
    let members: Vec<String> = nodes
        .iter()
        .filter_map(|node| node.rsplit_once('.').map(|(_, member)| member.to_string()))
        .fold(Vec::new(), |mut seen, member| {
            if !seen.contains(&member) {
                seen.push(member);
            }
            seen
        });
    loop {
        let mut joined = false;
        for (a, b) in connects {
            for member in &members {
                let (left, right) = (format!("{a}.{member}"), format!("{b}.{member}"));
                match (nodes.contains(&left), nodes.contains(&right)) {
                    (true, false) => {
                        nodes.push(right);
                        joined = true;
                    }
                    (false, true) => {
                        nodes.push(left);
                        joined = true;
                    }
                    _ => {}
                }
            }
        }
        if !joined {
            break;
        }
    }
    for (a, b) in connects {
        if nodes.contains(a) && nodes.contains(b) {
            edges.push((a.clone(), b.clone()));
            continue;
        }
        for member in &members {
            let (left, right) = (format!("{a}.{member}"), format!("{b}.{member}"));
            if nodes.contains(&left) && nodes.contains(&right) {
                edges.push((left, right));
            }
        }
    }

    let mut parent: Vec<usize> = (0..nodes.len()).collect();
    fn root_of(parent: &mut Vec<usize>, index: usize) -> usize {
        if parent[index] != index {
            let found = root_of(parent, parent[index]);
            parent[index] = found;
        }
        parent[index]
    }
    // Every edge names nodes of the graph: a branch puts its ends
    // there, and a connection counts only when both ends are already in.
    let at = |name: &String| {
        nodes
            .iter()
            .position(|known| known == name)
            .expect("edges name nodes of the graph")
    };
    for (a, b) in &edges {
        let (ra, rb) = (root_of(&mut parent, at(a)), root_of(&mut parent, at(b)));
        if ra != rb {
            parent[ra] = rb;
        }
    }

    let mut chosen: HashMap<String, bool> =
        nodes.iter().map(|node| (node.clone(), false)).collect();
    let mut parts: HashMap<usize, Vec<usize>> = HashMap::new();
    for index in 0..nodes.len() {
        let part = root_of(&mut parent, index);
        parts.entry(part).or_default().push(index);
    }
    let mut ordered: Vec<&Vec<usize>> = parts.values().collect();
    ordered.sort_by_key(|part| nodes[part[0]].clone());
    for part in ordered {
        let declared: Vec<&str> = part
            .iter()
            .filter(|index| {
                clauses.iter().any(
                    |clause| matches!(clause, GraphClause::Root(node) if node == &nodes[**index]),
                )
            })
            .map(|index| nodes[*index].as_str())
            .collect();
        if declared.len() > 1 {
            return Err(format!(
                "the connection graph holding {declared:?} has more than one root"
            ));
        }
        if let Some(root) = declared.first() {
            chosen.insert((*root).to_string(), true);
            continue;
        }
        // No declared root: the best potential one takes the part.
        let mut candidates: Vec<(i64, &str)> = part
            .iter()
            .filter_map(|index| {
                clauses.iter().find_map(|clause| match clause {
                    GraphClause::PotentialRoot(node, priority) if node == &nodes[*index] => {
                        Some((*priority, node.as_str()))
                    }
                    _ => None,
                })
            })
            .collect();
        candidates.sort();
        match candidates.first() {
            Some((_, root)) => {
                chosen.insert((*root).to_string(), true);
            }
            None => {
                return Err(format!(
                    "the connection graph holding `{}` has no root, so nothing says what the rest of it is measured against",
                    nodes[part[0]]
                ))
            }
        }
    }
    Ok(chosen)
}

/// Answer `Connections.isRoot` and `Connections.rooted` from the roots
/// that were chosen.
pub(super) fn answer_graph_queries(
    expr: &Expr,
    roots: &HashMap<String, bool>,
    connected: &HashMap<String, f64>,
) -> Expr {
    let recur = |e: &Expr| answer_graph_queries(e, roots, connected);
    match expr {
        Expr::Call(name, args)
            if (name == "Connections.isRoot" || name == "Connections.rooted")
                && args.len() == 1 =>
        {
            match &args[0] {
                Expr::Ref(node) => Expr::Bool(roots.get(node).copied().unwrap_or(false)),
                _ => Expr::Call(name.clone(), args.iter().map(recur).collect()),
            }
        }
        // `cardinality(c)` is how many `connect` equations name `c`,
        // and every one of them is already in hand. The specification
        // deprecates the operator and says it will be removed; it is
        // answered here because the specification still defines it.
        Expr::Call(name, args) if name == "cardinality" && args.len() == 1 => {
            // The count is kept under the name the array layer makes,
            // `inPort[1]`, and a condition that has not been through
            // that layer still holds the subscript apart.
            let port = match &args[0] {
                Expr::Ref(port) => Some(port.clone()),
                Expr::Index(inner, subscripts) => match (&**inner, subscripts.as_slice()) {
                    (Expr::Ref(port), [Expr::Number(at)]) => {
                        Some(format!("{port}[{}]", *at as i64))
                    }
                    _ => None,
                },
                _ => None,
            };
            match port {
                Some(port) => Expr::Number(connected.get(&port).copied().unwrap_or(0.0)),
                None => Expr::Call(name.clone(), args.iter().map(recur).collect()),
            }
        }
        // Everything else holds expressions and nothing this pass has
        // an opinion about, so the questions inside are answered
        // wherever they sit. The walk used to stop at the array forms,
        // which meant a `cardinality` written inside one went
        // unanswered; reaching them changes nothing for the models
        // that do not, and answers the question for the models that
        // do.
        _ => expr.map_children(&mut |child| recur(child)),
    }
}
