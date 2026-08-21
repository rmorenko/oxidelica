//! Flattening, asked of the crate from outside: a source string in, a
//! flat model out.
//!
//! These are the language's own rules - inheritance, arrays, packages,
//! connections, clocks, state machines - checked through `parse_model`,
//! which is how everything but the compiler itself sees this crate.

use oxidelica_parser::ast::BinOp;
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
            .contains("is used where a scalar is expected")
    );
    assert!(err("model M Real y; discrete Real d(start = 0); equation y = time; when 1:3 then d = 1; end when; end M;")
        .contains("is used where a scalar is expected"));

    assert!(err("model M Real v[2]; equation v = zeros({2}); end M;")
        .contains("is used where a scalar is expected"));
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
fn an_annotation_is_read_rather_than_stepped_over() {
    // An annotation is a tree of `name = value`, and a value is an
    // expression - a number, a string, a list, a call with named
    // arguments - so what is kept is what the expression parser reads.
    let classes = oxidelica_parser::parse_file(
        "model M \
         parameter Real k = 2 annotation(Dialog(group = \"Main\"), Evaluate = true); \
         Real y; equation y = k; \
         annotation(Icon(graphics = {Line(points = {{0, 0}, {1, 1}}), \
         Text(extent = {{-1, -1}, {1, 1}}, textString = \"%name\")}), \
         Documentation(info = \"<html>what it does</html>\"), version = \"1.0\"); end M;",
    )
    .unwrap();
    let info = oxidelica_parser::class_info(&classes, "M").unwrap();
    let written = format!("{:?}", info.annotations);
    for part in ["Icon", "Line", "Text", "%name", "Documentation", "version"] {
        assert!(written.contains(part), "{part} in {written}");
    }
    // On a declaration too.
    let k = classes[0]
        .components
        .iter()
        .find(|component| component.name == "k")
        .unwrap();
    let said = format!("{:?}", k.annotations);
    assert!(said.contains("Dialog") && said.contains("Main"), "{said}");
    assert!(said.contains("Evaluate"), "{said}");

    // What the expression parser cannot read is stepped over rather
    // than refused: an annotation says things to tools, and one it does
    // not understand must not stop it. The rest of the annotation is
    // still read.
    let odd = oxidelica_parser::parse_file(
        "model M Real y; equation y = 1; \
         annotation(__vendor(thing = = ), version = \"2.0\"); end M;",
    )
    .unwrap();
    let kept = format!("{:?}", odd[0].annotations);
    assert!(kept.contains("version") && kept.contains("2.0"), "{kept}");
}

