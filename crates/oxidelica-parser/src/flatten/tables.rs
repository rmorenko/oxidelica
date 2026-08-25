//! Tables the standard library keeps outside Modelica, written out.
//!
//! `Modelica.Blocks.Tables.CombiTable1Ds` holds its data in a handle -
//! an `ExternalObject` built from the table matrix - and asks C for a
//! value at every step. Nothing about that needs C where the matrix is
//! written in the model: interpolating between two rows is arithmetic,
//! and this compiler's whole way of working is to write arithmetic out
//! where it is called.
//!
//! So a table becomes an expression: a chain of `if`s over the abscissa
//! column, one branch per interval, each the line or the level that
//! interval carries. What comes out is differentiable, foldable and
//! readable, and there is nothing left to run.
//!
//! A table read from a file is a different question - the data is not
//! in the model - and is refused, by name, as before.

use super::*;

/// The smoothness numbers of `Modelica.Blocks.Types.Smoothness`,
/// counted from one the way an enumeration is.
const LINEAR_SEGMENTS: f64 = 1.0;
const CONSTANT_SEGMENTS: f64 = 3.0;

/// The extrapolation numbers of `Modelica.Blocks.Types.Extrapolation`.
const HOLD_LAST_POINT: f64 = 1.0;
const LAST_TWO_POINTS: f64 = 2.0;
const PERIODIC: f64 = 3.0;
const NO_EXTRAPOLATION: f64 = 4.0;

/// One table, as the model wrote it.
struct Table {
    /// The rows, each as many numbers wide as the matrix.
    rows: Vec<Vec<f64>>,
    /// Which column of the matrix each output reads, counted from one.
    columns: Vec<usize>,
    smoothness: f64,
    extrapolation: f64,
    /// Where the first column is time: what to add to it, and the
    /// instant before which the table says nothing at all. Both are
    /// zero for a table whose first column is an ordinary abscissa.
    shift: f64,
    starts: f64,
}

/// Work out every table call in the model, or say why one cannot be.
pub(super) fn resolve_tables(
    model: &mut Model,
    handles: &HashMap<String, Expr>,
    settled: &strings::Settled,
) -> Result<(), String> {
    if handles.is_empty() {
        return Ok(());
    }
    // A table asked for no extrapolation leaves a check behind where
    // it is written out, and this is what takes them up: the model
    // being built is here, and an expression has nowhere to put one.
    let mark = super::algorithms::checks_mark();
    let (texts, numbers) = (&settled.texts, &settled.numbers);
    let mut tables: HashMap<String, Table> = HashMap::new();
    for (name, built) in handles {
        if let Some(table) = read_table(&settle(built, texts, numbers), numbers) {
            tables.insert(name.clone(), table);
        }
    }
    if tables.is_empty() {
        return Ok(());
    }
    let mut trouble = None;
    let mut rewrite = |expr: &Expr| -> Expr {
        match written_out(expr, &tables) {
            Ok(said) => said,
            Err(why) => {
                trouble.get_or_insert(why);
                expr.clone()
            }
        }
    };
    for equation in model
        .equations
        .iter_mut()
        .chain(model.initial_equations.iter_mut())
    {
        equation.lhs = rewrite(&equation.lhs);
        equation.rhs = rewrite(&equation.rhs);
    }
    for component in &mut model.components {
        if let Some(binding) = &component.binding {
            component.binding = Some(rewrite(binding));
        }
        if let Some(start) = &component.start {
            component.start = Some(rewrite(start));
        }
    }
    for (condition, _) in &mut model.asserts {
        *condition = rewrite(condition);
    }
    // Whatever the writing out left behind, in the order it came.
    model.asserts.extend(super::algorithms::checks_taken(mark));
    // A table whose first column is time says when it next turns a
    // corner, and the block asks that at an event.
    for clause in &mut model.when_clauses {
        for branch in &mut clause.branches {
            branch.condition = rewrite(&branch.condition);
            for action in &mut branch.actions {
                match action {
                    WhenAction::Assign(_, value)
                    | WhenAction::Reinit(_, value)
                    | WhenAction::TupleAssign(_, value) => *value = rewrite(value),
                    // A check made at the event may ask a table the
                    // same way an assignment may.
                    WhenAction::Assert(condition, _) => *condition = rewrite(condition),
                    // Taken apart while flattening, so none of these
                    // reaches a flat model.
                    WhenAction::Terminate(_)
                    | WhenAction::Call(..)
                    | WhenAction::Loop(_)
                    | WhenAction::Choice(_) => {}
                }
            }
        }
    }
    for conditional in &mut model.conditional {
        for condition in &mut conditional.conditions {
            *condition = rewrite(condition);
        }
        for branch in &mut conditional.branches {
            for equation in branch {
                equation.lhs = rewrite(&equation.lhs);
                equation.rhs = rewrite(&equation.rhs);
            }
        }
    }
    match trouble {
        Some(why) => Err(why),
        None => Ok(()),
    }
}

