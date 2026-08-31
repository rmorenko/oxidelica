//! Function bodies and algorithm sections: what a call comes to, and what a body may say.

use super::shared::*;
use oxidelica_parser::ast::BinOp;
use oxidelica_parser::{parse_model, Expr};

#[test]
fn flat_model_passes_through_unchanged() {
    let m = parse_model("model M Real x(start = 1); equation der(x) = -x; end M;").unwrap();
    assert_eq!(m.name, "M");
    assert_eq!(m.components.len(), 1);
    assert_eq!(m.components[0].name, "x");
}

#[test]
fn functions_are_inlined_at_the_call_site() {
    let m = parse_model(
        "function scale input Real a; input Real b; output Real c; \
         algorithm c := a * b; end scale;\
         model M Real y; equation y = scale(time, 3); end M;",
    )
    .unwrap();
    // The call is gone: what remains is the substituted body.
    let mut refs = Vec::new();
    m.equations[0].rhs.collect_refs(&mut refs);
    assert!(refs.is_empty(), "unexpected references {refs:?}");
    assert!(!format!("{:?}", m.equations[0].rhs).contains("Call"));

    // A body of several assignments folds into one expression.
    let chained = parse_model(
        "function poly input Real x; output Real y; \
         algorithm y := x * x; y := y + x; end poly;\
         model M Real z; equation z = poly(3); end M;",
    )
    .unwrap();
    assert!(!format!("{:?}", chained.equations[0].rhs).contains("Call"));
}

#[test]
fn a_function_fills_a_tuple_of_targets() {
    const TWO: &str = "function two input Real a; output Real b; output Real c; \
         algorithm b := a + 1; c := a + 2; end two;";

    // Both outputs of one call, in one equation each.
    let m = parse_model(&format!(
        "{TWO} model M Real p; Real q; equation (p, q) = two(3); end M;"
    ))
    .unwrap();
    assert_eq!(m.equations.len(), 2);
    let text = format!("{:?}", m.equations);
    assert!(
        !text.contains("Call(\"two\""),
        "the call must inline: {text}"
    );

    // A skipped slot drops that output on the floor.
    let m = parse_model(&format!(
        "{TWO} model M Real q; equation (, q) = two(3); end M;"
    ))
    .unwrap();
    assert_eq!(m.equations.len(), 1);
    assert_eq!(format!("{:?}", m.equations[0].lhs), "Ref(\"q\")");

    // An expression context quietly takes the first output.
    let m = parse_model(&format!(
        "{TWO} model M Real y; equation y = two(3) * 10; end M;"
    ))
    .unwrap();
    let text = format!("{:?}", m.equations[0].rhs);
    assert!(text.contains("1.0"), "b = a + 1 is the value: {text}");
    assert!(!text.contains("2.0"), "c must not leak: {text}");

    // The same tuple inside an algorithm.
    let m = parse_model(&format!(
        "{TWO} model M Real p; Real q; algorithm (p, q) := two(3); end M;"
    ))
    .unwrap();
    assert_eq!(m.equations.len(), 2);

    // A parenthesised left side is still an ordinary equation.
    let m = parse_model("model M Real x; Real y; equation (x) = 2 * y; y = time; end M;").unwrap();
    assert_eq!(m.equations.len(), 2);
}

#[test]
fn named_arguments_and_defaults_fill_the_inputs() {
    const LINE: &str = "function line input Real x; input Real k = 2; input Real b = 10; \
         output Real y; algorithm y := k * x + b; end line;";

    // A named argument out of order; the untouched input defaults.
    let m = parse_model(&format!(
        "{LINE} model M Real y; equation y = line(5, b = 1); end M;"
    ))
    .unwrap();
    let text = format!("{:?}", m.equations[0].rhs);
    assert!(text.contains("2.0") && text.contains("5.0") && text.contains("1.0"));
    assert!(
        !text.contains("10.0"),
        "the default for b must lose: {text}"
    );

    // A default may lean on an earlier input.
    let m = parse_model(
        "function f input Real a; input Real half = a / 2; output Real y; \
         algorithm y := half; end f; \
         model M Real y; equation y = f(8); end M;",
    )
    .unwrap();
    assert!(format!("{:?}", m.equations[0].rhs).contains("8.0"));

    // The whole family of mistakes, each named.
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    let bad = |call: &str| {
        err(&format!(
            "{LINE} model M Real y; equation y = {call}; end M;"
        ))
    };
    assert!(bad("line(5, q = 1)").contains("no input named"));
    assert!(bad("line(5, x = 1)").contains("given twice"));
    assert!(bad("line(k = 2, 5)").contains("positional arguments must come before"));
    assert!(bad("line()").contains("missing its argument `x`"));
    assert!(err("model M Real y; equation y = sin(x = 1); end M;")
        .contains("cannot take named arguments"));
    assert!(err("model M Real p; Real q; equation (p, q) = 5; end M;")
        .contains("must be a function call"));
    assert!(err("model M Real p; Real q; algorithm (p, q) := 5; end M;")
        .contains("must be a function call"));
    assert!(err(
        "function f input Real a; output Real b; algorithm b := a; end f;\
         model M Real p; Real q; equation (p, q) = f(1); end M;"
    )
    .contains("1 output(s) for 2 target(s)"));
}

#[test]
fn while_break_and_return_run_at_compile_time() {
    // Euclid's algorithm: a `while` folding its state each round.
    let m = parse_model(
        "function gcd input Real a; input Real b; output Real g; \
         protected Real x; Real y; Real t; \
         algorithm x := a; y := b; \
         while y > 0.5 loop t := y; y := mod(x, y); x := t; end while; \
         g := x; end gcd;\
         model M Real r; equation r = gcd(48, 18); end M;",
    )
    .unwrap();
    assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(6.0)");

    // Newton's square root converges onto the analytic one.
    let m = parse_model(
        "function newton_sqrt input Real a; output Real r; \
         algorithm r := a; \
         while abs(r * r - a) > 1e-12 loop r := 0.5 * (r + a / r); end while; \
         end newton_sqrt;\
         model M Real y; equation y = newton_sqrt(2); end M;",
    )
    .unwrap();
    let Expr::Number(value) = &m.equations[0].rhs else {
        panic!("the loop must fold to a number: {:?}", m.equations[0].rhs);
    };
    assert!((value - 2.0f64.sqrt()).abs() < 1e-9, "{value}");

    // `break` ends a search as soon as it succeeds.
    let m = parse_model(
        "function first_square_above input Real limit; output Real k; \
         algorithm k := 0; \
         for i in 1:100 loop if i * i > limit then k := i; break; end if; end for; \
         end first_square_above;\
         model M Real y; equation y = first_square_above(20); end M;",
    )
    .unwrap();
    assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(5.0)");

    // `return` leaves early on one path and not the other.
    let m = parse_model(
        "function clipped input Real u; output Real y; \
         algorithm y := u; if u > 1 then y := 1; return; end if; y := y * 2; \
         end clipped;\
         model M Real a; Real b; equation a = clipped(3); b = clipped(0.25); end M;",
    )
    .unwrap();
    assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(1.0)");
    assert!(format!("{:?}", m.equations[1].rhs).contains("0.25"));

    // `break` inside a `while`, behind a decided `if`.
    let m = parse_model(
        "function capped output Real r; protected Real i; \
         algorithm i := 0; r := 0; \
         while 1 > 0 loop i := i + 1; \
         if i > 4.5 then break; end if; r := r + i; end while; \
         end capped;\
         model M Real y; equation y = capped(); end M;",
    )
    .unwrap();
    assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(10.0)");

    // `return` rides out of a `for` loop.
    let m = parse_model(
        "function findfirst input Real limit; output Real k; \
         algorithm k := 0; \
         for i in 1:100 loop if i * i > limit then k := i; return; end if; end for; \
         end findfirst;\
         model M Real y; equation y = findfirst(20); end M;",
    )
    .unwrap();
    assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(5.0)");

    // A tuple equation and a named argument inside a component:
    // both go through the prefixing walker.
    let m = parse_model(
        "function two input Real a; output Real b; output Real c; \
         algorithm b := a + 1; c := a * 2; end two;\
         model Sub parameter Real k = 3; Real p; Real q; \
         equation (p, q) = two(a = k); end Sub;\
         model M Sub s; Real y; equation y = s.p + s.q; end M;",
    )
    .unwrap();
    let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"s.p") && names.contains(&"s.q"));
    let text = format!("{:?}", m.equations);
    assert!(
        text.contains("Ref(\"s.k\")"),
        "the argument must prefix: {text}"
    );
}

#[test]
fn the_compile_time_folder_knows_the_numeric_builtins() {
    // One `while` round folds every builtin the folder knows; the
    // pi-flavoured results sum to exactly pi.
    let m = parse_model(
        "function burst output Real y; protected Real go; \
         algorithm go := 1; \
         while go > 0 loop \
         y := abs(-2) + sqrt(4) + exp(0) + log(1) + log10(10) \
            + sin(0) + cos(0) + tan(0) + asin(1) + acos(1) + atan(1) \
            + sinh(0) + cosh(0) + tanh(0) \
            + floor(1.5) + ceil(1.5) + integer(2.7) \
            + atan2(1, 1) + min(1, 2) + max(1, 2) \
            + div(7, 2) + mod(7, 4) + rem(7, 4); \
         go := 0; \
         end while; end burst;\
         model M Real y; equation y = burst(); end M;",
    )
    .unwrap();
    let Expr::Number(value) = &m.equations[0].rhs else {
        panic!("the burst must fold: {:?}", m.equations[0].rhs);
    };
    let expected = 25.0 + std::f64::consts::PI;
    assert!((value - expected).abs() < 1e-12, "{value} vs {expected}");
}

#[test]
fn flow_control_error_paths() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // A loop that can never finish hits the backstop.
    assert!(err("model M Real y; \
         algorithm y := 0; while y < 1e9 loop y := y + 1; end while; end M;")
    .contains("did not finish"));
    // A `break` guarded by a condition only the run decides: which
    // statements run is what the leaving decides, so there is nothing
    // to write out and the call stands for the run to walk.
    let walked = parse_model(
        "function f input Real u; output Real y; algorithm y := 0; \
         for i in 1:3 loop if u > 0 then break; end if; y := y + 1; end for; end f;\
         model M Real u; Real y; equation u = time; y = f(u); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .unwrap();
    assert!(walked.functions.iter().any(|body| body.name == "f"));
    assert!(
        format!("{:?}", walked.equations).contains("Call(\"f\""),
        "{:?}",
        walked.equations
    );

    // `return` is for functions, `break` for loops.
    assert!(
        err("model M Real y; algorithm y := 1; return; end M;").contains("belongs in a function")
    );
    assert!(err(
        "function f input Real a; output Real b; algorithm b := a; break; end f;\
         model M Real y; equation y = f(1); end M;"
    )
    .contains("`break` outside of a loop in function"));
    assert!(err("model M Real y; algorithm y := 1; break; end M;")
        .contains("`break` outside of a loop"));
}

#[test]
fn functions_reach_through_expressions_and_defaults() {
    // Local variables with bindings act as defaults inside a body.
    let m = parse_model(
        "function offset input Real a; output Real b; Real bias = 10; \
         algorithm b := a + bias; end offset;\
         model M Real y; equation y = offset(5) + offset(1); end M;",
    )
    .unwrap();
    let text = format!("{:?}", m.equations[0].rhs);
    assert!(!text.contains("Call"), "calls survived: {text}");

    // Nested calls inline from the inside out.
    let nested = parse_model(
        "function twice input Real a; output Real b; algorithm b := 2 * a; end twice;\
         model M Real y; equation y = twice(twice(3)); end M;",
    )
    .unwrap();
    assert!(!format!("{:?}", nested.equations[0].rhs).contains("Call"));

    // Built-ins are not shadowed by the registry lookup.
    let builtin = parse_model("model M Real y; equation y = sin(time); end M;").unwrap();
    assert!(format!("{:?}", builtin.equations[0].rhs).contains("Call(\"sin\""));
}

#[test]
fn a_function_may_leave_from_inside_a_loop() {
    // `return` reaches out through a loop, which is a thing the
    // walk that looks for one has to know.
    let m = parse_model(
        "function first_over input Real limit; output Real k; algorithm k := 0; for i in 1:10 loop if i * i > limit then k := i; return; end if; end for; end first_over; model M Real y; equation y = first_over(30); end M;",
    )
    .unwrap();
    assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(6.0)");

    // And a `while` inside a `for` is left the same way.
    let m = parse_model(
        "function counted output Real n; protected Real i; algorithm n := 0; for outer_step in 1:3 loop i := 0; while i < 2 loop i := i + 1; n := n + 1; if n > 4 then return; end if; end while; end for; end counted; model M Real y; equation y = counted(); end M;",
    )
    .unwrap();
    assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(5.0)");
}

#[test]
fn a_tuple_equation_may_sit_inside_a_component() {
    // Prefixing and constant substitution both walk a tuple, and
    // only inside a component do they have anything to do.
    let m = parse_model(
        "package K constant Real gain = 3; end K; function two input Real a; output Real b; output Real c; algorithm b := a * K.gain; c := a + 1; end two; model Sub parameter Real seed = 2; Real p; Real q; equation (p, q) = two(seed); end Sub; model M Sub s; Real out; equation out = s.p + s.q; end M;",
    )
    .unwrap();
    let named: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
    assert!(
        named.contains(&"s.p") && named.contains(&"s.q"),
        "{named:?}"
    );
    let text = format!("{:?}", m.equations);
    assert!(text.contains("Ref(\"s.seed\")"), "prefixed: {text}");
    assert!(text.contains("Number(3.0)"), "the constant folded: {text}");
}

#[test]
fn the_rest_of_the_refusals_are_named_too() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // An initial equation whose sides are different shapes.
    assert!(err("model M Real v[2](start = {0, 0}); equation der(v[1]) = 0; der(v[2]) = 0; initial equation v = {1, 2, 3}; end M;")
        .contains("shapes that do not match"));
    // A loop inside a `when` of an algorithm is unrolled like any
    // other: what it leaves changed is what the event assigns.
    let looped = parse_model("model M Real u; Real v[2]; discrete Real c(start = 0); equation u = time; v = {1, 2}; algorithm when u > 1 then for i in 1:2 loop v[i] := i; end for; end when; end M;")
        .unwrap();
    assert_eq!(looped.when_clauses[0].branches[0].actions.len(), 2);
    // A range with a fractional end is still a range: `1:2.5` runs over
    // 1 and 2, the way the range operator reads everywhere else. It was
    // refused here once for not being whole, which was stricter than
    // the language - so what it is refused for now is two equations for
    // the one unknown.
    let twice_over =
        parse_model("model M Real y; equation for i in 1:2.5 loop y = i; end for; end M;").unwrap();
    assert_eq!(twice_over.equations.len(), 2);
    let twice =
        parse_model("model M Real y; algorithm for i in 1:2.5 loop y := i; end for; end M;")
            .unwrap();
    assert_eq!(
        format!("{:?}", twice.equations),
        "[EquationItem { lhs: Ref(\"y\"), rhs: Number(2.0), origin: \"\" }]"
    );
    // A tuple filled by something that is not a function.
    assert!(err(
        "model Sub Real a; end Sub; model M Real p; Real q; Sub s; equation (p, q) = s.a; end M;"
    )
    .contains("must be a function call"));
    // A function that calls itself has no bottom to inline to, so it
    // is carried whole and the run walks it - where a call with no way
    // out is a runaway rather than a refusal.
    let carried = parse_model("function loops input Real a; output Real b; algorithm b := loops(a); end loops; model M Real y; equation y = loops(1); end M;")
        .unwrap();
    assert_eq!(carried.functions.len(), 1);
    assert_eq!(carried.functions[0].name, "loops");
}

