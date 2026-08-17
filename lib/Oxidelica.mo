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

  package Blocks "Input/output blocks"

    package Sources "Signal sources"
      model Constant "Constant signal"
        parameter Real k = 1 "value of the output";
        Real y "output";
      equation
        y = k;
      end Constant;

      model Step "Step at a given time"
        parameter Real height = 1;
        parameter Real offset = 0;
        parameter Real startTime = 0;
        Real y;
      equation
        y = offset + (if time < startTime then 0 else height);
      end Step;

      model Ramp "Ramp with a finite duration"
        parameter Real height = 1;
        parameter Real duration = 1;
        parameter Real offset = 0;
        parameter Real startTime = 0;
        Real y;
      equation
        y = offset + (if time < startTime then 0 elseif time < startTime + duration
          then height * (time - startTime) / duration else height);
      end Ramp;

      model Sine "Sine signal"
        parameter Real amplitude = 1;
        parameter Real f = 1 "frequency in Hz";
        parameter Real phase = 0;
        parameter Real offset = 0;
        Real y;
      equation
        y = offset + amplitude * sin(2 * Constants.pi * f * time + phase);
      end Sine;
    end Sources;

    package Math "Algebraic blocks"
      model Gain "Output is the input times a factor"
        parameter Real k = 1;
        Real u;
        Real y;
      equation
        y = k * u;
      end Gain;

      model Feedback "Difference of two signals"
        Real u1;
        Real u2;
        Real y;
      equation
        y = u1 - u2;
      end Feedback;
    end Math;

    package Continuous "Blocks with state"
      model Integrator "Integrates its input"
        parameter Real k = 1 "gain";
        parameter Real y_start = 0;
        Real u;
        Real y(start = y_start);
      equation
        der(y) = k * u;
      end Integrator;

      model FirstOrder "First-order lag"
        parameter Real k = 1 "gain";
        parameter Real T = 1 "time constant";
        parameter Real y_start = 0;
        Real u;
        Real y(start = y_start);
      equation
        der(y) = (k * u - y) / T;
      end FirstOrder;

      model PI "Proportional-integral controller"
        parameter Real k = 1 "proportional gain";
        parameter Real T = 1 "integral time constant";
        Real u;
        Real y;
        Real x(start = 0) "integrator state";
      equation
        der(x) = u / T;
        y = k * (u + x);
      end PI;
    end Continuous;
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
        end Ground;

        model Resistor "Ideal resistor"
          extends Interfaces.OnePort;
          parameter Units.Resistance R = 1;
        equation
          v = R * i;
        end Resistor;

        model Capacitor "Ideal capacitor"
          extends Interfaces.OnePort;
          parameter Units.Capacitance C = 1;
        equation
          der(v) = i / C;
        end Capacitor;

        model Inductor "Ideal inductor"
          extends Interfaces.OnePort;
          parameter Units.Inductance L = 1;
        equation
          der(i) = v / L;
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
      end Interfaces;

      package Components "Rotational components"
        model Fixed "Flange held at a fixed angle"
          parameter Units.Angle phi0 = 0;
          Interfaces.Flange flange;
        equation
          flange.phi = phi0;
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
          // Note: connecting this damper directly to `Fixed` makes the
          // relative angle redundant with the shaft angle, an index-2
          // system whose reduction needs derivative unknowns for
          // algebraic variables - not yet supported. Use
          // ViscousFriction for damping against the housing.
          extends Interfaces.TwoFlanges;
          parameter Real d = 1 "damping constant";
          Units.Angle phi_rel(start = 0) "relative angle";
          Units.AngularVelocity w_rel "relative speed";
        equation
          phi_rel = flange_b.phi - flange_a.phi;
          der(phi_rel) = w_rel;
          flange_b.tau = d * w_rel;
          flange_a.tau + flange_b.tau = 0;
        end Damper;
      end Components;

      package Sources "Torque sources"
        model ConstantTorque "Constant torque acting on a flange"
          parameter Units.Torque tau_constant = 1;
          Interfaces.Flange flange;
        equation
          flange.tau = -tau_constant;
        end ConstantTorque;
      end Sources;
    end Rotational;
  end Mechanics;
end Oxidelica;