#[test]
fn an_annotation_that_the_chapter_calls_an_error_is_one() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    let wired = |said: &str, connections: &str| {
        format!(
            "model M connector Pin Real v; flow Real i; end Pin; \
             model Node Pin p{said}; Real x; equation p.i = 0; x = p.v; end Node; \
             model Src Pin p; equation p.v = 5; end Src; \
             Node a; Src b; Src c; equation {connections} end M;"
        )
    };
    // `mustBeConnected`: 18.8 says it makes it an error, and the
    // message the declaration wrote is what the error says.
    assert!(err(&wired(
        " annotation(mustBeConnected = \"a node has to be wired in\")",
        "a.x = 1;"
    ))
    .contains("must be connected: a node has to be wired in"));
    assert!(parse_model(&wired(
        " annotation(mustBeConnected = \"wired\")",
        "connect(a.p, b.p);"
    ))
    .is_ok());
    // `mayOnlyConnectOnce`: the same, counted the other way.
    assert!(err(&wired(
        " annotation(mayOnlyConnectOnce = \"one wire to a node\")",
        "connect(a.p, b.p); connect(a.p, c.p);"
    ))
    .contains("may only be connected once: one wire to a node"));
    assert!(parse_model(&wired(
        " annotation(mayOnlyConnectOnce = \"one wire\")",
        "connect(a.p, b.p);"
    ))
    .is_ok());

    // `Evaluate = true` asks for a parameter the compiler settles.
    assert!(parse_model(
        "model M parameter Real p = 3 annotation(Evaluate = true); Real y; \
         equation y = p; end M;"
    )
    .is_ok());
    // A parameter with nothing to take its value from cannot be
    // settled, so asking for it to be is asking for what did not
    // happen.
    assert!(err("model M parameter Real q; \
         parameter Real p = q annotation(Evaluate = true); Real y; \
         equation y = p; end M;")
    .contains("asks to be evaluated before the run"));
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
    // The condition folded and its branch was taken with it: `wide`
    // and the package constant are both known before the run.
    assert!(
        format!("{:?}", q.binding) == "Some(Number(1.0))",
        "{:?}",
        q.binding
    );
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
    // size of a missing dimension, said with the shape it does have.
    assert!(
        err("model M Real v[2]; Real s; equation v = {1, 2}; s = size(v, 3); end M;")
            .contains("is of shape [2]")
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

/// The forms the standard library is written in that a smaller slice of
/// the language does without: the long shape of `type`, a connector
/// that is a predefined type, checks written inside a branch, and an
/// `if` equation with another one inside it.
#[test]
fn the_shapes_the_standard_library_is_written_in() {
    // `type X ... extends Real; ... end X;` - the long form, which is
    // what the standard library's icon package uses. It names a type
    // exactly as `type X = Real(...)` does.
    let model = parse_model(
        "type Level \"a level\" extends Real; annotation(Icon()); end Level; \
         model M Level x(start = 3); Real y; equation y = x; der(x) = 0; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("the long form of a type");
    let x = model.components.iter().find(|c| c.name == "x").unwrap();
    assert_eq!(x.type_name, "Real");
    assert!(matches!(x.start, Some(Expr::Number(n)) if n == 3.0));

    // `connector RealInput = input Real` - a connector that holds one
    // predefined value, which is how every signal in the standard
    // library is carried. `final` and `each` on an attribute belong to
    // the declaration and change nothing about the value.
    let model = parse_model(
        "connector Signal = input Real(final unit = \"V\", each min = -1); \
         model M Signal u; Real y; equation u = 2; y = u; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a connector of a predefined type");
    let u = model.components.iter().find(|c| c.name == "u").unwrap();
    assert_eq!(u.type_name, "Real");
    assert_eq!(u.unit.as_deref(), Some("V"));

    // An `if` equation inside another one. The branches are read into
    // one chain, with the conditions of the two joined: `k == 2` picks
    // the inner `else`, so y is 20 and not 10 or 30.
    let model = parse_model(
        "model M constant Integer k = 2; Real y; equation \
         if k == 1 then y = 1; elseif k == 2 then \
           if false then y = 10; else y = 20; end if; \
         else y = 30; end if; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an if inside an if");
    assert_eq!(model.equations.len(), 1);
    assert_eq!(format!("{:?}", model.equations[0].rhs), "Number(20.0)");

    // An inner chain with no `else` covers only part of the branch it
    // is written in: with the inner condition false, nothing is
    // defined by it, and the branch after must not be reached.
    let model = parse_model(
        "model M constant Integer k = 2; Real y; equation \
         if k == 1 then y = 1; elseif k == 2 then \
           if false then y = 10; end if; y = 20; \
         else y = 30; end if; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an inner chain with no else");
    assert_eq!(model.equations.len(), 1);
    assert_eq!(format!("{:?}", model.equations[0].rhs), "Number(20.0)");

    // A `for` equation inside a branch the compiler picks.
    let model = parse_model(
        "model M constant Boolean wide = true; Real v[3]; equation \
         if wide then for i in 1:3 loop v[i] = i; end for; \
         else for i in 1:3 loop v[i] = 0; end for; end if; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a for inside an if");
    assert_eq!(model.equations.len(), 3);
    assert_eq!(format!("{:?}", model.equations[2].rhs), "Number(3.0)");
}

/// Checks written inside an `if` equation, and the message one carries.
#[test]
fn a_check_inside_a_branch_holds_only_while_the_branch_does() {
    // The condition is one only the run holds, so no branch is picked
    // here and each check comes out guarded. The `else` branch's check
    // is guarded by the denial of the first condition, which is what
    // keeps it from firing while the first branch holds.
    let model = parse_model(
        "model M Real y; equation \
         if time > 0.5 then assert(time > -1, \"while loaded\"); y = 1; \
         else assert(time < -1, \"while idle\"); y = 2; end if; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("checks in branches");
    assert_eq!(model.asserts.len(), 2);
    let idle = model
        .asserts
        .iter()
        .find(|(_, message)| message == "while idle")
        .expect("the check of the else branch");
    // `not (not loaded) or time < -1` - written as the denial of the
    // guard, which is itself the denial of the first condition.
    let written = format!("{:?}", idle.0);
    assert!(written.starts_with("Or(Not(Not("), "{written}");

    // A message built by joining pieces keeps the parts that are text.
    // A warning is read and dropped: the language says the run carries
    // on, and there is nowhere here to put the text.
    let model = parse_model(
        "model M Real y; equation y = 1; \
         assert(y > 0, \"y is \" + String(y) + \", which is wrong\"); \
         assert(y > 2, \"only a warning\", AssertionLevel.warning); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a built message");
    assert_eq!(model.asserts.len(), 1);
    assert_eq!(model.asserts[0].1, "y is ?, which is wrong");
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

/// The forms a library writes that a smaller slice of the language did
/// without, and what each of them is refused for when it cannot be
/// honoured.
#[test]
fn the_library_forms_are_read_and_their_limits_named() {
    // A short class definition may repeat any of the prefixes a
    // declaration carries; none of them changes the type itself.
    for prefix in ["input", "output", "flow", "stream", "discrete"] {
        let m = parse_model(&format!(
            "connector Signal = {prefix} Real; model M Signal u; Real y; \
             equation u = 2; y = u; \
             annotation(experiment(StopTime = 1, Interval = 1)); end M;"
        ))
        .unwrap_or_else(|e| panic!("`{prefix}`: {e}"));
        assert!(m.components.iter().any(|c| c.name == "u"));
    }

    // An `initial algorithm` settles where the model starts; a `when`
    // among its statements would be an event, and there are none
    // before the run begins.
    let error = parse_model(
        "model M Real x; initial algorithm when time > 1 then x := 1; end when; \
         equation der(x) = 1; end M;",
    )
    .expect_err("no events before the start")
    .message;
    assert!(error.contains("not an initial one"), "{error}");

    // A `for` equation in a branch the run decides would make the
    // model a different size depending on the run.
    let error = parse_model(
        "model M Real v[2]; equation if time > 1 then for i in 1:2 loop v[i] = i; end for; \
         else for i in 1:2 loop v[i] = 0; end for; end if; end M;",
    )
    .expect_err("a loop in an undecided branch")
    .message;
    assert!(error.contains("settled before the run"), "{error}");

    // A `connect` in one is structural in the same way.
    let error = parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         model Part Pin p; equation p.i = 0; end Part; \
         model M Part a; Part b; equation if time > 1 then connect(a.p, b.p); \
         else connect(a.p, b.p); end if; end M;",
    )
    .expect_err("a connection in an undecided branch")
    .message;
    assert!(error.contains("connections are structural"), "{error}");

    // A call standing on its own has to name a function.
    let error = parse_model("model M Real y; algorithm sqrt(2); y := 1; end M;")
        .expect_err("not a function")
        .message;
    assert!(error.contains("is not a function"), "{error}");

    // An assertion level other than a warning is held as written.
    let m = parse_model(
        "model M Real y; equation y = 1; \
         assert(y > 0, \"positive\", level = AssertionLevel.error); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an error-level check");
    assert_eq!(m.asserts.len(), 1);

    // A library file that will not parse is set aside rather than made
    // everyone's problem: the model beside it still loads, and what
    // was not read is said.
    let (model, unread) = oxidelica_parser::parse_model_reading(
        &["package Broken model B Real x @ 1; end B; end Broken;".to_string()],
        "model M Real y; equation y = 1; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert!(model.is_ok());
    assert_eq!(unread.len(), 1);
    assert!(unread[0].contains("line"), "{}", unread[0]);
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

/// Arrays chosen by a condition the run holds, and the shapes a body
/// may build on the way to a number.
#[test]
fn an_array_may_be_chosen_by_a_condition_the_run_holds() {
    // `if time > t then a else b` over arrays: neither branch is
    // picked here, so the choice is made element by element.
    let m = parse_model(
        "model M Real a[2]; Real b[2]; Real v[2]; \
         equation a = {1, 2}; b = {3, 4}; v = if time > 0.5 then a else b; \
         annotation(experiment(StopTime = 1, Interval = 0.5)); end M;",
    )
    .expect("arrays under a condition");
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("v["))
            .count(),
        2
    );
    let written = format!("{:?}", m.equations);
    assert_eq!(written.matches("If(").count(), 2, "{written}");

    // A matrix literal and a named argument, both standing where an
    // earlier statement has already bound what they are built on.
    let m = parse_model(
        "model M function blend input Real k; input Real scale = 1; output Real y; \
         algorithm y := (k + k) * scale; end blend; \
         Real y; equation y = blend(sum({{1, 2}, {3, 4}}), scale = 2); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a matrix and a named argument");
    // (1+2+3+4) doubled and scaled by 2 is 40; what is left standing
    // is the sum written out, which comes to the same.
    let written = format!("{:?}", m.equations[0].rhs);
    assert_eq!(written.matches("Number(4.0)").count(), 2, "{written}");
    assert!(written.ends_with("Number(2.0))"), "{written}");
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

/// Where the dimensions of an array may be written: on the name, on
/// the type of the declaration, and on the type itself.
#[test]
fn an_array_may_take_its_shape_from_its_type() {
    // `Foo[n] x` is `Foo x[n]` - the standard library writes both.
    let m = parse_model(
        "model Cell Real k = 2; end Cell; \
         model M Cell[3] cells; Real y; equation y = cells[2].k; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("dimensions on the type of a declaration");
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("cells["))
            .count(),
        3
    );

    // One clause may declare several, each with its own dimensions and
    // its own value.
    let m = parse_model(
        "model M parameter Real a = 1, b[2] = {2, 3}, c = 4; Real y; \
         equation y = a + b[2] + c; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("several components in one clause");
    assert!(m.components.iter().any(|c| c.name == "b[2]"));
    assert_eq!(m.components.iter().filter(|c| c.name == "c").count(), 1);

    // A type that is an array lends its shape to whatever is declared
    // with it, and a chain of them still ends at the primitive.
    let m = parse_model(
        "type Row = Real[3](each unit = \"1\"); type Axis = Row; \
         model M parameter Axis n = {0, -1, 0}; Real y; equation y = n[2]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a type that is an array");
    let second = m.components.iter().find(|c| c.name == "n[2]").unwrap();
    assert_eq!(second.type_name, "Real");
    assert_eq!(second.unit.as_deref(), Some("1"));

    // The declaration's own dimensions come first: `Axis o[2]` is a
    // pair of axes, not three pairs.
    let m = parse_model(
        "type Axis = Real[3]; \
         model M parameter Axis o[2] = {{1, 2, 3}, {4, 5, 6}}; Real y; \
         equation y = o[2, 3]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an array of arrays");
    assert!(m.components.iter().any(|c| c.name == "o[2,3]"));
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("o["))
            .count(),
        6
    );

    // A malformed dimension list says so, in either place.
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    assert!(err("model M Real[2; 3] v; end M;").contains("array dimensions"));
    assert!(err("model M Real v[2; 3]; end M;").contains("array dimensions"));
    assert!(err("type T = Real[2; 3]; model M T v; end M;").contains("dimensions of a type"));
    // And a clause that ends in neither a comma nor a semicolon.
    assert!(err("model M Real a = 1 Real b; end M;").contains("after a declaration"));
}

/// `StateSelect`, and the slice of a modifier an element of an array
/// component is handed.
#[test]
fn the_language_supplies_state_select_and_an_array_takes_its_slice() {
    // `StateSelect` is the language's own enumeration: a model may
    // declare a parameter of it and name its literals, with no library
    // defining either.
    let m = parse_model(
        "model M parameter StateSelect pick = StateSelect.prefer; \
         Real x(start = 1, fixed = true, stateSelect = StateSelect.always); Real y; \
         equation der(x) = -x; y = if pick == StateSelect.prefer then 1 else 0; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("the built-in enumeration");
    let pick = m.components.iter().find(|c| c.name == "pick").unwrap();
    // The literals are ordered from the least to the most insistent,
    // so `prefer` is the fourth of the five.
    assert_eq!(pick.type_name, "Integer");
    assert!(matches!(pick.binding, Some(Expr::Number(n)) if n == 4.0));

    // A library may still define one of its own under its own name.
    let m = parse_model(
        "package P type StateSelect = enumeration(only, one); end P; \
         model M parameter P.StateSelect pick = P.StateSelect.one; Real y; \
         equation y = pick; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a library's own");
    let pick = m.components.iter().find(|c| c.name == "pick").unwrap();
    assert!(matches!(pick.binding, Some(Expr::Number(n)) if n == 2.0));

    // `cells(k = ks)` hands `cells[i].k` the value `ks[i]`, whether the
    // array is written out or named.
    let m = parse_model(
        "model Cell parameter Real k = 0; Real v; equation v = k * time; end Cell; \
         model M parameter Real ks[3] = {1, 2, 3}; Cell cells[3](k = ks); \
         Cell fixed[2](each k = 7); Real y; equation y = cells[2].v; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a named array as a modifier");
    let value = |name: &str| {
        m.components
            .iter()
            .find(|c| c.name == name)
            .and_then(|c| c.binding.clone())
            .map(|b| format!("{b:?}"))
            .unwrap_or_default()
    };
    assert_eq!(value("cells[1].k"), "Ref(\"ks[1]\")");
    assert_eq!(value("cells[3].k"), "Ref(\"ks[3]\")");
    // `each` spreads one value over every element instead of slicing.
    assert_eq!(value("fixed[1].k"), "Number(7.0)");
    assert_eq!(value("fixed[2].k"), "Number(7.0)");
}

/// A member read across an array of components, and a dimension
/// written on a parameter a base class brought.
#[test]
fn a_member_may_be_read_across_an_array_of_components() {
    // `plug.pin.v` is the array of the `v` of each pin. Nothing in the
    // text of `M` says how long `plug.pin` is - it belongs to another
    // class - so the length comes from what has been instantiated.
    let m = parse_model(
        "connector Pin Real v; end Pin; \
         model Plug parameter Integer m = 3; Pin pin[m]; end Plug; \
         model M Plug plug; Real v[3]; equation v = plug.pin.v; \
         plug.pin.v = {1, 2, 3}; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a member across an array");
    let written = format!("{:?}", m.equations);
    assert!(written.contains("plug.pin[1].v"), "{written}");
    assert!(written.contains("plug.pin[3].v"), "{written}");
    assert_eq!(m.equations.len(), 6);

    // The length of that array may be settled from outside.
    let m = parse_model(
        "connector Pin Real v; end Pin; \
         model Plug parameter Integer m = 3; Pin pin[m]; end Plug; \
         model M Plug plug(m = 2); Real v[2]; equation v = plug.pin.v; \
         plug.pin.v = {1, 2}; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a length settled from outside");
    assert_eq!(m.equations.len(), 4);

    // A base class's parameter is this class's too, and a dimension
    // may be written with it - as may a `fill` in a value, which is
    // read after the declaration has taken this instance's prefix.
    let m = parse_model(
        "model Base parameter Integer m = 3; parameter Real v[m] = fill(2, m); end Base; \
         model Middle extends Base(m = 2); Real w[m]; equation w = v * time; end Middle; \
         model M Middle mid; Real y; equation y = mid.w[2]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a dimension on an inherited parameter");
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("mid.v["))
            .count(),
        2
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

/// A modifier on one base written with a parameter another base
/// brought: by the time the second is reached, the first has said what
/// the parameter is worth.
#[test]
fn a_base_may_be_modified_with_what_another_base_settled() {
    // `extends Heat(T = fill(20, m))` in a class whose `m` comes from
    // `Phases`. The length is not in either base's own text, and it is
    // known all the same.
    let m = parse_model(
        "model Phases parameter Integer m = 3; end Phases; \
         model Heat parameter Integer mh = 1; parameter Real T[mh] = zeros(mh); end Heat; \
         model Diode extends Phases; extends Heat(final mh = m, final T = fill(20, m)); \
         end Diode; \
         model M Diode d(m = 4); Real y; equation y = d.T[2]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a modifier reaching across bases");
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("d.T["))
            .count(),
        4
    );
    let second = m.components.iter().find(|c| c.name == "d.T[2]").unwrap();
    assert!(matches!(second.binding, Some(Expr::Number(n)) if n == 20.0));

    // And a length nothing can see is still said to be one, with the
    // expression that was asked for.
    let error = parse_model("model M Real n; Real v[3]; equation n = time; v = fill(1, n); end M;")
        .expect_err("a length the run holds")
        .message;
    assert!(
        error.contains("needs a length the compiler can see"),
        "{error}"
    );
    assert!(error.contains("Ref(\"n\")"), "{error}");
}

/// A parameter built on an element of a constant array, and a clock
/// whose factor is one.
#[test]
fn a_parameter_may_be_built_on_an_element_of_a_table() {
    // The table a class builds before it instantiates anything knows a
    // whole array by one name; an element of it is worth a number only
    // once the elements are declarations of their own. `Evaluate` says
    // it has to be worth one.
    let m = parse_model(
        "model M type Resolution = enumeration(s, ms); \
         parameter Resolution resolution = Resolution.ms annotation(Evaluate = true); \
         constant Integer table[2] = {1, 1000}; \
         parameter Integer factor = table[Integer(resolution)] annotation(Evaluate = true); \
         Real y; equation y = factor * time; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a parameter off a table");
    // Every element of the table is a number of its own, so the
    // binding comes out as the one it picked - which is what
    // `Evaluate` asked for.
    let factor = m.components.iter().find(|c| c.name == "factor").unwrap();
    assert_eq!(
        format!("{:?}", factor.binding.as_ref().unwrap()),
        "Number(1000.0)"
    );

    // And a clock built the same way: the interval is a factor read
    // out of the table, over a resolution read out of it too.
    let m = parse_model(
        "model M constant Integer table[2] = {1, 1000}; \
         parameter Integer resolutionFactor = table[2]; \
         Clock c = Clock(2, resolutionFactor); \
         Real u; Real s; Real acc; Real out; \
         equation u = time; s = sample(u, c); \
         acc = previous(acc) + s * interval(c); out = hold(acc); end M;",
    )
    .expect("a clock off a table");
    // Two thousandths of a second: `interval(c)` comes out as the
    // number, and reaching this far is what says the factor was read.
    let ticks = format!("{:?}", m.when_clauses);
    assert!(ticks.contains("0.002"), "{ticks}");
}

/// A class that replaces one it inherited by extending it, and a
/// package member that is a name for another class.
#[test]
fn a_class_may_redeclare_the_one_it_inherited_by_extending_it() {
    // `redeclare replaceable model extends BaseProperties(...)` - the
    // class being defined is named by what it extends, and its body
    // adds to it. This is how the media libraries are built.
    let m = parse_model(
        "package Base \
           replaceable model Props parameter Real k = 1; Real v; \
             equation v = k * time; end Props; \
         end Base; \
         package Water extends Base; \
           redeclare replaceable model extends Props(k = 5) Real extra; \
             equation extra = 2 * v; end Props; \
         end Water; \
         model M Water.Props p; Real y; equation y = p.extra; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a class redeclared by extending it");
    // The body it added is there, and so is what it inherited, with
    // the modifier the `extends` carried.
    assert!(m.components.iter().any(|c| c.name == "p.extra"));
    let k = m.components.iter().find(|c| c.name == "p.k").unwrap();
    assert!(matches!(k.binding, Some(Expr::Number(n)) if n == 5.0));

    // `package StandardWater = WaterIF97_ph(...)` gives a package a
    // member that is a name for another class, and from outside it is
    // reached by the same dotted name a class would be.
    let m = parse_model(
        "package Lib \
           package Detailed model Cell parameter Real k = 3; Real v; \
             equation v = k * time; end Cell; end Detailed; \
           package Standard = Detailed; \
         end Lib; \
         model M Lib.Standard.Cell c; Real y; equation y = c.v; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a package that names another");
    let k = m.components.iter().find(|c| c.name == "c.k").unwrap();
    assert!(matches!(k.binding, Some(Expr::Number(n)) if n == 3.0));

    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // A class may only redeclare by extending one that a base of the
    // class it is written in declares. With no such base there is
    // nothing to replace, and looking the name up the ordinary way
    // would find the class itself.
    assert!(err("package Water \
           redeclare model extends Props Real extra; equation extra = 1; end Props; \
         end Water; \
         model M Water.Props p; Real y; equation y = p.extra; end M;")
    .contains("no base of"));

    // And a package that names a class that is not there says so where
    // the name is used.
    assert!(err("package Lib package Standard = Missing; end Lib; \
         model M Lib.Standard.Cell c; Real y; equation y = c.v; end M;")
    .contains("unknown type"));
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

/// A flexible `:` takes its length from whatever the declaration is
/// given, written out or worked out.
#[test]
fn a_flexible_size_is_measured_from_the_value_it_is_given() {
    // A list written out says its length by being written out; a list
    // scaled by a factor - which is how the standard library draws its
    // axis labels - has to be worked out before it can be measured.
    let m = parse_model(
        "model Lines parameter Real scale = 1; \
         input Real lines[:, 2] = zeros(0, 2); Real total; \
         equation total = sum(lines); end Lines; \
         model M parameter Real k = 2; \
         Lines drawn(lines = k * {{0, 0}, {1, 1}, {2, 2}}); \
         Real y; equation y = drawn.total; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a length worked out");
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("drawn.lines["))
            .count(),
        6
    );

    // And the declaration's own value still says it where nothing
    // else does: `zeros(0, 2)` is no rows at all.
    let m = parse_model(
        "model Lines input Real lines[:, 2] = zeros(0, 2); Real total; \
         equation total = sum(lines); end Lines; \
         model M Lines drawn; Real y; equation y = drawn.total; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a length of nothing");
    assert!(!m
        .components
        .iter()
        .any(|c| c.name.starts_with("drawn.lines[")));

    // A length may be read off an array measured a declaration
    // earlier: `Shape cylinders[n]` with `n = size(lines, 1)`.
    let m = parse_model(
        "model Cell Real v; equation v = time; end Cell; \
         model Lines input Real lines[:, 2] = zeros(0, 2); \
         parameter Integer n = size(lines, 1); Cell cells[n]; \
         Real total; equation total = cells[1].v; end Lines; \
         model M Lines drawn(lines = {{0, 0}, {1, 1}, {2, 2}}); \
         Real y; equation y = drawn.total; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a length off another array");
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("drawn.cells["))
            .count(),
        3
    );

    // A `:` with nothing to measure is still said to be one.
    let error = parse_model(
        "model Lines input Real lines[:, 2]; Real total; \
         equation total = sum(lines); end Lines; \
         model M Lines drawn; Real y; equation y = drawn.total; end M;",
    )
    .expect_err("nothing to measure")
    .message;
    assert!(error.contains("flexible size `:`"), "{error}");
}

