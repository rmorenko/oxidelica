//! The compile-time constant folder, which has no public face: the
//! rest of flattening is checked from outside the crate.

use crate::parser::parse_model;

#[test]
fn a_function_body_may_be_written_in_arrays_throughout() {
    // Inlining substitutes the arguments into the body, and the
    // body may be written in any of the array forms - which the
    // substitution has to walk through to reach the names.
    let m = parse_model(
        "function shaped input Real v[3]; input Real k; output Real y; protected Real ranged[3]; Real made[3]; Real rows[2, 3]; Real picked[2]; algorithm ranged := 1:3; made := {v[i] * k for i in 1:3}; rows := [v[1], v[2], v[3]; made[1], made[2], made[3]]; picked := {rows[1, 1], rows[2, 3]}; y := sum(ranged) + sum(made) + picked[1] + picked[2] + rows[2, 2]; end shaped; model M parameter Real a[3] = {1, 2, 3}; Real out; equation out = shaped(a, 2); end M;",
    )
    .unwrap();
    // 6 for the range, 12 for the doubled vector, 1 and 6 for the
    // corners picked out, and 4 in the middle.
    let mut known = std::collections::HashMap::new();
    for component in &m.components {
        if let Some(binding) = &component.binding {
            if let Some(number) = super::const_eval(binding, &known) {
                known.insert(component.name.clone(), number);
            }
        }
    }
    assert_eq!(super::const_eval(&m.equations[0].rhs, &known), Some(29.0));
}

#[test]
fn every_kind_of_expression_survives_being_inside_a_component() {
    // Prefixing, substitution and constant folding all walk the
    // same shapes, and inside a component they all have work to
    // do: at the top level the prefix is empty and they pass
    // straight through.
    let m = parse_model(
        "package K constant Real gain = 2; end K; function pick input Real v[3]; input Integer at; output Real y; algorithm y := v[at]; end pick; model Sub parameter Integer n = 3; parameter Real k[3] = {1, 2, 3}; Real v[3]; Real w[3]; Real mm[2, 2]; Real chosen; Real ranged[3]; Real made[3]; Real total; Boolean flag; equation v = k .* K.gain; w = -v .+ {1, 1, 1}; mm = [1, 2; 3, 4]; ranged = 1:3; made = {i * i for i in 1:3}; chosen = pick(v, 2); total = sum(v[1:end]) + max(v[1], v[3]); flag = not (v[1] > 0 and v[2] < 9) or false; end Sub; model M Sub s; Real out; equation out = s.total + s.chosen + s.mm[2, 2] + s.made[3] + s.ranged[2] + (if s.flag then 1 else 0); end M;",
    )
    .unwrap();
    // Everything folded to numbers the compiler could see.
    // Parameters stay symbolic - the tuner moves them - so they
    // are handed to the folding here.
    let mut known = std::collections::HashMap::new();
    for component in &m.components {
        if let Some(binding) = &component.binding {
            if let Some(number) = super::const_eval(binding, &known) {
                known.insert(component.name.clone(), number);
            }
        }
    }
    let value = |name: &str| {
        let equation = m
            .equations
            .iter()
            .find(|e| format!("{:?}", e.lhs) == format!("Ref({name:?})"))
            .unwrap_or_else(|| panic!("no equation for {name}"));
        super::const_eval(&equation.rhs, &known)
    };
    // What stands on parameters alone folds to a number.
    assert_eq!(value("s.v[3]"), Some(6.0));
    assert_eq!(value("s.mm[2,2]"), Some(4.0));
    assert_eq!(value("s.ranged[2]"), Some(2.0));
    assert_eq!(value("s.made[3]"), Some(9.0));
    // What stands on a variable keeps its shape, prefixed all the
    // way down.
    let shape = |name: &str| {
        let equation = m
            .equations
            .iter()
            .find(|e| format!("{:?}", e.lhs) == format!("Ref({name:?})"))
            .unwrap_or_else(|| panic!("no equation for {name}"));
        format!("{:?}", equation.rhs)
    };
    assert!(
        shape("s.w[1]").contains("Ref(\"s.v[1]\")"),
        "{}",
        shape("s.w[1]")
    );
    assert_eq!(shape("s.chosen"), "Ref(\"s.v[2]\")");
    assert!(shape("s.total").contains("Ref(\"s.v[3]\")"));
    assert!(shape("s.flag").contains("Ref(\"s.v[2]\")"));
}

