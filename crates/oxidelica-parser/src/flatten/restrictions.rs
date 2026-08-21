//! What a class may hold, and who may reach into it.
//!
//! Two rules of the language's class restrictions, neither of which
//! changes the flat model by so much as an equation - which is exactly
//! why they are worth checking. A model that breaks one is written on
//! something the library never promised, and it will break again the
//! next time the library moves.
//!
//! A declaration under `protected` is visible inside its own class and
//! inside classes extending it, and nowhere else. A class holding a
//! component may write its public members - `r.R`, `pin.v` - and may
//! not write the ones the component's class kept back, whether by
//! naming one or by modifying it from the declaration. A member that
//! is not found where it is looked for is left alone: it may be
//! inherited, and what a base class kept back is not decided here.
//!
//! A `block` is a model whose connectors all have a direction, so that
//! what goes in and what comes out is decided by the declaration
//! rather than by what it is connected to.

use super::*;

/// Refuse any reach into what a component's class keeps to itself.
///
/// The check is of one class against the components it declares, so it
/// is made once per class rather than once per instance.
pub(super) fn nothing_reaches_inside<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    class: &ClassDef,
    imports: &[(String, String)],
) -> Result<(), String> {
    // What each component in view is of, the inherited ones included:
    // a class reaches into what it holds however the holding came
    // about. Types with no members to keep back are left out.
    let mut own: HashMap<String, String> = HashMap::new();
    with_bases(registry, class, 0, &mut |of| {
        for component in &of.components {
            if !is_primitive(&component.type_name) {
                own.entry(component.name.clone())
                    .or_insert_with(|| component.type_name.clone());
            }
        }
    });
    if own.is_empty() {
        return Ok(());
    }
    // The class a component is of, looked up once however many times
    // it is reached into.
    let mut of: HashMap<String, Option<&'a ClassDef>> = HashMap::new();
    let mut kept_back = |component: &str, member: &str| -> bool {
        let Some(type_name) = own.get(component) else {
            return false;
        };
        let found = of
            .entry(component.to_string())
            .or_insert_with(|| lookup(registry, type_name, &class.name, imports));
        let Some(found) = *found else { return false };
        // What the component's class keeps back, and what the classes
        // it extends keep back: inheriting a declaration does not
        // publish it.
        let mut kept = false;
        with_bases(registry, found, 0, &mut |step| {
            kept = kept
                || step
                    .components
                    .iter()
                    .any(|one| one.name == member && one.protected);
        });
        kept
    };

    // Naming the member: `m.inertia` anywhere a value is written.
    let mut reached = None;
    every_value(class, &mut |expr| {
        if reached.is_none() {
            reached = reaching(expr, &mut kept_back);
        }
    });

    // Modifying it from the declaration: `Motor m(inertia = 2)`.
    for component in &class.components {
        for (name, _) in &component.modifiers {
            // A modifier path reaches the component's own member
            // first; what is written under that member is the member's
            // business.
            if reached.is_none() && kept_back(&component.name, head(name)) {
                reached = Some((component.name.clone(), head(name).to_string()));
            }
        }
    }

    match reached {
        None => Ok(()),
        Some((component, member)) => Err(format!(
            "`{component}.{member}` reaches a `protected` declaration of `{}`, which \
             is visible only inside that class and the classes extending it",
            own[&component]
        )),
    }
}

/// Refuse a `block` that holds a connector nothing gave a direction.
///
/// A `block` is a model whose every connector is causal: what goes in
/// and what comes out is decided by the declaration rather than by
/// what it is connected to. The direction may be written on the
/// component, on the short definition its connector came from - the
/// standard library's `connector RealInput = input Real` - or on
/// every variable the connector declares.
pub(super) fn every_connector_of_a_block_is_causal(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
) -> Result<(), String> {
    for component in &class.components {
        if component.causality != Causality::None {
            continue;
        }
        let Some(of) = lookup(registry, &component.type_name, &class.name, &class.imports) else {
            continue;
        };
        // An `expandable connector` is a bus: what it carries is put
        // there by the connections that name it, and the direction
        // comes with it. There is nothing here to read a direction off.
        if of.kind != ClassKind::Connector || of.alias_causality != Causality::None || of.expandable
        {
            continue;
        }
        // A connector built of variables of its own is causal where
        // every one of them says which way it goes. A parameter is not
        // a signal and says nothing either way, and neither does an
        // empty connector.
        let signals: Vec<&Component> = of
            .components
            .iter()
            .filter(|inside| {
                matches!(
                    inside.variability,
                    Variability::Continuous | Variability::Discrete
                )
            })
            .collect();
        if !signals.is_empty()
            && signals
                .iter()
                .all(|inside| inside.causality != Causality::None)
        {
            continue;
        }
        return Err(format!(
            "`{}.{}` is a connector of a `block` and nothing says which way it goes; \
             a block's connectors are `input` or `output`, written on the declaration, \
             on the connector or on every variable it holds",
            class.name, component.name
        ));
    }
    Ok(())
}

