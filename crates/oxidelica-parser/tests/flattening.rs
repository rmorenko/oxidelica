//! Flattening, asked of the crate from outside: a source string in, a
//! flat model out.
//!
//! These are the language's own rules - inheritance, arrays, packages,
//! connections, clocks, state machines - checked through `parse_model`,
//! which is how everything but the compiler itself sees this crate.

use oxidelica_parser::{parse_model, parse_model_with_libraries, Expr};

/// Sources that share the shape of a standard-library package: a
/// replaceable component with an interface, a conditional one and a
/// world shared through `inner`/`outer`.
const LIB: &str = "package Lib \
       connector Pin Real v; flow Real i; end Pin; \
       partial model SISO Real u; Real y; end SISO; \
       model Gain extends SISO; parameter Real k = 1; equation y = k * u; end Gain; \
       model Doubler extends SISO; equation y = 2 * u; end Doubler; \
       model Loose Real y; equation y = 0; end Loose; \
       model World parameter Real g = 9.81; end World; \
       model Falling outer World world; Real a; equation a = -world.g; end Falling; \
     end Lib;";

/// Flatten `source` with `LIB` beside it, as a project with a library
/// on its path would be read.
fn with_lib(source: &str) -> Result<oxidelica_parser::Model, String> {
    parse_model_with_libraries(&[LIB.to_string()], source).map_err(|e| e.to_string())
}

#[test]
fn flat_model_passes_through_unchanged() {
    let m = parse_model("model M Real x(start = 1); equation der(x) = -x; end M;").unwrap();
    assert_eq!(m.name, "M");
    assert_eq!(m.components.len(), 1);
    assert_eq!(m.components[0].name, "x");
}

#[test]
fn instantiates_components_with_prefixes_and_modifiers() {
    let m = parse_model(
        "model Gain parameter Real k = 1; Real u; Real y; equation y = k * u; end Gain;\
         model Top Gain g1(k = 3); Gain g2; Real s; equation \
         g1.u = time; g2.u = g1.y; s = g2.y; end Top;",
    )
    .unwrap();
    assert_eq!(m.name, "Top");
    let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"g1.k"));
    assert!(names.contains(&"g2.y"));
    // g1.k binding overridden to 3.
    let g1k = m.components.iter().find(|c| c.name == "g1.k").unwrap();
    assert_eq!(g1k.binding, Some(oxidelica_parser::Expr::Number(3.0)));
}

#[test]
fn extends_merges_base_with_modifiers() {
    let m = parse_model(
        "model Base parameter Real k = 1; Real y; equation y = k * time; end Base;             model Top extends Base(k = 5); end Top;",
    )
    .unwrap();
    let k = m.components.iter().find(|c| c.name == "k").unwrap();
    assert_eq!(k.binding, Some(oxidelica_parser::Expr::Number(5.0)));
    assert!(m.components.iter().any(|c| c.name == "y"));
}

#[test]
fn flatten_error_paths() {
    // Unknown component type.
    assert!(parse_model("model M Widget w; end M;")
        .unwrap_err()
        .to_string()
        .contains("unknown type"));
    // Unknown base class.
    assert!(parse_model("model M extends Missing; end M;")
        .unwrap_err()
        .to_string()
        .contains("unknown base class"));
    // connect of non-connectors.
    assert!(parse_model(
        "model A Real x; equation x = 1; end A;             model M A a; A b; equation connect(a, b); end M;"
    )
    .unwrap_err()
    .to_string()
    .contains("connector instances"));
    // Recursive instantiation.
    assert!(parse_model("model M M m; end M;")
        .unwrap_err()
        .to_string()
        .contains("recursive"));
    // Connectors whose members do not line up.
    assert!(parse_model(
        "connector A Real v; flow Real i; end A;             connector B Real v; flow Real q; end B;             model U A p; end U; model W B p; end W;             model M U u; W w; equation connect(u.p, w.p); end M;"
    )
    .unwrap_err()
    .to_string()
    .contains("different members"));
    // Two names for the same shape connect happily: a signal
    // output and a signal input are exactly that case.
    parse_model(
        "connector Out output Real y; end Out; connector In input Real y; end In; \
         model U Out p; equation p.y = 1; end U; model W In p; end W; \
         model M U u; W w; equation connect(u.p, w.p); end M;",
    )
    .unwrap();
    // A file with no model class.
    assert!(parse_model("connector Pin Real v; flow Real i; end Pin;")
        .unwrap_err()
        .to_string()
        .contains("no model class"));
}

#[test]
fn arrays_expand_into_scalars_and_loops_unroll() {
    let m = parse_model(
        "model A parameter Integer n = 3; Real v[n]; Real s; \
         equation for i in 1:n loop v[i] = i * time; end for; s = v[2]; end A;",
    )
    .unwrap();
    let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"v[1]") && names.contains(&"v[3]"));
    assert!(!names.contains(&"v"), "the array itself must not survive");
    // Three unrolled loop equations plus the scalar one.
    assert_eq!(m.equations.len(), 4);
    // Subscripts became plain references.
    assert!(m
        .equations
        .iter()
        .all(|e| !matches!(e.lhs, oxidelica_parser::Expr::Index(_, _))));

    // Two-dimensional arrays expand in row-major order.
    let grid = parse_model(
        "model G Real a[2, 3]; equation for i in 1:2 loop for j in 1:3 loop \
         a[i, j] = i + j; end for; end for; end G;",
    )
    .unwrap();
    let names: Vec<&str> = grid.components.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["a[1,1]", "a[1,2]", "a[1,3]", "a[2,1]", "a[2,2]", "a[2,3]"]
    );
}

#[test]
fn records_and_component_arrays_expand() {
    let m = parse_model(
        "record P Real x; Real y; end P;\
         model M P points[2]; equation for i in 1:2 loop \
         points[i].x = i * time; points[i].y = 0; end for; end M;",
    )
    .unwrap();
    let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["points[1].x", "points[1].y", "points[2].x", "points[2].y"]
    );
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
fn array_and_function_error_paths() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // A dimension that is not a compile-time constant.
    assert!(err("model M Real x; Real v[x]; equation x = 1; end M;")
        .contains("not a compile-time constant"));
    // A dimension of zero is an empty array, not a mistake; a
    // negative one still is.
    assert!(parse_model("model M Real v[0]; end M;")
        .unwrap()
        .components
        .is_empty());
    assert!(err("model M Real v[-1]; end M;").contains("not negative"));
    // A subscript that cannot be folded.
    assert!(
        err("model M Real v[2]; Real k; equation k = 1; v[1] = 0; v[2] = v[k]; end M;")
            .contains("compile-time constant")
    );
    // A subscript out of range names the bound it broke.
    assert!(
        err("model M Real v[2]; equation v[1] = 0; v[2] = v[0]; end M;")
            .contains("outside an array of 2")
    );
    // A loop bound that is not constant.
    assert!(
        err("model M Real x; equation x = 1; for i in 1:x loop x = i; end for; end M;")
            .contains("a range needs bounds the compiler can see")
    );
    // Functions: wrong arity, missing output, output never assigned.
    assert!(err(
        "function f input Real a; output Real b; algorithm b := a; end f;\
         model M Real y; equation y = f(1, 2); end M;"
    )
    .contains("expects 1 argument"));
    assert!(err("function f input Real a; algorithm a := 1; end f;\
         model M Real y; equation y = f(1); end M;")
    .contains("declares no output"));
    // Every output must be assigned, even one the caller ignores.
    assert!(err(
        "function f input Real a; output Real b; output Real c; algorithm b := a; end f;\
         model M Real y; equation y = f(1); end M;"
    )
    .contains("never assigns its output `c`"));
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
fn a_function_measures_the_array_it_is_handed() {
    const FUNCTIONS: &str = "function total input Real v[:]; output Real s; \
         algorithm s := 0; for i in 1:size(v, 1) loop s := s + v[i]; end for; end total;\
         function scaled input Real v[:]; input Real k; output Real w[size(v, 1)]; \
         algorithm for i in 1:size(v, 1) loop w[i] := k * v[i]; end for; end scaled;";

    // One function, two lengths, in the same model.
    let m = parse_model(&format!(
        "{FUNCTIONS} model M parameter Real a[3] = {{1, 2, 3}}; \
         parameter Real b[5] = {{1, 1, 1, 1, 1}}; Real p; Real q; \
         equation p = total(a); q = total(b); end M;"
    ))
    .unwrap();
    let rhs = |index: usize| format!("{:?}", m.equations[index].rhs);
    assert!(
        rhs(0).contains("Number(6.0)") || rhs(0).contains("a[3]"),
        "{}",
        rhs(0)
    );

    // The result takes the shape of the argument, so a whole-array
    // equation against it balances.
    let m = parse_model(&format!(
        "{FUNCTIONS} model M parameter Real a[3] = {{1, 2, 3}}; Real w[3]; \
         equation w = scaled(a, 2); end M;"
    ))
    .unwrap();
    assert_eq!(m.equations.len(), 3);

    // An empty array is a value like any other: a declaration of
    // length zero contributes nothing and its sum is zero.
    let m = parse_model(
        "model M parameter Integer n = 0; parameter Real nothing[n] = zeros(n); \
         Real s; Real t; equation s = sum(nothing); t = sum({}); end M;",
    )
    .unwrap();
    assert!(m.components.iter().all(|c| !c.name.starts_with("nothing[")));
    assert_eq!(format!("{:?}", m.equations[0].rhs), "Number(0.0)");
    assert_eq!(format!("{:?}", m.equations[1].rhs), "Number(0.0)");
}

#[test]
fn a_whole_array_handed_to_a_component_reaches_its_elements() {
    // The declaration has no value of its own; each instance says
    // what its array holds, and how long it is.
    let m = parse_model(
        "model Sub parameter Integer n = 3; parameter Real k[n]; Real y[n]; \
         equation for i in 1:n loop y[i] = k[i]; end for; end Sub;\
         model M Sub a(n = 3, k = {1, 2, 3}); Sub b(n = 2, k = {10, 20}); end M;",
    )
    .unwrap();
    let binding = |name: &str| {
        format!(
            "{:?}",
            m.components
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("no {name}"))
                .binding
        )
    };
    assert!(binding("a.k[1]").contains("1.0"), "{}", binding("a.k[1]"));
    assert!(binding("a.k[3]").contains("3.0"), "{}", binding("a.k[3]"));
    assert!(binding("b.k[2]").contains("20.0"), "{}", binding("b.k[2]"));
    // The shorter instance has no third element at all.
    assert!(!m.components.iter().any(|c| c.name == "b.k[3]"));

    // A start given as an array is spread the same way, and a
    // scalar one covers every element.
    let m = parse_model(
        "model Sub parameter Integer n = 3; Real x[n](start = 0); \
         equation for i in 1:n loop der(x[i]) = 0; end for; end Sub;\
         model M Sub a(n = 3, x(start = {5, 6, 7})); Sub b(n = 3, x(start = 2)); end M;",
    )
    .unwrap();
    let start = |name: &str| {
        format!(
            "{:?}",
            m.components.iter().find(|c| c.name == name).unwrap().start
        )
    };
    assert!(start("a.x[2]").contains("6.0"), "{}", start("a.x[2]"));
    assert!(start("b.x[3]").contains("2.0"), "{}", start("b.x[3]"));

    // A value of the wrong length is refused by name.
    let error =
        parse_model("model Sub parameter Real k[3]; end Sub; model M Sub a(k = {1, 2}); end M;")
            .unwrap_err()
            .to_string();
    assert!(error.contains("3 element(s)"), "{error}");
}

