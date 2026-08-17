//! Рекурсивный спуск по срезу Modelica (M0).
//!
//! Грамматика среза:
//! ```text
//! model      : "model" IDENT [STRING] { declaration } ["equation" { eq_item }]
//!              ["annotation" "(" ... ")" ";"] "end" IDENT ";"
//! declaration: ["parameter"|"constant"] "Real" IDENT ["(" attr {"," attr} ")"]
//!              ["=" expr] [STRING] ";"
//! attr       : IDENT "=" (expr | "true" | "false")
//! eq_item    : expr "=" expr [STRING] ";"
//! expr       : ["+"|"-"] term { ("+"|"-") term }
//! term       : factor { ("*"|"/") factor }
//! factor     : primary ["^" primary]
//! primary    : NUMBER | IDENT ["(" [expr {"," expr}] ")"] | "(" expr ")" | "time"
//! ```
//! Приоритеты соответствуют спецификации Modelica: `^` выше `*`/`/`,
//! унарный минус — на уровне сложения (`-a*b` = `-(a*b)`).

use crate::ast::*;
use crate::lexer::{lex, Spanned, Token};
use std::fmt;

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub line: u32,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "строка {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

pub fn parse_model(source: &str) -> Result<Model, ParseError> {
    let tokens = lex(source).map_err(|e| ParseError { message: e.message, line: e.line })?;
    Parser { tokens, pos: 0 }.model()
}

