//! Resolving names and folding what the compiler can already see:
//! prefixes, constants, substitutions and lookups.

use super::*;
use std::cell::{Cell, RefCell};

/// What the specification calls the arguments of the operators that
/// take named ones, in the order they are declared.
///
/// The language gives these operators a signature like any function's,
/// so a model may write `homotopy(actual = a, simplified = s)` and the
/// standard library does - the limiters, the operational amplifiers and
/// the clocked blocks all the way through. Nothing else about an
/// operator needs the names, so this is the whole of what is kept.
const BUILTIN_ARGUMENTS: &[(&str, &[&str])] = &[
    ("homotopy", &["actual", "simplified"]),
    ("Clock", &["interval", "resolution"]),
    ("smooth", &["p", "u"]),
    ("noEvent", &["expr"]),
    ("semiLinear", &["x", "positiveSlope", "negativeSlope"]),
    ("delay", &["expr", "delayTime", "delayMax"]),
    ("sample", &["start", "interval"]),
    ("previous", &["u"]),
    ("hold", &["u"]),
    ("subSample", &["u", "factor"]),
    ("superSample", &["u", "factor"]),
    ("shiftSample", &["u", "shiftCounter", "resolution"]),
    ("backSample", &["u", "backCounter", "resolution"]),
    ("noClock", &["u"]),
    (
        "spatialDistribution",
        &[
            "in0",
            "in1",
            "x",
            "positiveVelocity",
            "initialPoints",
            "initialValues",
        ],
    ),
];

