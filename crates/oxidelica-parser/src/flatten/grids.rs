//! Tables written as a grid: a second abscissa along the top row, a
//! first down the left column, and a value at every crossing.
//!
//! What the standard library calls a `CombiTable2D`. The
//! one-dimensional kind, whose rows are an abscissa and its outputs,
//! is next door in `tables`.

use super::tables::*;
use super::*;

/// What is wanted of a two-dimensional table at a point: the value
/// there, or how fast it changes along one abscissa or the other.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Wanted {
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
pub(super) fn on_the_grid(
    table: &Table,
    u1: &Expr,
    u2: &Expr,
    wanted: Wanted,
) -> Result<Expr, String> {
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
