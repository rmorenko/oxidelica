//! Resolving names and folding what the compiler can already see:
//! prefixes, constants, substitutions and lookups.

use super::*;
use std::cell::RefCell;
use std::collections::HashSet;

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
        // Nothing else is a number: a value written out is several, a
        // range is a span, a `:` is a question for the caller. They
        // are listed rather than swept up, so that a variant added to
        // `Expr` has to be decided about here rather than quietly
        // becoming "not a constant" and surfacing as a refusal three
        // layers further on.
        Expr::Str(_)
        | Expr::Time
        | Expr::WithDerivative(..)
        | Expr::Member(..)
        | Expr::Array(_)
        | Expr::MatrixRows(_)
        | Expr::Elementwise(..)
        | Expr::Range(..)
        | Expr::Comprehension(..)
        | Expr::ColonSubscript
        | Expr::EndSubscript
        | Expr::NamedArg(..)
        | Expr::Tuple(_) => return None,
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

thread_local! {
    /// The bodies a tuple left standing for the run to walk, by the
    /// name they were written under and the scope they were written
    /// in. What such a call was handed has to reach the run whole -
    /// including a table, which nothing else here carries.
    pub(super) static STANDING: RefCell<HashSet<(String, String)>> =
        RefCell::new(HashSet::new());
}

/// Say that a call is one the run will walk, so that what it was
/// handed travels whole.
pub(super) fn stands_for_the_run(name: &str, scope: &str) {
    STANDING.with(|held| {
        held.borrow_mut()
            .insert((name.to_string(), scope.to_string()))
    });
}

/// Whether a call was left standing for the run to walk.
pub(super) fn stands_for_the_run_here(name: &str, scope: &str) -> bool {
    STANDING.with(|held| {
        held.borrow()
            .contains(&(name.to_string(), scope.to_string()))
    })
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
        // A call the run walks takes what it was handed the same way:
        // a table travels as its rows, and the walk lays them out
        // under the two subscripts the body writes. Which call that is
        // was settled where it was left standing; here it is known by
        // the shape of what it was given.
        Expr::Call(name, args)
            if crate::outside::written_here(name)
                || (args.iter().any(|arg| matches!(arg, Expr::Array(_)))
                    && lookup(registry, name, scope, imports)
                        .is_some_and(|class| class.kind == ClassKind::Function)
                    && stands_for_the_run_here(name, scope)) =>
        {
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
            // A body written in a base may call what only a class
            // extending it declares. Asked where it was written there
            // is no such name; asked under the name the body was
            // reached by - the medium the model itself wrote - there
            // is, and that is the one meant.
            let found = lookup(registry, name, scope, imports)
                .or_else(|| inlining::found_under_the_asked_name(registry, name, scope, imports));
            match found {
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
                    // The name the call wrote, held across the body.
                    // Equations take a road that pushes it
                    // (`expand_call`); a parameter binding is settled
                    // down this one, which pushed nothing - so a body
                    // of a medium asked for by a parameter answered
                    // with its base's empty record instead of the
                    // medium's own. The road a call takes must not
                    // decide which class its names land in.
                    let _asked = name.rsplit_once('.').and_then(|(head, _)| {
                        (!class.name.starts_with(head)).then(|| {
                            let package = lookup(registry, head, scope, imports)?;
                            inlining::AskedAs::under(&package.name)
                        })?
                    });
                    let class = inlining::function_asked_under(class, registry);
                    inlining::inline_function(class, &args, &shapes, consts, registry, depth)?
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
            // The same guard the subscripts above are read under, and
            // for the same reason: where there are no loop variables
            // the parameters are read as they stand. Copied on every
            // `if` node instead, the cost of one walk grows with the
            // model - a six-cylinder engine carries a table six times
            // longer past the same expression - and six walks at six
            // times the price is what turns a linear model into a
            // quadratic one.
            let with_loop_vars = match loop_vars.is_empty() {
                true => None,
                false => {
                    let mut env = consts.clone();
                    env.extend(loop_vars.iter().map(|(k, v)| (k.clone(), *v)));
                    Some(env)
                }
            };
            let env = with_loop_vars.as_ref().unwrap_or(consts);
            let settled = const_eval(&condition, env);
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
