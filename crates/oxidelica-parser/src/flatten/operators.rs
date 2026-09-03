//! Operators a record declares for itself.

use super::shapes::*;
use super::*;

/// How an operator is spelled where a record declares it.
pub(super) fn operator_symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Pow => "^",
    }
}

/// How a relational operator is spelled where a record declares one.
pub(super) fn relation_symbol(op: RelOp) -> &'static str {
    match op {
        RelOp::Lt => "<",
        RelOp::Le => "<=",
        RelOp::Gt => ">",
        RelOp::Ge => ">=",
        RelOp::Eq => "==",
        RelOp::Ne => "<>",
    }
}

/// Work an overloaded operator, or say why the record cannot.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_operator(
    record: &str,
    symbol: &str,
    args: &[Expr],
    shapes: &Shapes,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
    depth: usize,
) -> Result<Value, String> {
    let Some(function) = operator_function(registry, record, symbol, args.len()) else {
        return Err(format!(
            "`{record}` has no operator `{symbol}` taking {} argument(s)",
            args.len()
        ));
    };
    let recur = |e: &Expr| expand(e, shapes, registry, scope, imports, depth + 1);
    let mut values = args
        .iter()
        .map(&recur)
        .collect::<Result<Vec<_>, String>>()?;
    // A number standing where the operator wants a record is that
    // record built from it: `N*i` of a `Complex` and a `Real` is the
    // multiplication of two complex numbers, the second of them with
    // no imaginary part. The library writes it that way and declares
    // no operator for the mixture, because the language says a value
    // is converted by the record's own constructor where one applies.
    let inputs: Vec<Component> = inlining::function_components(registry, function, 0)
        .into_iter()
        .filter(|c| c.causality == Causality::Input)
        .collect();
    for (input, value) in inputs.iter().zip(&mut values) {
        let wants_record = input.dimensions.is_empty()
            && record_fields::record_input_fields(registry, function, input)
                .is_some_and(|f| f.len() > 1);
        if wants_record && matches!(value, Value::Scalar(_)) {
            let number = value.clone().into_expr();
            *value = expand(
                &Expr::Call(input.type_name.clone(), vec![number]),
                shapes,
                registry,
                &function.name,
                &function.imports,
                depth + 1,
            )?;
        }
    }
    // Two vectors multiplied are their scalar product, not a
    // multiplication apiece. `y = k*u` of two `Complex[3]` is one
    // complex number - the way `Real[3]` times `Real[3]` is one real -
    // and it is how the complex blocks write a sum: `Sum` of the
    // complex library multiplies its gains by its inputs and answers
    // with a single value. Vectorized instead it answered with three,
    // and an equation between one record and three was refused.
    //
    // Only for `*`, only where both sides are arrays of the record,
    // and only where the record can add: the product of the pairs is
    // summed with the record's own `+`, which is what the language
    // means by a scalar product over anything but numbers.
    if symbol == "*" && values.len() == 2 {
        let lengths: Vec<Option<usize>> = values
            .iter()
            .map(|value| match value {
                Value::Array(items) => Some(items.len()),
                Value::Scalar(_) => None,
            })
            .collect();
        if let (Some(left), Some(right)) = (lengths[0], lengths[1]) {
            // A vector of records, not a record of fields: the fields
            // of one are what the operator's own body reads, and it
            // is the elements that pair off here.
            let of_records = values.iter().all(|value| {
                spread_of_records(function, registry, shapes, std::slice::from_ref(value)).is_some()
            });
            if of_records && operator_function(registry, record, "+", 2).is_some() {
                if left != right {
                    return Err(format!(
                        "a scalar product of `{record}` needs equal lengths, got {left} and {right}"
                    ));
                }
                let mut total: Option<Value> = None;
                for index in 0..left {
                    let each = one_of_each(&values, left, index, shapes, registry, &recur)?;
                    let product = apply_operator(
                        record,
                        symbol,
                        &each,
                        shapes,
                        registry,
                        scope,
                        imports,
                        depth + 1,
                    )?;
                    total = Some(match total {
                        None => product,
                        Some(so_far) => apply_operator(
                            record,
                            "+",
                            &[so_far.into_expr(), product.into_expr()],
                            shapes,
                            registry,
                            scope,
                            imports,
                            depth + 1,
                        )?,
                    });
                }
                if let Some(total) = total {
                    return Ok(total);
                }
            }
        }
    }
    // An operator written for one record and handed a whole array of
    // them works on each in turn: `v1 - v2` of two `Complex[3]` is
    // three subtractions. The language vectorizes a function this way
    // and an operator is one.
    if let Some(spread) = spread_of_records(function, registry, shapes, &values) {
        let elements = (0..spread)
            .map(|index| {
                let each = one_of_each(&values, spread, index, shapes, registry, &recur)?;
                apply_operator(
                    record,
                    symbol,
                    &each,
                    shapes,
                    registry,
                    scope,
                    imports,
                    depth + 1,
                )
            })
            .collect::<Result<Vec<_>, String>>()?;
        return Ok(Value::Array(elements));
    }
    let argument_shapes: Vec<Vec<i64>> = values.iter().map(shape_i64).collect();
    let arguments: Vec<Expr> = values.into_iter().map(Value::into_expr).collect();
    let result = inlining::inline_function_outputs(
        function,
        &arguments,
        &argument_shapes,
        shapes.consts,
        registry,
        depth + 1,
    )?
    .remove(0)
    .1;
    expand(&result, shapes, registry, scope, imports, depth + 1)
}

