//! The impure generator of the standard library, held in the model.
//!
//! `Modelica.Math.Random.Utilities` keeps one generator outside the
//! model: `initializeImpureRandom` hands a state of thirty-three
//! integers to `ModelicaRandom_setInternalState_xorshift1024star`, which
//! keeps it in C, and every `impureRandom(id)` after that draws a number
//! and moves the state on. The `id` is not a key to the state - there
//! is one state - but a dummy that makes the draw read something the
//! initializer wrote, "in order that sorting is correct".
//!
//! What the C keeps hidden is written here into the model instead: the
//! thirty-three integers become discrete variables of the flat model,
//! started at the state the initializer handed over, and each draw
//! becomes the actions of the `when` it stands in - the value drawn
//! from the state through the generator already written in
//! [`crate::outside`], and the state moved on. The run stays a function
//! of the model alone: nothing about the generator lives in the
//! simulator, and two models run side by side draw their own streams.
//!
//! The draws are made in the order the `when` states them, which is
//! the order the actions are carried out in. A draw anywhere a `when`
//! does not order it - an equation, a binding, a condition - has no
//! place in that order and is refused by name.

use super::*;
use std::cell::{Cell, RefCell};

/// What the initializer calls to hand the state over.
pub(super) const SET: &str = "ModelicaRandom_setInternalState_xorshift1024star";
/// What a draw calls.
pub(super) const DRAW: &str = "ModelicaRandom_impureRandom_xorshift1024star";
/// The generator itself, which takes the state and answers the value
/// and the state moved on.
const GENERATOR: &str = "ModelicaRandom_xorshift1024star";
/// Thirty-two halves of sixteen words, and the place the generator is
/// looking at.
const WORDS: usize = 33;

thread_local! {
    /// The state the initializer handed over in the flattening under
    /// way, as the expressions it came to.
    static HANDED: RefCell<Option<Vec<Expr>>> = const { RefCell::new(None) };
    /// How many draws the flattening under way has written. Each is
    /// numbered as it is inlined, because two draws written alike are
    /// two draws, and one draw read twice - `r := impureRandom(id)`
    /// then `r` twice in the integer made of it - is one.
    static DRAWN: Cell<usize> = const { Cell::new(0) };
}

/// How many draws have been written so far: a table of answers that
/// sees this move while it works something out must not remember the
/// answer, since the same asking again is another draw.
pub(super) fn draws_made() -> usize {
    DRAWN.with(Cell::get)
}

/// A draw where the function `class` is called, numbered.
pub(super) fn draw(class: &ClassDef) -> Result<Vec<(String, Expr)>, String> {
    let output = class
        .components
        .iter()
        .find(|c| c.causality == Causality::Output)
        .ok_or_else(|| format!("function `{}` declares no output", class.name))?;
    let serial = DRAWN.with(|drawn| {
        let serial = drawn.get();
        drawn.set(serial + 1);
        serial
    });
    Ok(vec![(
        output.name.clone(),
        Expr::Call(DRAW.to_string(), vec![Expr::Number(serial as f64)]),
    )])
}

/// Whether the draws are refused as they were before, as a body
/// written outside Modelica that nobody here answers for.
/// `OXIDELICA_NO_IMPURE_DRAWS` turns the road back, so that one binary
/// gives both numbers.
pub(super) fn draws_off() -> bool {
    std::env::var_os("OXIDELICA_NO_IMPURE_DRAWS").is_some()
}

/// Forget the state of the flattening before.
pub(super) fn forget() {
    HANDED.with(|held| held.borrow_mut().take());
    DRAWN.with(|drawn| drawn.set(0));
}

