//! Where a run begins: start values, what is fixed, and what the initial system decides.

use super::shared::*;
use oxidelica_parser::parse_model;
use oxidelica_sim::compile;

#[test]
fn parameter_uses_start_as_fallback_value() {
    let model =
        parse_model("model M parameter Real p(start = 3); Real x; equation x = p; end M;").unwrap();
    let compiled = compile(&model).unwrap();
    assert_eq!(compiled.parameters, vec![("p".to_string(), 3.0)]);
}

#[test]
fn mirrored_equation_forms_and_default_start() {
    // expr = der(v), expr = v, a state without start (zero-initialized),
    // and subtraction.
    let result = run("model M Real x; Real y; equation \
         -x - 1 = der(x); 2 + time = y; \
         annotation(experiment(StopTime=1.0, Interval=0.001, Tolerance=1e-12)); end M;");
    let first = &result.rows[0];
    assert_eq!(first[1], 0.0); // x(0) = 0 by default
    assert!((first[2] - 2.0).abs() < 1e-12); // y(0) = 2
                                             // der(x) = -x - 1, x(0)=0 -> x(t) = e^{-t} - 1
    let last = result.rows.last().unwrap();
    assert!(
        (last[1] - ((-1.0f64).exp() - 1.0)).abs() < 1e-9,
        "x(1)={}",
        last[1]
    );
}

#[test]
fn a_plain_start_is_a_guess_but_fixed_is_an_initial_condition() {
    // q is demoted by index reduction and solved from q = time^2,
    // so a plain start is only a Newton guess: q(0) = 0 wins.
    let result = run(
        "model D Real z; Real q(start = 1); equation der(q) = z; q = time ^ 2; \
         annotation(experiment(StopTime=1.0, Interval=0.01)); end D;",
    );
    let q_idx = result.columns.iter().position(|c| c == "q").unwrap();
    assert!(
        result.rows[0][q_idx].abs() < 1e-9,
        "q(0) = {}",
        result.rows[0][q_idx]
    );
    // Declaring it fixed turns the contradiction into an error.
    let model = parse_model(
        "model D Real z; Real q(start = 1, fixed = true); \
         equation der(q) = z; q = time ^ 2; \
         annotation(experiment(StopTime=1.0, Interval=0.01)); end D;",
    )
    .unwrap();
    let error = compile(&model).unwrap_err();
    assert!(error.0.contains("is fixed at"), "{}", error.0);
}

#[test]
fn the_initialisation_problem_is_solved_or_named() {
    // An initial equation that pins nothing.
    assert!(refused(
        "model M Real x(start = 0); equation der(x) = 1; initial equation 0 = 0; end M;"
    )
    .contains("initialization problem is singular"));
    // One that cannot be solved from where it starts.
    let model = parse_model(
        "model M Real x(start = 1); equation der(x) = 1; initial equation x * x = -1; end M;",
    )
    .unwrap();
    let outcome = compile(&model);
    assert!(outcome.is_err(), "an impossible start should be refused");
}

#[test]
fn initialization_mixes_fixed_starts_with_initial_equations() {
    // One state is pinned by `fixed = true`, the other follows from
    // an initial equation that ties the two together.
    let result = run(
        "model M Real a(start = 2, fixed = true); Real b(start = 0); \
         equation der(a) = -a; der(b) = a - b; \
         initial equation b = 3 * a; \
         annotation(experiment(StopTime = 0.1, Interval = 0.05)); end M;",
    );
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let first = &result.rows[0];
    assert!((first[index("a")] - 2.0).abs() < 1e-12);
    assert!((first[index("b")] - 6.0).abs() < 1e-9);
}

