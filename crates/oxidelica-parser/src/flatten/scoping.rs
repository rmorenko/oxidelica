//! Where a name means what: the imports in force at a point, the
//! `inner` declarations an `outer` reaches up to, and the flat path a
//! nested component ends up under.
//!
//! Carved out of `instantiate` unchanged.

use super::*;

/// The imports a class resolves names through, with its class aliases
/// folded in as further entries.
///
/// `package Medium = Media.Water` makes `Medium.density` mean
/// `Media.Water.density` exactly the way `import Medium = Media.Water`
/// would, so an alias becomes an import entry. A redeclaration from the
/// environment swaps the target before that - checked against the
/// alias's `constrainedby` interface, since a replacement medium has to
/// honour the interface the component was written against.
/// `depth` counts the bases already walked, since a class may name a
/// base that leads back to it.
pub(super) fn effective_imports(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    scope: &str,
    redeclares: &[Redeclare],
    depth: usize,
) -> Result<Vec<(String, String)>, String> {
    let mut imports = class.imports.clone();
    // An alias a base declared is one this class has. `replaceable
    // package Medium` is written once in `PartialSource`, and every
    // boundary of the fluid library extends that rather than repeating
    // it - then names `Medium.AbsolutePressure` in its own
    // declarations. Read from this class's aliases alone the name
    // stands for nothing, and the type it qualifies is unknown.
    if depth <= MAX_DEPTH {
        for extend in &class.extends {
            if let Some(base) = lookup(registry, &extend.base, class.name.as_str(), &imports) {
                for held in effective_imports(registry, base, scope, redeclares, depth + 1)? {
                    if !imports.iter().any(|(local, _)| *local == held.0) {
                        imports.push(held);
                    }
                }
            }
        }
    }
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
pub(super) fn collect_inners(
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
        // A shared instance may be a plain variable as readily as a
        // class: `inner Real v` is what a set of states writes between
        // them, and it is filed under the type it was written with.
        let named = lookup(registry, &component.type_name, scope, &class.imports)
            .map(|declared| declared.name.clone())
            .unwrap_or_else(|| component.type_name.clone());
        out.insert(
            component.name.clone(),
            InnerInstance {
                path: format!("{prefix}{}", component.name),
                class: named,
            },
        );
    }
}

/// Bind the `outer` declarations of a class to the visible `inner`
/// instances, yielding the name-to-path map references go through.
pub(super) fn bind_outers(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    inners: &HashMap<String, InnerInstance>,
) -> Result<HashMap<String, String>, String> {
    bind_outers_at(registry, class, inners, 0)
}

/// See [`bind_outers`]; `depth` counts the bases already walked, since
/// a class may name a base that leads back to it.
fn bind_outers_at(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    inners: &HashMap<String, InnerInstance>,
    depth: usize,
) -> Result<HashMap<String, String>, String> {
    let scope = class.name.as_str();
    let mut outers = HashMap::new();
    if depth > MAX_DEPTH {
        return Ok(outers);
    }
    // What the class inherits it declares. `outer World world` is
    // written once in `PartialTwoFrames` and every joint of the
    // multi-body library extends it rather than repeating it, so a
    // class asked about its own components alone would say it names no
    // `outer` at all - and the equations that read `world.something`
    // would be left pointing at a variable nothing owns. The bases are
    // walked the way `collect_inners` walks them.
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, scope, &class.imports) {
            outers.extend(bind_outers_at(registry, base, inners, depth + 1)?);
        }
    }
    for component in class.components.iter().filter(|c| c.scope == Scope::Outer) {
        let inner = inners.get(&component.name).ok_or_else(|| {
            format!(
                "`outer {} {}` in `{}` has no `inner` declaration above it",
                component.type_name, component.name, class.name
            )
        })?;
        // A primitive has no class to look up, and matches by the name
        // it was written with.
        let declared = match lookup(registry, &component.type_name, scope, &class.imports) {
            Some(declared) => declared.name.clone(),
            None => component.type_name.clone(),
        };
        if !extends_class(registry, &inner.class, &declared, 0) {
            return Err(format!(
                "`outer {} {}` does not match the `inner` instance, which is a `{}`",
                component.type_name, component.name, inner.class
            ));
        }
        outers.insert(component.name.clone(), inner.path.clone());
    }
    // A class may also name an `outer` a component of it holds: a
    // composite step reads the count of active steps as
    // `innerState.stateGraphRoot.subgraphStatePort.activeSteps`, where
    // it is `innerState` that declares the `outer`. An `outer` owns no
    // variable of its own, so the name has to be answered here or not
    // at all.
    for component in class.components.iter().filter(|c| c.scope != Scope::Outer) {
        let Some(of) = lookup(registry, &component.type_name, scope, &class.imports) else {
            continue;
        };
        // What that component declares `outer` itself, one level down:
        // going further would follow a class that holds one of its own
        // kind round for ever, and a name written that deep is one the
        // instance below answers for itself.
        for held in of.components.iter().filter(|c| c.scope == Scope::Outer) {
            if let Some(inner) = inners.get(&held.name) {
                outers.insert(
                    format!("{}.{}", component.name, held.name),
                    inner.path.clone(),
                );
            }
        }
    }
    Ok(outers)
}

