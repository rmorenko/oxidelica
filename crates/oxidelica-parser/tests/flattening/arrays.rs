//! Arrays: how long a declaration is, what a subscript reaches, and what an equation between shapes means.

use super::shared::*;
use oxidelica_parser::{parse_model, Expr};

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
    // A subscript that cannot be folded is not a refusal: it is read
    // at the run, by asking which place the index names.
    let chosen =
        parse_model("model M Real v[2]; Real k; equation k = 1; v[1] = 0; v[2] = v[k]; end M;")
            .unwrap();
    let text = format!("{:?}", chosen.equations);
    assert!(text.contains("If(Rel(Eq, Ref(\"k\")"), "{text}");
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

    // An array of one element is still an array. The rectifiers reuse
    // their polyphase blocks with the phase count set to one and pass
    // `zeros(1)` in, so a vector of a single entry has to be handed
    // out entry by entry like any other, not given whole to the one
    // element - which would leave a scalar parameter holding a vector.
    let m = parse_model(
        "model Sub parameter Integer m = 3; Inner p[m](k = zeros(m)); end Sub;\
         model Inner parameter Real k = 7; end Inner;\
         model M Sub one(m = 1); Sub two(m = 2); end M;",
    )
    .unwrap();
    for name in ["one.p[1].k", "two.p[1].k", "two.p[2].k"] {
        let binding = format!(
            "{:?}",
            m.components
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("no {name}"))
                .binding
        );
        assert!(binding.contains("Number(0.0)"), "{name}: {binding}");
    }
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
fn the_array_layer_says_what_it_cannot_do() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();
    // A subscript settled only at the run reads its element there, by
    // asking which place the index names.
    let chosen = parse_model(
        "model M Real v[2]; Real k; Real y; equation v = {1, 2}; k = time; y = v[k]; end M;",
    )
    .unwrap();
    let text = format!("{:?}", chosen.equations);
    assert!(text.contains("If(Rel(Eq, Ref(\"k\")"), "{text}");
    // Every place is asked, the last one included: an index outside
    // the array falls through to a value that is no number rather
    // than quietly taking the end.
    assert!(text.contains("NaN"), "{text}");
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

/// A subscript outside its array says which array, and where.
#[test]
fn a_subscript_outside_its_array_names_what_was_being_read() {
    // `subscript 1 is outside an array of 0` says which model was
    // refused and nothing at all about where to look in it. The name
    // is what turns a refusal into a place to start.
    let refused = parse_model(
        "package P model M parameter Integer n = 0; \
         Real empty[n]; Real y; \
         equation y = empty[1]; end M; end P;",
    )
    .expect_err("a subscript into nothing")
    .message;
    assert!(refused.contains("outside an array of 0"), "{refused}");
    assert!(refused.contains("empty"), "the array is named: {refused}");
}

