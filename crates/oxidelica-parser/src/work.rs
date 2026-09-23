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
}

const KINDS: usize = 6;

impl Step {
    fn at(self) -> usize {
        match self {
            Step::Instantiated => 0,
            Step::Expanded => 1,
            Step::Inlined => 2,
            Step::Point => 3,
            Step::Newton => 4,
            Step::Jacobian => 5,
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
            }
        );
        assert_eq!(done.plus(done).expanded, 4);
    }
}
