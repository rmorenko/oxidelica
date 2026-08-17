//! Лексер среза Modelica (M0): идентификаторы, числа, строки, комментарии,
//! ключевые слова и односимвольные знаки.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),
    Number(f64),
    Str(String),
    // ключевые слова
    Model,
    End,
    Equation,
    Parameter,
    Constant,
    True,
    False,
    Annotation,
    If,
    Then,
    ElseIf,
    Else,
    And,
    Or,
    Not,
    // знаки
    LParen,
    RParen,
    Semi,
    Comma,
    Assign, // =
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Dot,
    Lt,   // <
    Le,   // <=
    Gt,   // >
    Ge,   // >=
    EqEq, // ==
    Ne,   // <>
    Eof,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Ident(s) => write!(f, "{s}"),
            Token::Number(n) => write!(f, "{n}"),
            Token::Str(s) => write!(f, "\"{s}\""),
            Token::Model => write!(f, "model"),
            Token::End => write!(f, "end"),
            Token::Equation => write!(f, "equation"),
            Token::Parameter => write!(f, "parameter"),
            Token::Constant => write!(f, "constant"),
            Token::True => write!(f, "true"),
            Token::False => write!(f, "false"),
            Token::Annotation => write!(f, "annotation"),
            Token::If => write!(f, "if"),
            Token::Then => write!(f, "then"),
            Token::ElseIf => write!(f, "elseif"),
            Token::Else => write!(f, "else"),
            Token::And => write!(f, "and"),
            Token::Or => write!(f, "or"),
            Token::Not => write!(f, "not"),
            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
            Token::Semi => write!(f, ";"),
            Token::Comma => write!(f, ","),
            Token::Assign => write!(f, "="),
            Token::Plus => write!(f, "+"),
            Token::Minus => write!(f, "-"),
            Token::Star => write!(f, "*"),
            Token::Slash => write!(f, "/"),
            Token::Caret => write!(f, "^"),
            Token::Dot => write!(f, "."),
            Token::Lt => write!(f, "<"),
            Token::Le => write!(f, "<="),
            Token::Gt => write!(f, ">"),
            Token::Ge => write!(f, ">="),
            Token::EqEq => write!(f, "=="),
            Token::Ne => write!(f, "<>"),
            Token::Eof => write!(f, "<eof>"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Spanned {
    pub token: Token,
    pub line: u32,
}

#[derive(Debug)]
pub struct LexError {
    pub message: String,
    pub line: u32,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "строка {}: {}", self.line, self.message)
    }
}

pub fn lex(source: &str) -> Result<Vec<Spanned>, LexError> {
    let mut out = Vec::new();
    let bytes: Vec<char> = source.chars().collect();
    let mut i = 0;
    let mut line: u32 = 1;

    while i < bytes.len() {
        let c = bytes[i];
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            ' ' | '\t' | '\r' => i += 1,
            '/' if bytes.get(i + 1) == Some(&'/') => {
                while i < bytes.len() && bytes[i] != '\n' {
                    i += 1;
                }
            }
            '/' if bytes.get(i + 1) == Some(&'*') => {
                i += 2;
                loop {
                    if i + 1 >= bytes.len() {
                        return Err(LexError { message: "незакрытый комментарий /*".into(), line });
                    }
                    if bytes[i] == '\n' {
                        line += 1;
                    }
                    if bytes[i] == '*' && bytes[i + 1] == '/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            '"' => {
                i += 1;
                let mut s = String::new();
                loop {
                    match bytes.get(i) {
                        None => return Err(LexError { message: "незакрытая строка".into(), line }),
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some('\\') => {
                            if let Some(&next) = bytes.get(i + 1) {
                                s.push(next);
                                i += 2;
                            } else {
                                return Err(LexError { message: "незакрытая строка".into(), line });
                            }
                        }
                        Some(&ch) => {
                            if ch == '\n' {
                                line += 1;
                            }
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                out.push(Spanned { token: Token::Str(s), line });
            }
            '0'..='9' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == '.') {
                    // точка допустима только один раз и не как разделитель имени: 1.0.x не бывает в числах
                    if bytes[i] == '.' && bytes.get(i + 1).map_or(true, |c| !c.is_ascii_digit()) {
                        break;
                    }
                    i += 1;
                }
                // экспонента: 1e-3, 2.5E+10
                if i < bytes.len() && (bytes[i] == 'e' || bytes[i] == 'E') {
                    let mut j = i + 1;
                    if matches!(bytes.get(j), Some('+') | Some('-')) {
                        j += 1;
                    }
                    if bytes.get(j).is_some_and(|c| c.is_ascii_digit()) {
                        i = j;
                        while i < bytes.len() && bytes[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let text: String = bytes[start..i].iter().collect();
                let value = text.parse::<f64>().map_err(|_| LexError {
                    message: format!("некорректное число «{text}»"),
                    line,
                })?;
                out.push(Spanned { token: Token::Number(value), line });
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_alphanumeric() || bytes[i] == '_') {
                    i += 1;
                }
                let word: String = bytes[start..i].iter().collect();
                let token = match word.as_str() {
                    "model" => Token::Model,
                    "end" => Token::End,
                    "equation" => Token::Equation,
                    "parameter" => Token::Parameter,
                    "constant" => Token::Constant,
                    "true" => Token::True,
                    "false" => Token::False,
                    "annotation" => Token::Annotation,
                    "if" => Token::If,
                    "then" => Token::Then,
                    "elseif" => Token::ElseIf,
                    "else" => Token::Else,
                    "and" => Token::And,
                    "or" => Token::Or,
                    "not" => Token::Not,
                    _ => Token::Ident(word),
                };
                out.push(Spanned { token, line });
            }
            '(' => { out.push(Spanned { token: Token::LParen, line }); i += 1; }
            ')' => { out.push(Spanned { token: Token::RParen, line }); i += 1; }
            ';' => { out.push(Spanned { token: Token::Semi, line }); i += 1; }
            ',' => { out.push(Spanned { token: Token::Comma, line }); i += 1; }
            '=' if bytes.get(i + 1) == Some(&'=') => {
                out.push(Spanned { token: Token::EqEq, line });
                i += 2;
            }
            '=' => { out.push(Spanned { token: Token::Assign, line }); i += 1; }
            '<' if bytes.get(i + 1) == Some(&'=') => {
                out.push(Spanned { token: Token::Le, line });
                i += 2;
            }
            '<' if bytes.get(i + 1) == Some(&'>') => {
                out.push(Spanned { token: Token::Ne, line });
                i += 2;
            }
            '<' => { out.push(Spanned { token: Token::Lt, line }); i += 1; }
            '>' if bytes.get(i + 1) == Some(&'=') => {
                out.push(Spanned { token: Token::Ge, line });
                i += 2;
            }
            '>' => { out.push(Spanned { token: Token::Gt, line }); i += 1; }
            '+' => { out.push(Spanned { token: Token::Plus, line }); i += 1; }
            '-' => { out.push(Spanned { token: Token::Minus, line }); i += 1; }
            '*' => { out.push(Spanned { token: Token::Star, line }); i += 1; }
            '/' => { out.push(Spanned { token: Token::Slash, line }); i += 1; }
            '^' => { out.push(Spanned { token: Token::Caret, line }); i += 1; }
            '.' => { out.push(Spanned { token: Token::Dot, line }); i += 1; }
            other => {
                return Err(LexError { message: format!("неожиданный символ «{other}»"), line });
            }
        }
    }
    out.push(Spanned { token: Token::Eof, line });
    Ok(out)
}