#[test]
fn the_last_of_the_builtins_say_what_they_mean() {
    // `homotopy` offers an easier problem to start from; this
    // compiler takes the real one.
    let m = parse_model("model M Real y; equation y = homotopy(3 * time, time); end M;").unwrap();
    assert!(format!("{:?}", m.equations[0].rhs).contains("Number(3.0)"));

    // The operators take their arguments by name as readily as a
    // function does, and the standard library writes them that way.
    // Naming them puts them in order, whichever order they were written.
    let m = parse_model(
        "model M Real y; equation y = homotopy(simplified = time, actual = 3 * time); end M;",
    )
    .unwrap();
    assert!(format!("{:?}", m.equations[0].rhs).contains("Number(3.0)"));
    // A name the operator does not have says so, rather than being
    // quietly taken for the argument that stands in that place.
    let refused =
        parse_model("model M Real y; equation y = homotopy(actual = time, easier = 0); end M;")
            .unwrap_err()
            .to_string();
    assert!(refused.contains("no argument called `easier`"), "{refused}");
    // So does a name given twice, or given after the same argument
    // was already handed over by position.
    let refused = parse_model("model M Real y; equation y = homotopy(time, actual = 0); end M;")
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("both by name and by position"),
        "{refused}"
    );
    // The same name twice.
    let refused = parse_model(
        "model M Real y; equation y = homotopy(actual = time, actual = 0, simplified = 0); end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("`actual` twice"), "{refused}");
    // An argument passed over leaves the operator with a gap where one
    // of its arguments should be.
    let refused = parse_model("model M Real y; equation y = homotopy(simplified = time); end M;")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("nothing for `actual`"), "{refused}");
    // A positional argument after a named one is not something the
    // language allows, and the operator says so rather than reading it
    // as though the order had been kept.
    let refused = parse_model("model M Real y; equation y = semiLinear(x = time, 2, 5); end M;")
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("by position after one given by name"),
        "{refused}"
    );
    // An operator with no declared argument names keeps the refusal it
    // had: naming arguments is something a function takes.
    let refused = parse_model("model M Real y; equation y = sin(x = time); end M;")
        .unwrap_err()
        .to_string();
    assert!(
        !refused.is_empty(),
        "a named argument to `sin` was accepted"
    );

    // `semiLinear` is two slopes meeting at zero.
    let m = parse_model(
        "model M Real u; Real y; equation u = time - 1; y = semiLinear(u, 2, 5); end M;",
    )
    .unwrap();
    let text = format!("{:?}", m.equations[1].rhs);
    assert!(text.starts_with("If(Rel(Ge"), "{text}");
    assert!(
        text.contains("Number(2.0)") && text.contains("Number(5.0)"),
        "{text}"
    );
}

#[test]
fn a_when_may_stand_among_the_statements_of_an_algorithm() {
    let m = parse_model(
        "model M Real u; discrete Real counted(start = 0); Real doubled; equation u = time; algorithm doubled := 2 * u; when u > 0.5 then counted := pre(counted) + 1; elsewhen u > 0.9 then counted := pre(counted) + 2; end when; end M;",
    )
    .unwrap();
    // The `when` became a clause; the rest of the section is still
    // one equation per variable it assigns.
    assert_eq!(m.when_clauses.len(), 1);
    assert_eq!(m.when_clauses[0].branches.len(), 2);
    assert!(m
        .equations
        .iter()
        .any(|equation| format!("{:?}", equation.lhs) == "Ref(\"doubled\")"));

    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // Only at the top of a section, and only assignments.
    assert!(err(
        "model M Real u; discrete Real c(start = 0); equation u = time; algorithm if u > 0 then when u > 1 then c := 1; end when; end if; end M;"
    )
    .contains("not inside an `if`"));
    // The body of a `when` is an algorithm like any other: a write to
    // one element lands on that element's own name.
    let element = parse_model(
        "model M Real u; Real v[2]; discrete Real c(start = 0); equation u = time; v = {1, 2}; algorithm when u > 1 then c := 1; v[1] := 2; end when; end M;"
    )
    .unwrap();
    assert!(
        format!("{:?}", element.when_clauses).contains("Assign(\"v[1]\""),
        "{:?}",
        element.when_clauses
    );
}

#[test]
fn what_cannot_be_inlined_travels_with_the_model() {
    // A body that calls itself unrolls where what decides the
    // recursion is settled: `even(4)` counts down to a number, and
    // nothing has to travel.
    let m = parse_model(
        "model M function even input Real n; output Real y; \
         algorithm if n <= 0 then y := 1; else y := odd(n - 1); end if; end even; \
         function odd input Real n; output Real y; \
         algorithm if n <= 0 then y := 0; else y := even(n - 1); end if; end odd; \
         Real y; equation y = even(4); end M;",
    )
    .unwrap();
    assert!(m.functions.is_empty(), "{:?}", m.functions);
    assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(1.0)");

    // Where it is not settled - the count is a variable the run holds -
    // the function is carried whole, with everything it calls, and
    // named the way the registry knows it so the walk finds it.
    let m = parse_model(
        "model M function even input Real n; output Real y; \
         algorithm if n <= 0 then y := 1; else y := odd(n - 1); end if; end even; \
         function odd input Real n; output Real y; \
         algorithm if n <= 0 then y := 0; else y := even(n - 1); end if; end odd; \
         Real y; equation y = even(4 * time); end M;",
    )
    .unwrap();
    let carried: Vec<&str> = m
        .functions
        .iter()
        .map(|class| class.name.as_str())
        .collect();
    assert!(
        carried.contains(&"M.even") && carried.contains(&"M.odd"),
        "{carried:?}"
    );
    assert!(
        format!("{:?}", m.equations[0].rhs).starts_with("Call(\"M.even\""),
        "{:?}",
        m.equations[0].rhs
    );
    let inside = format!("{:?}", m.functions[0].algorithm);
    assert!(
        inside.contains("M.odd") || inside.contains("M.even"),
        "{inside}"
    );

    // A function inlines as before where it can, and then nothing
    // travels: the model carries only what the run has to walk.
    let plain = parse_model(
        "model M function twice input Real x; output Real y; algorithm y := x * 2; end twice; \
         Real y; equation y = twice(3); end M;",
    )
    .unwrap();
    assert!(plain.functions.is_empty());

    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // What a walked body may hold is narrower than what an inlined one
    // may: a walk carries numbers, one at a time. The count comes from
    // the run, so the recursion has no bottom the compiler can reach
    // and the body is walked rather than unrolled.
    let recursive = |extra: &str, body: &str| {
        format!(
            "model M function f input Real a; output Real b; {extra} \
             algorithm {body} if a > 0 then b := f(a - 1); end if; end f; \
             Real y; equation y = f(time); end M;"
        )
    };
    // An array the body holds while it runs is carried: the walk keeps
    // each element under its own name, the way the flat model does.
    let m = parse_model(&recursive("protected Real v[3];", "v[1] := a; b := v[1];"))
        .expect("an array held while the walk runs");
    assert_eq!(m.functions.len(), 1);
    // What comes back may be several numbers: the model takes them one
    // at a time, by the subscript Modelica would write.
    let m = parse_model(
        "model M function f input Real a; output Real b[2]; \
         algorithm b[1] := a; if a > 0 then b := f(a - 1); end if; end f; \
         Real y[2]; equation y = f(time); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an answer of two numbers");
    let written = format!("{:?}", m.equations);
    assert!(written.contains("Index(Call(\"M.f\""), "{written}");

    // A length the compiler cannot see is another matter: nothing
    // knows how many numbers to take, so the answer stands as one and
    // does not fit against what it was put beside.
    assert!(err("model M function f input Real a; output Real b[:]; \
         algorithm b[1] := a; if a > 0 then b := f(a - 1); end if; end f; \
         Real y[2]; equation y = f(time); end M;")
    .contains("shapes [2] and []"));
    assert!(err(&recursive("protected String s;", "b := a;")).contains("`s` is a String"));
}

#[test]
fn a_function_may_say_how_to_differentiate_itself() {
    // The call is inlined for its value as before, and keeps the rule
    // beside it. Nothing that reads the expression for what it is worth
    // sees the difference; only differentiation reaches for the rule.
    let m = parse_model(&format!(
        "model M {NOT_SMOOTH} Real x(start = 2, fixed = true); Real v; \
         equation der(x) = v; f(x) = 4 + time; end M;"
    ))
    .unwrap();
    let written = format!("{:?}", m.equations);
    assert!(written.contains("WithDerivative"), "{written}");
    // The rule was inlined with a name of the compiler's own standing
    // in for the argument's derivative, so nothing a model writes can
    // collide with it.
    assert!(written.contains("$seed0"), "{written}");

    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // A derivative that is not there, and one of the wrong shape.
    assert!(err(&format!(
        "model M {} Real x(start = 2, fixed = true); Real v; \
         equation der(x) = v; f(x) = 4 + time; end M;",
        NOT_SMOOTH.replace("derivative = fd", "derivative = nope")
    ))
    .contains("there is no such function"));
    assert!(err(&format!(
        "model M {} Real x(start = 2, fixed = true); Real v; \
         equation der(x) = v; f(x) = 4 + time; end M;",
        NOT_SMOOTH.replace("input Real x_der;", "")
    ))
    .contains("what `M.f` takes, and then one derivative for each"));
    // The options the specification allows beside it change what the
    // named function takes and answers, so an annotation carrying one
    // is read past and not kept: the call stands with no derivative
    // rule of its own rather than a wrong one.
    let m = parse_model(&format!(
        "model M {} Real x(start = 2, fixed = true); Real v; \
         equation der(x) = v; f(x) = 4 + time; end M;",
        NOT_SMOOTH.replace("derivative = fd", "derivative(order = 2) = fd")
    ))
    .unwrap();
    assert!(
        !format!("{:?}", m.equations).contains("WithDerivative"),
        "{:?}",
        m.equations
    );
}

#[test]
fn what_a_function_says_about_its_inverse_is_checked() {
    // The inverse is recorded and checked, and then set aside: the
    // nonlinear corrector already solves `f(x) = u` for `x`. What the
    // check is for is an annotation naming something that is not there.
    let inverse = |clause: &str| {
        format!(
            "model M function f input Real x; output Real y; algorithm y := x * x; \
             annotation(inverse({clause})); end f; \
             function g input Real y; output Real x; algorithm x := sqrt(y); end g; \
             Real w; equation w = f(2); end M;"
        )
    };
    assert!(parse_model(&inverse("x = g(y)")).is_ok());
    // Several entries, and an inverse handed more than one thing.
    assert!(parse_model(&inverse("x = g(y, x), x = g(y)")).is_ok());
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    assert!(err(&inverse("x = nope(y)")).contains("inverts it, and there is no such function"));
    assert!(err(&inverse("z = g(y)")).contains("which is not one of its inputs"));
    assert!(err(&inverse("x = g(q)")).contains("neither takes nor gives"));
    assert!(err(&inverse("x = g(y) x = g(y)")).contains("expected `,` or `)` in inverse"));
}

#[test]
fn a_check_may_be_written_where_the_statements_are() {
    // `assert` among the statements is a statement, and inside a loop
    // it is one check per round with the loop variable folded in.
    let m = parse_model(
        "model M parameter Real g[3] = {1, 2, 3}; Real y; \
         algorithm y := 0; \
         for i loop assert(g[i] > 0, \"every gain must be positive\"); y := y + g[i]; end for; \
         end M;",
    )
    .unwrap();
    assert_eq!(m.asserts.len(), 3);
    assert!(m
        .asserts
        .iter()
        .all(|(_, message)| message.contains("gain")));
    assert!(format!("{:?}", m.asserts[2].0).contains("g[3]"));

    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // A check inside a function cannot travel out through the
    // expression the call becomes, so it is set aside and taken up by
    // the model, which does have somewhere to put it. The call is made
    // at every step of the run, so the check holds at every step.
    let m = parse_model(
        "model M function f input Real x; output Real y; \
         algorithm assert(x > 0, \"positive\"); y := x; end f; \
         Real y; equation y = f(2); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a check from an inlined body");
    assert_eq!(m.asserts.len(), 1);
    assert_eq!(m.asserts[0].1, "positive");
    assert!(format!("{:?}", m.asserts[0].0).contains("Number(2.0)"));

    // Only the branch an `if` takes is worked out, so a check from a
    // branch comes with the condition in front of it.
    let m = parse_model(
        "model M function f input Real x; output Real y; \
         algorithm assert(x > 0, \"positive\"); y := x; end f; \
         parameter Boolean high = false; \
         Real y; equation y = if high then f(-1) else 3; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a check from a branch behind a condition");
    assert_eq!(m.asserts.len(), 1);
    assert!(
        format!("{:?}", m.asserts[0].0).starts_with("Or(Not(Ref(\"high\"))"),
        "{:?}",
        m.asserts[0].0
    );
    // A call standing on its own takes nothing from its outputs; what
    // it is written for is the checks its body makes, and here they
    // have somewhere to go.
    let m = parse_model(
        "model M function f input Real x; output Real y; \
         algorithm assert(x > 0, \"positive\"); y := x; end f; \
         Real y; algorithm f(1); y := 1; end M;",
    )
    .unwrap();
    assert_eq!(m.asserts.len(), 1);
    assert_eq!(m.asserts[0].1, "positive");
    // A name that is not a function cannot be called that way.
    assert!(
        err("model M Real g; Real y; algorithm g(1); y := 1; end M;").contains("is not a function")
    );
    assert!(
        err("model M Real y; algorithm terminate(\"now\"); y := 1; end M;")
            .contains("belongs in a `when`")
    );
}

#[test]
fn a_package_may_be_opened_wholesale() {
    const LIB: &str = "package Lib constant Real pi = 3.5; \
         model Gain parameter Real k = 2; Real y; equation y = k * time; end Gain; \
         model Lag parameter Real T = 1; Real z(start = 0); \
         equation der(z) = -z / T; end Lag; end Lib;";

    // `import Lib.*;` puts everything inside within reach by name.
    let m = parse_model(&format!(
        "{LIB} model M import Lib.*; Gain g(k = 3); Lag l(T = 2); end M;"
    ))
    .unwrap();
    let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
    assert!(
        names.contains(&"g.y") && names.contains(&"l.z"),
        "{names:?}"
    );

    // A list names several at once.
    let m = parse_model(&format!(
        "{LIB} model M import Lib.{{Gain, Lag}}; Gain g; Lag l; end M;"
    ))
    .unwrap();
    assert_eq!(m.components.len(), 4);

    // A constant reaches its user whether it was brought in by name or
    // through a wildcard - both open the package.
    let m = parse_model(&format!(
        "{LIB} model M import Lib.pi; Real y; equation y = pi; end M;"
    ))
    .unwrap();
    assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(3.5)");
    let m = parse_model(&format!(
        "{LIB} model M import Lib.*; Real y; equation y = pi; end M;"
    ))
    .unwrap();
    assert_eq!(
        format!("{:?}", m.equations[0].rhs),
        "Number(3.5)",
        "a wildcard opens the package's constants too"
    );

    // But a component of the model outranks a wildcard-imported
    // constant of the same name: `pi` here is the variable.
    let m = parse_model(&format!(
        "{LIB} model M import Lib.*; Real pi; equation pi = 9; end M;"
    ))
    .unwrap();
    assert_eq!(format!("{:?}", m.equations[0].lhs), "Ref(\"pi\")");

    // What has a name of its own outranks a package opened
    // wholesale: the local `Gain` wins over `Lib.Gain`.
    let m = parse_model(&format!(
        "{LIB} model M import Lib.*; \
         model Gain Real y; equation y = 42; end Gain; \
         Gain g; end M;"
    ))
    .unwrap();
    assert_eq!(m.components.len(), 1, "{:?}", m.components);
    assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(42.0)");
}

