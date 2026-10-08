//! A constraint on states that no single equation states, found among
//! the linear equations the matching has already paired.
//!
//! A matching sees which names an equation mentions and nothing of the
//! numbers it mentions them with. A five-phase winding whose stray
//! inductance is all common has five phase currents and two space
//! phasors that are states, so three current components are left for
//! the zero inductor's `m*i0 = sum(i)` and for the star's `sum(i) = 0`
//! to share. The matching hands the first to an algebraic phase
//! current, the star's balance takes another, every equation has an
//! unknown, and index reduction is never called. But the two equations
//! added together say `m*i0 = 0`: the star forbids a zero-sequence
//! current, `i0` is a state, and nothing in the structure shows it.
//! What shows it is a block whose Jacobian has a row of zeros, and the
//! run is refused there with the star's potential free.
//!
//! The arithmetic that shows it is exact. Only equations whose
//! unknowns stand with integer coefficients take part - connection
//! sets, current balances, `m*i0 = sum(i)` - and they are eliminated in
//! integers, so a dependency is a fact about the equations and not a
//! residue below some tolerance. An equation with any other coefficient
//! simply takes no part, which can only miss a constraint, never
//! invent one.

use crate::*;
use std::collections::{BTreeMap, HashSet};

/// What one equation is, read as an integer combination of names.
struct LinearRow {
    /// The unknowns and the states the equation holds linearly, by
    /// column, each with its integer coefficient.
    columns: BTreeMap<usize, i128>,
    /// Whatever names no unknown: constants, parameters, functions of
    /// time and of states. Kept whole and scaled.
    rest: Vec<(i128, Expr)>,
}

/// A row of the echelon form: the column it pivots on, its values over
/// the unknowns, and the combination of equations it is.
type Pivot = (usize, BTreeMap<usize, i128>, BTreeMap<usize, i128>);

/// The phase has a size of its own: elimination can fill in, and a
/// model with thousands of connection equations is not one that should
/// pay for a search it did not need. Past this many coefficient
/// updates the search gives up and the model is matched as before.
const MAX_ELIMINATION_WORK: usize = 4_000_000;

/// Whether the search for a constraint hidden among linear equations
/// runs. On by default; `OXIDELICA_NO_HIDDEN_CONSTRAINT=1` matches as
/// before, so that one binary can produce both numbers.
pub(crate) fn hidden_constraint_enabled() -> bool {
    std::env::var("OXIDELICA_NO_HIDDEN_CONSTRAINT").as_deref() != Ok("1")
}

/// An integer, if the expression names nothing but parameters and
/// numbers and comes to a whole number that fits and is not zero.
///
/// Zero is refused rather than read. A term scaled by a parameter
/// worth zero stands in the structure - `R.T[2,1]*r[2]` with
/// `r = {0.4, 0, 0}` is how a MultiBody gear's orientation is read -
/// and a search that let the zero take the term out would find
/// constraints the rest of the compiler does not see, which is the
/// quench this project already measured and turned down.
fn integer_of(expr: &Expr, params: &HashMap<String, f64>) -> Option<i128> {
    let mut refs = Vec::new();
    expr.collect_refs(&mut refs);
    let mut folded = expr.clone();
    for name in refs {
        folded = substitute(&folded, name, *params.get(name)?);
    }
    match simplify(&folded) {
        Expr::Number(n) if n != 0.0 && n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 => {
            Some(n as i128)
        }
        _ => None,
    }
}

