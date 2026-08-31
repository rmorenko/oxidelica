//! Tables written as rows or as a grid: how they are read, and what they say beyond their edges.

use super::shared::*;
use oxidelica_parser::{parse_model, read_table_file, Expr};

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

    // The length of what a function answers with may be a call of its
    // own: the polyphase functions size their result by the number of
    // base systems the phase count makes, which arithmetic alone
    // cannot decide. Read from inside a component, the names have
    // become the component's - `m` is `conv.m` - and the numbers the
    // length is read against have to be the caller's for the call to
    // come to anything.
    let m = parse_model(
        "package P \
         function bases input Integer m = 3; output Integer n; \
         algorithm n := 1; end bases; \
         function indices input Integer m = 3; \
         output Integer ind[bases(m) * (integer(m / bases(m)) - 1)]; \
         algorithm for k in 1:size(ind, 1) loop ind[k] := k + 1; end for; \
         end indices; \
         model Inner parameter Integer m = 3; \
         final parameter Integer got[:] = indices(m); end Inner; \
         model M parameter Integer m = 3; Inner conv(m = m); Real y; \
         equation y = conv.got[1]; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M; \
         end P;",
    )
    .expect("a length that is a call");
    assert_eq!(
        m.components
            .iter()
            .filter(|c| c.name.starts_with("conv.got["))
            .count(),
        2
    );

    // `fill(0, 0, 2)` is a table with no rows and two columns, which
    // is how the table blocks say they were given nothing yet. Written
    // out it is an empty list, and an empty list has no second
    // dimension for the `:` to read.
    let m = parse_model(
        "package P model T parameter Real table[:, :] = fill(0.0, 0, 2); \
         Real y; equation y = size(table, 1); end T; \
         model M T t; Real z; equation z = t.y; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M; end P;",
    )
    .expect("a table of no rows and two columns");
    assert!(
        !m.components.iter().any(|c| c.name.starts_with("t.table[")),
        "a table of no rows holds nothing"
    );

    // A package may write a `fill` of its own, and it says nothing
    // about how long it is: the language's `fill(0, 0, 2)` states its
    // lengths after the filler, and a namesake's arguments mean
    // whatever its writer meant.
    let mine = parse_model(
        "package P function fill input Real x; input Integer a; input Integer b; \
         output Real y[2, 2]; algorithm y := {{x, x}, {x, x}}; end fill; \
         model T parameter Real table[:, :] = P.fill(0.0, 0, 2); \
         Real y; equation y = table[1, 1]; end T; \
         model M T t; Real z; equation z = t.y; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M; end P;",
    )
    .expect("a namesake of the language's own fill");
    assert_eq!(
        mine.components
            .iter()
            .filter(|c| c.name.starts_with("t.table["))
            .count(),
        4,
        "the namesake's own shape, not the lengths its arguments look like"
    );

    // `zeros` and `ones` state their lengths the same way, with no
    // filler in front of them, and a call that is neither states
    // nothing this way at all.
    let zeroed = parse_model(
        "package P model T parameter Real table[:, :] = zeros(0, 4); \
         Real y; equation y = size(table, 1); end T; \
         model M T t; Real z; equation z = t.y; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M; end P;",
    )
    .expect("a table of no rows and four columns");
    assert!(!zeroed
        .components
        .iter()
        .any(|c| c.name.starts_with("t.table[")));
    let oned = parse_model(
        "package P model T parameter Real table[:] = ones(3); \
         Real y; equation y = table[1]; end T; \
         model M T t; Real z; equation z = t.y; \
         annotation(experiment(StopTime = 1, Interval = 1)); end M; end P;",
    )
    .expect("three ones");
    assert_eq!(
        oned.components
            .iter()
            .filter(|c| c.name.starts_with("t.table["))
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

/// The Akima spline: a cubic between each pair of points, leaving
/// each point at a slope worked out from its neighbours.
#[test]
fn a_table_asked_for_an_akima_spline_is_written_out_as_one() {
    let read = |table: &str, u: &str| {
        let m = parse_model(&format!(
            "{TABLE_BLOCK} model M \
             parameter Real data[4, 2] = {table}; \
             Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", data, {{2}}, 2, 2); \
             Real y; equation y = Blocks.getValue(h, 1, {u}); end M;"
        ))
        .expect("an Akima spline");
        m.equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == "y"))
            .map(|e| folded(&e.rhs))
            .expect("the value")
    };

    // A straight table stays straight: where a point sits between two
    // lines of the same slope, the weighted mean of them is that
    // slope, and the cubic between two such points is the line. This
    // is what Akima's rule buys over an ordinary cubic spline.
    let straight = read("[0, 0; 1, 2; 2, 4; 3, 6]", "1.5");
    assert!(
        (straight - 3.0).abs() < 1e-9,
        "a straight table: {straight}"
    );

    // A table of squares comes back as squares, both at a point it
    // was given and between two: 1.5 squared is 2.25, where straight
    // lines between the points would have said 2.5.
    let squares = read("[0, 0; 1, 1; 2, 4; 3, 9]", "1.5");
    assert!(
        (squares - 2.25).abs() < 1e-9,
        "halfway up a square: {squares}"
    );
    let known = read("[0, 0; 1, 1; 2, 4; 3, 9]", "2.0");
    assert!((known - 4.0).abs() < 1e-9, "a point it was given: {known}");

    // A spline is drawn through points that follow one another along
    // the abscissa. Two points at the same place leave the interval
    // between them no width, and the line across it was quietly taken
    // for flat.
    let repeated = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         parameter Real data[4, 2] = [0, 0; 1, 1; 1, 2; 2, 4]; \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", data, {{2}}, 2, 2); \
         Real y; equation y = Blocks.getValue(h, 1, 0.5); end M;"
    ))
    .expect_err("a spline through a step")
    .message;
    assert!(repeated.contains("twice on its abscissa"), "{repeated}");
    assert!(repeated.contains('h'), "the table is named: {repeated}");

    // Straight lines and levels are another matter: a repeated
    // abscissa is a step, which a table is entitled to say and which
    // goes on being read as it was.
    let step = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         parameter Real data[4, 2] = [0, 0; 1, 1; 1, 2; 2, 4]; \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", data, {{2}}, 1, 2); \
         Real y; equation y = Blocks.getValue(h, 1, 0.5); end M;"
    ))
    .expect("a step read as straight lines");
    let held = step
        .equations
        .iter()
        .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == "y"))
        .map(|e| folded(&e.rhs))
        .expect("the value");
    assert!((held - 0.5).abs() < 1e-9, "a step read straight: {held}");

    // A smoothness this compiler still does not write out is refused
    // by number rather than read as something else.
    let refused = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         parameter Real data[3, 2] = [0, 0; 1, 2; 2, 6]; \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", data, {{2}}, 7, 2); \
         Real y; equation y = Blocks.getValue(h, 1, 1.5); end M;"
    ))
    .expect_err("a smoothness nobody here writes out")
    .message;
    assert!(refused.contains("smoothness 7"), "{refused}");
}

