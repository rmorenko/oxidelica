//! What a class takes from the classes above it: its bases, the
//! declarations they hand down, and the redeclarations that replace
//! them on the way.
//!
//! Carved out of `instantiate` unchanged, so that the walk itself
//! reads as a walk.

use super::*;

/// What a class written `redeclare X extends Name` extends: the `Name`
/// that a base of the class enclosing it declared.
///
/// Looking the name up the ordinary way finds the class doing the
/// redeclaring, since that is what it is called; what is wanted is the
/// one it replaces, and that lives in a base of the class it is
/// written in.
pub(super) fn inherited_class<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    class: &ClassDef,
    wanted: &str,
    depth: usize,
) -> Option<&'a ClassDef> {
    if depth > MAX_DEPTH {
        return None;
    }
    let (enclosing, _) = class.name.rsplit_once('.')?;
    let owner = registry.get(enclosing)?;
    from_bases(registry, owner, wanted, &class.name, depth)
}

/// The class of a name declared by a base of `owner`, or by a base of
/// one of those, skipping the class that is asking.
fn from_bases<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    owner: &ClassDef,
    wanted: &str,
    asking: &str,
    depth: usize,
) -> Option<&'a ClassDef> {
    if depth > MAX_DEPTH {
        return None;
    }
    for extend in &owner.extends {
        let Some(base) = lookup(registry, &extend.base, &owner.name, &owner.imports) else {
            continue;
        };
        if let Some(found) =
            lookup(registry, wanted, &base.name, &base.imports).filter(|found| found.name != asking)
        {
            return Some(found);
        }
        if let Some(found) = from_bases(registry, base, wanted, asking, depth + 1) {
            return Some(found);
        }
    }
    None
}

/// The parameters and constants a class inherits, each with what the
/// `extends` clause that brought it said about its value.
///
/// A dimension may be written on one of them - `extends TwoPlug` brings
/// `m`, and `parameter Voltage V[m]` is written with it - so they have
/// to be worth a number here, before the dimensions are counted.
pub(super) fn inherited_parameters(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    depth: usize,
) -> Vec<(Component, Option<Expr>)> {
    let mut out = Vec::new();
    if depth > MAX_DEPTH {
        return out;
    }
    for extend in &class.extends {
        // A `redeclare function extends f` names its base by the name
        // it has itself, and that base belongs to the enclosing class:
        // looked for from inside, the search finds this very class and
        // walks in a circle. Where that happens there is no base.
        let base = match extend.from_base {
            true => inherited_class(registry, class, &extend.base, 0),
            false => lookup(registry, &extend.base, &class.name, &class.imports),
        }
        .filter(|found| found.name != class.name);
        let Some(base) = base else {
            continue;
        };
        for component in &base.components {
            if !matches!(
                component.variability,
                Variability::Parameter | Variability::Constant
            ) {
                continue;
            }
            let said = extend
                .modifiers
                .iter()
                .find(|(name, _)| name == &component.name)
                .map(|(_, value)| value.clone());
            out.push((component.clone(), said));
        }
        out.extend(inherited_parameters(registry, base, depth + 1));
    }
    out
}

/// Whether `candidate` is `target` or extends it, directly or not.
pub(super) fn extends_class(
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
/// `handed` says what the names in view already stand for. A class
/// handing its own replaceable package on - `Port one(redeclare package
/// Medium = Medium)`, which is how every fluid component passes the
/// medium to its ports - names the package by the name it has here, and
/// here it has already been replaced. Looked up among the class's own
/// imports the name is still the interface, so the child was handed the
/// interface and read its constants: a medium carrying no trace
/// substances, and a connector sized by that count a run of nothing.
pub(super) fn qualify_redeclare(
    redeclare: &Redeclare,
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    outers: &HashMap<String, String>,
    handed: &[(String, String)],
) -> Result<Redeclare, String> {
    let scope = class.name.as_str();
    let class_imports: Vec<(String, String)> = handed
        .iter()
        .cloned()
        .chain(class.imports.iter().cloned())
        .collect();
    let target =
        lookup(registry, &redeclare.type_name, scope, &class_imports).ok_or_else(|| {
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
                let e = substitute_class_constants(e, registry, scope, &class_imports, &[]);
                (n.clone(), prefix_expr(&e, prefix, outers))
            })
            .collect(),
    })
}

/// Check that a declaration may be replaced by a redeclaration: it must
/// be `replaceable`, and the new type must meet the `constrainedby`
/// interface where one is given.
pub(super) fn check_redeclare(
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
pub(super) fn resolve_type(
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
                "min" => {
                    if component.min.is_none() {
                        component.min = Some(value.clone());
                    }
                    false
                }
                "max" => {
                    if component.max.is_none() {
                        component.max = Some(value.clone());
                    }
                    false
                }
                _ => true,
            });
    }
    let mut scope = scope.to_string();
    let mut imports = imports.to_vec();
    let mut followed = false;
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
            // The name a chain of aliases ends at was written in the
            // terms of whoever wrote the last one - `record
            // SalientInductance = Salient(...)` names a record beside
            // it - and whoever asked is somewhere else entirely. So
            // once an alias has been followed, what comes back is the
            // class's own full name. A name that was never an alias is
            // left as it was written, since the way it was reached -
            // an import through an encapsulated wall - may be the only
            // way there is.
            if followed {
                component.type_name = class.name.clone();
            }
            return;
        };
        followed = true;
        component.type_name = base;
        // A type that is an array gives its dimensions to whatever is
        // declared with it, after that declaration's own: `Orientation
        // o[2]` with `type Orientation = Real[4]` is `[2, 4]`.
        component
            .dimensions
            .extend(class.alias_dimensions.iter().cloned());
        if component.unit.is_none() {
            component.unit = class.alias_unit.clone();
        }
        for (name, value) in attributes {
            match name.as_str() {
                "start" if component.start.is_none() => component.start = Some(value),
                "fixed" if component.fixed.is_none() => {
                    component.fixed = Some(matches!(value, Expr::Bool(true)))
                }
                "min" if component.min.is_none() => component.min = Some(value),
                "max" if component.max.is_none() => component.max = Some(value),
                _ => {}
            }
        }
        // The next alias in the chain resolves where it was written.
        scope = class.name.clone();
        imports = class.imports.clone();
    }
}

/// Whether a class is a handle to something outside Modelica: itself
/// `ExternalObject`, or built on one.
pub(super) fn descends_from_external_object(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    depth: usize,
) -> bool {
    if class.name == "ExternalObject" {
        return true;
    }
    if depth > MAX_DEPTH {
        return false;
    }
    class.extends.iter().any(|extend| {
        lookup(registry, &extend.base, &class.name, &class.imports)
            .is_some_and(|base| descends_from_external_object(registry, base, depth + 1))
    })
}
