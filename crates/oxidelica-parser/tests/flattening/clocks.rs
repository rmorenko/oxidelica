//! Clocks, `when` clauses and state machines: what a model does at an instant rather than throughout.

use super::shared::*;
use oxidelica_parser::{parse_model, Expr};

#[test]
fn a_clock_gathers_the_equations_that_belong_to_it() {
    let m = parse_model(
        "model M Clock c = Clock(0.1); Real u; Real s; Real acc; Real out; \
         equation u = time; s = sample(u, c); \
         acc = previous(acc) + s * interval(c); out = hold(acc); end M;",
    )
    .unwrap();

    // The clock is not a variable of the run.
    assert!(!m.components.iter().any(|component| component.name == "c"));
    // What belongs to it changes only at a tick.
    for name in ["s", "acc"] {
        let component = m.components.iter().find(|c| c.name == name).unwrap();
        assert_eq!(
            component.variability,
            oxidelica_parser::Variability::Discrete,
            "{name}"
        );
    }
    // What does not stays continuous.
    for name in ["u", "out"] {
        let component = m.components.iter().find(|c| c.name == name).unwrap();
        assert_eq!(
            component.variability,
            oxidelica_parser::Variability::Continuous,
            "{name}"
        );
    }

    // One clause on the clock's period, holding both equations,
    // with the conversions saying what they mean at a tick.
    assert_eq!(m.when_clauses.len(), 1);
    let branch = &m.when_clauses[0].branches[0];
    assert_eq!(
        format!("{:?}", branch.condition),
        "Call(\"sample\", [Number(0.0), Number(0.1)])"
    );
    assert_eq!(branch.actions.len(), 2);
    let text = format!("{:?}", branch.actions);
    assert!(
        text.contains("Call(\"pre\""),
        "previous becomes pre: {text}"
    );
    assert!(!text.contains("interval"), "the period is a number: {text}");
    // `hold` leaves nothing behind but the variable itself.
    let held = m
        .equations
        .iter()
        .find(|equation| format!("{:?}", equation.lhs) == "Ref(\"out\")")
        .unwrap();
    assert_eq!(format!("{:?}", held.rhs), "Ref(\"acc\")");
}

#[test]
fn a_clocked_component_reaches_through_every_shape() {
    // The clock walkers have the same arms to visit, and a state
    // machine's condition is read through another of them.
    let m = parse_model(
        "model Sub Clock c = Clock(0.25); Real u; Real held[2]; Real s; Real acc; Boolean flag; equation u = time; s = sample(u, c); flag = not (s > 1) and (s < 5 or false); acc = previous(acc) + (if flag then s else -s) * interval(c); held = {hold(acc), hold(s)}; end Sub; model M Sub sub; Real out; equation out = sub.held[1] + sub.held[2]; end M;",
    )
    .unwrap();
    assert_eq!(m.when_clauses.len(), 1);
    let text = format!("{:?}", m.when_clauses[0].branches[0].actions);
    assert!(text.contains("Call(\"pre\""), "{text}");
    assert!(!text.contains("interval"), "{text}");
    // `hold` left nothing of itself behind.
    assert!(!format!("{:?}", m.equations).contains("hold"));
}

#[test]
fn a_state_machine_inside_a_component_still_works() {
    let m = parse_model(
        "model Machine block Step parameter Real limit = 2; Real n(start = 0); equation n = previous(n) + 1; end Step; Clock c = Clock(0.5); Step a; Step b; Real lamp; equation initialState(a); transition(a, b, not (a.n < a.limit) and true); transition(b, a, b.n >= 1, reset = false, priority = 2); lamp = if activeState(a) then ticksInState() else timeInState(); end Machine; model M Machine m; Real out; equation out = m.lamp; end M;",
    )
    .unwrap();
    // The machine's own variables were made under the instance.
    assert!(m.components.iter().any(|c| c.name == "$state0"));
    let text = format!("{:?}", m.when_clauses);
    for asked in ["activeState", "ticksInState", "timeInState"] {
        assert!(!text.contains(asked), "{asked} survived");
    }
}

#[test]
fn a_when_may_watch_several_conditions_at_once() {
    // Each condition is a branch of its own: a disjunction has no
    // edge left once one of them already holds.
    let m = parse_model(
        "model M Real u; discrete Real hits(start = 0); equation u = time; when {u > 0.3, u > 0.6, u > 0.9} then hits = pre(hits) + 1; end when; end M;",
    )
    .unwrap();
    assert_eq!(m.when_clauses[0].branches.len(), 3);
    assert!(m.when_clauses[0]
        .branches
        .iter()
        .all(|branch| branch.actions.len() == 1));
}

