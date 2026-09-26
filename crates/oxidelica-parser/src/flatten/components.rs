//! The components of a class, put into the flat model: how long each
//! array is, what a declaration written over one comes to element by
//! element, and what each of those elements is worth.
//!
//! Carved out of `instantiate` unchanged.

use super::shapes::*;
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
        let minted = counted < acc.numbers.len();
        while counted < acc.numbers.len() {
            let (name, value) = &acc.numbers[counted];
            local_consts.entry(name.clone()).or_insert(*value);
            counted += 1;
        }
        // An element that has just become a number changes what a
        // body folds to, and the ledger of inlined bodies was written
        // before it did. A `while` whose head reads `w[1]` refused
        // for a trip count it could not settle while the element was
        // still a name, and the refusal answered every later asking -
        // by which time the number was here. Every other settling in
        // this flattener forgets the ledger on the same beat; the
        // elements of an array were the one mint that did not.
        if minted && std::env::var_os("OXIDELICA_KEEP_LEDGER_PAST_ELEMENTS").is_none() {
            inlining::forget_trip_refusals();
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

        // A base merged just now declared this very thing in the very
        // same words, so the declaration here is a repetition and not a
        // second element. Written out twice, its binding becomes two
        // equations for one variable, which is a model that cannot run.
        if std::env::var_os("OXIDELICA_NO_REPEATED_DECLARATION").is_none()
            && a_base_says_the_same(registry, class, component, 0)
        {
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
                // Under the instance path, where an `outer` is the
                // `inner` it stands for: `world.enableAnimation` read
                // inside `m.r1` is `m.world.enableAnimation`, and only
                // at the top of the model are the two spelt alike.
                .or_else(|| {
                    if std::env::var_os("OXIDELICA_NO_PREFIXED_CONDITION").is_some() {
                        return None;
                    }
                    const_eval(&prefix_expr(&named, prefix, outers), &env)
                })
                // `irRMS if smee.useDamperCage` stands above
                // `smee(useDamperCage = true)`: declarations are
                // instantiated in the order written, so the sibling
                // has not been built and its member holds nothing yet.
                // The value is written out on its declaration, and a
                // constant written out is what the member will be.
                .or_else(|| {
                    if std::env::var_os("OXIDELICA_NO_SIBLING_CONDITION").is_some() {
                        return None;
                    }
                    let Expr::Ref(wanted) = &named else {
                        return None;
                    };
                    sibling_written_value(wanted, class, inherited, overrides, &env)
                })
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

        // A class-level redeclaration carries modifiers of its own:
        // `redeclare model FlowModel = Detailed(dp_nominal = 1e5)`
        // both replaces the type and gives one of the replacement's
        // parameters a value. The alias itself is a pair of names and
        // has nowhere to hold them, so they were set aside by the
        // resolved type's name when the aliases were gathered. A
        // component typed by that alias is where they land - the
        // parameter is the component's, not a function input, so the
        // value belongs on the declaration the way any modifier does.
        //
        // Lowest precedence: a modifier written on the component
        // itself, or a component-level redeclaration, says the same
        // thing more locally and wins. Only a class-level redeclare
        // of a model is asked - a function's filled inputs are read
        // where its body is worked out, and a component is never a
        // function.
        if let Some(resolved) = lookup(registry, &component.type_name, scope, imports) {
            if resolved.kind != ClassKind::Function {
                if let Some(filled) = super::statements::filled_inputs(&resolved.name) {
                    for (name, value) in filled {
                        let already = extra_modifiers.iter().any(|(known, _)| known == &name)
                            || component.modifiers.iter().any(|(known, _)| known == &name);
                        if !already {
                            extra_modifiers.push((name, value));
                        }
                    }
                }
            }
        }

        // How long the declaration is along each axis.
        let sizes = measure_dimensions(
            class,
            inherited,
            &component,
            &flat_name,
            sizing_binding.as_ref(),
            env.sizing_shapes,
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
            sizing_shapes: env.sizing_shapes,
            outers,
            inners,
            overrides,
            consts: &local_consts,
            texts: local_texts,
            imports,
            scope,
            inside_a_parameter: env.inside_a_parameter,
            class,
            inherited,
        };
        // An array bound - or started - as a whole hands each element
        // its own value.
        // A start handed down from above is the model's word, whatever
        // the type said before it.
        let start_from_above = format!("{}.start", component.name);
        if extra_modifiers
            .iter()
            .chain(overrides.iter())
            .any(|(name, _)| name == &start_from_above)
        {
            component.start_from_type = false;
        }
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
                    // A call in the modifier is resolved to its full
                    // name here, where the class that wrote the
                    // modifier is still the scope: `Body` imports
                    // `to_unit1` and writes `sh(lengthDirection =
                    // to_unit1(r_CM))`, but the modifier is worked out
                    // when `Shape` is built, and `Shape` never heard of
                    // that import. Rewriting the call to the name the
                    // import stands for carries it across.
                    let value = resolve_call_names(&value, registry, scope, imports);
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
    // Lengths that came down with the modifiers, for measuring a `:`
    // and nothing else. Kept apart from `sizes_here` because that
    // table is read to decide whether a value spreads over elements,
    // and a length put there for a `:` told that reader the wrong
    // thing - twenty-one models refused for it where two were won.
    sizing_shapes: &HashMap<String, Vec<i64>>,
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
                    let count_of = |wanted: &str| local_consts.get(wanted).copied();
                    if let Some(length) = extents::size_of_a_table_in_a_file(
                        binding, axis, text_of, truth_of, count_of,
                    ) {
                        return Some(length);
                    }
                    if let Some(length) = flexible_size(binding, axis, registry, scope, imports) {
                        return Some(length);
                    }
                    // A value that is a bare name says its length by
                    // being that array, and the array may be one this
                    // class cannot reach: `extends Base(t(table = tbl))`
                    // hands a base the constant of the model doing the
                    // extending, and inside the base `tbl` is a name with
                    // no value at all. Its shape travelled down with the
                    // modifier and is in the table; asking the table is
                    // what the expansion below cannot do, because it
                    // wants the value and not the shape.
                    if let Expr::Ref(name) = binding {
                        let known = sizes_here
                            .get(name.as_str())
                            .or_else(|| sizes_here.get(format!("{prefix}{name}").as_str()))
                            .or_else(|| sizing_shapes.get(name.as_str()))
                            .or_else(|| sizing_shapes.get(format!("{prefix}{name}").as_str()));
                        if let Some(length) = known.and_then(|shape| shape.get(axis)) {
                            return Some(*length);
                        }
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
                    // A range says its length by its bounds rather
                    // than by any value: the table blocks write
                    // `columns[:] = 2:size(table, 2)`, and how many
                    // columns that is, the table beside it says. The
                    // early road through `shapes.rs` already reads a
                    // range this way, and a road that measures one
                    // declaration two ways is a road that will
                    // disagree with itself.
                    // So the range is measured by its bounds before it
                    // is expanded: `2:size(table, 2)` over a table that
                    // is `fill(0.0, 0, 2)` until a file fills it has a
                    // length of one and no rows to expand, and asking
                    // the expansion for its shape answered nothing. A
                    // table said to be on a file is not measured this
                    // way: its declaration is a placeholder, and the
                    // file, read above or not at all, is the width.
                    if !super::table_files::old_file_tables()
                        && truth_of("tableOnFile") != Some(true)
                    {
                        if let Some(length) =
                            shapes::range_length(&binding, axis, local_consts, sizes_here)
                        {
                            return Some(length);
                        }
                    }
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
        // Under the medium the model chose, where one is on the mark:
        // `Xi(start = reference_X[1:nXi])` is a slice whose end is a
        // constant of the medium, and read under the interface it is
        // a slice of nothing. The same road the dimension takes, and
        // for the same reason - a start of an array is measured, not
        // read by any unit layer.
        let _settling = constants::SettlingParameter::now();
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
    // The same, where the array is not the whole value but a term of
    // it. A pipe hands its base `final dheights = height_ab*dxs`, and
    // `dxs` is an array of the extending class that the base has never
    // heard of; spread whole, every element of `dheights` is bound to
    // `height_ab` times the entire array, and the run is left with a
    // parameter nothing gives a value to. Subscripted, each element
    // gets the term that was meant.
    if items.len() == 1 && !element_names.is_empty() {
        if let Some(per_element) =
            over_handed_elements(&items[0], env.handed_shapes, element_names.len())
        {
            return Ok(per_element);
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

/// One value written over an array of the class above, taken apart
/// into one value per element.
///
/// The rule the caller states is about a bare name; this is the same
/// rule where the name is a term of a larger expression. Every handed
/// array the value names must be the length of the component, or there
/// is no one way to subscript them together and the value is left
/// whole for the caller's other rules to judge. At least one such name
/// has to be found, so that a value naming none of them - a true
/// scalar - still spreads.
fn over_handed_elements(
    value: &Expr,
    handed: &HashMap<String, Vec<i64>>,
    count: usize,
) -> Option<Vec<Expr>> {
    if count == 0 {
        return None;
    }
    // What the value names that the class above knows as an array.
    // Gathered before anything is rewritten, because a length that
    // does not match means this value is not one to take apart at all.
    let mut shape: Option<Vec<i64>> = None;
    let mut mismatched = false;
    let mut survey = |expr: &Expr| {
        if let Expr::Ref(name) = expr {
            if let Some(named) = handed.get(name.as_str()) {
                match index_tuples(named).len() == count {
                    true => shape = Some(named.clone()),
                    false => mismatched = true,
                }
            }
        }
    };
    walk_refs(value, &mut survey);
    let shape = shape.filter(|_| !mismatched)?;
    Some(
        index_tuples(&shape)
            .into_iter()
            .map(|at| subscript_handed(value, handed, count, &at))
            .collect(),
    )
}

/// Every name in an expression, to a reader that only looks.
fn walk_refs(expr: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(expr);
    expr.map_children(&mut |child| {
        walk_refs(child, f);
        child.clone()
    });
}

/// One element of every handed array the value names, the rest of the
/// expression left as it stands. Only the arrays of the component's
/// own length are subscripted; a name of any other shape is not one
/// this element has a share of.
fn subscript_handed(
    expr: &Expr,
    handed: &HashMap<String, Vec<i64>>,
    count: usize,
    at: &[i64],
) -> Expr {
    if let Expr::Ref(name) = expr {
        if let Some(shape) = handed.get(name.as_str()) {
            if index_tuples(shape).len() == count {
                return Expr::Ref(element_name(name, at));
            }
        }
    }
    expr.map_children(&mut |child| subscript_handed(child, handed, count, at))
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
    // Every field the record has, its bases' among them. Read from
    // the record's own declarations alone, a field declared one
    // `extends` up is not in the list at all, so the value coming
    // apart has more pieces than there are names to take them and the
    // whole hand-over is dropped - which is what left the Spice3
    // mosfets' `IsGiven` fields, declared three records up, without
    // the values their constructor had already worked out. The same
    // gatherer the rest of the flattener reads a record with, so the
    // two cannot disagree about what a record is.
    // The class's own constants are among them here, and dropped
    // again below. A value has two right lengths: a constructor writes
    // no constant, and a record named whole and written out writes
    // every one - `cellData` of a battery brings the `constant String
    // CellType` its base declares. Counted one way only, the other
    // fitted nothing, and a matrix of six battery cells arrived as 774
    // things to be given to six names.
    let held: Vec<Component> = record_fields::record_components(registry, of, 0);
    let a_constant: Vec<bool> = held
        .iter()
        .map(|field| field.variability == Variability::Constant)
        .collect();
    let fields: Vec<String> = held.iter().map(|field| field.name.clone()).collect();
    let settable: Vec<bool> = held
        .iter()
        .zip(&a_constant)
        .map(|(field, constant)| !field.is_final && !constant)
        .collect();
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
    let without_constants = a_constant.iter().filter(|constant| !**constant).count();
    let one = |item: &Value| -> Option<Vec<Expr>> {
        let Value::Array(given) = item else {
            return None;
        };
        let mut given = given.iter().cloned().map(Value::into_expr);
        if given.len() == fields.len() {
            return Some(given.collect());
        }
        // A constructor's value says nothing about the constants, so
        // what it holds lines up with the fields that are not ones.
        // The places a constant sits are filled with what the class
        // declared it as, which is the only thing it can be - and not
        // with its bare name, which names nothing where this lands.
        if given.len() == without_constants {
            // A constant with nothing to be is not a zero. Filling its
            // place with one put a number the record never stated into
            // every field that follows it - and a wrong number given
            // quietly is the worst thing this compiler can do, where
            // the same case declined here is a refusal naming the
            // field. Nothing in the library reaches this with an
            // unbound constant; a model that does is owed the refusal.
            return held
                .iter()
                .zip(&a_constant)
                .map(|(field, constant)| match constant {
                    true => field.binding.clone(),
                    false => given.next(),
                })
                .collect();
        }
        None
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
    // The other right length: `numbers_of_one` counts what a
    // constructor writes, and a record named whole and written out
    // brings the class's own constants too. Both are allowed, and
    // `one` above knows which of the two it was handed.
    let constants = a_constant.iter().filter(|constant| **constant).count();
    let per_element = match of_one.map(|each| each + constants) {
        // A record whose shape holds a length the compiler
        // cannot see says nothing either way, and the reading
        // that was here before has its say.
        None | Some(0) => one_apiece().or_else(over_all),
        Some(each) if leaves.len() == each * element_names.len() => one_apiece(),
        Some(each) if leaves.len() == each => over_all(),
        Some(_) => match of_one {
            Some(each) if leaves.len() == each * element_names.len() => one_apiece(),
            Some(each) if leaves.len() == each => over_all(),
            _ => None,
        },
    }
    .unwrap_or_default();
    // A value taken apart by count is taken apart by the class that
    // wrote it, and matched against the fields of the class that
    // receives it. Those are the same class most of the time and not
    // always: `Impedance impedance(cellData = cellData)` declares its
    // parameter as the base record and is handed a derived one, which
    // holds more fields than the base declares. Counted, the two do
    // not fit and the whole hand-over is dropped, which is how
    // `ShowImpedance` came to have a `Qnom` that nothing gives a value
    // to although the model says `Qnom = 3600` in plain sight.
    //
    // Where the value is simply the name of a record, there is no
    // counting to be done at all: the field the target calls `Qnom`
    // takes the value at `cellData.Qnom`, whatever else either record
    // holds. Names carry the meaning here and positions are a guess at
    // it, so this is tried wherever the count came to nothing rather
    // than only where it disagreed.
    if per_element.is_empty() {
        if let Expr::Ref(path) = &expr {
            if !path.contains('[') {
                let by_name: Vec<(String, Expr)> = held
                    .iter()
                    .zip(&settable)
                    .filter(|(_, may)| **may)
                    .map(|(field, _)| field)
                    .map(|field| {
                        (
                            field.name.clone(),
                            Expr::Ref(format!("{path}.{}", field.name)),
                        )
                    })
                    .collect();
                if !by_name.is_empty() {
                    return vec![by_name; element_names.len()];
                }
            }
        }
    }
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
        sizing_shapes: handed_sizing,
        outers,
        inners,
        overrides,
        consts: local_consts,
        texts: local_texts,
        imports,
        scope,
        inside_a_parameter,
        class: _,
        inherited: _,
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
            // Two bases of one class may each declare the same name,
            // and the flat model has one place for it. `m_flow` is
            // declared by `PartialTwoPortTransport` and again by
            // `PartialLumpedFlow`, and every fitting of the fluid
            // library extends both: written out twice, the model
            // carries two unknowns where its equations settle one, and
            // the count comes out one short - which is what the refusal
            // `nothing determines simpleGenericOrifice.m_flow` was
            // saying all along, naming the second copy.
            //
            // The rule next door, which asks whether a base repeats a
            // declaration word for word, cannot see this pair: they
            // differ, one writing a `start` the other leaves off. What
            // settles the question is not the wording but the flat
            // name, and the flat name exists only here. The first
            // declaration stands - but only once the two are known to
            // say the same about the value, which is decided below,
            // where this declaration's own binding has been worked
            // out.
            let declared_before = match twice_declared_is_kept() {
                true => None,
                false => acc.declared.get(flat_name).cloned(),
            };
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
            // In view while the bodies this value calls are worked
            // out: the argument a body reads carries the caller's own
            // spelling, and only this table says it names a record.
            let _records = statements::Records::in_view(records_so_far);
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
            // The `fixed` flag written as an expression is carried
            // the same way a start is: in the terms of the flat model
            // rather than the class that wrote it, since anything
            // surviving flattening carries the flat model's names.
            // What it comes to is decided where the parameters are,
            // which is the only place the names in it are worth
            // anything.
            flat.fixed_expr = flat.fixed_expr.as_ref().map(|expr| {
                resolve_value(expr).unwrap_or_else(|_| prefix_expr(expr, prefix, outers))
            });
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
            // A nominal is read the same way as a bound: it is a
            // magnitude rather than an equation, and what cannot be
            // settled here is left as it was written.
            flat.nominal = flat.nominal.as_ref().map(bound);
            // A parent modifier `name = expr` overrides the binding, and
            // a nested one - `phi(start = 1)` - the attribute.
            let modifier = |target: &str| {
                extra_modifiers
                    .iter()
                    .chain(overrides.iter())
                    .find(|(n, _)| n == target)
                    .map(|(_, e)| e.clone())
            };
            // A modifier arrives written in the terms of the class
            // that supplied it, so it is not prefixed again - but
            // it still has to be worked out. Left as it stands, a
            // call inside it is never inlined, and a value that
            // came down an `extends` is a call the parameters
            // cannot evaluate: the machines say what their
            // nominal voltage is that way, through a function of
            // the resistance and the brush drop.
            let work_out = |value: &Expr| -> Result<Expr, String> {
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
                let worked = substitute_class_constants(value, registry, scope, imports, &[]);
                // The writer's own lengths go in with this class's
                // from the start rather than after a refusal. A
                // reduction over an array nothing here has measured
                // does not refuse: `sum` of a name it cannot see the
                // length of comes back as the name, which is a wrong
                // number wearing the shape of a right one, and a
                // second attempt is never made because the first
                // reported success. Where both know a name, this
                // class's own entry stays - the value is read here,
                // and what the writer called `v` is not what this
                // class calls `v`.
                let reach = match std::env::var_os("OXIDELICA_NO_WRITERS_LENGTHS").is_none()
                    && !outer_sizes.is_empty()
                {
                    true => {
                        let mut reach = outer_sizes.clone();
                        reach.extend(sizes.iter().map(|(n, s)| (n.clone(), s.clone())));
                        Some(reach)
                    }
                    false => None,
                };
                if let Some(reach) = reach.as_ref() {
                    shapes.sizes = reach;
                }
                expand(&worked, &shapes, registry, scope, imports, 0)
                    .and_then(|value| value.scalar())
            };
            if let Some(value) = modifier(local_name) {
                flat.binding = Some(work_out(&value).unwrap_or(value));
            }
            // On an array the start has already been handed out
            // element by element; this is the scalar case. A start
            // handed down is worked out the way a value is: taken as
            // written, `core1(H(start = HStart[1]))` kept the subscript
            // on a name the flat model holds only element by element,
            // and every reader of the start refused it or, where the
            // refusal was swallowed, read it as zero.
            if site.start.is_none() {
                if let Some(value) = modifier(&format!("{}.start", component.name)) {
                    flat.start = Some(
                        match std::env::var_os("OXIDELICA_RAW_START_MODIFIER").is_none() {
                            true => work_out(&value).unwrap_or(value),
                            false => value,
                        },
                    );
                    flat.start_from_type = false;
                }
            }
            if let Some(value) = modifier(&format!("{}.fixed", component.name)) {
                // A modifier from above outranks the declaration, and
                // outranks an expression the declaration carried too.
                match value {
                    Expr::Bool(_) | Expr::Number(_) => {
                        flat.fixed = Some(!matches!(value, Expr::Bool(false) | Expr::Number(0.0)));
                        flat.fixed_expr = None;
                    }
                    other => {
                        flat.fixed = None;
                        flat.fixed_expr = Some(other);
                    }
                }
            }
            // The second of two declarations of one name is settled
            // here, where its own binding has been worked out. Where
            // both say the same thing - or neither says anything, as
            // the fluid library's mass flow does - the repetition
            // falls away and the first declaration stands. Where they
            // differ, neither is a repetition of the other and there
            // is nothing to choose between them: taking the first
            // would answer with its number and say nothing, and a
            // wrong number presented as a right one is the worst this
            // compiler can do.
            if let Some(before) = declared_before {
                if first_of_two_is_taken() || same_binding(before.as_ref(), flat.binding.as_ref()) {
                    return Ok(());
                }
                let said = |binding: Option<&Expr>| match binding {
                    Some(expr) => format!("`{}`", expr.describe()),
                    None => "no value".to_string(),
                };
                return Err(format!(
                    "`{flat_name}` is declared twice over with different values: \
                     one declaration binds {}, the other {}; which of them the \
                     flat model is to carry is not something the compiler can choose",
                    said(before.as_ref()),
                    said(flat.binding.as_ref())
                ));
            }
            acc.declared
                .insert(flat_name.to_string(), flat.binding.clone());
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
            // How long the arrays a modifier names are. A value handed
            // to a component is written where this class stands -
            // `t_new(table = tbl)` names a constant of the model, not
            // anything the block ever heard of - so once the child is
            // being built the name has no shape and no value there at
            // all. The length was measured here, where the name still
            // means something, and travels down with the modifier.
            // Only the bare names a modifier writes are carried: a
            // shape nobody asked about is a guess about which class the
            // name lands in, and this compiler owes a refusal instead.
            let mut handed_below: HashMap<String, Vec<i64>> = HashMap::new();
            // The siblings' shapes among them, apart: a value naming
            // a sibling not yet instantiated is spread over elements
            // by these, where nothing else has measured it.
            let mut siblings_below: HashMap<String, Vec<i64>> = HashMap::new();
            for (_, value) in &mods {
                let Expr::Ref(named) = value else {
                    continue;
                };
                let known = sizes
                    .get(named.as_str())
                    .or_else(|| sizes.get(format!("{prefix}{named}").as_str()))
                    // And the lengths this class was itself handed: a
                    // table travels down three layers of partial
                    // classes before it reaches the block declaring
                    // the `:`, and a length dropped at the first is a
                    // length the last cannot ask anybody for.
                    .or_else(|| handed_sizing.get(named.as_str()))
                    .or_else(|| handed_sizing.get(format!("{prefix}{named}").as_str()));
                // A member of a sibling declared further down:
                // `startTime_0(table = startTime.table)` standing above
                // `startTime(table = [...])`. Modelica does not order
                // declarations, but they are instantiated in the order
                // they are written, so the sibling has not been
                // measured yet. Its value is written out on its own
                // declaration, and a value written out says its
                // length by being written out.
                let written;
                let known = match known {
                    Some(shape) => Some(shape),
                    None if std::env::var_os("OXIDELICA_NO_SIBLING_SHAPE").is_none() => {
                        written = sibling_written_shape(named, level, registry);
                        if let Some(shape) = &written {
                            siblings_below.insert(named.clone(), shape.clone());
                        }
                        written.as_ref()
                    }
                    None => None,
                };
                if let Some(shape) = known {
                    handed_below
                        .entry(named.clone())
                        .or_insert_with(|| shape.clone());
                }
            }
            // The writer's lengths go on down with the modifier. A value
            // handed two levels - `B b(pb(p = fr(vr)))` - is read inside
            // `pb`, whose view of lengths was this class's table alone,
            // and that table knows `vr` only if `vr` happened to be
            // declared above `b`. Otherwise the array reached the body
            // with no length, and `size(v, 1)` of an array of records
            // answered the number of fields of one record: a wrong
            // number with nothing said. What the modifiers name and the
            // level above measured is carried on, and nothing else, so
            // the table grows by the names asked about and not by the
            // model.
            let reaching;
            let below_sizes = match writers_lengths_carried(sizes, outer_sizes, &mods) {
                Some(widened) => {
                    reaching = widened;
                    &reaching
                }
                None => sizes,
            };
            let child_env = Env {
                overrides: &mods,
                redeclares,
                inners,
                broken: &[],
                handed_shapes: &siblings_below,
                sizing_shapes: &handed_below,
                outer_sizes: below_sizes,
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
            // A record kept empty in the base and redeclared whole by
            // the medium - `ThermodynamicState`, whose fields a medium
            // states and the interface leaves blank - is instantiated
            // as the medium's, not the interface's. Resolving the type
            // landed on the base that declares it; the mark above holds
            // the name it was reached by, and under that name the
            // redeclared record with its fields is found. Without this
            // a `state` component comes out with no fields, and an
            // equation reading `state.h` names a variable nothing
            // declares.
            let child = inlining::record_asked_under(child, registry);
            instantiate(registry, child, &child_prefix, &child_env, acc, depth + 1)?;
        }
    }
    Ok(())
}

/// Rewrite a bare function name in an expression to the full name its
/// import stands for, in the scope where the expression was written.
///
/// A modifier `sh(lengthDirection = to_unit1(r_CM))` is written in the
/// class that imports `to_unit1` but worked out in the child's scope,
/// which never heard of that import. Resolving the name here, before
/// the modifier descends, is what carries it across - and only where
/// the name resolves to something with a different, fuller name of its
/// own, so a name already whole or a local one is left alone.
fn resolve_call_names(
    expr: &Expr,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
) -> Expr {
    let mapped = expr
        .try_map_children(&mut |child| {
            Ok::<Expr, ()>(resolve_call_names(child, registry, scope, imports))
        })
        .expect("mapping cannot fail");
    match &mapped {
        Expr::Call(name, args) if !name.contains('.') => {
            match lookup(registry, name, scope, imports) {
                Some(found) if found.name != *name && found.name.ends_with(&format!(".{name}")) => {
                    Expr::Call(found.name.clone(), args.clone())
                }
                _ => mapped,
            }
        }
        _ => mapped,
    }
}

/// Whether a name declared twice over is written out twice, as it was
/// before. `OXIDELICA_KEEP_TWICE_DECLARED` is kept so that one binary
/// can be measured against itself over the whole library.
fn twice_declared_is_kept() -> bool {
    std::env::var_os("OXIDELICA_KEEP_TWICE_DECLARED").is_some()
}

/// Whether the first of two declarations that disagree is taken
/// silently, as it was before the refusal was written.
/// `OXIDELICA_TAKE_FIRST_OF_TWO` is kept so that one binary can be
/// measured against itself over the whole library, since a refusal
/// that costs models has to be seen to cost them.
fn first_of_two_is_taken() -> bool {
    std::env::var_os("OXIDELICA_TAKE_FIRST_OF_TWO").is_some()
}

/// Whether two declarations of one name say the same about its value.
/// Both silent is the fluid library's case and the commonest; two
/// bindings agree when they are the same expression, and two numbers
/// agree when they are the same number however they were spelled -
/// `2` and `2.0` are one value, and a refusal about the spelling
/// would be a test on the text standing in for a test on the value.
fn same_binding(before: Option<&Expr>, now: Option<&Expr>) -> bool {
    match (before, now) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            let empty = HashMap::new();
            match (const_eval(a, &empty), const_eval(b, &empty)) {
                (Some(x), Some(y)) => x == y,
                _ => a == b,
            }
        }
        _ => false,
    }
}

/// The shape of `sibling.member`, where `sibling` is a component of
/// the class being instantiated whose own declaration writes `member`
/// out in full.
///
/// Only a value written out is measured: a literal table says its
/// length by how it is written, wherever it stands and whatever has
/// been instantiated so far. A value handed down from above that
/// reaches the sibling or its member outranks the declaration, so
/// where there is one this answers nothing and the measurement is left
/// to the roads that know what the handed value says.
fn sibling_written_shape(
    named: &str,
    level: &Level,
    registry: &HashMap<&str, &ClassDef>,
) -> Option<Vec<i64>> {
    let local = named.strip_prefix(level.prefix).unwrap_or(named);
    let (head, member) = local.split_once('.')?;
    if member.contains('.') || member.contains('[') || head.contains('[') {
        return None;
    }
    let reaches = |target: &str| {
        target == head
            || target
                .strip_prefix(head)
                .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('['))
    };
    if level.overrides.iter().any(|(target, _)| reaches(target)) {
        return None;
    }
    let sibling = level
        .class
        .components
        .iter()
        .chain(level.inherited.iter().map(|(component, _)| component))
        .find(|component| component.name == head)?;
    if !sibling.dimensions.is_empty() {
        return None;
    }
    let (_, written) = sibling
        .modifiers
        .iter()
        .find(|(modified, _)| modified == member)?;
    let mut shape = Vec::new();
    while let Some(length) =
        extents::flexible_size(written, shape.len(), registry, level.scope, level.imports)
    {
        shape.push(length);
    }
    (!shape.is_empty()).then_some(shape)
}

/// What `sibling.member` comes to where the sibling is declared in this
/// class and writes the member a constant among its modifiers.
///
/// A condition may read a sibling declared further down, which has not
/// been instantiated when the condition is decided. Only a value
/// written out and constant on its own is taken; a value handed down
/// from above that reaches the sibling outranks the declaration, so
/// where there is one this answers nothing.
fn sibling_written_value(
    wanted: &str,
    class: &ClassDef,
    inherited: &[(Component, Option<Expr>)],
    overrides: &[(String, Expr)],
    env: &HashMap<String, f64>,
) -> Option<f64> {
    let (head, member) = wanted.split_once('.')?;
    if member.contains('.') || member.contains('[') || head.contains('[') {
        return None;
    }
    let reaches = |target: &str| {
        target == head
            || target
                .strip_prefix(head)
                .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('['))
    };
    if overrides.iter().any(|(target, _)| reaches(target)) {
        return None;
    }
    let sibling = class
        .components
        .iter()
        .chain(inherited.iter().map(|(component, _)| component))
        .find(|component| component.name == head)?;
    if !sibling.dimensions.is_empty() {
        return None;
    }
    let (_, written) = sibling
        .modifiers
        .iter()
        .find(|(modified, _)| modified == member)?;
    const_eval(written, env)
}

/// Whether a modifier handed more than one level down loses the lengths
/// of the class that wrote it, as it did before: the switch that lets
/// one binary give both numbers.
fn writers_lengths_below_off() -> bool {
    std::env::var_os("OXIDELICA_NO_WRITERS_LENGTHS_BELOW").is_some()
}

/// `here` widened by the lengths `writer` measured for the arrays the
/// modifiers name, where `here` has not measured them itself; `None`
/// where nothing is to be added.
///
/// A value handed more than one level down is read where it lands, and
/// the arrays it names belong to the class that wrote it. Only the
/// names the values write, and every prefix of them - `vr` in
/// `vr[1].re` - are looked for, so the table grows by what is asked
/// about and not by the model. What `here` already holds stays: the
/// value is read there, and its own entry is the nearer one.
pub(super) fn writers_lengths_carried(
    here: &HashMap<String, Vec<i64>>,
    writer: &HashMap<String, Vec<i64>>,
    mods: &[(String, Expr)],
) -> Option<HashMap<String, Vec<i64>>> {
    if writers_lengths_below_off() || writer.is_empty() {
        return None;
    }
    let mut wanted: Vec<&str> = Vec::new();
    for (_, value) in mods {
        value.collect_refs(&mut wanted);
    }
    let mut missing: Vec<(String, Vec<i64>)> = Vec::new();
    for name in wanted {
        let mut head = name;
        loop {
            if !here.contains_key(head) && !missing.iter().any(|(known, _)| known == head) {
                if let Some(shape) = writer.get(head) {
                    missing.push((head.to_string(), shape.clone()));
                }
            }
            match head.rfind(['.', '[']) {
                Some(at) => head = &head[..at],
                None => break,
            }
        }
    }
    if missing.is_empty() {
        return None;
    }
    let mut widened = here.clone();
    widened.extend(missing);
    Some(widened)
}
