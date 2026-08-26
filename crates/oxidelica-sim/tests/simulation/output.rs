//! What a run reports: the result columns, the intervals, and the units they carry.

use super::shared::*;
use oxidelica_parser::parse_model;
use oxidelica_sim::{compile, SolverMethod};

#[test]
fn reports_missing_equation() {
    let model = parse_model("model B Real x; Real y; equation x = 1; end B;").unwrap();
    let error = compile(&model).unwrap_err();
    assert!(error.0.contains("unbalanced"), "{}", error.0);
}

#[test]
fn csv_output_and_defaults() {
    let model = parse_model("model M Real x(start=1); equation der(x) = -x; end M;").unwrap();
    let compiled = compile(&model).unwrap();
    // Defaults apply without an annotation.
    assert_eq!(compiled.stop_time, 1.0);
    assert_eq!(compiled.step, 1e-3);
    let result = compiled.simulate().unwrap();
    let csv = result.to_csv();
    let mut lines = csv.lines();
    assert_eq!(lines.next().unwrap(), "time,x");
    // Values are written at full precision in the shortest text that
    // reads back as the same double, so a round number is round.
    assert_eq!(lines.next().unwrap(), "0,1");
    assert_eq!(csv.lines().count(), result.rows.len() + 1);
    // And a value that is not round keeps every digit it needs.
    let last = csv.lines().last().unwrap();
    let value: f64 = last.split(',').nth(1).unwrap().parse().unwrap();
    assert!((value - result.rows.last().unwrap()[1]).abs() == 0.0);
}

#[test]
fn the_discrete_layer_reports_its_error_paths() {
    let error = |source: &str| {
        let model = parse_model(source).unwrap();
        compile(&model).unwrap_err().to_string()
    };
    // `pre` needs a variable that has a value from before the event.
    assert!(error(
        "model M Real x(start = 0, fixed = true); Real y; equation der(x) = 1; y = pre(x); end M;"
    )
    .contains("not discrete"));
    // A clock with no period.
    assert!(error(
        "model M Real x(start = 0, fixed = true); Real u; equation der(x) = 1; \
         when sample(0, 0) then u = x; end when; end M;"
    )
    .contains("interval must be positive"));
    // A `when` assigning something that was never declared.
    assert!(error(
        "model M Real x(start = 0, fixed = true); equation der(x) = 1; \
         when x > 0.5 then u = 1; end when; end M;"
    )
    .contains("never declared"));
    // A discrete variable nothing ever assigns.
    assert!(error(
        "model M discrete Real u; Real x(start = 0, fixed = true); equation der(x) = 1; end M;"
    )
    .contains("never assigned"));
    // `pre` of an expression rather than a variable.
    assert!(error(
        "model M Real x(start = 0, fixed = true); Real u; equation der(x) = 1; \
         when x > 0.5 then u = pre(x + 1); end when; end M;"
    )
    .contains("takes a variable"));

    // The fixed grid of RK4 cannot step onto a time event.
    let model = parse_model(
        "model M Real x(start = 0, fixed = true); Real u; equation der(x) = 1; \
         when sample(0, 0.1) then u = x; end when; \
         annotation(experiment(StopTime = 1.0, Interval = 0.05)); end M;",
    )
    .unwrap();
    let mut compiled = compile(&model).unwrap();
    compiled.method = SolverMethod::Rk4;
    assert!(compiled
        .simulate()
        .unwrap_err()
        .to_string()
        .contains("dopri45 or bdf"));
    // The stiff solver steps onto them like the adaptive one does.
    compiled.method = SolverMethod::Bdf;
    let stiff = compiled.simulate().unwrap();
    let u = stiff.columns.iter().position(|c| c == "u").unwrap();
    assert!((stiff.rows.last().unwrap()[u] - 1.0).abs() < 1e-6);
}

#[test]
fn matrix_results_survive_the_csv_round_trip() {
    // A 2-D name holds a comma; the CSV quotes it and the value
    // reads back intact.
    let result = run(
        "model M parameter Real A[2, 2] = [1, 2; 3, 4]; Real mm[2, 2]; \
         equation mm = A * transpose(A); \
         annotation(experiment(StopTime = 1.0, Interval = 0.5)); end M;",
    );
    let csv = result.to_csv();
    assert!(csv.contains("\"mm[1,2]\""), "{csv}");
    let index = |name: &str| result.columns.iter().position(|c| c == name).unwrap();
    let last = result.rows.last().unwrap();
    assert_eq!(last[index("mm[1,1]")], 5.0);
    assert_eq!(last[index("mm[1,2]")], 11.0);
    assert_eq!(last[index("mm[2,2]")], 25.0);
}

#[test]
fn results_carry_the_parameter_values() {
    // Consumers such as the 3D view read sizes and colours from
    // here, since those never appear as columns.
    let result = run(
        "model P parameter Real k = 2.5; parameter Real m = 4; Real y; \
         equation y = k * m * time; \
         annotation(experiment(StopTime=1.0, Interval=0.5)); end P;",
    );
    assert_eq!(
        result.parameters,
        vec![("k".to_string(), 2.5), ("m".to_string(), 4.0)]
    );
    assert!((result.rows.last().unwrap()[1] - 10.0).abs() < 1e-12);
}

#[test]
fn a_bound_broken_only_by_the_run_is_reported_where_it_broke() {
    // Modelica calls `min` and `max` assertions on the value. The
    // checker settles the ones it can before the run; this one it
    // cannot, because nothing says beforehand where a falling level
    // ends up.
    let broken = run_on(
        "model B Real level(start = 2, fixed = true, min = 0); \
         equation der(level) = -1; \
         annotation(experiment(StopTime = 3, Interval = 0.1)); end B;",
        SolverMethod::Dopri45,
    )
    .expect_err("crosses its floor");
    assert!(
        broken.contains("`level` went below its min of 0") && broken.contains("t = 2."),
        "{broken}"
    );

    // A run that stays inside is not disturbed by the guard.
    let fine = run_on(
        "model G Real level(start = 2, fixed = true, min = 0, max = 5); \
         equation der(level) = -0.1; \
         annotation(experiment(StopTime = 3, Interval = 0.1)); end G;",
        SolverMethod::Dopri45,
    )
    .expect("stays inside");
    assert!((fine.rows.last().unwrap()[1] - 1.7).abs() < 1e-9);
}