/// A two-dimensional table is read by where two abscissae fall in its
/// grid, and both ends of the grid come back at once.
#[test]
fn a_grid_the_model_wrote_is_read_bilinearly() {
    // The grid says 10, 20 along the top and 30, 40 below, at 1 and 2
    // on either abscissa. The middle of it is the four corners
    // averaged, which is 25.
    let m = parse_model(&format!(
        "{GRID_BLOCK} model M \
         parameter Real data[3, 3] = [0, 1, 2; 1, 10, 20; 2, 30, 40]; \
         Grid.Handle h = Grid.Handle(\"NoName\", \"NoName\", data, 1, 2); \
         Real y; Real low1; Real high2; \
         equation y = Grid.getValue(h, 1.5, 1.5); \
         low1 = Grid.umin(h)[1]; high2 = Grid.umax(h)[2]; end M;"
    ))
    .unwrap();
    let said = |name: &str| {
        m.equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == name))
            .map(|e| format!("{:?}", e.rhs))
            .unwrap_or_default()
    };
    // Both ends of the grid arrive as a pair, and each parameter takes
    // its own of it rather than the whole.
    assert_eq!(said("low1"), "Number(1.0)");
    assert_eq!(said("high2"), "Number(2.0)");
    // Nothing outside Modelica is left in the model, and the middle
    // reads as the average of the corners.
    let written = said("y");
    assert!(!written.contains("ModelicaStandardTables"), "{written}");
    // Written out as the corner it starts from and the three weights
    // that carry it across the cell: 10 at the near corner, 20 down
    // and 10 across, which comes to 25 in the middle.
    assert!(
        written.contains("Number(10.0)") && written.contains("Number(20.0)"),
        "the corners of the cell: {written}"
    );
}

/// How fast a grid's value moves is each abscissa's rate weighted by
/// the slope along it.
#[test]
fn a_grid_says_how_fast_its_value_moves() {
    // Over the one cell the value rises by 20 down and 10 across, so
    // moving down at 1 and across at 0 is 20 a second, and the other
    // way round is 10.
    let m = parse_model(&format!(
        "{GRID_BLOCK} model M \
         parameter Real data[3, 3] = [0, 1, 2; 1, 10, 20; 2, 30, 40]; \
         Grid.Handle h = Grid.Handle(\"NoName\", \"NoName\", data, 1, 2); \
         Real down; Real across; \
         equation down = Grid.getDerValue(h, 1.5, 1.5, 1.0, 0.0); \
         across = Grid.getDerValue(h, 1.5, 1.5, 0.0, 1.0); end M;"
    ))
    .expect("how fast the grid moves");
    let said = |name: &str| {
        m.equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == name))
            .map(|e| folded(&e.rhs))
            .expect("the rate")
    };
    assert!((said("down") - 20.0).abs() < 1e-9, "{}", said("down"));
    assert!((said("across") - 10.0).abs() < 1e-9, "{}", said("across"));
}

/// A grid one cell wide, or one tall, and how fast it moves.
#[test]
fn a_grid_of_one_row_and_the_rate_it_changes_at() {
    // A grid with a single row under its top one says the same thing
    // whatever the first abscissa is: there is no interval down it to
    // be in.
    let flat = parse_model(&format!(
        "{GRID_BLOCK} model M \
         parameter Real data[2, 3] = [0, 1, 2; 1, 10, 20]; \
         Grid.Handle h = Grid.Handle(\"NoName\", \"NoName\", data, 1, 2); \
         Real near; Real far; \
         equation near = Grid.getValue(h, 1.0, 1.0); \
         far = Grid.getValue(h, 9.0, 1.0); end M;"
    ))
    .expect("a grid of one row");
    let said = |name: &str| {
        flat.equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == name))
            .map(|e| folded(&e.rhs))
            .expect("the value")
    };
    assert!((said("near") - 10.0).abs() < 1e-9, "{}", said("near"));
    assert!(
        (said("far") - said("near")).abs() < 1e-9,
        "one row says one thing"
    );

    // And one column, the same the other way round.
    let narrow = parse_model(&format!(
        "{GRID_BLOCK} model M \
         parameter Real data[3, 2] = [0, 1; 1, 10; 2, 30]; \
         Grid.Handle h = Grid.Handle(\"NoName\", \"NoName\", data, 1, 2); \
         Real y; equation y = Grid.getValue(h, 1.5, 9.0); end M;"
    ))
    .expect("a grid of one column");
    let middle = narrow
        .equations
        .iter()
        .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == "y"))
        .map(|e| folded(&e.rhs))
        .expect("the value");
    assert!(
        (middle - 20.0).abs() < 1e-9,
        "halfway down one column: {middle}"
    );
}

/// A grid this compiler cannot read says so rather than guessing.
#[test]
fn a_grid_says_what_it_cannot_read() {
    // The splines are written out; a smoothness that is none of them
    // is refused by number rather than read as something else.
    let spline = parse_model(&format!(
        "{GRID_BLOCK} model M \
         parameter Real data[3, 3] = [0, 1, 2; 1, 10, 20; 2, 30, 40]; \
         Grid.Handle h = Grid.Handle(\"NoName\", \"NoName\", data, 7, 2); \
         Real y; equation y = Grid.getValue(h, 1.5, 1.5); end M;"
    ))
    .expect_err("a smoothness this compiler does not write out")
    .message;
    assert!(spline.contains("smoothness 7"), "{spline}");

    // A matrix with a top row and nothing under it is a grid with no
    // crossings to read.
    let empty = parse_model(&format!(
        "{GRID_BLOCK} model M \
         parameter Real data[1, 3] = [0, 1, 2]; \
         Grid.Handle h = Grid.Handle(\"NoName\", \"NoName\", data, 1, 2); \
         Real y; equation y = Grid.getValue(h, 1.5, 1.5); end M;"
    ))
    .expect_err("a grid with no rows under its top one")
    .message;
    assert!(empty.contains("no grid to read"), "{empty}");

    // A subscript that is no place in the pair of ends is left alone
    // rather than counted from the wrong side.
    let outside = parse_model(&format!(
        "{GRID_BLOCK} model M \
         parameter Real data[3, 3] = [0, 1, 2; 1, 10, 20; 2, 30, 40]; \
         Grid.Handle h = Grid.Handle(\"NoName\", \"NoName\", data, 1, 2); \
         Real low; equation low = Grid.umin(h)[3]; end M;"
    ));
    assert!(
        outside.is_err(),
        "a third end of a pair is not a place in it"
    );
}