#[test]
fn initialization_reports_what_it_cannot_solve() {
    let error = |source: &str| {
        let model = parse_model(source).unwrap();
        compile(&model).unwrap_err().to_string()
    };
    // Two initial equations for one free state.
    assert!(error(
        "model M Real a(start = 1); equation der(a) = -a; \
         initial equation a = 1; der(a) = 0; end M;"
    )
    .contains("not square"));
    // An initial equation that says nothing about the state.
    assert!(error(
        "model M Real a(start = 1); Real b; equation der(a) = -a; b = 2 * a; \
         initial equation b = 2 * a; end M;"
    )
    .contains("singular"));
    // `der` of something that is not a state.
    assert!(error(
        "model M Real a(start = 1); Real b; equation der(a) = -a; b = a; \
         initial equation der(b) = 0; end M;"
    )
    .contains("is not a state"));
}

#[test]
fn a_start_written_through_a_type_alias_is_kept() {
    // `Units.AngularVelocity w(start = w0)` parses its parenthesis
    // as modifiers, not attributes; the initial condition used to
    // vanish without a sound. Found because the damper test above
    // started from rest instead of 5 rad/s.
    let result = run(
        "package Units type Speed = Real(unit = \"m/s\"); end Units; \
         model M parameter Real w0 = 5; parameter Real tau(unit = \"s\") = 1; \
         Units.Speed w(start = w0); \
         equation der(w) = -w / tau; \
         annotation(experiment(StopTime = 1.0, Interval = 0.1)); end M;",
    );
    assert!(
        (result.rows[0][1] - 5.0).abs() < 1e-12,
        "{}",
        result.rows[0][1]
    );
    // And the declaration's own start wins over an alias default.
    let overridden = run("package Units type Speed = Real(start = 7); end Units; \
         model M Units.Speed w(start = 5); equation der(w) = -w; \
         annotation(experiment(StopTime = 1.0, Interval = 0.1)); end M;");
    assert!((overridden.rows[0][1] - 5.0).abs() < 1e-12);
}

#[test]
fn a_carried_profile_starts_from_the_one_it_was_given() {
    // A step sitting in the middle of the pipe at t = 0: the far half
    // holds one, the near half zero. Carried forward at unit speed the
    // far end reads one until the step reaches it, half a second in.
    let result = run(
        "model P Real x(start = 0, fixed = true); Real out0; Real out1; \
         equation der(x) = 1; \
         (out0, out1) = spatialDistribution(0, 0, x, true, \
           {0.0, 0.5, 0.5, 1.0}, {0.0, 0.0, 1.0, 1.0}); \
         annotation(experiment(StopTime = 1, Interval = 0.1)); end P;",
    );
    let out1 = result.columns.iter().position(|c| c == "out1").unwrap();
    let seen: String = result
        .rows
        .iter()
        .map(|row| if row[out1] > 0.5 { '1' } else { '0' })
        .collect();
    assert_eq!(seen, "11111100000", "the step arrived at the wrong time");
}

/// An `initial algorithm` settles where the run starts, and a check
/// written inside an `if` branch fires only while that branch holds.
#[test]
fn an_initial_algorithm_and_a_guarded_check_run() {
    // `x` starts where the initial algorithm put it, gain * 2 = 6, and
    // rises at 1 a second: x(2) = 8.
    let result = run("model M parameter Real gain = 3; Real x; \
         initial algorithm x := gain * 2; \
         equation der(x) = 1; \
         annotation(experiment(StopTime = 2, Interval = 0.1, Tolerance = 1e-10)); end M;");
    let last = result.rows.last().unwrap();
    assert!((last[0] - 2.0).abs() < 1e-12);
    assert!((last[1] - 8.0).abs() < 1e-9, "x(2) = {}", last[1]);

    // The check of the second branch would fail at every moment the
    // first branch holds; guarded by the denial of the first
    // condition, it says nothing there and the run gets through.
    let result = run("model M Real y; equation \
         if time > 1 then assert(time > 0.5, \"late\"); y = 1; \
         else assert(time < 1.5, \"early\"); y = 2; end if; \
         annotation(experiment(StopTime = 2, Interval = 0.1, Tolerance = 1e-10)); end M;");
    assert!((result.rows.last().unwrap()[0] - 2.0).abs() < 1e-12);
}

