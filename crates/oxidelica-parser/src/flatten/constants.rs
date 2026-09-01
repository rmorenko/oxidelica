//! What a constant of a class is worth, where the name of one is
//! written somewhere that has to know: a dimension, a condition, a
//! loop bound.
//!
//! A constant may be declared in the class, in a package around it, or
//! brought in by an import, and the value may be a number, an array,
//! or a name for another constant. What is found is remembered while
//! one registry stands.
//!
//! Carved out of `names` unchanged.

use super::*;
use std::cell::RefCell;

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
        .iter()
        .find(|(n, _)| n == member)
        .and_then(|(_, binding)| binding.clone())?;
    // A constant written on another of the same package - `X_default
    // = reference_X` - is that other one, and the gathering knows
    // what each of them came to here. Substituting by scope alone
    // would ask the interface, where the name has no value.
    let binding = match &binding {
        Expr::Ref(other) => constants
            .iter()
            .find(|(n, _)| n == other)
            .and_then(|(_, held)| held.clone())
            .unwrap_or(binding),
        _ => binding,
    };
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
        let base = match extend.from_base {
            true => inherited_class(registry, class, &extend.base, 0),
            false => lookup(registry, &extend.base, &class.name, &class.imports),
        }
        .filter(|found| found.name != class.name);
        if let Some(base) = base {
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
            // A declaration of this level outranks what this level's
            // own `extends` said - but only where it says something.
            // `constant SpecificEnthalpy reference_h` in a medium's
            // interface is a declaration with nothing behind it, and
            // overwriting with nothing loses the 104929 the medium
            // gave it through the very `extends` above.
            let held = out
                .iter_mut()
                .find(|(existing, _)| existing == &component.name);
            match (held, binding) {
                (Some(held), Some(value)) => held.1 = Some(value),
                (Some(_), None) => {}
                (None, binding) => out.push((component.name.clone(), binding)),
            }
        }
    }
}

/// A constant's value in the shape its declaration gave it.
///
/// Values travel as numbers; a Boolean written back as `Number(0.0)`
/// is an Integer where a Boolean is needed.
fn as_declared(
    registry: &HashMap<&str, &ClassDef>,
    name: &str,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
    value: f64,
) -> Expr {
    match class_constant_is_boolean(registry, name, scope, imports, depth) {
        true => Expr::Bool(value != 0.0),
        false => Expr::Number(value),
    }
}

/// The same, for a constant found by walking out of the classes this
/// one is written inside: the name is bare, so the walk that found it
/// is the walk that says what it was declared as.
fn enclosing_as_declared(
    registry: &HashMap<&str, &ClassDef>,
    name: &str,
    scope: &str,
    depth: usize,
    value: f64,
) -> Expr {
    let mut at = scope;
    while let Some((outer, _)) = at.rsplit_once('.') {
        if class_constant_is_boolean(registry, &format!("{outer}.{name}"), outer, &[], depth) {
            return Expr::Bool(value != 0.0);
        }
        at = outer;
    }
    Expr::Number(value)
}

/// Whether a class constant was declared Boolean.
///
/// The value of a constant travels as a number - one and zero - which
/// is what arithmetic wants and what a condition cannot use: `false`
/// arriving as `Number(0.0)` is an Integer where a Boolean is needed,
/// and a medium's `final ph_explicit = true` reached every `if` that
/// asks it as a number. The declaration still says which it was, so
/// the substitution asks before it writes the value down.
pub(super) fn class_constant_is_boolean(
    registry: &HashMap<&str, &ClassDef>,
    name: &str,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
) -> bool {
    if depth > MAX_CONSTANT_DEPTH {
        return false;
    }
    let Some((class_path, member)) = name.rsplit_once('.') else {
        return false;
    };
    let Some(class) = lookup(registry, class_path, scope, imports) else {
        return false;
    };
    let mut kinds: Vec<(String, String)> = Vec::new();
    gather_package_constant_types(registry, class, 0, &mut kinds);
    kinds
        .iter()
        .any(|(held, kind)| held == member && kind == "Boolean")
}

