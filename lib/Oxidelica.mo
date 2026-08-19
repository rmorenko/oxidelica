package Oxidelica "A standard library laid out like the Modelica Standard Library"

  package Constants "Mathematical and physical constants"
    constant Real pi = 3.141592653589793;
    constant Real e = 2.718281828459045;
    constant Real g_n = 9.80665 "standard acceleration of gravity";
  end Constants;

  package Units "Named types carrying a physical quantity"
    type Voltage = Real(unit = "V");
    type Current = Real(unit = "A");
    type Resistance = Real(unit = "Ohm");
    type Capacitance = Real(unit = "F");
    type Inductance = Real(unit = "H");
    type Angle = Real(unit = "rad");
    type AngularVelocity = Real(unit = "rad/s");
    type Torque = Real(unit = "N.m");
    type Inertia = Real(unit = "kg.m2");
  end Units;

  package Types "Enumerations the library uses"
    type WaveformKind = enumeration(
      Sine "sine wave",
      Square "square wave between plus and minus the amplitude",
      Triangle "triangular wave") "Shape of a periodic signal";
  end Types;

  model World "Settings shared by every component of a model"
    // Declared `inner` once at the top of a model; components reach it
    // with `outer World world`, the way the Modelica Standard Library
    // shares a `world` in MultiBody and a `system` in Fluid.
    parameter Real g = Constants.g_n "gravity acceleration";
    parameter Real g_x = 0 "direction of gravity, x component";
    parameter Real g_y = -1 "direction of gravity, y component";
  end World;

  package Blocks "Input/output blocks"

    package Interfaces "Partial blocks that fix the signal interface"
      partial model SO "Block with one output signal"
        output Real y "output signal";
      end SO;

      partial model SISO "Block with one input and one output signal"
        input Real u "input signal";
        output Real y "output signal";
      end SISO;

      partial model SI2SO "Block with two inputs and one output signal"
        input Real u1 "first input signal";
        input Real u2 "second input signal";
        output Real y "output signal";
      end SI2SO;
    end Interfaces;

    package Sources "Signal sources"
      model Constant "Constant signal"
        extends Interfaces.SO;
        parameter Real k = 1 "value of the output";
      equation
        y = k;
      end Constant;

      model Step "Step at a given time"
        extends Interfaces.SO;
        parameter Real height = 1;
        parameter Real offset = 0;
        parameter Real startTime = 0;
      equation
        y = offset + (if time < startTime then 0 else height);
      end Step;

      model Ramp "Ramp with a finite duration"
        extends Interfaces.SO;
        parameter Real height = 1;
        parameter Real duration = 1;
        parameter Real offset = 0;
        parameter Real startTime = 0;
      equation
        y = offset + (if time < startTime then 0 elseif time < startTime + duration
          then height * (time - startTime) / duration else height);
      end Ramp;

      model Sine "Sine signal"
        extends Interfaces.SO;
        parameter Real amplitude = 1;
        parameter Real f = 1 "frequency in Hz";
        parameter Real phase = 0;
        parameter Real offset = 0;
      equation
        y = offset + amplitude * sin(2 * Constants.pi * f * time + phase);
      end Sine;

      model Waveform "Periodic signal whose shape an enumeration selects"
        extends Interfaces.SO;
        parameter Types.WaveformKind kind = Types.WaveformKind.Sine "shape of the signal";
        parameter Real amplitude = 1;
        parameter Real f = 1 "frequency in Hz";
        parameter Real offset = 0;
        Real theta "phase angle in radians";
        Real shape "the shape, between -1 and 1";
      equation
        theta = 2 * Constants.pi * f * time;
        // The square wave switches on a relation, so the solver stops at
        // the jump; the triangle is asin of a sine, which has no jumps.
        shape = if kind == Types.WaveformKind.Sine then sin(theta)
          elseif kind == Types.WaveformKind.Square then (if sin(theta) >= 0 then 1 else -1)
          else 2 * asin(sin(theta)) / Constants.pi;
        y = offset + amplitude * shape;
      end Waveform;
    end Sources;

    package Math "Algebraic blocks"
      model Gain "Output is the input times a factor"
        extends Interfaces.SISO;
        parameter Real k = 1;
      equation
        y = k * u;
      end Gain;

      model Feedback "Difference of two signals"
        extends Interfaces.SI2SO;
      equation
        y = u1 - u2;
      end Feedback;
    end Math;

    package Continuous "Blocks with state"
      model Integrator "Integrates its input"
        // The initial value reaches the inherited output through a
        // modifier on its `start` attribute.
        extends Interfaces.SISO(y(start = y_start));
        parameter Real k = 1 "gain";
        parameter Real y_start = 0;
      equation
        der(y) = k * u;
      end Integrator;

      model FirstOrder "First-order lag"
        extends Interfaces.SISO(y(start = y_start));
        parameter Real k = 1 "gain";
        parameter Real T = 1 "time constant";
        parameter Real y_start = 0;
      equation
        der(y) = (k * u - y) / T;
      end FirstOrder;

      model PI "Proportional-integral controller"
        extends Interfaces.SISO;
        parameter Real k = 1 "proportional gain";
        parameter Real T = 1 "integral time constant";
        Real x(start = 0) "integrator state";
      equation
        der(x) = u / T;
        y = k * (u + x);
      end PI;
    end Continuous;

    package Discrete "Blocks that run on a clock"
      // Every output here is assigned inside a `when`, which makes it a
      // discrete variable: it holds its value between ticks, so the
      // continuous part downstream sees a staircase.

      partial model SampledSISO "Block clocked by a sample period"
        extends Interfaces.SISO;
        parameter Real samplePeriod = 0.1 "time between two ticks";
        parameter Real startTime = 0 "instant of the first tick";
      end SampledSISO;

      model Sampler "Samples its input and holds it until the next tick"
        extends SampledSISO;
      equation
        when sample(startTime, samplePeriod) then
          y = u;
        end when;
      end Sampler;

      model UnitDelay "Output is the input of the previous tick"
        extends SampledSISO(y(start = y_start));
        parameter Real y_start = 0 "output before the first tick";
        Real held(start = y_start) "value carried between ticks";
      equation
        when sample(startTime, samplePeriod) then
          y = pre(held);
          held = u;
        end when;
      end UnitDelay;

      model PI "Proportional-integral controller on a clock"
        extends SampledSISO;
        parameter Real k = 1 "proportional gain";
        parameter Real Ti = 1 "integral time";
        Real integral(start = 0) "integrator state, updated at every tick";
      equation
        when sample(startTime, samplePeriod) then
          integral = pre(integral) + samplePeriod * u / Ti;
          y = k * (u + integral);
        end when;
      end PI;
    end Discrete;
  end Blocks;

  package Electrical "Electrical components"
    package Analog "Analog electrical components"

      package Interfaces "Connectors and partial components"
        connector Pin "Electrical pin"
          Units.Voltage v "potential at the pin";
          flow Units.Current i "current flowing into the pin";
        end Pin;

        partial model OnePort "Component with two pins and one current path"
          Pin p "positive pin";
          Pin n "negative pin";
          Units.Voltage v "voltage drop, p minus n";
          Units.Current i "current from p to n";
        equation
          v = p.v - n.v;
          p.i = i;
          n.i = -i;
        end OnePort;
      end Interfaces;

      package Basic "Basic components"
        model Ground "Reference potential"
          Interfaces.Pin p;
        equation
          p.v = 0;
          annotation(Icon(coordinateSystem(extent = {{-100, -100}, {100, 100}}),
            graphics = {Line(points = {{0, 90}, {0, 0}}), Line(points = {{-60, 0}, {60, 0}}), Line(points = {{-40, -20}, {40, -20}}), Line(points = {{-20, -40}, {20, -40}})}));
        end Ground;

        model Resistor "Ideal resistor"
          extends Interfaces.OnePort;
          parameter Units.Resistance R = 1;
        equation
          v = R * i;
          annotation(Icon(coordinateSystem(extent = {{-100, -100}, {100, 100}}),
            graphics = {Line(points = {{-90, 0}, {-70, 0}}), Rectangle(extent = {{-70, -25}, {70, 25}}), Line(points = {{70, 0}, {90, 0}})}));
        end Resistor;

        model Capacitor "Ideal capacitor"
          extends Interfaces.OnePort;
          parameter Units.Capacitance C = 1;
        equation
          der(v) = i / C;
          annotation(Icon(coordinateSystem(extent = {{-100, -100}, {100, 100}}),
            graphics = {Line(points = {{-90, 0}, {-10, 0}}), Line(points = {{-10, -40}, {-10, 40}}), Line(points = {{10, -40}, {10, 40}}), Line(points = {{10, 0}, {90, 0}})}));
        end Capacitor;

        model Inductor "Ideal inductor"
          extends Interfaces.OnePort;
          parameter Units.Inductance L = 1;
        equation
          der(i) = v / L;
          annotation(Icon(coordinateSystem(extent = {{-100, -100}, {100, 100}}),
            graphics = {Line(points = {{-90, 0}, {-60, 0}}), Ellipse(extent = {{-60, -20}, {-20, 20}}), Ellipse(extent = {{-20, -20}, {20, 20}}), Ellipse(extent = {{20, -20}, {60, 20}}), Line(points = {{60, 0}, {90, 0}})}));
        end Inductor;

        model EMF "Electromotive force: the electro-mechanical coupling"
          parameter Real k = 1 "transformation coefficient";
          Interfaces.Pin p;
          Interfaces.Pin n;
          Mechanics.Rotational.Interfaces.Flange flange;
          Units.Voltage v;
          Units.Current i;
          Units.AngularVelocity w;
        equation
          v = p.v - n.v;
          p.i = i;
          n.i = -i;
          w = der(flange.phi);
          v = k * w;
          flange.tau = -k * i;
          annotation(Icon(coordinateSystem(extent = {{-100, -100}, {100, 100}}),
            graphics = {Line(points = {{-90, 0}, {-40, 0}}), Ellipse(extent = {{-40, -40}, {40, 40}}), Line(points = {{40, 0}, {90, 0}}), Line(points = {{-30, 0}, {30, 0}})}));
        end EMF;
      end Basic;

      package Sources "Voltage sources"
        model ConstantVoltage "Source of constant voltage"
          extends Interfaces.OnePort;
          parameter Units.Voltage V = 1;
        equation
          v = V;
        end ConstantVoltage;

        model SineVoltage "Sinusoidal voltage source"
          extends Interfaces.OnePort;
          parameter Units.Voltage V = 1 "amplitude";
          parameter Real f = 1 "frequency in Hz";
        equation
          v = V * sin(2 * Constants.pi * f * time);
        end SineVoltage;

        model StepVoltage "Voltage that steps at a given time"
          extends Interfaces.OnePort;
          parameter Units.Voltage V = 1;
          parameter Real startTime = 0;
        equation
          v = if time < startTime then 0 else V;
        end StepVoltage;
      end Sources;
    end Analog;
  end Electrical;

  package Visualizers "Shapes the 3D view draws"
    model Shape "A body drawn in the 3D scene"
      // The 3D view finds these components by class and reads the
      // variables below at every output point, so a model says what to
      // draw instead of the viewer guessing from variable names.
      parameter Integer kind = 0 "0 = box, 1 = sphere, 2 = cylinder";
      parameter Real length = 1.0 "extent along the shape axis";
      parameter Real width = 0.1 "extent across the axis";
      parameter Real height = 0.1 "extent along z";
      parameter Real red = 0.21 "colour, 0..1";
      parameter Real green = 0.45;
      parameter Real blue = 0.94;
      Real x "origin x";
      Real y "origin y";
      Real z "origin z";
      Real phi "rotation about the z axis, measured from +x";
    end Shape;
  end Visualizers;

  package Mechanics "Mechanical components"
    package Rotational "One-dimensional rotational mechanics"

      package Interfaces "Connectors and partial components"
        connector Flange "One-dimensional rotational flange"
          Units.Angle phi "absolute rotation angle";
          flow Units.Torque tau "cut torque";
        end Flange;

        partial model TwoFlanges "Component with two flanges"
          Flange flange_a;
          Flange flange_b;
        end TwoFlanges;

        partial model OneFlangeAndSupport "Component with one flange and an optional support"
          // The standard library's way of letting a component either
          // react against the housing or expose that reaction: with
          // `useSupport = false` the support flange does not exist at
          // all, and the equation that would have described it is
          // replaced by one holding the support angle at zero.
          parameter Boolean useSupport = false "true: the support flange is exposed";
          Flange flange "flange of the shaft";
          Flange support(phi = phi_support, tau = -flange.tau) if useSupport
            "support flange, present only when used";
        protected
          Units.Angle phi_support "angle of the support, zero while it is not exposed";
        equation
          if not useSupport then
            phi_support = 0;
          end if;
        end OneFlangeAndSupport;
      end Interfaces;

      package Components "Rotational components"
        model Fixed "Flange held at a fixed angle"
          parameter Units.Angle phi0 = 0;
          Interfaces.Flange flange;
        equation
          flange.phi = phi0;
          annotation(Icon(coordinateSystem(extent = {{-100, -100}, {100, 100}}),
            graphics = {Line(points = {{0, -30}, {0, 30}}), Line(points = {{-30, 30}, {30, 30}}), Line(points = {{0, 30}, {0, 90}})}));
        end Fixed;

        model Inertia "Rotating body with inertia"
          extends Interfaces.TwoFlanges;
          parameter Units.Inertia J = 1;
          parameter Units.Angle phi_start = 0;
          parameter Units.AngularVelocity w_start = 0;
          Units.Angle phi(start = phi_start);
          Units.AngularVelocity w(start = w_start);
          Real a "angular acceleration";
        equation
          phi = flange_a.phi;
          phi = flange_b.phi;
          der(phi) = w;
          der(w) = a;
          J * a = flange_a.tau + flange_b.tau;
          annotation(Icon(coordinateSystem(extent = {{-100, -100}, {100, 100}}),
            graphics = {Line(points = {{-90, 0}, {-50, 0}}), Rectangle(extent = {{-50, -50}, {50, 50}}), Line(points = {{50, 0}, {90, 0}})}));
        end Inertia;

        model Spring "Linear rotational spring"
          extends Interfaces.TwoFlanges;
          parameter Real c = 1 "spring constant";
          parameter Units.Angle phi_rel0 = 0 "unstretched angle";
          Units.Angle phi_rel;
        equation
          phi_rel = flange_b.phi - flange_a.phi;
          flange_b.tau = c * (phi_rel - phi_rel0);
          flange_a.tau + flange_b.tau = 0;
          annotation(Icon(coordinateSystem(extent = {{-100, -100}, {100, 100}}),
            graphics = {Line(points = {{-90, 0}, {-60, 0}, {-45, 30}, {-15, -30}, {15, 30}, {45, -30}, {60, 0}, {90, 0}})}));
        end Spring;

        model ViscousFriction "Viscous friction between a shaft and the housing"
          parameter Real d = 1 "damping constant";
          Interfaces.Flange flange;
          Units.AngularVelocity w "shaft speed";
        equation
          w = der(flange.phi);
          flange.tau = d * w;
        end ViscousFriction;

        model Damper "Linear rotational damper between two flanges"
          extends Interfaces.TwoFlanges;
          parameter Real d = 1 "damping constant";
          Units.Angle phi_rel(start = 0) "relative angle";
          Units.AngularVelocity w_rel "relative speed";
        equation
          phi_rel = flange_b.phi - flange_a.phi;
          der(phi_rel) = w_rel;
          flange_b.tau = d * w_rel;
          flange_a.tau + flange_b.tau = 0;
          annotation(Icon(coordinateSystem(extent = {{-100, -100}, {100, 100}}),
            graphics = {Line(points = {{-90, 0}, {-30, 0}}), Rectangle(extent = {{-30, -30}, {20, 30}}), Line(points = {{20, 0}, {90, 0}}), Line(points = {{-10, -30}, {-10, 30}})}));
        end Damper;
      end Components;

      package Sources "Torque sources"
        model ConstantTorque "Constant torque acting on a flange"
          parameter Units.Torque tau_constant = 1;
          Interfaces.Flange flange;
        equation
          flange.tau = -tau_constant;
        end ConstantTorque;

        model Torque "Constant torque, with the reaction on an optional support"
          extends Interfaces.OneFlangeAndSupport;
          parameter Units.Torque tau_constant = 1;
        equation
          flange.tau = -tau_constant;
        end Torque;
      end Sources;
    end Rotational;

    package Planar "Point masses moving in a plane"
      model PointMass "A mass pulled by the gravity of the world"
        outer World world "settings shared by the model, declared inner above";
        parameter Real m = 1 "mass";
        parameter Real x_start = 0;
        parameter Real y_start = 0;
        parameter Real vx_start = 0;
        parameter Real vy_start = 0;
        Real x(start = x_start) "position, x";
        Real y(start = y_start) "position, y";
        Real vx(start = vx_start) "velocity, x";
        Real vy(start = vy_start) "velocity, y";
        Real ax "acceleration, x";
        Real ay "acceleration, y";
        Real fx "applied force besides gravity, x";
        Real fy "applied force besides gravity, y";
      equation
        der(x) = vx;
        der(y) = vy;
        der(vx) = ax;
        der(vy) = ay;
        m * ax = fx + m * world.g * world.g_x;
        m * ay = fy + m * world.g * world.g_y;
      end PointMass;
    end Planar;
  end Mechanics;
end Oxidelica;
