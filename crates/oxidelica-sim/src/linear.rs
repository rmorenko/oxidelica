//! Small dense and banded linear algebra, and the Lagrange
//! interpolation the multistep method is written with.

/// Derivatives of the Lagrange basis polynomials at the first node.
///
/// For nodes `[t0, t1, ...]` the result `c` satisfies
/// `P'(t0) = sum_j c[j] * y_j` for the interpolant `P` through them —
/// exactly the coefficients of a non-uniform BDF formula.
pub(crate) fn lagrange_derivative_coefficients(nodes: &[f64]) -> Vec<f64> {
    let count = nodes.len();
    let mut coefficients = vec![0.0; count];
    // j = 0: sum of reciprocal distances to the other nodes.
    coefficients[0] = nodes[1..].iter().map(|t| 1.0 / (nodes[0] - t)).sum();
    for j in 1..count {
        let mut numerator = 1.0;
        for (m, node) in nodes.iter().enumerate() {
            if m != j && m != 0 {
                numerator *= nodes[0] - node;
            }
        }
        let mut denominator = 1.0;
        for (m, node) in nodes.iter().enumerate() {
            if m != j {
                denominator *= nodes[j] - node;
            }
        }
        coefficients[j] = numerator / denominator;
    }
    coefficients
}

/// Value at `at` of the Lagrange interpolant through `nodes` for
/// component `i` of the stored vectors.
pub(crate) fn lagrange_value(nodes: &[f64], values: &[&[f64]], i: usize, at: f64) -> f64 {
    let mut sum = 0.0;
    for (j, &node) in nodes.iter().enumerate() {
        let mut basis = 1.0;
        for (m, &other) in nodes.iter().enumerate() {
            if m != j {
                basis *= (at - other) / (node - other);
            }
        }
        sum += basis * values[j][i];
    }
    sum
}

/// Extrapolate the history polynomial (component `i`) to `at`.
pub(crate) fn lagrange_extrapolate(nodes: &[f64], values: &[Vec<f64>], i: usize, at: f64) -> f64 {
    let borrowed: Vec<&[f64]> = values.iter().map(|v| v.as_slice()).collect();
    lagrange_value(nodes, &borrowed, i, at)
}

/// Solve `a * x = b` in place by Gaussian elimination with partial
/// pivoting; `None` on a (numerically) singular matrix.
/// Solve a banded system by elimination without pivoting.
///
/// `matrix[i][j - i + band]` holds the entry at row `i`, column `j`, so
/// each row is `2 * band + 1` wide. Skipping the pivot search is what
/// keeps the band narrow, and it is sound for the matrices this is used
/// on: `I - h*c*J` of a diffusion-like system has a diagonal that
/// dominates. A pivot that turns out too small to trust returns `None`
/// and the caller falls back to the dense path.
pub(crate) fn solve_banded(matrix: &mut [Vec<f64>], band: usize, rhs: &[f64]) -> Option<Vec<f64>> {
    let n = rhs.len();
    let width = 2 * band + 1;
    let mut x = rhs.to_vec();
    for i in 0..n {
        let pivot = matrix[i][band];
        if pivot.abs() < 1e-12 {
            return None;
        }
        for r in (i + 1)..(i + band + 1).min(n) {
            let offset = i + band - r;
            let factor = matrix[r][offset] / pivot;
            if factor == 0.0 {
                continue;
            }
            for column in i..(i + band + 1).min(n) {
                let source = matrix[i][column + band - i];
                let target = column + band - r;
                if target < width {
                    matrix[r][target] -= factor * source;
                }
            }
            x[r] -= factor * x[i];
        }
    }
    for i in (0..n).rev() {
        let mut sum = x[i];
        for column in (i + 1)..(i + band + 1).min(n) {
            sum -= matrix[i][column + band - i] * x[column];
        }
        x[i] = sum / matrix[i][band];
    }
    if x.iter().any(|value| !value.is_finite()) {
        return None;
    }
    Some(x)
}

/// Each row divided by its own largest entry, so that a pivot can be
/// judged against the number one rather than against whichever
/// equation of the block happens to carry the largest coefficient.
///
/// Whether a block has one solution does not depend on the units its
/// equations are written in, and dividing a row through is exactly a
/// change of those units - the same equation, stated per unit instead
/// of per thousand. The pivot test compares against the whole
/// matrix's largest entry, though, and that comparison does depend on
/// them: a magnetic circuit carries a permeance near `mu_0` beside a
/// reluctance near its reciprocal in one block by construction, so its
/// Jacobian spans eleven decades while being perfectly invertible, and
/// read unscaled every such block was called underdetermined. Scaled
/// by rows, the quadratic core's block has a smallest singular value
/// of 1.7e-5 against a largest of 2.1 - ill-conditioned, which is a
/// thing a solver lives with, and not at all the same as having a
/// family of solutions.
///
/// The columns are deliberately left alone, and so is the matrix of
/// one row, and the tests are what said so rather than an argument.
/// A column belongs to an unknown, and in exact arithmetic scaling one
/// would be as defensible - but this Jacobian is built by finite
/// differences, so the column of an unknown the residual does not
/// really depend on is not zero, it is noise near 1e-8. Divided by its
/// own largest entry that noise becomes a coefficient of one, and
/// `x = y + 1` beside `y = x - 1` - the same equation twice, which is
/// the thing this check exists to catch - comes back invertible.
/// The single row is the other end of it: scaled, its one entry is
/// always one, so no one-by-one block could ever read singular again,
/// and `1/x = 0` would be answered with a number instead of a
/// refusal. Below two rows there is no spread between equations to
/// take out, which is the whole point of scaling, so there is nothing
/// lost by leaving it.
pub(crate) fn equilibrate_rows(a: &mut [Vec<f64>]) {
    if a.len() < 2 {
        return;
    }
    for row in a.iter_mut() {
        let largest = row.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        // An all-zero row has no scale to be divided by, and it is
        // exactly the row the caller must go on calling singular.
        if largest > 0.0 {
            for value in row.iter_mut() {
                *value /= largest;
            }
        }
    }
}