#[test]
fn a_record_may_say_what_its_operators_mean() {
    const COMPLEX: &str = "operator record Complex Real re; Real im; \
         encapsulated operator function '+' input Complex a; input Complex b; \
         output Complex c; algorithm c := Complex(a.re + b.re, a.im + b.im); end '+';\
         encapsulated operator function '*' input Complex a; input Complex b; \
         output Complex c; algorithm \
         c := Complex(a.re * b.re - a.im * b.im, a.re * b.im + a.im * b.re); end '*';\
         encapsulated operator '-' \
         function negate input Complex a; output Complex c; \
         algorithm c := Complex(-a.re, -a.im); end negate; \
         function subtract input Complex a; input Complex b; output Complex c; \
         algorithm c := Complex(a.re - b.re, a.im - b.im); end subtract; end '-';\
         end Complex;";

    // Each operator gives one equation per field, folded to the
    // numbers the arithmetic works out to.
    let m = parse_model(&format!(
        "{COMPLEX} model M Complex s; Complex p; \
         equation s = Complex(1, 2) + Complex(3, 4); \
         p = Complex(1, 2) * Complex(3, 4); end M;"
    ))
    .unwrap();
    let value = |name: &str| {
        let equation = m
            .equations
            .iter()
            .find(|e| format!("{:?}", e.lhs) == format!("Ref({name:?})"))
            .unwrap_or_else(|| panic!("no equation for {name}"));
        super::const_eval(&equation.rhs, &std::collections::HashMap::new())
            .unwrap_or_else(|| panic!("{name} is not a number"))
    };
    assert_eq!((value("s.re"), value("s.im")), (4.0, 6.0));
    // (1 + 2i)(3 + 4i) = -5 + 10i.
    assert_eq!((value("p.re"), value("p.im")), (-5.0, 10.0));

    // A symbol may name a package of overloads: `-` is both the
    // difference of two and the negation of one, told apart by how
    // many arguments they take.
    let m = parse_model(&format!(
        "{COMPLEX} model M Complex d; Complex n; \
         equation d = Complex(1, 2) - Complex(3, 4); n = -Complex(1, 2); end M;"
    ))
    .unwrap();
    let value = |name: &str| {
        let equation = m
            .equations
            .iter()
            .find(|e| format!("{:?}", e.lhs) == format!("Ref({name:?})"))
            .unwrap();
        super::const_eval(&equation.rhs, &std::collections::HashMap::new()).unwrap()
    };
    assert_eq!((value("d.re"), value("d.im")), (-2.0, -2.0));
    assert_eq!((value("n.re"), value("n.im")), (-1.0, -2.0));

    // An operator the record does not offer is refused by name.
    let error = parse_model(&format!(
        "{COMPLEX} model M Complex a; Complex q; \
         equation a = Complex(1, 2); q = a / a; end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(error.contains("no operator `/`"), "{error}");

    // So is a constructor given the wrong number of fields.
    let error = parse_model(&format!(
        "{COMPLEX} model M Complex a; equation a = Complex(1); end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(error.contains("2 field(s), 1 given"), "{error}");
}

#[test]
fn a_record_may_declare_a_constructor_and_a_comparison() {
    // An operator record with a `'constructor'` that scales its inputs,
    // a `'<'` that compares by magnitude, and a `'+'` written field by
    // field. Each output is built up member by member, `v.x := ...`.
    const V: &str = "operator record V Real x; Real y; \
         encapsulated operator function 'constructor' input Real a; input Real b; \
         output V v; algorithm v.x := a * 2; v.y := b * 2; end 'constructor'; \
         encapsulated operator function '+' input V a; input V b; output V c; \
         algorithm c.x := a.x + b.x; c.y := a.y + b.y; end '+'; \
         encapsulated operator function '<' input V a; input V b; output Boolean r; \
         algorithm r := a.x * a.x + a.y * a.y < b.x * b.x + b.y * b.y; end '<'; end V; ";

    let m = parse_model(&format!(
        "{V} model M V p; V q; Boolean less; \
         equation p = V(1, 2); q = V(1, 1) + V(2, 2); less = V(1, 0) < V(3, 0); end M;"
    ))
    .unwrap();
    let value = |name: &str| {
        let equation = m
            .equations
            .iter()
            .find(|e| format!("{:?}", e.lhs) == format!("Ref({name:?})"))
            .unwrap_or_else(|| panic!("no equation for {name}"));
        super::const_eval(&equation.rhs, &std::collections::HashMap::new())
            .unwrap_or_else(|| panic!("{name} is not a number: {:?}", equation.rhs))
    };
    // The constructor doubled each field.
    assert_eq!((value("p.x"), value("p.y")), (2.0, 4.0));
    // V(1,1) + V(2,2), each doubled first, is (2+4, 2+4).
    assert_eq!((value("q.x"), value("q.y")), (6.0, 6.0));
    // |(1,0)| < |(3,0)| is true.
    assert_eq!(value("less"), 1.0);

    // The comparison the other way is false.
    let m = parse_model(&format!(
        "{V} model M Boolean less; equation less = V(3, 0) < V(1, 0); end M;"
    ))
    .unwrap();
    let less = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"less\")")
        .unwrap();
    assert_eq!(
        super::const_eval(&less.rhs, &std::collections::HashMap::new()),
        Some(0.0)
    );
}

#[test]
fn a_record_may_say_how_it_reads() {
    // `String(a)` on a record is what the record's `'String'` operator
    // makes of it, and it folds like any other string: the comparison
    // below settles before the run.
    const V: &str = "operator record V Real x; Real y; \
         encapsulated operator function 'String' input V a; output String s; \
         algorithm s := \"V=\" + String(a.x); end 'String'; end V; ";

    let m = parse_model(&format!(
        "{V} model M V p; Real z; \
         equation p = V(3, 0); z = if String(p) == \"V=3\" then 1 else 0; end M;"
    ))
    .unwrap();
    let z = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"z\")")
        .unwrap();
    assert!(
        format!("{:?}", z.rhs).contains("Bool(true)"),
        "the record's own String was not used: {:?}",
        z.rhs
    );

    // And it reads a different value differently.
    let m = parse_model(&format!(
        "{V} model M V p; Real z; \
         equation p = V(9, 0); z = if String(p) == \"V=3\" then 1 else 0; end M;"
    ))
    .unwrap();
    let z = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"z\")")
        .unwrap();
    assert!(
        format!("{:?}", z.rhs).contains("Bool(false)"),
        "{:?}",
        z.rhs
    );

    // `String` of a plain number still folds, now that the string layer
    // knows what a parameter is worth.
    let m = parse_model(
        "model M parameter Real k = 7; parameter String s = \"k=\" + String(k); \
         Real z; equation z = if s == \"k=7\" then 1 else 0; end M;",
    )
    .unwrap();
    let z = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"z\")")
        .unwrap();
    assert!(format!("{:?}", z.rhs).contains("Bool(true)"), "{:?}", z.rhs);
}

