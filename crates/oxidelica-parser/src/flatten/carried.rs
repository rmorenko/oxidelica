//! The bodies a flat model carries out to the run.
//!
//! Almost every function is written out where it is called. The ones
//! that cannot be - a loop the model decides, a recursion with no
//! bottom - are left standing, and this gathers what the run needs to
//! walk them: the bodies themselves, everything they call in turn, and
//! the refusals for what a walk cannot carry.

use super::inlining::*;
use super::*;

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
    // And the equations of an `if` whose condition the run decides.
    // Those do not sit among the equations above - they are held
    // apart, one list per branch, because which branch holds is not
    // known until the run picks it - and a body called only from
    // inside such a branch travelled with nothing at all. What the
    // model heard was that the compiler does not know a function
    // whose text it is carrying: a vessel writes its port pressure
    // through `Utilities.regSquare2` inside `if regularFlow[i]`, and
    // seven models of the standard library stopped there.
    //
    // Every branch, not the one that will be taken: which that is the
    // run works out, and the walk must have the body of whichever it
    // lands on.
    if std::env::var_os("OXIDELICA_NO_BRANCH_BODIES").is_none() {
        for conditional in &model.conditional {
            for condition in &conditional.conditions {
                look(condition);
            }
            for branch in &conditional.branches {
                for equation in branch {
                    look(&equation.lhs);
                    look(&equation.rhs);
                }
            }
        }
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
    // And what a declaration was written with. A parameter settled
    // before the run may be a call to a body nothing could inline -
    // a medium's `h_default = specificEnthalpy_pTX(p, T, X)` folds
    // to `waterBaseProp_pT(101325, 293.15, 0)[5]` and no further -
    // and the work before the run walks such a call the same way the
    // run does. Gathered from the equations alone that body travels
    // with nothing, and what the model hears is that nothing works
    // out a function the compiler is carrying the text of.
    //
    // Kept apart from what the equations want, because the two are
    // owed different answers. A body the equations call and the walk
    // cannot carry is a model that cannot run, and saying so is the
    // point. A body only a declaration names may never be asked at
    // all - the noise generators are written on a `startTime` nothing
    // reads - and refusing the whole model for one of those cost a
    // model that used to flatten. So one is required and the other is
    // taken if it can be had.
    let mut from_declarations: Vec<String> = Vec::new();
    for component in &model.components {
        for written in [&component.binding, &component.start].into_iter().flatten() {
            gather_calls(written, registry, "", &[], &mut from_declarations);
        }
    }
    // Everything those call, and everything that calls in turn.
    let mut out: Vec<ClassDef> = Vec::new();
    // What no equation asked for, by name, itself included and
    // everything it calls: a body reached only through one of these is
    // wanted only as much as the one that reached it.
    let mut optional: std::collections::HashSet<String> = from_declarations
        .iter()
        .filter(|name| !wanted.contains(name))
        .cloned()
        .collect();

    wanted.extend(from_declarations);
    while let Some(name) = wanted.pop() {
        if out.iter().any(|already| already.name == name) {
            continue;
        }
        // A specialized copy is not in the registry: it was made for
        // this model out of a function that was handed another
        // function, and it lives where such copies are kept.
        let made;
        // A copy carried under a medium is the body that wrote it,
        // prepared with the medium standing on the mark - and in the
        // digits a parameter's road takes, since the walk's frame
        // knows no minted name. Everything below reads the registry
        // under the writer's own name, which is the one it knows; the
        // copy takes the name the flat model calls it by last.
        let pair = carried_pair(&name);
        let marked = pair.as_ref().and_then(|(_, medium)| AskedAs::under(medium));
        let digits = pair.as_ref().map(|_| SettlingParameter::now());
        // And the calls the copy makes are named under its medium, so a
        // callee the medium changes is called as a copy of its own.
        let copy = PreparingCopy::under(pair.as_ref().map(|(_, medium)| medium.as_str()));
        let class = match registry.get(name.as_str()) {
            Some(held) => *held,
            None => match pair
                .as_ref()
                .and_then(|(body, _)| registry.get(body.as_str()))
            {
                Some(held) => *held,
                None => match super::statements::specialization(&name) {
                    Some(copy) => {
                        made = copy;
                        &made
                    }
                    None => continue,
                },
            },
        };
        // A body no equation asked for is taken if it can be had and
        // passed over if it cannot: refusing the whole model for a
        // generator nothing ever calls would lose a model that used
        // to flatten. One an equation named is refused as before.
        if let Err(why) = walkable(class, registry) {
            if !optional.contains(&name) {
                return Err(why);
            }
            continue;
        }
        // A body names what it calls the way it was written there; the
        // walk looks names up in one table, so they are made to agree.
        let mut carried = (*class).clone();
        // And what it inherits travels with it: the walk reads a
        // frame of names, and an input declared in a base is a name
        // like any other. A body written `extends partialScalarFunction`
        // reads `u` and writes `y`, and neither is declared here.
        carried.components = with_inherited_components(class, registry);
        // A local declared `constant Real eps = Modelica.Constants.eps`
        // is a name the walk's frame has never heard: it works out a
        // local's binding against its own frame, and a constant of
        // another package is nobody there. The lengths already get
        // this treatment; the bindings need it too.
        for held in &mut carried.components {
            for written in [&mut held.binding, &mut held.start].into_iter().flatten() {
                *written =
                    substitute_class_constants(written, registry, &class.name, &class.imports, &[]);
            }
        }
        // And a length written as a constant of the package is written
        // as the number it is. `walkable` already counts `state[nState]`
        // as a length the compiler can see; carried as the name, the
        // walk's frame had never heard of `nState`, laid the output out
        // as one number, and the generator's first state reached the
        // evaluator as the array `{localSeed, globalSeed}`.
        if package_lengths_open() {
            for held in &mut carried.components {
                for dimension in &mut held.dimensions {
                    let named = substitute_class_constants(
                        dimension,
                        registry,
                        &class.name,
                        &class.imports,
                        &[],
                    );
                    if let Some(length) = const_eval(&named, &HashMap::new()) {
                        *dimension = Expr::Number(length);
                    }
                }
            }
        }
        records_by_their_fields(&mut carried, class, registry);
        let renamed = records_as_arrays(&mut carried, registry);
        // A local's binding reads a record input the way a statement
        // does, and has to be spelled the way the walk's frame holds it.
        // `dp_curvedOverall_DP` works out every one of its thirty
        // coefficients as a protected local bound on `IN_con.d_hyd`;
        // renamed in the statements only, the first local the walk laid
        // out asked for a field the frame holds as `IN_con[1]`, and the
        // whole call came to nothing.
        if local_record_fields_open() {
            for held in &mut carried.components {
                for written in [&mut held.binding, &mut held.start].into_iter().flatten() {
                    *written =
                        substitute_refs(&subscripts_spelled_out(written, &renamed), &renamed);
                }
            }
        }
        // And a call in a local's binding is named the way the walk's
        // one table knows it, exactly as a call in a statement is.
        if local_binding_calls_open() {
            for held in &mut carried.components {
                for written in [&mut held.binding, &mut held.start].into_iter().flatten() {
                    *written = qualified_in(written, registry, &class.name, &class.imports);
                }
            }
        }
        // What the body's own frame gives a value to: an input, an
        // output, a local. Those are names the walk supplies, and a
        // package constant of the same spelling must not be folded
        // over them.
        let held: Vec<String> = carried
            .components
            .iter()
            .map(|component| component.name.clone())
            .collect();
        // A function the body hands over is specialized here, as it
        // would have been had the body been inlined: see
        // `handed_over_in_walked_body`.
        let algorithm = super::arrays::handed_over_in_walked_body(
            &class.algorithm,
            registry,
            &class.name,
            &class.imports,
        );
        carried.algorithm = qualified_calls(
            &algorithm,
            registry,
            &class.name,
            &class.imports,
            &renamed,
            &held,
        );
        carried.name = name.clone();
        out.push(carried);
        // What the copy calls is gathered with no medium standing, under
        // the writer's name, and then named the way the copy's
        // statements name it: under the pair where the callee is a copy
        // of its own.
        drop(digits);
        drop(marked);
        drop(copy);
        let mut calls = Vec::new();
        // What a local's binding calls is called by the body as much as
        // what a statement calls. `dp_curvedOverall_DP` works out its
        // laminar boundary as a protected local bound on
        // `Modelica.Math.exp(...)`, and a gathering that read the
        // statements alone left that body behind: the walk met the
        // library's `exp` as a function the run had never heard of.
        //
        // Taken if it can be had, the way a declaration's call is: a
        // body only a binding reaches was never carried before, and a
        // model whose walk never asks for it flattened all the same.
        // Refusing it now for a shape the walk cannot carry cost
        // `Inverse_sh_TX`, whose moist air binds a local on
        // `massToMoleFractions` - an answer of a length nobody can see.
        let mut from_bindings = Vec::new();
        if local_binding_calls_open() {
            for held in &class.components {
                for written in [&held.binding, &held.start].into_iter().flatten() {
                    gather_calls(
                        written,
                        registry,
                        &class.name,
                        &class.imports,
                        &mut from_bindings,
                    );
                }
            }
        }
        gather_calls_in_statements(
            &algorithm,
            registry,
            &class.name,
            &class.imports,
            &mut calls,
        );
        for called in from_bindings {
            if !calls.contains(&called) && !wanted.contains(&called) {
                optional.insert(called.clone());
            }
            calls.push(called);
        }
        if let Some((_, medium)) = &pair {
            for called in &mut calls {
                let Some(callee) = registry.get(called.as_str()).copied() else {
                    continue;
                };
                if let Some(paired) = paired_under(callee, registry, medium) {
                    if optional.remove(called.as_str()) {
                        optional.insert(paired.clone());
                    }
                    *called = paired;
                }
            }
        }
        // What an optional body calls is wanted only as much as it is:
        // a generator nothing asks for asks in turn for nothing.
        if optional.contains(&name) {
            optional.extend(calls.iter().filter(|c| !wanted.contains(c)).cloned());
        }
        wanted.extend(calls);
    }
    Ok(out)
}

