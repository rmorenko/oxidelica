//! The components of a class, put into the flat model: how long each
//! array is, what a declaration written over one comes to element by
//! element, and what each of those elements is worth.
//!
//! Carved out of `instantiate` unchanged.

use super::*;

/// Every component this class declares, turned into the variables,
/// equations and instances it stands for.
///
/// Moved out of `instantiate` unchanged.
#[allow(clippy::too_many_arguments)]
pub(super) fn instantiate_components(
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
    // The strings this class settled, in view while its bodies are
    // worked out: a `while` that goes round until it finds a piece of
    // text needs to know what the text says, and this is where that
    // is known.
    let _texts = statements::Texts::in_view(local_texts);
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
                    inlining::Inlined::forget();
                    local_consts.insert(format!("{prefix}{}", waiting.name), length as f64);
                    inlining::Inlined::forget();
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
                // A parameter nobody gave a value stands at its start.
                // The machines of the library write `parameter Boolean
                // useDamperCage(start = true)` in a connector and mean
                // it: a condition has to be settled before anything
                // can be handed down to the component it guards.
                .or_else(|| {
                    let Expr::Ref(wanted) = &named else {
                        return None;
                    };
                    let held = class
                        .components
                        .iter()
                        .chain(inherited.iter().map(|(component, _)| component))
                        .find(|c| &c.name == wanted)?;
                    const_eval(held.start.as_ref()?, &env)
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
            // A settled value travels as a number, which is what
            // arithmetic wants and what a condition cannot use: a
            // Boolean written back as `Number(0.0)` is an Integer
            // where a Boolean is needed, and a medium's
            // `ph_explicit = false` reached every `if` that asks it
            // in that shape. The declaration says which it is.
            component.binding = Some(match component.type_name == "Boolean" {
                true => Expr::Bool(*value != 0.0),
                false => Expr::Number(*value),
            });
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
                    // The same reading as the early roads take: a
                    // table on a file says how wide it is only once
                    // the file is read, and both roads have to answer
                    // alike or the shape settled early and the shape
                    // measured here part company without a word.
                    let in_view = statements::texts_in_view();
                    let text_of = |wanted: &str| in_view.get(wanted).cloned();
                    let truth_of =
                        |wanted: &str| local_consts.get(wanted).map(|value| *value != 0.0);
                    if let Some(length) =
                        extents::size_of_a_table_in_a_file(binding, axis, text_of, truth_of)
                    {
                        return Some(length);
                    }
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
                // A dimension wants the digit or nothing - no unit
                // layer ever reads a length - which is the same
                // argument that admitted a parameter's road to the
                // mark, arriving here from the shape side.
                let _settling = constants::SettlingParameter::now();
                let named = substitute_class_constants(dimension, registry, scope, imports, shadow);
                let value = const_eval(&named, local_consts)
                    // A length written on a name of this class -
                    // `size(a, 1) - 1` for a transfer function's
                    // states - reads a shape that was written down
                    // under the instance path, so it is asked for
                    // under that name too.
                    .or_else(|| {
                        let named = prefix_expr(&named, prefix, outers);
                        dimension_value(&named, local_consts, sizes_here)
                            .map(|length| length as f64)
                    })
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
                let e = instantiate::substitute_texts(&e, local_texts);
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
                None => {
                    // A parameter's own value is settled for a number
                    // and read by nothing else, so a constant its body
                    // reads may be answered from the medium the call
                    // was written under. On a variable the same
                    // expression is read by the dimensional layer,
                    // which wants the name and its unit rather than a
                    // digit.
                    let _settling = matches!(
                        flat.variability,
                        Variability::Parameter | Variability::Constant
                    )
                    .then(constants::SettlingParameter::now);
                    flat.binding.as_ref().map(&resolve_value).transpose()?
                }
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
            // The medium the model named, held while its own body is
            // worked out. `Medium.BaseProperties` resolves to the base
            // that declares it, and the equations of that base call
            // functions only the medium declares: without this the
            // body is worked out under the base alone, where those
            // names mean nothing.
            let _asked =
                inlining::AskedAs::resolving(&component.type_name, child, registry, scope, imports);
            instantiate(registry, child, &child_prefix, &child_env, acc, depth + 1)?;
        }
    }
    Ok(())
}
