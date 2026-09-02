//! Clocked partitions and the state machines that run on them.

use super::*;

/// The largest counter, factor or resolution a clock expression may
/// name. Nothing real needs more, and the bound keeps the exact
/// arithmetic below working in numbers it can hold.
const MAX_FACTOR: i64 = 1_000_000;

/// An exact fraction, in lowest terms with a positive denominator.
///
/// Clock arithmetic is fractions of an interval, and it has to be
/// exact. A clock super-sampled by three and sub-sampled by three again
/// is the clock it started from; two partitions that tick together have
/// to agree to the last bit about when. Seconds computed along the way
/// would not: `0.1 / 3 * 3` is not `0.1`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Ratio {
    num: i64,
    den: i64,
}

impl Ratio {
    const ZERO: Ratio = Ratio { num: 0, den: 1 };
    const ONE: Ratio = Ratio { num: 1, den: 1 };

    /// A fraction reduced to lowest terms, refusing one too large to
    /// hold. The multiplication is done wide so that only the reduced
    /// answer has to fit.
    fn new(num: i128, den: i128) -> Result<Ratio, String> {
        let (num, den) = if den < 0 { (-num, -den) } else { (num, den) };
        let divisor = gcd(num, den).max(1);
        match (i64::try_from(num / divisor), i64::try_from(den / divisor)) {
            (Ok(num), Ok(den)) => Ok(Ratio { num, den }),
            _ => Err(
                "the sampling factors of this model multiply out to a number \
                      too large to keep exactly"
                    .to_string(),
            ),
        }
    }

    fn whole(value: i64) -> Ratio {
        Ratio { num: value, den: 1 }
    }

    fn times(self, other: Ratio) -> Result<Ratio, String> {
        Ratio::new(
            i128::from(self.num) * i128::from(other.num),
            i128::from(self.den) * i128::from(other.den),
        )
    }

    fn plus(self, other: Ratio) -> Result<Ratio, String> {
        Ratio::new(
            i128::from(self.num) * i128::from(other.den)
                + i128::from(other.num) * i128::from(self.den),
            i128::from(self.den) * i128::from(other.den),
        )
    }

    fn negated(self) -> Ratio {
        Ratio {
            num: -self.num,
            den: self.den,
        }
    }

    fn value(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    fn is_negative(self) -> bool {
        self.num < 0
    }
}

/// Whether two instants are the same instant, to the last bit. Both
/// come from the same arithmetic on the same fractions, so agreement
/// here is exact agreement and not a tolerance.
fn same_number(mine: Option<f64>, theirs: Option<f64>) -> bool {
    match (mine, theirs) {
        (Some(mine), Some(theirs)) => mine.to_bits() == theirs.to_bits(),
        _ => false,
    }
}

/// Greatest common divisor, for keeping a fraction in lowest terms.
fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// A way of stepping a differential equation from one tick to the next,
/// and the tableau that says how.
///
/// Only the explicit methods are here. An implicit one asks for the
/// derivative at the point being solved for, which means solving an
/// equation at every tick, and the tick is a list of assignments rather
/// than a system - the same wall chapter 11 runs into. The
/// specification asks a tool to spell the methods it does support the
/// way it spells them here, not to support them all.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Solver {
    name: &'static str,
    /// What each stage adds to the state before working out its slope,
    /// in multiples of the step; one row per stage.
    weights: &'static [&'static [f64]],
    /// How the stages are mixed into the step that is taken.
    mix: &'static [f64],
}

/// The explicit methods of 16.8, under the names the specification
/// gives them.
const SOLVERS: [Solver; 3] = [
    Solver {
        name: "ExplicitEuler",
        weights: &[&[]],
        mix: &[1.0],
    },
    Solver {
        name: "ExplicitMidPoint2",
        weights: &[&[], &[0.5]],
        mix: &[0.0, 1.0],
    },
    Solver {
        name: "ExplicitRungeKutta4",
        weights: &[&[], &[0.5], &[0.0, 0.5], &[0.0, 0.0, 1.0]],
        mix: &[1.0 / 6.0, 1.0 / 3.0, 1.0 / 3.0, 1.0 / 6.0],
    },
];

/// The methods the specification names that ask for the derivative at
/// the point being solved for.
const IMPLICIT: [&str; 2] = ["ImplicitEuler", "ImplicitTrapezoid"];

/// What a clock's ticks are counted from.
#[derive(Clone, Debug)]
pub(super) enum Root {
    /// A fixed interval in seconds: `Clock(0.1)` or `Clock(1, 10)`.
    Every(f64),
    /// The rising edge of a condition: `Clock(b, startInterval)`. The
    /// number is what `interval` answers at the first tick, there being
    /// no earlier tick to measure back to.
    When(Expr, f64),
    /// A clock the model has left for the compiler to work out:
    /// `Clock()`, or a `subSample` with no factor. It is not a clock
    /// yet - it is a place where one has to turn up.
    Waiting {
        /// The clock this one is sampled from, where there is one.
        /// `Clock()` has none: nothing at all is known about it.
        base: Option<usize>,
        /// Whether the missing factor makes it faster or slower.
        faster: bool,
        /// What the interval found for it must be a whole number of
        /// parts of: `Clock(0, 5)` says one over five. `Clock()` says
        /// nothing about it and leaves this at zero.
        resolution: i64,
    },
}

/// A clock as the compiler keeps it: a root, and this clock's ticks as
/// exact fractions of the root's.
///
/// Every clock in a model is some root clock sub-sampled, super-sampled
/// and shifted, so that is what is stored - the root, the interval as a
/// fraction of the root's, and how far the first tick sits past it.
#[derive(Clone, Debug)]
pub(super) struct ClockSpec {
    root: Root,
    /// The interval between ticks, as a fraction of the root's.
    rate: Ratio,
    /// How far the first tick sits past the root's first, counted in
    /// root intervals.
    shift: Ratio,
    /// How a differential equation on this clock is stepped from one
    /// tick to the next. Without one, this clock carries no derivatives
    /// and a `der` on it is the mistake it has always been.
    solver: Option<Solver>,
}

impl ClockSpec {
    /// A root clock of its own, ticking every `period` seconds.
    fn every(period: f64) -> ClockSpec {
        ClockSpec {
            root: Root::Every(period),
            rate: Ratio::ONE,
            shift: Ratio::ZERO,
            solver: None,
        }
    }

    /// A clock ticking whenever a condition rises.
    fn when(condition: Expr, start_interval: f64) -> ClockSpec {
        ClockSpec {
            root: Root::When(condition, start_interval),
            rate: Ratio::ONE,
            shift: Ratio::ZERO,
            solver: None,
        }
    }

    /// The interval between two ticks, in seconds, when it is a number
    /// the compiler knows. An event clock's is not: it is however long
    /// the run takes to raise the condition again.
    fn interval(&self) -> Option<f64> {
        match &self.root {
            Root::Every(period) => Some(period * self.rate.value()),
            Root::When(..) | Root::Waiting { .. } => None,
        }
    }

    /// When the first tick falls, in seconds past the start.
    fn first(&self) -> Option<f64> {
        match &self.root {
            Root::Every(period) => Some(period * self.shift.value()),
            Root::When(..) | Root::Waiting { .. } => None,
        }
    }

    /// How many ticks of the root make one of this clock. An event
    /// clock sub-sampled by three fires on every third rising edge.
    fn every_nth(&self) -> i64 {
        self.rate.num
    }

    /// How to name this clock in a message.
    fn describe(&self) -> String {
        match (&self.root, self.interval()) {
            (_, Some(interval)) => format!("every {interval}"),
            (Root::Waiting { .. }, _) => "at a rate nothing has said yet".to_string(),
            _ => "on an event".to_string(),
        }
    }

    /// Whether this is a place where a clock has to turn up rather than
    /// a clock.
    fn waiting(&self) -> Option<(Option<usize>, bool, i64)> {
        match self.root {
            Root::Waiting {
                base,
                faster,
                resolution,
            } => Some((base, faster, resolution)),
            _ => None,
        }
    }

    /// Whether two clocks are the same clock, which is a question about
    /// when they tick and not about the road taken to them.
    ///
    /// `Clock(1, 5)` and `Clock(1, 10)` sub-sampled by two are one
    /// clock spelled two ways, and a model is free to reach the same
    /// instants either way. Two periodic clocks are therefore compared
    /// by the two numbers their `when` fires on; agreeing there to the
    /// last bit is what ticking together means, and the fractions are
    /// what puts the numbers where they belong rather than a rounding
    /// away from it.
    fn same(&self, other: &ClockSpec) -> bool {
        let roots = match (&self.root, &other.root) {
            (Root::Every(_), Root::Every(_)) => {
                same_number(self.interval(), other.interval())
                    && same_number(self.first(), other.first())
            }
            // No two conditions can be told to rise together except by
            // being the same condition, counted the same way.
            (Root::When(mine, start), Root::When(theirs, other_start)) => {
                mine == theirs
                    && start.to_bits() == other_start.to_bits()
                    && self.rate == other.rate
                    && self.shift == other.shift
            }
            // A clock still waiting to be worked out is not the same as
            // anything, itself included: two places where a clock has
            // to turn up may want different clocks.
            _ => false,
        };
        roots && self.solver == other.solver
    }

    /// The same clock ticking `factor` times more slowly, its first
    /// tick still where this one's is.
    fn sub_sampled(&self, factor: i64) -> Result<ClockSpec, String> {
        Ok(ClockSpec {
            rate: self.rate.times(Ratio::whole(factor))?,
            ..self.clone()
        })
    }

    /// The same clock ticking `factor` times faster, the interval split
    /// into that many equal parts.
    ///
    /// There is nothing to split on an event clock: the specification
    /// forbids super-sampling one, because no compiler can say when the
    /// condition will rise next, let alone a third of the way there.
    fn super_sampled(&self, factor: i64) -> Result<ClockSpec, String> {
        self.periodic_only("superSample")?;
        Ok(ClockSpec {
            rate: self.rate.times(Ratio::new(1, i128::from(factor))?)?,
            ..self.clone()
        })
    }

    /// The same clock with every tick moved `counter / resolution` of an
    /// interval later, or earlier when `back`.
    fn shifted(&self, counter: i64, resolution: i64, back: bool) -> Result<ClockSpec, String> {
        // A shift is a fraction of an interval, and `shiftSample` is
        // written in the specification as a `superSample` followed by a
        // `subSample` - so it is out of reach for the same reason.
        self.periodic_only(if back { "backSample" } else { "shiftSample" })?;
        let step = self
            .rate
            .times(Ratio::new(i128::from(counter), i128::from(resolution))?)?;
        let shift = self.shift.plus(if back { step.negated() } else { step })?;
        if shift.is_negative() {
            return Err(
                "`backSample` would put the first tick before the start of the run, \
                        which only a `shiftSample` of at least as much can make room for"
                    .to_string(),
            );
        }
        Ok(ClockSpec {
            shift,
            ..self.clone()
        })
    }

