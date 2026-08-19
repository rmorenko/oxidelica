//! Resolving names and folding what the compiler can already see:
//! prefixes, constants, substitutions and lookups.

use super::*;

/// Prefix every component reference in an expression, resolving `outer`
/// references to the instance that owns them.
pub(super) fn prefix_expr(expr: &Expr, prefix: &str, outers: &HashMap<String, String>) -> Expr {
    if prefix.is_empty() && outers.is_empty() {
        return expr.clone();
    }
    prefix_expr_under(expr, prefix, outers, &[])
}

/// As [`prefix_expr`], with the names an enclosing comprehension binds:
/// those belong to the iteration, not to the instance, and asking for
/// them under the instance path would find nothing.
pub(super) fn prefix_expr_under(
    expr: &Expr,
    prefix: &str,
    outers: &HashMap<String, String>,
    bound: &[&str],
) -> Expr {
    let recur = |e: &Expr| prefix_expr_under(e, prefix, outers, bound);
    match expr {
        // `getInstanceName()` answers with where it was written, and
        // this is the only place that still knows: the instance path
        // rides along as an argument until the strings pass puts the
        // model's own name in front of it.
        Expr::Call(name, args) if name == "getInstanceName" && args.is_empty() => Expr::Call(
            name.clone(),
            vec![Expr::Str(prefix.trim_end_matches('.').to_string())],
        ),
        Expr::Ref(name) if bound.contains(&name.as_str()) => expr.clone(),
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
        Expr::Array(items) => Expr::Array(items.iter().map(recur).collect()),
        Expr::Elementwise(op, l, r) => {
            Expr::Elementwise(*op, Box::new(recur(l)), Box::new(recur(r)))
        }
        Expr::Range(a, step, b) => Expr::Range(
            Box::new(recur(a)),
            step.as_ref().map(|s| Box::new(recur(s))),
            Box::new(recur(b)),
        ),
        // The iterator variable names the iteration, not a component
        // of the instance, so the body is prefixed with it set aside.
        Expr::Comprehension(body, var, range) => {
            let mut inner: Vec<&str> = bound.to_vec();
            inner.push(var);
            Expr::Comprehension(
                Box::new(prefix_expr_under(body, prefix, outers, &inner)),
                var.clone(),
                Box::new(recur(range)),
            )
        }
        Expr::MatrixRows(rows) => Expr::MatrixRows(
            rows.iter()
                .map(|row| row.iter().map(recur).collect())
                .collect(),
        ),
        Expr::ColonSubscript | Expr::EndSubscript => expr.clone(),
        Expr::Number(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Time => expr.clone(),
        // A call kept whole for its derivative is made while an
        // expression is expanded, and expanding is the last thing that
        // happens to one - so none can be here, where the names are
        // still being given their instance paths.
        Expr::WithDerivative(..) => expr.clone(),
        // The keyword names an input of the function, not a component.
        Expr::NamedArg(keyword, value) => Expr::NamedArg(keyword.clone(), Box::new(recur(value))),
        Expr::Tuple(targets) => Expr::Tuple(
            targets
                .iter()
                .map(|slot| slot.as_ref().map(recur))
                .collect(),
        ),
    }
}

/// Evaluate a compile-time constant expression (array dimensions, loop
/// bounds, subscripts). Only the arithmetic that can appear there is
/// supported; anything else means the value is not constant.
pub(crate) fn const_eval(expr: &Expr, env: &HashMap<String, f64>) -> Option<f64> {
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
        // The numeric builtins fold too: a `while` iterating Newton's
        // method or an AGM lives on `abs` and `sqrt` in its condition.
        Expr::Call(name, args) => {
            let one = || -> Option<f64> { const_eval(args.first()?, env) };
            let two = || -> Option<(f64, f64)> {
                Some((
                    const_eval(args.first()?, env)?,
                    const_eval(args.get(1)?, env)?,
                ))
            };
            match operator_name(name) {
                "abs" => one()?.abs(),
                "sqrt" => one()?.sqrt(),
                "exp" => one()?.exp(),
                "log" => one()?.ln(),
                "log10" => one()?.log10(),
                "sin" => one()?.sin(),
                "cos" => one()?.cos(),
                "tan" => one()?.tan(),
                "asin" => one()?.asin(),
                "acos" => one()?.acos(),
                "atan" => one()?.atan(),
                "sinh" => one()?.sinh(),
                "cosh" => one()?.cosh(),
                "tanh" => one()?.tanh(),
                "floor" => one()?.floor(),
                "ceil" => one()?.ceil(),
                "integer" => one()?.floor(),
                // `Integer(e)` is the ordinal of an enumeration value,
                // which is not the same thing as `integer(x)` cutting a
                // number down. An enumeration is carried as its ordinal
                // here, so there is nothing left to do.
                "Integer" => one()?,
                "atan2" => {
                    let (a, b) = two()?;
                    a.atan2(b)
                }
                "min" => {
                    let (a, b) = two()?;
                    a.min(b)
                }
                "max" => {
                    let (a, b) = two()?;
                    a.max(b)
                }
                "div" => {
                    let (a, b) = two()?;
                    (a / b).trunc()
                }
                "mod" => {
                    let (a, b) = two()?;
                    a - (a / b).floor() * b
                }
                "rem" => {
                    let (a, b) = two()?;
                    a - (a / b).trunc() * b
                }
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// Replace references according to a substitution map (function
/// inlining and loop-variable substitution).
pub(super) fn substitute_refs(expr: &Expr, map: &HashMap<String, Expr>) -> Expr {
    match expr {
        Expr::Ref(name) => map.get(name).cloned().unwrap_or_else(|| expr.clone()),
        Expr::Number(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Time => expr.clone(),
        Expr::WithDerivative(value, rule, seeds) => Expr::WithDerivative(
            Box::new(substitute_refs(value, map)),
            Box::new(substitute_refs(rule, map)),
            seeds
                .iter()
                .map(|(name, arg)| (name.clone(), substitute_refs(arg, map)))
                .collect(),
        ),
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
        Expr::Array(items) => Expr::Array(
            items
                .iter()
                .map(|item| substitute_refs(item, map))
                .collect(),
        ),
        Expr::Elementwise(op, l, r) => Expr::Elementwise(
            *op,
            Box::new(substitute_refs(l, map)),
            Box::new(substitute_refs(r, map)),
        ),
        Expr::Range(a, step, b) => Expr::Range(
            Box::new(substitute_refs(a, map)),
            step.as_ref().map(|s| Box::new(substitute_refs(s, map))),
            Box::new(substitute_refs(b, map)),
        ),
        Expr::Comprehension(body, var, range) => {
            // The iterator shadows any outer binding of the same name.
            let mut inner = map.clone();
            inner.remove(var);
            Expr::Comprehension(
                Box::new(substitute_refs(body, &inner)),
                var.clone(),
                Box::new(substitute_refs(range, map)),
            )
        }
        Expr::MatrixRows(rows) => Expr::MatrixRows(
            rows.iter()
                .map(|row| row.iter().map(|item| substitute_refs(item, map)).collect())
                .collect(),
        ),
        Expr::ColonSubscript | Expr::EndSubscript => expr.clone(),
        Expr::NamedArg(keyword, value) => {
            Expr::NamedArg(keyword.clone(), Box::new(substitute_refs(value, map)))
        }
        Expr::Tuple(targets) => Expr::Tuple(
            targets
                .iter()
                .map(|slot| slot.as_ref().map(|target| substitute_refs(target, map)))
                .collect(),
        ),
    }
}

/// How far a constant may reach through others before this gives up.
/// A chain of a few is ordinary; a longer one is a sign of a circle.
const MAX_CONSTANT_DEPTH: usize = 8;

/// Value of a constant declared inside a class: `Constants.pi`.
///
/// Package constants are compile-time values, so a reference to one is
/// replaced by its number before any prefixing happens - otherwise the
/// dotted name would be mistaken for a component of the instance.
/// `depth` counts how many constants deep the question already is: one
/// may be built on another, and on one of another package.
fn class_constant_at(
    registry: &HashMap<&str, &ClassDef>,
    name: &str,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
) -> Option<f64> {
    if depth > MAX_CONSTANT_DEPTH {
        return None;
    }
    let (class_path, member) = name.rsplit_once('.')?;
    let class = lookup(registry, class_path, scope, imports)?;
    // An enumeration literal is the position it was declared at.
    if let Some(index) = class.enumeration.iter().position(|l| l == member) {
        return Some(index as f64 + 1.0);
    }
    // A package's constants are its own and every base's: `extends`
    // brings the inherited ones into the same namespace, so `Derived.k`
    // reads a `k` that `Base` declared.
    let mut constants: Vec<(String, Option<Expr>)> = Vec::new();
    gather_package_constants(registry, class, 0, &mut constants);
    if !constants.iter().any(|(n, _)| n == member) {
        return None;
    }
    // A constant may be built on one of another package - the standard
    // library's `eps` is the machine's - and on the operators a library
    // has given a place in its tree. Both are settled in the binding
    // before it is asked for a number, and the depth counter is what
    // stops two packages naming each other from going round for ever.
    let constants: Vec<(String, Option<Expr>)> = constants
        .into_iter()
        .map(|(name, binding)| {
            let binding = binding.map(|expr| {
                substitute_at(
                    &expr,
                    registry,
                    &class.name,
                    &class.imports,
                    &[],
                    depth + 1,
                    true,
                )
            });
            (name, binding)
        })
        .collect();
    // Constants of one package may build on each other, so resolve the
    // whole set to a fixpoint before reading the one asked for.
    let mut values: HashMap<String, f64> = HashMap::new();
    loop {
        let mut progress = false;
        for (name, binding) in &constants {
            if values.contains_key(name) {
                continue;
            }
            if let Some(value) = binding.as_ref().and_then(|expr| const_eval(expr, &values)) {
                values.insert(name.clone(), value);
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
    values.get(member).copied()
}

/// The constants and parameters a package holds, its own and those it
/// inherits. Bases come first, so a class's own declaration overrides
/// an inherited one of the same name.
fn gather_package_constants<'a>(
    registry: &HashMap<&str, &'a ClassDef>,
    class: &'a ClassDef,
    depth: usize,
    out: &mut Vec<(String, Option<Expr>)>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, &class.name, &class.imports) {
            gather_package_constants(registry, base, depth + 1, out);
        }
    }
    for component in &class.components {
        if matches!(
            component.variability,
            Variability::Constant | Variability::Parameter
        ) {
            let binding = component
                .binding
                .clone()
                .or_else(|| component.start.clone());
            out.retain(|(existing, _)| existing != &component.name);
            out.push((component.name.clone(), binding));
        }
    }
}

/// Replace every reference to a class constant with its value.
pub(super) fn substitute_class_constants(
    expr: &Expr,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    shadow: &[&str],
) -> Expr {
    substitute_class_constants_at(expr, registry, scope, imports, shadow, 0)
}

/// See [`substitute_class_constants`]; `depth` counts how many
/// constants deep the question already is.
fn substitute_class_constants_at(
    expr: &Expr,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    shadow: &[&str],
    depth: usize,
) -> Expr {
    substitute_at(expr, registry, scope, imports, shadow, depth, false)
}

/// See [`substitute_class_constants`]. `settle_calls` says whether a
/// call to a library function is to be worked out here as well, which
/// is what a constant's own binding needs - `2*Modelica.Math.asin(1)`
/// is a number, and nothing later would make it one. Ordinary
/// expressions leave their calls standing: inlining them is the job of
/// the pass that knows the shapes involved.
#[allow(clippy::too_many_arguments)]
fn substitute_at(
    expr: &Expr,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    shadow: &[&str],
    depth: usize,
    settle_calls: bool,
) -> Expr {
    let recur = |e: &Expr| substitute_at(e, registry, scope, imports, shadow, depth, settle_calls);
    match expr {
        Expr::Ref(name) if name.contains('.') => {
            match class_constant_at(registry, name, scope, imports, depth) {
                Some(value) => Expr::Number(value),
                None => expr.clone(),
            }
        }
        Expr::Ref(name) => {
            // A constant brought in by name is written without one:
            // `import Modelica.Constants.pi;` and then `pi`.
            if let Some(value) = imports
                .iter()
                .find(|(local, _)| local == name)
                .and_then(|(_, target)| class_constant_at(registry, target, scope, imports, depth))
            {
                return Expr::Number(value);
            }
            // A package opened wholesale (`import A.*;`) also brings its
            // constants in, but at the bottom of the pile: a component
            // of the model with the same name outranks it, so only a
            // name that is not one is looked for among them.
            if !shadow.contains(&name.as_str()) {
                for (_, target) in imports.iter().filter(|(local, _)| local == WILDCARD_IMPORT) {
                    if let Some(value) = class_constant_at(
                        registry,
                        &format!("{target}.{name}"),
                        scope,
                        imports,
                        depth,
                    ) {
                        return Expr::Number(value);
                    }
                }
            }
            expr.clone()
        }
        Expr::Number(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Time => expr.clone(),
        Expr::WithDerivative(value, rule, seeds) => Expr::WithDerivative(
            Box::new(recur(value)),
            Box::new(recur(rule)),
            seeds
                .iter()
                .map(|(name, arg)| (name.clone(), recur(arg)))
                .collect(),
        ),
        // A library gives the language's own operators a place in its
        // tree with `external "builtin" y = asin(u)`; a call to one of
        // those is a call to the operator.
        Expr::Call(name, args) => {
            let args: Vec<Expr> = args.iter().map(recur).collect();
            let found = lookup(registry, name, scope, imports);
            if let Some(builtin) = found.and_then(|class| class.builtin.clone()) {
                return Expr::Call(builtin, args);
            }
            match found.filter(|_| settle_calls && depth <= MAX_CONSTANT_DEPTH) {
                Some(class) if class.kind == ClassKind::Function => {
                    let shapes: Vec<Vec<i64>> = args.iter().map(|_| Vec::new()).collect();
                    inline_function(class, &args, &shapes, &HashMap::new(), registry, depth)
                        .unwrap_or_else(|_| Expr::Call(name.clone(), args))
                }
                _ => Expr::Call(name.clone(), args),
            }
        }
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
        Expr::Array(items) => Expr::Array(items.iter().map(recur).collect()),
        Expr::Elementwise(op, l, r) => {
            Expr::Elementwise(*op, Box::new(recur(l)), Box::new(recur(r)))
        }
        Expr::Range(a, step, b) => Expr::Range(
            Box::new(recur(a)),
            step.as_ref().map(|s| Box::new(recur(s))),
            Box::new(recur(b)),
        ),
        Expr::Comprehension(body, var, range) => {
            Expr::Comprehension(Box::new(recur(body)), var.clone(), Box::new(recur(range)))
        }
        Expr::MatrixRows(rows) => Expr::MatrixRows(
            rows.iter()
                .map(|row| row.iter().map(recur).collect())
                .collect(),
        ),
        Expr::ColonSubscript | Expr::EndSubscript => expr.clone(),
        Expr::NamedArg(keyword, value) => Expr::NamedArg(keyword.clone(), Box::new(recur(value))),
        Expr::Tuple(targets) => Expr::Tuple(
            targets
                .iter()
                .map(|slot| slot.as_ref().map(recur))
                .collect(),
        ),
    }
}

/// A class named through one import list: `import Basic = A.B;` then
/// `Basic.Resistor`, or `import A.Widget;` then `Widget`. The wildcard
/// form is not tried here - it is the lowest-priority reading and left
/// to the end of [`lookup`].
fn named_import<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    head: &str,
    rest: Option<&str>,
    imports: &[(String, String)],
) -> Option<&'a ClassDef> {
    let (_, target) = imports
        .iter()
        .find(|(local, _)| local == head && local != WILDCARD_IMPORT)?;
    let qualified = match rest {
        Some(rest) => format!("{target}.{rest}"),
        None => target.clone(),
    };
    registry.get(qualified.as_str()).copied()
}

/// Resolve a class name the way Modelica scoping does: an import
/// alias first, then the class's own nested classes, then the
/// enclosing packages from the inside out, then the global name.
///
/// `scope` is the qualified name of the class doing the looking - not
/// its parent - so that `connector Pin` declared inside `model Bus` is
/// found by components of `Bus` itself.
pub(super) fn lookup<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    name: &str,
    scope: &str,
    imports: &[(String, String)],
) -> Option<&'a ClassDef> {
    // A name written with a leading dot is looked up from the top of
    // the tree and nowhere else. That is what lets a library write its
    // own `asin` and still reach the language's operator from inside
    // it: `.asin` is never the function being written.
    if let Some(global) = name.strip_prefix('.') {
        return registry.get(global).copied();
    }
    // `import Basic = Electrical.Analog.Basic;` then `Basic.Resistor`.
    let (head, rest) = match name.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (name, None),
    };
    if let Some(class) = named_import(registry, head, rest, imports) {
        return Some(class);
    }
    // Walk out of the enclosing packages: A.B.C -> A.B -> A -> global.
    // An `encapsulated` class is a wall: its own scope is searched, and
    // then the walk stops rather than reaching what encloses it, so a
    // simple name has to be imported or built in.
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
        // Each enclosing class brings its own imports to the lookup -
        // they are not inherited, but they are lexically in view - so
        // an `import` on the encapsulated wall is what a name inside it
        // reaches through.
        if let Some(enclosing) = registry.get(prefix.as_str()) {
            if let Some(class) = named_import(registry, head, rest, &enclosing.imports) {
                return Some(class);
            }
            // The wall is a package's: a name inside an encapsulated
            // package does not reach past it. The overloads gathered
            // under a quoted operator symbol (`Complex.'+'`) are a
            // package too, but they exist to serve their record and
            // still see it, so they are not a wall.
            let is_operator = enclosing
                .name
                .rsplit('.')
                .next()
                .is_some_and(|segment| segment.starts_with('\''));
            if enclosing.encapsulated && enclosing.kind == ClassKind::Package && !is_operator {
                break;
            }
        }
        match prefix.rfind('.') {
            Some(cut) => prefix.truncate(cut),
            None if prefix.is_empty() => break,
            None => prefix.clear(),
        }
    }
    // Last of all, the packages opened wholesale: an unqualified
    // import is outranked by everything with a name of its own, which
    // is what keeps `import A.*;` from quietly shadowing a class the
    // enclosing package already had.
    imports
        .iter()
        .filter(|(local, _)| local == WILDCARD_IMPORT)
        .find_map(|(_, target)| registry.get(format!("{target}.{name}").as_str()).copied())
}

