//! Walking the class tree: every component becomes variables,
//! equations and connections under its instance path.

use super::*;

/// Instantiate `class` under `prefix` with everything `env` carries.
pub(super) fn instantiate(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    env: &Env,
    acc: &mut Flat,
    depth: usize,
) -> Result<(), String> {
    // What a body comes to is remembered for as long as one class is
    // being instantiated: the parameter values a body folds with are
    // this class's, and they do not move while it is built.
    let _remembering = inlining::Inlined::open();
    if depth > MAX_DEPTH {
        return Err(format!(
            "instantiation deeper than {MAX_DEPTH} levels at `{}` (recursive classes?)",
            class.name
        ));
    }

    let scope = class.name.as_str();
    // A function called in an equation makes its checks every step of
    // the run, and those checks belong to the model. The pass that
    // inlines the call sets them aside; they are taken up here, once
    // this class is done, since here is where there is somewhere to
    // put them.
    let checks_from = checks_mark();
    // Every equation this class puts in is stamped with the instance it
    // belongs to; a class instantiated inside it stamps its own, and
    // this one is put back afterwards.
    let stamped = std::mem::replace(&mut acc.origin, prefix.trim_end_matches('.').to_string());
    // Which instances are classes in their own right. A record is not
    // one - its fields belong to whoever holds it - and the count that
    // asks whether a class balances needs to tell the two apart.
    if class.kind.is_model() {
        acc.instances.insert(acc.origin.clone(), class.name.clone());
    }

    // `inner` declarations of this class and of its bases own the
    // instances that `outer` declarations inside it refer to. They are
    // collected before anything is instantiated, so a base class may
    // refer to an `inner` of the class extending it and the other way
    // round.
    let mut inners = env.inners.clone();
    collect_inners(registry, class, prefix, &mut inners, 0);
    let outers = bind_outers(registry, class, &inners)?;

    // What names mean in this class, before anything is built with
    // them: the redeclarations that reach it, the imports they leave
    // in force, and the text of its `String` parameters.
    let Naming {
        redeclares,
        imports,
        shadow,
        local_texts,
    } = settle_naming(registry, class, prefix, env, acc, &outers)?;

    if class.components.iter().any(|c| c.scope == Scope::Inner) {
        settle_the_inner_instances(registry, &inners, acc, &imports, &shadow);
    }

    settle_parameters_early(
        registry, class, prefix, env, acc, &imports, &shadow, &outers,
    );

    instantiate_bases(
        registry,
        class,
        prefix,
        env,
        acc,
        depth,
        &imports,
        &shadow,
        &outers,
        &inners,
        &redeclares,
    )?;

    // A selective `extends` leaves out named elements of this class:
    // `break f` drops the component and its connections, `break
    // connect(a, b)` drops that one connection. Every break must match
    // something, so what did is tracked and checked at the end.
    let broken = env.broken;
    let broke_something = vec![false; broken.len()];
    let component_broken = |name: &str, hit: &mut [bool]| -> bool {
        let mut out = false;
        for (index, item) in broken.iter().enumerate() {
            if matches!(item, Deselect::Component(f) if f == name) {
                hit[index] = true;
                out = true;
            }
        }
        out
    };

    // What this class's parameters are worth, settled to a fixed
    // point and joined by what the model settled before it.
    // A base class's parameters are this class's too, and a dimension
    // may be written on one of them: `extends TwoPlug` brings `m`, and
    // `parameter Voltage V[m]` is written with it.
    let inherited = inherited_parameters(registry, class, 0);
    let local_consts = settle_parameters(
        registry,
        class,
        prefix,
        env,
        acc,
        &imports,
        &shadow,
        &outers,
        &inherited,
        &local_texts,
    );

    let (sizes, sizes_here) = measure_shapes(registry, class, prefix, env, acc, &local_consts);

    // How much of the growing list of measured arrays has been taken
    // into the table above. Each declaration brings its own, and the
    // one after it may be written with them.
    let taken = 0;
    // The same, for the numbers each declaration turns out to be worth.
    let counted = 0;
    // Which of this class's components are records, and of what: an
    // overloaded operator is chosen by the record its operands are of.
    // What a record-valued variable was given as its value, kept until
    // the array layer is ready to say it: the name, the value, and
    // whether it came already written in this class's terms.
    let record_values: Vec<(String, Expr, bool)> = Vec::new();
    let mut records_here: HashMap<String, String> = HashMap::new();
    collect_records(
        registry,
        class,
        prefix,
        scope,
        &imports,
        &mut records_here,
        0,
    );
    // A modifier arrives written in the terms of the class that
    // supplied it - `Shape s(R = mine)` names a record of the class
    // holding `s`, not of `Shape` - so what that class knows to be a
    // record has to be still in view here. Every class adds what it
    // knows as it is built, and the paths are full ones, so nothing
    // collides.
    acc.records.extend(
        records_here
            .iter()
            .map(|(path, of)| (path.clone(), of.clone())),
    );
    // The same view, for the values handed down to parameter records
    // while this class's components are built. Those are written in
    // the terms of the class that supplied them, so what it knows to
    // be a record has to be in view here too.
    let records_wider_for_fields: HashMap<String, String> = acc
        .records
        .iter()
        .map(|(path, of)| (path.clone(), of.clone()))
        .collect();

    // Every component of this class, in the order it was written.
    let Built {
        mut taken,
        mut counted,
        record_values,
        mut sizes,
        mut sizes_here,
        mut local_consts,
        mut broke_something,
    } = components::instantiate_components(
        registry,
        class,
        prefix,
        env,
        acc,
        depth,
        &imports,
        &shadow,
        &outers,
        &inners,
        &local_texts,
        &inherited,
        &records_wider_for_fields,
        &records_here,
        &redeclares,
        &component_broken,
        Built {
            taken,
            counted,
            record_values,
            sizes,
            sizes_here,
            local_consts,
            broke_something,
        },
    )?;

    // The class's own declarations are in by now, and so are the
    // declarations of everything they hold: an equation may name an
    // array that belongs to a component rather than to this class.
    while taken < acc.sizes.len() {
        let (name, shape) = &acc.sizes[taken];
        if name.starts_with(prefix) {
            sizes_here.insert(name.clone(), shape.clone());
        }
        taken += 1;
    }
    // A `:` length is settled only once the component has a value to
    // read it from, and the answer is filed under the component's full
    // path. What a statement of this class writes is the name this
    // class gave the component, so the same lengths go in again under
    // those names - and the full paths stay, for a statement naming
    // what a component below holds. The shorter names go in second, so
    // a length a value settled wins over one the declaration guessed
    // at.
    sizes.extend(sizes_here.clone());
    for (path, shape) in &sizes_here {
        if let Some(local) = path.strip_prefix(prefix) {
            sizes.insert(local.to_string(), shape.clone());
        }
    }

    // The last declarations' numbers, which the loop above added after
    // its final round.
    while counted < acc.numbers.len() {
        let (name, value) = &acc.numbers[counted];
        local_consts.entry(name.clone()).or_insert(*value);
        counted += 1;
    }

    // Everything the class states outright: its equations, the
    // branches and `when` clauses among them, its connections, and the
    // algorithm sections in between.
    equations::flatten_equations(
        registry,
        class,
        prefix,
        env,
        acc,
        depth,
        &imports,
        &shadow,
        &outers,
        &sizes,
        &sizes_here,
        &local_consts,
        &records_here,
        &record_values,
        &mut broke_something,
    )?;

    // A break that matched nothing is a mistake in the extending class.
    if let Some(index) = broke_something.iter().position(|hit| !hit) {
        return Err(match &broken[index] {
            Deselect::Component(name) => {
                format!("`break {name}` matches nothing in `{}`", class.name)
            }
            Deselect::Connection(a, b) => {
                format!(
                    "`break connect({a}, {b})` matches no connection in `{}`",
                    class.name
                )
            }
        });
    }
    // A check written inside a function body may hold arrays and calls
    // of its own, and working one out can leave another behind, so the
    // taking goes round until nothing is left. What comes out already
    // carries this class's prefix, since the call did.
    let no_loop_vars = HashMap::new();
    loop {
        let taken = checks_taken(checks_from);
        if taken.is_empty() {
            break;
        }
        let shapes = Shapes {
            sizes: &sizes_here,
            loop_vars: &no_loop_vars,
            consts: &local_consts,
            records: &records_here,
        };
        for (condition, message) in taken {
            let condition = expand(&condition, &shapes, registry, scope, &imports, 0)?.scalar()?;
            let check = (condition, message);
            // The same call may be worked out more than once - a
            // parameter settled in one pass and read again in the next
            // - and one check written twice is still one check.
            if !acc.asserts.contains(&check) {
                acc.asserts.push(check);
            }
        }
    }
    acc.origin = stamped;
    Ok(())
}

