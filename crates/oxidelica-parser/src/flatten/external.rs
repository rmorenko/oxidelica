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
        "ModelicaStrings_length"
            | "ModelicaStrings_hashString"
            | "ModelicaStrings_substring"
            | "ModelicaStrings_compare"
            | "ModelicaStrings_skipWhiteSpace"
            | "ModelicaStandardTables_CombiTable1D_minimumAbscissa"
            | "ModelicaStandardTables_CombiTable1D_maximumAbscissa"
            | "ModelicaStandardTables_CombiTable1D_getValue"
            | "ModelicaStandardTables_CombiTable1D_getDerValue"
            | "ModelicaStandardTables_CombiTimeTable_minimumTime"
            | "ModelicaStandardTables_CombiTimeTable_maximumTime"
            | "ModelicaStandardTables_CombiTimeTable_getValue"
            | "ModelicaStandardTables_CombiTimeTable_getDerValue"
            | "ModelicaStandardTables_CombiTimeTable_nextTimeEvent"
            | "ModelicaStandardTables_CombiTable2D_minimumAbscissa"
            | "ModelicaStandardTables_CombiTable2D_maximumAbscissa"
            | "ModelicaStandardTables_CombiTable2D_getValue"
            | "ModelicaStandardTables_CombiTable2D_getDerValue"
    )
}

/// Refuse anything this compiler said it answers for and then did not.
///
/// A name in `answered_here` is not refused where the call is made, on
/// the promise that something further on works it out. Where nothing
/// did - a table whose data is in a file rather than in the model -
/// the promise was not kept, and a call standing in a flat model with
/// nobody to run it would be worse than a refusal.
pub(super) fn nothing_left_unanswered(model: &Model) -> Result<(), String> {
    let mut standing = None;
    let mut look = |expr: &Expr| {
        walk_calls(expr, &mut |name| {
            if answered_here(name) && standing.is_none() {
                standing = Some(name.to_string());
            }
        });
    };
    for equation in model.equations.iter().chain(model.initial_equations.iter()) {
        look(&equation.lhs);
        look(&equation.rhs);
    }
    for component in &model.components {
        for said in [&component.binding, &component.start].into_iter().flatten() {
            look(said);
        }
    }
    for (condition, _) in &model.asserts {
        look(condition);
    }
    match standing {
        None => Ok(()),
        Some(name) => Err(format!(
            "`{name}` is written outside Modelica. This compiler answers for that name \
             where what it needs is in the model - a table written as a matrix, a \
             string it can read - and this call is not one of those"
        )),
    }
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
        // Where the white space at `startIndex` ends, counted from
        // one; past the end of the text where there is nothing else.
        // The second argument says where to start, and the standard
        // library's declaration gives it a default of one - a call
        // that leaves it out arrives here with one argument.
        ("ModelicaStrings_skipWhiteSpace", 1 | 2) => {
            let text = text(0)?;
            let from = match args.get(1) {
                None => 1.0,
                Some(given) => const_eval(given, numbers)?,
            }
            .max(1.0) as usize;
            let past = text
                .chars()
                .skip(from - 1)
                .take_while(|letter| letter.is_whitespace())
                .count();
            Some((from + past) as f64)
        }
        ("ModelicaStrings_compare", 3) => {
            let case_matters = const_eval(&args[2], numbers)? != 0.0;
            Some(compare(&text(0)?, &text(1)?, case_matters))
        }
        ("ModelicaStrings_hashString", 1) => Some(hash_string(&text(0)?)),
        _ => None,
    }
}

/// A hash of a string, the number the standard library's own C says.
///
/// The noise blocks seed themselves from where they sit in the model -
/// `automaticLocalSeed(getInstanceName())` - so two blocks of the same
/// model draw different numbers, and the same model run twice draws
/// the same ones. That only holds if this answers what the library's C
/// answers, so this is that code and not a hash of one's own choosing:
/// Arash Partow's, as uthash spells it, over the bytes of the text,
/// with the result read back as a signed integer.
fn hash_string(text: &str) -> f64 {
    let mut hash: u32 = 0xAAAA_AAAA;
    for (at, byte) in text.bytes().enumerate() {
        hash ^= match at % 2 == 0 {
            true => (hash << 7) ^ u32::from(byte).wrapping_mul(hash >> 3),
            false => !((hash << 11).wrapping_add(u32::from(byte) ^ (hash >> 5))),
        };
    }
    f64::from(hash as i32)
}

/// The file a `modelica://Library/path` URI names, where it is in
/// view.
///
/// A resource belongs to the library it is written under and sits
/// beside that library's own files, so the directories the compiler
/// reads libraries from are where it is looked for. The library part
/// of the URI may be a class inside one - `modelica://Modelica.Blocks/
/// x.txt` is a file of the Modelica library - so it is the first
/// segment that names the directory.
///
/// `None` where the file is not there, which is what a file name that
/// cannot be settled already means: the model says what it wanted and
/// nothing pretends to have read it.
pub(super) fn resource_named(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("modelica://")?;
    let (named, path) = rest.split_once('/')?;
    let library = named.split('.').next()?;
    for directory in crate::library::library_directories(None) {
        // A library is a directory of its own name, or a file beside
        // one - the resources are under the directory either way.
        for root in [directory.join(library), directory.clone()] {
            let candidate = root.join(path);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}
