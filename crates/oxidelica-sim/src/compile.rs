//! Turning a flat model into a plan: matching equations to unknowns,
//! reducing the index, tearing the loops and laying the result out as
//! stages a run can walk.

use crate::*;

/// Replace every `der(x)` with the right-hand side of that state.
pub(crate) fn substitute_derivatives(
    expr: &Expr,
    states: &[String],
    derivatives: &[Expr],
) -> Result<Expr, SimError> {
    // The one case this is about: `der(x)` becomes whatever stands for
    // the derivative of `x`, and everything else is the same
    // expression with its children substituted.
    if let Some(state) = expr.as_der_of() {
        let Some(index) = states.iter().position(|s| s == state) else {
            return err(format!(
                "der({state}): `{state}` is not a state of the model"
            ));
        };
        return Ok(derivatives[index].clone());
    }
    expr.try_map_children(&mut |child| substitute_derivatives(child, states, derivatives))
}

/// Which states each state's right-hand side depends on, and a grouping
/// of the columns that can be probed together.
///
/// The dependency runs through the algebraic plan: a derivative that
/// reads an algebraic variable depends on whatever that variable was
/// computed from. An implicit block is taken as a whole, since its
/// unknowns are solved together and each may move with any input of the
/// block.
///
/// Two columns may share a group when no row depends on both. Finding
/// the fewest groups is graph colouring, so this takes the greedy
/// answer, which for the banded Jacobians of a discretized field is
/// already the optimum: three groups for a tridiagonal one.
pub(crate) fn jacobian_structure(
    states: &[String],
    algebraics: &[String],
    derivatives: &[Expr],
    stages: &[PlanStage],
) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
    let index_of_state: HashMap<&str, usize> = states.iter().map(|s| s.as_str()).zip(0..).collect();
    let index_of_algebraic: HashMap<&str, usize> =
        algebraics.iter().map(|a| a.as_str()).zip(0..).collect();

    // What each algebraic variable depends on, in evaluation order.
    // `None` means "not worked out": such a variable is taken to depend
    // on every state, because a missing entry would quietly cost the
    // Jacobian a term while an extra one only costs an evaluation.
    let mut through_algebraic: Vec<Option<Vec<bool>>> = vec![None; algebraics.len()];
    let depends_on = |expr: &Expr, out: &mut Vec<bool>, through: &[Option<Vec<bool>>]| {
        let mut refs = Vec::new();
        expr.collect_refs(&mut refs);
        for name in refs {
            if let Some(&state) = index_of_state.get(name) {
                out[state] = true;
            } else if let Some(&algebraic) = index_of_algebraic.get(name) {
                match through.get(algebraic) {
                    Some(Some(inherited)) => {
                        for (state, &touched) in inherited.iter().enumerate() {
                            out[state] |= touched;
                        }
                    }
                    _ => out.iter_mut().for_each(|touched| *touched = true),
                }
            }
        }
    };

    for stage in stages {
        match stage {
            PlanStage::Explicit { var, expr } => {
                let mut row = vec![false; states.len()];
                depends_on(expr, &mut row, &through_algebraic);
                through_algebraic[*var] = Some(row);
            }
            PlanStage::Implicit {
                vars,
                inner,
                residuals,
                ..
            } => {
                // Everything the block reads reaches everything it solves.
                let mut row = vec![false; states.len()];
                for (_, expr) in inner {
                    depends_on(expr, &mut row, &through_algebraic);
                }
                for (lhs, rhs) in residuals {
                    depends_on(lhs, &mut row, &through_algebraic);
                    depends_on(rhs, &mut row, &through_algebraic);
                }
                for &var in vars {
                    through_algebraic[var] = Some(row.clone());
                }
            }
        }
    }

    // Rows of the Jacobian: the states each derivative depends on.
    let mut rows_of_column: Vec<Vec<usize>> = vec![Vec::new(); states.len()];
    for (row, expr) in derivatives.iter().enumerate() {
        let mut touched = vec![false; states.len()];
        depends_on(expr, &mut touched, &through_algebraic);
        for (column, &yes) in touched.iter().enumerate() {
            if yes {
                rows_of_column[column].push(row);
            }
        }
    }

    // Greedy colouring: a column joins the first group whose rows it
    // does not overlap.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut taken: Vec<Vec<bool>> = Vec::new();
    for (column, touched) in rows_of_column.iter().enumerate() {
        let mut placed = false;
        for (group, rows) in groups.iter_mut().zip(taken.iter_mut()) {
            if touched.iter().all(|&row| !rows[row]) {
                for &row in touched {
                    rows[row] = true;
                }
                group.push(column);
                placed = true;
                break;
            }
        }
        if !placed {
            let mut rows = vec![false; derivatives.len()];
            for &row in touched {
                rows[row] = true;
            }
            groups.push(vec![column]);
            taken.push(rows);
        }
    }
    (groups, rows_of_column)
}

/// Compile a parsed flat model into an executable form.
pub fn compile(model: &Model) -> Result<CompiledModel, SimError> {
    compile_at(model, None)
}

/// The order a run evaluates the algebraic layer in.
///
/// An equation that can be solved for its own unknown on its own is an
/// explicit assignment, and those go first in dependency order. What
/// is left is cyclic and becomes one torn block: inside it the
/// equations that can still be solved explicitly are evaluated in
/// order, and Newton iterates only on the few that cannot - which
/// keeps the Jacobian small.
fn build_plan(
    unknowns: &[String],
    algebraic_eqs: &[(Expr, Expr)],
    matched_var: &[usize],
    eq_vars: &[Vec<usize>],
    n_alg: usize,
    known: &HashMap<String, f64>,
) -> (Vec<String>, Vec<PlanStage>) {
    // Kahn topological order over equations.
    let producer: Vec<usize> = {
        let mut p = vec![0; n_alg];
        for (eq, &v) in matched_var.iter().enumerate() {
            p[v] = eq;
        }
        p
    };
    let mut indegree = vec![0usize; n_alg];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n_alg];
    for eq in 0..n_alg {
        for &v in &eq_vars[eq] {
            if v != matched_var[eq] {
                dependents[producer[v]].push(eq);
                indegree[eq] += 1;
            }
        }
    }
    let mut queue: Vec<usize> = (0..n_alg).filter(|&e| indegree[e] == 0).collect();
    let mut emitted = Vec::new();
    let mut done = vec![false; n_alg];
    while let Some(eq) = queue.pop() {
        if done[eq] {
            continue;
        }
        done[eq] = true;
        emitted.push(eq);
        for &dep in &dependents[eq] {
            indegree[dep] = indegree[dep].saturating_sub(1);
            if indegree[dep] == 0 && !done[dep] {
                queue.push(dep);
            }
        }
    }

    let mentions = |expr: &Expr, name: &str| -> bool {
        let mut refs = Vec::new();
        expr.collect_refs(&mut refs);
        refs.contains(&name)
    };
    // What the slope test may treat as a number. A parameter is one,
    // and so is an algebraic name whose own equation reads it off a
    // constant: `R_rel.T[3,2] = e[3]*e[2] + ... - e[1]*sin(phi)` with
    // an axis of `{0,0,1}` is the number zero however the run reaches
    // it. Judged without that, the slope looks live, the equation is
    // divided through by it, and the first residual of the block comes
    // out infinite before Newton has taken a step - a refusal naming
    // the solver, which is the one place nothing is wrong.
    //
    // A fixpoint, because one such name feeds the next - and the
    // tests are sorted by price, because the dear half here is the
    // fold and a model carries tens of thousands of equations. The
    // shape - a name standing alone on one side - is one comparison
    // and is asked once. What each candidate needs is gathered once.
    // Then only a candidate all of whose needs are already numbers is
    // folded at all, and a round revisits only what the last round's
    // new values could have unblocked.
    let settled = {
        struct Candidate<'a> {
            name: &'a str,
            body: &'a Expr,
            needs: Vec<String>,
        }
        let mut candidates: Vec<Candidate<'_>> = Vec::new();
        for (eq, &var) in matched_var.iter().enumerate().take(n_alg) {
            let name = unknowns[var].as_str();
            let (lhs, rhs) = &algebraic_eqs[eq];
            let body = if matches!(lhs, Expr::Ref(n) if n == name) {
                rhs
            } else if matches!(rhs, Expr::Ref(n) if n == name) {
                lhs
            } else {
                continue;
            };
            let mut refs = Vec::new();
            body.collect_refs(&mut refs);
            if refs.contains(&name) {
                continue;
            }
            let mut needs: Vec<String> = refs.into_iter().map(str::to_string).collect();
            needs.sort_unstable();
            needs.dedup();
            candidates.push(Candidate { name, body, needs });
        }
        let mut settled = known.clone();
        loop {
            let mut grew = false;
            candidates.retain(|candidate| {
                if settled.contains_key(candidate.name) {
                    return false;
                }
                // Two ways a body ends in a number, and both are
                // asked with lookups alone. Either every name it
                // reads is already one - or one of them is a zero,
                // which is how a term naming something live
                // disappears: `e[1]*sin(phi)` with an axis that has
                // no first component is nothing at all, whatever the
                // angle does. Anything else cannot fold and the
                // expense of trying is the thing being avoided.
                let all_known = candidate.needs.iter().all(|n| settled.contains_key(n));
                let has_zero = candidate
                    .needs
                    .iter()
                    .any(|n| matches!(settled.get(n), Some(v) if *v == 0.0));
                if !all_known && !has_zero {
                    return true;
                }
                let mut folded = candidate.body.clone();
                for need in &candidate.needs {
                    if let Some(value) = settled.get(need) {
                        folded = crate::symbolic::substitute(&folded, need, *value);
                    }
                }
                if let Expr::Number(value) = crate::symbolic::simplify(&folded) {
                    settled.insert(candidate.name.to_string(), value);
                    grew = true;
                }
                false
            });
            if !grew {
                break;
            }
        }
        settled
    };
    let known = &settled;
    let mut ordered_algs: Vec<String> = Vec::new();
    let mut stages: Vec<PlanStage> = Vec::new();
    for &eq in &emitted {
        let var_name = unknowns[matched_var[eq]].clone();
        let index = ordered_algs.len();
        let (lhs, rhs) = &algebraic_eqs[eq];
        let stage = if matches!(lhs, Expr::Ref(n) if n == &var_name) && !mentions(rhs, &var_name) {
            PlanStage::Explicit {
                var: index,
                expr: rhs.clone(),
            }
        } else if matches!(rhs, Expr::Ref(n) if n == &var_name) && !mentions(lhs, &var_name) {
            PlanStage::Explicit {
                var: index,
                expr: lhs.clone(),
            }
        } else if let Some(expr) = solve_linear_known(lhs, rhs, &var_name, known) {
            // Linear in its unknown: solved symbolically, no iteration.
            PlanStage::Explicit { var: index, expr }
        } else {
            PlanStage::Implicit {
                vars: vec![index],
                torn: vec![index],
                inner: Vec::new(),
                residuals: vec![(lhs.clone(), rhs.clone())],
            }
        };
        ordered_algs.push(var_name);
        stages.push(stage);
    }

    // The cyclic remainder becomes one torn block: equations that can
    // be solved explicitly for their unknown are evaluated in
    // dependency order, and Newton iterates only on the tearing
    // variables needed to break the remaining cycles.
    let remainder: Vec<usize> = (0..n_alg).filter(|&e| !done[e]).collect();
    if !remainder.is_empty() {
        let base = ordered_algs.len();
        let mut index_of: HashMap<usize, usize> = HashMap::new();
        for (offset, &eq) in remainder.iter().enumerate() {
            let var = matched_var[eq];
            index_of.insert(var, base + offset);
            ordered_algs.push(unknowns[var].clone());
        }
        let vars: Vec<usize> = (base..ordered_algs.len()).collect();

        // Which equations can be solved explicitly for their unknown?
        let solvable: HashMap<usize, Expr> = remainder
            .iter()
            .filter_map(|&eq| {
                let name = &unknowns[matched_var[eq]];
                let (lhs, rhs) = &algebraic_eqs[eq];
                if matches!(lhs, Expr::Ref(n) if n == name) && !mentions(rhs, name) {
                    Some((eq, rhs.clone()))
                } else if matches!(rhs, Expr::Ref(n) if n == name) && !mentions(lhs, name) {
                    Some((eq, lhs.clone()))
                } else {
                    solve_linear_known(lhs, rhs, name, known).map(|expr| (eq, expr))
                }
            })
            .collect();

        // Equations that resist explicit solution force their unknown
        // into the tearing set; then tear greedily until the rest sorts
        // topologically, preferring the most-referenced unknowns.
        let mut torn_eqs: Vec<usize> = remainder
            .iter()
            .copied()
            .filter(|eq| !solvable.contains_key(eq))
            .collect();
        let uses: HashMap<usize, usize> = {
            let mut counts: HashMap<usize, usize> = HashMap::new();
            for &eq in &remainder {
                for &v in &eq_vars[eq] {
                    if index_of.contains_key(&v) {
                        *counts.entry(v).or_default() += 1;
                    }
                }
            }
            counts
        };
        let inner_order = loop {
            let torn_vars: Vec<usize> = torn_eqs.iter().map(|&eq| matched_var[eq]).collect();
            let pending: Vec<usize> = remainder
                .iter()
                .copied()
                .filter(|eq| !torn_eqs.contains(eq))
                .collect();
            // Topological pass over the untorn equations.
            let mut placed: Vec<usize> = Vec::new();
            let mut available = torn_vars.clone();
            let mut left = pending.clone();
            loop {
                let before = left.len();
                left.retain(|&eq| {
                    let ready = eq_vars[eq].iter().all(|v| {
                        !index_of.contains_key(v) || *v == matched_var[eq] || available.contains(v)
                    });
                    if ready {
                        placed.push(eq);
                        available.push(matched_var[eq]);
                        false
                    } else {
                        true
                    }
                });
                if left.is_empty() || left.len() == before {
                    break;
                }
            }
            if left.is_empty() {
                break placed;
            }
            // Still cyclic: tear the most-referenced unknown left.
            let victim = *left
                .iter()
                .max_by_key(|&&eq| uses.get(&matched_var[eq]).copied().unwrap_or(0))
                .expect("non-empty cycle");
            torn_eqs.push(victim);
        };

        let inner: Vec<(usize, Expr)> = inner_order
            .iter()
            .map(|eq| (index_of[&matched_var[*eq]], solvable[eq].clone()))
            .collect();
        let torn: Vec<usize> = torn_eqs
            .iter()
            .map(|&eq| index_of[&matched_var[eq]])
            .collect();
        let residuals: Vec<(Expr, Expr)> = torn_eqs
            .iter()
            .map(|&eq| algebraic_eqs[eq].clone())
            .collect();
        stages.push(PlanStage::Implicit {
            vars,
            torn,
            inner,
            residuals,
        });
    }
    if std::env::var_os("OXIDELICA_MODE_PROBE").is_some() {
        probe_mode_conditions(&ordered_algs, &stages);
    }
    (ordered_algs, stages)
}

/// What an `if` inside a torn block branches on.
///
/// A block whose slope is an `if` is a block whose plan is a guess
/// about which branch holds, and two quite different illnesses read
/// alike from outside. A condition built from parameters and
/// discretes alone stands still between events, so a mode-wise
/// substitution could in principle give the block a plan that
/// survives; a condition naming a continuous unknown of the block
/// itself moves under Newton within the step, and no substitution
/// helps. This prints the condition and the names it reads so that
/// the two can be counted apart before either is worked on, which is
/// what said the substitution was not worth writing: of the
/// twenty-nine models standing at the residual-is-NaN wall, fourteen
/// look like the first kind, eight are mixed, four are the second,
/// and three have no conditional in the block at all.
///
/// Behind an environment switch so that both numbers come from one
/// binary.
fn probe_mode_conditions(ordered_algs: &[String], stages: &[PlanStage]) {
    let report = |what: &str, expr: &Expr, block: &[usize]| {
        let mut found = Vec::new();
        expr.for_each(&mut |node| {
            if let Expr::If(condition, _, _) = node {
                found.push(condition.as_ref().clone());
            }
        });
        for condition in found {
            let mut refs = Vec::new();
            condition.collect_refs(&mut refs);
            refs.sort_unstable();
            refs.dedup();
            // A name of the block itself is what makes the condition
            // move under iteration; everything else stands still
            // between events, whether it is a parameter or a discrete.
            let names: Vec<&str> = block.iter().map(|&v| ordered_algs[v].as_str()).collect();
            let continuous: Vec<&&str> = refs.iter().filter(|r| names.contains(r)).collect();
            eprintln!(
                "mode-probe: {what} branches on {condition:?}; reads {refs:?}; \
                 of the block: {continuous:?}; family {}",
                if continuous.is_empty() { "A" } else { "B" }
            );
        }
    };
    for stage in stages {
        let PlanStage::Implicit {
            vars,
            inner,
            residuals,
            ..
        } = stage
        else {
            continue;
        };
        for (i, (lhs, rhs)) in residuals.iter().enumerate() {
            // The residual whole, not only its conditions. A block
            // that cannot be evaluated at its starting values is
            // named by the solver by number alone, and the number
            // says nothing about which equation was meant.
            eprintln!(
                "mode-probe: residual {i} of block {:?}: {lhs:?} = {rhs:?}",
                vars.iter()
                    .map(|&v| ordered_algs[v].as_str())
                    .collect::<Vec<_>>()
            );
            report("a residual", lhs, vars);
            report("a residual", rhs, vars);
        }
        for (var, expr) in inner {
            eprintln!(
                "mode-probe: inner {} := {expr:?}",
                ordered_algs[*var].as_str()
            );
            report("an inner assignment", expr, vars);
        }
    }
}

