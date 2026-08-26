//! What a class takes from the classes above it: bases, redeclarations and the modifiers that ride on them.

use super::shared::*;
use oxidelica_parser::{parse_model, Expr};

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

/// The reach is refused wherever it is written, and whatever way the
/// two classes came to hold what they hold.
#[test]
fn a_protected_declaration_is_kept_back_through_inheritance_too() {
    // Kept back by a base of the component's class: inheriting a
    // declaration does not publish it.
    let refusal = parse_model(
        "model Base protected Real hidden; equation hidden = time; end Base; \
         model Inner extends Base; Real y; equation y = hidden; end Inner; \
         model M Inner a; Real z; equation z = a.hidden; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(refusal.contains("a.hidden"), "{refusal}");

    // Reached from a class that inherited the component rather than
    // declaring it, and reached from an algorithm rather than an
    // equation.
    let refusal = parse_model(
        "model Inner protected Real hidden; public Real y; equation hidden = time; y = 1; end Inner; \
         model Holder Inner a; end Holder; \
         model M extends Holder; Real z; algorithm z := a.hidden; end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(refusal.contains("a.hidden"), "{refusal}");
}

/// A block settles its own outputs; an interface leaves them to
/// whoever finishes it.
#[test]
fn a_block_settles_the_outputs_it_declares() {
    const INSIDE: &str = "block B input Real u; output Real y; ";

    let refusal = parse_model(&format!(
        "{INSIDE} end B; model M B b; equation b.u = time; b.y = 5; end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(refusal.contains("B.y"), "{refusal}");
    assert!(refusal.contains("output"), "{refusal}");

    // Settled by an equation, by a declaration value, by an algorithm,
    // and by the declaration that inherits it.
    for block in [
        format!("{INSIDE} equation y = 2 * u; end B;"),
        "block B input Real u; output Real y = 2 * u; end B;".to_string(),
        format!("{INSIDE} algorithm y := 2 * u; end B;"),
        format!("partial {INSIDE} end B; block C extends B(y = 2 * u); end C;"),
    ] {
        let named = match block.contains("block C") {
            true => "C",
            false => "B",
        };
        parse_model(&format!(
            "{block} model M {named} b; equation b.u = time; end M;"
        ))
        .unwrap();
    }

    // An interface is exactly the case where the outputs are left for
    // whoever extends it.
    parse_model(&format!(
        "partial {INSIDE} end B; block C extends B; equation y = 3 * u; end C; \
         model M C c; equation c.u = time; end M;"
    ))
    .unwrap();
}

/// A class reached by two paths of a diamond is one class.
#[test]
fn a_base_inherited_twice_is_merged_once() {
    let m = parse_model(
        "partial model Base input Real a = 1; Real b; end Base; \
         partial model Left extends Base; end Left; \
         model Both extends Left; extends Base; equation b = a * time; end Both; \
         model M Both t; Real z; equation z = t.b; end M;",
    )
    .unwrap();
    // One of each, not two: merging the base twice would give the
    // instance two of every variable and two of every equation.
    for name in ["t.a", "t.b"] {
        assert_eq!(
            m.components.iter().filter(|c| c.name == name).count(),
            1,
            "{name} in {:?}",
            m.components.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
    }
    assert_eq!(m.equations.len(), 3);
}

#[test]
fn an_outer_declared_by_a_base_is_bound_like_one_written_here() {
    // `outer World world` is written once in `PartialTwoFrames` and
    // every joint of the multi-body library extends it rather than
    // repeating it. A class asked about its own components alone would
    // say it names no `outer` at all, and every reference through it
    // would point at a variable nothing owns.
    let m = parse_model(
        "model W parameter Real g = 10; end W; \
         partial model Framed outer W world; end Framed; \
         model Joint extends Framed; Real a; equation a = world.g; end Joint; \
         model M inner W world; Joint j; end M;",
    )
    .unwrap();

    // The reference went to the `inner` instance rather than to a
    // variable of the joint's own.
    let equation = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"j.a\")")
        .unwrap();
    assert_eq!(format!("{:?}", equation.rhs), "Ref(\"world.g\")");
    assert!(!m.components.iter().any(|c| c.name == "j.world.g"));
}

#[test]
fn a_package_holds_its_base_constants_to_what_the_extends_said() {
    // A medium of the standard library is written `extends
    // PartialMedium(nC = 2)` and declares no `nC` of its own. Read
    // without what the `extends` said, the count came from the
    // interface, which says none - and every connector sized by it was
    // a run of nothing.
    let m = parse_model(
        "partial package PartialMedium constant Integer nC = 0; end PartialMedium; \
         package Water extends PartialMedium(nC = 2); end Water; \
         model Port replaceable package Medium = PartialMedium; Real seen; \
           equation seen = Medium.nC; end Port; \
         model M Port p(redeclare package Medium = Water); end M;",
    )
    .unwrap();

    let seen = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"p.seen\")")
        .unwrap();
    assert_eq!(format!("{:?}", seen.rhs), "Number(2.0)");
}

#[test]
fn a_package_handed_on_by_its_own_name_is_the_one_it_was_replaced_with() {
    // `Port one(redeclare package Medium = Medium)` is how every fluid
    // component passes its medium to its ports. The name on the right
    // is the one this class has, and here it has already been
    // replaced; looked up among the class's own imports it is still
    // the interface, and the child was handed that.
    let m = parse_model(
        "partial package PartialMedium constant Integer nC = 0; end PartialMedium; \
         package Water extends PartialMedium(nC = 2); end Water; \
         model Port replaceable package Medium = PartialMedium; Real seen; \
           equation seen = Medium.nC; end Port; \
         model Source replaceable package Medium = PartialMedium; \
           Port one(redeclare package Medium = Medium); Real mine; \
           equation mine = Medium.nC; end Source; \
         model M Source src(redeclare package Medium = Water); end M;",
    )
    .unwrap();

    // The class itself saw the medium already; now so does the port it
    // handed the medium on to.
    for name in ["src.mine", "src.one.seen"] {
        let equation = m
            .equations
            .iter()
            .find(|e| format!("{:?}", e.lhs) == format!("Ref({name:?})"))
            .unwrap();
        assert_eq!(format!("{:?}", equation.rhs), "Number(2.0)", "{name}");
    }
}

#[test]
fn an_equation_between_two_empty_arrays_says_nothing_rather_than_refusing() {
    // A medium with no mass fractions gives every port `Xi_outflow[0]`,
    // and the model says `ports[i].Xi_outflow = medium.Xi` about it.
    // One side is an array of nothing and the other a name that never
    // got a shape, so the two are written differently and hold the
    // same nothing. Refusing them for that refused the whole of the
    // Fluid library for saying something empty twice.
    let m = parse_model(
        "partial package PartialMedium constant Integer nXi = 0; \
           model BaseProperties Real Xi[nXi]; Real p; end BaseProperties; \
         end PartialMedium; \
         package Water extends PartialMedium; end Water; \
         connector FluidPort replaceable package Medium = PartialMedium; \
           Real p; Real Xi_outflow[Medium.nXi]; end FluidPort; \
         model Source replaceable package Medium = PartialMedium; \
           parameter Integer nPorts = 1; Medium.BaseProperties medium; \
           FluidPort ports[nPorts](redeclare each package Medium = Medium); \
           equation medium.p = 1; \
           for i in 1:nPorts loop ports[i].p = medium.p; \
             ports[i].Xi_outflow = medium.Xi; end for; end Source; \
         model M Source src(nPorts = 2, redeclare package Medium = Water); end M;",
    )
    .unwrap();

    // The ports are there and carry the pressure; the empty equation
    // left nothing behind.
    assert!(m.components.iter().any(|c| c.name == "src.ports[2].p"));
    assert!(!m.components.iter().any(|c| c.name.contains("Xi_outflow")));
    let text = format!("{:?}", m.equations);
    assert!(!text.contains("Xi_outflow"), "{text}");
}

#[test]
fn a_replaceable_package_a_base_declared_is_in_view_of_what_extends_it() {
    // `replaceable package Medium` is written once in `PartialSource`,
    // and every boundary of the fluid library extends that rather than
    // repeating it - then names `Medium.AbsolutePressure` in its own
    // declarations. Read from the class's own aliases alone the name
    // stands for nothing, and the type it qualifies is unknown.
    let m = parse_model(
        "package Types type AbsolutePressure = Real; end Types; \
         partial package PartialMedium extends Types; constant Real p_default = 1e5; \
         end PartialMedium; \
         package Water extends PartialMedium(p_default = 2e5); end Water; \
         partial model PartialSource replaceable package Medium = PartialMedium; \
           Real q; equation q = 0; end PartialSource; \
         model Boundary extends PartialSource; \
           parameter Medium.AbsolutePressure p = Medium.p_default; \
           Real y; equation y = p; end Boundary; \
         model M Boundary b(redeclare package Medium = Water); end M;",
    )
    .unwrap();

    // The type was found, and the constant read through the package is
    // the replacement's rather than the interface's.
    let held = m.components.iter().find(|c| c.name == "b.p").unwrap();
    assert_eq!(
        format!("{:?}", held.binding.as_ref().unwrap()),
        "Number(200000.0)"
    );

    // Two bases deep, and what the nearer one says wins: a class may
    // name a package of its own beside the one it inherits, and the
    // inherited name is only taken where nothing nearer holds it.
    let deep = parse_model(
        "package Types type AbsolutePressure = Real; end Types;          partial package PartialMedium extends Types; constant Real p_default = 1e5;          end PartialMedium;          package Water extends PartialMedium(p_default = 2e5); end Water;          package Oil extends PartialMedium(p_default = 3e5); end Oil;          partial model Innermost replaceable package Medium = PartialMedium;            Real q; equation q = 0; end Innermost;          partial model Middle extends Innermost; end Middle;          model Boundary extends Middle;            parameter Medium.AbsolutePressure p = Medium.p_default;            Real y; equation y = p; end Boundary;          model Own package Medium = Oil;            parameter Medium.AbsolutePressure p = Medium.p_default; end Own;          model M Boundary b(redeclare package Medium = Water); Own o; end M;",
    )
    .unwrap();
    let through_two = deep.components.iter().find(|c| c.name == "b.p").unwrap();
    assert_eq!(
        format!("{:?}", through_two.binding.as_ref().unwrap()),
        "Number(200000.0)"
    );
    let its_own = deep.components.iter().find(|c| c.name == "o.p").unwrap();
    assert_eq!(
        format!("{:?}", its_own.binding.as_ref().unwrap()),
        "Number(300000.0)"
    );
}

/// A `redeclare function extends density` writes a body and nothing
/// else: what it takes and answers with belongs to the function it
/// extends. Reading only its own declarations found none, so a
/// function taking a record was taken for one taking nothing, and the
/// call was spread over the record's fields as if they were elements -
/// a density of one number came back shaped like the state of two.
#[test]
fn a_redeclared_function_takes_what_its_base_declared() {
    let m = parse_model(
        "package P record State Real p; Real T; end State; \
         partial package Base replaceable partial function density \
         input State state; output Real d; end density; end Base; \
         package Simple extends Base; redeclare function extends density \
         algorithm d := 995; end density; end Simple; \
         model M State st; Real y; equation st.p = 100; st.T = 300; \
         y = Simple.density(st); end M; end P;",
    )
    .unwrap();
    let text = format!("{:?}", m.equations);
    assert!(text.contains("Ref(\"y\"), rhs: Number(995.0)"), "{text}");
}

/// A parameter with only a `start` is worth what its `start` says.
///
/// The machine library switches a thermal port on with `parameter
/// Boolean useDamperCage(start=true)` and no binding at all, and
/// hands that name down to a connector's condition through a
/// modifier on an `extends`. Read as having no value, the condition
/// could not be settled and sixteen models were refused.
#[test]
fn a_parameter_with_only_a_start_settles_a_condition() {
    let m = parse_model(
        "connector TP parameter Boolean useCage(start = true); Real p if useCage; Real q; end TP; \
         partial model Base replaceable TP tp; Real x; equation x = time; tp.q = x; end Base; \
         model Amb parameter Boolean useCage(start = true); \
         extends Base(tp(final useCage = useCage)); end Amb; \
         model M Amb a; Real y; equation y = a.x; end M;",
    )
    .unwrap();
    // The condition holds, so the optional member is there.
    let names: Vec<&str> = m.components.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"a.tp.p"), "{names:?}");
}
