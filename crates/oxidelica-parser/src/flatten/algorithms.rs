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
    pub(super) static SET_ASIDE: RefCell<Vec<(Expr, String)>> = const { RefCell::new(Vec::new()) };
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
pub(super) const MAX_NESTED_CALLS: usize = 64;

thread_local! {
    /// How many calls are being inlined inside one another.
    pub(super) static INLINING: Cell<usize> = const { Cell::new(0) };
}

/// One step further into the nesting, undone when it goes out of view.
pub(super) struct Nested;

impl Nested {
    pub(super) fn deeper() -> Self {
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
pub(super) fn holds_unbounded_call(expr: &Expr, registry: &HashMap<&str, &ClassDef>) -> bool {
    // The names an unrolling leaves behind are already qualified, so
    // the registry answers to them directly.
    let mut called = Vec::new();
    inlining::gather_calls(expr, registry, "", &[], &mut called);
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
    inlining::gather_calls_in_statements(
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
        inlining::gather_calls_in_statements(
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
pub(super) fn settled_in_body(
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
    // A local of a body may be worked out of a string - `Integer len =
    // Strings.length(s)` is how every search the standard library
    // writes begins - and the arithmetic layer has no strings to work
    // it out with. The strings the class settled are in view, so what
    // measures one comes to a number here.
    statements::settled_truth(&worked.into_expr(), consts, &statements::texts_in_view())
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
pub(super) fn walk_expr(expr: &Expr, eye: &mut impl FnMut(&Expr)) {
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

/// The fields of a record-typed argument that have dimensions, with
/// the shape each one turned out to have.
/// Every field a record holds, its bases' first, as the declarations
/// they are.
///
/// `redeclare record extends ThermodynamicState` writes no fields and
/// means the ones it extends: a medium's state is what its base
/// declared. A class's own declaration replaces an inherited one of
/// the same name rather than joining it.
pub(super) fn record_components(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    depth: usize,
) -> Vec<Component> {
    let mut out: Vec<Component> = Vec::new();
    if depth > MAX_DEPTH {
        return out;
    }
    for extend in &class.extends {
        let base = match extend.from_base {
            true => inherited_class(registry, class, &extend.base, 0),
            false => lookup(registry, &extend.base, &class.name, &class.imports),
        };
        if let Some(base) = base {
            out.extend(record_components(registry, base, depth + 1));
        }
    }
    for component in &class.components {
        out.retain(|kept: &Component| kept.name != component.name);
        out.push(component.clone());
    }
    out
}

pub(super) fn shaped_record_fields(
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
    // A record may take its fields from a base rather than write them:
    // `redeclare record extends ThermodynamicState` is how a medium
    // says its state is the one it inherits.
    record_components(registry, of, 0)
        .into_iter()
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

pub(super) fn scalar_record_fields(
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
    record_components(registry, of, 0)
        .into_iter()
        .filter(|field| field.dimensions.is_empty())
        .map(|field| field.name)
        .collect()
}

/// The fields of a record-typed argument of a function, when it is one.
pub(super) fn record_input_fields(
    registry: &HashMap<&str, &ClassDef>,
    function: &ClassDef,
    input: &Component,
) -> Option<Vec<String>> {
    // Asked under the name the body was called by, where that is
    // known: a media function takes `ThermodynamicState`, and which
    // record that is depends on the medium it was called through
    // rather than on the base that wrote the function. Where nothing
    // said, the class that wrote it answers as before.
    // The name a call wrote may be a short one - `Medium.density`,
    // where `Medium` is a package the model named - so it is put
    // through the same lookup as any other class name first. What
    // comes back is where the medium really lives, and that is the
    // scope the body's own names are asked from.
    // Asked under the name the body was called by, where that says
    // something the class does not: a media function takes
    // `ThermodynamicState`, and which record that is depends on the
    // medium it was called through. Where the two are the same, one
    // lookup is all that happens - which is what keeps the models
    // that have nothing to do with media asking exactly as before.
    let under = inlining::asked_under(function);
    let of = match under == function.name {
        true => lookup(
            registry,
            &input.type_name,
            &function.name,
            &function.imports,
        )?,
        false => lookup(registry, &input.type_name, &under, &function.imports).or_else(|| {
            lookup(
                registry,
                &input.type_name,
                &function.name,
                &function.imports,
            )
        })?,
    };
    // A record may take its fields from a base rather than write them:
    // `redeclare record extends ThermodynamicState` is how a medium
    // says its state is the one it inherits, and a function taking
    // that state was told it takes nothing at all.
    (of.kind == ClassKind::Record).then(|| record_fields_of(registry, of, 0))
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
pub(super) fn starts_at(
    name: &str,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
) -> Option<Expr> {
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