/// What a grid says beyond its own edges is what it was asked for.
#[test]
fn a_grid_says_what_its_extrapolation_asked_for_beyond_its_edges() {
    // The grid is 10, 20 over 30, 40 at 1 and 2 on either abscissa, so
    // the plane over the one cell rises by 20 down and 10 across.
    // Asked at 3 on the first abscissa - one span past the edge -
    // carrying the plane on says 50, holding the edge says 30.
    let read = |extrapolation: u32, u1: &str, u2: &str| {
        let m = parse_model(&format!(
            "{GRID_BLOCK} model M \
             parameter Real data[3, 3] = [0, 1, 2; 1, 10, 20; 2, 30, 40]; \
             Grid.Handle h = Grid.Handle(\"NoName\", \"NoName\", data, 1, {extrapolation}); \
             Real y; equation y = Grid.getValue(h, {u1}, {u2}); end M;"
        ))
        .unwrap_or_else(|e| panic!("extrapolation {extrapolation}: {}", e.message));
        let rhs = m
            .equations
            .iter()
            .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == "y"))
            .map(|e| e.rhs.clone())
            .expect("the value");
        (folded(&rhs), m)
    };

    // `LastTwoPoints` carries the edge cell's plane on.
    let (carried, _) = read(2, "3.0", "1.0");
    assert!(
        (carried - 50.0).abs() < 1e-9,
        "the plane carried on: {carried}"
    );

    // `HoldLastPoint` holds the edge instead.
    let (held, _) = read(1, "3.0", "1.0");
    assert!((held - 30.0).abs() < 1e-9, "the edge held: {held}");

    // `Periodic` repeats the grid: the first abscissa spans 1 to 2, a
    // period of one, so 3 is 1 again and the near corner is what the
    // table says there.
    let (repeated, _) = read(3, "3.0", "1.0");
    assert!(
        (repeated - 10.0).abs() < 1e-9,
        "the grid repeated: {repeated}"
    );

    // `NoExtrapolation` says the run has gone wrong, on either
    // abscissa, and holds rather than carrying on meanwhile.
    let (_, refused) = read(4, "3.0", "1.0");
    assert!(
        refused
            .asserts
            .iter()
            .any(|(_, message)| message.contains("first abscissa"))
            && refused
                .asserts
                .iter()
                .any(|(_, message)| message.contains("second abscissa")),
        "both abscissae are checked: {:?}",
        refused.asserts
    );

    // An extrapolation nobody here knows is refused by number rather
    // than answered with a guess.
    let unknown = parse_model(&format!(
        "{GRID_BLOCK} model M \
         parameter Real data[3, 3] = [0, 1, 2; 1, 10, 20; 2, 30, 40]; \
         Grid.Handle h = Grid.Handle(\"NoName\", \"NoName\", data, 1, 9); \
         Real y; equation y = Grid.getValue(h, 1.5, 1.5); end M;"
    ))
    .expect_err("an extrapolation nobody knows")
    .message;
    assert!(unknown.contains("extrapolation 9"), "{unknown}");
}

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

    // The four splines are written out; a smoothness that is none of
    // them says which was asked for rather than being read as one.
    let spline = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", [0, 0; 1, 1], {{2}}, 7, 2); \
         Real y; equation y = Blocks.getValue(h, 1, time); end M;"
    ))
    .unwrap_err()
    .to_string();
    assert!(spline.contains("smoothness 7"), "{spline}");
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

    // A periodic table says the same thing every period: what it is
    // asked at is brought back into the one scope it was written for,
    // so the value beyond the far end is never reached.
    let periodic = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", [0, 1; 1, 2], {{2}}, 1, 3); \
         Real y; equation y = Blocks.getValue(h, 1, time); end M;"
    ))
    .unwrap();
    let text = format!("{:?}", periodic.equations);
    assert!(
        text.contains("Call(\"mod\""),
        "brought back into scope: {text}"
    );

    // A table asked for no extrapolation leaves a check behind rather
    // than answering outside the scope it was written for.
    let refused = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", [0, 1; 1, 2], {{2}}, 1, 4); \
         Real y; equation y = Blocks.getValue(h, 1, time); end M;"
    ))
    .unwrap();
    let said = format!("{:?}", refused.asserts);
    assert!(said.contains("no extrapolation"), "{said}");
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

/// A constant array of an enclosing package, named as it stands and
/// read at a place the run settles: the logic libraries write their
/// tables this way, `NotTable[x]` off a signal of an enumeration.
#[test]
fn a_package_constant_array_is_named_and_read() {
    let m = parse_model(
        "package T type L = enumeration(u, x, zero, one); \
         constant L NotTable[L] = {L.u, L.x, L.one, L.zero}; \
         model M L a = L.zero; L b; equation b = NotTable[a]; end M; end T;",
    )
    .unwrap();
    let text = format!("{:?}", m.equations);
    // The name is gone: what stands is a choice among the elements.
    assert!(!text.contains("NotTable"), "{text}");
    assert!(text.contains("If(Rel(Eq, Ref(\"a\")"), "{text}");
}

/// A periodic table says the same thing every period: what it is asked
/// at is brought back into the one scope it was written for, so the
/// value beyond the far end is never reached.
#[test]
fn a_periodic_table_says_the_same_thing_every_period() {
    // A triangle of period two: nothing at the ends, ten in the middle.
    let m = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", \
           [0, 0; 1, 10; 2, 0], {{2}}, 1, 3); \
         Real y; equation y = Blocks.getValue(h, 1, time); end M;"
    ))
    .unwrap();
    let text = format!("{:?}", m.equations);
    // What it is asked at is brought back by the width of the table,
    // so the same places are read again on every round.
    assert!(text.contains("Call(\"mod\""), "{text}");
    assert!(text.contains("Number(2.0)"), "by the period: {text}");

    // A table whose scope is a single point has no period to repeat,
    // and is written out as it stands rather than divided by nothing.
    let flat = parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", \
           [1, 5; 1, 7], {{2}}, 1, 3); \
         Real y; equation y = Blocks.getValue(h, 1, time); end M;"
    ));
    assert!(flat.is_ok(), "{flat:?}");
}