    /// Refuse an operator that only means something on a clock whose
    /// ticks are known in advance.
    fn periodic_only(&self, operator: &str) -> Result<(), String> {
        match self.root {
            Root::Every(_) => Ok(()),
            Root::When(..) => Err(format!(
                "`{operator}` asks where a tick falls between two others, and an event \
                 clock has no answer - only `subSample`, which counts them, applies to one"
            )),
            Root::Waiting { .. } => Err(format!(
                "`{operator}` is asked of a clock that is itself still to be worked out, \
                 and one unknown cannot be measured against another"
            )),
        }
    }
}

/// Every clock a model needs, the derived ones included.
///
/// A clock is referred to by its place in this table rather than by
/// name, because most of them have no name: `subSample(u, 2)` puts the
/// equation it sits in on a clock the model never declared.
#[derive(Default)]
pub(super) struct Clocks {
    specs: Vec<ClockSpec>,
    named: HashMap<String, usize>,
}

impl Clocks {
    /// Where a clock sits in the table, putting it there if it is new.
    fn intern(&mut self, spec: ClockSpec) -> usize {
        match self.specs.iter().position(|known| known.same(&spec)) {
            Some(index) => index,
            None => {
                self.specs.push(spec);
                self.specs.len() - 1
            }
        }
    }

    /// A place where a clock has to turn up, which is never the same
    /// place twice - so it goes in without being looked for.
    fn waiting(&mut self, base: Option<usize>, faster: bool, resolution: i64) -> usize {
        self.specs.push(ClockSpec {
            root: Root::Waiting {
                base,
                faster,
                resolution,
            },
            rate: Ratio::ONE,
            shift: Ratio::ZERO,
            solver: None,
        });
        self.specs.len() - 1
    }

    /// Put a clock where a place waiting for one was, so that everything
    /// pointing at that place sees the clock at once.
    fn settle(&mut self, index: usize, spec: ClockSpec) {
        self.specs[index] = spec;
    }

    /// The first entry of the table saying the same thing as this one.
    ///
    /// A place waiting for a clock keeps its own row, and settling it
    /// writes the clock it found into that row rather than pointing at
    /// the row the clock already had - so the same clock can sit in
    /// the table twice. A partition is one row, so what a variable is
    /// put on is the earliest row saying it.
    fn canonical(&self, index: usize) -> usize {
        match self
            .specs
            .iter()
            .position(|known| known.same(&self.specs[index]))
        {
            Some(first) => first,
            None => index,
        }
    }

    fn spec(&self, index: usize) -> &ClockSpec {
        &self.specs[index]
    }

    fn by_name(&self, name: &str) -> Option<usize> {
        self.named.get(name).copied()
    }

    /// The one clock a model declared, when it declared exactly one.
    fn only_named(&self) -> Option<usize> {
        match self.named.len() {
            1 => self.named.values().next().copied(),
            _ => None,
        }
    }

    fn count(&self) -> usize {
        self.named.len()
    }
}

/// Split a model into its clocked partitions.
///
/// A clock is not a value the run carries: `Clock c = Clock(0.1)` says
/// when things happen, and the equations that happen then are lifted
/// into a `when` clause firing on that period. Inside one, the clock
/// conversions say what they always meant - `sample(u, c)` is reading
/// `u` at the tick, `previous(x)` is the value from the tick before,
/// `interval(c)` is the period - and the variables they define hold
/// their values in between, which is what `hold` asks for.
///
/// A model with no clocks in it passes through untouched.
pub(super) fn partition_clocks(model: &mut Model) -> Result<(), String> {
    // What every clock of the model ticks at, read off the
    // declarations and the equations that name them. `None` where the
    // model declares no clock at all, and the machines have been asked
    // about it already.
    let Some(DeclaredClocks {
        mut clocks,
        parameters,
        declared,
    }) = clocks_of_the_model(model)?
    else {
        return Ok(());
    };

    // A `when Clock() then ... end when` is a clocked partition
    // written out by hand, which is how the standard library's
    // samplers say that a handful of equations share one tick. The
    // actions are equations on that clock, so they are read out as
    // equations here and the clock they were written under is
    // remembered for each: a clause that named its clock hands it
    // straight over, and one that left it open - `Clock()` - has it
    // settled by whatever else the same equations touch, the whole
    // clause moving together because every target points at the one
    // place waiting for a clock.
    let mut grouped: HashMap<String, usize> = HashMap::new();
    let mut kept_clauses = Vec::new();
    for clause in model.when_clauses.drain(..) {
        let plain = clause.branches.len() == 1
            && clause.branches[0]
                .actions
                .iter()
                .all(|action| matches!(action, WhenAction::Assign(..)));
        let clock = match plain {
            true => clock_expr(&clause.branches[0].condition, &mut clocks, &parameters)?,
            false => None,
        };
        let Some(clock) = clock else {
            kept_clauses.push(clause);
            continue;
        };
        for action in &clause.branches[0].actions {
            let WhenAction::Assign(target, value) = action else {
                unreachable!("every action was checked to be an assignment")
            };
            grouped.insert(target.clone(), clock);
            model
                .equations
                .push(EquationItem::new(Expr::Ref(target.clone()), value.clone()));
        }
    }
    model.when_clauses = kept_clauses;

    // Which variable belongs to which clock. A `sample(u, c)` puts the
    // equation it sits in on `c`, and from there it spreads to
    // whatever those variables define.
    let mut clock_of: HashMap<String, usize> = HashMap::new();
    // A state machine is a clocked thing: it decides where it is at
    // each tick, and the equations of its states run only while their
    // state is the one it is in.
    build_state_machines(model, &clocks, &mut clock_of)?;
    for _ in 0..MAX_DEPTH {
        let mut settled = true;
        let mut found = Vec::new();
        for equation in &model.equations {
            let Some((target, is_rate)) = assigned_by(equation) else {
                continue;
            };
            if clock_of.contains_key(&target) {
                continue;
            }
            found.clear();
            if let Some(clock) = grouped.get(&target) {
                found.push(*clock);
            }
            clocks_touched(
                &equation.rhs,
                &mut clocks,
                &clock_of,
                &parameters,
                &mut found,
            )?;
            if let Some(clock) = one_clock(&found, &mut clocks, &target)? {
                // A derivative joins a clock only where the clock says
                // how to step it across a tick. On any other it stays
                // continuous, and reading a clocked value from it is
                // the mistake the check further down names.
                if is_rate && clocks.spec(clock).solver.is_none() {
                    continue;
                }
                clock_of.insert(target, clock);
                settled = false;
            }
        }
        if settled {
            break;
        }
    }

    // A clock left for the compiler to work out has to have met a known
    // one by now. Letting an unsettled one through would be worse than
    // refusing it: nothing would be lifted onto it, and the equations
    // that were meant to tick would quietly stay continuous.
    for name in &declared {
        let index = clocks.by_name(name).expect("every one was checked above");
        if clocks.spec(index).waiting().is_some() {
            return Err(format!(
                "nothing in this model says how often `{name}` ticks - a clock written as \
                 `Clock()` takes its rate from an equation where it meets a clock that has \
                 one"
            ));
        }
    }

    // A name a clocked equation reads is on that clock too, where its
    // own equation cannot stand off one. `counter = if
    // previous(counter) < startTick then ...` says nothing on its own
    // - `previous` asks for a clock rather than giving one - and the
    // clock stands one equation away, on the `y` the block was
    // written to answer with, or across the `connect` that gave the
    // block its clock in the first place.
    //
    // Only to a name whose own equation writes `previous` or its kin:
    // those cannot stand off a clock at all, so joining one is the
    // only reading. A name that asks for none - what a `sample` reads
    // - is continuous on purpose, and pulling it in would lift
    // equations that were meant to stay.
    for _ in 0..MAX_DEPTH {
        let asks_for_a_clock = |name: &str| {
            model.equations.iter().any(|other| {
                matches!(&other.lhs, Expr::Ref(target) if target == name)
                    && ["previous", "firstTick", "subSample", "superSample"]
                        .iter()
                        .any(|asked| mentions_call(&other.rhs, asked))
            })
        };
        let mut joined = Vec::new();
        for equation in &model.equations {
            let Expr::Ref(target) = &equation.lhs else {
                continue;
            };
            let Some(clock) = clock_of.get(target).copied() else {
                continue;
            };
            let mut named = Vec::new();
            named_within_the_partition(&equation.rhs, &mut named);
            for name in named {
                if clock_of.contains_key(&name)
                    || !model.components.iter().any(|held| held.name == name)
                {
                    continue;
                }
                // Either the name cannot stand off a clock itself, or
                // it is one end of a plain equality with something
                // that cannot: `assignClock1.y = assignClock1.u` and
                // `assignClock1.u = step.y` are how a `connect`
                // arrives, and the clock a model assigns has to cross
                // them to reach the block that wrote nothing about
                // one. An equality is the whole equation and holds no
                // boundary, so nothing continuous rides over.
                // A plain equality carries the clock only where the
                // name it reaches does not stand on a boundary of its
                // own. `s.y = sample(s.u)` is a sampler: `s.y` is on
                // the clock and `s.u` is the continuous signal it
                // reads, so the equality between them is exactly
                // where a clock must stop.
                let crosses = model.equations.iter().any(|other| {
                    matches!(&other.lhs, Expr::Ref(target) if target == &name)
                        && ["sample", "hold", "noClock"]
                            .iter()
                            .any(|edge| mentions_call(&other.rhs, edge))
                });
                let plain = matches!(&equation.rhs, Expr::Ref(_)) && !crosses;
                if asks_for_a_clock(&name) || plain {
                    joined.push((name, clock));
                }
            }
        }
        if joined.is_empty() {
            break;
        }
        for (name, clock) in joined {
            clock_of.insert(name, clock);
        }
    }

    // An operator that only makes sense on a clock has to be on one.
    for equation in &model.equations {
        if let Expr::Ref(target) = &equation.lhs {
            if clock_of.contains_key(target) {
                continue;
            }
            for asked in [
                "previous",
                "firstTick",
                "subSample",
                "superSample",
                "noClock",
            ] {
                if mentions_call(&equation.rhs, asked) {
                    return Err(format!(
                        "`{target}` uses `{asked}`, but nothing says which clock it is on"
                    ));
                }
            }
        }
    }

    // Lift the clocked equations into one `when` per clock.
    let mut kept = Vec::new();
    let mut lifted: HashMap<usize, Vec<(String, Expr)>> = HashMap::new();
    let mut rates: HashMap<usize, Vec<(String, Expr)>> = HashMap::new();
    for equation in model.equations.drain(..) {
        let clock = assigned_by(&equation)
            .and_then(|(target, is_rate)| Some((target.clone(), is_rate, *clock_of.get(&target)?)));
        match clock {
            Some((target, is_rate, clock)) => {
                let value = at_the_tick(&equation.rhs, &clocks, &clock_of, Some(clock));
                let into = if is_rate { &mut rates } else { &mut lifted };
                into.entry(clock).or_default().push((target, value));
            }
            None => kept.push(equation),
        }
    }
    model.equations = kept;
    let mut bookkeeping: Vec<(String, usize, f64)> = Vec::new();

    // A clock carrying derivatives steps them across its tick with the
    // method it was given, which turns each into an assignment like any
    // other. It happens before the partitions are ordered, so what the
    // step reads counts towards that order.
    let mut clocks_with_rates: Vec<usize> = rates.keys().copied().collect();
    clocks_with_rates.sort_unstable();
    for clock in clocks_with_rates {
        let mut states = rates.remove(&clock).expect("just listed");
        states.sort_by(|left, right| left.0.cmp(&right.0));
        let spec = clocks.spec(clock).clone();
        let solver = spec
            .solver
            .expect("a derivative only joins a clock that steps it");
        // The step just taken is one the run can measure. The step
        // about to be taken is not, on an event clock, and a method
        // with more than one stage has to guess where the state will be
        // partway through it - so those want a clock that says in
        // advance how long its ticks are.
        let step = match spec.interval() {
            Some(seconds) => Expr::Number(seconds),
            None if solver.weights.len() == 1 => elapsed_since_last_tick(&spec, clock),
            None => {
                return Err(format!(
                    "`{}` works out where the state will be partway through a step, and an \
                     event clock does not know how long its next step is - `ExplicitEuler` \
                     is what a clock ticking on a condition can be stepped with",
                    solver.name
                ))
            }
        };
        let stepped = one_step(solver, clock, &states, &step);
        for (target, _) in &stepped {
            if !states.iter().any(|(name, _)| name == target) {
                bookkeeping.push((target.clone(), clock, 0.0));
            }
        }
        lifted.entry(clock).or_default().extend(stepped);
    }
    for (name, clock, _) in &bookkeeping {
        clock_of.insert(name.clone(), *clock);
    }
    for clock in in_partition_order(&lifted)? {
        let mut actions = lifted.remove(&clock).expect("the order names each once");
        let spec = clocks.spec(clock).clone();
        let counter = counter_name(clock);
        let last = last_tick_name(clock);

        // `firstTick` needs the partition to count its own ticks, and
        // nothing but a counter will do it: a clock has no other way of
        // telling its first activation from its hundredth. An event
        // clock's `interval` reads the same counter to know whether
        // there was a tick before to measure back to.
        let asks_when = actions.iter().any(|(_, value)| mentions_ref(value, &last));
        if asks_when
            || actions
                .iter()
                .any(|(_, value)| mentions_call(value, "firstTick"))
        {
            for (_, value) in &mut actions {
                *value = answer_first_tick(value, &counter);
            }
            actions.push((counter.clone(), after(&counter, Expr::Number(1.0))));
            bookkeeping.push((counter.clone(), clock, 0.0));
        }
        if asks_when {
            actions.push((last.clone(), Expr::Time));
            bookkeeping.push((last, clock, 0.0));
        }

        // An event clock sub-sampled by n fires on every n-th rising
        // edge, but the edge itself arrives every time, so the
        // partition counts the ones it skips and holds what it had
        // through them. A periodic clock needs none of this: its rate
        // is already in the interval it ticks on.
        let condition = match &spec.root {
            Root::Every(_) => Expr::Call(
                "sample".to_string(),
                vec![
                    Expr::Number(spec.first().expect("a periodic clock has a first tick")),
                    Expr::Number(spec.interval().expect("and an interval")),
                ],
            ),
            Root::When(condition, _) => condition.clone(),
            Root::Waiting { .. } => {
                return Err(format!(
                    "nothing in this model says how often `{}` ticks - a clock left for the \
                     compiler to work out has to meet a known one somewhere in an equation",
                    actions
                        .first()
                        .map(|(target, _)| target.as_str())
                        .unwrap_or("it")
                ))
            }
        };
        if spec.interval().is_none() && spec.every_nth() > 1 {
            let skipped = format!("$every{clock}");
            let due = Expr::Rel(
                crate::ast::RelOp::Ge,
                Box::new(after(&skipped, Expr::Number(1.0))),
                Box::new(Expr::Number(spec.every_nth() as f64)),
            );
            for (target, value) in &mut actions {
                *value = Expr::If(
                    Box::new(Expr::Rel(
                        crate::ast::RelOp::Lt,
                        Box::new(Expr::Ref(skipped.clone())),
                        Box::new(Expr::Number(0.5)),
                    )),
                    Box::new(value.clone()),
                    Box::new(Expr::Call(
                        "pre".to_string(),
                        vec![Expr::Ref(target.clone())],
                    )),
                );
            }
            actions.push((
                skipped.clone(),
                Expr::If(
                    Box::new(due),
                    Box::new(Expr::Number(0.0)),
                    Box::new(after(&skipped, Expr::Number(1.0))),
                ),
            ));
            // Counting from one short of the factor makes the first
            // edge a firing one, as 16.5 asks: the sub-sampled clock's
            // first activation is its argument's first activation.
            bookkeeping.push((skipped, clock, spec.every_nth() as f64 - 1.0));
        }

        // The equations of a partition are equations, in no order of
        // their own; what the tick needs is an order in which each is
        // ready when its turn comes. `previous` reaches back to the
        // tick before, so it is not a reason to wait.
        let actions = in_dependency_order(actions)?;
        // What an event clock waits for happens in continuous time, so
        // its condition is written in continuous time too: a clocked
        // variable only changes at a tick, and a clock waiting on one of
        // its own would be waiting on itself.
        if let Some(clocked) = clocked_outside_hold(&condition, &clock_of) {
            return Err(format!(
                "an event clock waits on something the run varies between ticks, and \
                 `{clocked}` is clocked - `hold({clocked})` is how a clocked value is \
                 read in continuous time"
            ));
        }
        model.when_clauses.push(WhenClause {
            branches: vec![WhenBranch { condition, actions }],
            origin: String::new(),
        });
    }
    for (name, clock, start) in bookkeeping {
        model.components.push(Component {
            name: name.clone(),
            variability: Variability::Discrete,
            start: Some(Expr::Number(start)),
            description: Some("clock bookkeeping".to_string()),
            ..blank_component()
        });
        clock_of.insert(name, clock);
    }

    // What is left of the continuous part may only reach a clocked
    // variable through `hold`, which the rewrite above has already
    // turned into the variable itself - so anything still naming one
    // here was written without it.
    for equation in &model.equations {
        for side in [&equation.lhs, &equation.rhs] {
            if let Some(clocked) = clocked_outside_hold(side, &clock_of) {
                return Err(format!(
                    "`{clocked}` is a clocked variable, so a continuous equation may only \
                     read it through `hold({clocked})`"
                ));
            }
        }
    }
    // With that settled, `hold` has nothing left to say: a clocked
    // variable holds its value between ticks by itself.
    for equation in &mut model.equations {
        equation.lhs = at_the_tick(&equation.lhs, &clocks, &clock_of, None);
        equation.rhs = at_the_tick(&equation.rhs, &clocks, &clock_of, None);
    }

    // The clocked variables keep their values between ticks, and the
    // clocks themselves are not variables at all.
    for component in &mut model.components {
        if clock_of.contains_key(&component.name) {
            component.variability = Variability::Discrete;
            if component.start.is_none() {
                component.start = Some(Expr::Number(0.0));
            }
        }
    }
    model
        .components
        .retain(|component| component.type_name != "Clock");
    Ok(())
}

