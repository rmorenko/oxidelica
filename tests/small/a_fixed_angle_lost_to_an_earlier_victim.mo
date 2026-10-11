// Squeezed from Modelica.Mechanics.MultiBody.Examples.Elementary.
// PendulumWithSpringDamper to the world, a bar, the two joints, a body
// and the damper with no spring, and standing for a fixed angle lost by
// a choice made three reductions before the one that lost it.
//
// Reduction 1 differentiates the damper's own connection
// `damper1.frame_b.r_0[3] = body1.frame_a.r_0[3]` and spends
// `body1.frame_a.r_0[3]`, whose slope there is 1, while the damper's
// guarded length `s = max(length, s_small)` weighs nothing. Reductions 2
// and 3 spend the other two positions on the orientation row, and by
// reduction 4 only the joint coordinates the model fixed are left: the
// angle goes, and the run stops at `damper1.e_rel_0[1] =
// damper1.r_rel_0[1] / damper1.s` being -inf at t = 0.
//
// With the retreat, reduction 4 sends the walk back to reduction 1,
// which takes `damper1.s` instead, and the angle stays.
// OXIDELICA_NO_RETREAT gives back the first walk. The model is still
// refused after that: the blocks divide by the guard's slope, which is
// 0 at the start. Leaving names of zero slope out of the matching's
// rows makes it run, with the angle matching a hand-written polar ODE
// to 4.5e-4, but that pruning is parked (docs/ANALYSIS.md, m400): it
// costs GearConstraint, whose zero axis element is the same kind of
// zero as `prismatic.e[3]` here.
model a_fixed_angle_lost_to_an_earlier_victim
  inner Modelica.Mechanics.MultiBody.World world;
  Modelica.Mechanics.MultiBody.Parts.Body body1(m=1, I_11=1, I_22=1, I_33=1, r_CM={0,0,0}, animation=false);
  Modelica.Mechanics.MultiBody.Parts.FixedTranslation bar1(animation=false, r={0.3,0,0});
  Modelica.Mechanics.MultiBody.Forces.Damper damper1(d=1, animation=false);
  Modelica.Mechanics.MultiBody.Joints.Revolute revolute(phi(fixed=true), w(fixed=true), animation=false);
  Modelica.Mechanics.MultiBody.Joints.Prismatic prismatic(s(fixed=true, start=0.5), v(fixed=true), animation=false);
equation
  connect(world.frame_b, bar1.frame_a);
  connect(revolute.frame_a, bar1.frame_b);
  connect(prismatic.frame_a, revolute.frame_b);
  connect(body1.frame_a, prismatic.frame_b);
  connect(damper1.frame_a, bar1.frame_b);
  connect(damper1.frame_b, body1.frame_a);
  annotation(experiment(StopTime=1));
end a_fixed_angle_lost_to_an_earlier_victim;