/// `der(x)` written in an initial equation is the right-hand side the
/// model gives that state, wherever in the expression it stands.
#[test]
fn an_initial_equation_may_hold_a_derivative_anywhere_in_it() {
    // The condition is written the long way round on purpose: the
    // derivative has to be found under a call, a relation, a negation,
    // an `and`, an `or` and a choice, and it means the same in each.
    let result = run("model M Real x; Real y; \
         equation der(x) = 3 - x; y = x; \
         initial equation \
         0 = if abs(der(x)) > 100 or not (der(x) < 0) and true then der(x) else -der(x); \
         annotation(experiment(StopTime = 1, Interval = 0.5, Tolerance = 1e-10)); end M;");
    // A start where nothing moves: der(x) = 0 puts x at 3.
    let first = result.rows.first().unwrap();
    assert!((first[1] - 3.0).abs() < 1e-9, "x(0) = {}", first[1]);
    let last = result.rows.last().unwrap();
    assert!((last[1] - 3.0).abs() < 1e-9, "x(1) = {}", last[1]);
}

/// A parameter written `fixed = false` takes its value from the
/// initialisation rather than from its declaration.
#[test]
fn a_parameter_fixed_false_is_settled_by_an_initial_equation() {
    // The standard library catches the moment a simulation starts this
    // way - `parameter SI.Time t0(fixed=false)` with `t0 = time` among
    // the initial equations - and then reads it for the rest of the
    // run. Here the start is at zero, so `der(x)` is one throughout
    // and `x` reaches one.
    let result = run(
        "model M parameter Real t0(fixed = false); Real x(start = 0, fixed = true); \
         initial equation t0 = time; \
         equation der(x) = t0 + 1; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;",
    );
    let last = result.rows.last().expect("a final row");
    let at = |name: &str| {
        result
            .columns
            .iter()
            .position(|column| column == name)
            .map(|index| last[index])
    };
    assert!((at("x").expect("x") - 1.0).abs() < 1e-6, "{:?}", at("x"));

    // The equation that settled it is not counted twice: it did its
    // work among the parameters, and asking the initialisation for it
    // again would leave one equation more than there are unknowns.
    let started = run(
        "model M parameter Real t0(fixed = false); Real x(start = 3); \
         initial equation t0 = 2; x = t0; \
         equation der(x) = 0; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;",
    );
    let final_row = started.rows.last().expect("a final row");
    let index = started
        .columns
        .iter()
        .position(|column| column == "x")
        .expect("x");
    assert!((final_row[index] - 2.0).abs() < 1e-6, "{:?}", final_row);
}

/// What `fixed = false` on a parameter does and does not change.
#[test]
fn a_parameter_the_initialization_settles_is_read_the_way_the_language_says() {
    let final_value = |source: &str, wanted: &str| {
        let result = run(source);
        let last = result.rows.last().expect("a final row").clone();
        let at = result
            .columns
            .iter()
            .position(|column| column == wanted)
            .unwrap_or_else(|| panic!("{wanted} in {:?}", result.columns));
        last[at]
    };

    // A declaration that wrote a value keeps it: the language asks for
    // a warning where both are given, not for the value to be thrown
    // away. Here `p` is 3, `x` starts there and climbs by one.
    let both = final_value(
        "model M parameter Real p(fixed = false) = 3; Real x(start = 0, fixed = false); \
         initial equation x = p; equation der(x) = 1; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;",
        "x",
    );
    assert!((both - 4.0).abs() < 1e-6, "{both}");

    // Which equation settles the parameter is not the first one that
    // mentions it: `x = p` is about the state and cannot settle
    // anything, so `p = 7` is taken whichever order they stand in.
    for order in ["x = p; p = 7;", "p = 7; x = p;"] {
        let settled = final_value(
            &format!(
                "model M parameter Real p(fixed = false); Real x(start = 0, fixed = false); \
                 initial equation {order} equation der(x) = 1; \
                 annotation(experiment(StopTime = 1, Interval = 0.5)); end M;"
            ),
            "x",
        );
        assert!((settled - 8.0).abs() < 1e-6, "{order}: {settled}");
    }

    // A constant is settled by its declaration whatever it says about
    // `fixed`, since the language does not let one be an unknown.
    let held = final_value(
        "model M constant Real k(fixed = false) = 5; Real x(start = 0, fixed = true); \
         equation der(x) = k; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;",
        "x",
    );
    assert!((held - 5.0).abs() < 1e-6, "{held}");
}