/// The clocks a model declares, what they tick at, and the numbers
/// they were read against.
struct DeclaredClocks {
    clocks: Clocks,
    parameters: HashMap<String, f64>,
    declared: Vec<String>,
}

/// What every clock the model declares ticks at.
///
/// A clock says what it is either in its declaration or in an equation
/// naming it, and either may be written in terms of parameters built
/// on other parameters - the standard library's exact clock reads its
/// factor out of a table of constants - so the numbers are settled
/// first and the clocks read against them.
///
/// `None` where the model declares no clock: a machine with no clock
/// to run on still has to hear about it, and hears here.
///
/// Moved out of `partition_clocks` unchanged.
fn clocks_of_the_model(model: &mut Model) -> Result<Option<DeclaredClocks>, String> {
    let declared: Vec<String> = model
        .components
        .iter()
        .filter(|component| component.type_name == "Clock")
        .map(|component| component.name.clone())
        .collect();
    if declared.is_empty() {
        // A machine with no clock to run on still has to hear about
        // it, so it is asked before this pass gives up.
        build_state_machines(model, &Clocks::default(), &mut HashMap::new())?;
        return Ok(None);
    }
    // Parameters may be built on one another - the standard library's
    // exact clock reads its factor out of a table of constants - so
    // they are worked out until nothing new settles rather than in one
    // pass against nothing.
    let mut parameters: HashMap<String, f64> = HashMap::new();
    loop {
        let before = parameters.len();
        for component in &model.components {
            if parameters.contains_key(&component.name) {
                continue;
            }
            let Some(value) = component
                .binding
                .as_ref()
                .and_then(|value| const_eval(value, &parameters))
            else {
                continue;
            };
            parameters.insert(component.name.clone(), value);
        }
        if parameters.len() == before {
            break;
        }
    }

    // A clock says what it is either in its declaration or in an
    // equation of its own, and it may say it in terms of another -
    // `Clock fast = superSample(slow, 3)` - so the definitions are
    // gathered first and worked out until nothing new settles.
    let mut definitions: Vec<(String, Expr)> = model
        .components
        .iter()
        .filter(|component| component.type_name == "Clock")
        .filter_map(|component| Some((component.name.clone(), component.binding.clone()?)))
        .collect();
    let mut kept = Vec::new();
    for equation in model.equations.drain(..) {
        // Either side may be the clock being said: a connection
        // between two of them - a clock signal drawn from one block to
        // another - comes out with whichever name sorts first on the
        // left, and that one may be the one already known.
        let spoken_for = |name: &String| definitions.iter().any(|(known, _)| known == name);
        let said = match (&equation.lhs, &equation.rhs) {
            // Where both are clocks - a clock signal drawn from one
            // block to another is exactly that - the one being said is
            // the one nothing has said yet.
            (Expr::Ref(left), Expr::Ref(right))
                if declared.contains(left) && declared.contains(right) =>
            {
                match spoken_for(left) {
                    true => Some((right.clone(), equation.lhs.clone())),
                    false => Some((left.clone(), equation.rhs.clone())),
                }
            }
            (Expr::Ref(target), _) if declared.contains(target) => {
                Some((target.clone(), equation.rhs.clone()))
            }
            (_, Expr::Ref(target)) if declared.contains(target) => {
                Some((target.clone(), equation.lhs.clone()))
            }
            _ => None,
        };
        match said {
            Some(said) => definitions.push(said),
            None => kept.push(equation),
        }
    }
    model.equations = kept;

    let mut clocks = Clocks::default();
    for _ in 0..MAX_DEPTH {
        let mut settled = true;
        for (name, value) in &definitions {
            if clocks.by_name(name).is_some() {
                continue;
            }
            if let Some(index) = clock_expr(value, &mut clocks, &parameters)? {
                clocks.named.insert(name.clone(), index);
                settled = false;
            }
        }
        if settled {
            break;
        }
    }
    for name in &declared {
        let Some(index) = clocks.by_name(name) else {
            return Err(format!(
                "`{name}` is a Clock, so it needs an interval the compiler can see: \
                 `Clock {name} = Clock(0.1);`"
            ));
        };
        if clocks
            .spec(index)
            .interval()
            .is_some_and(|seconds| seconds <= 0.0)
        {
            return Err(format!("the interval of `{name}` must be positive"));
        }
    }

    Ok(Some(DeclaredClocks {
        clocks,
        parameters,
        declared,
    }))
}

