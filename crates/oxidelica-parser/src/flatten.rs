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

use crate::ast::*;
use std::collections::HashMap;

/// Maximum instantiation depth (guards against recursive classes).
const MAX_DEPTH: usize = 32;

/// Flatten the class named `top` into a flat model.
pub fn flatten(classes: &[ClassDef], top: &str) -> Result<Model, String> {
    let registry: HashMap<&str, &ClassDef> = classes.iter().map(|c| (c.name.as_str(), c)).collect();
    let top_class = registry
        .get(top)
        .ok_or_else(|| format!("unknown class `{top}`"))?;

    let mut acc = Flat::default();
    instantiate(&registry, top_class, "", &[], &mut acc, 0)?;

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
}

/// Instantiate `class` under `prefix` with modifier overrides.
fn instantiate(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    overrides: &[(String, Expr)],
    acc: &mut Flat,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "instantiation deeper than {MAX_DEPTH} levels at `{}` (recursive classes?)",
            class.name
        ));
    }

    // Bases first, with their modifiers (already parent-scoped).
    for extend in &class.extends {
        let base = registry
            .get(extend.base.as_str())
            .ok_or_else(|| format!("unknown base class `{}`", extend.base))?;
        let mods: Vec<(String, Expr)> = extend
            .modifiers
            .iter()
            .map(|(n, e)| (n.clone(), prefix_expr(e, prefix)))
            .chain(overrides.iter().cloned())
            .collect();
        instantiate(registry, base, prefix, &mods, acc, depth + 1)?;
    }

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
                        .map(|e| prefix_expr(e, prefix))
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

        for local_name in element_names {
            let flat_name = format!("{prefix}{local_name}");
            instantiate_one(
                registry,
                component,
                &local_name,
                &flat_name,
                prefix,
                overrides,
                &local_consts,
                acc,
                depth,
            )?;
        }
    }

    // Equations: subscripts resolved, function calls inlined.
    let resolve_here = |expr: &Expr| -> Result<Expr, String> {
        resolve(
            &prefix_expr(expr, prefix),
            &HashMap::new(),
            &local_consts,
            registry,
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
            registry,
            acc,
        )?;
    }

    for clause in &class.when_clauses {
        acc.when_clauses.push(WhenClause {
            condition: resolve_here(&clause.condition)?,
            actions: clause
                .actions
                .iter()
                .map(|action| match action {
                    WhenAction::Reinit(state, value) => Ok(WhenAction::Reinit(
                        format!("{prefix}{state}"),
                        resolve_here(value)?,
                    )),
                    WhenAction::Terminate(message) => Ok(WhenAction::Terminate(message.clone())),
                })
                .collect::<Result<Vec<_>, String>>()?,
        });
    }
    for (a, b) in &class.connects {
        acc.connects
            .push((format!("{prefix}{a}"), format!("{prefix}{b}")));
    }
    Ok(())
}

/// Unroll a `for` equation, recursing into nested loops. The loop
/// variable is a compile-time constant, so the body is emitted once per
/// value with every subscript already resolved.
fn unroll(
    loop_eq: &ForEquation,
    outer_vars: &HashMap<String, f64>,
    consts: &HashMap<String, f64>,
    prefix: &str,
    registry: &HashMap<&str, &ClassDef>,
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
                        &prefix_expr(&equation.lhs, prefix),
                        &loop_vars,
                        consts,
                        registry,
                        0,
                    )?,
                    rhs: resolve(
                        &prefix_expr(&equation.rhs, prefix),
                        &loop_vars,
                        consts,
                        registry,
                        0,
                    )?,
                }),
                ForBody::Nested(inner) => unroll(inner, &loop_vars, consts, prefix, registry, acc)?,
            }
        }
    }
    Ok(())
}

