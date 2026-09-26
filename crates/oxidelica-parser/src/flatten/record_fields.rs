//! The fields of a record: which of them a value is made of, what
//! each is called once the record is taken apart, and what one holds
//! before anything assigns it.
//!
//! Carved out of `algorithms` unchanged.

use super::*;

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
            let inherited = record_components(registry, base, depth + 1);
            // What the `extends` said about the base's fields is part
            // of what this record declares them as: `record M350_50A
            // extends BaseData(mu_i = 1210, n = 14)` is a record whose
            // `mu_i` is 1210, and reading the base's declaration alone
            // gives the 1 the base wrote as its own default. Every
            // reader of a record's fields goes through here, so the
            // value a constructor call `M350_50A()` leaves to the
            // declaration was the base's default and not the
            // material's - a permeability of one where the sheet
            // states twelve hundred, which divides a reluctance by a
            // permeance that is too small by three orders and hands
            // the solver an infinity.
            //
            // Only a modifier naming a field outright is one of these.
            // `mu_i.start = 3` and `friction.a = 1` say something
            // about a part of a field rather than about the value it
            // takes, and the layers that read those read them
            // elsewhere.
            out.extend(inherited.into_iter().map(|mut field| {
                if let Some((_, value)) = extend
                    .modifiers
                    .iter()
                    .find(|(name, _)| name == &field.name)
                {
                    field.binding = Some(value.clone());
                }
                field
            }));
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
            // A length written as a name is read where the record
            // stands: a medium's state declares `X[nX]`, and `nX` is
            // a constant of the medium. Asked of an empty table it
            // answered nothing, the field dropped out of the list,
            // and a body inlined for `st[1]` read `state.X[1]` with
            // nothing bound to it - a name of the function's own
            // carried out into the flat model, where nobody declares
            // it. The record handed by a plain name was spared only
            // because that road binds the bare name as well.
            let shape: Option<Vec<i64>> = field
                .dimensions
                .iter()
                .map(|d| {
                    const_eval(d, &HashMap::new())
                        .map(|n| n as i64)
                        .or_else(|| match named_lengths_off() {
                            true => None,
                            // Read under the medium on the mark first,
                            // where there is one and it descends from
                            // the package that wrote the record: a
                            // medium may give `nX` a value its base
                            // left open, and the base's reading of the
                            // same name would be a guess.
                            false => {
                                let package = of.name.rsplit_once('.').map(|(head, _)| head);
                                let under = package.and_then(|package| {
                                    super::inlining::asked_as_package(registry, package)
                                });
                                match under.as_deref().and_then(|under| registry.get(under)) {
                                    Some(medium) => {
                                        super::constants::declared_length(d, medium, registry)
                                    }
                                    None => super::constants::declared_length(d, of, registry),
                                }
                            }
                        })
                })
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
        // A field that is text is not one of these. The walk carries
        // numbers, and a medium's record holds its own `name` beside
        // its gas constants - handed over as a number it becomes a
        // name nothing gives a value to, which is how the whole
        // `data` family died one step after the record was read.
        .filter(|field| {
            !matches!(
                started_by(&field.type_name, registry, of, 0),
                Some(Started::Text)
            )
        })
        .map(|field| field.name)
        .collect()
}

