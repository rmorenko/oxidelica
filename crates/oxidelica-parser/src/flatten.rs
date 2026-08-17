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
    let scope = scope_of(&class.name);
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
        let class_name = acc.connectors[members[0]].clone();
        if members.iter().any(|m| acc.connectors[*m] != class_name) {
            return Err(format!(
                "connection set {members:?} mixes different connector classes"
            ));
        }
        let class = registry[class_name.as_str()];
        for member_component in &class.components {
            let var = |path: &str| format!("{path}.{}", member_component.name);
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

    Ok(Model {
        name: top_class.name.clone(),
        description: top_class.description.clone(),
        components: acc.components,
        equations: acc.equations,
        when_clauses: acc.when_clauses,
        experiment: top_class.experiment.clone(),
    })
}

/// Accumulated flat model contents.
#[derive(Default)]
struct Flat {
    components: Vec<Component>,
    equations: Vec<EquationItem>,
    when_clauses: Vec<WhenClause>,
    /// Connector instance path -> connector class name.
    connectors: HashMap<String, String>,
    /// Connect statements with fully prefixed paths.
    connects: Vec<(String, String)>,
    /// Values of parameters already instantiated, by flat name: array
    /// dimensions and loop bounds are resolved against them.
    const_values: HashMap<String, f64>,
    /// Instance paths of components a false condition left out.
    disabled: Vec<String>,
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

