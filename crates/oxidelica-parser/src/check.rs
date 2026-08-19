//! Static semantics of a flat model: a type layer and a unit layer.
//!
//! Both layers are permissive by design. A variable with no declared
//! unit, a call this module does not recognise, or a unit written in a
//! symbol it does not know all become "no information", and no
//! information absorbs everything. An error is reported only when two
//! *declared* facts contradict each other - a Boolean added to a
//! number, volts equated to amperes - so a model that never names a
//! type beyond `Real` and never writes a unit sails through untouched.
//!
//! Numeric literals sit in between: `2 * x` keeps the unit of `x`, and
//! `x + 1` or `x = 5` never complain, mirroring how models are
//! actually written.

use crate::ast::{operator_name, BinOp, Component, Expr, Model, RelOp, WhenAction};
use crate::flatten::const_eval;
use std::collections::HashMap;

/// Check a flattened model. An `Err` names the first contradiction.
pub fn verify(model: &Model) -> Result<(), String> {
    let types = TypeLayer::of(model);
    let units = UnitLayer::of(model);
    let declared: HashMap<&str, &Component> = model
        .components
        .iter()
        .map(|component| (component.name.as_str(), component))
        .collect();
    for component in &model.components {
        types.declaration(component)?;
    }
    for equation in model.equations.iter().chain(&model.initial_equations) {
        types.equation(&equation.lhs, &equation.rhs)?;
        types.exact_equality(&equation.lhs)?;
        types.exact_equality(&equation.rhs)?;
        types.pinned_to_a_fraction(&equation.lhs, &equation.rhs)?;
        pinned_bounds(&declared, &equation.lhs, &equation.rhs)?;
        units.equation(&equation.lhs, &equation.rhs)?;
    }
    for clause in &model.when_clauses {
        for branch in &clause.branches {
            types.condition(&branch.condition)?;
            types.exact_equality(&branch.condition)?;
            units.infer(&branch.condition)?;
            for action in &branch.actions {
                match action {
                    WhenAction::Assign(name, value) => {
                        types.assignment(name, value)?;
                        units.assignment(name, value)?;
                    }
                    WhenAction::Reinit(name, value) => {
                        types.assignment(name, value)?;
                        units.assignment(name, value)?;
                    }
                    WhenAction::Terminate(_) => {}
                    // Flattening inlines the call and hands each
                    // target its own assignment, and unrolls a loop
                    // into one assignment per round, so by here there
                    // is neither left to check.
                    WhenAction::TupleAssign(..) | WhenAction::Loop(_) | WhenAction::Choice(_) => {}
                }
            }
        }
    }
    for (condition, _) in &model.asserts {
        types.condition(condition)?;
        units.infer(condition)?;
    }
    Ok(())
}

/// A binding arrives here as an equation, so a declaration whose value
/// is settled before the run - `Real level(min = 0) = -1` - can be
/// caught without starting one.
fn pinned_bounds(
    declared: &HashMap<&str, &Component>,
    lhs: &Expr,
    rhs: &Expr,
) -> Result<(), String> {
    let nothing = HashMap::new();
    for (side, other) in [(lhs, rhs), (rhs, lhs)] {
        let Expr::Ref(name) = side else { continue };
        let Some(component) = declared.get(name.as_str()) else {
            continue;
        };
        if let Some(value) = const_eval(other, &nothing) {
            bounds(component, value, "is")?;
        }
    }
    Ok(())
}

