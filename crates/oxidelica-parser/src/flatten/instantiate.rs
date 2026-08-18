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

    // Bases first, with their modifiers (already parent-scoped).
    for extend in &class.extends {
        let base = lookup(registry, &extend.base, scope, &imports)
            .ok_or_else(|| format!("unknown base class `{}`", extend.base))?;
        let mods: Vec<(String, Expr)> = extend
            .modifiers
            .iter()
            .map(|(n, e)| {
                let e = substitute_class_constants(e, registry, scope, &imports);
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
        };
        instantiate(registry, base, prefix, &base_env, acc, depth + 1)?;
    }
    let overrides = env.overrides;

    // Parameter values of this class, resolved to numbers where
    // possible: array dimensions and loop bounds are compile-time
    // constants and must come from here.
    let mut local_consts: HashMap<String, f64> = HashMap::new();
    loop {
        let mut progress = false;
        for component in &class.components {
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
                .or_else(|| {
                    component
                        .binding
                        .as_ref()
                        .or(component.start.as_ref())
                        .map(|e| {
                            let e = substitute_class_constants(e, registry, scope, &imports);
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

        // Array dimensions expand into scalar elements.
        let mut sizes = Vec::new();
        for dimension in &component.dimensions {
            let value = const_eval(dimension, &local_consts).ok_or_else(|| {
                format!("dimension of `{flat_name}` is not a compile-time constant")
            })?;
            if value.fract() != 0.0 || value < 0.0 {
                return Err(format!(
                    "dimension of `{flat_name}` must be a whole number that is not negative, \
                     got {value}"
                ));
            }
            sizes.push(value as i64);
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

        // A `type` alias stands for a primitive plus attribute defaults,
        // and an enumeration for an `Integer`; substitute before
        // instantiating.
        resolve_type(registry, &mut component, scope, &imports);

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
                let expr = substitute_class_constants(expr, registry, scope, &imports);
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

        for (position, local_name) in element_names.iter().enumerate() {
            let flat_name = format!("{prefix}{local_name}");
            let site = Site {
                component: &component,
                local_name,
                flat_name: &flat_name,
                extra_modifiers: &extra_modifiers,
                redeclares: &child_redeclares,
                binding: element_bindings.as_ref().map(|items| &items[position]),
                start: element_starts.as_ref().map(|items| &items[position]),
            };
            instantiate_one(registry, &site, &level, acc, depth)?;
        }
    }

    // Equations: arrays expanded, subscripts resolved, calls inlined.
    let resolve_here = |expr: &Expr| -> Result<Expr, String> {
        let expr = substitute_class_constants(expr, registry, scope, &imports);
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
        let expr = substitute_class_constants(expr, registry, scope, &imports);
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
            let rhs = substitute_class_constants(&equation.rhs, registry, scope, &imports);
            let rhs = prefix_expr(&rhs, prefix, &outers);
            let Expr::Call(name, raw_args) = &rhs else {
                return Err("the right side of a tuple equation must be a function call".into());
            };
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
            acc.initial_equations.push(EquationItem { lhs, rhs });
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
        match execute(
            &plain,
            &mut bindings,
            &mut assigned,
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
            let actions = branch
                .actions
                .iter()
                .map(|action| match action {
                    WhenAction::Reinit(state, value) => Ok(WhenAction::Reinit(
                        flat_name(state, prefix, &outers),
                        resolve_here(value)?,
                    )),
                    WhenAction::Assign(target, value) => Ok(WhenAction::Assign(
                        flat_name(target, prefix, &outers),
                        resolve_here(value)?,
                    )),
                    WhenAction::Terminate(message) => Ok(WhenAction::Terminate(message.clone())),
                })
                .collect::<Result<Vec<_>, String>>()?;
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
        redeclares,
        binding: _,
        start: _,
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
                let e = substitute_class_constants(e, registry, scope, imports);
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
                    });
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
                .chain(component.modifiers.iter().map(|(n, e)| {
                    let e = substitute_class_constants(e, registry, scope, imports);
                    (n.clone(), prefix_expr(&e, prefix, outers))
                }))
                .collect();
            let child_prefix = format!("{flat_name}.");
            let child_env = Env {
                overrides: &mods,
                redeclares,
                inners,
            };
            instantiate(registry, child, &child_prefix, &child_env, acc, depth + 1)?;
        }
    }
    Ok(())
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
    let bound = |expr: &Expr| -> Result<i64, String> {
        let mut env = consts.clone();
        env.extend(outer_vars.iter().map(|(k, v)| (k.clone(), *v)));
        // A bound may ask an array how long it is, so it goes through
        // the same expansion as everything else before being folded.
        let shapes = Shapes {
            sizes,
            loop_vars: outer_vars,
            consts,
            records: no_records(),
        };
        let expr = &expand(
            &prefix_expr(expr, prefix, outers),
            &shapes,
            registry,
            scope,
            imports,
            0,
        )?
        .scalar()?;
        let value = const_eval(expr, &env)
            .ok_or_else(|| format!("loop bound is not a compile-time constant: {expr:?}"))?;
        if value.fract() != 0.0 {
            return Err(format!("loop bound must be a whole number, got {value}"));
        }
        Ok(value as i64)
    };
    let (lower, upper) = (bound(&loop_eq.range.0)?, bound(&loop_eq.range.1)?);
    for index in lower..=upper {
        let mut loop_vars = outer_vars.clone();
        loop_vars.insert(loop_eq.variable.clone(), index as f64);
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
                        let expr = substitute_class_constants(&expr, registry, scope, imports);
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
                scalars.push(EquationItem { lhs, rhs });
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
        if let Some(declared) = lookup(registry, &component.type_name, scope, &class.imports) {
            out.insert(
                component.name.clone(),
                InnerInstance {
                    path: format!("{prefix}{}", component.name),
                    class: declared.name.clone(),
                },
            );
        }
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
        let declared =
            lookup(registry, &component.type_name, scope, &class.imports).ok_or_else(|| {
                format!(
                    "unknown type `{}` of outer component `{}`",
                    component.type_name, component.name
                )
            })?;
        if !extends_class(registry, &inner.class, &declared.name, 0) {
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
                let e = substitute_class_constants(e, registry, scope, &class.imports);
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
        if component.unit.is_none() {
            component.unit = class.alias_unit.clone();
        }
        for (name, value) in attributes {
            match name.as_str() {
                "start" if component.start.is_none() => component.start = Some(value),
                "fixed" if component.fixed.is_none() => {
                    component.fixed = Some(matches!(value, Expr::Bool(true)))
                }
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