    let scope = scope_of(&class.name);

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
            },
            registry,
            class,
            prefix,
            &outers,
        )?);
    }
    redeclares.extend(env.redeclares.iter().cloned());

    // Bases first, with their modifiers (already parent-scoped).
    for extend in &class.extends {
        let base = lookup(registry, &extend.base, scope, &class.imports)
            .ok_or_else(|| format!("unknown base class `{}`", extend.base))?;
        let mods: Vec<(String, Expr)> = extend
            .modifiers
            .iter()
            .map(|(n, e)| {
                let e = substitute_class_constants(e, registry, scope, &class.imports);
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
                            let e = substitute_class_constants(e, registry, scope, &class.imports);
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
            if value.fract() != 0.0 || value < 1.0 {
                return Err(format!(
                    "dimension of `{flat_name}` must be a positive whole number, got {value}"
                ));
            }
            sizes.push(value as i64);
        }
        let element_names: Vec<String> = if sizes.is_empty() {
            vec![component.name.clone()]
        } else {
            index_tuples(&sizes)
                .into_iter()
                .map(|indices| element_name(&component.name, &indices))
                .collect()
        };

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
        resolve_type(registry, &mut component, scope, &class.imports);

        let level = Level {
            prefix,
            outers: &outers,
            inners: &inners,
            overrides,
            consts: &local_consts,
            imports: &class.imports,
            scope,
        };
        for local_name in element_names {
            let flat_name = format!("{prefix}{local_name}");
            let site = Site {
                component: &component,
                local_name: &local_name,
                flat_name: &flat_name,
                extra_modifiers: &extra_modifiers,
                redeclares: &child_redeclares,
            };
            instantiate_one(registry, &site, &level, acc, depth)?;
        }
    }

    // Equations: subscripts resolved, function calls inlined.
    let resolve_here = |expr: &Expr| -> Result<Expr, String> {
        let expr = substitute_class_constants(expr, registry, scope, &class.imports);
        resolve(
            &prefix_expr(&expr, prefix, &outers),
            &HashMap::new(),
            &local_consts,
            registry,
            scope,
            &class.imports,
            0,
        )
    };
    for equation in &class.equations {
        acc.equations.push(EquationItem {
            lhs: resolve_here(&equation.lhs)?,
            rhs: resolve_here(&equation.rhs)?,
        });
    }

    // `for` equations are unrolled: the loop variable is a constant.
    for loop_eq in &class.for_equations {
        unroll(
            loop_eq,
            &HashMap::new(),
            &local_consts,
            prefix,
            &outers,
            registry,
            scope,
            &class.imports,
            acc,
        )?;
    }

    // `if` equations: the branch that holds contributes its equations,
    // the others contribute nothing. Conditions are structural, so they
    // must be constant at compile time.
    for if_equation in &class.if_equations {
        let mut env = acc.const_values.clone();
        env.extend(local_consts.iter().map(|(k, v)| (k.clone(), *v)));
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
            acc.equations.push(EquationItem {
                lhs: resolve_here(&equation.lhs)?,
                rhs: resolve_here(&equation.rhs)?,
            });
        }
        for (a, b) in &branch.connects {
            let (a, b) = (flat_name(a, prefix, &outers), flat_name(b, prefix, &outers));
            if acc.is_disabled(&a) || acc.is_disabled(&b) {
                continue;
            }
            acc.connects.push((a, b));
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
    for (a, b) in &class.connects {
        let (a, b) = (flat_name(a, prefix, &outers), flat_name(b, prefix, &outers));
        // A connection to a component that a condition left out goes
        // with it: this is how the standard library switches a support
        // flange between an external connector and an internal ground.
        if acc.is_disabled(&a) || acc.is_disabled(&b) {
            continue;
        }
        acc.connects.push((a, b));
    }
    Ok(())
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
    let scope = scope_of(&class.name);
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
    let scope = scope_of(&class.name);
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
    let scope = scope_of(&class.name);
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
    let scope = scope_of(&class.name);
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
    let scope = scope_of(&class.name);
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
        scope = scope_of(&class.name).to_string();
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
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    acc: &mut Flat,
) -> Result<(), String> {
    let bound = |expr: &Expr| -> Result<i64, String> {
        let mut env = consts.clone();
        env.extend(outer_vars.iter().map(|(k, v)| (k.clone(), *v)));
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
        for item in &loop_eq.body {
            match item {
                ForBody::Equation(equation) => acc.equations.push(EquationItem {
                    lhs: resolve(
                        &prefix_expr(
                            &substitute_class_constants(&equation.lhs, registry, scope, imports),
                            prefix,
                            outers,
                        ),
                        &loop_vars,
                        consts,
                        registry,
                        scope,
                        imports,
                        0,
                    )?,
                    rhs: resolve(
                        &prefix_expr(
                            &substitute_class_constants(&equation.rhs, registry, scope, imports),
                            prefix,
                            outers,
                        ),
                        &loop_vars,
                        consts,
                        registry,
                        scope,
                        imports,
                        0,
                    )?,
                }),
                ForBody::Nested(inner) => unroll(
                    inner, &loop_vars, consts, prefix, outers, registry, scope, imports, acc,
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
            flat.start = flat.start.as_ref().map(&resolve_value).transpose()?;
            flat.binding = flat.binding.as_ref().map(&resolve_value).transpose()?;
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
        Expr::Number(_) | Expr::Bool(_) | Expr::Time => expr.clone(),
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
    }
}

/// Resolve a class name the way Modelica scoping does: an import
/// alias first, then the enclosing packages from the inside out, then
/// the global name.
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

/// The package path enclosing a class: `A.B.C` lives in `A.B`.
fn scope_of(qualified_name: &str) -> &str {
    match qualified_name.rfind('.') {
        Some(cut) => &qualified_name[..cut],
        None => "",
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
                    inline_function(class, &args, consts, registry, depth + 1)?
                }
                // `noEvent(x)` and `smooth(n, x)` are hints about event
                // generation and continuity; the value is the argument.
                _ if name == "noEvent" && args.len() == 1 => args[0].clone(),
                _ if name == "smooth" && args.len() == 2 => args[1].clone(),
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
        Expr::Number(_) | Expr::Bool(_) | Expr::Time => expr.clone(),
    })
}

/// Inline a function call: arguments are bound to the inputs, the
/// algorithm's assignments are substituted in order, and the output
/// expression replaces the call.
fn inline_function(
    class: &ClassDef,
    args: &[Expr],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
) -> Result<Expr, String> {
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
    if outputs.len() != 1 {
        return Err(format!(
            "function `{}` must declare exactly one output, found {}",
            class.name,
            outputs.len()
        ));
    }
    if args.len() != inputs.len() {
        return Err(format!(
            "function `{}` expects {} argument(s), got {}",
            class.name,
            inputs.len(),
            args.len()
        ));
    }
    let mut bindings: HashMap<String, Expr> = inputs
        .iter()
        .map(|c| c.name.clone())
        .zip(args.iter().cloned())
        .collect();
    for component in &class.components {
        if component.causality == Causality::None {
            if let Some(binding) = &component.binding {
                bindings.insert(component.name.clone(), binding.clone());
            }
        }
    }
    for assignment in &class.algorithm {
        let value = substitute_refs(&assignment.value, &bindings);
        let value = resolve(
            &value,
            &HashMap::new(),
            consts,
            registry,
            scope_of(&class.name),
            &class.imports,
            depth + 1,
        )?;
        bindings.insert(assignment.target.clone(), value);
    }
    bindings.get(&outputs[0].name).cloned().ok_or_else(|| {
        format!(
            "function `{}` never assigns its output `{}`",
            class.name, outputs[0].name
        )
    })
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
        // Mixing connector classes in one set.
        assert!(parse_model(
            "connector A Real v; flow Real i; end A;             connector B Real v; flow Real i; end B;             model U A p; end U; model W B p; end W;             model M U u; W w; equation connect(u.p, w.p); end M;"
        )
        .unwrap_err()
        .to_string()
        .contains("mixes different connector classes"));
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
        // A non-positive dimension.
        assert!(err("model M Real v[0]; end M;").contains("positive whole number"));
        // A subscript that cannot be folded.
        assert!(
            err("model M Real v[2]; Real k; equation k = 1; v[1] = 0; v[2] = v[k]; end M;")
                .contains("not constant")
        );
        // A subscript out of the whole-number domain.
        assert!(
            err("model M Real v[2]; equation v[1] = 0; v[2] = v[0]; end M;")
                .contains("positive whole number")
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
        .contains("exactly one output"));
        assert!(err(
            "function f input Real a; output Real b; output Real c; algorithm b := a; end f;\
             model M Real y; equation y = f(1); end M;"
        )
        .contains("exactly one output"));
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

        // A condition that is not constant cannot select equations.
        let error = crate::parser::parse_model(
            "model M Real gate; Real y; equation gate = time; \
             if gate > 0 then y = 1; else y = 2; end if; end M;",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("not a compile-time constant"), "{error}");
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