#[test]
fn a_record_may_say_what_its_zero_is() {
    // `sum` over an array of records adds them with the record's own
    // `'+'`, starting from its `'0'` - which is what that operator is
    // for. The result is one addition per element, off zero.
    const V: &str = "operator record V Real x; Real y; \
         encapsulated operator function '0' output V z; \
         algorithm z.x := 0; z.y := 0; end '0'; \
         encapsulated operator function '+' input V a; input V b; output V c; \
         algorithm c.x := a.x + b.x; c.y := a.y + b.y; end '+'; end V; ";

    let m = parse_model(&format!(
        "{V} model M V arr[3]; V total; Real z; \
         equation for i in 1:3 loop arr[i].x = i; arr[i].y = 0; end for; \
         total = sum(arr); z = total.x; end M;"
    ))
    .unwrap();
    let total = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"total.x\")")
        .unwrap();
    let text = format!("{:?}", total.rhs);
    // Every element is in the sum, and it started from the zero.
    for element in ["arr[1].x", "arr[2].x", "arr[3].x"] {
        assert!(text.contains(element), "{element} is missing: {text}");
    }
    assert!(
        text.contains("Number(0.0)"),
        "the zero was not used: {text}"
    );

    // Without a `'0'` the first element starts the sum instead, so the
    // same model still adds up - with one addition fewer.
    const NO_ZERO: &str = "operator record W Real x; \
         encapsulated operator function '+' input W a; input W b; output W c; \
         algorithm c.x := a.x + b.x; end '+'; end W; ";
    let m = parse_model(&format!(
        "{NO_ZERO} model M W arr[2]; W total; Real z; \
         equation for i in 1:2 loop arr[i].x = i; end for; \
         total = sum(arr); z = total.x; end M;"
    ))
    .unwrap();
    let total = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"total.x\")")
        .unwrap();
    assert_eq!(
        format!("{:?}", total.rhs),
        "Bin(Add, Ref(\"arr[1].x\"), Ref(\"arr[2].x\"))"
    );
}

#[test]
fn a_constant_may_be_the_length_of_a_constant_array() {
    // A medium counts its trace substances as
    // `size(extraPropertiesNames, 1)`, and the names are a constant
    // array with a value of its own. Nothing else can answer that
    // length: the array is a constant of a package rather than a
    // declaration anywhere near the connector it sizes.
    let m = parse_model(
        "partial package PartialMedium \
           constant String extraPropertiesNames[:] = fill(\"\", 0); \
           final constant Integer nC = size(extraPropertiesNames, 1); \
         end PartialMedium; \
         package Water extends PartialMedium; end Water; \
         model Port replaceable package Medium = PartialMedium; \
           Real C_outflow[Medium.nC]; Real p; equation p = 1; end Port; \
         model M Port one(redeclare package Medium = Water); end M;",
    )
    .unwrap();

    // The length came out nought, so the connector carries no trace
    // substances and nothing named `C_outflow` is in the flat model.
    assert!(m.components.iter().any(|c| c.name == "one.p"));
    assert!(!m.components.iter().any(|c| c.name.contains("C_outflow")));

    // The other ways a constant array says how long it is, and one
    // that says nothing: a value written out, `zeros`, and a name.
    let sized = |names: &str, sizing: &str| {
        parse_model(&format!(
            "package Held constant Real known[:] = {names};                final constant Integer n = {sizing}; end Held;              model M Real v[Held.n]; equation v = fill(1.0, Held.n); end M;"
        ))
    };
    // Written out: three elements, so three scalars come through.
    let three = sized("{1.0, 2.0, 3.0}", "size(known, 1)").unwrap();
    assert_eq!(
        three
            .components
            .iter()
            .filter(|c| c.name.starts_with("v["))
            .count(),
        3
    );
    // `zeros(2)` says two.
    let two = sized("zeros(2)", "size(known, 1)").unwrap();
    assert_eq!(
        two.components
            .iter()
            .filter(|c| c.name.starts_with("v["))
            .count(),
        2
    );
    // A second axis of a one-dimensional array is not a length, and
    // neither is the size of something that is not an array at all.
    for asking in ["size(known, 2)", "size(elsewhere, 1)"] {
        assert!(sized("zeros(2)", asking).is_err(), "{asking}");
    }
}