/// The table behind a handle, where the handle is a one-dimensional
/// table built from a matrix written in the model.
///
/// `None` where it is anything else - a table read from a file, a
/// two-dimensional one, a matrix this compiler could not settle - and
/// the call is left standing to be refused by name.
fn read_table(built: &Expr, numbers: &HashMap<String, f64>) -> Option<Table> {
    // The standard library has numbered its way to `_init3`; each
    // takes the same things first, and later ones say more about
    // reading a file, which is not a table this reads anyway. A file
    // name other than `"NoName"` means the data is not here at all.
    let Expr::Call(made, args) = built else {
        return None;
    };
    // The two kinds differ in what they were handed: a table whose
    // first column is time is also told where its time starts and how
    // far it is shifted.
    let time_table = made.starts_with("ModelicaStandardTables_CombiTimeTable_init");
    let plain = made.starts_with("ModelicaStandardTables_CombiTable1D_init");
    let wanted = if time_table { 8 } else { 6 };
    if (!time_table && !plain)
        || args.len() < wanted
        || !matches!(&args[1], Expr::Str(name) if name == "NoName")
    {
        return None;
    }
    // One row is a table too: the standard library's clutches give a
    // friction coefficient that way, and what it says is that value
    // everywhere.
    let rows = matrix(&args[2], numbers)?;
    let width = rows.first()?.len();
    if rows.iter().any(|row| row.len() != width) {
        return None;
    }
    let at = |which: usize| which + usize::from(time_table);
    let columns: Vec<usize> = vector(&args[at(3)], numbers)?
        .into_iter()
        .map(|column| column as usize)
        .collect();
    Some(Table {
        rows,
        columns,
        smoothness: const_eval(&args[at(4)], numbers)?,
        extrapolation: const_eval(&args[at(5)], numbers)?,
        shift: match time_table {
            true => const_eval(&args[7], numbers)?,
            false => 0.0,
        },
        starts: match time_table {
            true => const_eval(&args[3], numbers)?,
            false => f64::NEG_INFINITY,
        },
    })
}

/// Everything in an expression that is settled before the run, settled:
/// a string where one can be read, a number where one can be worked
/// out, and the branch that holds where an `if` can be decided.
///
/// The handle a table block builds is written for the general case -
/// the file name is `if tableOnFile then ... else "NoName"` - and this
/// is what makes the case at hand plain.
fn settle(expr: &Expr, texts: &HashMap<String, String>, numbers: &HashMap<String, f64>) -> Expr {
    if let Some(text) = strings::text_of(expr, texts, numbers) {
        return Expr::Str(text);
    }
    if let Some(number) = const_eval(expr, numbers) {
        return Expr::Number(number);
    }
    if let Expr::Call(name, args) = expr {
        if let Some(number) = external::number_of(name, args, texts, numbers) {
            return Expr::Number(number);
        }
    }
    // An `if` is settled here rather than by the string layer, because
    // what decides it may be another `if` - a table block asks whether
    // a file name is empty inside asking whether there is one at all -
    // and each round of that has to be read the same deep way.
    let recur = |e: &Expr| settle(e, texts, numbers);
    if let Expr::If(condition, then, otherwise) = expr {
        let condition = recur(condition);
        return match const_eval(&condition, numbers) {
            Some(truth) if truth != 0.0 => recur(then),
            Some(_) => recur(otherwise),
            None => Expr::If(
                Box::new(condition),
                Box::new(recur(then)),
                Box::new(recur(otherwise)),
            ),
        };
    }
    // The rest the string layer already knows how to read: a
    // comparison of two strings is the truth it is, and what holds no
    // string is handed back with its own parts read.
    let settled = match expr {
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        Expr::Array(items) => Expr::Array(items.iter().map(recur).collect()),
        other => strings::fold(other, texts, numbers).unwrap_or_else(|_| other.clone()),
    };
    match const_eval(&settled, numbers) {
        Some(number) => Expr::Number(number),
        None => settled,
    }
}