/// The same for a record handed on whole, and for a flag.
///
/// A body that sets a record in one branch and hands it on in both
/// needs the record itself to have a start, which is its fields'
/// starts gathered up. A `Boolean` starts at false rather than at
/// zero, and a field declared through a chain of unit aliases -
/// `SI.Temperature` is a `Real` by way of two other names - has to be
/// followed to the end to find that out.
#[test]
fn an_unset_record_starts_as_its_fields_do() {
    let result = run("package Top \
           type Temperature = Real; \
           record State Temperature warm; Boolean lit; end State; \
           function pick input Real u; output Real y; \
           protected State s; \
           algorithm \
             if u > 2 then s.warm := u; s.lit := true; end if; \
             y := s.warm + (if s.lit then 10 else 0); \
           end pick; \
           model M \
             Real cool = pick(time + 1); \
             annotation(experiment(StopTime=0.1)); \
           end M; \
         end Top;");
    let last = result.rows.last().expect("a final row");
    let cool = last[1];
    // Neither field was set: the number starts at zero and the flag at
    // false, so nothing is added for either.
    assert!(
        cool.abs() < 1e-9,
        "an unset record should start as its fields do: {cool}"
    );
}

/// A record set whole in one branch starts as its fields do.
///
/// The earlier test leaves the fields alone one by one; the steam
/// tables assign the record itself - `f := Basic.f3(bpro.d, bpro.T)`
/// inside the region 3 test - and read it back whichever way the test
/// went. The merge is then asked about the record's own name, and a
/// record has no start of its own to give: it starts as its fields
/// do, gathered in the order the record declares them. Without that
/// the whole body is turned down before anything else can be looked
/// at, and the Fluid fitting example that reaches the boiling curve
/// stops at that refusal rather than at whatever is really wrong
/// further in.
#[test]
fn a_record_assigned_whole_in_one_branch_starts_as_its_fields_do() {
    let source = "package Top \
           record Derivs Real d; Real t; end Derivs; \
           function f3 input Real u; output Derivs g; \
             algorithm g.d := u; g.t := 2*u; end f3; \
           function region input Real u; output Real y; \
           protected Derivs f; \
           algorithm \
             if u > 2 then f := f3(u); end if; \
             y := f.d + f.t; \
           end region; \
           model M \
             Real dry = region(time); \
             annotation(experiment(StopTime=0.1)); \
           end M; \
         end Top;";
    let said = match oxidelica_parser::parse_model(source) {
        Ok(_) => String::new(),
        Err(error) => error.message,
    };
    assert!(
        !said.contains("assigned in one branch only"),
        "the record should start at its fields rather than turn the body down: {said}"
    );
}

/// A flag left unset beside a run of numbers still starts at false.
///
/// Giving unset locals a start is held back where a branch writes an
/// array, because that lets a whole body be folded into the caller and
/// a quaternion's four elements of nested conditions cost the library
/// minutes. A flag is not that: it decides which branch runs rather
/// than being folded into arithmetic, so it can never grow the value.
/// The steam tables put one right beside an array - `hsubcrit` is set
/// once and read in both halves of the region test - and holding it
/// back kept thirty-two models from flattening at all.
#[test]
fn a_flag_beside_an_array_still_starts_at_false() {
    let result = run("package Top \
           function region input Real p; output Real r; \
           protected Boolean sub; Real[3] v; \
           algorithm \
             v := {0, 0, 0}; \
             if p > 1 then v := {p, 1, 2}; sub := true; end if; \
             r := v[1] + (if sub then 10 else 0); \
           end region; \
           model M \
             Real y = region(time); \
             annotation(experiment(StopTime=0.1)); \
           end M; \
         end Top;");
    let last = result.rows.last().expect("a final row");
    let y = last[1];
    // Time never passes 1, so the branch never runs: the array keeps
    // its zeros and the flag is false, adding nothing.
    assert!(y.abs() < 1e-9, "the unset flag should start at false: {y}");
}

