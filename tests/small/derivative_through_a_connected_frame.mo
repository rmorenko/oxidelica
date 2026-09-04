// Squeezed from Modelica.Mechanics.MultiBody.Examples.Elementary.Pendulum
// to nine lines, and standing for fifteen multibody models refused for
// `cannot differentiate through algebraic variable ...R.w` or `...R.T`.
//
// The connection puts `world.frame_b.R.w[3] = rev.frame_a.R.w[3]` in
// the system, and the world says its own side is zero in an equation
// of its own. But definitions are gathered while skipping the equation
// currently under reduction - and here that equation *is* the
// connection, so the joint's angular velocity is left with no
// definition to reach through, and a frame that never turns is called
// something nothing can differentiate.
//
// Reading the connection for what it says about the other side was
// built and measured: it takes this model and costs twelve elsewhere.
// The definition it supplies is right; what goes wrong is downstream
// of having it.
model DerivativeThroughAConnectedFrame
  inner Modelica.Mechanics.MultiBody.World world(
    gravityType = Modelica.Mechanics.MultiBody.Types.GravityTypes.UniformGravity);
  Modelica.Mechanics.MultiBody.Joints.Revolute rev(n = {0,0,1}, phi(fixed = true));
  Modelica.Mechanics.MultiBody.Parts.Body body(m = 1.0, r_CM = {0.5,0,0});
equation
  connect(world.frame_b, rev.frame_a);
  connect(rev.frame_b, body.frame_a);
  annotation(experiment(StopTime = 5));
end DerivativeThroughAConnectedFrame;