#[test]
fn nested_modifiers_reach_children_and_attributes() {
    let m = oxidelica_parser::parse_model(
        "model Leaf parameter Real k = 1; Real x(start = 0); \
         equation der(x) = k; end Leaf; \
         model Middle Leaf leaf; end Middle; \
         model Top Middle mid(leaf(k = 5, x(start = 7))); end Top;",
    )
    .unwrap();
    let k = m
        .components
        .iter()
        .find(|c| c.name == "mid.leaf.k")
        .unwrap();
    assert_eq!(k.binding, Some(Expr::Number(5.0)));
    let x = m
        .components
        .iter()
        .find(|c| c.name == "mid.leaf.x")
        .unwrap();
    assert_eq!(x.start, Some(Expr::Number(7.0)));

    // `fixed` travels the same way.
    let fixed = oxidelica_parser::parse_model(
        "model Leaf Real x(start = 0); equation der(x) = 1; end Leaf; \
         model Top Leaf leaf(x(fixed = true)); end Top;",
    )
    .unwrap();
    assert_eq!(
        fixed
            .components
            .iter()
            .find(|c| c.name == "leaf.x")
            .unwrap()
            .fixed,
        Some(true)
    );
}

#[test]
fn chained_type_aliases_resolve_to_a_primitive() {
    let m = oxidelica_parser::parse_model(
        "package SI type Angle = Real(unit = \"rad\", start = 3); end SI; \
         package Units type Turn = SI.Angle; end Units; \
         model M Units.Turn phi; equation der(phi) = 1; end M;",
    )
    .unwrap();
    let phi = m.components.iter().find(|c| c.name == "phi").unwrap();
    assert_eq!(phi.type_name, "Real");
    assert_eq!(phi.start, Some(Expr::Number(3.0)));
}

#[test]
fn an_alias_contributes_its_fixed_attribute() {
    let m = oxidelica_parser::parse_model(
        "package Units type Held = Real(start = 2, fixed = true); end Units; \
         model M Units.Held x; equation der(x) = 1; end M;",
    )
    .unwrap();
    let x = m.components.iter().find(|c| c.name == "x").unwrap();
    assert_eq!(x.fixed, Some(true));
    assert_eq!(x.start, Some(Expr::Number(2.0)));
}

#[test]
fn an_algorithm_section_of_a_model_becomes_equations() {
    let m = parse_model(
        "model M parameter Real limit = 1.5; Real u; Real y; Real gain; Real total; \
         equation u = 2 * time; \
         algorithm \
           gain := 1.0; \
           if u > limit then y := limit; gain := limit / u; \
           elseif u < -limit then y := -limit; gain := -limit / u; \
           else y := u; end if; \
           total := 0.0; \
           for i in 1:3 loop total := total + i * u; end for; \
         end M;",
    )
    .unwrap();
    // One equation per assigned variable, in the order the algorithm
    // writes them, plus the one from the equation section.
    let assigned: Vec<String> = m
        .equations
        .iter()
        .skip(1)
        .map(|e| format!("{:?}", e.lhs))
        .collect();
    assert_eq!(
        assigned,
        vec![
            "Ref(\"gain\")".to_string(),
            "Ref(\"y\")".to_string(),
            "Ref(\"total\")".to_string()
        ]
    );
    // The branch became one if-expression, and the loop unrolled
    // into 1*u + 2*u + 3*u rather than staying a loop.
    let gain = &m.equations[1].rhs;
    assert!(matches!(gain, Expr::If(_, _, _)), "{gain:?}");
    let total = format!("{:?}", m.equations[3].rhs);
    assert!(!total.contains("Ref(\"i\")"), "the loop variable survived");
    assert_eq!(total.matches("Ref(\"u\")").count(), 3);
}

#[test]
fn algorithm_error_paths() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // A variable written in one branch only, with nothing before it.
    assert!(err("model M Real u; Real y; equation u = time; \
         algorithm if u > 1 then y := 1; end if; end M;")
    .contains("assigned in one branch only"));

    // A model's own protected variable, written in one branch of a
    // `when` and read outside it: what the branch leaves alone holds
    // what it held, and the first time round that is its start. A
    // digital gate is written this way - `y_auxiliary` set when the
    // delay expires, read every step - and eleven models stood at
    // this refusal.
    let after = parse_model(
        "type Logic = enumeration(U, X0, X1); \
         model M Real x; Logic y; protected Logic held(start = Logic.U, fixed = true); \
         algorithm if x > 0.5 then held := Logic.X1; end if; y := held; \
         equation x = time; end M;",
    )
    .unwrap();
    assert!(format!("{:?}", after.equations).contains("held"));
    // What the rest of the model can see is another matter: an
    // equation may be solving for it, and a branch that says nothing
    // about it is a gap rather than a variable holding still.
    assert!(err("model M Real u; Real y; equation u = time; \
         algorithm if u > 1 then y := 1; end if; end M;")
    .contains("assigned in one branch only"));

    // A loop whose bounds are not constant.
    assert!(err("model M Real u; Real y; equation u = time; \
         algorithm y := 0; for i in 1:u loop y := y + i; end for; end M;")
    .contains("a range needs bounds the compiler can see"));
    // A `while` among a model's own statements is unrolled where it
    // stands, so its trip count has to be settled: only a call can be
    // left standing for the run to walk.
    assert!(err("model M Real u; Real y; equation u = time; \
         algorithm y := 0; while y < u loop y := y + 1; end while; end M;")
    .contains("the trip count of a loop is not settled here"));
    // An empty loop body is now allowed (valid Modelica).
}

#[test]
fn the_new_expression_forms_travel_through_every_walker() {
    // Ranges, comprehensions, matrices and `end` written where each
    // walker touches them: inside a component's binding (prefixing
    // and substitution), behind a class constant, and with logical
    // operators around them.
    let m = parse_model(
        "package K constant Real width = 3; end K;              model Inner parameter Real n = 3;              parameter Real edge_case[3] = {i + K.width for i in 1:3};              Real gate;              equation gate = if time > 0.5 and not time > 2.0 or time < -1                then edge_case[end] else edge_case[1]; end Inner;              model M Inner part; end M;",
    )
    .unwrap();
    let binding = m
        .components
        .iter()
        .find(|c| c.name == "part.edge_case[2]")
        .and_then(|c| c.binding.clone())
        .unwrap();
    // 2 + K.width with the constant substituted.
    assert!(format!("{binding:?}").contains("Number(3.0)"));
    let gate = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs).contains("gate"))
        .unwrap();
    assert!(format!("{:?}", gate.rhs).contains("edge_case[3]"));

    // A class alias may be a whole file: a library with one class to a
    // file writes short definitions that way, and what comes out is a
    // class standing for the other.
    let read = oxidelica_parser::parse_file("package P = Q;").unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].alias_of.as_ref().unwrap().0, "Q");

    // A range with a step, and one in a for loop with a step.
    let stepped = parse_model("model M Real v[3]; equation v = 1:3:7; end M;").unwrap();
    let text = format!("{:?}", stepped.equations);
    assert!(text.contains("Number(7.0)"), "{text}");
    // A loop takes a stepped range too, and runs over 1, 3, 5, 7, 9.
    // It used to be refused for having a step at all.
    let stepped =
        parse_model("model M Real x; equation for i in 1:2:9 loop x = i; end for; end M;").unwrap();
    assert_eq!(stepped.equations.len(), 5);
    assert!(format!("{:?}", stepped.equations).contains("Number(9.0)"));

    // `fixed` through an alias-typed declaration's modifier list.
    let held = parse_model(
        "package U type V = Real(unit = \"m\"); end U;              model M U.V x(start = 2, fixed = true); equation der(x) = 1; end M;",
    )
    .unwrap();
    assert_eq!(
        held.components
            .iter()
            .find(|c| c.name == "x")
            .unwrap()
            .fixed,
        Some(true)
    );
}

#[test]
fn functions_take_and_return_arrays() {
    // Reversal per element, a whole-array body, calls by qualified
    // name, and the result flowing on into a scalar product.
    let m = parse_model(
        "package Lib                function reverse input Real a[3]; output Real b[3];                algorithm for i in 1:3 loop b[i] := a[4 - i]; end for; end reverse;                function axpy input Real a; input Real x[3]; input Real y[3];                output Real z[3]; algorithm z := a * x .+ y; end axpy;              end Lib;              model M Real v[3]; Real r[3]; Real w[3]; Real check;              equation v = {1, 2, 3}; r = Lib.reverse(v);              w = Lib.axpy(10, v, r); check = w * {1, 1, 1}; end M;",
    )
    .unwrap();
    // Everything inlined: no calls survive into the flat model.
    let text = format!("{:?}", m.equations);
    assert!(!text.contains("Call"), "{text}");
    // r[1] is the last element of v: the function body reversed the
    // references, and v stays a variable rather than being folded.
    let r1 = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"r[1]\")")
        .unwrap();
    assert_eq!(r1.rhs, Expr::Ref("v[3]".to_string()));

    // An output never fully assigned is named element by element.
    let error = parse_model(
        "package Lib function half input Real a[2]; output Real b[2];              algorithm b[1] := a[1]; end half; end Lib;              model M Real v[2]; Real w[2];              equation v = {1, 2}; w = Lib.half(v); end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("b[2]"), "{error}");

    // A subscripted target with a subscript nothing can fold.
    let error = parse_model(
        "package Lib function bad input Real a; output Real b[2];              algorithm b[a] := 1; b[1] := 1; end bad; end Lib;              model M Real q; Real w[2]; equation q = 1; w = Lib.bad(q); end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("whole number the compiler can see"),
        "{error}"
    );
}

#[test]
fn a_string_travels_through_every_shape_of_expression() {
    // The string pass rewrites comparisons and has to hand every other
    // shape back untouched, so a model with a string in it must still
    // carry arrays, matrices, ranges, comprehensions, subscripts,
    // tuples and named arguments through unharmed.
    let m = parse_model(
        "function pick input Real a; input Real b; output Real hi; output Real lo; \
         algorithm hi := max(a, b); lo := min(a, b); end pick; \
         function scaled input Real v; input Real by = 2; output Real y; \
         algorithm y := v * by; end scaled; \
         model M \
           parameter String medium = \"water\"; \
           parameter Real row[2, 2] = [1, 2; 3, 4]; \
           parameter Real ramp[3] = {i * 2 for i in 1:3}; \
           parameter Real span[3] = 1:1:3; \
           Real hi; Real lo; Real picked; Real named; Real gate; Real total; \
         equation \
           (hi, lo) = pick(ramp[1], ramp[end]); \
           picked = row[2, 1] + sum(span) + sum({j * 1.0 for j in 1:3}); \
           named = scaled(3, by = 4); \
           gate = if medium == \"water\" and not medium <> \"water\" then 1 else 0; \
           total = hi + lo + picked + named + gate; \
           annotation(experiment(StopTime = 1, Interval = 1)); \
         end M;",
    )
    .expect("parses");

    // The comparison became what it comes to, and nothing else moved.
    let gate = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs).contains("gate"))
        .expect("gate is there");
    assert!(!format!("{:?}", gate.rhs).contains("medium"));
    assert!(format!("{:?}", gate.rhs).contains("Bool(true)"), "{gate:?}");
    assert!(!m.components.iter().any(|c| c.type_name == "String"));

    // Every other shape came through: the matrix, the comprehension,
    // the range, `end` as a subscript, the tuple and the named
    // argument are all still in the flat model.
    let all = format!("{:?}", m.equations);
    for kept in ["row[2,1]", "ramp[3]", "span[3]", "hi", "lo"] {
        assert!(all.contains(kept), "`{kept}` did not survive: {all}");
    }
}

