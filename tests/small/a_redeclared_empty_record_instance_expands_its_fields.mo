// A record kept empty in the base and redeclared whole by the medium,
// instantiated as a component whose fields an equation reads. The
// medium's `ThermodynamicState` is this shape: the interface declares
// it blank, the medium states its four fields with `redeclare record
// extends`, and `BaseProperties` holds a `state` of it whose `state.h`
// the base's equations read.
//
// Resolving the `state` component's type lands on the interface, where
// the record is empty, so the instance came out with no fields and
// `state.h` named a variable nothing declared. The fix instantiates the
// record under the name it was reached by - the medium - where the
// redeclared record with its fields is found (`record_asked_under`, the
// same the record-building layer already used). Without it this model
// is refused `unknown variable`; with it `state.h` is a component and
// the enthalpy flows.
package P
  partial package Base
    replaceable record State end State;
    replaceable partial model BaseProperties
      Real h;
      State state;
    equation
      h = state.h;
    end BaseProperties;
  end Base;
  package Air
    extends Base;
    redeclare record extends State Real h; end State;
    redeclare model extends BaseProperties
    end BaseProperties;
  end Air;
  model Use
    replaceable package Medium = Base;
    Medium.BaseProperties medium(h = 1000);
    Real z(start = 0, fixed = true);
  equation
    der(z) = medium.h - z;
    annotation(experiment(StopTime = 1));
  end Use;
  model M Use u(redeclare package Medium = Air); end M;
end P;