#[test]
fn a_constant_array_says_its_length_the_several_ways_it_is_written() {
    use crate::ast::Expr;
    use crate::flatten::constants::array_length;

    let number = |n: f64| Expr::Number(n);
    let call = |name: &str, args: Vec<Expr>| Expr::Call(name.to_string(), args);

    // Written out, the length is how many were written.
    let written = Expr::Array(vec![number(1.0), number(2.0), number(3.0)]);
    assert_eq!(array_length(&written, 0), Some(3));
    // A written-out array says nothing about a second axis here: the
    // one place this is asked from wants the first.
    assert_eq!(array_length(&written, 1), None);

    // `fill` says its lengths after the value it repeats, `zeros` and
    // `ones` say theirs outright.
    assert_eq!(
        array_length(
            &call("fill", vec![Expr::Str(String::new()), number(0.0)]),
            0
        ),
        Some(0)
    );
    assert_eq!(
        array_length(
            &call("fill", vec![number(1.0), number(2.0), number(4.0)]),
            1
        ),
        Some(4)
    );
    assert_eq!(array_length(&call("zeros", vec![number(2.0)]), 0), Some(2));
    assert_eq!(array_length(&call("ones", vec![number(5.0)]), 0), Some(5));

    // An axis the value does not have, a length that is not a whole
    // number, a negative one, and something that is not an array at
    // all: none of them is a length.
    assert_eq!(array_length(&call("zeros", vec![number(2.0)]), 1), None);
    assert_eq!(array_length(&call("zeros", vec![number(1.5)]), 0), None);
    assert_eq!(array_length(&call("zeros", vec![number(-1.0)]), 0), None);
    assert_eq!(array_length(&number(3.0), 0), None);
    assert_eq!(array_length(&call("sin", vec![number(0.0)]), 0), None);
}

#[test]
fn a_constant_written_as_a_length_is_measured_or_left_alone() {
    use crate::ast::Expr;
    use crate::flatten::constants::measured_constant;
    use std::collections::HashMap;

    let held: Vec<(String, Option<Expr>)> = vec![
        (
            "names".to_string(),
            Some(Expr::Call(
                "fill".to_string(),
                vec![Expr::Str(String::new()), Expr::Number(0.0)],
            )),
        ),
        ("bare".to_string(), None),
    ];
    let values = HashMap::new();
    let size = |args: Vec<Expr>| Expr::Call("size".to_string(), args);
    let named = |n: &str| Expr::Ref(n.to_string());

    // The length of a constant array of the same package.
    assert_eq!(
        measured_constant(
            &size(vec![named("names"), Expr::Number(1.0)]),
            &held,
            &values
        ),
        Some(0.0)
    );
    // Everything that is not that: another call, a call with only the
    // array, a length off something written out rather than named, a
    // constant nobody declared, one with no value, and an axis the
    // value has not got.
    for asking in [
        Expr::Call("cos".to_string(), vec![Expr::Number(0.0)]),
        size(vec![named("names")]),
        size(vec![Expr::Number(1.0), Expr::Number(1.0)]),
        size(vec![named("elsewhere"), Expr::Number(1.0)]),
        size(vec![named("bare"), Expr::Number(1.0)]),
        size(vec![named("names"), Expr::Number(2.0)]),
        // An axis that is not a number the compiler can see.
        size(vec![named("names"), named("later")]),
    ] {
        assert_eq!(
            measured_constant(&asking, &held, &values),
            None,
            "{asking:?}"
        );
    }

    // And a length that is not one either: the array says how long it
    // is with a name rather than a number.
    let unsettled: Vec<(String, Option<Expr>)> = vec![(
        "names".to_string(),
        Some(Expr::Call(
            "zeros".to_string(),
            vec![Expr::Ref("later".to_string())],
        )),
    )];
    assert_eq!(
        measured_constant(
            &size(vec![named("names"), Expr::Number(1.0)]),
            &unsettled,
            &values
        ),
        None
    );
}

#[test]
fn min_and_max_take_two_numbers_or_one_array() {
    // A block takes the longer of two lengths with
    // `max([size(a, 1); size(b, 1)])`, which is one argument holding
    // both. A list is the same question written another way, and two
    // numbers are the form the folder always knew.
    let folded = |written: &str| {
        let source =
            format!("model M parameter Real k = {written}; Real y; equation y = k; end M;");
        let m = parse_model(&source).unwrap();
        let known = std::collections::HashMap::new();
        super::const_eval(m.components[0].binding.as_ref().unwrap(), &known)
    };
    assert_eq!(folded("max([2; 1])"), Some(2.0));
    assert_eq!(folded("min([2; 1])"), Some(1.0));
    assert_eq!(folded("max({3, 7, 5})"), Some(7.0));
    assert_eq!(folded("min({3, 7, 5})"), Some(3.0));
    // Rows of several cells, and the two-number form.
    assert_eq!(folded("max([1, 9; 4, 2])"), Some(9.0));
    assert_eq!(folded("max(2, 5)"), Some(5.0));
    // A name it cannot settle leaves the whole question unanswered,
    // wherever in the value it sits.
    assert_eq!(folded("max({1, unknown})"), None);
    assert_eq!(folded("max([1; unknown])"), None);
    assert_eq!(folded("min({{1, 2}, {3, 4}})"), Some(1.0));
}