/// `min` and `max` against a value that is known before the run. Both
/// are declared facts, so a binding or a `start` outside them is the
/// same kind of contradiction this module reports everywhere else -
/// caught here rather than by the run-time assertion that also guards
/// them, because there is no need to start a run to find out.
fn bounds(component: &Component, value: f64, what: &str) -> Result<(), String> {
    let known = |bound: &Option<Expr>| -> Option<f64> {
        bound
            .as_ref()
            .and_then(|expr| const_eval(expr, &HashMap::new()))
    };
    if let Some(low) = known(&component.min) {
        if value < low {
            return Err(format!(
                "`{}` {what} {value}, below its min of {low}",
                component.name
            ));
        }
    }
    if let Some(high) = known(&component.max) {
        if value > high {
            return Err(format!(
                "`{}` {what} {value}, above its max of {high}",
                component.name
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- types

/// What an expression is, as far as the declarations say.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
    /// `Boolean` - never mixes with the numbers.
    Bool,
    /// `Integer`, or a literal that happens to be whole.
    Int,
    /// `Real`.
    Real,
    /// No information; compatible with everything.
    Unknown,
}

impl Ty {
    fn name(self) -> &'static str {
        match self {
            Ty::Bool => "Boolean",
            Ty::Int => "Integer",
            Ty::Real => "Real",
            Ty::Unknown => "unknown",
        }
    }
}

/// The Boolean/Integer/Real layer over a flat model.
struct TypeLayer {
    vars: HashMap<String, Ty>,
}

impl TypeLayer {
    fn of(model: &Model) -> TypeLayer {
        let vars = model
            .components
            .iter()
            .map(|component| {
                let ty = match component.type_name.as_str() {
                    "Boolean" => Ty::Bool,
                    "Integer" => Ty::Int,
                    "Real" => Ty::Real,
                    _ => Ty::Unknown,
                };
                (component.name.clone(), ty)
            })
            .collect();
        TypeLayer { vars }
    }

    /// The two sides of an equation must not put a Boolean against a
    /// number. `Integer` against `Real` is fine - integers promote, so
    /// `2 * i = x` is an ordinary equation, and which side an equation
    /// determines is the matcher's business rather than something to
    /// read off the text. Where the direction *is* known - a
    /// declaration, an assignment - see [`TypeLayer::declaration`].
    fn equation(&self, lhs: &Expr, rhs: &Expr) -> Result<(), String> {
        let (left, right) = (self.infer(lhs)?, self.infer(rhs)?);
        if bool_against_number(left, right) {
            return Err(format!(
                "type mismatch in `{} = {}`: {} against {}",
                describe(lhs),
                describe(rhs),
                left.name(),
                right.name()
            ));
        }
        Ok(())
    }

    /// An Integer pinned to a fraction. A declaration with a binding
    /// arrives here as an equation, and which side an equation
    /// determines is usually the matcher's business - but not when the
    /// other side works out to a constant on its own: `Integer i = 1.5`
    /// says the whole number is 1.5, and no matching can rescue that.
    ///
    /// The value is folded rather than merely typed, so `Integer i = 3.0`
    /// still passes. That is deliberate, and of a piece with the rest of
    /// this module: a complaint needs a real contradiction, not a
    /// literal that happens to be spelled with a point.
    fn pinned_to_a_fraction(&self, lhs: &Expr, rhs: &Expr) -> Result<(), String> {
        let nothing = HashMap::new();
        for (side, other) in [(lhs, rhs), (rhs, lhs)] {
            let Expr::Ref(name) = side else { continue };
            if self.vars.get(name) != Some(&Ty::Int) {
                continue;
            }
            if let Some(value) = const_eval(other, &nothing) {
                if value.fract() != 0.0 {
                    return Err(format!(
                        "`{name}` is an Integer but `{}` works out to {value}; use `integer()`",
                        describe(other)
                    ));
                }
            }
        }
        Ok(())
    }

    /// A `start` attribute is a value for its variable just as much as a
    /// binding is, and unlike a binding it survives flattening.
    fn declaration(&self, component: &Component) -> Result<(), String> {
        let Some(start) = &component.start else {
            return Ok(());
        };
        let Some(value) = const_eval(start, &HashMap::new()) else {
            return Ok(());
        };
        if self.vars.get(&component.name) == Some(&Ty::Int) && value.fract() != 0.0 {
            return Err(format!(
                "`{}` is an Integer but starts at {value}; use `integer()`",
                component.name
            ));
        }
        bounds(component, value, "starts at")
    }

    /// `:=` is directional: a Real value cannot land in an Integer.
    fn assignment(&self, name: &str, value: &Expr) -> Result<(), String> {
        let target = self.vars.get(name).copied().unwrap_or(Ty::Unknown);
        let given = self.infer(value)?;
        if bool_against_number(target, given) {
            return Err(format!(
                "type mismatch in `{name} := {}`: {} against {}",
                describe(value),
                target.name(),
                given.name()
            ));
        }
        if target == Ty::Int && given == Ty::Real {
            return Err(format!(
                "`{name}` is an Integer but `{}` is Real; use `integer()`",
                describe(value)
            ));
        }
        Ok(())
    }

    /// Two Reals may not be compared for equality. The specification
    /// says so outright, and the reason is worth keeping in mind: a
    /// Real is the result of a step, and asking a stepped quantity to
    /// land on a value exactly is asking the arithmetic for something
    /// it cannot promise. `abs(a - b) < eps` is the comparison that
    /// means something.
    fn exact_equality(&self, expr: &Expr) -> Result<(), String> {
        if let Expr::Rel(op @ (RelOp::Eq | RelOp::Ne), a, b) = expr {
            if self.infer(a)? == Ty::Real || self.infer(b)? == Ty::Real {
                return Err(format!(
                    "`{} {} {}` compares Reals for equality; \
                     compare the difference with a tolerance instead",
                    describe(a),
                    if matches!(op, RelOp::Eq) { "==" } else { "<>" },
                    describe(b)
                ));
            }
        }
        for child in children(expr) {
            self.exact_equality(child)?;
        }
        Ok(())
    }

    /// `when`, `assert` and `if` conditions must be Boolean.
    fn condition(&self, expr: &Expr) -> Result<(), String> {
        match self.infer(expr)? {
            Ty::Bool | Ty::Unknown => Ok(()),
            other => Err(format!(
                "the condition `{}` is {}, not Boolean",
                describe(expr),
                other.name()
            )),
        }
    }

    fn infer(&self, expr: &Expr) -> Result<Ty, String> {
        match expr {
            Expr::Number(n) => Ok(if n.fract() == 0.0 { Ty::Int } else { Ty::Real }),
            Expr::Bool(_) => Ok(Ty::Bool),
            Expr::Ref(name) => Ok(self.vars.get(name).copied().unwrap_or(Ty::Unknown)),
            Expr::Time => Ok(Ty::Real),
            Expr::Neg(inner) => {
                let ty = self.infer(inner)?;
                self.numeric(ty, inner, "unary minus")?;
                Ok(ty)
            }
            Expr::Bin(op, a, b) | Expr::Elementwise(op, a, b) => {
                let (ta, tb) = (self.infer(a)?, self.infer(b)?);
                self.numeric(ta, a, op_text(*op))?;
                self.numeric(tb, b, op_text(*op))?;
                Ok(match op {
                    // Division and powers are Real by definition.
                    BinOp::Div | BinOp::Pow => Ty::Real,
                    _ => join_numbers(ta, tb),
                })
            }
            Expr::Rel(_, a, b) => {
                let (ta, tb) = (self.infer(a)?, self.infer(b)?);
                if bool_against_number(ta, tb) {
                    return Err(format!(
                        "cannot compare `{}` ({}) with `{}` ({})",
                        describe(a),
                        ta.name(),
                        describe(b),
                        tb.name()
                    ));
                }
                Ok(Ty::Bool)
            }
            Expr::And(a, b) | Expr::Or(a, b) => {
                self.boolean(a)?;
                self.boolean(b)?;
                Ok(Ty::Bool)
            }
            Expr::Not(inner) => {
                self.boolean(inner)?;
                Ok(Ty::Bool)
            }
            Expr::If(condition, then, otherwise) => {
                self.condition(condition)?;
                let (yes, no) = (self.infer(then)?, self.infer(otherwise)?);
                if bool_against_number(yes, no) {
                    return Err(format!(
                        "the branches of `if {}` disagree: `{}` is {}, `{}` is {}",
                        describe(condition),
                        describe(then),
                        yes.name(),
                        describe(otherwise),
                        no.name()
                    ));
                }
                Ok(join(yes, no))
            }
            Expr::Call(name, args) => self.call(name, args),
            // Array forms are resolved away by flattening; anything
            // that slips through is left unjudged, but its pieces are
            // still visited.
            _ => {
                for child in children(expr) {
                    self.infer(child)?;
                }
                Ok(Ty::Unknown)
            }
        }
    }

    fn call(&self, name: &str, args: &[Expr]) -> Result<Ty, String> {
        let one = |layer: &TypeLayer| -> Result<Ty, String> {
            args.first().map_or(Ok(Ty::Unknown), |a| layer.infer(a))
        };
        match operator_name(name) {
            // `Integer(e)` is the ordinal of an enumeration value.
            "Integer" => Ok(Ty::Int),
            "der" | "sqrt" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh"
            | "tanh" | "exp" | "log" | "log10" | "atan2" | "floor" | "ceil" => {
                for arg in args {
                    let ty = self.infer(arg)?;
                    self.numeric(ty, arg, name)?;
                }
                Ok(Ty::Real)
            }
            "integer" => {
                let ty = one(self)?;
                if let Some(arg) = args.first() {
                    self.numeric(ty, arg, name)?;
                }
                Ok(Ty::Int)
            }
            "div" | "mod" | "rem" => {
                let mut tys = Vec::new();
                for arg in args {
                    let ty = self.infer(arg)?;
                    self.numeric(ty, arg, name)?;
                    tys.push(ty);
                }
                Ok(tys.into_iter().fold(Ty::Int, join_numbers))
            }
            "abs" => {
                let ty = one(self)?;
                if let Some(arg) = args.first() {
                    self.numeric(ty, arg, name)?;
                }
                Ok(ty)
            }
            "pre" | "noEvent" => one(self),
            "smooth" => args.get(1).map_or(Ok(Ty::Unknown), |a| self.infer(a)),
            "min" | "max" => {
                let mut result = Ty::Unknown;
                for arg in args {
                    let ty = self.infer(arg)?;
                    if bool_against_number(result, ty) {
                        return Err(format!("`{name}` mixes Boolean and numeric arguments"));
                    }
                    result = join(result, ty);
                }
                Ok(result)
            }
            "edge" | "change" | "initial" | "sample" => {
                for arg in args {
                    self.infer(arg)?;
                }
                Ok(Ty::Bool)
            }
            _ => {
                for arg in args {
                    self.infer(arg)?;
                }
                Ok(Ty::Unknown)
            }
        }
    }

    fn numeric(&self, ty: Ty, expr: &Expr, operation: &str) -> Result<(), String> {
        if ty == Ty::Bool {
            return Err(format!(
                "`{operation}` needs a number, but `{}` is Boolean",
                describe(expr)
            ));
        }
        Ok(())
    }

    fn boolean(&self, expr: &Expr) -> Result<(), String> {
        match self.infer(expr)? {
            Ty::Bool | Ty::Unknown => Ok(()),
            other => Err(format!(
                "`{}` is {}, but a Boolean is needed here",
                describe(expr),
                other.name()
            )),
        }
    }
}

/// A Boolean on one side and a number on the other.
fn bool_against_number(a: Ty, b: Ty) -> bool {
    matches!(
        (a, b),
        (Ty::Bool, Ty::Int | Ty::Real) | (Ty::Int | Ty::Real, Ty::Bool)
    )
}

/// The type both sides of a branch or comparison share.
fn join(a: Ty, b: Ty) -> Ty {
    match (a, b) {
        (Ty::Unknown, other) | (other, Ty::Unknown) => other,
        (Ty::Bool, _) => Ty::Bool,
        _ => join_numbers(a, b),
    }
}

/// Integer only when everything is; `Real` wins otherwise.
fn join_numbers(a: Ty, b: Ty) -> Ty {
    match (a, b) {
        (Ty::Int, Ty::Int) => Ty::Int,
        (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
        _ => Ty::Real,
    }
}

// ---------------------------------------------------------------- units

/// How many base dimensions are tracked: m, kg, s, A, K, mol, cd.
/// Angles (`rad`, `sr`, `deg`) count as dimensionless, which is what
/// lets `tau = J * der(w)` hold with torque in N.m.
const BASES: usize = 7;
const BASE_NAMES: [&str; BASES] = ["m", "kg", "s", "A", "K", "mol", "cd"];

/// Exponents over the base dimensions; scale factors are irrelevant
/// for consistency, so `g` and `kg` are the same dimension.
///
/// The exponents are rational, not whole: the grammar writes `m(1/2)`,
/// and the square root of an odd power lands there too. One shared
/// denominator across the bases is enough, and keeping it reduced makes
/// two equal dimensions equal field by field, which is what the derived
/// `Eq` needs.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Dim {
    powers: [i32; BASES],
    denominator: i32,
}

impl Dim {
    const ONE: Dim = Dim {
        powers: [0; BASES],
        denominator: 1,
    };

    /// Whole-number exponents, the common case: base and derived units.
    fn of(powers: [i32; BASES]) -> Dim {
        Dim {
            powers,
            denominator: 1,
        }
    }

    /// Reduce to the canonical form: a positive denominator, and no
    /// factor shared by it and every power.
    fn reduced(mut powers: [i32; BASES], mut denominator: i32) -> Dim {
        if denominator < 0 {
            denominator = -denominator;
            powers = powers.map(|power| -power);
        }
        let mut divisor = denominator;
        for &power in &powers {
            divisor = gcd(divisor, power);
        }
        if divisor == 0 {
            divisor = 1;
        }
        Dim {
            powers: powers.map(|power| power / divisor),
            denominator: denominator / divisor,
        }
    }

    /// Multiply every exponent by the rational `n / d`: a power, a root,
    /// or the sign of a division.
    fn scaled(self, n: i32, d: i32) -> Dim {
        Dim::reduced(self.powers.map(|power| power * n), self.denominator * d)
    }

    /// `self` times `other^sign`, over a common denominator.
    fn combine(self, other: Dim, sign: i32) -> Dim {
        let denominator = lcm(self.denominator, other.denominator);
        let mine = denominator / self.denominator;
        let theirs = denominator / other.denominator;
        let mut powers = [0; BASES];
        for (index, slot) in powers.iter_mut().enumerate() {
            *slot = self.powers[index] * mine + sign * other.powers[index] * theirs;
        }
        Dim::reduced(powers, denominator)
    }

    /// `m2.kg.s-3.A-1`, or `m(1/2)` where an exponent is not whole - the
    /// canonical spelling for error messages.
    fn text(self) -> String {
        let parts: Vec<String> = self
            .powers
            .iter()
            .zip(BASE_NAMES)
            .filter(|(&power, _)| power != 0)
            .map(|(&power, name)| {
                let divisor = gcd(power, self.denominator);
                let (whole, over) = (power / divisor, self.denominator / divisor);
                match (whole, over) {
                    (1, 1) => name.to_string(),
                    (_, 1) => format!("{name}{whole}"),
                    _ => format!("{name}({whole}/{over})"),
                }
            })
            .collect();
        if parts.is_empty() {
            "1".to_string()
        } else {
            parts.join(".")
        }
    }
}

/// Greatest common divisor, for keeping a dimension reduced.
fn gcd(a: i32, b: i32) -> i32 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Least common multiple of two denominators, both positive.
fn lcm(a: i32, b: i32) -> i32 {
    if a == 0 || b == 0 {
        1
    } else {
        a / gcd(a, b) * b
    }
}

/// What is known about the unit of an expression.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Units {
    /// No information; absorbs everything.
    Any,
    /// A bare number or a dimensionless result: bends to the other
    /// side of a sum or an equation, dimensionless in a product.
    Weak,
    /// Known dimensions.
    Of(Dim),
}

impl Units {
    fn text(self) -> String {
        match self {
            Units::Any => "unknown".to_string(),
            Units::Weak => "1".to_string(),
            Units::Of(dim) => dim.text(),
        }
    }
}

/// The dimensional layer over a flat model.
struct UnitLayer {
    vars: HashMap<String, Units>,
}

impl UnitLayer {
    fn of(model: &Model) -> UnitLayer {
        let vars = model
            .components
            .iter()
            .map(|component| {
                let units = component
                    .unit
                    .as_deref()
                    .and_then(parse_unit)
                    .map_or(Units::Any, Units::Of);
                (component.name.clone(), units)
            })
            .collect();
        UnitLayer { vars }
    }

    fn equation(&self, lhs: &Expr, rhs: &Expr) -> Result<(), String> {
        let (left, right) = (self.infer(lhs)?, self.infer(rhs)?);
        if clashes(left, right) {
            return Err(format!(
                "unit mismatch in `{} = {}`: {} against {}",
                describe(lhs),
                describe(rhs),
                left.text(),
                right.text()
            ));
        }
        Ok(())
    }

    fn assignment(&self, name: &str, value: &Expr) -> Result<(), String> {
        let target = self.vars.get(name).copied().unwrap_or(Units::Any);
        let given = self.infer(value)?;
        if clashes(target, given) {
            return Err(format!(
                "unit mismatch in `{name} := {}`: {} against {}",
                describe(value),
                target.text(),
                given.text()
            ));
        }
        Ok(())
    }

    fn infer(&self, expr: &Expr) -> Result<Units, String> {
        match expr {
            Expr::Number(_) | Expr::Bool(_) => Ok(Units::Weak),
            Expr::Ref(name) => Ok(self.vars.get(name).copied().unwrap_or(Units::Any)),
            // `time` is in seconds, but treating it so would flag the
            // ubiquitous `sin(time)` and `x = v * time` in models that
            // scale it with unitless factors. It absorbs instead.
            Expr::Time => Ok(Units::Any),
            Expr::Neg(inner) => self.infer(inner),
            Expr::Bin(op, a, b) | Expr::Elementwise(op, a, b) => {
                let (ua, ub) = (self.infer(a)?, self.infer(b)?);
                match op {
                    BinOp::Add | BinOp::Sub => {
                        if clashes(ua, ub) {
                            return Err(format!(
                                "cannot {} `{}` ({}) and `{}` ({})",
                                if *op == BinOp::Add { "add" } else { "subtract" },
                                describe(a),
                                ua.text(),
                                describe(b),
                                ub.text()
                            ));
                        }
                        Ok(merge(ua, ub))
                    }
                    BinOp::Mul => Ok(product(ua, ub, 1)),
                    BinOp::Div => Ok(product(ua, ub, -1)),
                    BinOp::Pow => self.power(ua, b),
                }
            }
            Expr::Rel(_, a, b) => {
                let (ua, ub) = (self.infer(a)?, self.infer(b)?);
                if clashes(ua, ub) {
                    return Err(format!(
                        "cannot compare `{}` ({}) with `{}` ({})",
                        describe(a),
                        ua.text(),
                        describe(b),
                        ub.text()
                    ));
                }
                Ok(Units::Weak)
            }
            Expr::And(a, b) | Expr::Or(a, b) => {
                self.infer(a)?;
                self.infer(b)?;
                Ok(Units::Weak)
            }
            Expr::Not(inner) => {
                self.infer(inner)?;
                Ok(Units::Weak)
            }
            Expr::If(condition, then, otherwise) => {
                self.infer(condition)?;
                let (yes, no) = (self.infer(then)?, self.infer(otherwise)?);
                if clashes(yes, no) {
                    return Err(format!(
                        "the branches of `if {}` disagree: `{}` is {}, `{}` is {}",
                        describe(condition),
                        describe(then),
                        yes.text(),
                        describe(otherwise),
                        no.text()
                    ));
                }
                Ok(merge(yes, no))
            }
            Expr::Call(name, args) => self.call(name, args),
            _ => {
                for child in children(expr) {
                    self.infer(child)?;
                }
                Ok(Units::Any)
            }
        }
    }

    fn call(&self, name: &str, args: &[Expr]) -> Result<Units, String> {
        let one = |layer: &UnitLayer| -> Result<Units, String> {
            args.first().map_or(Ok(Units::Any), |a| layer.infer(a))
        };
        match operator_name(name) {
            "der" => Ok(match one(self)? {
                Units::Of(dim) => {
                    let mut seconds = [0; BASES];
                    seconds[2] = 1;
                    Units::Of(dim.combine(Dim::of(seconds), -1))
                }
                _ => Units::Any,
            }),
            "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "exp"
            | "log" | "log10" => {
                if let Some(arg) = args.first() {
                    if let Units::Of(dim) = self.infer(arg)? {
                        if dim != Dim::ONE {
                            return Err(format!(
                                "the argument of `{name}` must be dimensionless, \
                                 but `{}` has unit {}",
                                describe(arg),
                                dim.text()
                            ));
                        }
                    }
                }
                Ok(Units::Weak)
            }
            "atan2" => {
                if let (Some(a), Some(b)) = (args.first(), args.get(1)) {
                    let (ua, ub) = (self.infer(a)?, self.infer(b)?);
                    if clashes(ua, ub) {
                        return Err(format!(
                            "the arguments of `atan2` disagree: {} against {}",
                            ua.text(),
                            ub.text()
                        ));
                    }
                }
                Ok(Units::Weak)
            }
            // The square root halves every exponent, and a rational
            // dimension holds a half exactly - `sqrt` of a length is
            // `m(1/2)`, not "no information".
            "sqrt" => Ok(match one(self)? {
                Units::Of(dim) => Units::Of(dim.scaled(1, 2)),
                other => other,
            }),
            "abs" | "pre" | "noEvent" | "floor" | "ceil" | "integer" => one(self),
            "smooth" => args.get(1).map_or(Ok(Units::Any), |a| self.infer(a)),
            "min" | "max" | "mod" | "rem" | "div" => {
                let mut result = Units::Any;
                for arg in args {
                    let units = self.infer(arg)?;
                    if clashes(result, units) {
                        return Err(format!(
                            "the arguments of `{name}` disagree: {} against {}",
                            result.text(),
                            units.text()
                        ));
                    }
                    result = merge(result, units);
                }
                Ok(result)
            }
            _ => {
                for arg in args {
                    self.infer(arg)?;
                }
                Ok(Units::Any)
            }
        }
    }

    /// `base ^ exponent`. Dimensions scale by a constant exponent;
    /// anything less determined gives up rather than guesses.
    fn power(&self, base: Units, exponent: &Expr) -> Result<Units, String> {
        if let Units::Of(dim) = self.infer(exponent)? {
            if dim != Dim::ONE {
                return Err(format!(
                    "an exponent must be dimensionless, but `{}` has unit {}",
                    describe(exponent),
                    dim.text()
                ));
            }
        }
        match base {
            Units::Of(dim) => {
                if let Expr::Number(n) = exponent {
                    // A whole power scales exactly; a fractional one
                    // still does when it is a simple ratio, which the
                    // rational dimension can now hold.
                    if n.fract() == 0.0 {
                        return Ok(Units::Of(dim.scaled(*n as i32, 1)));
                    }
                    for over in 2..=12 {
                        let whole = n * f64::from(over);
                        if whole.fract() == 0.0 {
                            return Ok(Units::Of(dim.scaled(whole as i32, over)));
                        }
                    }
                }
                Ok(Units::Any)
            }
            other => Ok(other),
        }
    }
}

/// Two fully known, different dimensions.
fn clashes(a: Units, b: Units) -> bool {
    matches!((a, b), (Units::Of(x), Units::Of(y)) if x != y)
}

/// What a sum or a pair of branches is known to be.
fn merge(a: Units, b: Units) -> Units {
    match (a, b) {
        (Units::Of(dim), _) | (_, Units::Of(dim)) => Units::Of(dim),
        (Units::Any, _) | (_, Units::Any) => Units::Any,
        _ => Units::Weak,
    }
}

/// A product or quotient; a bare number is dimensionless here.
fn product(a: Units, b: Units, sign: i32) -> Units {
    match (a, b) {
        (Units::Any, _) | (_, Units::Any) => Units::Any,
        (Units::Weak, Units::Weak) => Units::Weak,
        (Units::Of(x), Units::Weak) => Units::Of(x),
        (Units::Weak, Units::Of(y)) => Units::Of(Dim::ONE.combine(y, sign)),
        (Units::Of(x), Units::Of(y)) => Units::Of(x.combine(y, sign)),
    }
}

// -------------------------------------------------------- unit strings

/// Parse a Modelica unit string (`"N.m"`, `"m/s2"`, `"J/(kg.K)"`) into
/// dimensions. `None` means a symbol this table does not know; the
/// whole unit then carries no information rather than a wrong one.
fn parse_unit(text: &str) -> Option<Dim> {
    let mut reader = UnitReader {
        chars: text.chars().collect(),
        at: 0,
    };
    let dim = reader.expression()?;
    if reader.at == reader.chars.len() {
        Some(dim)
    } else {
        None
    }
}

struct UnitReader {
    chars: Vec<char>,
    at: usize,
}

impl UnitReader {
    /// `numerator [ "/" denominator ]`: one division at most, which is
    /// what the grammar allows - `N.m/s/K` is not a unit, `N.m/(s.K)`
    /// is. A denominator of several factors has to be parenthesised.
    fn expression(&mut self) -> Option<Dim> {
        let mut dim = self.numerator()?;
        if self.chars.get(self.at) == Some(&'/') {
            self.at += 1;
            dim = dim.combine(self.denominator()?, -1);
        }
        Some(dim)
    }

    /// `"1" | factors | "(" expression ")"`.
    fn numerator(&mut self) -> Option<Dim> {
        if self.chars.get(self.at) == Some(&'(') {
            return self.group();
        }
        let mut dim = self.factor()?;
        while self.chars.get(self.at) == Some(&'.') {
            self.at += 1;
            dim = dim.combine(self.factor()?, 1);
        }
        Some(dim)
    }

    /// `factor | "(" expression ")"`: a single factor, unless grouped.
    fn denominator(&mut self) -> Option<Dim> {
        if self.chars.get(self.at) == Some(&'(') {
            return self.group();
        }
        self.factor()
    }

    /// `"(" expression ")"`.
    fn group(&mut self) -> Option<Dim> {
        self.at += 1;
        let inner = self.expression()?;
        if self.chars.get(self.at) != Some(&')') {
            return None;
        }
        self.at += 1;
        Some(inner)
    }

    /// One symbol with an optional exponent: `m`, `s-1`, `m(1/2)`.
    fn factor(&mut self) -> Option<Dim> {
        let start = self.at;
        while self
            .chars
            .get(self.at)
            .is_some_and(|c| c.is_alphabetic() || *c == '%')
        {
            self.at += 1;
        }
        // A bare "1" is the dimensionless unit; any other digit here
        // belongs to an exponent and stays for the code below.
        if start == self.at && self.chars.get(self.at) == Some(&'1') {
            self.at += 1;
            return Some(Dim::ONE);
        }
        let symbol: String = self.chars[start..self.at].iter().collect();
        if symbol.is_empty() {
            return None;
        }
        let base = symbol_dimensions(&symbol)?;
        let (whole, over) = self.exponent()?;
        Some(base.scaled(whole, over))
    }

    /// The exponent of a factor: absent (so `1/1`), a signed integer
    /// (`m2`, `s-1`), or a signed rational in parentheses (`m(1/2)`).
    fn exponent(&mut self) -> Option<(i32, i32)> {
        let before = self.at;
        let sign = match self.chars.get(self.at) {
            Some('+') => {
                self.at += 1;
                1
            }
            Some('-') => {
                self.at += 1;
                -1
            }
            _ => 1,
        };
        if self.chars.get(self.at) == Some(&'(') {
            self.at += 1;
            let numerator = self.unsigned()?;
            if self.chars.get(self.at) != Some(&'/') {
                return None;
            }
            self.at += 1;
            let denominator = self.unsigned()?;
            if denominator == 0 || self.chars.get(self.at) != Some(&')') {
                return None;
            }
            self.at += 1;
            return Some((sign * numerator, denominator));
        }
        if let Some(whole) = self.unsigned() {
            return Some((sign * whole, 1));
        }
        // A sign with no number behind it was never an exponent: leave
        // it for whatever comes next to make sense of, or not.
        self.at = before;
        Some((1, 1))
    }

    /// A run of digits as a number, or `None` without consuming any.
    fn unsigned(&mut self) -> Option<i32> {
        let start = self.at;
        while self.chars.get(self.at).is_some_and(char::is_ascii_digit) {
            self.at += 1;
        }
        if self.at == start {
            return None;
        }
        self.chars[start..self.at]
            .iter()
            .collect::<String>()
            .parse()
            .ok()
    }
}

/// Dimensions of one unit symbol, with an SI prefix allowed in front.
fn symbol_dimensions(symbol: &str) -> Option<Dim> {
    if let Some(dim) = bare_symbol(symbol) {
        return Some(dim);
    }
    // A whole-symbol match wins over a prefix reading, so `cd` is the
    // candela and `min` the minute; `mm`, `kPa` and `MOhm` land here.
    // Every SI prefix is here, the two-letter `da` first so it is tried
    // before `d`, and the reading is the first prefix that leaves a
    // symbol behind - scale factors do not matter, only that the letter
    // is a prefix at all.
    let prefixes: [&str; 25] = [
        "da", "Q", "R", "Y", "Z", "E", "P", "T", "G", "M", "k", "h", "d", "c", "m", "u", "µ", "n",
        "p", "f", "a", "z", "y", "r", "q",
    ];
    prefixes
        .iter()
        .filter(|prefix| symbol.starts_with(**prefix))
        .find_map(|prefix| bare_symbol(&symbol[prefix.len()..]))
}

/// The dimensions of an unprefixed symbol: SI base and derived units,
/// with angles dimensionless and scale factors ignored.
fn bare_symbol(symbol: &str) -> Option<Dim> {
    let dim = |values: [i32; BASES]| Some(Dim::of(values));
    match symbol {
        "1" | "rad" | "sr" | "deg" | "%" => dim([0, 0, 0, 0, 0, 0, 0]),
        "m" => dim([1, 0, 0, 0, 0, 0, 0]),
        "g" | "kg" => dim([0, 1, 0, 0, 0, 0, 0]),
        "s" | "min" | "h" | "d" => dim([0, 0, 1, 0, 0, 0, 0]),
        "A" => dim([0, 0, 0, 1, 0, 0, 0]),
        "K" | "degC" => dim([0, 0, 0, 0, 1, 0, 0]),
        "mol" => dim([0, 0, 0, 0, 0, 1, 0]),
        "cd" => dim([0, 0, 0, 0, 0, 0, 1]),
        "Hz" | "Bq" => dim([0, 0, -1, 0, 0, 0, 0]),
        "N" => dim([1, 1, -2, 0, 0, 0, 0]),
        "Pa" | "bar" => dim([-1, 1, -2, 0, 0, 0, 0]),
        "J" | "eV" => dim([2, 1, -2, 0, 0, 0, 0]),
        "W" => dim([2, 1, -3, 0, 0, 0, 0]),
        "C" => dim([0, 0, 1, 1, 0, 0, 0]),
        "V" => dim([2, 1, -3, -1, 0, 0, 0]),
        "F" => dim([-2, -1, 4, 2, 0, 0, 0]),
        "Ohm" | "ohm" => dim([2, 1, -3, -2, 0, 0, 0]),
        "S" => dim([-2, -1, 3, 2, 0, 0, 0]),
        "Wb" => dim([2, 1, -2, -1, 0, 0, 0]),
        "T" => dim([0, 1, -2, -1, 0, 0, 0]),
        "H" => dim([2, 1, -2, -2, 0, 0, 0]),
        "lm" => dim([0, 0, 0, 0, 0, 0, 1]),
        "lx" => dim([-2, 0, 0, 0, 0, 0, 1]),
        "Gy" | "Sv" => dim([2, 0, -2, 0, 0, 0, 0]),
        "kat" => dim([0, 0, -1, 0, 0, 1, 0]),
        "L" | "l" => dim([3, 0, 0, 0, 0, 0, 0]),
        _ => None,
    }
}

// ----------------------------------------------------------- reporting

/// Immediate children of an expression, for the loose walks.
fn children(expr: &Expr) -> Vec<&Expr> {
    match expr {
        Expr::Index(base, subscripts) => std::iter::once(&**base).chain(subscripts).collect(),
        Expr::Member(base, _) => vec![base],
        Expr::Array(items) => items.iter().collect(),
        Expr::Range(lower, step, upper) => {
            let mut out = vec![&**lower];
            if let Some(step) = step {
                out.push(step);
            }
            out.push(upper);
            out
        }
        Expr::Comprehension(body, _, range) => vec![body, range],
        Expr::MatrixRows(rows) => rows.iter().flatten().collect(),
        _ => Vec::new(),
    }
}

/// A compact source-like spelling of an expression for error messages.
fn describe(expr: &Expr) -> String {
    match expr {
        Expr::Number(n) => format!("{n}"),
        Expr::Bool(b) => format!("{b}"),
        Expr::Ref(name) => name.clone(),
        Expr::Time => "time".to_string(),
        Expr::Neg(inner) => format!("-{}", describe(inner)),
        Expr::Bin(op, a, b) | Expr::Elementwise(op, a, b) => {
            format!("{} {} {}", describe(a), op_text(*op), describe(b))
        }
        Expr::Rel(op, a, b) => {
            let symbol = match op {
                RelOp::Lt => "<",
                RelOp::Le => "<=",
                RelOp::Gt => ">",
                RelOp::Ge => ">=",
                RelOp::Eq => "==",
                RelOp::Ne => "<>",
            };
            format!("{} {symbol} {}", describe(a), describe(b))
        }
        Expr::And(a, b) => format!("{} and {}", describe(a), describe(b)),
        Expr::Or(a, b) => format!("{} or {}", describe(a), describe(b)),
        Expr::Not(inner) => format!("not {}", describe(inner)),
        Expr::If(c, t, e) => format!(
            "if {} then {} else {}",
            describe(c),
            describe(t),
            describe(e)
        ),
        Expr::Call(name, args) => {
            let args: Vec<String> = args.iter().map(describe).collect();
            format!("{name}({})", args.join(", "))
        }
        _ => "…".to_string(),
    }
}

fn op_text(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Pow => "^",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_model;

    fn error_of(source: &str) -> String {
        parse_model(source).expect_err("should be rejected").message
    }

    #[test]
    fn booleans_do_not_mix_with_numbers() {
        let text = error_of("model M Boolean on = true; Real y; equation y = on + 1; end M;");
        assert!(text.contains("Boolean"), "{text}");

        let text = error_of("model M Real x = 1; Real y; equation y = if x then 1 else 0; end M;");
        assert!(text.contains("not Boolean"), "{text}");

        let text = error_of("model M Boolean on = true; Real y; equation y = on; end M;");
        assert!(text.contains("type mismatch"), "{text}");

        let text = error_of(
            "model M Boolean on = true; Real x = 1; Real y; \
             equation y = if time > 0.5 then on else x; end M;",
        );
        assert!(text.contains("branches"), "{text}");

        let text = error_of("model M Real x = 1; Boolean b; equation b = x > 0.5 and x; end M;");
        assert!(text.contains("Boolean is needed"), "{text}");

        let text = error_of("model M Boolean on = true; Real y; equation y = sin(on); end M;");
        assert!(text.contains("needs a number"), "{text}");

        let text = error_of("model M Boolean on = true; Real y; equation y = abs(on); end M;");
        assert!(text.contains("needs a number"), "{text}");

        let text = error_of("model M Boolean on = true; Real y; equation y = -on; end M;");
        assert!(text.contains("unary minus"), "{text}");

        let text = error_of("model M Boolean on = true; Real y; equation y = max(on, 1); end M;");
        assert!(text.contains("mixes"), "{text}");

        let text = error_of("model M Boolean on = true; Boolean b; equation b = on > 1; end M;");
        assert!(text.contains("cannot compare"), "{text}");

        let text = error_of(
            "model M discrete Boolean on(start = false); \
             equation when time > 1 then on = 1.5; end when; end M;",
        );
        assert!(text.contains("type mismatch"), "{text}");

        // Boolean branches agree with each other, and the numeric
        // helpers keep their argument types.
        parse_model(
            "model M Boolean on = true; Boolean b; Real y; \
             equation b = if time > 0.5 then true else on; \
             y = mod(6, 4) + div(7, 2) + rem(5, 3) + abs(-2) + smooth(1, time); end M;",
        )
        .unwrap();
    }

    #[test]
    fn an_integer_cannot_swallow_a_real_in_an_assignment() {
        let text = error_of(
            "model M discrete Integer n(start = 0); \
             equation when sample(0, 0.25) then n = pre(n) + 0.5; end when; end M;",
        );
        assert!(text.contains("integer()"), "{text}");

        // With `integer()` the same model is fine.
        parse_model(
            "model M discrete Integer n(start = 0); \
             equation when sample(0, 0.25) then n = pre(n) + integer(0.5); \
             end when; end M;",
        )
        .unwrap();
    }

    #[test]
    fn declared_units_must_agree_across_an_equation() {
        let text = error_of(
            "model M Real v(unit = \"V\") = 1; Real i(unit = \"A\"); \
             equation i = v; end M;",
        );
        assert!(text.contains("unit mismatch"), "{text}");
        assert!(text.contains("m2.kg.s-3.A-1"), "{text}");

        // Ohm's law balances: the same pair through a resistance.
        parse_model(
            "model M Real v(unit = \"V\") = 1; parameter Real r(unit = \"Ohm\") = 2; \
             Real i(unit = \"A\"); equation i = v / r; end M;",
        )
        .unwrap();
    }

    #[test]
    fn literals_bend_but_declared_dimensions_do_not() {
        // Scaling by numbers and offsets by literals never complain.
        parse_model(
            "model M Real x(unit = \"m\") = 1; Real y(unit = \"m\"); \
             equation y = 2 * x + 1; end M;",
        )
        .unwrap();
        // An undeclared variable absorbs whatever surrounds it.
        parse_model(
            "model M Real x(unit = \"m\") = 1; Real k = 3; Real y(unit = \"m\"); \
             equation y = k * x; end M;",
        )
        .unwrap();
        // A unit the table does not know carries no information.
        parse_model(
            "model M Real x(unit = \"furlong\") = 1; Real y(unit = \"m\"); \
             equation y = x; end M;",
        )
        .unwrap();
        // But metres plus volts is never right.
        let text = error_of(
            "model M Real x(unit = \"m\") = 1; Real v(unit = \"V\") = 1; Real y; \
             equation y = x + v; end M;",
        );
        assert!(text.contains("cannot add"), "{text}");
    }

    #[test]
    fn derivatives_divide_by_seconds() {
        parse_model(
            "model M Real x(unit = \"m\", start = 0); Real v(unit = \"m/s\") = 1; \
             equation der(x) = v; end M;",
        )
        .unwrap();
        let text = error_of(
            "model M Real x(unit = \"m\", start = 0); Real v(unit = \"m\") = 1; \
             equation der(x) = v; end M;",
        );
        assert!(text.contains("m.s-1"), "{text}");
    }

    #[test]
    fn function_arguments_have_dimensional_rules() {
        let text =
            error_of("model M Real v(unit = \"V\") = 1; Real y; equation y = sin(v); end M;");
        assert!(text.contains("dimensionless"), "{text}");

        // Angles are dimensionless, so trigonometry accepts them.
        parse_model(
            "model M Real phi(unit = \"rad\") = 1; Real y; \
             equation y = sin(phi); end M;",
        )
        .unwrap();

        // A square root halves even dimensions.
        parse_model(
            "model M Real a(unit = \"m2\") = 4; Real x(unit = \"m\"); \
             equation x = sqrt(a); end M;",
        )
        .unwrap();

        let text = error_of(
            "model M Real v(unit = \"V\") = 1; Real i(unit = \"A\") = 1; Real y; \
             equation y = max(v, i); end M;",
        );
        assert!(text.contains("disagree"), "{text}");

        let text = error_of(
            "model M Real v(unit = \"V\") = 1; Real i(unit = \"A\") = 1; \
             Boolean b; equation b = v > i; end M;",
        );
        assert!(text.contains("cannot compare"), "{text}");
    }

    #[test]
    fn powers_scale_dimensions_by_constant_exponents() {
        parse_model(
            "model M Real x(unit = \"m\") = 2; Real a(unit = \"m2\"); \
             equation a = x ^ 2; end M;",
        )
        .unwrap();
        let text = error_of(
            "model M Real x(unit = \"m\") = 2; Real a(unit = \"m\"); \
             equation a = x ^ 2; end M;",
        );
        assert!(text.contains("unit mismatch"), "{text}");
        let text = error_of(
            "model M Real x(unit = \"m\") = 2; Real t(unit = \"s\") = 1; Real y; \
             equation y = x ^ t; end M;",
        );
        assert!(text.contains("exponent"), "{text}");
    }

    #[test]
    fn the_unit_grammar_reads_the_usual_spellings() {
        let dims = |text: &str| parse_unit(text).map(Dim::text);
        assert_eq!(dims("N.m"), Some("m2.kg.s-2".to_string()));
        assert_eq!(dims("J/(kg.K)"), Some("m2.s-2.K-1".to_string()));
        assert_eq!(dims("m/s2"), Some("m.s-2".to_string()));
        assert_eq!(dims("kPa"), Some("m-1.kg.s-2".to_string()));
        assert_eq!(dims("MOhm"), Some("m2.kg.s-3.A-2".to_string()));
        assert_eq!(dims("mm"), Some("m".to_string()));
        assert_eq!(dims("rad/s"), Some("s-1".to_string()));
        assert_eq!(dims("1/min"), Some("s-1".to_string()));
        assert_eq!(dims("kW.h"), Some("m2.kg.s-2".to_string()));
        assert_eq!(dims("1"), Some("1".to_string()));
        // The candela is not a centi-day, the minute not a milli-inch.
        assert_eq!(dims("cd"), dims("lm"));
        assert_eq!(dims("min"), dims("s"));
        assert_eq!(dims("furlong"), None);
        assert_eq!(dims("m/"), None);
        assert_eq!(dims("(m"), None);
        assert_eq!(dims("9m"), None);

        // The whole derived table, by way of its equivalences.
        assert_eq!(dims("Hz"), dims("Bq"));
        assert_eq!(dims("Pa"), dims("bar"));
        assert_eq!(dims("J"), dims("eV"));
        assert_eq!(dims("W"), dims("J/s"));
        assert_eq!(dims("C"), dims("s.A"));
        assert_eq!(dims("V"), dims("W/A"));
        assert_eq!(dims("F"), dims("C/V"));
        assert_eq!(dims("Ohm"), dims("ohm"));
        assert_eq!(dims("S"), dims("1/Ohm"));
        assert_eq!(dims("Wb"), dims("V.s"));
        assert_eq!(dims("T"), dims("Wb/m2"));
        assert_eq!(dims("H"), dims("Wb/A"));
        assert_eq!(dims("lx"), dims("lm/m2"));
        assert_eq!(dims("Gy"), dims("Sv"));
        assert_eq!(dims("kat"), dims("mol/s"));
        assert_eq!(dims("L"), dims("l"));
        assert_eq!(dims("degC"), dims("K"));
        assert_eq!(dims("g"), dims("kg"));
        assert_eq!(dims("h"), dims("d"));
        assert_eq!(dims("%"), dims("sr"));
        assert_eq!(dims("N/(V.s+2)"), dims("N.V-1.s-2"));

        // A rational exponent, which the grammar writes in parentheses.
        // The half of a length is a half, kept exactly, so squaring it
        // gives the length back.
        assert_eq!(dims("m(1/2)"), Some("m(1/2)".to_string()));
        assert_eq!(dims("m(2/2)"), dims("m"));
        assert_eq!(dims("m(3/2)"), Some("m(3/2)".to_string()));
        // The sign of a rational exponent sits before the parenthesis,
        // as the grammar puts it, not inside.
        assert_eq!(dims("s-(1/2)"), Some("s(-1/2)".to_string()));
        assert_eq!(dims("s(-1/2)"), None);
        assert_eq!(dims("m(2/4)"), dims("m(1/2)"));
        assert_eq!(dims("m(1/3).m(2/3)"), dims("m"));
        // A malformed rational is no information, not a wrong guess.
        assert_eq!(dims("m(1/0)"), None);
        assert_eq!(dims("m(1/)"), None);
        assert_eq!(dims("m(/2)"), None);

        // Every SI prefix is recognised, the large ones included, and a
        // whole-symbol unit still wins over a prefix reading.
        assert_eq!(dims("PW"), dims("W").map(|_| "m2.kg.s-3".to_string()));
        assert_eq!(dims("EJ"), dims("J"));
        assert_eq!(dims("YN"), dims("N"));
        assert_eq!(dims("qs"), dims("s"));
        assert_eq!(dims("Pa"), dims("bar")); // pascal, not peta-are
        assert_eq!(dims("T"), dims("Wb/m2")); // tesla, not tera
        assert_eq!(dims("min"), dims("s")); // minute, not milli-inch

        // At most one `/`, and a compound denominator parenthesised -
        // `W/m/K` is not a unit, `W/(m.K)` is the one that was meant.
        assert_eq!(dims("W/m/K"), None);
        assert_eq!(dims("W/(m.K)"), dims("W.m-1.K-1"));
        assert_eq!(dims("J/(kg.K)"), dims("m2.s-2.K-1"));
    }

    #[test]
    fn dimensionless_results_and_reciprocals_come_out_right() {
        // A frequency from a plain reciprocal: the literal is
        // dimensionless in a quotient.
        parse_model(
            "model M parameter Real T(unit = \"s\") = 2; Real f(unit = \"Hz\"); \
             equation f = 1 / T; end M;",
        )
        .unwrap();
        // Square roots and fractional powers of odd dimensions give
        // up rather than guess.
        parse_model(
            "model M Real v(unit = \"V\") = 1; Real y; Real z; \
             equation y = sqrt(v); z = v ^ 0.5; end M;",
        )
        .unwrap();
        // `smooth` hands its expression through unchanged.
        parse_model(
            "model M Real x(unit = \"m\") = 1; Real y(unit = \"m\"); \
             equation y = smooth(1, x); end M;",
        )
        .unwrap();

        let text = error_of(
            "model M Real v(unit = \"V\") = 1; Real i(unit = \"A\") = 1; Real y; \
             equation y = if time > 0.5 then v else i; end M;",
        );
        assert!(text.contains("branches"), "{text}");

        let text = error_of(
            "model M Real v(unit = \"V\") = 1; Real i(unit = \"A\") = 1; Real y; \
             equation y = atan2(v, i); end M;",
        );
        assert!(text.contains("atan2"), "{text}");

        let text = error_of(
            "model M Real v(unit = \"V\") = 1; Real i(unit = \"A\") = 1; Real y; \
             equation y = mod(v, i); end M;",
        );
        assert!(text.contains("disagree"), "{text}");
    }

    #[test]
    fn the_reporting_helpers_spell_expressions() {
        let x = || Box::new(Expr::Ref("x".to_string()));
        let one = || Box::new(Expr::Number(1.0));
        let spell = |op: RelOp| describe(&Expr::Rel(op, x(), one()));
        assert_eq!(spell(RelOp::Lt), "x < 1");
        assert_eq!(spell(RelOp::Le), "x <= 1");
        assert_eq!(spell(RelOp::Gt), "x > 1");
        assert_eq!(spell(RelOp::Ge), "x >= 1");
        assert_eq!(spell(RelOp::Eq), "x == 1");
        assert_eq!(spell(RelOp::Ne), "x <> 1");
        assert_eq!(
            describe(&Expr::And(
                Box::new(Expr::Bool(true)),
                Box::new(Expr::Or(x(), Box::new(Expr::Not(x()))))
            )),
            "true and x or not x"
        );
        assert_eq!(
            describe(&Expr::If(
                x(),
                Box::new(Expr::Time),
                Box::new(Expr::Neg(one()))
            )),
            "if x then time else -1"
        );
        assert_eq!(
            describe(&Expr::Call("f".to_string(), vec![*x(), *one()])),
            "f(x, 1)"
        );
        assert_eq!(describe(&Expr::Array(Vec::new())), "…");

        // The loose walks see every child of the array forms.
        let range = Expr::Range(one(), Some(one()), one());
        assert_eq!(children(&range).len(), 3);
        assert_eq!(
            children(&Expr::Comprehension(x(), "i".to_string(), Box::new(range))).len(),
            2
        );
        assert_eq!(
            children(&Expr::MatrixRows(vec![vec![*one()], vec![*one()]])).len(),
            2
        );
        assert_eq!(children(&Expr::Index(x(), vec![*one()])).len(), 2);
        assert_eq!(children(&Expr::Member(x(), "y".to_string())).len(), 1);

        assert_eq!(Units::Any.text(), "unknown");
        assert_eq!(Units::Weak.text(), "1");
        assert_eq!(Ty::Real.name(), "Real");
        assert_eq!(Ty::Unknown.name(), "unknown");
    }

    #[test]
    fn units_travel_through_aliases_and_when_clauses() {
        // The alias carries the unit; the declaration's own wins.
        let text = error_of(
            "package U type Voltage = Real(unit = \"V\"); end U; \
             model M U.Voltage v = 1; Real i(unit = \"A\"); \
             equation i = v; end M;",
        );
        assert!(text.contains("unit mismatch"), "{text}");

        let text = error_of(
            "model M Real x(unit = \"m\", start = 0); discrete Real v(unit = \"V\", start = 0); \
             equation der(x) = 1; \
             when x > 1 then v = x; end when; end M;",
        );
        assert!(text.contains("unit mismatch"), "{text}");

        let text = error_of(
            "model M Real x(unit = \"m\", start = 0); Real v(unit = \"V\") = 1; \
             equation der(x) = 1; \
             when x > 1 then reinit(x, v); end when; end M;",
        );
        assert!(text.contains("unit mismatch"), "{text}");
    }

    #[test]
    fn an_integer_may_not_hold_a_fraction() {
        // A binding arrives at the checker as an equation, so this is
        // the one place a direction can be read off it: the other side
        // works out on its own, and it is not a whole number.
        let text = error_of("model M Integer i = 1.5; Real y; equation y = i; end M;");
        assert!(text.contains("works out to 1.5"), "{text}");

        let text = error_of("model M Integer i(start = 0.5); Real y; equation y = i; end M;");
        assert!(text.contains("starts at 0.5"), "{text}");

        // Spelling a whole number with a point is not a contradiction,
        // and neither is an equation that merely *mentions* an Integer
        // beside a Real: `2 * i = x` promotes, and which side it
        // determines is the matcher's business.
        assert!(parse_model("model M Integer i = 3.0; Real y; equation y = i; end M;").is_ok());
        assert!(parse_model("model M Integer i = 3; Real y; equation y = i; end M;").is_ok());
        assert!(
            parse_model("model M Integer i = 3; Real x; equation 2 * i = x; end M;").is_ok(),
            "an Integer beside a Real is ordinary"
        );
    }

    #[test]
    fn a_value_settled_before_the_run_must_sit_inside_its_bounds() {
        let text = error_of("model M Real level(min = 0) = -1; Real y; equation y = level; end M;");
        assert!(text.contains("below its min of 0"), "{text}");

        let text = error_of("model M Real p(max = 10) = 11; Real y; equation y = p; end M;");
        assert!(text.contains("above its max of 10"), "{text}");

        let text = error_of("model M Real x(start = -3, min = -1); equation der(x) = 1; end M;");
        assert!(text.contains("starts at -3"), "{text}");

        // Inside the bounds, and bounds with nothing to say about a
        // value the run has yet to produce, both pass.
        assert!(parse_model(
            "model M Real p(min = 0, max = 10) = 5; Real y; \
             equation y = p; end M;"
        )
        .is_ok());
        assert!(parse_model("model M Real x(min = 0); equation x = time; end M;").is_ok());
    }

    #[test]
    fn two_reals_are_not_compared_for_equality() {
        // A Real is what a step arrived at, and asking a stepped
        // quantity to land on a value exactly asks the arithmetic for
        // something it cannot promise. The specification forbids it.
        let text = error_of(
            "model M Real a; Real b; Boolean e; equation a = time; b = 2 * time; \
             e = a == b; end M;",
        );
        assert!(text.contains("compares Reals for equality"), "{text}");

        let text = error_of(
            "model M Real a; discrete Boolean e(start = false); equation a = time; \
             when a <> 1 then e = true; end when; end M;",
        );
        assert!(text.contains("compares Reals for equality"), "{text}");

        // The whole numbers have no such trouble, and neither do the
        // enumerations that become them.
        assert!(parse_model(
            "model M Integer i = 2; Real y; equation y = if i == 2 then 1 else 0; end M;"
        )
        .is_ok());
        assert!(parse_model(
            "model M Boolean b = true; Real y; equation y = if b == true then 1 else 0; end M;"
        )
        .is_ok());
    }
}
