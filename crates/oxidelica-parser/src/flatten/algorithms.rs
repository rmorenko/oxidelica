//! Algorithm sections executed symbolically, and functions inlined
//! at the place they were called.

use super::*;
use std::cell::{Cell, RefCell};

thread_local! {
    /// Checks set aside on the way out of an inlined function.
    ///
    /// An `assert` in a function body holds every time the function is
    /// called, and a call written in an equation is made at every step
    /// of the run - so the check is one the whole model carries. The
    /// pass that inlines the call answers with an expression, though,
    /// and an expression has nowhere to put a check. So the checks are
    /// left here, and the class being instantiated - which does have
    /// somewhere to put them - takes them up when it is done.
    static SET_ASIDE: RefCell<Vec<(Expr, String)>> = const { RefCell::new(Vec::new()) };
}

/// How many checks have been set aside so far.
///
/// Work that is thrown away must not leave its checks behind: an `if`
/// branch nobody takes, an unrolling that turned out not to come to an
/// end. Those places note the mark before and rewind to it after.
pub(super) fn checks_mark() -> usize {
    SET_ASIDE.with(|aside| aside.borrow().len())
}

/// Throw away every check set aside since `mark`.
pub(super) fn checks_rewind(mark: usize) {
    SET_ASIDE.with(|aside| aside.borrow_mut().truncate(mark));
}

/// Put a condition in front of every check set aside since `mark`.
///
/// Only the branch an `if` takes is worked out as the run goes, so a
/// check a branch made holds only when that branch is the one taken:
/// either the condition fell the other way, or the check holds.
pub(super) fn checks_guarded(mark: usize, condition: &Expr, on_true: bool) {
    let otherwise = if on_true {
        Expr::Not(Box::new(condition.clone()))
    } else {
        condition.clone()
    };
    SET_ASIDE.with(|aside| {
        for (check, _) in aside.borrow_mut().iter_mut().skip(mark) {
            *check = Expr::Or(Box::new(otherwise.clone()), Box::new(check.clone()));
        }
    });
}

/// Set a check aside for the class being instantiated to take up.
///
/// A table asked for no extrapolation is a table that says the run has
/// gone wrong where it is read outside its own scope. There is nowhere
/// in an expression to put that, so it is left here like the checks an
/// inlined body makes.
pub(super) fn check_aside(condition: Expr, message: String) {
    SET_ASIDE.with(|aside| aside.borrow_mut().push((condition, message)));
}

/// Take every check set aside since `mark`, in the order they came.
pub(super) fn checks_taken(mark: usize) -> Vec<(Expr, String)> {
    SET_ASIDE.with(|aside| aside.borrow_mut().split_off(mark))
}

/// How far one call may be inlined inside another before the answer is
/// that it did not come to an end here.
const MAX_NESTED_CALLS: usize = 64;

thread_local! {
    /// How many calls are being inlined inside one another.
    static INLINING: Cell<usize> = const { Cell::new(0) };
}

/// One step further into the nesting, undone when it goes out of view.
struct Nested;

impl Nested {
    fn deeper() -> Self {
        INLINING.with(|deep| deep.set(deep.get() + 1));
        Nested
    }
}

impl Drop for Nested {
    fn drop(&mut self) {
        INLINING.with(|deep| deep.set(deep.get() - 1));
    }
}

/// How a body says it cannot be unrolled.
///
/// The one place that can answer this differently - by leaving the call
/// standing for the run to walk - looks for these words, so they are
/// written once and matched against rather than repeated.
pub(super) const UNDECIDABLE_LOOP: &str = "the trip count of a loop is not settled here";

/// How an unrolling says it did not come to an end.
///
/// A body that calls itself unrolls only as far as what decides the
/// recursion is settled; an expression that nests deeper than the
/// compiler follows says the same thing about itself. Either way the
/// call is left standing and the run walks it, so the two are matched
/// against by these words rather than by where they came from.
pub(super) const NO_BOTTOM: &str = "did not come to an end here";

/// A body that leaves by a `break` or a `return` the compiler cannot
/// decide is one it cannot write out at all: which statements run is
/// what the leaving decides. Walking it is the answer, so this too is
/// a reason to leave the call standing rather than to refuse.
pub(super) const UNDECIDABLE_LEAVING: &str = "needs a condition the compiler can decide";

/// Whether an expression still holds a call to a function whose calls
/// lead back to itself. Such a call is what an unrolling that stopped
/// short leaves behind.
fn holds_unbounded_call(expr: &Expr, registry: &HashMap<&str, &ClassDef>) -> bool {
    // The names an unrolling leaves behind are already qualified, so
    // the registry answers to them directly.
    let mut called = Vec::new();
    gather_calls(expr, registry, "", &[], &mut called);
    called.iter().any(|name| {
        registry
            .get(name.as_str())
            .is_some_and(|class| class.kind == ClassKind::Function && recursive(class, registry))
    })
}

/// Whether a function's calls lead back to itself, directly or through
/// others. Such a call has no bottom to inline to.
pub(super) fn recursive(class: &ClassDef, registry: &HashMap<&str, &ClassDef>) -> bool {
    let mut seen: Vec<String> = Vec::new();
    let mut wanted: Vec<String> = Vec::new();
    gather_calls_in_statements(
        &class.algorithm,
        registry,
        &class.name,
        &class.imports,
        &mut wanted,
    );
    while let Some(name) = wanted.pop() {
        if name == class.name {
            return true;
        }
        if seen.contains(&name) {
            continue;
        }
        seen.push(name.clone());
        let called = registry[name.as_str()];
        gather_calls_in_statements(
            &called.algorithm,
            registry,
            &called.name,
            &called.imports,
            &mut wanted,
        );
    }
    false
}

/// Whether flow control could fire at this nesting level: a `break` or
/// `return` here or in an `if` here, or a `return` inside a loop here -
/// loops consume their own breaks but a return passes through them.
pub(super) fn has_flow_control(statements: &[Statement]) -> bool {
    statements.iter().any(|statement| match statement {
        Statement::Break | Statement::Return => true,
        Statement::If(branches) => branches.iter().any(|b| has_flow_control(&b.body)),
        Statement::For(_, _, body) | Statement::While(_, body) => has_return(body),
        _ => false,
    })
}

/// What an expression inside a body comes to, where that is a number.
///
/// Arithmetic alone answers most of them, and the array layer answers
/// the rest: a length, a call, an element of a list written out. The
/// layer is only asked where arithmetic could not, since asking it is
/// the expensive half.
#[allow(clippy::too_many_arguments)]
fn settled_in_body(
    expr: &Expr,
    bindings: &HashMap<String, Expr>,
    consts: &HashMap<String, f64>,
    sizes: &HashMap<String, Vec<i64>>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
) -> Option<f64> {
    let expr = substitute_refs(expr, bindings);
    if let Some(number) = const_eval(&expr, consts) {
        return Some(number);
    }
    let shapes = Shapes {
        sizes,
        loop_vars: &HashMap::new(),
        consts,
        records: no_records(),
    };
    let worked = expand(&expr, &shapes, registry, scope, imports, depth).ok()?;
    const_eval(&worked.into_expr(), consts)
}

/// What `for i loop` runs over among statements: the size of the array
/// along the dimension `i` is used to subscript, wherever the body
/// first uses it that way.
pub(super) fn implied_statement_range(
    body: &[Statement],
    variable: &str,
    sizes: &HashMap<String, Vec<i64>>,
) -> Result<Vec<f64>, String> {
    fn look(body: &[Statement], variable: &str, sizes: &HashMap<String, Vec<i64>>) -> Option<i64> {
        body.iter().find_map(|statement| match statement {
            Statement::Assign(name, subscripts, value) => sizes
                .get(name)
                .and_then(|shape| {
                    subscripts
                        .iter()
                        .position(|s| matches!(s, Expr::Ref(used) if used == variable))
                        .and_then(|at| shape.get(at).copied())
                })
                .or_else(|| subscript_extent(value, variable, sizes)),
            Statement::TupleAssign(_, value) => subscript_extent(value, variable, sizes),
            Statement::If(branches) | Statement::When(branches) => branches
                .iter()
                .find_map(|branch| look(&branch.body, variable, sizes)),
            Statement::For(_, _, inner) | Statement::While(_, inner) => {
                look(inner, variable, sizes)
            }
            _ => None,
        })
    }
    match look(body, variable, sizes) {
        Some(extent) => Ok((1..=extent).map(|index| index as f64).collect()),
        None => Err(format!(
            "`for {variable} loop` leaves the range to the body, and nothing in the body \
             uses `{variable}` to subscript an array of a length the compiler knows"
        )),
    }
}

/// Whether a `return` hides anywhere below, loops included.
pub(super) fn has_return(statements: &[Statement]) -> bool {
    statements.iter().any(|statement| match statement {
        Statement::Return => true,
        Statement::If(branches) => branches.iter().any(|b| has_return(&b.body)),
        Statement::For(_, _, body) | Statement::While(_, body) => has_return(body),
        _ => false,
    })
}

/// Whether an expression reads the given name anywhere in it.
///
/// [`mentions_ref`] answers for the nodes a clock has to look through
/// and stops at the rest; this has to be sure, since what it decides
/// is whether a value may be dropped. A name reached through a
/// subscript counts - `o[3]` reads `o` - and so does one asked for as
/// the whole, which is how an array is handed to a call.
pub(super) fn reads_name(expr: &Expr, wanted: &str) -> bool {
    let whole = wanted.split('[').next().unwrap_or(wanted);
    let mut found = false;
    walk_expr(expr, &mut |node| {
        if let Expr::Ref(name) = node {
            let here = name.split('[').next().unwrap_or(name);
            if name == wanted || here == whole {
                found = true;
            }
        }
    });
    found
}

/// Every node of an expression, handed to the given eye.
fn walk_expr(expr: &Expr, eye: &mut impl FnMut(&Expr)) {
    eye(expr);
    match expr {
        Expr::Neg(inner) | Expr::Not(inner) | Expr::Member(inner, _) => walk_expr(inner, eye),
        Expr::Bin(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r)
        | Expr::Elementwise(_, l, r) => {
            walk_expr(l, eye);
            walk_expr(r, eye);
        }
        Expr::If(c, a, b) => {
            walk_expr(c, eye);
            walk_expr(a, eye);
            walk_expr(b, eye);
        }
        Expr::Call(_, args) | Expr::Array(args) => args.iter().for_each(|a| walk_expr(a, eye)),
        Expr::Index(base, subscripts) => {
            walk_expr(base, eye);
            subscripts.iter().for_each(|s| walk_expr(s, eye));
        }
        Expr::Range(a, step, b) => {
            walk_expr(a, eye);
            if let Some(step) = step {
                walk_expr(step, eye);
            }
            walk_expr(b, eye);
        }
        Expr::Comprehension(body, _, range) => {
            walk_expr(body, eye);
            walk_expr(range, eye);
        }
        Expr::MatrixRows(rows) => rows.iter().flatten().for_each(|c| walk_expr(c, eye)),
        Expr::NamedArg(_, value) => walk_expr(value, eye),
        Expr::Tuple(slots) => slots.iter().flatten().for_each(|s| walk_expr(s, eye)),
        Expr::WithDerivative(value, rule, seeds) => {
            walk_expr(value, eye);
            walk_expr(rule, eye);
            seeds.iter().for_each(|(_, arg)| walk_expr(arg, eye));
        }
        Expr::Ref(_)
        | Expr::Number(_)
        | Expr::Bool(_)
        | Expr::Str(_)
        | Expr::Time
        | Expr::ColonSubscript
        | Expr::EndSubscript => {}
    }
}

