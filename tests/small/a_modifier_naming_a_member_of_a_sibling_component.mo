// A modifier handed down an `extends` may name a member of a sibling
// component rather than a declaration of the class that hands it:
// an induction machine writes `extends PartialBasicMachine(final
// idq_rs = airGap.i_rs)`, where `airGap` stands beside the `extends`.
//
// With the fault present the member has no measured shape, so the
// value comes back whole and every element of the bound array is
// tied to the whole array - one equation per pair rather than one
// per element. The refusal is a surplus of equations:
//
//   unbalanced model: 8 algebraic equation(s) for 6 unknown(s)
//
// Fixed, `a.u[1] = a.inner1.y[1]` and `a.u[2] = a.inner1.y[2]` are
// the only two equations the binding writes, and the model runs.
partial model Base
  input Real u[2];
  Real z[2];
equation
  z = 2 * u;
end Base;

model Inner
  Real y[2];
equation
  y[1] = 10;
  y[2] = 20;
end Inner;

model Mid
  extends Base(final u = inner1.y);
  Inner inner1;
end Mid;

model a_modifier_naming_a_member_of_a_sibling_component
  Mid a;
end a_modifier_naming_a_member_of_a_sibling_component;
