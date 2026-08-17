//! Recursive-descent parser for the M0 Modelica slice.
//!
//! Slice grammar:
//! ```text
//! model      : "model" IDENT [STRING] { declaration } ["equation" { eq_item }]
//!              ["annotation" "(" ... ")" ";"] "end" IDENT ";"
//! declaration: ["parameter"|"constant"] "Real" IDENT ["(" attr {"," attr} ")"]
//!              ["=" expr] [STRING] ";"
//! attr       : IDENT "=" (expr | "true" | "false")
//! eq_item    : expr "=" expr [STRING] ";"
//! expr       : if_expr | logical_or
//! if_expr    : "if" expr "then" expr {"elseif" expr "then" expr} "else" expr
//! logical_or : logical_and { "or" logical_and }
//! logical_and: logical_not { "and" logical_not }
//! logical_not: ["not"] relation
//! relation   : arith [("<"|"<="|">"|">="|"=="|"<>") arith]
//! arith      : ["+"|"-"] term { ("+"|"-") term }
//! term       : factor { ("*"|"/") factor }
//! factor     : primary ["^" primary]
//! primary    : NUMBER | "true" | "false" | IDENT ["(" [expr {"," expr}] ")"]
//!            | "(" expr ")" | "time"
//! ```
//! Precedence follows the Modelica specification: `^` binds tighter than
//! `*`/`/`; unary minus lives at the addition level (`-a*b` = `-(a*b)`).

use crate::ast::*;
use crate::lexer::{lex, Spanned, Token};
use std::fmt;

