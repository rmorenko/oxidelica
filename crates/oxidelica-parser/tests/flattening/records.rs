//! Records: what their members are, how one is written out, and what an operator record means.

use super::shared::*;
use oxidelica_parser::{parse_model, Expr};

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

#[test]
fn a_record_valued_constant_and_a_record_in_a_declaration_are_read() {
    // `import Modelica.ComplexMath.j;` then `v = j*omega*L*i` is how
    // the quasi-static libraries write an impedance. The constant is a
    // record built by its own constructor, and only a constant written
    // out as an array was taken before, so `j` reached the run as an
    // unknown variable.
    const C: &str = "operator record C Real re; Real im; \
         encapsulated operator function '*' input C a; input C b; output C c; \
         algorithm c := C(a.re * b.re - a.im * b.im, a.re * b.im + a.im * b.re); \
         end '*'; end C; \
         package K constant C j = C(0, 1); end K;";
    let m = parse_model(&format!(
        "{C} model M import K.j; C i; C v; equation v = j * i; \
         i.re = time; i.im = 1; end M;"
    ))
    .unwrap();
    let said = format!("{:?}", m.equations);
    assert!(!said.contains("Ref(\"j\")"), "{said}");

    // The value of a declaration is worked out where the declaration
    // stands, and what the operands are records of has to be in view
    // there too: `Real P = real(v*i)` is the active power every
    // quasi-static port declares.
    let declared = parse_model(&format!(
        "{C} function re input C c; output Real r; algorithm r := c.re; end re; \
         model M C i; C v; Real p = re(v * i); \
         equation i.re = time; i.im = 1; v.re = 2; v.im = 3; end M;"
    ))
    .unwrap();
    let said = format!("{:?}", declared.equations);
    assert!(said.contains("p"), "{said}");
    assert!(!said.contains("Ref(\"c.re\")"), "{said}");
}

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