/// `zeros`, `ones` and `fill` take as many dimensions as they are
/// given, not one.
#[test]
fn an_array_of_a_shape_may_be_asked_for_in_full() {
    let m = parse_model(
        "model M parameter Real a[3, 2] = zeros(3, 2); \
         parameter Real b[2, 2, 2] = ones(2, 2, 2); \
         parameter Real c[2, 3] = fill(7, 2, 3); Real y; \
         equation y = a[3, 2] + b[2, 2, 2] + c[2, 3]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("arrays of a shape");
    let count = |head: &str| {
        m.components
            .iter()
            .filter(|c| c.name.starts_with(head))
            .count()
    };
    assert_eq!(count("a["), 6);
    assert_eq!(count("b["), 8);
    assert_eq!(count("c["), 6);
    let filled = m.components.iter().find(|c| c.name == "c[2,3]").unwrap();
    assert!(matches!(filled.binding, Some(Expr::Number(n)) if n == 7.0));
}

/// A loop of assignments inside a `when`, one per round.
#[test]
fn a_loop_may_stand_inside_a_when() {
    // `for i in 1:n loop k[i] = ...; end for;` at an event is how the
    // standard library's routing blocks pick a channel. The loop is
    // unrolled the way one among the equations is, and each round
    // becomes an assignment of its own.
    let m = parse_model(
        "model M parameter Integer n = 3; parameter Integer pick = 2; \
         discrete Real k[n]; Real y; \
         equation y = time; \
         when time > 0.5 then \
           for i in 1:n loop k[i] = if pick == i then 1 else 0; end for; \
         end when; \
         annotation(experiment(StopTime = 1, Interval = 0.1)); end M;",
    )
    .expect("a loop inside a when");
    let actions = &m.when_clauses[0].branches[0].actions;
    assert_eq!(actions.len(), 3);
    let named: Vec<&str> = actions
        .iter()
        .map(|action| match action {
            oxidelica_parser::WhenAction::Assign(name, _) => name.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(named, ["k[1]", "k[2]", "k[3]"]);
    // The second round is the one the pick chose.
    let written = format!("{actions:?}");
    assert!(written.contains("Number(1.0)"), "{written}");

    // What such a loop may hold is assignments: a `connect` draws a
    // connection once and for all, not at an event.
    let error = parse_model(
        "connector Pin Real v; end Pin; \
         model M Pin a[2]; Pin b[2]; Real y; equation y = time; \
         when time > 0.5 then for i in 1:2 loop connect(a[i], b[i]); end for; end when; \
         end M;",
    )
    .expect_err("no connections at an event")
    .message;
    assert!(error.contains("one per round"), "{error}");
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

/// A slice standing where a value is wanted.
#[test]
fn a_slice_may_stand_where_a_value_is_wanted() {
    // `a[2, :]` is a row, `a[:, 3]` a column. What matters is that the
    // subscripts after a slice apply inside each element it kept: read
    // one after the other, `a[:, 3]` would take the third row.
    let m = parse_model(
        "model M parameter Real a[2, 3] = {{1, 2, 3}, {4, 5, 6}}; \
         Real row[3]; Real col[2]; Real y; \
         equation row = a[2, :]; col = a[:, 3]; y = sum(row) + sum(col); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a slice as a value");
    let rhs = |name: &str| {
        format!(
            "{:?}",
            m.equations
                .iter()
                .find(|e| matches!(&e.lhs, Expr::Ref(n) if n == name))
                .unwrap()
                .rhs
        )
    };
    assert_eq!(rhs("row[1]"), "Ref(\"a[2,1]\")");
    assert_eq!(rhs("row[3]"), "Ref(\"a[2,3]\")");
    assert_eq!(rhs("col[1]"), "Ref(\"a[1,3]\")");
    assert_eq!(rhs("col[2]"), "Ref(\"a[2,3]\")");

    // `ac.pin[:].v` - a member read off each of the connectors a slice
    // kept, which is how the converter library reads its plug.
    let m = parse_model(
        "connector Pin Real v; end Pin; \
         model Plug Pin pin[3]; end Plug; \
         model M Plug ac; Real vAC[3]; Real y; \
         equation vAC = ac.pin[:].v; ac.pin[:].v = {1, 2, 3}; y = vAC[2]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a member off a slice");
    let written = format!("{:?}", m.equations);
    assert!(written.contains("ac.pin[3].v"), "{written}");

    // `end` still stands for the length of the dimension it is written
    // in, and a slice may be taken with a vector of indices.
    let m = parse_model(
        "model M parameter Real v[4] = {1, 2, 3, 4}; Real last; Real picked[2]; Real y; \
         equation last = v[end]; picked = v[{2, 4}]; y = last + sum(picked); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("end and a vector subscript");
    assert_eq!(rhs_of(&m, "last"), "Ref(\"v[4]\")");
    assert_eq!(rhs_of(&m, "picked[1]"), "Ref(\"v[2]\")");
    assert_eq!(rhs_of(&m, "picked[2]"), "Ref(\"v[4]\")");
}

/// The right-hand side of the equation defining `name`.
fn rhs_of(model: &oxidelica_parser::Model, name: &str) -> String {
    format!(
        "{:?}",
        model
            .equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(n) if n == name))
            .unwrap_or_else(|| panic!("no equation for {name}"))
            .rhs
    )
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

/// `[ ]` puts its parts together the way the language says: a vector
/// is a column, not a row.
#[test]
fn a_matrix_is_built_from_columns_and_rows() {
    // `[v; 0]` with `v` of two is a column of three, which is what
    // `vector` reads back. Written the other way it would be two rows
    // of different widths and no matrix at all.
    let m = parse_model(
        "model M parameter Real a[2, 2] = {{1, 2}, {3, 4}}; Real v[3]; Real y; \
         equation v = vector([a[1, :]; 0]); y = sum(v); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a vector as a column");
    let rhs = |name: &str| rhs_of(&m, name);
    assert_eq!(rhs("v[1]"), "Ref(\"a[1,1]\")");
    assert_eq!(rhs("v[2]"), "Ref(\"a[1,2]\")");
    assert_eq!(rhs("v[3]"), "Number(0.0)");

    // Side by side within a row, one row under another.
    let m = parse_model(
        "model M parameter Real c[2] = {1, 2}; Real w[2, 2]; Real y; \
         equation w = [c, c]; y = w[2, 2]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("two columns side by side");
    assert_eq!(rhs_of(&m, "w[2,1]"), "Ref(\"c[2]\")");

    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    assert!(
        err("model M Real v[2]; Real w[2,2]; equation v = {1,2}; w = [v, 1]; end M;")
            .contains("equally tall")
    );
    assert!(
        err("model M parameter Real a[2,2] = {{1,2},{3,4}}; Real v[4]; \
         equation v = vector(a); end M;")
        .contains("one dimension worth more than one")
    );
}

/// A type may be written out at length, extending the one it is built
/// on - and that one may be an array.
#[test]
fn a_type_may_be_written_out_at_length() {
    let m = parse_model(
        "type Matrix = Real[3, 3]; \
         type Orientation \"a rotation\" extends Matrix; end Orientation; \
         model M parameter Orientation r = fill(1, 3, 3); Real y; \
         equation y = r[2, 2]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a type extending an array type");
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("r["))
            .count(),
        9
    );
    let corner = m.components.iter().find(|c| c.name == "r[3,3]").unwrap();
    assert_eq!(corner.type_name, "Real");
}

/// `ExternalObject` is the language's own, and a class extending it
/// holds nothing of its own.
#[test]
fn an_external_object_is_a_handle_and_no_variables() {
    // A table held outside Modelica: the class says how to make one
    // and how to let it go, both in another language. A component of
    // it is no variables at all.
    let m = parse_model(
        "model M class Table extends ExternalObject; \
           function constructor input String name; output Table t; \
             external \"C\" t = openTable(name); end constructor; \
           function destructor input Table t; external \"C\" closeTable(t); end destructor; \
         end Table; \
         Table handle = Table(\"data.txt\"); Real y; equation y = time; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a handle held outside");
    assert!(!m.components.iter().any(|c| c.name.starts_with("handle")));

    // What is done with the handle is done by calls this compiler
    // refuses where they are made.
    let error = parse_model(
        "model M class Table extends ExternalObject; \
           function constructor output Table t; external \"C\" t = openTable(); \
             end constructor; \
           function destructor input Table t; external \"C\" closeTable(t); end destructor; \
         end Table; \
         function readTable input Table t; output Real v; external \"C\" v = read(t); \
           end readTable; \
         Table handle = Table(); Real y; equation y = readTable(handle); end M;",
    )
    .expect_err("nothing here can read it")
    .message;
    assert!(error.contains("outside Modelica"), "{error}");

    // And `ExternalObject` itself is a base and nothing more.
    let error = parse_model("model M ExternalObject thing; Real y; equation y = 1; end M;")
        .expect_err("a base only")
        .message;
    assert!(error.contains("partial"), "{error}");
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

/// An `if` whose branches are of different shapes decides a structure
/// rather than a value.
#[test]
fn an_if_between_shapes_is_settled_before_the_run() {
    // A table built one way when there is something in it and another
    // when there is not: the two are of different shapes, so which one
    // stands is not something a run can be left to choose.
    let m = parse_model(
        "package P \
           block Held parameter Real t[:, 2] = fill(0.0, 0, 2); Real y; \
             equation y = t[1, 1] * time; end Held; \
           block Table parameter Real points[:] = {0, 1}; \
             Held held(final t = if n > 0 then [points[1], 0.0; points, {1.0, 0.0}] \
                                 else [0.0, 0.0]); \
             Real y; \
           protected parameter Integer n = size(points, 1); \
             equation y = held.y; end Table; \
         end P; \
         model M P.Table t; Real z; equation z = t.y; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a table chosen by a condition");
    // Two points and a row in front of them: three rows of two.
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("t.held.t["))
            .count(),
        6
    );

    // Where the branches are of one shape it stays a choice the run
    // makes, so the parameter is still one to re-run with.
    let m = parse_model(
        "model M parameter Boolean high = true; Real v[2]; Real y; \
         equation v = if high then {1, 2} else {3, 4}; y = v[1]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a choice of one shape");
    assert!(format!("{:?}", m.equations).contains("If("));

    // And where it decides a shape and cannot be settled, it says so.
    let error = parse_model(
        "model M Real v[2]; Real y; equation y = time; \
         v = if y > 0 then {1, 2} else {3}; end M;",
    )
    .expect_err("a shape the run would choose")
    .message;
    assert!(error.contains("decides the shape"), "{error}");
}

/// A tuple assignment may fill a field of a record.
#[test]
fn a_tuple_may_fill_a_field() {
    let m = parse_model(
        "model M record Token Real value; Real kind; end Token; \
         function scan input Real u; output Real next; output Real found; \
         algorithm next := u + 1; found := u * 2; end scan; \
         function read input Real u; output Real y; protected Real next; Token token; \
         algorithm (next, token.value) := scan(u); y := next + token.value; end read; \
         Real y; equation y = read(3); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a field filled from a tuple");
    // 3 + 1 and 3 * 2 make ten.
    let written = format!("{:?}", m.equations[0].rhs);
    assert!(written.contains("Number(3.0)"), "{written}");
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

/// An `if` equation whose condition compares against an enumeration
/// literal is settled here, so its branches may hold `for` equations.
#[test]
fn an_if_equation_may_compare_an_enumeration() {
    let m = parse_model(
        "package P type Shape = enumeration(Flat, Steep); \
           block B parameter Shape shape = Shape.Flat; parameter Integer n = 2; \
             Real y[n]; \
             equation if shape == Shape.Flat then for i in 1:n loop y[i] = i * time; end for; \
             else for i in 1:n loop y[i] = -i * time; end for; end if; end B; \
         end P; \
         model M P.B b; Real z; equation z = b.y[2]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an enumeration settles the branch");
    let written = format!("{:?}", m.equations);
    assert!(written.contains("b.y[2]"), "{written}");
    // The flat branch was taken, so the second element rises rather
    // than falls.
    assert!(!written.contains("Neg("), "{written}");
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

/// A function deals in arrays when its type says so, not only when its
/// declaration does.
#[test]
fn a_function_takes_arrays_from_its_type() {
    let m = parse_model(
        "model M type Orient = Real[3, 3]; \
         function turn input Orient a; input Orient b; output Orient c; \
         algorithm c := b * a; end turn; \
         parameter Orient one = identity(3); \
         Orient both = turn(one, one); Real y; \
         equation y = both[1, 1] * time; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a matrix that comes with the type");
    // Nine elements rather than a call applied to each of three rows.
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("both["))
            .count(),
        9
    );
}

/// The condition on a component may compare against an enumeration.
#[test]
fn a_component_condition_may_compare_an_enumeration() {
    let m = parse_model(
        "model M type Kind = enumeration(Uniform, Point); \
         parameter Kind gravity = Kind.Point; \
         Real shown = 1 if gravity == Kind.Uniform; \
         Real y; equation y = time; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an enumeration settles the component");
    assert!(!m.components.iter().any(|c| c.name == "shown"));
}

/// What a `when` gives a variable goes through the array layer.
#[test]
fn a_when_may_give_a_value_read_off_an_array() {
    let m = parse_model(
        "model M Real u; discrete Real y_min(start = 0, fixed = true); Real y; \
         equation u = time; \
         when sample(0.1, 0.1) then y_min = min({pre(y_min), u, pre(u)}); end when; \
         y = y_min; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("the smallest of three at an event");
    let given = format!("{:?}", m.when_clauses[0].branches[0].actions);
    // Three values folded into two comparisons.
    assert_eq!(given.matches("\"min\"").count(), 2, "{given}");
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

/// A record is a value: a function may answer with one, and an
/// equation between two of them is one equation per member.
#[test]
fn a_function_may_answer_with_a_record() {
    let m = parse_model(
        "package P record Orient Real T[3, 3]; Real w[3]; end Orient; \
           function turn input Real e[3]; input Real angle; output Orient R; \
           algorithm R := Orient(T = identity(3) * angle, w = e * angle); end turn; \
         end P; \
         model M parameter Real e[3] = {0, 0, 1}; P.Orient R; Real y; \
         equation R = P.turn(e, time); y = R.T[1, 1]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a record answered by a function");
    // Nine of the matrix and three of the rate, one equation each.
    assert_eq!(
        m.equations
            .iter()
            .filter(|e| format!("{:?}", e.lhs).contains("R.T["))
            .count(),
        9
    );
    assert_eq!(
        m.equations
            .iter()
            .filter(|e| format!("{:?}", e.lhs).contains("R.w["))
            .count(),
        3
    );

    // Members may be given by name in any order, and one nobody gives
    // stands on what its declaration says.
    let m = parse_model(
        "model M record Pair Real a; Real b; end Pair; \
         function make input Real u; output Pair p; algorithm p := Pair(b = u, a = 2 * u); \
         end make; \
         Pair one; Real y; \
         equation one = make(time); y = one.a + one.b; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a record written out by name");
    let given = |member: &str| {
        format!(
            "{:?}",
            m.equations
                .iter()
                .find(|e| format!("{:?}", e.lhs) == format!("Ref({member:?})"))
                .expect(member)
                .rhs
        )
    };
    assert_eq!(given("one.a"), "Bin(Mul, Number(2.0), Time)");
    assert_eq!(given("one.b"), "Time");

    // A member nobody gives stands on what its declaration says. Here
    // that is a second equation for `one.b`, since the declaration
    // makes one of its own - which is what a model doing this really
    // is, and the balance check says so further on.
    let m = parse_model(
        "model M record Pair Real a; Real b = 7; end Pair; \
         function spare input Real u; output Pair p; algorithm p := Pair(a = u); end spare; \
         Pair one; Real y; equation one = spare(time); y = one.a; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a member nobody gave");
    let for_b: Vec<String> = m
        .equations
        .iter()
        .filter(|e| format!("{:?}", e.lhs) == "Ref(\"one.b\")")
        .map(|e| format!("{:?}", e.rhs))
        .collect();
    assert_eq!(for_b, ["Number(7.0)", "Number(7.0)"]);

    // Given in order rather than by name, the count still has to match.
    let error = parse_model(
        "model M record Pair Real a; Real b; end Pair; Pair p; \
         equation p = Pair(1); end M;",
    )
    .expect_err("too few fields")
    .message;
    assert!(error.contains("2 field(s), 1 given"), "{error}");
}

/// A declaration may be written over arrays to come to one value.
#[test]
fn a_length_may_be_counted_over_arrays() {
    let m = parse_model(
        "model M parameter Real a[:] = {1, 2}; parameter Real b[:] = {1, 2, 3, 4}; \
         parameter Integer nout = max([size(a, 1); size(b, 1)]); \
         Real q[nout]; Real y; \
         equation for i in 1:nout loop q[i] = i * time; end for; y = q[nout]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("the longest of two, counted by stacking");
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("q["))
            .count(),
        4
    );
    assert!(format!("{:?}", m.equations).contains("q[4]"));
}

/// An element of a parameter array is a number the next declaration
/// may be subscripted with.
#[test]
fn a_subscript_may_come_out_of_a_parameter_array() {
    let m = parse_model(
        "package P type Seq = Integer[3]; \
           function axisOf input Integer sequence[3]; input Real m[3, 3]; output Real e[3]; \
           algorithm e := m[sequence[3], :]; end axisOf; \
         end P; \
         model M parameter P.Seq order = {3, 1, 2}; \
         Real t[3, 3] = {{1, 0, 0}, {0, 2, 0}, {0, 0, 3}} * time; \
         Real e[3]; Real y; \
         equation e = P.axisOf(order, t); y = e[2]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a subscript read out of a parameter array");
    // `order[3]` is 2, so the second row is taken and its middle is
    // the only element that is not zero.
    let given = |name: &str| {
        format!(
            "{:?}",
            m.equations
                .iter()
                .find(|e| format!("{:?}", e.lhs) == format!("Ref({name:?})"))
                .expect(name)
                .rhs
        )
    };
    assert_eq!(given("e[1]"), "Ref(\"t[2,1]\")");
    assert_eq!(given("e[2]"), "Ref(\"t[2,2]\")");
}

/// A branch of an `if` equation may say what the connection graph is.
#[test]
fn a_branch_may_say_where_the_graph_is_rooted() {
    let m = parse_model(
        "model M connector Frame Real r; flow Real f; end Frame; \
         parameter Boolean enforce = true; Frame frame_a; Real y; \
         equation frame_a.f = 0; y = time; \
         if enforce then Connections.root(frame_a); \
         else Connections.potentialRoot(frame_a); end if; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a root declared in a branch");
    assert_eq!(m.connection_graph.len(), 1);
    assert!(
        format!("{:?}", m.connection_graph[0]).starts_with("Root("),
        "{:?}",
        m.connection_graph[0]
    );

    let error = parse_model(
        "model M connector Frame Real r; flow Real f; end Frame; \
         Boolean high; Frame frame_a; Real y; \
         equation frame_a.f = 0; y = time; high = time > 0.5; \
         if high then Connections.root(frame_a); \
         else Connections.potentialRoot(frame_a); end if; end M;",
    )
    .expect_err("a graph the run would draw")
    .message;
    assert!(error.contains("drawn once and for all"), "{error}");
}

/// A question about the connection graph is answered by building the
/// model twice: once to draw the graph, once with the answer in hand.
#[test]
fn the_graph_is_drawn_before_it_is_asked() {
    // A body that is a root carries its own orientation and the states
    // for it; one that is not takes the orientation from what it is
    // connected to and carries none. The two branches are of different
    // lengths, which only holds together because the condition is one
    // the compiler settles.
    const PARTS: &str = "package P \
        connector Frame Real r; Real o; flow Real f; end Frame; \
        model Ground Frame frame_b; \
          equation Connections.root(frame_b.o); frame_b.o = 0; frame_b.f = 0; \
        end Ground; \
        model Body Frame frame_a; Real phi; Real w; \
          equation Connections.potentialRoot(frame_a.o); frame_a.f = 0; \
          if not Connections.isRoot(frame_a.o) then phi = frame_a.o; w = 0; \
          else frame_a.o = phi; w = der(phi); der(w) = 0; end if; \
        end Body; \
      end P; ";
    let m = parse_model(&format!(
        "{PARTS} model M P.Ground ground; P.Body body; Real y; \
         equation connect(ground.frame_b, body.frame_a); \
         body.frame_a.r = 0; ground.frame_b.r = 0; y = body.phi; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    ))
    .expect("a body that is not the root");
    // The ground is the root, so the body took the branch with no
    // states: two equations rather than three, and `w` is nailed to
    // nothing rather than being a derivative.
    let written = format!("{:?}", m.equations);
    assert!(!written.contains("Call(\"der\""), "{written}");

    // On its own the body is the only root there is, so it takes the
    // other branch and the states with it.
    let m = parse_model(&format!(
        "{PARTS} model M P.Body body; Real y; \
         equation body.frame_a.r = 0; body.frame_a.o = 0; y = body.phi; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    ))
    .expect("a body that is the root");
    assert!(format!("{:?}", m.equations).contains("Call(\"der\""));
}

/// A base's parameter may be given a value the class extending it
/// declares below the `extends`.
#[test]
fn a_base_may_be_sized_by_what_comes_after_it() {
    let m = parse_model(
        "package P partial block MIMO parameter Integer nin = 1; parameter Integer nout = 1; \
           input Real u[nin]; output Real y[nout]; end MIMO; \
         block Conv extends P.MIMO(final nin = m, final nout = 2); \
           parameter Integer m = 3; \
         protected parameter Real T[2, m] = {{1, 2, 3}, {4, 5, 6}}; \
           equation y = T * u; end Conv; \
         end P; \
         model M P.Conv c; Real z; \
         equation c.u = {time, 2 * time, 3 * time}; z = c.y[1]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a base sized from below the extends");
    // Three inputs rather than the base's one, and two outputs.
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("c.u["))
            .count(),
        3
    );
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("c.y["))
            .count(),
        2
    );
    // The first row against the input: 1u1 + 2u2 + 3u3.
    let first = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"c.y[1]\")")
        .expect("c.y[1]");
    let written = format!("{:?}", first.rhs);
    assert!(written.contains("c.u[3]"), "{written}");
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

/// A function body handed a record reads an element of one of its
/// members: what it was handed is a list written out, not a name.
#[test]
fn a_body_reads_an_element_off_what_it_was_handed() {
    let m = parse_model(
        "package P record Orient Real T[2, 2]; end Orient; \
           function corner input Orient R; output Real c; \
           algorithm c := R.T[2, 1]; end corner; \
         end P; \
         model M P.Orient r; Real y; \
         equation r.T = {{1, 2}, {3, 4}} * time; y = P.corner(r); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an element of a member of a record handed over");
    let written = format!("{:?}", m.equations);
    assert!(written.contains("r.T[2,1]"), "{written}");
}

/// A package handed to a class replaces the one it was written with,
/// and what is reached through it is reached in full.
#[test]
fn a_package_may_be_handed_to_a_class() {
    const PARTS: &str = "package P \
        package Base \
          constant String substances[:] = {\"water\", \"air\"}; \
          constant Integer n = size(substances, 1); \
          model Props Real T; Real x[n]; Real h; \
            equation h = T; for i in 1:n loop x[i] = i * T; end for; end Props; \
        end Base; \
        package Warm extends P.Base; end Warm; \
        partial model Source replaceable package Medium = P.Base; \
          Medium.Props medium; end Source; \
      end P; ";

    // The package is handed down, and `Props` belongs to a base of it.
    let m = parse_model(&format!(
        "{PARTS} model M extends P.Source(redeclare package Medium = P.Warm); \
         Real y; equation medium.T = time; y = medium.h; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    ))
    .expect("a package handed to a class");
    // Two substances counted off a list of names, so two elements and
    // two equations for them.
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("medium.x["))
            .count(),
        2
    );
}

/// A function may say only what it does and leave what it takes to the
/// one it extends.
#[test]
fn a_function_may_take_what_its_base_declares() {
    let m = parse_model(
        "package P partial package Shape \
           replaceable partial function area input Real side; output Real a; end area; \
         end Shape; \
         package Square extends P.Shape; \
           redeclare function extends area algorithm a := side * side; end area; \
         end Square; end P; \
         model M Real y; equation y = P.Square.area(3); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a function taking what its base declares");
    assert_eq!(
        format!("{:?}", m.equations[0].rhs),
        "Bin(Mul, Number(3.0), Number(3.0))"
    );
}

/// A record given a name by a short `connector` definition is one a
/// `connect` may join, and what a function answers with may be written
/// on its declaration.
#[test]
fn a_record_may_be_carried_by_a_connector() {
    const PARTS: &str = "package P \
        record Pair Real re; Real im; \
          encapsulated operator 'constructor' \
            function fromReal input Real re; input Real im = 0; \
              output P.Pair result(re = re, im = im); \
            algorithm end fromReal; \
          end 'constructor'; \
        end Pair; \
        connector PairOutput = output P.Pair; \
        connector PairInput = input P.Pair; \
        block Make PairOutput y; equation y = P.Pair(time); end Make; \
        block Take PairInput u; Real r; equation r = u.re; end Take; \
      end P; ";
    let m = parse_model(&format!(
        "{PARTS} model M P.Make make; P.Take take; Real z; \
         equation connect(make.y, take.u); z = take.r; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;"
    ))
    .expect("a record carried by a connector");
    // The connection joins the record's members, and the imaginary
    // part stands at what the constructor leaves it.
    let written = format!("{:?}", m.equations);
    assert!(written.contains("make.y.re"), "{written}");
    assert!(written.contains("take.u.im"), "{written}");
    let given = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"make.y.im\")")
        .expect("make.y.im");
    assert_eq!(format!("{:?}", given.rhs), "Number(0.0)");

    // The one value a declaration may give outright, with no algorithm
    // at all.
    let m = parse_model(
        "model M function twice input Real u; output Real y = 2 * u; algorithm end twice; \
         Real z; equation z = twice(3); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an answer given on the declaration");
    assert_eq!(
        format!("{:?}", m.equations[0].rhs),
        "Bin(Mul, Number(2.0), Number(3.0))"
    );
}

/// A condition that leaves out an array of components leaves out its
/// elements, and the connections to them.
#[test]
fn a_condition_leaves_out_a_whole_array() {
    let m = parse_model(
        "package P connector Sig = input Real; connector Out = output Real; \
         block Src Out y; equation y = time; end Src; \
         block Sink Sig u; Real r; equation r = u; end Sink; \
         block Many parameter Boolean useThem = false; parameter Integer m = 2; \
           Sig v[m]; Sink parts[m] if useThem; Real s; \
           equation for i in 1:m loop connect(v[i], parts[i].u); end for; \
           s = v[1]; end Many; \
         end P; \
         model M P.Src src; P.Many many; Real y; \
         equation connect(src.y, many.v[1]); many.v[2] = 0; y = many.s; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("an array of components nobody asked for");
    // Nothing of `parts` is there, and the connection to it fell away
    // with it - what is left is the signal reaching the model.
    assert!(!m.components.iter().any(|c| c.name.contains("parts")));
    assert!(m.components.iter().any(|c| c.name == "many.v[1]"));
}

/// A condition may be written on what a `String` parameter says, and
/// the value may come from a modifier.
#[test]
fn a_condition_may_read_a_string() {
    let m = parse_model(
        "package P model Box parameter String how(start = \"Y\"); \
           Real star = 1 if how <> \"D\"; Real delta = 2 if how == \"D\"; Real y; \
           equation y = time; end Box; end P; \
         model M P.Box b(how = \"Y\"); Real z; equation z = b.y; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a machine wired in star");
    assert!(m.components.iter().any(|c| c.name == "b.star"));
    assert!(!m.components.iter().any(|c| c.name == "b.delta"));

    // And the other way round, so it really is the value that decides.
    let m = parse_model(
        "package P model Box parameter String how(start = \"Y\"); \
           Real star = 1 if how <> \"D\"; Real delta = 2 if how == \"D\"; Real y; \
           equation y = time; end Box; end P; \
         model M P.Box b(how = \"D\"); Real z; equation z = b.y; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a machine wired in delta");
    assert!(m.components.iter().any(|c| c.name == "b.delta"));
    assert!(!m.components.iter().any(|c| c.name == "b.star"));
}

/// A chain of class aliases ends at a name written where the last one
/// was, and whoever asked is somewhere else.
#[test]
fn an_alias_answers_with_a_name_that_carries() {
    let m = parse_model(
        "package P package Types record Pair Real d; Real q; end Pair; \
           record Inductance = P.Types.Pair; end Types; \
         model Gap parameter P.Types.Inductance L0(d = 2, q = 3); Real y; \
           equation y = L0.d * time; end Gap; end P; \
         model M P.Gap gap; Real z; equation z = gap.y; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .expect("a record named through an alias in another package");
    // The record is reached at all, which is what the alias had to
    // carry; the value comes down the modifier as it would anywhere.
    assert!(m.components.iter().any(|c| c.name == "gap.L0.q"));
    assert!(format!("{:?}", m.equations).contains("gap.L0.d"));
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
fn a_branch_inside_a_loop_gives_the_round_its_equations() {
    // Settled before the run: the branch that holds is the only one
    // that leaves anything behind, once per round.
    let m = parse_model(
        "model M parameter Boolean detailed = true; Real v[3]; \
         equation for i in 1:3 loop \
         if detailed then v[i] = i * time; else v[i] = 0; end if; \
         end for; end M;",
    )
    .unwrap();
    assert_eq!(m.equations.len(), 3);
    assert!(m
        .equations
        .iter()
        .all(|e| !matches!(e.rhs, Expr::Number(0.0))));

    // The far branch, and a loop written inside one.
    let plain = parse_model(
        "model M parameter Boolean detailed = false; Real v[2]; Real w[2]; \
         equation for i in 1:2 loop \
         if detailed then v[i] = time; w[i] = time; \
         else v[i] = 0; for j in 1:1 loop w[i] = j; end for; end if; \
         end for; end M;",
    )
    .unwrap();
    assert_eq!(plain.equations.len(), 4);

    // A branch with nothing to choose from leaves the round empty.
    let empty = parse_model(
        "model M parameter Boolean detailed = false; Real x; \
         equation x = time; for i in 1:2 loop \
         if detailed then x = 0; end if; end for; end M;",
    )
    .unwrap();
    assert_eq!(empty.equations.len(), 1);

    // Only the run decides: the round makes one equation per position
    // that chooses its own side, the way an `if` among the equations
    // of a class does.
    let run = parse_model(
        "model M Real u; Real v[2]; \
         equation u = time; for i in 1:2 loop \
         if u > 1 then v[i] = i * u; else v[i] = 0; end if; \
         end for; end M;",
    )
    .unwrap();
    assert_eq!(run.equations.len(), 1);
    // One `if` per round, each balanced at one equation a side.
    assert_eq!(run.conditional.len(), 2);
    assert!(run
        .conditional
        .iter()
        .all(|held| held.branches.len() == 2 && held.branches[0].len() == 1));

    // A `connect` inside such a branch is structural: a connection is
    // drawn once and for all, whichever way the run falls.
    assert!(parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         model P Pin p; end P; \
         model M Real u; P a[2]; P b[2]; \
         equation u = time; for i in 1:2 loop \
         if u > 1 then connect(a[i].p, b[i].p); \
         else connect(a[i].p, b[i].p); end if; end for; end M;",
    )
    .unwrap_err()
    .to_string()
    .contains("structural"));
}

#[test]
fn a_loop_holding_only_a_warning_is_not_an_empty_loop() {
    // A check written at warning level is read and dropped, so a loop
    // that held only those leaves nothing behind - and is still not a
    // loop with no body.
    let m = parse_model(
        "model M parameter Real v[2] = {1, 2}; Real x; \
         equation x = time; for i in 1:2 loop \
         assert(v[i] > 0, \"positive\", AssertionLevel.warning); end for; end M;",
    )
    .unwrap();
    assert_eq!(m.equations.len(), 1);
    assert!(
        parse_model("model M Real x; equation x = time; for i in 1:2 loop end for; end M;")
            .unwrap_err()
            .to_string()
            .contains("no body")
    );
}

#[test]
fn a_settled_branch_inside_a_loop_carries_what_it_holds() {
    // A connection drawn in the branch that holds, once per round.
    let m = parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         model P Pin p; equation p.v = time; end P; \
         model M parameter Boolean joined = true; P a[2]; P b[2]; \
         equation for i in 1:2 loop \
         if joined then connect(a[i].p, b[i].p); end if; end for; end M;",
    )
    .unwrap();
    // Four equations of the parts, and a pair per connection drawn.
    assert_eq!(m.equations.len(), 8);

    // A check written in the branch that holds is a check of the model.
    let checked = parse_model(
        "model M parameter Boolean guarded = true; Real x; \
         equation x = time; for i in 1:2 loop \
         if guarded then assert(x > -1, \"low\"); end if; end for; end M;",
    )
    .unwrap();
    assert_eq!(checked.asserts.len(), 2);

    // A loop inside the branch is unrolled here, and what it cannot
    // settle is said here too.
    assert!(parse_model(
        "model M parameter Boolean guarded = true; Real x; \
         equation x = time; for i in 1:2 loop if guarded then \
         for j in 1:x loop x = j; end for; end if; end for; end M;",
    )
    .unwrap_err()
    .to_string()
    .contains("the trip count of a loop is not settled here"));

    // The loop's extent is read through the branch when the loop does
    // not say it: `v[i]` inside an `if` is what says how far it goes.
    let implied = parse_model(
        "model M parameter Boolean lit = true; Real v[3]; \
         equation for i loop if lit then v[i] = i * time; end if; end for; end M;",
    )
    .unwrap();
    assert_eq!(implied.equations.len(), 3);

    // A `when` there is part of the model rather than a value, and
    // this compiler says so rather than dropping it.
    assert!(parse_model(
        "model M parameter Boolean lit = true; Real v[2]; discrete Real k; \
         equation for i in 1:2 loop v[i] = time; if lit then \
         when time > 1 then k = i; end when; end if; end for; end M;",
    )
    .unwrap_err()
    .to_string()
    .contains("reads none of them"));
}

#[test]
fn cardinality_decides_a_branch_before_the_run() {
    // How many connections name a port is a question about the model
    // as a whole, so the first pass gathers them and the model is
    // built again with the answer in hand. The standard library's
    // state graph writes exactly this: a port nobody connected gets a
    // default equation, and one that is connected does not.
    let m = parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         model Part Pin p[2]; equation for i in 1:2 loop \
         if cardinality(p[i]) == 0 then p[i].v = 0; end if; end for; end Part; \
         model M Part a; Pin q; equation connect(a.p[1], q); q.v = time; end M;",
    )
    .unwrap();
    let sides: Vec<String> = m.equations.iter().map(|e| format!("{:?}", e.lhs)).collect();
    assert!(sides.iter().any(|lhs| lhs.contains("a.p[2].v")));
    assert!(!sides.iter().any(|lhs| lhs.contains("a.p[1].v")));

    // The same question asked among the equations of a class rather
    // than inside a loop.
    let plain = parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         model Part Pin p; equation \
         if cardinality(p) == 0 then p.v = 0; else p.v = 2; end if; end Part; \
         model M Part a; Part b; Pin q; equation connect(b.p, q); q.v = time; end M;",
    )
    .unwrap();
    let said: Vec<String> = plain
        .equations
        .iter()
        .map(|e| format!("{:?} = {:?}", e.lhs, e.rhs))
        .collect();
    assert!(said
        .iter()
        .any(|e| e.contains("a.p.v") && e.contains("0.0")));
    assert!(said
        .iter()
        .any(|e| e.contains("b.p.v") && e.contains("2.0")));
}

/// Sources with an operator record, the way `Complex` is written: two
/// fields, a constructor whose second argument has a default, and a
/// subtraction of its own.
const OPERATOR_RECORD: &str = "operator record C Real re; Real im; \
     encapsulated operator 'constructor' \
       function fromReal input Real re; input Real im = 0; \
       output .C c(re = re, im = im); algorithm end fromReal; \
     end 'constructor'; \
     encapsulated operator function '-' input .C a; input .C b; output .C c; \
       algorithm c := .C(a.re - b.re, a.im - b.im); end '-'; \
     end C; ";

#[test]
fn a_function_written_for_one_record_takes_a_whole_array_of_them() {
    // `Modelica.ComplexMath.abs(v)` of a `Complex[3]`: the function
    // was written for one, so it is called once per element.
    let m = parse_model(&format!(
        "{OPERATOR_RECORD}\
         function amp input C c; output Real y; algorithm y := c.re + c.im; end amp; \
         model M C v[3]; Real a[3]; \
         equation a = amp(v); \
         for i in 1:3 loop v[i].re = i * time; v[i].im = 0; end for; end M;"
    ))
    .unwrap();
    let said: Vec<String> = m
        .equations
        .iter()
        .map(|e| format!("{:?} = {:?}", e.lhs, e.rhs))
        .collect();
    for element in 1..=3 {
        assert!(
            said.iter()
                .any(|e| e.contains(&format!("a[{element}]"))
                    && e.contains(&format!("v[{element}].re"))),
            "{said:?}"
        );
    }

    // The same where the array is reached through a connector, which
    // is how the standard library's sensors read a plug, and where the
    // whole thing is a declaration's value rather than an equation.
    let through = parse_model(&format!(
        "{OPERATOR_RECORD}\
         function amp input C c; output Real y; algorithm y := c.re + c.im; end amp; \
         connector Pin C v; end Pin; \
         model Plug Pin pin[2]; end Plug; \
         model M Plug plug; Real a[2] = amp(plug.pin.v); \
         equation for i in 1:2 loop plug.pin[i].v.re = i * time; \
         plug.pin[i].v.im = 0; end for; end M;"
    ))
    .unwrap();
    assert!(through.equations.iter().any(|e| {
        format!("{:?} = {:?}", e.lhs, e.rhs).contains("plug.pin[2].v.re")
            && matches!(&e.lhs, Expr::Ref(name) if name == "a[2]")
    }));

    // An operator the record declares for itself spreads the same way:
    // `v1 - v2` of two arrays is one subtraction per element, and each
    // answers with a record, so the equation is one per field.
    let operated = parse_model(&format!(
        "{OPERATOR_RECORD}\
         model M C x[2]; C y[2]; C d[2]; \
         equation d = x - y; \
         for i in 1:2 loop x[i].re = i * time; x[i].im = 0; \
         y[i].re = 1; y[i].im = 2; end for; end M;"
    ))
    .unwrap();
    assert_eq!(operated.equations.len(), 12);
    assert!(operated.components.iter().any(|c| c.name == "d[2].im"));

    // A scalar travels unchanged to every round, and an array beside
    // the records goes down with them element by element.
    let beside = parse_model(&format!(
        "{OPERATOR_RECORD}\
         function amp input C c; input Real k; input Real b; output Real y; \
         algorithm y := k * c.re + b; end amp; \
         model M C v[2]; Real k[2]; Real a[2]; \
         equation a = amp(v, k, 2); \
         for i in 1:2 loop v[i].re = i * time; v[i].im = 0; k[i] = i; end for; end M;"
    ))
    .unwrap();
    assert!(beside.equations.iter().any(|e| {
        let said = format!("{:?}", e.rhs);
        said.contains("k[2]") && said.contains("v[2].re") && said.contains("2.0")
    }));

    // Two arrays of records of different lengths do not say how many
    // times the call spreads, so nothing spreads and the mismatch is
    // said where the body is written out.
    assert!(parse_model(&format!(
        "{OPERATOR_RECORD}\
         function gap input C a; input C b; output Real y; \
         algorithm y := a.re - b.re; end gap; \
         model M C u[2]; C w[3]; Real d[2]; \
         equation d = gap(u, w); \
         for i in 1:2 loop u[i].re = i * time; u[i].im = 0; end for; \
         for i in 1:3 loop w[i].re = i; w[i].im = 0; end for; end M;"
    ))
    .is_err());

    // `fill` repeats a whole record, not just a number.
    let filled = parse_model(&format!(
        "{OPERATOR_RECORD}\
         model M C v[2]; Real y; \
         equation v = fill(C(0), 2); y = v[1].re + time; end M;"
    ))
    .unwrap();
    assert_eq!(filled.equations.len(), 5);
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

/// A table block of the shape the standard library gives one: the data
/// in a handle built from a matrix, and the value asked for by a call
/// to a body written in C.
const TIME_TABLE_BLOCK: &str = "package Times \
     class Handle extends ExternalObject; \
       function constructor input String tableName; input String fileName; \
         input Real table[:, :]; input Real startTime; input Integer columns[:]; \
         input Integer smoothness; input Integer extrapolation; input Real shiftTime; \
         output Handle h; \
         external \"C\" h = ModelicaStandardTables_CombiTimeTable_init3(tableName, fileName, \
           table, size(table, 1), size(table, 2), startTime, columns, size(columns, 1), \
           smoothness, extrapolation, shiftTime); end constructor; \
       function destructor input Handle h; \
         external \"C\" ModelicaStandardTables_CombiTimeTable_close(h); end destructor; \
     end Handle; \
     function getValue input Handle h; input Integer column; input Real t; \
       input Real nextEvent; input Real preNextEvent; output Real y; \
       external \"C\" y = ModelicaStandardTables_CombiTimeTable_getValue(h, column, t, \
         nextEvent, preNextEvent); annotation(derivative = getDerValue); end getValue; \
     function nextEvent input Handle h; input Real t; output Real at; \
       external \"C\" at = ModelicaStandardTables_CombiTimeTable_nextTimeEvent(h, t); \
       end nextEvent; \
     function getDerValue input Handle h; input Integer column; input Real t; \
       input Real nextEvent; input Real preNextEvent; input Real der_t; \
       input Real der_next; input Real der_preNext; output Real der_y; \
       external \"C\" der_y = ModelicaStandardTables_CombiTimeTable_getDerValue(h, column, t, \
         nextEvent, preNextEvent, der_t, der_next, der_preNext); end getDerValue; \
     function tmin input Handle h; output Real t; \
       external \"C\" t = ModelicaStandardTables_CombiTimeTable_minimumTime(h); end tmin; \
     function tmax input Handle h; output Real t; \
       external \"C\" t = ModelicaStandardTables_CombiTimeTable_maximumTime(h); end tmax; \
   end Times; ";

#[test]
fn a_table_whose_first_column_is_time_says_when_it_turns() {
    // The same lines as an ordinary table, shifted along the time axis
    // and starting where the block was told to start.
    let m = parse_model(&format!(
        "{TIME_TABLE_BLOCK} model M \
         Times.Handle h = Times.Handle(\"NoName\", \"NoName\", [0, 0; 1, 2; 2, 6], \
           0, {{2}}, 1, 2, 0); \
         Real y; Real turns; Real first; Real last; \
         equation y = Times.getValue(h, 1, time, 0, 0); \
         turns = Times.nextEvent(h, time); \
         first = Times.tmin(h); last = Times.tmax(h); end M;"
    ))
    .unwrap();
    let said = |name: &str| {
        m.equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == name))
            .map(|e| format!("{:?}", e.rhs))
            .unwrap_or_default()
    };
    assert_eq!(said("first"), "Number(0.0)");
    assert_eq!(said("last"), "Number(2.0)");
    // Nothing outside Modelica is left, and the value is a chain of
    // tests on time.
    assert!(
        !said("y").contains("ModelicaStandardTables"),
        "{}",
        said("y")
    );
    assert!(
        said("y").contains("Rel(Lt, Time, Number(1.0))"),
        "{}",
        said("y")
    );
    // The corners, in order, and an infinity past the last one.
    let turns = said("turns");
    assert!(turns.contains("Number(inf)"), "{turns}");
    assert!(
        turns.contains("If(Rel(Lt, Time, Number(0.0)), Number(0.0),"),
        "{turns}"
    );

    // The slope of a table, which is what a model asks for when it
    // differentiates one: two up to the corner, four past it.
    let sloped = parse_model(&format!(
        "{TIME_TABLE_BLOCK} model M \
         Times.Handle h = Times.Handle(\"NoName\", \"NoName\", [0, 0; 1, 2; 2, 6], \
           0, {{2}}, 1, 2, 0); \
         Real y; Real rate; equation y = Times.getValue(h, 1, time, 0, 0); \
         rate = der(y); end M;"
    ))
    .unwrap();
    let rate = format!("{:?}", sloped.equations);
    assert!(
        rate.contains("Number(2.0)") && rate.contains("Number(4.0)"),
        "{rate}"
    );
    assert!(!rate.contains("ModelicaStandardTables"), "{rate}");

    // A table asked for a column it has none of.
    let missing = parse_model(&format!(
        "{TIME_TABLE_BLOCK} model M \
         Times.Handle h = Times.Handle(\"NoName\", \"NoName\", [0, 0; 1, 2], \
           0, {{3}}, 1, 2, 0); \
         Real y; equation y = Times.getValue(h, 1, time, 0, 0); end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(
        missing.contains("asked for column 3, and it has 2"),
        "{missing}"
    );

    // A table read inside a branch only the run settles is read there
    // too: the branches travel to the compiler as they were written.
    let branched = parse_model(&format!(
        "{TIME_TABLE_BLOCK} model M \
         Times.Handle h = Times.Handle(\"NoName\", \"NoName\", [0, 0; 1, 2], \
           0, {{2}}, 1, 2, 0); \
         Real u; Real y; equation u = time; \
         if u > 1 then y = Times.getValue(h, 1, time, 0, 0); else y = 0; end if; end M;"
    ))
    .unwrap();
    let written = format!("{:?}", branched.conditional);
    assert!(!written.contains("ModelicaStandardTables"), "{written}");
    assert!(written.contains("Rel(Lt, Time, Number(1.0))"), "{written}");

    // Shifted along its own axis, and saying nothing before it starts.
    let shifted = parse_model(&format!(
        "{TIME_TABLE_BLOCK} model M \
         Times.Handle h = Times.Handle(\"NoName\", \"NoName\", [0, 5; 1, 7], \
           0.5, {{2}}, 1, 2, 10); \
         Real y; Real first; equation y = Times.getValue(h, 1, time, 0, 0); \
         first = Times.tmin(h); end M;"
    ))
    .unwrap();
    let first = shifted
        .equations
        .iter()
        .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == "first"))
        .map(|e| format!("{:?}", e.rhs))
        .unwrap_or_default();
    assert_eq!(first, "Number(10.0)");
    let value = shifted
        .equations
        .iter()
        .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == "y"))
        .map(|e| format!("{:?}", e.rhs))
        .unwrap_or_default();
    assert!(
        value.contains("If(Rel(Lt, Time, Number(0.5)), Number(0.0),"),
        "{value}"
    );
    assert!(value.contains("Number(11.0)"), "{value}");
}

