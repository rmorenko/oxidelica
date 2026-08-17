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
        terminations: acc.terminations,
        experiment: top_class.experiment.clone(),
    })
}

/// Accumulated flat model contents.
#[derive(Default)]
struct Flat {
    components: Vec<Component>,
    equations: Vec<EquationItem>,
    terminations: Vec<Termination>,
    /// Connector instance path -> connector class name.
    connectors: HashMap<String, String>,
    /// Connect statements with fully prefixed paths.
    connects: Vec<(String, String)>,
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

    for component in &class.components {
        let flat_name = format!("{prefix}{}", component.name);
        if component.type_name == "Real" {
            let mut flat = component.clone();
            flat.name = flat_name.clone();
            flat.start = flat.start.map(|e| prefix_expr(&e, prefix));
            flat.binding = flat.binding.map(|e| prefix_expr(&e, prefix));
            // A parent modifier `name = expr` overrides the binding.
            if let Some((_, value)) = overrides.iter().find(|(n, _)| n == &component.name) {
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
                acc.connectors.insert(flat_name.clone(), child.name.clone());
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

    for equation in &class.equations {
        acc.equations.push(EquationItem {
            lhs: prefix_expr(&equation.lhs, prefix),
            rhs: prefix_expr(&equation.rhs, prefix),
        });
    }
    for termination in &class.terminations {
        acc.terminations.push(Termination {
            condition: prefix_expr(&termination.condition, prefix),
            message: termination.message.clone(),
        });
    }
    for (a, b) in &class.connects {
        acc.connects
            .push((format!("{prefix}{a}"), format!("{prefix}{b}")));
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
        Expr::Number(_) | Expr::Bool(_) | Expr::Time => expr.clone(),
    }
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