/// Whether an expression is Boolean, so a subscript of it indexes a
/// Boolean dimension off its `false` lower bound.
pub(super) fn is_boolean(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Bool(_) | Expr::Rel(..) | Expr::And(..) | Expr::Or(..) | Expr::Not(..)
    )
}

/// Built-in scalar types. `Integer` and `Boolean` are carried as
/// numbers, like everything else in the flat model.
pub(super) fn is_primitive(type_name: &str) -> bool {
    matches!(
        type_name,
        "Real" | "Integer" | "Boolean" | "String" | "Clock"
    )
}

/// Flat scalar name of one array element: `T[2]`, `A[1,3]`.
pub(super) fn element_name(base: &str, subscripts: &[i64]) -> String {
    let list: Vec<String> = subscripts.iter().map(|i| i.to_string()).collect();
    format!("{base}[{}]", list.join(","))
}

/// Every index tuple of an array with the given dimensions, in row-major
/// order: `[2, 3]` yields (1,1), (1,2), (1,3), (2,1), ...
pub(super) fn index_tuples(dimensions: &[i64]) -> Vec<Vec<i64>> {
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
pub(super) fn resolve(
    expr: &Expr,
    loop_vars: &HashMap<String, f64>,
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
) -> Result<Expr, String> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "an expression {NO_BOTTOM}, nested deeper than the compiler follows: {}",
            sketch(expr)
        ));
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
                // A Boolean subscript indexes a Boolean dimension, whose
                // lower bound is `false`: `x[false]` is the first
                // element, `x[true]` the second. Enumeration literals
                // already carry their 1-based position, so only the
                // Booleans need lifting off zero.
                if is_boolean(&resolved) {
                    indices.push(value as i64 + 1);
                    continue;
                }
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
                    inline_function(class, &args, &[], consts, registry, depth + 1)?
                }
                // `noEvent(x)` and `smooth(n, x)` are hints about event
                // generation and continuity; the value is the argument.
                _ if name == "noEvent" && args.len() == 1 => args[0].clone(),
                _ if name == "smooth" && args.len() == 2 => args[1].clone(),
                // `homotopy` offers an easier problem to start from. A
                // tool may take the real one and go straight at it,
                // which is what this one does.
                _ if name == "homotopy" && args.len() == 2 => args[0].clone(),
                // `semiLinear(x, a, b)` is `a * x` one way and `b * x`
                // the other, meeting at zero.
                _ if name == "semiLinear" && args.len() == 3 => Expr::If(
                    Box::new(Expr::Rel(
                        crate::ast::RelOp::Ge,
                        Box::new(args[0].clone()),
                        Box::new(Expr::Number(0.0)),
                    )),
                    Box::new(Expr::Bin(
                        BinOp::Mul,
                        Box::new(args[1].clone()),
                        Box::new(args[0].clone()),
                    )),
                    Box::new(Expr::Bin(
                        BinOp::Mul,
                        Box::new(args[2].clone()),
                        Box::new(args[0].clone()),
                    )),
                ),
                _ if args.iter().any(|a| matches!(a, Expr::NamedArg(_, _))) => {
                    return Err(format!(
                        "`{name}` is not a function, so it cannot take named arguments"
                    ))
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
        Expr::Array(_)
        | Expr::Elementwise(_, _, _)
        | Expr::Range(_, _, _)
        | Expr::Comprehension(_, _, _)
        | Expr::MatrixRows(_) => {
            return Err(format!(
                "an array value cannot be used where a scalar is expected: {}",
                sketch(expr)
            ))
        }
        Expr::ColonSubscript | Expr::EndSubscript => {
            return Err("`:` and `end` make sense only inside a subscript".to_string())
        }
        Expr::Number(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Time => expr.clone(),
        Expr::WithDerivative(value, rule, seeds) => Expr::WithDerivative(
            Box::new(recur(value)?),
            Box::new(recur(rule)?),
            seeds
                .iter()
                .map(|(name, arg)| Ok((name.clone(), recur(arg)?)))
                .collect::<Result<Vec<_>, String>>()?,
        ),
        Expr::NamedArg(keyword, value) => Expr::NamedArg(keyword.clone(), Box::new(recur(value)?)),
        Expr::Tuple(_) => {
            return Err("a tuple may only stand on the left of `=` or `:=`".to_string())
        }
    })
}

/// Replace `end` inside a subscript with the dimension's length.
pub(super) fn substitute_end(expr: &Expr, length: f64) -> Expr {
    match expr {
        Expr::EndSubscript => Expr::Number(length),
        Expr::Bin(op, l, r) => Expr::Bin(
            *op,
            Box::new(substitute_end(l, length)),
            Box::new(substitute_end(r, length)),
        ),
        Expr::Neg(inner) => Expr::Neg(Box::new(substitute_end(inner, length))),
        Expr::Range(a, step, b) => Expr::Range(
            Box::new(substitute_end(a, length)),
            step.as_ref().map(|s| Box::new(substitute_end(s, length))),
            Box::new(substitute_end(b, length)),
        ),
        other => other.clone(),
    }
}

/// A short rendering of an expression, for a message that has to say
/// which one it means without printing a tree.
pub(super) fn sketch(expr: &Expr) -> String {
    let text = format!("{expr:?}");
    match text.char_indices().nth(70) {
        Some((cut, _)) => format!("{}...", &text[..cut]),
        None => text,
    }
}