const TABLE_BLOCK: &str = "package Blocks \
     class Handle extends ExternalObject; \
       function constructor input String tableName; input String fileName; \
         input Real table[:, :]; input Integer columns[:]; input Integer smoothness; \
         input Integer extrapolation; output Handle h; \
         external \"C\" h = ModelicaStandardTables_CombiTable1D_init(tableName, fileName, \
           table, size(table, 1), size(table, 2), columns, size(columns, 1), smoothness, \
           extrapolation); end constructor; \
       function destructor input Handle h; \
         external \"C\" ModelicaStandardTables_CombiTable1D_close(h); end destructor; \
     end Handle; \
     function getValue input Handle h; input Integer column; input Real u; output Real y; \
       external \"C\" y = ModelicaStandardTables_CombiTable1D_getValue(h, column, u); \
       annotation(derivative = getDerValue); end getValue; \
     function getDerValue input Handle h; input Integer column; input Real u; \
       input Real der_u; output Real der_y; \
       external \"C\" der_y = ModelicaStandardTables_CombiTable1D_getDerValue(h, column, u, \
         der_u); end getDerValue; \
     function umin input Handle h; output Real u; \
       external \"C\" u = ModelicaStandardTables_CombiTable1D_minimumAbscissa(h); end umin; \
     function umax input Handle h; output Real u; \
       external \"C\" u = ModelicaStandardTables_CombiTable1D_maximumAbscissa(h); end umax; \
   end Blocks; ";