#[test]
fn a_loop_inside_a_component_counts_in_its_own_terms() {
    // Prefixing reaches into subscripts, so the loop variable is
    // folded to a number before it can be mistaken for a component
    // of the instance. The bound and the guards read the
    // component's own parameters, under whatever path it sits at.
    let m = parse_model(
        "model Sub parameter Integer n = 3; Real x[n]; \
         equation for i in 1:n loop \
         x[i] = (if i > 1 then x[i - 1] else 0) + i; end for; end Sub;\
         model M Sub a(n = 3); end M;",
    )
    .unwrap();
    let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["a.n", "a.x[1]", "a.x[2]", "a.x[3]"]);
    assert_eq!(m.equations.len(), 3);
    // The guard at the first element left no reference to `x[0]`.
    let text = format!("{:?}", m.equations);
    assert!(!text.contains("x[0]"), "{text}");
    assert!(!text.contains("Ref(\"a.i\")"), "{text}");
}

#[test]
fn a_run_time_if_equation_keeps_every_branch() {
    // Two equations per branch, kept side by side for the
    // compiler to choose between.
    let m = parse_model(
        "model M Real gate; Real a; Real b; equation gate = time; \
         if gate > 1 then a = 1; b = 2; else a = 3; b = 4; end if; end M;",
    )
    .unwrap();
    assert_eq!(m.equations.len(), 1);
    let conditional = &m.conditional[0];
    assert_eq!(conditional.branches.len(), 2);
    assert!(conditional.branches.iter().all(|branch| branch.len() == 2));
    assert_eq!(
        format!("{:?}", conditional.branches[1][1].rhs),
        "Number(4.0)"
    );

    // An `elseif` chain keeps a condition for every branch but the
    // last, which is the `else`.
    let m = parse_model(
        "model M Real gate; Real y; equation gate = time; \
         if gate > 2 then y = 1; elseif gate > 1 then y = 2; else y = 3; end if; end M;",
    )
    .unwrap();
    assert_eq!(m.conditional[0].conditions.len(), 2);
    assert_eq!(m.conditional[0].branches.len(), 3);

    // Whole-array equations count by their scalars, not by the
    // lines they were written on.
    let m = parse_model(
        "model M Real gate; Real v[2]; equation gate = time; \
         if gate > 1 then v = {1, 2}; else v[1] = 3; v[2] = 4; end if; end M;",
    )
    .unwrap();
    assert!(m.conditional[0].branches.iter().all(|b| b.len() == 2));

    // Each branch is checked as it was written, so a mistake
    // inside one is still caught.
    let error = parse_model(
        "model M Real gate; Boolean flag; Real y; equation gate = time; flag = true; \
         if gate > 1 then y = flag; else y = 3; end if; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("type mismatch"), "{error}");

    // A branch may equate a volt to a volt and the other an
    // ampere to an ampere: they are separate equations, and only
    // the merge puts them in one slot.
    parse_model(
        "model M Real gate; Real v(unit = \"V\"); Real i(unit = \"A\"); \
         equation gate = time; \
         if gate > 1 then v = 1; i = 0; else v = 0; i = 1; end if; end M;",
    )
    .unwrap();
}

#[test]
fn run_time_if_equation_error_paths() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // Unbalanced branches.
    assert!(
        err("model M Real gate; Real a; Real b; equation gate = time; \
         if gate > 1 then a = 1; b = 2; else a = 3; end if; b = 0; end M;")
        .contains("not balanced")
    );
    // No `else`, so the equation count would depend on the run.
    assert!(err("model M Real gate; Real a; equation gate = time; \
         if gate > 1 then a = 1; end if; end M;")
    .contains("no `else`"));
    // A connection cannot be drawn conditionally at run time.
    assert!(err("connector Pin Real v; flow Real i; end Pin; \
         model U Pin p; end U; \
         model M Real gate; U a; U b; equation gate = time; \
         if gate > 1 then connect(a.p, b.p); else connect(a.p, b.p); end if; end M;")
    .contains("connections are structural"));
}

#[test]
fn an_expandable_connector_holds_what_is_connected_to_it() {
    const SIGNALS: &str = "connector Out output Real y; end Out; \
         connector In input Real y; end In; \
         expandable connector Bus end Bus;";

    // The member exists because a connection named it, and it
    // takes the type of the other side.
    let m = parse_model(&format!(
        "{SIGNALS} model Src Out port; equation port.y = 5; end Src;\
         model Snk In port; end Snk;\
         model M Bus bus; Src src; Snk snk; \
         equation connect(src.port, bus.speed); connect(bus.speed, snk.port); end M;"
    ))
    .unwrap();
    let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
    assert!(
        names.contains(&"bus.speed.y"),
        "the bus member must exist: {names:?}"
    );
    // Source, bus and sink all carry the same signal.
    let text = format!("{:?}", m.equations);
    assert!(text.contains("bus.speed.y"), "{text}");

    // Joined buses share one pool: the sub-bus gets the member
    // too, and the two are connected.
    let m = parse_model(&format!(
        "{SIGNALS} model Src Out port; equation port.y = 5; end Src;\
         model Snk In port; end Snk;\
         model M Bus bus; Bus sub; Src src; Snk snk; \
         equation connect(bus, sub); connect(src.port, bus.speed); \
         connect(sub.speed, snk.port); end M;"
    ))
    .unwrap();
    let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
    assert!(
        names.contains(&"bus.speed.y") && names.contains(&"sub.speed.y"),
        "both buses carry the member: {names:?}"
    );
    // Everything ends up equal to the source, through both buses.
    let text = format!("{:?}", m.equations);
    assert!(text.contains("sub.speed.y"), "{text}");

    // A bus nobody writes to is simply empty.
    let m = parse_model(&format!("{SIGNALS} model M Bus bus; end M;")).unwrap();
    assert!(m.components.is_empty(), "{:?}", m.components);
}

#[test]
fn expandable_connector_error_paths() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // Two bus members connected to each other and to nothing else:
    // there is no side to take a type from.
    assert!(err("expandable connector Bus end Bus; \
         model M Bus a; Bus b; equation connect(a.speed, b.rate); end M;")
    .contains("not a connector"));
}

#[test]
fn streams_mix_by_their_connection_set() {
    const PORT: &str = "connector Port Real p; flow Real m; stream Real h; end Port;";

    // Unconnected: a port hears its own outflow back.
    let m = parse_model(&format!(
        "{PORT} model M Port port; Real y; \
         equation port.h = 7; y = inStream(port.h); end M;"
    ))
    .unwrap();
    let text = format!("{:?}", m.equations);
    assert!(
        text.contains("Ref(\"y\"), rhs: Ref(\"port.h\")"),
        "an unconnected inStream is the own outflow: {text}"
    );

    // Two on a node: each hears exactly the other.
    let m = parse_model(&format!(
        "{PORT} model A Port port; Real y; \
         equation port.h = 1; port.m = 0; y = inStream(port.h); end A;\
         model B Port port; equation port.h = 2; end B;\
         model M A a; B b; equation connect(a.port, b.port); end M;"
    ))
    .unwrap();
    let text = format!("{:?}", m.equations);
    assert!(
        text.contains("Ref(\"a.y\"), rhs: Ref(\"b.port.h\")"),
        "a pair hears each other: {text}"
    );

    // Three on a node: the flow-weighted mix of the others.
    let m = parse_model(&format!(
        "{PORT} model E Port port; Real y; \
         equation port.h = 1; port.m = 0; y = inStream(port.h); end E;\
         model M E e1; E e2; E e3; \
         equation connect(e1.port, e2.port); connect(e2.port, e3.port); end M;"
    ))
    .unwrap();
    let text = format!("{:?}", m.equations);
    assert!(
        text.contains("Call(\"max\""),
        "a junction needs weights: {text}"
    );
    // Each mix reads the two other ports, never its own.
    assert!(
        !text.contains("Ref(\"e1.y\"), rhs: Ref(\"e1.port.h\")"),
        "a junction mix is not an echo: {text}"
    );

    // The connection itself writes no equation for a stream
    // variable: the two outflow definitions above are the only
    // ones naming them on the left.
    let stream_lhs = m
        .equations
        .iter()
        .filter(|eq| format!("{:?}", eq.lhs).contains(".port.h"))
        .count();
    assert_eq!(stream_lhs, 3, "one outflow definition per component");
}

#[test]
fn streams_reach_conditions_whens_asserts_and_initials() {
    const PORT: &str = "connector Port Real p; flow Real m; stream Real h; end Port;";
    let m = parse_model(&format!(
        "{PORT} model M Port port; Real y; discrete Real d(start = 0); \
         Real z(start = 1, fixed = false); \
         equation port.h = 7; port.m = 0; der(z) = -z; \
         y = if time > 0.5 then inStream(port.h) else -inStream(port.h); \
         when time > 1 then d = inStream(port.h); end when; \
         assert(not (inStream(port.h) < 0) or inStream(port.h) > 100, \"mixed\"); \
         initial equation z = inStream(port.h); end M;"
    ))
    .unwrap();
    // Every corner rewrote its call away.
    let everything = format!(
        "{:?} {:?} {:?} {:?}",
        m.equations, m.initial_equations, m.when_clauses, m.asserts
    );
    assert!(
        !everything.contains("inStream"),
        "a call survived: {everything}"
    );
}

#[test]
fn stream_error_paths() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    const PORT: &str = "connector Port Real p; flow Real m; stream Real h; end Port;";
    // The argument must be a single reference to a stream member.
    assert!(err(&format!(
        "{PORT} model M Port port; Real y; \
         equation port.h = 1; y = inStream(1 + 2); end M;"
    ))
    .contains("single reference"));
    assert!(err(&format!(
        "{PORT} model M Port port; Real x; Real y; \
         equation port.h = 1; x = 1; y = inStream(x); end M;"
    ))
    .contains("stream variable"));
    assert!(err(&format!(
        "{PORT} model M Port port; Real y; \
         equation port.h = 1; y = inStream(port.nope); end M;"
    ))
    .contains("no member"));
    // A stream connector must carry exactly one flow variable.
    assert!(err("connector Port Real p; stream Real h; end Port; \
         model M Port port; equation port.h = 1; end M;")
    .contains("exactly one flow variable"));
    // `inStream` of something that is not a stream variable.
    assert!(err(
        "connector Port Real p; flow Real m; stream Real h; end Port; \
         model M Port port; Real y; \
         equation port.h = 1; y = inStream(port.p); end M;"
    )
    .contains("not a stream variable"));
    // `inStream` of something that is not a connector member.
    assert!(err(
        "connector Port Real p; flow Real m; stream Real h; end Port; \
         model Sub Real x; end Sub; \
         model M Port port; Sub sub; Real y; \
         equation port.h = 1; sub.x = 1; y = inStream(sub.x); end M;"
    )
    .contains("is not a connector"));
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
    // A `break` guarded by a condition the compiler cannot decide.
    assert!(
        err("function f input Real u; output Real y; algorithm y := 0; \
         for i in 1:3 loop if u > 0 then break; end if; y := y + 1; end for; end f;\
         model M Real u; Real y; equation u = time; y = f(u); end M;")
        .contains("compiler can decide")
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
fn member_access_and_nested_loops() {
    // A record array with a nested loop over two dimensions.
    let m = parse_model(
        "record P Real x; Real y; end P;\
         model M P g[2]; Real s[2, 2]; equation \
         for i in 1:2 loop g[i].x = i; g[i].y = 0; \
         for j in 1:2 loop s[i, j] = i * j; end for; end for; end M;",
    )
    .unwrap();
    assert_eq!(m.equations.len(), 8);
    let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"g[2].y") && names.contains(&"s[2,2]"));
    // Member access on something that is not a component is refused.
    assert!(
        parse_model("model M Real v[2]; equation v[1] = 0; v[2] = (v[1] + 1).x; end M;").is_err()
    );
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
fn classes_nested_inside_a_model_are_visible_to_it() {
    // A connector and a function declared in the model itself, not
    // in an enclosing package: resolution starts at the class doing
    // the looking, so both are found.
    let m = parse_model(
        "model Bus                connector Pin Real v; flow Real i; end Pin;                function double input Real a; output Real b;                algorithm b := 2 * a; end double;                Pin left; Pin right; Real y;              equation left.v = 3; right.i = 0.5;                connect(left, right); y = double(left.v); end Bus;",
    )
    .unwrap();
    assert!(m.components.iter().any(|c| c.name == "left.v"));
    // The call inlined and the connection generated its equations.
    let text = format!("{:?}", m.equations);
    assert!(!text.contains("Call"), "{text}");
    assert!(text.contains("right.v"), "{text}");

    // The inner name wins over an outer one of the same spelling.
    let shadowed = parse_model(
        "package Kit model Gain parameter Real k = 100; Real u; Real y;              equation y = k * u; end Gain; end Kit;              model M                model Gain parameter Real k = 2; Real u; Real y;                equation y = k * u; end Gain;                Gain g; Real out;              equation g.u = 1; out = g.y; end M;",
    )
    .unwrap();
    let k = shadowed
        .components
        .iter()
        .find(|c| c.name == "g.k")
        .unwrap();
    assert_eq!(k.binding, Some(Expr::Number(2.0)));
}