#[test]
fn a_state_machine_becomes_equations_on_its_clock() {
    const MACHINE: &str = "model M block Step Real n(start = 0); \
         equation n = previous(n) + 1; end Step; \
         Clock c = Clock(0.5); Step a; Step b; Real out; \
         equation initialState(a); \
         transition(a, b, a.n >= 2); \
         transition(b, a, b.n >= 1, priority = 2); \
         out = if activeState(a) then ticksInState() else timeInState(); end M;";
    let m = parse_model(MACHINE).unwrap();

    // The machine keeps two variables of its own, and they change
    // only at a tick.
    for name in ["$state0", "$ticks0"] {
        let component = m.components.iter().find(|c| c.name == name).unwrap();
        assert_eq!(
            component.variability,
            oxidelica_parser::Variability::Discrete,
            "{name}"
        );
    }
    // It starts nowhere, so the first tick is an arrival at the
    // initial state like any other.
    let state = m.components.iter().find(|c| c.name == "$state0").unwrap();
    assert_eq!(state.start, Some(Expr::Number(-1.0)));

    // Everything the machine does happens on the clock.
    assert_eq!(m.when_clauses.len(), 1);
    let actions = &m.when_clauses[0].branches[0].actions;
    let assigned: Vec<&str> = actions
        .iter()
        .map(|action| match action {
            oxidelica_parser::WhenAction::Assign(name, _) => name.as_str(),
            _ => "",
        })
        .collect();
    for wanted in ["$state0", "$ticks0", "a.n", "b.n"] {
        assert!(assigned.contains(&wanted), "{wanted} in {assigned:?}");
    }
    // `activeState`, `ticksInState` and `timeInState` are gone,
    // answered by the machine's own variables.
    let text = format!("{:?} {:?}", m.equations, m.when_clauses);
    for asked in ["activeState", "ticksInState", "timeInState", "transition"] {
        assert!(!text.contains(asked), "{asked} survived");
    }
    // A second's worth of ticks is two of them, at this period.
    assert!(text.contains("Number(0.5)"), "the period is in there");
}

#[test]
fn state_machine_error_paths() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // One class starts in one state.
    assert!(err(
        "model M block S Real n(start = 0); equation n = previous(n) + 1; end S; Clock c = Clock(0.5); S a; S b; equation initialState(a); initialState(b); end M;"
    )
    .contains("one initial state too many"));
    // A model may hold several machines: nothing joins one to
    // another, so the arrows say where one ends and the next begins.
    let two = parse_model(
        "model Machine block S Real n(start = 0); equation n = previous(n) + 1; end S; S a; equation initialState(a); end Machine; model M Clock c = Clock(0.5); Machine one; Machine two; end M;"
    )
    .unwrap();
    for name in ["$state0", "$state1"] {
        assert!(two.components.iter().any(|c| c.name == name), "{name}");
    }
    // A variable both merged from the states and defined outside them
    // has two definitions, and a variable has one.
    assert!(
        err("model M block Sig outer output Real v; Real n(start = 0); \
         equation n = previous(n) + 1; v = n; end Sig; \
         Clock c = Clock(1); inner Real v; Sig a; Real y; \
         equation initialState(a); v = 99; y = hold(v); end M;")
        .contains("written both inside a state and outside every state")
    );
    // Asked outside every state, a question about "the machine" has no
    // machine to be about where a model holds more than one.
    assert!(err(
        "model Machine block S Real n(start = 0); equation n = previous(n) + 1; end S; \
         S a; equation initialState(a); end Machine; \
         model M Clock c = Clock(0.5); Machine one; Machine two; Real y; \
         equation y = ticksInState(); end M;"
    )
    .contains("ask them among a state's own equations"));
    // A machine with no clock to run on, or with several.
    assert!(err(
        "model M block S Real n(start = 0); equation n = previous(n) + 1; end S; \
         S a; equation initialState(a); end M;"
    )
    .contains("declares 0 of them"));
    // Arrows with nowhere to start from.
    assert!(err(
        "model M block S Real n(start = 0); equation n = previous(n) + 1; end S; \
         Clock c = Clock(0.5); S a; S b; \
         equation transition(a, b, a.n >= 1); end M;"
    )
    .contains("none of them is where the machine starts"));
    // An arrow to a state the machine does not have is caught by
    // the numbering, which only knows the states it was given.
    assert!(err("model M Clock c = Clock(0.5); Real y; \
         equation initialState(y); y = previous(y) + 1; end M;")
    .contains("is not a component with anything in it"));
    // A priority that is not a whole number from one.
    assert!(err(
        "model M block S Real n(start = 0); equation n = previous(n) + 1; end S; Clock c = Clock(0.5); S a; S b; equation initialState(a); transition(a, b, a.n >= 1, priority = 0.5); end M;"
    )
    .contains("whole number from 1"));
    // Two arrows out of one state saying the same thing about which
    // goes first: the specification asks that they never do.
    assert!(err(
        "model M block S Real n(start = 0); equation n = previous(n) + 1; end S; \
         Clock c = Clock(0.5); S a; S b; \
         equation initialState(a); \
         transition(a, b, a.n >= 1, priority = 1); \
         transition(a, b, a.n >= 3, priority = 1); end M;"
    )
    .contains("nobody's decision"));
    // A delayed arrow keeps its answer for a tick, which takes a
    // variable of its own to keep it in.
    let delayed = parse_model(
        "model M block S Real n(start = 0); equation n = previous(n) + 1; end S; \
         Clock c = Clock(0.5); S a; S b; Real out; \
         equation initialState(a); \
         transition(a, b, a.n >= 1, immediate = false); \
         out = if activeState(a) then 1 else 2; end M;",
    )
    .unwrap();
    let kept = delayed
        .components
        .iter()
        .find(|component| component.name.starts_with("$arm"))
        .expect("a delayed arrow keeps its answer");
    assert_eq!(kept.type_name, "Boolean");
    // An arrow that waits for the machines inside the state it leaves,
    // where that state holds none.
    assert!(err(
        "model M block S Real n(start = 0); equation n = previous(n) + 1; end S; \
         Clock c = Clock(0.5); S a; S b; \
         equation initialState(a); \
         transition(a, b, a.n >= 1, synchronize = true); end M;"
    )
    .contains("there are none there to wait for"));
    // A setting this compiler does not know.
    assert!(err(
        "model M block S Real n(start = 0); equation n = previous(n) + 1; end S; \
         Clock c = Clock(0.5); S a; S b; \
         equation initialState(a); \
         transition(a, b, a.n >= 1, nonsense = true); end M;"
    )
    .contains("not a transition setting"));
}

