//! Function bodies the run walks for itself.
//!
//! Almost every function is inlined while the model is flattened, which
//! is what lets the compiler differentiate through one and fold it away
//! where the arguments are known. Two kinds cannot be: a function that
//! leads back to itself, which has no bottom to unroll to, and one
//! whose loop runs as long as the model says rather than as long as the
//! compiler can see. Those are left standing as calls, and this is what
//! answers them - a walk over the statements, one number at a time.
//!
//! Nothing here folds, orders or differentiates. A body walked this way
//! is opaque to all of that, which is the price of it running at all.

use crate::*;

/// How deep one call may lead to another. A function calling itself
/// with no way out would otherwise run the stack out rather than say
/// what is wrong.
const MAX_WALK: usize = 64;

/// The most rounds one loop may take. A `while` whose condition the
/// model never falsifies has to end somewhere, and ending with a
/// sentence beats ending with a hung process.
const MAX_ROUNDS: usize = 10_000_000;

/// What a scalar output nothing binds is laid out as before the body
/// runs: a NaN with a payload of its own, so that one still standing
/// after the run is known to be one the body never assigned. MLS 3.6
/// section 12.4.4: "If no binding equation is given for a non-input
/// component the variable is uninitialized ... It is an error to use
/// (or return) an uninitialized variable in a function." It used to be
/// laid out at zero, and a body whose only assignment to its output
/// stood in a branch the road did not take answered 0 and the model
/// ran on it.
const UNASSIGNED: u64 = 0x7ff8_dead_0000_0001;