/// How many times a constraint may be differentiated before the model
/// is called singular rather than merely of high index.
///
/// A guard against a reduction that gets nowhere, not against a deep
/// one: every accepted step adds an equation and matches more of
/// them, so a system that is making progress is not in a loop. The
/// resonance circuits go from 173 matched to 252 over sixteen steps,
/// gaining ground each time, and were called singular for running out
/// of turns rather than for standing still. A model of genuinely high
/// index needs as many reductions as it has index, and nothing says
/// that is a small number.
const MAX_INDEX_REDUCTIONS: usize = 256;

/// How large a differentiated constraint may grow before index
/// reduction gives up on it.
///
/// A refusal that names the model is worth a great deal more than a
/// process killed for its memory. Lifting one condition of the
/// grounding fixpoint made some model of the standard library copy
/// its constraint from reduction to reduction until the corpus ran
/// out of memory at every thread count, and because the death was the
/// operating system's rather than the compiler's, nothing said which
/// model it was. A ceiling on the size turns that into a sentence.
///
/// The number is set well above what a healthy model reaches: the
/// largest constraint measured over the library is some thousands of
/// nodes, and a copying chain passes a million within a handful of
/// reductions.
const MAX_CONSTRAINT_NODES: usize = 2_000_000;

/// The ceiling in force, which `OXIDELICA_MAX_CONSTRAINT_NODES` may
/// lower.
///
/// A guard whose threshold no test can reach is a guard nobody has
/// seen work. The models that pass this ceiling honestly are the ones
/// that eat a machine's memory getting there, which is precisely what
/// cannot be written into a test; lowering the number lets a model of
/// a dozen lines stand where they stand.
fn max_constraint_nodes() -> usize {
    if let Some(lowered) = lowered_ceiling() {
        return lowered;
    }
    std::env::var("OXIDELICA_MAX_CONSTRAINT_NODES")
        .ok()
        .and_then(|written| written.parse().ok())
        .unwrap_or(MAX_CONSTRAINT_NODES)
}

/// The ceiling lowered for this thread alone.
///
/// A test that lowers it through the environment lowers it for every
/// test compiling beside it, because the environment belongs to the
/// binary and the tests share one. A mutex around the setting does not
/// help: the readers are the other tests, which hold no lock and are
/// entitled to the real ceiling. This was a genuine race - it went
/// unseen only because the model it struck took two minutes to reach
/// the read, and it surfaced the moment that model got faster.
pub(crate) fn lowered_ceiling() -> Option<usize> {
    CEILING_HERE.with(|c| c.get())
}