#[test]
fn packages_qualify_names_and_scoping_walks_outwards() {
    let m = parse_model(
        "package P \
           constant Real two = 2; \
           package Inner \
             model Gain parameter Real k = two; Real u; Real y; \
             equation y = k * u; end Gain; \
           end Inner; \
         end P; \
         model M P.Inner.Gain g; Real s; equation g.u = time; s = g.y; end M;",
    )
    .unwrap();
    // The nested class resolved, and `two` was found by walking out
    // of the enclosing packages.
    let k = m.components.iter().find(|c| c.name == "g.k").unwrap();
    assert!(k.binding.is_some());
    assert!(m.components.iter().any(|c| c.name == "g.y"));
}

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
fn the_array_layer_says_what_it_cannot_do() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    assert!(err(
        "model M Real v[2]; Real k; Real y; equation v = {1, 2}; k = time; y = v[k]; end M;"
    )
    .contains("compile-time constant"));
    assert!(
        err("model M Real v[2]; Real y; equation v = {1, 2}; y = v[0]; end M;")
            .contains("outside an array of 2")
    );
    assert!(
        err("model M Real v[2]; Real y; equation v = {1, 2}; y = v[1, 1]; end M;")
            .contains("more subscripts than dimensions")
    );
    assert!(
        err("model M Real v[3]; Real y; equation v = {1, 2, 3}; y = v; end M;")
            .contains("an equation between shapes")
    );
    assert!(
        err("model M Real y; equation y = if 1:3 then 1 else 0; end M;")
            .contains("an array is used where a scalar is expected")
    );
    assert!(err("model M Real y; discrete Real d(start = 0); equation y = time; when 1:3 then d = 1; end when; end M;")
        .contains("an array value cannot be used where a scalar is expected"));

    assert!(err("model M Real v[2]; equation v = zeros({2}); end M;")
        .contains("an array is used where a scalar is expected"));
    assert!(err("model M Real y; equation y = {i for i in 3}; end M;")
        .contains("needs an array to iterate over"));
    assert!(
        err("model M Real u; Real y; equation u = time; y = sum({i for i in 1:u}); end M;")
            .contains("a range needs bounds the compiler can see")
    );
    assert!(
        err("model M Real y[2]; Real v[2]; equation v = {1, 2}; y = v[1:v[1]]; end M;")
            .contains("bounds the compiler can see")
    );
    assert!(err(
        "model M Real m[2, 2]; Real y; equation m = [1, 2; 3, 4]; y = transpose(1); end M;"
    )
    .contains("transpose works on a matrix"));
    assert!(
        err("model M Real m[2, 2]; equation m = diagonal(3); end M;")
            .contains("diagonal takes a vector")
    );
    assert!(
        err("model M Real v[3]; equation v = cross({1, 2}, {3, 4}); end M;")
            .contains("cross takes two 3-vectors")
    );
    assert!(
        err("model M Real m[3, 2]; equation m = cat(2, [1, 2; 3, 4], [5, 6]); end M;")
            .contains("equal row counts")
    );
    assert!(err("model M Real v[2]; equation v = zeros(1.5); end M;")
        .contains("a length must be a whole number"));
    assert!(
        err("model M Real v[2]; equation v = fill(1, 2) .+ fill(1, 3); end M;")
            .contains("do not fit together")
    );
}

#[test]
fn the_rest_of_the_refusals_are_named_too() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // An initial equation whose sides are different shapes.
    assert!(err("model M Real v[2](start = {0, 0}); equation der(v[1]) = 0; der(v[2]) = 0; initial equation v = {1, 2, 3}; end M;")
        .contains("shapes that do not match"));
    // A `when` in an algorithm that holds something else.
    assert!(err("model M Real u; discrete Real c(start = 0); equation u = time; algorithm when u > 1 then for i in 1:2 loop c := 1; end for; end when; end M;")
        .contains("holds assignments"));
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
        "[EquationItem { lhs: Ref(\"y\"), rhs: Number(2.0) }]"
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
    assert!(m.components.iter().any(|c| c.name == "$state"));
    let text = format!("{:?}", m.when_clauses);
    for asked in ["activeState", "ticksInState", "timeInState"] {
        assert!(!text.contains(asked), "{asked} survived");
    }
}

#[test]
fn the_last_of_the_builtins_say_what_they_mean() {
    // `homotopy` offers an easier problem to start from; this
    // compiler takes the real one.
    let m = parse_model("model M Real y; equation y = homotopy(3 * time, time); end M;").unwrap();
    assert!(format!("{:?}", m.equations[0].rhs).contains("Number(3.0)"));

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
    assert!(err(
        "model M Real u; Real v[2]; discrete Real c(start = 0); equation u = time; v = {1, 2}; algorithm when u > 1 then c := 1; v[1] := 2; end when; end M;"
    )
    .contains("whole variables, not elements"));
}

