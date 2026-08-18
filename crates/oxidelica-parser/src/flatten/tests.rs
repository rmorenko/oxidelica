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
