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
    static DERIVABLE: RefCell<HashMap<String, (String, Vec<bool>)>> =
        RefCell::new(HashMap::new());
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
pub(crate) fn derivative_of(name: &str) -> Option<(String, Vec<bool>)> {
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
    let mut tried: Vec<String> = Vec::new();
    // A body is written out once every body it calls has been, so the
    // passes go on while one of them adds something. Each derived body
    // promises one order less, which is what brings this to an end.
    loop {
        let mut added = false;
        for at in 0..functions.len() {
            let class = &functions[at];
            if smooth_order(class) < 1.0
                || tried.contains(&class.name)
                || derivative_of(&class.name).is_some()
            {
                continue;
            }
            match derive(class) {
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
fn derive(class: &ClassDef) -> Result<(ClassDef, Vec<bool>), String> {
    if class.external || class.builtin.is_some() || class.algorithm.is_empty() {
        return Err(format!(
            "`{}` has no statements to differentiate",
            class.name
        ));
    }
    if class.components.iter().any(|c| !c.dimensions.is_empty()) {
        return Err(format!("`{}` declares an array", class.name));
    }
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
    let still = HashMap::new();
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
        mask.push(carries_rate(input));
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
        Box::new(Expr::Number(smooth_order(class) - 1.0)),
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
