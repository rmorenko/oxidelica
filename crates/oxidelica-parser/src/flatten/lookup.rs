//! Finding what a name means: the walk out of the enclosing packages,
//! the imports in force where it was written, and the bases a member
//! may be inherited from.
//!
//! What is found is remembered for as long as one registry stands,
//! since the same question is asked thousands of times over.
//!
//! Carved out of `names` unchanged.

use super::*;
use std::cell::{Cell, RefCell};

/// What a name written inside a package was brought in as by that
/// package, or by a package holding it.
///
/// An import is written once where it reads well - at the top of a
/// library - and holds for everything written inside, which is how the
/// flux tubes name the magnetic constant `mu_0` throughout without
/// ever importing it again.
pub(super) fn enclosing_import(
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
pub(super) fn plain_lookup<'a>(
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
    COUNTED.with(|counted| {
        let (asked, walked) = counted.get();
        let walked_out =
            found.is_some_and(|class| !class.name.starts_with(scope) && !scope.is_empty());
        counted.set((asked + 1, walked + u64::from(walked_out)));
    });
    // What a name was asked as, where it was asked from, and what it
    // came to. A refusal that names a class of the base rather than
    // of the medium at hand is the whole of the media wall, and this
    // is what says which of the two a name landed on.
    ASKED.with(|held| {
        if let Some(trail) = held.borrow_mut().as_mut() {
            trail.push((
                name.to_string(),
                scope.to_string(),
                found.map(|class| class.name.clone()),
            ));
        }
    });
    found
}

thread_local! {
    /// The names asked for while a trail is being kept, each with the
    /// scope it was asked from and the class it landed on.
    ///
    /// Nothing is kept unless something asked for a trail: a library
    /// check asks for a name a hundred thousand times, and a list of
    /// those is not what a person wants unless they said so.
    static ASKED: RefCell<Option<Vec<Asking>>> = const { RefCell::new(None) };
}

/// One asking: the name, the scope it was asked from, and the class
/// it came to.
pub type Asking = (String, String, Option<String>);

/// Keep a trail of the names asked for while this lives, and give it
/// back when it is done.
pub struct Trail;

impl Trail {
    /// Start keeping one, forgetting whatever was being kept.
    ///
    /// A person debugging a media model asks for this from the
    /// outside - `OXIDELICA_NAME_TRAIL` on the library check - and a
    /// test asks for it outright.
    pub fn kept() -> Trail {
        ASKED.with(|held| *held.borrow_mut() = Some(Vec::new()));
        Trail
    }

    /// What has been asked for so far.
    pub fn so_far() -> Vec<Asking> {
        ASKED.with(|held| held.borrow().clone().unwrap_or_default())
    }
}

impl Drop for Trail {
    fn drop(&mut self) {
        ASKED.with(|held| *held.borrow_mut() = None);
    }
}