#[test]
fn a_clock_reaches_through_every_kind_of_expression() {
    // One tick's worth of every form the rewrite has to walk, so
    // the conversions are found wherever they were written.
    let m = parse_model(
        "model M parameter Real Ts = 0.1; parameter Clock p = Clock(Ts); \
         Real u; Boolean flag; Real a; Real b; Real d; Real out; \
         equation u = time; \
         flag = sample(u, p) > 0.5 and not (sample(u, p) < 0) or false; \
         a = if flag then -sample(u, p) else abs(sample(u, p)) + max(1, 2); \
         b = previous(b) + (if a > 0 then 1 else -1) * interval(p); \
         d = previous(a) .* 2; \
         out = hold(a) + hold(b) * 2; end M;",
    )
    .unwrap();
    let branch = &m.when_clauses[0].branches[0];
    assert_eq!(
        format!("{:?}", branch.condition),
        "Call(\"sample\", [Number(0.0), Number(0.1)])"
    );
    assert_eq!(branch.actions.len(), 4);
    let text = format!("{:?}", branch.actions);
    assert!(!text.contains("\"sample\""), "sampling is reading: {text}");
    assert!(!text.contains("interval"), "the period is a number: {text}");
    // `b` reads `a` at this tick, so it comes after it; `d` reads
    // `a` from the tick before and could have come anywhere.
    let order: Vec<&str> = branch
        .actions
        .iter()
        .map(|action| match action {
            oxidelica_parser::WhenAction::Assign(name, _) => name.as_str(),
            _ => "",
        })
        .collect();
    let at = |name: &str| order.iter().position(|n| *n == name).unwrap();
    assert!(at("flag") < at("a") && at("a") < at("b"), "{order:?}");
    // Only the continuous equations are left, with `hold` gone.
    let kept: Vec<String> = m
        .equations
        .iter()
        .map(|equation| format!("{:?}", equation.lhs))
        .collect();
    assert_eq!(kept, vec!["Ref(\"u\")", "Ref(\"out\")"]);
    assert!(!format!("{:?}", m.equations).contains("hold"));
}

#[test]
fn one_clock_is_derived_from_another_by_exact_fractions() {
    // Five clocks written down, but only three of them are clocks: the
    // shift undone by `backSample` and the round trip through
    // `superSample` and `subSample` both land back on `base`, which is
    // the whole reason the rates are kept as fractions. In seconds,
    // `0.1 / 3 * 3` is not `0.1`, and the two would drift apart.
    let m = parse_model(
        "model M Clock base = Clock(1, 10); \
         Clock fast = superSample(base, 2); \
         Clock late = shiftSample(base, 1, 4); \
         Clock back = backSample(late, 1, 4); \
         Clock round = subSample(superSample(base, 3), 3); \
         Real b; Real f; Real l; Real k; Real r; Real out; \
         equation b = previous(b) + interval(base); \
         f = previous(f) + interval(fast); \
         l = previous(l) + interval(late); \
         k = previous(k) + interval(back); \
         r = previous(r) + interval(round); \
         out = hold(b) + hold(f) + hold(l) + hold(k) + hold(r); end M;",
    )
    .unwrap();
    let mut ticks = ticks_of(&m);
    ticks.sort_by(|a, b| a.partial_cmp(b).expect("no clock ticks on a NaN"));
    assert_eq!(ticks, vec![(0.0, 0.05), (0.0, 0.1), (0.025, 0.1)]);
    // `b`, `k` and `r` share a clock, so they share a `when`.
    let together = m
        .when_clauses
        .iter()
        .find(|clause| {
            matches!(&clause.branches[0].condition,
                Expr::Call(_, args) if args[0] == Expr::Number(0.0) && args[1] == Expr::Number(0.1))
        })
        .expect("the base clock is one of them");
    assert_eq!(together.branches[0].actions.len(), 3);

    // Two roads to the same instants arrive at the same clock. Ticking
    // every 0.2 from the start is one clock however it was written, so
    // an equation may hold a clock reached by sub-sampling beside the
    // clock declared outright, and both land in one `when`.
    let m = parse_model(
        "model M Clock fast = Clock(1, 10); Clock slow = Clock(1, 5); \
         Real a; Real b; Real out; \
         equation a = previous(a) + interval(fast); \
         b = subSample(a, 2) + interval(slow); out = hold(b); end M;",
    )
    .unwrap();
    let mut ticks = ticks_of(&m);
    ticks.sort_by(|a, b| a.partial_cmp(b).expect("no clock ticks on a NaN"));
    assert_eq!(ticks, vec![(0.0, 0.1), (0.0, 0.2)]);
    // Sub-sampled by three it is a different clock, and saying so is
    // the point of the check rather than an accident of it.
    assert!(parse_model(
        "model M Clock fast = Clock(1, 10); Clock slow = Clock(1, 5); \
         Real a; Real b; Real out; \
         equation a = previous(a) + interval(fast); \
         b = subSample(a, 3) + interval(slow); out = hold(b); end M;"
    )
    .unwrap_err()
    .to_string()
    .contains("two clocks at once"));

    // A clock the model names only through the operators works the
    // same way: the equation lands on the derived clock, not on the one
    // its argument was written on.
    let m = parse_model(
        "model M Clock base = Clock(0.1); Real u; Real s; Real slow; Real out; \
         equation u = time; s = sample(u, base); slow = subSample(s, 4) + 1; \
         out = hold(slow); end M;",
    )
    .unwrap();
    let mut ticks = ticks_of(&m);
    ticks.sort_by(|a, b| a.partial_cmp(b).expect("no clock ticks on a NaN"));
    assert_eq!(ticks, vec![(0.0, 0.1), (0.0, 0.4)]);
}

