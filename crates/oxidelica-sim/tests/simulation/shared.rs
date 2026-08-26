//! What the simulation tests share: running a source to a result, and
//! the two ways of asking for the refusal instead.

use oxidelica_parser::parse_model;
use oxidelica_sim::{compile, SimResult, SolverMethod};

pub(crate) fn run(source: &str) -> SimResult {
    let model = parse_model(source).unwrap();
    compile(&model).unwrap().simulate().unwrap()
}

pub(crate) fn compile_err(source: &str) -> String {
    compile(&parse_model(source).unwrap())
        .unwrap_err()
        .to_string()
}

/// Compile a model and give back what it refused to do.
pub(crate) fn refused(source: &str) -> String {
    let model = parse_model(source).expect("parses");
    match compile(&model) {
        Ok(_) => panic!("should have been refused"),
        Err(e) => e.to_string(),
    }
}

/// Compile `source`, run it on `method`, and give back the result.
pub(crate) fn run_on(source: &str, method: SolverMethod) -> Result<SimResult, String> {
    let mut compiled = compile(&parse_model(source).unwrap()).unwrap();
    compiled.method = method;
    compiled.simulate().map_err(|e| e.to_string())
}

/// Compile a model, run it, and give back what stopped it.
pub(crate) fn run_err(source: &str) -> String {
    let model = parse_model(source).expect("parses");
    compile(&model)
        .expect("compiles")
        .simulate()
        .expect_err("should have been stopped")
        .to_string()
}

/// A table block of the shape the standard library gives one whose
/// first column is time: the data in a handle, the value asked for by
/// a body written in C, and the corners asked for so the run can put
/// events there.
pub(crate) const TIME_TABLE: &str = "package Times \
     class Handle extends ExternalObject; \
       function constructor input String tableName; input String fileName; \
         input Real table[:, :]; input Real startTime; input Integer columns[:]; \
         input Integer smoothness; input Integer extrapolation; input Real shiftTime; \
         output Handle h; \
         external \"C\" h = ModelicaStandardTables_CombiTimeTable_init3(tableName, fileName, \
           table, startTime, columns, smoothness, extrapolation, shiftTime); \
         end constructor; \
       function destructor input Handle h; \
         external \"C\" ModelicaStandardTables_CombiTimeTable_close(h); end destructor; \
     end Handle; \
     function getValue input Handle h; input Integer column; input Real t; \
       input Real nextEvent; input Real preNextEvent; output Real y; \
       external \"C\" y = ModelicaStandardTables_CombiTimeTable_getValue(h, column, t, \
         nextEvent, preNextEvent); end getValue; \
     function nextEvent input Handle h; input Real t; output Real at; \
       external \"C\" at = ModelicaStandardTables_CombiTimeTable_nextTimeEvent(h, t); \
       end nextEvent; \
   end Times; ";
