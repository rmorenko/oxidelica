//! The derivative of a body the run walks, written as a body of its own.
//!
//! A function nothing could inline is left standing as a call, and the
//! run walks its statements one number at a time. Index reduction then
//! meets that call as something to differentiate, and a walked body is
//! opaque to the differentiator: `regRoot2` of the fluid library has a
//! `return` under a condition the compiler cannot decide, and every
//! model asking for the derivative of a flow through it was refused.
//!
//! The function itself says the derivative exists: `smoothOrder = 2`
//! is the author's promise that the body is twice continuously
//! differentiable. What it does not say is how, and the body is the
//! only place that knows. So the derivative is written out from the
//! body, statement by statement, in the forward mode: every assignment
//! `v := e` is preceded by `$d1_v := de`, where `de` is what this
//! compiler's own differentiator makes of `e` with every name in the
//! body standing for its own rate. The branches, the loops and the
//! `return` are kept exactly as they were, so the derivative takes the
//! same road through the body as the value does, and a `return` the
//! compiler cannot decide is decided by the walk, as it is for the
//! value.
//!
//! Not numerical: no step size is chosen and nothing is approximated.
//! The answer is the exact derivative of the road the walk took, which
//! is the derivative of the function wherever the road does not change.
//! Where it does change, the `smoothOrder` promise is what says the two
//! roads agree in value and slope.
//!
//! A derived body is itself a walked body with one order of smoothness
//! less, so the second derivative a model asks for is built the same
//! way from the first.

use crate::*;
use std::cell::RefCell;

thread_local! {
    /// The walked bodies with a derivative written out: the name of the
    /// body, the name of its derivative, and for each input whether a
    /// rate goes with it. Set per model by `with_derivative_bodies`.
    static DERIVABLE: RefCell<HashMap<String, (String, Vec<Handed>)>> =
        RefCell::new(HashMap::new());
}

/// What a call to a derived body hands for one input of the value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Handed {
    /// The input's rate goes in front, then the input itself.
    Rate,
    /// The input does not move continuously - a flag, a count - and
    /// only the input itself is handed.
    Nothing,
    /// An array input, read inside the derived body as standing still.
    /// That reading is true only where the call's argument does not
    /// move, so the call checks it there and refuses where it does:
    /// a moving array handed to a body that read it as still would be
    /// a derivative with terms missing, a wrong number.
    Still,
}

/// Whether a body with an array among its inputs is written out, each
/// array read as standing still. `OXIDELICA_NO_STILL_ARRAY_INPUT`
/// keeps the old reading, which refused every body declaring an array.
fn still_array_inputs_open() -> bool {
    !walk::switch_set("OXIDELICA_NO_STILL_ARRAY_INPUT")
}

/// Whether a walked body is differentiated through its derived body.
/// `OXIDELICA_NO_WALKED_DERIVATIVE` keeps the old reading, which
/// refused the derivative of every walked call, so one binary can be
/// measured against itself.
fn walked_derivatives_open() -> bool {
    !walk::switch_set("OXIDELICA_NO_WALKED_DERIVATIVE")
}

/// The derived body of a walked function, where one was written out:
/// its name, and which inputs carry a rate.
pub(crate) fn derivative_of(name: &str) -> Option<(String, Vec<Handed>)> {
    DERIVABLE.with(|known| known.borrow().get(name).cloned())
}