#[test]
fn a_flexible_size_is_measured_only_where_it_can_be() {
    use crate::ast::Expr;
    let n = |v: f64| Expr::Number(v);
    // A matrix says its shape by how it is written.
    let matrix = Expr::MatrixRows(vec![vec![n(1.0), n(2.0)], vec![n(3.0), n(4.0)]]);
    assert_eq!(
        super::flexible_size(&matrix, 0, &std::collections::HashMap::new(), "", &[]),
        Some(2)
    );
    assert_eq!(
        super::flexible_size(&matrix, 1, &std::collections::HashMap::new(), "", &[]),
        Some(2)
    );
    // Deeper than it has axes, and rows of different widths, are no
    // shape at all rather than a guess.
    assert_eq!(
        super::flexible_size(&matrix, 2, &std::collections::HashMap::new(), "", &[]),
        None
    );
    let ragged = Expr::MatrixRows(vec![vec![n(1.0), n(2.0)], vec![n(3.0)]]);
    assert_eq!(
        super::flexible_size(&ragged, 0, &std::collections::HashMap::new(), "", &[]),
        None
    );
    let empty = Expr::MatrixRows(vec![]);
    assert_eq!(
        super::flexible_size(&empty, 0, &std::collections::HashMap::new(), "", &[]),
        None
    );
    // A list still says its length, and a name says nothing.
    assert_eq!(
        super::flexible_size(
            &Expr::Array(vec![n(1.0)]),
            0,
            &std::collections::HashMap::new(),
            "",
            &[]
        ),
        Some(1)
    );
    assert_eq!(
        super::flexible_size(
            &Expr::Ref("v".into()),
            0,
            &std::collections::HashMap::new(),
            "",
            &[]
        ),
        None
    );
}

#[test]
fn a_range_is_measured_only_along_its_one_axis() {
    use crate::ast::Expr;
    use std::collections::HashMap;
    let n = |v: f64| Box::new(Expr::Number(v));
    let consts = HashMap::new();
    let sizes = HashMap::new();
    let range = |from, step: Option<f64>, to| Expr::Range(n(from), step.map(n), n(to));
    // Two to five is four places, and a step of two makes it two.
    assert_eq!(
        super::shapes::range_length(&range(2.0, None, 5.0), 0, &consts, &sizes),
        Some(4)
    );
    assert_eq!(
        super::shapes::range_length(&range(2.0, Some(2.0), 5.0), 0, &consts, &sizes),
        Some(2)
    );
    // A range has one axis; there is no second to ask about.
    assert_eq!(
        super::shapes::range_length(&range(2.0, None, 5.0), 1, &consts, &sizes),
        None
    );
    // A step of nothing would never arrive, and a value that is not a
    // range is not one to measure.
    assert_eq!(
        super::shapes::range_length(&range(2.0, Some(0.0), 5.0), 0, &consts, &sizes),
        None
    );
    assert_eq!(
        super::shapes::range_length(&Expr::Number(1.0), 0, &consts, &sizes),
        None
    );
    // A bound the environment cannot settle leaves it unmeasured.
    let named = Expr::Range(n(1.0), None, Box::new(Expr::Ref("unknown".into())));
    assert_eq!(
        super::shapes::range_length(&named, 0, &consts, &sizes),
        None
    );
}

#[test]
fn the_walk_for_a_read_reaches_every_corner_of_an_expression() {
    use crate::ast::Expr;
    // Whether a value may be dropped turns on this answer, so the walk
    // has to be sure: a name in any corner counts, and a name in none
    // of them does not. These are the shapes an ordinary parse rarely
    // puts in front of it.
    let named = Expr::Ref("o".into());
    let other = Expr::Ref("p".into());
    let reads = |e: &Expr| super::algorithms::reads_name(e, "o");

    // A tuple of targets, and a slot left empty.
    let tuple = Expr::Tuple(vec![None, Some(named.clone())]);
    assert!(reads(&tuple));
    assert!(!reads(&Expr::Tuple(vec![None, Some(other.clone())])));

    // A value carrying its own rule of differentiation: the value, the
    // rule and the seeds are all read.
    let derived = |value: Expr, rule: Expr, seed: Expr| {
        Expr::WithDerivative(
            Box::new(value),
            Box::new(rule),
            vec![("x".to_string(), seed)],
        )
    };
    assert!(reads(&derived(named.clone(), other.clone(), other.clone())));
    assert!(reads(&derived(other.clone(), named.clone(), other.clone())));
    assert!(reads(&derived(other.clone(), other.clone(), named.clone())));
    assert!(!reads(&derived(
        other.clone(),
        other.clone(),
        other.clone()
    )));

    // An argument given by name, and a subscript reaching the whole.
    assert!(reads(&Expr::NamedArg("k".into(), Box::new(named.clone()))));
    assert!(reads(&Expr::Ref("o[1]".into())));
    assert!(!reads(&Expr::Ref("other".into())));
}

