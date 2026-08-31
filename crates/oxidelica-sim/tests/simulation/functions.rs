//! Functions at run time: bodies walked rather than written out, and what they answer with.

use super::shared::*;
use oxidelica_parser::parse_model;
use oxidelica_sim::{compile, SimResult};

#[test]
fn evaluates_every_builtin_function() {
    let result = run("model F Real y; equation \
         y = sin(1) + cos(1) + tan(1) + asin(0.5) + acos(0.5) + atan(1) \
           + atan2(1, 2) + sinh(1) + cosh(1) + tanh(1) + exp(1) + log(2) \
           + log10(100) + sqrt(4) + abs(-3) + sign(-2) + min(1, 2) + max(1, 5) + 2 ^ 10; \
         annotation(experiment(StopTime=0.01, Interval=0.01)); end F;");
    let expected = 1f64.sin()
        + 1f64.cos()
        + 1f64.tan()
        + 0.5f64.asin()
        + 0.5f64.acos()
        + 1f64.atan()
        + 1f64.atan2(2.0)
        + 1f64.sinh()
        + 1f64.cosh()
        + 1f64.tanh()
        + 1f64.exp()
        + 2f64.ln()
        + 100f64.log10()
        + 2.0
        + 3.0
        + (-1.0)
        + 1.0
        + 5.0
        + 1024.0;
    assert!((result.rows[0][1] - expected).abs() < 1e-12);
}

