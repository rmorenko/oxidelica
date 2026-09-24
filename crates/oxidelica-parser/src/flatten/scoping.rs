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
    prefix: &str,
    outers: &HashMap<String, String>,
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
                for held in
                    effective_imports(registry, base, scope, redeclares, prefix, outers, depth + 1)?
                {
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
            // The target is written in the terms of the class that
            // wrote the alias, not of whoever is using it: a pump
            // says `replaceable function flowCharacteristic =
            // PumpCharacteristics.baseFlow`, and that package is a
            // neighbour of the pump rather than of the model built
            // from it. So the class's own scope answers first, and
            // the site's after - which is what keeps an alias
            // written against a name in view at the site working.
            None => lookup(registry, &alias.target, class.name.as_str(), &imports)
                .or_else(|| lookup(registry, &alias.target, scope, &imports))
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
        // What a redeclaration wrote on its target is remembered
        // against the name it gave: `redeclare function
        // flowCharacteristic = quadraticFlow(V_flow_nominal = ...)`
        // fills in some of that function's inputs, and the call the
        // model writes gives only the rest. The alias itself is a
        // pair of names and has nowhere to carry them.
        //
        // What survives flattening carries the flat model's names, and
        // who wrote the modifier decides which names those are. A
        // redeclaration was qualified where it was written - the
        // model that said `pump(redeclare function f = g(a = k))`
        // means its own `k`, and `extends Base(redeclare function f =
        // g(a = k))` means the extending class's - so it already
        // wears the flat name and is taken as it stands. Prefixed a
        // second time it named `pump.pump.k`, which nothing declares,
        // and every controlled pump of the fluid library lost its
        // characteristic that way. The alias's own modifiers were
        // written by this class: `function accel = Scaled(c = k)` in
        // a component means *that component's* `k`, and only those
        // take the prefix here.
        let filled = match replacement {
            Some(held) if !held.modifiers.is_empty() => Some(
                held.modifiers
                    .iter()
                    .map(|(name, value)| match prefix_twice() {
                        true => (name.clone(), prefix_expr(value, prefix, outers)),
                        false => (name.clone(), value.clone()),
                    })
                    .collect(),
            ),
            _ if !alias.modifiers.is_empty() => Some(
                alias
                    .modifiers
                    .iter()
                    .map(|(name, value)| (name.clone(), prefix_expr(value, prefix, outers)))
                    .collect(),
            ),
            _ => None,
        };
        if let Some(filled) = filled {
            super::statements::remember_filled_inputs(&target, filled);
        }
        // What the alias itself wrote, kept apart from what a
        // redeclaration wrote: `package Medium = MoistAir(
        // extraPropertiesNames = {"CO2"})` gives a constant of that
        // package a value, and the package's own gathering is where
        // that belongs. A redeclaration is a different statement -
        // it replaces a class a base declared - and its modifiers
        // stay on the road above, which is where a call reads them.
        // Only where the alias still stands as it was written. A
        // redeclaration replaces the whole alias - target and
        // modifiers together - so what the replaced alias wrote is
        // not said about the class that took its place: `Medium =
        // Water(rho = 1)` redeclared to `Oil` leaves `Oil` with the
        // `rho` it declares for itself.
        if replacement.is_none() && !alias.modifiers.is_empty() {
            super::statements::remember_alias_modifiers(&target, alias.modifiers.clone());
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
                modifiers: component.modifiers.clone(),
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
    // A constant minted as a name of the flat model is a whole name
    // already - `Modelica.Media...cp_const`, one per medium rather
    // than one per instance - so an instance path on the front of it
    // would name something nothing declares.
    //
    // Judged here, at the top, rather than where a name with no dot
    // in it is prefixed: a minted name is all dots, so the walk down
    // the leads never reaches that arm. And judged cheaply - nothing
    // minted at all, or no dot in the name - because this is asked of
    // every name the flattener writes.
    if super::constants::is_minted(name) {
        return name.to_string();
    }
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
    // The names in force where this walk started: a component's
    // `redeclare package Medium = Medium` is written in the terms of
    // the class holding it, and what it says has to stay in front of
    // whatever the classes below bring with them. Without it, the
    // walk reads a base under the base's own names alone and lands on
    // the interface's empty record.
    let handed: Vec<(String, String)> = imports
        .iter()
        .filter(|(local, _)| local != "*")
        .cloned()
        .collect();
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, scope, imports) {
            // The base step under the names the class above had: a
            // `Medium.ThermodynamicState` two `extends` below the
            // class that named the medium means the medium that
            // class named.
            let mut below = handed.clone();
            for held in &base.imports {
                if !below.iter().any(|(local, _)| *local == held.0) {
                    below.push(held.clone());
                }
            }
            collect_records(registry, base, prefix, &base.name, &below, out, depth + 1);
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
        // A record kept empty in the interface and filled by the
        // medium - `ThermodynamicState`, whose fields `PartialMedium`
        // leaves blank and `PartialSimpleMedium` states - resolves to
        // the interface's own, which has no fields at all. Filed under
        // that, the table says `medium.state` is a record of nothing,
        // an equation between two such records is an equation between
        // two empty lists, and the value a modifier gave it is dropped
        // without a word. Instantiation already asks the question the
        // other way round; the table has to agree with it, or the
        // fields exist and nothing determines them.
        let of = match of.kind == ClassKind::Record {
            true => {
                let _asked = inlining::AskedAs::resolving(
                    &component.type_name,
                    of,
                    registry,
                    scope,
                    imports,
                );
                inlining::record_asked_under(of, registry)
            }
            false => of,
        };
        if of.kind == ClassKind::Record {
            out.insert(name.clone(), of.name.clone());
        }
        if matches!(
            of.kind,
            ClassKind::Record | ClassKind::Model | ClassKind::Block | ClassKind::Connector
        ) {
            let below = format!("{name}.");
            // The medium the declaration named, held while the tree
            // below it is walked. `Medium.BaseProperties` resolves to
            // the interface that declares it, and the `state` inside
            // that base is written with no path at all - so without
            // the name it was reached by, the walk lands on the
            // interface's empty record and files a state of no
            // fields. This is the same mark instantiation sets for
            // the same reason, one layer down.
            let _asked =
                inlining::AskedAs::resolving(&component.type_name, of, registry, scope, imports);
            // The component step, the same way: what the holding
            // class knows a name to mean outranks what the component's
            // own class does.
            let mut names: Vec<(String, String)> = Vec::new();
            for held in component.redeclares.iter().filter(|r| r.class_level) {
                if let Some(found) = lookup(registry, &held.type_name, scope, imports) {
                    names.push((held.name.clone(), found.name.clone()));
                }
            }
            for held in handed.iter().chain(of.imports.iter()) {
                if !names.iter().any(|(local, _)| *local == held.0) {
                    names.push(held.clone());
                }
            }
            collect_records(registry, of, &below, &of.name, &names, out, depth + 1);
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

/// The `outer` declarations under a class that no `inner` above them
/// answers, each with the class it was written against.
///
/// A helper of a library - `Utilities.ImpureRandom`, `BaseClasses.
/// TankWithTopPorts` - is written to sit inside a model that holds the
/// shared instance, and carries `outer GlobalSeed globalSeed` or
/// `outer System system` on that understanding. Checked on its own it
/// has nothing above it at all, and the declaration answers to
/// nobody. The language says what to do (MLS 5.4): the missing
/// `inner` is declared at the top of the model, with its class's own
/// defaults, and a diagnostic is given.
///
/// The walk goes down through the components because the class that
/// writes the `outer` is usually not the class being checked: it is a
/// part three levels inside it.
pub(super) fn outers_with_no_inner(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    inners: &HashMap<String, InnerInstance>,
    out: &mut Vec<(String, String)>,
    seen: &mut HashSet<String>,
    depth: usize,
) {
    if depth > MAX_DEPTH || !seen.insert(class.name.clone()) {
        return;
    }
    let scope = class.name.as_str();
    // An `inner` written anywhere on the way down answers the
    // `outer`s below it, so a class that declares one takes its own
    // name out of the reckoning for the subtree beneath it.
    let mut visible = inners.clone();
    collect_inners(registry, class, "", &mut visible, 0);
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, scope, &class.imports) {
            outers_with_no_inner(registry, base, &visible, out, seen, depth + 1);
        }
    }
    for component in &class.components {
        if component.scope == Scope::Outer {
            if visible.contains_key(&component.name)
                || out.iter().any(|(name, _)| name == &component.name)
            {
                continue;
            }
            // The class is named as the flat model will name it, so
            // that the minted declaration resolves from the top rather
            // than from wherever it was written.
            if let Some(declared) = lookup(registry, &component.type_name, scope, &class.imports) {
                out.push((component.name.clone(), declared.name.clone()));
            }
            continue;
        }
        if let Some(of) = lookup(registry, &component.type_name, scope, &class.imports) {
            outers_with_no_inner(registry, of, &visible, out, seen, depth + 1);
        }
    }
    seen.remove(&class.name);
}

/// Whether a redeclaration's filled inputs take the component prefix a
/// second time, as they did before the redeclaration was known to be
/// qualified where it was written. Kept so the corpus can be measured
/// both ways from one binary.
fn prefix_twice() -> bool {
    std::env::var_os("OXIDELICA_PREFIX_FILLED_TWICE").is_some()
}
