//! Connectors and connections: what a `connect` means, and what a graph of them comes to.

use super::shared::*;
use oxidelica_parser::{parse_model, Expr};

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
    // And where it cannot be settled, the flattening says so and
    // carries the parameter on. 18.3's word is "proposes" - the
    // annotation sits beside `Inline` in the code-generation chapter,
    // and the one consequence it names, that the value cannot be
    // changed after translation, follows from accepting the proposal
    // rather than from receiving it. An offer declined is no broken
    // law, which is what tells this annotation from the two above:
    // 18.8 calls those an error in as many words, and 18.3 calls this
    // one a proposal.
    //
    // The model still refuses further along if nothing can give the
    // parameter a value at all - the run says `parameter q has no
    // value` in its own words - but that is the run's verdict on a
    // model with a hole in it, not the flattener's on an annotation.
    assert!(parse_model(
        "model M parameter Real q = 2; \
         parameter Real p = q annotation(Evaluate = true); Real y; \
         equation y = p; end M;"
    )
    .is_ok());
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
fn cardinality_is_answered_inside_a_branch_the_run_decides() {
    // A branch nothing could settle here travels to the compiler as it
    // stands, apart from the equations, and the loop that answered the
    // connection questions only ever saw copies of it. The state graph
    // asks how many connections name a port inside such a branch, and
    // nothing later in the pipeline knows the count, so the question
    // reached the run and was refused there as an unknown function.
    let m = parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         model Part Pin p[2]; Real y; equation \
         if time > 1 then y = cardinality(p[1]); else y = 0; end if; end Part; \
         model M Part a; Pin q; equation connect(a.p[1], q); q.v = time; \
         a.p[2].v = 0; end M;",
    )
    .unwrap();
    let said = format!("{:?}", m.conditional);
    assert!(!said.contains("cardinality"), "{said}");
    assert!(said.contains("Number(1.0)"), "{said}");
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

