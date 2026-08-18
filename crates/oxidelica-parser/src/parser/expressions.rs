//! Expressions, in order of precedence, down to a primary.

use super::*;

impl Parser {
    pub(super) fn expr(&mut self) -> Result<Expr, ParseError> {
        if self.peek() == &Token::If {
            return self.if_expression();
        }
        let first = self.logical_or()?;
        // `a:b` and `a:step:b` are vector values. The colon does double
        // duty in the grammar; contexts that need a plain expression
        // (loop headers) take the range apart themselves.
        if self.peek() != &Token::Colon {
            return Ok(first);
        }
        self.bump();
        let second = self.logical_or()?;
        if self.peek() != &Token::Colon {
            return Ok(Expr::Range(Box::new(first), None, Box::new(second)));
        }
        self.bump();
        let third = self.logical_or()?;
        Ok(Expr::Range(
            Box::new(first),
            Some(Box::new(second)),
            Box::new(third),
        ))
    }

    /// `if c then a {elseif c2 then b} else d` — becomes nested `If`s.
    pub(super) fn if_expression(&mut self) -> Result<Expr, ParseError> {
        self.expect(&Token::If, "if")?;
        self.if_body()
    }

    /// Shared body for if and elseif:
    /// `<condition> then <expression> (elseif … | else …)`.
    pub(super) fn if_body(&mut self) -> Result<Expr, ParseError> {
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

    pub(super) fn logical_or(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.logical_and()?;
        while self.peek() == &Token::Or {
            self.bump();
            let rhs = self.logical_and()?;
            lhs = Expr::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    pub(super) fn logical_and(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.logical_not()?;
        while self.peek() == &Token::And {
            self.bump();
            let rhs = self.logical_not()?;
            lhs = Expr::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    pub(super) fn logical_not(&mut self) -> Result<Expr, ParseError> {
        if self.peek() == &Token::Not {
            self.bump();
            let inner = self.logical_not()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.relation()
    }

    pub(super) fn relation(&mut self) -> Result<Expr, ParseError> {
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

    pub(super) fn arith(&mut self) -> Result<Expr, ParseError> {
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
            let (op, elementwise) = match self.peek() {
                Token::Plus => (BinOp::Add, false),
                Token::Minus => (BinOp::Sub, false),
                Token::DotPlus => (BinOp::Add, true),
                Token::DotMinus => (BinOp::Sub, true),
                _ => break,
            };
            self.bump();
            let rhs = self.term()?;
            lhs = if elementwise {
                Expr::Elementwise(op, Box::new(lhs), Box::new(rhs))
            } else {
                Expr::Bin(op, Box::new(lhs), Box::new(rhs))
            };
        }
        Ok(lhs)
    }

    pub(super) fn term(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.factor()?;
        loop {
            let (op, elementwise) = match self.peek() {
                Token::Star => (BinOp::Mul, false),
                Token::Slash => (BinOp::Div, false),
                Token::DotStar => (BinOp::Mul, true),
                Token::DotSlash => (BinOp::Div, true),
                _ => break,
            };
            self.bump();
            let rhs = self.factor()?;
            lhs = if elementwise {
                Expr::Elementwise(op, Box::new(lhs), Box::new(rhs))
            } else {
                Expr::Bin(op, Box::new(lhs), Box::new(rhs))
            };
        }
        Ok(lhs)
    }

    pub(super) fn factor(&mut self) -> Result<Expr, ParseError> {
        let base = self.primary()?;
        match self.peek() {
            Token::Caret => {
                self.bump();
                let exponent = self.primary()?;
                Ok(Expr::Bin(BinOp::Pow, Box::new(base), Box::new(exponent)))
            }
            Token::DotCaret => {
                self.bump();
                let exponent = self.primary()?;
                Ok(Expr::Elementwise(
                    BinOp::Pow,
                    Box::new(base),
                    Box::new(exponent),
                ))
            }
            _ => Ok(base),
        }
    }

    pub(super) fn primary(&mut self) -> Result<Expr, ParseError> {
        if self.in_subscript && self.peek() == &Token::End {
            self.bump();
            return Ok(Expr::EndSubscript);
        }
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
            Token::LBrace => {
                self.bump();
                let mut items = Vec::new();
                if self.peek() != &Token::RBrace {
                    loop {
                        let item = self.expr()?;
                        // `{expr for i in range}` builds by iterating.
                        if items.is_empty() && self.peek() == &Token::For {
                            self.bump();
                            let variable = self.ident("iterator variable")?;
                            self.expect(&Token::In, "in after the iterator")?;
                            let range = self.expr()?;
                            self.expect(&Token::RBrace, "closing brace of a comprehension")?;
                            return Ok(Expr::Comprehension(
                                Box::new(item),
                                variable,
                                Box::new(range),
                            ));
                        }
                        items.push(item);
                        match self.peek() {
                            Token::Comma => {
                                self.bump();
                            }
                            _ => break,
                        }
                    }
                }
                self.expect(&Token::RBrace, "closing brace of an array")?;
                Ok(Expr::Array(items))
            }
            Token::LBracket => {
                // `[a, b; c, d]`: a matrix by rows. A single row is a
                // row matrix; the semicolon starts the next row.
                self.bump();
                let mut rows = Vec::new();
                let mut row = Vec::new();
                loop {
                    row.push(self.expr()?);
                    match self.peek() {
                        Token::Comma => {
                            self.bump();
                        }
                        Token::Semi => {
                            self.bump();
                            rows.push(std::mem::take(&mut row));
                        }
                        Token::RBracket => {
                            self.bump();
                            rows.push(row);
                            break;
                        }
                        other => {
                            return Err(self.err(format!(
                                "expected `,`, `;` or `]` in a matrix, found `{other}`"
                            )))
                        }
                    }
                }
                Ok(Expr::MatrixRows(rows))
            }
            Token::Ident(name) => {
                self.bump();
                if name == "time" {
                    return Ok(Expr::Time);
                }
                let mut name = name;
                while self.peek() == &Token::Dot {
                    self.bump();
                    name.push('.');
                    name.push_str(&self.ident("name after dot")?);
                }
                if self.peek() == &Token::LBracket {
                    self.bump();
                    let mut subscripts = Vec::new();
                    loop {
                        subscripts.push(self.subscript()?);
                        match self.bump() {
                            Token::Comma => continue,
                            Token::RBracket => break,
                            other => {
                                return Err(self.err(format!(
                                    "expected `,` or `]` in a subscript, found `{other}`"
                                )))
                            }
                        }
                    }
                    let indexed = Expr::Index(Box::new(Expr::Ref(name)), subscripts);
                    if self.peek() == &Token::Dot {
                        self.bump();
                        let mut path = self.ident("member after `.`")?;
                        while self.peek() == &Token::Dot {
                            self.bump();
                            path.push('.');
                            path.push_str(&self.ident("member after `.`")?);
                        }
                        return Ok(Expr::Member(Box::new(indexed), path));
                    }
                    return Ok(indexed);
                }
                // A call may be qualified: `Lib.reverse(v)` names a
                // function the way everything else is named.
                if self.peek() == &Token::LParen {
                    self.bump();
                    let mut args = Vec::new();
                    if self.peek() != &Token::RParen {
                        loop {
                            // `precision = 6`: an argument passed by
                            // name. One token of lookahead tells it
                            // from an expression starting with a name.
                            let arg = if let Token::Ident(keyword) = self.peek() {
                                if self.peek_at(1) == &Token::Assign {
                                    let keyword = keyword.clone();
                                    self.bump();
                                    self.bump();
                                    Expr::NamedArg(keyword, Box::new(self.expr()?))
                                } else {
                                    self.expr()?
                                }
                            } else {
                                self.expr()?
                            };
                            // `sum(expr for i in range)`: the reduction
                            // form is the comprehension passed whole.
                            if args.is_empty() && self.peek() == &Token::For {
                                self.bump();
                                let variable = self.ident("iterator variable")?;
                                self.expect(&Token::In, "in after the iterator")?;
                                let range = self.expr()?;
                                args.push(Expr::Comprehension(
                                    Box::new(arg),
                                    variable,
                                    Box::new(range),
                                ));
                                break;
                            }
                            args.push(arg);
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

    /// One subscript: an expression, a bare `:` for the whole
    /// dimension, or `end` for its length.
    pub(super) fn subscript(&mut self) -> Result<Expr, ParseError> {
        if self.peek() == &Token::Colon {
            self.bump();
            return Ok(Expr::ColonSubscript);
        }
        // `end` may sit inside arithmetic (`end - 1`), so the whole
        // subscript parses as an expression with the keyword allowed.
        let outer = self.in_subscript;
        self.in_subscript = true;
        let parsed = self.expr();
        self.in_subscript = outer;
        parsed
    }

    /// A dotted class name: `Modelica.Electrical.Analog.Basic.Resistor`.
    pub(super) fn dotted_name(&mut self, context: &str) -> Result<String, ParseError> {
        let mut name = self.ident(context)?;
        while self.peek() == &Token::Dot {
            self.bump();
            name.push('.');
            name.push_str(&self.ident(context)?);
        }
        Ok(name)
    }

    pub(super) fn number_literal(&mut self) -> Result<f64, ParseError> {
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
}
