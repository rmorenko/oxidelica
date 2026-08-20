//! Bodies written outside Modelica that this compiler answers for.
//!
//! `external "C" result = ModelicaStrings_length(string)` is a name and
//! a shape - what to call, what to hand it, what comes back - and says
//! nothing about who answers. This module is one of the two who may:
//! the answers written here, in Rust, compiled in and the same on every
//! machine. The other is the library's own C in a sandbox, which is not
//! here yet; where both could answer, this one wins.
//!
//! What a name here means is that the call is left standing rather than
//! refused, for whoever can work it out to work it out. A string is
//! settled at the end of flattening, so the string functions are
//! answered by the string layer; a number the run carries would be
//! answered by the walk. Nothing is answered twice.
//!
//! [`EXTERNAL.md`](../../../../docs/EXTERNAL.md) says how this is meant
//! to grow.

use super::*;

/// Whether this compiler answers for an outside name itself.
///
/// A function whose outside name is one of these is inlined as a call
/// to that name, which then stands until something works it out. Every
/// name here has an answer somewhere; adding one without the answer
/// would turn a clear refusal into a call that nothing resolves.
pub(super) fn answered_here(called: &str) -> bool {
    matches!(
        called,
        "ModelicaStrings_length" | "ModelicaStrings_substring" | "ModelicaStrings_compare"
    )
}

/// The substring of `text` from `first` to `last`, both counted from
/// one and both included.
///
/// The specification takes a start below one as one and an end past the
/// text as its end, each with a warning; an end before the start is the
/// empty string. Positions are counted in characters rather than bytes,
/// which is the same thing for everything the standard library asks
/// this of and the safer answer where it is not.
pub(super) fn substring(text: &str, first: f64, last: f64) -> String {
    let length = text.chars().count() as f64;
    let first = first.max(1.0);
    let last = last.min(length);
    if last < first {
        return String::new();
    }
    text.chars()
        .skip(first as usize - 1)
        .take((last - first) as usize + 1)
        .collect()
}

/// How two strings compare, as `Modelica.Utilities.Types.Compare`
/// numbers them: less is 1, equal 2, greater 3.
///
/// The comparison is of the characters in order, which is what C's
/// `strcmp` does to bytes and what Rust does to strings. Told to
/// ignore case, both sides are lowered first.
pub(super) fn compare(left: &str, right: &str, case_matters: bool) -> f64 {
    let (left, right) = match case_matters {
        true => (left.to_string(), right.to_string()),
        false => (left.to_lowercase(), right.to_lowercase()),
    };
    match left.cmp(&right) {
        std::cmp::Ordering::Less => 1.0,
        std::cmp::Ordering::Equal => 2.0,
        std::cmp::Ordering::Greater => 3.0,
    }
}

/// What an outside call this compiler answers for comes to, where its
/// arguments are strings that are settled and its answer is a number.
///
/// `None` where the name is not one of these or an argument is not
/// settled yet - the call stands, and either the string layer answers
/// it or the refusal comes later, by name.
pub(super) fn number_of(
    name: &str,
    args: &[Expr],
    values: &HashMap<String, String>,
    numbers: &HashMap<String, f64>,
) -> Option<f64> {
    let text = |position: usize| strings::text_of(args.get(position)?, values, numbers);
    match (name, args.len()) {
        ("ModelicaStrings_length", 1) => Some(text(0)?.chars().count() as f64),
        ("ModelicaStrings_compare", 3) => {
            let case_matters = const_eval(&args[2], numbers)? != 0.0;
            Some(compare(&text(0)?, &text(1)?, case_matters))
        }
        _ => None,
    }
}
