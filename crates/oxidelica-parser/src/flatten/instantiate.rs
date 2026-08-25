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
            &outers,
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
                    component.binding.as_ref().map(|e| {
                        let e = substitute_class_constants(e, registry, scope, &imports, &shadow);
                        prefix_expr(&e, prefix, &outers)
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
            false => lookup(registry, &extend.base, scope, &imports)
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
        let mods: Vec<(String, Expr)> = extend
            .modifiers
            .iter()
            .map(|(n, e)| {
                let e = substitute_class_constants(e, registry, scope, &imports, &shadow);
                (n.clone(), prefix_expr(&e, prefix, &outers))
            })
            .chain(env.overrides.iter().cloned())
            .collect();
        let mut base_redeclares = Vec::new();
        for redeclare in &extend.redeclares {
            base_redeclares.push(qualify_redeclare(
                redeclare,
                registry,
                class,
                prefix,
                &outers,
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
        collect_shapes(
            registry,
            class,
            &here,
            &HashMap::new(),
            &mut handed_shapes,
            0,
        );
        let handed_shapes = prefixed_sizes(&handed_shapes, prefix);
        let base_env = Env {
            overrides: &mods,
            handed_shapes: &handed_shapes,
            outer_sizes: env.outer_sizes,
            redeclares: &base_redeclares,
            inners: &inners,
            broken: &extend.broken,
            inside_a_parameter: env.inside_a_parameter,
        };
        instantiate(registry, base, prefix, &base_env, acc, depth + 1)?;
    }
    let overrides = env.overrides;

    // A selective `extends` leaves out named elements of this class:
    // `break f` drops the component and its connections, `break
    // connect(a, b)` drops that one connection. Every break must match
    // something, so what did is tracked and checked at the end.
    let broken = env.broken;
    let mut broke_something = vec![false; broken.len()];
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

    // Parameter values of this class, resolved to numbers where
    // possible: array dimensions and loop bounds are compile-time
    // constants and must come from here.
    let mut local_consts: HashMap<String, f64> = HashMap::new();
    // A base class's parameters are this class's too, and a dimension
    // may be written on one of them: `extends TwoPlug` brings `m`, and
    // `parameter Voltage V[m]` is written with it. What the `extends`
    // clause said about a base parameter comes with it.
    let inherited = inherited_parameters(registry, class, 0);
    loop {
        let mut progress = false;
        // What every declaration is read against: the model's numbers
        // and this class's own. It is built once a round and kept up as
        // values settle - building it per declaration is what a class
        // with a thousand numbers below it cannot afford.
        let mut env = acc.const_values.clone();
        env.extend(local_consts.iter().map(|(k, v)| (k.clone(), *v)));
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
            let binding = overrides
                .iter()
                .find(|(n, _)| n == &component.name)
                .map(|(_, e)| e.clone())
                .or_else(|| from_extends.cloned())
                .or_else(|| {
                    component
                        .binding
                        .as_ref()
                        .or(component.start.as_ref())
                        .map(|e| {
                            let e =
                                substitute_class_constants(e, registry, scope, &imports, &shadow);
                            prefix_expr(&e, prefix, &outers)
                        })
                });
            let Some(expr) = binding else { continue };
            // A parameter may be worked out by a function - the
            // standard library counts the base systems of an m-phase
            // winding that way - and a call is not something arithmetic
            // alone can fold, so the call is inlined first. Anything
            // the inlining will not do leaves the parameter for a
            // later round, or for no round at all.
            let settled = const_eval(&expr, &env).or_else(|| {
                let inlined = resolve(
                    &expr,
                    &HashMap::new(),
                    &env,
                    &HashMap::new(),
                    registry,
                    scope,
                    &imports,
                    0,
                )
                .ok()?;
                const_eval(&inlined, &env)
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

    // What each array component of this class - and of its bases - is
    // shaped like, so a value may name one as a whole.
    let mut sizes: HashMap<String, Vec<i64>> = HashMap::new();
    collect_shapes(
        registry,
        class,
        &local_consts,
        &HashMap::new(),
        &mut sizes,
        0,
    );
    let mut sizes_here = prefixed_sizes(&sizes, prefix);
    // How much of the growing list of measured arrays has been taken
    // into the table above. Each declaration brings its own, and the
    // one after it may be written with them.
    let mut taken = 0;
    // The same, for the numbers each declaration turns out to be worth.
    let mut counted = 0;
    // Parameters the lengths settled: `n = size(lines, 1)` is a number
    // by the time `lines` has been measured, and the declaration keeps
    // the number rather than the question, since nothing after
    // flattening knows how to measure an array.
    let mut settled: HashMap<String, f64> = HashMap::new();
    // Which of this class's components are records, and of what: an
    // overloaded operator is chosen by the record its operands are of.
    // What a record-valued variable was given as its value, kept until
    // the array layer is ready to say it: the name, the value, and
    // whether it came already written in this class's terms.
    let mut record_values: Vec<(String, Expr, bool)> = Vec::new();
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

    for component in &class.components {
        let fresh = taken < acc.sizes.len();
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
                let binding =
                    substitute_class_constants(binding, registry, scope, &imports, &shadow);
                let binding = prefix_expr(&binding, prefix, &outers);
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
                        let worked = expand(&binding, &shapes, registry, scope, &imports, 0);
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
            let named = substitute_class_constants(condition, registry, scope, &imports, &[]);
            let value = const_eval(&named, &env)
                .or_else(|| {
                    // The condition may be a comparison of strings.
                    let folded = strings::fold(&named, &local_texts, &env).ok()?;
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
        for redeclare in &redeclares {
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
                redeclare, registry, class, prefix, &outers, &imports,
            )?);
        }

        // A connector may be one value rather than a set of members:
        // `connector RealInput = input Real` is how every signal in
        // the standard library is carried. Resolving the type below
        // leaves the primitive behind, so what class it came from is
        // noted first - a connection to it is still a connection.
        let value_connector = lookup(registry, &component.type_name, scope, &imports)
            .filter(|class| {
                (class.kind == ClassKind::Connector && class.alias_of.is_some())
                    || names_a_connector(registry, &component.type_name, scope, &imports)
            })
            .map(|class| class.name.clone());

        // A `type` alias stands for a primitive plus attribute
        // defaults, and an enumeration for an `Integer`; substitute
        // before instantiating. This has to happen before the
        // dimensions are counted: a type may be an array of its own -
        // `type Axis = Real[3]` - and a redeclaration may have just
        // replaced the type with one of a different shape.
        resolve_type(registry, &mut component, scope, &imports);

        // Array dimensions expand into scalar elements. A dimension may
        // be a number, but also a type - `Real x[Boolean]` has two
        // elements, `Real x[E]` one per enumeration literal - or a `:`
        // that reads its length from the value the component is given.
        let mut sizes = Vec::new();
        for (axis, dimension) in component.dimensions.iter().enumerate() {
            let value = match dimension {
                Expr::Ref(name) if name == "Boolean" => 2,
                Expr::Ref(name)
                    if lookup(registry, name, scope, &imports)
                        .is_some_and(|c| !c.enumeration.is_empty()) =>
                {
                    lookup(registry, name, scope, &imports)
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
                        if let Some(length) = flexible_size(binding, axis) {
                            return Some(length);
                        }
                        let shapes = Shapes {
                            sizes: &sizes_here,
                            loop_vars: &HashMap::new(),
                            consts: &local_consts,
                            records: no_records(),
                        };
                        let binding = match prefixed {
                            true => binding.clone(),
                            false => {
                                let binding = substitute_class_constants(
                                    binding, registry, scope, &imports, &shadow,
                                );
                                prefix_expr(&binding, prefix, &outers)
                            }
                        };
                        // A measurement is not the model asking for a
                        // value, so nothing it works out is kept.
                        let mark = checks_mark();
                        let value = expand(&binding, &shapes, registry, scope, &imports, 0);
                        checks_rewind(mark);
                        let value = value.ok()?;
                        value.shape().get(axis).map(|length| *length as i64)
                    };
                    sizing_binding.as_ref().and_then(measured).ok_or_else(|| {
                        format!(
                            "the flexible size `:` of `{flat_name}` needs a value to read \
                             its length from, and {} is not one",
                            sizing_binding.as_ref().map_or_else(
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
                            return dimension_value(dimension, &local_consts, &sizes_here);
                        };
                        let bound = class
                            .components
                            .iter()
                            .chain(inherited.iter().map(|(component, _)| component))
                            .find(|c| &c.name == name)?
                            .binding
                            .as_ref()?;
                        let bound = prefix_expr(bound, prefix, &outers);
                        dimension_value(&bound, &local_consts, &sizes_here)
                    };
                    // A length may be a constant of a package the class
                    // is written inside - `Xi[nXi]` of a medium counts
                    // its substances - and that is a name no
                    // environment holds.
                    let named =
                        substitute_class_constants(dimension, registry, scope, &imports, &shadow);
                    let value = const_eval(&named, &local_consts)
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
            outers: &outers,
            inners: &inners,
            overrides,
            consts: &local_consts,
            imports: &imports,
            scope,
            inside_a_parameter: env.inside_a_parameter,
        };
        // An array bound - or started - as a whole hands each element
        // its own value.
        let spread = |expr: &Expr, what: &str, prefixed: bool| -> Result<Vec<Expr>, String> {
            let shapes = Shapes {
                sizes: &sizes_here,
                loop_vars: &HashMap::new(),
                consts: &local_consts,
                records: &records_here,
            };
            // A modifier arrives already written in the terms of the
            // class that supplied it; only a declaration's own value
            // still needs this class's prefix.
            let expr = if prefixed {
                expr.clone()
            } else {
                let expr = substitute_class_constants(expr, registry, scope, &imports, &shadow);
                prefix_expr(&expr, prefix, &outers)
            };
            let value = expand(&expr, &shapes, registry, scope, &imports, 0)?;
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
        // the run works out at the start. It is handed down as one
        // modifier per field instead, which is what `rcData(R = ..., C
        // = ...)` would have said. A field the record declares `final`
        // is not one a value may hand down, and where the value will
        // not come apart at all it is left where it was.
        let per_field = |value: &Expr, of: &ClassDef, prefixed: bool| -> Vec<Vec<(String, Expr)>> {
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
                sizes: &sizes_here,
                loop_vars: &HashMap::new(),
                consts: &local_consts,
                // A value handed down arrives written in the terms of
                // the class that supplied it - `Machine m(friction =
                // data.friction)` names a record that class holds, not
                // one of this one - so what every class built so far
                // knows has to be in view, as it is for a record-valued
                // variable further down. Without it the value is not
                // recognised as a record at all, comes apart into
                // nothing, and the fields are left to whatever their
                // declarations said.
                records: &records_wider_for_fields,
            };
            let expr = match prefixed {
                true => value.clone(),
                false => {
                    let expr =
                        substitute_class_constants(value, registry, scope, &imports, &shadow);
                    prefix_expr(&expr, prefix, &outers)
                }
            };
            let worked = expand(&expr, &shapes, registry, scope, &imports, 0).and_then(|worked| {
                records_written_out(worked, &shapes, registry, &|e| {
                    expand(e, &shapes, registry, scope, &imports, 0)
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
                    let value =
                        substitute_class_constants(value, registry, scope, &imports, &shadow);
                    let value = prefix_expr(&value, prefix, &outers);
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
                            &imports,
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

    // Equations: arrays expanded, subscripts resolved, calls inlined.
    let expand_here = |expr: &Expr, loop_vars: &HashMap<String, f64>| -> Result<Value, String> {
        let expr = substitute_class_constants(expr, registry, scope, &imports, &shadow);
        let expr = prefix_expr(&expr, prefix, &outers);
        let shapes = Shapes {
            sizes: &sizes_here,
            loop_vars,
            consts: &local_consts,
            records: &records_here,
        };
        let value = expand(&expr, &shapes, registry, scope, &imports, 0)?;
        records_written_out(value, &shapes, registry, &|e| {
            expand(e, &shapes, registry, scope, &imports, 0)
        })
    };
    // What has to come to one value - a condition, what a `when` gives
    // a variable - still goes through the array layer to get there:
    // `min({pre(y), u, pre(u)})` is one value made out of three, and
    // only the array layer knows how to read it.
    let resolve_here =
        |expr: &Expr| -> Result<Expr, String> { expand_here(expr, &HashMap::new())?.scalar() };
    let no_loop_vars = HashMap::new();
    // A record-valued variable's value, said now that both sides can
    // be written out as fields. It is an equation because that is what
    // a variable's declaration value is: `Complex vs[m] = plug.pin.v`
    // holds for the whole run.
    // A modifier arrives written in the terms of the class that
    // supplied it - `Shape s(R = mine)` names a record of the class
    // holding `s` - so those have to be in view as well as this
    // class's own. It is put together only where there is a record
    // value to say it of, since every class would otherwise pay for a
    // copy of every record the model holds.
    let records_wider = record_values
        .iter()
        .any(|(_, _, prefixed)| *prefixed)
        .then(|| {
            let mut all = acc.records.clone();
            all.extend(records_here.iter().map(|(k, v)| (k.clone(), v.clone())));
            all
        });
    for (name, value, prefixed) in &record_values {
        let lhs = expand_here(&Expr::Ref(name.clone()), &no_loop_vars)?;
        let rhs = match prefixed {
            // A modifier arrives already written in the terms of the
            // class that supplied it.
            true => {
                let shapes = Shapes {
                    sizes: &sizes_here,
                    loop_vars: &no_loop_vars,
                    consts: &local_consts,
                    records: records_wider.as_ref().unwrap_or(&records_here),
                };
                let worked = expand(value, &shapes, registry, scope, &imports, 0)?;
                records_written_out(worked, &shapes, registry, &|e| {
                    expand(e, &shapes, registry, scope, &imports, 0)
                })?
            }
            false => expand_here(value, &no_loop_vars)?,
        };
        // Where the value cannot be written out as fields - a record
        // named in a class this one knows nothing about - the two
        // sides do not line up, and the value is left where it was
        // rather than half-applied. That is what this compiler did
        // with every record value until now, so nothing is lost by
        // leaving it there.
        let held = acc.equations.len();
        if push_equations(&lhs, &rhs, acc).is_err() {
            acc.equations.truncate(held);
        }
    }
    // `(r, a, b, ku) = lowPass(cr, c0, c1, f_cut)`: one call fills
    // several targets, and a skipped slot costs its output nothing
    // since the expression is never used. A tuple may stand at the top
    // of a class or inside a branch the compiler settles, and it is
    // read the same way in both.
    let tuple_equation = |equation: &EquationItem, acc: &mut Flat| -> Result<bool, String> {
        let Expr::Tuple(targets) = &equation.lhs else {
            return Ok(false);
        };
        let rhs = substitute_class_constants(&equation.rhs, registry, scope, &imports, &shadow);
        let rhs = prefix_expr(&rhs, prefix, &outers);
        let Expr::Call(name, raw_args) = &rhs else {
            return Err("the right side of a tuple equation must be a function call".into());
        };
        // `spatialDistribution` fills a pair the way a function
        // does, but there is no body to inline: what it stands for
        // is a profile the run carries, so it is recorded here and
        // the equation becomes the two boundary values.
        if name == "spatialDistribution" {
            let shapes = Shapes {
                sizes: &sizes_here,
                loop_vars: &no_loop_vars,
                consts: &local_consts,
                records: &records_here,
            };
            let arguments = raw_args
                .iter()
                .map(|arg| Ok(expand(arg, &shapes, registry, scope, &imports, 0)?.into_expr()))
                .collect::<Result<Vec<Expr>, String>>()?;
            // The targets go through the usual pipeline, so the
            // names recorded are the flat ones.
            let mut named = Vec::new();
            for target in targets {
                let Some(target) = target else {
                    named.push(None);
                    continue;
                };
                named.push(Some(expand_here(target, &no_loop_vars)?.into_expr()));
            }
            spatial_transport(&named, &arguments, prefix, &outers, &local_consts, acc)?;
            return Ok(true);
        }
        let function = lookup(registry, name, scope, &imports)
            .filter(|c| c.kind == ClassKind::Function)
            .ok_or_else(|| format!("`{name}` is not a function, so it cannot fill a tuple"))?;
        let shapes = Shapes {
            sizes: &sizes_here,
            loop_vars: &no_loop_vars,
            consts: &local_consts,
            records: &records_here,
        };
        let values = raw_args
            .iter()
            .map(|arg| expand(arg, &shapes, registry, scope, &imports, 0))
            .collect::<Result<Vec<_>, String>>()?;
        let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
        let arguments: Vec<Expr> = values.into_iter().map(|value| value.into_expr()).collect();
        let outputs = inline_function_outputs(
            function,
            &arguments,
            &argument_shapes,
            &local_consts,
            registry,
            0,
        )?;
        if targets.len() > outputs.len() {
            return Err(format!(
                "`{name}` has {} output(s) for {} target(s)",
                outputs.len(),
                targets.len()
            ));
        }
        for (slot, (_, value)) in targets.iter().zip(outputs) {
            let Some(target) = slot else { continue };
            // The target goes through the usual pipeline; the
            // inlined value is already resolved and only needs the
            // array layer, or a second prefix would corrupt it.
            let lhs = expand_here(target, &no_loop_vars)?;
            let rhs = expand(&value, &shapes, registry, scope, &imports, 0)?;
            push_equations(&lhs, &rhs, acc)?;
        }
        Ok(true)
    };
    for equation in &class.equations {
        // `(a, , c) = f(...)`: one call fills several targets. The
        // call is inlined once per output; a skipped slot costs its
        // computation nothing, since the expression is never used.
        if tuple_equation(equation, acc)? {
            continue;
        }
        let lhs = expand_here(&equation.lhs, &no_loop_vars)?;
        let rhs = expand_here(&equation.rhs, &no_loop_vars)?;
        push_equations(&lhs, &rhs, acc)?;
    }

    for (condition, message) in &class.asserts {
        // A check may be written over arrays - `assert(length(n) >
        // 0, ...)` of an axis of three - so it goes through the array
        // layer like an equation, and what comes out is the one truth
        // it has to be.
        let condition = expand_here(condition, &no_loop_vars)?.scalar()?;
        acc.asserts.push((condition, message.clone()));
    }

    // The arrows of a state machine name instances of this class, so
    // they carry its prefix like everything else.
    for transition in &class.transitions {
        acc.transitions.push(Transition {
            from: flat_name(&transition.from, prefix, &outers),
            to: flat_name(&transition.to, prefix, &outers),
            condition: resolve_here(&transition.condition)?,
            reset: transition.reset,
            immediate: transition.immediate,
            synchronize: transition.synchronize,
            priority: transition.priority,
        });
    }
    if let Some(state) = &class.initial_state {
        acc.initial_states.push(flat_name(state, prefix, &outers));
    }
    for equation in &class.initial_equations {
        let (lhs, rhs) = (
            expand_here(&equation.lhs, &no_loop_vars)?,
            expand_here(&equation.rhs, &no_loop_vars)?,
        );
        let (mut left, mut right) = (Vec::new(), Vec::new());
        lhs.flatten_into(&mut left);
        rhs.flatten_into(&mut right);
        if left.len() != right.len() {
            return Err("an initial equation between shapes that do not match".to_string());
        }
        for (lhs, rhs) in left.into_iter().zip(right) {
            acc.initial_equations.push(EquationItem {
                lhs,
                rhs,
                origin: String::new(),
            });
        }
    }

    // An `algorithm` section of a model is executed symbolically: what
    // comes out is one equation per variable it assigns, which is what
    // the rest of the pipeline understands.
    if class.kind != ClassKind::Function && !class.algorithm.is_empty() {
        // A `when` written among the statements is an event, not a
        // step of the algorithm: it becomes a clause of its own and
        // the rest of the section runs without it.
        let mut plain = Vec::new();
        for statement in &class.algorithm {
            let Statement::When(branches) = statement else {
                plain.push(statement.clone());
                continue;
            };
            let mut lifted = Vec::new();
            for branch in branches {
                // The body of a `when` is an algorithm like any other:
                // it may hold an `if`, a loop, or a write to one
                // element. Running it is what says which names it
                // leaves changed and what each of them is worth, and
                // that is exactly what an event does.
                let mut written: HashMap<String, Expr> = HashMap::new();
                let mut order: Vec<String> = Vec::new();
                let mut checked: Vec<(Expr, String)> = Vec::new();
                match execute(
                    &branch.body,
                    &mut written,
                    &mut order,
                    &mut checked,
                    &local_consts,
                    &sizes,
                    registry,
                    scope,
                    &imports,
                    depth,
                    false,
                )? {
                    Flow::Normal => {}
                    Flow::Break => {
                        return Err("`break` outside of a loop, inside a `when`".to_string())
                    }
                    Flow::Return => {
                        return Err(
                            "`return` belongs in a function, not a `when` of a model".to_string()
                        )
                    }
                }
                for (condition, message) in checked {
                    acc.asserts.push((resolve_here(&condition)?, message));
                }
                let mut actions = Vec::new();
                for target in &order {
                    // Every name the run put in the order it also gave
                    // a value; there is nothing to say about one that
                    // is not there.
                    let Some(value) = written.get(target) else {
                        continue;
                    };
                    actions.push(WhenAction::Assign(
                        flat_name(target, prefix, &outers),
                        resolve_here(value)?,
                    ));
                }
                let condition = branch
                    .condition
                    .as_ref()
                    .ok_or_else(|| "a `when` has no `else`".to_string())?;
                lifted.push(WhenBranch {
                    condition: resolve_here(condition)?,
                    actions,
                });
            }
            acc.when_clauses.push(WhenClause {
                branches: lifted,
                origin: acc.origin.clone(),
            });
        }
        let mut bindings: HashMap<String, Expr> = HashMap::new();
        let mut assigned: Vec<String> = Vec::new();
        let mut section_asserts: Vec<(Expr, String)> = Vec::new();
        match execute(
            &plain,
            &mut bindings,
            &mut assigned,
            &mut section_asserts,
            &local_consts,
            &sizes,
            registry,
            scope,
            &imports,
            depth,
            false,
        )? {
            Flow::Normal => {}
            Flow::Break => return Err("`break` outside of a loop".to_string()),
            Flow::Return => {
                return Err("`return` belongs in a function, not a model algorithm".to_string())
            }
        }
        // Checks written among the statements are checks of the model,
        // and carry its prefix like everything else it says.
        for (condition, message) in section_asserts {
            acc.asserts.push((resolve_here(&condition)?, message));
        }
        for name in assigned {
            let value = bindings
                .get(&name)
                .ok_or_else(|| format!("`{name}` is assigned by the algorithm but has no value"))?;
            // Both sides may be arrays: `w := v .* k` assigns a whole
            // one, and comes out as one equation per element.
            push_equations(
                &expand_here(&Expr::Ref(name.clone()), &no_loop_vars)?,
                &expand_here(value, &no_loop_vars)?,
                acc,
            )?;
        }
    }

    // `initial algorithm` runs once, before the simulation starts. It
    // is executed the same way, and what it assigns is an equation of
    // the initial system: the statements are what decide where the
    // variables begin rather than how they move.
    if class.kind != ClassKind::Function && !class.initial_algorithm.is_empty() {
        if class
            .initial_algorithm
            .iter()
            .any(|s| matches!(s, Statement::When(_)))
        {
            return Err(
                "a `when` belongs in an algorithm that runs, not an initial one".to_string(),
            );
        }
        let mut bindings: HashMap<String, Expr> = HashMap::new();
        let mut assigned: Vec<String> = Vec::new();
        let mut section_asserts: Vec<(Expr, String)> = Vec::new();
        match execute(
            &class.initial_algorithm,
            &mut bindings,
            &mut assigned,
            &mut section_asserts,
            &local_consts,
            &sizes,
            registry,
            scope,
            &imports,
            depth,
            false,
        )? {
            Flow::Normal => {}
            Flow::Break => return Err("`break` outside of a loop".to_string()),
            Flow::Return => {
                return Err("`return` belongs in a function, not a model algorithm".to_string())
            }
        }
        for (condition, message) in section_asserts {
            acc.asserts.push((resolve_here(&condition)?, message));
        }
        // `push_equations` writes where ordinary equations go; what it
        // wrote here is the initial system, so it is moved across.
        let boundary = acc.equations.len();
        for name in assigned {
            let value = bindings.get(&name).ok_or_else(|| {
                format!("`{name}` is assigned by the initial algorithm but has no value")
            })?;
            push_equations(
                &expand_here(&Expr::Ref(name.clone()), &no_loop_vars)?,
                &expand_here(value, &no_loop_vars)?,
                acc,
            )?;
        }
        let written: Vec<EquationItem> = acc.equations.drain(boundary..).collect();
        acc.initial_equations.extend(written);
    }

    // A call written among the equations takes nothing back from what
    // it calls, so what it is there for is the checks the body makes.
    // They become the model's, carrying this instance's prefix like
    // everything else it says.
    let take_checks = |call: &Expr, acc: &mut Flat| -> Result<(), String> {
        let call = substitute_class_constants(call, registry, scope, &imports, &shadow);
        let call = prefix_expr(&call, prefix, &outers);
        let Expr::Call(name, args) = &call else {
            return Err("a line of an equation section that is not an equation is a call".into());
        };
        let called = lookup(registry, name, scope, &imports)
            .filter(|c| c.kind == ClassKind::Function)
            .ok_or_else(|| format!("`{name}` is not a function"))?;
        let shapes = Shapes {
            sizes: &sizes_here,
            loop_vars: &no_loop_vars,
            consts: &local_consts,
            records: &records_here,
        };
        let values = args
            .iter()
            .map(|arg| expand(arg, &shapes, registry, scope, &imports, 0))
            .collect::<Result<Vec<_>, String>>()?;
        let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
        let arguments: Vec<Expr> = values.into_iter().map(|value| value.into_expr()).collect();
        let checks = inline_function_checks(
            called,
            &arguments,
            &argument_shapes,
            &local_consts,
            registry,
            0,
        )?;
        acc.asserts.extend(checks);
        Ok(())
    };
    for call in &class.calls {
        take_checks(call, acc)?;
    }

    // `for` equations are unrolled: the loop variable is a constant.
    for loop_eq in &class.for_equations {
        unroll(
            loop_eq,
            &HashMap::new(),
            &local_consts,
            prefix,
            &outers,
            &sizes_here,
            &records_here,
            registry,
            scope,
            &imports,
            acc,
        )?;
    }

    // What a branch the compiler picked says about events joins what
    // the class says outright: `if use_reset then when reset then
    // reinit(y, y_start); end when; end if;` is how the standard
    // library gives a block a reset it can be built without.
    // What the pass before this one gathered about the connections:
    // the roots of the overconstrained graph, and how many `connect`
    // equations named each port. `answered` is what says a pass has
    // been made - a model with no overconstrained loop has no roots in
    // earnest, so emptiness says nothing.
    let known_roots = acc.roots.clone();
    let known_counts = acc.counts.clone();
    let answered = acc.answered;
    let mut whens_from_branches: Vec<&WhenClause> = Vec::new();
    let mut graph_from_branches: Vec<&GraphClause> = Vec::new();
    // `if` equations: the branch that holds contributes its equations,
    // the others contribute nothing. Conditions are structural, so they
    // must be constant at compile time.
    for if_equation in &class.if_equations {
        let mut env = acc.const_values.clone();
        env.extend(local_consts.iter().map(|(k, v)| (k.clone(), *v)));
        // A structural condition picks one branch and the model is
        // built from it. A condition only the run holds decides
        // nothing here, so every branch must contribute the same
        // number of equations and each position becomes one equation
        // that chooses its residual as it goes.
        // A condition is read with the constants of the classes it
        // names put in first: `smoothness == Smoothness.LinearSegments`
        // compares a parameter against an enumeration literal, and
        // neither is a name the environment holds on its own. A
        // question about the connection graph is answered from the
        // roots, which are in hand on the pass that follows the one
        // that drew them.
        let settle = |condition: &Expr| {
            let named = substitute_class_constants(condition, registry, scope, &imports, &[]);
            if let Some(value) = const_eval(&named, &env) {
                return Some(value);
            }
            if !answered {
                return None;
            }
            let asked = prefix_expr(&named, prefix, &outers);
            let told = answer_graph_queries(&asked, &known_roots, &known_counts);
            const_eval(&told, &env)
        };
        let decidable = if_equation.branches.iter().all(|branch| {
            branch
                .condition
                .as_ref()
                .is_none_or(|condition| settle(condition).is_some())
        });
        // A condition that asks the graph where the graph has not been
        // drawn cannot be answered here, and the branches of such an
        // `if` are not balanced - a body that is a root carries states
        // and one that is not carries none. So it is set aside, and
        // the whole model is built again once the graph is in.
        if !decidable && !answered && asks_the_graph(if_equation) {
            acc.graph_asked = true;
            continue;
        }
        if !decidable {
            push_conditional(
                if_equation,
                &class.name,
                resolve_here,
                expand_here,
                &no_loop_vars,
                acc,
            )?;
            continue;
        }
        let mut chosen = None;
        for branch in &if_equation.branches {
            match &branch.condition {
                None => {
                    chosen = Some(branch);
                    break;
                }
                Some(condition) => {
                    let value = settle(condition).ok_or_else(|| {
                        format!(
                            "condition of an `if` equation in `{}` is not a compile-time constant",
                            class.name
                        )
                    })?;
                    if value != 0.0 {
                        chosen = Some(branch);
                        break;
                    }
                }
            }
        }
        let Some(branch) = chosen else { continue };
        // The branch is the one taken, so its checks hold outright.
        for (condition, message) in &branch.asserts {
            acc.asserts
                .push((resolve_here(condition)?, message.clone()));
        }
        whens_from_branches.extend(branch.whens.iter());
        graph_from_branches.extend(branch.graph.iter());
        for call in &branch.calls {
            take_checks(call, acc)?;
        }
        for loop_eq in &branch.loops {
            unroll(
                loop_eq,
                &HashMap::new(),
                &local_consts,
                prefix,
                &outers,
                &sizes_here,
                &records_here,
                registry,
                scope,
                &imports,
                acc,
            )?;
        }
        for equation in &branch.equations {
            if tuple_equation(equation, acc)? {
                continue;
            }
            push_equations(
                &expand_here(&equation.lhs, &no_loop_vars)?,
                &expand_here(&equation.rhs, &no_loop_vars)?,
                acc,
            )?;
        }
        for (a, b) in &branch.connects {
            let shapes = Shapes {
                sizes: &sizes_here,
                loop_vars: &no_loop_vars,
                consts: &local_consts,
                records: &records_here,
            };
            push_connects(
                a, b, &shapes, prefix, &outers, registry, scope, &imports, acc,
            )?;
        }
    }

    // What the class says about the overconstrained graph, and what
    // the branches the compiler picked said about it.
    for clause in class
        .connection_graph
        .iter()
        .chain(graph_from_branches.iter().copied())
    {
        acc.connection_graph.push(match clause {
            GraphClause::Root(node) => GraphClause::Root(flat_name(node, prefix, &outers)),
            GraphClause::PotentialRoot(node, priority) => {
                GraphClause::PotentialRoot(flat_name(node, prefix, &outers), *priority)
            }
            GraphClause::Branch(a, b) => {
                GraphClause::Branch(flat_name(a, prefix, &outers), flat_name(b, prefix, &outers))
            }
        });
    }

    for clause in class.when_clauses.iter().chain(whens_from_branches) {
        let mut branches = Vec::new();
        for branch in &clause.branches {
            let mut actions = Vec::new();
            for action in &branch.actions {
                match action {
                    WhenAction::Reinit(state, value) => actions.push(WhenAction::Reinit(
                        flat_name(state, prefix, &outers),
                        resolve_here(value)?,
                    )),
                    // A `when` may give a whole array at once -
                    // `y = u` between two vectors is how the clocked
                    // samplers pass a bus through - and an event
                    // assigns one variable, so it is taken apart the
                    // way an equation between arrays is: one
                    // assignment per element, refusing sides that do
                    // not have the same shape. A scalar target goes
                    // the short way, which is every other `when` in
                    // the library.
                    WhenAction::Assign(target, value) => {
                        let named = flat_name(target, prefix, &outers);
                        let given = expand_here(value, &HashMap::new())?;
                        // The shapes are filed under the full path, and
                        // the target is written as this class named it,
                        // so it is the flat name that finds one.
                        match sizes_here.get(&named).filter(|shape| !shape.is_empty()) {
                            None => actions.push(WhenAction::Assign(named, given.scalar()?)),
                            Some(shape) => {
                                let mut elements = Vec::new();
                                given.flatten_into(&mut elements);
                                let wanted: usize =
                                    shape.iter().map(|length| *length as usize).product();
                                if elements.len() != wanted {
                                    return Err(format!(
                                        "`{named}` is given {} value(s) at an event and holds                                          {wanted}",
                                        elements.len()
                                    ));
                                }
                                for (indices, one) in index_tuples(shape).into_iter().zip(elements)
                                {
                                    actions.push(WhenAction::Assign(
                                        element_name(&named, &indices),
                                        one,
                                    ));
                                }
                            }
                        }
                    }
                    WhenAction::Terminate(message) => {
                        actions.push(WhenAction::Terminate(message.clone()))
                    }
                    // A check made when the event fires: the names it
                    // was written with are this class's, so it is
                    // resolved here like any other expression.
                    WhenAction::Assert(condition, message) => actions.push(WhenAction::Assert(
                        expand_here(&resolve_here(condition)?, &HashMap::new())?.scalar()?,
                        message.clone(),
                    )),
                    // `if c then x = a; else x = b; end if;` at an
                    // event: what `x` is given depends on the
                    // condition, so it gets one assignment whose value
                    // is the choice. A branch that says nothing about
                    // a variable leaves it what it had, which is what
                    // `pre` of it is.
                    WhenAction::Choice(chosen) => {
                        let mut targets: Vec<String> = Vec::new();
                        let mut branches: Vec<GivenBranch> = Vec::new();
                        for branch in &chosen.branches {
                            // A connection is drawn once and for all,
                            // and a check has nowhere to go from here;
                            // what an `if` at an event holds is values.
                            if !branch.connects.is_empty()
                                || !branch.loops.is_empty()
                                || !branch.asserts.is_empty()
                                || !branch.whens.is_empty()
                                || !branch.calls.is_empty()
                                || !branch.graph.is_empty()
                            {
                                return Err(
                                    "an `if` inside `when` gives values to variables".to_string()
                                );
                            }
                            let mut given = Vec::new();
                            for equation in &branch.equations {
                                let Expr::Ref(target) = &equation.lhs else {
                                    return Err("an `if` inside `when` gives values to variables"
                                        .to_string());
                                };
                                let target = flat_name(target, prefix, &outers);
                                if !targets.contains(&target) {
                                    targets.push(target.clone());
                                }
                                given.push((target, resolve_here(&equation.rhs)?));
                            }
                            let condition =
                                branch.condition.as_ref().map(&resolve_here).transpose()?;
                            branches.push((condition, given));
                        }
                        for target in targets {
                            // Built from the last branch back, so the
                            // conditions are tested in the order they
                            // were written.
                            let mut value =
                                Expr::Call("pre".to_string(), vec![Expr::Ref(target.clone())]);
                            for (condition, given) in branches.iter().rev() {
                                let Some(chosen) = given.iter().find(|(name, _)| name == &target)
                                else {
                                    continue;
                                };
                                value = match condition {
                                    None => chosen.1.clone(),
                                    Some(condition) => Expr::If(
                                        Box::new(condition.clone()),
                                        Box::new(chosen.1.clone()),
                                        Box::new(value),
                                    ),
                                };
                            }
                            actions.push(WhenAction::Assign(target, value));
                        }
                    }
                    // `for i in 1:n loop k[i] = ...; end for;` at an
                    // event: the loop is unrolled the way one among
                    // the equations is, and each round's equation
                    // becomes an assignment of its own. It is unrolled
                    // into the equations and taken straight back out,
                    // there being one unroller and no reason for two.
                    WhenAction::Loop(loop_eq) => {
                        let boundary = acc.equations.len();
                        let drawn = acc.connects.len();
                        unroll(
                            loop_eq,
                            &HashMap::new(),
                            &local_consts,
                            prefix,
                            &outers,
                            &sizes_here,
                            &records_here,
                            registry,
                            scope,
                            &imports,
                            acc,
                        )?;
                        if acc.connects.len() != drawn {
                            return Err("a loop inside `when` assigns variables, one per round; \
                                        a connection is drawn once and for all, not at an event"
                                .to_string());
                        }
                        for round in acc.equations.drain(boundary..).collect::<Vec<_>>() {
                            let Expr::Ref(target) = round.lhs else {
                                return Err(
                                    "a loop inside `when` assigns variables, one per round"
                                        .to_string(),
                                );
                            };
                            actions.push(WhenAction::Assign(target, round.rhs));
                        }
                    }
                    // `(a, b) = f(x)` at an event: the call is inlined
                    // once per output, and each target gets an
                    // assignment of its own. A skipped slot costs its
                    // output nothing, since it is never used.
                    WhenAction::TupleAssign(targets, value) => {
                        // Not through `resolve_here`: that would inline
                        // the call into the one value an expression can
                        // carry, and what is wanted here is the call
                        // itself, to be inlined once per target.
                        let value =
                            substitute_class_constants(value, registry, scope, &imports, &shadow);
                        let value = prefix_expr(&value, prefix, &outers);
                        let Expr::Call(name, raw_args) = &value else {
                            return Err(
                                "the right side of a tuple inside `when` must be a function call"
                                    .to_string(),
                            );
                        };
                        let function = lookup(registry, name, scope, &imports)
                            .filter(|c| c.kind == ClassKind::Function)
                            .ok_or_else(|| {
                                format!("`{name}` is not a function, so it cannot fill a tuple")
                            })?;
                        let shapes = Shapes {
                            sizes: &sizes_here,
                            loop_vars: &no_loop_vars,
                            consts: &local_consts,
                            records: &records_here,
                        };
                        let values = raw_args
                            .iter()
                            .map(|arg| expand(arg, &shapes, registry, scope, &imports, 0))
                            .collect::<Result<Vec<_>, String>>()?;
                        let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
                        let arguments: Vec<Expr> =
                            values.into_iter().map(|value| value.into_expr()).collect();
                        let outputs = inline_function_outputs(
                            function,
                            &arguments,
                            &argument_shapes,
                            &local_consts,
                            registry,
                            0,
                        )?;
                        if outputs.len() < targets.len() {
                            return Err(format!(
                                "`{name}` has {} output(s) and the tuple asks for {}",
                                outputs.len(),
                                targets.len()
                            ));
                        }
                        for (target, (_, worth)) in targets.iter().zip(outputs) {
                            let Some(target) = target else { continue };
                            let target = flat_name(target, prefix, &outers);
                            // An output of several numbers lands on
                            // several names: a generator answers with
                            // the state it moved to, and the model
                            // holds one number per name.
                            let placed = expand(
                                &Expr::Ref(target.clone()),
                                &shapes,
                                registry,
                                scope,
                                &imports,
                                0,
                            )?;
                            let (mut names, mut worths) = (Vec::new(), Vec::new());
                            placed.flatten_into(&mut names);
                            expand(&worth, &shapes, registry, scope, &imports, 0)?
                                .flatten_into(&mut worths);
                            if names.len() != worths.len() {
                                return Err(format!(
                                    "`{target}` is {} name(s) and what it is given at the \
                                     event is {} value(s)",
                                    names.len(),
                                    worths.len()
                                ));
                            }
                            for (name, worth) in names.into_iter().zip(worths) {
                                let Expr::Ref(name) = name else {
                                    return Err(format!(
                                        "`{target}` is not a name an event can assign"
                                    ));
                                };
                                actions.push(WhenAction::Assign(name, worth));
                            }
                        }
                    }
                }
            }
            branches.push(WhenBranch {
                condition: resolve_here(&branch.condition)?,
                actions,
            });
        }
        acc.when_clauses.push(WhenClause {
            branches,
            origin: acc.origin.clone(),
        });
    }
    // A connection to a component that a condition left out goes with
    // it: this is how the standard library switches a support flange
    // between an external connector and an internal ground.
    for (a, b) in &class.connects {
        // `break connect(a, b)` drops this exact connection, in either
        // order, before it becomes a set.
        let (na, nb) = (connect_side_name(a), connect_side_name(b));
        if let Some(index) = broken.iter().position(|item| {
            matches!(item, Deselect::Connection(x, y)
                if (Some(x) == na.as_ref() && Some(y) == nb.as_ref())
                    || (Some(x) == nb.as_ref() && Some(y) == na.as_ref()))
        }) {
            broke_something[index] = true;
            continue;
        }
        let shapes = Shapes {
            sizes: &sizes_here,
            loop_vars: &no_loop_vars,
            consts: &local_consts,
            records: no_records(),
        };
        push_connects(
            a, b, &shapes, prefix, &outers, registry, scope, &imports, acc,
        )?;
    }

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

/// Whether a condition asks the connections a question.
///
/// `Connections.isRoot(frame_a.R)` and `Connections.rooted(...)` are
/// answered from the roots the graph was broken open at, and
/// `cardinality(port)` from how many `connect` equations named the
/// port. Both are gathered by building the model, so until one pass
/// has been made there is nothing to answer with.
fn asks_the_connections(condition: &Expr) -> bool {
    let mut found = false;
    walk_calls(condition, &mut |name| {
        if name == "Connections.isRoot" || name == "Connections.rooted" || name == "cardinality" {
            found = true;
        }
    });
    found
}

/// Whether an `if` equation asks the connections a question.
fn asks_the_graph(if_equation: &IfEquation) -> bool {
    if_equation
        .branches
        .iter()
        .any(|branch| branch.condition.as_ref().is_some_and(asks_the_connections))
}

/// Which of the instances below a class are records, and of what.
///
/// An overloaded operator is chosen by the record its operands are of,
/// and an equation between records is one equation per member - both
/// need to know a record when they see one. The walk goes down through
/// the whole tree because a record is as often a member of something
/// as a component outright: a frame of a multibody model carries its
/// orientation as `frame_b.R`.
fn collect_records(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    scope: &str,
    imports: &[(String, String)],
    out: &mut HashMap<String, String>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, scope, imports) {
            collect_records(
                registry,
                base,
                prefix,
                &base.name,
                &base.imports,
                out,
                depth + 1,
            );
        }
    }
    for component in &class.components {
        // A type may be a name for a record - `connector ComplexOutput
        // = output Complex` - and a declaration of it is a record all
        // the same.
        let mut component = component.clone();
        resolve_type(registry, &mut component, scope, imports);
        let Some(of) = lookup(registry, &component.type_name, scope, imports) else {
            continue;
        };
        let name = format!("{prefix}{}", component.name);
        if of.kind == ClassKind::Record {
            out.insert(name.clone(), of.name.clone());
        }
        if matches!(
            of.kind,
            ClassKind::Record | ClassKind::Model | ClassKind::Block | ClassKind::Connector
        ) {
            let below = format!("{name}.");
            collect_records(registry, of, &below, &of.name, &of.imports, out, depth + 1);
        }
    }
}

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

/// Whether a class is a handle to something outside Modelica: itself
/// `ExternalObject`, or built on one.
pub(super) fn descends_from_external_object(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    depth: usize,
) -> bool {
    if class.name == "ExternalObject" {
        return true;
    }
    if depth > MAX_DEPTH {
        return false;
    }
    class.extends.iter().any(|extend| {
        lookup(registry, &extend.base, &class.name, &class.imports)
            .is_some_and(|base| descends_from_external_object(registry, base, depth + 1))
    })
}

/// The length of a value along one axis, for a flexible `:` size. Only
/// an array literal is measured: a `:` size on a model component is
/// read from a value written out in full.
pub(super) fn flexible_size(binding: &Expr, axis: usize) -> Option<i64> {
    let mut here = binding;
    for _ in 0..axis {
        here = match here {
            Expr::Array(items) => items.first()?,
            _ => return None,
        };
    }
    match here {
        Expr::Array(items) => Some(items.len() as i64),
        _ => None,
    }
}

/// A connect side written back as the dotted name a `break
/// connect(...)` would name it by, when it is a plain reference. A
/// subscripted or otherwise compound side is left unnamed, so a break
/// only matches what it can spell.
fn connect_side_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ref(name) => Some(name.clone()),
        Expr::Member(base, member) => Some(format!("{}.{member}", connect_side_name(base)?)),
        _ => None,
    }
}

/// One element's slice of a modifier value spread over an array.
///
/// A value written as a list of the right length is handed out one
/// entry per element - `items[3](w = {1, 2, 3})` gives each its own -
/// while anything else, a scalar most of all, reaches every element
/// whole.
#[allow(clippy::too_many_arguments)]
fn array_element(
    value: &Expr,
    position: usize,
    count: usize,
    sizes: &HashMap<String, Vec<i64>>,
    consts: &HashMap<String, f64>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
) -> Expr {
    // A literal says its length outright; a name has to be measured,
    // which is how `cells(k = ks)` hands `cells[i].k` the value
    // `ks[i]`. Anything that does not come to one value per element is
    // handed over whole: a scalar spreads, and a modifier reaching
    // into a member of the element is not an array at this level.
    if let Expr::Array(items) = value {
        if items.len() == count {
            return items[position].clone();
        }
    }
    let shapes = Shapes {
        sizes,
        loop_vars: &HashMap::new(),
        consts,
        records: no_records(),
    };
    let mark = checks_mark();
    let measured = expand(value, &shapes, registry, scope, imports, 0);
    // Slicing a modifier is a measurement rather than the model asking
    // for the value, and the value itself is expanded again where it
    // lands; keeping the checks here would count them twice.
    checks_rewind(mark);
    let Ok(measured) = measured else {
        return value.clone();
    };
    // The slice is taken along the outermost dimension: an element of
    // `cylinders[2]` gets one of the two values, and that value may
    // itself be a vector of three.
    match measured {
        Value::Array(items) if items.len() == count => items[position].clone().into_expr(),
        _ => value.clone(),
    }
}

/// One branch of an `if` at an event: its condition, where it has one,
/// and what each variable it names is given.
type GivenBranch = (Option<Expr>, Vec<(String, Expr)>);

/// What a class written `redeclare X extends Name` extends: the `Name`
/// that a base of the class enclosing it declared.
///
/// Looking the name up the ordinary way finds the class doing the
/// redeclaring, since that is what it is called; what is wanted is the
/// one it replaces, and that lives in a base of the class it is
/// written in.
pub(super) fn inherited_class<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    class: &ClassDef,
    wanted: &str,
    depth: usize,
) -> Option<&'a ClassDef> {
    if depth > MAX_DEPTH {
        return None;
    }
    let (enclosing, _) = class.name.rsplit_once('.')?;
    let owner = registry.get(enclosing)?;
    from_bases(registry, owner, wanted, &class.name, depth)
}

/// The class of a name declared by a base of `owner`, or by a base of
/// one of those, skipping the class that is asking.
fn from_bases<'a>(
    registry: &HashMap<&'a str, &'a ClassDef>,
    owner: &ClassDef,
    wanted: &str,
    asking: &str,
    depth: usize,
) -> Option<&'a ClassDef> {
    if depth > MAX_DEPTH {
        return None;
    }
    for extend in &owner.extends {
        let Some(base) = lookup(registry, &extend.base, &owner.name, &owner.imports) else {
            continue;
        };
        if let Some(found) =
            lookup(registry, wanted, &base.name, &base.imports).filter(|found| found.name != asking)
        {
            return Some(found);
        }
        if let Some(found) = from_bases(registry, base, wanted, asking, depth + 1) {
            return Some(found);
        }
    }
    None
}

/// The parameters and constants a class inherits, each with what the
/// `extends` clause that brought it said about its value.
///
/// A dimension may be written on one of them - `extends TwoPlug` brings
/// `m`, and `parameter Voltage V[m]` is written with it - so they have
/// to be worth a number here, before the dimensions are counted.
fn inherited_parameters(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    depth: usize,
) -> Vec<(Component, Option<Expr>)> {
    let mut out = Vec::new();
    if depth > MAX_DEPTH {
        return out;
    }
    for extend in &class.extends {
        let Some(base) = lookup(registry, &extend.base, &class.name, &class.imports) else {
            continue;
        };
        for component in &base.components {
            if !matches!(
                component.variability,
                Variability::Parameter | Variability::Constant
            ) {
                continue;
            }
            let said = extend
                .modifiers
                .iter()
                .find(|(name, _)| name == &component.name)
                .map(|(_, value)| value.clone());
            out.push((component.clone(), said));
        }
        out.extend(inherited_parameters(registry, base, depth + 1));
    }
    out
}

/// The values a loop variable takes, from whatever the range expanded
/// to. A range, a set and an array all expand to the same thing - the
/// values, in order - so there is nothing to tell apart here.
pub(super) fn loop_values(
    spread: &Value,
    env: &HashMap<String, f64>,
    variable: &str,
) -> Result<Vec<f64>, String> {
    let Value::Array(items) = spread else {
        return Err(format!(
            "`{variable}` needs something to run over - a range, a set or an array - and \
             a single value is not one"
        ));
    };
    items
        .iter()
        .map(|item| {
            let expr = item.clone().scalar()?;
            const_eval(&expr, env).ok_or_else(|| {
                format!("`{variable}` runs over values the compiler cannot work out: {expr:?}")
            })
        })
        .collect()
}

/// What `for i loop` runs over, which the body has to say: the size of
/// the array along the dimension `i` is used to subscript.
pub(super) fn implied_range(
    body: &[ForBody],
    variable: &str,
    prefix: &str,
    outers: &HashMap<String, String>,
    sizes: &HashMap<String, Vec<i64>>,
) -> Result<Vec<f64>, String> {
    let mut found = None;
    for item in body {
        let mut look = |expr: &Expr| {
            if found.is_none() {
                found = subscript_extent(&prefix_expr(expr, prefix, outers), variable, sizes);
            }
        };
        match item {
            ForBody::Equation(equation) => {
                look(&equation.lhs);
                look(&equation.rhs);
            }
            ForBody::Connect(a, b) => {
                look(a);
                look(b);
            }
            // A branch says what it holds the same way the loop does.
            ForBody::Branch(if_equation) => {
                for branch in &if_equation.branches {
                    for equation in &branch.equations {
                        look(&equation.lhs);
                        look(&equation.rhs);
                    }
                }
            }
            ForBody::Nested(inner) => {
                if found.is_none() {
                    found = implied_range(&inner.body, variable, prefix, outers, sizes)
                        .ok()
                        .map(|values| values.len() as i64);
                }
            }
            ForBody::Assert(condition, _) => look(condition),
        }
    }
    let Some(extent) = found else {
        return Err(format!(
            "`for {variable} loop` leaves the range to the body, and nothing in the body \
             uses `{variable}` to subscript an array of a length the compiler knows"
        ));
    };
    Ok((1..=extent).map(|index| index as f64).collect())
}

/// How long an array is along the dimension a name is used to subscript
/// it by, looking through a whole expression for the first such use.
pub(super) fn subscript_extent(
    expr: &Expr,
    variable: &str,
    sizes: &HashMap<String, Vec<i64>>,
) -> Option<i64> {
    let recur = |inner: &Expr| subscript_extent(inner, variable, sizes);
    match expr {
        Expr::Index(base, subscripts) => {
            if let Expr::Ref(name) = base.as_ref() {
                if let Some(shape) = sizes.get(name) {
                    let along = subscripts.iter().position(
                        |subscript| matches!(subscript, Expr::Ref(used) if used == variable),
                    );
                    if let Some(dimension) = along.and_then(|at| shape.get(at)) {
                        return Some(*dimension);
                    }
                }
            }
            recur(base).or_else(|| subscripts.iter().find_map(recur))
        }
        Expr::Call(_, args) | Expr::Array(args) => args.iter().find_map(recur),
        Expr::Neg(inner) | Expr::Not(inner) | Expr::Member(inner, _) => recur(inner),
        Expr::Bin(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r)
        | Expr::Elementwise(_, l, r) => recur(l).or_else(|| recur(r)),
        Expr::If(c, t, e) => recur(c).or_else(|| recur(t)).or_else(|| recur(e)),
        Expr::MatrixRows(rows) => rows.iter().find_map(|row| row.iter().find_map(recur)),
        _ => None,
    }
}

/// Unroll a `for` equation, recursing into nested loops. The loop
/// variable is a compile-time constant, so the body is emitted once per
/// value with every subscript already resolved.
#[allow(clippy::too_many_arguments)]
pub(super) fn unroll(
    loop_eq: &ForEquation,
    outer_vars: &HashMap<String, f64>,
    consts: &HashMap<String, f64>,
    prefix: &str,
    outers: &HashMap<String, String>,
    sizes: &HashMap<String, Vec<i64>>,
    records: &HashMap<String, String>,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    acc: &mut Flat,
) -> Result<(), String> {
    // Everything in the loop is prefixed before it is folded, so a
    // parameter of the class the loop is written in - the `n` of `for i
    // in 1:n` or of a guard `if i < n` - has to be findable under its
    // instance path as well as under its plain name.
    let consts: HashMap<String, f64> = consts
        .iter()
        .map(|(name, value)| (name.clone(), *value))
        .chain(
            consts
                .iter()
                .map(|(name, value)| (format!("{prefix}{name}"), *value)),
        )
        .collect();
    let consts = &consts;
    let values = match &loop_eq.range {
        Some(range) => {
            let mut env = consts.clone();
            env.extend(outer_vars.iter().map(|(k, v)| (k.clone(), *v)));
            // A range may ask an array how long it is, so it goes
            // through the same expansion as everything else - which is
            // also what turns `1:n`, `{1, 3, 5}` and the name of an
            // array into one thing: the values, in order.
            let shapes = Shapes {
                sizes,
                loop_vars: outer_vars,
                consts,
                records: no_records(),
            };
            // The bounds may be constants of a package the class is
            // written inside, and those are put in before the class's
            // own prefix goes on: `1:nX` counts a medium's substances.
            let range = substitute_class_constants(range, registry, scope, imports, &[]);
            let spread = expand(
                &prefix_expr(&range, prefix, outers),
                &shapes,
                registry,
                scope,
                imports,
                0,
            )?;
            loop_values(&spread, &env, &loop_eq.variable)?
        }
        None => implied_range(&loop_eq.body, &loop_eq.variable, prefix, outers, sizes)?,
    };
    for index in values {
        let mut loop_vars = outer_vars.clone();
        loop_vars.insert(loop_eq.variable.clone(), index);
        // The loop variable is a compile-time number, not a component,
        // and it is folded in before anything is prefixed: prefixing
        // reaches into subscripts too, and `x[i]` inside a component
        // would otherwise be asking for `a.i`.
        let folded: HashMap<String, Expr> = loop_vars
            .iter()
            .map(|(name, value)| (name.clone(), Expr::Number(*value)))
            .collect();
        for item in &loop_eq.body {
            match item {
                ForBody::Equation(equation) => {
                    let shapes = Shapes {
                        sizes,
                        loop_vars: &loop_vars,
                        consts,
                        records,
                    };
                    let side = |expr: &Expr| -> Result<Value, String> {
                        let expr = substitute_refs(expr, &folded);
                        let expr = substitute_class_constants(&expr, registry, scope, imports, &[]);
                        let value = expand(
                            &prefix_expr(&expr, prefix, outers),
                            &shapes,
                            registry,
                            scope,
                            imports,
                            0,
                        )?;
                        records_written_out(value, &shapes, registry, &|e| {
                            expand(e, &shapes, registry, scope, imports, 0)
                        })
                    };
                    push_equations(&side(&equation.lhs)?, &side(&equation.rhs)?, acc)?;
                }
                ForBody::Connect(a, b) => {
                    let shapes = Shapes {
                        sizes,
                        loop_vars: &loop_vars,
                        consts,
                        records,
                    };
                    let (a, b) = (substitute_refs(a, &folded), substitute_refs(b, &folded));
                    push_connects(
                        &a, &b, &shapes, prefix, outers, registry, scope, imports, acc,
                    )?;
                }
                ForBody::Nested(inner) => unroll(
                    inner, &loop_vars, consts, prefix, outers, sizes, records, registry, scope,
                    imports, acc,
                )?,
                // An `if` inside a loop: the branch that holds gives
                // its equations to the round, and the others give
                // nothing. What decides it is settled here, as it is
                // for an `if` written among the equations of a class.
                ForBody::Branch(if_equation) => {
                    let shapes = Shapes {
                        sizes,
                        loop_vars: &loop_vars,
                        consts,
                        records,
                    };
                    // Everything inside the loop is read with this
                    // round's value of the loop variable already put
                    // in, so what comes out is one round's worth of
                    // this branch and nothing about any other round.
                    let side = |expr: &Expr| -> Result<Value, String> {
                        let expr = substitute_refs(expr, &folded);
                        let expr = substitute_class_constants(&expr, registry, scope, imports, &[]);
                        let value = expand(
                            &prefix_expr(&expr, prefix, outers),
                            &shapes,
                            registry,
                            scope,
                            imports,
                            0,
                        )?;
                        records_written_out(value, &shapes, registry, &|e| {
                            expand(e, &shapes, registry, scope, imports, 0)
                        })
                    };
                    let mut settling = consts.clone();
                    settling.extend(loop_vars.iter().map(|(k, v)| (k.clone(), *v)));
                    // A condition that asks the connections is read
                    // the long way round: the array layer has to run
                    // first, since `cardinality(port[i])` is a
                    // question about one port of an array and it is
                    // the array layer that names it.
                    let (roots, counts, answered) =
                        (acc.roots.clone(), acc.counts.clone(), acc.answered);
                    let settle = |condition: &Expr| {
                        let plain = substitute_refs(condition, &folded);
                        let plain =
                            substitute_class_constants(&plain, registry, scope, imports, &[]);
                        if let Some(value) = const_eval(&plain, &settling) {
                            return Some(value);
                        }
                        if !answered {
                            return None;
                        }
                        let asked = side(condition).ok()?.scalar().ok()?;
                        let told = answer_graph_queries(&asked, &roots, &counts);
                        const_eval(&told, &settling)
                    };
                    let decidable = if_equation.branches.iter().all(|branch| {
                        branch
                            .condition
                            .as_ref()
                            .is_none_or(|condition| settle(condition).is_some())
                    });
                    // Undecidable only because nothing has looked at
                    // the connections yet: the whole model is built
                    // again once one pass has gathered them.
                    if !decidable
                        && !answered
                        && if_equation.branches.iter().any(|branch| {
                            branch.condition.as_ref().is_some_and(asks_the_connections)
                        })
                    {
                        acc.graph_asked = true;
                        continue;
                    }
                    // A condition the run decides makes the same `if`
                    // it would make among the equations of a class -
                    // one equation per position, choosing its residual
                    // as it goes - only written once per round.
                    if !decidable {
                        push_conditional(
                            if_equation,
                            scope,
                            |expr: &Expr| side(expr)?.scalar(),
                            |expr: &Expr, _: &HashMap<String, f64>| side(expr),
                            &HashMap::new(),
                            acc,
                        )?;
                        continue;
                    }
                    let mut chosen = None;
                    for branch in &if_equation.branches {
                        let Some(condition) = &branch.condition else {
                            chosen = Some(branch);
                            break;
                        };
                        if settle(condition) != Some(0.0) {
                            chosen = Some(branch);
                            break;
                        }
                    }
                    let Some(branch) = chosen else { continue };
                    if !branch.whens.is_empty()
                        || !branch.calls.is_empty()
                        || !branch.graph.is_empty()
                    {
                        return Err(format!(
                            "a `when`, a call on its own or a `Connections` clause sits in an \
                             `if` inside a `for` in `{scope}`, and this compiler reads none of \
                             them there"
                        ));
                    }
                    for (a, b) in &branch.connects {
                        let (a, b) = (substitute_refs(a, &folded), substitute_refs(b, &folded));
                        push_connects(
                            &a, &b, &shapes, prefix, outers, registry, scope, imports, acc,
                        )?;
                    }
                    for inner in &branch.loops {
                        unroll(
                            inner, &loop_vars, consts, prefix, outers, sizes, records, registry,
                            scope, imports, acc,
                        )?;
                    }
                    for equation in &branch.equations {
                        push_equations(&side(&equation.lhs)?, &side(&equation.rhs)?, acc)?;
                    }
                    for (condition, message) in &branch.asserts {
                        acc.asserts
                            .push((side(condition)?.scalar()?, message.clone()));
                    }
                }
                ForBody::Assert(condition, message) => {
                    let condition = substitute_refs(condition, &folded);
                    let condition =
                        substitute_class_constants(&condition, registry, scope, imports, &[]);
                    acc.asserts
                        .push((prefix_expr(&condition, prefix, outers), message.clone()));
                }
            }
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

/// The imports a class resolves names through, with its class aliases
/// folded in as further entries.
///
/// `package Medium = Media.Water` makes `Medium.density` mean
/// `Media.Water.density` exactly the way `import Medium = Media.Water`
/// would, so an alias becomes an import entry. A redeclaration from the
/// environment swaps the target before that - checked against the
/// alias's `constrainedby` interface, since a replacement medium has to
/// honour the interface the component was written against.
/// `depth` counts the bases already walked, since a class may name a
/// base that leads back to it.
pub(super) fn effective_imports(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    scope: &str,
    redeclares: &[Redeclare],
    depth: usize,
) -> Result<Vec<(String, String)>, String> {
    let mut imports = class.imports.clone();
    // An alias a base declared is one this class has. `replaceable
    // package Medium` is written once in `PartialSource`, and every
    // boundary of the fluid library extends that rather than repeating
    // it - then names `Medium.AbsolutePressure` in its own
    // declarations. Read from this class's aliases alone the name
    // stands for nothing, and the type it qualifies is unknown.
    if depth <= MAX_DEPTH {
        for extend in &class.extends {
            if let Some(base) = lookup(registry, &extend.base, class.name.as_str(), &imports) {
                for held in effective_imports(registry, base, scope, redeclares, depth + 1)? {
                    if !imports.iter().any(|(local, _)| *local == held.0) {
                        imports.push(held);
                    }
                }
            }
        }
    }
    for alias in &class.class_aliases {
        // A body-level `redeclare package X = ...` replaces an alias of
        // a base class; it is routed through the environment instead.
        if alias.redeclaration {
            continue;
        }
        let replacement = redeclares
            .iter()
            .find(|r| r.class_level && r.name == alias.name);
        let target = match replacement {
            Some(redeclare) => {
                if !alias.replaceable {
                    return Err(format!(
                        "class `{}` of `{}` is redeclared but not declared replaceable",
                        alias.name, class.name
                    ));
                }
                // Qualified already, at the site that wrote it.
                redeclare.type_name.clone()
            }
            None => lookup(registry, &alias.target, scope, &imports)
                .ok_or_else(|| {
                    format!(
                        "unknown class `{}` behind the alias `{}`",
                        alias.target, alias.name
                    )
                })?
                .name
                .clone(),
        };
        if let (Some(constraint), Some(_)) = (&alias.constrained_by, replacement) {
            let constraint = lookup(registry, constraint, scope, &imports).ok_or_else(|| {
                format!(
                    "unknown constraining class `{constraint}` of `{}`",
                    alias.name
                )
            })?;
            if !extends_class(registry, &target, &constraint.name, 0) {
                return Err(format!(
                    "`{target}` cannot replace `{}`: it does not extend `{}`",
                    alias.name, constraint.name
                ));
            }
        }
        imports.push((alias.name.clone(), target));
    }
    Ok(imports)
}

/// Collect the `inner` declarations of a class and of its bases.
pub(super) fn collect_inners(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    out: &mut HashMap<String, InnerInstance>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let scope = class.name.as_str();
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, scope, &class.imports) {
            collect_inners(registry, base, prefix, out, depth + 1);
        }
    }
    for component in class.components.iter().filter(|c| c.scope == Scope::Inner) {
        // A shared instance may be a plain variable as readily as a
        // class: `inner Real v` is what a set of states writes between
        // them, and it is filed under the type it was written with.
        let named = lookup(registry, &component.type_name, scope, &class.imports)
            .map(|declared| declared.name.clone())
            .unwrap_or_else(|| component.type_name.clone());
        out.insert(
            component.name.clone(),
            InnerInstance {
                path: format!("{prefix}{}", component.name),
                class: named,
            },
        );
    }
}

/// Bind the `outer` declarations of a class to the visible `inner`
/// instances, yielding the name-to-path map references go through.
pub(super) fn bind_outers(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    inners: &HashMap<String, InnerInstance>,
) -> Result<HashMap<String, String>, String> {
    bind_outers_at(registry, class, inners, 0)
}

/// See [`bind_outers`]; `depth` counts the bases already walked, since
/// a class may name a base that leads back to it.
fn bind_outers_at(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    inners: &HashMap<String, InnerInstance>,
    depth: usize,
) -> Result<HashMap<String, String>, String> {
    let scope = class.name.as_str();
    let mut outers = HashMap::new();
    if depth > MAX_DEPTH {
        return Ok(outers);
    }
    // What the class inherits it declares. `outer World world` is
    // written once in `PartialTwoFrames` and every joint of the
    // multi-body library extends it rather than repeating it, so a
    // class asked about its own components alone would say it names no
    // `outer` at all - and the equations that read `world.something`
    // would be left pointing at a variable nothing owns. The bases are
    // walked the way `collect_inners` walks them.
    for extend in &class.extends {
        if let Some(base) = lookup(registry, &extend.base, scope, &class.imports) {
            outers.extend(bind_outers_at(registry, base, inners, depth + 1)?);
        }
    }
    for component in class.components.iter().filter(|c| c.scope == Scope::Outer) {
        let inner = inners.get(&component.name).ok_or_else(|| {
            format!(
                "`outer {} {}` in `{}` has no `inner` declaration above it",
                component.type_name, component.name, class.name
            )
        })?;
        // A primitive has no class to look up, and matches by the name
        // it was written with.
        let declared = match lookup(registry, &component.type_name, scope, &class.imports) {
            Some(declared) => declared.name.clone(),
            None => component.type_name.clone(),
        };
        if !extends_class(registry, &inner.class, &declared, 0) {
            return Err(format!(
                "`outer {} {}` does not match the `inner` instance, which is a `{}`",
                component.type_name, component.name, inner.class
            ));
        }
        outers.insert(component.name.clone(), inner.path.clone());
    }
    // A class may also name an `outer` a component of it holds: a
    // composite step reads the count of active steps as
    // `innerState.stateGraphRoot.subgraphStatePort.activeSteps`, where
    // it is `innerState` that declares the `outer`. An `outer` owns no
    // variable of its own, so the name has to be answered here or not
    // at all.
    for component in class.components.iter().filter(|c| c.scope != Scope::Outer) {
        let Some(of) = lookup(registry, &component.type_name, scope, &class.imports) else {
            continue;
        };
        // What that component declares `outer` itself, one level down:
        // going further would follow a class that holds one of its own
        // kind round for ever, and a name written that deep is one the
        // instance below answers for itself.
        for held in of.components.iter().filter(|c| c.scope == Scope::Outer) {
            if let Some(inner) = inners.get(&held.name) {
                outers.insert(
                    format!("{}.{}", component.name, held.name),
                    inner.path.clone(),
                );
            }
        }
    }
    Ok(outers)
}

/// Whether `candidate` is `target` or extends it, directly or not.
pub(super) fn extends_class(
    registry: &HashMap<&str, &ClassDef>,
    candidate: &str,
    target: &str,
    depth: usize,
) -> bool {
    if candidate == target {
        return true;
    }
    if depth > MAX_DEPTH {
        return false;
    }
    let Some(class) = registry.get(candidate) else {
        return false;
    };
    let scope = class.name.as_str();
    class.extends.iter().any(|extend| {
        lookup(registry, &extend.base, scope, &class.imports)
            .is_some_and(|base| extends_class(registry, &base.name, target, depth + 1))
    })
}

/// Prepare a redeclaration for use further down: its type is resolved in
/// the scope where the redeclaration is written, and its modifier
/// expressions are prefixed with the instance path they belong to.
/// `handed` says what the names in view already stand for. A class
/// handing its own replaceable package on - `Port one(redeclare package
/// Medium = Medium)`, which is how every fluid component passes the
/// medium to its ports - names the package by the name it has here, and
/// here it has already been replaced. Looked up among the class's own
/// imports the name is still the interface, so the child was handed the
/// interface and read its constants: a medium carrying no trace
/// substances, and a connector sized by that count a run of nothing.
pub(super) fn qualify_redeclare(
    redeclare: &Redeclare,
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    outers: &HashMap<String, String>,
    handed: &[(String, String)],
) -> Result<Redeclare, String> {
    let scope = class.name.as_str();
    let class_imports: Vec<(String, String)> = handed
        .iter()
        .cloned()
        .chain(class.imports.iter().cloned())
        .collect();
    let target =
        lookup(registry, &redeclare.type_name, scope, &class_imports).ok_or_else(|| {
            format!(
                "unknown type `{}` in the redeclaration of `{}`",
                redeclare.type_name, redeclare.name
            )
        })?;
    Ok(Redeclare {
        name: redeclare.name.clone(),
        type_name: target.name.clone(),
        class_level: redeclare.class_level,
        modifiers: redeclare
            .modifiers
            .iter()
            .map(|(n, e)| {
                let e = substitute_class_constants(e, registry, scope, &class_imports, &[]);
                (n.clone(), prefix_expr(&e, prefix, outers))
            })
            .collect(),
    })
}

/// Check that a declaration may be replaced by a redeclaration: it must
/// be `replaceable`, and the new type must meet the `constrainedby`
/// interface where one is given.
pub(super) fn check_redeclare(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    component: &Component,
    redeclare: &Redeclare,
) -> Result<(), String> {
    if !component.replaceable {
        return Err(format!(
            "`{}` of `{}` is redeclared but not declared replaceable",
            component.name, class.name
        ));
    }
    let Some(constraint) = &component.constrained_by else {
        return Ok(());
    };
    let scope = class.name.as_str();
    let constraint = lookup(registry, constraint, scope, &class.imports).ok_or_else(|| {
        format!(
            "unknown constraining class `{constraint}` of `{}`",
            component.name
        )
    })?;
    if !extends_class(registry, &redeclare.type_name, &constraint.name, 0) {
        return Err(format!(
            "`{}` cannot replace `{}`: it does not extend `{}`",
            redeclare.type_name, component.name, constraint.name
        ));
    }
    Ok(())
}

/// Resolve a declared type down to a primitive: `type` aliases chain,
/// each contributing its attribute defaults, and an enumeration is
/// carried as an `Integer` holding the position of its literal.
pub(super) fn resolve_type(
    registry: &HashMap<&str, &ClassDef>,
    component: &mut Component,
    scope: &str,
    imports: &[(String, String)],
) {
    // A declaration typed by an alias writes its attributes the
    // modifier way - `Units.AngularVelocity w(start = w0)` parses into
    // modifiers, not into the attribute fields. They mean exactly what
    // the attribute form means, and they belong to the declaration, so
    // they take precedence over anything an alias contributes below.
    if lookup(registry, &component.type_name, scope, imports)
        .is_some_and(|class| class.alias_of.is_some() || !class.enumeration.is_empty())
    {
        component
            .modifiers
            .retain(|(name, value)| match name.as_str() {
                "start" => {
                    if component.start.is_none() {
                        component.start = Some(value.clone());
                    }
                    false
                }
                "fixed" => {
                    if component.fixed.is_none() {
                        component.fixed = Some(matches!(value, Expr::Bool(true)));
                    }
                    false
                }
                "min" => {
                    if component.min.is_none() {
                        component.min = Some(value.clone());
                    }
                    false
                }
                "max" => {
                    if component.max.is_none() {
                        component.max = Some(value.clone());
                    }
                    false
                }
                _ => true,
            });
    }
    let mut scope = scope.to_string();
    let mut imports = imports.to_vec();
    let mut followed = false;
    for _ in 0..MAX_DEPTH {
        if is_primitive(&component.type_name) {
            return;
        }
        let Some(class) = lookup(registry, &component.type_name, &scope, &imports) else {
            return;
        };
        if !class.enumeration.is_empty() {
            component.type_name = "Integer".to_string();
            return;
        }
        let Some((base, attributes)) = class.alias_of.clone() else {
            // The name a chain of aliases ends at was written in the
            // terms of whoever wrote the last one - `record
            // SalientInductance = Salient(...)` names a record beside
            // it - and whoever asked is somewhere else entirely. So
            // once an alias has been followed, what comes back is the
            // class's own full name. A name that was never an alias is
            // left as it was written, since the way it was reached -
            // an import through an encapsulated wall - may be the only
            // way there is.
            if followed {
                component.type_name = class.name.clone();
            }
            return;
        };
        followed = true;
        component.type_name = base;
        // A type that is an array gives its dimensions to whatever is
        // declared with it, after that declaration's own: `Orientation
        // o[2]` with `type Orientation = Real[4]` is `[2, 4]`.
        component
            .dimensions
            .extend(class.alias_dimensions.iter().cloned());
        if component.unit.is_none() {
            component.unit = class.alias_unit.clone();
        }
        for (name, value) in attributes {
            match name.as_str() {
                "start" if component.start.is_none() => component.start = Some(value),
                "fixed" if component.fixed.is_none() => {
                    component.fixed = Some(matches!(value, Expr::Bool(true)))
                }
                "min" if component.min.is_none() => component.min = Some(value),
                "max" if component.max.is_none() => component.max = Some(value),
                _ => {}
            }
        }
        // The next alias in the chain resolves where it was written.
        scope = class.name.clone();
        imports = class.imports.clone();
    }
}

/// Flat name of a reference written inside a class: an `outer`
/// declaration points at the enclosing `inner` instance, everything else
/// gets the instance prefix.
pub(super) fn flat_name(name: &str, prefix: &str, outers: &HashMap<String, String>) -> String {
    // The longest lead of the name that was bound, so that a member of
    // an `outer` a component holds - `innerState.stateGraphRoot` - is
    // answered by the whole of it rather than by its first word.
    let mut head = name;
    loop {
        if let Some(path) = outers.get(head) {
            return match name[head.len()..].is_empty() {
                true => path.clone(),
                false => format!("{path}{}", &name[head.len()..]),
            };
        }
        match head.rfind('.') {
            Some(cut) => head = &head[..cut],
            None => return format!("{prefix}{name}"),
        }
    }
}

/// Record one `spatialDistribution` and give the equation section the
/// two boundary values in its place.
///
/// The arguments are checked here rather than at the run, since every
/// one of them but the two inputs is settled before it: a profile that
/// does not span the coordinate, or a pair of arrays of different
/// lengths, is a mistake in the model rather than in the arithmetic.
fn spatial_transport(
    targets: &[Option<Expr>],
    arguments: &[Expr],
    prefix: &str,
    outers: &HashMap<String, String>,
    consts: &HashMap<String, f64>,
    acc: &mut Flat,
) -> Result<(), String> {
    if arguments.len() != 6 {
        return Err(format!(
            "spatialDistribution takes in0, in1, x, positiveVelocity, \
             initialPoints and initialValues, but got {} arguments",
            arguments.len()
        ));
    }
    if targets.len() != 2 {
        return Err("spatialDistribution fills two values, `(out0, out1)`".to_string());
    }
    let numbers = |expr: &Expr, what: &str| -> Result<Vec<f64>, String> {
        let Expr::Array(items) = expr else {
            return Err(format!("spatialDistribution needs an array for {what}"));
        };
        items
            .iter()
            .map(|item| {
                const_eval(item, consts).ok_or_else(|| {
                    format!("{what} of spatialDistribution must be known before the run")
                })
            })
            .collect()
    };
    let initial_points = numbers(&arguments[4], "initialPoints")?;
    let initial_values = numbers(&arguments[5], "initialValues")?;
    if initial_points.len() != initial_values.len() {
        return Err(format!(
            "spatialDistribution has {} initialPoints against {} initialValues",
            initial_points.len(),
            initial_values.len()
        ));
    }
    if initial_points.len() < 2
        || initial_points.first() != Some(&0.0)
        || initial_points.last() != Some(&1.0)
    {
        return Err("initialPoints of spatialDistribution must span 0 to 1".to_string());
    }
    if initial_points.windows(2).any(|pair| pair[1] < pair[0]) {
        return Err("initialPoints of spatialDistribution must not decrease".to_string());
    }
    let named = |target: &Option<Expr>, which: &str| -> Result<String, String> {
        match target {
            Some(Expr::Ref(name)) => Ok(name.clone()),
            _ => Err(format!(
                "the {which} of spatialDistribution must be a variable"
            )),
        }
    };
    acc.transports.push(SpatialTransport {
        out0: named(&targets[0], "first output")?,
        out1: named(&targets[1], "second output")?,
        in0: prefix_expr(&arguments[0], prefix, outers),
        in1: prefix_expr(&arguments[1], prefix, outers),
        x: prefix_expr(&arguments[2], prefix, outers),
        positive: prefix_expr(&arguments[3], prefix, outers),
        initial_points,
        initial_values,
    });
    Ok(())
}

/// The values that lie `depth` levels down inside one, in the order a
/// row-major walk meets them.
///
/// A value nests once per dimension and then once more for a record's
/// fields, and telling those apart by counting alone goes wrong as soon
/// as the two counts agree. The declaration says how many levels are
/// dimensions, so that is what is followed.
fn levels_down(value: &Value, depth: usize, out: &mut Vec<Value>) {
    match (depth, value) {
        (0, _) => out.push(value.clone()),
        (_, Value::Array(items)) => items
            .iter()
            .for_each(|item| levels_down(item, depth - 1, out)),
        _ => {}
    }
}

/// How many numbers one record of this class holds: its fields, each
/// as many times over as its own dimensions say, and a field that is
/// itself a record counted the same way.
///
/// `None` where a dimension is not a length written as a number, or
/// where a field's class is one this cannot look into. Then nothing
/// here can say what a value's shape means, and whoever asked is left
/// to decide by other means rather than by a count that might be
/// wrong.
fn numbers_of_one(
    registry: &HashMap<&str, &ClassDef>,
    of: &ClassDef,
    depth: usize,
) -> Option<usize> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut all = 0;
    for field in &of.components {
        let mut many = 1;
        for dimension in &field.dimensions {
            let length = const_eval(dimension, &HashMap::new())?;
            if length < 0.0 || length.fract() != 0.0 {
                return None;
            }
            many *= length as usize;
        }
        let each = match is_primitive(&field.type_name) {
            true => 1,
            false => {
                let inside = lookup(registry, &field.type_name, &of.name, &of.imports)?;
                // A type alias is a name for a primitive with
                // attributes attached - `type Power = Real(unit =
                // "W")` - and holds one number, not the none its
                // empty component list would suggest. Counting it as
                // none made a record of aliases look emptier than it
                // is, so a value handed to it matched neither reading
                // and was dropped, which is what left the machines'
                // friction records without their reference speed.
                match inside.alias_of.is_some() || !inside.enumeration.is_empty() {
                    true => 1,
                    false => numbers_of_one(registry, inside, depth + 1)?,
                }
            }
        };
        all += many * each;
    }
    Some(all)
}
