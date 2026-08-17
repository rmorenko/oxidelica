//! AST of the M0 Modelica slice. A flat model: declarations plus equations.

/// How a component's value may change over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variability {
    /// An ordinary continuous variable.
    Continuous,
    /// `parameter` — fixed for the duration of a simulation.
    Parameter,
    /// `constant`.
    Constant,
}

/// A component (variable) declaration.
#[derive(Debug, Clone)]
pub struct Component {
    /// Component name.
    pub name: String,
    /// Variability class of the component.
    pub variability: Variability,
    /// The `start` attribute from the modifier: `Real x(start = 1.0)`.
    pub start: Option<Expr>,
    /// The `fixed` attribute.
    pub fixed: Option<bool>,
    /// Declaration binding: `parameter Real a = 1.0`.
    pub binding: Option<Expr>,
    /// Optional description string.
    pub description: Option<String>,
}

/// A single equation `lhs = rhs`.
#[derive(Debug, Clone)]
pub struct EquationItem {
    /// Left-hand side expression.
    pub lhs: Expr,
    /// Right-hand side expression.
    pub rhs: Expr,
}

/// Simulation settings from `annotation(experiment(...))`.
#[derive(Debug, Clone, Default)]
pub struct Experiment {
    /// `StopTime` — simulation end time.
    pub stop_time: Option<f64>,
    /// `Interval` — output/integration step.
    pub interval: Option<f64>,
    /// `Tolerance` — solver tolerance (reserved for adaptive solvers).
    pub tolerance: Option<f64>,
}

/// A parsed flat model.
#[derive(Debug, Clone)]
pub struct Model {
    /// Model name.
    pub name: String,
    /// Optional description string after the model name.
    pub description: Option<String>,
    /// Component declarations in source order.
    pub components: Vec<Component>,
    /// Equations in source order.
    pub equations: Vec<EquationItem>,
    /// Experiment settings (defaults when absent).
    pub experiment: Experiment,
}

/// Binary arithmetic operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `^`
    Pow,
}

/// Relational operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelOp {
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `==`
    Eq,
    /// `<>`
    Ne,
}

/// An expression tree node.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Numeric literal.
    Number(f64),
    /// Boolean literal.
    Bool(bool),
    /// Component reference; in a flat M0 model just a name
    /// (`x`; dotted `body.m` arrives with M2).
    Ref(String),
    /// The built-in `time` variable.
    Time,
    /// Function call, including `der(x)`.
    Call(String, Vec<Expr>),
    /// Unary minus.
    Neg(Box<Expr>),
    /// Binary arithmetic.
    Bin(BinOp, Box<Expr>, Box<Expr>),
    /// Relational comparison.
    Rel(RelOp, Box<Expr>, Box<Expr>),
    /// Logical `and`.
    And(Box<Expr>, Box<Expr>),
    /// Logical `or`.
    Or(Box<Expr>, Box<Expr>),
    /// Logical `not`.
    Not(Box<Expr>),
    /// `if c then a else b` (`elseif` chains become nested `If`s).
    If(Box<Expr>, Box<Expr>, Box<Expr>),
}

impl Expr {
    /// If the expression is exactly `der(<name>)`, return the state name.
    pub fn as_der_of(&self) -> Option<&str> {
        if let Expr::Call(name, args) = self {
            if name == "der" && args.len() == 1 {
                if let Expr::Ref(var) = &args[0] {
                    return Some(var);
                }
            }
        }
        None
    }

    /// Whether the expression contains a `der(...)` call anywhere.
    pub fn contains_der(&self) -> bool {
        match self {
            Expr::Call(name, args) => name == "der" || args.iter().any(Expr::contains_der),
            Expr::Neg(inner) | Expr::Not(inner) => inner.contains_der(),
            Expr::Bin(_, l, r) | Expr::Rel(_, l, r) | Expr::And(l, r) | Expr::Or(l, r) => {
                l.contains_der() || r.contains_der()
            }
            Expr::If(c, t, e) => c.contains_der() || t.contains_der() || e.contains_der(),
            _ => false,
        }
    }

    /// Collect the names of all component references in the expression.
    pub fn collect_refs<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Expr::Ref(name) => out.push(name),
            Expr::Call(_, args) => args.iter().for_each(|a| a.collect_refs(out)),
            Expr::Neg(inner) | Expr::Not(inner) => inner.collect_refs(out),
            Expr::Bin(_, l, r) | Expr::Rel(_, l, r) | Expr::And(l, r) | Expr::Or(l, r) => {
                l.collect_refs(out);
                r.collect_refs(out);
            }
            Expr::If(c, t, e) => {
                c.collect_refs(out);
                t.collect_refs(out);
                e.collect_refs(out);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(name: &str) -> Expr {
        Expr::Ref(name.into())
    }

    fn der(name: &str) -> Expr {
        Expr::Call("der".into(), vec![r(name)])
    }

    #[test]
    fn as_der_of_only_matches_exact_shape() {
        assert_eq!(der("x").as_der_of(), Some("x"));
        // Two-argument der, der of an expression and non-der do not match.
        assert_eq!(
            Expr::Call("der".into(), vec![r("x"), r("y")]).as_der_of(),
            None
        );
        assert_eq!(
            Expr::Call("der".into(), vec![Expr::Number(1.0)]).as_der_of(),
            None
        );
        assert_eq!(Expr::Call("sin".into(), vec![r("x")]).as_der_of(), None);
        assert_eq!(r("x").as_der_of(), None);
    }

    #[test]
    fn contains_der_walks_every_variant() {
        let deep = Expr::If(
            Box::new(Expr::Rel(
                RelOp::Lt,
                Box::new(r("a")),
                Box::new(Expr::Number(0.0)),
            )),
            Box::new(Expr::And(
                Box::new(Expr::Bool(true)),
                Box::new(Expr::Not(Box::new(r("b")))),
            )),
            Box::new(Expr::Or(
                Box::new(Expr::Neg(Box::new(der("x")))),
                Box::new(Expr::Time),
            )),
        );
        assert!(deep.contains_der());
        assert!(!r("x").contains_der());
        assert!(!Expr::Time.contains_der());
    }

    #[test]
    fn collect_refs_walks_every_variant() {
        let deep = Expr::If(
            Box::new(Expr::Rel(
                RelOp::Ge,
                Box::new(r("a")),
                Box::new(Expr::Number(1.0)),
            )),
            Box::new(Expr::Bin(
                BinOp::Add,
                Box::new(Expr::Not(Box::new(r("b")))),
                Box::new(Expr::Call("sin".into(), vec![r("c")])),
            )),
            Box::new(Expr::Neg(Box::new(Expr::Bool(false)))),
        );
        let mut refs = Vec::new();
        deep.collect_refs(&mut refs);
        assert_eq!(refs, vec!["a", "b", "c"]);
    }
}