#[test]
fn whether_a_name_is_read_later_sees_every_kind_of_statement() {
    use crate::ast::{Expr, Statement, StatementBranch};
    let named = || Expr::Ref("o".into());
    let other = || Expr::Ref("p".into());
    let reads = |body: Vec<Statement>| super::algorithms::read_later(&body, "o", 0);

    // The target of an assignment is written, not read - but the
    // subscripts that find the place are read, and so is the value.
    assert!(!reads(vec![Statement::Assign("o".into(), vec![], other())]));
    assert!(reads(vec![Statement::Assign("q".into(), vec![], named())]));
    assert!(reads(vec![Statement::Assign(
        "q".into(),
        vec![named()],
        other()
    )]));

    // A tuple of targets, its subscripts and its value.
    assert!(reads(vec![Statement::TupleAssign(
        vec![Some(("q".into(), vec![named()]))],
        other()
    )]));
    assert!(reads(vec![Statement::TupleAssign(vec![None], named())]));

    // A branch reads through its condition and through its body.
    let branch =
        |condition: Option<Expr>, body: Vec<Statement>| StatementBranch { condition, body };
    assert!(reads(vec![Statement::If(vec![branch(
        Some(named()),
        vec![]
    )])]));
    assert!(reads(vec![Statement::When(vec![branch(
        None,
        vec![Statement::Assign("q".into(), vec![], named())]
    )])]));

    // A loop reads through its range and its body; a `while` through
    // its condition and its body.
    assert!(reads(vec![Statement::For(
        "i".into(),
        Some(named()),
        vec![]
    )]));
    assert!(reads(vec![Statement::While(named(), vec![])]));
    assert!(reads(vec![Statement::While(
        other(),
        vec![Statement::Assign("q".into(), vec![], named())]
    )]));

    // A check and a call read their arguments; leaving reads nothing.
    assert!(reads(vec![Statement::Assert(
        named(),
        Expr::Str("m".into())
    )]));
    assert!(reads(vec![Statement::Call("f".into(), vec![named()])]));
    assert!(!reads(vec![Statement::Break, Statement::Return]));

    // Too deep to say no safely: merging a name nothing reads costs
    // work, refusing one something reads is wrong.
    assert!(super::algorithms::read_later(&[], "o", 1_000));
}

#[test]
fn a_state_whose_fields_arrive_from_below_is_still_a_record() {
    // A medium leaves `ThermodynamicState` empty in the interface and
    // fills it by `redeclare record extends`, and a vessel hands the
    // state to a submodel as a modifier: `HeatTransfer ht(final states
    // = {medium.state})`. Resolved plainly, the declaration inside the
    // submodel lands on the interface's empty record, an equation
    // between two records of no fields is an equation between two
    // empty lists, and it is dropped without a word - which leaves the
    // fields declared and nothing to name them.
    let m = parse_model(
        "package P \
         package PartialMedium \
           replaceable record ThermodynamicState end ThermodynamicState; \
           replaceable partial model BaseProperties Real p; Real T; \
             ThermodynamicState state; end BaseProperties; \
         end PartialMedium; \
         package Water extends PartialMedium; \
           redeclare record extends ThermodynamicState Real p; Real T; \
           end ThermodynamicState; \
           redeclare model extends BaseProperties \
           equation state.p = p; state.T = T; end BaseProperties; \
         end Water; \
         partial model PartialHT replaceable package Medium = PartialMedium; \
           input Medium.ThermodynamicState states[1]; Real Ts[1] = {states[1].T}; \
         end PartialHT; \
         model IdealHT extends PartialHT; Real port[1]; \
         equation Ts = port; end IdealHT; \
         model Vessel replaceable package Medium = PartialMedium; \
           Medium.BaseProperties medium; \
           IdealHT ht(redeclare package Medium = Medium, \
             final states = {medium.state}); \
         equation medium.p = 1e5; medium.T = 300; end Vessel; \
         model M Vessel v(redeclare package Medium = Water); end M; \
         end P;",
    )
    .unwrap();
    let named = |name: &str| {
        m.equations
            .iter()
            .any(|e| format!("{:?}", e.lhs) == format!("Ref({name:?})"))
    };
    assert!(
        named("v.ht.states[1].p") && named("v.ht.states[1].T"),
        "the state handed to the submodel went unnamed"
    );
}