/// Whether a local's binding reads a record input under the walk's
/// spelling of it. `OXIDELICA_NO_LOCAL_RECORD_FIELDS` leaves the
/// bindings as they were written, so that one binary gives both numbers.
fn local_record_fields_open() -> bool {
    std::env::var_os("OXIDELICA_NO_LOCAL_RECORD_FIELDS").is_none()
}

/// Whether a body left standing under a medium that changes what it
/// reads is carried as a copy of its own, prepared under that medium.
/// `OXIDELICA_NO_CARRIED_MARK` carries every body under the name of the
/// class that wrote it, as before, so that one binary gives both
/// numbers.
fn carried_mark_open() -> bool {
    !CARRIED_MARK_HELD.with(std::cell::Cell::get)
        && std::env::var_os("OXIDELICA_NO_CARRIED_MARK").is_none()
}

thread_local! {
    /// Whether this thread asked for bodies to be carried under the
    /// name that wrote them, whatever medium they were asked under -
    /// what a test does to see the refusal the copy replaces.
    static CARRIED_MARK_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Carry every body left standing under the name that wrote it, on this
/// thread, until the guard is dropped. The environment switch is one
/// for the whole process; a test running beside others needs one of
/// its own.
pub fn hold_back_carried_mark_here() -> CarriedMarkGuard {
    CARRIED_MARK_HELD.with(|held| held.set(true));
    CarriedMarkGuard(())
}

/// Puts the carried mark back where it was.
pub struct CarriedMarkGuard(());

impl Drop for CarriedMarkGuard {
    fn drop(&mut self) {
        CARRIED_MARK_HELD.with(|held| held.set(false));
    }
}

/// Whether a walked body's statements have the answers written here
/// folded to their numbers, and a written list sliced by numbers cut to
/// the slice, before the walk. `OXIDELICA_NO_FOLD_OUTSIDE` leaves them
/// as they were written, so that one binary gives both numbers.
fn fold_outside_open() -> bool {
    !FOLD_OUTSIDE_HELD.with(std::cell::Cell::get)
        && std::env::var_os("OXIDELICA_NO_FOLD_OUTSIDE").is_none()
}

thread_local! {
    /// Whether this thread asked for the fold to be held back - what a
    /// test does to see the refusal the fold replaces.
    static FOLD_OUTSIDE_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Leave a walked body's answers and slices as written, on this thread,
/// until the guard is dropped.
pub fn hold_back_fold_outside_here() -> FoldOutsideGuard {
    FOLD_OUTSIDE_HELD.with(|held| held.set(true));
    FoldOutsideGuard(())
}

/// Puts the fold back where it was.
pub struct FoldOutsideGuard(());

impl Drop for FoldOutsideGuard {
    fn drop(&mut self) {
        FOLD_OUTSIDE_HELD.with(|held| held.set(false));
    }
}

/// An element of an answer this compiler writes itself - `dgelsy` and
/// its neighbours - handed nothing but numbers is folded to its number,
/// and a written list sliced by a range of numbers is cut to the slice.
/// A walked body subscripts only names: a fit a medium makes of its
/// table reached the walk as `dgelsy(...)[1]`, and the slice by the
/// package's `npol` as `{...}[1:npol]`, and the walk refused both,
/// though each is a number or a list of numbers the moment it is read.
/// An answer that is not wholly numbers is left for the walk, and a
/// slice outside the list is left for the walk to refuse by name.
fn fold_outside(e: &Expr) -> Expr {
    if let Expr::Index(base, _) = e {
        if let Expr::Call(called, _) = base.as_ref() {
            if crate::outside::written_here(called) {
                if let Some(value) = const_eval(e, &HashMap::new()) {
                    return Expr::Number(value);
                }
            }
        }
    }
    let below = e.map_children(&mut |child| fold_outside(child));
    if let Expr::Index(base, subs) = &below {
        if let (Expr::Array(items), [Expr::Range(a, None, b)]) = (base.as_ref(), subs.as_slice()) {
            let env = HashMap::new();
            if let (Some(a), Some(b)) = (const_eval(a, &env), const_eval(b, &env)) {
                if a.fract() == 0.0 && b.fract() == 0.0 && a >= 1.0 && b >= 0.0 {
                    let (a, b) = (a as usize, b as usize);
                    if b <= items.len() && a <= b + 1 {
                        return Expr::Array(items[a - 1..b].to_vec());
                    }
                }
            }
        }
    }
    below
}

thread_local! {
    /// The copies carried under a medium, by the name the flat model
    /// calls them: the body that wrote them and the medium they were
    /// asked under. The name is made from the pair, and nothing reads
    /// the pair back out of its spelling - it is looked up here.
    static PAIRS: std::cell::RefCell<HashMap<String, (String, String)>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Forget every pair: the names belong to one flattening, and the next
/// may be of another library.
pub(super) fn forget_pairs() {
    PAIRS.with(|pairs| pairs.borrow_mut().clear());
}

/// The body and the medium behind a name [`carried_under_mark`] made.
pub(super) fn carried_pair(name: &str) -> Option<(String, String)> {
    PAIRS.with(|pairs| pairs.borrow().get(name).cloned())
}

/// The name a call left standing is carried under, where the medium it
/// was asked under gives something the body reads another value.
///
/// A body left for the run is walked with what its preparation settled,
/// and the preparation used to run with no medium in sight: `Med.w`
/// with `w` written in `Base` read `Base`'s constants, and one the
/// medium gave a value to reached the walk as a name nobody declares or
/// as the base's number. So such a body is carried as a copy of its
/// own, prepared under the medium, and the call names that copy. The
/// name is the flat model's own; the pair behind it is kept in a table
/// rather than spelled out for a later reader to take apart.
///
/// Only where the medium changes something the body reads to a number,
/// a Boolean or a list of those: a body the medium changes nothing in
/// is carried exactly as before, under the name that wrote it.
pub(super) fn carried_under_mark(
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
) -> Option<String> {
    if !carried_mark_open() {
        return None;
    }
    let (package, _) = class.name.rsplit_once('.')?;
    let medium = asked_as_package(registry, package)?;
    paired_under(class, registry, &medium)
}

/// The name a body is carried under when a copy carried under `medium`
/// calls it: the pair's, where the medium stands on the line of the
/// package that wrote the body and changes something the body reads,
/// and nothing otherwise.
///
/// The same question [`carried_under_mark`] asks of a call left standing
/// in the flat model, asked of what a copy calls. A copy prepared under
/// its medium used to call its callees under the names that wrote them,
/// so a constant of the medium read one call further down reached the
/// walk as the base's number or as a name nobody declares: `Med.w`
/// calling `h`, which binds a local on `data.MM`, stopped at `unknown
/// variable data.MM`. Whether the medium is a relative is asked of the
/// registry's shape, never of the spelling of the path.
pub(super) fn paired_under(
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
    medium: &str,
) -> Option<String> {
    if !carried_mark_open() || !on_the_line(class, registry, medium) {
        return None;
    }
    if !super::arrays::reads_its_medium(class, registry, medium) {
        return None;
    }
    Some(minted_pair(class, medium))
}

/// Keep the pair of a body and its medium, and give back the name the
/// flat model calls the copy by.
fn minted_pair(class: &ClassDef, medium: &str) -> String {
    let name = format!("{}@{medium}", class.name);
    PAIRS.with(|pairs| {
        pairs
            .borrow_mut()
            .insert(name.clone(), (class.name.clone(), medium.to_string()))
    });
    name
}

/// The name a function handed to another is called by in the copy
/// made for the hand-over: the pair's, where the medium on the mark
/// stands over the class that wrote the function and changes something
/// it reads, and nothing otherwise.
///
/// The third place a medium was lost. A body hands `function g(s = s)`
/// to `solveOneNonlinearEquation`, and the copy made of the solver
/// called `g` under the name that wrote it, so a local of `g` bound on
/// the medium's `data.MM` reached the walk as a name nobody declares.
/// The function handed over is often written inside the function that
/// hands it, `T_s.g`, so what the medium must extend is the nearest
/// class enclosing it that the registry holds as a package - each one
/// asked of the registry, not guessed from the spelling. A function
/// written in the medium itself reads the medium already.
pub(super) fn handed_under_mark(
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
) -> Option<String> {
    if !carried_mark_open() || class.kind != ClassKind::Function {
        return None;
    }
    let medium = asked_as_mark();
    registry
        .get(medium.as_str())
        .filter(|found| found.kind == ClassKind::Package)?;
    let mut enclosing = class.name.as_str();
    let package = loop {
        let (outer, _) = enclosing.rsplit_once('.')?;
        enclosing = outer;
        let held = registry.get(outer)?;
        if held.kind == ClassKind::Package {
            break outer;
        }
    };
    if package == medium || !descends_from(registry, &medium, package) {
        return None;
    }
    if !super::arrays::reads_its_medium(class, registry, &medium) {
        return None;
    }
    Some(minted_pair(class, &medium))
}

/// Whether a function is written in a package `medium` extends, however
/// many steps away, and not in the medium itself: the only bodies a
/// medium can make read differently. Asked of the registry's shape, not
/// of the spelling of the path.
pub(super) fn on_the_line(
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
    medium: &str,
) -> bool {
    class.kind == ClassKind::Function
        && class.name.rsplit_once('.').is_some_and(|(package, _)| {
            package != medium && super::inlining::descends_from(registry, medium, package)
        })
}

thread_local! {
    /// The medium the copy being prepared was carried under, where one
    /// is: what the calls it makes are named under.
    static COPY_MEDIUM: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Name the calls a copy makes under its medium until dropped.
struct PreparingCopy(Option<String>);

impl PreparingCopy {
    fn under(medium: Option<&str>) -> Self {
        PreparingCopy(COPY_MEDIUM.with(|held| held.replace(medium.map(str::to_string))))
    }
}

impl Drop for PreparingCopy {
    fn drop(&mut self) {
        let before = self.0.take();
        COPY_MEDIUM.with(|held| *held.borrow_mut() = before);
    }
}

/// A call made by the copy being prepared, under the pair's name where
/// the callee is paired under the copy's medium.
fn named_in_copy(class: &ClassDef, registry: &HashMap<&str, &ClassDef>) -> String {
    COPY_MEDIUM
        .with(|held| held.borrow().clone())
        .and_then(|medium| paired_under(class, registry, &medium))
        .unwrap_or_else(|| class.name.clone())
}

/// Whether the calls a local's binding makes are named the way the
/// registry knows them and carried out with the body.
/// `OXIDELICA_NO_LOCAL_BINDING_CALLS` leaves them as they were written.
fn local_binding_calls_open() -> bool {
    std::env::var_os("OXIDELICA_NO_LOCAL_BINDING_CALLS").is_none()
}

/// A local record whose every field the declaration already settles,
/// carried as one local per field, each bound to its number.
///
/// A body left for the walk declares `R134aData.Residual res` and reads
/// `res.ns1` and `res.c[i]`, and the values are nowhere but in the
/// record: `extends EOSResidualCoeff(nc = 21, ns1 = 8, c = {...})`.
/// Inlined, such a local is bound field by field before the body runs;
/// carried, it was bound to nothing. Where the record was laid out as
/// an array the walk read each field as the zero an unwritten local
/// starts at, and where a field's length was a name it was not laid
/// out at all and the walk met `res.ns1` as a name nobody declares -
/// which is where the R134a properties stopped, three models on
/// `id.a[1]`. The fields are written as locals of their own - `res.ns1`
/// bound to 8, `res.c` a list of 21 - so a subscript the body decides
/// is read like any other local array.
///
/// Only a local nothing writes and nothing hands on whole, with no
/// modifier of its own, and only where every field comes to a number
/// or a list of numbers. Anything else is left as it was, for the
/// roads below. `OXIDELICA_NO_CARRIED_RECORD_LOCALS` leaves every one
/// as it was, so that one binary gives both numbers.
fn records_by_their_fields(
    carried: &mut ClassDef,
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
) {
    if std::env::var_os("OXIDELICA_NO_CARRIED_RECORD_LOCALS").is_some() {
        return;
    }
    let read = names_read(class);
    let mut out: Vec<Component> = Vec::new();
    for mut local in std::mem::take(&mut carried.components) {
        match settled_fields(&local, class, registry, &read) {
            Some(fields) => out.extend(fields),
            None => {
                if let Some(whole) = record_local_whole(&local, class, registry) {
                    local.binding = Some(whole);
                }
                out.push(local);
            }
        }
    }
    carried.components = out;
}

/// A local record of plain numbers bound, whole, to what its own
/// modifiers and its record's defaults give each field, in the order
/// the walk lays the record out.
///
/// `getPhase_ph` declares `SaturationProperties sat(psat = p, Tsat =
/// 0)` and reads `bubbleEnthalpy(sat)`. Laid out as an array with no
/// binding, `sat` held the zero an unwritten local starts at, the
/// saturation enthalpies were those of zero pressure, a liquid at ten
/// bar was taken for two phases and its heat capacity came out as 0 -
/// a wrong number where the body used to refuse. A field neither the
/// local nor the record gives a value is left alone, and so is a record
/// holding an array or text.
fn record_local_whole(
    local: &Component,
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
) -> Option<Expr> {
    if local.causality != Causality::None || local.binding.is_some() || !local.dimensions.is_empty()
    {
        return None;
    }
    let record = lookup(registry, &local.type_name, &class.name, &class.imports)
        .filter(|of| of.kind == ClassKind::Record)?;
    let declared = record_fields::record_components(registry, record, 0);
    let order = record_fields::handed_record_fields(registry, record);
    if order.is_empty() {
        return None;
    }
    let mut values = Vec::new();
    for name in &order {
        let field = declared.iter().find(|field| &field.name == name)?;
        if !field.dimensions.is_empty() {
            return None;
        }
        let value = match local.modifiers.iter().find(|(given, _)| given == name) {
            Some((_, value)) => value.clone(),
            None => substitute_class_constants(
                field.binding.as_ref()?,
                registry,
                &record.name,
                &record.imports,
                &[],
            ),
        };
        values.push(value);
    }
    Some(Expr::Array(values))
}

/// The fields of one local record as locals of their own, each bound to
/// the number or list the declaration gives it, or nothing where any
/// field is not settled so.
fn settled_fields(
    local: &Component,
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
    read: &[String],
) -> Option<Vec<Component>> {
    if local.causality != Causality::None
        || local.binding.is_some()
        || !local.dimensions.is_empty()
        || !local.modifiers.is_empty()
        || read.iter().any(|name| name == &local.name)
        || writes_to(&class.algorithm, &local.name)
    {
        return None;
    }
    let record = lookup(registry, &local.type_name, &class.name, &class.imports)
        .filter(|of| of.kind == ClassKind::Record)?;
    let mut known: HashMap<String, f64> = HashMap::new();
    let mut out = Vec::new();
    for field in record_fields::record_components(registry, record, 0) {
        let value = substitute_class_constants(
            field.binding.as_ref()?,
            registry,
            &record.name,
            &record.imports,
            &[],
        );
        let mut held = field.clone();
        held.name = format!("{}.{}", local.name, field.name);
        held.type_name = "Real".to_string();
        held.causality = Causality::None;
        held.modifiers = Vec::new();
        held.start = None;
        match field.dimensions.as_slice() {
            [] => {
                let number = const_eval(&value, &known)?;
                known.insert(field.name.clone(), number);
                held.binding = Some(Expr::Number(number));
            }
            [length] => {
                let Expr::Array(items) = &value else {
                    return None;
                };
                let numbers: Vec<f64> = items
                    .iter()
                    .map(|item| const_eval(item, &known))
                    .collect::<Option<_>>()?;
                let length = substitute_class_constants(
                    length,
                    registry,
                    &record.name,
                    &record.imports,
                    &[],
                );
                if const_eval(&length, &known)? != numbers.len() as f64 {
                    return None;
                }
                held.dimensions = vec![Expr::Number(numbers.len() as f64)];
                held.binding = Some(Expr::Array(numbers.into_iter().map(Expr::Number).collect()));
            }
            _ => return None,
        }
        out.push(held);
    }
    (!out.is_empty()).then_some(out)
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
        // The same list the hand-over road writes: arrays element by
        // element, text left out. A record holding an array used to be
        // passed over here altogether, and its every field reached the
        // run as a name nothing declared - which is how the NASA gas
        // data, seven coefficients at a time, stopped at `data.Tlimit`.
        let members = record_fields::handed_record_fields(registry, of);
        let plain = of
            .components
            .iter()
            .all(|member| reduces_to_primitive(registry, &member.type_name, &of.name, &of.imports));
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
    held: &[String],
) -> Vec<Statement> {
    let inner = |body: &[Statement]| qualified_calls(body, registry, scope, imports, renamed, held);
    // A body that is walked rather than inlined still reads the
    // constants of the package it belongs to - the air model's
    // `airBaseProp_pT` writes `aux.R_s := Constants.R_s`, a field of a
    // record constant that the walk's frame of names has never heard.
    // The parameter road folds such a constant to its number; the walk
    // needs the same, or `Constants.R_s` reaches the run as an unknown
    // and every value that flows from it is a NaN. So each statement's
    // scalar constants are folded here, the way a component's binding
    // above already is. Only the scalar ones: a constant that comes to
    // an array or a record is left for the walk, which carries its own
    // machinery for those - a list dropped over the top of it indexes
    // past the end and panics.
    // And a constant of the package the body is written in is named
    // with no path at all: a medium writes `constant AbsolutePressure
    // reference_p = 101325` beside the functions that read it, and a
    // body carried out to the walk rather than inlined took that name
    // to the run with nothing giving it a value. The dotted road above
    // never saw it, because the name has no dot. The body's own
    // components are held back: a local called the same as a
    // package's constant is the local.
    let shadow: Vec<&str> = held.iter().map(String::as_str).collect();
    let expr = |e: &Expr| {
        let e = substitute_scalar_class_constants(e, registry, scope, imports);
        let e = substitute_class_constants(&e, registry, scope, imports, &shadow);
        let e = if fold_outside_open() {
            fold_outside(&e)
        } else {
            e
        };
        let e = subscripts_spelled_out(&e, renamed);
        substitute_refs(&qualified_in(&e, registry, scope, imports), renamed)
    };
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
                    .map(|class| named_in_copy(class, registry))
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

/// Whether a call left standing in the flat model has its named
/// arguments put in the seats its callee declares.
/// `OXIDELICA_NO_FLAT_NAMED_ORDER` leaves them named, so that one
/// binary gives both numbers.
fn flat_named_order_open() -> bool {
    std::env::var_os("OXIDELICA_NO_FLAT_NAMED_ORDER").is_none()
}

/// Every call the flat model still makes, with its named arguments put
/// in the seats the callee declares.
///
/// The walk binds what it is handed by position, and a body carried out
/// to it already had its own calls reordered by [`qualified_in`]. A call
/// standing in the flat model's own equations never went through that
/// road: `f(p = 1 + time)` reached the run with the name still on its
/// argument, and the run refused it as a subscript it could not carry.
/// The R134a media and the check valve test stopped there. A call whose
/// names do not fit the callee is left exactly as it was, for whatever
/// reads it next to refuse by name.
pub(super) fn put_named_arguments_in_place(model: &mut Model, registry: &HashMap<&str, &ClassDef>) {
    if !flat_named_order_open() {
        return;
    }
    fn seat(expr: &Expr, registry: &HashMap<&str, &ClassDef>) -> Expr {
        let below = expr.map_children(&mut |child| seat(child, registry));
        let Expr::Call(name, args) = below else {
            return below;
        };
        if !args.iter().any(|arg| matches!(arg, Expr::NamedArg(..))) {
            return Expr::Call(name, args);
        }
        // The flat model names what it calls the way the registry knows
        // it: flattening qualified it on the way out.
        let written = carried_pair(&name).map_or_else(|| name.clone(), |(body, _)| body);
        match lookup(registry, &written, "", &[]).filter(|class| class.kind == ClassKind::Function)
        {
            Some(class) => {
                let args = in_declared_order(class, registry, args);
                Expr::Call(name, args)
            }
            None => Expr::Call(name, args),
        }
    }
    let mut fix = |expr: &mut Expr| *expr = seat(expr, registry);
    for equation in model
        .equations
        .iter_mut()
        .chain(model.initial_equations.iter_mut())
    {
        fix(&mut equation.lhs);
        fix(&mut equation.rhs);
    }
    for (condition, _) in &mut model.asserts {
        fix(condition);
    }
    for conditional in &mut model.conditional {
        conditional.conditions.iter_mut().for_each(&mut fix);
        for branch in &mut conditional.branches {
            for equation in branch {
                fix(&mut equation.lhs);
                fix(&mut equation.rhs);
            }
        }
    }
    for clause in &mut model.when_clauses {
        for branch in &mut clause.branches {
            fix(&mut branch.condition);
            for action in &mut branch.actions {
                match action {
                    WhenAction::Assign(_, value)
                    | WhenAction::Reinit(_, value)
                    | WhenAction::TupleAssign(_, value)
                    | WhenAction::Assert(value, _) => fix(value),
                    WhenAction::Call(name, args) => {
                        if let Expr::Call(_, seated) =
                            seat(&Expr::Call(name.clone(), args.clone()), registry)
                        {
                            *args = seated;
                        }
                    }
                    WhenAction::Terminate(_) | WhenAction::Loop(_) | WhenAction::Choice(_) => {}
                }
            }
        }
    }
    for component in &mut model.components {
        for written in [&mut component.binding, &mut component.start]
            .into_iter()
            .flatten()
        {
            fix(written);
        }
    }
}

/// A call's arguments in the order the callee declares its inputs.
///
/// What comes back has no `NamedArg` left in it where every name was
/// one the callee declares; where a name is not one of them, or an
/// input is left without a value and has nothing to fall back on, the
/// arguments are handed back untouched and whatever reads them next
/// says what is wrong with them. Guessing an order here would be a
/// wrong number where a refusal is owed.
fn in_declared_order(
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
    args: Vec<Expr>,
) -> Vec<Expr> {
    if !args.iter().any(|arg| matches!(arg, Expr::NamedArg(..))) {
        return args;
    }
    let held = with_inherited_components(class, registry);
    let inputs: Vec<&Component> = held
        .iter()
        .filter(|component| component.causality == Causality::Input)
        .collect();
    let at = args
        .iter()
        .position(|arg| matches!(arg, Expr::NamedArg(..)))
        .expect("just checked there is one");
    // Nothing positional may follow a named argument; where one does,
    // the call is a mistake about the function and is left as it was.
    if args[at..]
        .iter()
        .any(|arg| !matches!(arg, Expr::NamedArg(..)))
    {
        return args;
    }
    let mut out: Vec<Option<Expr>> = inputs.iter().map(|_| None).collect();
    if out.len() < at {
        return args;
    }
    for (seat, arg) in args[..at].iter().enumerate() {
        out[seat] = Some(arg.clone());
    }
    for arg in &args[at..] {
        let Expr::NamedArg(name, value) = arg else {
            unreachable!("the loop above checked every one of these");
        };
        let Some(seat) = inputs.iter().position(|input| &input.name == name) else {
            return args;
        };
        if out[seat].is_some() {
            return args;
        }
        out[seat] = Some((**value).clone());
    }
    // A seat left empty is an input the call did not fill: allowed
    // where the declaration gives it a value of its own, and the
    // trailing empties simply end the list. A gap before something
    // that was filled cannot be closed without moving an argument
    // into a seat nobody named.
    let last = out.iter().rposition(Option::is_some);
    let Some(last) = last else {
        return args;
    };
    let mut ordered = Vec::with_capacity(last + 1);
    for (seat, held) in out.into_iter().take(last + 1).enumerate() {
        match held {
            Some(value) => ordered.push(value),
            // The input's own default is written where the callee
            // was, and its names mean what they mean there: the ideal
            // gas enthalpy defaults `exclEnthForm =
            // excludeEnthalpyOfFormation`, a constant of its own
            // package, and put in the seat unread that name reached
            // the run from the body of whoever called it.
            None => match inputs[seat]
                .binding
                .clone()
                .or_else(|| inputs[seat].start.clone())
            {
                Some(value) if callee_defaults_open() => ordered.push(substitute_class_constants(
                    &value,
                    registry,
                    &class.name,
                    &class.imports,
                    &[],
                )),
                Some(value) => ordered.push(value),
                None => return args.clone(),
            },
        }
    }
    ordered
}

/// A subscripted record field written the way the renaming names it.
///
/// A body reads `data.alow[1]` as a subscript of the name
/// `data.alow`, and the renaming speaks for the flat spelling
/// `data.alow[1]` - one name, no subscript. The name alone is nothing
/// the map knows, so the field went to the run unrenamed. Only where
/// the subscripts are numbers and the flat name is one the map
/// speaks for: anything else is left exactly as it was.
fn subscripts_spelled_out(expr: &Expr, renamed: &HashMap<String, Expr>) -> Expr {
    if let Expr::Index(base, subscripts) = expr {
        if let Expr::Ref(name) = base.as_ref() {
            let indices: Option<Vec<i64>> = subscripts
                .iter()
                .map(|s| const_eval(s, &HashMap::new()).map(|n| n as i64))
                .collect();
            if let Some(indices) = indices {
                let flat = element_name(name, &indices);
                if let Some(instead) = renamed.get(&flat) {
                    return instead.clone();
                }
            }
        }
    }
    expr.map_children(&mut |child| subscripts_spelled_out(child, renamed))
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
        Expr::Call(name, args) if record_constructors_open() => {
            if let Some(fields) = constructed_record(name, args, registry, scope, imports) {
                return Expr::Array(fields.iter().map(recur).collect());
            }
            qualified_call(name, args, registry, scope, imports)
        }
        Expr::Call(name, args) => qualified_call(name, args, registry, scope, imports),
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
        // A call written as a named argument is a call all the same:
        // the reference air builds its state as `ThermodynamicState(h =
        // specificEnthalpy_dT(d, T), ...)`, and passed over here the
        // call reached the walk under the bare name it was written with.
        Expr::NamedArg(name, value) if named_values_open() => {
            Expr::NamedArg(name.clone(), Box::new(recur(value)))
        }
        _ => expr.clone(),
    }
}

/// Whether a call inside a named argument of a carried body is named the
/// way the registry knows it. `OXIDELICA_NO_NAMED_VALUES` leaves it as
/// written, so that one binary gives both numbers.
fn named_values_open() -> bool {
    std::env::var_os("OXIDELICA_NO_NAMED_VALUES").is_none()
}

/// Whether a record built by its constructor inside a carried body is
/// written out as its fields. `OXIDELICA_NO_WALKED_CONSTRUCTORS` leaves
/// the call, so that one binary gives both numbers.
fn record_constructors_open() -> bool {
    std::env::var_os("OXIDELICA_NO_WALKED_CONSTRUCTORS").is_none()
}

/// A call to a user function named the way the registry knows it, with
/// its arguments in the seats the callee declares.
fn qualified_call(
    name: &str,
    args: &[Expr],
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
) -> Expr {
    let of =
        lookup(registry, name, scope, imports).filter(|class| class.kind == ClassKind::Function);
    let named = of
        .map(|class| named_in_copy(class, registry))
        .unwrap_or_else(|| name.to_string());
    let args: Vec<Expr> = args
        .iter()
        .map(|arg| qualified_in(arg, registry, scope, imports))
        .collect();
    let args = match of {
        Some(class) => records_spelled_out(class, registry, scope, imports, args),
        None => args,
    };
    // A named argument is put in the seat the callee declares it in.
    // The walk binds what it is handed by position - the frame is a
    // list of inputs, and the name a call wrote is nothing to it - so
    // `h_T(data = data, T = u)`, which is how every ideal gas reads its
    // NASA coefficients, gave the walk `data` where `T` was declared and
    // left `data` itself standing as a name the run never heard of. The
    // inlining road already does this reordering; a body carried out to
    // the walk never had it.
    let args = match of {
        Some(class) => in_declared_order(class, registry, args),
        None => args,
    };
    Expr::Call(named, args)
}

/// The fields a record constructor call in a carried body builds, in
/// the order the walk holds a record: `ThermodynamicState(d = d, T = T,
/// h = ..., p = ...)` becomes the list of its fields. The walk holds a
/// record as an array and knows nothing of constructors, so the call
/// used to reach it as a function nobody had heard of. Only a record of
/// plain numbers every field of which the call gives or the declaration
/// binds; anything else is left as the call it was.
fn constructed_record(
    name: &str,
    args: &[Expr],
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
) -> Option<Vec<Expr>> {
    let record =
        lookup(registry, name, scope, imports).filter(|of| of.kind == ClassKind::Record)?;
    let fields = record_fields::record_components(registry, record, 0);
    let members = record_fields::handed_record_fields(registry, record);
    if fields.len() != members.len()
        || fields.iter().any(|field| !field.dimensions.is_empty())
        || !record.components.iter().all(|member| {
            reduces_to_primitive(registry, &member.type_name, &record.name, &record.imports)
        })
    {
        return None;
    }
    let at = args
        .iter()
        .position(|arg| matches!(arg, Expr::NamedArg(..)))
        .unwrap_or(args.len());
    if args[at..]
        .iter()
        .any(|arg| !matches!(arg, Expr::NamedArg(..)))
        || at > fields.len()
    {
        return None;
    }
    let mut out: Vec<Option<Expr>> = vec![None; fields.len()];
    for (seat, arg) in args[..at].iter().enumerate() {
        out[seat] = Some(arg.clone());
    }
    for arg in &args[at..] {
        let Expr::NamedArg(given, value) = arg else {
            return None;
        };
        let seat = fields.iter().position(|field| &field.name == given)?;
        if out[seat].is_some() {
            return None;
        }
        out[seat] = Some((**value).clone());
    }
    out.into_iter()
        .zip(&fields)
        .map(|(held, field)| held.or_else(|| field.binding.clone()))
        .collect()
}

/// Whether a record constant of an enclosing package, handed by its
/// bare name to a call in a carried body, is sent as its fields.
/// `OXIDELICA_NO_CARRIED_RECORD_CONSTANTS` closes the road, so that one
/// binary gives both numbers.
fn carried_record_constants_open() -> bool {
    std::env::var_os("OXIDELICA_NO_CARRIED_RECORD_CONSTANTS").is_none()
}

/// A call's record arguments written out as the fields the callee
/// reads, where the argument is a record constant of a package the
/// body is written inside.
///
/// Moist air's `h_pTX` is walked rather than inlined, and it hands
/// `data = steam` to the ideal gas enthalpy, where `steam` is the
/// `constant DataRecord steam = SingleGasesData.H2O` its package
/// declares beside it. The inlining road reads such a name as the
/// record it names; the road that carries a body out to the walk did
/// not, and `steam` reached the run as a name nothing gives a value
/// to - which stopped every moist-air model that asked for an
/// enthalpy. The callee reads its record by position in
/// [`record_fields::handed_record_fields`], the same list its own
/// renaming uses, so the fields are written out in that order. A field
/// that does not come to a number leaves the argument as it was, for
/// the run to refuse by name rather than read out of the wrong seat.
fn records_spelled_out(
    callee: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    args: Vec<Expr>,
) -> Vec<Expr> {
    if !carried_record_constants_open() {
        return args;
    }
    let held = with_inherited_components(callee, registry);
    let inputs: Vec<&Component> = held
        .iter()
        .filter(|component| component.causality == Causality::Input)
        .collect();
    let spelled = |input: Option<&&Component>, arg: &Expr| -> Option<Expr> {
        let Expr::Ref(named) = arg else {
            return None;
        };
        if named.contains('.') || named.contains('[') {
            return None;
        }
        let input = input.filter(|input| input.dimensions.is_empty())?;
        let of = lookup(registry, &input.type_name, &callee.name, &callee.imports)
            .filter(|of| of.kind == ClassKind::Record)?;
        let fields = record_fields::handed_record_fields(registry, of);
        if fields.is_empty() {
            return None;
        }
        let mut values = Vec::with_capacity(fields.len());
        for field in &fields {
            let whole = format!("{named}.{field}");
            let value = match field.contains('[') {
                true => constant_element(&whole, registry, scope, imports)
                    .filter(|value| matches!(value, Expr::Number(_)))?,
                false => Expr::Number(class_constant_at(registry, &whole, scope, imports, 0)?),
            };
            values.push(value);
        }
        Some(Expr::Array(values))
    };
    let mut seat = 0;
    args.into_iter()
        .map(|arg| match arg {
            Expr::NamedArg(name, value) => {
                let input = inputs.iter().find(|input| input.name == name);
                let value = spelled(input, &value).unwrap_or(*value);
                Expr::NamedArg(name, Box::new(value))
            }
            positional => {
                let input = inputs.get(seat);
                seat += 1;
                spelled(input, &positional).unwrap_or(positional)
            }
        })
        .collect()
}

/// Every user function an expression calls.
pub(super) fn gather_calls(
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
        // A specialized copy is named after what went into it and is
        // not in the registry: it was made for this model out of a
        // function handed another function.
        } else if super::statements::specialization(name).is_some() || carried_pair(name).is_some()
        {
            out.push(name.clone());
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
        // A call written inside a list, or as a named argument of
        // another: moist air's `h_pTX` answers with `{h_Tlow(data =
        // steam, ...), h_Tlow(data = dryair, ...)} * {X_steam, X_air}`,
        // and a gathering that stopped at the braces carried the body
        // out without the one it calls. The run then met `h_Tlow` as a
        // built-in it had never heard of.
        Expr::Array(items) if carried_array_calls_open() => items
            .iter()
            .for_each(|item| gather_calls(item, registry, scope, imports, out)),
        Expr::NamedArg(_, value) if carried_array_calls_open() => {
            gather_calls(value, registry, scope, imports, out)
        }
        _ => {}
    }
}

/// Whether a body answering with several things, one an array of a
/// length the compiler can see, is walked. `OXIDELICA_NO_MIXED_ANSWERS`
/// refuses it as before, so that one binary gives both numbers.
fn mixed_answers_open() -> bool {
    std::env::var_os("OXIDELICA_NO_MIXED_ANSWERS").is_none()
}

/// The length of a one-dimensional output, where it is a number or a
/// constant of the package the function belongs to. `None` for a
/// scalar, for more than one dimension, and for a length only the call
/// could say.
pub(super) fn settled_length(
    component: &Component,
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
) -> Option<usize> {
    let [only] = component.dimensions.as_slice() else {
        return None;
    };
    let named = substitute_class_constants(only, registry, &class.name, &class.imports, &[]);
    const_eval(&named, &HashMap::new())
        .filter(|length| *length >= 0.0 && length.fract() == 0.0)
        .map(|length| length as usize)
}

/// Whether a carried body's lengths written as package constants are
/// written as numbers. `OXIDELICA_NO_PACKAGE_LENGTHS` leaves them as
/// names, so that one binary gives both numbers.
fn package_lengths_open() -> bool {
    std::env::var_os("OXIDELICA_NO_PACKAGE_LENGTHS").is_none()
}

/// Whether a call inside a list or a named argument is gathered with
/// the body that writes it. `OXIDELICA_NO_CARRIED_ARRAY_CALLS` closes
/// the road, so that one binary gives both numbers.
fn carried_array_calls_open() -> bool {
    std::env::var_os("OXIDELICA_NO_CARRIED_ARRAY_CALLS").is_none()
}

/// Whether an input left out of a carried call takes its default read
/// where the callee wrote it. `OXIDELICA_NO_CALLEE_DEFAULTS` closes the
/// road, so that one binary gives both numbers.
fn callee_defaults_open() -> bool {
    std::env::var_os("OXIDELICA_NO_CALLEE_DEFAULTS").is_none()
}

/// Every user function the statements of a body call.
pub(super) fn gather_calls_in_statements(
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
                    // A body with no outputs answers nothing, so a
                    // walk has nothing to do with it: the standard
                    // library shouts through `Streams.error` and
                    // `print`, and those take a String and give back
                    // nothing at all. Carried along, they fail the
                    // walkability check and take the whole model with
                    // them - for a branch that may never be taken.
                    // Inlining already treats such a body as nothing
                    // (see `an external body with no outputs`); this
                    // is the same rule where bodies are carried.
                    let answers = class
                        .components
                        .iter()
                        .any(|held| held.causality == Causality::Output);
                    // A guard written in Modelica is another matter:
                    // it answers nothing, but its body is statements a
                    // walk can run, and the checks in them are the
                    // point of calling it. R134a guards every
                    // property it reads from `p` and `T` with
                    // `phaseBoundaryAssert(p, T)`; left behind, the
                    // walk met it as a function nobody had heard of.
                    let guard = !answers
                        && !class.external
                        && class.builtin.is_none()
                        && walkable(class, registry).is_ok()
                        && std::env::var_os("OXIDELICA_NO_CARRIED_GUARDS").is_none();
                    if answers || guard {
                        out.push(class.name.clone());
                    }
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

/// Whether `class` declares `name` as one of its own inputs.
fn is_input(class: &ClassDef, name: &str) -> bool {
    class
        .components
        .iter()
        .any(|c| c.name == name && c.causality == Causality::Input)
}

/// What a body the run walks may be made of. The run carries numbers,
/// so anything shaped otherwise is refused here rather than left to
/// fail at the first step.
pub(super) fn walkable(
    class: &ClassDef,
    registry: &HashMap<&str, &ClassDef>,
) -> Result<(), String> {
    for component in &class.components {
        // An array goes in, is held while the walk runs, and may come
        // back: a body answering with several numbers is asked once for
        // each of them. Only a length the compiler can see, though -
        // the model has to name every element it takes.
        if component.causality == Causality::Output && !component.dimensions.is_empty() {
            // A length written as a constant of the package the
            // function belongs to counts as one the compiler can see:
            // a random generator answers with `state[nState]`, and
            // `nState` is a number the package states outright.
            let settled = |dimension: &Expr| -> bool {
                let named = substitute_class_constants(
                    dimension,
                    registry,
                    &class.name,
                    &class.imports,
                    &[],
                );
                const_eval(&named, &HashMap::new()).is_some()
            };
            let settled_length = match component.dimensions.as_slice() {
                [only] => settled(only),
                _ => false,
            };
            // A length written as the size of an input is one the
            // call site settles: `Y[size(X, 1)]` is as long as the `X`
            // the model hands in, and the model names every element of
            // that. The compositions of a medium answer this way, and
            // the call site takes the length from the list it hands in.
            let of_an_input = matches!(
                component.dimensions.as_slice(),
                [Expr::Call(size, args)] if size == "size" && args.len() == 2
                    && matches!(&args[0], Expr::Ref(of) if is_input(class, of))
            ) && super::arrays::size_of_input_open();
            let [Expr::Number(_)] = component.dimensions.as_slice() else {
                if settled_length || of_an_input {
                    continue;
                }
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
    // A body may answer with several numbers, and the call asks for
    // the one it wants: `(d, T) := dTofph(...)` takes both, `f(x)[2]`
    // the second. What the run cannot carry is a mixture of shapes -
    // the numbers of the answer are laid out one after another, and
    // an array among them would need a length at every call site to
    // say where the next one starts.
    // What a function declares, its bases included: the standard
    // library writes `extends partialScalarFunction` and gets its `u`
    // and its `y` from there, declaring only the extra inputs it
    // wants. Looking at the class's own components alone, such a
    // function answers with nothing and is refused for it.
    let held = with_inherited_components(class, registry);
    let outputs: Vec<&Component> = held
        .iter()
        .filter(|c| c.causality == Causality::Output)
        .collect();
    match outputs.len() {
        // A guard answers nothing and is walked for its checks alone:
        // it is called as a statement, where nothing asks it for a
        // number. See `gather_calls_in_statements`.
        0 if !class.external
            && class.builtin.is_none()
            && std::env::var_os("OXIDELICA_NO_CARRIED_GUARDS").is_none() => {}
        0 => {
            return Err(format!(
                "`{}` is called where nothing could inline it, so the run walks its body - \
                 and a body walked at run time answers with something, not nothing",
                class.name
            ))
        }
        1 => {}
        several => {
            // An array among them is carried when its length is one the
            // compiler can see: the answer is then laid end to end at
            // lengths every reader knows, and each output starts where
            // the ones before it leave off. `random` answers with a
            // number and `stateOut[nState]`, and `nState` is a constant
            // of the package.
            let unseen = outputs.iter().find(|c| {
                !c.dimensions.is_empty()
                    && (!mixed_answers_open() || settled_length(c, class, registry).is_none())
            });
            if let Some(spread) = unseen {
                return Err(format!(
                    "`{}` is called where nothing could inline it, so the run walks its \
                     body - and it answers with {several} things, of which `{}` is an \
                     array: the run lays the answers end to end, and cannot say where \
                     one of unknown length leaves off",
                    class.name, spread.name
                ));
            }
        }
    }
    Ok(())
}
