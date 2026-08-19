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

/// Walk a function body and give back what its output holds.
pub(crate) fn walk(
    programs: &HashMap<String, ClassDef>,
    name: &str,
    args: &[f64],
    time: f64,
    depth: usize,
) -> Result<f64, SimError> {
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
    if inputs.len() != args.len() {
        return err(format!(
            "`{name}` takes {} argument(s), given {}",
            inputs.len(),
            args.len()
        ));
    }
    // The frame: the arguments under the names the body knows them by,
    // and everything else it declared starting where it was told to.
    let mut frame: HashMap<String, f64> = HashMap::new();
    for (input, value) in inputs.iter().zip(args) {
        frame.insert(input.name.clone(), *value);
    }
    for component in &class.components {
        if component.causality == Causality::Input {
            continue;
        }
        let start = component
            .binding
            .as_ref()
            .or(component.start.as_ref())
            .map(|expr| number_of(expr, &frame, programs, time, depth))
            .transpose()?
            .unwrap_or(0.0);
        frame.insert(component.name.clone(), start);
    }
    run(&class.algorithm, &mut frame, programs, time, depth)?;
    // Both of these were settled before the model was compiled: a body
    // the run walks was checked to give exactly one answer, and every
    // name it declares was put in the frame above.
    let output = class
        .components
        .iter()
        .find(|component| component.causality == Causality::Output)
        .expect("a walked body gives one answer");
    Ok(frame[&output.name])
}

/// What an expression is worth inside a frame, calls to other walked
/// bodies included.
fn number_of(
    expr: &Expr,
    frame: &HashMap<String, f64>,
    programs: &HashMap<String, ClassDef>,
    time: f64,
    depth: usize,
) -> Result<f64, SimError> {
    code::eval(
        expr,
        &EvalCtx {
            vars: frame,
            time,
            programs: Some(programs),
            depth,
        },
    )
}

/// Walk a run of statements.
fn run(
    body: &[Statement],
    frame: &mut HashMap<String, f64>,
    programs: &HashMap<String, ClassDef>,
    time: f64,
    depth: usize,
) -> Result<Flow, SimError> {
    for statement in body {
        match statement {
            Statement::Assign(target, subscripts, value) => {
                if !subscripts.is_empty() {
                    return err(format!(
                        "`{target}` is subscripted in a body the run walks, and a walk \
                         carries numbers rather than arrays"
                    ));
                }
                let worth = number_of(value, frame, programs, time, depth)?;
                frame.insert(target.clone(), worth);
            }
            Statement::Assert(condition, message) => {
                if number_of(condition, frame, programs, time, depth)? == 0.0 {
                    return err(message.clone());
                }
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
                    frame.insert(variable.clone(), value);
                    match run(inner, frame, programs, time, depth)? {
                        Flow::Onwards => {}
                        Flow::Broke => break,
                        Flow::Returned => return Ok(Flow::Returned),
                    }
                }
            }
            Statement::Break => return Ok(Flow::Broke),
            Statement::Return => return Ok(Flow::Returned),
            Statement::TupleAssign(_, _) => {
                return err(
                    "a body the run walks gives one answer, so it fills one target and not \
                     a tuple of them"
                        .to_string(),
                )
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
    frame: &HashMap<String, f64>,
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
