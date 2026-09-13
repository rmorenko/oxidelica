// A value handed down an `extends` may name an array of the class
// handing it, buried in arithmetic rather than standing alone: a pipe
// writes `extends PartialTwoPortFlow(final dheights = height_ab*dxs)`,
// where `dxs` is a declaration of the pipe and no name the base has
// heard of.
//
// A bare name was already taken apart one element apiece; a name
// inside a larger expression was not. With the fault present the whole
// array is spread over every element, so `dheights[1]` and
// `dheights[2]` are both bound to `height_ab` times the entire array -
// a value nothing can work out, and the refusal comes at the run:
//
//   cannot evaluate parameters [pipe1.dheights[1] = pipe1.height_ab *
//   pipe1.dxs, ...]: nothing gives a value to `pipe1.dxs`
//
// Fixed, `dheights[1]` is `height_ab*dxs[1]` and `dheights[2]` is
// `height_ab*dxs[2]`, both settle before the run, and the model runs.
model Base
  parameter Integer n = 2;
  parameter Real[n] dheights = zeros(n);
  Real y;
equation
  y = sum(dheights) + time;
end Base;

model Pipe
  extends Base(final dheights = height_ab*dxs);
  parameter Real height_ab = 1.0;
  final parameter Real[n] lengths = fill(3.0/n, n);
  final parameter Real[n] dxs = lengths/sum(lengths);
end Pipe;

model an_array_of_the_class_above_inside_a_handed_down_value
  Pipe pipe1;
end an_array_of_the_class_above_inside_a_handed_down_value;