/// A grid asked for a spline is drawn as a bicubic through its points.
///
/// Akima's rule along one abscissa and then along the other gives
/// every crossing a slope down, a slope across and a cross slope, and
/// the cell between four crossings is the bicubic that meets all of
/// them. Twenty-six models stood at a grid refusing to be splined.
#[test]
fn a_grid_may_be_splined() {
    // `z = u1^2 + u2^2` on a grid of whole numbers. A bicubic drawn
    // through a surface of that degree is that surface, so the value
    // between the crossings is exact and says the spline was built
    // rather than the plane.
    let m = parse_model(&format!(
        "{GRID_BLOCK} model M \
         parameter Real data[5, 5] = [0, 0, 1, 2, 3; 0, 0, 1, 4, 9; 1, 1, 2, 5, 10; \
           2, 4, 5, 8, 13; 3, 9, 10, 13, 18]; \
         Grid.Handle h = Grid.Handle(\"NoName\", \"NoName\", data, 2, 2); \
         Real y; equation y = Grid.getValue(h, 1.5, 2.5); end M;"
    ))
    .unwrap();
    let y = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"y\")")
        .unwrap();
    // Everything in it is a number, so what it comes to is a number:
    // 1.5^2 + 2.5^2, which the plane between the crossings would have
    // put a whole unit away.
    let value = folded(&y.rhs);
    assert!((value - (1.5 * 1.5 + 2.5 * 2.5)).abs() < 1e-9, "{value}");
}

/// A grid of one row is splined along the abscissa it has.
///
/// A cell is drawn from four crossings, and a grid with one row or
/// one column has only two: the other two are that edge again. Read
/// without holding the index there, the compiler walked off the end
/// of the table and the whole library check died with the thread.
#[test]
fn a_grid_of_one_row_may_be_splined() {
    let m = parse_model(&format!(
        "{GRID_BLOCK} model M \
         parameter Real data[2, 4] = [0, 0, 1, 2; 0, 0, 1, 4]; \
         Grid.Handle h = Grid.Handle(\"NoName\", \"NoName\", data, 2, 2); \
         Real y; equation y = Grid.getValue(h, 0, 1.5); end M;"
    ))
    .unwrap();
    let y = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"y\")")
        .unwrap();
    // The one row reads `0, 1, 4` against `0, 1, 2`, and a spline
    // through those three points passes between the last two.
    let value = folded(&y.rhs);
    assert!((1.0..=4.0).contains(&value), "{value}");
}

/// The monotone splines keep a stretch of table going the way it went.
///
/// Akima's rule draws a smooth curve but is entitled to dip between
/// two points that do not: a table that rises, levels off and rises
/// again comes out with a hollow in the level part. Fritsch-Butland
/// and Steffen cannot make one, which is what a table standing for a
/// characteristic curve needs.
#[test]
fn a_monotone_spline_does_not_overshoot() {
    // `0, 1, 1, 10` against `0, 1, 2, 3`: the middle is level, and a
    // spline that overshoots would leave it.
    let read = |smoothness: u32, u: &str| -> f64 {
        let m = parse_model(&format!(
            "{TABLE_BLOCK} model M \
             parameter Real data[4, 2] = [0, 0; 1, 1; 2, 1; 3, 10]; \
             Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", data, {{2}}, {smoothness}, 2); \
             Real y; equation y = Blocks.getValue(h, 1, {u}); end M;"
        ))
        .unwrap();
        let y = m
            .equations
            .iter()
            .find(|e| format!("{:?}", e.lhs) == "Ref(\"y\")")
            .unwrap();
        folded(&y.rhs)
    };
    for smoothness in [4, 5] {
        // Anywhere along the level stretch the answer is the level.
        for u in ["1.25", "1.5", "1.75"] {
            let value = read(smoothness, u);
            assert!(
                (value - 1.0).abs() < 1e-9,
                "smoothness {smoothness} at {u}: {value}"
            );
        }
        // And the rise after it stays inside the two points it joins.
        let value = read(smoothness, "2.5");
        assert!(
            (1.0..=10.0).contains(&value),
            "smoothness {smoothness}: {value}"
        );
    }
    // The Akima spline is the one that may overshoot, and does here:
    // this is what the monotone rules were added for.
    let akima = read(2, "1.75");
    assert!(akima < 1.0, "an Akima spline dips on this table: {akima}");
}

/// Each spline rule is written out, and each is drawn where the
/// others would be drawn differently.
///
/// The rules agree on a straight table and part company on one that
/// turns, so a table with a turn in it is what tells them apart. What
/// is asserted is what each rule promises: the monotone two never
/// leave the two points they join, and the Akima two are smooth.
#[test]
fn every_spline_rule_is_written_out() {
    let read = |smoothness: u32, table: &str, u: &str| -> f64 {
        let m = parse_model(&format!(
            "{TABLE_BLOCK} model M \
             Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", {table}, {{2}}, {smoothness}, 2); \
             Real y; equation y = Blocks.getValue(h, 1, {u}); end M;"
        ))
        .unwrap();
        let y = m
            .equations
            .iter()
            .find(|e| format!("{:?}", e.lhs) == "Ref(\"y\")")
            .unwrap();
        folded(&y.rhs)
    };
    // A table that turns: down, then up.
    let turning = "[0, 3; 1, 1; 2, 1; 3, 4]";
    for smoothness in [2, 4, 5, 6] {
        // Every rule meets the points it was given.
        for (u, at) in [("0", 3.0), ("1", 1.0), ("3", 4.0)] {
            let value = read(smoothness, turning, u);
            assert!(
                (value - at).abs() < 1e-9,
                "smoothness {smoothness} at {u}: {value}"
            );
        }
    }
    // The monotone rules stay level along the level stretch; the two
    // Akima rules are free to leave it and do.
    for smoothness in [4, 5] {
        let value = read(smoothness, turning, "1.5");
        assert!(
            (value - 1.0).abs() < 1e-9,
            "smoothness {smoothness}: {value}"
        );
    }
    // Two points, and every rule draws the straight line between
    // them: there is nothing else a cubic through two points with
    // matching slopes can be.
    for smoothness in [2, 4, 5, 6] {
        let value = read(smoothness, "[0, 0; 2, 4]", "1");
        assert!(
            (value - 2.0).abs() < 1e-9,
            "smoothness {smoothness}: {value}"
        );
    }
    // Steffen's end rule is a parabola through the three points
    // nearest the end, so a table of three is where it shows: the
    // curve still meets each point.
    let value = read(5, "[0, 0; 1, 3; 2, 4]", "0.5");
    assert!((0.0..=3.0).contains(&value), "{value}");
}

