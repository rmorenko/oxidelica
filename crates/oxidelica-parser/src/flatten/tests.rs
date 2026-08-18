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