/// Put an operator's named arguments where its declaration says they
/// go, so everything after this reads one shape.
///
/// A name nothing declared, or one given twice, or one given both by
/// name and by position, is a mistake about the operator rather than
/// about the model's physics, and is named as one. An operator this
/// knows nothing about keeps the refusal it had: named arguments are
/// something a function takes.
fn order_builtin_arguments(name: &str, args: Vec<Expr>) -> Result<Vec<Expr>, String> {
    if !args.iter().any(|a| matches!(a, Expr::NamedArg(..))) {
        return Ok(args);
    }
    let Some((_, declared)) = BUILTIN_ARGUMENTS.iter().find(|(known, _)| *known == name) else {
        return Ok(args);
    };
    // Everything before the first named argument stands where it is;
    // the specification does not allow one to follow a named argument.
    let positional = args
        .iter()
        .position(|a| matches!(a, Expr::NamedArg(..)))
        .expect("just checked there is one");
    if args[positional..]
        .iter()
        .any(|a| !matches!(a, Expr::NamedArg(..)))
    {
        return Err(format!(
            "`{name}` was given an argument by position after one given by name"
        ));
    }
    let mut out: Vec<Option<Expr>> = args[..positional].iter().cloned().map(Some).collect();
    for arg in &args[positional..] {
        let Expr::NamedArg(keyword, value) = arg else {
            unreachable!("the loop above checked every one of these");
        };
        let Some(at) = declared.iter().position(|d| d == keyword) else {
            return Err(format!(
                "`{name}` has no argument called `{keyword}`; it takes {}",
                declared.join(", ")
            ));
        };
        if at < positional {
            return Err(format!(
                "`{name}` was given `{keyword}` both by name and by position"
            ));
        }
        if out.len() <= at {
            out.resize(at + 1, None);
        }
        if out[at].is_some() {
            return Err(format!("`{name}` was given `{keyword}` twice"));
        }
        out[at] = Some((**value).clone());
    }
    // A gap is an argument the operator was never given. The arity
    // check further along is what says whether that matters - some of
    // these take an optional last argument - so what is handed on is
    // what was written, and a gap simply ends it.
    let mut settled = Vec::with_capacity(out.len());
    for (at, value) in out.into_iter().enumerate() {
        match value {
            Some(value) => settled.push(value),
            None => {
                return Err(format!(
                    "`{name}` was given nothing for `{}`",
                    declared.get(at).copied().unwrap_or("an argument")
                ))
            }
        }
    }
    Ok(settled)
}

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
        // A body written here in Rust folds where everything it is
        // handed is settled: the standard library builds a generator's
        // first state by drawing ten numbers from a seed, and all ten
        // fold into the two numbers the state is.
        Expr::Index(base, subscripts) => {
            let (Expr::Call(called, args), [which]) = (base.as_ref(), subscripts.as_slice()) else {
                return None;
            };
            let mut given = Vec::new();
            for arg in args {
                match arg {
                    Expr::Array(items) => {
                        for item in items {
                            given.push(const_eval(item, env)?);
                        }
                    }
                    one => given.push(const_eval(one, env)?),
                }
            }
            let which = const_eval(which, env)?;
            let place = (which as usize).checked_sub(1)?;
            *crate::outside::answer(called, &given)?.get(place)?
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

/// A class constant that is an array rather than a number: the
/// multibody world states its axis colours as `Types.Defaults
/// .FrameColor`, a constant vector of three, and a name that comes to
/// a vector is not one [`class_constant_at`] can answer. Left
/// unanswered the name travels into the flat model with the instance
/// path stuck on the front - `world.Modelica.Mechanics.MultiBody
/// .Types.Defaults.FrameColor` - which nothing declares.
fn class_constant_array_at(
    registry: &HashMap<&str, &ClassDef>,
    name: &str,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
) -> Option<Expr> {
    if depth > MAX_CONSTANT_DEPTH {
        return None;
    }
    let (class_path, member) = name.rsplit_once('.')?;
    let class = lookup(registry, class_path, scope, imports)?;
    let mut constants: Vec<(String, Option<Expr>)> = Vec::new();
    gather_package_constants(registry, class, 0, &mut constants);
    let binding = constants
        .into_iter()
        .find(|(n, _)| n == member)
        .and_then(|(_, binding)| binding)?;
    let binding = substitute_at(
        &binding,
        registry,
        &class.name,
        &class.imports,
        &[],
        depth + 1,
        true,
    );
    // A value written out as an array is taken, and so is a record
    // built by its own constructor: `constant Complex j = Complex(0,
    // 1)` is the imaginary unit the quasi-static libraries write their
    // impedances with, and the layer that knows records makes the
    // fields of it. Anything else is a name this pass has no
    // environment to read, and leaving it where it was is what
    // happened before.
    let a_record = matches!(&binding, Expr::Call(built, _)
        if lookup(registry, built, &class.name, &class.imports)
            .is_some_and(|of| of.kind == ClassKind::Record));
    (matches!(binding, Expr::Array(_)) || a_record).then_some(binding)
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
                None => class_constant_array_at(registry, name, scope, imports, depth)
                    .unwrap_or_else(|| expr.clone()),
            }
        }
        Expr::Ref(name) => {
            // A constant brought in by name is written without one:
            // `import Modelica.Constants.pi;` and then `pi`.
            if let Some(target) = imports
                .iter()
                .find(|(local, _)| local == name)
                .map(|(_, target)| target)
            {
                if let Some(value) = class_constant_at(registry, target, scope, imports, depth) {
                    return Expr::Number(value);
                }
                if let Some(value) =
                    class_constant_array_at(registry, target, scope, imports, depth)
                {
                    return value;
                }
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
            // A constant of a package this class is written inside is
            // named without one: `nXi` inside `BaseProperties` is the
            // medium's. Only packages are asked - what a model holds is
            // not in view of what is written inside another class of it.
            if !shadow.contains(&name.as_str()) {
                if let Some(value) = enclosing_constant(registry, name, scope, depth) {
                    return Expr::Number(value);
                }
                // What a package this class is written inside brought
                // in by name is in view here too: the flux tubes say
                // `import Modelica.Constants.mu_0` once at the top of
                // the library and every shape below writes `mu_0`.
                if let Some(value) = enclosing_import(registry, name, scope, depth) {
                    return Expr::Number(value);
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
        // `size(substanceNames, 1)` where the list is a constant of the
        // package: this is what a medium counts its substances with,
        // and the length is in the declaration rather than anywhere a
        // later pass would look.
        Expr::Call(name, args)
            if settle_calls
                && name == "size"
                && matches!(args.first(), Some(Expr::Ref(_)))
                && depth <= MAX_CONSTANT_DEPTH =>
        {
            let Some(Expr::Ref(named)) = args.first() else {
                unreachable!("just matched a name")
            };
            match enclosing_binding(registry, named, scope) {
                Some(Expr::Array(items)) => Expr::Number(items.len() as f64),
                _ => expr.clone(),
            }
        }
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

/// What a constant of an enclosing package was declared to be, before
/// anything is made of it.
///
/// `constant Integer nS = size(substanceNames, 1)` asks how long a
/// list of names is, and the list is a constant of the same package or
/// of one it is written inside. Nothing later knows the name, so the
/// length is read here.
fn enclosing_binding(registry: &HashMap<&str, &ClassDef>, name: &str, scope: &str) -> Option<Expr> {
    let mut prefix = scope;
    loop {
        if let Some(owner) = registry
            .get(prefix)
            .filter(|owner| owner.kind == ClassKind::Package)
        {
            let mut constants = Vec::new();
            gather_package_constants(registry, owner, 0, &mut constants);
            if let Some((_, binding)) = constants.iter().find(|(known, _)| known == name) {
                return binding.clone();
            }
        }
        let (head, _) = prefix.rsplit_once('.')?;
        prefix = head;
    }
}

/// A constant of a package the given scope is written inside.
///
/// `constant Integer nXi` belongs to the medium package, and
/// `BaseProperties`, written inside it, names it as it stands. The
/// walk goes out through the enclosing packages only: a model holding
/// a component called `nXi` says nothing about what another class of
/// it may write.
fn enclosing_constant(
    registry: &HashMap<&str, &ClassDef>,
    name: &str,
    scope: &str,
    depth: usize,
) -> Option<f64> {
    // Asked of every name written anywhere, and answered by gathering
    // a package's constants and working out what each comes to. Where
    // one registry stands the answer is the same every time, so it is
    // worked out once.
    if !REGISTRY_STANDS.with(|stands| stands.get()) {
        return enclosing_constant_at(registry, name, scope, depth);
    }
    let key = (name.to_string(), scope.to_string());
    if let Some(remembered) = NAMED.with(|named| named.borrow().get(&key).copied()) {
        return remembered;
    }
    let found = enclosing_constant_at(registry, name, scope, depth);
    NAMED.with(|named| named.borrow_mut().insert(key, found));
    found
}

thread_local! {
    /// What a name written inside a package came to, by name and by
    /// where it was written.
    static NAMED: RefCell<HashMap<(String, String), Option<f64>>> =
        RefCell::new(HashMap::new());
}

/// What a name written inside a package was brought in as by that
/// package, or by a package holding it.
///
/// An import is written once where it reads well - at the top of a
/// library - and holds for everything written inside, which is how the
/// flux tubes name the magnetic constant `mu_0` throughout without
/// ever importing it again.
fn enclosing_import(
    registry: &HashMap<&str, &ClassDef>,
    name: &str,
    scope: &str,
    depth: usize,
) -> Option<f64> {
    let mut prefix = scope;
    while let Some((head, _)) = prefix.rsplit_once('.') {
        if let Some(owner) = registry.get(head) {
            if let Some(value) = owner
                .imports
                .iter()
                .find(|(local, _)| local == name)
                .and_then(|(_, target)| {
                    class_constant_at(registry, target, head, &owner.imports, depth)
                })
            {
                return Some(value);
            }
        }
        prefix = head;
    }
    None
}

/// See [`enclosing_constant`]; this is the walk itself.
fn enclosing_constant_at(
    registry: &HashMap<&str, &ClassDef>,
    name: &str,
    scope: &str,
    depth: usize,
) -> Option<f64> {
    let mut prefix = scope;
    while let Some((head, _)) = prefix.rsplit_once('.') {
        if let Some(owner) = registry
            .get(head)
            .filter(|owner| owner.kind == ClassKind::Package)
        {
            let mut constants = Vec::new();
            gather_package_constants(registry, owner, 0, &mut constants);
            if constants.iter().any(|(known, _)| known == name) {
                return class_constant_at(
                    registry,
                    &format!("{head}.{name}"),
                    head,
                    &owner.imports,
                    depth,
                );
            }
        }
        prefix = head;
    }
    None
}

/// The class a short definition inside a package stands for.
///
/// `package StandardWater = WaterIF97_ph(...)` gives the package a
/// member that is a name for another class; from outside, the member
/// is reached by the same dotted name a class would be. The target is
/// written in the terms of the package that holds it, and may be
/// another such name, which is what the counter bounds.
fn through_alias<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    name: &str,
    depth: usize,
) -> Option<&'a ClassDef> {
    if depth > MAX_DEPTH {
        return None;
    }
    // The name may be a member of an alias rather than an alias
    // itself: `Lib.Standard.Cell` is the `Cell` of whatever `Standard`
    // names. So every split is tried, longest holder first.
    let mut cut = name.rfind('.')?;
    loop {
        let (holder, rest) = (&name[..cut], &name[cut + 1..]);
        if let Some(owner) = registry.get(holder) {
            let (member, tail) = match rest.split_once('.') {
                Some((member, tail)) => (member, Some(tail)),
                None => (rest, None),
            };
            if let Some(alias) = owner
                .class_aliases
                .iter()
                .find(|alias| alias.name == member && !alias.redeclaration)
            {
                let target = match tail {
                    Some(tail) => format!("{}.{tail}", alias.target),
                    None => alias.target.clone(),
                };
                if let Some(found) = lookup(registry, &target, holder, &owner.imports)
                    .or_else(|| through_alias(registry, &target, depth + 1))
                {
                    return Some(found);
                }
            }
        }
        cut = holder.rfind('.')?;
    }
}

/// Whether a name is one a short `connector` definition gave to a
/// class of its own: `connector ComplexOutput = output Complex`.
///
/// The record it names says nothing about being connectable, so the
/// name is what has to be asked. A dotted name is asked of the class
/// holding it; a plain one, of every class the scope is written
/// inside.
pub(super) fn names_a_connector(
    registry: &HashMap<&str, &ClassDef>,
    name: &str,
    scope: &str,
    imports: &[(String, String)],
) -> bool {
    let told = |owner: &ClassDef, member: &str| {
        owner
            .class_aliases
            .iter()
            .any(|alias| alias.name == member && alias.connector)
    };
    if let Some((holder, member)) = name.rsplit_once('.') {
        return plain_lookup(registry, holder, scope).is_some_and(|owner| told(owner, member));
    }
    if let Some((_, target)) = imports.iter().find(|(local, _)| local == name) {
        if let Some((holder, member)) = target.rsplit_once('.') {
            return plain_lookup(registry, holder, scope).is_some_and(|owner| told(owner, member));
        }
    }
    let mut prefix = scope;
    loop {
        if registry.get(prefix).is_some_and(|owner| told(owner, name)) {
            return true;
        }
        match prefix.rsplit_once('.') {
            Some((head, _)) => prefix = head,
            None => return false,
        }
    }
}

/// A class by name, without asking what anything inherits.
///
/// This is the walk out of the enclosing packages and nothing else. It
/// is what `member_of_base` names a base with: going through `lookup`
/// would ask about inherited members again, and about the same ones.
fn plain_lookup<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    name: &str,
    scope: &str,
) -> Option<&'a ClassDef> {
    let name = name.strip_prefix('.').unwrap_or(name);
    let mut here = Some(scope);
    while let Some(prefix) = here {
        let candidate = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        };
        if let Some(class) = registry
            .get(candidate.as_str())
            .copied()
            .or_else(|| through_alias(registry, &candidate, 0))
        {
            return Some(class);
        }
        here = match prefix.rsplit_once('.') {
            Some((head, _)) => Some(head),
            None if prefix.is_empty() => None,
            None => Some(""),
        };
    }
    None
}

