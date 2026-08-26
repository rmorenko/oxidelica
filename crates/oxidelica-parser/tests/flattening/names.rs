//! Where a name means what: packages, imports, `inner`/`outer`, and the conditions read before anything is built.

use super::shared::*;
use oxidelica_parser::{parse_model, Expr};

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
fn a_constant_of_nature_makes_no_claim_about_its_unit() {
    // The magnetic constant reaches the equations as the number
    // 1.2566e-6, having left its henry per metre behind in the library
    // it was declared in. Held to be dimensionless it would make
    // `H = B/(mu_0*mu_r)` a contradiction, which is what kept the flux
    // tubes from running.
    let m = parse_model(
        "model M Real B(unit = \"T\") = 1.5; Real mu_r(unit = \"1\") = 1000; \
         Real H(unit = \"A/m\"); \
         equation H = B / (0.00000125663706212 * mu_r); \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    );
    assert!(m.is_ok(), "{m:?}");

    // A whole number is a count or a factor and says so: scaling by
    // one keeps the unit it scaled, and a disagreement is still named.
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
fn a_package_brings_a_constant_in_by_name_for_everything_inside_it() {
    // `import Modelica.Constants.mu_0;` is written once at the top of
    // the flux tubes and every shape below writes `mu_0` bare. The
    // import belongs to the package, and what is written inside it is
    // written in view of it.
    let m = parse_model(
        "package P import P.C.k; \
         package C constant Real k = 3; end C; \
         package Q model M Real y; equation y = k * time; end M; end Q; end P; \
         model Top P.Q.M m; \
         annotation(experiment(StopTime = 1, Interval = 1)); end Top;",
    )
    .unwrap();
    let said = format!("{:?}", m.equations);
    assert!(said.contains("Number(3.0)"), "{said}");
}

#[test]
fn a_name_may_reach_an_outer_declared_by_a_component_it_holds() {
    // A composite step of the state graph reads its count of active
    // steps as `innerState.stateGraphRoot.subgraphStatePort
    // .activeSteps`: the `outer` belongs to the little block called
    // `innerState`, and the name is written by the class holding that
    // block. An `outer` owns no variable of its own, so unless the
    // whole lead of the name is answered here the equation is left
    // naming something that was never instantiated.
    let m = parse_model(
        "model Root Real a; end Root; \
         model Inner outer Root stateGraphRoot; end Inner; \
         model Sub Inner innerState; Real y; \
         equation y = innerState.stateGraphRoot.a; end Sub; \
         model M inner Root stateGraphRoot(a = time); Sub sub; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M;",
    )
    .unwrap();
    let about: Vec<String> = m
        .equations
        .iter()
        .filter(|e| format!("{:?}", e.lhs).contains("sub.y"))
        .map(|e| e.rhs.describe())
        .collect();
    assert_eq!(about, vec!["stateGraphRoot.a".to_string()]);
}
