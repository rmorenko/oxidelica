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

use super::grids::{on_the_grid, Wanted};
use super::*;

/// The smoothness numbers of `Modelica.Blocks.Types.Smoothness`,
/// counted from one the way an enumeration is.
pub(super) const LINEAR_SEGMENTS: f64 = 1.0;
pub(super) const AKIMA_SPLINE: f64 = 2.0;
pub(super) const CONSTANT_SEGMENTS: f64 = 3.0;
pub(super) const FRITSCH_BUTLAND: f64 = 4.0;
pub(super) const STEFFEN: f64 = 5.0;
pub(super) const MODIFIED_AKIMA: f64 = 6.0;

/// Whether a smoothness is one of the splines: a curve drawn through
/// the points with a slope worked out at each, rather than the
/// straight lines or the levels between them.
pub(super) fn is_a_spline(smoothness: f64) -> bool {
    matches!(
        smoothness,
        AKIMA_SPLINE | FRITSCH_BUTLAND | STEFFEN | MODIFIED_AKIMA
    )
}

/// The extrapolation numbers of `Modelica.Blocks.Types.Extrapolation`.
pub(super) const HOLD_LAST_POINT: f64 = 1.0;
pub(super) const LAST_TWO_POINTS: f64 = 2.0;
pub(super) const PERIODIC: f64 = 3.0;
pub(super) const NO_EXTRAPOLATION: f64 = 4.0;

