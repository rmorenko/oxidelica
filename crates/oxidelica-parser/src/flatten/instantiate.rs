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
    let _remembering = Inlined::open();
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
    } = instantiate_components(
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
            modifiers: Vec::new(),
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
        let mods: Vec<(String, Expr)> = extend
            .modifiers
            .iter()
            .map(|(n, e)| {
                let e = substitute_class_constants(e, registry, scope, imports, shadow);
                let e = prefix_expr(&e, prefix, outers);
                (n.clone(), measured_sizes(&e, &handed_shapes, &here))
            })
            .chain(env.overrides.iter().cloned())
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
fn substitute_texts(expr: &Expr, texts: &HashMap<String, String>) -> Expr {
    if let Expr::Ref(name) = expr {
        if let Some(text) = texts.get(name) {
            return Expr::Str(text.clone());
        }
    }
    expr.map_children(&mut |child| substitute_texts(child, texts))
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
            ) || local_consts.contains_key(&component.name)
            {
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
                Inlined::forget();
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
            Inlined::forget();
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
            Inlined::forget();
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

/// Every component this class declares, turned into the variables,
/// equations and instances it stands for.
///
/// Moved out of `instantiate` unchanged.
#[allow(clippy::too_many_arguments)]
fn instantiate_components(
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
    local_texts: &HashMap<String, String>,
    inherited: &[(Component, Option<Expr>)],
    records_wider_for_fields: &HashMap<String, String>,
    records_here: &HashMap<String, String>,
    redeclares: &[Redeclare],
    component_broken: &dyn Fn(&str, &mut [bool]) -> bool,
    built: Built,
) -> Result<Built, String> {
    let scope = class.name.as_str();
    let overrides = env.overrides;
    let Built {
        mut taken,
        mut counted,
        mut record_values,
        sizes,
        mut sizes_here,
        mut local_consts,
        mut broke_something,
    } = built;
    // Parameters the lengths settled: `n = size(lines, 1)` is a number
    // once `lines` has been measured. Nothing outside the loop reads
    // them.
    let mut settled: HashMap<String, f64> = HashMap::new();
    let mut first_round = true;
    for component in &class.components {
        // Whether anything new was measured since the last round of
        // this loop. The first time through it is true whatever the
        // model has measured elsewhere: the parameters of this class
        // have not been asked yet, and asking them is what the round
        // below is for. Read as `taken < acc.sizes.len()` alone, a
        // class whose neighbours happened to measure nothing while it
        // was being reached never asked them at all - which made one
        // instance's parameters depend on what stood beside it.
        let fresh = first_round || taken < acc.sizes.len();
        first_round = false;
        while taken < acc.sizes.len() {
            // Every array measured so far, whatever it belongs to: a
            // modifier handed down is written in the terms of the
            // class that wrote it, so a child asked to make sense of
            // `lines[i, 2, :]` has to know how long `drawn.lines` is,
            // and `drawn` is not below it but above. The names are
            // full paths and cannot be mistaken for one another.
            let (name, shape) = &acc.sizes[taken];
            sizes_here.insert(name.clone(), shape.clone());
            taken += 1;
        }
        // An element of a parameter array is a number of its own -
        // `sequence[3]` is 3 - and a declaration after it may be
        // written with that number. The elements are recorded as they
        // are instantiated, so what is new is taken up here.
        while counted < acc.numbers.len() {
            let (name, value) = &acc.numbers[counted];
            local_consts.entry(name.clone()).or_insert(*value);
            counted += 1;
        }
        // A parameter may be worth a number only once the
        // declarations before it have been measured: `Integer n =
        // size(lines, 1)` is one as soon as `lines` is. So each time a
        // declaration adds a length, the parameters still without a
        // value are asked again.
        if fresh {
            for waiting in class
                .components
                .iter()
                .chain(inherited.iter().map(|(component, _)| component))
            {
                if local_consts.contains_key(&waiting.name)
                    || !matches!(
                        waiting.variability,
                        Variability::Parameter | Variability::Constant
                    )
                {
                    continue;
                }
                let Some(binding) = waiting.binding.as_ref() else {
                    continue;
                };
                let binding = substitute_class_constants(binding, registry, scope, imports, shadow);
                let binding = prefix_expr(&binding, prefix, outers);
                // A length may be arithmetic over `size(...)`, which
                // measures on its own, or written over arrays -
                // `max([size(a, 1); size(b, 1)])` stacks the lengths of
                // four signals and takes the longest - which only the
                // array layer can read.
                let measured =
                    dimension_value(&binding, &local_consts, &sizes_here).or_else(|| {
                        let no_loop_vars = HashMap::new();
                        let shapes = Shapes {
                            sizes: &sizes_here,
                            loop_vars: &no_loop_vars,
                            consts: &local_consts,
                            records: no_records(),
                        };
                        let mark = checks_mark();
                        let worked = expand(&binding, &shapes, registry, scope, imports, 0);
                        checks_rewind(mark);
                        let value = const_eval(&worked.ok()?.into_expr(), &local_consts)?;
                        (value.fract() == 0.0).then_some(value as i64)
                    });
                if let Some(length) = measured {
                    local_consts.insert(waiting.name.clone(), length as f64);
                    Inlined::forget();
                    local_consts.insert(format!("{prefix}{}", waiting.name), length as f64);
                    Inlined::forget();
                    acc.const_values
                        .insert(format!("{prefix}{}", waiting.name), length as f64);
                    settled.insert(waiting.name.clone(), length as f64);
                }
            }
        }
        let flat_name = format!("{prefix}{}", component.name);

        // An `outer` declaration owns nothing: its references were bound
        // to the enclosing `inner` instance above. A `redeclare` in the
        // body replaced an inherited declaration instead of adding one.
        if component.scope == Scope::Outer || component.redeclaration {
            continue;
        }

        // A selective `extends` broke this component: leave it out, and
        // mark it disabled so the connections to it fall away too.
        if component_broken(&component.name, &mut broke_something) {
            acc.disabled.push(flat_name.clone());
            continue;
        }

        // A `final` declaration is closed to the enclosing class: an
        // `extends Base(k = ...)` or a component modifier that reaches
        // it - itself or any of its attributes - is refused, since the
        // whole point of `final` is that the value cannot be changed
        // from outside.
        if component.is_final {
            let modifies = |name: &str| {
                name == component.name
                    || name.starts_with(&format!("{}.", component.name))
                    || name.starts_with(&format!("{}[", component.name))
            };
            if let Some((target, _)) = overrides.iter().find(|(name, _)| modifies(name)) {
                return Err(format!(
                    "`{}` is final and cannot be modified from outside, but `{target}` does",
                    format_args!("{prefix}{}", component.name)
                ));
            }
        }

        // `Support support if useSupport;` — a condition that does not
        // hold removes the component, and later the connections to it.
        if let Some(condition) = &component.condition {
            let mut env = acc.const_values.clone();
            env.extend(local_consts.iter().map(|(k, v)| (k.clone(), *v)));
            // As with an `if` equation, the condition may compare
            // against an enumeration literal - `gravityType ==
            // GravityTypes.UniformGravity` - which no environment holds
            // as a name of its own.
            //
            // The condition is written in this class's terms and the
            // values are filed under the paths they were instantiated
            // at, so it is put under the path first. That is also what
            // answers a condition reading an `outer`: every animated
            // part of the multi-body library is written `if
            // world.enableAnimation and animation`, and `world` is an
            // `outer` that owns no value of its own - the parameter
            // belongs to the `inner` the name stands for.
            let named = substitute_class_constants(condition, registry, scope, imports, &[]);
            let value = const_eval(&named, &env)
                .or_else(|| {
                    // The condition may be a comparison of strings.
                    let folded = strings::fold(&named, local_texts, &env).ok()?;
                    const_eval(&folded, &env)
                })
                .ok_or_else(|| {
                    format!("condition of component `{flat_name}` is not a compile-time constant")
                })?;
            if value == 0.0 {
                acc.disabled.push(flat_name.clone());
                continue;
            }
        }

        // The value that fixes a flexible `:` size, if the component has
        // one: an override handed down, else the declaration's own.
        // A value handed down is already written in the terms of the
        // class that handed it down; only the declaration's own still
        // needs this class's prefix put on it.
        let sizing_binding = overrides
            .iter()
            .find(|(name, _)| name == &component.name)
            .map(|(_, e)| (e.clone(), true))
            .or_else(|| component.binding.clone().map(|e| (e, false)));

        let mut component = component.clone();
        if let Some(value) = settled.get(&component.name) {
            component.binding = Some(Expr::Number(*value));
        }

        // A redeclaration from above replaces the type; its modifiers
        // come first so they win over the original declaration's.
        let mut extra_modifiers = Vec::new();
        let mut child_redeclares = Vec::new();
        if let Some(redeclare) = redeclares.iter().find(|r| r.name == component.name) {
            check_redeclare(registry, class, &component, redeclare)?;
            component.type_name = redeclare.type_name.clone();
            extra_modifiers.extend(redeclare.modifiers.iter().cloned());
        }
        // Redeclarations aimed at a component of this child travel on,
        // with the child's name stripped off the front.
        for redeclare in redeclares {
            if let Some(rest) = redeclare
                .name
                .strip_prefix(&format!("{}.", component.name))
                .map(str::to_string)
            {
                child_redeclares.push(Redeclare {
                    name: rest,
                    ..redeclare.clone()
                });
            }
        }
        for redeclare in &component.redeclares {
            child_redeclares.push(qualify_redeclare(
                redeclare, registry, class, prefix, outers, imports,
            )?);
        }

        // A connector may be one value rather than a set of members:
        // `connector RealInput = input Real` is how every signal in
        // the standard library is carried. Resolving the type below
        // leaves the primitive behind, so what class it came from is
        // noted first - a connection to it is still a connection.
        let value_connector = lookup(registry, &component.type_name, scope, imports)
            .filter(|class| {
                (class.kind == ClassKind::Connector && class.alias_of.is_some())
                    || names_a_connector(registry, &component.type_name, scope, imports)
            })
            .map(|class| class.name.clone());

        // A `type` alias stands for a primitive plus attribute
        // defaults, and an enumeration for an `Integer`; substitute
        // before instantiating. This has to happen before the
        // dimensions are counted: a type may be an array of its own -
        // `type Axis = Real[3]` - and a redeclaration may have just
        // replaced the type with one of a different shape.
        resolve_type(registry, &mut component, scope, imports);

        // How long the declaration is along each axis.
        let sizes = measure_dimensions(
            class,
            inherited,
            &component,
            &flat_name,
            sizing_binding.as_ref(),
            registry,
            scope,
            prefix,
            imports,
            shadow,
            outers,
            &sizes_here,
            &local_consts,
        )?;
        if !sizes.is_empty() {
            acc.sizes
                .push((format!("{prefix}{}", component.name), sizes.clone()));
        }
        // A dimension of zero is legal and means there is nothing
        // there: the declaration contributes no variables at all.
        let element_names: Vec<String> = if sizes.is_empty() {
            vec![component.name.clone()]
        } else {
            index_tuples(&sizes)
                .into_iter()
                .map(|indices| element_name(&component.name, &indices))
                .collect()
        };
        if !sizes.is_empty() && element_names.is_empty() {
            continue;
        }

        let level = Level {
            prefix,
            sizes: &sizes_here,
            outer_sizes: env.outer_sizes,
            outers,
            inners,
            overrides,
            consts: &local_consts,
            texts: local_texts,
            imports,
            scope,
            inside_a_parameter: env.inside_a_parameter,
        };
        // An array bound - or started - as a whole hands each element
        // its own value.
        let spread = |expr: &Expr, what: &str, prefixed: bool| -> Result<Vec<Expr>, String> {
            spread_over_elements(
                expr,
                what,
                prefixed,
                &component,
                &element_names,
                registry,
                scope,
                prefix,
                imports,
                shadow,
                outers,
                env,
                &sizes_here,
                &local_consts,
                records_here,
            )
        };
        // A modifier naming the whole array - `Chain c(m = {1, 2, 3})`
        // - beats the declaration's own value and is handed out to the
        // elements the same way.
        let handed_down = |target: &str| -> Option<Expr> {
            extra_modifiers
                .iter()
                .chain(overrides.iter())
                .find(|(name, _)| name == target)
                .map(|(_, value)| value.clone())
        };
        // The same value where the declaration is a parameter, which
        // may not become an equation: a parameter has to stay a value
        // the run works out at the start.
        let per_field = |value: &Expr, of: &ClassDef, prefixed: bool| -> Vec<Vec<(String, Expr)>> {
            record_value_per_field(
                value,
                of,
                prefixed,
                class,
                &element_names,
                registry,
                scope,
                prefix,
                imports,
                shadow,
                outers,
                &sizes_here,
                &local_consts,
                records_wider_for_fields,
                &sizes,
            )
        };
        // A record's value is not one number per element: `Complex
        // vs[m] = plug.pin.v` says as much about `vs[1].re` as about
        // `vs[1]`, and there is no name in the flat model for `vs[1]`
        // itself. Where the declaration is a variable, its value is a
        // declaration equation anyway - and an equation between
        // records is one this compiler already writes out field by
        // field. A parameter is another matter: its value has to stay
        // a value, so it is left as it was.
        let named_record = records_here
            .get(&format!("{prefix}{}", component.name))
            .and_then(|of| registry.get(of.as_str()).copied());
        let of_record = named_record.is_some() && component.variability == Variability::Continuous;
        let of_parameter = named_record.filter(|_| !of_record);
        let fields_given: Vec<Vec<(String, Expr)>> = match (
            of_parameter,
            handed_down(&component.name),
            &component.binding,
        ) {
            (Some(of), Some(value), _) => per_field(&value, of, true),
            (Some(of), None, Some(binding)) => per_field(binding, of, false),
            _ => Vec::new(),
        };
        let element_bindings: Option<Vec<Expr>> = match (
            handed_down(&component.name),
            &component.binding,
            sizes.is_empty(),
            of_record,
        ) {
            (Some(value), _, _, true) => {
                record_values.push((component.name.clone(), value, true));
                None
            }
            (None, Some(binding), _, true) => {
                record_values.push((component.name.clone(), binding.clone(), false));
                None
            }
            _ if of_parameter.is_some() && !fields_given.is_empty() => None,
            (Some(value), _, false, false) => Some(spread(&value, "value", true)?),
            (None, Some(binding), false, false) => Some(spread(binding, "value", false)?),
            _ => None,
        };
        let start_target = format!("{}.start", component.name);
        let element_starts: Option<Vec<Expr>> = match (
            handed_down(&start_target),
            &component.start,
            sizes.is_empty(),
        ) {
            (Some(value), _, false) => Some(spread(&value, "start", true)?),
            (None, Some(start), false) => Some(spread(start, "start", false)?),
            _ => None,
        };

        let element_count = element_names.len();
        // Every element of the declaration, instantiated where it
        // stands. This is the one part of the loop left inline: it
        // reads eleven of the things worked out above - the modifiers,
        // the redeclarations, the values, the starts, the connector -
        // and handing all eleven to a stage would say less than
        // leaving them where they were worked out.
        for (position, local_name) in element_names.iter().enumerate() {
            let flat_name = format!("{prefix}{local_name}");
            // This element's own modifiers: each value substituted and
            // prefixed once, and - on an array component - handed this
            // element's slice of an array-valued modifier, unless the
            // modifier was written `each`, which spreads it whole.
            let element_modifiers: Vec<(String, Expr)> = component
                .modifiers
                .iter()
                .map(|(name, value)| {
                    let value = substitute_class_constants(value, registry, scope, imports, shadow);
                    let value = prefix_expr(&value, prefix, outers);
                    // A component with no dimensions at all takes its
                    // modifier whole, and so does one written `each`.
                    // An array of a single element is still an array:
                    // `p[1](k = zeros(1))` hands its one element the
                    // one entry, not the vector, which is what the
                    // rectifiers do when they reuse a polyphase block
                    // with `m = 1`.
                    let spread_whole =
                        sizes.is_empty() || component.each_modifiers.iter().any(|e| e == name);
                    let value = if spread_whole {
                        value
                    } else {
                        array_element(
                            &value,
                            position,
                            element_count,
                            &sizes_here,
                            &local_consts,
                            registry,
                            scope,
                            imports,
                        )
                    };
                    (name.clone(), value)
                })
                .chain(fields_given.get(position).into_iter().flatten().cloned())
                .collect();
            let site = Site {
                component: &component,
                local_name,
                flat_name: &flat_name,
                extra_modifiers: &extra_modifiers,
                modifiers: &element_modifiers,
                redeclares: &child_redeclares,
                binding: element_bindings.as_ref().map(|items| &items[position]),
                start: element_starts.as_ref().map(|items| &items[position]),
                value_connector: value_connector.as_deref(),
            };
            instantiate_one(registry, &site, &level, acc, depth)?;
        }
    }

    Ok(Built {
        taken,
        counted,
        record_values,
        sizes,
        sizes_here,
        local_consts,
        broke_something,
    })
}

/// How long one declaration is along each axis.
///
/// A dimension may be a number, a type - `Real x[Boolean]` has two
/// elements, `Real x[E]` one per enumeration literal - or a `:` that
/// reads its length from the value the component was given.
///
/// Moved out of `instantiate_components` unchanged.
#[allow(clippy::too_many_arguments)]
fn measure_dimensions(
    class: &ClassDef,
    inherited: &[(Component, Option<Expr>)],
    component: &Component,
    flat_name: &str,
    sizing_binding: Option<&(Expr, bool)>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    prefix: &str,
    imports: &[(String, String)],
    shadow: &[&str],
    outers: &HashMap<String, String>,
    sizes_here: &HashMap<String, Vec<i64>>,
    local_consts: &HashMap<String, f64>,
) -> Result<Vec<i64>, String> {
    // Array dimensions expand into scalar elements. A dimension may
    // be a number, but also a type - `Real x[Boolean]` has two
    // elements, `Real x[E]` one per enumeration literal - or a `:`
    // that reads its length from the value the component is given.
    let mut sizes = Vec::new();
    for (axis, dimension) in component.dimensions.iter().enumerate() {
        let value = match dimension {
            Expr::Ref(name) if name == "Boolean" => 2,
            Expr::Ref(name)
                if lookup(registry, name, scope, imports)
                    .is_some_and(|c| !c.enumeration.is_empty()) =>
            {
                lookup(registry, name, scope, imports)
                    .unwrap()
                    .enumeration
                    .len() as i64
            }
            Expr::ColonSubscript => {
                // A value written out says its length by being
                // written out. Anything else - a list scaled by a
                // factor, which is how the standard library draws
                // its axis labels - has to be worked out before it
                // can be measured.
                let measured = |(binding, prefixed): &(Expr, bool)| -> Option<i64> {
                    if let Some(length) = flexible_size(binding, axis, registry, scope, imports) {
                        return Some(length);
                    }
                    let shapes = Shapes {
                        sizes: sizes_here,
                        loop_vars: &HashMap::new(),
                        consts: local_consts,
                        records: no_records(),
                    };
                    let binding = match prefixed {
                        true => binding.clone(),
                        false => {
                            let binding = substitute_class_constants(
                                binding, registry, scope, imports, shadow,
                            );
                            prefix_expr(&binding, prefix, outers)
                        }
                    };
                    // A measurement is not the model asking for a
                    // value, so nothing it works out is kept.
                    let mark = checks_mark();
                    let value = expand(&binding, &shapes, registry, scope, imports, 0);
                    checks_rewind(mark);
                    let value = value.ok()?;
                    value.shape().get(axis).map(|length| *length as i64)
                };
                sizing_binding.and_then(measured).ok_or_else(|| {
                    format!(
                        "the flexible size `:` of `{flat_name}` needs a value to read \
                             its length from, and {} is not one",
                        sizing_binding.map_or_else(
                            || "nothing".to_string(),
                            |(binding, _)| crate::flatten::names::sketch(binding)
                        )
                    )
                })?
            }
            _ => {
                // `Shape cylinders[n]` where `n = size(lines, 1)`:
                // the length was written with one that only the
                // declarations before it can give, and by now they
                // have given it.
                let off_a_length = || -> Option<i64> {
                    let Expr::Ref(name) = dimension else {
                        return dimension_value(dimension, local_consts, sizes_here);
                    };
                    let bound = class
                        .components
                        .iter()
                        .chain(inherited.iter().map(|(component, _)| component))
                        .find(|c| &c.name == name)?
                        .binding
                        .as_ref()?;
                    let bound = prefix_expr(bound, prefix, outers);
                    dimension_value(&bound, local_consts, sizes_here)
                };
                // A length may be a constant of a package the class
                // is written inside - `Xi[nXi]` of a medium counts
                // its substances - and that is a name no
                // environment holds.
                let named = substitute_class_constants(dimension, registry, scope, imports, shadow);
                let value = const_eval(&named, local_consts)
                    .or_else(|| off_a_length().map(|length| length as f64))
                    .ok_or_else(|| {
                        format!("dimension of `{flat_name}` is not a compile-time constant")
                    })?;
                if value.fract() != 0.0 || value < 0.0 {
                    return Err(format!(
                        "dimension of `{flat_name}` must be a whole number that is not \
                             negative, got {value}"
                    ));
                }
                value as i64
            }
        };
        sizes.push(value);
    }

    Ok(sizes)
}

/// One value of a whole array handed out to its elements.
///
/// A declaration bound or started as a whole, as `Real k[3] = {2, 4,
/// 6}` is, says one thing about three elements, and each element wants
/// its own of it.
///
/// Moved out of `instantiate_components` unchanged.
#[allow(clippy::too_many_arguments)]
fn spread_over_elements(
    expr: &Expr,
    what: &str,
    prefixed: bool,
    component: &Component,
    element_names: &[String],
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    prefix: &str,
    imports: &[(String, String)],
    shadow: &[&str],
    outers: &HashMap<String, String>,
    env: &Env,
    sizes_here: &HashMap<String, Vec<i64>>,
    local_consts: &HashMap<String, f64>,
    records_here: &HashMap<String, String>,
) -> Result<Vec<Expr>, String> {
    let shapes = Shapes {
        sizes: sizes_here,
        loop_vars: &HashMap::new(),
        consts: local_consts,
        records: records_here,
    };
    // A modifier arrives already written in the terms of the
    // class that supplied it; only a declaration's own value
    // still needs this class's prefix.
    let expr = if prefixed {
        expr.clone()
    } else {
        let expr = substitute_class_constants(expr, registry, scope, imports, shadow);
        prefix_expr(&expr, prefix, outers)
    };
    let value = expand(&expr, &shapes, registry, scope, imports, 0)?;
    let mut items = Vec::new();
    value.flatten_into(&mut items);
    // A scalar start spreads over the whole array - but only
    // a real scalar. A value handed down an `extends` is
    // written in the terms of the class above, where `T =
    // T_ref` names an array; here that name means nothing, so
    // it comes back whole and looks exactly like a scalar.
    // Spread, it binds every element of the array to the
    // whole array, which is the shape nothing can check and
    // the parameters cannot evaluate. Where the name is known
    // above to be an array of the same length, its elements
    // are what was meant, one apiece.
    // An array of one is still an array: a resistance
    // connection of star points comes to a single base
    // system, and its `T = T_ref` hands one name to one
    // element. Spread rather than subscripted, that element
    // is bound to the array itself, which is a name no
    // parameter can be worked out from.
    if items.len() == 1 && !element_names.is_empty() {
        if let Expr::Ref(name) = &items[0] {
            if let Some(shape) = env.handed_shapes.get(name.as_str()) {
                let indices = index_tuples(shape);
                if indices.len() == element_names.len() {
                    return Ok(indices
                        .into_iter()
                        .map(|at| Expr::Ref(element_name(name, &at)))
                        .collect());
                }
            }
        }
    }
    if items.len() == 1 && element_names.len() > 1 {
        return Ok(vec![items[0].clone(); element_names.len()]);
    }
    if items.len() != element_names.len() {
        return Err(format!(
            "`{}` has {} element(s) but its {what} has {}",
            component.name,
            element_names.len(),
            items.len()
        ));
    }
    Ok(items)
}

/// A record value handed down as one modifier per field.
///
/// A parameter may not become an equation - it has to stay a value the
/// run works out at the start - so `rcData = data` is handed on as `R
/// = ..., C = ...`, which is what the writer would have said. A field
/// the record declares `final` is worked out from the others where it
/// lands and is not one a value may set; refusing the whole record for
/// having one left the machines' loss parameters unset.
///
/// Moved out of `instantiate_components` unchanged.
#[allow(clippy::too_many_arguments)]
fn record_value_per_field(
    value: &Expr,
    of: &ClassDef,
    prefixed: bool,
    _class: &ClassDef,
    element_names: &[String],
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    prefix: &str,
    imports: &[(String, String)],
    shadow: &[&str],
    outers: &HashMap<String, String>,
    sizes_here: &HashMap<String, Vec<i64>>,
    local_consts: &HashMap<String, f64>,
    records_wider_for_fields: &HashMap<String, String>,
    sizes: &[i64],
) -> Vec<Vec<(String, Expr)>> {
    // The value comes apart into every field the record has,
    // final ones among them, because that is what the record
    // is. Which of them may be handed on is a separate
    // question, answered once the value has been taken apart:
    // a `final` field is worked out from the others where it
    // lands and is not one a value may set. Refusing the whole
    // record for having one was what left the machines' loss
    // parameters unset, since a friction record states its
    // reference torque as a `final` field.
    let fields: Vec<String> = of
        .components
        .iter()
        .map(|field| field.name.clone())
        .collect();
    let settable: Vec<bool> = of.components.iter().map(|field| !field.is_final).collect();
    if fields.is_empty() || !settable.iter().any(|may| *may) {
        return Vec::new();
    }
    let shapes = Shapes {
        sizes: sizes_here,
        loop_vars: &HashMap::new(),
        consts: local_consts,
        // A value handed down arrives written in the terms of
        // the class that supplied it - `Machine m(friction =
        // data.friction)` names a record that class holds, not
        // one of this one - so what every class built so far
        // knows has to be in view, as it is for a record-valued
        // variable further down. Without it the value is not
        // recognised as a record at all, comes apart into
        // nothing, and the fields are left to whatever their
        // declarations said.
        records: records_wider_for_fields,
    };
    let expr = match prefixed {
        true => value.clone(),
        false => {
            let expr = substitute_class_constants(value, registry, scope, imports, shadow);
            prefix_expr(&expr, prefix, outers)
        }
    };
    let worked = expand(&expr, &shapes, registry, scope, imports, 0).and_then(|worked| {
        records_written_out(worked, &shapes, registry, &|e| {
            expand(e, &shapes, registry, scope, imports, 0)
        })
    });
    let Ok(worked) = worked else {
        return Vec::new();
    };
    // One record is its fields, and a field may be an array of
    // its own, so what is counted here is fields rather than
    // numbers.
    let one = |item: &Value| -> Option<Vec<Expr>> {
        match item {
            Value::Array(given) if given.len() == fields.len() => {
                Some(given.iter().cloned().map(Value::into_expr).collect())
            }
            _ => None,
        }
    };
    // An array of records comes apart twice over: once into its
    // elements and once into each element's fields. The
    // elements lie as many levels down as the declaration has
    // dimensions, so that is how far to go - `Complex sTM[m,
    // m]` is m rows of m records, and counting entries at one
    // level instead would take the two rows of a 2 by 2 for
    // the two fields of one record.
    let one_apiece = || -> Option<Vec<Vec<Expr>>> {
        let mut elements = Vec::new();
        levels_down(&worked, sizes.len(), &mut elements);
        match elements.len() == element_names.len() {
            true => elements.iter().map(one).collect(),
            false => None,
        }
    };
    // One record for all of them, which is what a scalar value
    // does for an array.
    let over_all = || one(&worked).map(|whole| vec![whole; element_names.len()]);
    // Which of the two the value is under is a question about
    // how many numbers it holds rather than about how many
    // entries any one level has: a record of two fields handed
    // to an array of two elements has the same length either
    // way, and reading it wrongly gives every element the same
    // wrong value with nothing said. One record of this class
    // is so many numbers, and the value is either that many or
    // that many times over.
    let mut leaves = Vec::new();
    worked.flatten_into(&mut leaves);
    let of_one = numbers_of_one(registry, of, 0);
    let per_element = match of_one {
        // A record whose shape holds a length the compiler
        // cannot see says nothing either way, and the reading
        // that was here before has its say.
        None | Some(0) => one_apiece().or_else(over_all),
        Some(each) if leaves.len() == each * element_names.len() => one_apiece(),
        Some(each) if leaves.len() == each => over_all(),
        Some(_) => None,
    }
    .unwrap_or_default();
    per_element
        .into_iter()
        .map(|given| {
            fields
                .iter()
                .cloned()
                .zip(given)
                .zip(&settable)
                .filter(|(_, may)| **may)
                .map(|(field, _)| field)
                .collect()
        })
        .collect()
}

/// Working an expression of this class out where it stands: the array
/// layer, with the class's names, shapes and loop variables in view.
pub(super) type ExpandHere<'a> = dyn Fn(&Expr, &HashMap<String, f64>) -> Result<Value, String> + 'a;

/// Instantiate one component element (a scalar, or one element of an
/// array).
pub(super) fn instantiate_one(
    registry: &HashMap<&str, &ClassDef>,
    site: &Site,
    level: &Level,
    acc: &mut Flat,
    depth: usize,
) -> Result<(), String> {
    let Site {
        component,
        local_name,
        flat_name,
        extra_modifiers,
        modifiers,
        redeclares,
        binding: _,
        start: _,
        value_connector,
    } = *site;
    let Level {
        prefix,
        sizes,
        outer_sizes,
        outers,
        inners,
        overrides,
        consts: local_consts,
        texts: local_texts,
        imports,
        scope,
        inside_a_parameter,
    } = *level;
    // A `parameter` record is a parameter all the way down: its
    // fields are declared plainly inside the record, and left as they
    // are each one's value becomes a declaration equation - which the
    // parameters are worked out without, so `R_start.T[1, 1]` is a
    // name nothing gives a value to.
    let of_a_parameter_record = matches!(
        component.variability,
        Variability::Parameter | Variability::Constant
    ) && lookup(registry, &component.type_name, scope, imports)
        .is_some_and(|of| of.kind == ClassKind::Record);
    let inside_a_parameter = inside_a_parameter || of_a_parameter_record;
    {
        if is_primitive(&component.type_name) {
            let mut flat = component.clone();
            flat.name = flat_name.to_string();
            flat.dimensions = Vec::new();
            let made_a_parameter =
                inside_a_parameter && flat.variability == Variability::Continuous;
            if made_a_parameter {
                flat.variability = Variability::Parameter;
            }
            // What a declaration says about itself has to come to one
            // value, but may be written over arrays to get there:
            // `nout = max([size(q_begin, 1); size(q_end, 1)])` counts
            // the longest of four by stacking their lengths.
            // What every path built so far is a record of, which is
            // what says that `v*i` of two complex numbers is the
            // record's own multiplication rather than arithmetic on
            // two names: a declaration's value is worked out here and
            // the operands are components of the class holding it.
            let records_so_far = &acc.records;
            let resolve_value = |e: &Expr| -> Result<Expr, String> {
                let e = substitute_class_constants(e, registry, scope, imports, &[]);
                // A value may be worked out of a `String` this class
                // settled - `findLast(fileName, ".csv")` is how a
                // table block asks what kind of file it was given -
                // and a name is nothing a body can measure. What each
                // string is worth is known here.
                let e = substitute_texts(&e, local_texts);
                let no_loop_vars = HashMap::new();
                let shapes = Shapes {
                    sizes,
                    loop_vars: &no_loop_vars,
                    consts: local_consts,
                    records: records_so_far,
                };
                expand(
                    &prefix_expr(&e, prefix, outers),
                    &shapes,
                    registry,
                    scope,
                    imports,
                    0,
                )?
                .scalar()
            };
            flat.start = match site.start {
                Some(expr) => Some(expr.clone()),
                None => flat.start.as_ref().map(&resolve_value).transpose()?,
            };
            flat.binding = match site.binding {
                // Already expanded from the array the declaration bound.
                Some(expr) => Some(expr.clone()),
                None => flat.binding.as_ref().map(&resolve_value).transpose()?,
            };
            // A bound is read the same way: `timeScale(min = Modelica
            // .Constants.eps)` names a constant of a package, and
            // nothing downstream holds a name like that. A bound
            // written over a whole array does not come to one value,
            // and is left as it was rather than refused - a bound is
            // something a model is held to, not something it is built
            // from.
            let bound = |expr: &Expr| resolve_value(expr).unwrap_or_else(|_| expr.clone());
            flat.min = flat.min.as_ref().map(bound);
            flat.max = flat.max.as_ref().map(bound);
            // A parent modifier `name = expr` overrides the binding, and
            // a nested one - `phi(start = 1)` - the attribute.
            let modifier = |target: &str| {
                extra_modifiers
                    .iter()
                    .chain(overrides.iter())
                    .find(|(n, _)| n == target)
                    .map(|(_, e)| e.clone())
            };
            if let Some(value) = modifier(local_name) {
                // A modifier arrives written in the terms of the class
                // that supplied it, so it is not prefixed again - but
                // it still has to be worked out. Left as it stands, a
                // call inside it is never inlined, and a value that
                // came down an `extends` is a call the parameters
                // cannot evaluate: the machines say what their
                // nominal voltage is that way, through a function of
                // the resistance and the brush drop.
                let no_loop_vars = HashMap::new();
                // A value handed down may name an array of the class
                // that wrote it - `Root r(s = anyTrue(suspend.reset))`
                // is worked out here, inside `Root`, and `suspend`
                // belongs to the class holding `r`. Those shapes are
                // in view only for reading a modifier: taken into the
                // table this class builds, they would also spread a
                // value over elements it was never meant for.
                let mut shapes = Shapes {
                    sizes,
                    loop_vars: &no_loop_vars,
                    consts: local_consts,
                    records: no_records(),
                };
                let worked = substitute_class_constants(&value, registry, scope, imports, &[]);
                let mut reach;
                let mut done = expand(&worked, &shapes, registry, scope, imports, 0)
                    .and_then(|value| value.scalar());
                if done.is_err() && !outer_sizes.is_empty() {
                    reach = sizes.clone();
                    reach.extend(outer_sizes.iter().map(|(n, s)| (n.clone(), s.clone())));
                    shapes.sizes = &reach;
                    done = expand(&worked, &shapes, registry, scope, imports, 0)
                        .and_then(|value| value.scalar());
                }
                let worked = done;
                flat.binding = Some(worked.unwrap_or(value));
            }
            // On an array the start has already been handed out
            // element by element; this is the scalar case.
            if site.start.is_none() {
                if let Some(value) = modifier(&format!("{}.start", component.name)) {
                    flat.start = Some(value);
                }
            }
            if let Some(value) = modifier(&format!("{}.fixed", component.name)) {
                flat.fixed = Some(!matches!(value, Expr::Bool(false) | Expr::Number(0.0)));
            }
            // On a variable rather than a parameter, a binding is a
            // declaration equation: `Support support(tau = -flange.tau)`
            // in the standard library ties a connector to its component.
            if flat.variability == Variability::Continuous {
                // An `input` that is not a connector is settled from
                // outside: either here by a value on the declaration,
                // or by whoever holds the class. Either way it is one
                // equation, and which of the two it was is known here
                // and nowhere later.
                if flat.causality == Causality::Input
                    && flat.binding.is_none()
                    && value_connector.is_none()
                    && acc.instances.contains_key(&acc.origin)
                {
                    acc.unsupplied.push((acc.origin.clone(), flat.name.clone()));
                }
                if let Some(value) = flat.binding.take() {
                    acc.equations.push(EquationItem {
                        lhs: Expr::Ref(flat.name.clone()),
                        rhs: value,
                        // A declaration equation belongs to the class
                        // that wrote the declaration.
                        origin: acc.origin.clone(),
                    });
                }
            }
            // A parameter or constant whose value comes out as a
            // number is worth one from here on. The table a class
            // builds before it instantiates anything knows a whole
            // array by one name and cannot say what an element of it
            // is; here each element is its own declaration, and
            // `conversionTable[6]` is a number the next declaration
            // may be written with.
            if matches!(
                flat.variability,
                Variability::Parameter | Variability::Constant
            ) && !made_a_parameter
            {
                if let Some(value) = flat
                    .binding
                    .as_ref()
                    .and_then(|expr| const_eval(expr, &acc.const_values))
                {
                    acc.const_values.insert(flat.name.clone(), value);
                    acc.numbers.push((flat.name.clone(), value));
                }
            }
            // A connector that is one value is still a connector: a
            // `connect` naming it joins the values themselves.
            if let Some(class_name) = value_connector {
                acc.connectors
                    .insert(flat_name.to_string(), class_name.to_string());
                // `connector RealInput = input Real` writes the
                // direction on the connector rather than on the
                // declaration, and the flat model is where everything
                // downstream looks for it.
                if flat.causality == Causality::None {
                    if let Some(of) = lookup(registry, class_name, scope, imports) {
                        flat.causality = of.alias_causality;
                    }
                }
                if !component.annotations.is_empty() {
                    acc.connect_rules
                        .push((flat_name.to_string(), component.annotations.clone()));
                }
            }
            acc.components.push(flat);
        } else {
            let child =
                lookup(registry, &component.type_name, scope, imports).ok_or_else(|| {
                    format!(
                        "unknown type `{}` of component `{flat_name}`",
                        component.type_name
                    )
                })?;
            if child.partial {
                return Err(format!(
                    "`{}` is partial and cannot be instantiated as `{flat_name}`",
                    child.name
                ));
            }
            if matches!(child.kind, ClassKind::Package | ClassKind::Function) {
                return Err(format!(
                    "`{}` is a {} and cannot be a component type",
                    child.name,
                    if child.kind == ClassKind::Package {
                        "package"
                    } else {
                        "function"
                    }
                ));
            }
            // A connector may be a name for a record - `connector
            // ComplexOutput = output Complex` carries a complex signal
            // the way `RealOutput` carries a real one. Resolving the
            // type leaves the record behind, so what it came from was
            // noted before; a connection to it joins the record's
            // members, which is what a connection to any connector
            // does.
            if child.kind == ClassKind::Connector || value_connector.is_some() {
                acc.connectors
                    .insert(flat_name.to_string(), child.name.clone());
                // What the declaration said about how it must be
                // connected travels with the port, since that is the
                // last place the two are seen together.
                if !component.annotations.is_empty() {
                    acc.connect_rules
                        .push((flat_name.to_string(), component.annotations.clone()));
                }
            }
            // Child modifiers, outermost first so they win: dotted
            // overrides handed down, then a redeclaration's, then the
            // ones written on this declaration.
            let inherited = overrides.iter().filter_map(|(name, value)| {
                name.strip_prefix(&format!("{local_name}."))
                    .map(|rest| (rest.to_string(), value.clone()))
            });
            let mods: Vec<(String, Expr)> = inherited
                .chain(extra_modifiers.iter().cloned())
                .chain(modifiers.iter().map(|(n, e)| {
                    // Already substituted and prefixed, and given this
                    // element its slice, before the site was built.
                    (n.clone(), e.clone())
                }))
                .collect();
            // A class extending `ExternalObject` is a handle: it holds
            // no variables, and what it stands for is whatever its
            // constructor was handed. Nothing downstream could reach
            // that once the declaration is gone, so it is kept here.
            if descends_from_external_object(registry, child, 0) {
                if let Some(built) = site.binding.cloned().or_else(|| component.binding.clone()) {
                    let no_loop_vars = HashMap::new();
                    let shapes = Shapes {
                        sizes,
                        loop_vars: &no_loop_vars,
                        consts: local_consts,
                        records: no_records(),
                    };
                    let built = substitute_class_constants(&built, registry, scope, imports, &[]);
                    let built = prefix_expr(&built, prefix, outers);
                    let built = expand(&built, &shapes, registry, scope, imports, 0)?;
                    // Filed under the name the constructor calls
                    // outside, since that is what says what kind of
                    // handle this is - the class it is declared in is
                    // a library's own business.
                    let outside = lookup(
                        registry,
                        &format!("{}.constructor", child.name),
                        scope,
                        imports,
                    )
                    .and_then(|made| made.external_call.as_ref())
                    .map(|call| call.called.clone());
                    let built = match (built.into_expr(), outside) {
                        (Expr::Call(_, args), Some(outside)) => Expr::Call(outside, args),
                        (built, _) => built,
                    };
                    acc.handles.insert(flat_name.to_string(), built);
                }
                return Ok(());
            }
            let child_prefix = format!("{flat_name}.");
            let child_env = Env {
                overrides: &mods,
                redeclares,
                inners,
                broken: &[],
                handed_shapes: &HashMap::new(),
                outer_sizes: sizes,
                inside_a_parameter,
            };
            instantiate(registry, child, &child_prefix, &child_env, acc, depth + 1)?;
        }
    }
    Ok(())
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
