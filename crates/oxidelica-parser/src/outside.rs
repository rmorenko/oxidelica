//! Bodies written outside Modelica, written again here in Rust.
//!
//! `external "C" ModelicaRandom_xorshift64star(stateIn, stateOut,
//! result)` is a name and a shape - what to call, what to hand it, what
//! comes back - and says nothing about who answers. This module is one
//! of the two who may: the answers written here, compiled in and the
//! same on every machine.
//!
//! What is here is numbers in and numbers out, which is why it sits
//! where both the compiler and the run can reach it. Flattening folds a
//! call whose arguments are all settled - the standard library builds a
//! generator's first state by drawing ten numbers from a seed, and all
//! ten fold - and the run answers the rest.
//!
//! The answer is the function's outputs in the order Modelica declared
//! them, written out flat: `random` answers three numbers, the value
//! first and then the two halves of the new state.
//!
//! [`EXTERNAL.md`](../../../docs/EXTERNAL.md) says how this is meant to
//! grow, and why the sandbox that will run the rest is not this.

/// Whether a body of this name is one written here.
pub fn written_here(called: &str) -> bool {
    matches!(called, "ModelicaRandom_xorshift64star" | "dgesv")
}

/// How many numbers a body of this name takes and how many it answers
/// with, given how many each of its arguments came to.
///
/// Some bodies work on whatever size they are handed - a system of
/// equations is as wide as its matrix - so the shape is a question
/// about the call rather than about the name alone. `None` where
/// nobody here answers for the name, or where what it was handed is
/// not a shape it works on.
pub fn shape(called: &str, given: &[usize]) -> Option<(usize, usize)> {
    let all: usize = given.iter().sum();
    match (called, given) {
        // Two halves of a state in; a value and the two halves of the
        // next state out. However the declaration grouped them, what
        // the body takes is the two numbers.
        ("ModelicaRandom_xorshift64star", _) if all == 2 => Some((2, 3)),
        // A square matrix and a right-hand side of its width; the
        // solution and word of whether there was one.
        ("dgesv", [square, width]) if *width * *width == *square && *width > 0 => {
            Some((all, width + 1))
        }
        _ => None,
    }
}

/// What a body of this name makes of these numbers.
///
/// `None` where nobody here answers for the name, or where it was
/// handed the wrong count - which the caller has already checked, and
/// which is a mistake in the compiler rather than in a model.
pub fn answer(called: &str, given: &[f64]) -> Option<Vec<f64>> {
    match (called, given) {
        ("ModelicaRandom_xorshift64star", [low, high]) => {
            let (state, value) = xorshift64star(halves_to_state(*low, *high));
            let (low, high) = state_to_halves(state);
            Some(vec![value, low, high])
        }
        // The matrix comes row by row and the right-hand side after
        // it, which is how a value written out arrives.
        ("dgesv", _) => {
            let width = square_and_side(given.len())?;
            let mut answer = solve(&given[..given.len() - width], &given[given.len() - width..])?;
            // Whether there was a solution, the way LAPACK says it: a
            // zero where all went well.
            answer.push(0.0);
            Some(answer)
        }
        _ => None,
    }
}

/// The width of a square matrix followed by a right-hand side of that
/// width, from how many numbers there are in all.
fn square_and_side(all: usize) -> Option<usize> {
    (1..=all).find(|width| width * width + width == all)
}