/// The names a sub-clock conversion goes by, and whether it takes a
/// resolution alongside its counter.
const SUB_CLOCK: [(&str, bool); 4] = [
    ("subSample", false),
    ("superSample", false),
    ("shiftSample", true),
    ("backSample", true),
];

/// An evaluable whole number an operator was given, within the bounds
/// the exact arithmetic can hold.
fn whole_argument(
    expr: &Expr,
    parameters: &HashMap<String, f64>,
    what: &str,
    least: i64,
) -> Result<i64, String> {
    let Some(value) = const_eval(expr, parameters) else {
        return Err(format!(
            "the {what} of a clock operator has to be a number the compiler can work out"
        ));
    };
    if value.fract() != 0.0 || value < least as f64 || value > MAX_FACTOR as f64 {
        return Err(format!(
            "the {what} of a clock operator has to be a whole number \
             between {least} and {MAX_FACTOR}, not {value}"
        ));
    }
    Ok(value as i64)
}

/// The clock an expression stands for, when it stands for one: a
/// constructor, the name of a clock already worked out, or a sub-clock
/// conversion of either.
pub(super) fn clock_expr(
    expr: &Expr,
    clocks: &mut Clocks,
    parameters: &HashMap<String, f64>,
) -> Result<Option<usize>, String> {
    let Expr::Call(name, args) = expr else {
        return Ok(match expr {
            Expr::Ref(name) => clocks.by_name(name),
            _ => None,
        });
    };
    match (name.as_str(), args.len()) {
        // `Clock(0.1)` says its interval in seconds.
        // `Clock()` says only that there is a clock here, and leaves
        // working out which to whatever else the model says.
        ("Clock", 0) => Ok(Some(clocks.waiting(None, false, 0))),
        ("Clock", 1) => Ok(Some(match const_eval(&args[0], parameters) {
            // A nought here is the fraction form with the denominator
            // left out, which is one - not an interval of no time.
            Some(0.0) => clocks.waiting(None, false, 1),
            Some(interval) => clocks.intern(ClockSpec::every(interval)),
            None => clocks.intern(ClockSpec::when(args[0].clone(), 0.0)),
        })),
        // `Clock(c, "ExplicitEuler")` is the clock `c` again, with a way
        // of stepping a differential equation across its ticks.
        ("Clock", 2) if matches!(&args[1], Expr::Str(_)) => {
            let Expr::Str(method) = &args[1] else {
                unreachable!("the guard just checked it")
            };
            let Some(base) = clock_expr(&args[0], clocks, parameters)? else {
                return Ok(None);
            };
            let Some(solver) = SOLVERS.iter().find(|known| known.name == method) else {
                let worked = SOLVERS
                    .iter()
                    .map(|known| known.name)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(if IMPLICIT.contains(&method.as_str()) {
                    format!(
                        "`{method}` asks for the derivative at the point it is solving for, \
                         so every tick becomes an equation to solve rather than a value to \
                         work out, and a tick here is a list of assignments - the explicit \
                         methods are {worked}"
                    )
                } else if method == "External" {
                    format!(
                        "`External` leaves the method to whatever is running the model, and \
                         this one has nothing to leave it to - name a method instead: {worked}"
                    )
                } else {
                    format!(
                        "`{method}` is not a solver method the specification names; \
                         this compiler works {worked}"
                    )
                });
            };
            let stepped = ClockSpec {
                solver: Some(*solver),
                ..clocks.spec(base).clone()
            };
            Ok(Some(clocks.intern(stepped)))
        }
        ("Clock", 2) => Ok(Some(match const_eval(&args[0], parameters) {
            // `Clock(1, 10)` says the interval as a fraction, which is
            // how a model asks for a rate no decimal writes exactly -
            // and `Clock(0, 10)` says the numerator is for the compiler
            // to find.
            Some(_) => {
                let counter = whole_argument(&args[0], parameters, "interval counter", 0)?;
                let resolution = whole_argument(&args[1], parameters, "resolution", 1)?;
                match counter {
                    0 => clocks.waiting(None, false, resolution),
                    _ => clocks.intern(ClockSpec::every(counter as f64 / resolution as f64)),
                }
            }
            None => {
                let Some(start) = const_eval(&args[1], parameters) else {
                    return Err("the start interval of an event clock has to be a number \
                                the compiler can work out"
                        .to_string());
                };
                clocks.intern(ClockSpec::when(args[0].clone(), start))
            }
        })),
        _ => {
            let Some((_, takes_resolution)) =
                SUB_CLOCK.iter().find(|(known, _)| known == name).copied()
            else {
                return Ok(None);
            };
            let Some(base) = clock_expr(&args[0], clocks, parameters)? else {
                return Ok(None);
            };
            Ok(Some(derive(
                clocks,
                base,
                name,
                &args[1..],
                takes_resolution,
                parameters,
            )?))
        }
    }
}

/// One sub-clock conversion applied to a clock.
fn derive(
    clocks: &mut Clocks,
    base: usize,
    operator: &str,
    args: &[Expr],
    takes_resolution: bool,
    parameters: &HashMap<String, f64>,
) -> Result<usize, String> {
    let shifting = operator == "shiftSample" || operator == "backSample";
    // Zero means different things either side of that line, and both
    // are allowed: no shift at all, or a sampling factor the model is
    // leaving to the compiler - which is what leaving it out means too.
    let counter = match args.first() {
        Some(given) => whole_argument(given, parameters, "factor", 0)?,
        None => 0,
    };
    let resolution = match (takes_resolution, args.get(1)) {
        (true, Some(given)) => whole_argument(given, parameters, "resolution", 1)?,
        _ => 1,
    };
    if !shifting && counter == 0 {
        return Ok(clocks.waiting(Some(base), operator == "superSample", 1));
    }
    let spec = clocks.spec(base).clone();
    let derived = match operator {
        "subSample" => spec.sub_sampled(counter),
        "superSample" => spec.super_sampled(counter),
        "shiftSample" => spec.shifted(counter, resolution, false),
        "backSample" => spec.shifted(counter, resolution, true),
        _ => unreachable!("the caller matched the name against the same table"),
    }?;
    Ok(clocks.intern(derived))
}

/// Every clock an expression puts its equation on.
///
/// A base-clock conversion is where a clock stops: what `hold` gives
/// back is continuous however clocked its argument was, and what
/// `sample` reads was continuous before it. A sub-clock conversion is
/// where one clock becomes another - the equation lands on the derived
/// clock, not on the one its argument was written on.
pub(super) fn clocks_touched(
    expr: &Expr,
    clocks: &mut Clocks,
    clock_of: &HashMap<String, usize>,
    parameters: &HashMap<String, f64>,
    found: &mut Vec<usize>,
) -> Result<(), String> {
    let recur = |e: &Expr, clocks: &mut Clocks, found: &mut Vec<usize>| {
        clocks_touched(e, clocks, clock_of, parameters, found)
    };
    match expr {
        // Where a clock stops, and where one is deliberately not
        // inferred: neither says anything about this equation.
        Expr::Call(name, _) if name == "hold" || name == "noClock" => Ok(()),
        // `sample(u, c)` names its clock and reads something that has
        // none; `interval(c)` names one and reads nothing.
        Expr::Call(name, args) if name == "sample" && args.len() == 2 => {
            if let Some(clock) = clock_expr(&args[1], clocks, parameters)? {
                found.push(clock);
            }
            Ok(())
        }
        // `sample(u)` with no clock named is a place where one has to
        // turn up: what it reads is continuous, so nothing under it
        // says which, and the answer comes from the rest of the
        // equation - the clock of what the sample is given to.
        Expr::Call(name, args) if name == "sample" && args.len() == 1 => {
            found.push(clocks.waiting(None, false, 0));
            Ok(())
        }
        Expr::Call(name, args) if name == "interval" && args.len() == 1 => {
            match clock_expr(&args[0], clocks, parameters)? {
                Some(clock) => {
                    found.push(clock);
                    Ok(())
                }
                None => recur(&args[0], clocks, found),
            }
        }
        Expr::Call(name, args)
            if SUB_CLOCK.iter().any(|(known, _)| known == name) && !args.is_empty() =>
        {
            let takes_resolution = SUB_CLOCK
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, takes)| *takes)
                .expect("the guard just found it");
            // The argument may be a clock outright - `subSample(c, 2)`
            // in a declaration - or a value carrying one of its own.
            let bases = match clock_expr(&args[0], clocks, parameters)? {
                Some(clock) => vec![clock],
                None => {
                    let mut inner = Vec::new();
                    recur(&args[0], clocks, &mut inner)?;
                    inner
                }
            };
            for base in bases {
                found.push(derive(
                    clocks,
                    base,
                    name,
                    &args[1..],
                    takes_resolution,
                    parameters,
                )?);
            }
            Ok(())
        }
        Expr::Ref(name) => {
            if let Some(clock) = clock_of.get(name) {
                found.push(*clock);
            }
            Ok(())
        }
        Expr::Call(_, args) => args.iter().try_for_each(|arg| recur(arg, clocks, found)),
        Expr::Neg(inner) | Expr::Not(inner) => recur(inner, clocks, found),
        Expr::Bin(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r)
        | Expr::Elementwise(_, l, r) => {
            recur(l, clocks, found)?;
            recur(r, clocks, found)
        }
        Expr::If(c, a, b) => {
            recur(c, clocks, found)?;
            recur(a, clocks, found)?;
            recur(b, clocks, found)
        }
        _ => Ok(()),
    }
}

/// The one clock an equation is on, refusing an equation that reaches
/// across two.
///
/// A variable belongs to exactly one clock, so an equation naming two
/// without a conversion between them is not an equation this language
/// has a meaning for.
pub(super) fn one_clock(
    found: &[usize],
    clocks: &mut Clocks,
    target: &str,
) -> Result<Option<usize>, String> {
    // A place waiting for a clock takes whichever the equation also
    // names, which is the whole of the inference: an equation is on one
    // clock, so where it names a known one beside a place waiting for
    // one, the known one is the answer.
    let settled: Vec<usize> = found
        .iter()
        .copied()
        .filter(|clock| clocks.spec(*clock).waiting().is_none())
        .collect();
    let Some(first) = settled
        .first()
        .copied()
        .map(|clock| clocks.canonical(clock))
    else {
        return Ok(None);
    };
    if let Some(other) = settled
        .iter()
        .copied()
        .find(|clock| !clocks.spec(*clock).same(clocks.spec(first)))
    {
        return Err(format!(
            "`{target}` is written on two clocks at once, one ticking {} and one ticking \
             {} - a value belongs to one clock, and crossing between them asks for \
             `subSample`, `superSample` or `hold`",
            clocks.spec(first).describe(),
            clocks.spec(other).describe()
        ));
    }
    let pending: Vec<usize> = found
        .iter()
        .copied()
        .filter(|clock| clocks.spec(*clock).waiting().is_some())
        .collect();
    for waiting in pending {
        work_out(waiting, first, clocks, target)?;
    }
    Ok(Some(first))
}

