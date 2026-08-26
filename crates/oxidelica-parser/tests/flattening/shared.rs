//! What the flattening tests share: sources shaped like a
//! standard-library package, and the handful of readers that turn a
//! flat model back into something an assertion can look at.

use oxidelica_parser::ast::{BinOp, RelOp};
use oxidelica_parser::{parse_model_with_libraries, Expr};

/// Sources that share the shape of a standard-library package: a
/// replaceable component with an interface, a conditional one and a
/// world shared through `inner`/`outer`.
pub(crate) const LIB: &str = "package Lib \
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
pub(crate) fn with_lib(source: &str) -> Result<oxidelica_parser::Model, String> {
    parse_model_with_libraries(&[LIB.to_string()], source).map_err(|e| e.to_string())
}

/// When each `when` clause of a flat model ticks: the start and the
/// interval the clock was lowered onto.
pub(crate) fn ticks_of(m: &oxidelica_parser::Model) -> Vec<(f64, f64)> {
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

/// A function whose body the differentiator cannot read, and the
/// derivative the model supplies for it.
pub(crate) const NOT_SMOOTH: &str = "function f input Real x; output Real y; \
     algorithm y := abs(x) * 2; annotation(derivative = fd); end f; \
     function fd input Real x; input Real x_der; output Real y_der; \
     algorithm y_der := (if x >= 0 then 2 else -2) * x_der; end fd; ";

/// The right-hand side of the equation defining `name`.
pub(crate) fn rhs_of(model: &oxidelica_parser::Model, name: &str) -> String {
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

/// Sources with an operator record, the way `Complex` is written: two
/// fields, a constructor whose second argument has a default, and a
/// subtraction of its own.
pub(crate) const OPERATOR_RECORD: &str = "operator record C Real re; Real im; \
     encapsulated operator 'constructor' \
       function fromReal input Real re; input Real im = 0; \
       output .C c(re = re, im = im); algorithm end fromReal; \
     end 'constructor'; \
     encapsulated operator function '-' input .C a; input .C b; output .C c; \
       algorithm c := .C(a.re - b.re, a.im - b.im); end '-'; \
     end C; ";

/// A table block of the shape the standard library gives one: the data
/// in a handle built from a matrix, and the value asked for by a call
/// to a body written in C.
pub(crate) const TIME_TABLE_BLOCK: &str = "package Times \
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

pub(crate) const TABLE_BLOCK: &str = "package Blocks \
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

pub(crate) const GRID_BLOCK: &str = "package Grid \
     class Handle extends ExternalObject; \
       function constructor input String tableName; input String fileName; \
         input Real table[:, :]; input Integer smoothness; \
         input Integer extrapolation; output Handle h; \
         external \"C\" h = ModelicaStandardTables_CombiTable2D_init(fileName, tableName, \
           table, size(table, 1), size(table, 2), smoothness, \
           extrapolation); end constructor; \
       function destructor input Handle h; \
         external \"C\" ModelicaStandardTables_CombiTable2D_close(h); end destructor; \
     end Handle; \
     function getValue input Handle h; input Real u1; input Real u2; output Real y; \
       external \"C\" y = ModelicaStandardTables_CombiTable2D_getValue(h, u1, u2); \
       end getValue; \
     function getDerValue input Handle h; input Real u1; input Real u2; \
       input Real der_u1; input Real der_u2; output Real der_y; \
       external \"C\" der_y = ModelicaStandardTables_CombiTable2D_getDerValue(h, u1, u2, \
         der_u1, der_u2); end getDerValue; \
     function umin input Handle h; output Real u[2]; \
       external \"C\" ModelicaStandardTables_CombiTable2D_minimumAbscissa(h, u); end umin; \
     function umax input Handle h; output Real u[2]; \
       external \"C\" ModelicaStandardTables_CombiTable2D_maximumAbscissa(h, u); end umax; \
   end Grid; ";

/// What a constant expression comes to, for the handful of operators a
/// written-out table uses.
pub(crate) fn folded(expr: &Expr) -> f64 {
    match expr {
        Expr::Number(n) => *n,
        Expr::Neg(inner) => -folded(inner),
        Expr::Bin(op, l, r) => {
            let (l, r) = (folded(l), folded(r));
            match op {
                BinOp::Add => l + r,
                BinOp::Sub => l - r,
                BinOp::Mul => l * r,
                BinOp::Div => l / r,
                other => panic!("a table wrote {other:?}"),
            }
        }
        Expr::Call(name, args) => {
            let args: Vec<f64> = args.iter().map(folded).collect();
            match name.as_str() {
                "min" => args[0].min(args[1]),
                "max" => args[0].max(args[1]),
                "mod" => args[0] - (args[0] / args[1]).floor() * args[1],
                other => panic!("a table wrote {other}"),
            }
        }
        Expr::If(condition, yes, no) => match holds(condition) {
            true => folded(yes),
            false => folded(no),
        },
        // A call the run walks carries its own rule for
        // differentiating it beside it; the value is the value.
        Expr::WithDerivative(value, _, _) => folded(value),
        other => panic!("a table wrote {other:?}"),
    }
}

/// The same, for the comparisons a written-out table branches on.
pub(crate) fn holds(expr: &Expr) -> bool {
    match expr {
        Expr::Rel(op, l, r) => {
            let (l, r) = (folded(l), folded(r));
            match op {
                RelOp::Lt => l < r,
                RelOp::Le => l <= r,
                RelOp::Gt => l > r,
                RelOp::Ge => l >= r,
                other => panic!("a table wrote {other:?}"),
            }
        }
        other => panic!("a table branched on {other:?}"),
    }
}

/// A generator of the shape the standard library declares one: the
/// state carried as two `Integer`s, a body written outside Modelica,
/// and an answer of a value and the state it moved to.
pub(crate) const GENERATOR: &str = "package Gen constant Integer nState = 2; \
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

/// Every equation of a flat model, written out as text.
pub(crate) fn equations_of(m: &oxidelica_parser::Model) -> Vec<String> {
    m.equations
        .iter()
        .map(|e| format!("{} = {}", e.lhs.describe(), e.rhs.describe()))
        .collect()
}