/// An `input` is what a class asks to be given, and something has to
/// give it.
#[test]
fn an_input_nothing_settles_is_refused() {
    const HELD: &str = "model Held input Real u; Real y; equation y = 2 * u; end Held; ";

    let refusal = parse_model(&format!(
        "{HELD}model M Held h; Real z; equation z = h.y; end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(refusal.contains("h.u"), "{refusal}");
    assert!(refusal.contains("`input`"), "{refusal}");

    // Settled by a value on the declaration, by a modifier, by an
    // equation of the class holding it, and by a connection.
    for held in [
        "model Held input Real u = time; Real y; equation y = 2 * u; end Held; \
         model M Held h; Real z; equation z = h.y; end M;",
        "model Held input Real u; Real y; equation y = 2 * u; end Held; \
         model M Held h(u = time); Real z; equation z = h.y; end M;",
        "model Held input Real u; Real y; equation y = 2 * u; end Held; \
         model M Held h; Real z; equation h.u = time; z = h.y; end M;",
        "connector In input Real v; end In; \
         model Held In p; Real y; equation y = 2 * p.v; end Held; \
         model M Held h; In q; Real z; equation q.v = time; connect(h.p, q); z = h.y; end M;",
    ] {
        parse_model(held).unwrap();
    }
}

/// The same model flattens to the same thing every time it is read.
#[test]
fn a_model_built_of_connections_flattens_the_same_way_twice() {
    // The connection sets used to be emitted in the order a hash map
    // handed them back, which Rust seeds afresh in every process. What
    // came of it was not a different message but a different model:
    // the equations in another order, so the index reduction found
    // another one left over, so another state was demoted, so the
    // count of pinned starts moved and the initialisation was square
    // or was not.
    const BRIDGE: &str = "connector Pin Real v; flow Real i; end Pin; \
         model R Pin p; Pin n; parameter Real R = 1; \
         equation p.i + n.i = 0; p.v - n.v = R * p.i; end R; \
         model Src Pin p; Pin n; \
         equation p.i + n.i = 0; p.v - n.v = 5; end Src; \
         model G Pin p; equation p.v = 0; end G; \
         model M R ra; R rb(R = 2); R rc(R = 3); R rd(R = 4); Src s; G g; \
         equation connect(s.p, ra.p); connect(s.p, rb.p); \
         connect(ra.n, rc.p); connect(rb.n, rd.p); \
         connect(rc.n, s.n); connect(rd.n, s.n); connect(s.n, g.p); end M;";

    let once = parse_model(BRIDGE).unwrap();
    let written = |model: &oxidelica_parser::Model| {
        let names: Vec<&str> = model.components.iter().map(|c| c.name.as_str()).collect();
        let equations: Vec<String> = model
            .equations
            .iter()
            .map(|equation| format!("{:?} = {:?}", equation.lhs, equation.rhs))
            .collect();
        (names.join(","), equations.join(";"))
    };
    // Reading it again in the same process would use the same seed, so
    // what is checked here is that the order does not come from the
    // map at all: the components and the equations come out sorted by
    // the names the model wrote, which is a property of the answer
    // rather than of the run.
    for _ in 0..4 {
        assert_eq!(written(&once), written(&parse_model(BRIDGE).unwrap()));
    }
}

/// A thermal port holds a heat port per winding, and the ports a
/// machine joins are the ones a `redeclare` widened: the members the
/// connection carries are the ones the instances turned out to have,
/// not the ones the connector class writes down for itself.
#[test]
fn two_connectors_of_connectors_are_joined_by_their_members() {
    let m = parse_model(
        "model M connector Heat Real T; flow Real Q_flow; end Heat; \
         connector Base Heat hs; end Base; \
         connector More extends Base; Heat hr; end More; \
         partial model PA replaceable Base thermalPort; end PA; \
         model Amb extends PA(redeclare final More thermalPort); end Amb; \
         partial model Mach replaceable Base internalPort; replaceable PA ambient; \
         equation connect(ambient.thermalPort, internalPort); end Mach; \
         model Machine extends Mach(redeclare final Amb ambient, \
           redeclare final More internalPort); end Machine; \
         Machine c; end M;",
    )
    .expect("a connection between two ports of ports is joined by the ports inside");
    let equations = equations_of(&m);
    assert!(
        equations.contains(&"c.internalPort.hr.T = c.ambient.thermalPort.hr.T".to_string()),
        "the heat port a redeclare added is carried across too: {equations:?}"
    );
    assert!(
        equations.contains(&"c.internalPort.hs.T = c.ambient.thermalPort.hs.T".to_string()),
        "and so is the one the base connector declares: {equations:?}"
    );
}

#[test]
fn a_connector_holds_what_its_base_class_holds() {
    let m = parse_model(
        "package P \
         connector Base Real e; flow Real f; end Base; \
         connector Pin extends Base; end Pin; \
         model M Pin p; Real x; equation p.e = 1; der(x) = p.f; end M; end P;",
    )
    .expect("a connector that says what it holds through a base class");
    let written = equations_of(&m);
    assert!(
        written.iter().any(|e| e.contains("p.f") && e.contains("0")),
        "the inherited flow is forced to zero where nothing is connected: {written:?}"
    );
}

#[test]
fn a_parameter_of_a_connector_is_not_solved_for() {
    let m = parse_model(
        "package P \
         connector Port parameter Real medium = 1; Real p; flow Real m_flow; end Port; \
         model Source Port a; equation a.p = 2; end Source; \
         model Sink Port a; equation a.m_flow = 0; end Sink; \
         model M Source s; Sink k; Real x; \
         equation connect(s.a, k.a); der(x) = k.a.p; end M; end P;",
    )
    .expect("two ports joined, each carrying the medium it is filled with");
    let written = equations_of(&m);
    assert!(
        !written
            .iter()
            .any(|e| e.contains("medium") && e.contains("=")),
        "the medium is settled before the run and no equation solves for it: {written:?}"
    );
}

#[test]
fn a_connector_member_that_is_a_record_joins_field_by_field() {
    let m = parse_model(
        "package P record Cx Real re; Real im; end Cx; \
         connector Port Cx V; flow Cx Phi; end Port; \
         model Gnd Port p; equation p.V.re = 0; p.V.im = 0; end Gnd; \
         model Src Port p; equation p.Phi.re = 1; p.Phi.im = 0; end Src; \
         model M Gnd g; Src s; Real x; \
         equation connect(g.p, s.p); der(x) = s.p.V.re; end M; end P;",
    )
    .expect("two ports whose potential and flow are complex");
    let written = equations_of(&m);
    assert!(
        written
            .iter()
            .any(|e| e.contains("s.p.V.re") && e.contains("g.p.V.re")),
        "the fields are equated one by one: {written:?}"
    );
    assert!(
        written
            .iter()
            .any(|e| e.contains("g.p.Phi.im") && e.contains("s.p.Phi.im")),
        "and each field of the flow is summed on its own: {written:?}"
    );
}

/// Three ways the standard library writes something this parser could
/// not read, each of which kept a whole file out.
#[test]
fn the_library_writes_these_and_they_are_read_now() {
    // A check among the actions of a `when`: made when the event
    // fires, which is what the Fluid steady-state tests say.
    let m = parse_model(
        "model M Real x; equation x = time; \
         when time > 1 then assert(x < 5, \"too big\"); end when; end M;",
    )
    .unwrap();
    assert_eq!(m.when_clauses.len(), 1);

    // A port reached through a member and a subscript after it, which
    // is how the polyphase library splits a plug into subsystems.
    let split = parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         connector Plug Pin pin[2]; end Plug; \
         model M Plug p; Plug q[2]; equation \
         for k in 1:2 loop connect(p.pin[k], q[k].pin[1]); end for; \
         p.pin[1].v = 1; p.pin[2].v = 2; \
         q[1].pin[2].v = 0; q[2].pin[2].v = 0; end M;",
    )
    .unwrap();
    // The connection reached both pins: each joined pair shares a
    // voltage and sums its currents.
    let text = format!("{:?}", split.equations);
    assert!(text.contains("q[1].pin[1].v"), "{text}");
    assert!(text.contains("q[2].pin[1].v"), "{text}");

    // An `inverse` whose arguments are named, the way the moist air
    // tables write theirs.
    let named = parse_model(
        "function f input Real p; input Real T; output Real h; \
         algorithm h := p * T; annotation (inverse(T = g(p = p, h = h))); end f; \
         function g input Real p; input Real h; output Real T; \
         algorithm T := h / p; end g; \
         model M Real y; equation y = f(2, 3); end M;",
    );
    assert!(named.is_ok(), "{named:?}");
}

/// A port may be reached through as many members and subscripts as the
/// model wrote, in an equation and in a `connect` alike, and a check
/// among the actions of a `when` is a truth the checker reads like any
/// other.
#[test]
fn a_port_is_reached_through_as_many_steps_as_are_written() {
    // Member, subscript, member, subscript, member: the whole chain.
    let deep = parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         connector Plug Pin pin[2]; end Plug; \
         model Box Plug plug[2]; end Box; \
         model M Box b[2]; equation \
         b[1].plug[1].pin[1].v = 1; b[1].plug[1].pin[2].v = 2; \
         b[1].plug[2].pin[1].v = 0; b[1].plug[2].pin[2].v = 0; \
         b[2].plug[1].pin[1].v = 0; b[2].plug[1].pin[2].v = 0; \
         b[2].plug[2].pin[1].v = 0; b[2].plug[2].pin[2].v = 0; \
         b[1].plug[1].pin[1].i = 0; b[1].plug[1].pin[2].i = 0; \
         b[1].plug[2].pin[1].i = 0; b[1].plug[2].pin[2].i = 0; \
         b[2].plug[1].pin[1].i = 0; b[2].plug[1].pin[2].i = 0; \
         b[2].plug[2].pin[1].i = 0; b[2].plug[2].pin[2].i = 0; end M;",
    )
    .unwrap();
    let text = format!("{:?}", deep.equations);
    assert!(text.contains("b[1].plug[1].pin[1].v"), "{text}");

    // The same chain inside a `connect`, where the parser reads the
    // ports rather than an expression.
    let joined = parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         connector Plug Pin pin[2]; end Plug; \
         model Box Plug plug[2]; end Box; \
         model M Box b[2]; equation \
         connect(b[1].plug[1].pin[1], b[2].plug[2].pin[2]); end M;",
    );
    assert!(joined.is_ok(), "{joined:?}");

    // A check among the actions of a `when` whose condition compares
    // two numbers: the checker reads it like any other truth.
    let checked = parse_model(
        "model M Real x; Integer n; equation x = time; n = 2; \
         when time > 1 then assert(x < n, \"held\"); end when; end M;",
    );
    assert!(checked.is_ok(), "{checked:?}");
}