#[test]
fn the_reserved_words_are_reserved() {
    // Chapter 2 lists them, so none of them may name anything - and
    // `der` and `initial` still have to work as the operators they are,
    // which is why they were identifiers here for so long.
    for word in [
        "class", "der", "external", "impure", "initial", "public", "pure",
    ] {
        let error = parse_model(&format!(
            "model M Real {word}; Real y; equation y = 1; \
             annotation(experiment(StopTime = 1, Interval = 1)); end M;"
        ))
        .expect_err("a reserved word cannot name a component")
        .message;
        assert!(error.contains(word), "{word}: {error}");
    }

    // Each of them in the place it belongs.
    let ok = |source: &str| assert!(parse_model(source).is_ok(), "{source}");
    ok("class M Real x; equation x = 1; \
        annotation(experiment(StopTime = 1, Interval = 1)); end M;");
    ok("model M public Real x; equation x = 1; \
        annotation(experiment(StopTime = 1, Interval = 1)); end M;");
    ok(
        "model M Real x(start = 1, fixed = true); equation der(x) = -x; \
        annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    ok(
        "model M Real x; discrete Real k(start = 0); equation x = k; \
        when initial() then k = 5; end when; \
        annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    ok("model M Real x(start = 0); equation der(x) = 1 - x; \
        initial equation der(x) = 0; \
        annotation(experiment(StopTime = 1, Interval = 1)); end M;");
    for prefix in ["pure", "impure"] {
        ok(&format!(
            "model M {prefix} function twice input Real a; output Real b; \
             algorithm b := 2 * a; end twice; Real y; equation y = twice(3); \
             annotation(experiment(StopTime = 1, Interval = 1)); end M;"
        ));
    }

    // A body written outside Modelica is read, so that a file holding
    // one alongside other classes still loads, and refused where such
    // a function is called.
    let with_external = "model M function f input Real a; output Real b; \
                         external \"C\"; end f; Real y; equation y = ";
    let tail = "; annotation(experiment(StopTime = 1, Interval = 1)); end M;";
    parse_model(&format!("{with_external}1{tail}")).expect("a file with one loads");
    let error = parse_model(&format!("{with_external}f(2){tail}"))
        .expect_err("no external bodies")
        .message;
    assert!(error.contains("outside Modelica"), "{error}");
}

#[test]
fn a_carried_profile_is_checked_before_the_run() {
    // Everything about `spatialDistribution` except the two inflows is
    // settled before a run starts, so a profile that does not describe
    // the coordinate is a mistake in the model rather than one the
    // arithmetic will find.
    let refused = |source: &str| {
        parse_model(&format!(
            "model M Real x(start = 0, fixed = true); Real a; Real b; \
             equation der(x) = 1; {source} \
             annotation(experiment(StopTime = 1, Interval = 1)); end M;"
        ))
        .expect_err("should be refused")
        .message
    };

    assert!(
        refused("(a, b) = spatialDistribution(0, 0, x, true, {0.5, 1.0}, {0.0, 0.0});")
            .contains("span 0 to 1")
    );
    assert!(refused(
        "(a, b) = spatialDistribution(0, 0, x, true, {0.0, 0.4, 0.2, 1.0}, {0.0, 0.0, 0.0, 0.0});"
    )
    .contains("must not decrease"));
    assert!(
        refused("(a, b) = spatialDistribution(0, 0, x, true, {0.0, 1.0}, {0.0, 0.0, 0.0});")
            .contains("2 initialPoints against 3 initialValues")
    );
    assert!(
        refused("(a, b) = spatialDistribution(0, 0, x, true, {0.0, 1.0});")
            .contains("but got 5 arguments")
    );
    assert!(
        refused("(a, b) = spatialDistribution(0, 0, x, true, {0.0, x}, {0.0, 0.0});")
            .contains("must be known before the run")
    );
}

/// `initial algorithm` runs once and settles where the model starts.
#[test]
fn an_initial_algorithm_says_where_the_model_starts() {
    // `count` is worked out once, at the start, from the parameters;
    // `x` integrates from it. At t = 0 the value is 3 * 2 = 6, and it
    // is an initial equation rather than one that holds throughout, so
    // the derivative is free to move it.
    let model = parse_model(
        "model M parameter Real gain = 3; Real x; Real count; \
         initial algorithm count := gain * 2; x := count; \
         equation der(x) = 1; count = gain * 2; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an initial algorithm");
    let starts: Vec<String> = model
        .initial_equations
        .iter()
        .map(|e| format!("{:?} = {:?}", e.lhs, e.rhs))
        .collect();
    assert!(
        starts.iter().any(|line| line.starts_with("Ref(\"x\")")),
        "{starts:?}"
    );
    assert!(
        starts.iter().any(|line| line.starts_with("Ref(\"count\")")),
        "{starts:?}"
    );
}

/// Names written from the top of the tree, and bodies written outside
/// Modelica - the two things a library needs to reach the operators the
/// language already has.
#[test]
fn a_name_may_be_written_from_the_top_of_the_tree() {
    // `.asin` inside a function of that very name is the operator and
    // not the function: without the dot it would call itself.
    let m = parse_model(
        "package Math function asin input Real u; output Real y; \
         algorithm y := .asin(u); end asin; end Math; \
         model M Real y; equation y = Math.asin(1); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a global name");
    // What is left is the operator, not the function that wrote it:
    // had the dot been dropped, the body would have called itself.
    let value = format!("{:?}", m.equations[0].rhs);
    assert_eq!(value, "Call(\".asin\", [Number(1.0)])");

    // A declaration may name its type the same way.
    let m = parse_model(
        "package P model Inner Real k = 3; end Inner; end P; \
         model M .P.Inner part; Real y; equation y = part.k; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a global type name");
    assert!(m.components.iter().any(|c| c.name == "part.k"));

    // `external "builtin" y = asin(u)` says the function is the
    // operator of that name, given a place in a library's tree.
    let m = parse_model(
        "package Math function acos input Real u; output Real y; \
         external \"builtin\" y = acos(u); end acos; end Math; \
         model M Real y; equation y = Math.acos(1); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a builtin body");
    assert_eq!(
        format!("{:?}", m.equations[0].rhs),
        "Call(\"acos\", [Number(1.0)])"
    );

    // A body in any other language is read, so a file holding one
    // alongside other classes still loads, and refused where it is
    // called - which is where it would have to run.
    let error = parse_model(
        "package L function f input Real u; output Real y; \
         external \"C\" y = cfun(u) annotation(Library = \"m\"); end f; end L; \
         model M Real y; equation y = L.f(1); end M;",
    )
    .expect_err("no C bodies")
    .message;
    assert!(error.contains("outside Modelica"), "{error}");

    // A constant may be built on one of another package, and on a
    // function the run never sees: both are settled here.
    let m = parse_model(
        "package Machine constant Real eps = 0.5; end Machine; \
         package Math function twice input Real u; output Real y; \
         algorithm y := 2 * u; end twice; end Math; \
         package Where constant Real k = Machine.eps; \
         constant Real doubled = Math.twice(k); end Where; \
         model M Real y; equation y = Where.doubled; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("constants reaching through others");
    assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(1.0)");
}

/// A tuple filled at an event, and one that asks for part of an array.
#[test]
fn a_call_may_fill_several_targets_at_an_event() {
    // `(a, b) = f(x)` inside `when`: the call is inlined once per
    // output and each target gets an assignment of its own.
    let m = parse_model(
        "model M function split input Real u; output Real lo; output Real hi; \
         algorithm lo := u - 1; hi := u + 1; end split; \
         discrete Real a; discrete Real b; Real y; \
         equation y = time; \
         when time > 0.5 then (a, b) = split(4); end when; \
         annotation(experiment(StopTime = 1, Interval = 0.1)); end M;",
    )
    .expect("a tuple at an event");
    let actions = format!("{:?}", m.when_clauses[0].branches[0].actions);
    assert!(
        actions.contains("Assign(\"a\", Bin(Sub, Number(4.0)"),
        "{actions}"
    );
    assert!(
        actions.contains("Assign(\"b\", Bin(Add, Number(4.0)"),
        "{actions}"
    );

    // A skipped slot costs its output nothing.
    let m = parse_model(
        "model M function split input Real u; output Real lo; output Real hi; \
         algorithm lo := u - 1; hi := u + 1; end split; \
         discrete Real b; Real y; \
         equation y = time; \
         when time > 0.5 then (, b) = split(4); end when; \
         annotation(experiment(StopTime = 1, Interval = 0.1)); end M;",
    )
    .expect("a skipped slot");
    assert_eq!(m.when_clauses[0].branches[0].actions.len(), 1);

    // Filling part of an array from a call is more than this does, and
    // it says so where such a call is made.
    let error = parse_model(
        "model M function split input Real u; output Real lo; output Real hi; \
         algorithm lo := u; hi := u; end split; \
         Real v[2]; Real y; \
         algorithm (v[1], y) := split(1); end M;",
    )
    .expect_err("part of an array")
    .message;
    assert!(error.contains("part of an array"), "{error}");
}

/// A tuple equation written inside a component, so that its targets
/// travel through every pass that carries a name.
#[test]
fn a_tuple_equation_inside_a_component_carries_its_prefix() {
    let m = parse_model(
        "function split input Real u; output Real lo; output Real hi; \
         algorithm lo := u - 1; hi := u + 1; end split; \
         model Part parameter Real k = 4; Real lo; Real hi; Real skipped; \
         equation (lo, hi) = split(k); (, skipped) = split(k + 10); end Part; \
         model M Part p; Real y; equation y = p.lo + p.hi; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a tuple inside a component");
    let written = format!("{:?}", m.equations);
    assert!(written.contains("p.lo"), "{written}");
    assert!(written.contains("p.hi"), "{written}");
    // The skipped slot took the first output and left the second.
    assert!(written.contains("p.skipped"), "{written}");
}

/// Attribute modifiers written on a component whose type is an alias,
/// and flow control that leaves a body from inside a loop.
#[test]
fn an_alias_takes_the_attributes_written_on_the_declaration() {
    // `min` and `max` written as modifiers on an aliased type mean
    // what the attribute form means, and they belong to the
    // declaration, so they outrank anything the alias says.
    let m = parse_model(
        "type Bounded = Real(min = -10, max = 10, start = 0, fixed = true); \
         model M Bounded x(min = -1, max = 1, start = 0.5, fixed = false); \
         equation der(x) = 0; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("attributes on an aliased declaration");
    let x = m.components.iter().find(|c| c.name == "x").unwrap();
    assert!(matches!(x.min, Some(Expr::Neg(_)) | Some(Expr::Number(_))));
    assert!(matches!(x.max, Some(Expr::Number(n)) if n == 1.0));
    assert!(matches!(x.start, Some(Expr::Number(n)) if n == 0.5));
    assert_eq!(x.fixed, Some(false));

    // A `return` inside a loop leaves the whole body, not the loop.
    let m = parse_model(
        "model M function upto input Real n; output Real y; \
         algorithm y := 0; \
         for i in 1:10 loop if i > n then return; end if; y := y + 1; end for; \
         end upto; \
         Real y; equation y = upto(4); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a return inside a loop");
    // Four rounds ran before the fifth left the body.
    let counted = format!("{:?}", m.equations[0].rhs);
    assert_eq!(counted.matches("Number(1.0)").count(), 4, "{counted}");

    // Subscripts of more than one dimension, assigned to and filled
    // from a call that hands back several values.
    let error = parse_model(
        "model M function pair input Real u; output Real a; output Real b; \
         algorithm a := u; b := u; end pair; \
         Real g[2, 2]; Real y; \
         algorithm g[1, 2] := 3; (g[2, 1], y) := pair(1); end M;",
    )
    .expect_err("part of an array from a call")
    .message;
    assert!(error.contains("part of an array"), "{error}");
}

/// The corners of the new forms: annotations where they may sit, and
/// what each malformed shape is refused for.
#[test]
fn the_new_forms_are_refused_by_shape() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();

    // An annotation may follow a short class definition of either kind.
    parse_model(
        "connector Signal = input Real annotation(Icon()); \
         package Alias = Other annotation(Icon()); package Other end Other; \
         model M Signal u; Real y; equation u = 1; y = u; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("annotations on short definitions");

    // And one after `initialState`: the clause is read with it, and
    // what the model is then refused for is having no clock to run a
    // machine on - which is a complaint from further in.
    assert!(err("model M model Idle Real k = 0; end Idle; Idle s; \
         equation initialState(s) annotation(Line()); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;")
    .contains("runs on a clock"));

    // `derivative` options may hold a call of their own; the whole of
    // it is read past.
    parse_model(
        "model M function fd input Real x; input Real x_der; output Real y; \
         algorithm y := x_der; end fd; \
         function f input Real x; output Real y; algorithm y := x * x; \
         annotation(derivative(noDerivative = size(x, 1)) = fd); end f; \
         Real y; equation y = f(2); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("derivative options holding a call");

    // A subscript list has to be separated by commas and closed.
    assert!(
        err("model M Real g[2]; Real y; algorithm g[1; 2] := 3; y := 1; end M;")
            .contains("in a subscript")
    );
    assert!(err(
        "model M function pair input Real u; output Real a; output Real b; \
         algorithm a := u; b := u; end pair; Real g[2]; Real y; \
         algorithm (g[1; 2], y) := pair(1); end M;"
    )
    .contains("in a subscript"));

    // A bare word in an annotation says something by being there;
    // a nested list of them is stepped over whole.
    let m = parse_model(
        "model M Real y = 1 annotation(Evaluate, Dialog(group = \"Gains\", \
         tab = {\"one\", \"two\"})); Real z; equation z = y; \
         annotation(experiment(StopTime = 1, Interval = 1), preferredView); end M;",
    )
    .expect("bare words in annotations");
    assert!(m.components.iter().any(|c| c.name == "y"));

    // A `for` statement over two indices at once is two loops, one
    // inside the other.
    let m = parse_model(
        "model M Real g[2, 2]; Real y; \
         algorithm for i, j in 1:2 loop g[i, j] := i * j; end for; y := g[2, 2]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a loop over two indices");
    assert_eq!(
        format!("{:?}", m.equations.last().unwrap().rhs),
        "Bin(Mul, Number(2.0), Number(2.0))"
    );

    // A name that is not a call at all, followed by a parenthesis.
    assert!(err("model M Real y; algorithm time(1); y := 1; end M;").contains("is not a call"));

    // A skipped slot may come first in an algorithm's tuple as well.
    let m = parse_model(
        "model M function pair input Real u; output Real a; output Real b; \
         algorithm a := u - 1; b := u + 1; end pair; \
         Real y; algorithm (, y) := pair(4); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a skipped first slot");
    assert_eq!(
        format!("{:?}", m.equations[0].rhs),
        "Bin(Add, Number(4.0), Number(1.0))"
    );
}

/// Every relational operator a record may declare, spelled the way the
/// record spells it.
#[test]
fn a_record_may_declare_every_comparison() {
    // One operator per relation, each answering with the same rule, so
    // what a comparison comes to says which one was reached.
    const V: &str = "operator record V Real x; \
         encapsulated operator function 'constructor' input Real a; output V v; \
         algorithm v.x := a; end 'constructor'; \
         encapsulated operator function '<' input V a; input V b; output Boolean r; \
         algorithm r := a.x < b.x; end '<'; \
         encapsulated operator function '<=' input V a; input V b; output Boolean r; \
         algorithm r := a.x <= b.x; end '<='; \
         encapsulated operator function '>' input V a; input V b; output Boolean r; \
         algorithm r := a.x > b.x; end '>'; \
         encapsulated operator function '>=' input V a; input V b; output Boolean r; \
         algorithm r := a.x >= b.x; end '>='; \
         encapsulated operator function '==' input V a; input V b; output Boolean r; \
         algorithm r := a.x == b.x; end '=='; \
         encapsulated operator function '<>' input V a; input V b; output Boolean r; \
         algorithm r := a.x <> b.x; end '<>'; end V; ";
    let m = parse_model(&format!(
        "{V} model M Real lt; Real le; Real gt; Real ge; Real eq; Real ne; \
         equation \
           lt = if V(1) < V(2) then 1 else 0; le = if V(2) <= V(2) then 1 else 0; \
           gt = if V(3) > V(2) then 1 else 0; ge = if V(2) >= V(2) then 1 else 0; \
           eq = if V(2) == V(2) then 1 else 0; ne = if V(1) <> V(2) then 1 else 0; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    ))
    .expect("every comparison");
    // Each comparison reached the record's own operator, which is what
    // the inlined body of it looks like: the fields compared, not the
    // records. Every one of them held.
    for (name, sign) in [
        ("lt", "Lt"),
        ("le", "Le"),
        ("gt", "Gt"),
        ("ge", "Ge"),
        ("eq", "Eq"),
        ("ne", "Ne"),
    ] {
        let equation = m
            .equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(n) if n == name))
            .unwrap_or_else(|| panic!("no equation for {name}"));
        let written = format!("{:?}", equation.rhs);
        assert!(written.contains(sign), "{name}: {written}");
    }
}

/// What an `initial algorithm` and a tuple inside `when` are refused
/// for when they ask for more than a section can give.
#[test]
fn the_initial_section_and_a_tuple_at_an_event_say_their_limits() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();

    // `break` and `return` belong to a loop and to a function; an
    // initial section is neither.
    assert!(
        err("model M Real x; initial algorithm break; equation der(x) = 1; end M;")
            .contains("`break` outside of a loop")
    );
    assert!(
        err("model M Real x; initial algorithm return; equation der(x) = 1; end M;")
            .contains("not a model algorithm")
    );

    // A tuple may not ask for more values than the call gives back.
    assert!(err(
        "model M function one input Real u; output Real a; algorithm a := u; end one; \
         discrete Real p; discrete Real q; Real y; equation y = time; \
         when time > 0.5 then (p, q) = one(1); end when; end M;"
    )
    .contains("the tuple asks for"));
}

/// A body that calls itself unrolls where what decides the recursion is
/// settled, and stands where it is not.
#[test]
fn a_recursion_unrolls_as_far_as_the_compiler_can_decide() {
    // A body that counts itself down: with the count a parameter,
    // every step of the recursion is decidable and the call comes out
    // as the answer. 4 + 3 + 2 + 1 is 10.
    let m = parse_model(
        "model M function down input Integer n; output Real y; \
         algorithm if n <= 0 then y := 0; else y := n + down(n - 1); end if; end down; \
         parameter Integer steps = 4; parameter Real total = down(steps); \
         Real y; equation y = total; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a recursion the compiler can follow");
    let total = m
        .components
        .iter()
        .find(|c| c.name == "total")
        .expect("the total");
    // Four rounds ran and the fifth ended it: the recursion is gone
    // and what is left is the sum written out, with the count kept as
    // a name so it stays tunable.
    let written = format!("{:?}", total.binding.as_ref().unwrap());
    assert_eq!(written.matches("Ref(\"steps\")").count(), 4, "{written}");
    assert!(written.ends_with("Number(0.0)))))"), "{written}");
    // Nothing had to travel: the whole recursion is gone.
    assert!(m.functions.is_empty(), "{:?}", m.functions);

    // Where the count comes from the run, the same body stands and is
    // walked - and the model carries it.
    let m = parse_model(
        "model M function down input Real n; output Real y; \
         algorithm if n <= 0 then y := 0; else y := n + down(n - 1); end if; end down; \
         Real n; Real y; equation n = 3 * time; y = down(n); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a recursion the run decides");
    assert_eq!(m.functions.len(), 1);
    assert!(
        format!("{:?}", m.equations.last().unwrap().rhs).starts_with("Call(\"M.down\""),
        "{:?}",
        m.equations.last().unwrap().rhs
    );
}

/// A call written among the equations, and a check written inside a
/// loop of them.
#[test]
fn a_call_may_stand_among_the_equations() {
    // `checkBoundary(...)` is written among the equations of every
    // fluid boundary in the standard library, and nothing takes what
    // it gives back: what it is there for is the check inside it.
    let m = parse_model(
        "model M function guard input Real u; output Real ok; \
         algorithm assert(u > 0, \"the boundary must be positive\"); ok := u; end guard; \
         parameter Real p = 2; Real y; \
         equation guard(p); y = p * time; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a call among the equations");
    assert_eq!(m.asserts.len(), 1);
    assert_eq!(m.asserts[0].1, "the boundary must be positive");
    // And it added no equation of its own.
    assert_eq!(m.equations.len(), 1);

    // A check inside a `for` equation is one per round, with the loop
    // variable folded in.
    let m = parse_model(
        "model M parameter Real k[3] = {1, 2, 3}; Real v[3]; \
         equation for i in 1:3 loop assert(k[i] > 0, \"every gain is positive\"); \
         v[i] = k[i] * time; end for; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a check inside a loop");
    assert_eq!(m.asserts.len(), 3);
    let rounds = format!("{:?}", m.asserts);
    assert!(rounds.contains("Number(3.0)"), "{rounds}");

    // A call may stand among the initial equations too, and a `for i
    // loop` may read its range off the check inside it.
    let m = parse_model(
        "model M function guard input Real u; output Real ok; \
         algorithm assert(u > -1, \"not too small\"); ok := u; end guard; \
         parameter Real k[3] = {1, 2, 3}; Real x; \
         initial equation guard(k[1]); \
         equation der(x) = 1; \
         for i loop assert(k[i] > 0, \"positive\"); end for; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a call among the initial equations");
    assert_eq!(m.asserts.len(), 4);

    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // A line that is neither an equation nor a call says so.
    assert!(err("model M Real y; equation y; end M;").contains("expected `=` in an equation"));
    // And a call that names something that is not a function.
    assert!(
        err("model M Real g; Real y; equation g(1); y = 1; end M;").contains("is not a function")
    );
    // A call among the equations of a `for` is not one this reads: the
    // loop unrolls into equations, and a call is not one.
    assert!(err(
        "model M function guard input Real u; output Real ok; algorithm ok := u; end guard; \
         parameter Real k[2] = {1, 2}; Real y; \
         equation for i in 1:2 loop guard(k[i]); end for; y = 1; end M;"
    )
    .contains("stands on its own where an equation is wanted"));
}

/// Parameters settled by what stands around them: a function, a length
/// measured a declaration earlier, and a name from the class above.
#[test]
fn a_parameter_is_settled_by_whatever_can_settle_it() {
    // A parameter worked out by a function - the standard library
    // counts the base systems of an m-phase winding that way.
    let m = parse_model(
        "model M function halves input Integer k; output Integer n; \
         algorithm n := integer(k / 2); end halves; \
         parameter Integer phases = 6; parameter Integer systems = halves(phases); \
         Real v[systems]; Real y; equation v = fill(time, systems); y = v[3]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a parameter off a function");
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("v["))
            .count(),
        3
    );

    // A length measured a declaration earlier, kept as the number:
    // nothing after flattening knows how to measure an array.
    let m = parse_model(
        "model Lines input Real lines[:, 2] = zeros(0, 2); \
         parameter Integer n = size(lines, 1); Real got[n]; \
         equation got = {lines[i, 1] for i in 1:n}; end Lines; \
         model M Lines drawn(lines = {{1, 2}, {3, 4}, {5, 6}}); Real y; \
         equation y = drawn.got[2]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a length off an earlier declaration");
    let n = m.components.iter().find(|c| c.name == "drawn.n").unwrap();
    assert!(matches!(n.binding, Some(Expr::Number(v)) if v == 3.0));
    assert_eq!(
        format!(
            "{:?}",
            m.equations
                .iter()
                .find(|e| matches!(&e.lhs, Expr::Ref(n) if n == "drawn.got[2]"))
                .unwrap()
                .rhs
        ),
        "Ref(\"drawn.lines[2,1]\")"
    );

    // And a modifier handed down carries names of the class that wrote
    // it: the child has to know what `drawn.n` is, and `drawn` is not
    // below it but above.
    let m = parse_model(
        "model Cell parameter Real gain = 0; Real v; \
         equation v = gain * time; end Cell; \
         model Holder parameter Integer n = 2; \
         Cell cells[2](gain = {i for i in 1:n}); end Holder; \
         model M Holder h; Real y; equation y = h.cells[2].v; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a modifier written above");
    let second = m
        .components
        .iter()
        .find(|c| c.name == "h.cells[2].gain")
        .unwrap();
    assert!(matches!(second.binding, Some(Expr::Number(v)) if v == 2.0));
}

/// A function asked how long what it was handed is.
#[test]
fn a_function_measures_what_it_was_handed_by_either_name() {
    // `quasiRMS(i)` where `i[m]` comes from a base class: the length
    // is the caller's to give, and the body reads it by the caller's
    // name once the argument is substituted in.
    let m = parse_model(
        "package P \
           function meanOf input Real x[:]; output Real y; \
           algorithm y := sum(x) / size(x, 1); end meanOf; \
           partial model TwoPlug parameter Integer m = 3; Real i[m]; end TwoPlug; \
           model Stray extends TwoPlug; Real iMean = meanOf(i); \
           equation i = fill(time, m); end Stray; \
         end P; \
         model M P.Stray s; Real y; equation y = s.iMean; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a length the caller gives");
    let mean = m
        .equations
        .iter()
        .find(|e| matches!(&e.lhs, Expr::Ref(n) if n == "s.iMean"))
        .expect("the mean");
    let written = format!("{:?}", mean.rhs);
    assert!(written.contains("s.i[3]"), "{written}");
    assert!(written.contains("Number(3.0)"), "{written}");
}

/// A call written for what it does outside the model rather than for
/// what it gives back.
#[test]
fn a_call_that_only_acts_outside_the_model_does_nothing_here() {
    // `Streams.print(...)` writes a line on a terminal. There is none
    // here and no value to miss, so the call does nothing and the run
    // is the same run.
    let m = parse_model(
        "model M impure function print input String s; \
           external \"C\" ModelicaInternal_print(s); end print; \
         Real y; equation print(\"working\"); y = time; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a call that only acts outside");
    assert_eq!(m.equations.len(), 1);
    assert!(m.asserts.is_empty());

    // A body written outside that does answer is another matter: its
    // value is wanted, and there is nothing here to produce it.
    let error = parse_model(
        "model M function measure input Real u; output Real v; \
           external \"C\" v = measure_c(u); end measure; \
         Real y; equation y = measure(1); end M;",
    )
    .expect_err("no value to be had")
    .message;
    assert!(error.contains("outside Modelica"), "{error}");
}

/// A branch the condition does not take need not be buildable.
#[test]
fn the_branch_not_taken_need_not_be_built() {
    // `if tableOnFile then length(fileName) else 0` with `tableOnFile`
    // false: the length has a body written in C, and it is not asked.
    let m = parse_model(
        "model M function outside input String s; output Integer n; \
         external \"C\" n = strlen(s); end outside; \
         parameter Boolean fromFile = false; \
         parameter Integer width = if fromFile then outside(\"abc\") else 7; \
         Real y; equation y = width * time; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a branch nobody takes");
    let width = m
        .components
        .iter()
        .find(|c| c.name == "width")
        .expect("width");
    assert_eq!(format!("{:?}", width.binding), "Some(Number(7.0))");

    // Turn the condition round and the trouble is said plainly.
    let error = parse_model(
        "model M function outside input String s; output Integer n; \
         external \"C\" n = strlen(s); end outside; \
         parameter Boolean fromFile = true; \
         parameter Integer width = if fromFile then outside(\"abc\") else 7; \
         Real y; equation y = width * time; end M;",
    )
    .expect_err("the branch that is taken")
    .message;
    assert!(error.contains("written outside Modelica"), "{error}");
}

/// A function that answers with nothing is called for what it checks.
#[test]
fn a_function_with_no_output_is_called_for_its_checks() {
    let m = parse_model(
        "model M parameter Real tab[:] = {1, 2, 3}; \
         function isValid input Real table[:]; protected Integer n = size(table, 1); \
         algorithm if n > 0 then for i in 2:n loop \
           assert(table[i] > table[i - 1], \"not increasing\"); end for; end if; \
         end isValid; \
         Real y; initial algorithm isValid(tab); \
         equation y = time; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a function that only checks");
    // Two neighbouring pairs, so two checks, and each names elements
    // of the list it was handed.
    assert_eq!(m.asserts.len(), 2);
    assert!(format!("{:?}", m.asserts).contains("tab[2]"));

    // Asked for a value, such a function is still refused.
    let error = parse_model(
        "model M function nothing input Real u; algorithm assert(u > 0, \"positive\"); \
         end nothing; Real y; equation y = nothing(1); end M;",
    )
    .expect_err("no value to be had")
    .message;
    assert!(error.contains("declares no output"), "{error}");
}

/// A branch of an `if` equation may hold a `when` and a bare call.
#[test]
fn a_branch_may_hold_an_event_and_a_call() {
    let m = parse_model(
        "model M function check input Real u; algorithm assert(u > 0, \"positive\"); \
         end check; \
         parameter Boolean resettable = true; parameter Real level = 2; \
         Boolean reset; Real y(start = 0, fixed = true); \
         equation reset = time > 0.5; der(y) = 1; \
         if resettable then \
           check(level); \
           when reset then reinit(y, 0); end when; \
         end if; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an event and a call in a branch the compiler picks");
    assert_eq!(m.when_clauses.len(), 1);
    assert_eq!(m.asserts.len(), 1);
    assert_eq!(m.asserts[0].1, "positive");

    // The branch nobody takes contributes neither.
    let m = parse_model(
        "model M function check input Real u; algorithm assert(u > 0, \"positive\"); \
         end check; \
         parameter Boolean resettable = false; parameter Real level = -1; \
         Boolean reset; Real y(start = 0, fixed = true); \
         equation reset = time > 0.5; der(y) = 1; \
         if resettable then \
           check(level); \
           when reset then reinit(y, 0); end when; \
         end if; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a branch nobody takes");
    assert!(m.when_clauses.is_empty());
    assert!(m.asserts.is_empty());

    // Neither may sit behind a condition only the run decides.
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    assert!(
        err("model M Boolean high; Real y(start = 0, fixed = true); \
         equation high = time > 0.5; der(y) = 1; \
         if high then when time > 0.7 then reinit(y, 0); end when; \
         else der(y) = 1; end if; end M;")
        .contains("what happens at an event")
    );
    assert!(err(
        "model M function check input Real u; algorithm assert(u > 0, \"positive\"); \
         end check; Boolean high; Real y; \
         equation high = time > 0.5; y = time; \
         if high then check(1); else check(2); end if; end M;"
    )
    .contains("a call standing on its own"));
}

/// What the media library is written with: a member read off a
/// subscripted component, a described import, a function handed as an
/// argument, an attribute worked out rather than written, and a member
/// that belongs to a base of the package naming it.
#[test]
fn the_forms_the_media_library_is_written_in() {
    // `medium[i].Xi[1]` - a member of one of several, subscripted.
    let m = parse_model(
        "model M record Mix Real Xi[2]; end Mix; Mix medium[2]; Real y; \
         equation for i in 1:2 loop \
           medium[i].Xi[1] = i * time; medium[i].Xi[2] = 0; end for; \
         y = medium[2].Xi[1]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a member of one of several, subscripted");
    assert!(format!("{:?}", m.equations).contains("medium[2].Xi[1]"));

    // An import may say what it is for.
    let m = parse_model(
        "package P constant Real g = 9.81; end P; \
         model M import Gravity = P \"where the constants live\"; \
         Real y; equation y = Gravity.g * time; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an import with a description");
    assert!(format!("{:?}", m.equations[0].rhs).contains("9.81"));

    // A type attribute may be worked out rather than written.
    let m = parse_model(
        "package P constant String name = \"water\"; \
         type Flow = Real(quantity = \"MassFlowRate.\" + name, min = -1e5); end P; \
         model M P.Flow w; Real y; equation w = time; y = w; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an attribute built out of a name");
    let w = m.components.iter().find(|c| c.name == "w").expect("w");
    assert_eq!(format!("{:?}", w.min), "Some(Neg(Number(100000.0)))");

    // A package's member may belong to a base of it.
    let m = parse_model(
        "package P package Base model Props Real T; Real h; equation h = 2 * T; end Props; \
         end Base; package Water extends P.Base; end Water; end P; \
         model M P.Water.Props medium; Real y; \
         equation medium.T = time; y = medium.h; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a member written in a base of the package");
    assert!(format!("{:?}", m.equations).contains("medium.h"));

    // A function handed as an argument is read, and refused where it
    // is used: there is nothing here to pass a function around in.
    let error = parse_model(
        "model M function f input Real u; input Real a; output Real y; \
         algorithm y := a * u; end f; \
         function solve input Real g; output Real x; algorithm x := g; end solve; \
         Real y; equation y = solve(function f(a = 2)); end M;",
    )
    .expect_err("a function passed around")
    .message;
    assert!(error.contains("pass a function around"), "{error}");
}

/// A body written outside Modelica says what it is: the name called,
/// what it is handed, and in what language.
#[test]
fn an_outside_body_names_itself() {
    let error = parse_model(
        "model M function sort input Real u[3]; output Real y[3]; \
         external \"C\" ModelicaSpecial_sort(u, y); end sort; \
         Real y[3]; equation y = sort({3, 1, 2}); end M;",
    )
    .expect_err("a body written outside")
    .message;
    assert!(error.contains("ModelicaSpecial_sort(u, y)"), "{error}");
    assert!(error.contains("in C"), "{error}");
    assert!(error.contains("no outside library was given"), "{error}");

    // A name this compiler answers for itself is not refused: the call
    // is left standing as that name, and the string layer works it out.
    let answered = parse_model(
        "model M function length input String s; output Integer n; \
         external \"C\" n = ModelicaStrings_length(s); end length; \
         Real y; equation y = length(\"abc\"); end M;",
    )
    .expect("a name answered here");
    assert!(matches!(
        answered.equations.first().map(|e| &e.rhs),
        Some(Expr::Number(three)) if *three == 3.0
    ));

    // A declaration that names no language still names the call.
    let error = parse_model(
        "model M function twice input Real u; output Real y; \
         external twice_it(u, 2); end twice; \
         Real y; equation y = twice(1); end M;",
    )
    .expect_err("a body written outside with no language")
    .message;
    assert!(error.contains("twice_it(u, Number(2.0))"), "{error}");

    // `external \"builtin\"` is the language's own operator given a
    // place in a library's tree, and is answered rather than refused.
    let m = parse_model(
        "model M package Math function arcsin input Real u; output Real y; \
         external \"builtin\" y = asin(u); end arcsin; end Math; \
         Real y; equation y = Math.arcsin(1); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an operator the language already has");
    assert!(format!("{:?}", m.equations[0].rhs).contains("asin"));
}

#[test]
fn the_string_bodies_written_outside_are_answered_here() {
    // `Modelica.Electrical.Machines` picks a transformer's ratio from
    // the letters of its vector group: `substring(VectorGroup, 1, 1)`
    // held in a constant and compared further down. Both the cut and
    // the comparison are settled before the run.
    const CUT: &str = "function substring input String s; input Integer i1; \
         input Integer i2; output String r; \
         external \"C\" r = ModelicaStrings_substring(s, i1, i2); end substring; \
         function length input String s; output Integer n; \
         external \"C\" n = ModelicaStrings_length(s); end length; \
         function compare input String a; input String b; input Boolean cased; \
         output Integer r; \
         external \"C\" r = ModelicaStrings_compare(a, b, cased); end compare; \
         function skip input String s; input Integer from; output Integer next; \
         external \"C\" next = ModelicaStrings_skipWhiteSpace(s, from); end skip; ";
    let m = parse_model(&format!(
        "model M {CUT} constant String group = \"Dyn\"; \
         constant String first = substring(group, 1, 1); \
         Real ratio; Real n; Real same; \
         equation ratio = if first == \"D\" then 1 else 3; \
         n = length(group); same = compare(\"a\", \"A\", false); end M;"
    ))
    .unwrap();
    let value = |name: &str| {
        m.equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(said) if said == name))
            .map(|e| format!("{:?}", e.rhs))
    };
    // The comparison came to a truth before the run; what the branch
    // makes of it is the solver's business.
    assert_eq!(
        value("ratio").as_deref(),
        Some("If(Bool(true), Number(1.0), Number(3.0))")
    );
    assert_eq!(value("n").as_deref(), Some("Number(3.0)"));
    // Equal, told to ignore case: `Types.Compare.Equal` is 2.
    assert_eq!(value("same").as_deref(), Some("Number(2.0)"));

    // The edges the specification sets out: a start below one is one,
    // an end past the text is its end, and an end before the start is
    // nothing at all.
    let edges = parse_model(&format!(
        "model M {CUT} constant String s = \"abc\"; \
         Real a; Real b; Real c; \
         equation a = if substring(s, -2, 2) == \"ab\" then 1 else 0; \
         b = if substring(s, 2, 9) == \"bc\" then 1 else 0; \
         c = if substring(s, 3, 1) == \"\" then 1 else 0; end M;"
    ))
    .unwrap();
    assert!(
        edges
            .equations
            .iter()
            .all(|e| format!("{:?}", e.rhs).starts_with("If(Bool(true)")),
        "{:?}",
        edges.equations
    );

    // Where the white space after a position ends, counted from one:
    // `Strings.isEmpty` is written on this, and a name that is nothing
    // but spaces is empty.
    let skipped = parse_model(&format!(
        "model M {CUT} constant String pad = \"  a\"; \
         Real first; Real none; \
         equation first = skip(pad, 1); none = skip(\"abc\", 1); end M;"
    ))
    .unwrap();
    let told = format!("{:?}", skipped.equations);
    assert!(
        told.contains("Number(3.0)") && told.contains("Number(1.0)"),
        "{told}"
    );

    // Case that matters, and the two ways round.
    let ordered = parse_model(&format!(
        "model M {CUT} Real less; Real more; \
         equation less = compare(\"a\", \"b\", true); \
         more = compare(\"b\", \"a\", true); end M;"
    ))
    .unwrap();
    let said = format!("{:?}", ordered.equations);
    assert!(
        said.contains("Number(1.0)") && said.contains("Number(3.0)"),
        "{said}"
    );
}