/// An `initial algorithm` that assigns a discrete variable says where
/// that variable starts, not what the states must satisfy: a periodic
/// source works out which period it is in before the first event ever
/// fires, and would otherwise be counted as a condition on states it
/// says nothing about.
#[test]
fn an_initial_algorithm_starts_the_discrete_variables_it_assigns() {
    let result = run(
        "model M parameter Real period = 1; Real T_start; Integer count; \
         initial algorithm count := integer((time + 2.5) / period); \
         T_start := count * period; \
         equation when time >= (pre(count) + 1) * period - 2.5 then \
         count = pre(count) + 1; T_start = time; end when; \
         annotation(experiment(StopTime = 0.4, Interval = 0.1, Tolerance = 1e-10)); end M;",
    );
    let last = result.rows.last().unwrap();
    let at = |name: &str| result.columns.iter().position(|n| n == name).unwrap();
    assert!(
        (last[at("count")] - 2.0).abs() < 1e-9,
        "the count the initial algorithm worked out stands: {}",
        last[at("count")]
    );
    assert!(
        (last[at("T_start")] - 2.0).abs() < 1e-9,
        "and what was worked out from it: {}",
        last[at("T_start")]
    );

    // The same when the value is one only the run knows: a count taken
    // off a state is still about where the count starts, not about
    // where the state does, and the state keeps its declared start.
    let result = run("model M Real x(start = 3); Integer count; \
         initial equation count = integer(x); \
         equation der(x) = 1; \
         when time >= 0.5 then count = pre(count) + 1; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.1, Tolerance = 1e-10)); end M;");
    let last = result.rows.last().unwrap();
    let at = |name: &str| result.columns.iter().position(|n| n == name).unwrap();
    assert!(
        (last[at("x")] - 4.0).abs() < 1e-9,
        "the state started where it was declared to: {}",
        last[at("x")]
    );

    // An initial equation that is about the states after all is left
    // where it was, and still says where the run begins.
    let result = run("model M Real x; Integer count; \
         initial equation 2 * x = 6; \
         equation der(x) = 1; \
         when time >= 0.5 then count = pre(count) + 1; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.1, Tolerance = 1e-10)); end M;");
    let last = result.rows.last().unwrap();
    let at = |name: &str| result.columns.iter().position(|n| n == name).unwrap();
    assert!(
        (last[at("x")] - 4.0).abs() < 1e-9,
        "the initial equation settled the state: {}",
        last[at("x")]
    );
}

#[test]
fn an_initial_equation_about_a_boolean_says_where_it_starts() {
    // The state graph writes `active = true` for the step a graph
    // begins in and `pre(newActive) = pre(localActive)` for the memory
    // behind it. Neither is a condition on the states - a Boolean keeps
    // its value between events whatever assigns it - so counting them
    // among the initial equations left the initialisation with ten
    // equations for no states at all.
    let result = run("model M Boolean on; Boolean seen; Real x(start = 0); \
         initial equation on = true; pre(seen) = true; \
         equation der(x) = 1; \
         when x > 0.5 then on = false; end when; \
         when x > 0.8 then seen = pre(on); end when; \
         annotation(experiment(StopTime=1.0, Interval=0.1)); end M;");
    let last = result.rows.last().unwrap();
    let on = result.columns.iter().position(|n| n == "on").unwrap();
    assert_eq!(last[on], 0.0);
    let first = result.rows.first().unwrap();
    assert_eq!(first[on], 1.0, "the initial equation says it starts true");
}