thread_local! {
    static CEILING_HERE: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

/// Lower the constraint ceiling for this thread, for a test that means
/// to see the guard fire. Undone by dropping the returned guard.
pub fn lower_the_ceiling_here(nodes: usize) -> CeilingGuard {
    CEILING_HERE.with(|c| c.set(Some(nodes)));
    CeilingGuard(())
}

/// Puts the constraint ceiling back where it was.
pub struct CeilingGuard(());

impl Drop for CeilingGuard {
    fn drop(&mut self) {
        CEILING_HERE.with(|c| c.set(None));
    }
}

/// What index reduction leaves behind: the system as it stands, and
/// the matching that covers it.
struct Reduction {
    /// The states that survived demotion.
    states: Vec<String>,
    /// Everything solved for algebraically, demoted states included.
    unknowns: Vec<String>,
    /// The algebraic equations, differentiated constraints included.
    algebraic_eqs: Vec<(Expr, Expr)>,
    /// Demoted state -> the dummy unknown standing for its derivative.
    dummies: HashMap<String, String>,
    /// Per reduction: the constraint, the victim, and what else was on
    /// offer - the run watches these to know when to choose again.
    selection_records: Vec<(Expr, String, Vec<String>)>,
    /// Which equation was matched to each unknown.
    matched_eq: Vec<Option<usize>>,
    /// The unknowns each equation mentions.
    eq_vars: Vec<Vec<usize>>,
    /// How many unknowns there are.
    n_alg: usize,
}

/// Which unknowns nothing gives a value to, or which equations have
/// nothing left to determine.
///
/// The counts alone say a model is unbalanced; they do not say what is
/// missing, and a list of every unknown in the model - thousands of
/// them - says no more than the count did. The matching that index
/// reduction already uses answers it: run it over the equations there
/// are, and whatever is left unmatched is what the model is short of.
/// Naming those few is what turns one wording of a refusal into the
/// several illnesses standing behind it.
fn unbalanced_because(algebraic_eqs: &[(Expr, Expr)], unknowns: &[String]) -> String {
    let var_index: HashMap<&str, usize> = unknowns
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let eq_vars: Vec<Vec<usize>> = algebraic_eqs
        .iter()
        .map(|(lhs, rhs)| {
            let mut refs = Vec::new();
            lhs.collect_refs(&mut refs);
            rhs.collect_refs(&mut refs);
            let mut vars: Vec<usize> = refs
                .iter()
                .filter_map(|r| var_index.get(r).copied())
                .collect();
            vars.sort_unstable();
            vars.dedup();
            vars
        })
        .collect();
    let mut matched_eq: Vec<Option<usize>> = vec![None; unknowns.len()];
    let mut unmatched_eqs: Vec<usize> = Vec::new();
    for eq in 0..algebraic_eqs.len() {
        let mut visited = vec![false; unknowns.len()];
        if !try_match(eq, &eq_vars, &mut matched_eq, &mut visited) {
            unmatched_eqs.push(eq);
        }
    }
    let (kind, mut named): (&str, Vec<String>) = if algebraic_eqs.len() < unknowns.len() {
        // Too few equations: name what nothing determines.
        (
            "nothing determines",
            matched_eq
                .iter()
                .enumerate()
                .filter(|(_, eq)| eq.is_none())
                .map(|(v, _)| unknowns[v].clone())
                .collect(),
        )
    } else {
        // Too many: name the equations with nothing left to solve for.
        (
            "nothing is left for",
            unmatched_eqs
                .iter()
                .map(|eq| {
                    let (lhs, rhs) = &algebraic_eqs[*eq];
                    format!("{} = {}", describe(lhs), describe(rhs))
                })
                .collect(),
        )
    };
    named.sort();
    named.dedup();
    // A handful of names is a barrier one can go and look at; the whole
    // list is the noise this refusal used to print.
    let shown = named.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
    let rest = match named.len() {
        0..=6 => String::new(),
        n => format!(" and {} more", n - 6),
    };
    format!(
        "unbalanced model: {} algebraic equation(s) for {} unknown(s); {kind} {shown}{rest}",
        algebraic_eqs.len(),
        unknowns.len(),
    )
}

/// Whether the matching may prefer an unknown it can solve for without
/// dividing by another unknown.
///
/// Behind a switch so that one binary can produce both numbers: two
/// counts are comparable only when the same build made them.
/// On by default; `OXIDELICA_NO_MATCH_ORDER=1` gives the old order.
fn match_order_enabled() -> bool {
    std::env::var_os("OXIDELICA_NO_MATCH_ORDER").is_none()
}

/// What ranking an equation for one of its names depends on, with the
/// dear half separated from the half that moves.
///
/// The rank asks two questions, and only one of them is about the
/// block. Whether the name stands alone on a side, and what the slope
/// of the residual mentions, are questions about the equation and the
/// name alone; which of those mentions are unknowns *of this block* is
/// the question that changes as reduction demotes states. Answering
/// both together made the dear half - a differentiation and a fold -
/// be paid again at every reduction for an answer that cannot have
/// changed, which is what took the library's run half from 591
/// seconds of processor time to 3153.
enum SolveShape {
    /// The name stands alone on one side and is named nowhere else, so
    /// the equation is read off with no arithmetic at all.
    Alone,
    /// The residual's slope in the name, reduced to what it mentions.
    /// `None` where the equation could not be differentiated, which is
    /// as dear as it gets.
    Slope(Option<Vec<String>>),
}

/// The half of the rank that does not move: worked once per equation
/// and name, under the same bracket as `solved_for` - inside one
/// reduction the equation list is only appended to, so the pair at an
/// index is the pair that was there before.
fn solve_shape(lhs: &Expr, rhs: &Expr, name: &str) -> SolveShape {
    let mentions = |expr: &Expr| {
        let mut refs = Vec::new();
        expr.collect_refs(&mut refs);
        refs.contains(&name)
    };
    if (matches!(lhs, Expr::Ref(n) if n == name) && !mentions(rhs))
        || (matches!(rhs, Expr::Ref(n) if n == name) && !mentions(lhs))
    {
        return SolveShape::Alone;
    }
    let residual = Expr::Bin(
        oxidelica_parser::BinOp::Sub,
        Box::new(lhs.clone()),
        Box::new(rhs.clone()),
    );
    let Ok(slope) = differentiate(&residual, &DiffTarget::Variable(name)) else {
        return SolveShape::Slope(None);
    };
    let slope = simplify(&slope);
    let mut refs = Vec::new();
    slope.collect_refs(&mut refs);
    let mut named: Vec<String> = refs.into_iter().map(str::to_string).collect();
    named.sort_unstable();
    named.dedup();
    SolveShape::Slope(Some(named))
}

/// How dearly an equation is solved for one of its unknowns: the
/// shape above, read against this block's other unknowns.
///
/// The matching is free to give an equation to any unknown it
/// mentions, and the choice decides what the run divides by. `a = b*c`
/// given to `a` is a multiplication; given to `b` it is `a/c`, and a
/// `c` that is zero at the start turns the block's first residual into
/// a NaN before Newton has taken a step. The rank is only a
/// preference - an augmenting path may still overrule it, which is why
/// this is measured rather than believed:
///
/// - 0: the unknown stands alone on one side, so the equation is read
///   off with no arithmetic at all;
/// - 1: linear in the unknown with a slope naming no other unknown of
///   the block, so what is divided by cannot move under Newton;
/// - 2: anything else, which is where the divisions by a live unknown
///   live.
fn solve_cost(shape: &SolveShape, others: &[String]) -> u8 {
    match shape {
        SolveShape::Alone => 0,
        SolveShape::Slope(None) => 2,
        SolveShape::Slope(Some(refs)) => match refs.iter().any(|r| others.iter().any(|o| o == r)) {
            true => 2,
            false => 1,
        },
    }
}

/// Order each equation's unknowns by what solving for them costs.
///
/// Cheapest first, and ties left in the order they arrived so that a
/// model with nothing to choose between is matched exactly as before.
fn order_by_solve_cost(
    eq_vars: &mut [Vec<usize>],
    algebraic_eqs: &[(Expr, Expr)],
    unknowns: &[String],
    shapes: &mut HashMap<(usize, String), SolveShape>,
) {
    for (eq, vars) in eq_vars.iter_mut().enumerate() {
        if vars.len() < 2 {
            continue;
        }
        let (lhs, rhs) = &algebraic_eqs[eq];
        let names: Vec<String> = vars.iter().map(|&v| unknowns[v].clone()).collect();
        let mut ranked: Vec<(u8, usize)> = Vec::with_capacity(vars.len());
        for (&v, name) in vars.iter().zip(&names) {
            let shape = shapes
                .entry((eq, name.clone()))
                .or_insert_with(|| solve_shape(lhs, rhs, name));
            let others: Vec<String> = names.iter().filter(|n| *n != name).cloned().collect();
            ranked.push((solve_cost(shape, &others), v));
        }
        ranked.sort_by_key(|(cost, _)| *cost);
        *vars = ranked.into_iter().map(|(_, v)| v).collect();
    }
}

// Augmenting-path maximum matching.
fn try_match(
    eq: usize,
    eq_vars: &[Vec<usize>],
    matched_eq: &mut [Option<usize>],
    visited: &mut [bool],
) -> bool {
    for &v in &eq_vars[eq] {
        if !visited[v] {
            visited[v] = true;
            if matched_eq[v].is_none()
                || try_match(matched_eq[v].unwrap(), eq_vars, matched_eq, visited)
            {
                matched_eq[v] = Some(eq);
                return true;
            }
        }
    }
    false
}

/// Whether to differentiate implicitly through an unknown no
/// rearrangement defines.
///
/// Behind a switch so that one binary can produce both numbers: two
/// counts are comparable only when the same build made them, and this
/// project has already once mistaken two builds for one measurement.
/// On by default; `OXIDELICA_NO_IMPLICIT_DIFF=1` gives the old walk.
fn implicit_enabled() -> bool {
    std::env::var_os("OXIDELICA_NO_IMPLICIT_DIFF").is_none()
}

/// Match every equation to an unknown, reducing the index where that
/// cannot be done.
///
/// An equation that cannot be matched is a constraint of the DAE, and
/// one step of Pantelides with dummy derivatives is taken for it:
/// differentiate the constraint and *add* the result (the original
/// stays, so it keeps holding exactly - no drift, no stabilisation
/// term), then demote one state appearing in it to an algebraic
/// unknown. Its former state equation determines the dummy that
/// replaces its derivative, which restores the balance.
#[allow(clippy::too_many_arguments)]
fn reduce_index(
    mut states: Vec<String>,
    mut unknowns: Vec<String>,
    mut algebraic_eqs: Vec<(Expr, Expr)>,
    state_rhs: &mut HashMap<String, Expr>,
    params: &HashMap<String, f64>,
    start_env: &HashMap<String, f64>,
    at_time: f64,
    anchored: &dyn Fn(&str) -> bool,
) -> Result<Reduction, SimError> {
    let mut dummies: HashMap<String, String> = HashMap::new();
    // States named in the right-hand side of an already-demoted state:
    // when `y` goes, its `der(y) = vy` marks `vy` as the companion the
    // next differentiation level should demote. Preferring companions
    // keeps each level of a chain of constraints demoting at its own
    // level - a velocity constraint takes a velocity, not a position
    // that happens to be numerically larger at this instant.
    let mut companions: Vec<String> = Vec::new();
    // Per reduction: the constraint residual, the demoted victim and
    // the states that were candidates - the runtime monitor watches the
    // victim's sensitivity against the alternatives and asks for a
    // re-selection while the numbers are still healthy.
    let mut selection_records: Vec<(Expr, String, Vec<String>)> = Vec::new();
    let mut reductions = 0usize;
    // The derivatives the walk has given names to, carried across
    // reductions. They arrive already grounded - each was built out of
    // states, parameters, dummies and other minted names at the moment
    // it was made - so they are handed to the walk directly rather than
    // offered to the fixpoint, which sees only `der(a) = der(b)` pairs
    // and settles neither.
    let mut minted_defs: HashMap<String, Expr> = HashMap::new();
    // What each equation solves for, worked once. Rearranging an
    // equation for a name is the dearest thing in this loop - it
    // differentiates the equation and folds the result three times -
    // and it is asked afresh at every reduction for every equation
    // and every name in it, though the answer cannot have changed:
    // the list of equations is only ever appended to here, so the
    // pair at a given index is the pair that was there before.
    //
    // That is the bracket the table needs, and it is what keeps the
    // key small: inside one call the equation is held still by its
    // index, so what is left to name is the index and the variable.
    // Both halves are bounded - a name is asked for only where the
    // equation mentions it - so a remembered refusal costs no more
    // than a remembered answer.
    let mut solved_for: HashMap<(usize, String), Option<Expr>> = HashMap::new();
    // The same bracket serves the matching's rank, whose dear half -
    // differentiating the equation and folding the slope - depends on
    // the equation and the name and on nothing else. Which of the
    // slope's names are unknowns of the block does move between
    // reductions, so that half is asked afresh each time and only the
    // shape is remembered.
    let mut solve_shapes: HashMap<(usize, String), SolveShape> = HashMap::new();
    let (matched_eq, eq_vars, n_alg) = loop {
        let var_index: HashMap<&str, usize> = unknowns
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();
        let eq_vars: Vec<Vec<usize>> = algebraic_eqs
            .iter()
            .map(|(lhs, rhs)| {
                let mut refs = Vec::new();
                lhs.collect_refs(&mut refs);
                rhs.collect_refs(&mut refs);
                let mut vars: Vec<usize> = refs
                    .iter()
                    .filter_map(|r| var_index.get(r).copied())
                    .collect();
                vars.sort_unstable();
                vars.dedup();
                vars
            })
            .collect();

        // Cheapest-to-solve unknown first. The matching is otherwise
        // indifferent about which unknown an equation takes, and the
        // indifference is what hands a product equation to a factor
        // and makes the run divide by another unknown of the block.
        let mut eq_vars = eq_vars;
        if match_order_enabled() {
            order_by_solve_cost(&mut eq_vars, &algebraic_eqs, &unknowns, &mut solve_shapes);
        }
        let eq_vars = eq_vars;

        let n_alg = unknowns.len();
        let mut matched_eq: Vec<Option<usize>> = vec![None; n_alg];
        let mut failed = None;
        for eq in 0..algebraic_eqs.len() {
            let mut visited = vec![false; n_alg];
            if !try_match(eq, &eq_vars, &mut matched_eq, &mut visited) {
                failed = Some(eq);
                break;
            }
        }
        let Some(eq) = failed else {
            break (matched_eq, eq_vars, n_alg);
        };

        let (lhs, rhs) = algebraic_eqs[eq].clone();
        if reductions >= MAX_INDEX_REDUCTIONS {
            return err(format!(
            "structurally singular model: equation {lhs:?} = {rhs:?} still cannot be matched after {MAX_INDEX_REDUCTIONS} index reductions"
        ));
        }
        reductions += 1;

        let mut candidates: Vec<(String, Expr)> = Vec::new();
        for (index, (l, r)) in algebraic_eqs.iter().enumerate() {
            // The equation under reduction cannot define its own
            // way out: `u = 3` must be read through `u = 2*x`.
            if index == eq {
                continue;
            }
            if let (Expr::Ref(name), other) | (other, Expr::Ref(name)) = (l, r) {
                if unknowns.contains(name) {
                    candidates.push((name.clone(), simplify(other)));
                }
            }
            let mut named = Vec::new();
            l.collect_refs(&mut named);
            r.collect_refs(&mut named);
            named.sort_unstable();
            named.dedup();
            for name in named {
                if !unknowns.iter().any(|u| u == name) {
                    continue;
                }
                let answer = solved_for
                    .entry((index, name.to_string()))
                    .or_insert_with(|| {
                        // Solved out of a connection equation, an
                        // equality arrives wrapped in the signs it
                        // was moved across and the coefficient it
                        // was divided by: `-p.i + r.p.i = 0` gives
                        // `-(-r.n.i)/-1`. Folded here, what is a
                        // plain name is written as one, and
                        // everything after this reads the shape
                        // rather than the wrapping.
                        solve_linear_for(l, r, name).map(|solved| simplify(&solved))
                    });
                if let Some(solved) = answer {
                    candidates.push((name.to_string(), solved.clone()));
                }
            }
        }

        // Definitions to differentiate through, built to a fixpoint so
        // the graph is acyclic and grounds out in states, parameters
        // and whatever is already grounded. A definition is accepted
        // only once everything it references is itself grounded, which
        // is what keeps `a := b` and `b := a` from chasing each other.
        let settle = |grounded: &HashMap<String, (Expr, Expr)>| {
            let mut accepted: HashMap<String, Expr> = HashMap::new();
            loop {
                let mut progress = false;
                for (name, expr) in &candidates {
                    // A name the implicit rule already grounds keeps
                    // that grounding. Taking a definition for it as
                    // well is how a two-name cycle gets built: `i = p.i`
                    // and `p.i = i` are both candidates, and with `i`
                    // grounded implicitly both would be accepted and
                    // chase each other until the depth guard fired.
                    if accepted.contains_key(name) || grounded.contains_key(name) {
                        continue;
                    }
                    let mut refs = Vec::new();
                    expr.collect_refs(&mut refs);
                    let ok = refs.iter().all(|r| {
                        *r != name
                            && (!unknowns.iter().any(|u| u == *r)
                                || accepted.contains_key(*r)
                                || grounded.contains_key(*r))
                    });
                    if ok {
                        accepted.insert(name.clone(), expr.clone());
                        progress = true;
                    }
                }
                if !progress {
                    break;
                }
            }
            accepted
        };

        // The unknowns nothing above grounds. `Psi = Linf*i +
        // c*atan(i/Ipar)` determines the current and cannot be solved
        // for it, so the fixpoint has nothing to say and the walk
        // refused the whole model. Its equation is kept whole instead,
        // for the implicit function theorem to differentiate.
        //
        // Read after the first settling rather than before it, because
        // the names that need this are exactly the ones a rearrangement
        // *appears* to define and does not: `i = p.i` and `p.i = i` are
        // each a definition of the other and ground nothing, so a name
        // tested against the candidate list alone is never offered
        // here. What settles is the test.
        //
        // Two conditions, and the second was paid for. The name must be
        // the single unsettled one in the equation - an equation naming
        // two determines neither on its own, and dividing by a slope
        // belonging to a different variable is a wrong number where a
        // refusal was owed. And the equation must be one no
        // rearrangement solves: `0 = p.i + n.i` names one unsettled
        // current and is perfectly linear in it, so taken here it would
        // answer `der(p.i)` with `-der(n.i)` and go round the circuit
        // for ever rather than reach the flux that actually moves. What
        // this rule is for is the equation that *cannot* be rearranged,
        // which is the one carrying the physics.
        let empty = HashMap::new();
        let settled = settle(&empty);
        let implicit_defs: HashMap<String, (Expr, Expr)> = if implicit_enabled() {
            let mut found: HashMap<String, (Expr, Expr)> = HashMap::new();
            for (index, (l, r)) in algebraic_eqs.iter().enumerate() {
                if index == eq {
                    continue;
                }
                let mut named = Vec::new();
                l.collect_refs(&mut named);
                r.collect_refs(&mut named);
                named.sort_unstable();
                named.dedup();
                let unsettled: Vec<&str> = named
                    .iter()
                    .copied()
                    .filter(|n| {
                        unknowns.iter().any(|u| u == *n)
                            && !dummies.contains_key(*n)
                            && !settled.contains_key(*n)
                    })
                    .collect();
                if let [only] = unsettled[..] {
                    if solve_linear_for(l, r, only).is_none() {
                        found
                            .entry(only.to_string())
                            .or_insert_with(|| (l.clone(), r.clone()));
                    }
                }
            }
            found
        } else {
            HashMap::new()
        };

        // Settled again, now that the implicit names count as ground:
        // `p.i = i` is a definition once `i` has one.
        let mut alg_defs = if implicit_defs.is_empty() {
            settled
        } else {
            settle(&implicit_defs)
        };
        for (minted, value) in &minted_defs {
            alg_defs
                .entry(minted.clone())
                .or_insert_with(|| value.clone());
        }

        if std::env::var_os("OXIDELICA_DEFS_PROBE").is_some() {
            eprintln!(
                "defs-probe: reduction {reductions} on {lhs:?} = {rhs:?}: {} candidates, {} settled, {} implicit, {} dummies",
                candidates.len(),
                alg_defs.len(),
                implicit_defs.len(),
                dummies.len()
            );
            let mut ds: Vec<&String> = dummies.keys().collect();
            ds.sort();
            eprintln!("defs-probe:   dummies: {ds:?}");
            for (name, expr) in &candidates {
                if !alg_defs.contains_key(name) {
                    eprintln!("defs-probe:   candidate not settled: {name} := {expr:?}");
                }
            }
        }

        let residual = Expr::Bin(
            oxidelica_parser::BinOp::Sub,
            Box::new(lhs.clone()),
            Box::new(rhs.clone()),
        );
        let derivative = match differentiate(
            &residual,
            &DiffTarget::Time {
                state_rhs: &*state_rhs,
                params,
                dummies: &dummies,
                alg_defs: &alg_defs,
                implicit_defs: &implicit_defs,
                holding: &[],
            },
        ) {
            Ok(d) => {
                let folded = simplify(&d);
                let mut nodes = 0usize;
                folded.for_each(&mut |_| nodes += 1);
                let ceiling = max_constraint_nodes();
                if nodes > ceiling {
                    return err(format!(
                        "structurally singular model: differentiating the equation \
                 {lhs:?} = {rhs:?} grew to {nodes} terms at reduction {reductions}, \
                 past the {ceiling} this compiler will carry"
                    ));
                }
                if std::env::var_os("OXIDELICA_GROWTH_PROBE").is_some() {
                    eprintln!(
                        "growth-probe: {reductions}\t{}",
                        format!("{folded:?}").len()
                    );
                }
                folded
            }
            Err(reason) => {
                // The reason first, the equation after it. Written the
                // other way the equation is a tree printed in full - some
                // of them run for lines - and the reason, which is the
                // only part that says what to do, fell off the end of
                // every list and every terminal. A dozen models looked
                // like a kind with no cause given.
                return err(format!(
                    "structurally singular model: {reason}, differentiating the equation \
                 {lhs:?} = {rhs:?}"
                ));
            }
        };

        // Every definition the walk gave a name to needs that name's
        // equation in the system, once. Minted afresh each reduction,
        // so a name already carrying its equation is skipped rather
        // than given a second one, which would unbalance the model.
        for (minted, value) in take_minted_derivatives() {
            if minted_defs.contains_key(&minted) {
                continue;
            }
            // A name the model already carries is not this walk's to
            // define: `der(x)` is an unknown in its own right wherever
            // the model wrote `der(x)` itself, and defining it a second
            // time would state one equation too many.
            if unknowns.iter().any(|u| u == &minted) || states.iter().any(|s| s == &minted) {
                continue;
            }
            let value = simplify(&value);
            minted_defs.insert(minted.clone(), value.clone());
            unknowns.push(minted.clone());
            algebraic_eqs.push((Expr::Ref(minted), value));
        }

        // A name the walk answered with a bare `der(x)` needs the
        // equation that determines `x`, differentiated. Substitution
        // could not find it - round a circuit it never terminates -
        // but the matching assigns every unknown exactly one equation,
        // and that is the one Pantelides differentiates.
        //
        // The matching in hand stopped at the equation under
        // reduction, so it is finished first over the rest: a name
        // matched by nobody even then is one nothing determines, and
        // that is the old refusal in its old words.
        //
        // Each supplier is differentiated at most once per reduction,
        // and differentiating it may name further unknowns, so the
        // loop runs to emptiness. The ledger is exact by construction:
        // one name in, one unknown and one equation out.
        let mut needed = take_needed_derivatives();
        // The equation the matching assigns to each name the walk had
        // to mint for. `choose_the_victim` reaches through these as it
        // already reaches through implicit ones: minting takes the
        // derivative past a name, and a reach that stops there reports
        // a constraint that does pin a state as pinning none.
        let mut matched_defs: HashMap<String, (Expr, Expr)> = HashMap::new();
        if !needed.is_empty() {
            let mut matched_eq = matched_eq.clone();
            // Only the equations the matching was built over. The
            // minting above pushed equations of its own onto
            // `algebraic_eqs`, and those have no row in `eq_vars`:
            // each defines a name outright and so needs no matching.
            for other in 0..eq_vars.len() {
                if other == eq || matched_eq.contains(&Some(other)) {
                    continue;
                }
                let mut visited = vec![false; n_alg];
                try_match(other, &eq_vars, &mut matched_eq, &mut visited);
            }
            let mut supplied: Vec<usize> = Vec::new();
            while let Some(name) = needed.pop() {
                let minted = derivative_name(&name);
                if minted_defs.contains_key(&minted)
                    || unknowns.iter().any(|u| u == &minted)
                    || states.iter().any(|s| s == &minted)
                {
                    continue;
                }
                let index = unknowns.iter().position(|u| u == &name);
                let Some(source) = index.and_then(|i| matched_eq.get(i).copied().flatten()) else {
                    return err(format!(
                        "structurally singular model: no equation determines `{name}`, \
                 whose derivative the equation {lhs:?} = {rhs:?} needs"
                    ));
                };
                if supplied.contains(&source) {
                    return err(format!(
                        "structurally singular model: the equation determining `{name}` \
                 has already been differentiated in this reduction"
                    ));
                }
                supplied.push(source);
                let (sl, sr) = algebraic_eqs[source].clone();
                matched_defs.insert(name.clone(), (sl.clone(), sr.clone()));
                let source_residual = Expr::Bin(
                    oxidelica_parser::BinOp::Sub,
                    Box::new(sl.clone()),
                    Box::new(sr.clone()),
                );
                let supply = match differentiate(
                    &source_residual,
                    &DiffTarget::Time {
                        state_rhs: &*state_rhs,
                        params,
                        dummies: &dummies,
                        alg_defs: &alg_defs,
                        implicit_defs: &implicit_defs,
                        holding: &[],
                    },
                ) {
                    Ok(d) => simplify(&d),
                    Err(reason) => {
                        return err(format!(
                            "structurally singular model: {reason}, differentiating the equation \
                     {sl:?} = {sr:?} that determines `{name}`"
                        ));
                    }
                };
                for (also, value) in take_minted_derivatives() {
                    if minted_defs.contains_key(&also)
                        || unknowns.iter().any(|u| u == &also)
                        || states.iter().any(|s| s == &also)
                    {
                        continue;
                    }
                    let value = simplify(&value);
                    minted_defs.insert(also.clone(), value.clone());
                    unknowns.push(also.clone());
                    algebraic_eqs.push((Expr::Ref(also), value));
                }
                needed.extend(take_needed_derivatives());
                unknowns.push(minted);
                algebraic_eqs.push((supply, Expr::Number(0.0)));
            }
        }

        // Demote a state the constraint actually constrains, choosing
        // the one it determines most strongly.
        // Reached through the equations the matching supplied as well
        // as the implicit ones - both are equations that determine a
        // name without defining it, and the reach cannot tell them
        // apart nor should it.
        let reach_defs = if matched_defs.is_empty() {
            implicit_defs.clone()
        } else {
            let mut both = implicit_defs.clone();
            for (name, pair) in matched_defs {
                both.entry(name).or_insert(pair);
            }
            both
        };
        let victim = choose_the_victim(
            &residual,
            &lhs,
            &rhs,
            &states,
            &alg_defs,
            &reach_defs,
            &companions,
            start_env,
            at_time,
            &mut selection_records,
            anchored,
            state_rhs,
        )?;
        let dummy = derivative_name(&victim);
        let victim_rhs = state_rhs
            .remove(&victim)
            .expect("a state has a defining derivative");
        {
            let mut named = Vec::new();
            victim_rhs.collect_refs(&mut named);
            companions.extend(
                named
                    .into_iter()
                    .filter(|name| states.iter().any(|s| s == name))
                    .map(str::to_string),
            );
        }
        states.retain(|s| s != &victim);
        unknowns.push(victim.clone());
        dummies.insert(victim.clone(), dummy.clone());
        // A state whose derivative nothing stated on its own already
        // has an unknown of exactly this name, and the equations
        // holding it are already in the system: there is no former
        // state equation to hand the dummy, and adding one would say
        // `der(v) = der(v)`.
        if !matches!(&victim_rhs, Expr::Ref(name) if name == &dummy) {
            unknowns.push(dummy.clone());
            // The former state equation `der(v) = rhs` now determines
            // the dummy, and the differentiated constraint joins the
            // system.
            algebraic_eqs.push((Expr::Ref(dummy), victim_rhs));
        }
        algebraic_eqs.push((derivative, Expr::Number(0.0)));
    };
    Ok(Reduction {
        states,
        unknowns,
        algebraic_eqs,
        dummies,
        selection_records,
        matched_eq,
        eq_vars,
        n_alg,
    })
}

/// Which state a constraint demotes.
///
/// The choice is a pivot: the constraint has to *determine* the
/// demoted variable, so the state with the largest sensitivity at the
/// start point wins. The selection is static, and models needing it to
/// change mid-run - a pendulum swinging full circle - are the known
/// limit of this implementation.
///
/// Moved out of `reduce_index` unchanged.
#[allow(clippy::too_many_arguments)]
fn choose_the_victim(
    residual: &Expr,
    lhs: &Expr,
    rhs: &Expr,
    states: &[String],
    alg_defs: &HashMap<String, Expr>,
    implicit_defs: &HashMap<String, (Expr, Expr)>,
    companions: &[String],
    start_env: &HashMap<String, f64>,
    at_time: f64,
    selection_records: &mut Vec<(Expr, String, Vec<String>)>,
    anchored: &dyn Fn(&str) -> bool,
    state_rhs: &HashMap<String, Expr>,
) -> Result<String, SimError> {
    // Demote a state the constraint actually constrains. The
    // choice is a pivot: the constraint has to *determine* the
    // demoted variable, so prefer the state with the largest
    // sensitivity at the start point. (The selection is static;
    // models that need it to change mid-run - a pendulum swinging
    // full circle - are the known limit of this implementation.)
    // Reachable states: the constraint may pin a state only
    // indirectly, through the definition of an algebraic unknown
    // (`u = 3` with `u = 2*x` constrains x).
    let mut reachable: Vec<String> = Vec::new();
    {
        let mut queue: Vec<String> = Vec::new();
        let mut direct = Vec::new();
        residual.collect_refs(&mut direct);
        queue.extend(direct.into_iter().map(str::to_string));
        let mut seen: Vec<String> = Vec::new();
        while let Some(name) = queue.pop() {
            if seen.contains(&name) {
                continue;
            }
            seen.push(name.clone());
            if states.iter().any(|s| s == &name) {
                reachable.push(name.clone());
            } else if let Some(definition) = alg_defs.get(&name) {
                let mut more = Vec::new();
                definition.collect_refs(&mut more);
                queue.extend(more.into_iter().map(str::to_string));
            } else if let Some((l, r)) = implicit_defs.get(&name) {
                // The same reach through an equation that determines a
                // name without defining it. Left out, the walk stops at
                // the current and never sees the flux behind it, so a
                // constraint that does pin a state is reported as
                // pinning none - the differentiation succeeds and the
                // model is refused one line later, which is the worse
                // half of a fix that only went halfway.
                let mut more = Vec::new();
                l.collect_refs(&mut more);
                r.collect_refs(&mut more);
                queue.extend(more.into_iter().map(str::to_string));
            }
        }
    }
    let sensitivity = |name: &str| -> f64 {
        differentiate(residual, &DiffTarget::Variable(name))
            .ok()
            .map(|d| simplify(&d))
            .and_then(|d| {
                eval(
                    &d,
                    &EvalCtx {
                        vars: start_env,
                        time: at_time,
                        programs: None,
                        depth: 0,
                    },
                )
                .ok()
            })
            .map(f64::abs)
            .unwrap_or(0.0)
    };
    // Companions of earlier victims first, by sensitivity; anything
    // else only when no companion is constrained here at all.
    let favoured: Vec<String> = reachable
        .iter()
        .filter(|name| companions.contains(name) && sensitivity(name) > 0.0)
        .cloned()
        .collect();
    let candidates = if favoured.is_empty() {
        reachable
    } else {
        favoured
    };
    // A state the model anchored - a start it declared as fixed - is
    // the worst thing to demote: demoted, its value is solved from
    // the constraints, and the initial condition the model wrote
    // about it is then contradicted rather than honoured. Prefer any
    // other state the constraint reaches, and fall back to the
    // anchored ones only when the constraint reaches nothing else.
    //
    // The anchor is not always at the level the choice is made at.
    // Demoting a position commits the next differentiation to the
    // velocity that position's own equation names, because that
    // velocity becomes the companion the level below prefers - and by
    // then it may be the only candidate there is, so the protection
    // arrives at a level where it can no longer do anything. A state
    // therefore counts as anchored when anything down its chain of
    // derivatives is: `s` whose `der(s) = v` with `v` fixed is as bad
    // a victim as `v` itself, and refusing it here is what leaves the
    // level below a free choice to make.
    let anchored_below = |name: &str| -> bool {
        let mut queue = vec![name.to_string()];
        let mut seen: Vec<String> = Vec::new();
        while let Some(here) = queue.pop() {
            if seen.contains(&here) {
                continue;
            }
            if anchored(&here) {
                return true;
            }
            seen.push(here.clone());
            let Some(rhs) = state_rhs.get(&here) else {
                continue;
            };
            let mut named = Vec::new();
            rhs.collect_refs(&mut named);
            queue.extend(
                named
                    .into_iter()
                    .filter(|n| states.iter().any(|s| s == n))
                    .map(str::to_string),
            );
        }
        false
    };
    let free: Vec<String> = candidates
        .iter()
        .filter(|name| !anchored_below(name))
        .cloned()
        .collect();
    let candidates = if free.is_empty() { candidates } else { free };
    // The runtime monitor compares the victim against exactly the
    // set the pivot weighed - alternatives of another derivative
    // level would make a healthy selection look wrong.
    let all_candidates = candidates.clone();
    let Some(victim) = candidates.into_iter().max_by(|a, b| {
        sensitivity(a)
            .partial_cmp(&sensitivity(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    }) else {
        return err(format!(
        "structurally singular model: equation {lhs:?} = {rhs:?} constrains no state, so index reduction cannot help"
    ));
    };

    selection_records.push((residual.clone(), victim.clone(), all_candidates));

    Ok(victim)
}

/// The equations of a model, sorted: what each state's derivative is,
/// and everything else.
///
/// The third part names the derivatives no equation stated on its own:
/// each is carried as an algebraic unknown of its own.
type SortedEquations = (HashMap<String, Expr>, Vec<(Expr, Expr)>, Vec<String>);

/// The name an unstated derivative is carried under.
///
/// It is spelled the way the model writes it, which no identifier can
/// be, so nothing a model declares can collide with it. Index
/// reduction already names a demoted state's derivative this way, so a
/// derivative is read by one name wherever it came from.
pub(crate) fn derivative_name(state: &str) -> String {
    format!("der({state})")
}

/// Replace every `der(x)` with the unknown standing for it.
fn name_derivatives(expr: &Expr, seen: &mut Vec<String>) -> Expr {
    if let Some(state) = expr.as_der_of() {
        let name = derivative_name(state);
        if !seen.iter().any(|s| s == state) {
            seen.push(state.to_string());
        }
        return Expr::Ref(name);
    }
    match expr {
        Expr::Call(name, args) => {
            let mut out = Vec::with_capacity(args.len());
            for arg in args {
                out.push(name_derivatives(arg, seen));
            }
            Expr::Call(name.clone(), out)
        }
        Expr::Neg(inner) => Expr::Neg(Box::new(name_derivatives(inner, seen))),
        Expr::Not(inner) => Expr::Not(Box::new(name_derivatives(inner, seen))),
        Expr::Bin(op, l, r) => {
            let left = name_derivatives(l, seen);
            Expr::Bin(*op, Box::new(left), Box::new(name_derivatives(r, seen)))
        }
        Expr::Rel(op, l, r) => {
            let left = name_derivatives(l, seen);
            Expr::Rel(*op, Box::new(left), Box::new(name_derivatives(r, seen)))
        }
        Expr::And(l, r) => {
            let left = name_derivatives(l, seen);
            Expr::And(Box::new(left), Box::new(name_derivatives(r, seen)))
        }
        Expr::Or(l, r) => {
            let left = name_derivatives(l, seen);
            Expr::Or(Box::new(left), Box::new(name_derivatives(r, seen)))
        }
        Expr::If(c, a, b) => {
            let condition = name_derivatives(c, seen);
            let then = name_derivatives(a, seen);
            Expr::If(
                Box::new(condition),
                Box::new(then),
                Box::new(name_derivatives(b, seen)),
            )
        }
        other => other.clone(),
    }
}

/// Sort the equations into the ones that give a state its derivative
/// and the ones that are algebraic.
///
/// `der(x)` standing alone on one side is what makes the first kind
/// recognisable, and an equation stating one in place - `i = C *
/// der(v)` - is rearranged into that form. What is left over holds a
/// derivative nothing states on its own: `der(x) + der(y) = 1` says
/// something about two of them at once, and no rearrangement gives
/// either. Those are carried as algebraic unknowns, one per
/// derivative, and the equations that hold them stay as they are -
/// the matching, the tearing and the solver then treat a derivative
/// like any other unknown, which is what a DAE is.
fn split_equations(
    equations: &[EquationItem],
    continuous: &[&str],
) -> Result<SortedEquations, SimError> {
    let mut state_rhs: HashMap<String, Expr> = HashMap::new();
    let mut algebraic_eqs: Vec<(Expr, Expr)> = Vec::new();

    for EquationItem { lhs, rhs, .. } in equations {
        // der(v) = expr  |  expr = der(v)
        let (target, value) = if let Some(v) = lhs.as_der_of() {
            (Some(v), rhs)
        } else if let Some(v) = rhs.as_der_of() {
            (Some(v), lhs)
        } else {
            (None, rhs)
        };
        // `der(x) = der(y)` states neither on its own: it is a
        // relation between two derivatives, and belongs with the
        // equations that are solved rather than stepped with.
        if let Some(state) = target.filter(|_| !value.contains_der()) {
            if !continuous.contains(&state) {
                return err(format!(
                    "der({state}): {state} is not a continuous variable"
                ));
            }
            if state_rhs.insert(state.to_string(), value.clone()).is_some() {
                return err(format!("two equations for der({state})"));
            }
            continue;
        }
        algebraic_eqs.push((lhs.clone(), rhs.clone()));
    }

    // What is left may still hold a derivative. Where an equation is
    // the only thing saying what one comes to - `i = C * der(v)`, which
    // is how a library states the relation - it is rearranged into the
    // form a solver steps with. An equation that merely *uses* a
    // derivative another equation already defines is not touched:
    // `y = der(x) + 1` alongside `der(x) = 1` says what `y` is, and
    // reading it the other way would claim a second definition of
    // `der(x)`. Equations are taken in the order they were written, so
    // which one defines a derivative is settled and repeatable.
    let mut left_over = Vec::new();
    for (lhs, rhs) in std::mem::take(&mut algebraic_eqs) {
        let taken = isolate_der(&lhs, &rhs)
            .filter(|(state, value)| !state_rhs.contains_key(state) && !value.contains_der());
        match taken {
            Some((state, value)) => {
                if !continuous.contains(&state.as_str()) {
                    return err(format!(
                        "der({state}): {state} is not a continuous variable"
                    ));
                }
                state_rhs.insert(state, value);
            }
            None => left_over.push((lhs, rhs)),
        }
    }
    for (lhs, rhs) in left_over {
        algebraic_eqs.push((lhs, rhs));
    }

    // What still holds a derivative gives it a name and keeps the
    // equation. The state stays a state - the run integrates it - and
    // its right-hand side is the unknown the algebraic layer solves
    // for.
    let mut implicit: Vec<String> = Vec::new();
    // A derivative another equation already states, and this one only
    // reads: `y = der(x) + 1` beside `der(x) = 3`. It is named the same
    // way, and what it is worth is said by an equation of its own, so
    // the balance is kept and nothing is defined twice.
    let mut read_only: Vec<String> = Vec::new();
    for (lhs, rhs) in &mut algebraic_eqs {
        if !lhs.contains_der() && !rhs.contains_der() {
            continue;
        }
        let mut seen = Vec::new();
        *lhs = name_derivatives(lhs, &mut seen);
        *rhs = name_derivatives(rhs, &mut seen);
        for state in seen {
            if !continuous.contains(&state.as_str()) {
                return err(format!(
                    "der({state}): {state} is not a continuous variable"
                ));
            }
            if state_rhs.contains_key(&state) {
                if !read_only.iter().any(|s| s == &state) {
                    read_only.push(state);
                }
                continue;
            }
            if !implicit.iter().any(|s| s == &state) {
                implicit.push(state);
            }
        }
    }
    for state in &implicit {
        state_rhs.insert(state.clone(), Expr::Ref(derivative_name(state)));
    }
    for state in &read_only {
        let value = state_rhs
            .get(state)
            .expect("a derivative another equation stated")
            .clone();
        algebraic_eqs.push((Expr::Ref(derivative_name(state)), value));
    }
    implicit.extend(read_only);
    Ok((state_rhs, algebraic_eqs, implicit))
}

/// Get the derivative out of an equation that states it in place:
/// `i = C * der(v)` is the same equation as `der(v) = i / C`, and the
/// second is the one an explicit solver can step with.
///
/// The derivative has to occur once, on one side and nowhere on the
/// other. Then the operations between it and the top of that side are
/// undone one at a time, each moving to the other side as its
/// opposite. Only the ones that can be undone without a case analysis
/// are tried: sums, differences, products, quotients and negation.
/// Anything else - a derivative under a power, inside a call, or on
/// both sides - is left alone and refused where it was.
fn isolate_der(lhs: &Expr, rhs: &Expr) -> Option<(String, Expr)> {
    let (mut held, mut other) = match (lhs.contains_der(), rhs.contains_der()) {
        (true, false) => (lhs.clone(), rhs.clone()),
        (false, true) => (rhs.clone(), lhs.clone()),
        _ => return None,
    };
    for _ in 0..MAX_ISOLATION {
        if let Some(state) = held.as_der_of() {
            return Some((state.to_string(), other));
        }
        let (next, moved) = match held {
            // `-a = other` is `a = -other`.
            Expr::Neg(inner) => (*inner, Expr::Neg(Box::new(other))),
            Expr::Bin(op, a, b) => {
                let left = a.contains_der();
                // The side without the derivative is what moves, so
                // exactly one of them may hold it.
                if left == b.contains_der() {
                    return None;
                }
                match (op, left) {
                    // a + b = other  ->  a = other - b
                    (BinOp::Add, true) => (*a, Expr::Bin(BinOp::Sub, Box::new(other), b)),
                    (BinOp::Add, false) => (*b, Expr::Bin(BinOp::Sub, Box::new(other), a)),
                    // a - b = other  ->  a = other + b  |  b = a - other
                    (BinOp::Sub, true) => (*a, Expr::Bin(BinOp::Add, Box::new(other), b)),
                    (BinOp::Sub, false) => (*b, Expr::Bin(BinOp::Sub, a, Box::new(other))),
                    // a * b = other  ->  a = other / b
                    (BinOp::Mul, true) => (*a, Expr::Bin(BinOp::Div, Box::new(other), b)),
                    (BinOp::Mul, false) => (*b, Expr::Bin(BinOp::Div, Box::new(other), a)),
                    // a / b = other  ->  a = other * b  |  b = a / other
                    (BinOp::Div, true) => (*a, Expr::Bin(BinOp::Mul, Box::new(other), b)),
                    (BinOp::Div, false) => (*b, Expr::Bin(BinOp::Div, a, Box::new(other))),
                    // A derivative under a power would need a root,
                    // and which root depends on the exponent.
                    (BinOp::Pow, _) => return None,
                }
            }
            _ => return None,
        };
        held = next;
        other = moved;
    }
    None
}

/// How many operations deep a derivative may sit and still be got out.
/// A physical relation states it within a few; a longer chain is a
/// sign that this is not the shape being looked for.
const MAX_ISOLATION: usize = 8;

/// The states, and the algebraic unknowns left over.
fn unknowns_of(
    continuous: &[&str],
    state_rhs: &HashMap<String, Expr>,
    implicit: &[String],
) -> (Vec<String>, Vec<String>) {
    let states = continuous
        .iter()
        .filter(|n| state_rhs.contains_key(**n))
        .map(|n| n.to_string())
        .collect();
    let mut unknowns: Vec<String> = continuous
        .iter()
        .filter(|n| !state_rhs.contains_key(**n))
        .map(|n| n.to_string())
        .collect();
    // A derivative no equation stated on its own is solved for beside
    // them, which is what keeps the equation holding it matched.
    unknowns.extend(implicit.iter().map(|state| derivative_name(state)));
    (states, unknowns)
}

/// Every name the equations use must be one the model declared.
fn check_references(
    state_rhs: &HashMap<String, Expr>,
    algebraic_eqs: &[(Expr, Expr)],
    continuous: &[&str],
    params: &HashMap<String, f64>,
    discretes: &[String],
) -> Result<(), SimError> {
    let mut refs = Vec::new();
    for expr in state_rhs.values() {
        expr.collect_refs(&mut refs);
    }
    for (lhs, rhs) in algebraic_eqs {
        lhs.collect_refs(&mut refs);
        rhs.collect_refs(&mut refs);
    }
    if let Some(bad) = refs.iter().find(|r| {
        !continuous.contains(r)
            && !params.contains_key(**r)
            && !discretes.iter().any(|d| d == **r)
            // `$pre.x`, `$initial` and `$sampleN` are supplied by the
            // event machinery, not by the equations.
            && !r.starts_with('$')
            // `der(x)` is a name the compiler made for a derivative
            // nothing stated on its own, not one a model wrote.
            && !(r.starts_with("der(") && r.ends_with(')'))
    }) {
        return err(format!("unknown variable `{bad}` in equation"));
    }
    Ok(())
}

/// Which branch of each run-time `if` equation holds at this point.
///
/// A condition usually names a variable an ordinary equation defines -
/// `energised = sin(...) >= 0` - and start attributes say nothing
/// about those, so plain `name = expr` definitions are followed to a
/// fixpoint first. Then the conditions can be asked, before anything
/// has been matched or solved.
fn settle_modes(
    model: &Model,
    start_env: &HashMap<String, f64>,
    mode_time: f64,
    resuming: bool,
) -> Result<Vec<usize>, SimError> {
    let mut mode_env = start_env.clone();
    for _ in 0..MAX_DEFINITION_PASSES {
        let mut progress = false;
        for equation in &model.equations {
            let Expr::Ref(name) = &equation.lhs else {
                continue;
            };
            if resuming && mode_env.contains_key(name) {
                // A continuation already knows this from the run.
                continue;
            }
            if let Ok(value) = eval(
                &equation.rhs,
                &EvalCtx {
                    vars: &mode_env,
                    time: mode_time,
                    programs: None,
                    depth: 0,
                },
            ) {
                if mode_env.insert(name.clone(), value) != Some(value) {
                    progress = true;
                }
            }
        }
        if !progress {
            break;
        }
    }
    let start_ctx = EvalCtx {
        vars: &mode_env,
        time: mode_time,
        programs: None,
        depth: 0,
    };
    let mut modes: Vec<usize> = Vec::new();
    for conditional in &model.conditional {
        let mut taken = conditional.branches.len() - 1;
        for (index, condition) in conditional.conditions.iter().enumerate() {
            if eval(condition, &start_ctx)? != 0.0 {
                taken = index;
                break;
            }
        }
        modes.push(taken);
    }
    Ok(modes)
}

/// What every variable stands at, at the point being compiled for.
///
/// A fresh compilation reads the start attributes; a continuation
/// reads the run. The pivot that decides which states to demote works
/// from these numbers, so they have to be the numbers of *here*.
fn values_at_this_point(
    model: &Model,
    params: &HashMap<String, f64>,
    discretes: &[String],
    discrete_start: &[f64],
    resume: &Option<ResumePoint>,
) -> HashMap<String, f64> {
    let resumed = |name: &str| -> Option<f64> {
        resume
            .as_ref()
            .and_then(|point| point.values.get(name))
            .copied()
    };
    let mut env = params.clone();
    for (name, value) in discretes.iter().zip(discrete_start) {
        env.insert(name.clone(), resumed(name).unwrap_or(*value));
    }
    for component in &model.components {
        if component.variability == Variability::Continuous {
            let value = resumed(&component.name)
                .or_else(|| {
                    component.start.as_ref().and_then(|expr| {
                        eval(
                            expr,
                            &EvalCtx {
                                vars: params,
                                time: 0.0,
                                programs: None,
                                depth: 0,
                            },
                        )
                        .ok()
                    })
                })
                .unwrap_or(0.0);
            env.insert(component.name.clone(), value);
        }
    }
    env
}

/// The discrete variables of a model, and the value each one starts
/// from.
///
/// A variable is discrete when it says so or when a `when` clause
/// assigns it: either way it keeps its value between events, so the
/// continuous part treats it as known. They come back in declaration
/// order, which is what keeps the result columns steady.
/// The discrete layer: the variables, where each one starts, and
/// which initial equations were spent saying so.
type DiscreteLayer = (Vec<String>, Vec<f64>, Vec<usize>, Vec<usize>);

fn discrete_layer(
    model: &Model,
    params: &HashMap<String, f64>,
    resume: &Option<ResumePoint>,
) -> Result<DiscreteLayer, SimError> {
    // 1b. The discrete layer. A variable is discrete when it says so or
    // when a `when` clause assigns it: either way it keeps its value
    // between events, so the continuous part treats it as known.
    let mut discrete_names: Vec<String> = Vec::new();
    // A Boolean or an Integer is discrete-valued by its type, whatever
    // assigns it, and an equation whose one side is such a name is
    // that name's definition: `off = s < 0` says what the switch is,
    // and the relation inside it is a generator of events rather than
    // something a continuous solver may walk through. The pair moves
    // together - the name to the discrete layer, its equation to the
    // rounds of the event iteration - so the count either side of the
    // move is what it was.
    let discrete_valued: Vec<&str> = model
        .components
        .iter()
        .filter(|c| {
            (c.type_name == "Boolean" || c.type_name == "Integer")
                // A parameter or a constant of that type is settled
                // before the run and is nobody's unknown; what moves
                // here is a variable the model solves for.
                && c.variability == Variability::Continuous
        })
        .map(|c| c.name.as_str())
        .collect();
    // A name that decides which branch of an `if` equation is in
    // force is settled before the model is built, and the mode is
    // compiled with the equations actually in force: that is exact
    // where a solver would only be close, and moving such a name into
    // the event iteration would take the exactness away. Only the
    // ones nothing branches on move.
    let branched_on: Vec<&str> = model
        .conditional
        .iter()
        .flat_map(|conditional| {
            let mut refs = Vec::new();
            for condition in &conditional.conditions {
                condition.collect_refs(&mut refs);
            }
            refs
        })
        .filter_map(|name| {
            discrete_valued
                .iter()
                .find(|known| ***known == *name)
                .copied()
        })
        .collect();
    let mut defined_here: Vec<usize> = Vec::new();
    for (at, equation) in model.equations.iter().enumerate() {
        // The pair moves only when the other side is a relation - the
        // knee of a switch, `s < 0` - which is what makes this an
        // event definition rather than an ordinary equation. A
        // Boolean equated to another Boolean is a connection, and it
        // belongs where the rest of the connection equations are: it
        // has no crossing of its own, and moving it would take an
        // equation from a set that still needs one.
        let decides = |other: &Expr| {
            let mut relations = Vec::new();
            collect_relations(other, &mut relations);
            !relations.is_empty()
        };
        let named = |side: &Expr, other: &Expr| match side {
            Expr::Ref(name) if decides(other) => discrete_valued
                .contains(&name.as_str())
                .then(|| name.clone()),
            _ => None,
        };
        let Some(name) =
            named(&equation.lhs, &equation.rhs).or_else(|| named(&equation.rhs, &equation.lhs))
        else {
            continue;
        };
        if branched_on.contains(&name.as_str()) {
            continue;
        }
        defined_here.push(at);
        if !discrete_names.contains(&name) {
            discrete_names.push(name);
        }
    }
    for clause in &model.when_clauses {
        for branch in &clause.branches {
            for action in &branch.actions {
                if let WhenAction::Assign(target, _) = action {
                    if !discrete_names.contains(target) {
                        discrete_names.push(target.clone());
                    }
                }
            }
        }
    }
    for name in &discrete_names {
        if !model.components.iter().any(|c| &c.name == name) {
            return err(format!(
                "`{name}` is assigned by a when clause but never declared"
            ));
        }
    }
    for component in &model.components {
        if component.variability == Variability::Discrete
            && !discrete_names.contains(&component.name)
        {
            return err(format!(
                "discrete variable `{}` is never assigned by a when clause",
                component.name
            ));
        }
    }
    // Declaration order, so the result columns are stable.
    let discretes: Vec<String> = model
        .components
        .iter()
        .filter(|c| discrete_names.contains(&c.name))
        .map(|c| c.name.clone())
        .collect();
    // An `initial algorithm` that assigns a discrete variable says
    // where that variable starts, and arrives here as an initial
    // equation naming it. That is a start value rather than a
    // condition on the states, so it is read out here and struck off
    // the list the initialisation is measured against. They are read
    // in the order written, each against what the ones before
    // settled, and one whose value only the run knows - a count taken
    // off a state - is struck off all the same, leaving the declared
    // start to stand.
    // A Boolean or an Integer keeps its value between events whatever
    // assigns it, so an initial equation naming one - or naming what
    // one was before the first event - says where it starts rather
    // than putting a condition on the states. The state graph writes
    // both: `active = true` for the step a graph begins in, and
    // `pre(newActive) = pre(localActive)` for the memory behind it.
    fn started(lhs: &Expr) -> Option<&String> {
        match lhs {
            Expr::Ref(named) => Some(named),
            Expr::Call(name, args) if name == "pre" => match args.first() {
                Some(Expr::Ref(named)) => Some(named),
                _ => None,
            },
            _ => None,
        }
    }
    let kept_between_events = |name: &String| {
        discrete_names.contains(name)
            || model
                .components
                .iter()
                .any(|c| &c.name == name && (c.type_name == "Boolean" || c.type_name == "Integer"))
    };
    let mut known = params.clone();
    let mut spent: Vec<usize> = Vec::new();
    for (at, equation) in model.initial_equations.iter().enumerate() {
        if let Some(named) = started(&equation.lhs) {
            if kept_between_events(named) {
                let vars = &known.clone();
                let now = EvalCtx {
                    vars,
                    time: 0.0,
                    programs: None,
                    depth: 0,
                };
                if let Ok(value) = eval(&equation.rhs, &now) {
                    known.insert(named.clone(), value);
                }
                spent.push(at);
            }
        }
    }
    let discrete_start: Vec<f64> = model
        .components
        .iter()
        .filter(|c| discrete_names.contains(&c.name))
        .map(|c| {
            resume
                .as_ref()
                .and_then(|point| point.values.get(&c.name))
                .copied()
                .or_else(|| known.get(&c.name).copied())
                .or_else(|| {
                    c.start.as_ref().or(c.binding.as_ref()).and_then(|expr| {
                        eval(
                            expr,
                            &EvalCtx {
                                vars: params,
                                time: 0.0,
                                programs: None,
                                depth: 0,
                            },
                        )
                        .ok()
                    })
                })
                .unwrap_or(0.0)
        })
        .collect();
    Ok((discretes, discrete_start, spent, defined_here))
}

/// Work out the value of every parameter and constant.
///
/// One may be written in terms of another, in any order, so they are
/// evaluated in passes: each pass settles whatever it can, and a pass
/// that settles nothing means what is left refers to itself or to
/// something that is not there.
/// The two boundary equations of every `spatialDistribution`.
///
/// Which end is the inflow depends on the direction, and so does which
/// end reads the profile: going forward, ξ = 0 is where the quantity
/// enters and ξ = 1 is where what entered a unit of `x` ago comes out.
/// Going backward the two swap over, which is why both readings are
/// kept and the condition picks between them.
fn with_transport_equations(model: &Model) -> Model {
    let mut out = model.clone();
    for (index, transport) in model.transports.iter().enumerate() {
        let pick = |forward: Expr, backward: Expr| {
            Expr::If(
                Box::new(transport.positive.clone()),
                Box::new(forward),
                Box::new(backward),
            )
        };
        let read = |name: String| Expr::Ref(name);
        out.equations.push(EquationItem {
            lhs: Expr::Ref(transport.out0.clone()),
            rhs: pick(
                transport.in0.clone(),
                read(format!("$carried_at_zero{index}")),
            ),
            origin: String::new(),
        });
        out.equations.push(EquationItem {
            lhs: Expr::Ref(transport.out1.clone()),
            rhs: pick(
                read(format!("$carried_at_one{index}")),
                transport.in1.clone(),
            ),
            origin: String::new(),
        });
    }
    out
}

/// The element of a written-out bound that a flat name is held to:
/// `x[2]` takes the second. A name with no subscript, with more than
/// one, or with a subscript that does not choose an element of what
/// was written, is answered with nothing rather than with a guess.
fn element_of<'a>(name: &str, items: &'a [Expr]) -> Option<&'a Expr> {
    let inside = name.strip_suffix(']')?.rsplit_once('[')?.1;
    let which: usize = inside.parse().ok()?;
    items.get(which.checked_sub(1)?)
}

/// Whether a bound is written over a whole array rather than over the
/// one value it is attached to.
fn names_an_array_maker(expr: &Expr) -> bool {
    match expr {
        Expr::Call(name, _) => matches!(name.as_str(), "zeros" | "ones" | "fill"),
        _ => false,
    }
}

/// The `min` and `max` attributes, as the assertions Modelica says they
/// are. A bound that is settled before the run is already refused by the
/// checker; these are for the values that only a run produces - a level
/// that drains past empty, a temperature that goes below absolute zero.
fn bound_asserts(model: &Model) -> Vec<(Expr, String)> {
    let mut out = Vec::new();
    for component in &model.components {
        for (bound, op, side) in [
            (&component.min, RelOp::Ge, "below its min of"),
            (&component.max, RelOp::Le, "above its max of"),
        ] {
            let Some(limit) = bound else { continue };
            // A bound written over a whole array - `Ron[m](min =
            // zeros(m))` - is one the flattener leaves standing,
            // since it does not come to a single value. Each element
            // of the array is its own component here, and comparing
            // one of them against the whole array is neither the
            // assertion Modelica means nor something the run can
            // evaluate: the call would reach the code generator as a
            // function nothing knows.
            if names_an_array_maker(limit) {
                continue;
            }
            // A bound written out - `x[3](min = {1, 1, 1})` - is
            // attached to the array as a whole, and each element of
            // that array is its own component here, named `x[2]` as
            // the flat model names it. The bound this element is held
            // to is the matching element of what was written, so the
            // subscript on the name chooses it. Anything else - a
            // matrix, a name whose subscript does not choose - has no
            // one value to compare against, and a comparison against
            // the whole array is not the assertion Modelica means.
            let limit = match limit {
                Expr::Array(items) => match element_of(&component.name, items) {
                    Some(one) => one,
                    None => continue,
                },
                one => one,
            };
            out.push((
                Expr::Rel(
                    op,
                    Box::new(Expr::Ref(component.name.clone())),
                    Box::new(limit.clone()),
                ),
                format!("`{}` went {side} {}", component.name, describe(limit)),
            ));
        }
    }
    out
}

/// A bound as it was written, for the message of the assertion above.
fn describe(expr: &Expr) -> String {
    match expr {
        Expr::Number(value) => format!("{value}"),
        Expr::Neg(inner) => format!("-{}", describe(inner)),
        Expr::Ref(name) => name.clone(),
        _ => "its limit".to_string(),
    }
}

/// What the parameters came to, the initial equations claimed for
/// them, and the ordinary equations that turned out to define one.
type Parameters = (HashMap<String, f64>, Vec<usize>, Vec<usize>);

fn evaluate_parameters(
    model: &Model,
    programs: &HashMap<String, ClassDef>,
) -> Result<Parameters, SimError> {
    let mut params: HashMap<String, f64> = HashMap::new();
    // What the first round could not settle, kept until the rounds
    // that might settle it have run.
    let mut stuck_on: Option<String> = None;
    let mut pending: Vec<(&str, &Expr)> = Vec::new();
    // The components whose `fixed` was written as an expression, held
    // until the parameters it names have values.
    let mut deferred: Vec<&oxidelica_parser::Component> = Vec::new();
    // The parameters the initialisation settles rather than the
    // declaration, and the initial equations claimed for them.
    let mut unknowns: Vec<&oxidelica_parser::Component> = Vec::new();
    let mut claimed: Vec<usize> = Vec::new();
    // The ordinary equations that turned out to define a parameter.
    let mut defined: Vec<usize> = Vec::new();
    for c in &model.components {
        if matches!(
            c.variability,
            Variability::Parameter | Variability::Constant
        ) {
            // `fixed = false` says the declaration is not where the
            // value comes from - unless it wrote one anyway, and then
            // the declaration wins and the language only asks for a
            // warning. A constant is settled by its declaration
            // whatever it says about `fixed`, since the language does
            // not let a constant be an unknown of anything.
            let asks_the_initialization = c.variability == Variability::Parameter
                && c.fixed == Some(false)
                && c.binding.is_none();
            if asks_the_initialization {
                unknowns.push(c);
                continue;
            }
            // A flag written as an expression - `Av(fixed = CvData ==
            // CvTypes.Av)` - is not known yet, because the names in it
            // are parameters this very round settles. Which side of
            // the question it falls on decides whether the parameter
            // is an unknown of the initialisation or wants a value
            // here, so it is put aside and asked once there is
            // something to ask with.
            if c.variability == Variability::Parameter
                && c.fixed.is_none()
                && c.fixed_expr.is_some()
                && c.binding.is_none()
            {
                deferred.push(c);
                continue;
            }
            match c.binding.as_ref().or(c.start.as_ref()) {
                Some(expr) => pending.push((&c.name, expr)),
                None => return err(format!("parameter {} has no value", c.name)),
            }
        }
    }
    loop {
        let before = pending.len();
        pending.retain(|(name, expr)| {
            match eval(
                expr,
                &EvalCtx {
                    vars: &params,
                    time: 0.0,
                    programs: Some(programs),
                    depth: 0,
                },
            ) {
                Ok(v) => {
                    params.insert((*name).to_string(), v);
                    false
                }
                Err(_) => true,
            }
        });
        if pending.is_empty() {
            break;
        }
        if pending.len() == before {
            // Which name is missing is the whole of the question, and
            // the list of what could not be worked out does not answer
            // it: every one of them is stuck on the same handful of
            // names, and reading the handful out of the expressions is
            // what a person would otherwise do by eye. A name nothing
            // declares is the usual cause, and a genuine cycle shows
            // up as a name that is pending rather than missing.
            let stuck: std::collections::BTreeSet<&str> =
                pending.iter().map(|(name, _)| *name).collect();
            let mut wanted: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for (_, value) in &pending {
                value.for_each(&mut |part| {
                    if let Expr::Ref(name) = part {
                        if !params.contains_key(name.as_str()) {
                            wanted.insert(name.clone());
                        }
                    }
                });
            }
            let undeclared: Vec<&String> = wanted
                .iter()
                .filter(|name| !stuck.contains(name.as_str()))
                .collect();
            let names: Vec<String> = pending
                .iter()
                .map(|(name, value)| format!("{name} = {}", value.describe()))
                .collect();
            // A call nothing worked out is the other usual cause, and
            // it names no free variable at all: a parameter written as
            // a function of literals waits on nobody, so calling that
            // a cycle names a shape the model does not have. What it
            // waits on is the call, so the call is what is said.
            let mut standing: std::collections::BTreeSet<String> =
                std::collections::BTreeSet::new();
            for (_, value) in &pending {
                value.for_each(&mut |part| {
                    if let Expr::Call(called, _) = part {
                        standing.insert(called.clone());
                    }
                });
            }
            let listed = |names: Vec<String>| names.join(", ");
            let because = match (undeclared.is_empty(), standing.is_empty()) {
                (false, _) => format!(
                    "nothing gives a value to {}",
                    listed(undeclared.iter().map(|name| format!("`{name}`")).collect())
                ),
                (true, false) => format!(
                    "nothing works out {}",
                    listed(
                        standing
                            .iter()
                            .map(|called| format!("`{called}`"))
                            .collect()
                    )
                ),
                (true, true) => "they wait on each other".to_string(),
            };
            // The two queues below - what the initialisation claims,
            // and what keeps its start - may be exactly what these are
            // waiting on: a parameter written on `p_ambient` waits for
            // a name the initialisation settles, and refusing here
            // kills the model before the round that would answer it
            // has begun. So the refusal is remembered and asked again
            // at the end, when there is nothing left to try.
            stuck_on = Some(format!(
                "cannot evaluate parameters [{}]: {because}",
                names.join(", ")
            ));
            break;
        }
    }

    // Now the flags written as expressions can be asked. A flag that
    // comes to false says the initialisation settles the parameter; a
    // flag that comes to true says the declaration does, and then the
    // start is where the value is. A flag that comes to nothing is
    // refused by name rather than guessed at: reading it as either
    // truth gives a wrong answer where a refusal is owed, and the two
    // wrong answers are a parameter solved for twice and a parameter
    // solved for never.
    for c in deferred {
        let flag = c.fixed_expr.as_ref().expect("a deferred flag was written");
        let context = EvalCtx {
            vars: &params,
            time: 0.0,
            programs: Some(programs),
            depth: 0,
        };
        let settled = match eval(flag, &context) {
            Ok(value) => value != 0.0,
            Err(_) => {
                return err(format!(
                    "parameter {} says `fixed = {}`, and that is not something \
                     the parameters settle",
                    c.name,
                    flag.describe()
                ))
            }
        };
        if settled {
            match c.start.as_ref() {
                Some(expr) => pending.push((&c.name, expr)),
                None => return err(format!("parameter {} has no value", c.name)),
            }
        } else {
            unknowns.push(c);
        }
    }
    // A start taken this way is usually a literal, but may stand on
    // another parameter, so it is settled by the same rounds.
    loop {
        let before = pending.len();
        pending.retain(|(name, expr)| {
            match eval(
                expr,
                &EvalCtx {
                    vars: &params,
                    time: 0.0,
                    programs: Some(programs),
                    depth: 0,
                },
            ) {
                Ok(v) => {
                    params.insert((*name).to_string(), v);
                    false
                }
                Err(_) => true,
            }
        });
        if pending.is_empty() || pending.len() == before {
            break;
        }
    }

    // What the initialisation settles. An equation is claimable where
    // it names the parameter on one side and the other side comes to a
    // number with what is settled so far - so `p = 7` is taken and
    // `x = p` is left to the state it is really about, whichever order
    // they were written in. Settling one may let another go, so they
    // are asked in rounds.
    loop {
        let mut progress = false;
        unknowns.retain(|c| {
            let taken = model
                .initial_equations
                .iter()
                .enumerate()
                .filter(|(at, _)| !claimed.contains(at))
                .find_map(|(at, equation)| {
                    let value = match (&equation.lhs, &equation.rhs) {
                        (Expr::Ref(named), value) | (value, Expr::Ref(named))
                            if *named == c.name =>
                        {
                            value
                        }
                        _ => return None,
                    };
                    let context = EvalCtx {
                        vars: &params,
                        time: 0.0,
                        programs: Some(programs),
                        depth: 0,
                    };
                    eval(value, &context).ok().map(|number| (at, number))
                });
            match taken {
                Some((at, number)) => {
                    claimed.push(at);
                    params.insert(c.name.clone(), number);
                    progress = true;
                    false
                }
                None => true,
            }
        });
        // And what an ordinary equation settles. A filter writes
        // `r[1] = -(2.5*cr[1])` among its equations, with `cr` left to
        // the initialisation and `r` to that line - so a parameter
        // whose declaration gives it no value can still be defined,
        // and by an equation that is not an initial one. The shape
        // asked for is the same: the parameter alone on a side, and
        // the other side a number with what is settled so far, which
        // is what keeps a state out of it. The equation has done its
        // work here and is dropped from the continuous set below,
        // where the parameter is not an unknown and counting the line
        // would leave one equation too many.
        unknowns.retain(|c| {
            let taken = model
                .equations
                .iter()
                .enumerate()
                .filter(|(at, _)| !defined.contains(at))
                .find_map(|(at, equation)| {
                    let value = match (&equation.lhs, &equation.rhs) {
                        (Expr::Ref(named), value) | (value, Expr::Ref(named))
                            if *named == c.name =>
                        {
                            value
                        }
                        _ => return None,
                    };
                    // An initial equation holds at t = 0 and evaluating
                    // it there is honest. An ordinary one holds
                    // throughout, so a side that names `time` has no
                    // one number to give: read at zero, `r = 2*time`
                    // would make `r` nothing but zero and take the
                    // line out of the continuous set, which is a wrong
                    // answer where the refusal that names `r` is owed.
                    if names_time(value) {
                        return None;
                    }
                    let context = EvalCtx {
                        vars: &params,
                        time: 0.0,
                        programs: Some(programs),
                        depth: 0,
                    };
                    eval(value, &context).ok().map(|number| (at, number))
                });
            match taken {
                Some((at, number)) => {
                    defined.push(at);
                    params.insert(c.name.clone(), number);
                    progress = true;
                    false
                }
                None => true,
            }
        });
        if unknowns.is_empty() || !progress {
            break;
        }
    }
    // What the initialisation settles without saying so outright.
    //
    // `(Lnom - Linf)/(Lzer - Linf) = Ipar/Inom*(pi/2 - atan(Ipar/Inom))`
    // is how a saturating inductor states its shape parameter, and no
    // rearrangement puts `Ipar` alone on a side. The round above asks
    // for a name against a number and finds neither, so the equation
    // stayed in the initialisation - where it counts against the
    // states, of which such a model may have none, and the whole model
    // was refused as an initialisation that is not square.
    //
    // The equation is a residual in one unknown, so it is solved as
    // one: Newton from the declared start, with the slope taken by
    // difference. Only where the equation names exactly one unsettled
    // parameter and everything else in it already has a value - an
    // equation naming two determines neither on its own, and a number
    // invented for one of them would be a wrong answer given as a
    // right one. A solve that does not converge leaves the parameter
    // to the round below, which falls back on its start.
    loop {
        let mut progress = false;
        unknowns.retain(|c| {
            let Some(start) = c.start.as_ref() else {
                return true;
            };
            fn context<'a>(
                vars: &'a HashMap<String, f64>,
                programs: &'a HashMap<String, ClassDef>,
            ) -> EvalCtx<'a> {
                EvalCtx {
                    vars,
                    time: 0.0,
                    programs: Some(programs),
                    depth: 0,
                }
            }
            let Ok(guess) = eval(start, &context(&params, programs)) else {
                return true;
            };
            let taken = model
                .initial_equations
                .iter()
                .enumerate()
                .filter(|(at, _)| !claimed.contains(at))
                .find_map(|(at, equation)| {
                    let mut named = Vec::new();
                    equation.lhs.collect_refs(&mut named);
                    equation.rhs.collect_refs(&mut named);
                    if !named.iter().any(|name| *name == c.name) {
                        return None;
                    }
                    if named
                        .iter()
                        .any(|name| *name != c.name && !params.contains_key(*name))
                    {
                        return None;
                    }
                    let residual = |value: f64| {
                        let mut vars = params.clone();
                        vars.insert(c.name.clone(), value);
                        let at = context(&vars, programs);
                        Ok::<f64, SimError>(eval(&equation.lhs, &at)? - eval(&equation.rhs, &at)?)
                    };
                    let mut value = guess;
                    for _ in 0..50 {
                        let f = residual(value).ok()?;
                        if f.abs() < 1e-12 {
                            return Some((at, value));
                        }
                        let step = 1e-7 * (1.0 + value.abs());
                        let slope = (residual(value + step).ok()? - f) / step;
                        if slope == 0.0 || !slope.is_finite() {
                            return None;
                        }
                        value -= f / slope;
                        if !value.is_finite() {
                            return None;
                        }
                    }
                    None
                });
            match taken {
                Some((at, number)) => {
                    claimed.push(at);
                    params.insert(c.name.clone(), number);
                    progress = true;
                    false
                }
                None => true,
            }
        });
        if unknowns.is_empty() || !progress {
            break;
        }
    }
    // One nothing settled keeps its start value, which is what the
    // language says a start is for where nothing else decides.
    for c in unknowns {
        match c.start.as_ref() {
            Some(start) => {
                let context = EvalCtx {
                    vars: &params,
                    time: 0.0,
                    programs: Some(programs),
                    depth: 0,
                };
                match eval(start, &context) {
                    Ok(number) => {
                        params.insert(c.name.clone(), number);
                    }
                    Err(_) => return err(format!("parameter {} has no value", c.name)),
                }
            }
            None => return err(format!("parameter {} has no value", c.name)),
        }
    }
    // Asked again, now that the initialisation has claimed what it
    // can and the starts have stood in for the rest: a parameter that
    // still has no value is one nothing in the model could give.
    // And one more round over what was left: the queues above may
    // have settled the very names it was waiting on.
    if stuck_on.is_some() {
        loop {
            let before = pending.len();
            pending.retain(|(name, expr)| {
                match eval(
                    expr,
                    &EvalCtx {
                        vars: &params,
                        time: 0.0,
                        programs: Some(programs),
                        depth: 0,
                    },
                ) {
                    Ok(value) => {
                        params.insert((*name).to_string(), value);
                        false
                    }
                    Err(_) => true,
                }
            });
            if pending.is_empty() || pending.len() == before {
                break;
            }
        }
    }
    if let Some(why) = stuck_on {
        if !pending.is_empty() {
            return err(why);
        }
    }
    Ok((params, claimed, defined))
}