/// Whether a switch is set in the environment, read once per process.
///
/// The walk asks its switches inside its innermost loop, and on macOS
/// `getenv` takes a lock the whole process shares. Sampled while a
/// water model walked IF97 on one thread of a corpus check, fifteen
/// percent of its busy time was `__findenv_locked` under
/// `elements_of`, and every other thread asking any switch queued on
/// the same lock: DynamicPipeEnergyConservationCheck ran in 212s alone
/// and in 417s beside two such models in one process. A switch is set
/// before the run and never during it, so its first answer is its
/// answer, and each thread asks the environment once per switch.
pub(crate) fn switch_set(name: &'static str) -> bool {
    thread_local! {
        static SEEN: std::cell::RefCell<Vec<(&'static str, bool)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }
    SEEN.with(|seen| {
        if let Some((_, set)) = seen.borrow().iter().find(|(had, _)| *had == name) {
            return *set;
        }
        let set = std::env::var_os(name).is_some();
        seen.borrow_mut().push((name, set));
        set
    })
}

/// Whether an unassigned output reads as zero, as it did before the
/// refusal: `OXIDELICA_UNASSIGNED_OUTPUT_ZERO` keeps the old reading so
/// one binary can be measured against itself.
fn unassigned_is_zero() -> bool {
    switch_set("OXIDELICA_UNASSIGNED_OUTPUT_ZERO")
}

/// Whether a local array whose written value the walk cannot lay out
/// stands at zero, as it did before that was refused:
/// `OXIDELICA_LOCAL_ARRAY_ZERO` keeps the old reading so one binary can
/// be measured against itself.
fn local_arrays_zero() -> bool {
    switch_set("OXIDELICA_LOCAL_ARRAY_ZERO")
}

/// Whether a walk lays out a local sized by a number it holds and
/// bound by `linspace`. `OXIDELICA_NO_WALKED_LINSPACE` keeps the old
/// reading, which left both to the evaluator, so one binary can be
/// measured against itself.
fn walked_linspace_open() -> bool {
    !switch_set("OXIDELICA_NO_WALKED_LINSPACE")
}

/// Where a walk left off: running on, out of a loop, or out of the
/// function.
#[derive(PartialEq)]
enum Flow {
    Onwards,
    Broke,
    Returned,
}

/// Walk a function body and give back what its output holds: one
/// number, or the elements of an array in turn.
pub(crate) fn walk(
    programs: &HashMap<String, ClassDef>,
    name: &str,
    args: &[f64],
    shapes: &[Vec<usize>],
    time: f64,
    depth: usize,
) -> Result<Vec<f64>, SimError> {
    if depth > MAX_WALK {
        return err(format!(
            "`{name}` called itself {MAX_WALK} deep without reaching an end"
        ));
    }
    let class = programs
        .get(name)
        .ok_or_else(|| SimError::from(format!("`{name}` is not a body this run carries")))?;
    let inputs: Vec<&Component> = class
        .components
        .iter()
        .filter(|component| component.causality == Causality::Input)
        .collect();
    // An input the caller left out stands at its own declared value:
    // the water of the library asks `region_pT(p, T)` of a body whose
    // third input is a region it defaults to zero. Only the inputs
    // with nothing to fall back on are required.
    let wanted = inputs
        .iter()
        .filter(|held| held.binding.is_none() && held.start.is_none())
        .count();
    if shapes.len() < wanted || shapes.len() > inputs.len() {
        return err(format!(
            "`{name}` takes {} argument(s), given {}",
            inputs.len(),
            shapes.len()
        ));
    }
    // The frame: the arguments under the names the body knows them by,
    // and everything else it declared starting where it was told to.
    // An array is held the way the flat model holds one - each element
    // under its own name, `v[1]`, `v[2]` - and how long it is is kept
    // beside it, since that is what `size` and a loop over it ask for.
    let mut frame = Frame::default();
    // Locals whose written value the walk could not lay out, and which
    // stand at NaN for it: an answer that comes out NaN with any of
    // these in the frame is refused naming them.
    let mut unlaid: Vec<String> = Vec::new();
    let mut taken = 0;
    for (input, shape) in inputs.iter().zip(shapes) {
        match shape.as_slice() {
            [] => {
                frame.numbers.insert(input.name.clone(), args[taken]);
                taken += 1;
            }
            [length] => {
                for index in 1..=*length {
                    frame
                        .numbers
                        .insert(format!("{}[{index}]", input.name), args[taken]);
                    taken += 1;
                }
                frame.lengths.insert(input.name.clone(), *length);
            }
            // A table goes in as a table: the rows one after another,
            // each element under the two subscripts the body writes -
            // `table[2, 1]` - which is how the flat model spells one
            // too. `size(table, 1)` reads the first of the two.
            dimensions => {
                let mut at = vec![1usize; dimensions.len()];
                let total: usize = dimensions.iter().product();
                for _ in 0..total {
                    let subscripts: Vec<String> =
                        at.iter().map(|index| index.to_string()).collect();
                    frame.numbers.insert(
                        format!("{}[{}]", input.name, subscripts.join(",")),
                        args[taken],
                    );
                    taken += 1;
                    for axis in (0..dimensions.len()).rev() {
                        at[axis] += 1;
                        if at[axis] <= dimensions[axis] {
                            break;
                        }
                        at[axis] = 1;
                    }
                }
                frame.shapes.insert(input.name.clone(), dimensions.to_vec());
                frame.lengths.insert(input.name.clone(), dimensions[0]);
            }
        }
    }
    for component in &class.components {
        // An input the caller filled in is already in the frame; one
        // it left out is laid out here like a local, which is where
        // its own declared value comes from. Only a scalar input:
        // an array one is laid out by the loop above, and a name in
        // `numbers` is how that loop says so.
        if component.causality == Causality::Input
            && (frame.numbers.contains_key(&component.name)
                || frame.lengths.contains_key(&component.name)
                || !component.dimensions.is_empty())
        {
            continue;
        }
        // A declaration of its own length is one the body may read
        // before it writes, so it is laid out before anything runs. A
        // constant or a start-valued array carries its numbers in a
        // binding - `constant Real[13] N_0 = {...}`, the coefficients of
        // the media's Helmholtz polynomial - and those are laid into the
        // elements here; without them each `N_0[k]` reads the zero the
        // placeholder left and the whole sum comes to nothing.
        if let Some(length) = declared_length(component, &frame) {
            frame.lengths.insert(component.name.clone(), length);
            let laid = component
                .binding
                .as_ref()
                .or(component.start.as_ref())
                .map(|expr| elements_of(expr, &frame, programs, time, depth))
                .transpose()?
                .flatten();
            // A declaration that writes a value the walk could not lay
            // out is a value missing, not a zero. `Real[nX] Y =
            // massToMoleFractions(X, MM)` bound on a body the run was
            // never handed came to nothing element by element, and the
            // entropy that read it answered as though every mole
            // fraction were zero, with no word said. Refused at once it
            // cost thirty-one water models whose `region_ph` declares
            // `constant Real[5] n = data.n` and never reads it, so the
            // elements are laid out as NaN instead, and a body whose
            // answer comes out NaN with such a local in its frame is
            // refused below, naming it. A body that never reads it is
            // untouched.
            let written = component.binding.as_ref().or(component.start.as_ref());
            let missing = if written.is_some() && !local_arrays_zero() {
                f64::NAN
            } else {
                0.0
            };
            if missing.is_nan() && !matches!(&laid, Some(items) if items.len() == length) {
                unlaid.push(component.name.clone());
            }
            for index in 1..=length {
                let worth = match &laid {
                    Some(items) if items.len() == length => {
                        number_of(&items[index - 1], &frame, programs, time, depth)?
                    }
                    _ => missing,
                };
                frame
                    .numbers
                    .insert(format!("{}[{index}]", component.name), worth);
            }
            continue;
        }
        if let Some(table) = declared_table(component) {
            let (shape, numbers) = table;
            let mut at = vec![1usize; shape.len()];
            for number in numbers {
                let subscripts: Vec<String> = at.iter().map(|index| index.to_string()).collect();
                frame.numbers.insert(
                    format!("{}[{}]", component.name, subscripts.join(",")),
                    number,
                );
                for axis in (0..shape.len()).rev() {
                    at[axis] += 1;
                    if at[axis] <= shape[axis] {
                        break;
                    }
                    at[axis] = 1;
                }
            }
            frame.lengths.insert(component.name.clone(), shape[0]);
            frame.shapes.insert(component.name.clone(), shape);
            continue;
        }
        let silent = if component.causality == Causality::Output && !unassigned_is_zero() {
            f64::from_bits(UNASSIGNED)
        } else {
            0.0
        };
        let start = component
            .binding
            .as_ref()
            .or(component.start.as_ref())
            .map(|expr| number_of(expr, &frame, programs, time, depth))
            .transpose()?
            .unwrap_or(silent);
        frame.numbers.insert(component.name.clone(), start);
    }
    // A body written outside Modelica and again here in Rust has no
    // statements to walk: its answer is the Rust one. The generators of
    // the standard library build their first state in a Modelica loop
    // that calls `random` ten times, and `random` is `external "C"`.
    // Walked as statements it answered with outputs nobody assigned.
    if let Some(answer) = outside_answer(class, args, shapes)? {
        return Ok(answer);
    }
    run(&class.algorithm, &mut frame, programs, time, depth)
        .map_err(|SimError(why, kind)| SimError(inside_body(why, name), claimed(kind, name)))?;
    // The answer, in the order the flat model asks for it: one number
    // for a plain output, and the elements in turn for an array. What
    // a body may answer with at all was settled before the run began.
    // Every output, in the order they were declared: a body may answer
    // with more than one number - `dofpt3` gives a density and an
    // error - and the call asks for the one it wants by its place
    // here. What was laid out before the run said the same order, so
    // the two agree without either being told.
    let outputs = class
        .components
        .iter()
        .filter(|component| component.causality == Causality::Output);
    // What a body answers with was laid out before it ran, so an
    // element it never filled stands at nothing - which is what the
    // language says an unassigned local is worth.
    let want = |named: &str| frame.numbers.get(named).copied().unwrap_or(0.0);
    let mut answer = Vec::new();
    for output in outputs {
        match frame.lengths.get(&output.name).copied() {
            None => {
                let value = want(&output.name);
                if value.to_bits() == UNASSIGNED {
                    return err(format!(
                        "the output `{}` of the walked body `{name}` was not assigned \
                         on the road the body took at t = {time}",
                        output.name
                    ));
                }
                answer.push(value);
            }
            Some(length) => {
                answer.extend((1..=length).map(|index| want(&format!("{}[{index}]", output.name))))
            }
        }
    }
    if !unlaid.is_empty() && answer.iter().any(|value| value.is_nan()) {
        return err(format!(
            "the walked body `{name}` answers with a value that is not a number, and it \
             declares {} with a value the walk cannot lay out element by element",
            unlaid
                .iter()
                .map(|local| format!("`{local}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(answer)
}

/// What a body written outside Modelica answers, where it is one of
/// those written again here. `None` for every other body, which is
/// walked as its statements say.
///
/// The Rust answer is the outputs in the order Modelica declared them,
/// laid end to end - which is also how a walk lays out its answer, so
/// the caller cannot tell the two apart. Only a shape the Rust side
/// takes whole is answered here: the solvers that want their counts
/// handed in front are laid out by the compiler where the call is
/// made, and a walk that met one would have to guess them.
fn outside_answer(
    class: &ClassDef,
    args: &[f64],
    shapes: &[Vec<usize>],
) -> Result<Option<Vec<f64>>, SimError> {
    let Some(call) = class.external_call.as_ref().filter(|call| {
        class.external
            && oxidelica_parser::outside::written_here(&call.called)
            && !switch_set("OXIDELICA_NO_WALKED_OUTSIDE")
    }) else {
        return Ok(None);
    };
    let handed: Vec<usize> = shapes
        .iter()
        .map(|shape| shape.iter().product::<usize>().max(1))
        .collect();
    let answer = match oxidelica_parser::outside::shape(&call.called, &handed) {
        Some((takes, answers)) if takes == args.len() && answers > 0 => {
            oxidelica_parser::outside::answer(&call.called, args)
                .filter(|answer| answer.len() == answers)
        }
        _ => None,
    };
    answer.map(Some).ok_or_else(|| {
        SimError::from(format!(
            "`{}` is written here, and a walk cannot hand it {} argument(s) of {} number(s) \
             in all",
            call.called,
            handed.len(),
            args.len()
        ))
    })
}

/// What a body carries while it is walked: numbers by name, and how
/// long each array among them is.
#[derive(Default)]
struct Frame {
    /// Every number the body holds, an array's elements under their
    /// own names.
    numbers: HashMap<String, f64>,
    /// How long each array is, by the name the body knows it by. For
    /// one of more than one dimension this is the first of them, which
    /// is what a body counting rows asks for.
    lengths: HashMap<String, usize>,
    /// Every dimension of an array of more than one, kept beside the
    /// first: `size(table, 2)` reads the second here.
    shapes: HashMap<String, Vec<usize>>,
}

/// A table a body declares with its numbers written out - `constant
/// Real hl_coef[:, :] = {{...}, ...}`, the spline coefficients of the
/// R134a saturation curves - as its shape and its numbers, rows one
/// after another. Only a table of two or more dimensions whose every
/// row is as long as the first and whose every element is a plain
/// number: anything else is left for the ordinary layout to refuse.
/// Laid out with nothing, such a table reached the evaluator whole at
/// the first element the body read.
fn declared_table(component: &Component) -> Option<(Vec<usize>, Vec<f64>)> {
    if component.dimensions.len() < 2 || switch_set("OXIDELICA_NO_WALKED_TABLES") {
        return None;
    }
    fn gather(expr: &Expr, shape: &mut Vec<usize>, depth: usize, out: &mut Vec<f64>) -> Option<()> {
        match expr {
            Expr::Number(number) if depth == shape.len() => {
                out.push(*number);
                Some(())
            }
            Expr::Neg(inner) if depth == shape.len() => match inner.as_ref() {
                Expr::Number(number) => {
                    out.push(-number);
                    Some(())
                }
                _ => None,
            },
            Expr::Array(items) if depth < shape.len() => {
                if shape[depth] == 0 {
                    shape[depth] = items.len();
                }
                if shape[depth] != items.len() || items.is_empty() {
                    return None;
                }
                items
                    .iter()
                    .try_for_each(|item| gather(item, shape, depth + 1, out))
            }
            _ => None,
        }
    }
    let written = component.binding.as_ref().or(component.start.as_ref())?;
    // A dimension written as a number holds the table to it; one written
    // `:` takes the length of what is written out.
    let mut shape: Vec<usize> = component
        .dimensions
        .iter()
        .map(|dimension| match dimension {
            Expr::Number(length) if *length >= 1.0 => Some(*length as usize),
            Expr::ColonSubscript => Some(0),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let mut numbers = Vec::new();
    gather(written, &mut shape, 0, &mut numbers)?;
    Some((shape, numbers))
}

/// How long a declaration is, where it says so in numbers the body
/// already holds. A length written as anything else - `size(v, 1)` of
/// something handed in - is read from what was handed in instead.
fn declared_length(component: &Component, frame: &Frame) -> Option<usize> {
    let [dimension] = component.dimensions.as_slice() else {
        return None;
    };
    match dimension {
        Expr::Number(length) => Some(*length as usize),
        Expr::Call(name, args) if name == "size" && args.len() == 2 => {
            let Expr::Ref(of) = &args[0] else { return None };
            frame.lengths.get(of).copied()
        }
        // `Real u[nPoints]`, where `nPoints` is an input the caller
        // handed in or left at its default: both are laid in the frame
        // before the locals, in the order they were declared, so the
        // length is a number the body already holds. Only a whole
        // number is a length; anything else is left to be refused.
        Expr::Ref(of) if walked_linspace_open() => frame
            .numbers
            .get(of)
            .copied()
            .filter(|length| length.fract() == 0.0 && *length >= 0.0)
            .map(|length| length as usize),
        // `Real a[:] = {-7.86, 1.84, ...}`: a length of `:` is the length
        // of what the declaration writes out. Without it the whole
        // literal was taken for one number, and the water's saturation
        // pressure - Wagner's six coefficients, declared this way -
        // refused every moist-air model that walked it.
        Expr::ColonSubscript if !switch_set("OXIDELICA_NO_COLON_LENGTH") => {
            match component.binding.as_ref().or(component.start.as_ref()) {
                Some(Expr::Array(items)) => Some(items.len()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// What an expression is worth inside a frame, calls to other walked
/// bodies included.
fn number_of(
    expr: &Expr,
    frame: &Frame,
    programs: &HashMap<String, ClassDef>,
    time: f64,
    depth: usize,
) -> Result<f64, SimError> {
    let scalar = to_scalar(expr, frame, programs, time, depth)?;
    code::eval(
        &scalar,
        &EvalCtx {
            vars: &frame.numbers,
            time,
            programs: Some(programs),
            depth,
        },
    )
    .map_err(|SimError(why, kind)| SimError(standing_in(why, expr), kind))
}

/// The wording the evaluator refuses a whole array with. Only that
/// refusal is given an address below: every other one already names
/// its own culprit, and a sentence that grew a clause at every storey
/// of a walk would bury it.
const ARRAY_REACHED: &str = "an array reached the evaluator";

/// The expression of the body an array arrived in, said once.
///
/// The evaluator sees only the array, and an array of six numbers says
/// nothing about where it was written: the refusal read `{-7.86, ...}`
/// for five models, and which of their bodies held it was a guess.
/// The innermost expression that asked for a number is the one that
/// wrote the array, so the first to see the refusal names itself and
/// the ones above it leave the sentence alone.
fn standing_in(why: String, expr: &Expr) -> String {
    if !why.contains(ARRAY_REACHED) || why.contains(", standing in `") {
        return why;
    }
    let written: String = expr.describe().chars().take(200).collect();
    format!("{why}, standing in `{written}`")
}

/// And the body that expression was written in, again said once: the
/// innermost body is the one whose statement it is.
fn inside_body(why: String, name: &str) -> String {
    if !why.contains(ARRAY_REACHED) || why.contains(" of the walked body `") {
        return why;
    }
    format!("{why} of the walked body `{name}`")
}

/// What a refusal a body raised itself is, decided by the innermost
/// body, whose statement it was. The kind is read off the body's
/// resolved name, never off the wording: the library's one-dimensional
/// root finder is `Modelica.Math.Nonlinear.solveOneNonlinearEquation`,
/// and a copy of it specialized for the function it was handed carries
/// that name with `$` and the function after it. A body further out,
/// which only called the one that refused, leaves the kind alone.
fn claimed(kind: Refusal, name: &str) -> Refusal {
    if kind != Refusal::Raised {
        return kind;
    }
    const ROOT_FINDER: &str = "Modelica.Math.Nonlinear.solveOneNonlinearEquation";
    let body = name.split_once('$').map_or(name, |(body, _)| body);
    if body == ROOT_FINDER {
        Refusal::Unbracketed
    } else {
        // An `assert` or `Streams.error` of any other body is the body
        // refusing its arguments, which is a statement about where it
        // was asked rather than about the model.
        Refusal::Outside
    }
}

/// What an expression written over arrays comes to as one number.
///
/// The answer of a walked body is one number, so an array can only
/// appear on its way to becoming one: subscripted, measured, folded by
/// `sum` and its like, or multiplied by another array. Each of those is
/// written out here in terms of the elements, and everything else is
/// left as it stands for the ordinary evaluation to do.
fn to_scalar(
    expr: &Expr,
    frame: &Frame,
    programs: &HashMap<String, ClassDef>,
    time: f64,
    depth: usize,
) -> Result<Expr, SimError> {
    let recur = |e: &Expr| to_scalar(e, frame, programs, time, depth);
    Ok(match expr {
        // `v[i]` where the body decides `i`: the element's own name.
        Expr::Index(base, subscripts) => {
            let Expr::Ref(name) = base.as_ref() else {
                return err(format!(
                    "only a name is subscripted in a walked body, not {base:?}"
                ));
            };
            let mut indices = Vec::new();
            for subscript in subscripts {
                indices.push(index_of(subscript, frame, programs, time, depth)?.to_string());
            }
            Expr::Ref(format!("{name}[{}]", indices.join(",")))
        }
        // `size(v, 1)` of what was handed in.
        Expr::Call(name, args) if name == "size" && args.len() == 2 => {
            let Expr::Ref(of) = &args[0] else {
                return err("`size` in a walked body asks about a name".to_string());
            };
            // Which axis was asked about: a table is asked for its
            // rows and for its columns, and only the first is what a
            // single length says.
            let axis = number_of(&args[1], frame, programs, time, depth)? as usize;
            let length = match frame.shapes.get(of) {
                Some(dimensions) => dimensions.get(axis.saturating_sub(1)).copied(),
                None => (axis == 1)
                    .then(|| frame.lengths.get(of).copied())
                    .flatten(),
            }
            .ok_or_else(|| {
                SimError::from(format!(
                    "`{of}` has no dimension {axis} this walk was given"
                ))
            })?;
            Expr::Number(length as f64)
        }
        // `scalar(size(breaks))`: the length of a list, the one
        // element of the one-element array `size` of a vector is.
        // The R134a interval search counts its grid this way, and the
        // flattener's fold of `scalar` never saw a body left to walk.
        Expr::Call(name, args) if name == "scalar" && args.len() == 1 && walked_slices_open() => {
            let length = match &args[0] {
                Expr::Call(size, inner) if size == "size" && inner.len() == 1 => match &inner[0] {
                    Expr::Ref(of) if frame.shapes.get(of).is_none_or(|shape| shape.len() == 1) => {
                        frame.lengths.get(of).copied()
                    }
                    _ => None,
                },
                _ => None,
            };
            match length {
                Some(length) => Expr::Number(length as f64),
                None => Expr::Call(name.clone(), vec![recur(&args[0])?]),
            }
        }
        // A fold over an array is the fold over its elements.
        Expr::Call(name, args)
            if matches!(name.as_str(), "sum" | "product" | "min" | "max") && args.len() == 1 =>
        {
            match elements_of(&args[0], frame, programs, time, depth)? {
                None => Expr::Call(name.clone(), vec![recur(&args[0])?]),
                Some(items) => fold(name, items)?,
            }
        }
        // Two arrays multiplied are their scalar product.
        Expr::Bin(BinOp::Mul, a, b) => {
            match (
                elements_of(a, frame, programs, time, depth)?,
                elements_of(b, frame, programs, time, depth)?,
            ) {
                (Some(left), Some(right)) => {
                    if left.len() != right.len() {
                        return err(format!(
                            "a scalar product needs equal lengths, got {} and {}",
                            left.len(),
                            right.len()
                        ));
                    }
                    let terms = left
                        .into_iter()
                        .zip(right)
                        .map(|(a, b)| Expr::Bin(BinOp::Mul, Box::new(a), Box::new(b)))
                        .collect();
                    fold("sum", terms)?
                }
                _ => Expr::Bin(BinOp::Mul, Box::new(recur(a)?), Box::new(recur(b)?)),
            }
        }
        Expr::Bin(op, a, b) => Expr::Bin(*op, Box::new(recur(a)?), Box::new(recur(b)?)),
        Expr::Rel(op, a, b) => Expr::Rel(*op, Box::new(recur(a)?), Box::new(recur(b)?)),
        Expr::And(a, b) => Expr::And(Box::new(recur(a)?), Box::new(recur(b)?)),
        Expr::Or(a, b) => Expr::Or(Box::new(recur(a)?), Box::new(recur(b)?)),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner)?)),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner)?)),
        Expr::If(condition, then, otherwise) => Expr::If(
            Box::new(recur(condition)?),
            Box::new(recur(then)?),
            Box::new(recur(otherwise)?),
        ),
        // A call inside a walked body is handed what the frame holds,
        // not what the body wrote. A record the frame carries as
        // `f.data[1]`, `f.data[2]` is named `f.data` where it is
        // passed on, and a bare name is something the evaluation has
        // no value for; written out as its elements it travels whole.
        Expr::Call(name, args) => Expr::Call(
            name.clone(),
            args.iter()
                .map(|arg| match arg {
                    Expr::Ref(held) if frame.lengths.contains_key(held) => {
                        let length = frame.lengths[held];
                        Ok(Expr::Array(
                            (1..=length)
                                .map(|index| Expr::Ref(format!("{held}[{index}]")))
                                .collect(),
                        ))
                    }
                    // A list written out of the body's own elements -
                    // `{b[5], b[4], b[3], b[2], b[1]}`, how the reference
                    // air hands a polynomial its coefficients - has each
                    // element read by the frame, a row at a time where
                    // it is a table. Left as written, `b[5]` reached the
                    // evaluator as a subscript nobody had resolved.
                    Expr::Array(_) if walked_slices_open() => {
                        fn each(
                            item: &Expr,
                            recur: &dyn Fn(&Expr) -> Result<Expr, SimError>,
                        ) -> Result<Expr, SimError> {
                            match item {
                                Expr::Array(items) => Ok(Expr::Array(
                                    items
                                        .iter()
                                        .map(|inner| each(inner, recur))
                                        .collect::<Result<Vec<_>, SimError>>()?,
                                )),
                                one => recur(one),
                            }
                        }
                        each(arg, &recur)
                    }
                    // A slice of a table goes over as its elements too.
                    Expr::Index(..) if walked_slices_open() => {
                        match elements_of(arg, frame, programs, time, depth)? {
                            Some(items) => Ok(Expr::Array(items)),
                            None => recur(arg),
                        }
                    }
                    _ => recur(arg),
                })
                .collect::<Result<Vec<_>, SimError>>()?,
        ),
        // What is already a scalar, or what a walked body cannot hold
        // by the time it is walked: either way the expression is
        // itself. They are listed rather than swept up, so that a
        // variant added to `Expr` has to be decided about here rather
        // than passing through a walk that cannot carry it.
        Expr::Number(_)
        | Expr::Ref(_)
        | Expr::Bool(_)
        | Expr::Str(_)
        | Expr::Time
        | Expr::WithDerivative(..)
        | Expr::Member(..)
        | Expr::Array(_)
        | Expr::MatrixRows(_)
        | Expr::Elementwise(..)
        | Expr::Range(..)
        | Expr::Comprehension(..)
        | Expr::ColonSubscript
        | Expr::EndSubscript
        | Expr::NamedArg(..)
        | Expr::Tuple(_) => expr.clone(),
    })
}

/// The elements an expression stands for, where it stands for several.
fn elements_of(
    expr: &Expr,
    frame: &Frame,
    programs: &HashMap<String, ClassDef>,
    time: f64,
    depth: usize,
) -> Result<Option<Vec<Expr>>, SimError> {
    Ok(match expr {
        Expr::Ref(name) => frame.lengths.get(name).map(|length| {
            (1..=*length)
                .map(|index| Expr::Ref(format!("{name}[{index}]")))
                .collect()
        }),
        Expr::Array(items) => Some(
            items
                .iter()
                .map(|item| to_scalar(item, frame, programs, time, depth))
                .collect::<Result<Vec<_>, SimError>>()?,
        ),
        // `array(0.5132047, 0.3205656, ...)` is the constructor written
        // out as a call, and for arguments that are single numbers it
        // is the same list as braces: the forty-two coefficients of the
        // water's viscosity in `visc_dTp` are written this way. An
        // argument that is itself an array would build a table, a new
        // axis in front, which is not a list and is left alone.
        Expr::Call(name, args) if name == "array" && !args.is_empty() => {
            let mut items = Vec::new();
            for arg in args {
                if elements_of(arg, frame, programs, time, depth)?.is_some() {
                    return Ok(None);
                }
                items.push(to_scalar(arg, frame, programs, time, depth)?);
            }
            Some(items)
        }
        // `linspace(x1, x2, n)`: n points from x1 to x2, evenly spaced,
        // the ends included. The random-number tests of the standard
        // library bind every local grid this way. Fewer than two
        // points is what the language forbids, and it is refused as the
        // flattener refuses it.
        Expr::Call(name, args)
            if name == "linspace" && args.len() == 3 && walked_linspace_open() =>
        {
            let from = number_of(&args[0], frame, programs, time, depth)?;
            let to = number_of(&args[1], frame, programs, time, depth)?;
            let count = number_of(&args[2], frame, programs, time, depth)?;
            if count.fract() != 0.0 || count < 2.0 {
                return err(format!(
                    "linspace needs a whole number of at least two points, got {count}"
                ));
            }
            let count = count as usize;
            Some(
                (0..count)
                    .map(|at| Expr::Number(from + (to - from) * at as f64 / (count - 1) as f64))
                    .collect(),
            )
        }
        // `ones(n)`, `zeros(n)` and `fill(x, n)`: a list of one number
        // n times over.
        Expr::Call(name, args)
            if walked_linspace_open()
                && matches!(
                    (name.as_str(), args.len()),
                    ("ones", 1) | ("zeros", 1) | ("fill", 2)
                ) =>
        {
            let (worth, count) = match name.as_str() {
                "ones" => (Expr::Number(1.0), &args[0]),
                "zeros" => (Expr::Number(0.0), &args[0]),
                _ => (to_scalar(&args[0], frame, programs, time, depth)?, &args[1]),
            };
            let count = number_of(count, frame, programs, time, depth)?;
            if count.fract() != 0.0 || count < 0.0 {
                return err(format!("`{name}` asked for {count} elements"));
            }
            Some(vec![worth; count as usize])
        }
        // `y1 - y3` of two arrays the body holds: one difference per
        // element, and the same for a sum. The language adds and
        // subtracts arrays only of one length, so anything else is
        // left to be refused where it stands.
        Expr::Bin(op @ (BinOp::Add | BinOp::Sub), a, b)
            if walked_linspace_open() && holds_a_list(a, frame) && holds_a_list(b, frame) =>
        {
            match (
                elements_of(a, frame, programs, time, depth)?,
                elements_of(b, frame, programs, time, depth)?,
            ) {
                (Some(left), Some(right)) if left.len() == right.len() => Some(
                    left.into_iter()
                        .zip(right)
                        .map(|(a, b)| Expr::Bin(*op, Box::new(a), Box::new(b)))
                        .collect(),
                ),
                _ => None,
            }
        }
        // `2*u` and `u/2`: a number and an array, the number going
        // with every element. Two arrays multiplied are a scalar
        // product and not a list, and that is `to_scalar`'s to write.
        Expr::Bin(op @ (BinOp::Mul | BinOp::Div), a, b)
            if walked_linspace_open() && (holds_a_list(a, frame) || holds_a_list(b, frame)) =>
        {
            let list = |side: &Expr| -> Result<Option<Vec<Expr>>, SimError> {
                if holds_a_list(side, frame) {
                    elements_of(side, frame, programs, time, depth)
                } else {
                    Ok(None)
                }
            };
            match (list(a)?, list(b)?) {
                (None, Some(right)) if *op == BinOp::Mul => {
                    let one = to_scalar(a, frame, programs, time, depth)?;
                    Some(
                        right
                            .into_iter()
                            .map(|item| Expr::Bin(*op, Box::new(one.clone()), Box::new(item)))
                            .collect(),
                    )
                }
                (Some(left), None) => {
                    let one = to_scalar(b, frame, programs, time, depth)?;
                    Some(
                        left.into_iter()
                            .map(|item| Expr::Bin(*op, Box::new(item), Box::new(one.clone())))
                            .collect(),
                    )
                }
                _ => None,
            }
        }
        // `-u` and `abs(y1 - y3)` of an array: the same, element by
        // element.
        Expr::Neg(inner) if walked_linspace_open() && holds_a_list(inner, frame) => {
            elements_of(inner, frame, programs, time, depth)?.map(|items| {
                items
                    .into_iter()
                    .map(|item| Expr::Neg(Box::new(item)))
                    .collect()
            })
        }
        // So is a built-in of one number, `sin(u3)` over the array: the
        // language applies it element by element.
        Expr::Call(name, args)
            if args.len() == 1
                && walked_linspace_open()
                && holds_a_list(&args[0], frame)
                && !programs.contains_key(name)
                && matches!(
                    name.as_str(),
                    "abs"
                        | "sign"
                        | "sqrt"
                        | "sin"
                        | "cos"
                        | "tan"
                        | "asin"
                        | "acos"
                        | "atan"
                        | "sinh"
                        | "cosh"
                        | "tanh"
                        | "exp"
                        | "log"
                        | "log10"
                ) =>
        {
            elements_of(&args[0], frame, programs, time, depth)?.map(|items| {
                items
                    .into_iter()
                    .map(|item| Expr::Call(name.clone(), vec![item]))
                    .collect()
            })
        }
        // A whole record answered by a call - `f := Basic.Helmholtz(d,
        // T)`, where `f` is a `HelmholtzDerivs` the walk holds as an
        // array of its fields, or `nDerivs := Helmholtz_pT(f)`, which
        // passes that record on and answers with another. The body is
        // walked and every number it answers with becomes an element, so
        // the assignment lands on `f[1]`, `f[2]`, and the rest, the same
        // as a written-out array. An argument that is itself a record or
        // an array is sent as its elements, under the shape the callee
        // reads it by.
        Expr::Call(name, args) if programs.contains_key(name) => {
            // `erf(u)` of an array `u`, where `erf` takes one number:
            // the language calls it once per element and the answer is
            // the list of those. Sent whole, the callee's scalar `u`
            // was laid out as `u[1]`, `u[2]`, ... and its first read of
            // `u` named nothing. Only where every argument at a scalar
            // input that holds a list holds one of the same length.
            if walked_linspace_open() && args.iter().any(|arg| holds_a_list(arg, frame)) {
                if let Some(items) = vectorised(name, args, frame, programs, time, depth)? {
                    return Ok(Some(items));
                }
            }
            let mut given = Vec::new();
            let mut shapes = Vec::new();
            for arg in args {
                match arg {
                    Expr::Ref(inner) if frame.lengths.contains_key(inner) => {
                        let length = frame.lengths[inner];
                        for index in 1..=length {
                            given.push(number_of(
                                &Expr::Ref(format!("{inner}[{index}]")),
                                frame,
                                programs,
                                time,
                                depth,
                            )?);
                        }
                        shapes.push(vec![length]);
                    }
                    // An argument that is itself a call answering with
                    // a record goes over as that record's fields, the
                    // same as a named one. The water tables are written
                    // this way throughout - `hvl_p(p, boilingcurve_p(p))`
                    // hands the boiling curve's whole property record
                    // straight to the reader - and taken as one number
                    // the callee's `bpro[1]` names nothing at all.
                    _ => match elements_of(arg, frame, programs, time, depth)? {
                        Some(items) => {
                            for item in &items {
                                given.push(number_of(item, frame, programs, time, depth)?);
                            }
                            shapes.push(vec![items.len()]);
                        }
                        None => {
                            given.push(number_of(arg, frame, programs, time, depth)?);
                            shapes.push(Vec::new());
                        }
                    },
                }
            }
            let answer = walk(programs, name, &given, &shapes, time, depth + 1)?;
            // A body that answers with one number is a scalar call, and
            // it stays one here: turning it into a one-element array
            // would upset a scalar product or an elementwise operation
            // that asked this of it. Only a genuine list - a record's
            // fields, an array output - is spread.
            (answer.len() > 1).then(|| answer.into_iter().map(Expr::Number).collect())
        }
        // `v .* i` and its like: one operation per element. One side
        // may be a single number, which then goes with every element.
        Expr::Elementwise(op, a, b) => {
            let (left, right) = (
                elements_of(a, frame, programs, time, depth)?,
                elements_of(b, frame, programs, time, depth)?,
            );
            let spread = |one: &Expr, many: Vec<Expr>, first: bool| {
                let one = to_scalar(one, frame, programs, time, depth);
                one.map(|one| {
                    many.into_iter()
                        .map(|item| match first {
                            true => Expr::Bin(*op, Box::new(one.clone()), Box::new(item)),
                            false => Expr::Bin(*op, Box::new(item), Box::new(one.clone())),
                        })
                        .collect()
                })
            };
            match (left, right) {
                (Some(left), Some(right)) if left.len() == right.len() => Some(
                    left.into_iter()
                        .zip(right)
                        .map(|(a, b)| Expr::Bin(*op, Box::new(a), Box::new(b)))
                        .collect(),
                ),
                (Some(left), None) => Some(spread(b, left, false)?),
                (None, Some(right)) => Some(spread(a, right, true)?),
                _ => None,
            }
        }
        // `hl_coef[int, 1:4]`: one row of a table the body holds,
        // picked by a subscript the body decides and cut by a range -
        // how every R134a spline hands its four coefficients to the
        // evaluator. One axis is a range or `:`, the others single
        // numbers; the elements are the names the frame holds them by.
        Expr::Index(base, subscripts) if walked_slices_open() => {
            let Expr::Ref(name) = base.as_ref() else {
                return Ok(None);
            };
            let Some(shape) = frame
                .shapes
                .get(name)
                .cloned()
                .or_else(|| frame.lengths.get(name).map(|length| vec![*length]))
            else {
                return Ok(None);
            };
            if shape.len() != subscripts.len() {
                return Ok(None);
            }
            let cut = |axis: usize, subscript: &Expr| -> Result<Option<Vec<i64>>, SimError> {
                Ok(match subscript {
                    Expr::ColonSubscript => Some((1..=shape[axis] as i64).collect()),
                    Expr::Range(from, None, to) => {
                        let from = index_of(from, frame, programs, time, depth)?;
                        let to = index_of(to, frame, programs, time, depth)?;
                        Some((from..=to).collect())
                    }
                    _ => None,
                })
            };
            let mut open: Option<(usize, Vec<i64>)> = None;
            let mut fixed: Vec<Option<i64>> = Vec::new();
            for (axis, subscript) in subscripts.iter().enumerate() {
                match cut(axis, subscript)? {
                    Some(range) if open.is_none() => {
                        open = Some((axis, range));
                        fixed.push(None);
                    }
                    Some(_) => return Ok(None),
                    None => fixed.push(Some(index_of(subscript, frame, programs, time, depth)?)),
                }
            }
            let Some((axis, range)) = open else {
                return Ok(None);
            };
            if range.iter().any(|at| *at < 1 || *at as usize > shape[axis]) {
                return err(format!(
                    "`{name}` is cut outside its {} element(s) on axis {}",
                    shape[axis],
                    axis + 1
                ));
            }
            Some(
                range
                    .into_iter()
                    .map(|at| {
                        let indices: Vec<String> = fixed
                            .iter()
                            .map(|index| index.unwrap_or(at).to_string())
                            .collect();
                        Expr::Ref(format!("{name}[{}]", indices.join(",")))
                    })
                    .collect(),
            )
        }
        _ => None,
    })
}

/// Whether an expression could stand for a list, read off its writing
/// alone: a name the frame holds as an array, a slice, a list written
/// out, or a constructor of one. It asks nothing of a walked body.
///
/// The arms of `elements_of` that write an operation out element by
/// element ask this first. Asking `elements_of` of both sides instead
/// walks every call beneath to learn whether it answers with a list,
/// and `to_scalar` then walks the same calls again for the number: one
/// repeat per storey, which the water tables, a call inside a call
/// inside a call, turned into a corpus pass that did not end.
fn holds_a_list(expr: &Expr, frame: &Frame) -> bool {
    match expr {
        Expr::Ref(name) => frame.lengths.contains_key(name),
        Expr::Array(_) => true,
        Expr::Index(_, subscripts) => subscripts
            .iter()
            .any(|subscript| matches!(subscript, Expr::ColonSubscript | Expr::Range(..))),
        Expr::Call(name, args) => {
            matches!(
                name.as_str(),
                "linspace" | "ones" | "zeros" | "fill" | "array"
            ) || args.iter().any(|arg| holds_a_list(arg, frame))
        }
        Expr::Bin(_, a, b) | Expr::Elementwise(_, a, b) => {
            holds_a_list(a, frame) || holds_a_list(b, frame)
        }
        Expr::Neg(inner) => holds_a_list(inner, frame),
        _ => false,
    }
}

/// A call of a body that takes plain numbers, handed a list at one or
/// more of them: the answer is the body called once per element, the
/// other arguments going with every call. `None` wherever that is not
/// the shape - a body taking an array or a record, one answering with
/// more than one number, a named argument, lists of two lengths, or no
/// list at all - and the call is then read as it always was.
fn vectorised(
    name: &str,
    args: &[Expr],
    frame: &Frame,
    programs: &HashMap<String, ClassDef>,
    time: f64,
    depth: usize,
) -> Result<Option<Vec<Expr>>, SimError> {
    let Some(class) = programs.get(name) else {
        return Ok(None);
    };
    let plain = |component: &Component| {
        component.dimensions.is_empty()
            && matches!(component.type_name.as_str(), "Real" | "Integer" | "Boolean")
    };
    let inputs: Vec<&Component> = class
        .components
        .iter()
        .filter(|component| component.causality == Causality::Input)
        .collect();
    let outputs: Vec<&Component> = class
        .components
        .iter()
        .filter(|component| component.causality == Causality::Output)
        .collect();
    if args.len() > inputs.len()
        || !inputs.iter().all(|input| plain(input))
        || !matches!(outputs.as_slice(), [one] if plain(one))
        || args.iter().any(|arg| matches!(arg, Expr::NamedArg(..)))
    {
        return Ok(None);
    }
    let mut spread: Vec<Option<Vec<Expr>>> = Vec::new();
    for arg in args {
        spread.push(elements_of(arg, frame, programs, time, depth)?);
    }
    let lengths: Vec<usize> = spread.iter().flatten().map(Vec::len).collect();
    let Some(&length) = lengths.first() else {
        return Ok(None);
    };
    if lengths.iter().any(|other| *other != length) {
        return Ok(None);
    }
    let mut fixed = Vec::new();
    for (arg, items) in args.iter().zip(&spread) {
        fixed.push(match items {
            Some(_) => None,
            None => Some(number_of(arg, frame, programs, time, depth)?),
        });
    }
    let shapes = vec![Vec::new(); args.len()];
    let mut answer = Vec::with_capacity(length);
    for at in 0..length {
        let mut given = Vec::with_capacity(args.len());
        for (items, one) in spread.iter().zip(&fixed) {
            given.push(match (items, one) {
                (Some(items), _) => number_of(&items[at], frame, programs, time, depth)?,
                (None, Some(one)) => *one,
                (None, None) => unreachable!("an argument is either spread or fixed"),
            });
        }
        let value = walk(programs, name, &given, &shapes, time, depth + 1)?;
        let [value] = value.as_slice() else {
            return err(format!(
                "`{name}` answered with {} numbers where one was declared",
                value.len()
            ));
        };
        answer.push(Expr::Number(*value));
    }
    Ok(Some(answer))
}

/// Whether a walk reads a slice of a table as its elements.
/// `OXIDELICA_NO_WALKED_SLICES` leaves the slice whole, so that one
/// binary gives both numbers.
fn walked_slices_open() -> bool {
    !switch_set("OXIDELICA_NO_WALKED_SLICES")
}

/// Whether a walk writes a run of an array element by element.
/// `OXIDELICA_NO_IMPURE_DRAWS` turns it back with the rest of the
/// impure generator's road, so that one binary gives both numbers.
fn walked_slice_writes_open() -> bool {
    !switch_set("OXIDELICA_NO_IMPURE_DRAWS")
}

/// A fold written out: `sum` of nothing is nothing, of one is itself.
fn fold(name: &str, items: Vec<Expr>) -> Result<Expr, SimError> {
    let joined = match name {
        "sum" => items
            .into_iter()
            .reduce(|a, b| Expr::Bin(BinOp::Add, Box::new(a), Box::new(b)))
            .unwrap_or(Expr::Number(0.0)),
        "product" => items
            .into_iter()
            .reduce(|a, b| Expr::Bin(BinOp::Mul, Box::new(a), Box::new(b)))
            .unwrap_or(Expr::Number(1.0)),
        _ => items
            .into_iter()
            .reduce(|a, b| Expr::Call(name.to_string(), vec![a, b]))
            .ok_or_else(|| SimError::from(format!("`{name}` of an array with nothing in it")))?,
    };
    Ok(joined)
}

/// A subscript as the whole number it has to be.
fn index_of(
    expr: &Expr,
    frame: &Frame,
    programs: &HashMap<String, ClassDef>,
    time: f64,
    depth: usize,
) -> Result<i64, SimError> {
    let value = number_of(expr, frame, programs, time, depth)?;
    if value.fract() != 0.0 || value < 1.0 {
        return err(format!(
            "a subscript must be a whole number from one, got {value}"
        ));
    }
    Ok(value as i64)
}

/// The sentence a refusal was written with, with the numbers in it
/// filled in.
///
/// A model that refuses says why in prose, and the prose is built by
/// joining literal pieces to `String(x)` of whatever the run knows.
/// Off the run, `message_text` can only put `?` where such a piece
/// stands, because nothing there has a value yet. Inside a walk the
/// frame holds the values, so the piece can be worked out and the
/// reader gets the number the library meant to show rather than a
/// question mark. A piece that still cannot be worked out - a name the
/// frame does not hold, a call that itself refuses - falls back to the
/// `?` rather than losing the whole sentence: the literal halves are
/// what say what went wrong.
fn prose(
    expr: &Expr,
    frame: &mut Frame,
    programs: &HashMap<String, ClassDef>,
    time: f64,
    depth: usize,
) -> String {
    match expr {
        Expr::Str(text) => text.clone(),
        Expr::Bin(oxidelica_parser::BinOp::Add, a, b) => {
            prose(a, frame, programs, time, depth) + &prose(b, frame, programs, time, depth)
        }
        // `String(x)` and `String(x, format)` alike: the first
        // argument is the value, and the rest say how to lay it out,
        // which a diagnostic can do without.
        Expr::Call(name, args) if name == "String" && !args.is_empty() => {
            match number_of(&args[0], frame, programs, time, depth) {
                Ok(value) => format!("{value}"),
                Err(_) => "?".to_string(),
            }
        }
        _ => oxidelica_parser::message_text(expr),
    }
}

/// Walk a run of statements.
fn run(
    body: &[Statement],
    frame: &mut Frame,
    programs: &HashMap<String, ClassDef>,
    time: f64,
    depth: usize,
) -> Result<Flow, SimError> {
    for statement in body {
        match statement {
            Statement::Assign(target, subscripts, value) => {
                // `state[i:i+1] := aux` fills a run of one array from
                // another, element by element: the generators' seeding
                // steps a state two numbers at a time this way.
                if let [Expr::Range(from, None, to)] = subscripts.as_slice() {
                    if walked_slice_writes_open() {
                        let from = index_of(from, frame, programs, time, depth)?;
                        let to = index_of(to, frame, programs, time, depth)?;
                        let items =
                            elements_of(value, frame, programs, time, depth)?.ok_or_else(|| {
                                SimError::from(format!(
                                    "`{target}[{from}:{to}]` is given something that is not \
                                     a list: {value:?}"
                                ))
                            })?;
                        if items.len() as i64 != (to - from + 1).max(0) {
                            return err(format!(
                                "`{target}[{from}:{to}]` is {} long and was given {}",
                                (to - from + 1).max(0),
                                items.len()
                            ));
                        }
                        let mut worths = Vec::new();
                        for item in &items {
                            worths.push(number_of(item, frame, programs, time, depth)?);
                        }
                        for (at, worth) in (from..=to).zip(worths) {
                            frame.numbers.insert(format!("{target}[{at}]"), worth);
                        }
                        continue;
                    }
                }
                // `q[i] := ...` lands on the element's own name, which
                // is how an array is held here.
                let mut named = target.clone();
                if !subscripts.is_empty() {
                    let mut indices = Vec::new();
                    for subscript in subscripts {
                        indices
                            .push(index_of(subscript, frame, programs, time, depth)?.to_string());
                    }
                    named = format!("{target}[{}]", indices.join(","));
                }
                // A whole array assigned at once - `w := 2 .* v` -
                // lands on the elements, since that is how one is held.
                if subscripts.is_empty() {
                    if let (Some(length), Some(items)) = (
                        frame.lengths.get(target).copied(),
                        elements_of(value, frame, programs, time, depth)?,
                    ) {
                        if items.len() != length {
                            return err(format!(
                                "`{target}` is {length} long and was given {}",
                                items.len()
                            ));
                        }
                        for (index, item) in items.iter().enumerate() {
                            let worth = number_of(item, frame, programs, time, depth)?;
                            frame
                                .numbers
                                .insert(format!("{target}[{}]", index + 1), worth);
                        }
                        continue;
                    }
                }
                let worth = number_of(value, frame, programs, time, depth)?;
                frame.numbers.insert(named, worth);
            }
            Statement::Assert(condition, message) => {
                if number_of(condition, frame, programs, time, depth)? == 0.0 {
                    return err_of(
                        Refusal::Raised,
                        prose(message, frame, programs, time, depth),
                    );
                }
            }
            // `Streams.error(text)` is how the standard library
            // refuses in prose: it is `assert(false, text)` written as
            // a call, and its one argument is a sentence rather than a
            // number. Read as a number - which is what a call standing
            // on its own is read as - the first piece of the sentence
            // is a String and the walk refuses about the String, so the
            // reason the library took the trouble to write is thrown
            // away and replaced by a complaint about its spelling. Take
            // the text the way an `assert` takes it instead.
            Statement::Call(name, args)
                if name == "Modelica.Utilities.Streams.error" && args.len() == 1 =>
            {
                return err_of(
                    Refusal::Raised,
                    prose(&args[0], frame, programs, time, depth),
                );
            }
            // `Streams.print(text)` writes a line on a terminal, and
            // there is none here and no value to miss: inlining and the
            // carrying of bodies already take it for nothing, and the
            // walk does the same. Read as a number, its String argument
            // refused the whole body at its first line.
            Statement::Call(name, _)
                if name == "Modelica.Utilities.Streams.print" && walked_linspace_open() => {}
            // A guard - a body with no outputs, carried for the checks
            // it makes - is walked for those and answers nothing, so it
            // is not read as a number: the evaluator's call asks the
            // walk for its first number and a guard has none.
            Statement::Call(name, args)
                if programs.get(name).is_some_and(|class| {
                    !class
                        .components
                        .iter()
                        .any(|held| held.causality == Causality::Output)
                }) && !switch_set("OXIDELICA_NO_CARRIED_GUARDS") =>
            {
                let given = args
                    .iter()
                    .map(|arg| number_of(arg, frame, programs, time, depth))
                    .collect::<Result<Vec<f64>, SimError>>()?;
                let shapes = vec![Vec::new(); given.len()];
                walk(programs, name, &given, &shapes, time, depth + 1)?;
            }
            // A call on its own: nothing takes its outputs, so it is
            // walked for the checks its body makes and for nothing
            // else. Reading its value is what runs those checks.
            Statement::Call(name, args) => {
                number_of(
                    &Expr::Call(name.clone(), args.clone()),
                    frame,
                    programs,
                    time,
                    depth,
                )?;
            }
            Statement::If(branches) => {
                for branch in branches {
                    let taken = match &branch.condition {
                        Some(condition) => {
                            number_of(condition, frame, programs, time, depth)? != 0.0
                        }
                        None => true,
                    };
                    if taken {
                        match run(&branch.body, frame, programs, time, depth)? {
                            Flow::Onwards => {}
                            other => return Ok(other),
                        }
                        break;
                    }
                }
            }
            // This is the whole point of walking rather than unrolling:
            // the condition is asked again each round, of the values the
            // body has by then.
            Statement::While(condition, inner) => {
                let mut rounds = 0;
                while number_of(condition, frame, programs, time, depth)? != 0.0 {
                    rounds += 1;
                    if rounds > MAX_ROUNDS {
                        return err(format!(
                            "a `while` in a walked body took more than {MAX_ROUNDS} rounds \
                             without its condition turning false"
                        ));
                    }
                    match run(inner, frame, programs, time, depth)? {
                        Flow::Onwards => {}
                        Flow::Broke => break,
                        Flow::Returned => return Ok(Flow::Returned),
                    }
                }
            }
            Statement::For(variable, range, inner) => {
                for value in loop_over(range.as_ref(), frame, programs, time, depth)? {
                    frame.numbers.insert(variable.clone(), value);
                    match run(inner, frame, programs, time, depth)? {
                        Flow::Onwards => {}
                        Flow::Broke => break,
                        Flow::Returned => return Ok(Flow::Returned),
                    }
                }
            }
            Statement::Break => return Ok(Flow::Broke),
            Statement::Return => return Ok(Flow::Returned),
            Statement::TupleAssign(targets, value) => {
                // `(d, T) := dTofph(...)`: a body answering with
                // several numbers fills several targets, in the order
                // it declares them. A hole - `(d, ) := ...` - takes
                // its number and drops it.
                let Expr::Call(name, args) = value else {
                    return err(
                        "the right of a tuple assignment is a call to something that \
                         answers with several things"
                            .to_string(),
                    );
                };
                let mut given = Vec::new();
                let mut shapes: Vec<Vec<usize>> = Vec::new();
                for arg in args {
                    // An array goes over as its elements, under the
                    // length the callee reads it by: the generators
                    // write `(r, state) := random(state)`, and a state
                    // read as one number was an array reaching the
                    // evaluator.
                    let spread = match arg {
                        Expr::Ref(inner)
                            if frame.lengths.contains_key(inner) && tuple_arrays_open() =>
                        {
                            elements_of(arg, frame, programs, time, depth)?
                        }
                        Expr::Array(_) | Expr::Range(..) if tuple_arrays_open() => {
                            elements_of(arg, frame, programs, time, depth)?
                        }
                        // A run of an array - `random(state[i-2:i-1])`
                        // steps the seeding two numbers at a time - is
                        // its elements too; a single element is not.
                        Expr::Index(_, subscripts)
                            if walked_slice_writes_open()
                                && subscripts.iter().any(|subscript| {
                                    matches!(subscript, Expr::Range(..) | Expr::ColonSubscript)
                                }) =>
                        {
                            elements_of(arg, frame, programs, time, depth)?
                        }
                        _ => None,
                    };
                    match spread {
                        Some(items) => {
                            for item in &items {
                                given.push(number_of(item, frame, programs, time, depth)?);
                            }
                            shapes.push(vec![items.len()]);
                        }
                        None => {
                            given.push(number_of(arg, frame, programs, time, depth)?);
                            shapes.push(Vec::new());
                        }
                    }
                }
                let answer = walk(programs, name, &given, &shapes, time, depth + 1)?;
                // The answer is the outputs laid end to end, an array
                // output as its elements. A target that is a whole
                // array takes as many numbers as it is long; every
                // other target takes one. A hole takes the length of
                // the output in its place, which only the callee knows.
                let outputs: Vec<&Component> = programs
                    .get(name.as_str())
                    .map(|class| {
                        class
                            .components
                            .iter()
                            .filter(|c| c.causality == Causality::Output)
                            .collect()
                    })
                    .unwrap_or_default();
                let mut at = 0;
                for (place, target) in targets.iter().enumerate() {
                    let length = match target {
                        Some((named, subscripts))
                            if subscripts.is_empty() && tuple_arrays_open() =>
                        {
                            frame.lengths.get(named).copied()
                        }
                        Some(_) => None,
                        None => outputs.get(place).and_then(|output| {
                            match output.dimensions.as_slice() {
                                [Expr::Number(length)] if tuple_arrays_open() => {
                                    Some(*length as usize)
                                }
                                _ => None,
                            }
                        }),
                    };
                    let take = length.unwrap_or(1);
                    if at + take > answer.len() {
                        return err(format!(
                            "`{name}` answers with {} number(s), and {} target(s) asked \
                             for more",
                            answer.len(),
                            targets.len()
                        ));
                    }
                    let worths = &answer[at..at + take];
                    at += take;
                    let Some((named, subscripts)) = target else {
                        continue;
                    };
                    if let Some(length) = length {
                        for (index, worth) in (1..=length).zip(worths) {
                            frame.numbers.insert(format!("{named}[{index}]"), *worth);
                        }
                        continue;
                    }
                    let mut held = named.clone();
                    if !subscripts.is_empty() {
                        let mut indices = Vec::new();
                        for subscript in subscripts {
                            indices.push(
                                index_of(subscript, frame, programs, time, depth)?.to_string(),
                            );
                        }
                        held = format!("{named}[{}]", indices.join(","));
                    }
                    frame.numbers.insert(held, worths[0]);
                }
            }
            Statement::When(_) => {
                return err(
                    "a `when` belongs to a model's own statements, not to a body the run \
                     walks: there is no event inside a call"
                        .to_string(),
                )
            }
        }
    }
    Ok(Flow::Onwards)
}

/// What a `for` inside a walked body runs over. The bounds are numbers
/// by the time they are asked for, so a range the model decides is as
/// good as one the compiler could have seen.
fn loop_over(
    range: Option<&Expr>,
    frame: &Frame,
    programs: &HashMap<String, ClassDef>,
    time: f64,
    depth: usize,
) -> Result<Vec<f64>, SimError> {
    let Some(range) = range else {
        return err(
            "a `for` with no range needs an array to read one from, and a walked body \
             holds no arrays"
                .to_string(),
        );
    };
    let mut number = |expr: &Expr| number_of(expr, frame, programs, time, depth);
    match range {
        Expr::Range(from, step, to) => {
            let (from, to) = (number(from)?, number(to)?);
            let step = step.as_deref().map(number).transpose()?.unwrap_or(1.0);
            if step == 0.0 {
                return err("a range cannot step by zero".to_string());
            }
            let count = ((to - from) / step + 1e-9).floor() as i64 + 1;
            Ok((0..count.max(0))
                .map(|index| from + index as f64 * step)
                .collect())
        }
        Expr::Array(items) => items.iter().map(&mut number).collect(),
        other => err(format!(
            "a `for` in a walked body runs over a range or a set written out, not {other:?}"
        )),
    }
}

/// Whether a tuple assignment inside a walked body hands an array
/// argument over whole and lets an array target take its elements.
/// `OXIDELICA_NO_WALKED_TUPLE_ARRAYS` keeps the old reading, one
/// number per argument and per target, so that one binary gives both
/// numbers.
fn tuple_arrays_open() -> bool {
    !switch_set("OXIDELICA_NO_WALKED_TUPLE_ARRAYS")
}