/// A parse error with its source line.
#[derive(Debug)]
pub struct ParseError {
    /// Human-readable description of the problem.
    pub message: String,
    /// 1-based line number where the error occurred.
    pub line: u32,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Parse a single flat model from Modelica source text.
pub fn parse_model(source: &str) -> Result<Model, ParseError> {
    let tokens = lex(source).map_err(|e| ParseError {
        message: e.message,
        line: e.line,
    })?;
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
            Err(self.err(format!(
                "expected `{expected}` ({context}), found `{}`",
                self.peek()
            )))
        }
    }

    fn err(&self, message: String) -> ParseError {
        ParseError {
            message,
            line: self.line(),
        }
    }

    fn ident(&mut self, context: &str) -> Result<String, ParseError> {
        match self.peek().clone() {
            Token::Ident(name) => {
                self.bump();
                Ok(name)
            }
            other => Err(self.err(format!("expected identifier ({context}), found `{other}`"))),
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
        self.expect(&Token::Model, "model keyword")?;
        let name = self.ident("model name")?;
        let description = self.opt_string();

        let mut components = Vec::new();
        let mut equations = Vec::new();
        let mut experiment = Experiment::default();
        let mut in_equations = false;

        loop {
            match self.peek() {
                Token::End => {
                    self.bump();
                    let end_name = self.ident("name after end")?;
                    if end_name != name {
                        return Err(
                            self.err(format!("end {end_name}; does not match model name {name}"))
                        );
                    }
                    self.expect(&Token::Semi, "semicolon after end")?;
                    break;
                }
                Token::Equation => {
                    self.bump();
                    in_equations = true;
                }
                Token::Annotation => {
                    self.parse_annotation(&mut experiment)?;
                }
                Token::Eof => return Err(self.err("unexpected end of file: missing end".into())),
                _ => {
                    if in_equations {
                        equations.push(self.equation_item()?);
                    } else {
                        components.push(self.declaration()?);
                    }
                }
            }
        }

        Ok(Model {
            name,
            description,
            components,
            equations,
            experiment,
        })
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

        let type_name = self.ident("component type")?;
        if type_name != "Real" {
            return Err(self.err(format!("M0 supports only type Real, found `{type_name}`")));
        }
        let name = self.ident("component name")?;

        let mut start = None;
        let mut fixed = None;
        if self.peek() == &Token::LParen {
            self.bump();
            loop {
                let attr = self.ident("attribute name")?;
                self.expect(&Token::Assign, "`=` in attribute")?;
                match attr.as_str() {
                    "start" => start = Some(self.expr()?),
                    "fixed" => {
                        fixed = Some(match self.bump() {
                            Token::True => true,
                            Token::False => false,
                            other => {
                                return Err(
                                    self.err(format!("fixed expects true/false, found `{other}`"))
                                )
                            }
                        });
                    }
                    other => {
                        return Err(
                            self.err(format!("unknown attribute `{other}` (M0: start, fixed)"))
                        );
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
                        return Err(self.err(format!(
                            "expected `,` or `)` in attributes, found `{other}`"
                        )))
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
        self.expect(&Token::Semi, "semicolon after declaration")?;

        Ok(Component {
            name,
            variability,
            start,
            fixed,
            binding,
            description,
        })
    }

    fn equation_item(&mut self) -> Result<EquationItem, ParseError> {
        let lhs = self.expr()?;
        self.expect(&Token::Assign, "`=` in equation")?;
        let rhs = self.expr()?;
        self.opt_string();
        self.expect(&Token::Semi, "semicolon after equation")?;
        Ok(EquationItem { lhs, rhs })
    }

    /// `annotation ( ... ) ;` — parsed tolerantly: only
    /// `experiment(StopTime=…, Interval=…, Tolerance=…)` is extracted,
    /// everything else is skipped by balancing parentheses.
    fn parse_annotation(&mut self, experiment: &mut Experiment) -> Result<(), ParseError> {
        self.expect(&Token::Annotation, "annotation")?;
        self.expect(&Token::LParen, "parenthesis after annotation")?;
        let mut depth = 1usize;
        while depth > 0 {
            match self.peek().clone() {
                Token::Eof => return Err(self.err("unterminated annotation".into())),
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
                    self.expect(&Token::LParen, "parenthesis after experiment")?;
                    loop {
                        let key = self.ident("experiment parameter")?;
                        self.expect(&Token::Assign, "`=` in experiment")?;
                        let value = self.number_literal()?;
                        match key.as_str() {
                            "StopTime" => experiment.stop_time = Some(value),
                            "Interval" => experiment.interval = Some(value),
                            "Tolerance" => experiment.tolerance = Some(value),
                            "StartTime" if value != 0.0 => {
                                return Err(self.err("M0: StartTime must be 0".into()));
                            }
                            "StartTime" => {}
                            _ => {} // Unknown keys are silently skipped.
                        }
                        match self.bump() {
                            Token::Comma => continue,
                            Token::RParen => break,
                            other => {
                                return Err(self.err(format!(
                                    "expected `,` or `)` in experiment, found `{other}`"
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
        self.expect(&Token::Semi, "semicolon after annotation")?;
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
            other => Err(self.err(format!("expected a number, found `{other}`"))),
        }
    }

    // --- expressions ---
    // Hierarchy per the Modelica specification:
    // expr -> if | logical_or; or -> and -> not -> relation -> arithmetic.

    fn expr(&mut self) -> Result<Expr, ParseError> {
        if self.peek() == &Token::If {
            return self.if_expression();
        }
        self.logical_or()
    }

    /// `if c then a {elseif c2 then b} else d` — becomes nested `If`s.
    fn if_expression(&mut self) -> Result<Expr, ParseError> {
        self.expect(&Token::If, "if")?;
        self.if_body()
    }

    /// Shared body for if and elseif:
    /// `<condition> then <expression> (elseif … | else …)`.
    fn if_body(&mut self) -> Result<Expr, ParseError> {
        let condition = self.expr()?;
        self.expect(&Token::Then, "then after condition")?;
        let then_branch = self.expr()?;
        let else_branch = match self.bump() {
            Token::ElseIf => self.if_body()?,
            Token::Else => self.expr()?,
            other => {
                return Err(self.err(format!("expected else/elseif, found `{other}`")));
            }
        };
        Ok(Expr::If(
            Box::new(condition),
            Box::new(then_branch),
            Box::new(else_branch),
        ))
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
        // Unary sign at the addition level.
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
                self.expect(&Token::RParen, "closing parenthesis")?;
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
                    self.expect(&Token::RParen, "closing parenthesis of call")?;
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Ref(name))
                }
            }
            other => Err(self.err(format!("unexpected token in expression: `{other}`"))),
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
        let Expr::Neg(inner) = rhs else {
            panic!("expected unary minus: {rhs:?}")
        };
        let Expr::Bin(BinOp::Mul, _, r) = inner.as_ref() else {
            panic!("expected multiplication: {inner:?}")
        };
        assert!(matches!(r.as_ref(), Expr::Bin(BinOp::Pow, _, _)));
        // Unary plus is transparent.
        let m2 = parse_model("model M Real c; equation c = +5; end M;").unwrap();
        assert_eq!(m2.equations[0].rhs, Expr::Number(5.0));
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

    fn err_of(source: &str) -> String {
        parse_model(source).unwrap_err().to_string()
    }

    #[test]
    fn error_paths_are_reported() {
        // End of file without `end`.
        assert!(err_of("model M Real x;").contains("end of file"));
        // Not Real.
        assert!(err_of("model M Integer x; end M;").contains("only type Real"));
        // Unknown attribute.
        assert!(err_of("model M Real x(min=0); end M;").contains("unknown attribute"));
        // Non-boolean fixed.
        assert!(err_of("model M Real x(fixed=1); end M;").contains("true/false"));
        // Missing comma between attributes.
        assert!(err_of("model M Real x(start=1 fixed=true); end M;").contains("`,` or `)`"));
        // Missing semicolon after an equation.
        assert!(err_of("model M Real x; equation x = 1 end M;").contains("semicolon"));
        // If without else.
        assert!(err_of("model M Real y; equation y = if 1 > 0 then 1; end M;").contains("else"));
        // Garbage in an expression.
        assert!(err_of("model M Real x; equation x = ;; end M;").contains("unexpected token"));
        // Unclosed parenthesis.
        assert!(err_of("model M Real x; equation x = (1 + 2; end M;").contains("closing"));
        // Model name is not an identifier.
        assert!(err_of("model 1 end 1;").contains("identifier"));
    }

    #[test]
    fn annotation_error_paths() {
        // Unterminated annotation (cut off while balancing parentheses).
        assert!(err_of("model M Real x; equation x = 1; annotation(uses(")
            .contains("unterminated annotation"));
        // Cut off inside experiment has its own diagnostics.
        assert!(
            err_of("model M Real x; equation x = 1; annotation(experiment(")
                .contains("experiment parameter")
        );
        // StartTime != 0 is unsupported.
        assert!(err_of(
            "model M Real x; equation x = 1; annotation(experiment(StartTime=2)); end M;"
        )
        .contains("StartTime"));
        // Not a number in experiment.
        assert!(err_of(
            "model M Real x; equation x = 1; annotation(experiment(StopTime=abc)); end M;"
        )
        .contains("expected a number"));
        // Garbage instead of `,` or `)`.
        assert!(err_of(
            "model M Real x; equation x = 1; annotation(experiment(StopTime=1 abc)); end M;"
        )
        .contains("`,` or `)` in experiment"));
    }

    #[test]
    fn annotation_accepts_start_time_zero_and_negatives() {
        let m = parse_model(
            "model M Real x; equation x = 1; \
             annotation(experiment(StartTime=0, StopTime=2, Tolerance=1e-6, Whatever=3)); end M;",
        )
        .unwrap();
        assert_eq!(m.experiment.stop_time, Some(2.0));
        assert_eq!(m.experiment.tolerance, Some(1e-6));
        // A negative literal goes through number_literal.
        let m2 = parse_model(
            "model M Real x; equation x = 1; annotation(experiment(StopTime=-1)); end M;",
        )
        .unwrap();
        assert_eq!(m2.experiment.stop_time, Some(-1.0));
    }

    #[test]
    fn parses_descriptions_and_der_on_rhs() {
        let m = parse_model(
            "model M \"a model\" Real x \"a variable\"; equation -x = der(x) \"an equation\"; end M;",
        )
        .unwrap();
        assert_eq!(m.description.as_deref(), Some("a model"));
        assert_eq!(m.components[0].description.as_deref(), Some("a variable"));
        assert_eq!(m.equations[0].rhs.as_der_of(), Some("x"));
    }

    #[test]
    fn parses_calls_booleans_and_constants() {
        let m = parse_model(
            "model M constant Real c = 2; parameter Real p = c + 1; Real y; \
             equation y = max(sin(time), 0) + (if true then p else 0); end M;",
        )
        .unwrap();
        assert_eq!(m.components[0].variability, Variability::Constant);
        assert_eq!(m.components[1].variability, Variability::Parameter);
        let mut refs = Vec::new();
        m.equations[0].rhs.collect_refs(&mut refs);
        assert_eq!(refs, vec!["p"]);
    }

    #[test]
    fn parses_if_elseif_chain() {
        let m = parse_model(
            "model M Real x; Real y; equation \
             y = if x > 1 then 1 elseif x < -1 then -1 else x; end M;",
        )
        .unwrap();
        // elseif unfolds into a nested If in the else branch.
        let Expr::If(_, _, else_branch) = &m.equations[0].rhs else {
            panic!("expected If: {:?}", m.equations[0].rhs)
        };
        assert!(matches!(else_branch.as_ref(), Expr::If(_, _, _)));
        // fixed=false parses as well.
        let m2 =
            parse_model("model M Real x(start=0, fixed=false); equation der(x)=1; end M;").unwrap();
        assert_eq!(m2.components[0].fixed, Some(false));
    }

    #[test]
    fn parses_logic_precedence() {
        // a or b and not c  ==  a or (b and (not c))
        let m = parse_model(
            "model M Real a; Real b; Real c; Real y; equation \
             y = if a > 0 or b > 0 and not c > 0 then 1 else 0; end M;",
        )
        .unwrap();
        let Expr::If(cond, _, _) = &m.equations[0].rhs else {
            panic!("expected If: {:?}", m.equations[0].rhs)
        };
        let Expr::Or(_, or_rhs) = cond.as_ref() else {
            panic!("expected Or at the top level: {cond:?}")
        };
        assert!(matches!(or_rhs.as_ref(), Expr::And(_, _)));
    }
}