#[test]
fn a_table_the_model_wrote_is_written_out_rather_than_run() {
    // Straight lines between the rows, carried on beyond the ends -
    // which is what `LastTwoPoints` means. The table is 0, 2, 6 at 0,
    // 1, 2, so the slopes are 2 and 4.
    let m = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         parameter Real data[3, 2] = [0, 0; 1, 2; 2, 6]; \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", data, {{2}}, 1, 2); \
         Real u; Real y; Real low; Real high; \
         equation u = time; y = Blocks.getValue(h, 1, u); \
         low = Blocks.umin(h); high = Blocks.umax(h); end M;"
    ))
    .unwrap();
    let said = |name: &str| {
        m.equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == name))
            .map(|e| format!("{:?}", e.rhs))
            .unwrap_or_default()
    };
    assert_eq!(said("low"), "Number(0.0)");
    assert_eq!(said("high"), "Number(2.0)");
    // Nothing outside Modelica is left in the model.
    let written = said("y");
    assert!(!written.contains("ModelicaStandardTables"), "{written}");
    assert!(
        written.contains("Rel(Lt, Ref(\"u\"), Number(1.0))"),
        "{written}"
    );

    // Constant segments hold the value of the row they start at.
    let held = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", [0, 5; 1, 7], {{2}}, 3, 2); \
         Real y; equation y = Blocks.getValue(h, 1, time); end M;"
    ))
    .unwrap();
    let written = format!("{:?}", held.equations[0].rhs);
    // Five below the second row and seven at or past it, with no line
    // between them: a level has no `u` in it.
    assert!(
        written.contains("If(Rel(Lt, Time, Number(1.0)), Number(5.0), Number(7.0))"),
        "{written}"
    );

    // A table read from a file is not one this compiler holds, so the
    // call stands and is refused by the name it was written with.
    let outside = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\"t\", \"t.txt\", [0, 0; 1, 1], {{2}}, 1, 2); \
         Real y; equation y = Blocks.getValue(h, 1, time); end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(
        outside.contains("ModelicaStandardTables_CombiTable1D_getValue"),
        "{outside}"
    );

    // Spline interpolation is not written out, and says so.
    let spline = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", [0, 0; 1, 1], {{2}}, 2, 2); \
         Real y; equation y = Blocks.getValue(h, 1, time); end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(spline.contains("spline interpolation"), "{spline}");
}