/// Instantiate one component element (a scalar, or one element of an
/// array) under `flat_name`.
#[allow(clippy::too_many_arguments)]
fn instantiate_one(
    registry: &HashMap<&str, &ClassDef>,
    component: &Component,
    local_name: &str,
    flat_name: &str,
    prefix: &str,
    overrides: &[(String, Expr)],
    local_consts: &HashMap<String, f64>,
    acc: &mut Flat,
    depth: usize,
) -> Result<(), String> {
    {
        if is_primitive(&component.type_name) {
            let mut flat = component.clone();
            flat.name = flat_name.to_string();
            flat.dimensions = Vec::new();
            flat.start = flat
                .start
                .map(|e| {
                    resolve(
                        &prefix_expr(&e, prefix),
                        &HashMap::new(),
                        local_consts,
                        registry,
                        0,
                    )
                })
                .transpose()?;
            flat.binding = flat
                .binding
                .map(|e| {
                    resolve(
                        &prefix_expr(&e, prefix),
                        &HashMap::new(),
                        local_consts,
                        registry,
                        0,
                    )
                })
                .transpose()?;
            // A parent modifier `name = expr` overrides the binding.
            if let Some((_, value)) = overrides.iter().find(|(n, _)| n == local_name) {
                flat.binding = Some(value.clone());
            }
            acc.components.push(flat);
        } else {
            let child = registry.get(component.type_name.as_str()).ok_or_else(|| {
                format!(
                    "unknown type `{}` of component `{flat_name}`",
                    component.type_name
                )
            })?;
            if child.kind == ClassKind::Connector {
                acc.connectors
                    .insert(flat_name.to_string(), child.name.clone());
            }
            // Child modifiers: own modifiers (parent scope) plus
            // matching dotted overrides from above.
            let mods: Vec<(String, Expr)> = component
                .modifiers
                .iter()
                .map(|(n, e)| (n.clone(), prefix_expr(e, prefix)))
                .collect();
            let child_prefix = format!("{flat_name}.");
            instantiate(registry, child, &child_prefix, &mods, acc, depth + 1)?;
        }
    }
    Ok(())
}

/// Prefix every component reference in an expression.
fn prefix_expr(expr: &Expr, prefix: &str) -> Expr {
    if prefix.is_empty() {
        return expr.clone();
    }
    match expr {
        Expr::Ref(name) => Expr::Ref(format!("{prefix}{name}")),
        Expr::Call(name, args) => Expr::Call(
            name.clone(),
            args.iter().map(|a| prefix_expr(a, prefix)).collect(),
        ),
        Expr::Neg(inner) => Expr::Neg(Box::new(prefix_expr(inner, prefix))),
        Expr::Not(inner) => Expr::Not(Box::new(prefix_expr(inner, prefix))),
        Expr::Bin(op, l, r) => Expr::Bin(
            *op,
            Box::new(prefix_expr(l, prefix)),
            Box::new(prefix_expr(r, prefix)),
        ),
        Expr::Rel(op, l, r) => Expr::Rel(
            *op,
            Box::new(prefix_expr(l, prefix)),
            Box::new(prefix_expr(r, prefix)),
        ),
        Expr::And(l, r) => Expr::And(
            Box::new(prefix_expr(l, prefix)),
            Box::new(prefix_expr(r, prefix)),
        ),
        Expr::Or(l, r) => Expr::Or(
            Box::new(prefix_expr(l, prefix)),
            Box::new(prefix_expr(r, prefix)),
        ),
        Expr::If(c, a, b) => Expr::If(
            Box::new(prefix_expr(c, prefix)),
            Box::new(prefix_expr(a, prefix)),
            Box::new(prefix_expr(b, prefix)),
        ),
        Expr::Index(base, subscripts) => Expr::Index(
            Box::new(prefix_expr(base, prefix)),
            subscripts.iter().map(|s| prefix_expr(s, prefix)).collect(),
        ),
        Expr::Member(base, path) => Expr::Member(Box::new(prefix_expr(base, prefix)), path.clone()),
        Expr::Number(_) | Expr::Bool(_) | Expr::Time => expr.clone(),
    }
}

/// Evaluate a compile-time constant expression (array dimensions, loop
/// bounds, subscripts). Only the arithmetic that can appear there is
/// supported; anything else means the value is not constant.
fn const_eval(expr: &Expr, env: &HashMap<String, f64>) -> Option<f64> {
    use crate::ast::BinOp::*;
    Some(match expr {
        Expr::Number(n) => *n,
        Expr::Ref(name) => *env.get(name)?,
        Expr::Neg(inner) => -const_eval(inner, env)?,
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
fn resolve(
    expr: &Expr,
    loop_vars: &HashMap<String, f64>,
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
) -> Result<Expr, String> {
    if depth > MAX_DEPTH {
        return Err("expression nested deeper than the instantiation limit".to_string());
    }
    let recur = |e: &Expr| resolve(e, loop_vars, consts, registry, depth + 1);
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
            match registry.get(name.as_str()) {
                Some(class) if class.kind == ClassKind::Function => {
                    inline_function(class, &args, consts, registry, depth + 1)?
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
        let value = resolve(&value, &HashMap::new(), consts, registry, depth + 1)?;
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
