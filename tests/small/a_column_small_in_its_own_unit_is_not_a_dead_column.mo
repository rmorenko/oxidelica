// An enthalpy in a cooling circuit is carried by a mass flow, and at
// rest that flow is zero: `PumpAndValve` hands the solver a block
// whose five enthalpy columns sit at 1e-24 beside a volume flow at
// 1e-4. How small a column's entries are is the unit its unknown is
// measured in, and no honest test of whether a block determines a
// step may notice that - but `solve_linear` judges its pivots against
// 1e-14 flat, so the enthalpy columns read as though nothing in the
// block moved when they did, and the refusal came back as a singular
// Jacobian about a block with one plain answer.
//
// Divided each column through by its own largest entry the block is
// invertible, and the step comes back in the scaled unknowns to be
// divided out again. The same argument `equilibrate_columns` was
// written for, one path over: the check that a *converged* block is
// determined already scaled, and the step that has to get there did
// not.
//
// A step rescued that way is shortened from its first use, which is
// the same fact read the other way round: a pivot too small to solve
// against unscaled says the block is nearly flat along that unknown,
// so the full step crosses a direction the linear model barely
// describes.
//
// Here `m` is the mass flow at rest and `h` the enthalpy it carries.
// The first equation fixes the flow: `q*|q| = 0.25` gives `q = 0.5`.
// The second then fixes the enthalpy at `h = 293.4`, which is the
// number to check rather than the fact that anything was solved.

model a_column_small_in_its_own_unit_is_not_a_dead_column
  parameter Real m = 1e-24 "the mass flow that carries the enthalpy";
  Real h(start = 288.0) "the enthalpy, measured in its own unit";
  Real q(start = 0.1) "the volume flow, measured in its own";
equation
  q*abs(q) + m*h = 0.25 + 293.15*m;
  m*h*abs(h) = m*293.4*abs(293.4) + q*1e-24 - 0.5e-24;
end a_column_small_in_its_own_unit_is_not_a_dead_column;