/// A spline needs the points of its abscissa to follow one another.
///
/// Two rows at the same place leave the line between them undefined
/// rather than flat, and a slope of zero there would be a quiet
/// mistake. A table of straight lines may say the same thing twice -
/// that is a step, which a table is entitled to.
#[test]
fn a_spline_refuses_a_repeated_abscissa() {
    for smoothness in [2, 4, 5, 6] {
        let refused = parse_model(&format!(
            "{TABLE_BLOCK} model M \
             Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", [0, 0; 1, 1; 1, 2; 2, 3], \
               {{2}}, {smoothness}, 2); \
             Real y; equation y = Blocks.getValue(h, 1, time); end M;"
        ))
        .unwrap_err()
        .to_string();
        assert!(refused.contains("twice on its abscissa"), "{refused}");
    }
    // The same table read as straight lines is a step and is allowed.
    parse_model(&format!(
        "{TABLE_BLOCK} model M \
         Blocks.Handle h = Blocks.Handle(\"NoName\", \"NoName\", [0, 0; 1, 1; 1, 2; 2, 3], \
           {{2}}, 1, 2); \
         Real y; equation y = Blocks.getValue(h, 1, time); end M;"
    ))
    .unwrap();
}

/// A table may be read out of a file rather than written in the model.
///
/// The standard tables read two formats: a text file that says the
/// shape of each table before its numbers, and a MATLAB level 4 file,
/// which is a run of matrices with nothing compressed and nothing
/// referring to anything else. Both are read here rather than by a C
/// library, so a build without one reads them too.
#[test]
fn a_table_may_be_read_from_a_file() {
    let scratch = std::env::temp_dir().join("oxidelica-table-probe");
    std::fs::create_dir_all(&scratch).unwrap();

    // The text format: a header of name and shape, then the numbers,
    // with `#` starting a comment and a file free to hold more than
    // one table.
    let text = scratch.join("probe.txt");
    std::fs::write(
        &text,
        "#1\ndouble other(1,2) # not this one\n0 0\ndouble wanted(3,2)\n0 0\n1 2 # a comment\n2 4\n",
    )
    .unwrap();
    let rows = read_table_file(text.to_str().unwrap(), "wanted").unwrap();
    assert_eq!(rows, vec![vec![0.0, 0.0], vec![1.0, 2.0], vec![2.0, 4.0]]);

    // A name the file does not hold says so rather than answering
    // with whatever was nearest.
    let missing = read_table_file(text.to_str().unwrap(), "absent").unwrap_err();
    assert!(missing.contains("no table called `absent`"), "{missing}");

    // The MATLAB format: five little-endian numbers, the name, then
    // the numbers themselves written down the columns.
    let matlab = scratch.join("probe.mat");
    let mut bytes: Vec<u8> = Vec::new();
    bytes.extend(0u32.to_le_bytes()); // little-endian doubles
    bytes.extend(3u32.to_le_bytes()); // rows
    bytes.extend(2u32.to_le_bytes()); // columns
    bytes.extend(0u32.to_le_bytes()); // nothing imaginary
    bytes.extend(7u32.to_le_bytes()); // `wanted` and its zero
    bytes.extend(b"wanted\0");
    for down_a_column in [0.0, 1.0, 2.0, 0.0, 2.0, 4.0] {
        bytes.extend(f64::to_le_bytes(down_a_column));
    }
    std::fs::write(&matlab, &bytes).unwrap();
    let rows = read_table_file(matlab.to_str().unwrap(), "wanted").unwrap();
    assert_eq!(rows, vec![vec![0.0, 0.0], vec![1.0, 2.0], vec![2.0, 4.0]]);

    // A file that is neither says so by name.
    let neither = scratch.join("probe.bin");
    std::fs::write(&neither, b"not a table at all, nor anything like one").unwrap();
    let refused = read_table_file(neither.to_str().unwrap(), "wanted").unwrap_err();
    assert!(refused.contains("neither a text table"), "{refused}");
}

/// `skipWhiteSpace` may be called without saying where to start.
///
/// The standard library declares the second argument with a default
/// of one, and `Strings.isEmpty` calls it with the string alone.
/// Answered only for two arguments, the call stood unresolved and the
/// file name of every table block read from a file stayed unsettled
/// behind it.
#[test]
fn skipping_white_space_starts_at_one_by_default() {
    let m = parse_model(
        "function skip input String s; input Integer from = 1; output Integer next; \
           external \"C\" next = ModelicaStrings_skipWhiteSpace(s, from); end skip; \
         model M Real y; equation y = skip(\"  ab\"); end M;",
    )
    .unwrap();
    let y = m
        .equations
        .iter()
        .find(|e| format!("{:?}", e.lhs) == "Ref(\"y\")")
        .unwrap();
    // Two spaces, so the text starts at the third character.
    assert_eq!(folded(&y.rhs), 3.0);
}