/// A matrix written out as an array of arrays of numbers.
fn matrix(expr: &Expr, numbers: &HashMap<String, f64>) -> Option<Vec<Vec<f64>>> {
    let Expr::Array(rows) = expr else {
        return None;
    };
    rows.iter().map(|row| vector(row, numbers)).collect()
}

/// A vector written out as an array of numbers.
fn vector(expr: &Expr, numbers: &HashMap<String, f64>) -> Option<Vec<f64>> {
    let Expr::Array(cells) = expr else {
        return None;
    };
    cells.iter().map(|cell| const_eval(cell, numbers)).collect()
}

/// Replace every table call in an expression with what the table says.
fn written_out(expr: &Expr, tables: &HashMap<String, Table>) -> Result<Expr, String> {
    let recur = |e: &Expr| written_out(e, tables);
    if let Expr::Call(name, args) = expr {
        if let Some(said) = one_call(name, args, tables)? {
            return Ok(said);
        }
    }
    Ok(match expr {
        Expr::Call(name, args) => Expr::Call(
            name.clone(),
            args.iter().map(recur).collect::<Result<Vec<_>, _>>()?,
        ),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner)?)),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner)?)),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)?), Box::new(recur(r)?)),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)?), Box::new(recur(r)?)),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)?), Box::new(recur(r)?)),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)?), Box::new(recur(r)?)),
        Expr::If(c, a, b) => Expr::If(
            Box::new(recur(c)?),
            Box::new(recur(a)?),
            Box::new(recur(b)?),
        ),
        // A call the run walks carries its own rule for differentiating
        // it beside it, and both sides are expressions like any other.
        Expr::WithDerivative(value, rule, seeds) => Expr::WithDerivative(
            Box::new(recur(value)?),
            Box::new(recur(rule)?),
            seeds
                .iter()
                .map(|(name, seed)| Ok((name.clone(), recur(seed)?)))
                .collect::<Result<Vec<_>, String>>()?,
        ),
        other => other.clone(),
    })
}

