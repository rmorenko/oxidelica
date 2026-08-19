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
    if depth > MAX_DEPTH {
        return Err(format!(
            "instantiation deeper than {MAX_DEPTH} levels at `{}` (recursive classes?)",
            class.name
        ));
    }

    let scope = class.name.as_str();
    // Every equation this class puts in is stamped with the instance it
    // belongs to; a class instantiated inside it stamps its own, and
    // this one is put back afterwards.
    let stamped = std::mem::replace(&mut acc.origin, prefix.trim_end_matches('.').to_string());

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
    let imports = effective_imports(registry, class, scope, &redeclares)?;
    // Names a wildcard-imported constant may not quietly stand in for:
    // a component of this class outranks anything `import A.*;` opened.
    let shadow: Vec<&str> = class
        .components
        .iter()
        .map(|component| component.name.as_str())
        .collect();

    // Bases first, with their modifiers (already parent-scoped).
    for extend in &class.extends {
        let base = lookup(registry, &extend.base, scope, &imports)
            .ok_or_else(|| format!("unknown base class `{}`", extend.base))?;
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
                redeclare, registry, class, prefix, &outers,
            )?);
        }
        base_redeclares.extend(redeclares.iter().cloned());
        let base_env = Env {
            overrides: &mods,
            redeclares: &base_redeclares,
            inners: &inners,
            broken: &extend.broken,
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
            let mut env = acc.const_values.clone();
            for (name, value) in &local_consts {
                env.insert(name.clone(), *value);
            }
            if let Some(value) = const_eval(&expr, &env) {
                local_consts.insert(component.name.clone(), value);
                acc.const_values
                    .insert(format!("{prefix}{}", component.name), value);
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
        }
    }
    // And the parameters of this instance that another base already
    // settled. `extends ConditionalHeatPort(T = fill(293.15, m))` is
    // written in a class whose `m` comes from a base of its own, and
    // by the time the second base is reached the first has said what
    // `m` is - under the instance path, which is the only name the
    // modifier still carries. What this class says itself wins.
    for (name, value) in &acc.const_values {
        if name.starts_with(prefix) && !local_consts.contains_key(name) {
            local_consts.insert(name.clone(), *value);
        }
    }

    // What each array component of this class - and of its bases - is
    // shaped like, so a value may name one as a whole.
    let mut sizes: HashMap<String, Vec<i64>> = HashMap::new();
    collect_shapes(registry, class, &local_consts, &mut sizes, 0);
    let sizes_here = prefixed_sizes(&sizes, prefix);
    // Which of this class's components are records, and of what: an
    // overloaded operator is chosen by the record its operands are of.
    let records_here: HashMap<String, String> = class
        .components
        .iter()
        .filter_map(|component| {
            let of = lookup(registry, &component.type_name, scope, &imports)?;
            (of.kind == ClassKind::Record)
                .then(|| (format!("{prefix}{}", component.name), of.name.clone()))
        })
        .collect();

    for component in &class.components {
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
            let value = const_eval(condition, &env).ok_or_else(|| {
                format!("condition of component `{flat_name}` is not a compile-time constant")
            })?;
            if value == 0.0 {
                acc.disabled.push(flat_name.clone());
                continue;
            }
        }

        // The value that fixes a flexible `:` size, if the component has
        // one: an override handed down, else the declaration's own.
        let sizing_binding = overrides
            .iter()
            .find(|(name, _)| name == &component.name)
            .map(|(_, e)| e.clone())
            .or_else(|| component.binding.clone());

        let mut component = component.clone();

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
                redeclare, registry, class, prefix, &outers,
            )?);
        }

        // A connector may be one value rather than a set of members:
        // `connector RealInput = input Real` is how every signal in
        // the standard library is carried. Resolving the type below
        // leaves the primitive behind, so what class it came from is
        // noted first - a connection to it is still a connection.
        let value_connector = lookup(registry, &component.type_name, scope, &imports)
            .filter(|class| class.kind == ClassKind::Connector && class.alias_of.is_some())
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
                    let shape = sizing_binding
                        .as_ref()
                        .and_then(|binding| flexible_size(binding, axis))
                        .ok_or_else(|| {
                            format!(
                                "the flexible size `:` of `{flat_name}` needs a value \
                                 to read its length from"
                            )
                        })?;
                    shape
                }
                _ => {
                    let value = const_eval(dimension, &local_consts).ok_or_else(|| {
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
                .insert(format!("{prefix}{}", component.name), sizes.clone());
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
            outers: &outers,
            inners: &inners,
            overrides,
            consts: &local_consts,
            imports: &imports,
            scope,
        };
        // An array bound - or started - as a whole hands each element
        // its own value.
        let spread = |expr: &Expr, what: &str, prefixed: bool| -> Result<Vec<Expr>, String> {
            let shapes = Shapes {
                sizes: &sizes_here,
                loop_vars: &HashMap::new(),
                consts: &local_consts,
                records: no_records(),
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
            // A scalar start spreads over the whole array.
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
        let element_bindings: Option<Vec<Expr>> = match (
            handed_down(&component.name),
            &component.binding,
            sizes.is_empty(),
        ) {
            (Some(value), _, false) => Some(spread(&value, "value", true)?),
            (None, Some(binding), false) => Some(spread(binding, "value", false)?),
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
                    let spread_whole =
                        element_count == 1 || component.each_modifiers.iter().any(|e| e == name);
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
    let mut sizes_here = sizes_here;
    for (name, shape) in &acc.sizes {
        if name.starts_with(prefix) {
            sizes_here.insert(name.clone(), shape.clone());
        }
    }

    // Equations: arrays expanded, subscripts resolved, calls inlined.
    let resolve_here = |expr: &Expr| -> Result<Expr, String> {
        let expr = substitute_class_constants(expr, registry, scope, &imports, &shadow);
        resolve(
            &prefix_expr(&expr, prefix, &outers),
            &HashMap::new(),
            &local_consts,
            registry,
            scope,
            &imports,
            0,
        )
    };
    let expand_here = |expr: &Expr, loop_vars: &HashMap<String, f64>| -> Result<Value, String> {
        let expr = substitute_class_constants(expr, registry, scope, &imports, &shadow);
        let expr = prefix_expr(&expr, prefix, &outers);
        let shapes = Shapes {
            sizes: &sizes_here,
            loop_vars,
            consts: &local_consts,
            records: &records_here,
        };
        expand(&expr, &shapes, registry, scope, &imports, 0)
    };
    let no_loop_vars = HashMap::new();
    for equation in &class.equations {
        // `(a, , c) = f(...)`: one call fills several targets. The
        // call is inlined once per output; a skipped slot costs its
        // computation nothing, since the expression is never used.
        if let Expr::Tuple(targets) = &equation.lhs {
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
                continue;
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
            continue;
        }
        let lhs = expand_here(&equation.lhs, &no_loop_vars)?;
        let rhs = expand_here(&equation.rhs, &no_loop_vars)?;
        push_equations(&lhs, &rhs, acc)?;
    }

    for (condition, message) in &class.asserts {
        acc.asserts
            .push((resolve_here(condition)?, message.clone()));
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
    for clause in &class.connection_graph {
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
                let mut actions = Vec::new();
                for inner in &branch.body {
                    let Statement::Assign(target, subscripts, value) = inner else {
                        return Err("a `when` in an algorithm holds assignments".to_string());
                    };
                    if !subscripts.is_empty() {
                        return Err(
                            "a `when` in an algorithm assigns whole variables, not elements"
                                .to_string(),
                        );
                    }
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
            acc.when_clauses.push(WhenClause { branches: lifted });
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

    // `for` equations are unrolled: the loop variable is a constant.
    for loop_eq in &class.for_equations {
        unroll(
            loop_eq,
            &HashMap::new(),
            &local_consts,
            prefix,
            &outers,
            &sizes_here,
            registry,
            scope,
            &imports,
            acc,
        )?;
    }

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
        let decidable = if_equation.branches.iter().all(|branch| {
            branch
                .condition
                .as_ref()
                .is_none_or(|condition| const_eval(condition, &env).is_some())
        });
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
                    let value = const_eval(condition, &env).ok_or_else(|| {
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
        for loop_eq in &branch.loops {
            unroll(
                loop_eq,
                &HashMap::new(),
                &local_consts,
                prefix,
                &outers,
                &sizes_here,
                registry,
                scope,
                &imports,
                acc,
            )?;
        }
        for equation in &branch.equations {
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

    for clause in &class.when_clauses {
        let mut branches = Vec::new();
        for branch in &clause.branches {
            let mut actions = Vec::new();
            for action in &branch.actions {
                match action {
                    WhenAction::Reinit(state, value) => actions.push(WhenAction::Reinit(
                        flat_name(state, prefix, &outers),
                        resolve_here(value)?,
                    )),
                    WhenAction::Assign(target, value) => actions.push(WhenAction::Assign(
                        flat_name(target, prefix, &outers),
                        resolve_here(value)?,
                    )),
                    WhenAction::Terminate(message) => {
                        actions.push(WhenAction::Terminate(message.clone()))
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
                            actions.push(WhenAction::Assign(
                                flat_name(target, prefix, &outers),
                                worth,
                            ));
                        }
                    }
                }
            }
            branches.push(WhenBranch {
                condition: resolve_here(&branch.condition)?,
                actions,
            });
        }
        acc.when_clauses.push(WhenClause { branches });
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
    acc.origin = stamped;
    Ok(())
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
        outers,
        inners,
        overrides,
        consts: local_consts,
        imports,
        scope,
    } = *level;
    {
        if is_primitive(&component.type_name) {
            let mut flat = component.clone();
            flat.name = flat_name.to_string();
            flat.dimensions = Vec::new();
            let resolve_value = |e: &Expr| -> Result<Expr, String> {
                let e = substitute_class_constants(e, registry, scope, imports, &[]);
                resolve(
                    &prefix_expr(&e, prefix, outers),
                    &HashMap::new(),
                    local_consts,
                    registry,
                    scope,
                    imports,
                    0,
                )
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
                flat.binding = Some(value);
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
                if let Some(value) = flat.binding.take() {
                    acc.equations.push(EquationItem {
                        lhs: Expr::Ref(flat.name.clone()),
                        rhs: value,
                        origin: String::new(),
                    });
                }
            }
            // A connector that is one value is still a connector: a
            // `connect` naming it joins the values themselves.
            if let Some(class_name) = value_connector {
                acc.connectors
                    .insert(flat_name.to_string(), class_name.to_string());
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
            if child.kind == ClassKind::Connector {
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
            let child_prefix = format!("{flat_name}.");
            let child_env = Env {
                overrides: &mods,
                redeclares,
                inners,
                broken: &[],
            };
            instantiate(registry, child, &child_prefix, &child_env, acc, depth + 1)?;
        }
    }
    Ok(())
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
    let Ok(measured) = expand(value, &shapes, registry, scope, imports, 0) else {
        return value.clone();
    };
    let mut items = Vec::new();
    measured.flatten_into(&mut items);
    if items.len() == count {
        items[position].clone()
    } else {
        value.clone()
    }
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
            ForBody::Nested(inner) => {
                if found.is_none() {
                    found = implied_range(&inner.body, variable, prefix, outers, sizes)
                        .ok()
                        .map(|values| values.len() as i64);
                }
            }
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
            let spread = expand(
                &prefix_expr(range, prefix, outers),
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
                        records: no_records(),
                    };
                    let side = |expr: &Expr| -> Result<Value, String> {
                        let expr = substitute_refs(expr, &folded);
                        let expr = substitute_class_constants(&expr, registry, scope, imports, &[]);
                        expand(
                            &prefix_expr(&expr, prefix, outers),
                            &shapes,
                            registry,
                            scope,
                            imports,
                            0,
                        )
                    };
                    push_equations(&side(&equation.lhs)?, &side(&equation.rhs)?, acc)?;
                }
                ForBody::Connect(a, b) => {
                    let shapes = Shapes {
                        sizes,
                        loop_vars: &loop_vars,
                        consts,
                        records: no_records(),
                    };
                    let (a, b) = (substitute_refs(a, &folded), substitute_refs(b, &folded));
                    push_connects(
                        &a, &b, &shapes, prefix, outers, registry, scope, imports, acc,
                    )?;
                }
                ForBody::Nested(inner) => unroll(
                    inner, &loop_vars, consts, prefix, outers, sizes, registry, scope, imports, acc,
                )?,
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
                    origin: String::new(),
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
pub(super) fn effective_imports(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    scope: &str,
    redeclares: &[Redeclare],
) -> Result<Vec<(String, String)>, String> {
    let mut imports = class.imports.clone();
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
    let scope = class.name.as_str();
    let mut outers = HashMap::new();
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
pub(super) fn qualify_redeclare(
    redeclare: &Redeclare,
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    prefix: &str,
    outers: &HashMap<String, String>,
) -> Result<Redeclare, String> {
    let scope = class.name.as_str();
    let target =
        lookup(registry, &redeclare.type_name, scope, &class.imports).ok_or_else(|| {
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
                let e = substitute_class_constants(e, registry, scope, &class.imports, &[]);
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
            return;
        };
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
    let (head, rest) = match name.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (name, None),
    };
    if let Some(path) = outers.get(head) {
        return match rest {
            Some(rest) => format!("{path}.{rest}"),
            None => path.clone(),
        };
    }
    format!("{prefix}{name}")
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
