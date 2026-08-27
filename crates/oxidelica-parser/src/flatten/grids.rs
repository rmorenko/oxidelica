//! Tables written as a grid: a second abscissa along the top row, a
//! first down the left column, and a value at every crossing.
//!
//! What the standard library calls a `CombiTable2D`. The
//! one-dimensional kind, whose rows are an abscissa and its outputs,
//! is next door in `tables`.

use super::tables::*;
use super::*;

/// The Hermite basis written in powers of the fraction: row `a` is
/// how basis function `a` is made of `1`, `t`, `t^2`, `t^3`.
///
/// The four are `h00 = 1 - 3t^2 + 2t^3`, `h10 = t - 2t^2 + t^3`,
/// `h01 = 3t^2 - 2t^3` and `h11 = -t^2 + t^3`: the first pair carry
/// the values at the ends of an interval, the second pair the slopes.
const HERMITE: [[f64; 4]; 4] = [
    [1.0, 0.0, -3.0, 2.0],
    [0.0, 1.0, -2.0, 1.0],
    [0.0, 0.0, 3.0, -2.0],
    [0.0, 0.0, -1.0, 1.0],
];

/// The slopes an Akima spline gives a grid, along each abscissa and
/// across both.
///
/// A two-dimensional table is splined the way the standard tables do
/// it: Akima's rule along one abscissa, then along the other. What
/// comes out is a slope down, a slope across and a cross slope at
/// every crossing of the grid, which is what a bicubic needs to be
/// drawn through them.
struct GridSlopes {
    down: Vec<Vec<f64>>,
    across: Vec<Vec<f64>>,
    corner: Vec<Vec<f64>>,
}

fn grid_slopes(down: &[f64], across: &[f64], cell: &dyn Fn(usize, usize) -> f64) -> GridSlopes {
    let (rows, columns) = (down.len(), across.len());
    // Along the first abscissa, one column at a time.
    let mut slope_down = vec![vec![0.0; columns]; rows];
    for j in 0..columns {
        let at = |i: usize| down[i];
        let value = |i: usize| cell(i, j);
        let along = akima_slopes(rows, &at, &value);
        for (row, m) in slope_down.iter_mut().zip(along) {
            row[j] = m;
        }
    }
    // Along the second, one row at a time.
    let mut slope_across = vec![vec![0.0; columns]; rows];
    for (i, row) in slope_across.iter_mut().enumerate() {
        let at = |j: usize| across[j];
        let value = |j: usize| cell(i, j);
        for (j, m) in akima_slopes(columns, &at, &value).into_iter().enumerate() {
            row[j] = m;
        }
    }
    // Across both: the slope down, splined along the second abscissa.
    // Which of the two orders it is worked out in makes no difference
    // to a grid whose points follow one another.
    let mut corner = vec![vec![0.0; columns]; rows];
    for (i, row) in corner.iter_mut().enumerate() {
        let at = |j: usize| across[j];
        let value = |j: usize| slope_down[i][j];
        for (j, m) in akima_slopes(columns, &at, &value).into_iter().enumerate() {
            row[j] = m;
        }
    }
    GridSlopes {
        down: slope_down,
        across: slope_across,
        corner,
    }
}

/// One cell of a splined grid, written in powers of how far into it
/// the point is: `C[p][q]` multiplies `s^p * t^q`.
///
/// The cell is the bicubic that meets the four corners at the values
/// the table gives and leaves each corner at the slopes worked out
/// above. In the Hermite basis that is `basis(s) * M * basis(t)`,
/// where `M` holds the corner values and their slopes; written in
/// powers it is `H' * M * H`, which is what this comes to.
fn cell_powers(
    slopes: &GridSlopes,
    cell: &dyn Fn(usize, usize) -> f64,
    i: usize,
    j: usize,
    span_down: f64,
    span_across: f64,
) -> [[f64; 4]; 4] {
    // The far corner of the cell, held at the edge of the grid: an
    // abscissa of one point has no interval along it, and all four
    // corners are then that one point.
    let (rows, columns) = (slopes.down.len(), slopes.down[0].len());
    let (i, j) = (i.min(rows - 1), j.min(columns - 1));
    let (i1, j1) = ((i + 1).min(rows - 1), (j + 1).min(columns - 1));
    // Values and slopes at the four corners. A slope is written in
    // the fraction rather than in the abscissa, so it is multiplied
    // by how wide the interval is.
    let m = [
        [
            cell(i, j),
            slopes.across[i][j] * span_across,
            cell(i, j1),
            slopes.across[i][j1] * span_across,
        ],
        [
            slopes.down[i][j] * span_down,
            slopes.corner[i][j] * span_down * span_across,
            slopes.down[i][j1] * span_down,
            slopes.corner[i][j1] * span_down * span_across,
        ],
        [
            cell(i1, j),
            slopes.across[i1][j] * span_across,
            cell(i1, j1),
            slopes.across[i1][j1] * span_across,
        ],
        [
            slopes.down[i1][j] * span_down,
            slopes.corner[i1][j] * span_down * span_across,
            slopes.down[i1][j1] * span_down,
            slopes.corner[i1][j1] * span_down * span_across,
        ],
    ];
    let mut powers = [[0.0f64; 4]; 4];
    for (p, row) in powers.iter_mut().enumerate() {
        for (q, out) in row.iter_mut().enumerate() {
            for (a, m_row) in m.iter().enumerate() {
                for (b, value) in m_row.iter().enumerate() {
                    *out += HERMITE[a][p] * value * HERMITE[b][q];
                }
            }
        }
    }
    powers
}

