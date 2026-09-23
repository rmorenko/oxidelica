// Squeezed down from Modelica.Fluid.Examples.BranchingDynamicPipes.
//
// `ports[1].Xi = {0.5}` where each port carries `Xi[1]` is how every
// source of a moist medium writes its trace fractions. The subscript
// left one name, and its member was read as a scalar although the size
// table held `ports[1].Xi` as a vector of one: refused as `an equation
// between shapes [] and [1]`.
model AMemberOfOneElementOfAnArrayOfComponents
  connector Port
    Real Xi[1];
  end Port;
  Port ports[2];
equation
  ports[1].Xi = {0.5};
  ports[2].Xi = {0.25};
  annotation(experiment(StopTime = 0.01));
end AMemberOfOneElementOfAnArrayOfComponents;