/// One call to a table body, where it is one this compiler answers.
fn one_call(
    name: &str,
    args: &[Expr],
    tables: &HashMap<String, Table>,
) -> Result<Option<Expr>, String> {
    let handle = match args.first() {
        Some(Expr::Ref(handle)) => handle,
        _ => return Ok(None),
    };
    let Some(table) = tables.get(handle) else {
        return Ok(None);
    };
    let abscissa: Vec<f64> = table.rows.iter().map(|row| row[0]).collect();
    match name {
        "ModelicaStandardTables_CombiTable1D_minimumAbscissa" => {
            Ok(Some(Expr::Number(abscissa[0])))
        }
        "ModelicaStandardTables_CombiTable1D_maximumAbscissa" => {
            Ok(Some(Expr::Number(abscissa[abscissa.len() - 1])))
        }
        "ModelicaStandardTables_CombiTable1D_getValue" if args.len() == 3 => {
            Ok(Some(interpolate(table, &args[1], &args[2], false)?))
        }
        "ModelicaStandardTables_CombiTable1D_getDerValue" if args.len() == 4 => {
            let slope = interpolate(table, &args[1], &args[2], true)?;
            Ok(Some(Expr::Bin(
                BinOp::Mul,
                Box::new(slope),
                Box::new(args[3].clone()),
            )))
        }
        // The same, where the abscissa is time. Both are asked for
        // the instant to look at and, in the time table's case, for
        // the two event instants around it - which say which side of a
        // jump the run is on, and are not needed to say what the value
        // is: the chain of `if`s below already tests one side at a
        // time.
        "ModelicaStandardTables_CombiTimeTable_minimumTime" => {
            Ok(Some(Expr::Number(abscissa[0] + table.shift)))
        }
        "ModelicaStandardTables_CombiTimeTable_maximumTime" => Ok(Some(Expr::Number(
            abscissa[abscissa.len() - 1] + table.shift,
        ))),
        "ModelicaStandardTables_CombiTimeTable_getValue" if args.len() == 5 => {
            Ok(Some(interpolate(table, &args[1], &args[2], false)?))
        }
        "ModelicaStandardTables_CombiTimeTable_getDerValue" if args.len() >= 6 => {
            let slope = interpolate(table, &args[1], &args[2], true)?;
            Ok(Some(Expr::Bin(
                BinOp::Mul,
                Box::new(slope),
                Box::new(args[5].clone()),
            )))
        }
        // When the table next turns a corner, which is what a run
        // needs to put an event there. Past the last corner there is
        // none, and the standard library reads that as an infinity.
        "ModelicaStandardTables_CombiTimeTable_nextTimeEvent" if args.len() == 2 => {
            let mut written = Expr::Number(f64::INFINITY);
            for corner in abscissa.iter().rev() {
                let corner = corner + table.shift;
                written = Expr::If(
                    Box::new(Expr::Rel(
                        RelOp::Lt,
                        Box::new(args[1].clone()),
                        Box::new(Expr::Number(corner)),
                    )),
                    Box::new(Expr::Number(corner)),
                    Box::new(written),
                );
            }
            Ok(Some(written))
        }
        _ => Ok(None),
    }
}