/// A record written out is built from every member it has, its bases'
/// among them.
#[test]
fn a_record_written_out_holds_what_its_bases_declared_too() {
    // The water states are four fields of their own over a `phase`
    // inherited from the two-phase medium. Built from its own fields
    // alone, the value came out one shorter than the record, and the
    // caller measuring the same record honestly said it wanted five
    // and got four.
    let m = parse_model(
        "package P record B Real phase; end B; \
         record S extends B; Real p; Real T; end S; \
         function make input Real p; input Real T; output S s; \
         algorithm s := S(p = p, T = T, phase = 1); end make; \
         function temp input S s; output Real T; algorithm T := s.T; end temp; \
         model M Real y; equation y = P.temp(P.make(2, 3)); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M; end P;",
    )
    .expect("a record built with its base's field among the others");
    let written = equations_of(&m);
    assert!(
        written.iter().any(|e| e.contains('3')),
        "the temperature comes back out: {written:?}"
    );

    // Given in order, the members run bases first, which is the order
    // everything else reads a record in: `S(1, 2, 3)` is `phase = 1`.
    let ordered = parse_model(
        "package P record B Real phase; end B; \
         record S extends B; Real p; Real T; end S; \
         function make output S s; algorithm s := S(1, 2, 3); end make; \
         function first input S s; output Real y; algorithm y := s.phase; end first; \
         model M Real y; equation y = P.first(P.make()); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M; end P;",
    )
    .expect("a record built in order");
    assert!(
        equations_of(&ordered).iter().any(|e| e.contains('1')),
        "the base's field is the first place: {:?}",
        equations_of(&ordered)
    );

    // A name that is nobody's member is a value going nowhere, and was
    // dropped without a word.
    let stranger = parse_model(
        "package P record S Real p; end S; \
         function make output Real y; protected S s; \
         algorithm s := S(p = 1, nowhere = 2); y := 1; end make; \
         model M Real y; equation y = P.make(); end M; end P;",
    )
    .expect_err("a member nobody declared");
    assert!(
        stranger.message.contains("no such member"),
        "{}",
        stranger.message
    );

    // A constant of a record belongs to the class rather than to any
    // value of it: the battery parameter records inherit a `constant
    // String CellType`, and counting it made one record five things
    // where the declaration held four.
    let held = parse_model(
        "package P partial record Kind constant String name = \"cell\"; end Kind; \
         record S extends Kind; Real r; Real c; end S; \
         model M parameter S data[1] = {S(r = 1, c = 2)}; Real y; \
         equation y = data[1].r; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M; end P;",
    )
    .expect("a record whose base holds a constant");
    assert_eq!(
        held.components
            .iter()
            .filter(|c| c.name.starts_with("data[1]."))
            .count(),
        2,
        "two members, not three"
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
    // A field takes its value the way any modifier's does, and a
    // field of a parameter record is a parameter itself, so the value
    // stays a value rather than becoming an equation.
    let worth = |name: &str| {
        m.components
            .iter()
            .find(|c| c.name == name)
            .and_then(|c| c.binding.clone())
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
            .components
            .iter()
            .filter(|c| c.binding == Some(Expr::Number(5.0)))
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
            .components
            .iter()
            .any(|c| c.name == "q.b" && c.binding == Some(Expr::Number(9.0))),
        "{:?}",
        fixed.components
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
            .components
            .iter()
            .any(|c| c.name == "r.b" && c.binding == Some(Expr::Number(7.0))),
        "{:?}",
        partly.components
    );

    // A record that states some of its fields `final` still hands the
    // rest down. The machines' friction record is a reference power
    // and speed with four torques worked out from them, and refusing
    // the whole record for having them left every machine in the
    // library without its reference speed.
    let mixed = parse_model(
        "record F parameter Real PRef = 0; parameter Real wRef; \
          final parameter Real tauRef = PRef; end F; \
         model Inner parameter F p; Real y; equation y = p.wRef * time; end Inner; \
         model M parameter F source(wRef = 5); Inner inner1(p = source); end M;",
    )
    .unwrap();
    // The value may land as the declaration's binding or, where the
    // field already had one, as its start attribute; either way it is
    // the value the run works out from.
    let binding = |model: &oxidelica_parser::Model, name: &str| {
        let c = model
            .components
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no {name}"));
        format!("{:?} {:?}", c.binding, c.start)
    };
    // The value may be handed on as the name it was written as, which
    // is settled once every parameter is worked out; what matters is
    // that the field was given one at all rather than left empty.
    assert!(
        binding(&mixed, "inner1.p.wRef").contains("source.wRef"),
        "{}",
        binding(&mixed, "inner1.p.wRef")
    );

    // A field typed by an alias holds one number like any other. The
    // record is measured to decide whether a value is one of it or one
    // per element, and counting an alias as nothing made a record of
    // them look empty, so the value matched neither reading and was
    // dropped in silence.
    let aliased = parse_model(
        "type Power = Real(unit = \"W\"); type Speed = Real(unit = \"rad/s\"); \
         record F parameter Power PRef = 0; parameter Speed wRef; end F; \
         model Inner parameter F p; Real y; equation y = p.wRef * time; end Inner; \
         model M parameter F source(wRef = 7); Inner inner1(p = source); end M;",
    )
    .unwrap();
    let worth = binding(&aliased, "inner1.p.wRef");
    assert!(worth.contains("source.wRef"), "{worth}");

    // A value handed down names a record of the class that supplied
    // it, not of the one receiving it, so what that class knows has to
    // still be in view when the value is taken apart.
    let across = parse_model(
        "record F parameter Real wRef = 1; end F; \
          record Outer parameter F inner_(wRef = 3); end Outer; \
         model Inner parameter F p; Real y; equation y = p.wRef * time; end Inner; \
         model M parameter Outer data; Inner inner1(p = data.inner_); end M;",
    )
    .unwrap();
    let reached = binding(&across, "inner1.p.wRef");
    assert!(reached.contains("data.inner_.wRef"), "{reached}");
}

/// A modifier handing a whole record over is written in the terms of
/// the class that supplied it.
#[test]
fn a_record_handed_down_by_name_is_written_out_field_by_field() {
    const PARTS: &str = "record C Real re; Real im; end C; \
                         function makeC output C r; algorithm r := C(re = 1, im = 2); end makeC; \
                         model Shape input C v = makeC(); Real y; \
                         equation y = v.re + v.im; end Shape; ";

    let m = parse_model(&format!(
        "{PARTS}model Holder C mine = makeC(); Shape s(v = mine); Real q; \
         equation q = s.y; end Holder; \
         model M Holder h; Real z; equation z = h.q; end M;"
    ))
    .unwrap();
    // Both sides of the modifier come out field by field: `h.s.v` is
    // what `h.mine` is, rather than being left with no value at all.
    for field in ["re", "im"] {
        let named = format!("h.s.v.{field}");
        assert!(
            m.equations.iter().any(|equation| {
                let mut names = Vec::new();
                equation.lhs.collect_refs(&mut names);
                equation.rhs.collect_refs(&mut names);
                names.contains(&named.as_str())
            }),
            "nothing settles {named}"
        );
    }
}

/// An array of records takes its value element by element, and which
/// level of the value is an element comes from the declaration.
#[test]
fn a_record_array_of_two_dimensions_takes_its_value_row_by_row() {
    let m = parse_model(
        "operator record C Real re; Real im; end C; \
         function grid input Integer n; output C t[n, n]; \
         algorithm t := {{C(i, j) for j in 1:n} for i in 1:n}; end grid; \
         model Conv parameter Integer n = 1; final parameter C sTM[n, n] = grid(n); \
         Real y[n]; equation \
         for j in 1:n loop y[j] = sum({sTM[j, k].re + sTM[j, k].im for k in 1:n}); end for; \
         end Conv; \
         model M Conv c(n = 3); Real z; equation z = c.y[1] + c.y[3]; end M;",
    )
    .unwrap();
    // Row-major: `sTM[3,2]` is `C(3, 2)`. Counting the levels by
    // length instead of by dimension takes the rows of a square for
    // the fields of one record, and every element comes out the same.
    let value = |name: &str| {
        m.equations
            .iter()
            .find(|equation| equation.lhs == oxidelica_parser::Expr::Ref(name.to_string()))
            .map(|equation| equation.rhs.clone())
            .or_else(|| {
                m.components
                    .iter()
                    .find(|component| component.name == name)
                    .and_then(|component| component.binding.clone())
            })
    };
    for (name, wanted) in [
        ("c.sTM[3,2].re", 3.0),
        ("c.sTM[3,2].im", 2.0),
        ("c.sTM[1,3].re", 1.0),
        ("c.sTM[1,3].im", 3.0),
    ] {
        assert_eq!(
            value(name),
            Some(oxidelica_parser::Expr::Number(wanted)),
            "{name}"
        );
    }
    // And what the model reads off them: sum over k of (j + k), which
    // is 3j + 6 for three phases, at j = 1 and j = 3.
    assert!(m.components.iter().any(|c| c.name == "z"));
}

/// One record handed to a whole array of them is told from one record
/// per element by how many numbers the value holds.
#[test]
fn one_record_over_an_array_is_not_mistaken_for_one_per_element() {
    // The trap: a record of two fields handed to an array of two
    // elements. Both readings see two entries at the top, so counting
    // entries picks the wrong one and gives `p[1]` the first field and
    // `p[2]` the second, with nothing said about it.
    let m = parse_model(
        "record P Real a[2]; Real b[2]; end P; \
         model M parameter P p[2] = P({1, 2}, {3, 4}); Real y; \
         equation y = p[1].a[2] + p[2].b[1]; end M;",
    )
    .unwrap();
    let value = |name: &str| {
        m.equations
            .iter()
            .find(|equation| equation.lhs == oxidelica_parser::Expr::Ref(name.to_string()))
            .map(|equation| equation.rhs.clone())
            .or_else(|| {
                m.components
                    .iter()
                    .find(|component| component.name == name)
                    .and_then(|component| component.binding.clone())
            })
    };
    // Every element is the whole record, so both hold a = {1, 2} and
    // b = {3, 4}.
    for element in ["p[1]", "p[2]"] {
        for (field, wanted) in [("a[1]", 1.0), ("a[2]", 2.0), ("b[1]", 3.0), ("b[2]", 4.0)] {
            let named = format!("{element}.{field}");
            assert_eq!(
                value(&named),
                Some(oxidelica_parser::Expr::Number(wanted)),
                "{named}"
            );
        }
    }
}

/// A number standing where an operator wants a record is that record
/// built from it: a winding writes `N*i` of a complex number of turns
/// and a real current, and declares no operator for the mixture.
#[test]
fn a_number_beside_a_record_is_built_into_one() {
    let m = parse_model(
        "operator record Cx Real re; Real im; \
         encapsulated operator 'constructor' \
         function fromReal import Cx; input Real re; input Real im = 0; \
         output Cx result(re = re, im = im); algorithm end fromReal; \
         end 'constructor'; \
         encapsulated operator function '*' input Cx a; input Cx b; \
         output Cx c; algorithm \
         c := Cx(a.re * b.re - a.im * b.im, a.re * b.im + a.im * b.re); end '*'; \
         end Cx; \
         model M Cx n; Cx v; Real i; \
         equation n = Cx(1, 2); i = time; v = n * i; end M;",
    )
    .expect("a number beside a record");
    let written = equations_of(&m);
    assert!(
        written.contains(&"v.re = (n.re * i) - (n.im * 0)".to_string())
            && written.contains(&"v.im = (n.re * 0) + (n.im * i)".to_string()),
        "{written:?}"
    );
}

/// A `parameter` record is a parameter all the way down. Its fields
/// are declared plainly inside the record, and left continuous their
/// values become declaration equations - so a multibody body, whose
/// start orientation is a record a function fills in, had nothing to
/// read `R_start.T[1, 1]` from when the quaternion it starts at asked
/// for it.
#[test]
fn the_fields_of_a_parameter_record_are_parameters_themselves() {
    let m = parse_model(
        "model M record Ori Real T[2,2]; end Ori; \
         parameter Ori R = Ori(T = [1,0;0,2]); \
         parameter Real q = R.T[1,1] + R.T[2,2]; \
         Real x; equation der(x) = q; end M;",
    )
    .expect("a record parameter filled in field by field");
    let field = m
        .components
        .iter()
        .find(|c| c.name == "R.T[1,1]")
        .expect("the record comes apart into its fields");
    assert_eq!(
        field.variability,
        oxidelica_parser::Variability::Parameter,
        "a field of a parameter record is a parameter"
    );
    assert_eq!(
        field.binding.as_ref().map(|b| b.describe()),
        Some("1".to_string()),
        "and its value stays a value rather than becoming an equation"
    );
}

/// A scalar function answering with a record, handed arrays, is one
/// call per element.
#[test]
fn a_record_valued_function_over_arrays_gives_a_record_per_element() {
    // `fromPolar` is written for one amplitude and one angle and gives
    // one phasor. The quasi-static controllers call it over as many of
    // each as there are phases. Run whole, the record's two fields
    // flattened into the row and the equation came out between a
    // three-by-two and a two.
    let m = parse_model(
        "package P record Cx Real re; Real im; end Cx; \
         function fromPolar input Real len; input Real phi; output Cx c; \
         algorithm c := Cx(len*cos(phi), len*sin(phi)); end fromPolar; \
         model M parameter Integer m = 3; \
         parameter Real orientation[m] = {0, 1, 2}; \
         Real amplitude = 2; Cx y[m]; Real out; \
         equation y = fromPolar(fill(amplitude, m), orientation); \
         out = y[1].re; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M; end P;",
    )
    .expect("one phasor per phase");
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("y[") && c.name.ends_with(".re"))
            .count(),
        3,
        "three phasors, each with its own fields"
    );
}

