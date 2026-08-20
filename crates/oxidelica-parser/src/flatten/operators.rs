//! Operators a record declares for itself.

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
    let mut arguments = Vec::new();
    let mut argument_shapes = Vec::new();
    for arg in args {
        let value = expand(arg, shapes, registry, scope, imports, depth + 1)?;
        argument_shapes.push(shape_i64(&value));
        arguments.push(value.into_expr());
    }
    let result = inline_function_outputs(
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
pub(super) fn record_fields(class: &ClassDef) -> Vec<String> {
    class
        .components
        .iter()
        .map(|component| component.name.clone())
        .collect()
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
        let inputs: Vec<&Component> = class
            .components
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