/// The declared type of every constant a package holds, its bases
/// included, in the same order [`gather_package_constants`] gathers
/// their values.
fn gather_package_constant_types(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    depth: usize,
    out: &mut Vec<(String, String)>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, &class.name, &class.imports) {
            gather_package_constant_types(registry, base, depth + 1, out);
        }
    }
    for component in &class.components {
        if matches!(
            component.variability,
            Variability::Constant | Variability::Parameter
        ) {
            out.retain(|(existing, _)| existing != &component.name);
            out.push((component.name.clone(), component.type_name.clone()));
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
                Some(value) if class_constant_is_boolean(registry, name, scope, imports, depth) => {
                    Expr::Bool(value != 0.0)
                }
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
                    return as_declared(registry, target, scope, imports, depth, value);
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
                        return as_declared(
                            registry,
                            &format!("{target}.{name}"),
                            scope,
                            imports,
                            depth,
                            value,
                        );
                    }
                }
            }
            // A constant of a package this class is written inside is
            // named without one: `nXi` inside `BaseProperties` is the
            // medium's. Only packages are asked - what a model holds is
            // not in view of what is written inside another class of it.
            if !shadow.contains(&name.as_str()) {
                if let Some(value) = enclosing_constant(registry, name, scope, depth) {
                    return enclosing_as_declared(registry, name, scope, depth, value);
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
                    // The function the call really means: the medium this
                    // was asked under may have redeclared it with inputs
                    // its base never had.
                    let class = inlining::function_asked_under(class, registry);
                    inlining::inline_function(
                        class,
                        &args,
                        &shapes,
                        &HashMap::new(),
                        registry,
                        depth,
                    )
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
    // The medium a body was reached by belongs to the question: one
    // interface constant under two media is two numbers, and a first
    // asking made without a mark - the way a body is carried out to
    // the walk - would otherwise answer every later one.
    // Which road the asking came down belongs to the question as much
    // as the mark does: a parameter being settled may be answered from
    // the medium a body was reached by, and no other reader may have
    // that answer. Held in the key rather than by passing the cache
    // by, which cost the library an hour of walks it had already made.
    let key = (
        name.to_string(),
        scope.to_string(),
        match SETTLING_PARAMETER.with(|on| on.get()) {
            false => super::inlining::asked_as_mark(),
            true => format!("parameter|{}", super::inlining::asked_as_mark()),
        },
    );
    if let Some(remembered) = NAMED.with(|named| named.borrow().get(&key).copied()) {
        return remembered;
    }
    let found = enclosing_constant_at(registry, name, scope, depth);
    NAMED.with(|named| named.borrow_mut().insert(key, found));
    found
}

thread_local! {
    /// Whether what is being settled is a parameter's value, where a
    /// number is the whole of what is wanted.
    pub(super) static SETTLING_PARAMETER: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

/// Hold [`SETTLING_PARAMETER`] for as long as a parameter is being
/// settled, and put it back after - including where the settling left
/// by an error.
pub(super) struct SettlingParameter(bool);

impl SettlingParameter {
    pub(super) fn now() -> SettlingParameter {
        SettlingParameter(SETTLING_PARAMETER.with(|on| on.replace(true)))
    }
}

impl Drop for SettlingParameter {
    fn drop(&mut self) {
        SETTLING_PARAMETER.with(|on| on.set(self.0));
    }
}

thread_local! {
    /// What a name written inside a package came to, by name and by
    /// where it was written, under the name the body was asked as.
    pub(super) static NAMED: RefCell<HashMap<(String, String, String), Option<f64>>> =
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
            // What the gathering found is the answer: it walked the
            // bases in order of nearness and took what each level's
            // `extends` said about the level below. Asking the path
            // again would land on the declaration in the interface,
            // which for a medium's `reference_h` says nothing at all.
            if let Some((_, held)) = constants.iter().find(|(known, _)| known == name) {
                let Some(held) = held else {
                    // An interface declares the name and says nothing
                    // about it - `constant SpecificEnthalpy
                    // reference_h` of `PartialLinearFluid` - because
                    // whoever extends it is meant to. The medium the
                    // model wrote is on the asked-as mark, and its own
                    // gathering holds the value the `extends` gave.
                    return asked_as_constant(registry, name, &owner.name, depth);
                };
                // A value written on this package's other constants -
                // `nXi = if reducedX then nS - 1 else nS` - is worked
                // out against the gathering itself, which knows what
                // each of them came to for this package. Reading them
                // by path would ask the interface again.
                // One built on another settles a round later, so the
                // rounds run until nothing new comes of them.
                let mut known: HashMap<String, f64> = HashMap::new();
                loop {
                    let before = known.len();
                    for (other, value) in &constants {
                        if known.contains_key(other) {
                            continue;
                        }
                        let Some(value) = value else { continue };
                        let settled = substitute_at(
                            value,
                            registry,
                            &owner.name,
                            &owner.imports,
                            &[],
                            depth + 1,
                            true,
                        );
                        if let Some(number) = const_eval(&settled, &known) {
                            known.insert(other.clone(), number);
                        }
                    }
                    if known.len() == before {
                        break;
                    }
                }
                let settled = const_eval(held, &known).or_else(|| {
                    let settled = substitute_at(
                        held,
                        registry,
                        &owner.name,
                        &owner.imports,
                        &[],
                        depth + 1,
                        true,
                    );
                    const_eval(&settled, &known)
                });
                return settled;
            }
        }
        prefix = head;
    }
    None
}

/// The same name asked of the package a body was reached by.
///
/// A medium's function is written in the interface, and a constant it
/// reads is declared there without a value: the value stands in the
/// `extends` of whichever medium the model chose. Only that call chain
/// knows the choice, and it carries it on the asked-as mark; the
/// registry cannot be asked which subclass a model wrote.
///
/// Guarded the way every other reader of the mark is guarded: the mark
/// must descend from the package that declared the name, or it says
/// nothing about it.
fn asked_as_constant(
    registry: &HashMap<&str, &ClassDef>,
    name: &str,
    owner: &str,
    depth: usize,
) -> Option<f64> {
    if depth > MAX_CONSTANT_DEPTH {
        return None;
    }
    // Only where a number is what is wanted. A constant of a medium
    // carries a unit, and the number that replaces it does not: fold
    // `cp_const` into `h = cp_const*T` and the dimensional layer reads
    // kelvin against joules per kilogram and refuses a sound model.
    // A parameter asking to be evaluated before the run has no such
    // reader - it wants the digit or nothing - so that is the one
    // road this answers on.
    if !SETTLING_PARAMETER.with(|on| on.get()) {
        return None;
    }
    let under = super::inlining::asked_as_package(registry, owner)?;
    class_constant_at(registry, &format!("{under}.{name}"), &under, &[], depth + 1)
}