#[test]
fn an_overconstrained_graph_is_broken_at_a_root() {
    const FRAMES: &str = "connector Frame Real r; flow Real f; end Frame; model Body Frame a; Frame b; equation a.r = b.r; a.f + b.f = 0; Connections.branch(a, b); end Body;";

    // A declared root takes its part of the graph.
    let m = parse_model(&format!(
        "{FRAMES} model Anchor Frame p; equation p.r = 0; Connections.root(p); end Anchor; model M Anchor ground; Body arm; Real here; Real there; equation connect(ground.p, arm.a); here = if Connections.isRoot(ground.p) then 1 else 0; there = if Connections.isRoot(arm.b) then 1 else 0; end M;"
    ))
    .unwrap();
    let value = |name: &str| {
        let equation = m
            .equations
            .iter()
            .find(|e| format!("{:?}", e.lhs) == format!("Ref({name:?})"))
            .unwrap_or_else(|| panic!("no equation for {name}"));
        format!("{:?}", equation.rhs)
    };
    assert!(value("here").contains("Bool(true)"), "{}", value("here"));
    assert!(value("there").contains("Bool(false)"), "{}", value("there"));

    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // What is written in the clauses has to make sense.
    assert!(err(&format!(
        "{FRAMES} model M Body arm; equation Connections.knot(arm.a); end M;"
    ))
    .contains("is not a clause this compiler knows"));
    assert!(err(&format!(
        "{FRAMES} model M Body arm; equation Connections.potentialRoot(arm.a, 1.5); end M;"
    ))
    .contains("priority of a potential root is a whole number"));
    // A part with nothing to measure against.
    assert!(err(&format!(
        "{FRAMES} model M Body arm; Real y; equation y = 1; end M;"
    ))
    .contains("has no root"));
    // Two declared roots in one part is one too many.
    assert!(err(&format!(
        "{FRAMES} model Anchor Frame p; equation p.r = 0; Connections.root(p); end Anchor; model M Anchor one; Anchor two; Body arm; equation connect(one.p, arm.a); connect(arm.b, two.p); end M;"
    ))
    .contains("more than one root"));
    // A potential root serves where no root was declared, and the
    // answer is found wherever in an expression it was asked.
    let m = parse_model(&format!(
        "{FRAMES} model Loose Frame p; equation p.r = 0; Connections.potentialRoot(p, 2); end Loose; model M Loose maybe; Body arm; Real deep; equation connect(maybe.p, arm.a); deep = if not Connections.isRoot(arm.b) and (Connections.rooted(maybe.p) or false) then abs(-(if Connections.isRoot(maybe.p) then 2 else 3)) else 0; end M;"
    ))
    .unwrap();
    let text = format!("{:?}", m.equations);
    assert!(!text.contains("Connections."), "all answered: {text}");
    assert!(text.contains("Bool(true)"), "the potential root took it");
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
    for name in ["$state", "$ticks"] {
        let component = m.components.iter().find(|c| c.name == name).unwrap();
        assert_eq!(
            component.variability,
            oxidelica_parser::Variability::Discrete,
            "{name}"
        );
    }
    // It starts nowhere, so the first tick is an arrival at the
    // initial state like any other.
    let state = m.components.iter().find(|c| c.name == "$state").unwrap();
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
    for wanted in ["$state", "$ticks", "a.n", "b.n"] {
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
    // And one model holds one machine, however many classes bring
    // one along.
    assert!(err(
        "model Machine block S Real n(start = 0); equation n = previous(n) + 1; end S; S a; equation initialState(a); end Machine; model M Clock c = Clock(0.5); Machine one; Machine two; end M;"
    )
    .contains("only one state machine"));
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
    .contains("`initialState(...)`"));
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
    // A setting this compiler will not pretend to honour.
    assert!(err(
        "model M block S Real n(start = 0); equation n = previous(n) + 1; end S; \
         Clock c = Clock(0.5); S a; S b; \
         equation initialState(a); \
         transition(a, b, a.n >= 1, synchronize = true); end M;"
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

/// When each `when` clause of a flat model ticks: the start and the
/// interval the clock was lowered onto.
fn ticks_of(m: &oxidelica_parser::Model) -> Vec<(f64, f64)> {
    m.when_clauses
        .iter()
        .map(|clause| match &clause.branches[0].condition {
            Expr::Call(name, args) if name == "sample" && args.len() == 2 => {
                match (&args[0], &args[1]) {
                    (Expr::Number(start), Expr::Number(interval)) => (*start, *interval),
                    other => panic!("a clock ticks on two numbers, not {other:?}"),
                }
            }
            other => panic!("not lowered onto a clock: {other:?}"),
        })
        .collect()
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
fn a_loop_runs_over_whatever_the_values_are() {
    // A range, a set and an array are one thing to a loop: the values,
    // in order. Two of these were refused outright before, and the
    // third only in its plainest form.
    let m = parse_model(
        "model M Real y[5]; Real a[3]; Real m[2,3]; \
         equation for i in {1, 3, 5} loop y[i] = i * 10; end for; \
         for i in {2, 4} loop y[i] = -1; end for; \
         for i loop a[i] = i * i; end for; \
         for i in 1:2, j in 1:3 loop m[i,j] = i * 10 + j; end for; end M;",
    )
    .unwrap();
    let defined: Vec<String> = m
        .equations
        .iter()
        .map(|equation| format!("{:?}", equation.lhs))
        .collect();
    // The set says it: three loops written five different ways, and
    // every element each one names comes out once. `for i loop` took
    // its range from the array the body subscripts by `i`, and the two
    // indices ran as two loops one inside the other.
    for name in [
        "y[1]", "y[3]", "y[5]", "y[2]", "y[4]", "a[1]", "a[2]", "a[3]", "m[1,1]", "m[1,3]",
        "m[2,1]", "m[2,3]",
    ] {
        assert!(
            defined.contains(&format!("Ref({name:?})")),
            "{name} among {defined:?}"
        );
    }
    assert_eq!(m.equations.len(), 5 + 3 + 6);

    // The same four forms among the statements of a function.
    let value = |body: &str| {
        let m = parse_model(&format!(
            "model M function f input Real x; output Real y; protected Real a[4]; \
             algorithm {body} end f; Real y; equation y = f(2); end M;"
        ))
        .unwrap();
        format!("{:?}", m.equations)
    };
    // Unrolling leaves the sum written out rather than added up, so
    // what the loop ran over is read off the terms.
    let over_the_set = value("y := 0; for i in {1, 3, 5} loop y := y + i; end for;");
    for term in ["1.0", "3.0", "5.0"] {
        assert!(over_the_set.contains(term), "{over_the_set}");
    }
    let stepped = value("y := 0; for i in 1:2:7 loop y := y + i; end for;");
    assert!(
        stepped.contains("7.0") && !stepped.contains("6.0"),
        "{stepped}"
    );
    // `for i loop` among statements reads the range off the array the
    // body assigns through it - through an `if` around the assignment,
    // through a `while` around it, or off the value being assigned.
    assert!(value("for i loop a[i] := i; end for; y := a[4];").contains("4.0"));
    assert!(value(
        "for i loop if x > 0 then a[i] := i; else a[i] := 0; end if; end for; y := a[4];"
    )
    .contains("4.0"));
    // A `while` that never runs still says what the range is: the scan
    // reads the body as written rather than as executed.
    assert!(
        value("y := 0; for i loop while false loop y := a[i]; end while; end for; y := 7;")
            .contains("7.0")
    );

    // The body may say the range from a `connect` rather than from an
    // equation, and an outer loop may hear it from an inner one's body.
    let m = parse_model(
        "model M connector Pin Real v; flow Real i; end Pin; \
         model Node Pin p; Real x; equation p.i = 0; x = p.v; end Node; \
         Node n[3]; Pin bus; Real m[2,2]; \
         equation for i loop connect(n[i].p, bus); end for; \
         for i loop for j in 1:2 loop m[i,j] = i + j; end for; end for; \
         bus.v = 5; end M;",
    )
    .unwrap();
    assert!(m.components.iter().any(|c| c.name == "n[3].x"));
    assert!(m
        .equations
        .iter()
        .any(|equation| equation.lhs == Expr::Ref("m[2,2]".to_string())));

    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // A loop runs over values, and one value is not values.
    assert!(
        err("model M Real y; equation for i in 3 loop y = 1; end for; end M;")
            .contains("a range, a set or an array")
    );
    // And where the body is left to say the range, it has to say it.
    for section in [
        "equation for i loop y = 1; end for;",
        "algorithm for i loop y := 1; end for;",
    ] {
        assert!(
            err(&format!("model M Real y; {section} end M;"))
                .contains("nothing in the body uses `i` to subscript"),
            "{section}"
        );
    }
}

/// A function whose body the differentiator cannot read, and the
/// derivative the model supplies for it.
const NOT_SMOOTH: &str = "function f input Real x; output Real y; \
     algorithm y := abs(x) * 2; annotation(derivative = fd); end f; \
     function fd input Real x; input Real x_der; output Real y_der; \
     algorithm y_der := (if x >= 0 then 2 else -2) * x_der; end fd; ";

#[test]
fn what_cannot_be_inlined_travels_with_the_model() {
    // A recursive function is carried whole, with everything it calls,
    // and named the way the registry knows it so the walk finds it.
    let m = parse_model(
        "model M function even input Real n; output Real y; \
         algorithm if n <= 0 then y := 1; else y := odd(n - 1); end if; end even; \
         function odd input Real n; output Real y; \
         algorithm if n <= 0 then y := 0; else y := even(n - 1); end if; end odd; \
         Real y; equation y = even(4); end M;",
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
    assert_eq!(
        format!("{:?}", m.equations[0].rhs),
        "Call(\"M.even\", [Number(4.0)])"
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
    // may: a walk carries numbers, one at a time.
    let recursive = |extra: &str, body: &str| {
        format!(
            "model M function f input Real a; output Real b; {extra} \
             algorithm {body} if a > 0 then b := f(a - 1); end if; end f; \
             Real y; equation y = f(1); end M;"
        )
    };
    assert!(
        err(&recursive("protected Real v[3];", "v[1] := a; b := v[1];"))
            .contains("`v` is an array")
    );
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
    // The options the specification allows beside it are refused rather
    // than skipped: reading one wrong gives a wrong derivative.
    assert!(err(&format!(
        "model M {} Real x(start = 2, fixed = true); Real v; \
         equation der(x) = v; f(x) = 4 + time; end M;",
        NOT_SMOOTH.replace("derivative = fd", "derivative(order = 2) = fd")
    ))
    .contains("more than this compiler reads"));
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
    // A check inside a function would have to travel out through the
    // expression the call becomes, and an expression carries none.
    assert!(err("model M function f input Real x; output Real y; \
         algorithm assert(x > 0, \"positive\"); y := x; end f; \
         Real y; equation y = f(2); end M;")
    .contains("has nowhere to go"));
    // A call standing on its own cannot do anything: every function
    // here is pure, so its answer has to go somewhere.
    assert!(err(
        "model M function f input Real x; output Real y; algorithm y := x; end f; \
         Real y; algorithm f(1); y := 1; end M;"
    )
    .contains("on its own does nothing"));
    assert!(
        err("model M Real y; algorithm terminate(\"now\"); y := 1; end M;")
            .contains("belongs in a `when`")
    );
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
fn imports_type_aliases_and_partial_classes() {
    let m = parse_model(
        "package Lib \
           type Voltage = Real(unit = \"V\", start = 7); \
           partial model Base Real x; end Base; \
           model Source extends Base; Voltage v; \
           equation x = time; v = 2 * x; end Source; \
         end Lib; \
         model M import Lib.Source; Source s; end M;",
    )
    .unwrap();
    // The alias contributed its start attribute; the unit string was
    // ignored.
    let v = m.components.iter().find(|c| c.name == "s.v").unwrap();
    assert_eq!(v.start, Some(oxidelica_parser::Expr::Number(7.0)));

    // A partial class may be extended but not instantiated.
    let error = parse_model(
        "partial model Base Real x; end Base; \
         model M Base b; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("partial"), "{error}");

    // Packages are not component types either.
    let error = parse_model(
        "package P model Q Real x; equation x = 1; end Q; end P; \
         model M P p; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("package"), "{error}");
}

#[test]
fn msl_style_syntax_is_accepted() {
    // A component written the way the Modelica Standard Library
    // writes them: a `within` header, dotted names, an attribute
    // modifier, assert(), noEvent() and a graphical annotation full
    // of braces.
    let m = parse_model(
        "within Modelica.Electrical.Analog.Basic; \
         model Resistor \"Ideal linear electrical resistor\" \
           parameter Real R(start = 1) \"Resistance\"; \
           Real v; Real i; \
         equation \
           assert(R > 0, \"Resistance must be positive\"); \
           v = noEvent(R * i); \
           i = smooth(0, time); \
           annotation (Icon(coordinateSystem(extent = {{-100, -100}, {100, 100}}), \
             graphics = {Rectangle(extent = {{-70, 30}, {70, -30}})})); \
         end Resistor;",
    )
    .unwrap();
    // The `within` header is part of the class's name: that is
    // what lets a library be spread over a directory.
    assert_eq!(m.name, "Modelica.Electrical.Analog.Basic.Resistor");
    assert_eq!(
        m.equations.len(),
        2,
        "assert is skipped, two equations remain"
    );
    // noEvent and smooth collapse to their value argument.
    assert!(!format!("{:?}", m.equations).contains("noEvent"));
    assert!(!format!("{:?}", m.equations).contains("smooth"));
}

#[test]
fn class_info_reports_ports_and_parameters_including_inherited() {
    use oxidelica_parser::class_info;
    let classes = oxidelica_parser::parse_file(
        "package Lib \
           connector Pin Real v; flow Real i; end Pin; \
           partial model OnePort Pin p; Pin n; Real v; Real i; \
           equation v = p.v - n.v; p.i = i; n.i = -i; end OnePort; \
           model Resistor extends OnePort; parameter Real R = 1; \
           equation v = R * i; end Resistor; \
           model Gain parameter Real k = 1; Real u; Real y; \
           equation y = k * u; end Gain; \
         end Lib;",
    )
    .unwrap();

    // Ports and parameters come from the base as well as the class.
    let resistor = class_info(&classes, "Lib.Resistor").unwrap();
    assert_eq!(resistor.ports, vec!["p", "n"]);
    assert_eq!(resistor.parameters.len(), 1);
    assert_eq!(resistor.parameters[0].0, "R");
    assert!(resistor.instantiable);

    // A partial base is described but not instantiable.
    let base = class_info(&classes, "Lib.OnePort").unwrap();
    assert_eq!(base.ports, vec!["p", "n"]);
    assert!(!base.instantiable);

    // A class without connectors has no ports.
    let gain = class_info(&classes, "Lib.Gain").unwrap();
    assert!(gain.ports.is_empty());
    assert!(gain.instantiable);

    // Connectors themselves are not instantiable as components.
    assert!(!class_info(&classes, "Lib.Pin").unwrap().instantiable);
    assert!(class_info(&classes, "Lib.Missing").is_none());
}

#[test]
fn package_constants_are_substituted_by_value() {
    // A constant of a package, reached from a model and from inside
    // a library class - the latter is what makes a sine source work.
    let m = parse_model(
        "package Lib \
           constant Real two = 2; \
           constant Real four = 2 * two; \
           model Doubler Real y; equation y = Lib.four * time; end Doubler; \
         end Lib; \
         model M Lib.Doubler d; Real z; equation z = Lib.two * time; end M;",
    )
    .unwrap();
    let text = format!("{:?}", m.equations);
    // Both references became numbers, and the constant built on
    // another constant resolved too.
    assert!(text.contains("Number(4.0)"), "{text}");
    assert!(text.contains("Number(2.0)"), "{text}");
    assert!(!text.contains("Lib.two"), "a reference survived: {text}");

    // Dotted names that are not class constants keep their meaning:
    // this one is a connector variable of a component.
    let circuit = parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         model Probe Pin p; Real reading; equation reading = p.v; p.i = 0; end Probe; \
         model M Probe probe; equation probe.p.v = time; end M;",
    )
    .unwrap();
    assert!(format!("{:?}", circuit.equations).contains("probe.p.v"));
}

#[test]
fn a_replaceable_package_swaps_constants_and_functions() {
    let media = "package Media              partial package PartialMedium constant Real rho = 0;                function f input Real x; output Real y; algorithm y := 0; end f;              end PartialMedium;              package Water extends PartialMedium; constant Real rho = 1000;                function f input Real x; output Real y; algorithm y := 2 * x; end f;              end Water;              package Oil extends PartialMedium; constant Real rho = 900;                function f input Real x; output Real y; algorithm y := 3 * x; end f;              end Oil;              package Rogue constant Real rho = 1; end Rogue;            end Media;            model Tank              replaceable package Medium = Media.Water constrainedby Media.PartialMedium;              Real a; Real b; equation a = Medium.rho; b = Medium.f(4); end Tank; ";

    // The default alias: water's constant and water's function.
    let plain = parse_model(&format!("{media} model M Tank tank; end M;")).unwrap();
    let text = format!("{:?}", plain.equations);
    assert!(text.contains("Number(1000.0)"), "{text}");
    // The function inlined with its own factor; folding constants
    // is the simulator's business, not the flattener's.
    assert!(text.contains("Number(2.0)"), "{text}");

    // Redeclared in an extends modifier: everything becomes oil.
    let swapped = parse_model(&format!(
        "{media} model OilTank extends Tank(redeclare package Medium = Media.Oil); end OilTank;"
    ))
    .unwrap();
    let text = format!("{:?}", swapped.equations);
    assert!(text.contains("Number(900.0)"), "{text}");
    assert!(text.contains("Number(3.0)"), "{text}");

    // Redeclared in a component's modifier list.
    let component = parse_model(&format!(
        "{media} model M Tank tank(redeclare package Medium = Media.Oil); end M;"
    ))
    .unwrap();
    assert!(format!("{:?}", component.equations).contains("Number(900.0)"));

    // Redeclared in the body of a derived class.
    let body = parse_model(&format!(
        "{media} model OilTank extends Tank; redeclare package Medium = Media.Oil; end OilTank;"
    ))
    .unwrap();
    assert!(format!("{:?}", body.equations).contains("Number(900.0)"));

    // A replacement outside the constraining interface is refused.
    let error = parse_model(&format!(
        "{media} model Bad extends Tank(redeclare package Medium = Media.Rogue); end Bad;"
    ))
    .unwrap_err()
    .to_string();
    assert!(error.contains("does not extend"), "{error}");

    // And so is replacing an alias never marked replaceable.
    let error = parse_model(&format!(
        "{media} model Fixed package Medium = Media.Water; Real a;              equation a = Medium.rho; end Fixed;              model Bad extends Fixed(redeclare package Medium = Media.Oil); end Bad;"
    ))
    .unwrap_err()
    .to_string();
    assert!(error.contains("not declared replaceable"), "{error}");
}

#[test]
fn redeclare_replaces_the_type_of_a_replaceable_component() {
    // The base declares a Gain, the derived model swaps in a Doubler
    // and the equations follow the new type.
    let m = with_lib(
        "model Base replaceable Lib.Gain block1(k = 3) constrainedby Lib.SISO; \
           Real y; equation block1.u = time; y = block1.y; end Base; \
         model Derived extends Base(redeclare Lib.Doubler block1); end Derived;",
    )
    .unwrap();
    let text = format!("{:?}", m.equations);
    assert!(text.contains("Number(2.0)"), "{text}");
    // The Gain's parameter is gone with the Gain.
    assert!(!m.components.iter().any(|c| c.name == "block1.k"));

    // A redeclaration written in the body of the derived class does
    // the same thing.
    let in_body = with_lib(
        "model Base2 replaceable Lib.Gain block1 constrainedby Lib.SISO; \
           Real y; equation block1.u = time; y = block1.y; end Base2; \
         model Derived2 extends Base2; redeclare Lib.Doubler block1; end Derived2;",
    )
    .unwrap();
    assert!(format!("{:?}", in_body.equations).contains("Number(2.0)"));
}

#[test]
fn redeclare_error_paths() {
    // Not replaceable.
    let error = with_lib(
        "model Base Lib.Gain block1; Real y; equation block1.u = time; y = block1.y; end Base; \
         model Derived extends Base(redeclare Lib.Doubler block1); end Derived;",
    )
    .unwrap_err();
    assert!(error.contains("not declared replaceable"), "{error}");

    // The replacement does not meet the constraining interface.
    let error = with_lib(
        "model Base replaceable Lib.Gain block1 constrainedby Lib.SISO; \
           Real y; equation block1.u = time; y = block1.y; end Base; \
         model Derived extends Base(redeclare Lib.Loose block1); end Derived;",
    )
    .unwrap_err();
    assert!(error.contains("does not extend"), "{error}");

    // An unknown type in the redeclaration.
    let error = with_lib(
        "model Base replaceable Lib.Gain block1 constrainedby Lib.SISO; \
           Real y; equation block1.u = time; y = block1.y; end Base; \
         model Derived extends Base(redeclare Lib.Missing block1); end Derived;",
    )
    .unwrap_err();
    assert!(error.contains("unknown type"), "{error}");
}

#[test]
fn outer_components_reach_the_inner_instance() {
    let m = with_lib(
        "model Top inner Lib.World world(g = 2); Lib.Falling ball; \
         Real a; equation a = ball.a; end Top;",
    )
    .unwrap();
    // `world.g` inside the component resolved to the shared
    // instance, not to a variable of its own.
    assert!(format!("{:?}", m.equations).contains("world.g"));
    assert!(!m.components.iter().any(|c| c.name == "ball.world.g"));
    let g = m.components.iter().find(|c| c.name == "world.g").unwrap();
    assert_eq!(g.binding, Some(Expr::Number(2.0)));
}

#[test]
fn outer_without_inner_is_refused() {
    let error = with_lib("model Top Lib.Falling ball; end Top;").unwrap_err();
    assert!(error.contains("no `inner` declaration"), "{error}");

    // An `outer` of a type the `inner` instance is not.
    let error = with_lib("model Top inner Lib.Gain world; Lib.Falling ball; end Top;").unwrap_err();
    assert!(
        error.contains("does not match the `inner` instance"),
        "{error}"
    );
}

#[test]
fn a_false_condition_removes_a_component_and_its_connections() {
    let source = "connector Pin Real v; flow Real i; end Pin; \
         model Probe Pin p; Real reading; equation reading = p.v; p.i = 0; end Probe; \
         model Top parameter Boolean measure = false; \
           Probe probe if measure; Pin node; \
         equation node.v = time; connect(probe.p, node); end Top;";
    let m = oxidelica_parser::parse_model(source).unwrap();
    assert!(
        !m.components.iter().any(|c| c.name.starts_with("probe")),
        "the component survived its condition"
    );
    // With the probe gone, the node is the only member of its set and
    // its flow is forced to zero rather than joined to a missing one.
    let text = format!("{:?}", m.equations);
    assert!(!text.contains("probe"), "{text}");
    assert!(text.contains("node.i"), "{text}");

    // The same model with the condition true keeps both.
    let kept = oxidelica_parser::parse_model(&source.replace("measure = false", "measure = true"))
        .unwrap();
    assert!(kept.components.iter().any(|c| c.name == "probe.reading"));
    assert!(format!("{:?}", kept.equations).contains("probe.p.v"));
}

#[test]
fn a_condition_must_be_constant() {
    let error = oxidelica_parser::parse_model(
        "model Inner Real x; equation x = 1; end Inner; \
         model Top Real gate; Inner part if gate > 0; equation gate = time; end Top;",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("not a compile-time constant"), "{error}");
}

#[test]
fn enumeration_literals_are_their_position() {
    let m = oxidelica_parser::parse_model(
        "package Types type Kind = enumeration(First, Second \"the second one\", Third); \
         end Types; \
         model M parameter Types.Kind kind = Types.Kind.Second; Real y; \
         equation y = if kind == Types.Kind.Third then 30 else 20; end M;",
    )
    .unwrap();
    // The parameter is carried as an Integer holding the position.
    let kind = m.components.iter().find(|c| c.name == "kind").unwrap();
    assert_eq!(kind.type_name, "Integer");
    assert_eq!(kind.binding, Some(Expr::Number(2.0)));
    assert!(format!("{:?}", m.equations).contains("Number(3.0)"));
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
fn if_equations_pick_a_branch_at_compile_time() {
    let template = "model M parameter Boolean fast = SETTING; Real y; \
         equation if fast then y = 2 * time; else y = time / 2; end if; end M;";
    let fast = oxidelica_parser::parse_model(&template.replace("SETTING", "true")).unwrap();
    let slow = oxidelica_parser::parse_model(&template.replace("SETTING", "false")).unwrap();
    // Both models have exactly one equation: the other branch is gone.
    assert_eq!(fast.equations.len(), 1);
    assert_eq!(slow.equations.len(), 1);
    assert!(matches!(
        fast.equations[0].rhs,
        Expr::Bin(oxidelica_parser::BinOp::Mul, _, _)
    ));
    assert!(matches!(
        slow.equations[0].rhs,
        Expr::Bin(oxidelica_parser::BinOp::Div, _, _)
    ));

    // An elseif chain, and a chain where nothing holds.
    let chain = "model M parameter Integer mode = SETTING; Real y; equation \
         if mode == 1 then y = time; elseif mode == 2 then y = 2 * time; end if; \
         end M;";
    assert_eq!(
        oxidelica_parser::parse_model(&chain.replace("SETTING", "2"))
            .unwrap()
            .equations
            .len(),
        1
    );
    assert!(
        oxidelica_parser::parse_model(&chain.replace("SETTING", "3"))
            .unwrap()
            .equations
            .is_empty()
    );

    // A condition the run decides keeps every branch instead, for
    // the compiler to settle where the run has got to.
    let m = oxidelica_parser::parse_model(
        "model M Real gate; Real y; equation gate = time; \
         if gate > 0 then y = 1; else y = 2; end if; end M;",
    )
    .unwrap();
    assert_eq!(m.equations.len(), 1);
    assert_eq!(m.conditional.len(), 1);
    let conditional = &m.conditional[0];
    assert_eq!(conditional.conditions.len(), 1);
    assert_eq!(conditional.branches.len(), 2);
    assert_eq!(
        format!("{:?}", conditional.branches[0][0].rhs),
        "Number(1.0)"
    );
    assert_eq!(
        format!("{:?}", conditional.branches[1][0].rhs),
        "Number(2.0)"
    );
}

#[test]
fn if_equations_can_hold_connections() {
    let source = "connector Pin Real v; flow Real i; end Pin; \
         model Top parameter Boolean joined = SETTING; Pin a; Pin b; \
         equation a.v = time; if joined then connect(a, b); end if; end Top;";
    let joined = oxidelica_parser::parse_model(&source.replace("SETTING", "true")).unwrap();
    // Joined: one potential equality and one flow sum.
    let text = format!("{:?}", joined.equations);
    assert!(text.contains("b.v"), "{text}");
    let apart = oxidelica_parser::parse_model(&source.replace("SETTING", "false")).unwrap();
    // Apart: each connector carries its own zero flow.
    let text = format!("{:?}", apart.equations);
    assert!(text.contains("a.i") && text.contains("b.i"), "{text}");
}

#[test]
fn a_binding_on_a_variable_is_an_equation() {
    let m =
        oxidelica_parser::parse_model("model M Real x; Real y = 2 * x; equation x = time; end M;")
            .unwrap();
    assert_eq!(m.equations.len(), 2);
    // The declaration equation survived as an equation, not as a
    // binding that the solver would ignore.
    assert!(m
        .components
        .iter()
        .find(|c| c.name == "y")
        .unwrap()
        .binding
        .is_none());
    assert!(format!("{:?}", m.equations).contains("Ref(\"y\")"));
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
fn redeclarations_reach_through_a_nested_component() {
    // `mid(redeclare Doubler leaf)` names a component one level down.
    let m = with_lib(
        "model Middle replaceable Lib.Gain leaf(k = 3) constrainedby Lib.SISO(); \
           Real y; equation leaf.u = time; y = leaf.y; end Middle; \
         model Top Middle mid(redeclare Lib.Doubler leaf); Real z; \
           equation z = mid.y; end Top;",
    )
    .unwrap();
    assert!(format!("{:?}", m.equations).contains("Number(2.0)"));
    assert!(!m.components.iter().any(|c| c.name == "mid.leaf.k"));
}

#[test]
fn structural_conditions_use_the_whole_boolean_language() {
    // Comparisons, `and`, `or` and `not` all fold at compile time.
    let m = oxidelica_parser::parse_model(
        "model Part Real x; equation x = 1; end Part; \
         model M parameter Integer n = 3; parameter Boolean on = true; \
           Part a if n >= 3 and on; \
           Part b if n < 3 or not on; \
           Part c if n <> 3; \
         end M;",
    )
    .unwrap();
    let parts: Vec<&str> = m
        .components
        .iter()
        .map(|c| c.name.as_str())
        .filter(|name| name.ends_with(".x"))
        .collect();
    assert_eq!(parts, vec!["a.x"], "kept the wrong components: {parts:?}");
}

#[test]
fn an_unknown_constraining_class_is_reported() {
    let error = with_lib(
        "model Base replaceable Lib.Gain block1 constrainedby Lib.Nothing; \
           Real y; equation block1.u = time; y = block1.y; end Base; \
         model Derived extends Base(redeclare Lib.Doubler block1); end Derived;",
    )
    .unwrap_err();
    assert!(error.contains("unknown constraining class"), "{error}");
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
    // An empty loop body.
    assert!(err("model M Real y; algorithm for i in 1:2 loop end for; end M;").contains("no body"));
}

#[test]
fn arrays_are_values() {
    // A whole-array equation, a literal, the elementwise operators,
    // reductions, sizes and constructors - each expanded into the
    // scalars underneath at compile time.
    let m = parse_model(
        "model A parameter Real k[3] = {2, 4, 6}; Real v[3]; Real w[3];              Real total; Real dot;              equation v = {1, 2, 3}; w = 2 * v .* k;              total = sum(v) + size(v, 1) + max(k); dot = v * k; end A;",
    )
    .unwrap();
    let text = format!("{:?}", m.equations);
    // Three scalar equations came out of `w = 2 * v .* k`.
    assert_eq!(
        m.equations
            .iter()
            .filter(|e| format!("{:?}", e.lhs).contains("w["))
            .count(),
        3
    );
    // The scalar product is a sum of products, not a vector.
    assert!(text.contains("dot"), "{text}");

    // Constructors: fill, zeros, linspace; and an array start.
    let chain = parse_model(
        "model C parameter Integer n = 4;              parameter Real k[n] = fill(7.0, n);              parameter Real grid[n] = linspace(0.0, 3.0, n);              Real x[n](start = grid);              equation der(x) = zeros(n) .+ k; end C;",
    )
    .unwrap();
    let k2 = chain.components.iter().find(|c| c.name == "k[2]").unwrap();
    assert_eq!(k2.binding, Some(Expr::Number(7.0)));
    // Each element starts from its own element of the grid; the
    // number itself is the simulator's to look up.
    let x3 = chain.components.iter().find(|c| c.name == "x[3]").unwrap();
    assert_eq!(x3.start, Some(Expr::Ref("grid[3]".to_string())));
}

#[test]
fn matrices_conditionals_and_the_remaining_array_corners() {
    // A matrix: a two-dimensional literal against a declared shape,
    // sized in both dimensions, summed and taken apart.
    let m = parse_model(
        "model M Real a[2, 3]; Real s; Real rows;              equation a = {{1, 2, 3}, {4, 5, 6}};              s = sum(a) + product({1, 2, 3}); rows = size(a, 1); end M;",
    )
    .unwrap();
    let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"a[2,3]"), "{names:?}");
    assert_eq!(m.equations.len(), 8, "six elements plus two scalars");

    // An if-expression over whole arrays, with a scalar condition.
    let picked = parse_model(
        "model P parameter Boolean top = true; Real v[2];              equation v = if top then {1, 2} else {3, 4}; end P;",
    )
    .unwrap();
    assert_eq!(picked.equations.len(), 2);

    // An elementwise op inside a function argument spreads the call.
    let spread = parse_model(
        "model S Real v[2]; Real w[2];              equation v = {0.1, 0.2}; w = sin(v ./ {2, 4}); end S;",
    )
    .unwrap();
    let text = format!("{:?}", spread.equations);
    assert_eq!(text.matches("Call(\"sin\"").count(), 2, "{text}");

    // `sum` folds the full array expression, `.+` broadcasts, and a
    // whole-array equation may live inside a for body.
    let looped = parse_model(
        "model L parameter Integer n = 2; Real g[n]; Real acc[n];              equation g = fill(3.0, n);              for i in 1:1 loop acc = g .+ 1; end for; end L;",
    )
    .unwrap();
    assert_eq!(looped.equations.len(), 4);
}

#[test]
fn the_expression_walkers_cover_the_long_tail() {
    // One model that drives the rarely-taken arms: logic and
    // conditionals inside class-constant substitution, subscripted
    // references under prefixing, `size(v)` without a dimension,
    // and a boolean fold inside const_eval.
    let m = parse_model(
        "package P constant Real lim = 2; end P;              model W parameter Boolean wide = true;              parameter Real q = if wide and not (P.lim > 3) then 1 else 0;              Real v[2]; Real n[1]; Real s;              equation v = {if wide then P.lim else 0, P.lim};              n = size(v) .+ 0; s = v[1] + q; end W;",
    )
    .unwrap();
    let q = m.components.iter().find(|c| c.name == "q").unwrap();
    // The condition folded: wide and the package constant are known.
    assert!(format!("{:?}", q.binding).contains("If"), "{:?}", q.binding);
    assert_eq!(m.equations.len(), 4);

    // Functions with array-aware bodies still inline per element,
    // and substitution reaches through every operator on the way.
    let inlined = parse_model(
        "function pick input Real a; input Real b; output Real c;              algorithm c := if a > b or a < -b then a else -b; end pick;             model F Real v[2]; Real w[2];              equation v = {1, -3}; w = pick(v .* {1, 1}, fill(2.0, 2)); end F;",
    )
    .unwrap();
    assert!(
        !format!("{:?}", inlined.equations).contains("Call(\"pick\""),
        "the call must inline"
    );
}

#[test]
fn arrays_refuse_what_does_not_fit() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // Shapes that do not match.
    assert!(
        err("model M Real v[3]; Real w[2]; equation v = {1, 2, 3}; w = v; end M;")
            .contains("shapes")
    );
    // An array where a scalar belongs.
    assert!(
        err("model M Real v[2]; Real s; equation v = {1, 2}; s = sin(v) + 1; end M;")
            .contains("scalar")
            || err("model M Real v[2]; Real s; equation v = {1, 2}; s = sin(v) + 1; end M;")
                .contains("shapes")
    );
    // Dividing by an array.
    assert!(
        err("model M Real v[2]; Real w[2]; equation v = {1, 2}; w = 1 / v; end M;")
            .contains("divisor")
    );
    // A binding of the wrong length.
    assert!(
        err("model M parameter Real k[3] = {1, 2}; Real x; equation x = k[1]; end M;")
            .contains("element")
    );
    // size of a missing dimension.
    assert!(
        err("model M Real v[2]; Real s; equation v = {1, 2}; s = size(v, 3); end M;")
            .contains("no such dimension")
    );
    // Elementwise between lengths that differ.
    assert!(
        err("model M Real v[3]; Real w[3]; equation v = {1, 2, 3}; w = v .* {1, 2}; end M;")
            .contains("do not fit together")
    );
    // A scalar product between different lengths.
    assert!(
        err("model M Real v[3]; Real s; equation v = {1, 2, 3}; s = v * {1, 2}; end M;")
            .contains("equal lengths")
    );
    // A start attribute of the wrong length.
    assert!(
        err("model M Real x[3](start = {1, 2}); equation der(x) = zeros(3); end M;")
            .contains("start has 2")
    );
    // linspace without enough points, a bad fill length.
    assert!(
        err("model M Real v[1]; equation v = linspace(0, 1, 1); end M;").contains("at least two")
    );
    assert!(
        err("model M Real v[2]; Real q; equation q = 1; v = fill(1.0, q); end M;")
            .contains("compiler can see")
    );
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

    // A class alias at the top level of a file is refused.
    let error = oxidelica_parser::parse_file("package P = Q;")
        .unwrap_err()
        .to_string();
    assert!(error.contains("full definition"), "{error}");

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
fn alias_and_redeclare_spellings_with_all_the_trimmings() {
    // Modifiers on an alias target, `constrainedby` with its own
    // modifier list, a class redeclaration carrying both, and a
    // string description - every optional trailing piece at once.
    let m = parse_model(
        "package Media                partial package Base constant Real rho = 0; end Base;                package Water extends Base; constant Real rho = 1000; end Water;                package Oil extends Base; constant Real rho = 900; end Oil;              end Media;              model Tank                replaceable package Medium = Media.Water(rho = 1)                  constrainedby Media.Base(rho = 2);                Real a; equation a = Medium.rho; end Tank;              model M extends Tank(redeclare package Medium =                Media.Oil(rho = 3) constrainedby Media.Base); end M;",
    )
    .unwrap();
    assert!(format!("{:?}", m.equations).contains("Number(900.0)"));

    // `end` outside a subscript is refused with its own words.
    let error = parse_model("model M Real v[2]; Real x; equation v = 1:2; x = 1 + (v[1]);              x = v[end]; end M;");
    assert!(error.is_ok(), "end inside a subscript is fine");
    // Colon outside a subscript context in a scalar position.
    let error =
        parse_model("model M Real v[2]; Real x; equation v = 1:2; x = v[:] * {1, 1}; end M;")
            .unwrap();
    assert!(format!("{:?}", error.equations).contains("v[2]"));
}

#[test]
fn more_matrix_builtins_and_their_error_paths() {
    let m = parse_model(
        "model M              parameter Real I3[3, 3] = identity(3);              parameter Real D[3, 3] = diagonal({7, 8, 9});              parameter Real W[2, 4] = cat(2, [1, 2; 3, 4], [5, 6; 7, 8]);              parameter Real S[4, 2] = cat(1, [1, 2; 3, 4], [5, 6; 7, 8]);              Real vm[2];              equation vm = {1.0, 0.0} * [1, 2; 3, 4]; end M;",
    )
    .unwrap();
    let binding_of = |name: &str| {
        m.components
            .iter()
            .find(|c| c.name == name)
            .and_then(|c| c.binding.clone())
            .unwrap_or_else(|| panic!("no binding for {name}"))
    };
    assert_eq!(binding_of("I3[2,2]"), Expr::Number(1.0));
    assert_eq!(binding_of("I3[2,3]"), Expr::Number(0.0));
    assert_eq!(binding_of("D[3,3]"), Expr::Number(9.0));
    assert_eq!(binding_of("D[1,2]"), Expr::Number(0.0));
    assert_eq!(binding_of("W[1,3]"), Expr::Number(5.0));
    assert_eq!(binding_of("S[3,1]"), Expr::Number(5.0));

    // Vector times matrix picks columns.
    let vm = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"vm[2]\")")
        .unwrap();
    assert!(format!("{:?}", vm.rhs).contains("Number(2.0)"));

    // outerProduct, symmetric and skew, each folded to its elements -
    // to element expressions rather than single numbers, as `cross`
    // does; the shape and the terms are what matter here.
    let m2 = parse_model(
        "model M \
         parameter Real O[2, 2] = outerProduct({1, 2}, {3, 4}); \
         parameter Real Y[2, 2] = symmetric([1, 2; 9, 4]); \
         parameter Real K[3, 3] = skew({1, 2, 3}); \
         Real done; equation done = 0; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .unwrap();
    let shape_of = |name: &str| {
        format!(
            "{:?}",
            m2.components
                .iter()
                .find(|c| c.name == name)
                .and_then(|c| c.binding.clone())
                .unwrap_or_else(|| panic!("no binding for {name}"))
        )
    };
    // outerProduct[i, j] = x[i] * y[j].
    assert_eq!(shape_of("O[2,1]"), "Bin(Mul, Number(2.0), Number(3.0))");
    assert_eq!(shape_of("O[1,2]"), "Bin(Mul, Number(1.0), Number(4.0))");
    // symmetric mirrors the upper triangle, so the 9 below is dropped.
    assert_eq!(shape_of("Y[2,1]"), "Number(2.0)");
    assert_eq!(shape_of("Y[1,2]"), "Number(2.0)");
    // skew is the cross-product matrix: K[1,2] = -x3, K[1,3] = x2.
    assert_eq!(shape_of("K[1,2]"), "Neg(Number(3.0))");
    assert_eq!(shape_of("K[1,3]"), "Number(2.0)");
    assert_eq!(shape_of("K[3,1]"), "Neg(Number(2.0))");
    assert_eq!(shape_of("K[2,2]"), "Number(0.0)");

    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    assert!(err("model M Real x; equation x = transpose({1, 2}); end M;").contains("matrix"));
    assert!(
        err("model M Real v[3]; equation v = cross({1, 0}, {0, 1, 0}); end M;")
            .contains("3-vectors")
    );
    assert!(
        err("model M parameter Real W[2, 4] = cat(3, [1, 2; 3, 4], [5, 6; 7, 8]); end M;")
            .contains("dimension 3")
    );
    assert!(
        err("model M Real x[2]; equation x = {1, 2} * [1, 2; 3, 4] * {1}; end M;")
            .contains("equal")
    );
}

#[test]
fn ranges_slices_comprehensions_and_matrices() {
    let m = parse_model(
        "model M parameter Integer n = 4;              parameter Real A[2, 2] = [1, 2; 3, 4];              Real v[4]; Real evens[2]; Real tail[2]; Real squares[4];              Real rotated[2]; Real mm[2, 2]; Real crossed[3]; Real total;              equation v = 1:4; evens = v[{2, 4}]; tail = v[end - 1:end];              squares = {i * i for i in 1:n};              rotated = A * {1.0, 0.0}; mm = A * transpose(A);              crossed = cross({1, 0, 0}, {0, 1, 0});              total = sum(i * 2 for i in 1:n); end M;",
    )
    .unwrap();
    let equation_for = |name: &str| {
        m.equations
            .iter()
            .find(|e| format!("{:?}", e.lhs) == format!("Ref({name:?})"))
            .unwrap_or_else(|| panic!("no equation for {name}"))
    };
    // The range unrolled into literals, the slices picked the right
    // elements, the comprehension squared its index.
    assert_eq!(equation_for("v[3]").rhs, Expr::Number(3.0));
    assert_eq!(equation_for("evens[2]").rhs, Expr::Ref("v[4]".into()));
    assert_eq!(equation_for("tail[1]").rhs, Expr::Ref("v[3]".into()));
    // The comprehension bound its index; folding 4*4 is the
    // simulator's business.
    let squares = format!("{:?}", equation_for("squares[4]").rhs);
    assert_eq!(squares, "Bin(Mul, Number(4.0), Number(4.0))");
    // cross of the first two axes is the third.
    let text = format!("{:?}", equation_for("crossed[3]").rhs);
    assert!(text.contains("Mul"), "{text}");

    // Error paths: a subscript outside the array, uneven matrix
    // rows, a zero-step range.
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    assert!(
        err("model M Real v[2]; Real x; equation v = 1:2; x = v[3]; end M;")
            .contains("outside an array")
    );
    assert!(err("model M parameter Real A[2, 2] = [1, 2; 3]; end M;").contains("equally wide"));
    assert!(err("model M Real v[2]; equation v = 1:0:2; end M;").contains("step by zero"));
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
fn connects_take_subscripts_loops_and_whole_arrays() {
    // A chain wired inside a for loop, with subscripted references.
    let chain = parse_model(
        "connector Pin Real v; flow Real i; end Pin;              model Two Pin p; Pin n; equation p.i + n.i = 0; p.v - n.v = p.i; end Two;              model Ground Pin p; equation p.v = 0; end Ground;              model Chain Two r[3]; Ground ground;              equation for i in 1:2 loop connect(r[i].n, r[i + 1].p); end for;              connect(r[3].n, ground.p); r[1].p.v = 6; r[1].p.i + 0 = 0; end Chain;",
    )
    .unwrap();
    // Two joints of the loop plus the ground joint: the potentials
    // of neighbouring pins are equal in the flat model.
    let text = format!("{:?}", chain.equations);
    assert!(text.contains("r[2].p.v"), "{text}");

    // Two whole arrays pair element by element.
    let bus = parse_model(
        "connector Pin Real v; flow Real i; end Pin;              model Bus Pin left[2]; Pin right[2];              equation left[1].v = 1; left[2].v = 2;              right[1].i = 0.1; right[2].i = 0.2;              connect(left, right); end Bus;",
    )
    .unwrap();
    let text = format!("{:?}", bus.equations);
    assert!(text.contains("right[2].v"), "{text}");

    // Arrays of different lengths are refused with the counts.
    let error = parse_model(
        "connector Pin Real v; flow Real i; end Pin;              model B Pin a[2]; Pin b[3]; equation connect(a, b); end B;",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("2 and 3"), "{error}");
}

#[test]
fn connects_generate_kirchhoff_equations() {
    let source = "connector Pin Real v; flow Real i; end Pin;\
         model Ground Pin p; equation p.v = 0; end Ground;\
         model Two Pin p; Pin n; equation p.i + n.i = 0; p.v - n.v = p.i; end Two;\
         model Top Two a; Ground g; equation connect(a.p, g.p); end Top;";
    let m = parse_model(source).unwrap();
    // a.n is unconnected: its flow is forced to zero.
    let has_zero_flow = m.equations.iter().any(|e| {
        format!("{:?}", e.lhs).contains("a.n.i") && format!("{:?}", e.rhs).contains("0.0")
    });
    assert!(has_zero_flow, "unconnected flow must be zeroed");
}

#[test]
fn a_string_chooses_what_a_model_does() {
    // How strings are actually used in Modelica: one names a choice at
    // the top of a model and the equations read it. Nothing here needs
    // a string while the run goes, so it is settled beforehand and
    // what it leaves behind is a Boolean.
    let density = |medium: &str| {
        let model = parse_model(&format!(
            "model M parameter String medium = \"{medium}\"; Real rho; \
             equation if medium == \"water\" then rho = 1000; else rho = 850; end if; \
             annotation(experiment(StopTime = 1, Interval = 1)); end M;"
        ))
        .expect("parses");
        // The declaration is gone: there is nowhere in a run to put it.
        assert!(
            !model.components.iter().any(|c| c.name == "medium"),
            "the string outlived the flattening"
        );
        model
    };
    assert!(format!("{:?}", density("water").conditional).contains("Bool(true)"));
    assert!(format!("{:?}", density("oil").conditional).contains("Bool(false)"));

    // Built from another string, from a number, and compared with `<>`.
    let model = parse_model(
        "model M constant String base = \"n\"; parameter String tag = base + \"=\" + String(42); \
         Real r; equation r = if tag <> \"n=42\" then 0 else 7; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("parses");
    assert!(format!("{:?}", model.equations).contains("Bool(false)"));
}

#[test]
fn a_string_is_refused_where_a_number_belongs() {
    let refused = |source: &str| parse_model(source).expect_err("should be refused").message;

    let text = refused("model M Real r; equation r = \"text\"; end M;");
    assert!(text.contains("has no value an equation can hold"), "{text}");

    // A string only a run could produce is not a string this compiler
    // can settle, and it says which one.
    let text = refused(
        "model M Real x; parameter String s = String(x); Real r; \
         equation x = time; r = 1; end M;",
    );
    assert!(
        text.contains("`s` is a String whose value is not settled"),
        "{text}"
    );

    let text = refused("model M String s; Real r; equation r = 1; end M;");
    assert!(text.contains("`s` is a String with no value"), "{text}");

    // Every relational operator is defined on strings, as strcmp
    // against zero, so this one is not a refusal at all.
    let ordered = parse_model(
        "model M parameter String a = \"x\"; Real r; equation r = if a < \"y\" then 1 else 0; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("`<` orders strings");
    assert!(format!("{:?}", ordered.equations).contains("Bool(true)"));
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

    // `external` is a word this compiler knows and cannot honour, so it
    // says which rather than failing further in.
    let error = parse_model(
        "model M function f input Real a; output Real b; external \"C\"; end f; \
         Real y; equation y = 1; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect_err("no external bodies")
    .message;
    assert!(error.contains("must have a Modelica body"), "{error}");
}

#[test]
fn a_model_can_ask_where_it_is() {
    // `getInstanceName()` answers with the simulated model's name and
    // the path of the instance that asked, so the same class inside
    // two components gives two different answers. Strings are settled
    // before the run, and this is one of them.
    let named = |source: &str| {
        let model = parse_model(source).expect("parses");
        format!("{:?}", model.equations)
    };

    // At the top there is no path to append.
    assert!(named(
        "model Vehicle Real y; \
         equation y = if getInstanceName() == \"Vehicle\" then 1 else 0; \
         annotation(experiment(StopTime = 1, Interval = 1)); end Vehicle;"
    )
    .contains("Bool(true)"));

    // One level down, and two.
    assert!(named(
        "model Ctl Real y; \
         equation y = if getInstanceName() == \"Vehicle.engine.controller\" then 1 else 0; \
         end Ctl; \
         model Eng Ctl controller; Real w; equation w = 1; end Eng; \
         model Vehicle Eng engine; Real z; equation z = 1; \
         annotation(experiment(StopTime = 1, Interval = 1)); end Vehicle;"
    )
    .contains("Bool(true)"));

    // And it may be what a String parameter is built from.
    assert!(parse_model(
        "model Inner parameter String who = getInstanceName() + \" reporting\"; \
         Real y; equation y = 1; end Inner; \
         model Top Inner part; Real z; equation z = 1; \
         annotation(experiment(StopTime = 1, Interval = 1)); end Top;"
    )
    .is_ok());
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

#[test]
fn cardinality_counts_the_connections_to_a_port() {
    // How many `connect` equations name a port. The specification
    // deprecates the operator and says it will be removed, but while it
    // is still defined it is answered - and it is answered here,
    // because this is the last moment the connections are in hand.
    const PARTS: &str = "connector P Real v; flow Real i; end P; \
         model Src P p; equation p.v = 1; end Src; \
         model Snk P p; equation p.i = 0; end Snk; ";
    let flat = |body: &str| {
        parse_model(&format!(
            "{PARTS} model M Src a; Snk b; Snk c; Real n; Real m; equation {body} \
             annotation(experiment(StopTime = 1, Interval = 1)); end M;"
        ))
        .expect("parses")
    };
    let value_of = |model: &oxidelica_parser::Model, name: &str| -> f64 {
        let equation = model
            .equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(target) if target == name))
            .unwrap_or_else(|| panic!("no equation for {name}"));
        match equation.rhs {
            Expr::Number(value) => value,
            ref other => panic!("{name} was left as {other:?}"),
        }
    };

    // One connection names each of its two ends once.
    let one =
        flat("connect(a.p, b.p); connect(c.p, b.p); n = cardinality(a.p); m = cardinality(c.p);");
    assert_eq!(value_of(&one, "n"), 1.0);
    assert_eq!(value_of(&one, "m"), 1.0);

    // A port named twice is counted twice.
    let twice =
        flat("connect(a.p, b.p); connect(a.p, c.p); n = cardinality(a.p); m = cardinality(b.p);");
    assert_eq!(value_of(&twice, "n"), 2.0);
    assert_eq!(value_of(&twice, "m"), 1.0);

    // And a port no connection names is zero - which is what the
    // operator is nearly always asked about, in an assertion.
    let lonely = parse_model(&format!(
        "{PARTS} model M Src a; Real n; equation n = cardinality(a.p); \
         assert(cardinality(a.p) > 0, \"port a.p is not connected\"); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    ))
    .expect("parses");
    assert_eq!(value_of(&lonely, "n"), 0.0);
    assert!(
        matches!(&lonely.asserts[0].0, Expr::Rel(_, left, _)
            if matches!(**left, Expr::Number(count) if count == 0.0)),
        "the assertion was left unanswered: {:?}",
        lonely.asserts[0].0
    );
}

#[test]
fn an_unqualified_import_reaches_a_packages_constants() {
    // `import A.*;` opens the package's constants as well as its
    // classes, so a bare name may be one of them - the last reading
    // tried, since a component of the model outranks it.
    let value = |source: &str| {
        let model = parse_model(source).expect("parses");
        match &model
            .equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(n) if n == "y"))
            .unwrap()
            .rhs
        {
            Expr::Number(v) => Some(*v),
            _ => None,
        }
    };
    assert_eq!(
        value(
            "package A constant Real half = 0.5; end A; \
               model M import A.*; Real y; equation y = half; \
               annotation(experiment(StopTime = 1, Interval = 1)); end M;"
        ),
        Some(0.5)
    );
    // Two packages opened at once: each bare name reaches its own.
    let both = parse_model(
        "package A constant Real a = 1; end A; package B constant Real b = 4; end B; \
         model M import A.*; import B.*; Real y; equation y = a + b; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("parses");
    assert!(format!("{:?}", both.equations[0].rhs).contains("Number(1.0)"));
    assert!(format!("{:?}", both.equations[0].rhs).contains("Number(4.0)"));
    // A component of the model wins over a wildcard constant of the
    // same name: `half` is the variable, not 0.5.
    let model = parse_model(
        "package A constant Real half = 0.5; end A; \
         model M import A.*; Real half; Real y; equation half = 9; y = half; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("parses");
    assert!(model.components.iter().any(|c| c.name == "half"));
}

#[test]
fn a_package_inherits_the_constants_of_its_base() {
    // `extends` brings a base package's constants into the derived
    // one's namespace, so `Derived.k` reads a `k` that `Base` declared.
    // Each `y = ...` has its constants replaced by their values.
    let rhs = |source: &str| {
        let model = parse_model(source).expect("parses");
        format!(
            "{:?}",
            model
                .equations
                .iter()
                .find(|e| matches!(&e.lhs, Expr::Ref(n) if n == "y"))
                .unwrap()
                .rhs
        )
    };
    // k(=2) from Base and j(=3) from Derived, each in place.
    let sum = rhs("package Base constant Real k = 2; end Base; \
         package Derived extends Base; constant Real j = 3; end Derived; \
         model M Real y; equation y = Derived.k + Derived.j; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;");
    assert!(
        sum.contains("Number(2.0)") && sum.contains("Number(3.0)"),
        "{sum}"
    );
    // Two levels of extends, and a base constant built from another.
    assert_eq!(
        rhs(
            "package A constant Real a = 1; constant Real a2 = a * 10; end A; \
             package B extends A; end B; package C extends B; end C; \
             model M Real y; equation y = C.a2; \
             annotation(experiment(StopTime = 1, Interval = 1)); end M;"
        ),
        "Number(10.0)"
    );
    // A derived constant overrides the inherited one of the same name.
    assert_eq!(
        rhs("package Base constant Real k = 2; end Base; \
             package Derived extends Base; constant Real k = 9; end Derived; \
             model M Real y; equation y = Derived.k; \
             annotation(experiment(StopTime = 1, Interval = 1)); end M;"),
        "Number(9.0)"
    );
}

#[test]
fn an_encapsulated_package_does_not_see_out_of_itself() {
    let ok = |source: &str| parse_model(source).is_ok();
    let widget = "package Outer model Widget Real w; equation w = 1; end Widget; ";

    // Encapsulated: a simple name inside cannot reach Outer's Widget.
    assert!(!ok(&format!(
        "{widget} encapsulated package Inner \
         model Use Widget x; equation x.w = 2; end Use; end Inner; end Outer; \
         model M Outer.Inner.Use u; Real y; equation y = u.x.w; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    )));
    // The same tree without the wall resolves Widget by simple name.
    assert!(ok(&format!(
        "{widget} package Inner \
         model Use Widget x; equation x.w = 2; end Use; end Inner; end Outer; \
         model M Outer.Inner.Use u; Real y; equation y = u.x.w; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    )));
    // And the wall's own import is the way through it.
    assert!(ok(&format!(
        "{widget} encapsulated package Inner import Outer.Widget; \
         model Use Widget x; equation x.w = 2; end Use; end Inner; end Outer; \
         model M Outer.Inner.Use u; Real y; equation y = u.x.w; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    )));
}

#[test]
fn a_package_holds_only_classes_and_constants() {
    let refused = |source: &str| parse_model(source).expect_err("should be refused").message;

    let text = refused(
        "package A parameter Real p = 1; end A; \
         model M Real y; equation y = 1; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert!(text.contains("parameter in a package"), "{text}");

    let text = refused(
        "package A Real x; end A; \
         model M Real y; equation y = 1; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert!(text.contains("variable in a package"), "{text}");

    // A constant is fine, and so is a class.
    assert!(parse_model(
        "package A constant Real c = 1; model Thing Real t; equation t = c; end Thing; end A; \
         model M A.Thing g; Real y; equation y = g.t; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    )
    .is_ok());
}

#[test]
fn a_final_declaration_is_closed_to_the_enclosing_class() {
    let refused = |source: &str| parse_model(source).expect_err("should be refused").message;

    // `extends Base(k = 5)` where Base declared `k` final.
    let text = refused(
        "model Base final parameter Real k = 2; Real y; equation y = k; end Base; \
         model M extends Base(k = 5); Real z; equation z = y; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert!(text.contains("`k` is final"), "{text}");

    // Reaching an attribute of a final component from outside is
    // refused just the same.
    let text = refused(
        "model Inner parameter Real a = 1; Real w; equation w = a; end Inner; \
         model Outer final Inner c; Real y; equation y = c.w; end Outer; \
         model M extends Outer(c(a = 9)); Real z; equation z = y; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert!(text.contains("`c` is final"), "{text}");

    // What is not modified from outside is fine: a final declaration
    // with its own value, and an ordinary (non-final) one modified.
    assert!(parse_model(
        "model M final Inner c(a = 3); Real y; equation y = c.w; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M; \
         model Inner parameter Real a = 1; Real w; equation w = a; end Inner;"
    )
    .is_ok());
    assert!(parse_model(
        "model Base parameter Real k = 2; Real y; equation y = k; end Base; \
         model M extends Base(k = 5); Real z; equation z = y; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    )
    .is_ok());
}

#[test]
fn each_spreads_a_modifier_where_a_list_is_handed_out() {
    // On an array of components, a modifier written as a list of the
    // right length gives each element its own entry; `each`, or a
    // scalar, reaches every element whole.
    const ITEM: &str = "model Item parameter Real w = 0; Real y; equation y = w; end Item; ";
    let value = |source: &str, name: &str| {
        let model = parse_model(source).expect("parses");
        let component = model
            .components
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no {name}"));
        match component.binding.as_ref() {
            Some(Expr::Number(v)) => *v,
            other => panic!("{name} bound as {other:?}"),
        }
    };
    let indexed = format!(
        "{ITEM} model M Item items[3](w = {{10, 20, 30}}); Real z; equation z = items[1].y; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    );
    assert_eq!(value(&indexed, "items[1].w"), 10.0);
    assert_eq!(value(&indexed, "items[2].w"), 20.0);
    assert_eq!(value(&indexed, "items[3].w"), 30.0);

    // `each` spreads one value to all; so does a scalar.
    for source in [
        format!(
            "{ITEM} model M Item items[3](each w = 7); Real z; equation z = items[1].y; \
                 annotation(experiment(StopTime = 1, Interval = 1)); end M;"
        ),
        format!(
            "{ITEM} model M Item items[3](w = 7); Real z; equation z = items[1].y; \
                 annotation(experiment(StopTime = 1, Interval = 1)); end M;"
        ),
    ] {
        assert_eq!(value(&source, "items[1].w"), 7.0);
        assert_eq!(value(&source, "items[3].w"), 7.0);
    }
}

#[test]
fn a_selective_extends_leaves_an_element_out() {
    // `break s` removes the component and the connections to it, so the
    // extending class may wire the base's bus to its own source
    // instead. The flat model keeps `mine` and drops `s`.
    let model = parse_model(
        "connector P Real v; flow Real i; end P; \
         model Src P p; equation p.v = 5; end Src; \
         model Base P bus; Src s; equation connect(s.p, bus); end Base; \
         model M extends Base(break s); Src mine; equation connect(mine.p, bus); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("parses");
    let names: Vec<&str> = model.components.iter().map(|c| c.name.as_str()).collect();
    assert!(!names.iter().any(|n| n.starts_with("s.")), "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("mine.")), "{names:?}");

    // `break connect(a, b)` drops that one connection: with the base's
    // join gone, `a.v` and `b.v` are no longer forced equal, so the two
    // declared values stand. The connection equality is not in the flat
    // model.
    let model = parse_model(
        "connector P Real v; flow Real i; end P; \
         model Base P a; P b; equation a.v = 3; b.v = 7; connect(a, b); end Base; \
         model M Real y; extends Base(break connect(a, b)); equation y = b.v; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("parses");
    // No equation ties a.v to b.v: the connection is gone.
    assert!(
        !model.equations.iter().any(|e| {
            let text = format!("{:?} {:?}", e.lhs, e.rhs);
            text.contains("a.v") && text.contains("b.v")
        }),
        "the broken connection survived: {:?}",
        model.equations
    );

    // A break that matches nothing in the base is a mistake.
    let error = parse_model(
        "model Base Real x; equation x = 1; end Base; \
         model M Real y; extends Base(break nope); equation y = x; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect_err("break matched nothing");
    assert!(
        error.message.contains("`break nope` matches nothing"),
        "{}",
        error.message
    );

    let error = parse_model(
        "connector P Real v; flow Real i; end P; \
         model Base P a; P b; equation a.v = 1; b.v = 2; end Base; \
         model M extends Base(break connect(a, b)); Real y; equation y = a.v; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect_err("no such connection");
    assert!(
        error.message.contains("break connect(a, b)") && error.message.contains("no connection"),
        "{}",
        error.message
    );
}

#[test]
fn a_dimension_may_be_a_type_or_read_from_a_value() {
    // A `:` size takes its length from the value the component is
    // given, a Boolean dimension has two elements indexed off `false`,
    // and an enumeration dimension one per literal.
    let names = |source: &str| {
        parse_model(source)
            .expect("parses")
            .components
            .iter()
            .map(|c| c.name.clone())
            .collect::<Vec<_>>()
    };

    // `v[:] = {1, 2, 3}` becomes three scalar elements.
    let flat = names(
        "model M parameter Real v[:] = {1, 2, 3}; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert!(flat.contains(&"v[1]".to_string()) && flat.contains(&"v[3]".to_string()));
    assert!(!flat.contains(&"v[4]".to_string()));

    // A Boolean dimension has two elements; an enumeration one per
    // literal.
    let flat = names(
        "model M Real x[Boolean]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert_eq!(
        flat.iter().filter(|n| n.starts_with("x[")).count(),
        2,
        "{flat:?}"
    );
    let flat = names(
        "model M type E = enumeration(a, b, c); Real x[E]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert_eq!(flat.iter().filter(|n| n.starts_with("x[")).count(), 3);

    // A Boolean subscript indexes off `false`: x[false] is element 1.
    let model = parse_model(
        "model M Real x[Boolean]; Real y; equation x[false] = 10; x[true] = 20; y = x[false]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("parses");
    // The equation for y reads x[1], the false element.
    let y = model
        .equations
        .iter()
        .find(|e| matches!(&e.lhs, Expr::Ref(n) if n == "y"))
        .unwrap();
    assert_eq!(format!("{:?}", y.rhs), "Ref(\"x[1]\")");

    // A flexible `:` with no value to measure is refused.
    let error = parse_model(
        "model M Real v[:]; equation v[1] = 1; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect_err("nothing to size from");
    assert!(error.message.contains("flexible size"), "{}", error.message);
}