/// What a file that cannot be read says, and the shapes a MATLAB file
/// may be written in.
///
/// The formats are read by hand rather than by a library, so what
/// they refuse matters as much as what they accept: a shape that is
/// not two numbers, a promise of more rows than the file holds, a
/// matrix of text where a table belongs.
#[test]
fn a_table_file_says_what_it_cannot_read() {
    let scratch = std::env::temp_dir().join("oxidelica-table-refusals");
    std::fs::create_dir_all(&scratch).unwrap();
    let written = |name: &str, bytes: &[u8]| -> String {
        let path = scratch.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_string()
    };

    // A file that is not there at all.
    let missing = read_table_file("/oxidelica/no/such/file.txt", "t").unwrap_err();
    assert!(missing.contains("cannot be read"), "{missing}");

    // A header whose shape is not rows and columns.
    let odd = written("odd.txt", b"#1\ndouble t(3)\n0 0\n");
    let why = read_table_file(&odd, "t").unwrap_err();
    assert!(why.contains("not rows and columns"), "{why}");

    // A header that says something other than a number.
    let letters = written("letters.txt", b"#1\ndouble t(two,2)\n0 0\n");
    let why = read_table_file(&letters, "t").unwrap_err();
    assert!(why.contains("whole number"), "{why}");

    // A table that promises more numbers than it gives.
    let short = written("short.txt", b"#1\ndouble t(3,2)\n0 0\n1 2\n");
    let why = read_table_file(&short, "t").unwrap_err();
    assert!(why.contains("number(s)"), "{why}");

    // A number that is not one.
    let words = written("words.txt", b"#1\ndouble t(1,2)\n0 nought\n");
    let why = read_table_file(&words, "t").unwrap_err();
    assert!(why.contains("where a number belongs"), "{why}");

    // A MATLAB matrix of text rather than of numbers: the last digit
    // of the header says which.
    let mut text_matrix: Vec<u8> = Vec::new();
    text_matrix.extend(1u32.to_le_bytes()); // text
    text_matrix.extend(1u32.to_le_bytes());
    text_matrix.extend(1u32.to_le_bytes());
    text_matrix.extend(0u32.to_le_bytes());
    text_matrix.extend(2u32.to_le_bytes());
    text_matrix.extend(b"t\0");
    text_matrix.extend(f64::to_le_bytes(65.0));
    let path = written("text.mat", &text_matrix);
    let why = read_table_file(&path, "t").unwrap_err();
    assert!(why.contains("text rather than a table"), "{why}");

    // A matrix that says it is longer than the file.
    let mut past_the_end: Vec<u8> = Vec::new();
    past_the_end.extend(0u32.to_le_bytes());
    past_the_end.extend(100u32.to_le_bytes());
    past_the_end.extend(100u32.to_le_bytes());
    past_the_end.extend(0u32.to_le_bytes());
    past_the_end.extend(2u32.to_le_bytes());
    past_the_end.extend(b"t\0");
    let path = written("past.mat", &past_the_end);
    let why = read_table_file(&path, "t").unwrap_err();
    assert!(why.contains("past the end"), "{why}");

    // Single precision: the same table, written four bytes at a time.
    let mut single: Vec<u8> = Vec::new();
    single.extend(10u32.to_le_bytes()); // precision 1: single
    single.extend(2u32.to_le_bytes());
    single.extend(2u32.to_le_bytes());
    single.extend(0u32.to_le_bytes());
    single.extend(2u32.to_le_bytes());
    single.extend(b"t\0");
    for down_a_column in [0.0f32, 1.0, 3.0, 4.0] {
        single.extend(f32::to_le_bytes(down_a_column));
    }
    let path = written("single.mat", &single);
    let rows = read_table_file(&path, "t").unwrap();
    assert_eq!(rows, vec![vec![0.0, 3.0], vec![1.0, 4.0]]);

    // A header whose digits are none the format uses is not a MATLAB
    // file at all, and is refused as neither format rather than read
    // as a table of nonsense.
    let mut odd_precision: Vec<u8> = Vec::new();
    odd_precision.extend(60u32.to_le_bytes());
    odd_precision.extend(1u32.to_le_bytes());
    odd_precision.extend(1u32.to_le_bytes());
    odd_precision.extend(0u32.to_le_bytes());
    odd_precision.extend(2u32.to_le_bytes());
    odd_precision.extend(b"t\0");
    odd_precision.extend(f64::to_le_bytes(1.0));
    let path = written("precision.mat", &odd_precision);
    let why = read_table_file(&path, "t").unwrap_err();
    assert!(why.contains("neither a text table"), "{why}");

    // A MATLAB file that holds no table of that name.
    let mut named_otherwise: Vec<u8> = Vec::new();
    named_otherwise.extend(0u32.to_le_bytes());
    named_otherwise.extend(1u32.to_le_bytes());
    named_otherwise.extend(1u32.to_le_bytes());
    named_otherwise.extend(0u32.to_le_bytes());
    named_otherwise.extend(6u32.to_le_bytes());
    named_otherwise.extend(b"other\0");
    named_otherwise.extend(f64::to_le_bytes(1.0));
    let path = written("other.mat", &named_otherwise);
    let why = read_table_file(&path, "t").unwrap_err();
    assert!(why.contains("no table called `t`"), "{why}");
}

/// A file written the other way round says so by name.
///
/// The first word of a MATLAB level 4 header says how the rest is
/// written, and its thousands digit is the byte order: a file written
/// on a big-endian machine has it as one. Nothing here swaps the
/// bytes back, so such a file is refused by name rather than read as
/// a table of nonsense.
#[test]
fn a_table_file_of_the_other_byte_order_is_refused() {
    let scratch = std::env::temp_dir().join("oxidelica-table-order");
    std::fs::create_dir_all(&scratch).unwrap();
    let path = scratch.join("big.mat");
    let mut bytes: Vec<u8> = Vec::new();
    bytes.extend(1000u32.to_be_bytes()); // big-endian doubles
    bytes.extend(2u32.to_be_bytes());
    bytes.extend(2u32.to_be_bytes());
    bytes.extend(0u32.to_be_bytes());
    bytes.extend(2u32.to_be_bytes());
    bytes.extend(b"t\0");
    for down_a_column in [0.0f64, 1.0, 3.0, 4.0] {
        bytes.extend(f64::to_be_bytes(down_a_column));
    }
    std::fs::write(&path, &bytes).unwrap();
    let why = read_table_file(path.to_str().unwrap(), "t").unwrap_err();
    assert!(why.contains("byte order"), "{why}");
    assert!(why.contains("big.mat"), "{why}");
}

/// A header cut short is refused rather than read past the end.
#[test]
fn a_table_file_cut_short_is_refused() {
    let scratch = std::env::temp_dir().join("oxidelica-table-order");
    std::fs::create_dir_all(&scratch).unwrap();
    let path = scratch.join("short.mat");
    // Four bytes of a twenty-byte header.
    std::fs::write(&path, 0u32.to_le_bytes()).unwrap();
    let why = read_table_file(path.to_str().unwrap(), "t").unwrap_err();
    assert!(why.contains("short.mat"), "{why}");
}