/// One table, as the model wrote it.
pub(super) struct Table {
    /// The rows, each as many numbers wide as the matrix.
    pub(super) rows: Vec<Vec<f64>>,
    /// Which column of the matrix each output reads, counted from one.
    pub(super) columns: Vec<usize>,
    pub(super) smoothness: f64,
    pub(super) extrapolation: f64,
    /// Where the first column is time: what to add to it, and the
    /// instant before which the table says nothing at all. Both are
    /// zero for a table whose first column is an ordinary abscissa.
    pub(super) shift: f64,
    pub(super) starts: f64,
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
    // Whatever the reading below refuses, it is this table's reading:
    // the handle is the block the model wrote, and naming it is the
    // difference between a refusal and a place to look.
    let named = |why: String| format!("{why}, reading `{handle}`");
    match name {
        "ModelicaStandardTables_CombiTable1D_minimumAbscissa" => {
            Ok(Some(Expr::Number(abscissa[0])))
        }
        "ModelicaStandardTables_CombiTable1D_maximumAbscissa" => {
            Ok(Some(Expr::Number(abscissa[abscissa.len() - 1])))
        }
        "ModelicaStandardTables_CombiTable1D_getValue" if args.len() == 3 => Ok(Some(
            interpolate(table, &args[1], &args[2], false).map_err(named)?,
        )),
        "ModelicaStandardTables_CombiTable1D_getDerValue" if args.len() == 4 => {
            let slope = interpolate(table, &args[1], &args[2], true).map_err(named)?;
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
        "ModelicaStandardTables_CombiTable2D_getValue" if args.len() == 3 => Ok(Some(
            on_the_grid(table, &args[1], &args[2], Wanted::Value).map_err(named)?,
        )),
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
                    on_the_grid(table, &args[1], &args[2], Wanted::SlopeDown).map_err(named)?,
                    &args[3],
                )),
                Box::new(times(
                    on_the_grid(table, &args[1], &args[2], Wanted::SlopeAcross).map_err(named)?,
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
        "ModelicaStandardTables_CombiTimeTable_getValue" if args.len() == 5 => Ok(Some(
            interpolate(table, &args[1], &args[2], false).map_err(named)?,
        )),
        "ModelicaStandardTables_CombiTimeTable_getDerValue" if args.len() >= 6 => {
            let slope = interpolate(table, &args[1], &args[2], true).map_err(named)?;
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

/// The slope the Akima spline gives each point of a table.
///
/// A weighted mean of the two straight lines meeting there, the
/// weights taken from how sharply the lines beyond them turn: where
/// the neighbours bend little the near line counts for more. Akima's
/// rule needs two lines on either side, and a table has none past its
/// ends, so the missing ones are made by carrying the end lines on -
/// which is what the standard tables do.
pub(super) fn akima_slopes(
    rows: usize,
    at: &dyn Fn(usize) -> f64,
    value: &dyn Fn(usize) -> f64,
) -> Vec<f64> {
    // The straight line of each interval, with two made up at either
    // end. Index `k + 2` of this is the line from point `k` to `k + 1`.
    let line = |first: usize| -> f64 {
        let run = at(first + 1) - at(first);
        match run == 0.0 {
            true => 0.0,
            false => (value(first + 1) - value(first)) / run,
        }
    };
    let mut lines: Vec<f64> = Vec::with_capacity(rows + 3);
    lines.push(0.0);
    lines.push(0.0);
    for first in 0..rows.saturating_sub(1) {
        lines.push(line(first));
    }
    lines.push(0.0);
    lines.push(0.0);
    let inner = rows.saturating_sub(1);
    if inner >= 2 {
        // Beyond the ends, the line carried on as the two before it
        // turned: `m[-1] = 2*m[0] - m[1]`, and the same at the far end.
        lines[1] = 2.0 * lines[2] - lines[3];
        lines[0] = 2.0 * lines[1] - lines[2];
        lines[inner + 2] = 2.0 * lines[inner + 1] - lines[inner];
        lines[inner + 3] = 2.0 * lines[inner + 2] - lines[inner + 1];
    } else if inner == 1 {
        // One interval: nothing turns, so every made-up line is that
        // one line and the spline comes out straight.
        for slot in [0, 1, 3, 4] {
            lines[slot] = lines[2];
        }
    }
    (0..rows)
        .map(|k| {
            let (before, near, far, beyond) = (lines[k], lines[k + 1], lines[k + 2], lines[k + 3]);
            let (turn_near, turn_far) = ((near - before).abs(), (beyond - far).abs());
            // Both sides straight: the two lines meeting here are the
            // answer, and where they differ their mean is.
            match turn_near + turn_far == 0.0 {
                true => (near + far) / 2.0,
                false => (turn_far * near + turn_near * far) / (turn_near + turn_far),
            }
        })
        .collect()
}

/// The slopes a spline gives a run of points, by the rule asked for.
///
/// The four splines differ only in how they choose a slope at each
/// point; what is drawn between two points is the same cubic either
/// way. This is where the choice is made, so that a table and a grid
/// make it the same way.
pub(super) fn spline_slopes(
    rows: usize,
    at: &dyn Fn(usize) -> f64,
    value: &dyn Fn(usize) -> f64,
    smoothness: f64,
) -> Vec<f64> {
    match smoothness {
        FRITSCH_BUTLAND | STEFFEN => monotone_slopes(rows, at, value, smoothness),
        MODIFIED_AKIMA => modified_akima_slopes(rows, at, value),
        _ => akima_slopes(rows, at, value),
    }
}

/// The slope a monotone spline gives each point of a table.
///
/// Akima's rule draws a smooth curve but may overshoot: between two
/// points that rise, the curve is entitled to dip. Where a table
/// stands for something that only ever rises - a characteristic
/// curve, a lookup of one quantity against another - that dip is
/// wrong, and the standard tables offer two rules that cannot make
/// one.
///
/// Fritsch-Butland takes the harmonic mean of the two lines meeting
/// at a point, weighted towards the shorter interval, and nothing at
/// all where they disagree in sign - a peak or a trough of the table
/// is flat, which is what keeps each stretch going the way it went.
///
/// Steffen starts from the ordinary central difference and then pulls
/// it back to twice the smaller of the two lines, which is the most a
/// cubic can be given at a point and still not turn back on itself.
fn monotone_slopes(
    rows: usize,
    at: &dyn Fn(usize) -> f64,
    value: &dyn Fn(usize) -> f64,
    rule: f64,
) -> Vec<f64> {
    let line = |first: usize| -> f64 {
        let run = at(first + 1) - at(first);
        match run == 0.0 {
            true => 0.0,
            false => (value(first + 1) - value(first)) / run,
        }
    };
    let width = |first: usize| at(first + 1) - at(first);
    (0..rows)
        .map(|k| {
            // The ends take the line they have, which is what both
            // rules come to where there is only one.
            if rows < 2 {
                return 0.0;
            }
            if k == 0 || k == rows - 1 {
                let only = match k == 0 {
                    true => line(0),
                    false => line(rows - 2),
                };
                if rule != STEFFEN || rows < 3 {
                    return only;
                }
                // Steffen's end: the parabola through the three
                // points nearest the end, held to twice the line so
                // that the end cannot overshoot either.
                let (near, far, h1, h2) = match k == 0 {
                    true => (line(0), line(1), width(0), width(1)),
                    false => (
                        line(rows - 2),
                        line(rows - 3),
                        width(rows - 2),
                        width(rows - 3),
                    ),
                };
                let guessed = ((2.0 * h1 + h2) * near - h1 * far) / (h1 + h2);
                if guessed * near <= 0.0 {
                    return 0.0;
                }
                if (guessed).abs() > 2.0 * near.abs() {
                    return 2.0 * near;
                }
                return guessed;
            }
            let (before, after) = (line(k - 1), line(k));
            // A peak or a trough: the two lines disagree in sign, or
            // one of them is flat. Either way the point is where the
            // table turns, and a monotone curve is flat there.
            if before * after <= 0.0 {
                return 0.0;
            }
            let (h1, h2) = (width(k - 1), width(k));
            match rule == STEFFEN {
                // Steffen: the central difference, held to twice the
                // smaller line.
                true => {
                    let central = (before * h2 + after * h1) / (h1 + h2);
                    let most = 2.0 * before.abs().min(after.abs());
                    match central.abs() > most {
                        true => most * central.signum(),
                        false => central,
                    }
                }
                // Fritsch-Butland: the weighted harmonic mean, which
                // is never larger than three times the smaller line
                // and so never overshoots.
                false => {
                    let alpha = (h1 + 2.0 * h2) / (3.0 * (h1 + h2));
                    before * after / (alpha * after + (1.0 - alpha) * before)
                }
            }
        })
        .collect()
}

/// The slope the modified Akima spline gives each point.
///
/// Akima weights the two lines meeting at a point by how sharply the
/// lines beyond them turn, and where two neighbouring intervals are
/// flat those weights are nothing and the rule falls back on an
/// average that puts a kink in a straight stretch. The modification
/// adds the width of the turn to each weight, which leaves the rule
/// as it was wherever it worked and settles the flat case towards the
/// nearer line. It is what MATLAB calls `makima`.
fn modified_akima_slopes(
    rows: usize,
    at: &dyn Fn(usize) -> f64,
    value: &dyn Fn(usize) -> f64,
) -> Vec<f64> {
    let line = |first: usize| -> f64 {
        let run = at(first + 1) - at(first);
        match run == 0.0 {
            true => 0.0,
            false => (value(first + 1) - value(first)) / run,
        }
    };
    let mut lines: Vec<f64> = Vec::with_capacity(rows + 3);
    lines.push(0.0);
    lines.push(0.0);
    for first in 0..rows.saturating_sub(1) {
        lines.push(line(first));
    }
    lines.push(0.0);
    lines.push(0.0);
    let inner = rows.saturating_sub(1);
    if inner >= 2 {
        lines[1] = 2.0 * lines[2] - lines[3];
        lines[0] = 2.0 * lines[1] - lines[2];
        lines[inner + 2] = 2.0 * lines[inner + 1] - lines[inner];
        lines[inner + 3] = 2.0 * lines[inner + 2] - lines[inner + 1];
    } else if inner == 1 {
        for slot in [0, 1, 3, 4] {
            lines[slot] = lines[2];
        }
    }
    (0..rows)
        .map(|k| {
            let (before, near, far, beyond) = (lines[k], lines[k + 1], lines[k + 2], lines[k + 3]);
            let weight_near = (beyond - far).abs() + (beyond + far).abs() / 2.0;
            let weight_far = (near - before).abs() + (near + before).abs() / 2.0;
            match weight_near + weight_far == 0.0 {
                true => (near + far) / 2.0,
                false => (weight_near * near + weight_far * far) / (weight_near + weight_far),
            }
        })
        .collect()
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
    if !matches!(table.smoothness, LINEAR_SEGMENTS | CONSTANT_SEGMENTS)
        && !is_a_spline(table.smoothness)
    {
        return Err(format!(
            "a table asks for smoothness {}, which this compiler does not know",
            table.smoothness
        ));
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
    // The slope the Akima spline gives each point: a weighted mean of
    // the two straight lines meeting there, the weights taken from how
    // sharply the lines on either side turn. Where a point sits between
    // two lines of the same slope the mean is that slope, so a straight
    // stretch of table stays straight - which is the whole point of
    // Akima's rule over an ordinary cubic.
    let akima: Vec<f64> = match is_a_spline(table.smoothness) {
        false => Vec::new(),
        true => {
            // A spline is drawn through points that follow one another
            // along the abscissa, and two points at the same place
            // leave it nothing to be drawn through: the interval
            // between them has no width, so the line across it is
            // undefined rather than flat. Left alone it came out as a
            // slope of zero and the curve quietly went wrong.
            //
            // Straight lines and levels are another matter: there a
            // repeated abscissa is a step, which the table is entitled
            // to say, and it keeps working as it did.
            if let Some(repeated) = (1..table.rows.len()).find(|&row| at(row) == at(row - 1)) {
                return Err(format!(
                    "a table asked for a spline gives {} twice on its abscissa, \
                     and a spline needs the points to follow one another",
                    at(repeated)
                ));
            }
            spline_slopes(table.rows.len(), &at, &value, table.smoothness)
        }
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
        // Between two points the Akima spline is the cubic that starts
        // and ends where the table says and leaves each end at the
        // slope worked out above. Written in how far along the
        // interval the run is - `t` from nothing to one - it is
        // `y0 + (m0*t + (3*rise - 2*m0 - m1)*t^2 + (m0 + m1 - 2*rise)*t^3) * run`,
        // and its slope is that differentiated by `u`.
        if is_a_spline(table.smoothness) {
            let (m0, m1) = (akima[first], akima[last]);
            let (a, b) = (3.0 * rise - 2.0 * m0 - m1, m0 + m1 - 2.0 * rise);
            let t = Expr::Bin(
                BinOp::Div,
                Box::new(Expr::Bin(
                    BinOp::Sub,
                    Box::new(u.clone()),
                    Box::new(Expr::Number(at(first))),
                )),
                Box::new(Expr::Number(run)),
            );
            let times =
                |k: f64, of: Expr| Expr::Bin(BinOp::Mul, Box::new(Expr::Number(k)), Box::new(of));
            let plus = |l: Expr, r: Expr| Expr::Bin(BinOp::Add, Box::new(l), Box::new(r));
            let squared = Expr::Bin(BinOp::Mul, Box::new(t.clone()), Box::new(t.clone()));
            let cubed = Expr::Bin(BinOp::Mul, Box::new(squared.clone()), Box::new(t.clone()));
            return match slope {
                // `m0 + 2*a*t + 3*b*t^2`: the cubic differentiated by
                // `u`, where the `run` of the chain rule cancels the
                // one the value is multiplied by.
                true => plus(
                    plus(Expr::Number(m0), times(2.0 * a, t)),
                    times(3.0 * b, squared),
                ),
                false => plus(
                    Expr::Number(value(first)),
                    times(
                        run,
                        plus(plus(times(m0, t), times(a, squared)), times(b, cubed)),
                    ),
                ),
            };
        }
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