/// A class asks its own parameters whatever its neighbours measured.
#[test]
fn a_class_measures_its_parameters_whoever_stands_beside_it() {
    // `n = size(lines, 1)` is a number as soon as `lines` is measured,
    // and the round that asks the waiting parameters used to run only
    // where the model's list of measured arrays had grown since the
    // last component. A class reached when nothing had grown never
    // asked at all - so what one instance came to depended on what
    // stood beside it.
    let source = |before: &str| {
        format!(
            "package P model Inner \
             parameter Real lines[:, 2] = {{{{0, 0}}, {{1, 1}}, {{2, 2}}}}; \
             parameter Integer n = size(lines, 1); Real y = n; end Inner; \
             model M {before} Inner alone; Real out; \
             equation out = alone.y; \
             annotation(experiment(StopTime = 1, Interval = 1)); end M; end P;"
        )
    };
    let value_of = |before: &str| {
        let m = parse_model(&source(before)).expect("a length read off a neighbour");
        m.components
            .iter()
            .find(|c| c.name == "alone.n")
            .and_then(|c| c.binding.clone())
            .map(|binding| format!("{binding:?}"))
            .unwrap_or_default()
    };
    // Alone, and with a neighbour that measures nothing at all: the
    // same number either way, which is what not depending on the
    // neighbours means.
    assert!(value_of("").contains("3"), "{}", value_of(""));
    assert_eq!(value_of(""), value_of("Real quiet = time;"));
    assert_eq!(value_of(""), value_of("Inner ahead; Real quiet = time;"));
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

/// A call that answers with an array may be asked for one of its
/// elements where it stands.
#[test]
fn a_call_may_be_subscripted_where_it_is_written() {
    let m = parse_model(
        "function pair input Real x; output Real y[2]; algorithm y := {x, 2 * x}; end pair; \
         package Lib function twice input Real x; output Real y[2]; \
         algorithm y := {3 * x, 4 * x}; end twice; end Lib; \
         model M Real a; Real b; Real c[2]; \
         equation a = pair(2)[2]; b = Lib.twice(2)[1]; c = pair(5); end M;",
    )
    .unwrap();
    let settled = |name: &str| {
        m.equations
            .iter()
            .find(|equation| equation.lhs == oxidelica_parser::Expr::Ref(name.to_string()))
            .map(|equation| equation.rhs.clone())
    };
    // The second of {2, 4}, the first of {6, 8}, and a whole array
    // still handed over whole. Each comes out as arithmetic on
    // numbers alone: the element was picked, not left standing.
    let numbers = |name: &str| {
        let mut names = Vec::new();
        let value = settled(name).expect(name);
        value.collect_refs(&mut names);
        assert!(names.is_empty(), "{name}: {value:?}");
        format!("{value:?}")
    };
    assert_eq!(numbers("a"), "Bin(Mul, Number(2.0), Number(2.0))");
    assert_eq!(numbers("b"), "Bin(Mul, Number(3.0), Number(2.0))");
    assert_eq!(numbers("c[1]"), "Number(5.0)");
}

/// A call nothing could write out is still asked for one of its
/// answers.
#[test]
fn a_standing_call_may_be_subscripted() {
    // The loop runs as many rounds as the model decides, so the body
    // cannot be written out and the call is left for the run to walk.
    // The subscript goes with it: there is no list to pick from until
    // the call is made.
    let m = parse_model(
        "function grow input Real x; output Real y[2]; protected Real k; \
         algorithm k := 0; while k < x loop k := k + 1; end while; y := {k, 10 * k}; end grow; \
         model M Real z; equation z = grow(time * 3)[2]; end M;",
    )
    .unwrap();
    let settled = m
        .equations
        .iter()
        .find(|equation| equation.lhs == oxidelica_parser::Expr::Ref("z".to_string()))
        .expect("nothing settles z");
    assert!(
        matches!(&settled.rhs, oxidelica_parser::Expr::Index(base, _)
            if matches!(base.as_ref(), oxidelica_parser::Expr::Call(name, _) if name == "grow")),
        "{:?}",
        settled.rhs
    );
}

/// An array handed down an `extends` reaches the base as its elements.
///
/// `extends ConditionalHeatPort(T = T_ref)` is how every machine in
/// the standard library says its windings start at their reference
/// temperature. In the base, `T_ref` is not a name, so the value came
/// back whole and looked like a scalar - and a scalar spreads over the
/// whole array, binding every element of `T` to the entire array.
/// Seventeen machine models refused to start on the parameters that
/// made.
#[test]
fn an_array_handed_down_an_extends_arrives_as_its_elements() {
    let model = parse_model(
        "package Top \
           partial model Cond parameter Real T[3] = fill(293.15, 3); end Cond; \
           model R parameter Real T_ref[3] = fill(300.15, 3); \
             extends Cond(T = T_ref); \
           end R; \
         end Top;",
    )
    .expect("flattens");
    for element in 1..=3 {
        let name = format!("T[{element}]");
        let bound = model
            .components
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{name} survives"))
            .binding
            .as_ref()
            .map(|b| b.describe());
        assert_eq!(
            bound,
            Some(format!("T_ref[{element}]")),
            "{name} should take its own element, not the whole array"
        );
    }
}

/// A result as long as a constant of the package around the function.
///
/// A random generator declares `output Integer state[nState]` where
/// `nState` is a constant of the package holding it, and then measures
/// its own result with `size(state, 1)`. Read without the constants of
/// that package the length says nothing, the result has no shape, and
/// the body cannot ask how long it is.
#[test]
fn a_result_is_as_long_as_a_constant_of_its_own_package() {
    let m = parse_model(
        "model M package P constant Integer n = 3; \
         function f input Integer a; output Integer s[n]; \
         algorithm s := fill(a, size(s, 1)); end f; end P; \
         Real y; equation y = P.f(2)[3]; end M;",
    )
    .expect("a result measured by its own package's constant");
    assert!(
        matches!(m.equations.first().map(|e| &e.rhs), Some(Expr::Number(two)) if *two == 2.0),
        "{:?}",
        m.equations.first()
    );
}

/// A sum read across an array that belongs to a neighbour.
///
/// A value written on an `extends` may read one member off every
/// element of an array a neighbour holds: a machine adds up
/// `sum(rs.resistor.LossPower)` over its phases. The base is built
/// before the components of the class extending it, so where that
/// value is read nothing yet knows `rs.resistor` is an array, and the
/// name travels out whole. Summed there it would come to the name
/// itself; left standing, it is written out once every shape is in
/// hand.
#[test]
fn a_sum_reaches_across_an_array_a_neighbour_holds() {
    let m = parse_model(
        "model M model Leaf Real p; equation p = 2; end Leaf; \
         model Box parameter Integer m = 3; Leaf leaf[m]; end Box; \
         partial model Sink input Real w; Real y; equation y = w; end Sink; \
         Box b(m = 3); extends Sink(w = sum(b.leaf.p)); end M;",
    )
    .expect("a sum across a neighbour's array");
    let summed = m
        .equations
        .iter()
        .find(|e| matches!(&e.lhs, Expr::Ref(name) if name == "w"))
        .map(|e| e.rhs.describe())
        .expect("the value handed to the base");
    assert!(
        summed.contains("b.leaf[1].p") && summed.contains("b.leaf[3].p"),
        "{summed}"
    );
}

/// An array of one handed a name is still handed an array.
///
/// A resistance connection of star points comes to a single symmetric
/// base system, so the resistor it holds is one phase wide, and its
/// `T = T_ref` hands one name to one element. A value that arrives as
/// a single item is otherwise spread over every element - that is what
/// a scalar means - but here the item is the array itself, and
/// spreading it binds the one element to a name no parameter can be
/// worked out from.
#[test]
fn a_name_handed_to_an_array_of_one_is_read_as_its_element() {
    let m = parse_model(
        "model M partial model Cond parameter Integer mh = 3; \
         parameter Real T[mh] = fill(293.15, mh); Real L[mh]; \
         equation L = T; end Cond; \
         model Res parameter Integer m = 3; \
         parameter Real T_ref[m] = fill(300.15, m); \
         extends Cond(final mh = m, T = T_ref); end Res; \
         Res r(m = 1); end M;",
    )
    .expect("a name handed to an array of one");
    let bound = m
        .components
        .iter()
        .find(|c| c.name == "r.T[1]")
        .and_then(|c| c.binding.as_ref())
        .map(|b| b.describe())
        .expect("the one element of the array");
    assert_eq!(bound, "r.T_ref[1]");
}

/// A name of a different length is not what the elements were, so it
/// is left to stand: spreading it apiece would bind the wrong ones.
#[test]
fn a_name_of_another_length_is_not_spread_over_the_elements() {
    let m = parse_model(
        "model M partial model Cond parameter Integer mh = 3; \
         parameter Real T[mh] = fill(293.15, mh); Real L[mh]; \
         equation L = T; end Cond; \
         model Res parameter Integer m = 3; \
         parameter Real T_ref[2] = fill(300.15, 2); \
         extends Cond(final mh = m, T = T_ref); end Res; \
         Res r(m = 1); end M;",
    )
    .expect("a name of another length");
    let bound = m
        .components
        .iter()
        .find(|c| c.name == "r.T[1]")
        .and_then(|c| c.binding.as_ref())
        .map(|b| b.describe())
        .expect("the one element of the array");
    assert_eq!(bound, "r.T_ref");
}

/// A member read off an array of components, in an equation already
/// written out one element at a time, is the element of the same
/// index rather than the whole slice.
#[test]
fn a_member_of_an_array_is_read_element_by_element() {
    let m = parse_model(
        "model M model Pin Real v; end Pin; \
         model Plug parameter Integer m = 3; Pin pin[m]; end Plug; \
         parameter Integer m = 3; \
         output Real vs[m] = plug_sp.pin.v - plug_sn.pin.v; \
         Plug plug_sp(final m = m); Plug plug_sn(final m = m); end M;",
    )
    .expect("a member read element by element");
    let written: Vec<String> = m
        .equations
        .iter()
        .map(|e| format!("{} = {}", e.lhs.describe(), e.rhs.describe()))
        .collect();
    assert!(
        written.contains(&"vs[2] = plug_sp.pin[2].v - plug_sn.pin[2].v".to_string()),
        "{written:?}"
    );
}

/// An equation between two whole arrays nothing had measured yet is
/// one equation per element. Written on a base, it is read before the
/// components it names are built, so their shape is not known there.
#[test]
fn an_equation_between_two_whole_arrays_is_taken_apart() {
    let m = parse_model(
        "model M model Phasor parameter Integer n = 2; Real v_[n]; end Phasor; \
         model Gap Phasor port; end Gap; model Cage Phasor port; end Cage; \
         partial model Base equation gap.port.v_ = cage.port.v_; end Base; \
         model Use extends Base; Gap gap; Cage cage; end Use; Use u; end M;",
    )
    .expect("an equation between two whole arrays");
    let written = equations_of(&m);
    assert_eq!(
        written,
        vec![
            "u.gap.port.v_[1] = u.cage.port.v_[1]",
            "u.gap.port.v_[2] = u.cage.port.v_[2]",
        ]
    );
}

/// A reduction over such an array is still one number, so the
/// equation around it is left whole.
#[test]
fn a_sum_over_a_whole_array_is_not_taken_apart() {
    let m = parse_model(
        "model M model Phasor parameter Integer n = 2; Real v_[n]; end Phasor; \
         model Gap Phasor port; end Gap; \
         partial model Base Real total; equation total = sum(gap.port.v_); end Base; \
         model Use extends Base; Gap gap; end Use; Use u; end M;",
    )
    .expect("a sum over a whole array");
    assert_eq!(
        equations_of(&m),
        vec!["u.total = u.gap.port.v_[1] + u.gap.port.v_[2]"]
    );
}

/// The largest of an array read the same late way is folded the same
/// way a sum is, one comparison at a time.
#[test]
fn a_maximum_over_a_whole_array_is_folded() {
    let m = parse_model(
        "model M model Phasor parameter Integer n = 3; Real v_[n]; end Phasor; \
         model Gap Phasor port; end Gap; \
         partial model Base Real top; equation top = max(gap.port.v_); end Base; \
         model Use extends Base; Gap gap; end Use; Use u; end M;",
    )
    .expect("a maximum over a whole array");
    assert_eq!(
        equations_of(&m),
        vec!["u.top = max(max(u.gap.port.v_[1], u.gap.port.v_[2]), u.gap.port.v_[3])"]
    );
}

/// An equation whose two sides are arrays of different lengths is not
/// something a walk over elements could pair up, so it is left as it
/// stands rather than written out wrongly.
#[test]
fn an_equation_between_arrays_of_different_lengths_is_left_alone() {
    let m = parse_model(
        "model M model Two parameter Integer n = 2; Real v_[n]; end Two; \
         model Three parameter Integer n = 3; Real v_[n]; end Three; \
         partial model Base equation a.v_ = b.v_; end Base; \
         model Use extends Base; Two a; Three b; end Use; Use u; end M;",
    )
    .expect("arrays of different lengths");
    assert_eq!(equations_of(&m), vec!["u.a.v_ = u.b.v_"]);
}

/// A name subscripted by something that is not a plain number - a
/// slice `v_[1:2]` - names no single element, so the walk that pairs
/// two arrays up element by element passes over it.
#[test]
fn a_name_subscripted_by_a_range_is_not_one_element() {
    let m = parse_model(
        "model M model Phasor parameter Integer n = 2; Real v_[n]; end Phasor; \
         model Gap Phasor port; end Gap; Real w[2]; \
         equation w = gap.port.v_[1:2]; \
         public Gap gap; end M;",
    )
    .expect("a name subscripted by a range");
    assert_eq!(
        equations_of(&m),
        vec!["w[1] = gap.port.v_[1]", "w[2] = gap.port.v_[2]"]
    );
}

/// A member read off an array of more dimensions than the subscripts
/// in hand is not the element they name, so it is left as it stands.
#[test]
fn a_member_of_a_deeper_array_is_left_alone() {
    let m = parse_model(
        "model M model Pin Real v; end Pin; \
         model Plug parameter Integer r = 2; Pin pin[r, r]; end Plug; \
         model Flat parameter Integer n = 2; Real v_[n]; end Flat; \
         partial model Base equation flat.v_ = plug.pin.v; end Base; \
         model Use extends Base; Plug plug; Flat flat; end Use; Use u; end M;",
    )
    .expect("a member of a deeper array");
    let written = equations_of(&m);
    assert!(
        written.iter().all(|line| line.contains("u.plug.pin.v")),
        "{written:?}"
    );
}

/// A whole array standing where one element is wanted is that
/// element: a machine writes `idq_ss = airGap.i_ss`, and the air gap
/// is built after the equation that reads it.
#[test]
fn a_whole_array_read_beside_an_element_is_that_element() {
    let m = parse_model(
        "model M model Gap parameter Integer n = 2; Real i_ss[n]; end Gap; \
         Real idq_ss[2]; \
         equation idq_ss = gap.i_ss; \
         public Gap gap; end M;",
    )
    .expect("a whole array read beside an element");
    assert_eq!(
        equations_of(&m),
        vec!["idq_ss[1] = gap.i_ss[1]", "idq_ss[2] = gap.i_ss[2]"]
    );
}

/// A local of a function body is an array, and its value names what
/// the call handed in: the multibody world builds an orientation from
/// an axis vector that way. Left unbound the body carried the input's
/// name out with it, and out there `n_x` means nothing. A default that
/// names a class constant is read where it was written, too.
#[test]
fn a_local_array_of_a_function_reads_what_the_call_handed_in() {
    let m = parse_model(
        "package P constant Real small = 0.5; \
         function f input Real n[3]; input Real eps = P.small; output Real y; \
         protected Real e[3] = if n[1] < eps then {1,0,0} else n; \
         algorithm y := e[1]; end f; \
         model M Real a[3] = {3,0,0}; Real x; equation der(x) = f(a); end M; end P;",
    )
    .expect("a body whose local array is built from its argument");
    let written = equations_of(&m);
    assert!(
        !written
            .iter()
            .any(|e| e.contains("n[1]") || e.contains("P.small")),
        "neither the argument's name nor the constant's travels out: {written:?}"
    );
    assert!(
        written
            .iter()
            .any(|e| e.contains("a[1]") && e.contains("0.5")),
        "the caller's array and the constant's value stand in their place: {written:?}"
    );
}

#[test]
fn a_local_array_built_by_a_call_is_read_element_by_element() {
    let m = parse_model(
        "package P \
         function f input Real n[3]; output Real y; \
         protected Real e[3] = if n[1] < 1e-10 then {1,0,0} else n; \
         protected Real z[3] = cross(e, {0,1,0}); \
         algorithm y := z[3]; end f; \
         model M Real a[3] = {3,4,5}; Real x; equation der(x) = f(a); end M; end P;",
    )
    .expect("a body whose local array is a call on another local");
    let written = equations_of(&m);
    assert!(
        !written.iter().any(|e| e.contains("e[")),
        "the local's own name does not travel out: {written:?}"
    );
    assert!(
        written.iter().any(|e| e.contains("a[1]")),
        "the caller's array stands in its place: {written:?}"
    );
}

#[test]
fn a_value_handed_down_may_name_an_array_of_the_class_that_wrote_it() {
    // A composite step tells the state graph root about its own
    // suspend ports: `inner outer CompositeStepState stateGraphRoot(
    // suspend = anyTrue(suspend.reset))`. The value is worked out
    // inside the root, where `suspend` is not a name at all - it
    // belongs to the class that wrote the modifier - so what that
    // class knows to be an array has to travel with the value, or the
    // call is handed one port instead of a run of them.
    let m = parse_model(
        "connector P Boolean reset; end P; \
         function anyT input Boolean b[:]; output Boolean r; \
         algorithm r := false; \
         for i in 1:size(b, 1) loop r := r or b[i]; end for; end anyT; \
         model Root Boolean s; end Root; \
         model Sub Root r(s = anyT(suspend.reset)); P suspend[2]; \
         equation suspend[1].reset = time > 0.5; suspend[2].reset = false; end Sub; \
         model M Sub sub; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .unwrap();
    let about: Vec<String> = m
        .equations
        .iter()
        .filter(|e| format!("{:?}", e.lhs).contains("sub.r.s"))
        .map(|e| format!("{:?}", e.rhs))
        .collect();
    assert_eq!(about.len(), 1, "{about:?}");
    assert!(about[0].contains("sub.suspend[2].reset"), "{about:?}");
}

#[test]
fn a_comprehension_may_name_several_iterators() {
    // `{f(i, j) for i in 1:n, j in 1:m}` is one array of n times m
    // elements, and 10.4.2 says the last iterator moves fastest.
    let m = parse_model(
        "model M parameter Real b[6] = {i * 10 + j for i in 1:2, j in 1:3}; \
         Real y; equation y = b[1] + b[2] + b[6]; end M;",
    )
    .unwrap();

    // Each element's value says which round of each iterator made it:
    // `i * 10 + j` with the numbers put in.
    let element = |at: usize| {
        let name = format!("b[{at}]");
        let component = m.components.iter().find(|c| c.name == name).unwrap();
        format!("{:?}", component.binding.as_ref().unwrap())
    };
    let made_by = |i: usize, j: usize| {
        format!("Bin(Add, Bin(Mul, Number({i}.0), Number(10.0)), Number({j}.0))")
    };
    // Row-major: the first three are i = 1 with j running fastest.
    assert_eq!(element(1), made_by(1, 1));
    assert_eq!(element(2), made_by(1, 2));
    assert_eq!(element(3), made_by(1, 3));
    assert_eq!(element(4), made_by(2, 1));
    assert_eq!(element(6), made_by(2, 3));
}

/// A Boolean index names its place by being `false` or `true` rather
/// than by counting: comparing one against a number is comparing two
/// different kinds, which the checker refuses outright.
#[test]
fn a_boolean_subscript_the_run_settles_asks_in_booleans() {
    let m = parse_model(
        "model M constant Real B[2] = {10, 20}; Real k; Real y; \
         equation k = time; y = B[k > 0.5]; end M;",
    )
    .unwrap();
    let text = format!("{:?}", m.equations);
    // Each place is asked as a Boolean, and `false` comes first: it is
    // the lower bound of the dimension.
    assert!(text.contains("Bool(false)), Ref(\"B[1]\")"), "{text}");
    assert!(text.contains("Bool(true)), Ref(\"B[2]\")"), "{text}");
}

/// A subscript the run settles reads its element by asking which place
/// the index names, and what that builds is a choice among values. A
/// choice can be read but not assigned: standing on the left it names
/// no variable, and left alone it came out as a number no one asked
/// for.
#[test]
fn a_subscript_the_run_settles_cannot_be_assigned_to() {
    let err = parse_model(
        "model M Real v[3]; Real k; Real t; equation t = time; \
         k = if t > 0.05 then 3 else 1; v[2] = 2; v[3] = 3; v[k] = 9; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("must name a variable"), "{err}");
}

/// Whether a name is read after an `if` decides whether its value has
/// to be merged, so the walk that answers it has to look through every
/// kind of expression: a call, a comprehension, a range, a matrix, a
/// slice. Missing one would drop a value something still reads.
#[test]
fn the_walk_for_a_later_read_looks_everywhere() {
    // Each body writes `o` in one branch only, then reads it after the
    // `if` through a different kind of expression. Every one must be
    // merged, and merging a name a branch never set is refused - so a
    // refusal here is the walk having found the read.
    let reads = [
        "T := sum({o[1], o[2]});",                      // an array and a call
        "T := sum({o[i] for i in 1:2});",               // a comprehension
        "T := o[1] + (if p > 0 then o[2] else 0);",     // a choice
        "T := abs(-o[1]);",                             // a negation inside a call
        "T := if o[1] > 0 and o[2] > 0 then 1 else 2;", // a comparison and an `and`
        "T := sum(o[1:2]);",                            // a range as a subscript
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
            err.contains("assigned in one branch only"),
            "`{tail}` reads `o` after the `if`, so it must be merged: {err}"
        );
    }
}

/// A flexible `:` length is read from the value a component is given,
/// and a value handed down beats the one the declaration wrote. A
/// number in the shape table that the model has already overruled is
/// worse than none: a parameter settled from it is settled for good,
/// while a name with no shape is asked again later.
#[test]
fn a_handed_value_measures_the_flexible_size_not_the_default() {
    // Two instances of one block, each handed its own vector. The
    // second used to be measured by the declaration's default and
    // came out with the first one's length.
    let m = parse_model(
        "package P block B \
         parameter Real v[:] = {0, 1}; \
         parameter Integer n = size(v, 1); \
         output Real y[n]; \
         equation for i in 1:n loop y[i] = v[i]; end for; end B; \
         model M B a(v = {2, 4, 6, 8}); B b(v = {1, 3, 5, 7}); \
         Real p; Real q; equation p = a.y[4]; q = b.y[4]; end M; end P;",
    )
    .unwrap();
    let text = format!("{:?}", m.equations);
    // Both reached their fourth element, so both were measured as four.
    assert!(text.contains("a.y[4]"), "{text}");
    assert!(text.contains("b.y[4]"), "{text}");

    // A matrix written out says its shape by how it is written.
    let matrix = parse_model(
        "package P block B \
         parameter Real t[:,:] = [0, 1]; \
         parameter Integer rows = size(t, 1); \
         output Real y; equation y = rows; end B; \
         model M B b(t = [0, 10; 1, 20; 2, 30]); Real z; \
         equation z = b.y; end M; end P;",
    )
    .unwrap();
    // Three rows, so `y` is three: the shape was read from the matrix
    // handed down rather than from the `[0, 1]` the block declared.
    let rows = matrix
        .components
        .iter()
        .find(|c| c.name == "b.rows")
        .and_then(|c| c.binding.clone());
    assert!(
        format!("{rows:?}").contains("3.0") || format!("{:?}", matrix.equations).contains("3.0"),
        "rows: {rows:?} eqs: {:?}",
        matrix.equations
    );

    // A range says its length by its bounds, and a bound may ask after
    // an array already measured: this is how the table blocks write
    // which columns they take.
    let ranged = parse_model(
        "package P block B \
         parameter Real t[:,:] = [0, 1]; \
         parameter Integer cols[:] = 2:size(t, 2); \
         parameter Integer n = size(cols, 1); \
         output Real y; equation y = n; end B; \
         model M B b(t = [0, 10, 20; 1, 30, 40]); Real z; \
         equation z = b.y; end M; end P;",
    )
    .unwrap();
    let n = ranged
        .components
        .iter()
        .find(|c| c.name == "b.n")
        .and_then(|c| c.binding.clone());
    assert!(
        format!("{n:?}").contains("2.0") || format!("{:?}", ranged.equations).contains("2.0"),
        "columns 2:3 is two: {n:?} eqs: {:?}",
        ranged.equations
    );
}

/// A modifier handed to a base is worked out where it was written, and
/// what it asks about an array may be buried anywhere in it: inside
/// arithmetic, a choice, a list, a range, a subscript.
#[test]
fn a_size_asked_in_a_handed_value_is_answered_wherever_it_sits() {
    let flattens = |modifier: &str| {
        let source = format!(
            "package P partial block MO parameter Integer n = 1; \
             output Real y[n]; end MO; \
             block T extends MO(final n = {modifier}); \
             parameter Real v[:] = {{0, 1}}; \
             equation for i in 1:n loop y[i] = i; end for; end T; \
             model M T b(v = {{1, 2, 3}}); Real u; \
             equation u = b.y[3]; end M; end P;"
        );
        parse_model(&source)
            .map(|m| format!("{:?}", m.equations).contains("b.y[3]"))
            .unwrap_or(false)
    };
    // Each of these comes to three, and the third element must exist.
    assert!(flattens("size(v, 1)"), "plain");
    assert!(flattens("size(v, 1) + 0"), "inside arithmetic");
    assert!(flattens("-(-size(v, 1))"), "inside a negation");
    assert!(
        flattens("if size(v, 1) > 2 then 3 else 1"),
        "inside a choice"
    );
    assert!(flattens("max({size(v, 1), 1})"), "inside a list");
    assert!(flattens("max([size(v, 1); 1])"), "inside a matrix");
    assert!(flattens("integer(size(v, 1))"), "inside a call");
    assert!(flattens("size(v, 1) * 1"), "a product");
    // A truth and a negation of one, a range read for its length, and
    // a place picked out of a list.
    assert!(
        flattens("if not (size(v, 1) < 2) and size(v, 1) > 1 then 3 else 1"),
        "inside `not` and `and`"
    );
    assert!(
        flattens("if size(v, 1) > 9 or size(v, 1) == 3 then 3 else 1"),
        "inside `or`"
    );
    // A comprehension over a range whose bound asks the question is
    // not answered here: what it comes to is worked out by the pass
    // that expands arrays, and this one only replaces the question
    // where it stands. The value is still read correctly where the
    // parameter is settled without an `extends` in the way.
    let comprehended = parse_model(
        "model M parameter Real v[3] = {1, 2, 3}; \
         parameter Integer n = size({0 for i in 1:size(v, 1)}, 1); \
         Real y; equation y = n; end M;",
    );
    assert!(comprehended.is_ok(), "{comprehended:?}");
}

/// A range says its length by its bounds, and a step of its own counts
/// the places between them. A range that cannot be settled leaves the
/// length unmeasured rather than guessing at it.
#[test]
fn a_range_read_for_a_flexible_size_counts_its_places() {
    let n = |written: &str| {
        let source = format!(
            "package P block B parameter Integer r[:] = {written}; \
             parameter Integer n = size(r, 1); output Real y; \
             equation y = n; end B; \
             model M B b; Real z; equation z = b.y; end M; end P;"
        );
        parse_model(&source).map(|m| {
            m.components
                .iter()
                .find(|c| c.name == "b.n")
                .and_then(|c| c.binding.clone())
                .map(|e| format!("{e:?}"))
                .unwrap_or_default()
        })
    };
    // Two to five is four places; a step of two makes it two.
    assert!(n("2:5").unwrap().contains("4.0"), "{:?}", n("2:5"));
    assert!(n("2:2:5").unwrap().contains("2.0"), "{:?}", n("2:2:5"));
    // A range that runs backwards holds nothing.
    assert!(n("5:2").unwrap().contains("0.0"), "{:?}", n("5:2"));
}

/// A name sliced by an empty range is the empty array, not a scalar.
#[test]
fn a_slice_by_an_empty_range_holds_nothing() {
    // `X_default[1:nXi]` with one substance and none of it independent
    // is how the standard media write "no trace substances here". The
    // range says the slice is empty; the name it is written on need
    // not be looked at to know that. Read as a scalar instead, the
    // range was refused for being an array, and every fluid sensor in
    // the library stopped at it.
    let m = parse_model(
        "package M \
           partial package Base \
             constant Integer nS = 1; \
             constant Boolean reducedX = true; \
             constant Integer nXi = if reducedX then nS - 1 else nS; \
             constant Real reference_X[nS] = fill(1/nS, nS); \
             constant Real X_default[nS] = reference_X; \
           end Base; \
           package Water extends Base(nS = 1); end Water; \
           model Sensor \
             replaceable package Medium = Base; \
             Real Xi_outflow[Medium.nXi]; \
             Real y; \
           equation \
             Xi_outflow = Medium.X_default[1:Medium.nXi]; \
             y = time; \
           end Sensor; \
           model E Sensor s(redeclare package Medium = Water); end E; \
         end M;",
    )
    .expect("an empty slice is an empty array");
    // Nothing is left of the slice: no equation for it, and the one
    // equation the sensor has is the one that says what y is.
    assert!(
        m.components.iter().all(|c| c.name != "s.Xi_outflow[1]"),
        "an empty slice made an element"
    );
    assert!(
        m.equations
            .iter()
            .any(|e| matches!(&e.lhs, Expr::Ref(name) if name == "s.y")),
        "the model lost the equation that follows the slice"
    );
}

/// A length written as arithmetic on a `size` is a length.
#[test]
fn a_length_may_be_a_size_with_something_done_to_it() {
    // A transfer function has one state fewer than its denominator
    // has coefficients, and the standard library writes exactly that:
    // `x_start[size(a, 1) - 1]`. The shapes are measured and written
    // down under the instance path, and `size` was answered only
    // where it stood alone as the whole dimension - not where it sat
    // inside arithmetic, which is where the library puts it.
    let m = parse_model(
        "package P \
           block TF \
             parameter Real a[:] = {1, 1}; \
             parameter Real x_start[size(a, 1) - 1] = zeros(size(a, 1) - 1); \
             Real y; \
           equation \
             y = time + x_start[1]; \
           end TF; \
           model E TF f(a = {1, 2, 1}); end E; \
         end P;",
    )
    .expect("a length worked out of a size");
    // Three coefficients, so two states.
    let held = m
        .components
        .iter()
        .filter(|c| c.name.starts_with("f.x_start["))
        .count();
    assert_eq!(held, 2, "the run came out {held} long");
}