/// Refuse a `block` that leaves one of its own outputs to somebody
/// else to work out.
///
/// A block is locally balanced: an input comes from outside and an
/// output is settled inside, so a block declaring an output that
/// nothing in it ever mentions is an interface being used as though it
/// were finished. Whether the model around it happens to balance out
/// is beside the point - the block is what does not.
///
/// A `partial` block is exactly the case where the outputs are left
/// for whoever extends it, and is not asked.
///
/// What counts as mentioning is wide on purpose: a name written
/// anywhere in an equation, a connection, an algorithm or a
/// declaration value of the block or of anything it extends. Nothing
/// here works out which side of an equation settles what, so the only
/// thing refused is an output nobody speaks of at all.
pub(super) fn every_output_of_a_block_is_settled_inside(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
) -> Result<(), String> {
    if class.partial {
        return Ok(());
    }
    let mut outputs: Vec<(String, String)> = Vec::new();
    let mut spoken = HashSet::new();
    let mut ask = |of: &ClassDef| {
        for component in &of.components {
            let causal = component.causality == Causality::Output
                || lookup(registry, &component.type_name, &of.name, &of.imports)
                    .is_some_and(|found| found.alias_causality == Causality::Output);
            if causal && component.condition.is_none() {
                outputs.push((component.name.clone(), of.name.clone()));
            }
            if component.binding.is_some() {
                spoken.insert(component.name.clone());
            }
        }
        // `extends Base(y = 2 * u)` settles an inherited output from
        // the declaration that inherits it.
        for extend in &of.extends {
            for (name, _) in &extend.modifiers {
                spoken.insert(head(name).to_string());
            }
        }
        every_value(of, &mut |expr| {
            let mut names = Vec::new();
            expr.collect_refs(&mut names);
            spoken.extend(names.into_iter().map(|name| head(name).to_string()));
        });
    };
    with_bases(registry, class, 0, &mut ask);

    match outputs.iter().find(|(name, _)| !spoken.contains(name)) {
        None => Ok(()),
        Some((name, of)) => Err(format!(
            "`{of}.{name}` is an output of a `block` and nothing in the block settles it; \
             a block's outputs are worked out inside it and its inputs come from outside, \
             which is what makes it a block rather than a model"
        )),
    }
}

/// The name a dotted or subscripted reference starts from.
fn head(name: &str) -> &str {
    let name = name.split('.').next().unwrap_or(name);
    name.split('[').next().unwrap_or(name)
}

/// Visit a class and every class it extends, once each.
fn with_bases(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    depth: usize,
    visit: &mut impl FnMut(&ClassDef),
) {
    visit(class);
    if depth > MAX_DEPTH {
        return;
    }
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, &class.name, &class.imports) {
            with_bases(registry, base, depth + 1, visit);
        }
    }
}

/// Every value a class writes: both sides of its equations, what its
/// connections join, the targets and values of its statements, the
/// conditions everything hangs on, and the values of its declarations.
///
/// One walk answers both questions here. What may not be reached needs
/// every place a name can be written; what settles an output is
/// forgiving by design, and a wider walk only makes it more so.
fn every_value(class: &ClassDef, look: &mut impl FnMut(&Expr)) {
    for equation in class.equations.iter().chain(class.initial_equations.iter()) {
        look(&equation.lhs);
        look(&equation.rhs);
    }
    for (left, right) in &class.connects {
        look(left);
        look(right);
    }
    for (condition, _) in &class.asserts {
        look(condition);
    }
    for call in &class.calls {
        look(call);
    }
    for loop_ in &class.for_equations {
        bodies(loop_, look);
    }
    for branch in class.if_equations.iter().flat_map(|it| &it.branches) {
        branches(branch, look);
    }
    for clause in &class.when_clauses {
        when(clause, look);
    }
    for statement in class.algorithm.iter().chain(class.initial_algorithm.iter()) {
        written(statement, look);
    }
    for component in &class.components {
        for said in [
            &component.binding,
            &component.condition,
            &component.start,
            &component.min,
            &component.max,
        ]
        .into_iter()
        .flatten()
        .chain(&component.dimensions)
        {
            look(said);
        }
        for (_, value) in &component.modifiers {
            look(value);
        }
    }
}

