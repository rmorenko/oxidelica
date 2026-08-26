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
    // A two-dimensional table is handed the same things in the same
    // order, and differs in what the matrix means rather than in how
    // it arrives.
    let grid = made.starts_with("ModelicaStandardTables_CombiTable2D_init");
    // A grid says one thing fewer than a list of outputs: it has no
    // columns to be told about.
    let wanted = match (time_table, grid) {
        (true, _) => 8,
        (_, true) => 5,
        _ => 6,
    };
    if (!time_table && !plain && !grid)
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
    // A grid says nothing about which columns to read - it is read by
    // where the two abscissae fall - and is handed the smoothness
    // where a list of outputs is handed its columns.
    let columns: Vec<usize> = match grid {
        true => Vec::new(),
        false => vector(&args[at(3)], numbers)?
            .into_iter()
            .map(|column| column as usize)
            .collect(),
    };
    let said = |which: usize| at(which) - usize::from(grid);
    Some(Table {
        rows,
        columns,
        smoothness: const_eval(&args[said(4)], numbers)?,
        extrapolation: const_eval(&args[said(5)], numbers)?,
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
    // Both ends of a grid come back as a pair, and each of the two
    // parameters holding them wants one of it: `u_min[1]` is the first
    // abscissa's least, `u_min[2]` the second's. Flattening has already
    // made them separate parameters, so the subscript is here to be
    // read and the pair is not.
    if let Expr::Index(of, at) = expr {
        if let Expr::Array(pair) = written_out(of, tables)? {
            // Modelica counts from one, so a zero or a negative
            // subscript is no place in the pair at all - and taking one
            // off it first would go round the bottom of an unsigned
            // number rather than say so.
            if let [Expr::Number(index)] = at.as_slice() {
                if let Some(one) = (*index >= 1.0)
                    .then(|| pair.get(*index as usize - 1))
                    .flatten()
                {
                    return Ok(one.clone());
                }
            }
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
        // A two-dimensional table is read by where two abscissae fall
        // in the grid: the top row is the second, the left column the
        // first, and the corner cell belongs to neither.
        // Both ends of a grid come back at once: the C function is
        // handed the array to fill rather than asked which one is
        // wanted, so what answers it is the pair.
        "ModelicaStandardTables_CombiTable2D_minimumAbscissa" => Ok(Some(Expr::Array(vec![
            Expr::Number(table.rows[1][0]),
            Expr::Number(table.rows[0][1]),
        ]))),
        "ModelicaStandardTables_CombiTable2D_maximumAbscissa" => Ok(Some(Expr::Array(vec![
            Expr::Number(table.rows[table.rows.len() - 1][0]),
            Expr::Number(table.rows[0][table.rows[0].len() - 1]),
        ]))),
        "ModelicaStandardTables_CombiTable2D_getValue" if args.len() == 3 => {
            Ok(Some(on_the_grid(table, &args[1], &args[2], Wanted::Value)?))
        }
        // How fast the value moves is how fast each abscissa moves,
        // weighted by the slope along it - the chain rule, written out
        // because the run is what knows the two rates.
        "ModelicaStandardTables_CombiTable2D_getDerValue" if args.len() == 5 => {
            let times = |slope: Expr, rate: &Expr| {
                Expr::Bin(BinOp::Mul, Box::new(slope), Box::new(rate.clone()))
            };
            Ok(Some(Expr::Bin(
                BinOp::Add,
                Box::new(times(
                    on_the_grid(table, &args[1], &args[2], Wanted::SlopeDown)?,
                    &args[3],
                )),
                Box::new(times(
                    on_the_grid(table, &args[1], &args[2], Wanted::SlopeAcross)?,
                    &args[4],
                )),
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
/// What is wanted of a two-dimensional table at a point: the value
/// there, or how fast it changes along one abscissa or the other.
#[derive(Clone, Copy, PartialEq)]
enum Wanted {
    Value,
    SlopeDown,
    SlopeAcross,
}

/// What a two-dimensional table says at `(u1, u2)`.
///
/// The matrix is a grid: its top row is the second abscissa, its left
/// column the first, and the corner cell belongs to neither. A point
/// inside falls in one cell of it, and what the table says there is
/// the four corners weighted by how far into the cell it is - the
/// bilinear reading the standard tables do.
///
/// Outside the grid, what the table asked for: the edge cell's plane
/// carried on (`LastTwoPoints`), the edge itself held
/// (`HoldLastPoint`), the grid repeated (`Periodic`), or the run told
/// it has gone wrong (`NoExtrapolation`).
fn on_the_grid(table: &Table, u1: &Expr, u2: &Expr, wanted: Wanted) -> Result<Expr, String> {
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
        return Err(format!(
            "a two-dimensional table asks for extrapolation {}, which this compiler does not know",
            table.extrapolation
        ));
    }
    let down: Vec<f64> = table.rows[1..].iter().map(|row| row[0]).collect();
    let across: Vec<f64> = table.rows[0][1..].to_vec();
    if down.is_empty() || across.is_empty() {
        return Err("a two-dimensional table has no grid to read".to_string());
    }
    let cell = |i: usize, j: usize| table.rows[1 + i][1 + j];
    // A grid asked to repeat says the same thing every period along
    // each abscissa on its own: what it is asked at is brought back
    // into the one grid it was written for, the way the one
    // dimensional table does it.
    let repeated = |axis: &[f64], u: &Expr| -> Expr {
        let period = axis[axis.len() - 1] - axis[0];
        if table.extrapolation != PERIODIC || period <= 0.0 {
            return u.clone();
        }
        Expr::Bin(
            BinOp::Add,
            Box::new(Expr::Number(axis[0])),
            Box::new(Expr::Call(
                "mod".to_string(),
                vec![
                    Expr::Bin(
                        BinOp::Sub,
                        Box::new(u.clone()),
                        Box::new(Expr::Number(axis[0])),
                    ),
                    Expr::Number(period),
                ],
            )),
        )
    };
    let (u1, u2) = (&repeated(&down, u1), &repeated(&across, u2));
    // A grid asked for no extrapolation says the run has gone wrong
    // where it is read outside the scope it was written for, on either
    // abscissa. The check is left for the class being instantiated to
    // take up, the way an inlined body's checks are.
    if table.extrapolation == NO_EXTRAPOLATION {
        for (axis, u, which) in [(&down, u1, "first"), (&across, u2, "second")] {
            let (near, far) = (axis[0], axis[axis.len() - 1]);
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
                    "a two-dimensional table asked for no extrapolation is read outside the \
                     scope its {which} abscissa was written for, which is {near} to {far}"
                ),
            );
        }
    }
    // How far into an interval a value is, held at the ends: the
    // fraction is written out rather than worked out, since what the
    // table is asked at is not known until the run.
    // Constant segments hold whatever the near corner says, and so
    // does a table asked to hold its last point once the run is past
    // the edge: the fraction is pinned to the interval rather than
    // carrying the plane on. `LastTwoPoints` lets it run, which is
    // what carrying on the edge cell's plane comes to.
    let held_beyond = table.extrapolation == HOLD_LAST_POINT
        || table.smoothness == CONSTANT_SEGMENTS
        || table.extrapolation == NO_EXTRAPOLATION;
    let along = |axis: &[f64], u: &Expr, at: usize| -> Expr {
        let (near, far) = (axis[at], axis[at + 1]);
        let run = far - near;
        if run == 0.0 || table.smoothness == CONSTANT_SEGMENTS {
            return Expr::Number(0.0);
        }
        let fraction = Expr::Bin(
            BinOp::Div,
            Box::new(Expr::Bin(
                BinOp::Sub,
                Box::new(u.clone()),
                Box::new(Expr::Number(near)),
            )),
            Box::new(Expr::Number(run)),
        );
        match held_beyond {
            false => fraction,
            true => Expr::Call(
                "min".to_string(),
                vec![
                    Expr::Number(1.0),
                    Expr::Call("max".to_string(), vec![Expr::Number(0.0), fraction]),
                ],
            ),
        }
    };
    // One cell of the grid, read bilinearly:
    // `c00 + (c10 - c00)*s + (c01 - c00)*t + (c00 - c10 - c01 + c11)*s*t`
    //
    // The slope along either abscissa is that expression differentiated
    // by the fraction along it, divided by how wide the interval is:
    // `d/du1 = ((c10 - c00) + (c00 - c10 - c01 + c11)*t) / (u1 span)`.
    let piece = |i: usize, j: usize| -> Expr {
        let (c00, c10, c01, c11) = match (down.len() > 1, across.len() > 1) {
            (true, true) => (
                cell(i, j),
                cell(i + 1, j),
                cell(i, j + 1),
                cell(i + 1, j + 1),
            ),
            (true, false) => (cell(i, j), cell(i + 1, j), cell(i, j), cell(i + 1, j)),
            (false, true) => (cell(i, j), cell(i, j), cell(i, j + 1), cell(i, j + 1)),
            (false, false) => (cell(i, j), cell(i, j), cell(i, j), cell(i, j)),
        };
        let s = match down.len() > 1 {
            true => along(&down, u1, i),
            false => Expr::Number(0.0),
        };
        let t = match across.len() > 1 {
            true => along(&across, u2, j),
            false => Expr::Number(0.0),
        };
        let times = |a: Expr, b: Expr| Expr::Bin(BinOp::Mul, Box::new(a), Box::new(b));
        let plus = |a: Expr, b: Expr| Expr::Bin(BinOp::Add, Box::new(a), Box::new(b));
        let corner = c00 - c10 - c01 + c11;
        let span = |axis: &[f64], at: usize| -> f64 {
            match axis.len() > 1 {
                true => axis[at + 1] - axis[at],
                false => 0.0,
            }
        };
        match wanted {
            Wanted::Value => plus(
                plus(
                    plus(Expr::Number(c00), times(Expr::Number(c10 - c00), s.clone())),
                    times(Expr::Number(c01 - c00), t.clone()),
                ),
                times(Expr::Number(corner), times(s, t)),
            ),
            Wanted::SlopeDown => {
                let run = span(&down, i);
                if run == 0.0 || table.smoothness == CONSTANT_SEGMENTS {
                    return Expr::Number(0.0);
                }
                plus(
                    Expr::Number((c10 - c00) / run),
                    times(Expr::Number(corner / run), t),
                )
            }
            Wanted::SlopeAcross => {
                let run = span(&across, j);
                if run == 0.0 || table.smoothness == CONSTANT_SEGMENTS {
                    return Expr::Number(0.0);
                }
                plus(
                    Expr::Number((c01 - c00) / run),
                    times(Expr::Number(corner / run), s),
                )
            }
        }
    };
    // Built from the far end back, so the nearest test ends up first
    // and a point past the last edge lands on the last cell held.
    let last_i = down.len().saturating_sub(2);
    let last_j = across.len().saturating_sub(2);
    let mut written = piece(last_i, last_j);
    for i in (0..=last_i).rev() {
        let mut row = piece(i, last_j);
        for j in (0..=last_j).rev() {
            if j == last_j {
                continue;
            }
            row = Expr::If(
                Box::new(Expr::Rel(
                    RelOp::Lt,
                    Box::new(u2.clone()),
                    Box::new(Expr::Number(across[j + 1])),
                )),
                Box::new(piece(i, j)),
                Box::new(row),
            );
        }
        if i == last_i {
            written = row;
            continue;
        }
        written = Expr::If(
            Box::new(Expr::Rel(
                RelOp::Lt,
                Box::new(u1.clone()),
                Box::new(Expr::Number(down[i + 1])),
            )),
            Box::new(row),
            Box::new(written),
        );
    }
    Ok(written)
}

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