#[test]
fn first_tick_is_answered_from_a_counter_the_partition_keeps() {
    // A clock has no way of telling its first activation from its
    // hundredth, so the partition counts, and `firstTick` reads the
    // count. The counter has to be raised before anything asks.
    let m = parse_model(
        "model M Clock c = Clock(0.1); Real n; Real out; \
         equation n = if firstTick() then 0 else previous(n) + interval(c); \
         out = hold(n); end M;",
    )
    .unwrap();
    let actions = &m.when_clauses[0].branches[0].actions;
    assert_eq!(actions.len(), 2);
    assert!(format!("{:?}", actions[0]).contains("$tick"));
    assert!(!format!("{:?}", actions[1]).contains("firstTick"));
    let counter = m
        .components
        .iter()
        .find(|component| component.name.starts_with("$tick"))
        .expect("the partition keeps one");
    assert_eq!(counter.variability, oxidelica_parser::Variability::Discrete);
}

#[test]
fn an_event_clock_ticks_on_its_condition_rather_than_on_a_period() {
    // What the first argument is decides which clock this is: a number
    // the compiler can work out is an interval, anything else is a
    // condition. So the `when` this one is lowered onto fires on the
    // condition itself, not on a `sample`.
    let m = parse_model(
        "model M Real x(start = 0, fixed = true); Clock e = Clock(x > 0.5, 0.25); \
         Real gap; Real n; Real out; \
         equation der(x) = 1; gap = interval(e); n = previous(n) + gap; \
         out = hold(n); end M;",
    )
    .unwrap();
    assert_eq!(m.when_clauses.len(), 1);
    let branch = &m.when_clauses[0].branches[0];
    assert!(
        matches!(&branch.condition, Expr::Rel(..)),
        "{:?}",
        branch.condition
    );
    // Its interval is measured rather than known - the time now less
    // the time at the tick before - so the partition remembers both the
    // count of its ticks and when the last one was.
    for name in ["$tick0", "$last0"] {
        assert!(
            m.components.iter().any(|c| c.name == name),
            "{name} is kept"
        );
    }
    let printed = format!("{:?}", branch.actions);
    assert!(printed.contains("Time"), "{printed}");
    assert!(printed.contains("0.25"), "{printed}");
}

#[test]
fn an_event_clock_may_be_sub_sampled_but_not_placed_between_its_ticks() {
    // Counting rising edges is something a run can do, so `subSample`
    // works; the others ask where a tick falls between two others, and
    // no compiler can say when a condition will rise next.
    let event = "model M Real x(start = 0, fixed = true); Clock e = Clock(x > 0.5, 0.25); ";
    let m = parse_model(&format!(
        "{event} Clock half = subSample(e, 2); Real gap; Real n; Real m; Real out; \
         equation der(x) = 1; gap = interval(e); n = previous(n) + gap; \
         m = subSample(n, 2) + 1; out = hold(m); end M;"
    ))
    .unwrap();
    // The edge arrives every time, so the slower partition counts the
    // ones it skips, starting one short of the factor so that the first
    // edge is a firing one.
    let skipped = m
        .components
        .iter()
        .find(|c| c.name.starts_with("$every"))
        .expect("the sub-sampled partition counts");
    assert_eq!(skipped.start, Some(Expr::Number(1.0)));

    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    for operator in [
        "superSample(e, 2)",
        "shiftSample(e, 1, 2)",
        "backSample(e, 1, 2)",
    ] {
        assert!(
            err(&format!(
                "{event} Clock bad = {operator}; Real z; Real out; \
                 equation der(x) = 1; z = previous(z) + interval(bad); \
                 out = hold(z); end M;"
            ))
            .contains("an event clock has no answer"),
            "{operator}"
        );
    }
    // The start interval is what `interval` answers before there is a
    // tick to measure back to, so it has to be known before the run.
    assert!(err(
        "model M Real x(start = 0, fixed = true); Real q; Clock e = Clock(x > 0.5, q); \
         Real g; Real out; equation der(x) = 1; q = time; g = interval(e); \
         out = hold(g); end M;"
    )
    .contains("start interval of an event clock"));
    // A machine on an event clock can count its ticks but cannot turn
    // them into seconds.
    assert!(err(
        "model M block Step Real n(start = 0); equation n = previous(n) + 1; end Step; \
         Real x(start = 0, fixed = true); Clock c = Clock(x > 0.5, 0.25); \
         Step a; Step b; Real lamp; Real out; \
         equation der(x) = 1; initialState(a); transition(a, b, a.n >= 1); \
         lamp = timeInState(); out = hold(lamp); end M;"
    )
    .contains("`ticksInState` is what it can answer"));
    // What an event clock waits for happens in continuous time. A clock
    // waiting on a value that only its own ticks change would be
    // waiting on itself.
    assert!(err(
        "model M Clock c = Clock(0.1); Real u; Real s; Clock e = Clock(s > 0.5, 0.25); \
         Real n; Real out; equation u = time; s = sample(u, c); \
         n = previous(n) + interval(e); out = hold(n) + hold(s); end M;"
    )
    .contains("`hold(s)` is how a clocked value is read"));
}

