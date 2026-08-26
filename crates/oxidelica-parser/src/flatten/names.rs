//! Resolving names and folding what the compiler can already see:
//! prefixes, constants, substitutions and lookups.

use super::*;
use std::cell::RefCell;

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
        // A call kept whole for its derivative is made while an
        // expression is expanded, and expanding is the last thing that
        // happens to one - so none can be here, where the names are
        // still being given their instance paths.
        Expr::WithDerivative(..) => expr.clone(),
        _ => expr.map_children(&mut |child| recur(child)),
    }
}

/// Every number a value written out holds, however deeply it nests.
///
/// `[a; b]` is a matrix of two rows and `{a, b}` a list of two, and
/// `min` and `max` of either is over all the numbers in it.
fn flat_numbers(expr: &Expr, env: &HashMap<String, f64>, out: &mut Vec<f64>) -> Option<()> {
    match expr {
        Expr::Array(items) => {
            for item in items {
                flat_numbers(item, env, out)?;
            }
        }
        Expr::MatrixRows(rows) => {
            for cell in rows.iter().flatten() {
                flat_numbers(cell, env, out)?;
            }
        }
        one => out.push(const_eval(one, env)?),
    }
    Some(())
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
            // `min` and `max` take two numbers or one array: a block
            // takes the longer of two lengths with
            // `max([size(a, 1); size(b, 1)])`, which is one argument
            // holding both.
            if matches!(operator_name(name), "min" | "max") && args.len() == 1 {
                let mut held = Vec::new();
                flat_numbers(args.first()?, env, &mut held)?;
                let (first, rest) = held.split_first()?;
                return Some(match operator_name(name) {
                    "min" => rest.iter().fold(*first, |a, b| a.min(*b)),
                    _ => rest.iter().fold(*first, |a, b| a.max(*b)),
                });
            }
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
        // The one case this pass is about: a name the map speaks for
        // becomes what the map says, and everything else is the same
        // expression with its children substituted.
        Expr::Ref(name) => map.get(name).cloned().unwrap_or_else(|| expr.clone()),
        _ => expr.map_children(&mut |child| substitute_refs(child, map)),
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
pub(super) fn class_constant_at(
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
    // How long a constant array of the same package is, for the
    // constants that are written as a measurement of one. A medium
    // counts its trace substances as `size(extraPropertiesNames, 1)`,
    // and the names are a constant array with a value of its own -
    // `fill("", 0)` where there are none. Nothing else can answer that
    // length: the array is a constant of a package rather than a
    // declaration anywhere near what it sizes.
    // Constants of one package may build on each other, so resolve the
    // whole set to a fixpoint before reading the one asked for.
    let mut values: HashMap<String, f64> = HashMap::new();
    loop {
        let mut progress = false;
        for (name, binding) in &constants {
            if values.contains_key(name) {
                continue;
            }
            if let Some(value) = binding.as_ref().and_then(|expr| {
                const_eval(expr, &values).or_else(|| measured_constant(expr, &constants, &values))
            }) {
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

/// A constant written as the length of another constant of the same
/// package, worked out.
///
/// A medium counts its trace substances as `size(extraPropertiesNames,
/// 1)`, and the names are a constant array with a value of its own -
/// `fill("", 0)` where there are none. Nothing else can answer that
/// length: the array is a constant of a package rather than a
/// declaration anywhere near what it sizes.
pub(super) fn measured_constant(
    expr: &Expr,
    constants: &[(String, Option<Expr>)],
    values: &HashMap<String, f64>,
) -> Option<f64> {
    let Expr::Call(called, args) = expr else {
        return None;
    };
    if called != "size" || args.len() != 2 {
        return None;
    }
    let Expr::Ref(named) = &args[0] else {
        return None;
    };
    let axis = const_eval(&args[1], values)? as usize - 1;
    let held = constants
        .iter()
        .find(|(name, _)| name == named)?
        .1
        .as_ref()?;
    array_length(held, axis).map(|length| length as f64)
}

/// How many elements a constant value has along one axis, where that
/// can be told by looking: an array written out, or a `fill` and a
/// `zeros` and an `ones`, which is how an empty one is usually said.
pub(super) fn array_length(expr: &Expr, axis: usize) -> Option<i64> {
    let stated = match expr {
        Expr::Array(items) if axis == 0 => return Some(items.len() as i64),
        // `fill(what, n, m)` says its lengths after the value it
        // repeats; `zeros(n)` and `ones(n)` say theirs outright.
        Expr::Call(name, args) if name == "fill" => args.get(axis + 1)?,
        Expr::Call(name, args) if matches!(name.as_str(), "zeros" | "ones") => args.get(axis)?,
        _ => return None,
    };
    let length = const_eval(stated, &HashMap::new())?;
    (length.fract() == 0.0 && length >= 0.0).then_some(length as i64)
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
            // What the `extends` says about the base's constants is
            // what this package holds them to be. A medium of the
            // standard library is written `extends PartialMedium(nC =
            // 2)` and declares no `nC` of its own, so the count read
            // through it came from the interface, which says none -
            // and every connector sized by it came out a run of
            // nothing.
            for (name, value) in &extend.modifiers {
                if name.contains('.') {
                    continue;
                }
                if let Some(held) = out.iter_mut().find(|(existing, _)| existing == name) {
                    held.1 = Some(value.clone());
                }
            }
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
                // The same walk for a constant that comes to a list
                // rather than a number: `NotTable` of a logic package
                // is a constant array, named inside the package as it
                // stands. Left unanswered the name travels into the
                // flat model with the instance path on the front,
                // which nothing declares.
                if let Some(value) = enclosing_constant_array(registry, name, scope, depth) {
                    return value;
                }
            }
            expr.clone()
        }
        Expr::Number(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Time => expr.clone(),
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
        _ => expr.map_children(&mut |child| recur(child)),
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

/// A constant of an enclosing package that comes to a list rather
/// than a number.
///
/// [`enclosing_constant`] answers a name that is a number; a name that
/// is a vector - a logic package's `NotTable`, a colour of three - has
/// no answer there, and left alone it travels into the flat model with
/// the instance path on the front, which nothing declares. The binding
/// is read in the package that holds it, since what it is written in
/// terms of is that package's own.
fn enclosing_constant_array(
    registry: &HashMap<&str, &ClassDef>,
    name: &str,
    scope: &str,
    depth: usize,
) -> Option<Expr> {
    if depth > MAX_CONSTANT_DEPTH {
        return None;
    }
    let mut prefix = scope;
    loop {
        if let Some(owner) = registry
            .get(prefix)
            .filter(|owner| owner.kind == ClassKind::Package)
        {
            let mut constants = Vec::new();
            gather_package_constants(registry, owner, 0, &mut constants);
            if let Some((_, binding)) = constants.iter().find(|(known, _)| known == name) {
                let binding = binding.clone()?;
                let binding = substitute_at(
                    &binding,
                    registry,
                    &owner.name,
                    &owner.imports,
                    &[],
                    depth + 1,
                    true,
                );
                return matches!(binding, Expr::Array(_)).then_some(binding);
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
    if !super::lookup::REGISTRY_STANDS.with(|stands| stands.get()) {
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
    pub(super) static NAMED: RefCell<HashMap<(String, String), Option<f64>>> =
        RefCell::new(HashMap::new());
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
    // A value that is already a number has nothing left to resolve,
    // and nothing below it to run away into. Refusing one for being
    // deep is refusing the arithmetic that led to it rather than any
    // expression: the media library folds a property of a property of
    // a state, and by the time the innermost is a number the count has
    // been raised thirty times by callers that have all come to an end.
    // A value that is already a number has nothing left to resolve,
    // and nothing below it to run away into. Refusing one for being
    // deep is refusing the arithmetic that led to it rather than any
    // expression: the media library folds a property of a property of
    // a state, and by the time the innermost is a number the count has
    // been raised thirty times by callers that have all come to an end.
    if matches!(expr, Expr::Number(_) | Expr::Bool(_) | Expr::Str(_)) {
        return Ok(expr.clone());
    }
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
                    // The body is entered at the depth the call was
                    // written at, not one deeper. How many bodies are
                    // inside one another is counted by the inliner
                    // itself, which has a bound of its own; what this
                    // count is for is how deep one expression nests.
                    // Raising it here spent the expression's budget on
                    // the calls, so a name at the bottom of a property
                    // of a property of a state was refused for being
                    // deep when what was deep was the road to it.
                    inline_function(class, &args, &shapes, consts, registry, depth)?
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
        // Arithmetic and logic hold expressions and nothing this pass
        // has an opinion about.
        Expr::Neg(_)
        | Expr::Not(_)
        | Expr::Bin(..)
        | Expr::Rel(..)
        | Expr::And(..)
        | Expr::Or(..) => expr.try_map_children(&mut |child| recur(child))?,
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
        // A call kept whole for its derivative, and a keyword naming
        // an input rather than a component: both hold expressions and
        // nothing else this pass reads.
        Expr::WithDerivative(..) | Expr::NamedArg(..) => {
            expr.try_map_children(&mut |child| recur(child))?
        }
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