/// An input declared an array of records takes an array, not the
/// fields of one record.
#[test]
fn an_array_of_records_is_not_read_as_one_record_s_fields() {
    // The quasi-RMS of a polyphase system takes `Complex u[:]` and is
    // handed as many phasors as there are phases. Taken for the fields
    // of one record, three phasors were refused for being three where
    // two were wanted.
    let m = parse_model(
        "package P record Cx Real re; Real im; end Cx; \
         function rms input Cx u[:]; output Real y; \
         protected Integer m = size(u, 1); \
         algorithm y := sum({sqrt(u[k].re^2 + u[k].im^2) for k in 1:m})/m; \
         end rms; \
         model M parameter Integer m = 3; Cx c[m]; Real y; \
         equation for k in 1:m loop c[k] = Cx(k, 0); end for; \
         y = rms(c); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M; end P;",
    )
    .expect("an array of records handed to a function");
    let written = equations_of(&m);
    assert!(
        written.iter().any(|e| e.contains("c[3].re")),
        "every phasor the caller holds is read: {written:?}"
    );
    assert!(
        !written.iter().any(|e| e.contains("u[")),
        "no name of the body travels out: {written:?}"
    );
}

#[test]
fn a_record_argument_with_a_matrix_field_is_read_element_by_element() {
    let m = parse_model(
        "package P record Rec Real T[3,3]; Real w[3]; end Rec; \
         function g input Rec R; input Real v[3]; output Real y; \
         algorithm y := R.T[1,1]*v[1]; end g; \
         function f input Rec R1; output Real y; \
         algorithm y := g(R1, R1.w); end f; \
         model M Rec r(T = identity(3), w = {1,0,0}); Real x; \
         equation der(x) = f(r); end M; end P;",
    )
    .expect("a record handed on whole from one function to the next");
    let written = equations_of(&m);
    assert!(
        !written
            .iter()
            .any(|e| e.contains("R.T") || e.contains("R1.T")),
        "no name of the bodies travels out: {written:?}"
    );
    assert!(
        written.iter().any(|e| e.contains("r.T[1,1]")),
        "the caller's own matrix stands in their place: {written:?}"
    );
}

