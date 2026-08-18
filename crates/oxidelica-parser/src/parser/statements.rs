//! Algorithm sections: assignments and the control flow around them.

use super::*;

impl Parser {
    /// An algorithm section: assignments, `if` and `for` statements, up
    /// to whatever ends the section.
    pub(super) fn statements(&mut self) -> Result<Vec<Statement>, ParseError> {
        let mut out = Vec::new();
        loop {
            match self.peek() {
                Token::Ident(_) => {
                    let target = self.component_ref()?;
                    // `c[i] := ...` assigns one element of an array.
                    let mut subscripts = Vec::new();
                    if self.peek() == &Token::LBracket {
                        self.bump();
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
                    }
                    self.expect(&Token::Becomes, "`:=` in assignment")?;
                    let value = self.expr()?;
                    self.opt_string();
                    if self.peek() == &Token::Annotation {
                        self.annotation_body(&mut Experiment::default())?;
                    }
                    self.expect(&Token::Semi, "semicolon after assignment")?;
                    out.push(Statement::Assign(target, subscripts, value));
                }
                // `(a, , c) := f(...);` — nothing else in an algorithm
                // starts with a parenthesis.
                Token::LParen => {
                    self.bump();
                    let mut targets = Vec::new();
                    loop {
                        match self.peek() {
                            Token::Comma | Token::RParen => targets.push(None),
                            _ => targets.push(Some(self.ident("target of a tuple assignment")?)),
                        }
                        match self.bump() {
                            Token::Comma => continue,
                            Token::RParen => break,
                            other => {
                                return Err(self.err(format!(
                                    "expected `,` or `)` in a tuple of targets, found `{other}`"
                                )))
                            }
                        }
                    }
                    self.expect(&Token::Becomes, "`:=` after the tuple of targets")?;
                    let value = self.expr()?;
                    self.opt_string();
                    if self.peek() == &Token::Annotation {
                        self.annotation_body(&mut Experiment::default())?;
                    }
                    self.expect(&Token::Semi, "semicolon after assignment")?;
                    out.push(Statement::TupleAssign(targets, value));
                }
                Token::If => out.push(self.if_statement()?),
                Token::For => out.push(self.for_statement()?),
                Token::When => out.push(self.when_statement()?),
                Token::While => {
                    self.bump();
                    let condition = self.expr()?;
                    self.expect(&Token::Loop, "loop after the condition of a while")?;
                    let body = self.statements()?;
                    self.expect(&Token::End, "end closing the while")?;
                    self.expect(&Token::While, "while after end")?;
                    self.expect(&Token::Semi, "semicolon after end while")?;
                    out.push(Statement::While(condition, body));
                }
                Token::Break => {
                    self.bump();
                    self.expect(&Token::Semi, "semicolon after break")?;
                    out.push(Statement::Break);
                }
                Token::Return => {
                    self.bump();
                    self.expect(&Token::Semi, "semicolon after return")?;
                    out.push(Statement::Return);
                }
                _ => break,
            }
        }
        Ok(out)
    }

    /// `when c then … elsewhen … end when;` inside an algorithm.
    pub(super) fn when_statement(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::When, "when")?;
        let mut branches = Vec::new();
        loop {
            let condition = self.expr()?;
            self.expect(&Token::Then, "then after a when condition")?;
            let body = self.statements()?;
            // A vector condition is a branch apiece here too, and for
            // the same reason.
            match condition {
                Expr::Array(conditions) if !conditions.is_empty() => {
                    for condition in conditions {
                        branches.push(StatementBranch {
                            condition: Some(condition),
                            body: body.clone(),
                        });
                    }
                }
                condition => branches.push(StatementBranch {
                    condition: Some(condition),
                    body,
                }),
            }
            if self.peek() == &Token::ElseWhen {
                self.bump();
                continue;
            }
            break;
        }
        self.expect(&Token::End, "end closing the when")?;
        self.expect(&Token::When, "when after end")?;
        self.expect(&Token::Semi, "semicolon after end when")?;
        Ok(Statement::When(branches))
    }

    /// `if c then … elseif … else … end if;` inside an algorithm.
    pub(super) fn if_statement(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::If, "if")?;
        let mut branches = Vec::new();
        loop {
            let condition = self.expr()?;
            self.expect(&Token::Then, "then after the condition of an if statement")?;
            branches.push(StatementBranch {
                condition: Some(condition),
                body: self.statements()?,
            });
            match self.peek() {
                Token::ElseIf => {
                    self.bump();
                }
                Token::Else => {
                    self.bump();
                    branches.push(StatementBranch {
                        condition: None,
                        body: self.statements()?,
                    });
                    break;
                }
                _ => break,
            }
        }
        self.expect(&Token::End, "end if")?;
        self.expect(&Token::If, "if after end")?;
        self.expect(&Token::Semi, "semicolon after end if")?;
        Ok(Statement::If(branches))
    }

    /// `for i in lo:hi loop … end for;` inside an algorithm.
    pub(super) fn for_statement(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::For, "for")?;
        let variable = self.ident("loop variable")?;
        self.expect(&Token::In, "in after the loop variable")?;
        let Expr::Range(lower, step, upper) = self.expr()? else {
            return Err(self.err("a loop needs a range: `for i in 1:n`".into()));
        };
        if step.is_some() {
            return Err(self.err("a loop range with a step is not supported yet".into()));
        }
        let (lower, upper) = (*lower, *upper);
        self.expect(&Token::Loop, "loop after the range")?;
        let body = self.statements()?;
        self.expect(&Token::End, "end for")?;
        self.expect(&Token::For, "for after end")?;
        self.expect(&Token::Semi, "semicolon after end for")?;
        if body.is_empty() {
            return Err(self.err("for statement has no body".into()));
        }
        Ok(Statement::For(variable, (lower, upper), body))
    }
}