/// A record's fields as the flat names a walk can carry: one per
/// number, arrays written out element by element, text left out.
///
/// The one list two roads must agree on. A specialized copy declares
/// an input per name here and the call appends a value per name here,
/// and a body carried out to the walk renames its record fields by
/// position in this same list - so a disagreement between the two is
/// a number read out of the wrong seat, which is worse than a
/// refusal. The NASA gas record is the reason arrays are in it: seven
/// low coefficients and seven high ones, against four scalars.
pub(super) fn handed_record_fields(
    registry: &HashMap<&str, &ClassDef>,
    of: &ClassDef,
) -> Vec<String> {
    let mut out = Vec::new();
    for field in record_components(registry, of, 0) {
        if matches!(
            started_by(&field.type_name, registry, of, 0),
            Some(Started::Text)
        ) {
            continue;
        }
        if field.dimensions.is_empty() {
            out.push(field.name.clone());
            continue;
        }
        let shape: Option<Vec<i64>> = field
            .dimensions
            .iter()
            .map(|d| const_eval(d, &HashMap::new()).map(|n| n as i64))
            .collect();
        let Some(shape) = shape else {
            return Vec::new();
        };
        for indices in super::names::index_tuples(&shape) {
            out.push(super::names::element_name(&field.name, &indices));
        }
    }
    out
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
    // A function body, or the algorithm section of a model. Both
    // decide a local one way or another, and a variable a branch
    // leaves alone holds what it held - which the first time round is
    // its start value. A digital gate writes `y_auxiliary` in one
    // branch of a `when` and reads it outside.
    //
    // In a model only what it keeps to itself: a variable the rest of
    // the model can see is one the equations may be solving for, and
    // a branch that leaves it alone is the model saying nothing about
    // it rather than saying it stays. That is still a mistake worth
    // refusing.
    let a_model = class.kind == ClassKind::Model;
    if !matches!(class.kind, ClassKind::Function) && !a_model {
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
    if a_model && !declared.protected {
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
        for field in &record_components(registry, record?, 0) {
            fields.push(starts_at(
                &format!("{name}.{}", field.name),
                registry,
                scope,
            )?);
        }
        return Some(Expr::Array(fields));
    }
    // What the field's own declaration says it starts at beats the
    // language's default for its type: `SI.Voltage m_vds(start = -2.0)`
    // starts at minus two, and answering zero for it would be a wrong
    // number where the declaration gave a right one.
    if let Some(written) = written_start(name, declared, registry, class) {
        return Some(written);
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

/// Whether `name` - a variable or one element of it - is declared by
/// the model `scope` as one that only changes at events: `discrete`
/// outright, or an `Integer`, a `Boolean` or an enumeration, which are
/// discrete whatever they are called. Not an input, which the section
/// does not own.
pub(super) fn held_between_events(
    name: &str,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
) -> bool {
    let Some(class) = registry.get(scope) else {
        return false;
    };
    if class.kind != ClassKind::Model {
        return false;
    }
    let root = name.split(['[', '.']).next().unwrap_or(name);
    let Some(declared) = class.components.iter().find(|c| c.name == root) else {
        return false;
    };
    if declared.causality == Causality::Input || name[root.len()..].contains('.') {
        return false;
    }
    if declared.variability == Variability::Discrete {
        return true;
    }
    discrete_type(&declared.type_name, registry, class, 0)
}

/// Whether a type is followed through its bases to one that is
/// discrete by nature.
fn discrete_type(
    type_name: &str,
    registry: &HashMap<&str, &ClassDef>,
    within: &ClassDef,
    depth: usize,
) -> bool {
    if depth > 32 {
        return false;
    }
    match type_name {
        "Integer" | "Boolean" => return true,
        "Real" | "String" => return false,
        _ => {}
    }
    let Some(class) = lookup(registry, type_name, &within.name, &within.imports) else {
        return false;
    };
    if !class.enumeration.is_empty() {
        return true;
    }
    let base = match &class.alias_of {
        Some((base, _)) => base.clone(),
        None => match class.extends.first() {
            Some(first) => first.base.clone(),
            None => return false,
        },
    };
    discrete_type(&base, registry, class, depth + 1)
}

/// The `start` a record's own declaration writes on the field `name`
/// names, where there is one. `None` where the field is reached but
/// says nothing about its start, and where it cannot be reached at all.
fn written_start(
    name: &str,
    declared: &Component,
    registry: &HashMap<&str, &ClassDef>,
    within: &ClassDef,
) -> Option<Expr> {
    let mut current = declared.type_name.clone();
    let mut within = within;
    let mut found: Option<Component> = None;
    for field in name.split('.').skip(1) {
        let holding = lookup(registry, &current, &within.name, &within.imports)?;
        let held = record_components(registry, holding, 0)
            .into_iter()
            .find(|c| c.name == field)?;
        current = held.type_name.clone();
        found = Some(held);
        within = holding;
    }
    let held = found?;
    // Written either way round: as an attribute of the declaration, or
    // as the one value a declaration may give outright.
    held.start.clone().or_else(|| held.binding.clone())
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
        // Its bases' fields among its own: the Spice3 records are
        // three `extends` deep and most of what they hold is declared
        // above the name the value was written against, so reading
        // only the record's own declarations loses the field's type
        // and with it the start it was owed.
        current = record_components(registry, holding, 0)
            .into_iter()
            .find(|c| c.name == field)?
            .type_name;
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

/// Whether a record field whose length is written as a name is to be
/// left out of the fields bound one by one, as it was before.
/// `OXIDELICA_NO_NAMED_FIELD_LENGTHS` is kept so that one binary can be
/// measured against itself over the whole library.
fn named_lengths_off() -> bool {
    std::env::var_os("OXIDELICA_NO_NAMED_FIELD_LENGTHS").is_some()
}