#[test]
fn a_state_below_a_subscript_is_still_a_record_where_the_equation_is_written() {
    // A pipe holds its media as an array - `Medium.BaseProperties[n]
    // mediums` - and writes an equation between one of their states
    // and a state it declared itself: `statesFM[1] = mediums[1].state`.
    // The walk that files what is a record descends into each element
    // of the array and files `mediums[1].state` under the medium's
    // own record, but it does so as a walk of its own: the class
    // writing the equation never steps through the subscript, so its
    // table holds `statesFM` and not the other side. One side is then
    // written out as its fields and the other stays a bare name, and
    // what the compiler sees is an equation between shapes that do
    // not fit.
    let m = parse_model(
        "package Q \
         package PartialMedium \
           replaceable record ThermodynamicState end ThermodynamicState; \
           replaceable partial model BaseProperties Real p; Real T; \
             ThermodynamicState state; end BaseProperties; \
         end PartialMedium; \
         package Water extends PartialMedium; \
           redeclare record extends ThermodynamicState Real p; Real T; \
           end ThermodynamicState; \
           redeclare model extends BaseProperties \
           equation state.p = p; state.T = T; end BaseProperties; \
         end Water; \
         partial model PartialDistributedVolume \
           replaceable package Medium = PartialMedium; \
           parameter Integer n = 2; Medium.BaseProperties mediums[n]; \
         end PartialDistributedVolume; \
         partial model PartialTwoPortFlow \
           extends PartialDistributedVolume; \
           Medium.ThermodynamicState statesFM[2]; \
         equation statesFM[1] = mediums[1].state; \
           statesFM[2] = mediums[n].state; end PartialTwoPortFlow; \
         model Pipe extends PartialTwoPortFlow; \
         equation for i in 1:n loop mediums[i].p = 1e5; \
           mediums[i].T = 300; end for; end Pipe; \
         model M Pipe pipe1(redeclare package Medium = Water); end M; \
         end Q;",
    )
    .unwrap();
    let named = |name: &str| {
        m.equations
            .iter()
            .any(|e| format!("{:?}", e.lhs) == format!("Ref({name:?})"))
    };
    assert!(
        named("pipe1.statesFM[1].p") && named("pipe1.statesFM[1].T"),
        "the state below a subscript was not read as a record"
    );
}

/// A reduction over an elementwise product keeps the reduction.
///
/// A machine writes `powerStator = sum(vs .* is)` about two arrays it
/// reads off its own plugs. Both are met before the plug array exists,
/// so each measures as a scalar; the product then came to a single
/// term, the reduction reduced nothing and vanished, and what was left
/// was the bare `vs .* is`. The pass that writes whole-array equations
/// out element by element found two whole arrays in it and wrote the
/// equation once per phase - seven equations at seven phases, six of
/// them false, and the balance check counting all seven. That is the
/// machines' surplus of `m - 1`, and worse than the refusal it caused:
/// where a model did balance, the stator power of one phase came out
/// presented as the power of all of them.
///
/// The guard is tested here rather than through a flattened model on
/// purpose. What makes the names late is the depth of the machine's
/// inheritance, and a model small enough to write out is a model whose
/// shapes are all in hand by the time the reduction is met - the
/// lateness cannot be reached at that size. The corpus is the witness
/// that the whole path is fixed; this is the witness for the rule.
#[test]
fn a_reduction_over_an_elementwise_product_keeps_the_reduction() {
    use crate::ast::{BinOp, Expr};
    use crate::flatten::builtins::unmeasured_name_under;
    use crate::flatten::{no_records, Shapes};
    use std::collections::HashMap;
    let sizes: HashMap<String, Vec<i64>> = [("m.known".to_string(), vec![3])].into_iter().collect();
    let empty = HashMap::new();
    let consts = HashMap::new();
    let shapes = Shapes {
        sizes: &sizes,
        loop_vars: &empty,
        consts: &consts,
        records: no_records(),
    };
    let named = |name: &str| Expr::Ref(name.to_string());
    // A name whose array is not built yet, standing alone, was always
    // held back.
    assert!(unmeasured_name_under(&named("m.vs"), &shapes));
    // The same name under an elementwise product is exactly as late,
    // and this is what was missed.
    assert!(unmeasured_name_under(
        &Expr::Elementwise(BinOp::Mul, Box::new(named("m.vs")), Box::new(named("m.is"))),
        &shapes
    ));
    // A plain operator between the two is late for the same reason.
    assert!(unmeasured_name_under(
        &Expr::Bin(BinOp::Add, Box::new(named("m.vs")), Box::new(named("m.is"))),
        &shapes
    ));
    // A name that was measured is not late, whatever it stands under,
    // and holding a reduction back over one would leave a `sum` nobody
    // opens.
    assert!(!unmeasured_name_under(&named("m.known"), &shapes));
    assert!(!unmeasured_name_under(
        &Expr::Elementwise(
            BinOp::Mul,
            Box::new(named("m.known")),
            Box::new(named("m.known"))
        ),
        &shapes
    ));
}

#[test]
fn a_record_constant_is_read_past_a_package_of_the_same_name() {
    // `R_s = R_NASA_2002/Air.MM` is how every NASA ideal gas states
    // its gas constant, written beside a record constant called
    // `Air` - and `Modelica.Media.Air` is a package of that name one
    // branch over. The head resolved to the package, the package had
    // no `MM`, and the answer was nothing rather than the sibling's
    // field: the gas constant of every ideal gas was lost to a
    // namesake, and with it the density of nineteen fluid models.
    // The number is what is checked, not that anything flattened:
    // 8.314510 / 0.0289651159 is 287.05 J/(kg.K), the gas constant of
    // dry air.
    let m = parse_model(
        "package Air constant Real unrelated = 1; end Air; \
         record DataRecord Real MM; Real R_s; end DataRecord; \
         package GasData \
           constant Real R_NASA = 8.314510; \
           constant DataRecord Air(MM = 0.0289651159, R_s = R_NASA/Air.MM); \
         end GasData; \
         model M parameter Real r = GasData.Air.R_s; Real x; equation der(x) = r; end M;",
    )
    .unwrap();
    let read = m
        .components
        .iter()
        .find(|c| c.name == "r")
        .and_then(|c| c.binding.clone())
        .and_then(|b| super::const_eval(&b, &std::collections::HashMap::new()))
        .expect("the gas constant is a number");
    assert!(
        (read - 287.0525368759184).abs() < 1e-9,
        "gas constant came out {read}"
    );
}