/// The record class an expression is of, when it is of one.
///
/// An overloaded operator is chosen by the record its operands belong
/// to, and that has to be known before the operands are expanded -
/// once they are, a record and an array of the same length look alike.
pub(super) fn record_class_of(
    expr: &Expr,
    shapes: &Shapes,
    registry: &HashMap<&str, &ClassDef>,
    scope: &str,
    imports: &[(String, String)],
) -> Option<String> {
    let recur = |e: &Expr| record_class_of(e, shapes, registry, scope, imports);
    match expr {
        Expr::Ref(name) => shapes.records.get(name).cloned(),
        // An operator returns the record it was found on; the operands
        // of a mixed expression need only one of them to say which.
        Expr::Bin(_, l, r) | Expr::Elementwise(_, l, r) => recur(l).or_else(|| recur(r)),
        Expr::Neg(inner) => recur(inner),
        Expr::If(_, then, otherwise) => recur(then).or_else(|| recur(otherwise)),
        // `Complex(1, 2)` builds one.
        Expr::Call(name, _) => lookup(registry, name, scope, imports)
            .filter(|class| class.kind == ClassKind::Record)
            .map(|class| class.name.clone()),
        _ => None,
    }
}

/// The record class the elements of an array belong to, when they
/// belong to one: `V arr[3]` gives `V`. An array of records is filed
/// under both tables - a shape and a record class - and it is the
/// second that says what its elements are.
pub(super) fn element_record_of(expr: &Expr, shapes: &Shapes) -> Option<String> {
    let Expr::Ref(name) = expr else {
        return None;
    };
    shapes.sizes.get(name)?;
    shapes.records.get(name).cloned()
}

/// The fields of a record class, in the order they were declared.
///
/// Only what the class wrote for itself. A record that takes its
/// fields from a base - `redeclare record extends ThermodynamicState`,
/// which is how a medium says its state is the one it inherits - has
/// none of its own, and [`record_fields_of`] is what answers for it.
pub(super) fn record_fields(class: &ClassDef) -> Vec<String> {
    class
        .components
        .iter()
        .map(|component| component.name.clone())
        .collect()
}

/// The same, with the fields of every base first.
///
/// `redeclare record extends ThermodynamicState` writes no fields and
/// means the ones it extends: a medium's state is what its base
/// declared, and a function taking that state was told it takes
/// nothing. A class's own declarations replace an inherited one of the
/// same name rather than joining it, so a record naming a field its
/// base also names has one field, not two.
pub(super) fn record_fields_of(
    registry: &HashMap<&str, &ClassDef>,
    class: &ClassDef,
    depth: usize,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if depth > MAX_DEPTH {
        return out;
    }
    for extend in &class.extends {
        let base = match extend.from_base {
            true => inherited_class(registry, class, &extend.base, 0),
            false => lookup(registry, &extend.base, &class.name, &class.imports),
        };
        if let Some(base) = base {
            for field in record_fields_of(registry, base, depth + 1) {
                if !out.contains(&field) {
                    out.push(field);
                }
            }
        }
    }
    for component in &class.components {
        out.retain(|kept| kept != &component.name);
        out.push(component.name.clone());
    }
    out
}

/// The function a record offers for an operator, by symbol and by how
/// many arguments it takes.
///
/// A symbol may name a function outright or a package of them, which
/// is how `'-'` holds both the subtraction and the negation.
pub(super) fn operator_function<'a>(
    registry: &HashMap<&str, &'a ClassDef>,
    record: &str,
    symbol: &str,
    arity: usize,
) -> Option<&'a ClassDef> {
    let named = registry.get(format!("{record}.'{symbol}'").as_str())?;
    // An input with a value of its own may be left out: `Complex(1)`
    // calls a constructor that takes a real part and an imaginary one
    // that stands at zero unless it is given.
    let takes = |class: &ClassDef| {
        // Through the gatherer: an operator function of a record may
        // extend one written elsewhere, and then takes what that base
        // declared rather than nothing.
        let held = inlining::function_components(registry, class, 0);
        let inputs: Vec<&Component> = held
            .iter()
            .filter(|c| c.causality == Causality::Input)
            .collect();
        let wanted = inputs.iter().filter(|c| c.binding.is_none()).count();
        (wanted..=inputs.len()).contains(&arity)
    };
    if named.kind == ClassKind::Function {
        return takes(named).then_some(*named);
    }
    let prefix = format!("{record}.'{symbol}'.");
    let mut inside: Vec<&&ClassDef> = registry
        .iter()
        .filter(|(name, class)| {
            name.starts_with(&prefix) && class.kind == ClassKind::Function && takes(class)
        })
        .map(|(_, class)| class)
        .collect();
    inside.sort_by_key(|class| class.name.clone());
    inside.first().map(|class| **class)
}