/// Compile a model, either from its declared start (`resume` absent) or
/// as a continuation from mid-run.
///
/// A continuation matters for one reason: the choice of which states to
/// demote during index reduction is a numerical pivot at a point, and a
/// choice that was right at the start can become singular later - a
/// pendulum in Cartesian coordinates crossing the horizontal. Compiling
/// again at the current point re-makes the choice with the sensitivities
/// of *now*, and everything downstream (matching, tearing, slots) simply
/// follows.
/// Compile the model as it stands at one point of a run.
///
/// The stages, in the order they happen, which is the order the
/// numbered comments below follow:
///
/// 0. `spatialDistribution` becomes ordinary equations, so that
///    everything after this sees equations and nothing else.
/// 1. Parameters and constants are worked out, in whatever order they
///    depend on one another, and the discrete layer with them: what
///    changes only at an event, and what each of those starts at. The
///    mode is settled with them: an `if` equation the compiler could
///    not decide contributes whichever branch holds here, and the run
///    is told what to watch so it can ask for a fresh compilation.
/// 2. The equations are split into those giving a state its derivative
///    and those that are algebraic.
/// 3. What is left to solve for is counted, and every name in the
///    model is checked to be something the run can reach.
/// 4. Structural analysis: equations are matched to unknowns, and
///    where the match fails the index is reduced by differentiating
///    the constraint that could not be matched.
/// 5. The order to evaluate in: what can be solved on its own is
///    solved on its own, and what cannot is torn into blocks.
/// 6. What the run needs beside the equations: event indicators, the
///    Jacobian's shape, the slots `pre` reads, the function bodies
///    nothing could inline, and where every value lives in one array.
///
/// Each of these is being given a name of its own, one at a time.
pub(crate) fn compile_at(
    model: &Model,
    resume: Option<ResumePoint>,
) -> Result<CompiledModel, SimError> {
    // 0. `spatialDistribution` becomes two ordinary equations: the two
    // ends of the profile, each of them either what is entering there
    // or what the profile has carried to it. Everything after this
    // point sees equations and nothing else.
    let carried;
    let model = if model.transports.is_empty() {
        model
    } else {
        carried = with_transport_equations(model);
        &carried
    };

    // 1. Parameters and constants, in whatever order they depend on
    // each other.
    // The bodies nothing could inline travel with the model, and the
    // work before the run begins meets them as often as the run does:
    // a parameter written as a call to one, a start value, a branch of
    // an `if` nobody could decide. They are put in view once here and
    // handed on, rather than each of those places deciding on its own
    // that a call it cannot answer is a call nobody can.
    let programs: HashMap<String, ClassDef> = model
        .functions
        .iter()
        .map(|class| (class.name.clone(), class.clone()))
        .collect();
    let (params, settled_parameters, parameter_definitions) =
        evaluate_parameters(model, &programs)?;

    // 1b. The discrete layer: what changes only at an event, and what
    // each of those starts at.
    let (discretes, discrete_start, started_discretes, discrete_equations) =
        discrete_layer(model, &params, &resume)?;

    // Where everything stands at the point being compiled for: the
    // pivot that chooses which states to demote reads it, and so does
    // the question of which branch of a run-time `if` holds here.
    let resumed = |name: &str| -> Option<f64> {
        resume
            .as_ref()
            .and_then(|point| point.values.get(name))
            .copied()
    };
    let start_env = values_at_this_point(model, &params, &discretes, &discrete_start, &resume);

    // The event built-ins become references the evaluator can look up.
    // A Boolean or an Integer is discrete-valued by its type, whatever
    // assigns it, so `pre` reaches it as well as it reaches a `when`
    // target.
    let declared: Vec<String> = model.components.iter().map(|c| c.name.clone()).collect();
    let discrete_valued: Vec<String> = model
        .components
        .iter()
        .filter(|c| c.type_name == "Boolean" || c.type_name == "Integer")
        .map(|c| c.name.clone())
        .collect();
    let mut rewrite = EventRewrite {
        discretes: &discretes,
        discrete_valued: &discrete_valued,
        pre_wanted: Vec::new(),
        declared: &declared,
        inside_a_when: false,
        params: &params,
        samples: Vec::new(),
        delays: Vec::new(),
    };
    // An `if` equation the compiler could not decide is settled here:
    // whichever branch holds at this point joins the model, and this
    // mode is then matched, torn and solved as its own set of
    // equations. The run watches the conditions and asks for a fresh
    // compilation when one of them flips.
    let mode_time = resume.as_ref().map_or(0.0, |point| point.time);
    let modes = settle_modes(model, &start_env, mode_time, resume.is_some())?;
    // The equations that define a discrete-valued name have gone to
    // the event iteration, and go with their variable rather than
    // standing among the continuous ones: one unknown and one
    // equation leave together, so the balance is what it was.
    // The definitions themselves, rewritten the same way as the rest:
    // they are what the event iteration asks each round, and what the
    // run watches for crossings.
    let discrete_defs: Vec<EquationItem> = discrete_equations
        .iter()
        .map(|at| {
            let equation = &model.equations[*at];
            Ok(EquationItem {
                lhs: rewrite.expr(&equation.lhs)?,
                rhs: rewrite.expr(&equation.rhs)?,
                origin: equation.origin.clone(),
            })
        })
        .collect::<Result<Vec<_>, SimError>>()?;
    let equations: Vec<EquationItem> = model
        .equations
        .iter()
        .enumerate()
        .filter(|(at, _)| !discrete_equations.contains(at) && !parameter_definitions.contains(at))
        .map(|(_, equation)| equation)
        .chain(
            model
                .conditional
                .iter()
                .zip(&modes)
                .flat_map(|(conditional, &taken)| &conditional.branches[taken]),
        )
        .map(|equation| {
            Ok(EquationItem {
                lhs: rewrite.expr(&equation.lhs)?,
                rhs: rewrite.expr(&equation.rhs)?,
                origin: String::new(),
            })
        })
        .collect::<Result<Vec<_>, SimError>>()?;
    // The conditions, rewritten the same way, so the run can tell when
    // the mode it was compiled for has been left behind.
    let mode_conditions: Vec<Vec<Expr>> = model
        .conditional
        .iter()
        .map(|conditional| {
            conditional
                .conditions
                .iter()
                .map(|condition| rewrite.expr(condition))
                .collect::<Result<Vec<_>, SimError>>()
        })
        .collect::<Result<Vec<_>, SimError>>()?;
    // The body of a `when` is evaluated at the instant of an event, so
    // the left limit of any variable it names is a value that exists.
    rewrite.inside_a_when = true;
    let mut when_clauses: Vec<WhenClause> = Vec::new();
    for clause in &model.when_clauses {
        let mut branches = Vec::new();
        for branch in &clause.branches {
            let mut actions = Vec::new();
            for action in &branch.actions {
                actions.push(match action {
                    WhenAction::Assign(target, value) => {
                        WhenAction::Assign(target.clone(), rewrite.expr(value)?)
                    }
                    WhenAction::Reinit(state, value) => {
                        WhenAction::Reinit(state.clone(), rewrite.expr(value)?)
                    }
                    WhenAction::Terminate(message) => WhenAction::Terminate(message.clone()),
                    // A check made when the event fires.
                    WhenAction::Assert(condition, message) => {
                        WhenAction::Assert(rewrite.expr(condition)?, message.clone())
                    }
                    // Flattening inlines the call and hands each
                    // target an assignment of its own, so no tuple
                    // reaches this far - nor a call on its own, whose
                    // checks flattening took and whose effect this
                    // compiler has no way to have.
                    WhenAction::TupleAssign(..)
                    | WhenAction::Loop(_)
                    | WhenAction::Choice(_)
                    | WhenAction::Call(..) => {
                        return err("a tuple, a loop, a choice or a call inside `when` should \
                                    have been taken apart while flattening"
                            .to_string())
                    }
                });
            }
            branches.push(WhenBranch {
                condition: rewrite.expr(&branch.condition)?,
                actions,
            });
        }
        when_clauses.push(WhenClause {
            branches,
            origin: clause.origin.clone(),
        });
    }
    rewrite.inside_a_when = false;
    // An initial equation that settled a `fixed = false` parameter has
    // done its work among the parameters, and counting it again here
    // would leave the initialisation with one equation more than it
    // has unknowns.
    let initial_equations: Vec<EquationItem> = model
        .initial_equations
        .iter()
        .enumerate()
        .filter(|(at, _)| !settled_parameters.contains(at) && !started_discretes.contains(at))
        .map(|(_, equation)| equation)
        .map(|equation| {
            Ok(EquationItem {
                lhs: rewrite.expr(&equation.lhs)?,
                rhs: rewrite.expr(&equation.rhs)?,
                origin: String::new(),
            })
        })
        .collect::<Result<Vec<_>, SimError>>()?;
    let samples = rewrite.samples;
    let delayed = rewrite.delays;
    let pre_wanted = rewrite.pre_wanted;

    // 2. Which equations give a state its derivative, and which are
    // algebraic.
    let continuous: Vec<&str> = model
        .components
        .iter()
        .filter(|c| c.variability == Variability::Continuous && !discretes.contains(&c.name))
        .map(|c| c.name.as_str())
        .collect();
    let (mut state_rhs, algebraic_eqs, implicit) = split_equations(&equations, &continuous)?;

    // 3. What is left to solve for, and whether every name in the
    // equations is one the model knows.
    let (states, unknowns) = unknowns_of(&continuous, &state_rhs, &implicit);
    check_references(&state_rhs, &algebraic_eqs, &continuous, &params, &discretes)?;
    if algebraic_eqs.len() != unknowns.len() {
        return err(unbalanced_because(&algebraic_eqs, &unknowns));
    }

    // 4. Structural analysis: match equations to unknowns. An equation
    // that cannot be matched is a DAE constraint, and index reduction
    // takes one step (Pantelides with dummy derivatives):
    //
    //   * differentiate the constraint symbolically and *add* the
    //     result as a new equation (the original stays, so it keeps
    //     holding exactly - no drift, no stabilization term);
    //   * demote one state appearing in the constraint to an algebraic
    //     unknown. Its former state equation becomes an ordinary
    //     equation and its derivative becomes a dummy unknown, which
    //     restores the equation/unknown balance.

    // 4b. Matching, and the index reduction it may call for.
    let Reduction {
        states,
        unknowns,
        algebraic_eqs,
        dummies,
        selection_records,
        matched_eq,
        eq_vars,
        n_alg,
    } = reduce_index(
        states,
        unknowns,
        algebraic_eqs,
        &mut state_rhs,
        &params,
        &start_env,
        resume.as_ref().map_or(0.0, |point| point.time),
        &|name: &str| {
            model
                .components
                .iter()
                .any(|c| c.name == name && c.fixed == Some(true) && c.start.is_some())
        },
    )?;

    let mut matched_var: Vec<usize> = vec![0; n_alg];
    for (v, eq) in matched_eq.iter().enumerate() {
        matched_var[eq.expect("maximum matching covers every unknown")] = v;
    }

    // Initial values of the states that survived demotion.
    let ctx0 = EvalCtx {
        vars: &params,
        time: 0.0,
        programs: Some(&programs),
        depth: 0,
    };
    let mut initial = Vec::new();
    for s in &states {
        if let Some(value) = resumed(s) {
            initial.push(value);
            continue;
        }
        let comp = model
            .components
            .iter()
            .find(|c| &c.name == s)
            .expect("states come from declared components");
        let value = match &comp.start {
            Some(expr) => eval(expr, &ctx0).map_err(|e| SimError(format!("start of {s}: {e}")))?,
            None => 0.0,
        };
        initial.push(value);
    }

    // 5. The order to evaluate in: what can be solved on its own is
    // an explicit assignment, and what is left over becomes one torn
    // block.
    let (ordered_algs, stages) = build_plan(
        &unknowns,
        &algebraic_eqs,
        &matched_var,
        &eq_vars,
        n_alg,
        &params,
    );

    let ctx = ctx0;
    let derivatives: Vec<Expr> = states.iter().map(|s| state_rhs[s].clone()).collect();
    // What the model itself says about where things stand at the
    // start: the parameters, and every variable given a `start` that
    // can be worked out before the run.
    let mut stated: HashMap<String, f64> = params.clone();
    for component in &model.components {
        if let Some(expr) = &component.start {
            if let Ok(value) = eval(expr, &ctx) {
                stated.insert(component.name.clone(), value);
            }
        }
    }
    // A variable with no `start` of its own may still be spoken for by
    // the model: `T_port = flowPort.h/cp` with `T_port(start = 288.15)`
    // says exactly what `flowPort.h` starts from, and zero - which is
    // what a silent start means - says a temperature of absolute zero
    // and divides the next equation by it. So a start is *read* here
    // rather than invented: only from an equation that names the
    // variable and nothing else the model has not already valued, and
    // only where the variable enters it linearly, which is the one
    // case with a single answer. Anything else keeps the zero it had.
    //
    // Read in one pass over the equations rather than one scan per
    // variable: an equation is a candidate for exactly one name - the
    // single one it mentions that nothing has valued - so asking the
    // equations what they can say costs the model once, where asking
    // each variable what the equations say costs it as many times as
    // there are unknowns.
    let mut read_starts: HashMap<&str, f64> = HashMap::new();
    {
        // Only the torn variables are asked about. Everything else in
        // an algebraic block is assigned outright before the residual
        // is formed, so its start is written over without being read,
        // and a whole model's worth of names solved for nothing is
        // what the cost of this would otherwise be.
        let iterated: std::collections::HashSet<&str> = stages
            .iter()
            .flat_map(|stage| match stage {
                PlanStage::Implicit { torn, .. } => torn.as_slice(),
                PlanStage::Explicit { .. } => &[],
            })
            .map(|&var| ordered_algs[var].as_str())
            .collect();
        let mut named: Vec<&str> = Vec::new();
        for (lhs, rhs) in &algebraic_eqs {
            named.clear();
            lhs.collect_refs(&mut named);
            rhs.collect_refs(&mut named);
            let mut wanted: Option<&str> = None;
            let mut alone = true;
            for other in &named {
                if stated.contains_key(*other) {
                    continue;
                }
                match wanted {
                    Some(first) if first == *other => {}
                    Some(_) => {
                        alone = false;
                        break;
                    }
                    None => wanted = Some(other),
                }
            }
            let (true, Some(name)) = (alone, wanted) else {
                continue;
            };
            if !iterated.contains(name) || read_starts.contains_key(name) {
                continue;
            }
            let Some(solved) = crate::symbolic::solve_linear_known(lhs, rhs, name, &stated) else {
                continue;
            };
            let value = eval(
                &solved,
                &EvalCtx {
                    vars: &stated,
                    time: 0.0,
                    programs: Some(&programs),
                    depth: 0,
                },
            );
            if let Ok(value) = value {
                if value.is_finite() {
                    read_starts.insert(name, value);
                }
            }
        }
    }
    let algebraic_start: Vec<f64> = ordered_algs
        .iter()
        .map(|name| {
            resumed(name)
                .or_else(|| {
                    model
                        .components
                        .iter()
                        .find(|c| &c.name == name)
                        .and_then(|c| c.start.as_ref())
                        .and_then(|expr| eval(expr, &ctx).ok())
                })
                .or_else(|| read_starts.get(name.as_str()).copied())
                .unwrap_or(0.0)
        })
        .collect();

    // What the run has to watch: one indicator per relation anywhere
    // in the model.
    let indicators = what_the_run_watches(
        &algebraic_eqs,
        &discrete_defs,
        &state_rhs,
        &when_clauses,
        &mode_conditions,
    );
    let fixed_starts: Vec<(String, usize, f64)> = ordered_algs
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            let component = model.components.iter().find(|c| &c.name == name)?;
            if component.fixed != Some(true) {
                return None;
            }
            let value = eval(component.start.as_ref()?, &ctx).ok()?;
            Some((name.clone(), index, value))
        })
        .collect();

    let (jacobian_groups, jacobian_rows) =
        jacobian_structure(&states, &ordered_algs, &derivatives, &stages);
    // A discretized field gives a Jacobian that only reaches a step or
    // two from the diagonal. Eliminating such a matrix as a band costs
    // n*b^2 instead of n^3, which on a rod of a few hundred nodes is the
    // difference between seconds and milliseconds.
    let jacobian_band = jacobian_rows
        .iter()
        .enumerate()
        .flat_map(|(column, rows)| rows.iter().map(move |&row| row.abs_diff(column)))
        .max()
        .filter(|band| 4 * (band + 1) < states.len());

    let mut parameters: Vec<(String, f64)> = params.into_iter().collect();
    parameters.sort_by(|a, b| a.0.cmp(&b.0));

    // Everything a run reads gets a place in one array, and every
    // expression is resolved against it: after this point the solvers
    // never look a variable up by name.
    // The bodies nothing could inline travel with the model; a call to
    // one of them is compiled into a walk rather than into arithmetic.
    let mut table = SlotTable::new(
        model
            .functions
            .iter()
            .map(|class| (class.name.clone(), class.clone()))
            .collect(),
    );
    for (name, value) in &parameters {
        table.constant(name, *value);
    }
    let state_slots: Vec<Slot> = states.iter().map(|name| table.slot(name)).collect();
    let algebraic_slots: Vec<Slot> = ordered_algs.iter().map(|name| table.slot(name)).collect();
    let discrete_slots: Vec<Slot> = discretes.iter().map(|name| table.slot(name)).collect();
    // What each definition of a discrete-valued name computes, and
    // where it writes: `off = s < 0` compiles to the relation and the
    // slot `off` is read from. The side that names the variable is the
    // one being written; the other side is what it is worth.
    let pre_slots: Vec<(Slot, Slot)> = discretes
        .iter()
        .chain(&pre_wanted)
        .map(|name| (table.slot(name), table.slot(&format!("$pre.{name}"))))
        .collect();
    // The definitions of the discrete-valued names are compiled after
    // the `pre` slots exist: a switch may be written in terms of what
    // it was - `off = s < 0 or pre(off) and not fire` is how a
    // thyristor holds itself on - and the slot that holds what it was
    // has to be in the table before the expression naming it is
    // turned into code.
    //
    // The slots the event machinery supplies are made first: a
    // discrete variable may be defined in terms of `initial()`, and
    // compiling that definition reads `$initial` before the line
    // below would have made it. Made here, the read finds it.
    let initial_slot = table.slot("$initial");
    let terminal_slot = table.slot("$terminal");
    let discrete_definitions: Vec<(Slot, Code)> = discrete_defs
        .iter()
        .map(|equation| {
            let named = |side: &Expr| match side {
                Expr::Ref(name) => discretes.contains(name).then(|| name.clone()),
                _ => None,
            };
            let (name, value) = match named(&equation.lhs) {
                Some(name) => (name, &equation.rhs),
                None => (
                    named(&equation.rhs).expect("a definition names its variable"),
                    &equation.lhs,
                ),
            };
            Ok((table.slot(&name), table.compile(value)?))
        })
        .collect::<Result<Vec<_>, SimError>>()?;
    // Every variable `pre` was asked about needs the slot it is read
    // from beside the one holding what it was when the event began:
    // the `when` targets, and whatever else was asked for by type.
    let sample_slots: Vec<Slot> = (0..samples.len())
        .map(|index| table.slot(&format!("$sample{index}")))
        .collect();
    let delay_slots: Vec<Slot> = (0..delayed.len())
        .map(|index| table.slot(&format!("$delay{index}")))
        .collect();
    let transport_slots: Vec<(Slot, Slot)> = (0..model.transports.len())
        .map(|index| {
            (
                table.slot(&format!("$carried_at_zero{index}")),
                table.slot(&format!("$carried_at_one{index}")),
            )
        })
        .collect();
    for (name, value) in discretes.iter().zip(&discrete_start) {
        let slot = table.slot(name);
        table.template[slot] = *value;
        let pre = table.slot(&format!("$pre.{name}"));
        table.template[pre] = *value;
    }

    let compiled_stages: Vec<AlgStage> = stages
        .iter()
        .map(|stage| match stage {
            PlanStage::Explicit { var, expr } => Ok(AlgStage::Explicit {
                var: *var,
                code: table.compile(expr)?,
            }),
            PlanStage::Implicit {
                vars,
                torn,
                inner,
                residuals,
            } => Ok(AlgStage::Implicit {
                vars: vars.clone(),
                torn: torn.clone(),
                inner: inner
                    .iter()
                    .map(|(var, expr)| Ok((*var, table.compile(expr)?)))
                    .collect::<Result<Vec<_>, SimError>>()?,
                residuals: residuals
                    .iter()
                    .map(|(lhs, rhs)| Ok((table.compile(lhs)?, table.compile(rhs)?)))
                    .collect::<Result<Vec<_>, SimError>>()?,
            }),
        })
        .collect::<Result<Vec<_>, SimError>>()?;

    let compiled_derivatives: Vec<Code> = derivatives
        .iter()
        .map(|expr| table.compile(expr))
        .collect::<Result<Vec<_>, SimError>>()?;
    let compiled_indicators: Vec<Code> = indicators
        .iter()
        .map(|expr| table.compile(expr))
        .collect::<Result<Vec<_>, SimError>>()?;

    let mut compiled_whens: Vec<CompiledWhen> = Vec::new();
    for clause in &when_clauses {
        let mut branches = Vec::new();
        for branch in &clause.branches {
            let mut actions = Vec::new();
            for action in &branch.actions {
                actions.push(match action {
                    WhenAction::Terminate(message) => CompiledAction::Terminate(message.clone()),
                    WhenAction::Reinit(name, value) => {
                        let Some(index) = states.iter().position(|state| state == name) else {
                            return err(format!(
                                "reinit({name}, ...): `{name}` is not a state of the flattened model"
                            ));
                        };
                        CompiledAction::Reinit(index, table.compile(value)?)
                    }
                    WhenAction::Assign(name, value) => {
                        let index = discretes
                            .iter()
                            .position(|discrete| discrete == name)
                            .expect("when targets were collected from these");
                        CompiledAction::Assign(index, table.compile(value)?)
                    }
                    // A check made when the event fires.
                    WhenAction::Assert(condition, message) => {
                        CompiledAction::Assert(table.compile(condition)?, message.clone())
                    }
                    WhenAction::TupleAssign(..)
                    | WhenAction::Loop(_)
                    | WhenAction::Choice(_)
                    | WhenAction::Call(..) => {
                        return err("a tuple, a loop, a choice or a call inside `when` should \
                                    have been taken apart while flattening"
                            .to_string())
                    }
                });
            }
            branches.push(CompiledBranch {
                condition: table.compile(&branch.condition)?,
                actions,
            });
        }
        compiled_whens.push(CompiledWhen { branches });
    }

    // The selection monitor: for every reduction, the sensitivity of
    // the constraint to the chosen victim and to each alternative, as
    // runnable code. Watching their ratio after every accepted step
    // asks for a re-selection while the current one is still sound.
    let mut selection_monitor: Vec<(Code, Vec<Code>)> = Vec::new();
    for (residual, victim, candidates) in &selection_records {
        let sensitivity_of = |name: &str| -> Result<Code, SimError> {
            let derivative =
                differentiate(residual, &DiffTarget::Variable(name)).map_err(SimError)?;
            table.compile(&simplify(&derivative))
        };
        let own = sensitivity_of(victim)?;
        let alternatives = candidates
            .iter()
            .filter(|name| *name != victim)
            .map(|name| sensitivity_of(name))
            .collect::<Result<Vec<_>, SimError>>()?;
        selection_monitor.push((own, alternatives));
    }

    // A declaration may ask for its value not to be written down, which
    // is what 18.3's `HideResult` says. Nothing else about the variable
    // changes: it is solved for and read like any other.
    let hidden = |name: &str| {
        model
            .components
            .iter()
            .find(|component| component.name == name)
            .is_some_and(|component| asks_to_hide(&component.annotations))
    };
    let output_algebraics: Vec<(String, Slot)> = ordered_algs
        .iter()
        .zip(&algebraic_slots)
        .filter(|(name, _)| !dummies.values().any(|dummy| dummy == *name))
        // A derivative the compiler named to solve for is not a
        // variable the model declared, so it is not one of its results.
        .filter(|(name, _)| {
            !implicit
                .iter()
                .any(|state| &derivative_name(state) == *name)
        })
        .filter(|(name, _)| !hidden(name))
        .map(|(name, &slot)| (name.clone(), slot))
        .collect();

    let mut compiled = CompiledModel {
        name: model.name.clone(),
        parameters,
        states,
        initial,
        derivatives: compiled_derivatives,
        values_template: table.template.clone(),
        state_slots,
        algebraic_slots,
        discrete_slots,
        discrete_definitions,
        pre_slots,
        initial_slot,
        terminal_slot,
        sample_slots,
        delays: delayed
            .iter()
            .zip(&delay_slots)
            .map(|((source, seconds), slot)| {
                Ok(CompiledDelay {
                    slot: *slot,
                    source: table.compile(source)?,
                    seconds: *seconds,
                })
            })
            .collect::<Result<Vec<_>, SimError>>()?,
        history: std::cell::RefCell::new(vec![Vec::new(); delayed.len()]),
        transports: model
            .transports
            .iter()
            .zip(&transport_slots)
            .map(|(transport, &(at_zero_slot, at_one_slot))| {
                Ok(CompiledTransport {
                    at_zero_slot,
                    at_one_slot,
                    in0: table.compile(&transport.in0)?,
                    in1: table.compile(&transport.in1)?,
                    x: table.compile(&transport.x)?,
                    positive: table.compile(&transport.positive)?,
                    // The profile is given along the coordinate; the
                    // entry positions it stands for need `x` at the
                    // start, which only the run knows, so the pairs are
                    // kept as written and turned round on the first
                    // point. Reversed, since a position further along
                    // ξ entered earlier.
                    initial: transport
                        .initial_points
                        .iter()
                        .zip(&transport.initial_values)
                        .rev()
                        .map(|(&point, &value)| (point, value))
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>, SimError>>()?,
        profiles: std::cell::RefCell::new(vec![Vec::new(); model.transports.len()]),
        algebraics: ordered_algs,
        algebraic_start,
        fixed_starts,
        stages: compiled_stages,
        stop_time: model.experiment.stop_time.unwrap_or(1.0),
        step: model.experiment.interval.unwrap_or(1e-3),
        tolerance: model.experiment.tolerance.unwrap_or(1e-6),
        method: SolverMethod::default(),
        output_algebraics,
        // A run continued from a point starts where it left off;
        // otherwise it starts where the model asked to, which is zero
        // unless it said otherwise. The force-stroke curves of the
        // flux tubes sweep a coil from `-4`, `time` standing for the
        // position rather than for a clock.
        start_time: resume.as_ref().map_or_else(
            || model.experiment.start_time.unwrap_or(0.0),
            |point| point.time,
        ),
        resume: resume.is_some(),
        reselectable: !dummies.is_empty(),
        selection_monitor,
        mode_monitor: mode_conditions
            .iter()
            .zip(&modes)
            .map(|(conditions, &taken)| {
                Ok((
                    conditions
                        .iter()
                        .map(|condition| table.compile(condition))
                        .collect::<Result<Vec<_>, SimError>>()?,
                    taken,
                ))
            })
            .collect::<Result<Vec<_>, SimError>>()?,
        asserts: model
            .asserts
            .iter()
            .map(|(condition, message)| Ok((table.compile(condition)?, message.clone())))
            .chain(
                bound_asserts(model)
                    .iter()
                    .map(|(condition, message)| Ok((table.compile(condition)?, message.clone()))),
            )
            .collect::<Result<Vec<_>, SimError>>()?,
        flat: model.clone(),
        jacobian_groups,
        jacobian_rows,
        jacobian_band,
        discretes,
        samples,
        when_clauses: compiled_whens,
        walked: table.walked.clone(),
        indicators: compiled_indicators,
    };
    // States the model pins down itself: `fixed = true` says the start
    // value is an initial condition, not a guess Newton may move.
    let fixed_states: Vec<bool> = compiled
        .states
        .iter()
        .map(|name| {
            model
                .components
                .iter()
                .find(|c| &c.name == name)
                .and_then(|c| c.fixed)
                .unwrap_or(false)
        })
        .collect();
    if resume.is_none() {
        compiled.solve_initialization(&initial_equations, &fixed_states, &derivatives, &table)?;
        compiled.check_block_regularity()?;
    }
    Ok(compiled)
}

/// One event indicator per relation anywhere in the model: the run
/// steps onto each of them rather than through it.
///
/// The condition of a run-time `if` belongs here even though the
/// branch it chose may not mention it: the step has to land on the
/// switch, or the mode would change somewhere inside a step that was
/// taken under the old one.
///
/// Moved out of `compile_at` unchanged.
fn what_the_run_watches(
    algebraic_eqs: &[(Expr, Expr)],
    discrete_eqs: &[EquationItem],
    state_rhs: &HashMap<String, Expr>,
    when_clauses: &[WhenClause],
    mode_conditions: &[Vec<Expr>],
) -> Vec<Expr> {
    let mut out = Vec::new();
    let mut collect = |expr: &Expr| collect_relations(expr, &mut out);
    for (lhs, rhs) in algebraic_eqs {
        collect(lhs);
        collect(rhs);
    }
    // The equations that define the discrete-valued names left the
    // algebraic set, and the relations inside them left with it. They
    // are exactly the crossings the run must step onto - `s < 0` of
    // every ideal switch - and without them here the model would
    // build, walk straight through the knee and give a smooth
    // untruth.
    for equation in discrete_eqs {
        collect(&equation.lhs);
        collect(&equation.rhs);
    }
    for expr in state_rhs.values() {
        collect(expr);
    }
    for clause in when_clauses {
        for branch in &clause.branches {
            collect(&branch.condition);
        }
    }
    // The condition of a run-time `if` equation belongs here even
    // though the branch it chose may not mention it.
    for conditions in mode_conditions {
        for condition in conditions {
            collect(condition);
        }
    }
    out
}

/// Symbolic time-differentiation of an expression.
///
/// `d(state)/dt` substitutes the state's defining right-hand side;
/// parameters and literals differentiate to zero; `time` to one.
/// Differentiating through an algebraic unknown or a non-smooth
/// function is reported as an error (dummy derivatives arrive with the
/// full M3).
/// Whether a declaration's annotation asks for its value to be left
/// out of the results: `annotation(HideResult = true)`.
pub(crate) fn asks_to_hide(annotations: &[Expr]) -> bool {
    annotations.iter().any(|entry| match entry {
        Expr::NamedArg(name, value) => {
            name == "HideResult" && matches!(value.as_ref(), Expr::Bool(true))
        }
        _ => false,
    })
}

/// Whether an expression names `time` anywhere below it.
fn names_time(expr: &Expr) -> bool {
    if matches!(expr, Expr::Time) {
        return true;
    }
    let mut found = false;
    expr.map_children(&mut |child| {
        found |= names_time(child);
        child.clone()
    });
    found
}

/// Collect `lhs - rhs` for every relation in an expression: these are
/// the functions whose sign changes mark an event.
pub(crate) fn collect_relations(expr: &Expr, out: &mut Vec<Expr>) {
    match expr {
        // Only what the run evaluates can turn: the rule beside it is
        // an ordinary expression by the time differentiation has used
        // it, and is collected from there.
        Expr::WithDerivative(value, _, seeds) => {
            collect_relations(value, out);
            seeds
                .iter()
                .for_each(|(_, argument)| collect_relations(argument, out));
        }
        // A string holds no relation, and cannot become one.
        Expr::Str(_) => {}
        Expr::Rel(_, l, r) => {
            out.push(simplify(&Expr::Bin(
                oxidelica_parser::BinOp::Sub,
                Box::new((**l).clone()),
                Box::new((**r).clone()),
            )));
            collect_relations(l, out);
            collect_relations(r, out);
        }
        Expr::Bin(_, l, r) | Expr::And(l, r) | Expr::Or(l, r) => {
            collect_relations(l, out);
            collect_relations(r, out);
        }
        Expr::Neg(inner) | Expr::Not(inner) => collect_relations(inner, out),
        Expr::If(c, a, b) => {
            collect_relations(c, out);
            collect_relations(a, out);
            collect_relations(b, out);
        }
        Expr::Call(_, args) => args.iter().for_each(|a| collect_relations(a, out)),
        // Arrays are expanded into scalars before any of this runs.
        Expr::Index(_, _)
        | Expr::Member(_, _)
        | Expr::Array(_)
        | Expr::Elementwise(_, _, _)
        | Expr::Range(_, _, _)
        | Expr::Comprehension(_, _, _)
        | Expr::ColonSubscript
        | Expr::EndSubscript
        | Expr::MatrixRows(_)
        | Expr::NamedArg(_, _)
        | Expr::Tuple(_)
        | Expr::Number(_)
        | Expr::Bool(_)
        | Expr::Ref(_)
        | Expr::Time => {}
    }
}

impl CompiledModel {
    /// Evaluate the plan once at the initial point, verifying that every
    /// implicit block is regular there. Catches models that are
    /// structurally fine but numerically underdetermined.
    pub(crate) fn check_block_regularity(&self) -> Result<(), SimError> {
        if self.fixed_starts.is_empty()
            && !self
                .stages
                .iter()
                .any(|s| matches!(s, AlgStage::Implicit { .. }))
        {
            return Ok(());
        }
        let mut values = self.values_template.clone();
        for (&slot, value) in self.state_slots.iter().zip(&self.initial) {
            values[slot] = *value;
        }
        let mut alg_guess = self.algebraic_start.clone();
        for stage in &self.stages {
            match stage {
                AlgStage::Explicit { var, code } => {
                    values[self.algebraic_slots[*var]] = code.run(&values, 0.0);
                }
                stage @ AlgStage::Implicit { .. } => {
                    self.solve_implicit_block(0.0, &mut values, stage, &mut alg_guess, true)?;
                }
            }
        }
        // A variable demoted by index reduction is solved from the
        // constraints; if it was declared `fixed = true`, that solution
        // has to agree with the declared initial condition.
        for (name, index, expected) in &self.fixed_starts {
            let actual = values[self.algebraic_slots[*index]];
            if (actual - expected).abs() > 1e-6 {
                return err(format!(
                    "initial value of `{name}` is fixed at {expected} but the constraints require {actual}"
                ));
            }
        }
        Ok(())
    }

    /// Solve the initialization problem: the state vector a run starts
    /// from is the one satisfying the `initial equation` section
    /// together with every state declared `fixed = true`.
    ///
    /// Without such a section nothing changes and the declared `start`
    /// values are the initial state. With one they become what they mean
    /// in the language once a model says something else about its start:
    /// the guess Newton begins from.
    pub(crate) fn solve_initialization(
        &mut self,
        initial_equations: &[EquationItem],
        fixed: &[bool],
        derivative_exprs: &[Expr],
        table: &SlotTable,
    ) -> Result<(), SimError> {
        if initial_equations.is_empty() {
            return Ok(());
        }
        let n = self.states.len();
        let pinned = fixed.iter().filter(|f| **f).count();
        if initial_equations.len() + pinned != n {
            return err(format!(
                "initialization is not square: {} initial equation(s) and {pinned} fixed start(s) for {n} state(s)",
                initial_equations.len()
            ));
        }

        // `der(x)` in an initial equation is the right-hand side the
        // model gives that state, so a steady start reads `der(x) = 0`.
        let substituted: Vec<(Code, Code)> = initial_equations
            .iter()
            .map(|equation| {
                let lhs = substitute_derivatives(&equation.lhs, &self.states, derivative_exprs)?;
                let rhs = substitute_derivatives(&equation.rhs, &self.states, derivative_exprs)?;
                Ok((table.compile(&lhs)?, table.compile(&rhs)?))
            })
            .collect::<Result<Vec<_>, SimError>>()?;

        let guess = self.initial.clone();
        let mut values = self.values_template.clone();
        let mut derivatives = Vec::new();
        let mut alg_guess = self.algebraic_start.clone();
        let mut y = guess.clone();

        let residual = |y: &[f64],
                        values: &mut Vec<f64>,
                        derivatives: &mut Vec<f64>,
                        alg_guess: &mut Vec<f64>|
         -> Result<Vec<f64>, SimError> {
            self.eval_point(0.0, y, values, derivatives, alg_guess)?;
            let mut out = Vec::with_capacity(n);
            for (lhs, rhs) in &substituted {
                out.push(lhs.run(values, 0.0) - rhs.run(values, 0.0));
            }
            for (index, pinned) in fixed.iter().enumerate() {
                if *pinned {
                    out.push(y[index] - guess[index]);
                }
            }
            Ok(out)
        };

        for _ in 0..50 {
            let f = residual(&y, &mut values, &mut derivatives, &mut alg_guess)?;
            let solved = f.iter().all(|r| r.abs() < 1e-10);
            let mut jac = vec![vec![0.0; n]; n];
            for j in 0..n {
                let h = 1e-7 * (1.0 + y[j].abs());
                let mut probe = y.clone();
                probe[j] += h;
                let fp = residual(&probe, &mut values, &mut derivatives, &mut alg_guess)?;
                for (i, row) in jac.iter_mut().enumerate() {
                    row[j] = (fp[i] - f[i]) / h;
                }
            }
            if solved {
                // Satisfied is not the same as determined: a singular
                // Jacobian means the equations leave a whole family of
                // starting points and this one is just the guess.
                let probe = vec![1.0; n];
                if solve_linear(&mut jac.clone(), &probe).is_none() {
                    return err(
                        "the initialization problem is singular: its equations do not pin the states down"
                            .to_string(),
                    );
                }
                self.initial = y;
                return Ok(());
            }
            let Some(step) = solve_linear(&mut jac, &f) else {
                return err(
                    "the initialization problem is singular: its equations do not pin the states down"
                        .to_string(),
                );
            };
            for j in 0..n {
                y[j] -= step[j];
            }
            if y.iter().any(|value| !value.is_finite()) {
                return err("initialization diverged".to_string());
            }
        }
        err("initialization did not converge in 50 Newton iterations".to_string())
    }
}

impl CompiledModel {
    /// How many evaluations one Jacobian costs: the number of column
    /// groups, which for a banded structure is far below the number of
    /// states. Reported by the CLI, and what the colouring is for.
    pub fn jacobian_cost(&self) -> usize {
        self.jacobian_groups.len()
    }

    /// Human-readable summary of the algebraic evaluation plan:
    /// one line per stage (explicit assignment or implicit block).
    pub fn plan_summary(&self) -> Vec<String> {
        self.stages
            .iter()
            .map(|stage| match stage {
                AlgStage::Explicit { var, .. } => {
                    format!("explicit: {}", self.algebraics[*var])
                }
                AlgStage::Implicit { vars, torn, .. } => {
                    let names: Vec<&str> =
                        vars.iter().map(|&i| self.algebraics[i].as_str()).collect();
                    format!(
                        "implicit block of {} (iterating on {}): {}",
                        vars.len(),
                        torn.len(),
                        names.join(", ")
                    )
                }
            })
            .collect()
    }
}
