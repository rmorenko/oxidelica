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
    answers(called).is_some()
}

/// How many numbers a body of this name answers with, and how many it
/// takes. `None` where nobody here answers for it.
pub fn answers(called: &str) -> Option<(usize, usize)> {
    match called {
        // Two halves of a state in; a value and the two halves of the
        // next state out.
        "ModelicaRandom_xorshift64star" => Some((2, 3)),
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
        _ => None,
    }
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

    #[test]
    fn the_name_is_what_says_who_answers() {
        assert!(written_here("ModelicaRandom_xorshift64star"));
        assert_eq!(answers("ModelicaRandom_xorshift64star"), Some((2, 3)));
        assert!(!written_here("ModelicaRandom_xorshift1024star"));
        assert!(answer("ModelicaRandom_xorshift1024star", &[1.0]).is_none());
        // The right name handed the wrong count answers nothing.
        assert!(answer("ModelicaRandom_xorshift64star", &[1.0]).is_none());

        let drawn = answer("ModelicaRandom_xorshift64star", &[126247697.0, 0.0]).unwrap();
        assert_eq!(drawn.len(), 3);
        assert!(drawn[0] > 0.0 && drawn[0] <= 1.0, "{drawn:?}");
    }
}