/// A cell of powers written out as an expression in the two
/// fractions, as a value or as a slope along one abscissa.
///
/// The slope is the polynomial differentiated by the fraction and
/// divided by how wide the interval is, which is the chain rule
/// written once.
fn powers_written(
    powers: &[[f64; 4]; 4],
    s: &Expr,
    t: &Expr,
    wanted: Wanted,
    span_down: f64,
    span_across: f64,
) -> Expr {
    let raised = |base: &Expr, power: usize| -> Option<Expr> {
        match power {
            0 => None,
            _ => {
                let mut out = base.clone();
                for _ in 1..power {
                    out = Expr::Bin(BinOp::Mul, Box::new(out), Box::new(base.clone()));
                }
                Some(out)
            }
        }
    };
    let mut written: Option<Expr> = None;
    for (p, row) in powers.iter().enumerate() {
        for (q, coefficient) in row.iter().enumerate() {
            // What the differentiation does to one term: `s^p`
            // becomes `p * s^(p-1)`, and a term it wipes out is left
            // out of the sum entirely.
            let (factor, p, q) = match wanted {
                Wanted::Value => (1.0, p, q),
                Wanted::SlopeDown if p == 0 => continue,
                Wanted::SlopeDown => (p as f64 / span_down, p - 1, q),
                Wanted::SlopeAcross if q == 0 => continue,
                Wanted::SlopeAcross => (q as f64 / span_across, p, q - 1),
            };
            let coefficient = coefficient * factor;
            if coefficient == 0.0 {
                continue;
            }
            let mut term = Expr::Number(coefficient);
            for part in [raised(s, p), raised(t, q)].into_iter().flatten() {
                term = Expr::Bin(BinOp::Mul, Box::new(term), Box::new(part));
            }
            written = Some(match written {
                None => term,
                Some(so_far) => Expr::Bin(BinOp::Add, Box::new(so_far), Box::new(term)),
            });
        }
    }
    written.unwrap_or(Expr::Number(0.0))
}

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
    if !matches!(
        table.smoothness,
        LINEAR_SEGMENTS | CONSTANT_SEGMENTS | AKIMA_SPLINE
    ) {
        return Err(format!(
            "a two-dimensional table asks for smoothness {}, and this compiler writes out the \
             linear, the constant and the Akima spline only",
            table.smoothness
        ));
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
    // An abscissa of one point is read at that point however far the
    // index reaches: a cell of the grid is drawn from four corners,
    // and where there is only one row to take them from, all four
    // come from it.
    let cell = |i: usize, j: usize| {
        let i = i.min(table.rows.len() - 2);
        let j = j.min(table.rows[0].len() - 2);
        table.rows[1 + i][1 + j]
    };
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
    // The slopes a spline gives the grid, worked out once for the
    // whole table. A spline needs the points to follow one another
    // along both abscissas: two rows at the same place leave the line
    // between them undefined rather than flat, and a slope of zero
    // there would be a quiet mistake rather than an answer.
    let splined = match table.smoothness == AKIMA_SPLINE {
        false => None,
        true => {
            for (axis, which) in [(&down, "first"), (&across, "second")] {
                if let Some(repeated) = (1..axis.len()).find(|&k| axis[k] == axis[k - 1]) {
                    return Err(format!(
                        "a two-dimensional table asked for an Akima spline gives {} twice on \
                         its {which} abscissa, and a spline needs the points to follow one \
                         another",
                        axis[repeated]
                    ));
                }
            }
            Some(grid_slopes(&down, &across, &cell))
        }
    };
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
        // A splined cell is a bicubic rather than a plane: the four
        // corners, the slopes along each abscissa there and the cross
        // slope, written in powers of the two fractions.
        if let Some(slopes) = &splined {
            let (span_down, span_across) = (
                match down.len() > 1 {
                    true => down[i + 1] - down[i],
                    false => 1.0,
                },
                match across.len() > 1 {
                    true => across[j + 1] - across[j],
                    false => 1.0,
                },
            );
            // An abscissa of one point has no interval along it, so
            // the cell is drawn as if the grid repeated there and the
            // fraction stays at nothing.
            let (last_i, last_j) = (
                match down.len() > 1 {
                    true => i,
                    false => 0,
                },
                match across.len() > 1 {
                    true => j,
                    false => 0,
                },
            );
            let powers = match (down.len() > 1, across.len() > 1) {
                (true, true) => cell_powers(slopes, &cell, i, j, span_down, span_across),
                _ => {
                    // With one row or one column there is no bicubic
                    // to draw: what the table says along the abscissa
                    // it does have is a cubic, and the other fraction
                    // multiplies nothing.
                    let mut powers = [[0.0f64; 4]; 4];
                    let held = cell_powers(
                        slopes,
                        &|a: usize, b: usize| cell(a.min(last_i + 1), b.min(last_j + 1)),
                        last_i,
                        last_j,
                        span_down,
                        span_across,
                    );
                    powers.copy_from_slice(&held);
                    powers
                }
            };
            return powers_written(&powers, &s, &t, wanted, span_down, span_across);
        }
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