#[test]
fn a_table_reads_what_the_model_settled_around_it() {
    // The handle a table block builds is written for the general case:
    // a file name chosen by an `if`, a smoothness held in a parameter,
    // and the matrix itself a parameter rather than digits. All of it
    // is settled before the table is read.
    let m = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         parameter Boolean onFile = false; \
         parameter String fileName = \"NoName\"; \
         parameter Real data[2, 3] = [0, 1, 10; 4, 3, 30]; \
         parameter Integer how = 1; \
         Blocks.Handle h = Blocks.Handle( \
           if onFile then fileName else \"NoName\", \
           if not (fileName == \"NoName\") or onFile then fileName else \"NoName\", \
           data, {{2, 3}}, how, 1); \
         Real a; Real b; Real low; Real high; \
         Real held(start = Blocks.umax(h)); \
         equation a = Blocks.getValue(h, 1, time); b = Blocks.getValue(h, 2, time); \
         low = Blocks.umin(h); high = Blocks.umax(h); der(held) = 0; \
         assert(Blocks.getValue(h, 1, time) > -100, \"in range\"); end M;"
    ))
    .unwrap();
    let said = |name: &str| {
        m.equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == name))
            .map(|e| format!("{:?}", e.rhs))
            .unwrap_or_default()
    };
    assert_eq!(said("low"), "Number(0.0)");
    assert_eq!(said("high"), "Number(4.0)");
    // The second output reads the third column: 10 to 30 over 0 to 4
    // is a slope of five.
    assert!(said("b").contains("Number(5.0)"), "{}", said("b"));
    // Holding the ends: below the first row and past the last, the
    // value is the row's own rather than a line carried on.
    assert!(
        said("a").contains("If(Rel(Lt, Time, Number(0.0)), Number(1.0)"),
        "{}",
        said("a")
    );
    assert!(said("a").contains("Number(3.0)"), "{}", said("a"));

    // A table call stands wherever an expression may: under an
    // operator, inside a branch, beside a comparison.
    let among = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", [0, 1; 1, 3], {{2}}, 1, 2); \
         Real y; Boolean over; \
         equation y = 2 * (-Blocks.getValue(h, 1, time)) + \
           (if time > 0 and not (time > 5) then 1 else 0); \
         over = Blocks.getValue(h, 1, time) > 2 or time < 0; end M;"
    ))
    .unwrap();
    let written = format!("{:?}", among.equations);
    assert!(!written.contains("ModelicaStandardTables"), "{written}");

    // A table of one row has no interval to be in, and what it says
    // it says everywhere: the standard library's clutches give a
    // friction coefficient that way.
    let single = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", [0, 7], {{2}}, 1, 2); \
         Real y; Real low; Real high; \
         equation y = Blocks.getValue(h, 1, time); \
         low = Blocks.umin(h); high = Blocks.umax(h); end M;"
    ))
    .unwrap();
    let told = |name: &str| {
        single
            .equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == name))
            .map(|e| format!("{:?}", e.rhs))
            .unwrap_or_default()
    };
    // Seven wherever it is asked, and a slope of nothing.
    assert!(
        told("y").starts_with("WithDerivative(Number(7.0)"),
        "{}",
        told("y")
    );
    assert!(told("y").contains("Mul, Number(0.0)"), "{}", told("y"));
    assert_eq!(told("low"), "Number(0.0)");
    assert_eq!(told("high"), "Number(0.0)");

    // An output the table has no column for is said, not guessed.
    let missing = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", [0, 1; 1, 2], {{2}}, 1, 2); \
         Real y; equation y = Blocks.getValue(h, 2, time); end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(missing.contains("has no output 2"), "{missing}");

    // So is a periodic table, which this compiler does not write out.
    let periodic = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", [0, 1; 1, 2], {{2}}, 1, 3); \
         Real y; equation y = Blocks.getValue(h, 1, time); end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(periodic.contains("periodic extrapolation"), "{periodic}");
}