#[test]
fn a_solver_clock_turns_a_derivative_into_a_step() {
    // A `der` on a clock that says how to step it becomes assignments
    // like everything else on the tick: the step first, then the slopes
    // where the step has left the state, for the tick after.
    let m = parse_model(
        "model M Clock c = Clock(Clock(0.1), \"ExplicitEuler\"); \
         Real u; Real x(start = 1); Real out; \
         equation u = sample(0, c); der(x) = -x + u; out = hold(x); end M;",
    )
    .unwrap();
    assert!(!format!("{:?}", m.equations).contains("der"));
    let x = m.components.iter().find(|c| c.name == "x").unwrap();
    assert_eq!(x.variability, oxidelica_parser::Variability::Discrete);
    assert_eq!(x.start, Some(Expr::Number(1.0)));
    // One slope for the one stage, and four for the four-stage method.
    let stages = |source: &str| {
        parse_model(source)
            .unwrap()
            .components
            .iter()
            .filter(|c| c.name.starts_with("$slope"))
            .count()
    };
    let of = |method: &str| {
        format!(
            "model M Clock c = Clock(Clock(0.1), \"{method}\"); \
             Real u; Real x(start = 1); Real out; \
             equation u = sample(0, c); der(x) = -x + u; out = hold(x); end M;"
        )
    };
    assert_eq!(stages(&of("ExplicitEuler")), 1);
    assert_eq!(stages(&of("ExplicitMidPoint2")), 2);
    assert_eq!(stages(&of("ExplicitRungeKutta4")), 4);

    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // An implicit method would make every tick an equation to solve.
    for method in ["ImplicitEuler", "ImplicitTrapezoid"] {
        assert!(
            err(&of(method)).contains("rather than a value to work out"),
            "{method}"
        );
    }
    assert!(err(&of("External")).contains("nothing to leave it to"));
    assert!(err(&of("Bogus")).contains("not a solver method the specification names"));
    // A method with more than one stage guesses where the state will be
    // partway through the step, which needs a step known in advance.
    let event = "model M Real p(start = 0, fixed = true); \
         Clock c = Clock(Clock(p > 0.5, 0.2), \"{}\"); \
         Real u; Real x(start = 1); Real out; \
         equation der(p) = 1; u = sample(0, c); der(x) = -x + u; \
         out = hold(x); end M;";
    assert!(err(&event.replace("{}", "ExplicitRungeKutta4"))
        .contains("does not know how long its next step is"));
    // The one-stage method needs no such guess, so it works there.
    assert!(parse_model(&event.replace("{}", "ExplicitEuler")).is_ok());
}