/// Whether any of these statements still reads the given name.
///
/// A working array of a steam table is filled and used inside one
/// branch of an `if` and never looked at again: `o` of `tph2` holds
/// the powers of a dimensionless pressure while the branch builds a
/// temperature from them. Merging such a name across the branches asks
/// what it should be where a branch never set it, and there is no
/// answer - but there is no question either, since nothing downstream
/// asks. What is read after the `if` is what has to be merged.
pub(super) fn read_later(statements: &[Statement], name: &str, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        // Too deep to say no safely: merging a name that nothing reads
        // costs work, and refusing one that something reads is wrong.
        return true;
    }
    let in_expr = |expr: &Expr| reads_name(expr, name);
    let in_subscripts = |subscripts: &[Expr]| subscripts.iter().any(in_expr);
    statements.iter().any(|statement| match statement {
        // The target of an assignment is written rather than read, but
        // its subscripts are read to find the place.
        Statement::Assign(_, subscripts, value) => in_subscripts(subscripts) || in_expr(value),
        Statement::TupleAssign(targets, value) => {
            in_expr(value)
                || targets
                    .iter()
                    .flatten()
                    .any(|(_, subscripts)| in_subscripts(subscripts))
        }
        Statement::If(branches) | Statement::When(branches) => branches.iter().any(|branch| {
            branch.condition.as_ref().is_some_and(in_expr)
                || read_later(&branch.body, name, depth + 1)
        }),
        Statement::For(_, range, body) => {
            range.as_ref().is_some_and(in_expr) || read_later(body, name, depth + 1)
        }
        Statement::While(condition, body) => {
            in_expr(condition) || read_later(body, name, depth + 1)
        }
        Statement::Assert(condition, _) => in_expr(condition),
        Statement::Call(_, args) => args.iter().any(in_expr),
        Statement::Break | Statement::Return => false,
    })
}