/// What the parser says when the things it learned to read this week
/// are written wrongly.
#[test]
fn the_newly_read_forms_say_what_is_wrong_with_them() {
    let err = |source: &str| parse_model(source).unwrap_err().to_string();

    // A subscript inside a `connect` that never closes.
    assert!(err("connector Pin Real v; flow Real i; end Pin; \
         connector Plug Pin pin[2]; end Plug; \
         model M Plug p; Plug q; equation connect(p.pin[1, q.pin[1]); end M;")
    .contains("subscript"));

    // An `inverse` whose named argument has no value after the `=`.
    assert!(
        err("function f input Real p; output Real h; algorithm h := p; \
         annotation (inverse(p = g(h = ))); end f; \
         model M Real y; equation y = f(1); end M;")
        .contains("argument")
    );

    // An `assert` among the actions of a `when` with no semicolon.
    assert!(err("model M Real x; equation x = time; \
         when time > 1 then assert(x < 5, \"m\") end when; end M;")
    .contains("semicolon"));
}

/// A port is reached through as many members and subscripts as are
/// written, in an equation and in a `connect` alike, and a subscript
/// that never closes says so wherever it sits in the chain.
#[test]
fn a_chain_of_members_and_subscripts_goes_as_deep_as_it_is_written() {
    // Four steps: rack, box, plug, pin, each subscripted.
    let deep = parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         connector Plug Pin pin[2]; end Plug; \
         model Box Plug plug[2]; end Box; \
         model Rack Box box[2]; end Rack; \
         model M Rack r[2]; Real z; \
         equation z = r[1].box[1].plug[1].pin[1].v; end M;",
    );
    let text = format!("{deep:?}");
    assert!(
        text.contains("r[1].box[1].plug[1].pin[1].v"),
        "the whole chain: {text}"
    );

    // The same chain inside a `connect`.
    let joined = parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         connector Plug Pin pin[2]; end Plug; \
         model Box Plug plug[2]; end Box; \
         model Rack Box box[2]; end Rack; \
         model M Rack r[2]; equation \
         connect(r[1].box[1].plug[1].pin[1], r[2].box[2].plug[2].pin[2]); end M;",
    );
    assert!(
        !format!("{joined:?}").contains("closing parenthesis of connect"),
        "{joined:?}"
    );

    // A subscript deep in the chain that never closes.
    let err = parse_model(
        "connector Pin Real v; flow Real i; end Pin; \
         connector Plug Pin pin[2]; end Plug; \
         model Box Plug plug[2]; end Box; \
         model M Box b[2]; equation \
         connect(b[1].plug[1].pin[1, b[2].plug[1].pin[1]); end M;",
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("subscript"), "{err}");
}

/// A connect to a component that was switched off is nothing.
#[test]
fn a_connect_to_a_disabled_component_is_no_connection() {
    // The machines of the library write `connect(ir, damperCage.i)`
    // beside a `damperCage ... if useDamperCage`, and with the cage
    // switched off the right side is one bare name of no shape while
    // the left is a run of two. The two were counted against each
    // other and the model refused - for a connection nobody asked
    // for, since one end of it does not exist.
    let m = parse_model(
        "package P \
           connector Out = output Real; \
           model Cage Out i[2] = {1.0, 2.0}; end Cage; \
           model Machine \
             parameter Boolean useCage = false; \
             Out ir[2]; \
             Cage damperCage if useCage; \
             Real y; \
           equation \
             connect(ir, damperCage.i); \
             ir = {time, 2*time}; \
             y = ir[1]; \
           end Machine; \
           model E Machine m; end E; \
         end P;",
    )
    .expect("a connect to a component that is not there");
    // The model keeps the equations it does have.
    assert!(
        m.equations
            .iter()
            .any(|e| matches!(&e.lhs, Expr::Ref(name) if name == "m.y")),
        "the model lost what it was left with"
    );
}