#[test]
fn a_record_constant_named_bare_is_folded_by_its_own_package() {
    // Inside a medium's own functions the gas data is named bare -
    // `data.Tlimit` in the body of `specificEnthalpy`, and `data`
    // handed whole to `h_T`. A bare name standing for a record is
    // neither a number nor a list, so every road in the substitution
    // passed it by and it reached the flat model with the instance
    // path on its front as `medium.data`, which nothing declares.
    // Asked by its whole path the same record answers, so the package
    // that declares the name is found first and the dotted road taken
    // from there.
    //
    // The number is what is checked: the temperature limit between the
    // two NASA coefficient sets is 1000 K.
    let m = parse_model(
        "record DataRecord Real MM; Real Tlimit; end DataRecord; \
         package GasData constant DataRecord N2(MM = 0.0280134, Tlimit = 1000); end GasData; \
         package Common \
           function of_record input DataRecord d; output Real y; algorithm y := d.Tlimit; \
             end of_record; \
         end Common; \
         partial package SingleGas constant DataRecord data; \
           model Base Real t; equation t = Common.of_record(data); end Base; \
         end SingleGas; \
         package Nitrogen extends SingleGas(data = GasData.N2); end Nitrogen; \
         model M Nitrogen.Base b; Real x; equation der(x) = b.t; end M;",
    )
    .unwrap();
    let read = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == format!("Ref({:?})", "b.t"))
        .and_then(|e| super::const_eval(&e.rhs, &std::collections::HashMap::new()))
        .expect("the temperature limit is a number");
    assert!(
        (read - 1000.0).abs() < 1e-9,
        "temperature limit came out {read}"
    );
}

#[test]
fn a_record_constant_bound_to_another_record_constant_carries_its_fields() {
    // Every NASA ideal gas is written `extends SingleGasNasa(data =
    // Common.SingleGasesData.N2)`: the record constant is given the
    // name of another record constant rather than a constructor. The
    // field wanted is then that other record's field, and settling a
    // name that stands for a record folds nothing - so the whole of a
    // gas's coefficient table read as a variable nothing declares, and
    // the models that read it were refused over `data.Tlimit`.
    //
    // The number is what is checked, not that anything flattened: the
    // temperature limit between the two NASA coefficient sets is 1000 K.
    let m = parse_model(
        "record DataRecord Real MM; Real Tlimit; end DataRecord; \
         package GasData constant DataRecord N2(MM = 0.0280134, Tlimit = 1000); end GasData; \
         package Gas constant DataRecord data = GasData.N2; end Gas; \
         model M parameter Real t = Gas.data.Tlimit; Real x; equation der(x) = t; end M;",
    )
    .unwrap();
    let read = m
        .components
        .iter()
        .find(|c| c.name == "t")
        .and_then(|c| c.binding.clone())
        .and_then(|b| super::const_eval(&b, &std::collections::HashMap::new()))
        .expect("the temperature limit is a number");
    assert!(
        (read - 1000.0).abs() < 1e-9,
        "temperature limit came out {read}"
    );
}

#[test]
fn a_constructor_scaled_by_a_number_is_built_as_the_array_it_names() {
    // Every medium states the nominal size of its trace substances as
    // `C_nominal[nC] = 1.0e-6*ones(nC)`, with the length a constant of
    // whoever extends the interface. Built, the product was a bare name
    // nothing declares, and a volume scaling its trace masses by it
    // stood on that name.
    let flat = |binding: &str| {
        parse_model(&format!(
            "partial package PM \
               constant String extraPropertiesNames[:] = fill(\"\", 0); \
               final constant Integer nC = size(extraPropertiesNames, 1); \
               constant Real C_nominal[nC] = {binding}; \
             end PM; \
             package Air extends PM(extraPropertiesNames = {{\"CO2\", \"H2O\"}}); end Air; \
             model Vol replaceable package Medium = PM; \
               Real s[Medium.nC]; Real c[Medium.nC]; \
             equation s = ones(Medium.nC); c = s .* Medium.C_nominal; end Vol; \
             model M Vol v(redeclare package Medium = Air); end M;"
        ))
    };
    for (binding, factor) in [
        ("2.0e-6*ones(nC)", 2.0e-6),
        ("ones(nC)*2.0e-6", 2.0e-6),
        ("fill(6.0e-6, nC)/2", 3.0e-6),
    ] {
        let m = flat(binding).unwrap_or_else(|e| panic!("{binding}: {e}"));
        for at in 1..=2 {
            let equation = m
                .equations
                .iter()
                .find(|e| format!("{:?}", e.lhs) == format!("Ref(\"v.c[{at}]\")"))
                .unwrap_or_else(|| panic!("{binding}: no equation for v.c[{at}]"));
            let written = format!("{:?}", equation.rhs);
            assert!(
                written.contains(&format!("Number({factor:?})")),
                "{binding}: v.c[{at}] = {written}"
            );
        }
    }
}
