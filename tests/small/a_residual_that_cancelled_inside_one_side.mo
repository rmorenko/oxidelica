// A residual is a difference, and how large it has to be before it
// means anything is set by the numbers it was subtracted from. The
// convergence test asks that of the two sides of the equation, which
// is right where the cancellation is between them - and blind where a
// side cancels *within itself*.
//
// Here both sides carry `big`, which is 2^22, and the difference the
// block is chasing is 5e-10 - between 2^-31 and 2^-30 of it. Newton
// walks to within one ulp and can go no further: the arithmetic has
// no number in between. Read from the sides, the floor is 1e-12 times
// about 8.4e6, which is 8e-6 and would accept almost anything; but
// the sides here are the small difference itself, not the large
// numbers, so the floor computed from them is 4e-22 and the ulp
// stands a million times above it. Read from the loudest number the
// row met on the way - 4194305 - the floor is 4 eps times that, and
// what is left is under it.
//
// Without the loudness floor the refusal reads:
//   the Newton direction of algebraic loop ["x"] does not reduce the
//   residual at t = 0: from |f| = 4.313225746154785e-10
// which is `/tmp/m238/rect.txt`'s seventh row of
// `Modelica.Electrical.Analog.Examples.Rectifier` in eight lines.

model a_residual_that_cancelled_inside_one_side
  Real x(start = 1.0);
  parameter Real big = 4.194304e6 "2^22, so that an ulp of it is 2^-30";
equation
  (big + x^3) - (big + x) = 5e-10;
end a_residual_that_cancelled_inside_one_side;
