//! Counts of the work done, as opposed to the time it took.
//!
//! A wall clock measured twice over the same models with the same
//! binary has come out 1.98 times apart on a desk and 29% apart on the
//! build machine, so a ratchet on time can only be as narrow as the
//! weather allows - which is wide enough for a doubling of the work to
//! pass under it without a word. What a compiler does to one model is
//! not weather: the same binary over the same library does the same
//! walk, and counting the steps of the walk gives a number that can be
//! held to a few percent.
//!
//! Each count lives on the thread doing the work, so counting costs an
//! increment and no lock. A caller that wants the work of one model
//! reads the counts before and after it on the same thread and takes
//! the difference.

use std::cell::Cell;

/// One kind of step worth counting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// A class instantiated under some prefix.
    Instantiated,
    /// An expression handed to the array layer to be expanded.
    Expanded,
    /// A function body worked out for a call.
    Inlined,
    /// A point of a run evaluated: states in, derivatives out.
    Point,
    /// One iteration of Newton's method on an algebraic loop.
    Newton,
    /// A Jacobian of the states built by differences.
    Jacobian,
    /// A damped trial: the residual evaluated again on a step cut
    /// short, whether by a retreat from an edge or by a line search.
    /// Not a Newton iteration, which is why it is counted apart: a
    /// change whose cost is spent here moves no other count.
    Damped,
    /// One reduction of the index: a structurally singular subset
    /// found and one of its members differentiated.
    ReductionPass,
    /// An equation read by a reduction while it gathers the
    /// definitions it may reach through.
    ReductionEq,
    /// A definition so gathered.
    ReductionCandidate,
}

const KINDS: usize = 10;

impl Step {
    fn at(self) -> usize {
        match self {
            Step::Instantiated => 0,
            Step::Expanded => 1,
            Step::Inlined => 2,
            Step::Point => 3,
            Step::Newton => 4,
            Step::Jacobian => 5,
            Step::Damped => 6,
            Step::ReductionPass => 7,
            Step::ReductionEq => 8,
            Step::ReductionCandidate => 9,
        }
    }
}

thread_local! {
    static COUNTED: [Cell<u64>; KINDS] = const {
        [
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
        ]
    };
}

/// Count one step of the given kind on this thread.
#[inline]
pub fn tick(step: Step) {
    COUNTED.with(|counted| {
        let cell = &counted[step.at()];
        cell.set(cell.get() + 1);
    });
}

/// Count `n` steps of the given kind at once, for a loop too hot to
/// pay a thread-local access on every turn: it counts in a local and
/// hands the total over here when it ends.
#[inline]
pub fn add(step: Step, n: u64) {
    COUNTED.with(|counted| {
        let cell = &counted[step.at()];
        cell.set(cell.get() + n);
    });
}

/// The work this thread has done so far, by kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    /// Classes instantiated.
    pub instantiated: u64,
    /// Expressions expanded by the array layer.
    pub expanded: u64,
    /// Function bodies worked out.
    pub inlined: u64,
    /// Names looked up.
    pub names: u64,
    /// Points of a run evaluated.
    pub points: u64,
    /// Newton iterations on algebraic loops.
    pub newton: u64,
    /// Jacobians built.
    pub jacobians: u64,
    /// Damped trials of a step cut short.
    pub damped: u64,
    /// Reductions of the index.
    pub reductions: u64,
    /// Equations read by the reductions in search of definitions.
    pub reduction_eqs: u64,
    /// Definitions the reductions gathered.
    pub reduction_candidates: u64,
}

impl Work {
    /// What this thread has counted since it started.
    pub fn here() -> Work {
        let get = |step: Step| COUNTED.with(|counted| counted[step.at()].get());
        Work {
            instantiated: get(Step::Instantiated),
            expanded: get(Step::Expanded),
            inlined: get(Step::Inlined),
            names: crate::flatten::name_counts().0,
            points: get(Step::Point),
            newton: get(Step::Newton),
            jacobians: get(Step::Jacobian),
            damped: get(Step::Damped),
            reductions: get(Step::ReductionPass),
            reduction_eqs: get(Step::ReductionEq),
            reduction_candidates: get(Step::ReductionCandidate),
        }
    }

    /// The work done between `earlier` and `self`.
    pub fn since(self, earlier: Work) -> Work {
        Work {
            instantiated: self.instantiated - earlier.instantiated,
            expanded: self.expanded - earlier.expanded,
            inlined: self.inlined - earlier.inlined,
            names: self.names - earlier.names,
            points: self.points - earlier.points,
            newton: self.newton - earlier.newton,
            jacobians: self.jacobians - earlier.jacobians,
            damped: self.damped - earlier.damped,
            reductions: self.reductions - earlier.reductions,
            reduction_eqs: self.reduction_eqs - earlier.reduction_eqs,
            reduction_candidates: self.reduction_candidates - earlier.reduction_candidates,
        }
    }

    /// Two counts added together.
    pub fn plus(self, other: Work) -> Work {
        Work {
            instantiated: self.instantiated + other.instantiated,
            expanded: self.expanded + other.expanded,
            inlined: self.inlined + other.inlined,
            names: self.names + other.names,
            points: self.points + other.points,
            newton: self.newton + other.newton,
            jacobians: self.jacobians + other.jacobians,
            damped: self.damped + other.damped,
            reductions: self.reductions + other.reductions,
            reduction_eqs: self.reduction_eqs + other.reduction_eqs,
            reduction_candidates: self.reduction_candidates + other.reduction_candidates,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_counted_is_seen_in_the_difference() {
        let before = Work::here();
        tick(Step::Instantiated);
        tick(Step::Expanded);
        tick(Step::Expanded);
        tick(Step::Inlined);
        tick(Step::Point);
        tick(Step::Newton);
        tick(Step::Jacobian);
        tick(Step::Damped);
        tick(Step::ReductionPass);
        add(Step::ReductionEq, 5);
        add(Step::ReductionCandidate, 3);
        let done = Work::here().since(before);
        assert_eq!(
            done,
            Work {
                instantiated: 1,
                expanded: 2,
                inlined: 1,
                names: 0,
                points: 1,
                newton: 1,
                jacobians: 1,
                damped: 1,
                reductions: 1,
                reduction_eqs: 5,
                reduction_candidates: 3,
            }
        );
        assert_eq!(done.plus(done).expanded, 4);
    }
}
