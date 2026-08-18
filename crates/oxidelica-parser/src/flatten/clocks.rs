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

/// Greatest common divisor, for keeping a fraction in lowest terms.
fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// What a clock's ticks are counted from.
#[derive(Clone, Debug)]
pub(super) enum Root {
    /// A fixed interval in seconds: `Clock(0.1)` or `Clock(1, 10)`.
    Every(f64),
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
}

impl ClockSpec {
    /// A root clock of its own, ticking every `period` seconds.
    fn every(period: f64) -> ClockSpec {
        ClockSpec {
            root: Root::Every(period),
            rate: Ratio::ONE,
            shift: Ratio::ZERO,
        }
    }

    /// The interval between two ticks, in seconds.
    fn interval(&self) -> f64 {
        let Root::Every(period) = self.root;
        period * self.rate.value()
    }

    /// When the first tick falls, in seconds past the start.
    fn first(&self) -> f64 {
        let Root::Every(period) = self.root;
        period * self.shift.value()
    }

    /// Whether two clocks are the same clock - which is a question
    /// about the fractions, not about the seconds they work out to.
    fn same(&self, other: &ClockSpec) -> bool {
        let (Root::Every(mine), Root::Every(theirs)) = (&self.root, &other.root);
        mine.to_bits() == theirs.to_bits() && self.rate == other.rate && self.shift == other.shift
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
    fn super_sampled(&self, factor: i64) -> Result<ClockSpec, String> {
        Ok(ClockSpec {
            rate: self.rate.times(Ratio::new(1, i128::from(factor))?)?,
            ..self.clone()
        })
    }

    /// The same clock with every tick moved `counter / resolution` of an
    /// interval later, or earlier when `back`.
    fn shifted(&self, counter: i64, resolution: i64, back: bool) -> Result<ClockSpec, String> {
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
        if clocks.spec(index).interval() <= 0.0 {
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
            let Expr::Ref(target) = &equation.lhs else {
                continue;
            };
            if clock_of.contains_key(target) {
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
            if let Some(clock) = one_clock(&found, &clocks, target)? {
                clock_of.insert(target.clone(), clock);
                settled = false;
            }
        }
        if settled {
            break;
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
    for equation in model.equations.drain(..) {
        let clock = match &equation.lhs {
            Expr::Ref(target) => clock_of.get(target).copied(),
            _ => None,
        };
        match (clock, &equation.lhs) {
            (Some(clock), Expr::Ref(target)) => {
                let value = at_the_tick(&equation.rhs, &clocks, &clock_of, Some(clock));
                lifted
                    .entry(clock)
                    .or_default()
                    .push((target.clone(), value));
            }
            _ => kept.push(equation),
        }
    }
    model.equations = kept;
    let mut counters = Vec::new();
    for clock in in_partition_order(&lifted)? {
        let mut actions = lifted.remove(&clock).expect("the order names each once");
        // `firstTick` needs the partition to count its own ticks, and
        // nothing but a counter will do it: a clock has no other way of
        // telling its first activation from its hundredth.
        if actions
            .iter()
            .any(|(_, value)| mentions_call(value, "firstTick"))
        {
            let counter = format!("$tick{clock}");
            for (_, value) in &mut actions {
                *value = answer_first_tick(value, &counter);
            }
            actions.push((
                counter.clone(),
                Expr::Bin(
                    BinOp::Add,
                    Box::new(Expr::Call(
                        "pre".to_string(),
                        vec![Expr::Ref(counter.clone())],
                    )),
                    Box::new(Expr::Number(1.0)),
                ),
            ));
            counters.push((counter, clock));
        }
        // The equations of a partition are equations, in no order of
        // their own; what the tick needs is an order in which each is
        // ready when its turn comes. `previous` reaches back to the
        // tick before, so it is not a reason to wait.
        let actions = in_dependency_order(actions)?;
        let spec = clocks.spec(clock);
        model.when_clauses.push(WhenClause {
            branches: vec![WhenBranch {
                condition: Expr::Call(
                    "sample".to_string(),
                    vec![Expr::Number(spec.first()), Expr::Number(spec.interval())],
                ),
                actions,
            }],
        });
    }
    for (counter, clock) in counters {
        model.components.push(Component {
            name: counter.clone(),
            variability: Variability::Discrete,
            start: Some(Expr::Number(0.0)),
            description: Some("clock tick counter".to_string()),
            ..blank_component()
        });
        clock_of.insert(counter, clock);
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
) -> Result<i64, String> {
    let Some(value) = const_eval(expr, parameters) else {
        return Err(format!(
            "the {what} of a clock operator has to be a number the compiler can work out"
        ));
    };
    if value.fract() != 0.0 || value < 1.0 || value > MAX_FACTOR as f64 {
        return Err(format!(
            "the {what} of a clock operator has to be a whole number \
             between 1 and {MAX_FACTOR}, not {value}"
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
        ("Clock", 1) => match const_eval(&args[0], parameters) {
            Some(interval) => Ok(Some(clocks.intern(ClockSpec::every(interval)))),
            None => {
                Err("`Clock(interval)` needs an interval the compiler can work out".to_string())
            }
        },
        // `Clock(1, 10)` says it as a fraction, which is how a model
        // asks for a rate that no decimal writes exactly.
        ("Clock", 2) => {
            let counter = whole_argument(&args[0], parameters, "interval counter")?;
            let resolution = whole_argument(&args[1], parameters, "resolution")?;
            let interval = counter as f64 / resolution as f64;
            Ok(Some(clocks.intern(ClockSpec::every(interval))))
        }
        _ => {
            let Some((_, takes_resolution)) =
                SUB_CLOCK.iter().find(|(known, _)| known == name).copied()
            else {
                return Ok(None);
            };
            let Some(base) = clock_expr(&args[0], clocks, parameters)? else {
                return Ok(None);
            };
            let spec = clocks.spec(base).clone();
            Ok(Some(clocks.intern(derive(
                &spec,
                name,
                &args[1..],
                takes_resolution,
                parameters,
            )?)))
        }
    }
}

/// One sub-clock conversion applied to a clock.
fn derive(
    base: &ClockSpec,
    operator: &str,
    args: &[Expr],
    takes_resolution: bool,
    parameters: &HashMap<String, f64>,
) -> Result<ClockSpec, String> {
    if args.is_empty() {
        return Err(format!(
            "`{operator}` needs its factor spelled out: this compiler does not infer one"
        ));
    }
    let counter = whole_argument(&args[0], parameters, "factor")?;
    let resolution = match (takes_resolution, args.get(1)) {
        (true, Some(given)) => whole_argument(given, parameters, "resolution")?,
        _ => 1,
    };
    match operator {
        "subSample" => base.sub_sampled(counter),
        "superSample" => base.super_sampled(counter),
        "shiftSample" => base.shifted(counter, resolution, false),
        "backSample" => base.shifted(counter, resolution, true),
        _ => unreachable!("the caller matched the name against the same table"),
    }
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
                let spec = clocks.spec(base).clone();
                let derived = derive(&spec, name, &args[1..], takes_resolution, parameters)?;
                found.push(clocks.intern(derived));
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
    clocks: &Clocks,
    target: &str,
) -> Result<Option<usize>, String> {
    let Some(first) = found.first().copied() else {
        return Ok(None);
    };
    if let Some(other) = found
        .iter()
        .copied()
        .find(|clock| !clocks.spec(*clock).same(clocks.spec(first)))
    {
        return Err(format!(
            "`{target}` is written on two clocks at once, one ticking every {} and one \
             every {} - a value belongs to one clock, and crossing between them asks for \
             `subSample`, `superSample` or `hold`",
            clocks.spec(first).interval(),
            clocks.spec(other).interval()
        ));
    }
    Ok(Some(first))
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
                Some(clock) => Expr::Number(clocks.spec(clock).interval()),
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
    let period = clocks.spec(clock).interval();
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