/// Put a clock where one was waiting, given what the same equation says
/// it has to tick along with.
///
/// A bare `Clock()` simply becomes that clock. A `subSample` with no
/// factor has to find the factor: the answer is however many of the
/// base's ticks make one of the wanted clock's, and it counts only if
/// sampling by it really does give that clock back.
fn work_out(
    waiting: usize,
    wanted: usize,
    clocks: &mut Clocks,
    target: &str,
) -> Result<(), String> {
    let (base, faster, resolution) = clocks
        .spec(waiting)
        .waiting()
        .expect("the caller filtered for one");
    let Some(base) = base else {
        let found = clocks.spec(wanted).clone();
        // `Clock(0, 5)` leaves the numerator to the compiler but keeps
        // the denominator, so whatever turns up has to be a whole
        // number of fifths. A bare `Clock()` said no denominator and
        // takes whatever it meets.
        if let Some(interval) = found.interval() {
            if resolution > 0 && (interval * resolution as f64).fract() != 0.0 {
                return Err(format!(
                    "`{target}` puts a clock ticking every {interval} where the model asked \
                     for one counted in parts of one over {resolution}"
                ));
            }
        }
        clocks.settle(waiting, found);
        return Ok(());
    };
    let operator = if faster { "superSample" } else { "subSample" };
    let from = clocks.spec(base).clone();
    let goal = clocks.spec(wanted).clone();
    let (Some(theirs), Some(mine)) = (from.interval(), goal.interval()) else {
        return Err(format!(
            "the factor of the `{operator}` in `{target}` is left for the compiler to find, \
             and a clock ticking on an event gives it nothing to count"
        ));
    };
    let ratio = if faster { theirs / mine } else { mine / theirs };
    let nowhere = || {
        format!(
            "the factor of the `{operator}` in `{target}` is left for the compiler to find, \
             and no whole number is it: sampling {} to tick {} would take a factor of {ratio}",
            from.describe(),
            goal.describe()
        )
    };
    if !(1.0..=MAX_FACTOR as f64).contains(&ratio) {
        return Err(nowhere());
    }
    let rounded = ratio.round();
    let candidate = if faster {
        from.super_sampled(rounded as i64)?
    } else {
        from.sub_sampled(rounded as i64)?
    };
    // Working the answer back out is the test, not the division: the
    // factor has to give this clock exactly, and a ratio that only
    // rounds to a whole number gives a different clock.
    if !candidate.same(&goal) {
        return Err(nowhere());
    }
    clocks.settle(waiting, candidate);
    Ok(())
}

/// Put the partitions in an order where one that reads another's
/// variables comes after it.
///
/// Two clocks may tick at the same instant, and at that instant the
/// `when` clauses fire in the order they were written, each once. A
/// partition placed before the one defining what it reads would take
/// the value from the tick before - which is not what `subSample` and
/// `superSample` mean.
pub(super) fn in_partition_order(
    lifted: &HashMap<usize, Vec<(String, Expr)>>,
) -> Result<Vec<usize>, String> {
    let mut clocks: Vec<usize> = lifted.keys().copied().collect();
    clocks.sort_unstable();
    let mut placed: Vec<usize> = Vec::new();
    while placed.len() < clocks.len() {
        let next = clocks.iter().copied().find(|clock| {
            if placed.contains(clock) {
                return false;
            }
            let mut named = Vec::new();
            for (_, value) in &lifted[clock] {
                collect_immediate_refs(value, &mut named);
            }
            !clocks.iter().any(|other| {
                other != clock
                    && !placed.contains(other)
                    && lifted[other]
                        .iter()
                        .any(|(target, _)| named.contains(&target.as_str()))
            })
        });
        match next {
            Some(clock) => placed.push(clock),
            None => {
                return Err(
                    "the partitions of this model read each other's values within one tick, \
                     which leaves no order to compute them in - one of the crossings has to \
                     go through `previous`"
                        .to_string(),
                )
            }
        }
    }
    Ok(placed)
}

/// What an equation defines, and whether it defines its rate of change
/// rather than its value.
fn assigned_by(equation: &EquationItem) -> Option<(String, bool)> {
    match &equation.lhs {
        Expr::Ref(name) => Some((name.clone(), false)),
        Expr::Call(name, args) if name == "der" && args.len() == 1 => match &args[0] {
            Expr::Ref(inner) => Some((inner.clone(), true)),
            _ => None,
        },
        _ => None,
    }
}

/// The value a partition worked out at the tick before.
fn pre_of(name: &str) -> Expr {
    Expr::Call("pre".to_string(), vec![Expr::Ref(name.to_string())])
}

fn add(left: Expr, right: Expr) -> Expr {
    Expr::Bin(BinOp::Add, Box::new(left), Box::new(right))
}

fn mul(left: Expr, right: Expr) -> Expr {
    Expr::Bin(BinOp::Mul, Box::new(left), Box::new(right))
}

/// Read a slope somewhere other than where the tick left the state:
/// wherever the expression names one, hand it the stage's guess
/// instead. That is the whole of an explicit method. What reaches back
/// to an earlier tick is left where it is - a guess about this step
/// says nothing about that one.
fn at_the_stage(expr: &Expr, guesses: &HashMap<String, Expr>) -> Expr {
    let recur = |e: &Expr| at_the_stage(e, guesses);
    match expr {
        Expr::Call(name, _) if name == "pre" => expr.clone(),
        Expr::Ref(name) => match guesses.get(name) {
            Some(guess) => guess.clone(),
            None => expr.clone(),
        },
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Elementwise(op, l, r) => {
            Expr::Elementwise(*op, Box::new(recur(l)), Box::new(recur(r)))
        }
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::If(c, a, b) => Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b))),
        _ => expr.clone(),
    }
}

/// One step of an explicit method, written as assignments on the tick.
///
/// `der(x) = f` says nothing about how to get from one tick to the
/// next; the solver method the clock carries does. A tick does two
/// things: it takes the step the slopes worked out at the tick before
/// call for, and then works out the slopes where that step has left it,
/// for the tick after. That is 16.8's `x[i] = x[i-1] + h * xdot[i-1]`
/// with the stages of an explicit Runge-Kutta in place of the one
/// slope, and it makes the first tick leave the state at its start
/// value, there being no earlier slope to step on.
///
/// The stages are kept in variables of their own rather than written
/// out where they are used: a four-stage method whose stages quoted
/// each other would carry four nested copies of every slope.
fn one_step(
    solver: Solver,
    clock: usize,
    states: &[(String, Expr)],
    step: &Expr,
) -> Vec<(String, Expr)> {
    let stage_name = |stage: usize, which: usize| format!("$slope{stage}_{clock}_{which}");
    // The step first: what it lands on is where the slopes are read.
    let mut actions: Vec<(String, Expr)> = states
        .iter()
        .enumerate()
        .map(|(which, (name, _))| {
            let taken = solver
                .mix
                .iter()
                .enumerate()
                .filter(|(_, weight)| **weight != 0.0)
                .fold(pre_of(name), |so_far, (stage, weight)| {
                    add(
                        so_far,
                        mul(
                            mul(step.clone(), Expr::Number(*weight)),
                            pre_of(&stage_name(stage, which)),
                        ),
                    )
                });
            (name.clone(), taken)
        })
        .collect();
    for (stage, weights) in solver.weights.iter().enumerate() {
        // Where this stage thinks each state will have got to: where
        // the tick left it, plus what the earlier stages suggest.
        let guesses: HashMap<String, Expr> = states
            .iter()
            .enumerate()
            .map(|(which, (name, _))| {
                let guess = weights
                    .iter()
                    .enumerate()
                    .filter(|(_, weight)| **weight != 0.0)
                    .fold(Expr::Ref(name.clone()), |so_far, (earlier, weight)| {
                        add(
                            so_far,
                            mul(
                                mul(step.clone(), Expr::Number(*weight)),
                                Expr::Ref(stage_name(earlier, which)),
                            ),
                        )
                    });
                (name.clone(), guess)
            })
            .collect();
        for (which, (_, slope)) in states.iter().enumerate() {
            actions.push((stage_name(stage, which), at_the_stage(slope, &guesses)));
        }
    }
    actions
}

/// The variable a partition counts its own ticks in.
fn counter_name(clock: usize) -> String {
    format!("$tick{clock}")
}

/// The variable an event partition remembers the time of its last tick
/// in, so that `interval` has something to measure back to.
fn last_tick_name(clock: usize) -> String {
    format!("$last{clock}")
}

/// What a bookkeeping variable held at the tick before, plus a step.
fn after(name: &str, step: Expr) -> Expr {
    Expr::Bin(
        BinOp::Add,
        Box::new(Expr::Call(
            "pre".to_string(),
            vec![Expr::Ref(name.to_string())],
        )),
        Box::new(step),
    )
}

/// How long an event clock's last interval was: the time now less the
/// time of the tick before. There is no tick before the first, which
/// is what the start interval of the constructor answers for.
fn elapsed_since_last_tick(spec: &ClockSpec, clock: usize) -> Expr {
    let Root::When(_, start_interval) = &spec.root else {
        unreachable!("a periodic clock answers with the interval it was declared with")
    };
    Expr::If(
        Box::new(Expr::Rel(
            crate::ast::RelOp::Lt,
            Box::new(Expr::Ref(counter_name(clock))),
            Box::new(Expr::Number(1.5)),
        )),
        Box::new(Expr::Number(*start_interval)),
        Box::new(Expr::Bin(
            BinOp::Sub,
            Box::new(Expr::Time),
            Box::new(Expr::Call(
                "pre".to_string(),
                vec![Expr::Ref(last_tick_name(clock))],
            )),
        )),
    )
}

/// Whether a name appears anywhere in an expression, `pre` included -
/// which is where the bookkeeping variables are read.
pub(super) fn mentions_ref(expr: &Expr, wanted: &str) -> bool {
    match expr {
        Expr::Ref(name) => name == wanted,
        Expr::Call(_, args) => args.iter().any(|arg| mentions_ref(arg, wanted)),
        Expr::Neg(inner) | Expr::Not(inner) => mentions_ref(inner, wanted),
        Expr::Bin(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r)
        | Expr::Elementwise(_, l, r) => mentions_ref(l, wanted) || mentions_ref(r, wanted),
        Expr::If(c, a, b) => {
            mentions_ref(c, wanted) || mentions_ref(a, wanted) || mentions_ref(b, wanted)
        }
        _ => false,
    }
}

/// Answer `firstTick` from the counter its partition keeps.
pub(super) fn answer_first_tick(expr: &Expr, counter: &str) -> Expr {
    let recur = |e: &Expr| answer_first_tick(e, counter);
    match expr {
        Expr::Call(name, _) if name == "firstTick" => Expr::Rel(
            crate::ast::RelOp::Eq,
            Box::new(Expr::Ref(counter.to_string())),
            Box::new(Expr::Number(1.0)),
        ),
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Elementwise(op, l, r) => {
            Expr::Elementwise(*op, Box::new(recur(l)), Box::new(recur(r)))
        }
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::If(c, a, b) => Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b))),
        _ => expr.clone(),
    }
}

