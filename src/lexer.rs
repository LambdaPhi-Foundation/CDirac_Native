//! CDirac lexer: source text -> tokens with spans.
//!
//! `var` declares, `set` reassigns, `const` binds once; `{…}`-only
//! blocks; `;` terminates; `//` comments. `=` never compares (use
//! `==`); `new`/`:`/`then`/`end`/`do` are rejected downstream with
//! migration hints. Signs never fold into numbers (`-7` is unary minus
//! on `7`); digit-led longest match keeps `3.5` one token while
//! `sc.next` splits at the dot.

use crate::ast::Span;

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Semicolon,
    Comma,
    Dot,
    Colon,
    Pipe,
    Plus,
    Minus,
    Star,
    Slash,
    Lt,
    Gt,
    LtEq,
    GtEq,
    EqEq,
    BangEq,
    Bang,
    Assign,
    AndAnd,
    OrOr,
    Question,
    Str,
    Number,
    Bool,
    Symbol,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub span: Span,
    pub str_val: String,
    pub bool_val: bool,
    pub is_float: bool,
    pub int_num: i32,
    pub float_num: f64,
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0B' | '\x0C')
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_cont(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

pub struct Lexer<'a> {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    _src: &'a str,
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<char> {
        self.chars.get(self.pos + off).copied()
    }

    fn adv(&mut self) {
        if let Some(c) = self.peek() {
            if c == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
            self.pos += 1;
        }
    }

    fn span(&self) -> Span {
        Span::new(self.line, self.col)
    }

    fn here(&self, kind: TokenKind, lexeme: &str) -> Token {
        Token {
            kind,
            lexeme: lexeme.to_string(),
            span: self.span(),
            str_val: String::new(),
            bool_val: false,
            is_float: false,
            int_num: 0,
            float_num: 0.0,
        }
    }

    fn lex_string(&mut self) -> Result<Token, String> {
        let span = self.span();
        self.adv(); // opening quote
        let mut buf = String::new();
        let mut closed = false;
        while let Some(d) = self.peek() {
            if d == '"' {
                closed = true;
                self.adv();
                break;
            } else if d == '\\' {
                self.adv();
                match self.peek() {
                    None => break,
                    Some(e) => {
                        match e {
                            'n' => buf.push('\n'),
                            't' => buf.push('\t'),
                            'r' => buf.push('\r'),
                            '"' => buf.push('"'),
                            '\\' => buf.push('\\'),
                            _ => buf.push(e),
                        }
                        self.adv();
                    }
                }
            } else {
                buf.push(d);
                self.adv();
            }
        }
        if !closed {
            return Err(format!("line {}: unterminated string literal", span.line));
        }
        Ok(Token {
            kind: TokenKind::Str,
            lexeme: buf.clone(),
            span,
            str_val: buf,
            bool_val: false,
            is_float: false,
            int_num: 0,
            float_num: 0.0,
        })
    }

    fn lex_hash(&mut self) -> Result<Token, String> {
        let span = self.span();
        let mut j = self.pos + 1;
        while j < self.chars.len()
            && !is_space(self.chars[j])
            && !matches!(
                self.chars[j],
                '{' | '}'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | ';'
                    | ','
                    | '.'
                    | ':'
                    | '|'
                    | '<'
                    | '>'
                    | '='
                    | '!'
                    | '&'
                    | '+'
                    | '-'
                    | '*'
                    | '/'
                    | '?'
                    | '"'
                    | '\''
                    | '`'
            )
        {
            j += 1;
        }
        let word: String = self.chars[self.pos..j].iter().collect();
        while self.pos < j {
            self.adv();
        }
        let low = word.to_lowercase();
        if low == "#t" || low == "#true" {
            Ok(Token {
                kind: TokenKind::Bool,
                lexeme: word,
                span,
                str_val: String::new(),
                bool_val: true,
                is_float: false,
                int_num: 0,
                float_num: 0.0,
            })
        } else if low == "#f" || low == "#false" {
            Ok(Token {
                kind: TokenKind::Bool,
                lexeme: word,
                span,
                str_val: String::new(),
                bool_val: false,
                is_float: false,
                int_num: 0,
                float_num: 0.0,
            })
        } else {
            Err(format!(
                "line {}: unknown #-literal '{}' (literals are true/false/null)",
                span.line, word
            ))
        }
    }

    /// Unsigned number, digit-led longest match (`3.5` one token;
    /// `3.foo` lexes `3` then `.` then `foo`). `>i32` falls back to float.
    fn lex_number(&mut self) -> Token {
        let span = self.span();
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.adv();
        }
        let mut is_float = false;
        if self.peek() == Some('.') && matches!(self.peek_at(1), Some(d) if d.is_ascii_digit()) {
            is_float = true;
            self.adv();
            while matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                self.adv();
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            let save = self.pos;
            self.adv();
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.adv();
            }
            if matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                is_float = true;
                while matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                    self.adv();
                }
            } else {
                self.pos = save;
            }
        }
        let word: String = self.chars[start..self.pos].iter().collect();
        if !is_float {
            if let Ok(n) = word.parse::<i32>() {
                return Token {
                    kind: TokenKind::Number,
                    lexeme: word,
                    span,
                    str_val: String::new(),
                    bool_val: false,
                    is_float: false,
                    int_num: n,
                    float_num: 0.0,
                };
            }
        }
        let x: f64 = word.parse().unwrap_or(f64::NAN);
        Token {
            kind: TokenKind::Number,
            lexeme: word,
            span,
            str_val: String::new(),
            bool_val: false,
            is_float: true,
            int_num: 0,
            float_num: x,
        }
    }

    /// `.5`-style numbers (dot followed by a digit).
    fn lex_dot_number(&mut self) -> Token {
        let span = self.span();
        let start = self.pos;
        self.adv();
        while matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
            self.adv();
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            let save = self.pos;
            self.adv();
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.adv();
            }
            if matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                while matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                    self.adv();
                }
            } else {
                self.pos = save;
            }
        }
        let word: String = self.chars[start..self.pos].iter().collect();
        let x: f64 = word.parse().unwrap_or(f64::NAN);
        Token {
            kind: TokenKind::Number,
            lexeme: word,
            span,
            str_val: String::new(),
            bool_val: false,
            is_float: true,
            int_num: 0,
            float_num: x,
        }
    }

    fn lex_ident(&mut self) -> Token {
        let span = self.span();
        let start = self.pos;
        while matches!(self.peek(), Some(c) if is_ident_cont(c)) {
            self.adv();
        }
        let word: String = self.chars[start..self.pos].iter().collect();
        Token {
            kind: TokenKind::Symbol,
            lexeme: word,
            span,
            str_val: String::new(),
            bool_val: false,
            is_float: false,
            int_num: 0,
            float_num: 0.0,
        }
    }
}