/// A generator of the shape the standard library declares one: the
/// state carried as two `Integer`s, a body written outside Modelica,
/// and an answer of a value and the state it moved to.
const GENERATOR: &str = "package Gen constant Integer nState = 2; \
     pure function random input Integer stateIn[nState]; output Real result; \
       output Integer stateOut[nState]; \
       external \"C\" ModelicaRandom_xorshift64star(stateIn, stateOut, result); end random; \
     function initialState input Integer localSeed; input Integer globalSeed; \
       output Integer state[nState]; protected Real r; constant Integer p = 3; \
       algorithm \
       if localSeed == 0 and globalSeed == 0 then state := {126247697, globalSeed}; \
       else state := {localSeed, globalSeed}; end if; \
       for i in 1:p loop (r, state) := random(state); end for; end initialState; \
     function withN input Integer localSeed; input Integer globalSeed; input Integer n; \
       output Integer state[n]; protected Integer aux[2]; algorithm \
       aux := initialState(localSeed, globalSeed); state[1:2] := aux; end withN; \
   end Gen; ";

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
fn a_record_valued_variable_says_its_value_field_by_field() {
    // `output SI.ComplexVoltage vs[m] = plug_sp.pin.v - plug_sn.pin.v`
    // is how the quasi-static machines read a stator voltage: a whole
    // array of records given a value on the declaration. There is no
    // name in the flat model for `vs[1]` itself, so what the value
    // says is said of its fields.
    let m = parse_model(&format!(
        "{OPERATOR_RECORD}\
         model M C a[2]; C b[2]; C d[2] = a - b; \
         equation for i in 1:2 loop a[i].re = i * time; a[i].im = 0; \
         b[i].re = 1; b[i].im = 2; end for; end M;"
    ))
    .unwrap();
    let said: Vec<String> = m
        .equations
        .iter()
        .map(|e| format!("{:?} = {:?}", e.lhs, e.rhs))
        .collect();
    for name in ["d[1].re", "d[1].im", "d[2].re", "d[2].im"] {
        assert!(
            said.iter()
                .any(|e| e.starts_with(&format!("Ref(\"{name}\")"))),
            "{said:?}"
        );
    }
    assert!(m.components.iter().any(|c| c.name == "d[2].im"));

    // One record rather than an array of them, and one whose value is
    // built rather than named.
    let one = parse_model(&format!(
        "{OPERATOR_RECORD}model M C u; C v = u; \
         equation u.re = time; u.im = 1; end M;"
    ))
    .unwrap();
    let said = format!("{:?}", one.equations);
    assert!(
        said.contains("Ref(\"v.re\")") && said.contains("Ref(\"v.im\")"),
        "{said}"
    );

    // A value handed down as a modifier rather than written on the
    // declaration: it arrives in the terms of the class that supplied
    // it and is read there.
    let handed = parse_model(&format!(
        "{OPERATOR_RECORD}\
         model Inner C v; Real y; equation y = v.re + v.im; end Inner; \
         model M C u; Inner k(v = u); equation u.re = time; u.im = 2; end M;"
    ))
    .unwrap();
    let said = format!("{:?}", handed.equations);
    assert!(
        said.contains("Ref(\"k.v.re\")") && said.contains("Ref(\"k.v.im\")"),
        "{said}"
    );

    // A value this compiler cannot take apart is left where it was,
    // which is what it did with every record value until now: the
    // model is no worse off than before, and says nothing untrue.
    let opaque = parse_model(&format!(
        "{OPERATOR_RECORD}\
         function make output C c; algorithm c := C(1, 2); end make; \
         model M C v = make(); Real y; equation y = v.re + time; end M;"
    ));
    assert!(opaque.is_ok(), "{opaque:?}");
}

