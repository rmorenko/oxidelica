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
    let declared: Vec<String> = model
        .components
        .iter()
        .filter(|component| component.type_name == "Clock")
        .map(|component| component.name.clone())
        .collect();
    if declared.is_empty() {
        // A machine with no clock to run on still has to hear about
        // it, so it is asked before this pass gives up.
        return build_state_machines(model, &Clocks::default(), &mut HashMap::new());
    }
    let parameters: HashMap<String, f64> = model
        .components
        .iter()
        .filter_map(|component| {
            let value = component.binding.as_ref()?;
            const_eval(value, &HashMap::new()).map(|number| (component.name.clone(), number))
        })
        .collect();

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
        match &equation.lhs {
            Expr::Ref(target) if declared.contains(target) => {
                definitions.push((target.clone(), equation.rhs));
            }
            _ => kept.push(equation),
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
    let Some(first) = settled.first().copied() else {
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
        // Sampling is reading, at the instant of the tick.
        Expr::Call(name, args)
            if name == "sample" && args.len() == 2 && is_clock_expr(&args[1], clocks) =>
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
    let Some(start) = model.initial_states.first().cloned() else {
        return Err("a state machine needs `initialState(...)` to say where it starts".to_string());
    };
    if model.initial_states.len() > 1 {
        return Err("only one state machine to a model, for now".to_string());
    }

    // The states, numbered: the one it starts in first, the rest in
    // the order the arrows name them.
    let mut states = vec![start.clone()];
    for transition in &model.transitions {
        for end in [&transition.from, &transition.to] {
            if !states.contains(end) {
                states.push(end.clone());
            }
        }
    }
    let number = |state: &str| {
        states
            .iter()
            .position(|candidate| candidate == state)
            .map(|index| index as f64)
    };
    let index_of = |state: &str| number(state).expect("the states were gathered from the arrows");

    // A state is an instance with equations of its own; a plain
    // variable cannot be one.
    for state in &states {
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

    // Which state each variable of the model belongs to, by the
    // instance path it was flattened under. A parameter of a state is
    // not one of these: it does not change, so it has no value from
    // before to reach back to.
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
    let owner = |name: &str| -> Option<usize> {
        if !varying.iter().any(|known| known == name) {
            return None;
        }
        states
            .iter()
            .position(|state| name.starts_with(&format!("{state}.")))
    };

    let active = "$state".to_string();
    let ticks = "$ticks".to_string();
    let previous_of = |name: &str| Expr::Call("previous".to_string(), vec![Expr::Ref(name.into())]);

    // Where the machine goes next, arrows in priority order.
    let mut arrows: Vec<&Transition> = model.transitions.iter().collect();
    arrows.sort_by_key(|transition| (transition.priority, transition.from.clone()));
    // Before the first tick the machine is nowhere, so the first tick
    // is an arrival at the initial state like any other - which is
    // what makes its variables start from their start values.
    let mut next = previous_of(&active);
    for transition in arrows.iter().rev() {
        let (from, to) = (index_of(&transition.from), index_of(&transition.to));
        // The condition is judged on the values from the tick before:
        // this tick's belong to whichever state the machine settles
        // on, which is what is being decided.
        let condition = Expr::And(
            Box::new(Expr::Rel(
                crate::ast::RelOp::Eq,
                Box::new(previous_of(&active)),
                Box::new(Expr::Number(from)),
            )),
            Box::new(look_back(&transition.condition, &owner)),
        );
        next = Expr::If(
            Box::new(condition),
            Box::new(Expr::Number(to)),
            Box::new(next),
        );
    }

    // The machine's own variables, and the arrival counter behind
    // `ticksInState` and `timeInState`.
    let nowhere = -1.0;
    let next = Expr::If(
        Box::new(Expr::Rel(
            crate::ast::RelOp::Lt,
            Box::new(previous_of(&active)),
            Box::new(Expr::Number(0.0)),
        )),
        Box::new(Expr::Number(number(&start).unwrap_or(0.0))),
        Box::new(next),
    );
    let mut machine = vec![
        (active.clone(), next),
        (
            ticks.clone(),
            Expr::If(
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
            ),
        ),
    ];
    for (name, start) in [(&active, nowhere), (&ticks, 0.0)] {
        model.components.push(Component {
            name: name.clone(),
            type_name: "Real".to_string(),
            variability: Variability::Discrete,
            start: Some(Expr::Number(start)),
            description: Some("state machine bookkeeping".to_string()),
            ..blank_component()
        });
        clock_of.insert(name.clone(), clock);
    }

    // Which states are entered with their variables put back to their
    // start values, as `reset = true` asks.
    let resets: Vec<bool> = states
        .iter()
        .map(|state| {
            model
                .transitions
                .iter()
                .any(|transition| &transition.to == state && transition.reset)
        })
        .collect();

    // The states' equations, guarded by the state being in force.
    let starts: HashMap<String, Expr> = model
        .components
        .iter()
        .filter_map(|component| Some((component.name.clone(), component.start.clone()?)))
        .collect();
    let mut kept = Vec::new();
    for equation in model.equations.drain(..) {
        let Expr::Ref(target) = &equation.lhs else {
            kept.push(equation);
            continue;
        };
        let Some(state) = owner(target) else {
            kept.push(equation);
            continue;
        };
        let in_force = Expr::Rel(
            crate::ast::RelOp::Eq,
            Box::new(Expr::Ref(active.clone())),
            Box::new(Expr::Number(state as f64)),
        );
        let holding = previous_of(target);
        let mut value = Expr::If(
            Box::new(in_force.clone()),
            Box::new(equation.rhs),
            Box::new(holding),
        );
        if resets[state] {
            let entered = Expr::And(
                Box::new(in_force),
                Box::new(Expr::Not(Box::new(Expr::Rel(
                    crate::ast::RelOp::Eq,
                    Box::new(previous_of(&active)),
                    Box::new(Expr::Number(state as f64)),
                )))),
            );
            let back_to = starts.get(target).cloned().unwrap_or(Expr::Number(0.0));
            value = Expr::If(Box::new(entered), Box::new(back_to), Box::new(value));
        }
        clock_of.insert(target.clone(), clock);
        kept.push(EquationItem {
            lhs: equation.lhs,
            rhs: value,
        });
    }
    model.equations = kept;

    // `activeState`, `ticksInState` and `timeInState` say what they
    // mean once the machine has a variable to say it with.
    for equation in &mut model.equations {
        equation.rhs = machine_queries(&equation.rhs, &states, &active, &ticks, period);
    }
    for (_, value) in &mut machine {
        *value = machine_queries(value, &states, &active, &ticks, period);
    }
    for (target, value) in machine {
        model.equations.push(EquationItem {
            lhs: Expr::Ref(target),
            rhs: value,
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
    states: &[String],
    active: &str,
    ticks: &str,
    period: f64,
) -> Expr {
    let recur = |e: &Expr| machine_queries(e, states, active, ticks, period);
    match expr {
        Expr::Call(name, args) if name == "activeState" && args.len() == 1 => {
            let wanted = match &args[0] {
                Expr::Ref(state) => states.iter().position(|s| s == state),
                _ => None,
            };
            match wanted {
                Some(index) => Expr::Rel(
                    crate::ast::RelOp::Eq,
                    Box::new(Expr::Ref(active.to_string())),
                    Box::new(Expr::Number(index as f64)),
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
        each_modifiers: Vec::new(),
    }
}