/// Tokenize CDirac source text; errors carry migration hints.
pub fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let mut lx = Lexer {
        chars: src.chars().collect(),
        pos: 0,
        line: 1,
        col: 1,
        _src: src,
    };
    let mut out = Vec::new();
    while let Some(c) = lx.peek() {
        if is_space(c) {
            lx.adv();
            continue;
        }
        if c == '/' && lx.peek_at(1) == Some('/') {
            while let Some(d) = lx.peek() {
                lx.adv();
                if d == '\n' {
                    break;
                }
            }
            continue;
        }
        match c {
            '{' => {
                out.push(lx.here(TokenKind::LBrace, "{"));
                lx.adv();
            }
            '}' => {
                out.push(lx.here(TokenKind::RBrace, "}"));
                lx.adv();
            }
            '(' => {
                out.push(lx.here(TokenKind::LParen, "("));
                lx.adv();
            }
            ')' => {
                out.push(lx.here(TokenKind::RParen, ")"));
                lx.adv();
            }
            '[' => {
                out.push(lx.here(TokenKind::LBracket, "["));
                lx.adv();
            }
            ']' => {
                out.push(lx.here(TokenKind::RBracket, "]"));
                lx.adv();
            }
            ';' => {
                out.push(lx.here(TokenKind::Semicolon, ";"));
                lx.adv();
            }
            ',' => {
                out.push(lx.here(TokenKind::Comma, ","));
                lx.adv();
            }
            '.' => {
                if matches!(lx.peek_at(1), Some(d) if d.is_ascii_digit()) {
                    out.push(lx.lex_dot_number());
                } else {
                    out.push(lx.here(TokenKind::Dot, "."));
                    lx.adv();
                }
            }
            ':' => {
                out.push(lx.here(TokenKind::Colon, ":"));
                lx.adv();
            }
            '|' => {
                if lx.peek_at(1) == Some('|') {
                    let span = lx.span();
                    lx.adv();
                    lx.adv();
                    out.push(Token {
                        kind: TokenKind::OrOr,
                        lexeme: "||".to_string(),
                        span,
                        str_val: String::new(),
                        bool_val: false,
                        is_float: false,
                        int_num: 0,
                        float_num: 0.0,
                    });
                } else {
                    out.push(lx.here(TokenKind::Pipe, "|"));
                    lx.adv();
                }
            }
            '&' => {
                if lx.peek_at(1) == Some('&') {
                    let span = lx.span();
                    lx.adv();
                    lx.adv();
                    out.push(Token {
                        kind: TokenKind::AndAnd,
                        lexeme: "&&".to_string(),
                        span,
                        str_val: String::new(),
                        bool_val: false,
                        is_float: false,
                        int_num: 0,
                        float_num: 0.0,
                    });
                } else {
                    return Err(format!(
                        "line {}: unexpected '&' (CDirac uses && for and)",
                        lx.line
                    ));
                }
            }
            '=' => {
                if lx.peek_at(1) == Some('=') {
                    let span = lx.span();
                    lx.adv();
                    lx.adv();
                    out.push(Token {
                        kind: TokenKind::EqEq,
                        lexeme: "==".to_string(),
                        span,
                        str_val: String::new(),
                        bool_val: false,
                        is_float: false,
                        int_num: 0,
                        float_num: 0.0,
                    });
                } else {
                    out.push(lx.here(TokenKind::Assign, "="));
                    lx.adv();
                }
            }
            '!' => {
                if lx.peek_at(1) == Some('=') {
                    let span = lx.span();
                    lx.adv();
                    lx.adv();
                    out.push(Token {
                        kind: TokenKind::BangEq,
                        lexeme: "!=".to_string(),
                        span,
                        str_val: String::new(),
                        bool_val: false,
                        is_float: false,
                        int_num: 0,
                        float_num: 0.0,
                    });
                } else {
                    out.push(lx.here(TokenKind::Bang, "!"));
                    lx.adv();
                }
            }
            '<' => {
                if lx.peek_at(1) == Some('=') {
                    let span = lx.span();
                    lx.adv();
                    lx.adv();
                    out.push(Token {
                        kind: TokenKind::LtEq,
                        lexeme: "<=".to_string(),
                        span,
                        str_val: String::new(),
                        bool_val: false,
                        is_float: false,
                        int_num: 0,
                        float_num: 0.0,
                    });
                } else {
                    out.push(lx.here(TokenKind::Lt, "<"));
                    lx.adv();
                }
            }
            '>' => {
                if lx.peek_at(1) == Some('=') {
                    let span = lx.span();
                    lx.adv();
                    lx.adv();
                    out.push(Token {
                        kind: TokenKind::GtEq,
                        lexeme: ">=".to_string(),
                        span,
                        str_val: String::new(),
                        bool_val: false,
                        is_float: false,
                        int_num: 0,
                        float_num: 0.0,
                    });
                } else {
                    out.push(lx.here(TokenKind::Gt, ">"));
                    lx.adv();
                }
            }
            '+' => {
                out.push(lx.here(TokenKind::Plus, "+"));
                lx.adv();
            }
            '-' => {
                if lx.peek_at(1) == Some('-') {
                    return Err(format!(
                        "line {}: found '--' (CDirac comments are //, not --)",
                        lx.line
                    ));
                }
                out.push(lx.here(TokenKind::Minus, "-"));
                lx.adv();
            }
            '*' => {
                out.push(lx.here(TokenKind::Star, "*"));
                lx.adv();
            }
            '/' => {
                out.push(lx.here(TokenKind::Slash, "/"));
                lx.adv();
            }
            '?' => {
                out.push(lx.here(TokenKind::Question, "?"));
                lx.adv();
            }
            '"' => out.push(lx.lex_string()?),
            '#' => out.push(lx.lex_hash()?),
            '\'' => {
                return Err(format!(
                    "line {}: found '\\'' (strings use \"...\"; quote syntax is not code-is-data)",
                    lx.line
                ));
            }
            '`' => {
                return Err(format!(
                    "line {}: found '`' (quote/unquote is not supported: code-is-not-data)",
                    lx.line
                ));
            }
            _ if c.is_ascii_digit() => out.push(lx.lex_number()),
            _ if is_ident_start(c) => out.push(lx.lex_ident()),
            _ => {
                return Err(format!("line {}: unexpected character '{}'", lx.line, c));
            }
        }
    }
    out.push(Token {
        kind: TokenKind::Eof,
        lexeme: String::new(),
        span: lx.span(),
        str_val: String::new(),
        bool_val: false,
        is_float: false,
        int_num: 0,
        float_num: 0.0,
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        tokenize(src)
            .unwrap()
            .iter()
            .map(|t| t.kind.clone())
            .collect()
    }

    #[test]
    fn braces_operators_spans() {
        let toks = tokenize("var x = 5;").unwrap();
        assert_eq!(toks[0].lexeme, "var");
        assert_eq!(toks[0].span, Span::new(1, 1));
        assert_eq!(toks[2].span, Span::new(1, 7));
        assert_eq!(
            kinds("fn f(x) { return x; }"),
            vec![
                TokenKind::Symbol,
                TokenKind::Symbol,
                TokenKind::LParen,
                TokenKind::Symbol,
                TokenKind::RParen,
                TokenKind::LBrace,
                TokenKind::Symbol,
                TokenKind::Symbol,
                TokenKind::Semicolon,
                TokenKind::RBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn compound_operators() {
        assert_eq!(
            kinds("a == b; c != d; e && f; g || h;"),
            vec![
                TokenKind::Symbol,
                TokenKind::EqEq,
                TokenKind::Symbol,
                TokenKind::Semicolon,
                TokenKind::Symbol,
                TokenKind::BangEq,
                TokenKind::Symbol,
                TokenKind::Semicolon,
                TokenKind::Symbol,
                TokenKind::AndAnd,
                TokenKind::Symbol,
                TokenKind::Semicolon,
                TokenKind::Symbol,
                TokenKind::OrOr,
                TokenKind::Symbol,
                TokenKind::Semicolon,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn numbers_dot_splits() {
        let toks = tokenize("3.5; sc.next; 42;").unwrap();
        assert!(toks[0].is_float);
        assert_eq!(toks[1].kind, TokenKind::Semicolon);
        assert_eq!(toks[2].lexeme, "sc");
        assert_eq!(toks[3].kind, TokenKind::Dot);
        assert_eq!(toks[4].lexeme, "next");
    }

    #[test]
    fn comments_are_slash_slash() {
        let toks = tokenize("// hi\nx = 1;").unwrap();
        assert_eq!(toks[0].lexeme, "x");
        assert!(tokenize("-- hi").is_err());
    }

    #[test]
    fn migration_errors() {
        assert!(tokenize("'a'").is_err());
        assert!(tokenize("a & b").is_err());
    }
}