/// What a clocked expression says once the tick has arrived.
///
/// Every conversion falls away here. A clocked variable is a discrete
/// one that keeps its value between ticks, so reading it at any tick -
/// its own, a faster one, a slower one, a shifted one - is reading the
/// variable. What the conversions did was decide which clock the
/// equation is on, and that is already settled by the time this runs.
pub(super) fn at_the_tick(
    expr: &Expr,
    clocks: &Clocks,
    clock_of: &HashMap<String, usize>,
    here: Option<usize>,
) -> Expr {
    let recur = |e: &Expr| at_the_tick(e, clocks, clock_of, here);
    match expr {
        // Sampling is reading, at the instant of the tick. The clock
        // may be named - `sample(u, c)` - or left for inference, which
        // is what a sampler block writes: `sample(u)` takes the clock
        // of whatever the equation lands on, and by the time this runs
        // that is already settled.
        Expr::Call(name, args)
            if name == "sample"
                && (args.len() == 1 || (args.len() == 2 && is_clock_expr(&args[1], clocks))) =>
        {
            recur(&args[0])
        }
        // The value from the tick before is the value from before this
        // event, which is what `pre` has always meant here.
        Expr::Call(name, args) if name == "previous" && args.len() == 1 => {
            Expr::Call("pre".to_string(), vec![recur(&args[0])])
        }
        Expr::Call(name, args) if name == "interval" && args.len() <= 1 => {
            let named = args
                .first()
                .and_then(|arg| clock_of_expr(arg, clocks, clock_of));
            match named.or(here) {
                Some(clock) => match clocks.spec(clock).interval() {
                    Some(seconds) => Expr::Number(seconds),
                    // An event clock's interval is however long the run
                    // took to raise the condition again, so it is
                    // measured rather than known: the time now, less
                    // the time at the tick before. There is no tick
                    // before the first, which is what the start
                    // interval of the constructor answers for.
                    None => elapsed_since_last_tick(clocks.spec(clock), clock),
                },
                None => Expr::Call(name.clone(), args.iter().map(recur).collect()),
            }
        }
        // `hold` asks for the value a clocked variable keeps between
        // ticks, and every sub-clock conversion asks for the value the
        // other clock last left there. Both are the variable.
        Expr::Call(name, args)
            if !args.is_empty()
                && (name == "hold"
                    || name == "noClock"
                    || SUB_CLOCK.iter().any(|(known, _)| known == name)) =>
        {
            recur(&args[0])
        }
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Elementwise(op, l, r) => {
            Expr::Elementwise(*op, Box::new(recur(l)), Box::new(recur(r)))
        }
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::If(c, a, b) => Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b))),
        _ => expr.clone(),
    }
}

/// The clock an argument of `interval` stands for: a clock by name, or
/// a clocked variable, whose clock is the one being asked about.
fn clock_of_expr(expr: &Expr, clocks: &Clocks, clock_of: &HashMap<String, usize>) -> Option<usize> {
    match expr {
        Expr::Ref(name) => clocks.by_name(name).or_else(|| clock_of.get(name).copied()),
        _ => None,
    }
}

/// Whether an argument is a clock rather than a value: a declared one,
/// a constructor, or a sub-clock conversion of either.
pub(super) fn is_clock_expr(expr: &Expr, clocks: &Clocks) -> bool {
    match expr {
        Expr::Ref(name) => clocks.by_name(name).is_some(),
        Expr::Call(name, args) => {
            name == "Clock"
                || (SUB_CLOCK.iter().any(|(known, _)| known == name)
                    && args
                        .first()
                        .is_some_and(|inner| is_clock_expr(inner, clocks)))
        }
        _ => false,
    }
}

/// A clocked variable read by a continuous equation without asking for
/// the value it holds - which is the one thing that is not allowed.
pub(super) fn clocked_outside_hold(
    expr: &Expr,
    clock_of: &HashMap<String, usize>,
) -> Option<String> {
    let recur = |e: &Expr| clocked_outside_hold(e, clock_of);
    match expr {
        // Inside `hold` is exactly where a clocked variable may be.
        Expr::Call(name, _) if name == "hold" => None,
        Expr::Ref(name) => clock_of.contains_key(name).then(|| name.clone()),
        Expr::Call(_, args) => args.iter().find_map(recur),
        Expr::Neg(inner) | Expr::Not(inner) => recur(inner),
        Expr::Bin(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r)
        | Expr::Elementwise(_, l, r) => recur(l).or_else(|| recur(r)),
        Expr::If(c, a, b) => recur(c).or_else(|| recur(a)).or_else(|| recur(b)),
        _ => None,
    }
}

/// Whether a call by this name appears anywhere in an expression.
pub(super) fn mentions_call(expr: &Expr, wanted: &str) -> bool {
    let mut found = false;
    walk_calls(expr, &mut |name| {
        if name == wanted {
            found = true;
        }
    });
    found
}

/// The names an expression reads on this side of a clock boundary.
///
/// `sample`, `hold` and `noClock` are where one partition meets
/// another: what is under them belongs to the other side and says
/// nothing about this one. Everything else is read through.
fn named_within_the_partition(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Call(name, _) if matches!(name.as_str(), "sample" | "hold" | "noClock") => {}
        Expr::Ref(name) => out.push(name.clone()),
        other => {
            // One level down, then the same rule again: the boundary
            // has to be seen at the node that holds it.
            other.map_children(&mut |child| {
                named_within_the_partition(child, out);
                child.clone()
            });
        }
    }
}

/// Visit the name of every call in an expression.
pub(super) fn walk_calls(expr: &Expr, seen: &mut impl FnMut(&str)) {
    match expr {
        Expr::Call(name, args) => {
            seen(name);
            for arg in args {
                walk_calls(arg, seen);
            }
        }
        Expr::Neg(inner) | Expr::Not(inner) => walk_calls(inner, seen),
        Expr::Bin(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r)
        | Expr::Elementwise(_, l, r) => {
            walk_calls(l, seen);
            walk_calls(r, seen);
        }
        Expr::If(c, a, b) => {
            walk_calls(c, seen);
            walk_calls(a, seen);
            walk_calls(b, seen);
        }
        // A call the run walks carries its own rule for
        // differentiating it, and that rule is calls too.
        Expr::WithDerivative(value, rule, seeds) => {
            walk_calls(value, seen);
            walk_calls(rule, seen);
            for (_, seed) in seeds {
                walk_calls(seed, seen);
            }
        }
        _ => {}
    }
}

/// Put the assignments of one tick in an order where each is ready
/// when its turn comes.
pub(super) fn in_dependency_order(actions: Vec<(String, Expr)>) -> Result<Vec<WhenAction>, String> {
    let targets: Vec<String> = actions.iter().map(|(target, _)| target.clone()).collect();
    let mut placed = vec![false; actions.len()];
    let mut order = Vec::new();
    for _ in 0..actions.len() {
        let next = (0..actions.len()).find(|&index| {
            if placed[index] {
                return false;
            }
            // What the value reads, other than through `previous`.
            let mut named = Vec::new();
            collect_immediate_refs(&actions[index].1, &mut named);
            !named.iter().any(|name| {
                targets
                    .iter()
                    .enumerate()
                    .any(|(other, target)| other != index && !placed[other] && target == name)
            })
        });
        match next {
            Some(index) => {
                placed[index] = true;
                order.push(index);
            }
            None => {
                let stuck: Vec<&str> = (0..actions.len())
                    .filter(|index| !placed[*index])
                    .map(|index| targets[index].as_str())
                    .collect();
                return Err(format!(
                    "the equations on one clock depend on each other in a circle: {stuck:?}"
                ));
            }
        }
    }
    let mut actions: Vec<Option<(String, Expr)>> = actions.into_iter().map(Some).collect();
    Ok(order
        .into_iter()
        .map(|index| {
            let (target, value) = actions[index].take().expect("each placed once");
            WhenAction::Assign(target, value)
        })
        .collect())
}

/// The names an expression reads at this tick: what sits inside
/// `previous` came from the tick before and does not count.
pub(super) fn collect_immediate_refs<'a>(expr: &'a Expr, out: &mut Vec<&'a str>) {
    match expr {
        Expr::Call(name, _) if name == "pre" || name == "previous" => {}
        Expr::Ref(name) => out.push(name),
        Expr::Call(_, args) => args.iter().for_each(|arg| collect_immediate_refs(arg, out)),
        Expr::Neg(inner) | Expr::Not(inner) => collect_immediate_refs(inner, out),
        Expr::Bin(_, l, r)
        | Expr::Rel(_, l, r)
        | Expr::And(l, r)
        | Expr::Or(l, r)
        | Expr::Elementwise(_, l, r) => {
            collect_immediate_refs(l, out);
            collect_immediate_refs(r, out);
        }
        Expr::If(c, a, b) => {
            collect_immediate_refs(c, out);
            collect_immediate_refs(a, out);
            collect_immediate_refs(b, out);
        }
        _ => {}
    }
}

/// What the states of a machine say about a variable declared outside
/// them: which machine, which of its states, and the value it gives.
type SaidInStates = Vec<(usize, usize, Expr)>;

/// One state machine: its states, its arrows, and where it sits.
#[derive(Clone)]
pub(super) struct Machine {
    /// The states, the one it starts in first.
    states: Vec<String>,
    /// The arrows joining them.
    arrows: Vec<Transition>,
    /// The machine this one is inside, and which of that machine's
    /// states holds it. A machine at the top of the model has none.
    inside: Option<(usize, usize)>,
}