/// Read `expr * scale` into a row, or say it is not linear in the
/// unknowns with integer coefficients.
fn read_terms(
    expr: &Expr,
    scale: i128,
    column_of: &HashMap<&str, usize>,
    unknown: &HashSet<&str>,
    params: &HashMap<String, f64>,
    row: &mut LinearRow,
) -> Option<()> {
    use oxidelica_parser::BinOp::*;
    let names_unknown = |e: &Expr| {
        let mut refs = Vec::new();
        e.collect_refs(&mut refs);
        refs.iter().any(|r| unknown.contains(r))
    };
    match expr {
        Expr::Ref(name) if column_of.contains_key(name.as_str()) => {
            let slot = row.columns.entry(column_of[name.as_str()]).or_insert(0);
            *slot = slot.checked_add(scale)?;
        }
        Expr::Neg(inner) => {
            read_terms(inner, scale.checked_neg()?, column_of, unknown, params, row)?
        }
        Expr::Bin(Add, l, r) => {
            read_terms(l, scale, column_of, unknown, params, row)?;
            read_terms(r, scale, column_of, unknown, params, row)?;
        }
        Expr::Bin(Sub, l, r) => {
            read_terms(l, scale, column_of, unknown, params, row)?;
            read_terms(r, scale.checked_neg()?, column_of, unknown, params, row)?;
        }
        Expr::Bin(Mul, l, r) if names_unknown(expr) => {
            if let Some(k) = integer_of(l, params) {
                read_terms(r, scale.checked_mul(k)?, column_of, unknown, params, row)?;
            } else {
                let k = integer_of(r, params)?;
                read_terms(l, scale.checked_mul(k)?, column_of, unknown, params, row)?;
            }
        }
        _ if names_unknown(expr) => return None,
        _ => row.rest.push((scale, expr.clone())),
    }
    Some(())
}

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// `a*x - b*y` over sparse integer vectors, or nothing on overflow.
fn combine(
    x: &BTreeMap<usize, i128>,
    a: i128,
    y: &BTreeMap<usize, i128>,
    b: i128,
) -> Option<BTreeMap<usize, i128>> {
    let mut out = BTreeMap::new();
    for (&k, &v) in x {
        out.insert(k, v.checked_mul(a)?);
    }
    for (&k, &v) in y {
        let slot = out.entry(k).or_insert(0);
        *slot = slot.checked_sub(v.checked_mul(b)?)?;
    }
    out.retain(|_, v| *v != 0);
    Some(out)
}

/// An equation every unknown has cancelled out of and a state has not:
/// which equation it may stand in for, and the equation itself as
/// `lhs = 0`.
///
/// The equation replaced is one the combination took a nonzero share
/// of, so the system is the same system written differently. Nothing is
/// found where the equations hold no such combination, where the
/// arithmetic would overflow, or where the search grows past its
/// ceiling.
pub(crate) fn hidden_constraint(
    algebraic_eqs: &[(Expr, Expr)],
    unknowns: &[String],
    states: &[String],
    params: &HashMap<String, f64>,
) -> Option<(usize, Expr)> {
    let unknown: HashSet<&str> = unknowns.iter().map(String::as_str).collect();
    let mut column_of: HashMap<&str, usize> = HashMap::new();
    for name in unknowns.iter().chain(states) {
        let next = column_of.len();
        column_of.entry(name.as_str()).or_insert(next);
    }
    let is_unknown_column = |c: usize| c < unknowns.len();

    let mut rows: Vec<(usize, LinearRow)> = Vec::new();
    for (index, (lhs, rhs)) in algebraic_eqs.iter().enumerate() {
        let mut row = LinearRow {
            columns: BTreeMap::new(),
            rest: Vec::new(),
        };
        let read = read_terms(lhs, 1, &column_of, &unknown, params, &mut row)
            .and_then(|()| read_terms(rhs, -1, &column_of, &unknown, params, &mut row));
        if read.is_some() {
            row.columns.retain(|_, v| *v != 0);
            rows.push((index, row));
        }
    }
    // Nothing to find unless some linear equation holds a state.
    let holds_state = |row: &LinearRow| {
        !row.rest.is_empty() || row.columns.keys().any(|&c| !is_unknown_column(c))
    };
    if !rows.iter().any(|(_, row)| holds_state(row)) {
        return None;
    }

    // Echelon form over the unknown columns, each row carrying the
    // combination of equations it now is.
    let mut work = 0usize;
    let mut pivots: Vec<Pivot> = Vec::new();
    let mut pivot_columns: HashSet<usize> = HashSet::new();
    for (position, (_, row)) in rows.iter().enumerate() {
        let mut values: BTreeMap<usize, i128> = row
            .columns
            .iter()
            .filter(|(&c, _)| is_unknown_column(c))
            .map(|(&c, &v)| (c, v))
            .collect();
        let mut combination: BTreeMap<usize, i128> = BTreeMap::from([(position, 1)]);
        for (column, pivot_values, pivot_combination) in &pivots {
            let Some(&r) = values.get(column) else {
                continue;
            };
            let p = pivot_values[column];
            work += values.len() + pivot_values.len() + combination.len() + pivot_combination.len();
            if work > MAX_ELIMINATION_WORK {
                return None;
            }
            values = combine(&values, p, pivot_values, r)?;
            combination = combine(&combination, p, pivot_combination, r)?;
            let common = values
                .values()
                .chain(combination.values())
                .fold(0, |g, &v| gcd(g, v));
            if common > 1 {
                values.values_mut().for_each(|v| *v /= common);
                combination.values_mut().for_each(|v| *v /= common);
            }
        }
        if let Some((&column, _)) = values.iter().find(|(c, _)| !pivot_columns.contains(c)) {
            pivot_columns.insert(column);
            pivots.push((column, values, combination));
            continue;
        }
        if !values.is_empty() {
            continue;
        }
        // Every unknown has cancelled. What is left is a constraint
        // on states, if any state is left in it.
        let mut state_columns: BTreeMap<usize, i128> = BTreeMap::new();
        let mut rest: Vec<(i128, Expr)> = Vec::new();
        for (&member, &share) in &combination {
            let member_row = &rows[member].1;
            for (&c, &v) in &member_row.columns {
                let slot = state_columns.entry(c).or_insert(0);
                *slot = slot.checked_add(v.checked_mul(share)?)?;
            }
            for (k, expr) in &member_row.rest {
                rest.push((k.checked_mul(share)?, expr.clone()));
            }
        }
        state_columns.retain(|_, v| *v != 0);
        let mut terms: Vec<Expr> = Vec::new();
        let names: Vec<&String> = unknowns.iter().chain(states).collect();
        for (&c, &v) in &state_columns {
            let name = Expr::Ref(names[c].clone());
            terms.push(scaled(v, name));
        }
        for (k, expr) in rest {
            terms.push(scaled(k, expr));
        }
        let Some(first) = terms.first().cloned() else {
            continue;
        };
        let lhs = terms.into_iter().skip(1).fold(first, |sum, term| {
            Expr::Bin(oxidelica_parser::BinOp::Add, Box::new(sum), Box::new(term))
        });
        let mut named = Vec::new();
        lhs.collect_refs(&mut named);
        if !named.iter().any(|n| states.iter().any(|s| s == n)) {
            continue;
        }
        let replaced = combination
            .keys()
            .map(|&member| rows[member].0)
            .max()
            .expect("a combination holds the row it started from");
        return Some((replaced, simplify(&lhs)));
    }
    None
}