/// The smallest pivot Gaussian elimination with partial pivoting meets
/// on this matrix.
///
/// `solve_linear` answers only "did it come apart", against a floor of
/// 1e-14, which is the right question for arithmetic that is exact but
/// the wrong one for a Jacobian built by finite differences: there, a
/// column that ought to cancel comes back as noise near 1e-8 and reads
/// as a live coefficient. The caller compares this against the
/// matrix's own scale, which is a question about the block rather than
/// about the floating-point format.
pub(crate) fn smallest_pivot(a: &mut [Vec<f64>]) -> f64 {
    let n = a.len();
    let mut smallest = f64::INFINITY;
    for col in 0..n {
        let Some(pivot_row) = (col..n).max_by(|&r1, &r2| {
            a[r1][col]
                .abs()
                .partial_cmp(&a[r2][col].abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        }) else {
            return 0.0;
        };
        let pivot = a[pivot_row][col].abs();
        smallest = smallest.min(pivot);
        if pivot == 0.0 {
            return 0.0;
        }
        a.swap(col, pivot_row);
        for row in (col + 1)..n {
            let factor = a[row][col] / a[col][col];
            let (upper, lower) = a.split_at_mut(row);
            for (k, value) in lower[0].iter_mut().enumerate().take(n).skip(col) {
                *value -= factor * upper[col][k];
            }
        }
    }
    smallest
}

/// A direction the matrix sends to zero, or `None` if there is none.
///
/// This is what a singular initialisation problem owes its reader.
/// The column elimination stops on is not a fact about the model: the
/// columns are walked in the order the unknowns happen to sit in the
/// vector, so a degeneracy spanning three states is blamed on
/// whichever of them was declared first, and reordering the
/// declarations moves the blame. The null direction is the whole
/// family, and every unknown with a component in it is unpinned.
///
/// Elimination runs as in `solve_linear`. Where a column has no pivot
/// left, that unknown is free: it is set to one and the leading
/// triangular block is solved backwards for the rest. Rows below the
/// failing column have zeros in every earlier column by elimination
/// and a zero in this one by assumption, so they are satisfied too.
pub(crate) fn null_direction(a: &mut [Vec<f64>]) -> Option<Vec<f64>> {
    let n = a.len();
    for col in 0..n {
        let pivot_row = (col..n).max_by(|&r1, &r2| {
            a[r1][col]
                .abs()
                .partial_cmp(&a[r2][col].abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        if a[pivot_row][col].abs() < 1e-14 {
            // This unknown is free. Back-substitute the triangular
            // block above it for what the other unknowns must be.
            let mut z = vec![0.0; n];
            z[col] = 1.0;
            for row in (0..col).rev() {
                let mut sum = a[row][col];
                for k in (row + 1)..col {
                    sum += a[row][k] * z[k];
                }
                z[row] = -sum / a[row][row];
            }
            return Some(z);
        }
        a.swap(col, pivot_row);
        for row in (col + 1)..n {
            let factor = a[row][col] / a[col][col];
            let (upper, lower) = a.split_at_mut(row);
            for (k, value) in lower[0].iter_mut().enumerate().take(n).skip(col) {
                *value -= factor * upper[col][k];
            }
        }
    }
    None
}

pub(crate) fn solve_linear(a: &mut [Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    let n = b.len();
    let mut x = b.to_vec();
    for col in 0..n {
        let pivot_row = (col..n).max_by(|&r1, &r2| {
            a[r1][col]
                .abs()
                .partial_cmp(&a[r2][col].abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        if a[pivot_row][col].abs() < 1e-14 {
            return None;
        }
        a.swap(col, pivot_row);
        x.swap(col, pivot_row);
        for row in (col + 1)..n {
            let factor = a[row][col] / a[col][col];
            let (upper, lower) = a.split_at_mut(row);
            for (k, value) in lower[0].iter_mut().enumerate().take(n).skip(col) {
                *value -= factor * upper[col][k];
            }
            x[row] -= factor * x[col];
        }
    }
    for col in (0..n).rev() {
        for k in (col + 1)..n {
            let prev = x[k];
            x[col] -= a[col][k] * prev;
        }
        x[col] /= a[col][col];
    }
    Some(x)
}
