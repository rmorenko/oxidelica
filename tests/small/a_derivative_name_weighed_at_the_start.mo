// Squeezed from Modelica.Mechanics.MultiBody.Examples.Systems.RobotR3.
// Utilities.MechanicalStructure to four revolute joints and their
// bodies, and standing for a tie index reduction could not weigh.
//
// The first reduction spends `r1.phi` and makes `der(r1.phi)`, which
// the former state equation defines as `r1.w`. Nothing declared that
// name, so it has no start. Every derivative a rotation matrix passes
// down the chain of bodies reads it in the end, and the eighty-sixth
// reduction is a tie whose cone of definitions reads it: weighed with
// the starts alone the cone could not be read, the tie fell to the
// order of the walk and `r3.w` was demoted.
//
// Weighed with what the names without a start work out to at the start
// (`held_at_start`), the cone reads, the tie keeps only what the
// constraint determines, and `b4.body.v_0[2]` is demoted instead.
// OXIDELICA_NO_DERIVATIVES_AT_REDUCTION gives back the old choice, and
// `scripts/victim_witness.sh` asks both of one binary.
//
// Neither side runs. Both stop at a start where `r3.R_rel.T[2,3]`, a
// divisor, reads zero - the wall of `sin(phi) = 0` the census already
// names, and a different one from the wall this model stands for.
model a_derivative_name_weighed_at_the_start
  import Modelica.Mechanics.MultiBody.*;
  Real q[4]; Real qd[4]; Real qdd[4];
  inner World world(n={0,-1,0}, animateWorld=false, animateGravity=false, enableAnimation=false);
  Joints.Revolute r1(n={0,1,0}, useAxisFlange=true, animation=false);
  Joints.Revolute r2(n={1,0,0}, useAxisFlange=true, animation=false);
  Joints.Revolute r3(n={1,0,0}, useAxisFlange=true, animation=false);
  Parts.BodyShape b0(r={0,0.351,0}, r_CM={0,0,0}, m=1, animation=false);
  Parts.BodyShape b1(r={0,0.324,0.3}, I_22=1.16, r_CM={0,0,0}, m=1, animation=false);
  Parts.BodyShape b2(r={0,0.65,0}, r_CM={0.172,0.205,0}, m=56.5, I_11=2.58, I_22=0.64, I_33=2.73, I_21=-0.46, animation=false);
  Parts.BodyShape b3(r={0,0.414,-0.155}, r_CM={0.064,-0.034,0}, m=26.4, I_11=0.279, I_22=0.245, I_33=0.413, I_21=-0.070, animation=false);
  Modelica.Mechanics.Rotational.Components.Inertia i1(J=1, phi(fixed=true, start=0), w(fixed=true, start=0));
  Modelica.Mechanics.Rotational.Components.Inertia i2(J=1, phi(fixed=true, start=0), w(fixed=true, start=0));
  Modelica.Mechanics.Rotational.Components.Inertia i3(J=1, phi(fixed=true, start=0), w(fixed=true, start=0));
  Joints.Revolute r4(n={0,1,0}, useAxisFlange=true, animation=false);
  Parts.BodyShape b4(r={0,0.186,0}, r_CM={0,0,0}, m=28.7, I_11=1.67, I_22=0.081, I_33=1.67, animation=false);
  Modelica.Mechanics.Rotational.Components.Inertia i4(J=1, phi(fixed=true, start=0), w(fixed=true, start=0));
equation
  q = {r1.phi, r2.phi, r3.phi, r4.phi};
  qd = der(q);
  qdd = der(qd);
  connect(world.frame_b, b0.frame_a);
  connect(b0.frame_b, r1.frame_a);
  connect(r1.frame_b, b1.frame_a);
  connect(b1.frame_b, r2.frame_a);
  connect(r2.frame_b, b2.frame_a);
  connect(b2.frame_b, r3.frame_a);
  connect(r3.frame_b, b3.frame_a);
  connect(r1.axis, i1.flange_b);
  connect(r2.axis, i2.flange_b);
  connect(r3.axis, i3.flange_b);
  connect(b3.frame_b, r4.frame_a);
  connect(r4.frame_b, b4.frame_a);
  connect(r4.axis, i4.flange_b);
  annotation(experiment(StopTime=0.5));
end a_derivative_name_weighed_at_the_start;