/// Symbolically execute an algorithm section.
///
/// `bindings` maps every variable the section has written to the
/// expression it now holds; reading a variable substitutes that
/// expression, which is what turns a sequence of assignments into one
/// expression per assigned variable. `assigned` collects the targets in
/// the order they were first written, so the equations a model gets out
/// of the section are in source order.
///
/// An `if` runs both ways: each branch is executed on its own copy of
/// the bindings and the results are merged into one `if` expression per
/// variable, with the value from before the statement as the fallback -
/// unless a branch holds `break` or `return`, in which case the
/// conditions must be decidable and only the taken branch runs.
/// A `for` is unrolled, its variable being a compile-time constant.
/// A `while` runs for real: its condition must be decidable each round,
/// and `fold` collapses the loop's assignments to numbers so the
/// expressions do not grow with the iteration count.
#[allow(clippy::too_many_arguments)]
pub(super) fn execute(
    statements: &[Statement],
    bindings: &mut HashMap<String, Expr>,
    assigned: &mut Vec<String>,
    asserts: &mut Vec<(Expr, String)>,
    consts: &HashMap<String, f64>,
    sizes: &HashMap<String, Vec<i64>>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
    fold: bool,
) -> Result<Flow, String> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "an algorithm {NO_BOTTOM}, nested deeper than the compiler follows"
        ));
    }
    for (at, statement) in statements.iter().enumerate() {
        match statement {
            Statement::Assign(target, subscripts, value) => {
                // A body may name a constant of a package the way an
                // equation may, and it is resolved where it was
                // written - in this class, not at the call site.
                let value = substitute_class_constants(value, registry, scope, imports, &[]);
                let value = substitute_refs(&value, bindings);
                // Through the array layer, so `c := a .* b` binds a whole
                // array and a scalar stays a scalar.
                let no_loop_vars = HashMap::new();
                let shapes = Shapes {
                    sizes,
                    loop_vars: &no_loop_vars,
                    consts,
                    records: no_records(),
                };
                let value =
                    expand(&value, &shapes, registry, scope, imports, depth + 1)?.into_expr();
                // Expansion turns `p[i - 1]` into the element's own name,
                // which may itself be bound by an earlier statement - so
                // the bindings are applied once more.
                let value = substitute_refs(&value, bindings);
                // A name bound whole and then written into element by
                // element - `a := zeros(3, 3); a[1, 1] := 5` - has to
                // be taken apart first, or what it was bound to whole
                // would still be what it answers with.
                if !subscripts.is_empty() {
                    if let Some(whole) = bindings.remove(target) {
                        let mut items = Vec::new();
                        fn leaves(expr: &Expr, out: &mut Vec<Expr>) {
                            match expr {
                                Expr::Array(items) => items.iter().for_each(|i| leaves(i, out)),
                                one => out.push(one.clone()),
                            }
                        }
                        leaves(&whole, &mut items);
                        match sizes.get(target) {
                            Some(shape) if index_tuples(shape).len() == items.len() => {
                                for (indices, item) in index_tuples(shape).into_iter().zip(items) {
                                    bindings
                                        .entry(element_name(target, &indices))
                                        .or_insert(item);
                                }
                            }
                            // Nothing says what shape it is, so it goes
                            // back as it was and the write below says
                            // what it can.
                            _ => {
                                bindings.insert(target.clone(), whole);
                            }
                        }
                    }
                }
                // `oM[1:mBase, 1:mBase] := ...` names a run of
                // elements rather than one, and what it is given is
                // that many values: the standard library fills a
                // generator's state and a transformation matrix that
                // way. Each element is assigned its own, which is what
                // the run of names comes to.
                if subscripts.iter().any(|s| matches!(s, Expr::Range(_, _, _))) {
                    let settled = |e: &Expr| {
                        settled_in_body(e, bindings, consts, sizes, registry, scope, imports, depth)
                    };
                    let mut spans: Vec<Vec<i64>> = Vec::new();
                    for subscript in subscripts {
                        let span = match subscript {
                            Expr::Range(from, step, to) => {
                                let (Some(from), Some(to)) = (settled(from), settled(to)) else {
                                    return Err(format!(
                                        "`{target}` is given a run of elements whose bounds \
                                         this compiler cannot see"
                                    ));
                                };
                                let step = step.as_ref().and_then(|s| settled(s)).unwrap_or(1.0);
                                std::iter::successors(Some(from), |at| {
                                    Some(at + step).filter(|next| (next - to) * step <= 0.0)
                                })
                                .map(|at| at as i64)
                                .collect()
                            }
                            one => match settled(one) {
                                Some(at) => vec![at as i64],
                                None => {
                                    return Err(format!(
                                        "the subscript of `{target}` must be a whole number \
                                         the compiler can see"
                                    ))
                                }
                            },
                        };
                        spans.push(span);
                    }
                    // Every element the run covers, in the order the
                    // language writes an array out: the last subscript
                    // moves fastest.
                    let mut named: Vec<Vec<i64>> = vec![Vec::new()];
                    for span in &spans {
                        named = named
                            .iter()
                            .flat_map(|so_far| {
                                span.iter().map(move |at| {
                                    let mut with = so_far.clone();
                                    with.push(*at);
                                    with
                                })
                            })
                            .collect();
                    }
                    // What it is given, written out the same way.
                    fn leaves(expr: &Expr, out: &mut Vec<Expr>) {
                        match expr {
                            Expr::Array(items) => items.iter().for_each(|i| leaves(i, out)),
                            one => out.push(one.clone()),
                        }
                    }
                    let mut items = Vec::new();
                    leaves(&value, &mut items);
                    if items.len() != named.len() {
                        return Err(format!(
                            "`{target}` is given a run of {} element(s) and {} value(s)",
                            named.len(),
                            items.len()
                        ));
                    }
                    for (indices, item) in named.into_iter().zip(items) {
                        let one = element_name(target, &indices);
                        if !assigned.contains(&one) {
                            assigned.push(one.clone());
                        }
                        bindings.insert(one, item);
                    }
                    continue;
                }
                // `c[i] := ...` lands on the element's own name.
                let target = if subscripts.is_empty() {
                    target.clone()
                } else {
                    let indices = subscripts
                        .iter()
                        .map(|subscript| {
                            let subscript = substitute_refs(subscript, bindings);
                            // A subscript may be written with a
                            // length rather than a digit, and only the
                            // array layer knows a length.
                            settled_in_body(
                                &subscript,
                                bindings,
                                consts,
                                sizes,
                                registry,
                                scope,
                                imports,
                                depth,
                            )
                            .filter(|v| v.fract() == 0.0 && *v >= 1.0)
                                .map(|v| v as i64)
                                .ok_or_else(|| {
                                    format!(
                                        "the subscript of `{target}` must be a whole number the compiler can see"
                                    )
                                })
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    element_name(target, &indices)
                };
                if !assigned.contains(&target) {
                    assigned.push(target.clone());
                }
                // Inside a `while`, a value that folds to a number is
                // stored as one, or the expressions would double in
                // size with every round.
                let value = match const_eval(&value, consts) {
                    Some(number) if fold => Expr::Number(number),
                    _ => value,
                };
                bindings.insert(target, value);
            }
            Statement::TupleAssign(targets, value) => {
                let value = substitute_refs(value, bindings);
                let Expr::Call(name, raw_args) = &value else {
                    return Err(
                        "the right side of a tuple assignment must be a function call".into(),
                    );
                };
                let function = lookup(registry, name, scope, imports)
                    .filter(|c| c.kind == ClassKind::Function)
                    .ok_or_else(|| {
                        format!("`{name}` is not a function, so it cannot fill a tuple")
                    })?;
                let no_loop_vars = HashMap::new();
                let shapes = Shapes {
                    sizes,
                    loop_vars: &no_loop_vars,
                    consts,
                    records: no_records(),
                };
                let values = raw_args
                    .iter()
                    .map(|arg| expand(arg, &shapes, registry, scope, imports, depth + 1))
                    .collect::<Result<Vec<_>, String>>()?;
                let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
                let arguments: Vec<Expr> = values
                    .into_iter()
                    .map(|value| substitute_refs(&value.into_expr(), bindings))
                    .collect();
                let outputs = inline_function_outputs(
                    function,
                    &arguments,
                    &argument_shapes,
                    consts,
                    registry,
                    depth + 1,
                )?;
                if targets.len() > outputs.len() {
                    return Err(format!(
                        "`{name}` has {} output(s) for {} target(s)",
                        outputs.len(),
                        targets.len()
                    ));
                }
                for (slot, (_, output)) in targets.iter().zip(outputs) {
                    let Some((target, subscripts)) = slot else { continue };
                    if !subscripts.is_empty() {
                        return Err(format!(
                            "`{target}` takes part of an array from a call filling several \
                             targets, which is more than this compiler does"
                        ));
                    }
                    if !assigned.contains(target) {
                        assigned.push(target.clone());
                    }
                    bindings.insert(target.clone(), output);
                }
            }
            Statement::If(branches) => {
                // A condition the compiler can decide picks one branch,
                // and only that one runs. Merging both would be the
                // same answer written at greater length - and where a
                // body calls itself, it would be no answer at all: the
                // branch that ends the recursion cannot end it if the
                // branch that continues it is taken as well.
                let decidable = branches.iter().all(|branch| {
                    branch.condition.as_ref().is_none_or(|condition| {
                        const_eval(&substitute_refs(condition, bindings), consts).is_some()
                    })
                });
                // A branch that may `break` or `return` cannot be
                // merged symbolically either - whether it fires must be
                // known. The conditions are decided and only the taken
                // branch runs, its flow passed on.
                if decidable || branches.iter().any(|b| has_flow_control(&b.body)) {
                    let mut taken = None;
                    for branch in branches {
                        match &branch.condition {
                            None => {
                                taken = Some(&branch.body);
                                break;
                            }
                            Some(condition) => {
                                let condition = substitute_refs(condition, bindings);
                                let value = const_eval(&condition, consts).ok_or_else(|| {
                                    format!(
                                        "a branch holding `break` or `return` \
                                         {UNDECIDABLE_LEAVING}"
                                    )
                                })?;
                                if value != 0.0 {
                                    taken = Some(&branch.body);
                                    break;
                                }
                            }
                        }
                    }
                    if let Some(body) = taken {
                        let flow = execute(
                            body,
                            bindings,
                            assigned,
                            asserts,
                            consts,
                            sizes,
                            registry,
                            scope,
                            imports,
                            depth + 1,
                            fold,
                        )?;
                        if flow != Flow::Normal {
                            return Ok(flow);
                        }
                    }
                    continue;
                }
                let before = bindings.clone();
                let mut outcomes: Vec<(Option<Expr>, HashMap<String, Expr>)> = Vec::new();
                for branch in branches {
                    let mut local = before.clone();
                    execute(
                        &branch.body,
                        &mut local,
                        assigned,
                        asserts,
                        consts,
                        sizes,
                        registry,
                        scope,
                        imports,
                        depth + 1,
                        fold,
                    )?;
                    let condition = branch
                        .condition
                        .as_ref()
                        .map(|c| {
                            let c = substitute_class_constants(c, registry, scope, imports, &[]);
                            let c = substitute_refs(&c, &before);
                            // The condition has to come to one truth,
                            // but may be written over arrays to get
                            // there: `if Q*Q_guess >= 0` asks which of
                            // two four-vectors points the same way.
                            let no_loop_vars = HashMap::new();
                            let shapes = Shapes {
                                sizes,
                                loop_vars: &no_loop_vars,
                                consts,
                                records: no_records(),
                            };
                            expand(&c, &shapes, registry, scope, imports, depth + 1)?.scalar()
                        })
                        .transpose()?;
                    outcomes.push((condition, local));
                }
                // Every variable any branch wrote gets one merged value.
                let mut touched: Vec<String> = Vec::new();
                for (_, local) in &outcomes {
                    for name in local.keys() {
                        if before.get(name) != local.get(name) && !touched.contains(name) {
                            touched.push(name.clone());
                        }
                    }
                }
                touched.sort();
                for name in touched {
                    // A working array filled and used inside one branch
                    // and never looked at again needs no merged value:
                    // `o` of the steam tables holds the powers of a
                    // pressure while the branch builds a temperature
                    // from them, and asking what it should be where
                    // another branch never set it has no answer and no
                    // question. Fifty-three models stood at that
                    // refusal. What is read after the `if` is merged as
                    // before; the rest is left in the branch it belongs
                    // to, which is also the cheaper answer, since a
                    // merged array is a nest of conditions expanded
                    // again at every use.
                    //
                    // Only an array is left behind this way. A scalar
                    // costs nothing to merge, and one of them is the
                    // function's own output, which is read by whoever
                    // called rather than by any statement here - so
                    // asking these statements alone would drop it.
                    // The merged names are the elements - `o[3]` -
                    // while the declaration is of the whole, so the
                    // subscripts come off before asking.
                    let whole = name.split('[').next().unwrap_or(&name);
                    let an_array = sizes.contains_key(whole);
                    if an_array
                        && !read_later(&statements[at + 1..], &name, 0)
                        && !read_later(&statements[at + 1..], whole, 0)
                    {
                        continue;
                    }
                    // A variable of a function body that no branch
                    // wrote still has a value: the language says an
                    // unassigned local starts at what its type starts
                    // at, and a `Real` starts at zero. The steam
                    // tables are written that way on purpose - the
                    // boiling curve fills `cp` on one side of the
                    // region 3 boundary and `cv` on the other, and
                    // each is meant to be left at zero where the other
                    // was set. Refusing that took thirty-two models
                    // out, the whole of the Fluid examples among them.
                    // Outside a function the same shape is a real
                    // mistake and is still refused: a model's
                    // algorithm has to say what the variable is before
                    // the `if` decides whether to change it.
                    // An `if` whose branches write arrays is left
                    // alone: a quaternion conversion assigns four
                    // elements in each of four branches, and giving
                    // its scalars a start lets the whole thing be
                    // inlined - four elements of nested conditions,
                    // expanded again at every use. One multi-body
                    // model went from a second and a half to half a
                    // minute that way, and the library from seventeen
                    // seconds to a quarter of an hour. What the
                    // library needs this rule for is bodies that
                    // decide a scalar or a record field one way or
                    // another, and those cost nothing.
                    let writes_arrays = outcomes.iter().any(|(_, local)| {
                        local.keys().any(|written| sizes.contains_key(written))
                    });
                    let fallback = before.get(&name).cloned().or_else(|| {
                        let start = starts_at(&name, registry, scope)?;
                        // A flag costs nothing to give a start to: it
                        // decides a branch rather than being folded
                        // into arithmetic, so it cannot grow the value
                        // the way a run of numbers can. Neither does a
                        // string: it is settled before the run and has
                        // no place in the arithmetic at all.
                        let is_flag = matches!(start, Expr::Bool(_) | Expr::Str(_));
                        if writes_arrays && !is_flag {
                            return None;
                        }
                        Some(start)
                    });
                    let mut value = match outcomes.last() {
                        // A trailing `else` supplies the last value.
                        Some((None, local)) => local.get(&name).cloned().or(fallback.clone()),
                        _ => fallback.clone(),
                    };
                    for (condition, local) in outcomes.iter().rev() {
                        let Some(condition) = condition else { continue };
                        let taken = local.get(&name).cloned().or_else(|| fallback.clone());
                        match (taken, value) {
                            (Some(taken), Some(otherwise)) => {
                                value = Some(Expr::If(
                                    Box::new(condition.clone()),
                                    Box::new(taken),
                                    Box::new(otherwise),
                                ));
                            }
                            _ => {
                                return Err(format!(
                                    "`{name}` is assigned in one branch only and has no value before the `if`"
                                ))
                            }
                        }
                    }
                    let Some(value) = value else {
                        return Err(format!(
                            "`{name}` is assigned in one branch only and has no value before the `if`"
                        ));
                    };
                    bindings.insert(name, value);
                }
            }
            // A check written where the statements are, carried out
            // to the model with whatever the section has assigned so
            // far already substituted into it.
            Statement::Assert(condition, message) => {
                // The check is worked out here, where the names it was
                // written with mean what they meant to whoever wrote
                // them: a body that says `length(n)` with `length`
                // imported is asking for that function, and the place
                // the check ends up has never heard of the import.
                let condition = substitute_class_constants(condition, registry, scope, imports, &[]);
                let condition = substitute_refs(&condition, bindings);
                let no_loop_vars = HashMap::new();
                let shapes = Shapes {
                    sizes,
                    loop_vars: &no_loop_vars,
                    consts,
                    records: no_records(),
                };
                let condition = expand(&condition, &shapes, registry, scope, imports, depth + 1)?
                    .into_expr();
                asserts.push((substitute_refs(&condition, bindings), message.clone()));
            }
            // A call on its own: nothing takes its outputs, so what it
            // was written for is the checks its body makes, and those
            // join the section's.
            Statement::Call(name, args) => {
                let called = lookup(registry, name, scope, imports)
                    .filter(|c| c.kind == ClassKind::Function)
                    .ok_or_else(|| format!("`{name}` is not a function"))?;
                let no_loop_vars = HashMap::new();
                let shapes = Shapes {
                    sizes,
                    loop_vars: &no_loop_vars,
                    consts,
                    records: no_records(),
                };
                let values = args
                    .iter()
                    .map(|arg| expand(arg, &shapes, registry, scope, imports, depth + 1))
                    .collect::<Result<Vec<_>, String>>()?;
                let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
                let arguments: Vec<Expr> = values
                    .into_iter()
                    .map(|value| substitute_refs(&value.into_expr(), bindings))
                    .collect();
                let checks = inline_function_checks(
                    called,
                    &arguments,
                    &argument_shapes,
                    consts,
                    registry,
                    depth + 1,
                )?;
                asserts.extend(checks);
            }
            Statement::For(variable, range, body) => {
                let values = match range {
                    Some(range) => {
                        let expr = substitute_refs(range, bindings);
                        // Through the array layer first, so a range
                        // written `1:size(v, 1)` is a list of numbers by
                        // the time it is asked to be constant - and so
                        // that `{1, 3, 5}` and the name of an array come
                        // out as the same kind of list.
                        let no_loop_vars = HashMap::new();
                        let spread = expand(
                            &expr,
                            &Shapes {
                                sizes,
                                loop_vars: &no_loop_vars,
                                consts,
                                records: no_records(),
                            },
                            registry,
                            scope,
                            imports,
                            depth + 1,
                        )?;
                        loop_values(&spread, consts, variable)?
                    }
                    None => implied_statement_range(body, variable, sizes)?,
                };
                for index in values {
                    bindings.insert(variable.clone(), Expr::Number(index));
                    let flow = execute(
                        body,
                        bindings,
                        assigned,
                        asserts,
                        consts,
                        sizes,
                        registry,
                        scope,
                        imports,
                        depth + 1,
                        fold,
                    )?;
                    match flow {
                        Flow::Normal => {}
                        Flow::Break => break,
                        Flow::Return => {
                            bindings.remove(variable);
                            assigned.retain(|name| name != variable);
                            return Ok(Flow::Return);
                        }
                    }
                }
                bindings.remove(variable);
                assigned.retain(|name| name != variable);
            }
            Statement::While(condition, body) => {
                let mut rounds = 0;
                loop {
                    let now = substitute_refs(condition, bindings);
                    let truth = const_eval(&now, consts).ok_or_else(|| {
                        format!(
                            "{UNDECIDABLE_LOOP}: a `while` here is unrolled, so the trip \
                             count cannot depend on a simulated variable"
                        )
                    })?;
                    if truth == 0.0 {
                        break;
                    }
                    rounds += 1;
                    if rounds > MAX_WHILE_ROUNDS {
                        return Err(format!(
                            "a `while` did not finish within {MAX_WHILE_ROUNDS} rounds"
                        ));
                    }
                    let flow = execute(
                        body,
                        bindings,
                        assigned,
                        asserts,
                        consts,
                        sizes,
                        registry,
                        scope,
                        imports,
                        depth + 1,
                        true,
                    )?;
                    match flow {
                        Flow::Normal => {}
                        Flow::Break => break,
                        Flow::Return => return Ok(Flow::Return),
                    }
                }
            }
            Statement::Break => return Ok(Flow::Break),
            Statement::Return => return Ok(Flow::Return),
            // A `when` is lifted out of the section before the rest of
            // it is executed, so nothing should arrive here.
            Statement::When(_) => {
                return Err("a `when` may sit at the top of a model's algorithm section, not inside an `if`, a loop or a function".to_string())
            }
        }
    }
    Ok(Flow::Normal)
}

/// Inline a function call: arguments are bound to the inputs, the
/// algorithm's assignments are substituted in order, and the output
/// expression replaces the call.
/// Inline a call in an expression: the value is the first output. A
/// function with several outputs may still be called this way; the
/// rest are computed for nothing and dropped, as the spec allows.
pub(super) fn inline_function(
    class: &ClassDef,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
) -> Result<Expr, String> {
    // A call nothing can inline is left standing, and the run walks the
    // body for itself. Two things cannot be inlined: a loop whose trip
    // count the model decides rather than the compiler, and a
    // recursion that does not come to an end here.
    //
    // Neither is known by looking. A function that leads back to
    // itself unrolls perfectly well when what decides the recursion is
    // settled - the standard library builds an m-phase winding by
    // halving m until it is odd, and with m a parameter every step of
    // that is decidable. So the unrolling is tried, and the depth
    // guard inside is what says it will not come to an end.
    // A body is inlined into a body into a body: the media library
    // asks a property of a property of a state, and each step starts
    // its own count. What is nested this deep did not come to an end
    // by inlining, so the call is left standing and the run walks it.
    if INLINING.with(|deep| deep.get()) > MAX_NESTED_CALLS {
        return Ok(Expr::Call(class.name.clone(), args.to_vec()));
    }
    let _nested = Nested::deeper();
    let standing = || Ok(Expr::Call(class.name.clone(), args.to_vec()));
    // Where a body leads back to itself the unrolling is a try rather
    // than a demand: the walk is waiting behind it, so anything the
    // inliner will not do - a loop it cannot unroll, a shape it cannot
    // carry, a recursion with no bottom - means the call stands and
    // the run walks it. For a body that does not lead back to itself
    // there is nothing behind, and a refusal is a refusal.
    let speculative = recursive(class, registry);
    // A call that ends up standing was not inlined, so whatever checks
    // the attempt set aside are checks of a body nobody ran.
    let mark = checks_mark();
    let standing = || {
        checks_rewind(mark);
        standing()
    };
    let attempt = inline_function_outputs(class, args, shapes, consts, registry, depth);
    let mut outputs = match attempt {
        Ok(outputs) => outputs,
        // A body that leaves by a `break` or a `return` the compiler
        // cannot decide is one it cannot write out at all: which
        // statements run is what the leaving decides. Walking it is
        // the answer, so the call stands - unless a walk could not
        // carry what the body takes or answers with, and then the
        // refusal is a refusal after all.
        Err(why)
            if why.starts_with(UNDECIDABLE_LOOP)
                || why.contains(NO_BOTTOM)
                || (why.contains(UNDECIDABLE_LEAVING) && walkable(class).is_ok()) =>
        {
            return standing()
        }
        Err(_) if speculative => return standing(),
        Err(why) => {
            checks_rewind(mark);
            return Err(why);
        }
    };
    // An unrolling that still holds a call of its own cycle did not
    // reach the bottom: it stopped where the compiler stopped
    // following, and what it left behind is the same call under a pile
    // of conditions. The call is better off standing.
    if outputs
        .iter()
        .any(|(_, value)| holds_unbounded_call(value, registry))
    {
        return standing();
    }
    let value = outputs.remove(0).1;
    // What a function says about its own inverse is checked and then
    // set aside. The nonlinear corrector already solves `f(x) = u` for
    // `x` where an inverse would have said the answer outright, so
    // reaching for one would save work rather than make anything
    // possible - and an annotation nobody reads is still worth
    // refusing when it names something that is not there.
    check_inverse(class, registry)?;
    // A function that said how to differentiate itself is inlined for
    // its value like any other, and keeps its rule beside it - so a
    // body the differentiator could not read is still differentiable.
    match &class.derivative {
        None => Ok(value),
        Some(named) => {
            let rule = derivative_rule(class, named, args, shapes, consts, registry, depth)?;
            Ok(Expr::WithDerivative(
                Box::new(value),
                Box::new(rule.0),
                rule.1,
            ))
        }
    }
}

/// The functions a flat model still calls, and everything they call in
/// turn.
///
/// A call standing in a flat model is one nothing could inline, so its
/// body has to travel with the model for the run to walk. What such a
/// body may hold is narrower than what an inlined one may: the run
/// carries numbers and nothing else, so an array or a string inside one
/// is refused here rather than at the first step of a simulation.
pub(super) fn programs_used(
    model: &Model,
    registry: &HashMap<&str, &ClassDef>,
) -> Result<Vec<ClassDef>, String> {
    let mut wanted: Vec<String> = Vec::new();
    // What the flat model itself calls is named the way the registry
    // knows it: flattening qualified it on the way out.
    let mut look = |expr: &Expr| gather_calls(expr, registry, "", &[], &mut wanted);
    for equation in model.equations.iter().chain(&model.initial_equations) {
        look(&equation.lhs);
        look(&equation.rhs);
    }
    for (condition, _) in &model.asserts {
        look(condition);
    }
    for clause in &model.when_clauses {
        for branch in &clause.branches {
            look(&branch.condition);
            for action in &branch.actions {
                match action {
                    WhenAction::Assign(_, value)
                    | WhenAction::Reinit(_, value)
                    | WhenAction::TupleAssign(_, value) => look(value),
                    // A call on its own names a body to be walked
                    // the same way one inside an expression does.
                    WhenAction::Call(name, args) => {
                        // The call itself is named the way an
                        // expression's would be, so it goes through the
                        // same gathering rather than round it.
                        look(&Expr::Call(name.clone(), args.clone()));
                    }
                    // A check made at the event may call as freely as
                    // any other expression.
                    WhenAction::Assert(condition, _) => look(condition),
                    WhenAction::Terminate(_) => {}
                    // Taken apart while flattening, so neither a loop
                    // nor a choice is left.
                    WhenAction::Loop(_) | WhenAction::Choice(_) => {}
                }
            }
        }
    }
    // Everything those call, and everything that calls in turn.
    let mut out: Vec<ClassDef> = Vec::new();
    while let Some(name) = wanted.pop() {
        if out.iter().any(|already| already.name == name) {
            continue;
        }
        let class = registry[name.as_str()];
        walkable(class)?;
        // A body names what it calls the way it was written there; the
        // walk looks names up in one table, so they are made to agree.
        let mut carried = (*class).clone();
        let renamed = records_as_arrays(&mut carried, registry);
        carried.algorithm = qualified_calls(
            &class.algorithm,
            registry,
            &class.name,
            &class.imports,
            &renamed,
        );
        out.push(carried);
        gather_calls_in_statements(
            &class.algorithm,
            registry,
            &class.name,
            &class.imports,
            &mut wanted,
        );
    }
    Ok(out)
}

/// Every record a body deals in written as an array of its members.
///
/// A walk carries numbers under names, and an array is those names
/// subscripted - `v[2]`. A record is the same thing under another
/// spelling, so it is given that spelling here: `bpro.cp` becomes
/// `bpro[7]`, in the order the record declared its members. Nothing in
/// the walk then has to know what a record is.
///
/// Only a record of plain numbers, though. One holding an array or
/// another record would need more than a name and a subscript, and is
/// left as it was for the walk to refuse by name.
fn records_as_arrays(
    class: &mut ClassDef,
    registry: &HashMap<&str, &ClassDef>,
) -> HashMap<String, Expr> {
    let mut renamed: HashMap<String, Expr> = HashMap::new();
    for component in &mut class.components {
        let Some(of) = lookup(registry, &component.type_name, &class.name, &class.imports)
            .filter(|of| of.kind == ClassKind::Record)
        else {
            continue;
        };
        let members = record_fields(of);
        let plain = of
            .components
            .iter()
            .all(|member| member.dimensions.is_empty() && is_primitive(&member.type_name));
        if !plain || members.is_empty() {
            continue;
        }
        for (index, member) in members.iter().enumerate() {
            renamed.insert(
                format!("{}.{member}", component.name),
                Expr::Ref(format!("{}[{}]", component.name, index + 1)),
            );
        }
        component.type_name = "Real".to_string();
        component.dimensions = vec![Expr::Number(members.len() as f64)];
    }
    renamed
}

/// The same statements with every call to a user function named the
/// way the registry knows it.
fn qualified_calls(
    body: &[Statement],
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    renamed: &HashMap<String, Expr>,
) -> Vec<Statement> {
    let inner = |body: &[Statement]| qualified_calls(body, registry, scope, imports, renamed);
    let expr = |e: &Expr| substitute_refs(&qualified_in(e, registry, scope, imports), renamed);
    // A member of a record is written as an element of an array, and a
    // statement may be filling one.
    let target = |name: &String| match renamed.get(name) {
        Some(Expr::Ref(instead)) => instead.clone(),
        _ => name.clone(),
    };
    body.iter()
        .map(|statement| match statement {
            Statement::Assign(name, subscripts, value) => Statement::Assign(
                target(name),
                subscripts.iter().map(&expr).collect(),
                expr(value),
            ),
            Statement::TupleAssign(targets, value) => Statement::TupleAssign(
                targets
                    .iter()
                    .map(|slot| {
                        slot.as_ref()
                            .map(|(name, subs)| (target(name), subs.clone()))
                    })
                    .collect(),
                expr(value),
            ),
            Statement::Assert(condition, message) => {
                Statement::Assert(expr(condition), message.clone())
            }
            Statement::Call(name, args) => Statement::Call(
                lookup(registry, name, scope, imports)
                    .map(|class| class.name.clone())
                    .unwrap_or_else(|| name.clone()),
                args.iter().map(&expr).collect(),
            ),
            Statement::If(branches) => Statement::If(rebranch(branches, &expr, &inner)),
            Statement::When(branches) => Statement::When(rebranch(branches, &expr, &inner)),
            Statement::For(variable, range, body) => {
                Statement::For(variable.clone(), range.as_ref().map(&expr), inner(body))
            }
            Statement::While(condition, body) => Statement::While(expr(condition), inner(body)),
            Statement::Break => Statement::Break,
            Statement::Return => Statement::Return,
        })
        .collect()
}

/// The branches of an `if` or a `when`, rebuilt through the same two
/// rewrites.
fn rebranch(
    branches: &[StatementBranch],
    expr: &impl Fn(&Expr) -> Expr,
    inner: &impl Fn(&[Statement]) -> Vec<Statement>,
) -> Vec<StatementBranch> {
    branches
        .iter()
        .map(|branch| StatementBranch {
            condition: branch.condition.as_ref().map(expr),
            body: inner(&branch.body),
        })
        .collect()
}

/// The same expression with every call to a user function named the way
/// the registry knows it.
fn qualified_in(
    expr: &Expr,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
) -> Expr {
    let recur = |inner: &Expr| qualified_in(inner, registry, scope, imports);
    match expr {
        Expr::Call(name, args) => {
            let named = lookup(registry, name, scope, imports)
                .filter(|class| class.kind == ClassKind::Function)
                .map(|class| class.name.clone())
                .unwrap_or_else(|| name.clone());
            Expr::Call(named, args.iter().map(recur).collect())
        }
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::If(c, a, b) => Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b))),
        Expr::Range(a, step, b) => Expr::Range(
            Box::new(recur(a)),
            step.as_ref().map(|s| Box::new(recur(s))),
            Box::new(recur(b)),
        ),
        Expr::Array(items) => Expr::Array(items.iter().map(recur).collect()),
        _ => expr.clone(),
    }
}