#[test]
fn clock_error_paths() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // A clock with no interval anyone can work out.
    assert!(err("model M Clock c; Real y; equation y = 1; end M;")
        .contains("interval the compiler can see"));
    assert!(
        err("model M Clock c = Clock(-1); Real y; equation y = 1; end M;")
            .contains("must be positive")
    );
    // `previous` with nothing to say which clock it is on.
    assert!(err("model M Clock c = Clock(0.1); Real y; \
         equation y = previous(y) + 1; end M;")
    .contains("which clock"));
    // A clock spreads to whatever reads it, so `y = s + 1` is on
    // the clock too rather than a mistake.
    let m = parse_model(
        "model M Clock c = Clock(0.1); Real u; Real s; Real y; \
         equation u = time; s = sample(u, c); y = s + 1; end M;",
    )
    .unwrap();
    assert_eq!(m.when_clauses[0].branches[0].actions.len(), 2);
    // What cannot be on a clock, though, must ask for the held
    // value: a derivative is continuous by its nature.
    assert!(err(
        "model M Clock c = Clock(0.1); Real u; Real s; Real x(start = 0); \
         equation u = time; s = sample(u, c); der(x) = -s + max(1, 2); end M;"
    )
    .contains("only read it through `hold(s)`"));
    assert!(err(
        "model M Clock c = Clock(0.1); Real u; Real s; Real x(start = 0); \
         equation u = time; s = sample(u, c); der(x) = s; end M;"
    )
    .contains("only read it through `hold(s)`"));
    // Equations on one clock that need each other in a circle.
    assert!(err("model M Clock c = Clock(0.1); Real u; Real a; Real b; \
         equation u = time; a = sample(u, c) + b; b = a + 1; end M;")
    .contains("in a circle"));

    // A value belongs to one clock. Reaching across two without saying
    // how they meet is not an equation this language has a meaning for.
    assert!(err(
        "model M Clock base = Clock(0.1); Clock slow = subSample(base, 2); \
         Real u; Real v; Real out; equation u = previous(u) + interval(base); \
         v = u + interval(slow); out = hold(v); end M;"
    )
    .contains("two clocks at once"));
    // Two partitions that need each other within one tick leave no
    // order to compute them in.
    assert!(err(
        "model M Clock base = Clock(0.1); Clock slow = subSample(base, 2); \
         Real u; Real v; Real out; \
         equation u = superSample(v, 2) + interval(base); \
         v = subSample(u, 2) + interval(slow); out = hold(u) + hold(v); end M;"
    )
    .contains("leaves no order"));
    // A clock cannot start before the run does.
    assert!(err(
        "model M Clock base = Clock(0.1); Clock early = backSample(base, 1, 2); \
         Real u; Real out; equation u = previous(u) + interval(early); \
         out = hold(u); end M;"
    )
    .contains("before the start of the run"));
    // The factors are counted, so they are whole numbers, and small
    // enough that the exact arithmetic can hold what they multiply to.
    assert!(err(
        "model M Clock base = Clock(0.1); Clock odd = subSample(base, 2.5); \
         Real u; Real out; equation u = previous(u) + interval(odd); \
         out = hold(u); end M;"
    )
    .contains("whole number between 0 and"));
    assert!(err("model M Clock base = Clock(0.1); \
         Clock huge = superSample(superSample(superSample(base, 999999), 999999), 999999); \
         Clock worse = superSample(superSample(huge, 999999), 999999); \
         Real u; Real out; equation u = previous(u) + interval(worse); \
         out = hold(u); end M;")
    .contains("too large to keep exactly"));
    // `Clock(counter, resolution)` is counted the same way.
    assert!(
        err("model M Clock c = Clock(1, 0); Real y; equation y = 1; end M;")
            .contains("whole number between 1 and")
    );
    // A factor left for the compiler with nothing to work it out from.
    assert!(err(
        "model M Clock base = Clock(0.1); Real u; Real v; Real out; \
         equation u = previous(u) + interval(base); v = subSample(u); \
         out = hold(v); end M;"
    )
    .contains("nothing says which clock it is on"));
}

#[test]
fn a_clock_or_a_factor_left_unsaid_is_worked_out_from_the_equation() {
    // An equation is on one clock, so where it names a clock that knows
    // its rate beside one that does not, the second takes the first.
    // Both of these say the same thing as `Clock(1, 5)` written out and
    // `subSample(a, 2)` written out - which the run below confirms by
    // arriving at the same number.
    let inferred = |slow: &str, sampled: &str| {
        format!(
            "model M Clock fast = Clock(1, 10); Clock slow = {slow}; \
             Real a; Real b; Real out; \
             equation a = previous(a) + interval(fast); \
             b = {sampled} + interval(slow); out = hold(b); end M;"
        )
    };
    for (slow, sampled) in [
        ("Clock(1, 5)", "subSample(a, 2)"),
        ("Clock()", "subSample(a, 2)"),
        ("Clock(0, 5)", "subSample(a, 2)"),
        ("Clock(1, 5)", "subSample(a)"),
        ("Clock(1, 5)", "subSample(a, 0)"),
    ] {
        let m = parse_model(&inferred(slow, sampled)).unwrap();
        let mut ticks = ticks_of(&m);
        ticks.sort_by(|a, b| a.partial_cmp(b).expect("no clock ticks on a NaN"));
        assert_eq!(ticks, vec![(0.0, 0.1), (0.0, 0.2)], "{slow} / {sampled}");
    }
    // The same the other way about: a `superSample` with no factor
    // takes it from the faster clock the equation also names.
    let m = parse_model(
        "model M Clock slow = Clock(1, 5); Clock fast = Clock(1, 10); \
         Real a; Real b; Real out; \
         equation a = previous(a) + interval(slow); \
         b = superSample(a) + interval(fast); out = hold(b); end M;",
    )
    .unwrap();
    let mut ticks = ticks_of(&m);
    ticks.sort_by(|a, b| a.partial_cmp(b).expect("no clock ticks on a NaN"));
    assert_eq!(ticks, vec![(0.0, 0.1), (0.0, 0.2)]);

    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // Nothing to work it out from is refused rather than guessed. It
    // has to be: an unsettled clock would have nothing lifted onto it,
    // and the equations meant to tick would quietly stay continuous.
    for empty in ["Clock()", "Clock(0, 5)"] {
        assert!(
            err(&format!(
                "model M Clock c = {empty}; Real u; Real s; Real out; \
                 equation u = time; s = sample(u, c); out = hold(s); end M;"
            ))
            .contains("nothing in this model says how often `c` ticks"),
            "{empty}"
        );
    }
    // A rate that no whole factor reaches is refused with both rates
    // named, since the model has to be told which of the two is wrong.
    // 0.25 is not a whole number of 0.1s. The message names the factor
    // it would have taken, which is what says which of the two rates is
    // the mistaken one.
    assert!(err(&inferred("Clock(1, 4)", "subSample(a)"))
        .contains("sampling every 0.1 to tick every 0.25 would take a factor of 2.5"));
    // `Clock(0, 5)` leaves the numerator to the compiler and keeps the
    // denominator, so what turns up has to be a whole number of fifths.
    assert!(err(
        "model M Clock fast = Clock(1, 8); Clock slow = Clock(0, 5); \
         Real a; Real b; Real out; equation a = previous(a) + interval(fast); \
         b = subSample(a, 2) + interval(slow); out = hold(b); end M;"
    )
    .contains("counted in parts of one over 5"));
    // And an event clock gives a factor nothing to count.
    assert!(err(
        "model M Real p(start = 0, fixed = true); Clock e = Clock(p > 0.5, 0.2); \
         Clock other = Clock(0.1); Real a; Real b; Real out; \
         equation der(p) = 1; a = previous(a) + interval(e); \
         b = subSample(a) + interval(other); out = hold(b); end M;"
    )
    .contains("gives it nothing to count"));
}