/// The table's value - or its slope - at `u`, written out.
///
/// Each interval of the abscissa is one branch of a chain of `if`s,
/// tested in order, so the first that holds is the interval `u` is in.
/// The last branch is what stands beyond the table, which is the same
/// line as the last interval where the extrapolation says to carry it
/// on and a level where it says to hold.
fn interpolate(table: &Table, column: &Expr, u: &Expr, slope: bool) -> Result<Expr, String> {
    let which = const_eval(column, &HashMap::new())
        .ok_or_else(|| "a table is asked for a column the compiler cannot see".to_string())?;
    let column = *table
        .columns
        .get(which as usize - 1)
        .ok_or_else(|| format!("a table has no output {which}"))?;
    if column < 2 || column > table.rows[0].len() {
        return Err(format!(
            "a table is asked for column {column}, and it has {}",
            table.rows[0].len()
        ));
    }
    if table.smoothness != LINEAR_SEGMENTS && table.smoothness != CONSTANT_SEGMENTS {
        return Err(
            "a table asks for spline interpolation, and this compiler writes out the linear \
             and the constant only"
                .to_string(),
        );
    }
    if !matches!(
        table.extrapolation,
        LAST_TWO_POINTS | HOLD_LAST_POINT | PERIODIC | NO_EXTRAPOLATION
    ) {
        return Err("a table asks for an extrapolation this compiler does not know".to_string());
    }
    let value = |row: usize| table.rows[row][column - 1];
    // Where the first column is time, the table may sit shifted along
    // it; everywhere else the shift is zero and this is the column as
    // written.
    let at = |row: usize| table.rows[row][0] + table.shift;
    // A table asked to repeat says the same thing every period: what
    // it is asked at is brought back into the one scope it was written
    // for, and everything below reads that instead. `mod` of the
    // distance from the near end by the width of the table is where in
    // it the run has arrived; the value beyond the far end is then
    // never reached, since nothing gets there.
    let period = at(table.rows.len() - 1) - at(0);
    let u = &match table.extrapolation == PERIODIC && period > 0.0 {
        false => u.clone(),
        true => Expr::Bin(
            BinOp::Add,
            Box::new(Expr::Number(at(0))),
            Box::new(Expr::Call(
                "mod".to_string(),
                vec![
                    Expr::Bin(
                        BinOp::Sub,
                        Box::new(u.clone()),
                        Box::new(Expr::Number(at(0))),
                    ),
                    Expr::Number(period),
                ],
            )),
        ),
    };
    // What one interval says, as a value or as a slope.
    let piece = |first: usize| -> Expr {
        let last = first + 1;
        let run = at(last) - at(first);
        if table.smoothness == CONSTANT_SEGMENTS || run == 0.0 {
            return Expr::Number(match slope {
                true => 0.0,
                false => value(first),
            });
        }
        let rise = (value(last) - value(first)) / run;
        if slope {
            return Expr::Number(rise);
        }
        // `y[k] + (u - u[k]) * rise`
        Expr::Bin(
            BinOp::Add,
            Box::new(Expr::Number(value(first))),
            Box::new(Expr::Bin(
                BinOp::Mul,
                Box::new(Expr::Bin(
                    BinOp::Sub,
                    Box::new(u.clone()),
                    Box::new(Expr::Number(at(first))),
                )),
                Box::new(Expr::Number(rise)),
            )),
        )
    };
    // Before it starts, a table whose first column is time says
    // nothing: what the block puts out there is its offset alone.
    let before_it_starts = |written: Expr| match table.starts.is_finite() {
        false => written,
        true => Expr::If(
            Box::new(Expr::Rel(
                RelOp::Lt,
                Box::new(u.clone()),
                Box::new(Expr::Number(table.starts)),
            )),
            Box::new(Expr::Number(0.0)),
            Box::new(written),
        ),
    };
    // A table of one row has no interval to be in: what it says, it
    // says everywhere, and it has no slope.
    if table.rows.len() == 1 {
        return Ok(before_it_starts(Expr::Number(match slope {
            true => 0.0,
            false => value(0),
        })));
    }
    // Beyond the far end: the last interval's line carried on, or the
    // last point held. Constant segments hold whatever the ends say,
    // since there is no line to carry.
    let intervals = table.rows.len() - 1;
    let held_beyond =
        table.extrapolation == HOLD_LAST_POINT || table.smoothness == CONSTANT_SEGMENTS;
    let beyond = match held_beyond {
        true => Expr::Number(match slope {
            true => 0.0,
            false => value(intervals),
        }),
        false => piece(intervals - 1),
    };
    // A table asked for no extrapolation says the run has gone wrong
    // where it is read outside its own scope. The check is left for
    // the class being instantiated to take up, the way an inlined
    // body's checks are: an expression has nowhere to put one.
    if table.extrapolation == NO_EXTRAPOLATION {
        let (near, far) = (at(0), at(table.rows.len() - 1));
        super::algorithms::check_aside(
            Expr::And(
                Box::new(Expr::Rel(
                    RelOp::Ge,
                    Box::new(u.clone()),
                    Box::new(Expr::Number(near)),
                )),
                Box::new(Expr::Rel(
                    RelOp::Le,
                    Box::new(u.clone()),
                    Box::new(Expr::Number(far)),
                )),
            ),
            format!(
                "a table asked for no extrapolation is read outside \
                 the scope it was written for, which is {near} to {far}"
            ),
        );
    }
    let mut written = beyond;
    // Built from the far end back, so the nearest test ends up first.
    for first in (0..intervals).rev() {
        let held = piece(first);
        // Before the near end the first point is held, where holding
        // is what the table asked for. A level is already what it
        // would be held at, so only a line needs saying.
        let hold_near = table.extrapolation == HOLD_LAST_POINT
            && table.smoothness != CONSTANT_SEGMENTS
            && at(0) != at(1);
        let held = match first == 0 && hold_near {
            // Before the near end, the first point is held; the first
            // interval's own branch is what follows.
            true => Expr::If(
                Box::new(Expr::Rel(
                    RelOp::Lt,
                    Box::new(u.clone()),
                    Box::new(Expr::Number(at(0))),
                )),
                Box::new(Expr::Number(match slope {
                    true => 0.0,
                    false => value(0),
                })),
                Box::new(held),
            ),
            false => held,
        };
        written = Expr::If(
            Box::new(Expr::Rel(
                RelOp::Lt,
                Box::new(u.clone()),
                Box::new(Expr::Number(at(first + 1))),
            )),
            Box::new(held),
            Box::new(written),
        );
    }
    Ok(before_it_starts(written))
}