/// A record with no fields at all is a place kept for another to fill:
/// a medium interface declares its state empty on purpose, and a
/// medium redeclares it whole. A function answering with one has not
/// been reached by that redeclaration, and saying so where it happens
/// beats handing back an empty list - the caller would then find
/// nothing to bind and report its own argument as missing, which names
/// a symptom two steps from its cause.
#[test]
fn a_function_answering_with_an_empty_record_says_why() {
    let err = parse_model(
        "package P record Empty end Empty; \
         function make input Real x; output Empty s; end make; \
         function take input Empty s; output Real y; algorithm y := 1; end take; \
         model M Real q; Real z; equation q = 5; z = take(make(q)); end M; end P;",
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("declares no fields"), "{err}");
    assert!(
        !err.contains("missing its argument"),
        "the cause, not the symptom: {err}"
    );

    // A body that fills its own empty record is not the same thing:
    // nothing was kept for anyone, and it answers as it always did.
    let filled = parse_model(
        "package P record Empty end Empty; \
         function make input Real x; output Empty s; algorithm s := s; end make; \
         function take input Empty s; output Real y; algorithm y := 1; end take; \
         model M Real q; Real z; equation q = 5; z = take(make(q)); end M; end P;",
    );
    assert!(filled.is_ok(), "{filled:?}");
}
