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
        .ok_or_else(|| SimError(format!("`{name}` is not a body this run carries")))?;
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
        // before it writes, so it is laid out before anything runs.
        if let Some(length) = declared_length(component, &frame) {
            frame.lengths.insert(component.name.clone(), length);
            for index in 1..=length {
                frame
                    .numbers
                    .insert(format!("{}[{index}]", component.name), 0.0);
            }
            continue;
        }
        let start = component
            .binding
            .as_ref()
            .or(component.start.as_ref())
            .map(|expr| number_of(expr, &frame, programs, time, depth))
            .transpose()?
            .unwrap_or(0.0);
        frame.numbers.insert(component.name.clone(), start);
    }
    run(&class.algorithm, &mut frame, programs, time, depth)?;
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
            None => answer.push(want(&output.name)),
            Some(length) => {
                answer.extend((1..=length).map(|index| want(&format!("{}[{index}]", output.name))))
            }
        }
    }
    Ok(answer)
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
                SimError(format!(
                    "`{of}` has no dimension {axis} this walk was given"
                ))
            })?;
            Expr::Number(length as f64)
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
        Expr::Call(name, args) => Expr::Call(
            name.clone(),
            args.iter()
                .map(recur)
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
        _ => None,
    })
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
            .ok_or_else(|| SimError(format!("`{name}` of an array with nothing in it")))?,
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
                    return err(message.clone());
                }
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
                for arg in args {
                    given.push(number_of(arg, frame, programs, time, depth)?);
                }
                // One number per argument: what stands here is a
                // scalar, and an array argument would say how many.
                let shapes: Vec<Vec<usize>> = vec![Vec::new(); given.len()];
                let answer = walk(programs, name, &given, &shapes, time, depth + 1)?;
                if answer.len() < targets.len() {
                    return err(format!(
                        "`{name}` answers with {} thing(s), and {} were asked for",
                        answer.len(),
                        targets.len()
                    ));
                }
                for (target, worth) in targets.iter().zip(answer) {
                    let Some((named, subscripts)) = target else {
                        continue;
                    };
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
                    frame.numbers.insert(held, worth);
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
            let step = step.as_deref().map(&mut number).transpose()?.unwrap_or(1.0);
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
