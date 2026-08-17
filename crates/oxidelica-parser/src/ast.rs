//! AST среза Modelica (M0). Плоская модель: объявления + уравнения.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variability {
    /// Обычная непрерывная переменная.
    Continuous,
    /// `parameter` — фиксируется на время симуляции.
    Parameter,
    /// `constant`.
    Constant,
}

#[derive(Debug, Clone)]
pub struct Component {
    pub name: String,
    pub variability: Variability,
    /// Атрибут `start` из модификатора: `Real x(start = 1.0)`.
    pub start: Option<Expr>,
    /// Атрибут `fixed`.
    pub fixed: Option<bool>,
    /// Правая часть объявления: `parameter Real a = 1.0`.
    pub binding: Option<Expr>,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EquationItem {
    pub lhs: Expr,
    pub rhs: Expr,
}

#[derive(Debug, Clone, Default)]
pub struct Experiment {
    pub stop_time: Option<f64>,
    pub interval: Option<f64>,
    pub tolerance: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Model {
    pub name: String,
    pub description: Option<String>,
    pub components: Vec<Component>,
    pub equations: Vec<EquationItem>,
    pub experiment: Experiment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Bool(bool),
    /// Ссылка на компонент; в плоской модели M0 — просто имя (`x`, `body.m` — позже).
    Ref(String),
    /// Встроенная переменная времени.
    Time,
    /// Вызов функции, включая `der(x)`.
    Call(String, Vec<Expr>),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Rel(RelOp, Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    /// `if c then a else b` (elseif разворачивается во вложенные If).
    If(Box<Expr>, Box<Expr>, Box<Expr>),
}

impl Expr {
    /// Если выражение — это ровно `der(<имя>)`, вернуть имя состояния.
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

    /// Обход: содержит ли выражение вызов `der(...)`.
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

    /// Собрать имена всех переменных-ссылок в выражении.
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
        // der с двумя аргументами, der от выражения, не-der — не совпадают
        assert_eq!(Expr::Call("der".into(), vec![r("x"), r("y")]).as_der_of(), None);
        assert_eq!(Expr::Call("der".into(), vec![Expr::Number(1.0)]).as_der_of(), None);
        assert_eq!(Expr::Call("sin".into(), vec![r("x")]).as_der_of(), None);
        assert_eq!(r("x").as_der_of(), None);
    }

    #[test]
    fn contains_der_walks_every_variant() {
        let deep = Expr::If(
            Box::new(Expr::Rel(RelOp::Lt, Box::new(r("a")), Box::new(Expr::Number(0.0)))),
            Box::new(Expr::And(Box::new(Expr::Bool(true)), Box::new(Expr::Not(Box::new(r("b")))))),
            Box::new(Expr::Or(Box::new(Expr::Neg(Box::new(der("x")))), Box::new(Expr::Time))),
        );
        assert!(deep.contains_der());
        assert!(!r("x").contains_der());
        assert!(!Expr::Time.contains_der());
    }

    #[test]
    fn collect_refs_walks_every_variant() {
        let deep = Expr::If(
            Box::new(Expr::Rel(RelOp::Ge, Box::new(r("a")), Box::new(Expr::Number(1.0)))),
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
