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

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<Token> {
        lex(source).unwrap().into_iter().map(|s| s.token).collect()
    }

    #[test]
    fn display_covers_every_token() {
        let all = [
            Token::Ident("x".into()),
            Token::Number(1.5),
            Token::Str("s".into()),
            Token::Model,
            Token::End,
            Token::Equation,
            Token::Parameter,
            Token::Constant,
            Token::True,
            Token::False,
            Token::Annotation,
            Token::If,
            Token::Then,
            Token::ElseIf,
            Token::Else,
            Token::And,
            Token::Or,
            Token::Not,
            Token::LParen,
            Token::RParen,
            Token::Semi,
            Token::Comma,
            Token::Assign,
            Token::Plus,
            Token::Minus,
            Token::Star,
            Token::Slash,
            Token::Caret,
            Token::Dot,
            Token::Lt,
            Token::Le,
            Token::Gt,
            Token::Ge,
            Token::EqEq,
            Token::Ne,
            Token::Eof,
        ];
        let rendered: Vec<String> = all.iter().map(|t| t.to_string()).collect();
        assert!(rendered.iter().all(|s| !s.is_empty()));
        assert_eq!(rendered[rendered.len() - 1], "<eof>");
    }

    #[test]
    fn lexes_operators_and_keywords() {
        assert_eq!(
            tokens("== <= >= <> < > = if then elseif else and or not true false"),
            vec![
                Token::EqEq,
                Token::Le,
                Token::Ge,
                Token::Ne,
                Token::Lt,
                Token::Gt,
                Token::Assign,
                Token::If,
                Token::Then,
                Token::ElseIf,
                Token::Else,
                Token::And,
                Token::Or,
                Token::Not,
                Token::True,
                Token::False,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn lexes_numbers_with_exponents() {
        assert_eq!(
            tokens("1e3 2.5E-2 1e+10 7 0.5"),
            vec![
                Token::Number(1e3),
                Token::Number(2.5e-2),
                Token::Number(1e10),
                Token::Number(7.0),
                Token::Number(0.5),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn dot_after_number_is_separate_token() {
        // "1.x" — это Number(1), Dot, Ident: точка не съедается числом
        assert_eq!(
            tokens("1.x"),
            vec![Token::Number(1.0), Token::Dot, Token::Ident("x".into()), Token::Eof]
        );
    }

    #[test]
    fn rejects_double_dot_number() {
        let e = lex("1.2.3").unwrap_err();
        assert!(e.message.contains("некорректное число"), "{}", e.message);
    }

    #[test]
    fn lexes_strings_with_escapes_and_newlines() {
        assert_eq!(
            tokens(r#""a\"b" "line1
line2""#),
            vec![Token::Str("a\"b".into()), Token::Str("line1\nline2".into()), Token::Eof]
        );
    }

    #[test]
    fn skips_comments_and_counts_lines() {
        let spanned = lex("// комментарий\n/* блок\nещё */ x").unwrap();
        assert_eq!(spanned[0].token, Token::Ident("x".into()));
        assert_eq!(spanned[0].line, 3);
    }

    #[test]
    fn error_reports_line_number() {
        let e = lex("x\ny\n#").unwrap_err();
        assert_eq!(e.line, 3);
        assert!(e.to_string().contains("строка 3"));
        assert!(e.message.contains("неожиданный символ"));
    }

    #[test]
    fn rejects_unclosed_constructs() {
        assert!(lex("/* без конца").unwrap_err().message.contains("незакрытый комментарий"));
        assert!(lex("\"без конца").unwrap_err().message.contains("незакрытая строка"));
        assert!(lex("\"хвост\\").unwrap_err().message.contains("незакрытая строка"));
    }
}
