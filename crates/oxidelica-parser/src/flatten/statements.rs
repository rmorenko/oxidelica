//! An algorithm section run symbolically: each statement in turn,
//! with what it assigns kept as an expression rather than a number.
//!
//! What a body of a function comes to is worked out this way, and so
//! is the algorithm section of a model - the difference is only in
//! what stands unsettled at the end of it.
//!
//! Carved out of `algorithms` unchanged.

use super::*;
use std::cell::RefCell;

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
            Statement::Assign(target, subscripts, value) => one_assignment(
                target,
                subscripts,
                value,
                bindings,
                assigned,
                consts,
                sizes,
                registry,
                scope,
                imports,
                depth,
                fold,
            )?,
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
                let outputs = inlining::inline_function_outputs(
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
                if let Some(flow) = one_if_statement(
                    branches,
                    &statements[at + 1..],
                    bindings,
                    assigned,
                    asserts,
                    consts,
                    sizes,
                    registry,
                    scope,
                    imports,
                    depth,
                    fold,
                )? {
                    return Ok(flow);
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
                let checks = inlining::inline_function_checks(
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
                    let truth = settled_truth(&now, consts, &texts_in_view()).ok_or_else(|| {
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

/// One assignment among the statements: what the right-hand side comes
/// to, put where the left-hand side says - a name, an element of an
/// array, or a member of a record.
///
/// Moved out of `execute` unchanged.
#[allow(clippy::too_many_arguments)]
fn one_assignment(
    target: &str,
    subscripts: &[Expr],
    value: &Expr,
    bindings: &mut HashMap<String, Expr>,
    assigned: &mut Vec<String>,
    consts: &HashMap<String, f64>,
    sizes: &HashMap<String, Vec<i64>>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
    fold: bool,
) -> Result<(), String> {
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
    let value = expand(&value, &shapes, registry, scope, imports, depth + 1)?.into_expr();
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
                    bindings.insert(target.to_string(), whole);
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
            algorithms::settled_in_body(e, bindings, consts, sizes, registry, scope, imports, depth)
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
        // A run of elements may be given one call that answers with
        // as many: a random generator's first state is `state :=
        // initialState(seed, global)`, and what stands here is the
        // call rather than the list it comes to. Each element takes
        // its own place of the answer, which is how a body written
        // outside Modelica hands over several numbers at once.
        if items.len() == 1 && named.len() > 1 {
            // The call answers with several numbers and what stands
            // here is one place of it: the array output of a random
            // generator arrives as `call(...)[k]`, where `k` is where
            // its first element sits among the answers. The elements
            // after it are the places after that.
            let spread = match &items[0] {
                Expr::Index(call, subscripts) => match (call.as_ref(), subscripts.as_slice()) {
                    (Expr::Call(called, _), [Expr::Number(first)])
                        if crate::outside::written_here(called) =>
                    {
                        Some((call.as_ref().clone(), *first as usize))
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some((call, first)) = spread {
                items = (0..named.len())
                    .map(|step| {
                        Expr::Index(
                            Box::new(call.clone()),
                            vec![Expr::Number((first + step) as f64)],
                        )
                    })
                    .collect();
            }
        }
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
        return Ok(());
    }
    // `c[i] := ...` lands on the element's own name.
    let target = if subscripts.is_empty() {
        target.to_string()
    } else {
        let indices = subscripts
            .iter()
            .map(|subscript| {
                let subscript = substitute_refs(subscript, bindings);
                // A subscript may be written with a
                // length rather than a digit, and only the
                // array layer knows a length.
                algorithms::settled_in_body(
                    &subscript, bindings, consts, sizes, registry, scope, imports, depth,
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
    // A record assigned whole - `qm := mosCalcDEVqmeyer(...)` - is
    // given the fields the function gathered, one list in the order
    // the record declares them. Kept under its own name it answers
    // nothing when a later statement asks for `qm.qm_capgd`, and the
    // model was refused for an unknown variable. So it is taken apart
    // here, into the field names the rest of the body reads.
    if subscripts.is_empty() {
        if let Some(fields) = record_fields_named(&target, registry, scope, imports) {
            if let Expr::Array(items) = &value {
                if items.len() == fields.len() {
                    bindings.remove(&target);
                    for (field, item) in fields.into_iter().zip(items.iter()) {
                        let member = format!("{target}.{field}");
                        if !assigned.contains(&member) {
                            assigned.push(member.clone());
                        }
                        bindings.insert(member, item.clone());
                    }
                    return Ok(());
                }
            }
        }
    }
    if !assigned.contains(&target) {
        assigned.push(target.to_string());
    }
    // Inside a `while`, a value that folds to a number is stored as
    // one, or the expressions would double in size with every round.
    // A value worked out of a string folds to a number as well, and a
    // loop that counts back from the length of a piece of text cannot
    // start without it: `i := length(s) - length(needle) + 1` is the
    // first line of every search the standard library writes.
    let value = match settled_truth(&value, consts, &texts_in_view()) {
        Some(number) if fold || holds_a_string(&value) => Expr::Number(number),
        _ => value,
    };
    bindings.insert(target, value);
    Ok(())
}

/// Whether an expression asks something of a string.
///
/// A value that only does arithmetic is left as it was written, so
/// that what the model says stays legible; one that measures or
/// compares a piece of text is worth folding wherever it stands,
/// because the layers after this one have no strings to fold it
/// with.
fn holds_a_string(expr: &Expr) -> bool {
    let mut found = false;
    super::clocks::walk_calls(expr, &mut |name| {
        if name.starts_with("ModelicaStrings_") {
            found = true;
        }
    });
    found
}

/// The trail of names asked for while a refusal was being made, as a
/// suffix for it.
///
/// A media function is declared in a base and called with the state
/// of the medium at hand, and which of the two a name landed on is
/// the whole of the question. The trail says it: what was asked, from
/// where, and what it came to. Only the last few, and only the ones
/// that landed somewhere other than where they were asked - the rest
/// is the ordinary business of finding a class and says nothing.
pub(super) fn where_the_names_landed() -> String {
    let trail = super::lookup::Trail::so_far();
    let told: Vec<String> = trail
        .iter()
        .rev()
        .filter(|(asked, from, landed)| match landed {
            None => false,
            Some(landed) => landed != asked && !from.is_empty(),
        })
        .take(4)
        .map(|(asked, from, landed)| {
            let landed = landed.as_deref().unwrap_or("nothing");
            format!("`{asked}` asked from `{from}` landed on `{landed}`")
        })
        .collect();
    match told.is_empty() {
        true => String::new(),
        false => format!("; the names it asked for: {}", told.join(", ")),
    }
}

/// The fields a name's declaration holds, where that declaration is a
/// record of the class being worked out. `None` for anything else: a
/// number, an array, a name from somewhere other than this body.
fn record_fields_named(
    name: &str,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
) -> Option<Vec<String>> {
    let class = registry.get(scope)?;
    let declared = class.components.iter().find(|c| c.name == name)?;
    if !declared.dimensions.is_empty() {
        return None;
    }
    let held = lookup(registry, &declared.type_name, scope, imports)?;
    if held.kind != ClassKind::Record {
        return None;
    }
    let fields = record_fields_of(registry, held, 0);
    match fields.is_empty() {
        true => None,
        false => Some(fields),
    }
}

/// What an expression comes to, where a string may stand in it.
///
/// `const_eval` works out arithmetic and knows nothing of strings, so
/// a condition that compares two of them has no truth to it as far as
/// that layer can see: `substring(s, i, i) == "c"` is a comparison it
/// cannot make. Folding the strings first turns such a comparison
/// into the Boolean it stands for, and then the arithmetic layer can
/// finish.
///
/// It is the same two steps a component's condition is settled by,
/// and `findLast` needs them: its `while` goes round until the piece
/// of text it is looking at is the one it was looking for.
pub(super) fn settled_truth(
    expr: &Expr,
    consts: &HashMap<String, f64>,
    texts: &HashMap<String, String>,
) -> Option<f64> {
    if let Some(number) = const_eval(expr, consts) {
        return Some(number);
    }
    let folded = strings::fold(expr, texts, consts).ok()?;
    const_eval(&folded, consts)
}

/// The strings in view, for a fold that may need them.
///
/// Nothing puts any here yet: what a body works out of a string it
/// works out of one written in the body itself - `substring(s, i, i)`
/// where `s` came in as an argument - and those arrive as literals.
/// A class's own `String` parameters do not reach this far, which is
/// why a file name held in one still stands.
pub(super) fn texts_in_view() -> HashMap<String, String> {
    TEXTS.with(|held| held.borrow().clone())
}

thread_local! {
    /// The `String` parameters the class being instantiated settled.
    ///
    /// A body folds a comparison of two strings only if it knows what
    /// they say, and what they say is worked out by the class the
    /// call was written in - long before this pass runs. The class
    /// leaves them here on its way in, because the road from there to
    /// a loop head runs through a dozen signatures that have no
    /// business carrying a dictionary of text.
    static TEXTS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Let the strings a class settled be seen while its bodies are
/// worked out, and put back whatever was in view before.
pub(super) struct Texts(HashMap<String, String>);

impl Texts {
    pub(super) fn in_view(texts: &HashMap<String, String>) -> Texts {
        Texts(TEXTS.with(|held| held.replace(texts.clone())))
    }
}

impl Drop for Texts {
    fn drop(&mut self) {
        TEXTS.with(|held| *held.borrow_mut() = std::mem::take(&mut self.0));
    }
}

thread_local! {
    /// What runs after the `if` being worked out, for the `if`s
    /// nested inside it.
    ///
    /// A branch is executed on its own, and what its variables are
    /// read by afterwards decides which of them need a merged value.
    /// The statements after an inner `if` are only the rest of the
    /// branch it sits in - `Q := Q*0.5/sqrt(t)` of a quaternion
    /// conversion is after the outer one - so the outer `if` leaves
    /// its own rest here for the inner ones to look at.
    static AFTERWARDS: RefCell<Vec<Vec<Statement>>> = const { RefCell::new(Vec::new()) };
}

/// Let what follows an `if` be seen while its branches are worked
/// out, and take it away again after.
struct Afterwards;

impl Afterwards {
    fn holding(rest: &[Statement]) -> Afterwards {
        AFTERWARDS.with(|held| held.borrow_mut().push(rest.to_vec()));
        Afterwards
    }
}

impl Drop for Afterwards {
    fn drop(&mut self) {
        AFTERWARDS.with(|held| {
            held.borrow_mut().pop();
        });
    }
}

/// Whether a name is read after the statements given, counting what
/// runs after every `if` these sit inside.
fn read_after(rest: &[Statement], name: &str) -> bool {
    if read_later(rest, name, 0) {
        return true;
    }
    AFTERWARDS.with(|held| held.borrow().iter().any(|outer| read_later(outer, name, 0)))
}

/// One `if` among the statements: the branch whose condition holds is
/// executed, and where the condition cannot be settled the branches
/// are executed apart and what they assign is merged into one value
/// per variable.
///
/// `Some(flow)` where the branch left the body early, `None` where
/// execution goes on with the statement after.
///
/// Moved out of `execute` unchanged.
#[allow(clippy::too_many_arguments)]
fn one_if_statement(
    branches: &[StatementBranch],
    rest: &[Statement],
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
) -> Result<Option<Flow>, String> {
    // A condition the compiler can decide picks one branch,
    // and only that one runs. Merging both would be the
    // same answer written at greater length - and where a
    // body calls itself, it would be no answer at all: the
    // branch that ends the recursion cannot end it if the
    // branch that continues it is taken as well.
    let decidable = branches.iter().all(|branch| {
        branch.condition.as_ref().is_none_or(|condition| {
            settled_truth(
                &substitute_refs(condition, bindings),
                consts,
                &texts_in_view(),
            )
            .is_some()
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
                    let value =
                        settled_truth(&condition, consts, &texts_in_view()).ok_or_else(|| {
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
                return Ok(Some(flow));
            }
        }
        return Ok(None);
    }
    let before = bindings.clone();
    // What follows this `if` is what the `if`s inside its branches
    // are read by: their own rest ends where the branch does.
    let _afterwards = Afterwards::holding(rest);
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
        if an_array && !read_after(rest, &name) && !read_after(rest, whole) {
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
        // What a branch wrote, not what the frame holds. A branch is
        // worked out on a copy of everything standing before the
        // `if`, so its bindings name the whole body - arguments and
        // all - and asking that copy whether an array is in it
        // answers about the signature rather than about the branch. A
        // Spice3 transistor takes a vector of four voltages as its
        // eighth argument and writes no array anywhere; read the old
        // way, every `if` in it counted as one that writes arrays.
        let writes_arrays = outcomes.iter().any(|(_, local)| {
            local.iter().any(|(written, value)| {
                sizes.contains_key(written) && before.get(written) != Some(value)
            })
        });
        let fallback = before.get(&name).cloned().or_else(|| {
            let start = record_fields::starts_at(&name, registry, scope)?;
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
        // A merged value that comes to a number is kept as one. Inside
        // a `while` this is what stops the branches piling up: the
        // loop counter of a search comes out of the `if` as a nest of
        // conditions, and the next round's head cannot be decided
        // through it.
        let value = match settled_truth(&value, consts, &texts_in_view()) {
            Some(number) if fold => Expr::Number(number),
            _ => value,
        };
        bindings.insert(name, value);
    }
    Ok(None)
}

thread_local! {
    /// The specialized copies of functions that were handed another
    /// function as an argument.
    ///
    /// A function value has one sink in this language - an argument -
    /// so a receiver handed one can be copied with that input
    /// replaced by ordinary numeric ones. The copies belong to the
    /// model being flattened rather than to any library, and the road
    /// from where they are made to where the model collects its
    /// functions runs through signatures that have no business
    /// carrying a second registry.
    static SPECIALIZED: RefCell<HashMap<String, ClassDef>> = RefCell::new(HashMap::new());
}

/// Keep a specialized copy where the rest of the pass can find it.
pub(super) fn remember_specialization(copy: ClassDef) {
    SPECIALIZED.with(|held| held.borrow_mut().insert(copy.name.clone(), copy));
}

/// A specialized copy by name, if one was made.
pub(super) fn specialization(name: &str) -> Option<ClassDef> {
    SPECIALIZED.with(|held| held.borrow().get(name).cloned())
}