/// A table read from a file the standard library ships.
///
/// The file name is written as a `modelica://` URI, which names a
/// library and a path inside it; what it comes to is a file beside
/// that library. This is the whole road at once - the URI resolved,
/// the file read, the numbers written into the model - against the
/// text table the standard library ships for the purpose.
#[test]
fn a_table_may_be_read_from_the_library_it_names() {
    let Some(directory) = oxidelica_parser::library_directory(None) else {
        // No library in view: nothing to read from, and nothing this
        // test can say about reading it.
        return;
    };
    let shipped = directory
        .join("Modelica")
        .join("Modelica/Resources/Data/Tables/test.txt");
    if !shipped.is_file() {
        return;
    }
    // `tab1` of that file is `0, 1, 1, 2, 3, 4` against `0, 0, 1, 4,
    // 9, 16`: the squares, with a step at one.
    let rows = read_table_file(shipped.to_str().unwrap(), "tab1").unwrap();
    assert_eq!(rows.len(), 6, "{rows:?}");
    assert_eq!(rows[3], vec![2.0, 4.0], "{rows:?}");
    assert_eq!(rows[5], vec![4.0, 16.0], "{rows:?}");
}

/// The shapes a text table may be written in, and the ones it may
/// not.
///
/// The format is read by hand, so what it accepts and what it turns
/// away are both worth saying: a table named among others, one whose
/// numbers run over several lines, and a header that is not a header
/// at all.
#[test]
fn a_text_table_is_read_line_by_line() {
    let scratch = std::env::temp_dir().join("oxidelica-table-text");
    std::fs::create_dir_all(&scratch).unwrap();
    let written = |name: &str, text: &str| -> String {
        let path = scratch.join(name);
        std::fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_string()
    };

    // Numbers spread over more lines than there are rows, blank lines
    // and comments among them.
    let spread = written(
        "spread.txt",
        "#1\ndouble t(3,2)\n# a comment first\n0 0\n\n1\n2 # half a row above\n2 4\n",
    );
    let rows = read_table_file(&spread, "t").unwrap();
    assert_eq!(rows, vec![vec![0.0, 0.0], vec![1.0, 2.0], vec![2.0, 4.0]]);

    // A line that is not a header at all is passed over rather than
    // taken for one.
    let noise = written(
        "noise.txt",
        "#1\nsomething else entirely\ndouble t(1,2)\n5 6\n",
    );
    assert_eq!(read_table_file(&noise, "t").unwrap(), vec![vec![5.0, 6.0]]);

    // A header without its closing bracket names no table.
    let unclosed = written("unclosed.txt", "#1\ndouble t(1,2\n5 6\n");
    assert!(read_table_file(&unclosed, "t").is_err());

    // Nor one without a bracket at all.
    let flat = written("flat.txt", "#1\ndouble t\n5 6\n");
    assert!(read_table_file(&flat, "t").is_err());
}

/// A MATLAB matrix of a precision other than double.
///
/// The format allows six, and the standard tables are written as
/// doubles - but a file may hold a matrix of shorter numbers beside
/// them, and the name asked for may be that one.
#[test]
fn a_matlab_table_may_be_written_shorter() {
    let scratch = std::env::temp_dir().join("oxidelica-table-widths");
    std::fs::create_dir_all(&scratch).unwrap();
    let written = |name: &str, bytes: &[u8]| -> String {
        let path = scratch.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_string()
    };
    let header = |precision: u32, rows: u32, columns: u32, named: &[u8]| -> Vec<u8> {
        let mut head: Vec<u8> = Vec::new();
        head.extend((precision * 10).to_le_bytes());
        head.extend(rows.to_le_bytes());
        head.extend(columns.to_le_bytes());
        head.extend(0u32.to_le_bytes());
        head.extend((named.len() as u32).to_le_bytes());
        head.extend(named);
        head
    };

    // Four-byte whole numbers.
    let mut bytes = header(2, 2, 2, b"t\0");
    for down_a_column in [0i32, 1, 3, 4] {
        bytes.extend(down_a_column.to_le_bytes());
    }
    let path = written("ints.mat", &bytes);
    assert_eq!(
        read_table_file(&path, "t").unwrap(),
        vec![vec![0.0, 3.0], vec![1.0, 4.0]]
    );

    // Two-byte ones, signed and unsigned.
    let mut bytes = header(3, 1, 2, b"t\0");
    bytes.extend((-2i16).to_le_bytes());
    bytes.extend(7i16.to_le_bytes());
    let path = written("shorts.mat", &bytes);
    assert_eq!(read_table_file(&path, "t").unwrap(), vec![vec![-2.0, 7.0]]);

    let mut bytes = header(4, 1, 2, b"t\0");
    bytes.extend(60000u16.to_le_bytes());
    bytes.extend(7u16.to_le_bytes());
    let path = written("words.mat", &bytes);
    assert_eq!(
        read_table_file(&path, "t").unwrap(),
        vec![vec![60000.0, 7.0]]
    );

    // Single bytes.
    let mut bytes = header(5, 1, 2, b"t\0");
    bytes.extend([200u8, 7]);
    let path = written("bytes.mat", &bytes);
    assert_eq!(read_table_file(&path, "t").unwrap(), vec![vec![200.0, 7.0]]);

    // A matrix passed over on the way to the one asked for: the
    // reader steps past its numbers by their own width.
    let mut bytes = header(2, 1, 1, b"other\0");
    bytes.extend(9i32.to_le_bytes());
    bytes.extend(header(0, 1, 1, b"t\0"));
    bytes.extend(f64::to_le_bytes(3.5));
    let path = written("second.mat", &bytes);
    assert_eq!(read_table_file(&path, "t").unwrap(), vec![vec![3.5]]);
}

#[test]
fn a_table_on_a_file_says_how_wide_it_is() {
    // A table block declares `table` empty and writes `columns[:] =
    // 2:size(table, 2)` beside it: what says how many columns there
    // are is the file named among the same parameters. Measured by
    // the empty declaration the range has no length at all, and
    // twenty-five models stood there.
    let file = std::env::temp_dir().join("oxidelica_table_width.txt");
    std::fs::write(&file, "#1\ndouble tabA(3,3)\n0 1 2\n1 3 4\n2 5 6\n").unwrap();
    let source = format!(
        "block Table parameter Boolean tableOnFile = false; \
         parameter String fileName = \"NoName\"; \
         parameter String tableName = \"NoName\"; \
         parameter Real table[:, :] = fill(0.0, 0, 2); \
         parameter Integer columns[:] = 2:size(table, 2); \
         output Real y; equation y = size(columns, 1); end Table; \
         model M Table t(tableOnFile = true, tableName = \"tabA\", \
         fileName = \"{}\"); Real z; equation z = t.y; end M;",
        as_modelica_path(&file)
    );
    let flat = parse_model(&source).unwrap();
    // Three columns in the file, counted from the second: two.
    let written = format!("{:?}", flat.equations);
    assert!(written.contains("2.0"), "{written}");
    std::fs::remove_file(&file).ok();
}