/// A member a class inherits rather than declares.
///
/// `WaterIF97_ph.BaseProperties` is written in `WaterIF97_base`, which
/// `WaterIF97_ph` extends. Only the last dot is split: the holder is a
/// class by its own name, which is how the standard library names a
/// medium. Trying every split as well would be a walk of the whole
/// tree on every name that is not found, and most names that are not
/// found are simply not there.
fn member_of_base<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    name: &str,
    depth: usize,
) -> Option<&'a ClassDef> {
    if depth > MAX_DEPTH {
        return None;
    }
    let (holder, member) = name.rsplit_once('.')?;
    let owner = registry.get(holder)?;
    owner.extends.iter().find_map(|extend| {
        let base = plain_lookup(registry, &extend.base, holder)?;
        let reached = format!("{}.{member}", base.name);
        registry
            .get(reached.as_str())
            .copied()
            .or_else(|| through_alias(registry, &reached, depth + 1))
            .or_else(|| member_of_base(registry, &reached, depth + 1))
    })
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
    // What the import names may itself name something else, and what
    // is reached through it may be written in a base of it: `Medium`
    // stands for `WaterIF97_ph`, and its `BaseProperties` belongs to
    // `WaterIF97_base`. This is how a redeclared package is reached,
    // so it has to see as far as an ordinary name does.
    registry
        .get(qualified.as_str())
        .copied()
        .or_else(|| through_alias(registry, &qualified, 0))
        .or_else(|| member_of_base(registry, &qualified, 0))
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
    // A name may be a name for a name, and what it stands for may be
    // written in a base of something else with a name of its own. Two
    // libraries naming each other that way would send this round for
    // ever, so the going round is counted.
    if LOOKING.with(|deep| deep.get()) > MAX_DEPTH {
        return None;
    }
    LOOKING.with(|deep| deep.set(deep.get() + 1));
    let found = lookup_at(registry, name, scope, imports);
    LOOKING.with(|deep| deep.set(deep.get() - 1));
    found
}