/// Flat name of a reference written inside a class: an `outer`
/// declaration points at the enclosing `inner` instance, everything else
/// gets the instance prefix.
pub(super) fn flat_name(name: &str, prefix: &str, outers: &HashMap<String, String>) -> String {
    // The longest lead of the name that was bound, so that a member of
    // an `outer` a component holds - `innerState.stateGraphRoot` - is
    // answered by the whole of it rather than by its first word.
    let mut head = name;
    loop {
        if let Some(path) = outers.get(head) {
            return match name[head.len()..].is_empty() {
                true => path.clone(),
                false => format!("{path}{}", &name[head.len()..]),
            };
        }
        match head.rfind('.') {
            Some(cut) => head = &head[..cut],
            None => return format!("{prefix}{name}"),
        }
    }
}

/// Which of the instances below a class are records, and of what.
///
/// An overloaded operator is chosen by the record its operands are of,
/// and an equation between records is one equation per member - both
/// need to know a record when they see one. The walk goes down through
/// the whole tree because a record is as often a member of something
/// as a component outright: a frame of a multibody model carries its
/// orientation as `frame_b.R`.
pub(super) fn collect_records(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    scope: &str,
    imports: &[(String, String)],
    out: &mut HashMap<String, String>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, scope, imports) {
            collect_records(
                registry,
                base,
                prefix,
                &base.name,
                &base.imports,
                out,
                depth + 1,
            );
        }
    }
    for component in &class.components {
        // A type may be a name for a record - `connector ComplexOutput
        // = output Complex` - and a declaration of it is a record all
        // the same.
        let mut component = component.clone();
        resolve_type(registry, &mut component, scope, imports);
        let Some(of) = lookup(registry, &component.type_name, scope, imports) else {
            continue;
        };
        let name = format!("{prefix}{}", component.name);
        if of.kind == ClassKind::Record {
            out.insert(name.clone(), of.name.clone());
        }
        if matches!(
            of.kind,
            ClassKind::Record | ClassKind::Model | ClassKind::Block | ClassKind::Connector
        ) {
            let below = format!("{name}.");
            collect_records(registry, of, &below, &of.name, &of.imports, out, depth + 1);
        }
    }
}

/// A connect side written back as the dotted name a `break
/// connect(...)` would name it by, when it is a plain reference. A
/// subscripted or otherwise compound side is left unnamed, so a break
/// only matches what it can spell.
pub(super) fn connect_side_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ref(name) => Some(name.clone()),
        Expr::Member(base, member) => Some(format!("{}.{member}", connect_side_name(base)?)),
        _ => None,
    }
}
