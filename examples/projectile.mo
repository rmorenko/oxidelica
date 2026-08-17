model Projectile "A thrown ball that takes gravity from the world"
  import Oxidelica.Mechanics.Planar;
  import Oxidelica.Visualizers;
  import Oxidelica.World;

  // `inner` declares the instance once; the point mass reaches it with
  // `outer World world` and never mentions the path.
  inner World world(g = 9.81, g_x = 0, g_y = -1);

  parameter Real drag = 0.0 "linear drag coefficient, 0 gives the analytic parabola";

  Planar.PointMass ball(m = 0.145, x_start = 0, y_start = 0,
    vx_start = 12, vy_start = 16);

  Visualizers.Shape body(kind = 1, length = 1.2, width = 1.2, height = 1.2,
    red = 0.95, green = 0.61, blue = 0.16);
  Real height "height of the ball, for plotting";
equation
  // The only forces besides gravity: drag proportional to velocity.
  ball.fx = -drag * ball.vx;
  ball.fy = -drag * ball.vy;
  height = ball.y;
  body.x = ball.x;
  body.y = ball.y;
  body.z = 0;
  body.phi = 0;
  annotation(experiment(StopTime = 3.3, Interval = 0.005, Tolerance = 1e-9));
end Projectile;