#[test]
fn a_derivative_is_handed_a_rate_only_for_what_has_one() {
    // The standard library asks a table for a value by `(tableID,
    // column, u)` and declares the derivative of that as a function of
    // four: the three it was given, and `der_u` alone. Neither a handle
    // nor a column number has a rate of change.
    let m = parse_model(
        "model M function look input Integer column; input Real u; output Real y; \
         algorithm y := column * u * u; annotation(derivative = dlook); end look; \
         function dlook input Integer column; input Real u; input Real der_u; \
         output Real der_y; algorithm der_y := 2 * column * u * der_u; end dlook; \
         Real x; Real v; equation x = look(2, time); v = der(x); end M;",
    )
    .unwrap();
    let said = format!("{:?}", m.equations);
    assert!(said.contains("Number(2.0)"), "{said}");

    // A derivative that wants a rate for what has none is still wrong,
    // and says how many it should take.
    let error = parse_model(
        "model M function look input Integer column; input Real u; output Real y; \
         algorithm y := column * u; annotation(derivative = dlook); end look; \
         function dlook input Integer column; input Real u; input Real dc; \
         input Real du; output Real dy; algorithm dy := du; end dlook; \
         Real x; Real v; equation x = look(2, time); v = der(x); end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("so it takes 3 inputs"), "{error}");
}

/// A body remembered for one caller answers another only where the
/// values it folds with are worth the same.
#[test]
fn a_remembered_body_is_told_apart_by_what_it_folds_with() {
    // Two instances of the same class, each with as many values in
    // view as the other and a different value among them. The answers
    // are remembered under a key that once counted the values rather
    // than reading them, which made these one question.
    let m = parse_model(
        "package P function grow input Integer n; output Real out[n]; \
         algorithm for i in 1:n loop out[i] := n; end for; end grow; \
         model Inner parameter Integer n = 3; \
         parameter Real got[:] = grow(n); Real y = got[1]; end Inner; \
         model M Inner a(n = 3); Inner b(n = 4); Real out; \
         equation out = a.y * 100 + b.y; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M; end P;",
    )
    .expect("two instances of one class");
    // Each instance is as long as its own `n` says, and says its own
    // number: three threes and four fours.
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("a.got["))
            .count(),
        3
    );
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("b.got["))
            .count(),
        4
    );
}

