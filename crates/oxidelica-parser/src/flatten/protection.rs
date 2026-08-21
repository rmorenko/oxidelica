//! What a `protected` section keeps to itself.
//!
//! A declaration under `protected` is visible inside its own class and
//! inside classes extending it, and nowhere else. A class holding a
//! component may write its public members - `r.R`, `pin.v` - and may
//! not write the ones the component's class kept back.
//!
//! Nothing about the flat model turns on this, which is exactly why it
//! is worth checking: a model that reaches inside a component is
//! reading something the library never promised to keep, and it will
//! break the next time the library moves it. Saying so at the point of
//! the reach is the whole of what this module does.
//!
//! Two ways of reaching are refused: naming the member, and modifying
//! it from the declaration - `Motor m(inertia = 2)` where the motor
//! keeps `inertia` to itself. A member that is not found where it is
//! looked for is left alone: it may be inherited, and what a base
//! class kept back is not decided here.

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