#[test]
fn a_body_nothing_could_inline_is_walked_by_the_run() {
    // Two things inlining cannot do. A function that leads back to
    // itself has no bottom to unroll to where what decides the
    // recursion comes from the run - `fact(5)` written out would be
    // unrolled here, so the count is what the run holds. `5!` is the
    // plainest example there is, and it comes out at 120.
    let factorial = run("model M function fact input Real n; output Real y; \
         algorithm if n <= 1 then y := 1; else y := n * fact(n - 1); end if; end fact; \
         Real n; Real y; equation n = 5 * time; y = fact(n); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;");
    let index =
        |result: &SimResult, name: &str| result.columns.iter().position(|c| c == name).unwrap();
    assert_eq!(
        factorial.rows.last().unwrap()[index(&factorial, "y")],
        120.0
    );

    // And a loop whose trip count the model decides rather than the
    // compiler: `u` counts up with time, and the body counts with it.
    let counted = run("model M function count input Real x; output Real y; \
         algorithm y := 0; while y < x loop y := y + 1; end while; end count; \
         Real u; Real y; equation u = time * 3; y = count(u); \
         annotation(experiment(StopTime = 2, Interval = 1)); end M;");
    let y = index(&counted, "y");
    // At t = 0 nothing to count to, at 1 up to 3, at 2 up to 6.
    assert_eq!(counted.rows[0][y], 0.0);
    assert_eq!(counted.rows.last().unwrap()[y], 6.0);

    // Everything a walked body may hold, in one body: a loop whose
    // range the model decides, a `break` out of it, a set written out,
    // a check, an early `return`, and the call that made it walked at
    // all. Worked out by hand: 0, 31, 64, 100, 136, 172.
    let broad = run(
        "model M function walkme input Real n; output Real y; protected Real acc; \
         algorithm acc := 0; \
         for i in 1:n loop if i > 3 then break; end if; acc := acc + i; end for; \
         for j in {10, 20} loop acc := acc + j; end for; \
         assert(acc > 0, \"the sum must be positive\"); \
         if n <= 0 then y := 0; return; end if; \
         y := acc + walkme(n - 1); end walkme; \
         Real n; Real y; equation n = 5 * time; y = walkme(n); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert_eq!(broad.rows.last().unwrap()[index(&broad, "y")], 172.0);

    let runaway = |source: &str| {
        compile(&parse_model(source).unwrap())
            .expect("builds")
            .simulate()
            .expect_err("does not end")
            .to_string()
    };

    // A walk that will not end is stopped and told about, rather than
    // left to run the stack or the clock out.
    assert!(runaway(
        "model M function loops input Real a; output Real b; algorithm b := loops(a); end loops; \
         Real y; equation y = loops(1); \
         annotation(experiment(StopTime = 0, Interval = 1)); end M;"
    )
    .contains("called itself 64 deep"));
    // A range with a step, worked out where it stands: 1 + 3 + 5 twice
    // over, once for each round of the recursion.
    let stepped = run(
        "model M function f input Real a; output Real b; protected Real acc; \
         algorithm acc := 0; for i in 1:2:5 loop acc := acc + i; end for; b := acc; \
         if a > 0 then b := b + f(a - 1); end if; end f; \
         Real n; Real y; equation n = time; y = f(n); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert_eq!(stepped.rows.last().unwrap()[index(&stepped, "y")], 18.0);

    // What a `for` in a walked body cannot run over: a range the body
    // was meant to read off an array, and a single value.
    let walked = |body: &str| {
        runaway(&format!(
            "model M function f input Real a; output Real b; protected Real acc; \
             algorithm acc := 0; {body} \
             if a > 0 then b := b + f(a - 1); end if; end f; \
             Real n; Real y; equation n = time; y = f(n); \
             annotation(experiment(StopTime = 0, Interval = 1)); end M;"
        ))
    };
    assert!(walked("for i loop acc := acc + i; end for; b := acc;")
        .contains("a walked body holds no arrays"));
    assert!(
        walked("for i in acc loop acc := acc + i; end for; b := acc;")
            .contains("runs over a range or a set written out")
    );
    // And a call given more than the body takes.
    assert!(runaway(
        "model M function f input Real a; output Real b; \
         algorithm b := a; if a > 0 then b := f(a - 1); end if; end f; \
         Real y; equation y = f(1, 2); \
         annotation(experiment(StopTime = 0, Interval = 1)); end M;"
    )
    .contains("takes 1 argument(s), given 2"));

    // A `when` has no meaning inside a call: there is no event there,
    // and the walk says so where it meets one.
    assert!(runaway(
        "model M function w input Real a; output Real b; \
         algorithm when a > 0 then b := 1; end when; b := b + w(a - 1); end w; \
         Real y; equation y = w(1); \
         annotation(experiment(StopTime = 0, Interval = 1)); end M;"
    )
    .contains("no event inside a call"));
    assert!(runaway(
        "model M function away input Real a; output Real b; \
         algorithm b := 0; while b < a loop b := b - 1; end while; end away; \
         Real u; Real y; equation u = time + 1; y = away(u); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    )
    .contains("without its condition turning false"));
}

/// A call standing on its own inside a body the run walks: nothing
/// takes its outputs, so what it is there for is the checks it makes.
#[test]
fn a_call_on_its_own_is_walked_for_its_checks() {
    // `counted` cannot be unrolled - how many rounds it runs is the
    // model's to decide - so it stays a call and the run walks it. The
    // walk meets `guard(n);`, which nothing receives, and carries out
    // the check inside it.
    let source = "model M \
         function guard input Real u; output Real ok; \
         algorithm assert(u > -1, \"not too small\"); ok := u; end guard; \
         function counted input Real n; output Real y; \
         algorithm y := 0; guard(n); while y < n loop y := y + 1; end while; end counted; \
         Real y; equation y = counted(3 * time); \
         annotation(experiment(StopTime = 1, Interval = 0.5, Tolerance = 1e-10)); end M;";
    let result = run(source);
    let last = result.rows.last().unwrap();
    // At t = 1 the loop counts to 3.
    assert!((last[1] - 3.0).abs() < 1e-12, "y(1) = {}", last[1]);

    // The same check, failing: the walk stops the run and says what
    // the body said.
    let failing = source.replace("u > -1", "u < -1");
    let model = parse_model(&failing).unwrap();
    let error = compile(&model).unwrap().simulate().unwrap_err().to_string();
    assert!(error.contains("not too small"), "{error}");
}

#[test]
fn a_walked_body_carries_arrays() {
    // A length worked out by a `while` whose rounds the model decides,
    // over an array handed in. Nothing here can be unrolled while
    // flattening, so the run walks the body - and now it may be handed
    // an array and hold one while it goes. Three and four make five.
    let result = run(
        "model M function norm input Real v[:]; input Real a; output Real n; \
         protected Real acc; Integer k; \
         algorithm acc := 0; k := 1; \
         while k <= size(v, 1) and a > 0 loop acc := acc + v[k] * v[k]; k := k + 1; end while; \
         n := sqrt(acc); end norm; \
         Real y; equation y = norm({3 * time, 4 * time}, time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let column = result.columns.iter().position(|c| c == "y").unwrap();
    let y = result.rows.last().unwrap()[column];
    assert!((y - 5.0).abs() < 1e-12, "y(1)={y}, expected 5");

    // The same over a scalar product and a fold, which the walk writes
    // out element by element rather than one number at a time.
    let result = run(
        "model M function power input Real v[:]; input Real i[size(v, 1)]; input Real a; \
         output Real p; \
         protected Integer rounds; \
         algorithm rounds := 0; \
         while rounds < 1 and a > 0 loop p := v * i; rounds := rounds + 1; end while; end power; \
         Real y; equation y = power({1, 2 * time}, {3, 4}, time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let column = result.columns.iter().position(|c| c == "y").unwrap();
    let y = result.rows.last().unwrap()[column];
    assert!((y - 11.0).abs() < 1e-12, "y(1)={y}, expected 1*3 + 2*4");

    // An array the body declares for itself: as long as what it was
    // handed, filled in a loop, then folded. Sum, product and the
    // smallest of them are all written out element by element.
    let result = run(
        "model M function shape input Real v[:]; input Real a; output Real p;          protected Real w[size(v, 1)]; Real doubled[2]; Integer k;          algorithm k := 1;          while k <= size(v, 1) and a > 0 loop w[k] := v[k] + 1; k := k + 1; end while;          doubled := 2 .* w;          p := sum(w) + product(w) + min(w) + max(w) + sum(doubled) + sum({a, 0}) - a; end shape;          Real y; equation y = shape({1 * time, 2 * time}, time);          annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let column = result.columns.iter().position(|c| c == "y").unwrap();
    let y = result.rows.last().unwrap()[column];
    // w = {2, 3}: 5 + 6 + 2 + 3, and twice w sums to 10.
    assert!((y - 26.0).abs() < 1e-12, "y(1)={y}, expected 26");
}

#[test]
fn a_walked_body_says_what_it_cannot_carry() {
    // A subscript the model decides, and decides badly: the loop runs
    // as long as `time` says, so nothing could have been settled while
    // flattening. What the walk cannot do it says, and the answer that
    // comes back is not a number.
    let model = parse_model(
        "model M function odd input Real v[:]; input Real a; output Real y; \
         protected Integer k; \
         algorithm k := 0; while k < a loop y := v[k]; k := k + 1; end while; end odd; \
         Real y; equation y = odd({1, 2}, time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("parses");
    let trouble = compile(&model)
        .expect("compiles")
        .simulate()
        .expect_err("a subscript the model decided badly");
    assert!(
        trouble.to_string().contains("a whole number from one"),
        "{trouble}"
    );

    // An array given a value of the wrong length.
    let model = parse_model(
        "model M function odd input Real v[:]; input Real a; output Real y; \
         protected Real w[3]; Integer k; \
         algorithm k := 0; while k < a loop w := 2 .* v; k := k + 1; end while; y := w[1]; \
         end odd; \
         Real y; equation y = odd({1, 2}, time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("parses");
    let trouble = compile(&model)
        .expect("compiles")
        .simulate()
        .expect_err("three long, given two");
    assert!(trouble.to_string().contains("was given 2"), "{trouble}");
}

#[test]
fn a_walked_body_lays_out_what_it_answers_with() {
    // What a body answers with is laid out before it runs, so an
    // element it never fills is nothing rather than a name with no
    // value - which is what the language says an unassigned local is.
    let result = run("model M function half input Real a; output Real w[2]; \
         protected Integer k; \
         algorithm k := 0; while k < a loop w[1] := a; k := k + 1; end while; end half; \
         Real y[2]; equation y = half(time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;");
    let last = result.rows.last().unwrap();
    let at = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    assert!((last[at("y[1]")] - 1.0).abs() < 1e-12);
    assert_eq!(last[at("y[2]")], 0.0);
}

#[test]
fn a_walked_body_decides_over_arrays() {
    // Conditions, choices and negation over what was handed in, all
    // written out element by element on the way to one answer.
    let result = run(
        "model M function pick input Real v[:]; input Real a; output Real y; \
         protected Integer k; Real best; \
         algorithm best := -1e30; k := 1; \
         while k <= size(v, 1) and a > 0 loop \
           if v[k] > best and not (v[k] > 100 or v[k] < -100) then best := v[k]; end if; \
           k := k + 1; end while; \
         y := if sum(v) > 0 then best else -best; end pick; \
         Real y; equation y = pick({1, 5 * time, 200}, time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let column = result.columns.iter().position(|c| c == "y").unwrap();
    let y = result.rows.last().unwrap()[column];
    // The two hundred is passed over, so the largest kept is five.
    assert!((y - 5.0).abs() < 1e-12, "y(1)={y}, expected 5");

    // Two arrays multiplied are their scalar product, and a single
    // number goes with every element whichever side it is written on.
    let result = run(
        "model M function paired input Real v[:]; input Real w[size(v, 1)]; input Real a; \
         output Real y; protected Real scaled[size(v, 1)]; Integer k; \
         algorithm k := 0; \
         while k < 1 and a > 0 loop scaled := v .* 2; y := scaled * w + sum(3 .* w); \
           k := k + 1; end while; end paired; \
         Real y; equation y = paired({1, 2 * time}, {3, 4}, time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let column = result.columns.iter().position(|c| c == "y").unwrap();
    let y = result.rows.last().unwrap()[column];
    // Twice {1, 2} against {3, 4} is 22, and three times {3, 4} sums
    // to 21.
    assert!((y - 43.0).abs() < 1e-12, "y(1)={y}, expected 43");
}

#[test]
fn a_walked_body_answers_with_several_numbers() {
    // Nothing here can be unrolled while flattening, so the run walks
    // the body - and it answers with three numbers rather than one.
    // The model takes them one at a time, by the subscript Modelica
    // would write. v = {1, 2, 3} scaled by position is {1, 4, 9}.
    let result = run(
        "model M function scaled input Real v[3]; input Real a; output Real w[3]; \
         protected Integer k; \
         algorithm k := 1; \
         while k <= 3 and a > 0 loop w[k] := v[k] * k; k := k + 1; end while; end scaled; \
         Real y[3]; Real z; equation y = scaled({1, 2, 3 * time}, time); z = y[3]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let last = result.rows.last().unwrap();
    for (name, want) in [("y[1]", 1.0), ("y[2]", 4.0), ("y[3]", 9.0), ("z", 9.0)] {
        assert!(result.columns.iter().any(|c| c == name), "{name} is there");
        let column = result.columns.iter().position(|c| c == name).unwrap();
        assert!(
            (last[column] - want).abs() < 1e-12,
            "{name} = {}, expected {want}",
            last[column]
        );
    }
}

#[test]
fn a_record_given_to_a_function_whole_is_read_field_by_field() {
    // The shape a machine works its nominal voltage out in: a
    // function takes a record of brush parameters and reads `V` and
    // `ILinear` out of it, and the caller hands the record over by
    // name rather than writing its fields out. Binding the name alone
    // left the body reading fields nothing was bound to, and the
    // parameter it was working out went missing with no word said
    // about which name was wanting.
    let result = run(
        "record Brush parameter Real V = 2; parameter Real ILinear = 4; end Brush; \
         function drop input Brush brush; input Real i; output Real v; \
         algorithm v := if i > brush.ILinear then brush.V \
           else brush.V * i / brush.ILinear; end drop; \
         model M parameter Brush brushParameters; \
         parameter Real i = 2; \
         parameter Real v = drop(brushParameters, i); \
         Real y; equation y = v * time; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let y = result.columns.iter().position(|c| c == "y").expect("no y");
    // `2 * 2 / 4` is one, and a reading of one says both fields
    // arrived: a missing `V` or a missing `ILinear` refuses the model
    // outright rather than answering with the wrong number.
    assert!((result.rows.last().unwrap()[y] - 1.0).abs() < 1e-9);
}

#[test]
fn a_walked_body_answers_with_a_record() {
    // The shape the standard library's water is written in: a body
    // that cannot be unrolled fills one member of a record in one
    // branch and another in the other, and whoever reads a member
    // knows which branch it was. What no branch filled is nothing.
    let result = run(
        "package P record Props Real a; Real b; Real c; end Props; end P; \
         model M function boundary input Real p; input Real go; output P.Props pro; \
         protected Integer k; \
         algorithm k := 0; \
         while k < go loop \
           if p > 1 then pro.a := p; pro.b := 2 * p; \
           else pro.a := -p; pro.c := 3 * p; end if; \
           k := k + 1; end while; end boundary; \
         P.Props q; Real y; \
         equation q = boundary(2 * time, time); y = q.a + q.b + q.c; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let last = result.rows.last().unwrap();
    let at = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    // Two at the end, so the first branch: 2 and 4, and nothing for
    // the member the other branch would have filled.
    assert!((last[at("q.a")] - 2.0).abs() < 1e-12);
    assert!((last[at("q.b")] - 4.0).abs() < 1e-12);
    assert_eq!(last[at("q.c")], 0.0);
    assert!((last[at("y")] - 6.0).abs() < 1e-12);

    // The other branch, and with it the other member.
    let result = run(
        "package P record Props Real a; Real b; Real c; end Props; end P; \
         model M function boundary input Real p; input Real go; output P.Props pro; \
         protected Integer k; \
         algorithm k := 0; \
         while k < go loop \
           if p > 1 then pro.a := p; pro.b := 2 * p; \
           else pro.a := -p; pro.c := 3 * p; end if; \
           k := k + 1; end while; end boundary; \
         P.Props q; Real y; \
         equation q = boundary(0.5 * time, time); y = q.a + q.b + q.c; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    let last = result.rows.last().unwrap();
    let at = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    assert!((last[at("q.a")] + 0.5).abs() < 1e-12);
    assert_eq!(last[at("q.b")], 0.0);
    assert!((last[at("q.c")] - 1.5).abs() < 1e-12);

    // A record of more than plain numbers is left as it was written:
    // a name and a subscript are not enough for it, and the model is
    // told what does not add up rather than given a guess.
    let model = parse_model(
        "package P record Deep Real a; Real v[2]; end Deep; end P; \
         model M function boundary input Real p; input Real go; output P.Deep pro; \
         protected Integer k; \
         algorithm k := 0; while k < go loop pro.a := p; k := k + 1; end while; \
         end boundary; \
         P.Deep q; Real y; equation q = boundary(time, time); y = q.a; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("parses");
    let trouble = compile(&model).expect_err("a record of more than numbers");
    assert!(trouble.to_string().contains("unbalanced"), "{trouble}");
}

/// A value handed down an `extends` reaches the run as a number.
///
/// Flattening leaves the call inlined; what matters here is that the
/// parameters can then evaluate it, which is where a model whose
/// nominal voltage comes down from a base class used to stop.
#[test]
fn a_parameter_handed_a_call_down_an_extends_is_evaluated() {
    let result = run("package Top \
           function twice input Real i; output Real v; algorithm v := 2 * i; end twice; \
           partial model Base parameter Real k = 0; Real x(start = 1, fixed = true); \
             equation der(x) = -k; end Base; \
           model M extends Base(final k = twice(3)); \
             annotation(experiment(StopTime=1.0)); end M; \
         end Top;");
    let x = result.columns.iter().position(|c| c == "x").unwrap();
    let last = result.rows.last().unwrap();
    // k is 6, so x falls from 1 to -5 over the second.
    assert!(
        (last[x] + 5.0).abs() < 1e-6,
        "x = {}, expected the run to have used k = 6",
        last[x]
    );
}

/// A function body may leave a variable unset in one branch.
///
/// The language says an unassigned local or output of a function
/// starts where its type starts, and the standard library writes whole
/// property functions that way: the steam tables fill `cp` on one side
/// of a region boundary and `cv` on the other, each meant to be left
/// at zero where the other was set. Refusing that took thirty-two
/// models out of reach, the Fluid examples among them.
#[test]
fn a_function_may_leave_a_value_unset_in_one_branch() {
    let result = run("package Top \
           record Pair Real hot; Real cold; end Pair; \
           function pick input Real u; output Real y; \
           protected Pair p; \
           algorithm \
             if u > 0 then p.hot := u; else p.cold := -u; end if; \
             y := p.hot + p.cold; \
           end pick; \
           model M \
             Real warm = pick(time + 1); \
             annotation(experiment(StopTime=0.1)); \
           end M; \
         end Top;");
    let last = result.rows.last().expect("a final row");
    let warm = last[1];
    // The branch that ran set `hot` to 1.1; the branch that did not
    // leaves `cold` where its type starts, which is zero.
    assert!(
        (warm - 1.1).abs() < 1e-9,
        "the unset field should count as its own start of zero: {warm}"
    );
}

/// A carried body may shout, and may read a package constant.
#[test]
fn a_walked_body_may_shout_and_read_a_constant() {
    // Two things the standard library's own numerical bodies do, and
    // both used to kill the model rather than the body. A body with a
    // `while` on simulated values is carried to the run rather than
    // inlined, and then: a call to something that takes a String and
    // answers nothing was carried too, where it failed the walk for
    // both reasons at once - and a local declared from
    // `Modelica.Constants.eps` was a name the frame had never heard.
    let result = run(
        "package Modelica package Constants constant Real eps = 1e-15; end Constants; end Modelica; \
         package Top \
           function shout input String s; \
             external \"C\" ModelicaStreams_print(s); end shout; \
           function halve input Real u; output Real y; \
           protected \
             Real step; \
             constant Real eps = Modelica.Constants.eps; \
           algorithm \
             y := u; \
             step := 1.0; \
             while abs(step) > 100*eps loop \
               step := step/2; \
               y := y + step; \
               if y < 0 then shout(\"negative\"); end if; \
             end while; \
           end halve; \
           model M \
             Real y = halve(time); \
             annotation(experiment(StopTime=0.1)); \
           end M; \
         end Top;",
    );
    let last = result.rows.last().expect("a final row");
    // The steps halve from one and sum to one: 0.1 + 1 at the end.
    assert!(
        (last[1] - 1.1).abs() < 1e-6,
        "the body did not run to its end: {}",
        last[1]
    );
}

/// A function handed to another function, with some inputs filled in.
#[test]
fn a_function_may_be_handed_over_with_its_inputs_filled_in() {
    // `solveOneNonlinearEquation(function f(a = 2, b = -1), 0, 1)` is
    // how the standard library asks for a root. There is nowhere to
    // put a function value and nowhere it would survive to - the walk
    // takes numbers - so the receiver is specialized instead: a copy
    // with the function input replaced by ordinary numeric ones, and
    // every call to it rewritten into a direct call of the target.
    //
    // The receiver here bisects, as Brent's method does, so it never
    // inlines and is walked at run time. The root of `2u - 1` is a
    // half, and that is the number this asks for.
    let result = run("package P \
           partial function Scalar input Real u; output Real y; end Scalar; \
           function solve \
             input Scalar f; input Real lo; input Real hi; output Real x; \
           protected \
             Real mid; Real step; \
           algorithm \
             x := lo; \
             step := hi - lo; \
             while abs(step) > 1e-9 loop \
               step := step/2; \
               mid := x + step; \
               if f(mid) < 0 then x := mid; end if; \
             end while; \
           end solve; \
           model M \
             function line extends Scalar; input Real a = 1; input Real b = 0; \
               algorithm y := a*u + b; end line; \
             Real root; \
           equation \
             root = solve(function line(a = 2, b = -1), 0, 1); \
             annotation(experiment(StopTime=0.1)); \
           end M; \
         end P;");
    let last = result.rows.last().expect("a final row");
    assert!(
        (last[1] - 0.5).abs() < 1e-6,
        "the root of 2u - 1 is a half, and this said {}",
        last[1]
    );
}

/// What a handed-over function is refused for.
#[test]
fn a_handed_over_function_says_what_it_cannot_be() {
    // A name that is not a function here.
    let why = parse_model(
        "model M function solve input Real g; output Real x; \
           algorithm x := g; end solve; \
         Real y; equation y = solve(function nowhere(a = 2)); end M;",
    )
    .expect_err("no such function")
    .message;
    assert!(
        why.contains("is handed over and is not a function"),
        "{why}"
    );
    // A function with nothing to be solved for.
    let why = parse_model(
        "model M function flat output Real y; algorithm y := 1; end flat; \
         function solve input Real g; output Real x; algorithm x := g; end solve; \
         Real y; equation y = solve(function flat()); end M;",
    )
    .expect_err("nothing to solve for")
    .message;
    assert!(why.contains("takes nothing to be solved for"), "{why}");
}