thread_local! {
    /// How deep the search for a name is into itself.
    static LOOKING: Cell<usize> = const { Cell::new(0) };
}

fn lookup_at<'a>(
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
    // Walk out of the enclosing packages. What that walk finds depends
    // on the name, where it is written and the classes themselves -
    // never on the imports of whoever asked - so the answer is
    // remembered and given again.
    if let Some(class) = walked(registry, name, scope) {
        return Some(class);
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

thread_local! {
    /// What the walk out of the enclosing packages found, by name and
    /// by where the name was written. Classes are kept by name rather
    /// than by reference, so the table outlives nothing it should not:
    /// it is only ever read against the registry it was filled from.
    static WALKED: RefCell<HashMap<(String, String), Option<String>>> =
        RefCell::new(HashMap::new());
    /// Whether a registry stands still for long enough to remember
    /// anything about it.
    static REGISTRY_STANDS: Cell<bool> = const { Cell::new(false) };
}

/// A registry that stands still, so what is found in it may be
/// remembered.
///
/// Held for as long as one registry is in use and dropped with it.
/// Outside one - a caller asking about a class on its own - nothing is
/// remembered, since the next question may be about another library.
pub(super) struct StandingNames;

impl StandingNames {
    /// Start remembering, forgetting whatever came before.
    pub(super) fn open() -> Self {
        WALKED.with(|walked| walked.borrow_mut().clear());
        NAMED.with(|named| named.borrow_mut().clear());
        REGISTRY_STANDS.with(|stands| stands.set(true));
        StandingNames
    }
}

impl Drop for StandingNames {
    fn drop(&mut self) {
        REGISTRY_STANDS.with(|stands| stands.set(false));
        WALKED.with(|walked| walked.borrow_mut().clear());
        NAMED.with(|named| named.borrow_mut().clear());
    }
}

/// The walk out of the enclosing packages, answered from what it found
/// last time where it can be.
fn walked<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    name: &str,
    scope: &str,
) -> Option<&'a ClassDef> {
    if !REGISTRY_STANDS.with(|stands| stands.get()) {
        return walk_out(registry, name, scope);
    }
    let key = (name.to_string(), scope.to_string());
    if let Some(remembered) = WALKED.with(|walked| walked.borrow().get(&key).cloned()) {
        return remembered.and_then(|found| registry.get(found.as_str()).copied());
    }
    let found = walk_out(registry, name, scope);
    WALKED.with(|walked| {
        walked
            .borrow_mut()
            .insert(key, found.map(|class| class.name.clone()))
    });
    found
}

