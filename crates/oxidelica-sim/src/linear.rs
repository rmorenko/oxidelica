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