/// Split what the model declared into machines.
///
/// A machine is a set of states joined by arrows, and nothing joins one
/// machine to another - so the arrows say where one ends and the next
/// begins. A machine whose states all live under one state of another
/// is inside it.
fn partition(model: &Model) -> Result<Vec<Machine>, String> {
    // Every state named anywhere, gathered into groups that arrows
    // reach across.
    let mut groups: Vec<Vec<String>> = Vec::new();
    let join = |a: &str, b: &str, groups: &mut Vec<Vec<String>>| {
        let at = |name: &str, groups: &Vec<Vec<String>>| {
            groups
                .iter()
                .position(|group| group.iter().any(|s| s == name))
        };
        match (at(a, groups), at(b, groups)) {
            (Some(one), Some(other)) if one != other => {
                let moved = groups.remove(one.max(other));
                groups[one.min(other)].extend(moved);
            }
            (Some(_), Some(_)) => {}
            (Some(one), None) => groups[one].push(b.to_string()),
            (None, Some(other)) => groups[other].push(a.to_string()),
            (None, None) => groups.push(vec![a.to_string(), b.to_string()]),
        }
    };
    for transition in &model.transitions {
        join(&transition.from, &transition.to, &mut groups);
    }
    // A machine of one state has no arrows to be found by.
    for state in &model.initial_states {
        if !groups.iter().any(|group| group.iter().any(|s| s == state)) {
            groups.push(vec![state.clone()]);
        }
    }

    let mut machines = Vec::new();
    for group in groups {
        let starts: Vec<&String> = model
            .initial_states
            .iter()
            .filter(|state| group.contains(state))
            .collect();
        let start = match starts.as_slice() {
            [one] => (*one).clone(),
            [] => {
                return Err(format!(
                    "the states {group:?} are joined by arrows and none of them is where \
                     the machine starts - one `initialState` to a machine"
                ))
            }
            several => {
                return Err(format!(
                    "the states {group:?} are one machine, and {} of them are named as \
                     where it starts",
                    several.len()
                ))
            }
        };
        // The one it starts in first, the rest in the order the arrows
        // named them.
        let mut states = vec![start];
        for transition in &model.transitions {
            for end in [&transition.from, &transition.to] {
                if group.contains(end) && !states.contains(end) {
                    states.push(end.clone());
                }
            }
        }
        let arrows = model
            .transitions
            .iter()
            .filter(|transition| group.contains(&transition.from))
            .cloned()
            .collect();
        machines.push(Machine {
            states,
            arrows,
            inside: None,
        });
    }

    // Which machine holds which: a machine sits inside the state whose
    // instance path every one of its own states is under, and inside
    // the innermost such state where there are several.
    for index in 0..machines.len() {
        let mut best: Option<(usize, usize, usize)> = None;
        for (outer, holding) in machines.iter().enumerate() {
            if outer == index {
                continue;
            }
            for (at, state) in holding.states.iter().enumerate() {
                let under = format!("{state}.");
                if machines[index]
                    .states
                    .iter()
                    .all(|inner| inner.starts_with(&under))
                    && best.is_none_or(|(_, _, deep)| state.len() > deep)
                {
                    best = Some((outer, at, state.len()));
                }
            }
        }
        machines[index].inside = best.map(|(outer, at, _)| (outer, at));
    }
    // Outermost first, so a machine can be built knowing the one that
    // holds it has been.
    let depth = |machine: &Machine, machines: &Vec<Machine>| {
        let mut deep = 0;
        let mut at = machine.inside;
        while let Some((outer, _)) = at {
            deep += 1;
            at = machines[outer].inside;
        }
        deep
    };
    let order: Vec<usize> = {
        let mut order: Vec<usize> = (0..machines.len()).collect();
        order.sort_by_key(|index| depth(&machines[*index], &machines));
        order
    };
    let mut sorted: Vec<Machine> = Vec::new();
    for index in &order {
        let mut machine = machines[*index].clone();
        machine.inside = machine
            .inside
            .map(|(outer, at)| (order.iter().position(|o| o == &outer).expect("kept"), at));
        sorted.push(machine);
    }
    Ok(sorted)
}

/// Turn the arrows of a state machine into equations on its clock.
///
/// A machine keeps one variable of its own, the state it is in. At
/// each tick it looks at the arrows leaving that state, in priority
/// order, and takes the first whose condition holds - judged on the
/// values from the tick before, since this tick's are decided by where
/// the machine goes. The states' own equations then run guarded: the
/// one that is in force computes, and the others hold what they had.
pub(super) fn build_state_machines(
    model: &mut Model,
    clocks: &Clocks,
    clock_of: &mut HashMap<String, usize>,
) -> Result<(), String> {
    if model.transitions.is_empty() && model.initial_states.is_empty() {
        return Ok(());
    }
    let Some(clock) = clocks.only_named() else {
        return Err(format!(
            "a state machine runs on a clock, and this model declares {} of them",
            clocks.count()
        ));
    };
    // `timeInState` is the tick count times the period, and an event
    // clock has no period to multiply by: how long a state has been
    // held is then a question only the run can answer.
    let period = clocks.spec(clock).interval();
    if period.is_none()
        && model
            .equations
            .iter()
            .any(|equation| mentions_call(&equation.rhs, "timeInState"))
    {
        return Err(
            "`timeInState` counts periods, and this machine's clock ticks on an event \
             rather than on a period - `ticksInState` is what it can answer"
                .to_string(),
        );
    }
    // Never read where there is none: the check above saw to that.
    let period = period.unwrap_or(0.0);
    let machines = partition(model)?;

    // A state is an instance with equations of its own; a plain
    // variable cannot be one.
    for machine in &machines {
        for state in &machine.states {
            let under = format!("{state}.");
            if !model
                .components
                .iter()
                .any(|component| component.name.starts_with(&under))
            {
                return Err(format!(
                    "`{state}` is named as a state but is not a component with anything in it"
                ));
            }
        }
        // Two arrows out of one state must say which goes first, and
        // the specification asks that they never say the same thing.
        for (index, one) in machine.arrows.iter().enumerate() {
            for other in machine.arrows.iter().skip(index + 1) {
                if one.from == other.from && one.priority == other.priority {
                    return Err(format!(
                        "two arrows leave `{}` with priority {}, and the one to take is then \
                         nobody's decision",
                        one.from, one.priority
                    ));
                }
            }
        }
    }

    // Which machine and which of its states every variable belongs to,
    // by the instance path it was flattened under - the innermost state
    // that holds it, since a state inside a state holds both paths. A
    // parameter is not one of these: it does not change, so it has no
    // value from before to reach back to.
    let varying: Vec<String> = model
        .components
        .iter()
        .filter(|component| {
            !matches!(
                component.variability,
                Variability::Parameter | Variability::Constant
            )
        })
        .map(|component| component.name.clone())
        .collect();
    let owner = |name: &str| -> Option<(usize, usize)> {
        if !varying.iter().any(|known| known == name) {
            return None;
        }
        let mut best: Option<(usize, usize, usize)> = None;
        for (tag, machine) in machines.iter().enumerate() {
            for (at, state) in machine.states.iter().enumerate() {
                if name.starts_with(&format!("{state}."))
                    && best.is_none_or(|(_, _, deep)| state.len() > deep)
                {
                    best = Some((tag, at, state.len()));
                }
            }
        }
        best.map(|(tag, at, _)| (tag, at))
    };
    let in_a_state = |name: &str| owner(name).map(|(_, at)| at);

    let state_var = |tag: usize| format!("$state{tag}");
    let ticks_var = |tag: usize| format!("$ticks{tag}");
    let reset_var = |tag: usize| format!("$reset{tag}");
    let previous_of = |name: &str| Expr::Call("previous".to_string(), vec![Expr::Ref(name.into())]);
    let is = |name: &str, index: usize| {
        Expr::Rel(
            crate::ast::RelOp::Eq,
            Box::new(Expr::Ref(name.to_string())),
            Box::new(Expr::Number(index as f64)),
        )
    };
    let was = |name: &str, index: usize| {
        Expr::Rel(
            crate::ast::RelOp::Eq,
            Box::new(previous_of(name)),
            Box::new(Expr::Number(index as f64)),
        )
    };

    // A machine sitting inside a state runs only while that state is in
    // force, and starts over where the arrow that reached it asked.
    // This is 17.3.3's `active` input, and it is the whole of what makes
    // a machine hierarchical.
    let held_by = |machine: &Machine| -> Option<(Expr, Expr)> {
        let (outer, at) = machine.inside?;
        let alive = is(&state_var(outer), at);
        let arrived = Expr::And(
            Box::new(Expr::Not(Box::new(was(&state_var(outer), at)))),
            Box::new(Expr::Rel(
                crate::ast::RelOp::Gt,
                Box::new(Expr::Ref(reset_var(outer))),
                Box::new(Expr::Number(0.5)),
            )),
        );
        Some((alive.clone(), Expr::And(Box::new(alive), Box::new(arrived))))
    };

    // The states a machine may stop in: those no arrow leaves. A
    // `synchronize` arrow waits for every machine inside the state it
    // leaves to have reached one - and asks about the tick before, the
    // way every other condition here does. Asking about this one would
    // be asking the machine inside about a tick whose answer waits on
    // the machine outside, which waits on it.
    let settled = |tag: usize| -> Option<Expr> {
        let machine = &machines[tag];
        let mut resting: Option<Expr> = None;
        for (at, state) in machine.states.iter().enumerate() {
            if machine.arrows.iter().any(|arrow| &arrow.from == state) {
                continue;
            }
            let here = was(&state_var(tag), at);
            resting = Some(match resting {
                Some(so_far) => Expr::Or(Box::new(so_far), Box::new(here)),
                None => here,
            });
        }
        resting
    };

    let mut bookkeeping: Vec<(String, &str, Expr)> = Vec::new();
    let mut machine_equations: Vec<(String, Expr)> = Vec::new();
    for (tag, machine) in machines.iter().enumerate() {
        let (active, ticks, resetting) = (state_var(tag), ticks_var(tag), reset_var(tag));
        let index_of = |state: &str| {
            machine
                .states
                .iter()
                .position(|candidate| candidate == state)
                .expect("the states were gathered from the arrows") as f64
        };

        let mut arrows: Vec<(usize, &Transition)> = machine.arrows.iter().enumerate().collect();
        arrows.sort_by_key(|(_, transition)| (transition.priority, transition.from.clone()));
        // An arrow is ready when the machine is where it leaves from and
        // its condition holds. An immediate arrow is taken on that at
        // once; a delayed one keeps the answer for a tick and is taken
        // on what it kept, which is the whole of `immediate = false`.
        let mut armed: Vec<(String, Expr)> = Vec::new();
        let mut guards: Vec<(Expr, &Transition)> = Vec::new();
        for (index, transition) in &arrows {
            let mut ready = Expr::And(
                Box::new(was(&active, index_of(&transition.from) as usize)),
                Box::new(look_back(&transition.condition, &in_a_state)),
            );
            // A `synchronize` arrow waits for the machines inside the
            // state it leaves to have finished.
            if transition.synchronize {
                let under = format!("{}.", transition.from);
                let mut inside = machines
                    .iter()
                    .enumerate()
                    .filter(|(_, held)| held.states.iter().all(|s| s.starts_with(&under)))
                    .filter_map(|(held, _)| settled(held));
                let Some(first) = inside.next() else {
                    return Err(format!(
                        "the arrow leaving `{}` waits for the machines inside it to finish, \
                         and there are none there to wait for",
                        transition.from
                    ));
                };
                let finished = inside.fold(first, |so_far, one| {
                    Expr::And(Box::new(so_far), Box::new(one))
                });
                ready = Expr::And(Box::new(ready), Box::new(finished));
            }
            let guard = if transition.immediate {
                ready
            } else {
                let kept = format!("$arm{tag}_{index}");
                armed.push((kept.clone(), ready));
                previous_of(&kept)
            };
            guards.push((guard, transition));
        }
        let mut next = previous_of(&active);
        let mut resets_now = Expr::Number(0.0);
        for (guard, transition) in guards.iter().rev() {
            next = Expr::If(
                Box::new(guard.clone()),
                Box::new(Expr::Number(index_of(&transition.to))),
                Box::new(next),
            );
            // Only the arrow taken decides whether what it arrives at is
            // put back to its start values.
            resets_now = Expr::If(
                Box::new(guard.clone()),
                Box::new(Expr::Number(if transition.reset { 1.0 } else { 0.0 })),
                Box::new(resets_now),
            );
        }

        let nowhere = -1.0;
        let first_tick = Expr::Rel(
            crate::ast::RelOp::Lt,
            Box::new(previous_of(&active)),
            Box::new(Expr::Number(0.0)),
        );
        // Before the first tick the machine is nowhere, so the first
        // tick is an arrival at the state it starts in like any other -
        // which is what makes that state's variables start from their
        // start values.
        let mut next = Expr::If(
            Box::new(first_tick.clone()),
            Box::new(Expr::Number(0.0)),
            Box::new(next),
        );
        let mut resets_now = Expr::If(
            Box::new(first_tick),
            Box::new(Expr::Number(1.0)),
            Box::new(resets_now),
        );
        let mut ticks_now = Expr::If(
            Box::new(Expr::Rel(
                crate::ast::RelOp::Eq,
                Box::new(Expr::Ref(active.clone())),
                Box::new(previous_of(&active)),
            )),
            Box::new(Expr::Bin(
                BinOp::Add,
                Box::new(previous_of(&ticks)),
                Box::new(Expr::Number(1.0)),
            )),
            Box::new(Expr::Number(0.0)),
        );
        // A machine inside a state holds still while that state is not
        // the one in force, and starts over where it was entered afresh.
        if let Some((alive, restart)) = held_by(machine) {
            next = Expr::If(
                Box::new(restart),
                Box::new(Expr::Number(0.0)),
                Box::new(Expr::If(
                    Box::new(alive.clone()),
                    Box::new(next),
                    Box::new(previous_of(&active)),
                )),
            );
            resets_now = Expr::If(
                Box::new(alive.clone()),
                Box::new(resets_now),
                Box::new(Expr::Number(0.0)),
            );
            ticks_now = Expr::If(
                Box::new(alive.clone()),
                Box::new(ticks_now),
                Box::new(previous_of(&ticks)),
            );
            for (_, value) in &mut armed {
                *value = Expr::And(Box::new(alive.clone()), Box::new(value.clone()));
            }
        }

        machine_equations.push((active.clone(), next));
        machine_equations.push((resetting.clone(), resets_now));
        machine_equations.push((ticks.clone(), ticks_now));
        machine_equations.extend(armed.iter().cloned());
        bookkeeping.push((active, "Real", Expr::Number(nowhere)));
        bookkeeping.push((ticks, "Real", Expr::Number(0.0)));
        bookkeeping.push((resetting, "Real", Expr::Number(0.0)));
        // What a delayed arrow keeps for a tick is a truth, not a number.
        bookkeeping.extend(
            armed
                .iter()
                .map(|(name, _)| (name.clone(), "Boolean", Expr::Bool(false))),
        );
    }
    for (name, of, start) in bookkeeping {
        model.components.push(Component {
            name: name.clone(),
            type_name: of.to_string(),
            variability: Variability::Discrete,
            start: Some(start),
            description: Some("state machine bookkeeping".to_string()),
            ..blank_component()
        });
        clock_of.insert(name, clock);
    }

    // The states' equations, guarded by the state being in force.
    let starts: HashMap<String, Expr> = model
        .components
        .iter()
        .filter_map(|component| Some((component.name.clone(), component.start.clone()?)))
        .collect();
    // Which state an equation was written inside, for one whose target
    // lives outside every state: `outer output v` written by several of
    // them is one definition of `v`, merged here.
    let written_in = |origin: &str| -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize, usize)> = None;
        for (tag, machine) in machines.iter().enumerate() {
            for (at, state) in machine.states.iter().enumerate() {
                if (origin == state || origin.starts_with(&format!("{state}.")))
                    && best.is_none_or(|(_, _, deep)| state.len() > deep)
                {
                    best = Some((tag, at, state.len()));
                }
            }
        }
        best.map(|(tag, at, _)| (tag, at))
    };
    let mut shared: Vec<(String, SaidInStates)> = Vec::new();
    let mut kept = Vec::new();
    for equation in model.equations.drain(..) {
        let Expr::Ref(target) = &equation.lhs else {
            kept.push(equation);
            continue;
        };
        let Some((tag, state)) = owner(target) else {
            // Not one of a state's own variables. Written inside one,
            // it is that state's say in what the variable holds, and
            // the says are merged into one definition below.
            match written_in(&equation.origin) {
                Some((tag, at)) => {
                    let name = target.clone();
                    let rhs = equation.rhs.clone();
                    match shared.iter_mut().find(|(known, _)| known == &name) {
                        Some((_, says)) => says.push((tag, at, rhs)),
                        None => shared.push((name, vec![(tag, at, rhs)])),
                    }
                }
                None => kept.push(equation),
            }
            continue;
        };
        let (active, resetting) = (state_var(tag), reset_var(tag));
        let in_force = is(&active, state);
        let holding = previous_of(target);
        let mut value = Expr::If(
            Box::new(in_force.clone()),
            Box::new(equation.rhs),
            Box::new(holding),
        );
        // Arriving here puts this state's variables back to their start
        // values where the arrow taken asked for it - the arrow, not
        // the state: one leading here may ask and another not.
        let entered = Expr::And(
            Box::new(in_force),
            Box::new(Expr::Not(Box::new(was(&active, state)))),
        );
        let asked = Expr::And(
            Box::new(entered),
            Box::new(Expr::Rel(
                crate::ast::RelOp::Gt,
                Box::new(Expr::Ref(resetting)),
                Box::new(Expr::Number(0.5)),
            )),
        );
        let back_to = starts.get(target).cloned().unwrap_or(Expr::Number(0.0));
        value = Expr::If(Box::new(asked), Box::new(back_to), Box::new(value));
        clock_of.insert(target.clone(), clock);
        kept.push(EquationItem {
            lhs: equation.lhs,
            rhs: value,
            origin: String::new(),
        });
    }
    model.equations = kept;
    // What several states say about one variable is one definition of
    // it: whichever state is in force has its say, and where none does
    // the variable keeps what it held. That is 17.3.5, with `last` here
    // being simply the value from the tick before.
    for (target, says) in shared {
        if model
            .equations
            .iter()
            .any(|equation| equation.lhs == Expr::Ref(target.clone()))
        {
            return Err(format!(
                "`{target}` is written both inside a state and outside every state, and \
                 a variable has one definition"
            ));
        }
        let mut value = previous_of(&target);
        for (tag, at, rhs) in says.iter().rev() {
            value = Expr::If(
                Box::new(is(&state_var(*tag), *at)),
                Box::new(rhs.clone()),
                Box::new(value),
            );
        }
        clock_of.insert(target.clone(), clock);
        model.equations.push(EquationItem {
            lhs: Expr::Ref(target),
            rhs: value,
            origin: String::new(),
        });
    }

    // `activeState`, `ticksInState` and `timeInState` say what they
    // mean once the machines have variables to say it with. Which
    // machine a question is about is the one whose state it was asked
    // inside; asked outside every state, only a model with one machine
    // can answer.
    let named: HashMap<String, (usize, usize)> = machines
        .iter()
        .enumerate()
        .flat_map(|(tag, machine)| {
            machine
                .states
                .iter()
                .enumerate()
                .map(move |(at, state)| (state.clone(), (tag, at)))
        })
        .collect();
    let mut asked_outside = false;
    for equation in &mut model.equations {
        let here = match &equation.lhs {
            Expr::Ref(target) => owner(target).map(|(tag, _)| tag),
            _ => None,
        };
        if here.is_none()
            && machines.len() > 1
            && (mentions_call(&equation.rhs, "ticksInState")
                || mentions_call(&equation.rhs, "timeInState"))
        {
            asked_outside = true;
        }
        let tag = here.unwrap_or(0);
        equation.rhs = machine_queries(&equation.rhs, &named, &state_var, &ticks_var(tag), period);
    }
    if asked_outside {
        return Err(
            "`ticksInState` and `timeInState` are about the machine they are asked inside, \
             and this model has more than one - ask them among a state's own equations"
                .to_string(),
        );
    }
    for (target, value) in machine_equations {
        let tag = owner(&target).map(|(tag, _)| tag).unwrap_or(0);
        let value = machine_queries(&value, &named, &state_var, &ticks_var(tag), period);
        model.equations.push(EquationItem {
            lhs: Expr::Ref(target),
            rhs: value,
            origin: String::new(),
        });
    }
    model.transitions.clear();
    model.initial_states.clear();
    Ok(())
}