/// The order of smoothness a function promises, or zero.
fn smooth_order(class: &ClassDef) -> f64 {
    class
        .annotations
        .iter()
        .find_map(|entry| match entry {
            Expr::NamedArg(name, value) if name == "smoothOrder" => match value.as_ref() {
                Expr::Number(order) => Some(*order),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or(0.0)
}

/// Whether a body is nothing but assignments: no branch, no loop, no
/// `return`. Such a body is one expression written in steps, and its
/// derivative is that expression's - exactly what inlining it would
/// have handed the differentiator. `Modelica.Math.log` is `y :=
/// .log(u)` and promises nothing, and a walked `h_T` that calls it
/// could not be written out for want of the log's rate.
/// `OXIDELICA_NO_STRAIGHT_BODY` keeps the old reading.
fn straight(class: &ClassDef) -> bool {
    !walk::switch_set("OXIDELICA_NO_STRAIGHT_BODY")
        && !class.name.ends_with("$der")
        && class
            .algorithm
            .iter()
            .all(|s| matches!(s, Statement::Assign(..)))
}

/// The order of smoothness a body may be differentiated to: what it
/// promises, or for a straight body as many orders as any body of the
/// model promises, since that is the most a promised body calling it
/// can ask for. A derived body holds the order it was given and no
/// more, which is what brings the writing out to an end.
fn order_of(class: &ClassDef, ceiling: f64) -> f64 {
    match straight(class) {
        true => smooth_order(class).max(ceiling),
        false => smooth_order(class),
    }
}

/// Whether a declaration holds a number with a rate of change. A
/// Boolean, an Integer and a String do not move continuously, and
/// their rate is nothing.
fn carries_rate(component: &Component) -> bool {
    !matches!(
        component.type_name.as_str(),
        "Boolean" | "Integer" | "String"
    )
}

/// The prefix a body's rates are held under: the first `$dN_` no name
/// of the body already begins with. A derived body is differentiated
/// again for a second derivative, and its own rates are inputs by
/// then - so the rate of `x` must not be spelled as the input `$d_x`
/// that already stands for something else. Spelled the same, the
/// second body read one name for two numbers and the first derivative
/// of `k*x^2` came out as `-2x`.
fn rate_prefix(class: &ClassDef) -> String {
    (1..)
        .map(|level| format!("$d{level}_"))
        .find(|prefix| {
            !class
                .components
                .iter()
                .any(|c| c.name.starts_with(prefix.as_str()))
        })
        .expect("a body declares finitely many names")
}

/// The model with a derived body for every walked function that
/// promises a derivative and can be written out, or `None` where there
/// is none. The register of derived bodies is set for this thread
/// either way, so a model with none never reads another's.
pub(crate) fn with_derivative_bodies(model: &Model) -> Option<Model> {
    DERIVABLE.with(|known| known.borrow_mut().clear());
    if !walked_derivatives_open() || model.functions.is_empty() {
        return None;
    }
    let mut functions = model.functions.clone();
    let ceiling = functions.iter().map(smooth_order).fold(0.0, f64::max);
    let mut tried: Vec<String> = Vec::new();
    // A body is written out once every body it calls has been, so the
    // passes go on while one of them adds something. Each derived body
    // promises one order less, which is what brings this to an end.
    loop {
        let mut added = false;
        for at in 0..functions.len() {
            let class = &functions[at];
            if order_of(class, ceiling) < 1.0
                || tried.contains(&class.name)
                || derivative_of(&class.name).is_some()
            {
                continue;
            }
            match derive(class, order_of(class, ceiling)) {
                Ok((derived, mask)) => {
                    DERIVABLE.with(|known| {
                        known
                            .borrow_mut()
                            .insert(class.name.clone(), (derived.name.clone(), mask))
                    });
                    functions.push(derived);
                    added = true;
                    tried.clear();
                    break;
                }
                Err(_) => tried.push(class.name.clone()),
            }
        }
        if !added {
            break;
        }
    }
    if functions.len() == model.functions.len() {
        return None;
    }
    let mut out = model.clone();
    out.functions = functions;
    Some(out)
}

/// Turns on, for as long as it lives, the rules that hold inside a
/// body and nowhere else.
struct InBody;

impl InBody {
    fn enter() -> InBody {
        symbolic::IN_BODY.with(|on| on.set(true));
        InBody
    }
}

impl Drop for InBody {
    fn drop(&mut self) {
        symbolic::IN_BODY.with(|on| on.set(false));
    }
}

/// Write out the derivative of one body, or say why it cannot be.
fn derive(class: &ClassDef, order: f64) -> Result<(ClassDef, Vec<Handed>), String> {
    if class.external || class.builtin.is_some() || class.algorithm.is_empty() {
        return Err(format!(
            "`{}` has no statements to differentiate",
            class.name
        ));
    }
    // An array input is taken as standing still: the gas data of the
    // media, a table of coefficients. Its elements are read in the
    // body by their own names, `data[4]`, and each stands for a rate
    // of nothing. What moves is the scalars; the call is where the
    // array is checked not to. An array declared anywhere else in the
    // body would need a rate laid out as an array, which this does not
    // write.
    let still_array = |c: &Component| {
        still_array_inputs_open() && c.causality == Causality::Input && !c.dimensions.is_empty()
    };
    if let Some(array) = class
        .components
        .iter()
        .find(|c| !c.dimensions.is_empty() && !still_array(c))
    {
        return Err(format!(
            "`{}` declares an array `{}`",
            class.name, array.name
        ));
    }
    let still_names: Vec<&str> = class
        .components
        .iter()
        .filter(|c| still_array(c))
        .map(|c| c.name.as_str())
        .collect();
    let Some(output) = class
        .components
        .iter()
        .find(|c| c.causality == Causality::Output)
    else {
        return Err(format!("`{}` answers with nothing", class.name));
    };
    if !carries_rate(output) {
        return Err(format!("`{}` answers with a discrete value", class.name));
    }
    // Every name of the body stands for its own rate, and a name that
    // does not move - a flag, a count, a loop's index - for nothing.
    let prefix = rate_prefix(class);
    let rate = |name: &str| format!("{prefix}{name}");
    let mut rates: HashMap<String, Expr> = HashMap::new();
    for component in &class.components {
        let worth = match carries_rate(component) {
            true => Expr::Ref(rate(&component.name)),
            false => Expr::Number(0.0),
        };
        rates.insert(component.name.clone(), worth);
    }
    let mut indices = Vec::new();
    loop_indices(&class.algorithm, &mut indices);
    for index in indices {
        rates.entry(index).or_insert(Expr::Number(0.0));
    }
    // A still array and every element of it the body reads stand in
    // the table of what does not move with time, and not among the
    // rates: `a[1]` written in a body is a subscript of `a`, and in a
    // flattened one the element is a name of its own, `data[4]`; both
    // are still. The table holds values for the proofs of sign that
    // `abs` asks for, and an array input has no value until the call,
    // so each holds not a number: a proof that leans on one fails and
    // the body is refused, rather than proved by a value it never had.
    let mut still: HashMap<String, f64> = HashMap::new();
    for name in &still_names {
        rates.remove(*name);
        still.insert(name.to_string(), f64::NAN);
    }
    each_read(&class.algorithm, &mut |name| {
        if let Some((head, _)) = name.split_once('[') {
            if still_names.contains(&head) && name.ends_with(']') {
                still.insert(name.to_string(), f64::NAN);
            }
        }
    });
    let none = HashMap::new();
    let no_defs = HashMap::new();
    let no_implicit = HashMap::new();
    let target = DiffTarget::Time {
        state_rhs: &rates,
        params: &still,
        dummies: &none,
        alg_defs: &no_defs,
        implicit_defs: &no_implicit,
        holding: &[],
    };
    let _inside = InBody::enter();
    let rate_of = |expr: &Expr| -> Result<Expr, String> {
        let worked = differentiate(expr, &target)?;
        let needed = take_needed_derivatives();
        if !needed.is_empty() {
            return Err(format!(
                "the body reads `{}`, which it does not declare",
                needed[0]
            ));
        }
        Ok(simplify(&worked))
    };
    // The rates first, in the order the value's inputs come, so that a
    // call may leave the value's trailing inputs to their defaults.
    let mut components = Vec::new();
    let mut mask = Vec::new();
    for input in class
        .components
        .iter()
        .filter(|c| c.causality == Causality::Input)
    {
        if still_names.contains(&input.name.as_str()) {
            mask.push(Handed::Still);
            continue;
        }
        mask.push(match carries_rate(input) {
            true => Handed::Rate,
            false => Handed::Nothing,
        });
        if carries_rate(input) {
            components.push(rate_component(
                input,
                rate(&input.name),
                Causality::Input,
                None,
            ));
        }
    }
    for component in &class.components {
        let mut kept = component.clone();
        if kept.causality == Causality::Output {
            kept.causality = Causality::None;
        }
        components.push(kept);
        if component.causality == Causality::Input || !carries_rate(component) {
            continue;
        }
        let causality = match component.name == output.name {
            true => Causality::Output,
            false => Causality::None,
        };
        let binding = component.binding.as_ref().map(rate_of).transpose()?;
        components.push(rate_component(
            component,
            rate(&component.name),
            causality,
            binding,
        ));
    }
    let algorithm = statements(&class.algorithm, &rates, &rate_of)?;
    let mut derived = class.clone();
    derived.name = format!("{}$der", class.name);
    derived.components = components;
    derived.algorithm = algorithm;
    derived.derivative = None;
    derived.annotations = vec![Expr::NamedArg(
        "smoothOrder".to_string(),
        Box::new(Expr::Number(order - 1.0)),
    )];
    Ok((derived, mask))
}

/// The declaration a rate is held under: the value's own, renamed,
/// with nothing of the value's written on it.
fn rate_component(
    of: &Component,
    name: String,
    causality: Causality,
    binding: Option<Expr>,
) -> Component {
    let mut held = of.clone();
    held.name = name;
    held.causality = causality;
    held.binding = binding;
    held.start = None;
    held.start_from_type = false;
    held.start_given_by_package = None;
    held.min = None;
    held.max = None;
    held.nominal = None;
    held
}

/// Every name a body's statements read, in conditions, values, ranges
/// and subscripts alike.
fn each_read(list: &[Statement], look: &mut dyn FnMut(&str)) {
    let expr = |e: &Expr, look: &mut dyn FnMut(&str)| {
        let mut refs = Vec::new();
        e.collect_refs(&mut refs);
        for r in refs {
            look(r);
        }
    };
    for statement in list {
        match statement {
            Statement::Assign(_, subscripts, value) => {
                subscripts.iter().for_each(|s| expr(s, look));
                expr(value, look);
            }
            Statement::TupleAssign(targets, value) => {
                for (_, subscripts) in targets.iter().flatten() {
                    subscripts.iter().for_each(|s| expr(s, look));
                }
                expr(value, look);
            }
            Statement::If(branches) | Statement::When(branches) => {
                for branch in branches {
                    if let Some(condition) = &branch.condition {
                        expr(condition, look);
                    }
                    each_read(&branch.body, look);
                }
            }
            Statement::For(_, range, inner) => {
                if let Some(range) = range {
                    expr(range, look);
                }
                each_read(inner, look);
            }
            Statement::While(condition, inner) => {
                expr(condition, look);
                each_read(inner, look);
            }
            Statement::Call(_, args) => args.iter().for_each(|a| expr(a, look)),
            Statement::Assert(condition, message) => {
                expr(condition, look);
                expr(message, look);
            }
            Statement::Break | Statement::Return => {}
        }
    }
}

/// The names a body's `for` loops count with.
fn loop_indices(list: &[Statement], out: &mut Vec<String>) {
    for statement in list {
        match statement {
            Statement::For(index, _, inner) => {
                out.push(index.clone());
                loop_indices(inner, out);
            }
            Statement::While(_, inner) => loop_indices(inner, out),
            Statement::If(branches) | Statement::When(branches) => {
                for branch in branches {
                    loop_indices(&branch.body, out);
                }
            }
            _ => {}
        }
    }
}

/// The statements of the derived body: each assignment preceded by the
/// assignment of its rate, and everything that decides the road kept as
/// it was.
fn statements(
    list: &[Statement],
    rates: &HashMap<String, Expr>,
    rate_of: &dyn Fn(&Expr) -> Result<Expr, String>,
) -> Result<Vec<Statement>, String> {
    let mut out = Vec::new();
    for statement in list {
        match statement {
            Statement::Assign(target, subscripts, value) if subscripts.is_empty() => {
                match rates.get(target) {
                    Some(Expr::Ref(held)) => {
                        out.push(Statement::Assign(held.clone(), Vec::new(), rate_of(value)?))
                    }
                    Some(_) => {}
                    None => {
                        return Err(format!(
                            "the body assigns `{target}`, which it does not declare"
                        ))
                    }
                }
                out.push(statement.clone());
            }
            Statement::If(branches) => {
                let mut kept = Vec::new();
                for branch in branches {
                    kept.push(oxidelica_parser::StatementBranch {
                        condition: branch.condition.clone(),
                        body: statements(&branch.body, rates, rate_of)?,
                    });
                }
                out.push(Statement::If(kept));
            }
            Statement::For(index, range, inner) => out.push(Statement::For(
                index.clone(),
                range.clone(),
                statements(inner, rates, rate_of)?,
            )),
            Statement::While(condition, inner) => out.push(Statement::While(
                condition.clone(),
                statements(inner, rates, rate_of)?,
            )),
            Statement::Break | Statement::Return | Statement::Assert(..) => {
                out.push(statement.clone())
            }
            Statement::Assign(..)
            | Statement::TupleAssign(..)
            | Statement::When(_)
            | Statement::Call(..) => {
                return Err("a statement the forward mode does not write out".to_string())
            }
        }
    }
    Ok(out)
}