#[test]
fn a_body_written_here_answers_with_as_many_numbers_as_it_declares() {
    // Every seed settled: the ten-line algorithm runs while the model
    // is being built, and what is left is two numbers.
    let m = parse_model(&format!(
        "{GENERATOR} model M parameter Integer s[2] = Gen.initialState(7, 3); \
         Real y; equation y = s[1] * time; end M;"
    ))
    .unwrap();
    let value = |name: &str| {
        m.components
            .iter()
            .find(|c| c.name == name)
            .and_then(|c| c.binding.clone())
    };
    // Three rounds, each the outside call asked for its own place of
    // the answer: the first for the value it drew, the second and
    // third for the halves of the state it moved to.
    let written = format!("{:?}", value("s[1]"));
    assert_eq!(written.matches("ModelicaRandom_xorshift64star").count(), 7);
    assert!(written.contains("Number(7.0), Number(3.0)"), "{written}");
    assert!(written.ends_with("[Number(2.0)]))"), "{written}");
    let second = format!("{:?}", value("s[2]"));
    assert!(second.ends_with("[Number(3.0)]))"), "{second}");

    // A run of elements assigned at once: `state[1:2] := aux` is what
    // the standard library fills a longer state with.
    let sliced = parse_model(&format!(
        "{GENERATOR} model M parameter Integer s[2] = Gen.withN(7, 3, 2); \
         Real y; equation y = s[2] * time; end M;"
    ))
    .unwrap();
    assert_eq!(
        format!(
            "{:?}",
            sliced
                .components
                .iter()
                .find(|c| c.name == "s[2]")
                .and_then(|c| c.binding.clone())
        ),
        second
    );

    // A run given the wrong number of values is said, not guessed.
    let mismatched = parse_model(&format!(
        "{GENERATOR} model M function odd output Integer s[3]; \
         protected Integer aux[2] = {{1, 2}}; algorithm s[1:3] := aux; end odd; \
         parameter Integer s[3] = odd(); Real y; equation y = s[1] * time; end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(
        mismatched.contains("run of 3 element(s) and 2 value(s)"),
        "{mismatched}"
    );
}

#[test]
fn a_body_written_here_is_held_to_what_it_declares() {
    // A run of elements whose bounds only the run would know.
    let loose = parse_model(
        "model M function fill3 input Real u; output Integer s[3]; \
         protected Integer aux[2] = {1, 2}; Integer n; \
         algorithm n := integer(u); s[1:n] := aux; end fill3; \
         Integer v[3]; Real y; equation v = fill3(time); y = v[1] * time; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(loose.contains("bounds this compiler cannot see"), "{loose}");

    // A run given one value rather than a list of them.
    let one = parse_model(
        "model M function fill2 output Integer s[2]; algorithm s[1:2] := 7; end fill2; \
         parameter Integer s[2] = fill2(); Real y; equation y = s[1] * time; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(one.contains("run of 2 element(s) and 1 value(s)"), "{one}");

    // An answer whose length nothing says.
    let lengthless = parse_model(&format!(
        "{GENERATOR} model M function odd input Integer a[2]; output Real v; \
         output Integer w[:]; \
         external \"C\" ModelicaRandom_xorshift64star(a, w, v); end odd; \
         Real y; equation y = odd({{1, 2}}) * time; end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(
        lengthless.contains("whose length this compiler cannot see"),
        "{lengthless}"
    );

    // An answer of more than one dimension: a body written here gives
    // one flat list, and nothing says how to fold it into a matrix.
    let shapeless = parse_model(&format!(
        "{GENERATOR} model M function odd input Integer a[2]; output Real v; \
         output Integer w[2, 2]; \
         external \"C\" ModelicaRandom_xorshift64star(a, w, v); end odd; \
         Real y; equation y = odd({{1, 2}}) * time; end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(
        shapeless.contains("whose shape this compiler cannot see"),
        "{shapeless}"
    );

    // An answer of the wrong number of numbers: the body written here
    // gives three, and this declaration asks for four.
    let mismatched = parse_model(&format!(
        "{GENERATOR} model M function odd input Integer a[2]; output Real v; \
         output Integer w[3]; \
         external \"C\" ModelicaRandom_xorshift64star(a, w, v); end odd; \
         Real y; equation y = odd({{1, 2}}) * time; end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(
        mismatched.contains("answers with 4 number(s), and the body written here answers with 3"),
        "{mismatched}"
    );
}

#[test]
fn a_body_reads_its_own_locals_before_they_leave_it() {
    // `Integer m = size(x, 1)` of a space-phasor transform is written
    // in terms of an input. Left to be read where it is used, it would
    // carry `x` out of the body, and out there the name means nothing.
    let m = parse_model(
        "function toPhasor input Real x[:]; output Real y[2]; \
         protected Integer n = size(x, 1); Real gain[2, 1] = fill(2.0 / n, 2, 1); \
         algorithm y := gain * {sum(x)}; end toPhasor; \
         model M Real v[3]; Real p[2]; \
         equation v = {time, 2 * time, 3 * time}; p = toPhasor(v); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .unwrap();
    // Three phases, so the gain is two thirds, and the sum is six
    // times the clock.
    let said = format!("{:?}", m.equations);
    assert!(!said.contains("size"), "{said}");
    assert!(said.contains("Div, Number(2.0), Number(3.0)"), "{said}");

    // A local that comes to a number through a call is stored as the
    // number, so an `if` written on it is one arithmetic can decide.
    let counted = parse_model(
        "function halves input Integer m; output Integer n; \
         algorithm n := if m > 3 then 2 else 1; end halves; \
         function pick input Real x[:]; output Real y; \
         protected Integer m = size(x, 1); Integer k = halves(m); \
         algorithm if k == 2 then y := x[1]; else y := x[2]; end if; end pick; \
         model M Real v[4]; Real p; \
         equation v = {time, 2, 3, 4}; p = pick(v); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .unwrap();
    // Four phases, so `halves` gives two and the first element is
    // taken - the clock itself.
    assert!(
        counted
            .equations
            .iter()
            .any(|e| matches!(&e.lhs, Expr::Ref(n) if n == "p")
                && matches!(&e.rhs, Expr::Ref(taken) if taken == "v[1]")),
        "{:?}",
        counted.equations
    );
}

#[test]
fn a_run_of_elements_may_be_named_along_more_than_one_axis() {
    // `oM[1:mBase, 1:mBase] := {o[1:mBase], -o[1:mBase]}` is how the
    // polyphase library fills a transformation matrix: a run of names
    // along two axes at once, and that many values to fill them.
    let m = parse_model(
        "function corner output Real a[3, 3]; \
         algorithm a := zeros(3, 3); a[1:2, 1:2] := {{1, 2}, {3, 4}}; \
         a[3, 2:3] := {5, 6}; end corner; \
         model M parameter Real b[3, 3] = corner(); Real y; \
         equation y = b[1, 2] * time; end M;",
    )
    .unwrap();
    let worth = |name: &str| {
        m.components
            .iter()
            .find(|c| c.name == name)
            .and_then(|c| c.binding.clone())
    };
    // The last subscript moves fastest, the way an array is written
    // out: 1, 2 fill the first row and 3, 4 the second.
    assert_eq!(worth("b[1,1]"), Some(Expr::Number(1.0)));
    assert_eq!(worth("b[1,2]"), Some(Expr::Number(2.0)));
    assert_eq!(worth("b[2,1]"), Some(Expr::Number(3.0)));
    assert_eq!(worth("b[2,2]"), Some(Expr::Number(4.0)));
    // A plain subscript beside a run names one row of it.
    assert_eq!(worth("b[3,2]"), Some(Expr::Number(5.0)));
    assert_eq!(worth("b[3,3]"), Some(Expr::Number(6.0)));
    // What the run did not name kept what it had.
    assert_eq!(worth("b[3,1]"), Some(Expr::Number(0.0)));

    // A run given the wrong number of values is said, not guessed.
    let wrong = parse_model(
        "function odd output Real a[2, 2]; \
         algorithm a := zeros(2, 2); a[1:2, 1:2] := {{1, 2}}; end odd; \
         model M parameter Real b[2, 2] = odd(); Real y; \
         equation y = b[1, 1] * time; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(
        wrong.contains("run of 4 element(s) and 2 value(s)"),
        "{wrong}"
    );
    // A plain subscript beside a run has to come to a number too.
    let loose = parse_model(
        "function odd input Real u; output Real a[2, 2]; \
         protected Integer k; \
         algorithm a := zeros(2, 2); k := integer(u); a[k, 1:2] := {1, 2}; end odd; \
         model M Real b[2, 2]; Real y; \
         equation b = odd(time); y = b[1, 1] * time; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(loose.contains("must be a whole number"), "{loose}");

    // A name bound whole and then written into element by element,
    // where nothing says what shape the whole is: the write says what
    // it can and what it does not name is left alone.
    let shapeless = parse_model(
        "function odd input Real u[:]; output Real y; \
         protected Real a[size(u, 1)] = u; \
         algorithm a[1] := 5; y := a[1] + a[2]; end odd; \
         model M Real p; equation p = odd({time, 2}); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert!(shapeless.is_ok(), "{shapeless:?}");
}

#[test]
fn a_when_of_an_algorithm_holds_an_algorithm() {
    // `Modelica.Blocks.Math.RealFFT` counts ticks at an event and
    // branches on the count: the body of a `when` is an algorithm like
    // any other, and running it is what says which names it leaves
    // changed and what each of them is worth.
    let m = parse_model(
        "model M Real u; discrete Integer n(start = 0, fixed = true); \
         discrete Real held(start = 0, fixed = true); \
         equation u = time; \
         algorithm when u > 0.5 then n := pre(n) + 1; \
         if n <= 2 then held := u; else held := 0; end if; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;",
    )
    .unwrap();
    let actions = &m.when_clauses[0].branches[0].actions;
    // Two names changed, and the second is a choice made at the event
    // rather than two writes.
    assert_eq!(actions.len(), 2);
    let said = format!("{actions:?}");
    assert!(
        said.contains("Assign(\"n\"") && said.contains("Assign(\"held\""),
        "{said}"
    );
    assert!(said.contains("If("), "{said}");

    // A check written inside is a check of the model.
    let checked = parse_model(
        "model M Real u; discrete Real c(start = 0, fixed = true); \
         equation u = time; \
         algorithm when u > 0.5 then assert(u > 0, \"positive\"); c := 1; end when; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;",
    )
    .unwrap();
    assert_eq!(checked.asserts.len(), 1);
}

#[test]
fn a_when_of_an_algorithm_is_no_place_to_leave_from() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // `break` belongs to a loop and `return` to a function; an event
    // is neither, and there is nowhere for either to go.
    assert!(err(
        "model M Real u; discrete Real c(start = 0); equation u = time; \
             algorithm when u > 1 then c := 1; break; end when; end M;"
    )
    .contains("`break` outside of a loop"),);
    assert!(err(
        "model M Real u; discrete Real c(start = 0); equation u = time; \
             algorithm when u > 1 then c := 1; return; end when; end M;"
    )
    .contains("`return` belongs in a function"),);
}

#[test]
fn a_tuple_may_stand_inside_a_branch_the_compiler_settles() {
    // `Modelica.Blocks.Continuous.Filter` picks its coefficients by
    // kind: `if filterType == LowPass then (r, a, b, ku) = lowPass(...)`.
    // A tuple reads the same way inside a branch as at the top of a
    // class - one call filling several targets.
    let m = parse_model(
        "model M function split input Real u; output Real a; output Real b; \
         algorithm a := u; b := 2 * u; end split; \
         parameter Boolean low = true; Real p; Real q; \
         equation if low then (p, q) = split(time); else p = 0; q = 0; end if; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .unwrap();
    let said = |name: &str| {
        m.equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == name))
            .map(|e| format!("{:?}", e.rhs))
            .unwrap_or_default()
    };
    assert_eq!(said("p"), "Time");
    assert_eq!(said("q"), "Bin(Mul, Number(2.0), Time)");

    // A slot left out costs its output nothing, in a branch as
    // anywhere else.
    let skipped = parse_model(
        "model M function split input Real u; output Real a; output Real b; \
         algorithm a := u; b := 2 * u; end split; \
         parameter Boolean low = true; Real q; \
         equation if low then (, q) = split(time); else q = 0; end if; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .unwrap();
    assert_eq!(skipped.equations.len(), 1);

    // The right side of a tuple is a call, wherever the tuple stands.
    let odd = parse_model(
        "model M parameter Boolean low = true; Real p; Real q; \
         equation if low then (p, q) = time; else p = 0; q = 0; end if; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(odd.contains("must be a function call"), "{odd}");
}

#[test]
fn an_answer_of_more_than_one_dimension_is_an_array_of_arrays() {
    // `Modelica.Blocks.Continuous.Filter` builds `den2[:, 2]` element
    // by element and hands it on, and whoever was handed it reads
    // `den2[i, 2]`. Gathered flat, the second subscript would have
    // nowhere to go.
    let m = parse_model(
        "model M function pairs input Integer n; output Real d[n, 2]; \
         algorithm for i in 1:n loop d[i, 1] := i; d[i, 2] := 2 * i; end for; end pairs; \
         function second input Real d[:, 2]; input Integer k; output Real y; \
         algorithm y := d[k, 2]; end second; \
         Real y; equation y = second(pairs(3), 2) * time; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .unwrap();
    // The second row's second column is two times two.
    assert!(
        format!("{:?}", m.equations).contains("Mul, Number(2.0), Number(2.0)"),
        "{:?}",
        m.equations
    );

    // Read as a whole, it is still the shape it was declared.
    let whole = parse_model(
        "model M function pairs input Integer n; output Real d[n, 2]; \
         algorithm for i in 1:n loop d[i, 1] := i; d[i, 2] := 2 * i; end for; end pairs; \
         parameter Real g[2, 2] = pairs(2); Real y; equation y = g[2, 1] * time; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .unwrap();
    assert_eq!(
        whole
            .components
            .iter()
            .find(|c| c.name == "g[2,2]")
            .and_then(|c| c.binding.clone()),
        Some(Expr::Bin(
            BinOp::Mul,
            Box::new(Expr::Number(2.0)),
            Box::new(Expr::Number(2.0))
        ))
    );

    // An element the body never assigned is said by name.
    let missing = parse_model(
        "model M function pairs output Real d[2, 2]; \
         algorithm d[1, 1] := 1; end pairs; \
         parameter Real g[2, 2] = pairs(); Real y; equation y = g[1, 1] * time; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(missing.contains("never assigns `d[1,2]`"), "{missing}");
}

/// The reach is looked for in every place a class writes a value:
/// inside a loop, inside a branch, inside a `when`, inside an
/// algorithm, and in what a declaration says about itself.
#[test]
fn a_protected_declaration_is_kept_back_wherever_the_reach_is_written() {
    const INNER: &str = "model Inner protected parameter Real hidden = 2; \
                         public Real y; equation y = hidden * time; end Inner; \
                         function two input Real x; output Real p; output Real q; \
                         algorithm p := x; q := 2 * x; end two; ";
    const HOLDS: &str = "Inner a; parameter Integer n = 2; Real z[2]; Boolean fired; ";

    for body in [
        // In a loop, and in a branch inside one.
        "equation for i in 1:2 loop z[i] = a.hidden; end for;",
        "equation for i in 1:2 loop if i > 1 then z[i] = a.hidden; else z[i] = 0; end if; end for;",
        // In a branch, in what a branch asserts, and in what it calls.
        "equation if n > 1 then z = {a.hidden, 0}; else z = {0, 0}; end if;",
        // In what a `when` fires on and in what it does.
        "equation z = {0, 0}; algorithm when time > a.hidden then fired := true; end when;",
        "equation z = {0, 0}; algorithm when time > 1 then fired := time > a.hidden; end when;",
        // In an algorithm, in a loop inside one, and in a `while`.
        "algorithm z[1] := a.hidden; z[2] := 0;",
        "algorithm for i in 1:2 loop z[i] := a.hidden; end for;",
        "algorithm z := {0, 0}; while time < a.hidden loop break; end while;",
        // In what a loop asserts, in a loop inside a loop, and in what
        // a branch asserts or fires on.
        "equation z = {0, 0}; for i in 1:2 loop assert(a.hidden > 0, \"positive\"); end for;",
        "equation for i in 1:2 loop for j in 1:1 loop z[i] = a.hidden * j; end for; end for;",
        "equation z = {0, 0}; if n > 1 then assert(a.hidden > 0, \"positive\"); end if;",
        "equation z = {0, 0}; if n > 1 then when time > a.hidden then fired = true; end when; end if;",
        "equation z = {0, 0}; if n > 1 then for i in 1:2 loop assert(a.hidden > i, \"above\"); end for; end if;",
        // In a loop and in a branch inside a `when`.
        "equation z = {0, 0}; when time > 1 then for i in 1:2 loop assert(a.hidden > i, \"above\"); end for; end when;",
        "equation z = {0, 0}; when time > 1 then if n > 1 then fired = a.hidden > 0; else fired = false; end if; end when;",
        // In what several outputs at once are taken from, and in a
        // call nothing takes the outputs of.
        "algorithm (z[1], z[2]) := two(a.hidden);",
        "algorithm z := {0, 0}; two(a.hidden);",
        "algorithm z := {0, 0}; assert(a.hidden > 0, \"positive\");",
        // In what the class asserts and in what it calls outright.
        "equation z = {0, 0}; assert(a.hidden > 0, \"positive\");",
        // In what a declaration says about itself.
        "Real w(start = a.hidden, min = -a.hidden, max = a.hidden); equation z = {0, 0}; w = 0;",
    ] {
        let source = format!("{INNER} model M {HOLDS} {body} end M;");
        let refusal = parse_model(&source).unwrap_err().to_string();
        assert!(refusal.contains("a.hidden"), "{body}: {refusal}");
    }

    // The same places with the public member instead, so that what is
    // refused above is the reach and not the shape of the model.
    for body in [
        "equation for i in 1:2 loop z[i] = a.y; end for;",
        "algorithm z[1] := a.y; z[2] := 0;",
        "equation z = {0, 0}; assert(a.y > -1, \"above\");",
    ] {
        parse_model(&format!("{INNER} model M {HOLDS} {body} end M;")).unwrap();
    }
}

/// A class a package keeps to itself is named inside it and nowhere
/// else.
#[test]
fn a_protected_class_is_not_named_from_outside() {
    const PACKAGE: &str = "package P \
                           function open input Real x; output Real y; algorithm y := 2 * x; end open; \
                           protected \
                           function shut input Real x; output Real y; algorithm y := 3 * x; end shut; \
                           model Working Real w; equation w = time; end Working; \
                           end P; ";

    // Called from outside, and declared from outside.
    for reach in [
        "model M Real z; equation z = P.shut(time); end M;",
        "model M P.Working w; end M;",
        "model M extends P.Working; end M;",
    ] {
        let refusal = parse_model(&format!("{PACKAGE}{reach}"))
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("keeps to itself"), "{reach}: {refusal}");
    }

    // The public one beside it, and the kept-back one named from
    // inside the package that holds it.
    parse_model(&format!(
        "{PACKAGE}model M Real z; equation z = P.open(time); end M;"
    ))
    .unwrap();
    parse_model(
        "package P \
         protected function shut input Real x; output Real y; algorithm y := 3 * x; end shut; \
         public function open input Real x; output Real y; algorithm y := P.shut(x); end open; \
         end P; \
         model M Real z; equation z = P.open(time); end M;",
    )
    .unwrap();
}

/// A chain of type aliases is read where each name was written.
#[test]
fn a_derivative_seeds_an_input_named_through_a_chain_of_aliases() {
    // `Temperature` is what `Units` calls `Absolute`, which is what it
    // calls a `Real` - and the second name is written inside `Units`,
    // not where the declaration using it stands. Reading it from the
    // wrong place makes the input look like something no derivative is
    // handed for, and the rule is refused for taking one argument too
    // many.
    let m = parse_model(
        "package Units type Absolute = Real(unit = \"K\"); type Temperature = Absolute; end Units; \
         function warmth input Units.Temperature T; output Real p; \
         algorithm p := 2 * T; annotation(derivative = warmth_der); end warmth; \
         function warmth_der input Units.Temperature T; input Real der_T; output Real der_p; \
         algorithm der_p := 2 * der_T; end warmth_der; \
         model M Real x; Real y; equation x = 300 + time; y = der(warmth(x)); end M;",
    )
    .unwrap();
    assert!(m.components.iter().any(|c| c.name == "y"));
}

/// A declaration value inside a function reads a constant of a class
/// the way a statement does.
#[test]
fn a_local_declaration_value_reads_a_constant_of_a_class() {
    let m = parse_model(
        "package Lib \
           record Limits constant Real TOP = 7; end Limits; \
           function capped input Real x; output Real y; \
           protected Real held = min(x, Limits.TOP); \
           algorithm y := 2 * held; end capped; \
         end Lib; \
         model M Real z; equation z = Lib.capped(10); end M;",
    )
    .unwrap();
    // 10 capped at 7, doubled. Read where the name was written, or
    // `Limits.TOP` would still be standing with nothing to say.
    let z = m.components.iter().find(|c| c.name == "z").unwrap();
    let mut equations = m.equations.iter().filter(|e| {
        let mut names = Vec::new();
        e.lhs.collect_refs(&mut names);
        names.contains(&"z")
    });
    let settled = equations.next().expect("nothing settles z");
    // Nothing of `Limits.TOP` is left standing: the value came out as
    // arithmetic on numbers alone.
    let mut left = Vec::new();
    settled.rhs.collect_refs(&mut left);
    assert!(left.is_empty(), "{:?} in {z:?}", left);
}

/// A length only the component's value settles is one the statements
/// of the class holding it can read.
#[test]
fn a_length_a_modifier_settled_is_seen_by_an_algorithm() {
    // `t[:, 2]` says the declaration will not say how long it is; the
    // modifier does. What the length came to is filed under the
    // component's full path, and a statement writes the name this
    // class gave the component, so it has to be filed under that too.
    let m = parse_model(
        "block Holder parameter Real t[:, 2] = fill(0.0, 0, 2); \
         Real y; \
         algorithm y := size(t, 1); end Holder; \
         model M Holder h(t = [1, 2; 3, 4; 5, 6]); Real z; equation z = h.y; end M;",
    )
    .unwrap();
    let settled = m
        .equations
        .iter()
        .find(|equation| equation.lhs == oxidelica_parser::Expr::Ref("h.y".to_string()))
        .map(|equation| equation.rhs.clone())
        .or_else(|| {
            m.components
                .iter()
                .find(|component| component.name == "h.y")
                .and_then(|component| component.binding.clone())
        });
    assert_eq!(settled, Some(oxidelica_parser::Expr::Number(3.0)));
}

/// A value handed down an `extends` is worked out, not left standing.
///
/// A modifier arrives written in the terms of the class that supplied
/// it, so it is not prefixed a second time - but it was not expanded
/// either, and a call inside one was never inlined. The parameters
/// then had a function call where they wanted a number and could
/// evaluate nothing. The machines of the standard library state their
/// nominal voltage exactly this way, as a function of the resistance
/// and the brush voltage drop handed down to a base class.
#[test]
fn a_value_handed_down_an_extends_is_worked_out() {
    let model = parse_model(
        "package Top \
           function twice input Real i; output Real v; algorithm v := 2 * i; end twice; \
           partial model Base parameter Real k = 0; Real x; equation der(x) = -k; end Base; \
           model M extends Base(final k = twice(3)); end M; \
         end Top;",
    )
    .expect("flattens");
    let k = model
        .components
        .iter()
        .find(|c| c.name == "k")
        .expect("k survives");
    assert_eq!(
        k.binding.as_ref().map(|b| b.describe()),
        Some("2 * 3".to_string()),
        "the call should have been inlined: {:?}",
        k.binding
    );
}

/// The hash of a string is the number the standard library's own C
/// says it is.
///
/// A noise block seeds itself from where it sits in the model, by
/// hashing its own instance name, so that two blocks of one model draw
/// different numbers and one model run twice draws the same ones. That
/// only holds if this compiler's hash agrees with the one the library
/// ships in C; the two numbers checked here are the ones the library's
/// own documentation prints for these two strings.
#[test]
fn a_string_hashes_to_what_the_library_says_it_does() {
    let m = parse_model(
        "model M function hashString input String s; output Integer h; \
         external \"C\" h = ModelicaStrings_hashString(s); end hashString; \
         Real a, b; equation a = hashString(\"this is a test\"); \
         b = hashString(\"Controller.noise1\"); end M;",
    )
    .expect("a hash worked out here");
    let value = |at: usize| match m.equations.get(at).map(|e| &e.rhs) {
        Some(Expr::Number(number)) => *number,
        other => panic!("expected a number, got {other:?}"),
    };
    assert_eq!(value(0), 1_827_717_433.0);
    assert_eq!(value(1), -1_025_762_750.0);
}

/// A class constant may be a vector: the multibody world states the
/// colour of its axis arrows as `Types.Defaults.FrameColor`, three
/// numbers under one name. Only a constant worth a single number was
/// substituted, so the name travelled into the flat model with the
/// instance path stuck on the front and nothing declared it.
#[test]
fn a_class_constant_that_is_a_vector_is_substituted_too() {
    let m = parse_model(
        "package P constant Real FrameColor[3] = {1,2,3}; \
         model W Real c[3] = P.FrameColor; end W; \
         model M W w; Real x; equation der(x) = w.c[1]; end M; end P;",
    )
    .expect("a vector constant read from another package");
    let written = equations_of(&m);
    assert!(
        written.contains(&"w.c[1] = 1".to_string()),
        "the vector is read element by element: {written:?}"
    );
}

#[test]
fn a_string_a_body_writes_in_one_branch_starts_empty() {
    // The language leaves an unassigned local at its type's own start,
    // and a `String` starts empty. The fluid library checks its
    // boundary compositions that way: `String X_str;` is built up
    // inside an `if` that only fires when the check fails, and read
    // there by the message it raises. Refusing that took forty-four
    // models out.
    let m = parse_model(
        "package Streams function error input String message; \
           external \"builtin\"; end error; end Streams; \
         function check input Real x; output Real y; protected String note; \
         algorithm if x > 1 then note := \"over\"; \
           Streams.error(\"failed: \" + note); end if; y := x; end check; \
         model M Real out; equation out = check(time); end M;",
    )
    .unwrap();

    // The call was written out rather than refused for a string with
    // no value before the branch.
    let out = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"out\")")
        .unwrap();
    assert_eq!(format!("{:?}", out.rhs), "Time");
}

/// A working array filled and used inside one branch and never looked
/// at again needs no merged value: `o` of the steam tables holds the
/// powers of a pressure while the branch builds a temperature from
/// them, and asking what it should be where another branch never set
/// it has no answer and no question.
#[test]
fn a_working_array_of_one_branch_needs_no_merged_value() {
    // Read only inside the branches: nothing to merge.
    let m = parse_model(
        "package P function tph input Real p; output Real T; protected Real[3] o; \
         algorithm if p < 1 then o[1] := p; o[2] := 2*p; o[3] := 3*p; T := o[1] + o[3]; \
         else o[1] := 5*p; o[2] := 6*p; T := o[1] + o[2]; end if; end tph; \
         model M Real q; Real y; equation q = 5; y = tph(q); end M; end P;",
    )
    .unwrap();
    assert!(!format!("{:?}", m.equations).contains("Call(\"P.tph\""));

    // Read after the `if`, and set in every branch: merged as before.
    let after = parse_model(
        "package P function tph input Real p; output Real T; protected Real[3] o; \
         algorithm if p < 1 then o[1] := p; o[3] := 3*p; \
         else o[1] := 5*p; o[3] := 7*p; end if; T := o[1] + o[3]; end tph; \
         model M Real q; Real y; equation q = 5; y = tph(q); end M; end P;",
    )
    .unwrap();
    assert!(!format!("{:?}", after.equations).contains("Call(\"P.tph\""));

    // Read after the `if` and set in one branch only: still refused,
    // since now there is a question and no answer.
    let err = parse_model(
        "package P function tph input Real p; output Real T; protected Real[3] o; \
         algorithm if p < 1 then o[1] := p; o[3] := 3*p; \
         else o[1] := 5*p; end if; T := o[1] + o[3]; end tph; \
         model M Real q; Real y; equation q = 5; y = tph(q); end M; end P;",
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("assigned in one branch only"), "{err}");
}

/// A call on its own among the actions of a `when` goes through the
/// same passes an assignment's value does: its arguments are resolved,
/// its strings settled, and the checks its body makes are taken up by
/// the model.
#[test]
fn a_call_at_an_event_goes_through_the_passes() {
    // Two arguments, one of them a name the class has to resolve and
    // one a constant to be folded, and a check inside the body.
    let m = parse_model(
        "package P \
         constant Real limit = 3; \
         function note input Real x; input Real y; output Real z; \
         algorithm assert(y > 0, \"y must be positive\"); z := x + y; end note; \
         model M Real t; equation t = time; \
         when terminal() then note(t, limit); end when; end M; end P;",
    )
    .unwrap();
    // The check travelled out of the body and into the model.
    let checks = format!("{:?}", m.asserts);
    assert!(checks.contains("y must be positive"), "{checks}");
    // The constant was folded where the call was written.
    assert!(
        checks.contains("3.0") || format!("{m:?}").contains("3.0"),
        "{m:?}"
    );

    // A call whose argument is a string constant: settled before the
    // run like any other string.
    let named = parse_model(
        "package P \
         constant String what = \"a file\"; \
         function shut input String name; output Real done; \
         algorithm done := 1; end shut; \
         model M Real t; equation t = time; \
         when terminal() then shut(what); end when; end M; end P;",
    );
    assert!(named.is_ok(), "{named:?}");
}

/// Whether a name is read after an `if` is answered by walking the
/// statements that follow, and the walk has to look through every kind
/// of expression there is: missing one would drop a value something
/// still reads. Each of these reads `o` in a different shape.
#[test]
fn the_walk_looks_through_every_shape_an_expression_takes() {
    let reads = [
        "T := o[1] + o[2];",                            // arithmetic
        "T := -o[1] - (-o[2]);",                        // negation
        "T := if not (o[1] > 0) then o[2] else o[1];",  // `not`
        "T := if o[1] > 0 or o[2] > 0 then 1 else 2;",  // `or`
        "T := if o[1] > 0 and o[2] > 0 then 1 else 2;", // `and`
        "T := sum({o[1], o[2]} .* {1, 1});",            // elementwise
        "T := sum([o[1], o[2]; 1, 1]);",                // a matrix
        "T := sum(o[1:2]);",                            // a range
        "T := sum({o[i] for i in 1:2});",               // a comprehension
        "T := abs(x = o[1]);",                          // a named argument
    ];
    for tail in reads {
        let source = format!(
            "package P function tph input Real p; output Real T; protected Real[2] o; \
             algorithm if p < 1 then o[1] := p; else o[1] := 2*p; o[2] := 3*p; end if; \
             {tail} end tph; \
             model M Real q; Real y; equation q = 5; y = tph(q); end M; end P;"
        );
        let err = parse_model(&source)
            .map(|_| String::from("accepted"))
            .unwrap_or_else(|e| e.to_string());
        assert!(
            err.contains("assigned in one branch only") || err.contains("not a function"),
            "`{tail}` reads `o` after the `if`, so it must be merged: {err}"
        );
    }
}

/// A body may fold what it works out of a string.
///
/// `Strings.findLast` counts back from the length of a piece of text
/// until it finds what it was looking for, and every step of that is
/// a number the arithmetic layer cannot reach on its own: the length
/// of a string, and whether two of them are the same. Left unfolded,
/// the loop head had no truth to it and the call stood.
#[test]
fn a_body_folds_what_it_works_out_of_a_string() {
    let m = parse_model(
        r#"function len input String s; output Integer n; external "C" n = ModelicaStrings_length(s); end len; function part input String s; input Integer from; input Integer to; output String piece; external "C" piece = ModelicaStrings_substring(s, from, to); end part; function last input String s; input String needle; output Integer index; protected Integer i; algorithm i := len(s) - 4 + 1; index := 0; while i >= 1 loop if part(s, i, i + 3) == needle then index := i; i := 0; else i := i - 1; end if; end while; end last; model M parameter Integer k = last("a/b/test.txt", ".txt"); Real y; equation y = k; end M;"#,
    )
    .unwrap();
    let k = m.components.iter().find(|c| c.name == "k").unwrap();
    assert_eq!(format!("{:?}", k.binding), "Some(Number(9.0))");
}

/// A function may take its body from a base.
///
/// `loadResource` of the standard library says what it takes and
/// answers with in one base and how it works in a second. Only the
/// class's own algorithm was run, so a function written that way
/// never assigned its output and the model was refused for it.
#[test]
fn a_function_may_inherit_its_body() {
    let m = parse_model(
        "partial function Base input Real x; output Real y; end Base; \
         function Mid extends Base; algorithm y := 2 * x; end Mid; \
         function Top extends Base; extends Mid; end Top; \
         model M Real z; equation z = Top(3); end M;",
    )
    .unwrap();
    let z = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"z\")")
        .unwrap();
    assert_eq!(folded(&z.rhs), 6.0);
}

/// A value worked out of a `String` parameter is worked out.
///
/// A table block asks `findLast(fileName, ".csv")` what kind of file
/// it was given, and `fileName` is a parameter of the block rather
/// than a string written where the call is. Handed the name, the body
/// had nothing to measure, and the parameter stayed a call that
/// nothing could evaluate.
#[test]
fn a_string_parameter_reaches_the_body_it_is_handed_to() {
    let m = parse_model(
        r#"function len input String s; output Integer n; protected Integer i; algorithm i := 0; while i < 1 loop i := i + 1; end while; n := ModelicaStrings_length(s) + i - 1; end len; model M parameter String path = "a/b/test.txt"; parameter Integer k = len(path); Real y; equation y = k; end M;"#,
    )
    .unwrap();
    let k = m.components.iter().find(|c| c.name == "k").unwrap();
    assert_eq!(format!("{:?}", k.binding), "Some(Number(12.0))");
}

/// An `if` inside an `if` knows what is read after the outer one.
///
/// A branch is executed on its own, and which of its variables need a
/// merged value is decided by what reads them afterwards. For an `if`
/// nested inside another, the statements that follow are only the
/// rest of the branch it sits in - the quaternion conversion of the
/// multi-body library writes its four elements in four branches two
/// deep and reads them in the line after the outer `if`. Read as
/// nothing, the array was left in the branch it was written in and
/// came out of the body under the name it had inside.
#[test]
fn a_nested_if_sees_what_follows_the_one_around_it() {
    let m = parse_model(
        "function fromT input Real T[3, 3]; output Real Q[4]; protected Real t; \
         algorithm \
           if T[3, 3] < 0 then \
             if T[1, 1] > T[2, 2] then t := 1 + T[1, 1]; Q := {t, 1, 2, 3}; \
             else t := 1 + T[2, 2]; Q := {1, t, 2, 3}; end if; \
           else \
             if T[1, 1] < -T[2, 2] then t := 1 - T[1, 1]; Q := {1, 2, t, 3}; \
             else t := 1 + T[3, 3]; Q := {1, 2, 3, t}; end if; \
           end if; \
           Q := Q * 0.5 / t; \
         end fromT; \
         model M parameter Real Qs[4] = fromT([1, 0, 0; 0, 1, 0; 0, 0, 0]); \
           Real y; equation y = Qs[4]; end M;",
    )
    .unwrap();
    // The last branch holds, so `t` is 1 and the fourth element is
    // `1 * 0.5 / 1`. What matters is that it comes to a number at
    // all: read as unmerged, the array came out of the body under
    // the name it had inside and nothing could evaluate it.
    let q = m.components.iter().find(|c| c.name == "Qs[4]").unwrap();
    assert_eq!(q.binding.as_ref().map(folded), Some(0.5), "{:?}", q.binding);
}

/// A body walked at run time may answer as long as a constant says.
///
/// The run carries numbers, so an answer has to be numbers the model
/// can name. A random generator answers with `state[nState]`, and
/// `nState` is a number its package states outright - that counts as
/// a length the compiler can see, and the walk can hand it back.
#[test]
fn a_walked_body_says_what_it_may_answer_with() {
    let m = with_lib(
        "package Gen constant Integer nState = 2; \
           function make input Real seed; output Real state[nState]; \
             algorithm state := {seed, seed + 1}; end make; \
         end Gen; \
         model M Real y; equation y = Gen.make(time)[2]; end M;",
    );
    assert!(m.is_ok(), "{:?}", m.err());
}

/// A body folds a comparison against a string the class settled.
///
/// A `while` that goes round until a piece of text is what it was
/// looking for has to know what the text says, and what a `String`
/// parameter says is worked out by the class the call was written in.
/// Left out of view, the loop head had no truth to it and the call
/// stood as one nothing worked out.
#[test]
fn a_body_folds_a_string_the_class_settled() {
    let m = with_lib(
        "model M parameter String kind = \"csv\"; parameter Integer k = f(); \
         function f output Integer y; protected Integer i; \
           algorithm i := 3; y := 0; \
             while i >= 1 loop \
               if kind == \"csv\" then y := i; i := 0; else i := i - 1; end if; \
             end while; \
         end f; \
         Real z; equation z = k; end M;",
    )
    .unwrap();
    // The text matches on the first round, so the loop stops at three.
    let k = m.components.iter().find(|c| c.name == "k").unwrap();
    assert_eq!(format!("{:?}", k.binding), "Some(Number(3.0))");
}

#[test]
fn an_array_in_the_frame_is_not_an_array_written_by_a_branch() {
    // A branch is worked out on a copy of everything standing before
    // the `if`, so its bindings name the whole body - arguments and
    // all. The guard that leaves a scalar unmerged where the branches
    // write arrays asked that copy whether an array was in it, which
    // answers about the signature rather than about the branch.
    //
    // Here no branch writes an array and one is in the signature. A
    // Spice3 transistor is written that way - a vector of four
    // voltages as its eighth argument - so every `if` in it counted
    // as one that writes arrays, and the scalars it decides lost the
    // zero their type starts at. Twenty-nine models stood at the
    // refusal that followed.
    let flat = parse_model(
        "function picker input Real x; input Real mode; \
         input Real[4] volts; output Real y; \
         protected Real held; \
         algorithm \
         if mode > 0 then held := x + volts[1]; end if; \
         y := held * 10; end picker; \
         model M Real y; Real u; equation u = time + 1; \
         y = picker(u, u - 0.5, {1.0, 2.0, 3.0, 4.0}); end M;",
    )
    .unwrap();
    // The branch that does not write it leaves the zero a `Real`
    // starts at, and the body flattens.
    assert!(flat
        .equations
        .iter()
        .any(|e| format!("{:?}", e.lhs).contains('y')));
}

/// What a path names, answered here rather than by the C the library
/// ships.
///
/// `ModelicaInternal_stat` is a POSIX call the standard library wraps
/// to ask whether a file exists and what kind it is, and four models
/// of the library stop at it. The answer is `Types.FileType`, an
/// enumeration counted from one - no file, a regular file, a
/// directory, anything else - and this compiler can say all four
/// without linking anything.
#[test]
fn what_a_path_names_is_answered_here() {
    let dir = std::env::temp_dir().join("oxidelica_stat_test");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("here.txt");
    std::fs::write(&file, "something").unwrap();
    let path = |p: &std::path::Path| p.display().to_string().replace('\\', "/");
    let m = parse_model(&format!(
        "model M function stat input String n; output Integer t; \
         external \"C\" t = ModelicaInternal_stat(n); end stat; \
         Real a, b, c; equation a = stat(\"{}\"); b = stat(\"{}\"); \
         c = stat(\"{}/nothing-is-here\"); end M;",
        path(&file),
        path(&dir),
        path(&dir),
    ))
    .expect("a path this compiler can look at");
    let value = |at: usize| match m.equations.get(at).map(|e| &e.rhs) {
        Some(Expr::Number(number)) => *number,
        other => panic!("expected a number, got {other:?}"),
    };
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(value(0), 2.0, "a regular file");
    assert_eq!(value(1), 3.0, "a directory");
    assert_eq!(value(2), 1.0, "nothing at all");
}

/// A whole number read off the front of a string, answered here.
#[test]
fn a_number_at_the_front_of_a_string_is_read_here() {
    // `ModelicaStrings_scanInteger` is how the standard library reads
    // a number out of a line it was given. Written in C there, and
    // this compiler does the same reading in Rust elsewhere.
    let m = parse_model(
        "model M function scan input String s; input Integer at; output Integer n; \
         external \"C\" n = ModelicaStrings_scanInteger(s, at); end scan; \
         Real a, b, c; equation a = scan(\"42 rest\", 1); \
         b = scan(\"  -17\", 1); c = scan(\"x 5\", 3); end M;",
    )
    .expect("numbers this compiler can read");
    let value = |at: usize| match m.equations.get(at).map(|e| &e.rhs) {
        Some(Expr::Number(number)) => *number,
        other => panic!("expected a number, got {other:?}"),
    };
    assert_eq!(value(0), 42.0);
    assert_eq!(value(1), -17.0, "leading space and a sign");
    assert_eq!(value(2), 5.0, "started from the third letter");
}

/// A line of a file, answered here rather than by the library's C.
#[test]
fn a_line_of_a_file_is_read_here() {
    // `readLine` is written in C in the standard library, and this
    // compiler reads files in Rust wherever else it needs one.
    // The strings leave the flat model, so what the line was is asked
    // by its length: "two" is three letters, and past the end of the
    // file is the empty string.
    let dir = std::env::temp_dir().join("oxidelica_readline_test");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lines.txt");
    std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
    let path = file.display().to_string().replace('\\', "/");
    let m = parse_model(&format!(
        "model M \
         function readLine input String f; input Integer n; output String line; \
           external \"C\" line = ModelicaInternal_readLine(f, n); end readLine; \
         function len input String s; output Integer n; \
           external \"C\" n = ModelicaStrings_length(s); end len; \
         parameter Integer second = len(readLine(\"{path}\", 2)); \
         parameter Integer past = len(readLine(\"{path}\", 9)); \
         Real y; equation y = time; end M;"
    ))
    .expect("a file this compiler can read");
    let settled = |wanted: &str| {
        m.components
            .iter()
            .find(|c| c.name == wanted)
            .map(|c| format!("{:?}", c.binding))
            .unwrap_or_default()
    };
    let _ = std::fs::remove_dir_all(&dir);
    assert!(settled("second").contains('3'), "{}", settled("second"));
    assert!(settled("past").contains('0'), "{}", settled("past"));
}