/// `terminate` may be given a message built rather than written out.
#[test]
fn terminate_takes_the_message_it_is_given() {
    let m = parse_model(
        "model M parameter String why = \"done\"; Real x; \
         equation x = time; \
         when x > 0.5 then terminate(\"stopped: \" + why); end when; \
         annotation(experiment(StopTime = 1, Interval = 0.1)); end M;",
    )
    .expect("a built message");
    let said = format!("{:?}", m.when_clauses[0].branches[0].actions);
    assert!(said.contains("stopped: "), "{said}");

    // A number is not a message, and it says so.
    let error = parse_model(
        "model M Real x; equation x = 1; when x > 1 then terminate(42); end when; end M;",
    )
    .expect_err("not a message")
    .message;
    assert!(error.contains("string message"), "{error}");
}

/// A choice between assignments at an event.
#[test]
fn an_if_may_stand_inside_a_when() {
    // What a variable is given depends on the condition, so it gets
    // one assignment whose value is the choice.
    let m = parse_model(
        "model M discrete Real low; discrete Real high; Real y; \
         equation y = time; \
         when time > 0.5 then \
           if y > 1 then low = 1; high = 2; else low = 3; high = 4; end if; \
         end when; \
         annotation(experiment(StopTime = 1, Interval = 0.1)); end M;",
    )
    .expect("a choice at an event");
    let actions = &m.when_clauses[0].branches[0].actions;
    assert_eq!(actions.len(), 2);
    let written = format!("{actions:?}");
    assert!(written.contains("If(Rel(Gt"), "{written}");
    assert!(written.contains("Number(3.0)"), "{written}");

    // A branch that says nothing about a variable leaves it what it
    // had, which is what `pre` of it is.
    let m = parse_model(
        "model M discrete Real kept; Real y; \
         equation y = time; \
         when time > 0.5 then if y > 1 then kept = 1; end if; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.1)); end M;",
    )
    .expect("a branch that says nothing");
    let written = format!("{:?}", m.when_clauses[0].branches[0].actions);
    assert!(written.contains("Call(\"pre\""), "{written}");

    // What such a choice may hold is values for variables.
    let error = parse_model(
        "connector Pin Real v; end Pin; \
         model M Pin a; Pin b; Real y; equation y = time; \
         when time > 0.5 then if y > 1 then connect(a, b); end if; end when; end M;",
    )
    .expect_err("no connections at an event")
    .message;
    assert!(error.contains("gives values to variables"), "{error}");
}

/// A clock drawn from one block to another by a connection.
#[test]
fn a_clock_may_arrive_through_a_connection() {
    // `connect(src.y, use.clock)` between two clock connectors is an
    // equation between two clocks, and it comes out with whichever
    // name sorts first on the left. The one being said is the one
    // nothing has said yet.
    let m = parse_model(
        "connector ClockOutput = output Clock; connector ClockInput = input Clock; \
         block Source ClockOutput y; equation y = Clock(0.1); end Source; \
         block Use ClockInput clock; Real u; Real y; \
         equation u = time; y = sample(u, clock); end Use; \
         model M Source src; Use use; Real out; \
         equation connect(src.y, use.clock); out = hold(use.y); \
         annotation(experiment(StopTime = 1, Interval = 0.1)); end M;",
    )
    .expect("a clock through a connection");
    // Neither clock is a variable of the run, and what runs on it is
    // one partition firing every tenth of a second.
    assert!(!m.components.iter().any(|c| c.name.ends_with(".clock")));
    let ticks = format!("{:?}", m.when_clauses);
    assert!(ticks.contains("0.1"), "{ticks}");
}

