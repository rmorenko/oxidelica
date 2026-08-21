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
    // What each component of this class is of, minus the ones whose
    // type has no members to keep back.
    let mut own: HashMap<&str, &str> = HashMap::new();
    for component in &class.components {
        if !is_primitive(&component.type_name) {
            own.insert(component.name.as_str(), component.type_name.as_str());
        }
    }
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
        of.entry(component.to_string())
            .or_insert_with(|| lookup(registry, type_name, &class.name, imports))
            .is_some_and(|found| {
                found
                    .components
                    .iter()
                    .any(|one| one.name == member && one.protected)
            })
    };

    // Naming the member: `m.inertia` anywhere a value is written.
    let mut reached = None;
    let mut look = |expr: &Expr| {
        if reached.is_none() {
            reached = reaching(expr, &mut kept_back);
        }
    };
    for equation in class.equations.iter().chain(class.initial_equations.iter()) {
        look(&equation.lhs);
        look(&equation.rhs);
    }
    for (left, right) in &class.connects {
        look(left);
        look(right);
    }
    for component in &class.components {
        for said in [&component.binding, &component.condition]
            .into_iter()
            .flatten()
        {
            look(said);
        }
    }

    // Modifying it from the declaration: `Motor m(inertia = 2)`.
    for component in &class.components {
        for (name, _) in &component.modifiers {
            // A modifier path reaches the component's own member
            // first; what is written under that member is the member's
            // business.
            let member = name.split('.').next().unwrap_or(name);
            if reached.is_none() && kept_back(&component.name, member) {
                reached = Some((component.name.clone(), member.to_string()));
            }
        }
    }

    match reached {
        None => Ok(()),
        Some((component, member)) => Err(format!(
            "`{component}.{member}` reaches a `protected` declaration of `{}`, which \
             is visible only inside that class and the classes extending it",
            own[component.as_str()]
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