/// Every user function an expression calls.
fn gather_calls(
    expr: &Expr,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    out: &mut Vec<String>,
) {
    // A body names what it calls the way it was written there, so the
    // name is resolved where it was written before it is filed under
    // the one the registry knows it by.
    if let Expr::Call(name, _) = expr {
        if let Some(class) = lookup(registry, name, scope, imports) {
            if class.kind == ClassKind::Function {
                out.push(class.name.clone());
            }
        }
    }
    match expr {
        Expr::Call(_, args) => args
            .iter()
            .for_each(|arg| gather_calls(arg, registry, scope, imports, out)),
        Expr::WithDerivative(value, rule, seeds) => {
            gather_calls(value, registry, scope, imports, out);
            gather_calls(rule, registry, scope, imports, out);
            seeds
                .iter()
                .for_each(|(_, arg)| gather_calls(arg, registry, scope, imports, out));
        }
        Expr::Neg(inner) | Expr::Not(inner) => gather_calls(inner, registry, scope, imports, out),
        Expr::Bin(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r)
        | Expr::Elementwise(_, l, r) => {
            gather_calls(l, registry, scope, imports, out);
            gather_calls(r, registry, scope, imports, out);
        }
        Expr::If(c, a, b) => {
            gather_calls(c, registry, scope, imports, out);
            gather_calls(a, registry, scope, imports, out);
            gather_calls(b, registry, scope, imports, out);
        }
        // `f(x)[2]` - a call answering with several numbers, asked for
        // one of them. The call is under the subscript.
        Expr::Index(base, _) => gather_calls(base, registry, scope, imports, out),
        _ => {}
    }
}