#[test]
fn a_generator_draws_at_an_event_and_carries_its_state() {
    // The whole of what a noise block does: a state held across
    // events, one draw per sample, and the state the draw moved to
    // landing on the names the state is held in.
    let m = parse_model(&format!(
        "{GENERATOR} model M Integer state[2](start = {{7, 3}}, each fixed = true); \
         discrete Real r(start = 0, fixed = true); \
         equation when sample(0, 1) then (r, state) = Gen.random(pre(state)); end when; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    ))
    .unwrap();
    let actions = &m.when_clauses[0].branches[0].actions;
    // Three names assigned: the number drawn and the two halves of the
    // state, each taking its own place of what the body answers with.
    assert_eq!(actions.len(), 3);
    let said = format!("{actions:?}");
    assert!(
        said.contains("state[1]") && said.contains("state[2]"),
        "{said}"
    );
    assert!(said.contains("ModelicaRandom_xorshift64star"), "{said}");
}

/// A class settling its own input in a `when` settles nothing: asking
/// to be given a value is what declaring an input means.
#[test]
fn a_when_inside_the_class_does_not_settle_its_own_input() {
    let refusal = parse_model(
        "model Held input Real u; discrete Real held; \
         equation when time > 1 then held = u; end when; end Held; \
         model M Held h; Real z; equation z = h.held; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(refusal.contains("h.u"), "{refusal}");

    // The same `when` written by the class holding it does settle it.
    parse_model(
        "model Held input Real u; discrete Real held; \
         equation when time > 1 then held = u; end when; end Held; \
         model M Held h; Real z; \
         equation when time > 0.5 then h.u = time; end when; z = h.held; end M;",
    )
    .unwrap();
}

#[test]
fn a_when_on_a_clock_is_a_partition_written_out_by_hand() {
    let m = parse_model(
        "model M Clock c = Clock(0.1); Real u; Real s; Real n(start = 0); Real out; \
         equation u = time; \
         when Clock() then s = sample(u, c); n = previous(n) + s; end when; \
         out = hold(n); end M;",
    )
    .unwrap();

    // The clause named no rate of its own, so the clock is the one
    // `sample` brought in, and both of its equations landed there.
    assert_eq!(m.when_clauses.len(), 1);
    let branch = &m.when_clauses[0].branches[0];
    assert_eq!(
        format!("{:?}", branch.condition),
        "Call(\"sample\", [Number(0.0), Number(0.1)])"
    );
    let targets: Vec<&str> = branch
        .actions
        .iter()
        .filter_map(|action| match action {
            oxidelica_parser::WhenAction::Assign(target, _) => Some(target.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(targets, vec!["s", "n"]);
    // `previous` is the value from the tick before, which is `pre`
    // once the clause it sits in fires on the tick.
    let text = format!("{:?}", branch.actions);
    assert!(text.contains("Call(\"pre\", [Ref(\"n\")])"), "{text}");
    assert!(!text.contains("previous"), "{text}");
}

#[test]
fn a_when_may_give_a_whole_array_at_once() {
    // The clocked samplers pass a bus through with one equation
    // between two vectors, and an event assigns one variable - so the
    // assignment is taken apart the way an equation between arrays is.
    let m = parse_model(
        "block Pass parameter Integer n = 1; Boolean u[n]; Boolean y[n]; Clock clock; \
         equation when clock then y = u; end when; end Pass; \
         model M Clock c = Clock(0.1); Pass p(n = 2); Boolean src[2]; \
         equation src[1] = time > 0.05; src[2] = time > 0.15; \
         p.clock = c; p.u[1] = src[1]; p.u[2] = src[2]; end M;",
    )
    .unwrap();

    let targets: Vec<String> = m
        .when_clauses
        .iter()
        .flat_map(|clause| &clause.branches)
        .flat_map(|branch| &branch.actions)
        .filter_map(|action| match action {
            oxidelica_parser::WhenAction::Assign(target, value) => {
                Some(format!("{target} = {value:?}"))
            }
            _ => None,
        })
        .filter(|line| line.starts_with("p.y"))
        .collect();
    // One assignment per element, each reading the element beside it.
    assert_eq!(
        targets,
        vec![
            "p.y[1] = Ref(\"p.u[1]\")".to_string(),
            "p.y[2] = Ref(\"p.u[2]\")".to_string(),
        ]
    );
}

#[test]
fn a_sample_may_leave_its_clock_to_inference() {
    // 16.3's `sample(u)` reads a continuous signal at the tick of
    // whatever clock the equation lands on, and it is what every
    // sampler block of the standard library writes. It is the same
    // word as the event operator `sample(start, interval)`, which
    // rises at every tick and is Boolean - so the one-argument form
    // has to answer with what it read instead.
    // Written the way the library builds a sampler: the block samples
    // with the clock left open, and the model wires a clock to the
    // block beside it - which is where the clock comes from.
    let m = parse_model(
        "block Sampler Real u; Real y; equation y = sample(u); end Sampler; \
         block Assign Clock clock; Real u; Real y; \
           equation when clock then y = u; end when; end Assign; \
         model M Clock c = Clock(0.1); Sampler s; Assign a; Real src; Real out; \
         equation src = time; s.u = src; a.clock = c; a.u = s.y; \
         out = hold(a.y); end M;",
    )
    .unwrap();

    // The sample is gone by the time the model is flat: reading at a
    // tick is reading, and nothing named `sample` is left anywhere.
    let text = format!("{:?}{:?}", m.equations, m.when_clauses);
    assert!(!text.contains("Call(\"sample\", [Ref"), "{text}");
    // What the sampler gives is what it read, and it is a Real: the
    // type layer would have refused a Boolean against `s.y`.
    let y = m.components.iter().find(|c| c.name == "s.y").unwrap();
    assert_eq!(y.type_name, "Real");
    let read = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"s.y\")")
        .unwrap();
    assert_eq!(format!("{:?}", read.rhs), "Ref(\"s.u\")");
}