/// Solve `A x = b` for `x`, the matrix given row by row.
///
/// Gaussian elimination with partial pivoting, which is what LAPACK's
/// `dgesv` does: the largest remaining entry of a column is brought to
/// the diagonal, the rows below it are cleared, and the answer is read
/// back from the bottom up. `None` where the matrix turns out to be
/// singular, which is what a zero pivot means.
pub fn solve(matrix: &[f64], side: &[f64]) -> Option<Vec<f64>> {
    let width = side.len();
    if width == 0 {
        return Some(Vec::new());
    }
    let mut rows: Vec<Vec<f64>> = matrix.chunks(width).map(<[f64]>::to_vec).collect();
    let mut side = side.to_vec();
    for step in 0..width {
        // The largest entry of this column, so that dividing by it
        // loses as little as it can.
        let pivot = (step..width).max_by(|a, b| {
            rows[*a][step]
                .abs()
                .partial_cmp(&rows[*b][step].abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        if rows[pivot][step] == 0.0 {
            return None;
        }
        rows.swap(step, pivot);
        side.swap(step, pivot);
        for below in step + 1..width {
            let share = rows[below][step] / rows[step][step];
            if share == 0.0 {
                continue;
            }
            let cleared: Vec<f64> = rows[step][step..].iter().map(|at| share * at).collect();
            for (cell, taken) in rows[below][step..].iter_mut().zip(cleared) {
                *cell -= taken;
            }
            side[below] -= share * side[step];
        }
    }
    let mut answer = vec![0.0; width];
    for step in (0..width).rev() {
        let known: f64 = (step + 1..width).map(|c| rows[step][c] * answer[c]).sum();
        answer[step] = (side[step] - known) / rows[step][step];
    }
    Some(answer)
}

/// One round of the xorshift64\* generator: the state it moves to, and
/// the number it draws.
///
/// Three shifts and three exclusive-ors move the state; the number is
/// the state times a fixed odd multiplier, scaled into `(0, 1]`. The
/// algorithm is Vigna's and is published; the constants below are the
/// ones the standard library's C uses, so a model gets the same stream
/// here as it would there.
pub fn xorshift64star(state: u64) -> (u64, f64) {
    let mut state = state;
    state ^= state >> 12;
    state ^= state << 25;
    state ^= state >> 27;
    let drawn = state.wrapping_mul(2685821657736338717);
    (state, drawn as f64 * 5.421_010_862_427_522e-20)
}

/// The two halves Modelica carries a state in, put back together.
///
/// A state is 64 bits and an `Integer` is 32, so the standard library
/// keeps one as two of them - the low half first, as a little-endian
/// machine lays them out, which is what its C reads. Each half arrives
/// as a number because that is all the run carries, and a 32-bit whole
/// number is exact in one.
fn halves_to_state(low: f64, high: f64) -> u64 {
    let half = |value: f64| (value as i64 as u32) as u64;
    half(low) | (half(high) << 32)
}

/// The same, taken apart again, each half signed the way an `Integer`
/// is.
fn state_to_halves(state: u64) -> (f64, f64) {
    let half = |value: u64| ((value & 0xffff_ffff) as u32 as i32) as f64;
    (half(state), half(state >> 32))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stream the algorithm draws, checked against what its own
    /// definition gives step by step. A generator is only useful if
    /// every tool draws the same numbers from the same seed, so this
    /// is the one place where copying the constants is the point.
    #[test]
    fn a_state_moves_the_way_the_algorithm_says() {
        // Worked by hand from the definition: 1 shifted and xored
        // three times.
        let mut state: u64 = 1;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let (moved, drawn) = xorshift64star(1);
        assert_eq!(moved, state);
        assert!(drawn > 0.0 && drawn <= 1.0, "{drawn}");

        // A state of zero stays zero, which is why the standard
        // library refuses a seed of zero and uses a prime instead.
        assert_eq!(xorshift64star(0), (0, 0.0));
    }

    #[test]
    fn a_state_survives_being_carried_as_two_integers() {
        for state in [1u64, 0, u64::MAX, 1 << 63, 0x1234_5678_9abc_def0] {
            let (low, high) = state_to_halves(state);
            assert_eq!(halves_to_state(low, high), state, "{state}");
            // Each half is a whole number an `Integer` could hold.
            for half in [low, high] {
                assert!(half.fract() == 0.0 && half.abs() <= 2147483648.0, "{half}");
            }
        }
    }

    /// A system whose answer is known by putting it back in, and one
    /// that has no answer at all.
    #[test]
    fn a_system_is_solved_by_clearing_it_column_by_column() {
        // 2x + y = 3, x + 3y = 5 has x = 0.8, y = 1.4.
        let answer = solve(&[2.0, 1.0, 1.0, 3.0], &[3.0, 5.0]).unwrap();
        assert!((answer[0] - 0.8).abs() < 1e-12, "{answer:?}");
        assert!((answer[1] - 1.4).abs() < 1e-12, "{answer:?}");

        // A pivot that has to be swapped in from below: the first
        // column starts at zero.
        let swapped = solve(&[0.0, 1.0, 1.0, 0.0], &[2.0, 3.0]).unwrap();
        assert_eq!(swapped, vec![3.0, 2.0]);

        // One equation, and a bigger one checked by putting the answer
        // back where it came from.
        assert_eq!(solve(&[4.0], &[8.0]), Some(vec![2.0]));
        let rows = [2.0, 1.0, -1.0, -3.0, -1.0, 2.0, -2.0, 1.0, 2.0];
        let side = [8.0, -11.0, -3.0];
        let answer = solve(&rows, &side).unwrap();
        for (row, wanted) in rows.chunks(3).zip(side) {
            let got: f64 = row.iter().zip(&answer).map(|(a, x)| a * x).sum();
            assert!((got - wanted).abs() < 1e-12, "{got} against {wanted}");
        }

        // Two equations saying the same thing have no one answer.
        assert_eq!(solve(&[1.0, 2.0, 2.0, 4.0], &[3.0, 6.0]), None);
        // Nothing to solve is nothing to answer with.
        assert_eq!(solve(&[], &[]), Some(Vec::new()));

        // Through the name it is called by outside: the matrix row by
        // row, the side after it, and word of how it went at the end.
        let told = super::answer("dgesv", &[2.0, 1.0, 1.0, 3.0, 3.0, 5.0]).unwrap();
        assert_eq!(told.len(), 3);
        assert_eq!(told[2], 0.0);
        assert!((told[0] - 0.8).abs() < 1e-12, "{told:?}");
        assert_eq!(shape("dgesv", &[4, 2]), Some((6, 3)));
        assert_eq!(shape("dgesv", &[4, 3]), None);
    }

    #[test]
    fn the_name_is_what_says_who_answers() {
        assert!(written_here("ModelicaRandom_xorshift64star"));
        // However the two numbers were grouped.
        assert_eq!(shape("ModelicaRandom_xorshift64star", &[2]), Some((2, 3)));
        assert_eq!(
            shape("ModelicaRandom_xorshift64star", &[1, 1]),
            Some((2, 3))
        );
        assert_eq!(shape("ModelicaRandom_xorshift64star", &[3]), None);
        assert!(!written_here("ModelicaRandom_xorshift1024star"));
        assert!(answer("ModelicaRandom_xorshift1024star", &[1.0]).is_none());
        // The right name handed the wrong count answers nothing.
        assert!(answer("ModelicaRandom_xorshift64star", &[1.0]).is_none());

        let drawn = answer("ModelicaRandom_xorshift64star", &[126247697.0, 0.0]).unwrap();
        assert_eq!(drawn.len(), 3);
        assert!(drawn[0] > 0.0 && drawn[0] <= 1.0, "{drawn:?}");
    }
}