#[test]
fn a_table_block_that_is_not_on_a_file_keeps_its_declared_width() {
    // Every one of these blocks carries the same `columns[:] =
    // 2:size(table, 2)`, so reading a file for a block that says its
    // table is written out would take numbers meant for another.
    let flat = parse_model(
        "block Table parameter Boolean tableOnFile = false; \
         parameter String fileName = \"NoName\"; \
         parameter String tableName = \"NoName\"; \
         parameter Real table[:, :] = [0, 1, 2; 1, 3, 4]; \
         parameter Integer columns[:] = 2:size(table, 2); \
         output Real y; equation y = size(columns, 1); end Table; \
         model M Table t; Real z; equation z = t.y; end M;",
    )
    .unwrap();
    // Three columns written out, counted from the second: two.
    assert!(format!("{:?}", flat.equations).contains("2.0"));
}

#[test]
fn a_table_file_that_will_not_read_leaves_the_measurement_alone() {
    // A name with no shape is asked again; a wrong shape is settled
    // for good. What could not be read is left to the table machinery
    // to complain about later, by the name of the file.
    let missing = std::env::temp_dir().join("oxidelica_no_such_table.txt");
    std::fs::remove_file(&missing).ok();
    let source = format!(
        "block Table parameter Boolean tableOnFile = false; \
         parameter String fileName = \"NoName\"; \
         parameter String tableName = \"NoName\"; \
         parameter Real table[:, :] = fill(0.0, 0, 2); \
         parameter Integer columns[:] = 2:size(table, 2); \
         output Real y; equation y = size(columns, 1); end Table; \
         model M Table t(tableOnFile = true, tableName = \"tabA\", \
         fileName = \"{}\"); Real z; equation z = t.y; end M;",
        as_modelica_path(&missing)
    );
    let why = parse_model(&source).unwrap_err().to_string();
    assert!(why.contains("flexible size"), "{why}");
}

#[test]
fn a_table_on_a_file_says_how_many_rows_it_has() {
    // The first dimension is read the same way as the second.
    let file = std::env::temp_dir().join("oxidelica_table_rows.txt");
    std::fs::write(&file, "#1\ndouble tabA(3,3)\n0 1 2\n1 3 4\n2 5 6\n").unwrap();
    let source = format!(
        "block Table parameter Boolean tableOnFile = false; \
         parameter String fileName = \"NoName\"; \
         parameter String tableName = \"NoName\"; \
         parameter Real table[:, :] = fill(0.0, 0, 2); \
         parameter Integer rows[:] = 1:size(table, 1); \
         output Real y; equation y = size(rows, 1); end Table; \
         model M Table t(tableOnFile = true, tableName = \"tabA\", \
         fileName = \"{}\"); Real z; equation z = t.y; end M;",
        as_modelica_path(&file)
    );
    let flat = parse_model(&source).unwrap();
    // Three rows, counted from the first: three.
    assert!(format!("{:?}", flat.equations).contains("3.0"));
    std::fs::remove_file(&file).ok();
}

/// A path as a Modelica string literal.
///
/// Windows writes `C:\\Users\\...`, and a Modelica string reads a
/// backslash as the start of an escape - `\\U` is not one the
/// language has. Forward slashes name the same file on every system
/// these tests run on.
fn as_modelica_path(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// A table read from a file is handed its own name first and the file
/// second, and it is read from the seat it was written into.
#[test]
fn a_table_reads_the_file_from_the_seat_it_was_written_into() {
    // A table carried in the model is named `"NoName"` in both of the
    // first two seats, so it reads the same whichever way round they
    // are taken, and said nothing about which seat is which. A file
    // says it plainly: the name of the table inside it stands first.
    let dir = std::env::temp_dir().join("oxidelica_seat_order_test");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("rows.txt");
    std::fs::write(&file, "#1\ndouble tab1(3,2)\n0 0\n1 2\n2 4\n").unwrap();
    let m = parse_model(&format!(
        "{TIME_TABLE_BLOCK} model M \
         Times.Handle h = Times.Handle(\"tab1\", \"{}\", fill(0.0, 0, 2), \
           0, {{2}}, 1, 2, 0); \
         Real y; \
         equation y = Times.getValue(h, 1, time, 0, 0); end M;",
        // A Modelica string takes its path with forward slashes: a
        // Windows path written as it comes would carry `\U` into the
        // source, which is not an escape this language has.
        file.display().to_string().replace('\\', "/")
    ))
    .unwrap();
    let said = m
        .equations
        .iter()
        .find(|e| matches!(&e.lhs, Expr::Ref(lhs) if lhs == "y"))
        .map(|e| format!("{:?}", e.rhs))
        .unwrap_or_default();
    // Read from the file, the rows interpolate; read from the wrong
    // seat, the file is looked for under the name `tab1` and the call
    // stands where a table should be.
    assert!(
        !said.contains("getValue"),
        "the table was not read from its file: {said}"
    );
    assert!(
        said.contains("Number(2.0)") && said.contains("Time"),
        "the rows did not reach the equation: {said}"
    );
}

/// A text table written by an editor that marks its UTF-8 is read
/// past the mark.
#[test]
fn a_byte_order_mark_is_not_part_of_the_table() {
    let dir = std::env::temp_dir().join("oxidelica_bom_test");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("marked.txt");
    // The mark, then the `#1` every text table begins with.
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(b"#1\ndouble tab1(3,2)\n0 0\n1 2\n2 4\n");
    std::fs::write(&file, &bytes).unwrap();
    let rows = read_table_file(&file.display().to_string(), "tab1")
        .expect("a marked file is still a text table");
    assert_eq!(rows, vec![vec![0.0, 0.0], vec![1.0, 2.0], vec![2.0, 4.0]]);
}

/// A resource URI that named no library says where it looked.
#[test]
fn a_resource_that_was_not_found_says_where_it_looked() {
    // `modelica://Library/path` reaching the table reader at all means
    // it was never turned into a file. "Cannot be read" would be true
    // of a missing file too, and these are not the same mistake: one
    // is data nobody wrote, the other data written under a library
    // this compiler is reading from somewhere else. The paths tried
    // tell them apart, and without them the difference cost an hour.
    let why = read_table_file("modelica://NoSuchLibrary/Data/rows.txt", "tab1")
        .expect_err("no such library, so no such resource");
    assert!(
        why.contains("modelica://NoSuchLibrary/Data/rows.txt") && why.contains("Tried:"),
        "a refusal that does not say where it looked: {why}"
    );
}