/// What names mean inside one class: the redeclarations that reach it,
/// the imports in force once they are applied, the component names a
/// wildcard import may not stand in for, and what its `String`
/// parameters are worth.
///
/// This is the opening of `instantiate` moved out whole, so that the
/// walk starts at the walk.
pub(super) struct Naming<'a> {
    pub(super) redeclares: Vec<Redeclare>,
    pub(super) imports: Vec<(String, String)>,
    pub(super) shadow: Vec<&'a str>,
    pub(super) local_texts: HashMap<String, String>,
}

fn settle_naming<'a>(
    registry: &HashMap<&str, &ClassDef>,
    class: &'a ClassDef,
    prefix: &str,
    env: &Env,
    acc: &mut Flat,
    outers: &HashMap<String, String>,
) -> Result<Naming<'a>, String> {
    let scope = class.name.as_str();
    // Redeclarations that reach this class: those written here as
    // `redeclare Type name;`, then the ones handed down.
    let mut redeclares = Vec::new();
    for component in class.components.iter().filter(|c| c.redeclaration) {
        redeclares.push(qualify_redeclare(
            &Redeclare {
                name: component.name.clone(),
                type_name: component.type_name.clone(),
                modifiers: component.modifiers.clone(),
                class_level: false,
            },
            registry,
            class,
            prefix,
            outers,
            &[],
        )?);
    }
    redeclares.extend(env.redeclares.iter().cloned());

    // Body-level class redeclarations replace aliases of the bases.
    for alias in class.class_aliases.iter().filter(|a| a.redeclaration) {
        let target = lookup(registry, &alias.target, scope, &class.imports)
            .ok_or_else(|| {
                format!(
                    "unknown class `{}` in the redeclaration of `{}`",
                    alias.target, alias.name
                )
            })?
            .name
            .clone();
        redeclares.push(Redeclare {
            name: alias.name.clone(),
            type_name: target,
            // What the redeclaration wrote on its target travels with
            // it: `redeclare function f = g(a = 1)` fills in one of
            // `g`'s inputs and leaves the rest to the call.
            modifiers: alias.modifiers.clone(),
            class_level: true,
        });
    }

    // The class's own aliases join its imports, with any redeclarations
    // from outside already applied.
    let imports = effective_imports(registry, class, scope, &redeclares, 0)?;
    // What a component's class keeps to itself is nobody else's to
    // read. This says nothing about the flat model, so it is asked
    // once per class rather than once per instance.
    if acc.restrictions_checked.insert(class.name.clone()) {
        restrictions::nothing_reaches_inside(registry, class, &imports)?;
        restrictions::no_class_kept_back_is_named_from_outside(registry, class, &imports)?;
    }
    // Names a wildcard-imported constant may not quietly stand in for:
    // a component of this class outranks anything `import A.*;` opened.
    let shadow: Vec<&str> = class
        .components
        .iter()
        .map(|component| component.name.as_str())
        .collect();

    // What the `String` parameters of this class are worth. A
    // condition may be written on one - `Star star if
    // terminalConnection <> "D"` is how a machine is wired - and
    // strings are otherwise settled at the very end of flattening,
    // long after a condition has to be decided.
    let mut local_texts: HashMap<String, String> = HashMap::new();
    for _ in 0..MAX_DEPTH {
        let mut progress = false;
        for component in class.components.iter().chain(
            inherited_parameters(registry, class, 0)
                .iter()
                .map(|(c, _)| c),
        ) {
            if component.type_name != "String" || local_texts.contains_key(&component.name) {
                continue;
            }
            let said = env
                .overrides
                .iter()
                .find(|(name, _)| name == &component.name)
                .map(|(_, value)| value.clone())
                .or_else(|| component.binding.clone())
                .or_else(|| component.start.clone());
            let Some(said) = said else { continue };
            let said = substitute_class_constants(&said, registry, scope, &imports, &shadow);
            if let Some(text) = strings::text_of(&said, &local_texts, &HashMap::new()) {
                local_texts.insert(component.name.clone(), text.clone());
                local_texts.insert(format!("{prefix}{}", component.name), text);
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }

    Ok(Naming {
        redeclares,
        imports,
        shadow,
        local_texts,
    })
}

fn settle_the_inner_instances(
    registry: &HashMap<&str, &ClassDef>,
    inners: &HashMap<String, InnerInstance>,
    acc: &mut Flat,
    imports: &[(String, String)],
    shadow: &[&str],
) {
    let mut named: Vec<(&String, &InnerInstance)> = inners.iter().collect();
    named.sort_by(|a, b| a.1.path.cmp(&b.1.path));
    for (_, instance) in named {
        // What this pass has settled here, under the short names the
        // class writes. A shared instance says `massDynamics =
        // energyDynamics` about itself, one parameter reading its
        // neighbour, and the neighbour's value goes into the model's
        // table under a path - never under the bare name the binding
        // asks for. Kept beside, the chain settles in the order it is
        // written, and nothing is copied to hold it.
        let mut settled_here: HashMap<String, f64> = HashMap::new();
        let Some(shared) = registry.get(instance.class.as_str()) else {
            continue;
        };
        for component in &shared.components {
            if !matches!(
                component.variability,
                Variability::Parameter | Variability::Constant
            ) || !component.dimensions.is_empty()
            {
                continue;
            }
            let written = instance
                .modifiers
                .iter()
                .find(|(name, _)| name == &component.name)
                .map(|(_, value)| value.clone())
                .or_else(|| component.binding.clone())
                .or_else(|| component.start.clone());
            let Some(written) = written else { continue };
            let written =
                substitute_class_constants(&written, registry, &shared.name, imports, shadow);
            let Some(value) = const_eval(&written, &acc.const_values)
                .or_else(|| const_eval(&written, &settled_here))
            else {
                continue;
            };
            settled_here.insert(component.name.clone(), value);
            acc.const_values
                .insert(format!("{}.{}", instance.path, component.name), value);
        }
    }
}

/// What this class's own parameters are worth before its bases are
/// built, for the bases that are given values written down here.
///
/// Moved out of `instantiate` unchanged.
#[allow(clippy::too_many_arguments)]
fn settle_parameters_early(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    env: &Env,
    acc: &mut Flat,
    imports: &[(String, String)],
    shadow: &[&str],
    outers: &HashMap<String, String>,
) {
    let scope = class.name.as_str();
    // A base's parameter may be given a value this class declares:
    // `extends MIMO(final nin = m)` with `m` written below the
    // `extends`, which is how the standard library gives a block as
    // many inputs as it has phases. The bases are instantiated first,
    // so what this class says about itself has to be worth something
    // before that happens. Only what folds on its own is settled here;
    // anything that needs a base's value waits for the round below.
    let mut settled_early: Vec<String> = Vec::new();
    loop {
        let mut progress = false;
        for component in &class.components {
            let named = format!("{prefix}{}", component.name);
            if !matches!(
                component.variability,
                Variability::Parameter | Variability::Constant
            ) || settled_early.contains(&named)
            {
                continue;
            }
            let binding = env
                .overrides
                .iter()
                .find(|(n, _)| n == &component.name)
                .map(|(_, e)| e.clone())
                .or_else(|| {
                    // A parameter given no value but a `start` takes
                    // the `start`: that is what a parameter's start
                    // is for. The machine library switches a thermal
                    // port on with `parameter Boolean
                    // useDamperCage(start=true)` and nothing else,
                    // and a component's condition reading that name
                    // has to find a value behind it.
                    component
                        .binding
                        .as_ref()
                        .or(component.start.as_ref())
                        .map(|e| {
                            let e = substitute_class_constants(e, registry, scope, imports, shadow);
                            prefix_expr(&e, prefix, outers)
                        })
                });
            let Some(binding) = binding else { continue };
            // What has settled is already in the model's table, so the
            // table is what the next one is read against - copying it
            // per declaration would cost more than the whole round.
            if let Some(value) = const_eval(&binding, &acc.const_values) {
                settled_early.push(named.clone());
                acc.const_values.insert(named.clone(), value);
                acc.numbers.push((named, value));
                progress = true;
            }
        }
        if !progress {
            break;
        }
    }
}

/// The classes this one extends, merged in under the same prefix
/// before anything it declares itself.
///
/// Moved out of `instantiate` unchanged.
#[allow(clippy::too_many_arguments)]
fn instantiate_bases(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    env: &Env,
    acc: &mut Flat,
    depth: usize,
    imports: &[(String, String)],
    shadow: &[&str],
    outers: &HashMap<String, String>,
    inners: &HashMap<String, InnerInstance>,
    redeclares: &[Redeclare],
) -> Result<(), String> {
    let scope = class.name.as_str();
    // Bases first, with their modifiers (already parent-scoped).
    for extend in &class.extends {
        let base = match extend.from_base {
            true => inherited_class(registry, class, &extend.base, 0).ok_or_else(|| {
                format!(
                    "`{}` redeclares `{}` by extending it, and no base of `{}` declares one",
                    class.name,
                    extend.base,
                    class.name.rsplit_once('.').map_or("", |(head, _)| head)
                )
            })?,
            false => lookup(registry, &extend.base, scope, imports)
                .ok_or_else(|| format!("unknown base class `{}`", extend.base))?,
        };
        // A class reached by more than one path of a diamond is one
        // class: `Shape` extends both the animation shape and the
        // partial shape that one is built on, and what they share is
        // shared rather than doubled. Merging it twice would give the
        // instance two of every variable its base declares and two of
        // every equation.
        if !acc.extended.insert((prefix.to_string(), base.name.clone())) {
            continue;
        }
        let mut base_redeclares = Vec::new();
        for redeclare in &extend.redeclares {
            base_redeclares.push(qualify_redeclare(
                redeclare,
                registry,
                class,
                prefix,
                outers,
                &[],
            )?);
        }
        base_redeclares.extend(redeclares.iter().cloned());
        // What the values handed down name, measured here where those
        // names still mean something.
        let mut handed_shapes: HashMap<String, Vec<i64>> = HashMap::new();
        // A length written as a parameter of this very class - `Real
        // T_ref[m]` beside `parameter Integer m` - is a short name,
        // while the table of settled numbers knows it by the full
        // path it has in the model. The numbers of this class are
        // offered under the names its own declarations use, so a
        // length like that can be measured at all.
        let here: HashMap<String, f64> = acc
            .const_values
            .iter()
            .filter_map(|(named, value)| {
                let short = named.strip_prefix(prefix)?;
                match short.contains('.') {
                    true => None,
                    false => Some((short.to_string(), *value)),
                }
            })
            .collect();
        collect_shapes_given(
            registry,
            class,
            &here,
            &HashMap::new(),
            env.overrides,
            &mut handed_shapes,
            0,
        );
        let handed_shapes = prefixed_sizes(&handed_shapes, prefix);
        // A value handed to a base is written where this class stands,
        // and may ask how long an array of this class is: a table
        // block says `extends MO(final nout = size(columns, 1))` about
        // itself. The base is instantiated next and has never heard of
        // `columns`, so the question is answered here, where the
        // shapes were just measured - which is where the language says
        // a modifier is worked out anyway.
        // What the site wrote outranks what this class's own `extends`
        // says: a record declared `smpmData(useDamperCage = false)`
        // means false, whatever the base it extends says about that
        // field. Both end up in this list and the reader takes the
        // first of the two, so the site's go first.
        let mods: Vec<(String, Expr)> = env
            .overrides
            .iter()
            .cloned()
            .chain(extend.modifiers.iter().map(|(n, e)| {
                let e = substitute_class_constants(e, registry, scope, imports, shadow);
                let e = prefix_expr(&e, prefix, outers);
                (n.clone(), measured_sizes(&e, &handed_shapes, &here))
            }))
            .collect();
        let base_env = Env {
            overrides: &mods,
            handed_shapes: &handed_shapes,
            outer_sizes: env.outer_sizes,
            redeclares: &base_redeclares,
            inners,
            broken: &extend.broken,
            inside_a_parameter: env.inside_a_parameter,
        };
        instantiate(registry, base, prefix, &base_env, acc, depth + 1)?;
    }

    Ok(())
}

/// What every parameter in view is worth, run to a fixed point: this
/// class's own declarations and the ones its bases hand down, then the
/// same values under the instance path and everything the model has
/// settled so far.
///
/// Moved out of `instantiate` unchanged.
#[allow(clippy::too_many_arguments)]
/// Every `String` name of an expression put back as the text it
/// stands for.
///
/// A parameter worked out by a function may be handed one - a table
/// block asks `findLast(fileName, ".csv")` what kind of file it was
/// given - and a name is nothing a body can measure. What each string
/// is worth is known where the class is instantiated, so it is put in
/// before the call is inlined.
pub(super) fn substitute_texts(expr: &Expr, texts: &HashMap<String, String>) -> Expr {
    if let Expr::Ref(name) = expr {
        if let Some(text) = texts.get(name) {
            return Expr::Str(text.clone());
        }
    }
    expr.map_children(&mut |child| substitute_texts(child, texts))
}

/// Settle the fields of a parameter record, by name.
///
/// `parameter SmpmData smpmData` holds `useDamperCage`, and a machine
/// beside it is written `smpm(useDamperCage = smpmData.useDamperCage)`,
/// one of a dozen fields an example hands over that way. Reached in
/// declaration order the record may come after what reads it, and
/// then the name is live but looks dead: the condition it decides has
/// nothing to decide by, and the same model with the two declarations
/// swapped works. So the fields are settled in the parameter round,
/// which turns until nothing moves.
///
/// What the model wrote on the declaration beats what the record
/// declares, the way a modifier always does. Arrays of records are
/// left alone - each element would need its own name - and a field
/// that is itself a record is settled the same way, one level at a
/// time as the rounds come.
#[allow(clippy::too_many_arguments)]
fn fields_of_a_record(
    component: &Component,
    from_extends: Option<&Expr>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    prefix: &str,
    outers: &HashMap<String, String>,
    env: &mut HashMap<String, f64>,
    local_consts: &mut HashMap<String, f64>,
    acc: &mut Flat,
) -> bool {
    if !component.dimensions.is_empty() || from_extends.is_some() {
        return false;
    }
    let Some(record) = lookup(registry, &component.type_name, scope, imports) else {
        return false;
    };
    if record.kind != ClassKind::Record {
        return false;
    }
    // And the fields it inherits: the machine records of the library
    // are three deep - `SM_PermanentMagnetData` extends
    // `SM_ReluctanceRotorData` extends `InductionMachineData` - and
    // `useDamperCage` is declared at the middle one. Read from the
    // record's own components alone, such a field is never offered
    // here at all, and everything written on it waits for ever.
    let held = super::inlining::with_inherited_components(record, registry);
    let mut moved = false;
    for field in &held {
        if !matches!(
            field.variability,
            Variability::Parameter | Variability::Constant
        ) {
            continue;
        }
        let named = format!("{}.{}", component.name, field.name);
        if local_consts.contains_key(&named) {
            continue;
        }
        // What the model wrote on the declaration, else what the
        // record says of itself.
        let written = component
            .modifiers
            .iter()
            .find(|(name, _)| name == &field.name)
            .map(|(_, value)| value.clone())
            .or_else(|| field.binding.clone())
            .or_else(|| field.start.clone());
        let Some(written) = written else { continue };
        let written = substitute_class_constants(&written, registry, scope, imports, &[]);
        let written = prefix_expr(&written, prefix, outers);
        let Some(value) = const_eval(&written, env) else {
            continue;
        };
        local_consts.insert(named.clone(), value);
        env.insert(named.clone(), value);
        let flat = format!("{prefix}{named}");
        env.insert(flat.clone(), value);
        acc.const_values.insert(flat, value);
        moved = true;
    }
    moved
}

#[allow(clippy::too_many_arguments)]
fn settle_parameters(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    env: &Env,
    acc: &mut Flat,
    imports: &[(String, String)],
    shadow: &[&str],
    outers: &HashMap<String, String>,
    inherited: &[(Component, Option<Expr>)],
    local_texts: &HashMap<String, String>,
) -> HashMap<String, f64> {
    let scope = class.name.as_str();
    // Parameter values of this class, resolved to numbers where
    // possible: array dimensions and loop bounds are compile-time
    // constants and must come from here.
    let mut local_consts: HashMap<String, f64> = HashMap::new();
    // What the `extends` clause said about a base parameter comes with
    // the parameter.
    let env_overrides = env.overrides;
    // What every declaration is read against: the model's numbers and
    // this class's own. The round keeps it up itself - every value it
    // settles goes in here as well as into the two tables it is
    // settling - so it is built once for all the rounds rather than
    // once each. A class with a thousand numbers below it copied them
    // per round for nothing.
    let mut env = acc.const_values.clone();
    env.extend(local_consts.iter().map(|(k, v)| (k.clone(), *v)));
    // What this class's arrays are shaped like, as far as the numbers
    // settled so far can say. A value handed down by an `extends` may
    // ask after one of them, and the round after this sees whatever
    // settled in this one - so it is gathered again only where a round
    // settled something, since nothing else can have changed a shape.
    //
    // Measured rather than assumed: a second round almost never
    // happens, so this saves less than the noise between two runs of
    // the same build. It is kept because the work it removes is work
    // that cannot be needed, not because the clock showed it.
    let mut shapes_here: HashMap<String, Vec<i64>> = HashMap::new();
    let gather = |shapes_here: &mut HashMap<String, Vec<i64>>,
                  local_consts: &HashMap<String, f64>| {
        shapes_here.clear();
        collect_shapes_given(
            registry,
            class,
            local_consts,
            &HashMap::new(),
            env_overrides,
            shapes_here,
            0,
        );
    };
    gather(&mut shapes_here, &local_consts);
    loop {
        let mut progress = false;
        for (component, from_extends) in class
            .components
            .iter()
            .map(|component| (component, None))
            .chain(
                inherited
                    .iter()
                    .map(|(component, value)| (component, value.as_ref())),
            )
        {
            if !matches!(
                component.variability,
                Variability::Parameter | Variability::Constant
            ) {
                continue;
            }
            // A parameter record hands its fields out by name:
            // `smpmData.useDamperCage` is what an example writes on
            // the machine, and the machine passes it down to the
            // condition of a heat port. The fields have to be settled
            // in this round rather than when the record is reached in
            // declaration order, or a record declared after what uses
            // it leaves a live name looking dead - and the same model
            // written the other way round works, which is no way for
            // a compiler to behave.
            if fields_of_a_record(
                component,
                from_extends,
                registry,
                scope,
                imports,
                prefix,
                outers,
                &mut env,
                &mut local_consts,
                acc,
            ) {
                progress = true;
            }
            if local_consts.contains_key(&component.name) {
                continue;
            }
            let binding = env_overrides
                .iter()
                .find(|(n, _)| n == &component.name)
                .map(|(_, e)| e.clone())
                // A value the `extends` handed down may ask how long
                // an array of this class is - `extends MO(final n =
                // size(cols, 1))` - and the shapes measured so far are
                // what answers it. The base was handed the same value
                // already folded; this is the same parameter seen from
                // the class that wrote it.
                .or_else(|| from_extends.map(|e| measured_sizes(e, &shapes_here, &local_consts)))
                .or_else(|| {
                    component
                        .binding
                        .as_ref()
                        .or(component.start.as_ref())
                        .map(|e| {
                            let e = substitute_class_constants(e, registry, scope, imports, shadow);
                            prefix_expr(&e, prefix, outers)
                        })
                });
            let Some(expr) = binding else { continue };
            // A parameter may be worked out by a function - the
            // standard library counts the base systems of an m-phase
            // winding that way - and a call is not something arithmetic
            // alone can fold, so the call is inlined first. Anything
            // the inlining will not do leaves the parameter for a
            // later round, or for no round at all.
            // A call may be handed a `String` parameter of this class
            // - `findLast(fileName, ".csv")` is how a table block
            // decides what kind of file it was given - and a name is
            // not something the body can measure. What each string is
            // worth is known here, so it is put in before the call is
            // inlined.
            let expr = substitute_texts(&expr, local_texts);
            let settled = const_eval(&expr, &env).or_else(|| {
                let inlined = resolve(
                    &expr,
                    &HashMap::new(),
                    &env,
                    &HashMap::new(),
                    registry,
                    scope,
                    imports,
                    0,
                )
                .ok()?;
                // What the body worked out of the strings it was
                // handed folds to a number here, the way a condition
                // written on strings does.
                const_eval(&inlined, &env).or_else(|| {
                    let folded = strings::fold(&inlined, local_texts, &env).ok()?;
                    const_eval(&folded, &env)
                })
            });
            if let Some(value) = settled {
                local_consts.insert(component.name.clone(), value);
                inlining::Inlined::forget();
                env.insert(component.name.clone(), value);
                let named = format!("{prefix}{}", component.name);
                env.insert(named.clone(), value);
                acc.const_values.insert(named, value);
                progress = true;
            }
        }
        if !progress {
            break;
        }
        // A value settled in this round may be a length in the next.
        gather(&mut shapes_here, &local_consts);
    }

    // The same values under the instance path. A declaration's own
    // value is written in the terms of this class and then prefixed -
    // `fill(1, m)` becomes `fill(1, b.m)` - so whatever asks what it
    // comes to has to find the parameter under either name.
    if !prefix.is_empty() {
        for (name, value) in local_consts.clone() {
            local_consts.insert(format!("{prefix}{name}"), value);
            inlining::Inlined::forget();
        }
    }
    // And every parameter the model has settled so far, by its full
    // path. Two things need them. A base of this class may have
    // settled one before another base was reached - `extends
    // ConditionalHeatPort(T = fill(293.15, m))` is written in a class
    // whose `m` comes from a base of its own. And a modifier handed
    // down is written in the terms of the class that wrote it, so a
    // child asked to make sense of `1:drawn.n` has to know what
    // `drawn.n` is, and `drawn` is not below it but above. The names
    // are full paths, so nothing here can be mistaken for anything
    // else; what this class says itself still wins.
    for (name, value) in &acc.const_values {
        if !local_consts.contains_key(name) {
            local_consts.insert(name.clone(), *value);
            inlining::Inlined::forget();
        }
    }

    local_consts
}

/// What every array of this class is shaped like, as far as the
/// settled numbers can say - by the class's own names and by the
/// instance path - with this class's constant arrays offered to the
/// model for its components to read.
///
/// Moved out of `instantiate` unchanged.
fn measure_shapes(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    env: &Env,
    acc: &mut Flat,
    local_consts: &HashMap<String, f64>,
) -> (HashMap<String, Vec<i64>>, HashMap<String, Vec<i64>>) {
    // What each array component of this class - and of its bases - is
    // shaped like, so a value may name one as a whole.
    let mut sizes: HashMap<String, Vec<i64>> = HashMap::new();
    collect_shapes_given(
        registry,
        class,
        local_consts,
        &HashMap::new(),
        env.overrides,
        &mut sizes,
        0,
    );
    let sizes_here = prefixed_sizes(&sizes, prefix);
    // A constant array of this class is measured here and never
    // reaches the loop below, which builds the elements of what the
    // model holds: a constant has no elements to build. But a value
    // handed to a component may name one - a table block is given
    // `table = Lin`, one of three curves the class keeps - and the
    // component is instantiated with the shapes gathered so far.
    //
    // Only the constants are offered, and only those nothing else has
    // measured. Offering every shape this class knows put entries in
    // front of the ones the components measure for themselves, and
    // seventeen models were refused for it.
    let constants_here: Vec<String> = class
        .components
        .iter()
        .filter(|c| c.variability == Variability::Constant && !c.dimensions.is_empty())
        .map(|c| format!("{prefix}{}", c.name))
        .collect();
    for name in constants_here {
        if acc.sizes.iter().any(|(known, _)| known == &name) {
            continue;
        }
        if let Some(shape) = sizes_here.get(&name) {
            acc.sizes.push((name, shape.clone()));
        }
    }

    (sizes, sizes_here)
}

/// What building this class's components reads and adds to: the
/// lengths and numbers taken up so far, what the declarations settled,
/// and which `break` of a selective `extends` found something to drop.
///
/// One struct rather than eight arguments, because the loop carries
/// all eight along together and hands them all back.
pub(super) struct Built {
    /// How much of the model's list of measured arrays has been read.
    pub(super) taken: usize,
    /// The same for its list of settled numbers.
    pub(super) counted: usize,
    /// Record-valued declarations, kept until the array layer can say
    /// them: the name, the value, and whether it is already prefixed.
    pub(super) record_values: Vec<(String, Expr, bool)>,
    /// Array lengths by the class's own names.
    pub(super) sizes: HashMap<String, Vec<i64>>,
    /// The same by the instance path.
    pub(super) sizes_here: HashMap<String, Vec<i64>>,
    /// Parameter values in view.
    pub(super) local_consts: HashMap<String, f64>,
    /// Which `break` of a selective `extends` matched something.
    pub(super) broke_something: Vec<bool>,
}

/// Record an `if` equation whose condition only the run can decide.
///
/// The spec calls such an `if` balanced: every branch, `else`
/// included, contributes the same number of equations, so the model
/// has one equation per position however the condition falls. What a
/// branch may not do is change the structure - no `connect`, since a
/// connection is drawn once and for all.
pub(super) fn push_conditional<R, E>(
    if_equation: &IfEquation,
    class_name: &str,
    resolve_here: R,
    expand_here: E,
    no_loop_vars: &HashMap<String, f64>,
    acc: &mut Flat,
) -> Result<(), String>
where
    R: Fn(&Expr) -> Result<Expr, String>,
    E: Fn(&Expr, &HashMap<String, f64>) -> Result<Value, String>,
{
    let mut conditions = Vec::new();
    let mut branches: Vec<Vec<EquationItem>> = Vec::new();
    for (position, branch) in if_equation.branches.iter().enumerate() {
        let last = position + 1 == if_equation.branches.len();
        match (&branch.condition, last) {
            (Some(condition), false) => conditions.push(resolve_here(condition)?),
            (None, true) => {}
            (Some(_), true) => {
                return Err(format!(
                    "an `if` equation in `{class_name}` has a condition the compiler cannot \
                     decide and no `else`, so the model would have a different number of \
                     equations depending on it"
                ))
            }
            (None, false) => unreachable!("an else branch is always last"),
        }
        if !branch.connects.is_empty() {
            return Err(format!(
                "a `connect` in `{class_name}` sits in an `if` branch whose condition is not \
                 known at compile time; connections are structural"
            ));
        }
        if !branch.loops.is_empty() {
            return Err(format!(
                "a `for` equation in `{class_name}` sits in an `if` branch whose condition is \
                 not known at compile time; how many equations a loop makes is settled before \
                 the run"
            ));
        }
        if !branch.graph.is_empty() {
            return Err(format!(
                "a `Connections` clause in `{class_name}` sits in an `if` branch whose \
                 condition is not known at compile time; the connection graph is drawn once \
                 and for all"
            ));
        }
        if !branch.calls.is_empty() {
            return Err(format!(
                "a call standing on its own in `{class_name}` sits in an `if` branch whose \
                 condition is not known at compile time; what it is written for is what its \
                 body checks, and a check has nowhere to go from here"
            ));
        }
        if !branch.whens.is_empty() {
            return Err(format!(
                "a `when` in `{class_name}` sits in an `if` branch whose condition is not known \
                 at compile time; what happens at an event is part of the model rather than a \
                 value it works out"
            ));
        }
        let mut scalars = Vec::new();
        for equation in &branch.equations {
            let lhs = expand_here(&equation.lhs, no_loop_vars)?;
            let rhs = expand_here(&equation.rhs, no_loop_vars)?;
            let (mut left, mut right) = (Vec::new(), Vec::new());
            lhs.flatten_into(&mut left);
            rhs.flatten_into(&mut right);
            if left.len() != right.len() {
                return Err(format!(
                    "an equation in `{class_name}` puts {} value(s) against {}",
                    left.len(),
                    right.len()
                ));
            }
            for (lhs, rhs) in left.into_iter().zip(right) {
                scalars.push(EquationItem {
                    lhs,
                    rhs,
                    origin: acc.origin.clone(),
                });
            }
        }
        branches.push(scalars);
    }
    let wanted = branches[0].len();
    if let Some(odd) = branches.iter().position(|branch| branch.len() != wanted) {
        return Err(format!(
            "the branches of an `if` equation in `{class_name}` are not balanced: \
             {wanted} equation(s) in the first, {} in branch {}",
            branches[odd].len(),
            odd + 1
        ));
    }
    // A check written in a branch holds only while that branch is the
    // one taken, and which one that is the run decides. So each check
    // becomes one that always holds and says nothing where the branch
    // is not taken: `not guard or condition`. The guard is the branch's
    // own condition together with the denial of every condition before
    // it, which is what testing a chain in order comes to.
    for (position, branch) in if_equation.branches.iter().enumerate() {
        if branch.asserts.is_empty() {
            continue;
        }
        let mut guard: Option<Expr> = None;
        for earlier in conditions.iter().take(position) {
            let denied = Expr::Not(Box::new(earlier.clone()));
            guard = Some(match guard {
                None => denied,
                Some(built) => Expr::And(Box::new(built), Box::new(denied)),
            });
        }
        if let Some(own) = conditions.get(position) {
            guard = Some(match guard {
                None => own.clone(),
                Some(built) => Expr::And(Box::new(built), Box::new(own.clone())),
            });
        }
        for (condition, message) in &branch.asserts {
            let condition = resolve_here(condition)?;
            let held = match &guard {
                None => condition,
                Some(guard) => Expr::Or(
                    Box::new(Expr::Not(Box::new(guard.clone()))),
                    Box::new(condition),
                ),
            };
            acc.asserts.push((held, message.clone()));
        }
    }
    acc.conditional.push(ConditionalEquations {
        conditions,
        branches,
    });
    Ok(())
}