#[test]
fn zero_fits_a_unit_of_any_kind() {
    // A rate of change may be nothing, and nothing has no dimension of
    // its own: the multibody library writes `w = T * w1 + e * 0` where
    // `e` is a unit vector and the whole is an angular velocity.
    let m = parse_model(
        "model M Real w(unit = \"rad/s\"); Real e(unit = \"1\") = 1; \
         Real v(unit = \"rad/s\") = time; \
         equation w = v + e * 0; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert!(m.is_ok(), "{m:?}");

    // What is not zero is still held to its unit.
    let wrong = parse_model(
        "model M Real w(unit = \"rad/s\"); Real e(unit = \"1\") = 1; \
         Real v(unit = \"rad/s\") = time; \
         equation w = v + e * 2; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(wrong.contains("cannot add"), "{wrong}");
}

#[test]
fn a_record_valued_parameter_is_handed_down_field_by_field() {
    // `parameter RCData rcData[nRC] = {RCData(R = 0, C = 0)}` is how
    // the battery library carries the parameters of its RC elements. A
    // parameter may not become an equation - its value has to stay a
    // value the run works out at the start - so it is handed down as
    // one modifier per field.
    const PAIR: &str = "record P Real a; Real b; end P; ";
    let m = parse_model(&format!(
        "{PAIR}model M parameter P p[2] = {{P(1, 2), P(3, 4)}}; \
         Real y; equation y = p[2].a * time + p[1].b; end M;"
    ))
    .unwrap();
    // A field takes its value the way any modifier's does, which for
    // a field of a record is a declaration equation.
    let worth = |name: &str| {
        m.equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == name))
            .map(|e| e.rhs.clone())
    };
    assert_eq!(worth("p[1].a"), Some(Expr::Number(1.0)));
    assert_eq!(worth("p[1].b"), Some(Expr::Number(2.0)));
    assert_eq!(worth("p[2].a"), Some(Expr::Number(3.0)));
    assert_eq!(worth("p[2].b"), Some(Expr::Number(4.0)));

    // One record given to a whole array reaches every element of it,
    // the way a single number does.
    let spread = parse_model(&format!(
        "{PAIR}model M parameter P p[3] = P(5, 6); \
         Real y; equation y = p[3].a * time; end M;"
    ))
    .unwrap();
    assert_eq!(
        spread
            .equations
            .iter()
            .filter(|e| e.rhs == Expr::Number(5.0))
            .count(),
        3
    );

    // A field the record declares `final` is not one a value may hand
    // down, and the value is left where it was rather than half
    // applied - which is what this compiler did with every record
    // value until now.
    let fixed = parse_model(
        "record Q Real a; final Real b = 9; end Q; \
         model M parameter Q q = Q(1); Real y; equation y = q.b * time; end M;",
    )
    .unwrap();
    assert!(
        fixed
            .equations
            .iter()
            .any(|e| matches!(&e.lhs, Expr::Ref(name) if name == "q.b")
                && e.rhs == Expr::Number(9.0)),
        "{:?}",
        fixed.equations
    );

    // A record built from fewer values than it has fields takes the
    // rest from its own declaration.
    let partly = parse_model(
        "record R Real a; Real b = 7; end R; \
         model M parameter R r = R(1); Real y; equation y = r.b * time; end M;",
    )
    .unwrap();
    assert!(
        partly
            .equations
            .iter()
            .any(|e| matches!(&e.lhs, Expr::Ref(name) if name == "r.b")
                && e.rhs == Expr::Number(7.0)),
        "{:?}",
        partly.equations
    );
}

#[test]
fn a_handle_may_be_built_by_naming_what_it_is_handed() {
    // `ExternalCombiTable1D(tableName = "NoName", table = lossTable,
    // columns = {2, 3, 4, 5}, ...)` is how the standard library's
    // gears build a table handle: entirely by name, and one argument
    // left out for its own declaration to give. What is behind the
    // handle can only be read once the names are back in their places.
    let m = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\
           columns = {{3}}, table = [0, 1, 4; 1, 2, 8], \
           fileName = \"NoName\", tableName = \"NoName\", smoothness = 1, \
           extrapolation = 2); \
         Real y; Real high; equation y = Blocks.getValue(h, 1, time); \
         high = Blocks.umax(h); end M;"
    ))
    .unwrap();
    let told = |name: &str| {
        m.equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == name))
            .map(|e| format!("{:?}", e.rhs))
            .unwrap_or_default()
    };
    assert_eq!(told("high"), "Number(1.0)");
    // The third column, 4 to 8 over 0 to 1, is a slope of four.
    assert!(told("y").contains("Number(4.0)"), "{}", told("y"));

    // A name the constructor does not take is said, not ignored.
    let odd = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", [0, 1], {{2}}, 1, \
           extrapolation = 2, verbose = true); \
         Real y; equation y = Blocks.getValue(h, 1, time); end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(odd.contains("no argument named `verbose`"), "{odd}");
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
fn a_call_spread_over_arrays_wants_one_length_for_all_of_them() {
    // A scalar function handed arrays is called once per element, so
    // the arrays have to agree on how many elements there are.
    let error = parse_model(
        "model M Real a[2]; Real b[3]; Real y[2]; \
         equation a = {time, 1}; b = {1, 2, 3}; y = atan2(a, b); end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("array arguments must have one length"),
        "{error}"
    );

    // Agreeing, they spread element by element, a scalar reaching
    // every one of them.
    let m = parse_model(
        "model M Real a[2]; Real y[2]; \
         equation a = {time, 1}; y = atan2(a, 2); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .unwrap();
    assert_eq!(m.equations.len(), 4);
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

/// What a `protected` section keeps back is the class's own: a model
/// holding one of its instances may write its public members and not
/// the rest.
#[test]
fn a_protected_declaration_is_not_reachable_from_outside() {
    const INNER: &str = "model Inner protected parameter Real k = 1; Real hidden; \
                         public Real y; equation hidden = time; y = k * hidden; end Inner; ";

    // Named from outside, however it is written.
    for reach in [
        "model M Inner a; Real z; equation z = a.hidden; end M;",
        "model M Inner a[2]; Real z; equation z = a[1].hidden; end M;",
    ] {
        let refusal = parse_model(&format!("{INNER}{reach}"))
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("`protected`"), "{refusal}");
        assert!(refusal.contains("a.hidden"), "{refusal}");
    }

    // Modified from the declaration, which is the same reach with the
    // value going the other way.
    let refusal = parse_model(&format!(
        "{INNER}model M Inner a(k = 5); Real z; equation z = a.y; end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(refusal.contains("a.k"), "{refusal}");

    // A public member of the same class, and the same names read
    // where they belong: inside the class, and inside one extending
    // it.
    parse_model(&format!(
        "{INNER}model M Inner a; Real z; equation z = a.y; end M;"
    ))
    .unwrap();
    parse_model(&format!(
        "{INNER}model M extends Inner; Real z; equation z = hidden + k; end M;"
    ))
    .unwrap();
}

/// A connector kept back is kept back from `connect` too.
#[test]
fn a_protected_connector_is_not_reachable_from_outside() {
    let refusal = parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         model Part protected Pin p; equation p.v = time; end Part; \
         model M Part a; Pin q; equation connect(a.p, q); end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(refusal.contains("a.p"), "{refusal}");
}

/// A `block` is a model whose connectors all have a direction.
#[test]
fn a_block_may_not_hold_a_connector_without_a_direction() {
    // A potential-and-flow connector says nothing about which way it
    // goes, which is the one thing a block's connectors must.
    let refusal = parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         block B Pin p; equation p.v = time; end B; \
         model M B b; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(refusal.contains("`block`"), "{refusal}");
    assert!(refusal.contains("B.p"), "{refusal}");

    // The direction may be written on the connector, on the short
    // definition it came from, or on the declaration.
    for block in [
        "connector RealInput = input Real; block B RealInput u; Real y; equation y = 2 * u; end B;",
        "connector Signal Real v; end Signal; \
         block B input Signal u; Real y; equation y = 2 * u.v; end B;",
        "connector Causal input Real v; end Causal; \
         block B Causal u; Real y; equation y = 2 * u.v; end B;",
    ] {
        parse_model(&format!("{block} model M B b; equation b.y = time; end M;")).unwrap();
    }
}