/// Every user function the statements of a body call.
fn gather_calls_in_statements(
    body: &[Statement],
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    out: &mut Vec<String>,
) {
    for statement in body {
        match statement {
            Statement::Assign(_, subscripts, value) => {
                subscripts
                    .iter()
                    .for_each(|s| gather_calls(s, registry, scope, imports, out));
                gather_calls(value, registry, scope, imports, out);
            }
            Statement::TupleAssign(_, value) => gather_calls(value, registry, scope, imports, out),
            Statement::Assert(condition, _) => {
                gather_calls(condition, registry, scope, imports, out)
            }
            Statement::Call(name, args) => {
                if let Some(class) = lookup(registry, name, scope, imports) {
                    out.push(class.name.clone());
                }
                args.iter()
                    .for_each(|arg| gather_calls(arg, registry, scope, imports, out));
            }
            Statement::If(branches) | Statement::When(branches) => {
                for branch in branches {
                    if let Some(condition) = &branch.condition {
                        gather_calls(condition, registry, scope, imports, out);
                    }
                    gather_calls_in_statements(&branch.body, registry, scope, imports, out);
                }
            }
            Statement::For(_, range, inner) => {
                if let Some(range) = range {
                    gather_calls(range, registry, scope, imports, out);
                }
                gather_calls_in_statements(inner, registry, scope, imports, out);
            }
            Statement::While(condition, inner) => {
                gather_calls(condition, registry, scope, imports, out);
                gather_calls_in_statements(inner, registry, scope, imports, out);
            }
            Statement::Break | Statement::Return => {}
        }
    }
}

/// What a body the run walks may be made of. The run carries numbers,
/// so anything shaped otherwise is refused here rather than left to
/// fail at the first step.
fn walkable(class: &ClassDef) -> Result<(), String> {
    for component in &class.components {
        // An array goes in, is held while the walk runs, and may come
        // back: a body answering with several numbers is asked once for
        // each of them. Only a length the compiler can see, though -
        // the model has to name every element it takes.
        if component.causality == Causality::Output && !component.dimensions.is_empty() {
            let [Expr::Number(_)] = component.dimensions.as_slice() else {
                return Err(format!(
                    "`{}` is called where nothing could inline it, so the run walks its body \
                     - and it answers with `{}`, whose length is not one the compiler can \
                     see",
                    class.name, component.name
                ));
            };
        }
        if component.type_name == "String" {
            return Err(format!(
                "`{}` is called where nothing could inline it, so the run walks its body - \
                 and `{}` is a String, which no step carries",
                class.name, component.name
            ));
        }
    }
    if class
        .components
        .iter()
        .filter(|c| c.causality == Causality::Output)
        .count()
        != 1
    {
        return Err(format!(
            "`{}` is called where nothing could inline it, so the run walks its body - \
             and a body walked at run time gives one output, not {}",
            class.name,
            class
                .components
                .iter()
                .filter(|c| c.causality == Causality::Output)
                .count()
        ));
    }
    Ok(())
}

/// Check what a function says about its own inverse: the function it
/// names has to be there, the input it claims to solve for has to be
/// one of its own, and what it hands the inverse has to be something it
/// has to hand.
fn check_inverse(class: &ClassDef, registry: &HashMap<&str, &ClassDef>) -> Result<(), String> {
    let named = |wanted: &str, causality: Causality| {
        class
            .components
            .iter()
            .any(|component| component.name == wanted && component.causality == causality)
    };
    for (solved_for, called, arguments) in &class.inverse {
        if lookup(registry, called, &class.name, &class.imports).is_none() {
            return Err(format!(
                "`{}` says `{called}` inverts it, and there is no such function",
                class.name
            ));
        }
        if !named(solved_for, Causality::Input) {
            return Err(format!(
                "`{}` says its inverse solves for `{solved_for}`, which is not one of its \
                 inputs",
                class.name
            ));
        }
        for argument in arguments {
            if !named(argument, Causality::Input) && !named(argument, Causality::Output) {
                return Err(format!(
                    "the inverse of `{}` is handed `{argument}`, which `{}` neither takes \
                     nor gives",
                    class.name, class.name
                ));
            }
        }
    }
    Ok(())
}

/// Inline the function a `derivative` annotation names, leaving a name
/// where each argument's own derivative belongs.
///
/// The derivative function takes what the original takes and then one
/// more for each of those, in the same order - so the second half of
/// its inputs is what the rule is a rule about.
#[allow(clippy::too_many_arguments)]
fn derivative_rule(
    class: &ClassDef,
    named: &str,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
) -> Result<(Expr, Vec<(String, Expr)>), String> {
    let of = lookup(registry, named, &class.name, &class.imports).ok_or_else(|| {
        format!(
            "`{}` says its derivative is `{named}`, and there is no such function",
            class.name
        )
    })?;
    let inputs = |class: &ClassDef| {
        class
            .components
            .iter()
            .filter(|component| component.causality == Causality::Input)
            .count()
    };
    // Only what can be differentiated gets a derivative handed to it.
    // A table is asked for a value by `(tableID, column, u)`, and
    // neither the table nor the column has a rate of change: the
    // derivative function takes the three and then `der_u` alone.
    let differentiable: Vec<bool> = class
        .components
        .iter()
        .filter(|component| component.causality == Causality::Input)
        .map(|component| is_real(registry, component, &class.name, &class.imports))
        .collect();
    let seeded = differentiable.iter().filter(|real| **real).count();
    let (given, wanted) = (inputs(of), args.len() + seeded);
    if given != wanted {
        return Err(format!(
            "`{named}` is the derivative of `{}`, so it takes {wanted} inputs - what `{}` \
             takes, and then one derivative for each - but it takes {given}",
            class.name, class.name
        ));
    }
    // The names standing in for the derivatives are the compiler's own,
    // so nothing a model can write collides with them.
    let seeds: Vec<(String, Expr)> = args
        .iter()
        .enumerate()
        .filter(|(index, _)| differentiable.get(*index).copied().unwrap_or(true))
        .map(|(index, argument)| (format!("$seed{index}"), argument.clone()))
        .collect();
    let handed: Vec<Expr> = args
        .iter()
        .cloned()
        .chain(seeds.iter().map(|(name, _)| Expr::Ref(name.clone())))
        .collect();
    let seeded_shapes: Vec<Vec<i64>> = args
        .iter()
        .enumerate()
        .filter(|(index, _)| differentiable.get(*index).copied().unwrap_or(true))
        .filter_map(|(index, _)| shapes.get(index).cloned())
        .collect();
    let shapes: Vec<Vec<i64>> = shapes.iter().cloned().chain(seeded_shapes).collect();
    let rule = inline_function(of, &handed, &shapes, consts, registry, depth + 1)?;
    Ok((rule, seeds))
}

/// Execute a function body symbolically and return every output, in
/// declaration order, as `(name, expression)`. Arguments are matched
/// positionally, then by name (`f(x, precision = 6)`); an input left
/// unmatched falls back to its declared default.
pub(super) fn inline_function_outputs(
    class: &ClassDef,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
) -> Result<Vec<(String, Expr)>, String> {
    let mut checks = Vec::new();
    let outputs = inline_body(class, args, shapes, consts, registry, depth, &mut checks)?;
    if outputs.is_empty() {
        return Err(format!("function `{}` declares no output", class.name));
    }
    // An `assert` in a function body cannot travel out through the
    // expression the call becomes, so it is set aside for the model
    // being built to take up.
    if !checks.is_empty() {
        SET_ASIDE.with(|aside| aside.borrow_mut().extend(checks));
    }
    Ok(outputs)
}

/// The checks a call makes, for a call that stands on its own as a
/// statement. Nothing receives its outputs, so what is left of it is
/// what its body asserted - and here, unlike in an expression, there
/// is somewhere for that to go.
pub(super) fn inline_function_checks(
    class: &ClassDef,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
) -> Result<Vec<(Expr, String)>, String> {
    // A body written outside Modelica that answers with nothing is
    // there for what it does rather than for what it says:
    // `Streams.print(...)` writes a line on the terminal. There is no
    // terminal here and no value to miss, so the call does nothing and
    // the run is the same run. A body that does answer is another
    // matter - its value is wanted, and it is refused below.
    if class.external
        && !class
            .components
            .iter()
            .any(|c| c.causality == Causality::Output)
    {
        return Ok(Vec::new());
    }
    let mut checks = Vec::new();
    inline_body(class, args, shapes, consts, registry, depth, &mut checks)?;
    Ok(checks)
}