thread_local! {
    /// How deep the search for a name is into itself.
    static LOOKING: Cell<usize> = const { Cell::new(0) };
    /// How many names have been asked for, and how many of those were
    /// answered by walking out of the scope they were asked from.
    ///
    /// A change to how names are resolved has to leave everything
    /// outside the media alone, and counting is what says so before a
    /// whole library check does: equal counts on the same model mean
    /// the same work was done, whatever the clock happened to say.
    static COUNTED: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

/// How many names were asked for and how many walked out, since the
/// counts were last read.
pub fn counts() -> (u64, u64) {
    COUNTED.with(Cell::get)
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
    if let Some(class) = imports
        .iter()
        .filter(|(local, _)| local == WILDCARD_IMPORT)
        .find_map(|(_, target)| registry.get(format!("{target}.{name}").as_str()).copied())
    {
        return Some(class);
    }
    // A name whose head is a *component* rather than a package:
    // `world.gravityAcceleration`, where `world` is an instance and
    // the function is a member of the class it was declared from.
    // Everything above looks for a class called `world` and there is
    // none, so the name fell through and the run met a call it could
    // not place. It is tried last of all, after every reading that
    // does not need the enclosing class's components, so nothing that
    // resolved before resolves differently now.
    //
    // On by default now, as the last link of the chain it opened. Read
    // alone it cost three models - MultiBody's constraint examples,
    // which stopped one step later at `standardGravityAcceleration is
    // missing its argument gravityType` - and reaching the function
    // is precisely what let them reach that wall and the two behind
    // it. The chain was walked to its end before any of it was turned
    // on, which is why the numbers here are the chain's and not this
    // reading's. `OXIDELICA_COMPONENT_MEMBER` is kept as the
    // instrument that measured it, turned around: it now says which
    // side of the change a number came from by taking the reading
    // away.
    if component_member_refused() {
        return None;
    }
    member_of_a_component(registry, name, scope, 0)
}

/// Whether the reading through a component is held back here.
///
/// The reading is in force by default; this is what takes it away
/// again, so that one binary can print the numbers from either side
/// of the change. The environment answers for a whole corpus run,
/// which is what a measurement wants; a thread answers for one test,
/// which is what a test binary wants. Setting the environment from a
/// test would be the race this project has already paid for once: the
/// variable belongs to the process, and every other test compiling
/// beside it would read the setting as its own.
fn component_member_refused() -> bool {
    HERE.with(Cell::get) || std::env::var_os("OXIDELICA_COMPONENT_MEMBER_OFF").is_some()
}

thread_local! {
    /// Whether this thread asked for the reading through a component
    /// to be held back.
    static HERE: Cell<bool> = const { Cell::new(false) };
}

/// Hold the reading through components back on this thread until the
/// guard is dropped.
pub fn hold_back_components_here() -> ComponentMemberGuard {
    HERE.with(|here| here.set(true));
    ComponentMemberGuard(())
}

/// Puts the reading back where it was.
pub struct ComponentMemberGuard(());

impl Drop for ComponentMemberGuard {
    fn drop(&mut self) {
        HERE.with(|here| here.set(false));
    }
}

/// A member reached through a component of the class asking for it.
///
/// `world.gravityAcceleration` inside a body is not a package member:
/// `world` is a component, and what stands after the dot is a member
/// of the class the component was declared from. The language reaches
/// class members through an instance this way, and it is how the
/// MultiBody library names its gravity field.
///
/// The head is looked for among the components of the class the name
/// was written in and of everything that class extends, since a body
/// may call through an instance a base declared. What is reached is
/// only ever a class - a function or a package - so this cannot
/// answer a question about a variable.
fn member_of_a_component<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    name: &str,
    scope: &str,
    depth: usize,
) -> Option<&'a ClassDef> {
    if depth > MAX_DEPTH {
        return None;
    }
    let (head, member) = name.split_once('.')?;
    let holder = registry.get(scope)?;
    let of_the_component = |component: &Component| -> Option<&'a ClassDef> {
        if component.name != head {
            return None;
        }
        let declared = plain_lookup(registry, &component.type_name, scope)?;
        // Asked under the component's own class, so a member written
        // in a base of it is reached the same way it would be from
        // inside: `lookup` rather than a bare registry hit.
        lookup(registry, member, &declared.name, &declared.imports)
    };
    if let Some(found) = holder.components.iter().find_map(of_the_component) {
        return Some(found);
    }
    holder.extends.iter().find_map(|extend| {
        let base = plain_lookup(registry, &extend.base, scope)?;
        member_of_a_component(registry, name, &base.name, depth + 1)
    })
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
    pub(super) static REGISTRY_STANDS: Cell<bool> = const { Cell::new(false) };
}

thread_local! {
    /// Which classes reach which others by extending them, for as
    /// long as one registry stands. Asked once per pair, it costs the
    /// models that never ask nothing at all.
    static KINDRED: RefCell<HashMap<(String, String), bool>> = RefCell::new(HashMap::new());
}

/// What was found about kinship before, or what is found now.
pub(super) fn kindred_remembers(under: &str, wrote: &str, ask: impl FnOnce() -> bool) -> bool {
    let key = (under.to_string(), wrote.to_string());
    if let Some(told) = KINDRED.with(|kindred| kindred.borrow().get(&key).copied()) {
        return told;
    }
    let found = ask();
    KINDRED.with(|kindred| kindred.borrow_mut().insert(key, found));
    found
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
        KINDRED.with(|kindred| kindred.borrow_mut().clear());
        super::constants::NAMED.with(|named| named.borrow_mut().clear());
        super::constants::MINTED.with(|minted| minted.borrow_mut().clear());
        super::constants::ANY_MINTED.with(|any| any.set(false));
        super::constants::REFUSED_MINT.with(|held| held.borrow_mut().clear());
        REGISTRY_STANDS.with(|stands| stands.set(true));
        StandingNames
    }
}

impl Drop for StandingNames {
    fn drop(&mut self) {
        REGISTRY_STANDS.with(|stands| stands.set(false));
        WALKED.with(|walked| walked.borrow_mut().clear());
        super::constants::NAMED.with(|named| named.borrow_mut().clear());
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