/// The same, for a `for` equation and everything in its body.
fn bodies(loop_: &ForEquation, look: &mut impl FnMut(&Expr)) {
    if let Some(range) = &loop_.range {
        look(range);
    }
    for item in &loop_.body {
        match item {
            ForBody::Equation(equation) => {
                look(&equation.lhs);
                look(&equation.rhs);
            }
            ForBody::Connect(left, right) => {
                look(left);
                look(right);
            }
            ForBody::Assert(condition, _) => look(condition),
            ForBody::Nested(inside) => bodies(inside, look),
            ForBody::Branch(branch) => branch.branches.iter().for_each(|it| branches(it, look)),
        }
    }
}

/// The same, for one branch of an `if` equation.
fn branches(branch: &IfBranch, look: &mut impl FnMut(&Expr)) {
    if let Some(condition) = &branch.condition {
        look(condition);
    }
    for equation in &branch.equations {
        look(&equation.lhs);
        look(&equation.rhs);
    }
    for (left, right) in &branch.connects {
        look(left);
        look(right);
    }
    for (condition, _) in &branch.asserts {
        look(condition);
    }
    for call in &branch.calls {
        look(call);
    }
    for loop_ in &branch.loops {
        bodies(loop_, look);
    }
    for clause in &branch.whens {
        when(clause, look);
    }
}

/// The same, for what a `when` clause fires on and does.
fn when(clause: &WhenClause, look: &mut impl FnMut(&Expr)) {
    for branch in &clause.branches {
        look(&branch.condition);
        for action in &branch.actions {
            match action {
                WhenAction::Assign(name, value) | WhenAction::Reinit(name, value) => {
                    look(&Expr::Ref(name.clone()));
                    look(value);
                }
                WhenAction::TupleAssign(targets, value) => {
                    targets
                        .iter()
                        .flatten()
                        .for_each(|name| look(&Expr::Ref(name.clone())));
                    look(value);
                }
                WhenAction::Loop(loop_) => bodies(loop_, look),
                WhenAction::Choice(taken) => {
                    taken.branches.iter().for_each(|it| branches(it, look))
                }
                WhenAction::Terminate(_) => {}
            }
        }
    }
}

/// The same, for a statement: what it assigns and what it assigns it.
fn written(statement: &Statement, look: &mut impl FnMut(&Expr)) {
    let mut target = |name: &str, subscripts: &[Expr]| {
        look(&Expr::Ref(name.to_string()));
        subscripts.iter().for_each(&mut *look);
    };
    match statement {
        Statement::Assign(name, subscripts, value) => {
            target(name, subscripts);
            look(value);
        }
        Statement::TupleAssign(targets, value) => {
            for (name, subscripts) in targets.iter().flatten() {
                target(name, subscripts);
            }
            look(value);
        }
        Statement::If(taken) | Statement::When(taken) => {
            for branch in taken {
                if let Some(condition) = &branch.condition {
                    look(condition);
                }
                branch.body.iter().for_each(|inside| written(inside, look));
            }
        }
        Statement::For(_, range, body) => {
            if let Some(range) = range {
                look(range);
            }
            body.iter().for_each(|inside| written(inside, look));
        }
        Statement::While(condition, body) => {
            look(condition);
            body.iter().for_each(|inside| written(inside, look));
        }
        Statement::Assert(condition, _) => look(condition),
        Statement::Call(_, args) => args.iter().for_each(&mut *look),
        Statement::Break | Statement::Return => {}
    }
}

/// The first reach into something kept back that this expression
/// makes, as the component reached into and the member named.
fn reaching(
    expr: &Expr,
    kept_back: &mut impl FnMut(&str, &str) -> bool,
) -> Option<(String, String)> {
    let mut found: Option<(String, String)> = None;
    expr.for_each(&mut |inside| {
        // `m.inertia` is one name with a dot in it; `m[1].inertia` is
        // a member of something subscripted, and both reach the same
        // way.
        let reach = match inside {
            Expr::Ref(name) => name
                .split_once('.')
                .map(|(of, member)| (of, member.split('.').next().unwrap_or(member))),
            Expr::Member(base, member) => {
                let mut base = base.as_ref();
                while let Expr::Index(of, _) = base {
                    base = of.as_ref();
                }
                match base {
                    Expr::Ref(name) if !name.contains('.') => {
                        Some((name.as_str(), member.as_str()))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some((of, member)) = reach {
            if found.is_none() && kept_back(of, member) {
                found = Some((of.to_string(), member.to_string()));
            }
        }
    });
    found
}