/// Take the state `setInternalState` was handed, as the argument its
/// declaration passes on first.
///
/// A model built twice hands the same state twice, and that is one
/// state. Two different states are two initializers, and which one the
/// draws read then depends on the order the C happened to be called
/// in - which is a guess, so it is refused.
pub(super) fn state_handed(class: &ClassDef, args: &[Expr]) -> Result<(), String> {
    let first = class
        .external_call
        .as_ref()
        .and_then(|call| call.arguments.first());
    let at = class
        .components
        .iter()
        .filter(|c| c.causality == Causality::Input)
        .position(|input| matches!(first, Some(Expr::Ref(name)) if *name == input.name));
    let Some(handed) = at.and_then(|at| args.get(at)) else {
        return Err(format!(
            "`{}` hands `{SET}` something other than one of its inputs",
            class.name
        ));
    };
    let mut words = Vec::new();
    laid_out(handed, &mut words);
    if words.len() != WORDS {
        return Err(format!(
            "`{SET}` is handed {} number(s) where the generator's state is {WORDS}: {}",
            words.len(),
            names::sketch(handed)
        ));
    }
    HANDED.with(|held| {
        let mut held = held.borrow_mut();
        match held.as_ref() {
            Some(before) if *before != words => Err(format!(
                "`{SET}` is handed two different states, and which one the draws read \
                 depends on an order the model does not state"
            )),
            _ => {
                *held = Some(words);
                Ok(())
            }
        }
    })
}

/// The numbers of a value written out, in order.
///
/// A state the compiler could not work out stands as the call that
/// makes it - the library seeds it from a parameter settled only when
/// the run begins - and is laid out a place at a time, the way a
/// walked body's answer is read.
fn laid_out(expr: &Expr, into: &mut Vec<Expr>) {
    match expr {
        Expr::Array(items) => items.iter().for_each(|item| laid_out(item, into)),
        Expr::Call(..) => into.extend(
            (1..=WORDS)
                .map(|at| Expr::Index(Box::new(expr.clone()), vec![Expr::Number(at as f64)])),
        ),
        other => into.push(other.clone()),
    }
}

fn state_name(word: usize) -> String {
    format!("$randomState{word}")
}

fn next_name(word: usize) -> String {
    format!("$randomNext{word}")
}

fn draw_name(draw: usize) -> String {
    format!("$randomDraw{draw}")
}

/// Whether an expression still holds a draw.
fn holds_draw(expr: &Expr) -> bool {
    let mut found = false;
    expr.for_each(&mut |inner| {
        if matches!(inner, Expr::Call(name, _) if name == DRAW) {
            found = true;
        }
    });
    found
}