struct Parser {
    tokens: Vec<Spanned>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    fn line(&self) -> u32 {
        self.tokens[self.pos].line
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].token.clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, expected: &Token, context: &str) -> Result<(), ParseError> {
        if self.peek() == expected {
            self.bump();
            Ok(())
        } else {
            Err(self.err(format!("ожидалось «{expected}» ({context}), найдено «{}»", self.peek())))
        }
    }

    fn err(&self, message: String) -> ParseError {
        ParseError { message, line: self.line() }
    }

    fn ident(&mut self, context: &str) -> Result<String, ParseError> {
        match self.peek().clone() {
            Token::Ident(name) => {
                self.bump();
                Ok(name)
            }
            other => Err(self.err(format!("ожидался идентификатор ({context}), найдено «{other}»"))),
        }
    }

    fn opt_string(&mut self) -> Option<String> {
        if let Token::Str(s) = self.peek().clone() {
            self.bump();
            Some(s)
        } else {
            None
        }
    }

    fn model(&mut self) -> Result<Model, ParseError> {
        self.expect(&Token::Model, "начало модели")?;
        let name = self.ident("имя модели")?;
        let description = self.opt_string();

        let mut components = Vec::new();
        let mut equations = Vec::new();
        let mut experiment = Experiment::default();
        let mut in_equations = false;

        loop {
            match self.peek() {
                Token::End => {
                    self.bump();
                    let end_name = self.ident("имя после end")?;
                    if end_name != name {
                        return Err(self.err(format!(
                            "end {end_name}; не совпадает с именем модели {name}"
                        )));
                    }
                    self.expect(&Token::Semi, "точка с запятой после end")?;
                    break;
                }
                Token::Equation => {
                    self.bump();
                    in_equations = true;
                }
                Token::Annotation => {
                    self.parse_annotation(&mut experiment)?;
                }
                Token::Eof => return Err(self.err("неожиданный конец файла: нет end".into())),
                _ => {
                    if in_equations {
                        equations.push(self.equation_item()?);
                    } else {
                        components.push(self.declaration()?);
                    }
                }
            }
        }

        Ok(Model { name, description, components, equations, experiment })
    }

    fn declaration(&mut self) -> Result<Component, ParseError> {
        let variability = match self.peek() {
            Token::Parameter => {
                self.bump();
                Variability::Parameter
            }
            Token::Constant => {
                self.bump();
                Variability::Constant
            }
            _ => Variability::Continuous,
        };

        let type_name = self.ident("тип компонента")?;
        if type_name != "Real" {
            return Err(self.err(format!("M0 поддерживает только тип Real, найден «{type_name}»")));
        }
        let name = self.ident("имя компонента")?;

        let mut start = None;
        let mut fixed = None;
        if self.peek() == &Token::LParen {
            self.bump();
            loop {
                let attr = self.ident("имя атрибута")?;
                self.expect(&Token::Assign, "= в атрибуте")?;
                match attr.as_str() {
                    "start" => start = Some(self.expr()?),
                    "fixed" => {
                        fixed = Some(match self.bump() {
                            Token::True => true,
                            Token::False => false,
                            other => {
                                return Err(self.err(format!(
                                    "fixed ожидает true/false, найдено «{other}»"
                                )))
                            }
                        });
                    }
                    other => {
                        return Err(self.err(format!("неизвестный атрибут «{other}» (M0: start, fixed)")));
                    }
                }
                match self.peek() {
                    Token::Comma => {
                        self.bump();
                    }
                    Token::RParen => {
                        self.bump();
                        break;
                    }
                    other => {
                        return Err(self.err(format!("ожидалась , или ) в атрибутах, найдено «{other}»")))
                    }
                }
            }
        }

        let binding = if self.peek() == &Token::Assign {
            self.bump();
            Some(self.expr()?)
        } else {
            None
        };

        let description = self.opt_string();
        self.expect(&Token::Semi, "точка с запятой после объявления")?;

        Ok(Component { name, variability, start, fixed, binding, description })
    }

    fn equation_item(&mut self) -> Result<EquationItem, ParseError> {
        let lhs = self.expr()?;
        self.expect(&Token::Assign, "= в уравнении")?;
        let rhs = self.expr()?;
        self.opt_string();
        self.expect(&Token::Semi, "точка с запятой после уравнения")?;
        Ok(EquationItem { lhs, rhs })
    }

    /// `annotation ( ... ) ;` — разбираем толерантно: из всего содержимого
    /// извлекаем только experiment(StopTime=…, Interval=…, Tolerance=…),
    /// остальное пропускаем по балансу скобок.
    fn parse_annotation(&mut self, experiment: &mut Experiment) -> Result<(), ParseError> {
        self.expect(&Token::Annotation, "annotation")?;
        self.expect(&Token::LParen, "скобка после annotation")?;
        let mut depth = 1usize;
        while depth > 0 {
            match self.peek().clone() {
                Token::Eof => return Err(self.err("незакрытая annotation".into())),
                Token::LParen => {
                    depth += 1;
                    self.bump();
                }
                Token::RParen => {
                    depth -= 1;
                    self.bump();
                }
                Token::Ident(name) if depth == 1 && name == "experiment" => {
                    self.bump();
                    self.expect(&Token::LParen, "скобка после experiment")?;
                    loop {
                        let key = self.ident("параметр experiment")?;
                        self.expect(&Token::Assign, "= в experiment")?;
                        let value = self.number_literal()?;
                        match key.as_str() {
                            "StopTime" => experiment.stop_time = Some(value),
                            "Interval" => experiment.interval = Some(value),
                            "Tolerance" => experiment.tolerance = Some(value),
                            "StartTime" => {
                                if value != 0.0 {
                                    return Err(self.err("M0: StartTime должен быть 0".into()));
                                }
                            }
                            _ => {} // незнакомые ключи молча пропускаем
                        }
                        match self.bump() {
                            Token::Comma => continue,
                            Token::RParen => break,
                            other => {
                                return Err(self.err(format!(
                                    "ожидалась , или ) в experiment, найдено «{other}»"
                                )))
                            }
                        }
                    }
                }
                _ => {
                    self.bump();
                }
            }
        }
        self.expect(&Token::Semi, "точка с запятой после annotation")?;
        Ok(())
    }

    fn number_literal(&mut self) -> Result<f64, ParseError> {
        let negative = if self.peek() == &Token::Minus {
            self.bump();
            true
        } else {
            false
        };
        match self.bump() {
            Token::Number(n) => Ok(if negative { -n } else { n }),
            other => Err(self.err(format!("ожидалось число, найдено «{other}»"))),
        }
    }

    // --- выражения ---
    // Иерархия по спецификации Modelica:
    // expr → if | logical_or; or → and → not → relation → арифметика.

    fn expr(&mut self) -> Result<Expr, ParseError> {
        if self.peek() == &Token::If {
            return self.if_expression();
        }
        self.logical_or()
    }

    /// `if c then a {elseif c2 then b} else d` → вложенные `If`.
    fn if_expression(&mut self) -> Result<Expr, ParseError> {
        self.expect(&Token::If, "if")?;
        self.if_body()
    }

    /// Общее тело для if и elseif: `<условие> then <выражение> (elseif …| else …)`.
    fn if_body(&mut self) -> Result<Expr, ParseError> {
        let condition = self.expr()?;
        self.expect(&Token::Then, "then после условия")?;
        let then_branch = self.expr()?;
        let else_branch = match self.bump() {
            Token::ElseIf => self.if_body()?,
            Token::Else => self.expr()?,
            other => {
                return Err(self.err(format!("ожидалось else/elseif, найдено «{other}»")));
            }
        };
        Ok(Expr::If(Box::new(condition), Box::new(then_branch), Box::new(else_branch)))
    }

    fn logical_or(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.logical_and()?;
        while self.peek() == &Token::Or {
            self.bump();
            let rhs = self.logical_and()?;
            lhs = Expr::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn logical_and(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.logical_not()?;
        while self.peek() == &Token::And {
            self.bump();
            let rhs = self.logical_not()?;
            lhs = Expr::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn logical_not(&mut self) -> Result<Expr, ParseError> {
        if self.peek() == &Token::Not {
            self.bump();
            let inner = self.logical_not()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.relation()
    }

    fn relation(&mut self) -> Result<Expr, ParseError> {
        let lhs = self.arith()?;
        let op = match self.peek() {
            Token::Lt => RelOp::Lt,
            Token::Le => RelOp::Le,
            Token::Gt => RelOp::Gt,
            Token::Ge => RelOp::Ge,
            Token::EqEq => RelOp::Eq,
            Token::Ne => RelOp::Ne,
            _ => return Ok(lhs),
        };
        self.bump();
        let rhs = self.arith()?;
        Ok(Expr::Rel(op, Box::new(lhs), Box::new(rhs)))
    }

    fn arith(&mut self) -> Result<Expr, ParseError> {
        // унарный знак уровня сложения
        let leading_neg = match self.peek() {
            Token::Minus => {
                self.bump();
                true
            }
            Token::Plus => {
                self.bump();
                false
            }
            _ => false,
        };
        let mut lhs = self.term()?;
        if leading_neg {
            lhs = Expr::Neg(Box::new(lhs));
        }
        loop {
            let op = match self.peek() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let rhs = self.term()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn term(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.factor()?;
        loop {
            let op = match self.peek() {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                _ => break,
            };
            self.bump();
            let rhs = self.factor()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn factor(&mut self) -> Result<Expr, ParseError> {
        let base = self.primary()?;
        if self.peek() == &Token::Caret {
            self.bump();
            let exponent = self.primary()?;
            return Ok(Expr::Bin(BinOp::Pow, Box::new(base), Box::new(exponent)));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().clone() {
            Token::Number(n) => {
                self.bump();
                Ok(Expr::Number(n))
            }
            Token::True => {
                self.bump();
                Ok(Expr::Bool(true))
            }
            Token::False => {
                self.bump();
                Ok(Expr::Bool(false))
            }
            Token::LParen => {
                self.bump();
                let inner = self.expr()?;
                self.expect(&Token::RParen, "закрывающая скобка")?;
                Ok(inner)
            }
            Token::Ident(name) => {
                self.bump();
                if name == "time" {
                    return Ok(Expr::Time);
                }
                if self.peek() == &Token::LParen {
                    self.bump();
                    let mut args = Vec::new();
                    if self.peek() != &Token::RParen {
                        loop {
                            args.push(self.expr()?);
                            match self.peek() {
                                Token::Comma => {
                                    self.bump();
                                }
                                _ => break,
                            }
                        }
                    }
                    self.expect(&Token::RParen, "закрывающая скобка вызова")?;
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Ref(name))
                }
            }
            other => Err(self.err(format!("неожиданный токен в выражении: «{other}»"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_model() {
        let m = parse_model("model M Real x(start=1); equation der(x) = -x; end M;").unwrap();
        assert_eq!(m.name, "M");
        assert_eq!(m.components.len(), 1);
        assert_eq!(m.equations.len(), 1);
        assert_eq!(m.equations[0].lhs.as_der_of(), Some("x"));
    }

    #[test]
    fn respects_precedence() {
        // -a*b^2 == -((a)*(b^2))
        let m = parse_model("model M Real a; Real b; Real c; equation c = -a*b^2; end M;").unwrap();
        let rhs = &m.equations[0].rhs;
        match rhs {
            Expr::Neg(inner) => match inner.as_ref() {
                Expr::Bin(BinOp::Mul, _, r) => {
                    assert!(matches!(r.as_ref(), Expr::Bin(BinOp::Pow, _, _)));
                }
                other => panic!("ожидалось умножение, получено {other:?}"),
            },
            other => panic!("ожидался унарный минус, получено {other:?}"),
        }
    }

    #[test]
    fn extracts_experiment() {
        let m = parse_model(
            "model M Real x; equation x = time; \
             annotation(uses(Modelica(version=\"4.0.0\")), experiment(StopTime=5.0, Interval=0.01)); \
             end M;",
        )
        .unwrap();
        assert_eq!(m.experiment.stop_time, Some(5.0));
        assert_eq!(m.experiment.interval, Some(0.01));
    }

    #[test]
    fn rejects_mismatched_end() {
        assert!(parse_model("model A Real x; equation x = 1; end B;").is_err());
    }

    #[test]
    fn parses_if_elseif_chain() {
        let m = parse_model(
            "model M Real x; Real y; equation \
             y = if x > 1 then 1 elseif x < -1 then -1 else x; end M;",
        )
        .unwrap();
        // elseif развернулся во вложенный If в ветке else
        match &m.equations[0].rhs {
            Expr::If(_, _, else_branch) => {
                assert!(matches!(else_branch.as_ref(), Expr::If(_, _, _)));
            }
            other => panic!("ожидался If, получено {other:?}"),
        }
    }

    #[test]
    fn parses_logic_precedence() {
        // a or b and not c  ==  a or (b and (not c))
        let m = parse_model(
            "model M Real a; Real b; Real c; Real y; equation \
             y = if a > 0 or b > 0 and not c > 0 then 1 else 0; end M;",
        )
        .unwrap();
        match &m.equations[0].rhs {
            Expr::If(cond, _, _) => match cond.as_ref() {
                Expr::Or(_, rhs) => assert!(matches!(rhs.as_ref(), Expr::And(_, _))),
                other => panic!("ожидался Or на верхнем уровне, получено {other:?}"),
            },
            other => panic!("ожидался If, получено {other:?}"),
        }
    }
}
