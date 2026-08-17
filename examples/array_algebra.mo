model ArrayAlgebra "Ranges, slices, comprehensions and matrix algebra in one place"
  // Every line here used to be a documented gap: ranges as values,
  // vector and range subscripts with `end`, array comprehensions and
  // reductions over them, matrix literals in brackets, matrix products
  // and transpose, and the cross product.
  parameter Integer n = 4;
  parameter Real A[2, 2] = [1, 2; 3, 4];
  Real v[4];
  Real evens[2];
  Real last_two[2];
  Real squares[4];
  Real rotated[2];
  Real mm[2, 2];
  Real crossed[3];
  Real total;
equation
  v = 1:4;
  evens = v[{2, 4}];
  last_two = v[end - 1:end];
  squares = {i * i for i in 1:n};
  rotated = A * {1.0, 0.0};
  mm = A * transpose(A);
  crossed = cross({1, 0, 0}, {0, 1, 0});
  total = sum(i * 2 for i in 1:n);
  annotation(experiment(StopTime = 1, Interval = 0.5));
end ArrayAlgebra;