/// Write every draw standing in a `when` out as the state it reads and
/// moves, and refuse one standing anywhere else.
pub(super) fn lower_draws(model: &mut Model) -> Result<(), String> {
    let mut drawn = 0;
    for clause in &mut model.when_clauses {
        for branch in &mut clause.branches {
            if holds_draw(&branch.condition) {
                return Err(refused_outside("the condition of a `when`"));
            }
            let mut actions = Vec::new();
            let mut seen = HashMap::new();
            for action in std::mem::take(&mut branch.actions) {
                match action {
                    WhenAction::Assign(target, value) => {
                        let value = draws_taken(&value, &mut drawn, &mut seen, &mut actions);
                        actions.push(WhenAction::Assign(target, value));
                    }
                    WhenAction::Reinit(target, value) => {
                        let value = draws_taken(&value, &mut drawn, &mut seen, &mut actions);
                        actions.push(WhenAction::Reinit(target, value));
                    }
                    WhenAction::Assert(condition, _) if holds_draw(&condition) => {
                        return Err(refused_outside("a check made at an event"));
                    }
                    other => actions.push(other),
                }
            }
            branch.actions = actions;
        }
    }
    let equations = model
        .equations
        .iter()
        .chain(model.initial_equations.iter())
        .chain(
            model
                .conditional
                .iter()
                .flat_map(|c| c.branches.iter().flatten()),
        );
    for equation in equations {
        if holds_draw(&equation.lhs) || holds_draw(&equation.rhs) {
            return Err(refused_outside("an equation"));
        }
    }
    for component in &model.components {
        for said in [&component.binding, &component.start].into_iter().flatten() {
            if holds_draw(said) {
                return Err(refused_outside(&format!(
                    "what `{}` is declared with",
                    component.name
                )));
            }
        }
    }
    if model
        .asserts
        .iter()
        .any(|(condition, _)| holds_draw(condition))
    {
        return Err(refused_outside("a check"));
    }
    if drawn == 0 {
        return Ok(());
    }
    let Some(state) = HANDED.with(|held| held.borrow().clone()) else {
        return Err(format!(
            "`{DRAW}` draws from a state nothing handed to `{SET}` - the draw has no \
             `initializeImpureRandom` behind it that this compiler reached"
        ));
    };
    for (word, start) in state.into_iter().enumerate() {
        model.components.push(Component {
            name: state_name(word),
            type_name: "Integer".to_string(),
            variability: Variability::Discrete,
            start: Some(start),
            description: Some("the state of the impure random generator".to_string()),
            ..machines::blank_component()
        });
        model.components.push(Component {
            name: next_name(word),
            type_name: "Integer".to_string(),
            variability: Variability::Discrete,
            start: Some(Expr::Number(0.0)),
            description: Some("the state of the impure random generator, moved on".to_string()),
            ..machines::blank_component()
        });
    }
    for draw in 0..drawn {
        model.components.push(Component {
            name: draw_name(draw),
            variability: Variability::Discrete,
            start: Some(Expr::Number(0.0)),
            description: Some("a number drawn from the impure random generator".to_string()),
            ..machines::blank_component()
        });
    }
    Ok(())
}

fn refused_outside(place: &str) -> String {
    format!(
        "`{DRAW}` is drawn in {place}, where no `when` says when the draw is made; \
         each draw moves the generator on, so it is taken only among the actions of a `when`"
    )
}

/// The expression with each draw in it replaced by a name of its own,
/// and the actions that give that name its value and move the state on
/// put in front of it - in the order the draws are read.
///
/// A draw is known by the number it was given where it was inlined, so
/// one draw read twice - the integer generator reads its real draw in
/// two places - is drawn once. The table is the branch's: the same
/// actions copied onto a second branch are drawn afresh there.
fn draws_taken(
    expr: &Expr,
    drawn: &mut usize,
    seen: &mut HashMap<u64, String>,
    actions: &mut Vec<WhenAction>,
) -> Expr {
    match expr {
        Expr::Call(name, args) if name == DRAW => {
            let serial = match args.as_slice() {
                [Expr::Number(serial)] => *serial as u64,
                _ => u64::MAX,
            };
            if let Some(named) = seen.get(&serial) {
                return Expr::Ref(named.clone());
            }
            let draw = *drawn;
            *drawn += 1;
            seen.insert(serial, draw_name(draw));
            // The generator answers the value first, then the halves
            // of the state it moved to, then the place. Every place is
            // read off the state as it stood before this draw, so the
            // new state is gathered first and put in after: an action
            // sees what the one before it wrote.
            let state: Vec<Expr> = (0..WORDS).map(|word| Expr::Ref(state_name(word))).collect();
            let place = |at: usize| {
                Expr::Index(
                    Box::new(Expr::Call(GENERATOR.to_string(), state.clone())),
                    vec![Expr::Number(at as f64)],
                )
            };
            actions.push(WhenAction::Assign(draw_name(draw), place(1)));
            for word in 0..WORDS {
                actions.push(WhenAction::Assign(next_name(word), place(word + 2)));
            }
            for word in 0..WORDS {
                actions.push(WhenAction::Assign(
                    state_name(word),
                    Expr::Ref(next_name(word)),
                ));
            }
            Expr::Ref(draw_name(draw))
        }
        _ => expr.map_children(&mut |child| draws_taken(child, drawn, seen, actions)),
    }
}