/// Wrap the state machine's own variables in `previous`, so a
/// condition is judged on the values from the tick before.
pub(super) fn look_back(expr: &Expr, owner: &impl Fn(&str) -> Option<usize>) -> Expr {
    let recur = |e: &Expr| look_back(e, owner);
    match expr {
        Expr::Ref(name) if owner(name).is_some() => {
            Expr::Call("previous".to_string(), vec![expr.clone()])
        }
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::If(c, a, b) => Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b))),
        _ => expr.clone(),
    }
}

/// Answer what a model asks about the machine it declared.
pub(super) fn machine_queries(
    expr: &Expr,
    named: &HashMap<String, (usize, usize)>,
    state_var: &impl Fn(usize) -> String,
    ticks: &str,
    period: f64,
) -> Expr {
    let recur = |e: &Expr| machine_queries(e, named, state_var, ticks, period);
    match expr {
        Expr::Call(name, args) if name == "activeState" && args.len() == 1 => {
            let wanted = match &args[0] {
                Expr::Ref(state) => named.get(state),
                _ => None,
            };
            match wanted {
                Some((tag, at)) => Expr::Rel(
                    crate::ast::RelOp::Eq,
                    Box::new(Expr::Ref(state_var(*tag))),
                    Box::new(Expr::Number(*at as f64)),
                ),
                None => Expr::Call(name.clone(), args.iter().map(recur).collect()),
            }
        }
        Expr::Call(name, args) if name == "ticksInState" && args.is_empty() => {
            Expr::Ref(ticks.to_string())
        }
        Expr::Call(name, args) if name == "timeInState" && args.is_empty() => Expr::Bin(
            BinOp::Mul,
            Box::new(Expr::Ref(ticks.to_string())),
            Box::new(Expr::Number(period)),
        ),
        Expr::Call(name, args) => Expr::Call(name.clone(), args.iter().map(recur).collect()),
        Expr::Neg(inner) => Expr::Neg(Box::new(recur(inner))),
        Expr::Not(inner) => Expr::Not(Box::new(recur(inner))),
        Expr::Bin(op, l, r) => Expr::Bin(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::Rel(op, l, r) => Expr::Rel(*op, Box::new(recur(l)), Box::new(recur(r))),
        Expr::And(l, r) => Expr::And(Box::new(recur(l)), Box::new(recur(r))),
        Expr::Or(l, r) => Expr::Or(Box::new(recur(l)), Box::new(recur(r))),
        Expr::If(c, a, b) => Expr::If(Box::new(recur(c)), Box::new(recur(a)), Box::new(recur(b))),
        _ => expr.clone(),
    }
}

/// A component with nothing said about it, for the ones the compiler
/// makes up for its own bookkeeping.
pub(super) fn blank_component() -> Component {
    Component {
        name: String::new(),
        type_name: "Real".to_string(),
        flow: false,
        stream: false,
        dimensions: Vec::new(),
        causality: Causality::None,
        modifiers: Vec::new(),
        variability: Variability::Continuous,
        start: None,
        fixed: None,
        unit: None,
        min: None,
        max: None,
        binding: None,
        description: None,
        scope: Scope::Local,
        replaceable: false,
        constrained_by: None,
        condition: None,
        redeclares: Vec::new(),
        redeclaration: false,
        is_final: false,
        protected: false,
        each_modifiers: Vec::new(),
        annotations: Vec::new(),
    }
}