fn scaled(k: i128, expr: Expr) -> Expr {
    match k {
        1 => expr,
        -1 => Expr::Neg(Box::new(expr)),
        _ => Expr::Bin(
            oxidelica_parser::BinOp::Mul,
            Box::new(Expr::Number(k as f64)),
            Box::new(expr),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Expr {
        let model =
            oxidelica_parser::parse_model(&format!("model T Real a; equation a = {text}; end T;"))
                .unwrap();
        model.equations[0].rhs.clone()
    }

    #[test]
    fn a_star_behind_a_zero_sequence_sum_constrains_the_state() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let unknowns = names(&["i1", "i2", "i3"]);
        let states = names(&["i0"]);
        let params = HashMap::from([("m".to_string(), 3.0)]);
        let eqs = vec![
            (parse("m * i0"), parse("i1 + i2 + i3")),
            (parse("i1 + i2 + i3"), Expr::Number(0.0)),
        ];
        let (replaced, lhs) = hidden_constraint(&eqs, &unknowns, &states, &params).unwrap();
        assert_eq!(replaced, 1);
        // The equation is the zero inductor's own, `m*i0`, written as
        // the model wrote it: the parameter stays a name.
        let mut named = Vec::new();
        lhs.collect_refs(&mut named);
        assert_eq!(named, vec!["m", "i0"]);
    }

    #[test]
    fn independent_equations_hide_nothing() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let eqs = vec![
            (parse("2 * x + y"), parse("s")),
            (parse("x - y"), Expr::Number(0.0)),
        ];
        let found = hidden_constraint(&eqs, &names(&["x", "y"]), &names(&["s"]), &HashMap::new());
        assert!(found.is_none());
    }

    #[test]
    fn a_coefficient_that_is_not_whole_takes_no_part() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let eqs = vec![
            (parse("0.5 * x"), parse("s")),
            (parse("x"), Expr::Number(0.0)),
        ];
        let found = hidden_constraint(&eqs, &names(&["x"]), &names(&["s"]), &HashMap::new());
        assert!(found.is_none());
    }
}