/// A.B.C -> A.B -> A -> global.
///
/// An `encapsulated` class is a wall: its own scope is searched, and
/// then the walk stops rather than reaching what encloses it, so a
/// simple name has to be imported or built in.
fn walk_out<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    name: &str,
    scope: &str,
) -> Option<&'a ClassDef> {
    let (head, rest) = match name.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (name, None),
    };
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
        // A package may name a class rather than define one -
        // `package StandardWater = WaterIF97_ph(...)` - and a name
        // from outside reaches it the same way it reaches a class.
        if let Some(class) = through_alias(registry, &candidate, 0) {
            return Some(class);
        }
        // A member may be written in a base of the class holding it:
        // `WaterIF97_ph extends WaterIF97_base`, and `BaseProperties`
        // belongs to the base. Naming it through the package that
        // extends is how the standard library names a medium.
        if let Some(class) = member_of_base(registry, &candidate, 0) {
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
    None
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

/// Pick one element out of a list written in full: `{1, 2, 3}[2]`, and
/// a dimension at a time for a list of lists.
fn pick_from_list(items: &[Expr], indices: &[i64]) -> Option<Expr> {
    let (first, rest) = indices.split_first()?;
    let item = items.get(usize::try_from(*first - 1).ok()?)?;
    if rest.is_empty() {
        return Some(item.clone());
    }
    match item {
        Expr::Array(inner) => pick_from_list(inner, rest),
        _ => None,
    }
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
    sizes: &HashMap<String, Vec<i64>>,
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
    let recur = |e: &Expr| {
        resolve(
            e,
            loop_vars,
            consts,
            sizes,
            registry,
            scope,
            imports,
            depth + 1,
        )
    };
    Ok(match expr {
        // A body written here in Rust takes what it was handed as it
        // was handed it: an array argument stays whole however deep it
        // goes, since a matrix is an array of its rows and the body
        // takes the numbers in the order they were written.
        Expr::Call(name, args) if crate::outside::written_here(name) => {
            fn whole(
                expr: &Expr,
                one: &impl Fn(&Expr) -> Result<Expr, String>,
            ) -> Result<Expr, String> {
                match expr {
                    Expr::Array(items) => Ok(Expr::Array(
                        items
                            .iter()
                            .map(|item| whole(item, one))
                            .collect::<Result<Vec<_>, String>>()?,
                    )),
                    plain => one(plain),
                }
            }
            Expr::Call(
                name.clone(),
                args.iter()
                    .map(|arg| whole(arg, &recur))
                    .collect::<Result<Vec<_>, String>>()?,
            )
        }
        Expr::Index(base, subscripts) => {
            // A function body reads `table[i]` off whatever it was
            // handed, and what it was handed may be a list written out
            // in full rather than a name. The base may also be a member
            // read off a subscripted component - `medium[i].Xi[1]` -
            // which is a name once the subscript in the middle is
            // settled.
            // A body written here answers with one flat list, and a
            // subscript takes a place of it. There is nothing to look
            // into: the call stands, and the run reads the place.
            // A call nothing could write out answers where it stands,
            // and one of its answers is asked for by subscript: the
            // whole of it is left for the run, which walks the body and
            // reads the element off what comes back. Nothing here can
            // pick the element, since there is no list to pick from
            // until the call is made.
            if let Expr::Call(..) = base.as_ref() {
                return Ok(Expr::Index(
                    Box::new(recur(base)?),
                    subscripts
                        .iter()
                        .map(&recur)
                        .collect::<Result<Vec<_>, String>>()?,
                ));
            }
            let base = match base.as_ref() {
                Expr::Ref(_) | Expr::Array(_) => (**base).clone(),
                other => recur(other)?,
            };
            let name = match &base {
                Expr::Ref(name) => name.clone(),
                Expr::Array(_) => "a list".to_string(),
                other => {
                    return Err(format!(
                        "only variables can be subscripted, found {other:?}"
                    ))
                }
            };
            // Subscripts see both loop variables and parameters: they
            // must be constant at compile time. Where there are no loop
            // variables the parameters are read as they stand, since
            // copying them to add nothing is what a model with a
            // thousand of them cannot afford.
            let subscript_env = match loop_vars.is_empty() {
                true => None,
                false => {
                    let mut env = consts.clone();
                    env.extend(loop_vars.iter().map(|(k, v)| (k.clone(), *v)));
                    Some(env)
                }
            };
            let subscript_env = subscript_env.as_ref().unwrap_or(consts);
            let mut indices = Vec::new();
            for subscript in subscripts {
                let resolved = recur(subscript)?;
                let value = const_eval(&resolved, subscript_env).ok_or_else(|| {
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
            match &base {
                Expr::Array(items) => {
                    let picked = pick_from_list(items, &indices).ok_or_else(|| {
                        format!("{indices:?} is not an element of a list of {}", items.len())
                    })?;
                    recur(&picked)?
                }
                _ => Expr::Ref(element_name(&name, &indices)),
            }
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
            let args = order_builtin_arguments(name, args?)?;
            match lookup(registry, name, scope, imports) {
                Some(class) if class.kind == ClassKind::Function => {
                    // A function may ask how long what it was handed
                    // is - `size(x, 1)` of an input declared `[:]` -
                    // and the answer is here rather than in the body.
                    let shapes: Vec<Vec<i64>> = args
                        .iter()
                        .map(|arg| match arg {
                            Expr::Ref(name) => sizes.get(name).cloned().unwrap_or_default(),
                            _ => Vec::new(),
                        })
                        .collect();
                    inline_function(class, &args, &shapes, consts, registry, depth + 1)?
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
        Expr::If(c, a, b) => {
            let condition = recur(c)?;
            let mut env = consts.clone();
            env.extend(loop_vars.iter().map(|(k, v)| (k.clone(), *v)));
            let settled = const_eval(&condition, &env);
            let (first, second) = match settled {
                Some(0.0) => (b, a),
                _ => (a, b),
            };
            let first = recur(first)?;
            let second = match (recur(second), settled) {
                (Ok(resolved), _) => resolved,
                // Where the compiler settles the condition, the branch
                // it does not take need not be one this compiler can
                // build: the standard library asks the length of a file
                // name only `if tableOnFile`, and that length has a
                // body written in C. What stands is then all there is.
                (Err(_), Some(_)) => return Ok(first),
                (Err(trouble), None) => return Err(trouble),
            };
            match settled {
                Some(0.0) => Expr::If(Box::new(condition), Box::new(second), Box::new(first)),
                _ => Expr::If(Box::new(condition), Box::new(first), Box::new(second)),
            }
        }
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