/// What to say about a function whose body is written outside
/// Modelica and which nobody here answers for.
///
/// The declaration says what is called and what it is handed, so the
/// refusal says it too: a name to look for is worth more than the fact
/// that there is one.
pub(super) fn outside_this_language(class: &ClassDef) -> String {
    let Some(call) = &class.external_call else {
        return format!(
            "`{}` has a body written outside Modelica, which this compiler cannot run",
            class.name
        );
    };
    // An argument is nearly always a name, and a name reads better as
    // itself than as the shape it is held in.
    let handed: Vec<String> = call
        .arguments
        .iter()
        .map(|argument| match argument {
            Expr::Ref(name) => name.clone(),
            other => names::sketch(other),
        })
        .collect();
    format!(
        "`{}` is `{}({})`{}, written outside Modelica; this compiler has none of its own for \
         that name, and no outside library was given",
        class.name,
        call.called,
        handed.join(", "),
        match &call.language {
            Some(language) => format!(" in {language}"),
            None => String::new(),
        }
    )
}

/// How many numbers a value written out holds. A matrix arrives as an
/// array of its rows, and what a body written here is handed is the
/// numbers themselves.
fn numbers_in(expr: &Expr) -> usize {
    match expr {
        Expr::Array(items) => items.iter().map(numbers_in).sum(),
        _ => 1,
    }
}

/// The outputs of a body written here, each taking its own place of
/// what the call answers with.
///
/// The answer is one flat list of numbers, in the order the outputs
/// were declared and each written out: an output of two numbers takes
/// two places. A scalar output is one place, and a call asked for one
/// place is written `f(...)[k]` - the same shape a walked body's
/// answer takes, so nothing downstream needs a second rule.
#[allow(clippy::too_many_arguments)]
fn numbered_outputs(
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
    consts: &HashMap<String, f64>,
    given_shapes: &HashMap<String, Vec<i64>>,
    made: &Expr,
    answers: usize,
) -> Result<Vec<(String, Expr)>, String> {
    let place = |which: usize| {
        Expr::Index(
            Box::new(made.clone()),
            vec![Expr::Number(which as f64 + 1.0)],
        )
    };
    let mut outputs = Vec::new();
    let mut taken = 0;
    for output in function_components(registry, class, 0)
        .iter()
        .filter(|c| c.causality == Causality::Output)
    {
        // A length is nearly always a constant of the package the
        // function is written in - `stateOut[nState]` of a generator -
        // or a length the call handed over - `x[size(A, 1)]` of a
        // solver - and neither is a name an environment holds.
        let length = match output.dimensions.as_slice() {
            [] => None,
            [dimension] => {
                let named = substitute_class_constants(
                    dimension,
                    registry,
                    &class.name,
                    &class.imports,
                    &[],
                );
                Some(
                    const_eval(&named, consts)
                        .map(|length| length as i64)
                        .or_else(|| dimension_value(&named, consts, given_shapes))
                        .ok_or_else(|| {
                            format!(
                                "`{}` answers with `{}`, whose length this compiler cannot see",
                                class.name, output.name
                            )
                        })? as usize,
                )
            }
            _ => {
                return Err(format!(
                    "`{}` answers with `{}`, whose shape this compiler cannot see",
                    class.name, output.name
                ))
            }
        };
        let said = match length {
            None => place(taken),
            Some(length) => Expr::Array((0..length).map(|step| place(taken + step)).collect()),
        };
        taken += length.unwrap_or(1);
        outputs.push((output.name.clone(), said));
    }
    if taken != answers {
        return Err(format!(
            "`{}` answers with {taken} number(s), and the body written here answers with \
             {answers}",
            class.name
        ));
    }
    Ok(outputs)
}

/// Whether a declaration is of `Real`, following the aliases a library
/// wraps it in: `SI.Voltage` is a `Real` and so is `Modelica.Units.SI
/// .Time`, while an `Integer`, a `Boolean` or an `ExternalObject` is
/// not. What this decides is which inputs of a function have a rate of
/// change to be handed alongside them.
fn is_real(
    registry: &HashMap<&str, &ClassDef>,
    component: &Component,
    scope: &str,
    imports: &[(String, String)],
) -> bool {
    let mut named = component.type_name.clone();
    // Each step of the chain is a name written inside the class that
    // came before it: `SI.Temperature` is `ThermodynamicTemperature`
    // as `Modelica.Units.SI` spells it, and looking that up from where
    // the declaration stands finds nothing. So the place to look moves
    // along with the name.
    let mut scope = scope.to_string();
    let mut imports = imports.to_vec();
    for _ in 0..MAX_DEPTH {
        if named == "Real" {
            return true;
        }
        let Some(of) = lookup(registry, &named, &scope, &imports) else {
            return false;
        };
        let Some(alias) = &of.alias_of else {
            return false;
        };
        named = alias.0.clone();
        scope = of.name.clone();
        imports = of.imports.clone();
    }
    false
}

/// Every declaration of a function, its bases' first.
///
/// A function may say only what it does - `redeclare function extends
/// bubbleEnthalpy` - and leave what it takes and answers with to the
/// one it extends. The base's declarations come first, since that is
/// the order the arguments are given in.
pub(super) fn function_components(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    depth: usize,
) -> Vec<Component> {
    let mut out = Vec::new();
    if depth > MAX_DEPTH {
        return out;
    }
    for extend in &class.extends {
        let base = match extend.from_base {
            true => inherited_class(registry, class, &extend.base, 0),
            false => lookup(registry, &extend.base, &class.name, &class.imports),
        };
        if let Some(base) = base {
            out.extend(function_components(registry, base, depth + 1));
        }
    }
    for component in &class.components {
        // What the class writes for itself replaces what it inherited
        // of that name rather than joining it.
        out.retain(|kept: &Component| kept.name != component.name);
        out.push(component.clone());
    }
    out
}

/// What a body came to last time it was handed exactly this, for as
/// long as one class is being instantiated.
///
/// A model asks the same question of the same body over and over: the
/// transistor bodies of `Spice3` are written out four million times
/// between them, and a hundred thousand of those askings are
/// different. What a body answers depends on what it was handed, on
/// the shapes it was handed, and on the parameter values in view -
/// and the last of those is what stands still while one class is
/// instantiated, which is why the remembering lives exactly that long.
type Remembered = Result<(Vec<(String, Expr)>, Vec<(Expr, String)>), String>;

thread_local! {
    static INLINED: std::cell::RefCell<HashMap<String, Remembered>> =
        std::cell::RefCell::new(HashMap::new());
    static REMEMBERING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Remember what bodies come to while this stands, and forget it when
/// it falls: one class is being instantiated, and its parameter values
/// do not move while it is.
pub(super) struct Inlined(HashMap<String, Remembered>, bool);

impl Inlined {
    pub(super) fn open() -> Self {
        let held = INLINED.with(|held| std::mem::take(&mut *held.borrow_mut()));
        let before = REMEMBERING.with(|on| on.replace(true));
        Inlined(held, before)
    }
}

impl Inlined {
    /// Forget what was remembered, because what a body would fold with
    /// has moved: a parameter this class holds has just been settled,
    /// and a body asked before it was settled may answer differently
    /// now.
    pub(super) fn forget() {
        INLINED.with(|held| held.borrow_mut().clear());
    }
}

impl Drop for Inlined {
    fn drop(&mut self) {
        INLINED.with(|held| *held.borrow_mut() = std::mem::take(&mut self.0));
        REMEMBERING.with(|on| on.set(self.1));
    }
}

/// Run a function body and give back what each output came to, with
/// the checks the body made collected into `checks`.
///
/// Asked the same thing twice, it answers the second from the first.
fn inline_body(
    class: &ClassDef,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
    checks: &mut Vec<(Expr, String)>,
) -> Result<Vec<(String, Expr)>, String> {
    if !REMEMBERING.with(|on| on.get()) {
        return worked_body(class, args, shapes, consts, registry, depth, checks);
    }
    // How deep the asking is belongs to the question: a body that
    // will not come to an end is refused at a depth rather than by
    // what it was handed, and the same asking higher up may be
    // answered. So does how many values are in view, since a body
    // folds with them and a caller further along has more.
    let asked = format!(
        "{}|{depth}|{}|{args:?}|{shapes:?}",
        class.name,
        consts.len()
    );
    if let Some(told) = INLINED.with(|held| held.borrow().get(&asked).cloned()) {
        let (outputs, said) = told?;
        checks.extend(said);
        return Ok(outputs);
    }
    let mut said = Vec::new();
    let answer = worked_body(class, args, shapes, consts, registry, depth, &mut said);
    let told: Remembered = match &answer {
        Ok(outputs) => Ok((outputs.clone(), said.clone())),
        Err(why) => Err(why.clone()),
    };
    INLINED.with(|held| held.borrow_mut().insert(asked, told));
    checks.extend(said);
    answer
}

/// The same, worked out rather than remembered.
fn worked_body(
    class: &ClassDef,
    args: &[Expr],
    shapes: &[Vec<i64>],
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    depth: usize,
    checks: &mut Vec<(Expr, String)>,
) -> Result<Vec<(String, Expr)>, String> {
    if depth > MAX_DEPTH {
        return Err(format!("`{}` {NO_BOTTOM}", class.name));
    }
    // `external "builtin" y = asin(u)` says the function is the
    // operator the language already has, given a place in a library's
    // tree. The call becomes a call to the operator, arguments in the
    // order they were written.
    if let Some(builtin) = &class.builtin {
        let output = class
            .components
            .iter()
            .find(|c| c.causality == Causality::Output)
            .ok_or_else(|| format!("function `{}` declares no output", class.name))?;
        return Ok(vec![(
            output.name.clone(),
            Expr::Call(builtin.clone(), args.to_vec()),
        )]);
    }
    // A function whose body is written outside Modelica is read as far
    // as its declaration and no further. Where this compiler answers
    // for the name itself, the call is written as that name and left
    // standing for whoever can work it out; where nobody answers, the
    // refusal says which name was wanted.
    if class.external {
        let Some(call) = class.external_call.as_ref().filter(|call| {
            external::answered_here(&call.called) || crate::outside::written_here(&call.called)
        }) else {
            return Err(outside_this_language(class));
        };
        let made = Expr::Call(call.called.clone(), args.to_vec());
        // A body written here in Rust answers with numbers rather than
        // with a string, and may answer with several: the generators
        // give a value and the state they moved to. Each output takes
        // its own place of that answer, the way a walked body's does.
        let handed: Vec<usize> = args.iter().map(numbers_in).collect();
        if let Some((_, answers)) = crate::outside::shape(&call.called, &handed) {
            // The shapes the call handed over, under the names the
            // declaration knows them by: an answer as long as `size(A,
            // 1)` reads its length back out of here.
            let mut given_shapes: HashMap<String, Vec<i64>> = HashMap::new();
            for (input, shape) in function_components(registry, class, 0)
                .iter()
                .filter(|c| c.causality == Causality::Input)
                .zip(shapes)
            {
                given_shapes.insert(input.name.clone(), shape.clone());
            }
            return numbered_outputs(class, registry, consts, &given_shapes, &made, answers);
        }
        if crate::outside::written_here(&call.called) {
            return Err(format!(
                "`{}` is written here, and not for what it was handed: {} argument(s) of {} \
                 number(s) in all",
                call.called,
                handed.len(),
                handed.iter().sum::<usize>()
            ));
        }
        let output = class
            .components
            .iter()
            .find(|c| c.causality == Causality::Output)
            .ok_or_else(|| format!("function `{}` declares no output", class.name))?;
        return Ok(vec![(output.name.clone(), made)]);
    }
    // What a function takes and answers with may be written in a base
    // of it: `redeclare function extends bubbleEnthalpy` says only what
    // the body is, and the base says what goes in and comes out.
    let declared = function_components(registry, class, 0);
    let inputs: Vec<&Component> = declared
        .iter()
        .filter(|c| c.causality == Causality::Input)
        .collect();
    let outputs: Vec<&Component> = declared
        .iter()
        .filter(|c| c.causality == Causality::Output)
        .collect();
    let mut bindings: HashMap<String, Expr> = HashMap::new();
    let mut given_shapes: HashMap<String, Vec<i64>> = HashMap::new();
    let mut named_seen = false;
    let mut position = 0;
    for (index, arg) in args.iter().enumerate() {
        if let Expr::NamedArg(name, value) = arg {
            if !inputs.iter().any(|input| &input.name == name) {
                return Err(format!(
                    "function `{}` has no input named `{name}`",
                    class.name
                ));
            }
            if bindings.insert(name.clone(), (**value).clone()).is_some() {
                return Err(format!(
                    "argument `{name}` of function `{}` is given twice",
                    class.name
                ));
            }
            named_seen = true;
        } else {
            if named_seen {
                return Err(format!(
                    "function `{}`: positional arguments must come before named ones",
                    class.name
                ));
            }
            let Some(input) = inputs.get(position) else {
                return Err(format!(
                    "function `{}` expects {} argument(s), got more",
                    class.name,
                    inputs.len()
                ));
            };
            // A `[:]` input is as long as whatever was handed to it.
            if !input.dimensions.is_empty() {
                if let Some(shape) = shapes.get(index) {
                    if !shape.is_empty() {
                        given_shapes.insert(input.name.clone(), shape.clone());
                        // The body reads the argument by the caller's
                        // name once the binding is substituted in, so
                        // `size(x, 1)` becomes `size(s.i, 1)` and has
                        // to find the length under that name too.
                        if let Expr::Ref(given) = arg {
                            given_shapes.insert(given.clone(), shape.clone());
                        }
                    }
                }
            }
            // A record input arrives as its fields, and the body reads
            // them by name: `c1.re` has to be bound, not `c1`.
            if let Some(fields) = record_input_fields(registry, class, input) {
                if let Expr::Array(items) = arg {
                    if items.len() != fields.len() {
                        return Err(format!(
                            "function `{}` wants {} field(s) for `{}`, got {}",
                            class.name,
                            fields.len(),
                            input.name,
                            items.len()
                        ));
                    }
                    for (field, value) in fields.iter().zip(items) {
                        let here = format!("{}.{field}", input.name);
                        // A field that is itself an array is bound
                        // element by element as well: the body of an
                        // orientation function reads `R.T[1, 1]`, and
                        // the list bound to `R.T` whole is not
                        // something a name with a subscript can be read
                        // off.
                        by_element(&here, value, &mut Vec::new(), &mut bindings);
                        bindings.insert(here, value.clone());
                    }
                    position += 1;
                    continue;
                }
                // A record handed over by name rather than written out
                // is the commoner way of it: the caller has the record
                // as a variable and passes it whole. Flattening has
                // already taken that variable apart, so its fields are
                // there to be named one by one - and binding the name
                // alone would leave the body reading `p.V` with
                // nothing bound to it, which is a value gone missing
                // rather than a refusal.
                //
                // The name itself is bound too, below: a body may hand
                // the record on to another function whole, and that
                // call wants the record and not its fields.
                //
                // Only the fields that are single numbers. A field
                // with dimensions of its own - an orientation carries
                // a three by three - has a shape the caller knows and
                // this does not, and binding a bare name to it loses
                // the shape and refuses the model further along.
                if let Expr::Ref(given) = arg {
                    for field in scalar_record_fields(registry, class, input) {
                        bindings.insert(
                            format!("{}.{field}", input.name),
                            Expr::Ref(format!("{given}.{field}")),
                        );
                    }
                    // A field with dimensions of its own - an
                    // orientation carries a three by three - is bound
                    // element by element as well as whole: a body
                    // reading `R.T[1, 1]` has to find the caller's own
                    // `R1.T[1, 1]` under it, and a name alone is not
                    // something a subscript can be read off here.
                    for (field, shape) in shaped_record_fields(registry, class, input) {
                        let here = format!("{}.{field}", input.name);
                        let there = format!("{given}.{field}");
                        for indices in index_tuples(&shape) {
                            let source = Expr::Ref(element_name(&there, &indices));
                            bindings.insert(element_name(&here, &indices), source);
                        }
                        bindings.insert(here, spread_out(&there, &shape, &mut Vec::new()));
                    }
                }
            }
            bindings.insert(input.name.clone(), arg.clone());
            position += 1;
        }
    }
    // Whatever the call left unsaid falls back to the input's own
    // default. Defaults may name earlier inputs, so they are resolved
    // against what is already bound.
    for input in &inputs {
        // A record was bound field by field, so its own name is not
        // among the bindings and it is not missing either.
        let field_prefix = format!("{}.", input.name);
        let bound = bindings.contains_key(&input.name)
            || bindings.keys().any(|name| name.starts_with(&field_prefix));
        if !bound {
            let Some(default) = &input.binding else {
                return Err(format!(
                    "function `{}` is missing its argument `{}`",
                    class.name, input.name
                ));
            };
            // A default may name a constant of a class - `input Real
            // eps = 100*Modelica.Constants.eps` is how the standard
            // library keeps a normalization off zero - and the name
            // means what it meant where it was written, not where the
            // call is.
            let default =
                substitute_class_constants(default, registry, &class.name, &class.imports, &[]);
            let default = substitute_refs(&default, &bindings);
            bindings.insert(input.name.clone(), default);
        }
    }
    // What the call handed in, which a local's value may name: the
    // multibody world builds an orientation from an axis vector that
    // way - `Real e_x[3] = if length(n_x) < 1e-10 then {1, 0, 0} else
    // normalize(n_x)` - and left unbound the body carried `n_x` out
    // with it, a name meaning nothing outside the function. A local
    // naming another local is left alone: the array layer reads an
    // element off a name and cannot read one off the list written in
    // its place.
    // The lengths the call decided go in first: a declared dimension
    // that is a colon measures nothing on its own, and a result sized
    // `size(v, 1)` reads its length back out of here.
    let mut sizes: HashMap<String, Vec<i64>> = given_shapes;
    // What the call handed over as numbers: a result declared
    // `Integer[nState]` takes its length from the `nState` it was
    // given, and nowhere else says what that is.
    let given: HashMap<String, f64> = bindings
        .iter()
        .filter_map(|(name, value)| Some((name.clone(), const_eval(value, consts)?)))
        .collect();
    collect_shapes(registry, class, consts, &given, &mut sizes, 0);
    let no_loop_vars = HashMap::new();
    let local_shapes = Shapes {
        sizes: &sizes,
        loop_vars: &no_loop_vars,
        consts,
        records: no_records(),
    };
    let mut handed: HashMap<String, Expr> = bindings.clone();
    for component in &class.components {
        if component.causality == Causality::None {
            if let Some(binding) = &component.binding {
                // A local that is one number is read here rather than
                // where it is used: `m = size(x, 1)` of a space-phasor
                // transform is written in terms of an input, and the
                // inputs are bound by now. Left to be read later it
                // would carry the input's name out of the body, and
                // out there the name means nothing.
                //
                // A local that is an array keeps its name, because the
                // array layer reads an element off a name and cannot
                // read one off a list written in its place.
                // A declaration value may name a constant of a class
                // the way a statement may - `Real Tlim = min(T,
                // data.TCRIT)` reads a constant of a record beside the
                // function - and it is read here, where the names mean
                // what they meant to whoever wrote them.
                let binding = &substitute_class_constants(
                    binding,
                    registry,
                    &class.name,
                    &class.imports,
                    &[],
                );
                let bound = match component.dimensions.is_empty() {
                    true => substitute_refs(binding, &bindings),
                    false => substitute_refs(binding, &handed),
                };
                // Where it comes to a number, it is stored as one: the
                // number of base systems of an m-phase winding is a
                // call, and a call is not something arithmetic alone
                // can decide an `if` by. Worked out once here, it is a
                // digit everywhere it is used.
                let bound = match settled_in_body(
                    &bound,
                    &HashMap::new(),
                    consts,
                    &sizes,
                    registry,
                    &class.name,
                    &class.imports,
                    depth,
                ) {
                    Some(number) if component.dimensions.is_empty() => Expr::Number(number),
                    _ => bound,
                };
                // A local array may be read by a later local, so it
                // joins what the next value is written against - but
                // only where its value is a name rather than a list
                // written out, since the array layer reads an element
                // off a name and cannot read one off a list.
                // A local array written out as a list is also bound
                // element by element - `Real e[3] = n; Real z[3] = e`
                // of a body that then reads `z[2]` comes through the
                // array layer as the name `e[2]`, and only an element
                // name answers that.
                // The value may be a call the array layer works out -
                // `Real e_z_aux[3] = cross(e_x, n_y_aux)` of the
                // multibody frames - and until it is worked out there
                // is no element to read.
                let worked = expand(
                    &bound,
                    &local_shapes,
                    registry,
                    &class.name,
                    &class.imports,
                    depth + 1,
                );
                let bound = match (component.dimensions.is_empty(), worked) {
                    (false, Ok(value)) => substitute_refs(&value.into_expr(), &bindings),
                    _ => bound,
                };
                if let Expr::Array(_) = &bound {
                    let mut elements = HashMap::new();
                    by_element(&component.name, &bound, &mut Vec::new(), &mut elements);
                    handed.extend(elements.clone());
                    bindings.extend(elements);
                }
                bindings.insert(component.name.clone(), bound);
            }
        }
    }
    let mut assigned = Vec::new();
    // `Return` is simply an early landing here; the outputs are read
    // out the same way. A `break` with no loop has nowhere to go.
    if execute(
        &class.algorithm,
        &mut bindings,
        &mut assigned,
        checks,
        consts,
        &sizes,
        registry,
        &class.name,
        &class.imports,
        depth + 1,
        false,
    )? == Flow::Break
    {
        return Err(format!(
            "`break` outside of a loop in function `{}`",
            class.name
        ));
    }
    outputs
        .iter()
        .map(|output| {
            let name = &output.name;
            // A whole-array assignment bound the name itself;
            // per-element assignments bound `c[1]`, `c[2]`, ... -
            // gather them in order.
            if let Some(expr) = bindings.get(name) {
                return Ok((name.clone(), expr.clone()));
            }
            if let Some(dimensions) = sizes.get(name) {
                // An answer of more than one dimension is an array of
                // arrays, not a flat list: `den2[:, 2]` is read
                // `den2[i, 2]` by whoever was handed it, and a second
                // subscript has nowhere to go in a flat list.
                fn gathered(
                    name: &str,
                    dimensions: &[i64],
                    so_far: &mut Vec<i64>,
                    bindings: &HashMap<String, Expr>,
                    class: &str,
                ) -> Result<Expr, String> {
                    let Some((&length, rest)) = dimensions.split_first() else {
                        let element = element_name(name, so_far);
                        return bindings.get(&element).cloned().ok_or_else(|| {
                            format!("function `{class}` never assigns `{element}` of its output")
                        });
                    };
                    let mut items = Vec::new();
                    for index in 1..=length {
                        so_far.push(index);
                        items.push(gathered(name, rest, so_far, bindings, class)?);
                        so_far.pop();
                    }
                    Ok(Expr::Array(items))
                }
                let mut so_far = Vec::new();
                let items = gathered(name, dimensions, &mut so_far, &bindings, &class.name)?;
                return Ok((name.clone(), items));
            }
            // A record-typed output built up field by field - `v.x :=`,
            // `v.y :=`, as an operator record's constructor does - is
            // gathered into the record value its fields make.
            if let Some(record) = lookup(registry, &output.type_name, &class.name, &class.imports)
                .filter(|c| c.kind == ClassKind::Record)
            {
                // A field the body never assigned may have been given
                // on the declaration instead: `output Complex result(re
                // = re, im = im)` is how a record says what it is made
                // of without an algorithm at all.
                let declared = |field: &str| -> Option<Expr> {
                    let (_, value) = output.modifiers.iter().find(|(given, _)| given == field)?;
                    let value = substitute_class_constants(
                        value,
                        registry,
                        &class.name,
                        &class.imports,
                        &[],
                    );
                    Some(substitute_refs(&value, &bindings))
                };
                let fields = record_fields(record)
                    .into_iter()
                    .map(|field| {
                        let member = format!("{name}.{field}");
                        bindings
                            .get(&member)
                            .cloned()
                            .or_else(|| declared(&field))
                            .ok_or_else(|| {
                                format!(
                                    "function `{}` never assigns `{member}` of its output, and \
                                     its declaration says nothing about it either",
                                    class.name
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                return Ok((name.clone(), Expr::Array(fields)));
            }
            // The one value a declaration may give outright.
            if let Some(value) = &output.binding {
                let value =
                    substitute_class_constants(value, registry, &class.name, &class.imports, &[]);
                return Ok((name.clone(), substitute_refs(&value, &bindings)));
            }
            Err(format!(
                "function `{}` never assigns its output `{name}`",
                class.name
            ))
        })
        .collect()
}

/// The fields of a record-typed argument that are single numbers.
///
/// A field with dimensions is left out: what a caller handed over
/// knows its shape and a bare name written in its place does not, so
/// binding one would turn a matrix into something of no shape at all.
/// Those are still reached through the record's own name.
/// Every element of a value written out as a list, under the name it
/// is bound to: `e` bound to `{a, b}` also binds `e[1]` and `e[2]`,
/// which is how a name with a subscript is read.
fn by_element(name: &str, value: &Expr, so_far: &mut Vec<i64>, out: &mut HashMap<String, Expr>) {
    match value {
        Expr::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                so_far.push(index as i64 + 1);
                by_element(name, item, so_far, out);
                so_far.pop();
            }
        }
        one => {
            out.insert(element_name(name, so_far), one.clone());
        }
    }
}

/// A name of a given shape written out as the list of its elements:
/// `T` of three by three is three rows of three names. Written whole a
/// matrix is a list of lists, the way `T[1, :]` reads a row.
fn spread_out(name: &str, shape: &[i64], so_far: &mut Vec<i64>) -> Expr {
    let Some((&length, rest)) = shape.split_first() else {
        return Expr::Ref(element_name(name, so_far));
    };
    let mut items = Vec::new();
    for index in 1..=length {
        so_far.push(index);
        items.push(spread_out(name, rest, so_far));
        so_far.pop();
    }
    Expr::Array(items)
}

/// The fields of a record-typed argument that have dimensions, with
/// the shape each one turned out to have.
fn shaped_record_fields(
    registry: &HashMap<&str, &ClassDef>,
    function: &ClassDef,
    input: &Component,
) -> Vec<(String, Vec<i64>)> {
    let Some(of) = lookup(
        registry,
        &input.type_name,
        &function.name,
        &function.imports,
    ) else {
        return Vec::new();
    };
    if of.kind != ClassKind::Record {
        return Vec::new();
    }
    of.components
        .iter()
        .filter(|field| !field.dimensions.is_empty())
        .filter_map(|field| {
            let shape: Option<Vec<i64>> = field
                .dimensions
                .iter()
                .map(|d| const_eval(d, &HashMap::new()).map(|n| n as i64))
                .collect();
            Some((field.name.clone(), shape?))
        })
        .collect()
}

fn scalar_record_fields(
    registry: &HashMap<&str, &ClassDef>,
    function: &ClassDef,
    input: &Component,
) -> Vec<String> {
    let Some(of) = lookup(
        registry,
        &input.type_name,
        &function.name,
        &function.imports,
    ) else {
        return Vec::new();
    };
    if of.kind != ClassKind::Record {
        return Vec::new();
    }
    of.components
        .iter()
        .filter(|field| field.dimensions.is_empty())
        .map(|field| field.name.clone())
        .collect()
}

/// The fields of a record-typed argument of a function, when it is one.
pub(super) fn record_input_fields(
    registry: &HashMap<&str, &ClassDef>,
    function: &ClassDef,
    input: &Component,
) -> Option<Vec<String>> {
    let of = lookup(
        registry,
        &input.type_name,
        &function.name,
        &function.imports,
    )?;
    (of.kind == ClassKind::Record).then(|| record_fields(of))
}

/// What a name of a function body holds before anything assigns it.
///
/// Inside a function this is not a missing value but a stated one: an
/// unassigned local or output starts at its type's own start, which
/// for a number is zero and for a Boolean is false. Outside a function
/// there is no such rule, so nothing comes back and the branch that
/// left the variable unset is refused as before.
///
/// A field of a record - `bpro.cp` - starts where the field's own type
/// starts, which is the same answer arrived at by the name of the
/// field rather than of the record holding it.
fn starts_at(name: &str, registry: &HashMap<&str, &ClassDef>, scope: &str) -> Option<Expr> {
    let class = registry.get(scope)?;
    if class.kind != ClassKind::Function {
        return None;
    }
    // Only a name the body declares, reached through the record it
    // may be a field of. A name from anywhere else is not this rule's
    // business and keeps the refusal it had.
    let root = name.split('.').next()?;
    let declared = class.components.iter().find(|c| c.name == root)?;
    if declared.causality == Causality::Input || !declared.dimensions.is_empty() {
        return None;
    }
    // Only a record, whole or by one of its fields. A plain local
    // that one branch sets and another does not is the shape a
    // quaternion conversion is written in - four branches, each
    // assigning the same handful of names - and merging those builds
    // a pile of nested conditions that is expanded again at every
    // use. One multi-body model went from a second and a half to half
    // a minute that way and the whole library from seventeen seconds
    // to a quarter of an hour, for models it did not rescue. What the
    // library does rely on is records: the steam tables leave `cp`
    // unset on one side of a boundary and `cv` on the other, and each
    // field is a name of its own that nothing multiplies.
    let held = lookup(registry, &declared.type_name, &class.name, &class.imports);
    let record = held.filter(|held| held.kind == ClassKind::Record);
    // A plain local - not a record at all - has a start too, and the
    // guard above is what keeps that affordable: where the branches
    // write arrays, nothing here answers.
    if record.is_none() && name == root {
        return match started_by(&declared.type_name, registry, class, 0) {
            Some(Started::Boolean) => Some(Expr::Bool(false)),
            Some(Started::Number) => Some(Expr::Number(0.0)),
            Some(Started::Text) => Some(Expr::Str(String::new())),
            None => None,
        };
    }
    // A record named whole starts as its fields do, gathered in the
    // order the record declares them. A body may assign the whole
    // record in one branch - the steam tables say `f := Basic.f3(d,
    // T)` inside a region test and read `f` after it - and then it is
    // the record's own name the merge is asked about rather than any
    // field of it.
    if name == root {
        let mut fields = Vec::new();
        for field in &record?.components {
            fields.push(starts_at(
                &format!("{name}.{}", field.name),
                registry,
                scope,
            )?);
        }
        return Some(Expr::Array(fields));
    }
    Some(match starting_type(name, declared, registry, class) {
        Some(Started::Boolean) => Expr::Bool(false),
        Some(Started::Number) => Expr::Number(0.0),
        Some(Started::Text) => Expr::Str(String::new()),
        // A start this cannot name - a record field whose type is not
        // in view - is left to the refusal, which says something true
        // about a value that is really missing.
        None => return None,
    })
}

/// What kind of start a declaration has.
enum Started {
    /// A `Real` or `Integer`, which starts at zero.
    Number,
    /// A `Boolean`, which starts at false.
    Boolean,
    /// A `String`, which starts empty.
    Text,
}

/// The start of `name`, following it into the record it is a field of.
fn starting_type(
    name: &str,
    declared: &Component,
    registry: &HashMap<&str, &ClassDef>,
    within: &ClassDef,
) -> Option<Started> {
    // A field's type is written where the record is written, not
    // where the function using it is: a steam property record says
    // `DerPressureByTemperature`, a name that means something in the
    // media package and nothing in the function reading it. So the
    // record that holds a field becomes the place the next name is
    // looked up from.
    let mut current = declared.type_name.clone();
    let mut within = within;
    for field in name.split('.').skip(1) {
        let holding = lookup(registry, &current, &within.name, &within.imports)?;
        current = holding
            .components
            .iter()
            .find(|c| c.name == field)?
            .type_name
            .clone();
        within = holding;
    }
    started_by(&current, registry, within, 0)
}

/// A type name followed through its own bases until it is one of the
/// language's own: `SI.SpecificHeatCapacity` is a `Real`.
fn started_by(
    type_name: &str,
    registry: &HashMap<&str, &ClassDef>,
    within: &ClassDef,
    depth: usize,
) -> Option<Started> {
    if depth > 32 {
        return None;
    }
    match type_name {
        "Real" | "Integer" => return Some(Started::Number),
        "Boolean" => return Some(Started::Boolean),
        "String" => return Some(Started::Text),
        _ => {}
    }
    let class = lookup(registry, type_name, &within.name, &within.imports)?;
    // An enumeration counts from one and is held as a number.
    if !class.enumeration.is_empty() {
        return Some(Started::Number);
    }
    // A type reaches what it is by either road: the short form -
    // `type Current = SI.Current` - keeps its base as an alias, and
    // the long one as an `extends`.
    let base = match &class.alias_of {
        Some((base, _)) => base.clone(),
        None => class.extends.first()?.base.clone(),
    };
    // The next name along is written where this type is written, so
    // that is where it is looked up from: `SpecificEnthalpy` is
    // `SpecificEnergy` in the units package, and the function that
    // started the asking has never heard of either.
    started_by(&base, registry, class, depth + 1)
}
