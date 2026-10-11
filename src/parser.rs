//! CDirac parser: tokens -> span-carrying AST.
//!
//! Desugars (all shared shapes, identical to the reference surface):
//! `var`/`const`/`set`, `fn`, braces-only blocks, `if/else-if/else`,
//! `while`, C-style `for`, `for x in xs` (index loop with hygienic
//! `__for_*` temps inside an `[if true …]` scope), `f(a)`, `r.m(a)`,
//! `C(a)` construction, `a.b` field reads, `==`/`!=`/`&&`/`||`,
//! chained comparisons, hyphen/`->`/`?` builtins in call position,
//! operator members (`machine.+(a, b)`), and `[...]` for directives,
//! nil, value spellings, and migration-compat forms.

use crate::ast::{Node, NodeKind, Span, TypeExpr};
use crate::lexer::{Token, TokenKind};

const STMT_KEYWORDS: &[&str] = &[
    "var", "set", "const", "fn", "if", "else", "while", "for", "break", "continue", "return",
];

fn is_stmt_kw(s: &str) -> bool {
    STMT_KEYWORDS.contains(&s)
}

/// Operator token after a dot (`machine.+(a, b)`) mapped to its builtin
/// name, so the node matches SDirac `[machine + a b]` exactly.
fn dot_operator_name(kind: &TokenKind) -> Option<&'static str> {
    match kind {
        TokenKind::Plus => Some("+"),
        TokenKind::Minus => Some("-"),
        TokenKind::Star => Some("*"),
        TokenKind::Slash => Some("/"),
        TokenKind::Lt => Some("<"),
        TokenKind::Gt => Some(">"),
        TokenKind::LtEq => Some("<="),
        TokenKind::GtEq => Some(">="),
        TokenKind::EqEq => Some("="),
        TokenKind::Bang => Some("not"),
        TokenKind::BangEq => Some("!="),
        TokenKind::AndAnd => Some("and"),
        TokenKind::OrOr => Some("or"),
        _ => None,
    }
}

fn sym(name: String, span: Span) -> Node {
    Node::new(NodeKind::Sym(name), span)
}

fn call(head: &str, head_span: Span, mut args: Vec<Node>) -> Node {
    let mut elems = vec![sym(head.to_string(), head_span)];
    elems.append(&mut args);
    Node::new(NodeKind::List(elems, None), head_span)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    /// Fresh-name counter for `for-in` desugaring (`__for_*` temps).
    tmp: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.toks[self.pos]
    }

    fn next_tok(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn at_end(&self) -> bool {
        self.peek().kind == TokenKind::Eof
    }

    fn eat(&mut self, kind: TokenKind) -> bool {
        if self.peek().kind == kind {
            self.next_tok();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokenKind, what: &str) -> Result<Token, String> {
        if self.peek().kind == kind {
            Ok(self.next_tok())
        } else {
            Err(format!(
                "line {}: expected {} (found '{}')",
                self.peek().span.line,
                what,
                self.peek().lexeme
            ))
        }
    }

    fn peek_sym(&self) -> Option<&str> {
        if self.peek().kind == TokenKind::Symbol {
            Some(self.peek().lexeme.as_str())
        } else {
            None
        }
    }

    fn err(&self, msg: impl Into<String>) -> String {
        format!("line {}: {}", self.peek().span.line, msg.into())
    }

    // ------------------------------------------------------------ types

    fn parse_type(&mut self) -> Result<TypeExpr, String> {
        let mut parts = vec![self.parse_primary_type()?];
        while self.peek().kind == TokenKind::Pipe {
            self.next_tok();
            parts.push(self.parse_primary_type()?);
        }
        if parts.len() == 1 {
            Ok(parts.into_iter().next().unwrap())
        } else {
            Ok(TypeExpr::Union(parts))
        }
    }

    fn parse_primary_type(&mut self) -> Result<TypeExpr, String> {
        if self.peek().kind == TokenKind::LBracket {
            self.next_tok();
            let head = self.expect(TokenKind::Symbol, "array or queue")?;
            if head.lexeme != "array" && head.lexeme != "queue" {
                return Err(format!(
                    "line {}: only [array T] and [queue T] are parameterized",
                    head.span.line
                ));
            }
            let is_queue = head.lexeme == "queue";
            let elem = self.parse_type()?;
            self.expect(TokenKind::RBracket, "']' to close the array type")?;
            return Ok(if is_queue {
                TypeExpr::Queue(Box::new(elem))
            } else {
                TypeExpr::Array(Box::new(elem))
            });
        }
        if self.peek().kind != TokenKind::Symbol {
            return Err(self.err("expected a type"));
        }
        let t = self.next_tok();
        match t.lexeme.as_str() {
            "int" => Ok(TypeExpr::Int),
            "float" => Ok(TypeExpr::Float),
            "num" => Ok(TypeExpr::Num),
            "bool" => Ok(TypeExpr::Bool),
            "true" => Ok(TypeExpr::True),
            "false" => Ok(TypeExpr::False),
            "string" => Ok(TypeExpr::Str),
            "void" => Ok(TypeExpr::Void),
            "nil" => Ok(TypeExpr::Nil),
            "any" => Ok(TypeExpr::Any),
            "ptr" => Ok(TypeExpr::Ptr),
            "never" => Err(format!(
                "line {}: never is not a valid annotation",
                t.span.line
            )),
            // Class types (`Scanner`) arrive with typed interop; keep the
            // reference message so both frontends agree.
            other => Err(format!("line {}: unknown type: {}", t.span.line, other)),
        }
    }

    fn parse_opt_type(&mut self) -> Result<Option<TypeExpr>, String> {
        if self.peek().kind == TokenKind::Pipe {
            self.next_tok();
            Ok(Some(self.parse_type()?))
        } else {
            Ok(None)
        }
    }

    fn expect_ident(&mut self, what: &str) -> Result<Token, String> {
        if self.peek().kind == TokenKind::Symbol {
            let t = self.next_tok();
            if is_stmt_kw(&t.lexeme) {
                return Err(format!(
                    "line {}: '{}' is a keyword and cannot be used as {}",
                    t.span.line, t.lexeme, what
                ));
            }
            if t.lexeme.starts_with("__for_") {
                return Err(format!(
                    "line {}: names starting with '__for_' are reserved (for-in desugaring)",
                    t.span.line
                ));
            }
            Ok(t)
        } else {
            Err(format!(
                "line {}: expected {} (found '{}')",
                self.peek().span.line,
                what,
                self.peek().lexeme
            ))
        }
    }

    /// Fresh hygienic temp for `for-in` desugaring.
    fn fresh_tmp(&mut self, tag: &str) -> String {
        self.tmp += 1;
        format!("__for_{tag}_{}", self.tmp)
    }

    // ------------------------------------------------------------ program

    fn parse_program(&mut self) -> Result<Vec<Node>, String> {
        let mut out = Vec::new();
        while !self.at_end() {
            if self.peek().kind == TokenKind::Semicolon {
                self.next_tok();
                continue;
            }
            out.push(self.parse_top_level()?);
        }
        Ok(out)
    }

    fn parse_top_level(&mut self) -> Result<Node, String> {
        if self.peek().kind == TokenKind::LBracket {
            return self.parse_bracket_datum();
        }
        if self.peek_sym() == Some("fn") {
            return self.parse_proc();
        }
        self.parse_stmt()
    }

    // ------------------------------------------------------------ statements

    fn parse_stmt(&mut self) -> Result<Node, String> {
        match self.peek().kind {
            TokenKind::LBrace => {
                // Standalone blocks have no surface node; scope them as a
                // single-arm `if true` (bodies scope in every backend).
                let brace_span = self.peek().span;
                let body = self.parse_block()?;
                let mut elems = vec![
                    sym("if".to_string(), brace_span),
                    Node::new(NodeKind::Bool(true), brace_span),
                ];
                elems.extend(body);
                Ok(Node::new(NodeKind::List(elems, None), brace_span))
            }
            TokenKind::LBracket => {
                let n = self.parse_bracket_datum()?;
                self.expect(TokenKind::Semicolon, "';'")?;
                Ok(n)
            }
            TokenKind::Symbol => {
                let kw = self.peek().lexeme.clone();
                let kw_span = self.peek().span;
                match kw.as_str() {
                    "var" => {
                        let n = self.parse_var_decl()?;
                        self.expect(TokenKind::Semicolon, "';'")?;
                        Ok(n)
                    }
                    "const" => {
                        let n = self.parse_const_decl()?;
                        self.expect(TokenKind::Semicolon, "';'")?;
                        Ok(n)
                    }
                    "set" => {
                        let n = self.parse_set_stmt()?;
                        self.expect(TokenKind::Semicolon, "';'")?;
                        Ok(n)
                    }
                    "fn" => self.parse_proc(),
                    "if" => self.parse_if(),
                    "while" => self.parse_while(),
                    "for" => self.parse_for(),
                    "break" => {
                        self.next_tok();
                        self.expect(TokenKind::Semicolon, "';'")?;
                        Ok(call("break", kw_span, vec![]))
                    }
                    "continue" => {
                        self.next_tok();
                        self.expect(TokenKind::Semicolon, "';'")?;
                        Ok(call("continue", kw_span, vec![]))
                    }
                    "return" => {
                        self.next_tok();
                        if self.peek().kind == TokenKind::Semicolon {
                            self.next_tok();
                            Ok(call("return", kw_span, vec![]))
                        } else {
                            let e = self.parse_expr()?;
                            self.expect(TokenKind::Semicolon, "';'")?;
                            Ok(call("return", kw_span, vec![e]))
                        }
                    }
                    "else" | "elif" => Err(self.err(format!(
                        "'{kw}' without if (write if (c) {{...}} else {{...}})"
                    ))),
                    "then" | "end" | "do" | "done" => {
                        Err(self.err(format!("'{kw}' is not CDirac (blocks are {{...}} only)")))
                    }
                    "and" | "or" | "not" => {
                        Err(self.err(format!("'{kw}' is not CDirac (write && / || / !)")))
                    }
                    "new" => Err(self.err("'new' is not CDirac (write Scanner(...) without new)")),
                    _ => {
                        let e = self.parse_expr()?;
                        self.expect(TokenKind::Semicolon, "';'")?;
                        Ok(e)
                    }
                }
            }
            TokenKind::Semicolon => Err(self.err("unexpected ';'")),
            _ => {
                let e = self.parse_expr()?;
                self.expect(TokenKind::Semicolon, "';'")?;
                Ok(e)
            }
        }
    }

    fn parse_block(&mut self) -> Result<Vec<Node>, String> {
        self.expect(TokenKind::LBrace, "'{'")?;
        let mut out = Vec::new();
        while self.peek().kind != TokenKind::RBrace {
            if self.at_end() {
                return Err(self.err("unexpected end of input: missing '}'"));
            }
            if self.peek().kind == TokenKind::Semicolon {
                self.next_tok();
                continue;
            }
            out.push(self.parse_stmt()?);
        }
        self.next_tok(); // '}'
        Ok(out)
    }

    fn parse_var_decl(&mut self) -> Result<Node, String> {
        let kw = self.next_tok(); // var
        let name = self.expect_ident("a variable name")?;
        let ty = self.parse_opt_type()?;
        self.expect(TokenKind::Assign, "'='")?;
        let val = self.parse_expr()?;
        Ok(Node::new(
            NodeKind::Var {
                name: name.lexeme,
                ty,
                val: Box::new(val),
            },
            kw.span,
        ))
    }

    fn parse_const_decl(&mut self) -> Result<Node, String> {
        let kw = self.next_tok(); // const
        let name = self.expect_ident("a constant name")?;
        let ty = self.parse_opt_type()?;
        self.expect(TokenKind::Assign, "'='")?;
        let val = self.parse_expr()?;
        Ok(Node::new(
            NodeKind::Const {
                name: name.lexeme,
                ty,
                val: Box::new(val),
            },
            kw.span,
        ))
    }

    /// `set target = expr` where target is `name(.name)*`.
    fn parse_set_stmt(&mut self) -> Result<Node, String> {
        let kw = self.next_tok(); // set
        let target = self.parse_dotted_target()?;
        self.expect(TokenKind::Assign, "'='")?;
        let val = self.parse_expr()?;
        Ok(Node::new(
            NodeKind::Set {
                name: target,
                val: Box::new(val),
            },
            kw.span,
        ))
    }

    fn parse_dotted_target(&mut self) -> Result<String, String> {
        let first = self.expect_ident("a variable name")?;
        let mut parts = vec![first.lexeme];
        while self.peek().kind == TokenKind::Dot {
            self.next_tok();
            let seg = self.expect_ident("a member name after '.'")?;
            parts.push(seg.lexeme);
        }
        if self.peek().kind == TokenKind::Colon {
            return Err(self.err("CDirac uses . not : (write a.b, set a.b = v;)"));
        }
        Ok(parts.join("."))
    }

    fn parse_proc(&mut self) -> Result<Node, String> {
        let kw = self.next_tok(); // fn
        let name = self.expect_ident("a procedure name")?;
        self.expect(TokenKind::LParen, "'('")?;
        let mut params = Vec::new();
        if self.peek().kind != TokenKind::RParen {
            loop {
                let p = self.expect_ident("a parameter name")?;
                let ty = self.parse_opt_type()?;
                params.push((p.lexeme, ty));
                if self.eat(TokenKind::Comma) {
                    continue;
                }
                break;
            }
        }
        self.expect(TokenKind::RParen, "')'")?;
        let ret = self.parse_opt_type()?;
        let body = self.parse_block()?;
        Ok(Node::new(
            NodeKind::Proc {
                name: name.lexeme,
                ret,
                params,
                body,
            },
            kw.span,
        ))
    }

    fn parse_if(&mut self) -> Result<Node, String> {
        let kw = self.next_tok(); // if
        if self.peek_sym().map(|s| s == "then").unwrap_or(false) {
            return Err(self.err("'then' is not CDirac (write if (c) {...})"));
        }
        let test = self.parse_expr()?;
        let mut then_body = self.parse_block()?;
        let mut elems = vec![sym("if".to_string(), kw.span), test];
        elems.append(&mut then_body);
        self.parse_else_tail(&mut elems)?;
        Ok(Node::new(NodeKind::List(elems, None), kw.span))
    }

    /// Shared `else` handling: flattens `else if` into `elif` separators.
    fn parse_else_tail(&mut self, elems: &mut Vec<Node>) -> Result<(), String> {
        loop {
            if self.peek_sym() != Some("else") {
                break;
            }
            self.next_tok(); // else
            if self.peek_sym() == Some("if") {
                let kw = self.next_tok(); // if
                let _ = kw;
                let test = self.parse_expr()?;
                let mut body = self.parse_block()?;
                elems.push(sym("elif".to_string(), Span::synthetic()));
                elems.push(test);
                elems.append(&mut body);
            } else {
                let mut else_body = self.parse_block()?;
                elems.push(sym("else".to_string(), Span::synthetic()));
                elems.append(&mut else_body);
                break;
            }
        }
        Ok(())
    }

    fn parse_while(&mut self) -> Result<Node, String> {
        let kw = self.next_tok(); // while
        let test = self.parse_expr()?;
        let mut body = self.parse_block()?;
        let mut elems = vec![sym("while".to_string(), kw.span), test];
        elems.append(&mut body);
        Ok(Node::new(NodeKind::List(elems, None), kw.span))
    }

    fn parse_for(&mut self) -> Result<Node, String> {
        let kw = self.next_tok(); // for
                                  // `for x[|T] in xs { … }` (no parens) vs C-style `for (init;…;…)`.
        if self.peek().kind == TokenKind::Symbol {
            let save = self.pos;
            let name = self.next_tok();
            let ty = if self.peek().kind == TokenKind::Pipe {
                self.next_tok();
                match self.parse_type() {
                    Ok(t) => Some(t),
                    Err(_) => {
                        self.pos = save;
                        return self.parse_c_style_for(kw.span);
                    }
                }
            } else {
                None
            };
            if self.peek_sym() == Some("in") {
                if is_stmt_kw(&name.lexeme) {
                    return Err(format!(
                        "line {}: '{}' is a keyword and cannot be a loop variable",
                        name.span.line, name.lexeme
                    ));
                }
                if name.lexeme.starts_with("__for_") {
                    return Err(format!(
                        "line {}: names starting with '__for_' are reserved (for-in desugaring)",
                        name.span.line
                    ));
                }
                return self.parse_for_in_rest(kw.span, name, ty);
            }
            self.pos = save;
        }
        self.parse_c_style_for(kw.span)
    }

    fn parse_c_style_for(&mut self, kw_span: Span) -> Result<Node, String> {
        self.expect(TokenKind::LParen, "'('")?;
        // init: var-decl | const-decl | set-stmt | expr
        let init = match self.peek_sym() {
            Some("var") => self.parse_var_decl()?,
            Some("const") => self.parse_const_decl()?,
            Some("set") => self.parse_set_stmt()?,
            _ => self.parse_expr()?,
        };
        self.expect(TokenKind::Semicolon, "';'")?;
        let test = self.parse_expr()?;
        self.expect(TokenKind::Semicolon, "';'")?;
        let update = if self.peek_sym() == Some("set") {
            self.parse_set_stmt()?
        } else {
            self.parse_expr()?
        };
        self.expect(TokenKind::RParen, "')'")?;
        let mut body = self.parse_block()?;
        let mut elems = vec![sym("for".to_string(), kw_span), init, test, update];
        elems.append(&mut body);
        Ok(Node::new(NodeKind::List(elems, None), kw_span))
    }

    /// `for x[|T] in xs { body }` — surface sugar over C-style `for`.
    /// Entry: `for`, loop variable and optional type consumed, `in` next.
    fn parse_for_in_rest(
        &mut self,
        kw_span: Span,
        var_tok: Token,
        item_ty: Option<TypeExpr>,
    ) -> Result<Node, String> {
        let in_tok = self.expect(TokenKind::Symbol, "'in'")?;
        debug_assert_eq!(in_tok.lexeme, "in");
        let iterable = self.parse_expr()?;
        let mut body = self.parse_block()?;
        let xs = self.fresh_tmp("xs");
        let idx = self.fresh_tmp("i");
        let bind_iter = Node::new(
            NodeKind::Var {
                name: xs.clone(),
                ty: None,
                val: Box::new(iterable),
            },
            kw_span,
        );
        let init = Node::new(
            NodeKind::Var {
                name: idx.clone(),
                ty: None,
                val: Box::new(Node::new(NodeKind::Int(0), kw_span)),
            },
            kw_span,
        );
        let test = Node::new(
            NodeKind::List(
                vec![
                    sym("<".to_string(), kw_span),
                    sym(idx.clone(), kw_span),
                    Node::new(
                        NodeKind::List(
                            vec![sym("len".to_string(), kw_span), sym(xs.clone(), kw_span)],
                            None,
                        ),
                        kw_span,
                    ),
                ],
                None,
            ),
            kw_span,
        );
        let update = Node::new(
            NodeKind::Set {
                name: idx.clone(),
                val: Box::new(Node::new(
                    NodeKind::List(
                        vec![
                            sym("+".to_string(), kw_span),
                            sym(idx.clone(), kw_span),
                            Node::new(NodeKind::Int(1), kw_span),
                        ],
                        None,
                    ),
                    kw_span,
                )),
            },
            kw_span,
        );
        let bind_item = Node::new(
            NodeKind::Var {
                name: var_tok.lexeme,
                ty: item_ty,
                val: Box::new(Node::new(
                    NodeKind::List(
                        vec![
                            sym("get".to_string(), kw_span),
                            sym(xs, kw_span),
                            sym(idx, kw_span),
                        ],
                        None,
                    ),
                    kw_span,
                )),
            },
            kw_span,
        );
        let mut for_elems = vec![
            sym("for".to_string(), kw_span),
            init,
            test,
            update,
            bind_item,
        ];
        for_elems.append(&mut body);
        Ok(Node::new(
            NodeKind::List(
                vec![
                    sym("if".to_string(), kw_span),
                    Node::new(NodeKind::Bool(true), kw_span),
                    bind_iter,
                    Node::new(NodeKind::List(for_elems, None), kw_span),
                ],
                None,
            ),
            kw_span,
        ))
    }

    // ------------------------------------------------------------ expressions

    fn parse_expr(&mut self) -> Result<Node, String> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Node, String> {
        let first = self.parse_and()?;
        let span = first.span;
        let mut args = vec![first];
        while self.peek().kind == TokenKind::OrOr {
            self.next_tok();
            args.push(self.parse_and()?);
        }
        if args.len() == 1 {
            Ok(args.into_iter().next().unwrap())
        } else {
            Ok(call("or", span, args))
        }
    }

    fn parse_and(&mut self) -> Result<Node, String> {
        let first = self.parse_equality()?;
        let span = first.span;
        let mut args = vec![first];
        while self.peek().kind == TokenKind::AndAnd {
            self.next_tok();
            args.push(self.parse_equality()?);
        }
        if args.len() == 1 {
            Ok(args.into_iter().next().unwrap())
        } else {
            Ok(call("and", span, args))
        }
    }

    fn parse_equality(&mut self) -> Result<Node, String> {
        let first = self.parse_comparison()?;
        let span = first.span;
        let mut operands = vec![first];
        let mut ops: Vec<bool> = Vec::new(); // true = ==, false = !=
        while matches!(self.peek().kind, TokenKind::EqEq | TokenKind::BangEq) {
            let is_eq = self.peek().kind == TokenKind::EqEq;
            self.next_tok();
            operands.push(self.parse_comparison()?);
            ops.push(is_eq);
        }
        if ops.is_empty() {
            return Ok(operands.into_iter().next().unwrap());
        }
        if ops.iter().all(|o| *o) {
            return Ok(call("=", span, operands));
        }
        // Mixed/!= chain: pairwise `and` of binary tests.
        let mut pairs = Vec::new();
        for (i, is_eq) in ops.iter().enumerate() {
            let l = operands[i].clone();
            let r = operands[i + 1].clone();
            let eq = call("=", span, vec![l, r]);
            if *is_eq {
                pairs.push(eq);
            } else {
                pairs.push(call("not", span, vec![eq]));
            }
        }
        if pairs.len() == 1 {
            Ok(pairs.into_iter().next().unwrap())
        } else {
            Ok(call("and", span, pairs))
        }
    }

    fn parse_comparison(&mut self) -> Result<Node, String> {
        let first = self.parse_add()?;
        let span = first.span;
        let mut operands = vec![first];
        let mut ops: Vec<&'static str> = Vec::new();
        while matches!(
            self.peek().kind,
            TokenKind::Lt | TokenKind::Gt | TokenKind::LtEq | TokenKind::GtEq
        ) {
            let op = match self.next_tok().kind {
                TokenKind::Lt => "<",
                TokenKind::Gt => ">",
                TokenKind::LtEq => "<=",
                TokenKind::GtEq => ">=",
                _ => unreachable!(),
            };
            operands.push(self.parse_add()?);
            ops.push(op);
        }
        if ops.is_empty() {
            return Ok(operands.into_iter().next().unwrap());
        }
        if ops.iter().all(|o| *o == ops[0]) {
            return Ok(call(ops[0], span, operands));
        }
        // Mixed chain (`a < b <= c`): pairwise `and`.
        let mut pairs = Vec::new();
        for (i, op) in ops.iter().enumerate() {
            pairs.push(call(
                op,
                span,
                vec![operands[i].clone(), operands[i + 1].clone()],
            ));
        }
        if pairs.len() == 1 {
            Ok(pairs.into_iter().next().unwrap())
        } else {
            Ok(call("and", span, pairs))
        }
    }

    fn parse_add(&mut self) -> Result<Node, String> {
        let first = self.parse_mul()?;
        let span = first.span;
        let mut operands = vec![first];
        let mut ops: Vec<&'static str> = Vec::new();
        while matches!(self.peek().kind, TokenKind::Plus | TokenKind::Minus) {
            let op = match self.next_tok().kind {
                TokenKind::Plus => "+",
                TokenKind::Minus => "-",
                _ => unreachable!(),
            };
            operands.push(self.parse_mul()?);
            ops.push(op);
        }
        if ops.is_empty() {
            return Ok(operands.into_iter().next().unwrap());
        }
        if ops.iter().all(|o| *o == ops[0]) {
            return Ok(call(ops[0], span, operands));
        }
        // Mixed `+`/`-`: left-assoc binary nesting.
        let mut acc = operands.into_iter();
        let mut node = acc.next().unwrap();
        for op in ops {
            let rhs = acc.next().unwrap();
            node = call(op, span, vec![node, rhs]);
        }
        Ok(node)
    }

    fn parse_mul(&mut self) -> Result<Node, String> {
        let first = self.parse_unary()?;
        let span = first.span;
        let mut operands = vec![first];
        let mut ops: Vec<&'static str> = Vec::new();
        while matches!(self.peek().kind, TokenKind::Star | TokenKind::Slash) {
            let op = match self.next_tok().kind {
                TokenKind::Star => "*",
                TokenKind::Slash => "/",
                _ => unreachable!(),
            };
            operands.push(self.parse_unary()?);
            ops.push(op);
        }
        if ops.is_empty() {
            return Ok(operands.into_iter().next().unwrap());
        }
        if ops.iter().all(|o| *o == ops[0]) {
            return Ok(call(ops[0], span, operands));
        }
        let mut acc = operands.into_iter();
        let mut node = acc.next().unwrap();
        for op in ops {
            let rhs = acc.next().unwrap();
            node = call(op, span, vec![node, rhs]);
        }
        Ok(node)
    }

    fn parse_unary(&mut self) -> Result<Node, String> {
        match self.peek().kind {
            TokenKind::Bang => {
                let span = self.next_tok().span;
                let e = self.parse_unary()?;
                Ok(call("not", span, vec![e]))
            }
            TokenKind::Minus => {
                let span = self.next_tok().span;
                let e = self.parse_unary()?;
                Ok(call("-", span, vec![e]))
            }
            TokenKind::Plus => {
                let span = self.next_tok().span;
                let e = self.parse_unary()?;
                Ok(call("+", span, vec![e]))
            }
            _ => self.parse_call_or_primary(),
        }
    }

    /// Calls, member access and field reads.
    fn parse_call_or_primary(&mut self) -> Result<Node, String> {
        if self.peek().kind == TokenKind::Symbol {
            let save = self.pos;
            if let Some(node) = self.try_parse_call()? {
                return Ok(node);
            }
            self.pos = save;
            // Dotted field read `a.b` (no parens, no hyphen combining).
            if self.peek_at(1).kind == TokenKind::Dot {
                let first = self.next_tok();
                let fspan = first.span;
                self.next_tok(); // dot
                let second = self.expect_ident("a member name after '.'")?;
                if self.peek().kind == TokenKind::Dot {
                    return Err(format!(
                        "line {}: only one dot per access (use import-java aliases for longer paths)",
                        second.span.line
                    ));
                }
                if self.peek().kind == TokenKind::Colon {
                    return Err(self.err("CDirac uses . not : (write a.b)"));
                }
                return Ok(Node::new(
                    NodeKind::Sym(format!("{}.{}", first.lexeme, second.lexeme)),
                    fspan,
                ));
            }
            if self.peek_at(1).kind == TokenKind::Colon {
                return Err(self.err("CDirac uses . not : (write a.b, f(a))"));
            }
        }
        self.parse_primary()
    }

    fn peek_at(&self, off: usize) -> &Token {
        let i = (self.pos + off).min(self.toks.len() - 1);
        &self.toks[i]
    }

    /// Identifier-led callee with `(` after it. `Ok(None)` = not a call.
    fn try_parse_call(&mut self) -> Result<Option<Node>, String> {
        // First segment: hyphen/`->`/trailing-`?` combined builtin-ish name.
        let cspan = self.peek().span;
        let first = self.parse_name_segment()?;
        let mut segments = vec![first];
        while self.peek().kind == TokenKind::Dot {
            self.next_tok();
            if self.peek().kind == TokenKind::Symbol {
                segments.push(self.parse_name_segment()?);
            } else if let Some(op) = dot_operator_name(&self.peek().kind) {
                self.next_tok();
                segments.push(op.to_string());
            } else {
                return Err(self.err("expected a member name after '.'"));
            }
            if segments.len() > 2 {
                return Err(
                    self.err("only one dot per access (use import-java aliases for longer paths)")
                );
            }
        }
        if self.peek().kind == TokenKind::Colon {
            return Err(self.err("CDirac uses . not : (write a.b, f(a))"));
        }
        if self.peek().kind != TokenKind::LParen {
            return Ok(None);
        }
        self.next_tok(); // '('
        let mut args = Vec::new();
        if self.peek().kind != TokenKind::RParen {
            loop {
                args.push(self.parse_expr()?);
                if self.eat(TokenKind::Comma) {
                    continue;
                }
                break;
            }
        }
        self.expect(TokenKind::RParen, "')'")?;
        if segments.len() == 1 {
            let mut elems = vec![Node::new(
                NodeKind::Sym(segments.into_iter().next().unwrap()),
                cspan,
            )];
            elems.extend(args);
            Ok(Some(Node::new(NodeKind::List(elems, None), cspan)))
        } else {
            let recv = segments[0].clone();
            let member = segments[1].clone();
            let mut elems = vec![
                Node::new(NodeKind::Sym(recv), cspan),
                Node::new(NodeKind::Sym(member), cspan),
            ];
            elems.extend(args);
            Ok(Some(Node::new(NodeKind::List(elems, None), cspan)))
        }
    }

    /// One callee segment: `name(-name)*` with `->` links and trailing `?`
    /// (`string-append`, `number->string`, `zero?`). Call position only.
    fn parse_name_segment(&mut self) -> Result<String, String> {
        if self.peek().kind != TokenKind::Symbol {
            return Err(self.err("expected a name"));
        }
        let head = self.next_tok();
        if is_stmt_kw(&head.lexeme) {
            return Err(format!(
                "line {}: '{}' is a keyword",
                head.span.line, head.lexeme
            ));
        }
        let mut name = head.lexeme;
        loop {
            if self.peek().kind == TokenKind::Minus {
                if self.peek_at(1).kind == TokenKind::Gt {
                    self.next_tok();
                    self.next_tok();
                    if self.peek().kind != TokenKind::Symbol {
                        return Err(self.err("expected a name after '->'"));
                    }
                    name.push_str("->");
                    name.push_str(&self.next_tok().lexeme);
                } else if self.peek_at(1).kind == TokenKind::Symbol {
                    self.next_tok();
                    name.push('-');
                    name.push_str(&self.next_tok().lexeme);
                } else {
                    break;
                }
            } else if self.peek().kind == TokenKind::Question {
                self.next_tok();
                name.push('?');
                break;
            } else {
                break;
            }
        }
        if self.peek().kind == TokenKind::Question {
            return Err(self.err("unexpected '?' (predicates spell zero? with one ?)"));
        }
        Ok(name)
    }

    fn parse_primary(&mut self) -> Result<Node, String> {
        let t = self.next_tok();
        match t.kind {
            TokenKind::Number => {
                if t.is_float {
                    Ok(Node::new(NodeKind::Float(t.float_num), t.span))
                } else {
                    Ok(Node::new(NodeKind::Int(t.int_num), t.span))
                }
            }
            TokenKind::Str => Ok(Node::new(NodeKind::Str(t.str_val), t.span)),
            TokenKind::Bool => Ok(Node::new(NodeKind::Bool(t.bool_val), t.span)),
            TokenKind::Symbol => match t.lexeme.as_str() {
                "true" => Ok(Node::new(NodeKind::Bool(true), t.span)),
                "false" => Ok(Node::new(NodeKind::Bool(false), t.span)),
                "null" | "nil" => Ok(Node::new(NodeKind::Nil, t.span)),
                "var" | "set" | "const" | "fn" | "if" | "else" | "while" | "for" | "break"
                | "continue" | "return" => Err(format!(
                    "line {}: '{}' starts a statement, not an expression",
                    t.span.line, t.lexeme
                )),
                "then" | "end" | "do" | "done" => Err(format!(
                    "line {}: '{}' is not CDirac (blocks are {{...}} only)",
                    t.span.line, t.lexeme
                )),
                "and" | "or" | "not" => Err(format!(
                    "line {}: '{}' is not CDirac (write && / || / !)",
                    t.span.line, t.lexeme
                )),
                "elif" => Err(format!(
                    "line {}: 'elif' is not CDirac (write else if)",
                    t.span.line
                )),
                "new" => Err(format!(
                    "line {}: 'new' is not CDirac (write Scanner(...) without new)",
                    t.span.line
                )),
                _ => Ok(Node::new(NodeKind::Sym(t.lexeme), t.span)),
            },
            TokenKind::LParen => {
                let e = self.parse_expr()?;
                self.expect(TokenKind::RParen, "')'")?;
                Ok(e)
            }
            TokenKind::LBracket => self.parse_bracket_after_open(t.span),
            TokenKind::Colon => Err(format!(
                "line {}: CDirac uses . not : (write a.b, f(a))",
                t.span.line
            )),
            TokenKind::Assign => Err(format!(
                "line {}: bare '=' is not CDirac (write var x = e; / set x = e; / const x = e;)",
                t.span.line
            )),
            TokenKind::Comma => Err(format!(
                "line {}: unexpected ',' (unquote is not supported: code-is-not-data)",
                t.span.line
            )),
            _ => Err(format!(
                "line {}: unexpected '{}' in expression",
                t.span.line, t.lexeme
            )),
        }
    }

    // ------------------------------------------------------------ brackets
    //
    // `[...]` survives for directives, nil, value spellings, and generic
    // migration-compat forms. Contents parse as SDirac-style datums where
    // `-`/`->`/`?` stay word chars and operators read as heads.

    fn parse_bracket_datum(&mut self) -> Result<Node, String> {
        let open = self.expect(TokenKind::LBracket, "'['")?;
        self.parse_bracket_after_open(open.span)
    }

    fn parse_bracket_after_open(&mut self, open_span: Span) -> Result<Node, String> {
        if self.peek().kind == TokenKind::RBracket {
            self.next_tok();
            return Ok(Node::new(NodeKind::Nil, open_span));
        }
        let head = self.parse_sdatum_head()?;
        let hw = match &head.kind {
            NodeKind::Sym(s) => s.clone(),
            _ => {
                let mut elems = vec![head];
                while self.peek().kind != TokenKind::RBracket {
                    if self.at_end() {
                        return Err(self.err("unexpected end of input: missing ']'"));
                    }
                    elems.push(self.parse_sdatum()?);
                }
                self.next_tok();
                return Ok(Node::new(NodeKind::List(elems, None), open_span));
            }
        };
        if hw == "var" || hw == "const" {
            return self.parse_bracket_var_form(&hw, open_span);
        }
        if hw == "set" {
            return self.parse_bracket_set_form(open_span);
        }
        if hw == "proc" {
            return self.parse_bracket_proc_form(open_span);
        }
        let mut elems = vec![Node::new(NodeKind::Sym(hw), open_span)];
        let mut tail: Option<Box<Node>> = None;
        loop {
            if self.at_end() {
                return Err(self.err("unexpected end of input: missing ']'"));
            }
            if self.peek().kind == TokenKind::RBracket {
                self.next_tok();
                break;
            }
            if self.peek().kind == TokenKind::Dot {
                if tail.is_some() {
                    return Err(self.err("multiple '.' in list literal"));
                }
                self.next_tok();
                tail = Some(Box::new(self.parse_sdatum()?));
                self.expect(TokenKind::RBracket, "']' after the dotted tail")?;
                break;
            }
            elems.push(self.parse_sdatum()?);
        }
        Ok(Node::new(NodeKind::List(elems, tail), open_span))
    }

    /// Head position inside `[...]`: hyphen-combined words plus operator
    /// spellings (`+` -> `+`, `==` -> `=`).
    fn parse_sdatum_head(&mut self) -> Result<Node, String> {
        let op = match self.peek().kind {
            TokenKind::Plus => Some("+"),
            TokenKind::Minus => Some("-"),
            TokenKind::Star => Some("*"),
            TokenKind::Slash => Some("/"),
            TokenKind::Lt => Some("<"),
            TokenKind::Gt => Some(">"),
            TokenKind::LtEq => Some("<="),
            TokenKind::GtEq => Some(">="),
            TokenKind::EqEq => Some("="),
            _ => None,
        };
        if let Some(name) = op {
            let span = self.next_tok().span;
            return Ok(Node::new(NodeKind::Sym(name.to_string()), span));
        }
        match self.peek().kind {
            TokenKind::Symbol => {
                let span = self.peek().span;
                let mut name = self.next_tok().lexeme;
                loop {
                    if self.peek().kind == TokenKind::Minus {
                        if self.peek_at(1).kind == TokenKind::Gt {
                            self.next_tok();
                            self.next_tok();
                            if self.peek().kind != TokenKind::Symbol {
                                return Err(self.err("expected a name after '->'"));
                            }
                            name.push_str("->");
                            name.push_str(&self.next_tok().lexeme);
                        } else if self.peek_at(1).kind == TokenKind::Symbol {
                            self.next_tok();
                            name.push('-');
                            name.push_str(&self.next_tok().lexeme);
                        } else {
                            break;
                        }
                    } else if self.peek().kind == TokenKind::Question {
                        self.next_tok();
                        name.push('?');
                        break;
                    } else {
                        break;
                    }
                }
                Ok(Node::new(NodeKind::Sym(name), span))
            }
            _ => Err(self.err(format!("unexpected '{}' inside [...]", self.peek().lexeme))),
        }
    }

    fn parse_sdatum(&mut self) -> Result<Node, String> {
        match self.peek().kind {
            TokenKind::Str => {
                let t = self.next_tok();
                Ok(Node::new(NodeKind::Str(t.str_val), t.span))
            }
            TokenKind::Number => {
                let t = self.next_tok();
                if t.is_float {
                    Ok(Node::new(NodeKind::Float(t.float_num), t.span))
                } else {
                    Ok(Node::new(NodeKind::Int(t.int_num), t.span))
                }
            }
            TokenKind::Bool => {
                let t = self.next_tok();
                Ok(Node::new(NodeKind::Bool(t.bool_val), t.span))
            }
            TokenKind::LBracket => {
                let open = self.next_tok();
                self.parse_bracket_after_open(open.span)
            }
            TokenKind::Pipe => {
                Err(self.err("unexpected '|' (pipes attach types to names: [var x|int ...])"))
            }
            TokenKind::Dot => Err(self.err("unexpected '.'")),
            TokenKind::Colon => Err(self.err("CDirac uses . not : (write a.b)")),
            TokenKind::RBracket => Err(self.err("unexpected ']'")),
            TokenKind::Eof => Err(self.err("unexpected end of input: missing ']'")),
            TokenKind::Semicolon => Err(self.err("unexpected ';' inside [...]")),
            TokenKind::Comma => Err(self.err("unexpected ',' inside [...] (commas are call-only)")),
            _ => self.parse_sdatum_head(),
        }
    }

    fn parse_bracket_var_form(&mut self, kw: &str, span: Span) -> Result<Node, String> {
        let name_node = self.parse_sdatum()?;
        let name_str = match &name_node.kind {
            NodeKind::Sym(s) => s.clone(),
            _ => return Err(self.err(format!("{kw}: expects a variable name"))),
        };
        let mut declared = None;
        if self.peek().kind == TokenKind::Pipe {
            self.next_tok();
            declared = Some(self.parse_type()?);
        }
        if self.peek().kind == TokenKind::RBracket {
            return Err(self.err(format!("{kw}: expects a name and a value")));
        }
        let val = self.parse_sdatum()?;
        if self.peek().kind != TokenKind::RBracket {
            return Err(self.err(format!("{kw}: expects a name and a value")));
        }
        self.next_tok();
        if kw == "const" {
            Ok(Node::new(
                NodeKind::Const {
                    name: name_str,
                    ty: declared,
                    val: Box::new(val),
                },
                span,
            ))
        } else {
            Ok(Node::new(
                NodeKind::Var {
                    name: name_str,
                    ty: declared,
                    val: Box::new(val),
                },
                span,
            ))
        }
    }

    fn parse_bracket_set_form(&mut self, span: Span) -> Result<Node, String> {
        let name_node = self.parse_sdatum()?;
        let name_str = match &name_node.kind {
            NodeKind::Sym(s) => s.clone(),
            _ => return Err(self.err("set: expects a variable name")),
        };
        if self.peek().kind == TokenKind::Pipe {
            return Err(self.err("set takes no type annotation"));
        }
        if self.peek().kind == TokenKind::RBracket {
            return Err(self.err("set: expects a name and a value"));
        }
        let val = self.parse_sdatum()?;
        if self.peek().kind != TokenKind::RBracket {
            return Err(self.err("set: expects a name and a value"));
        }
        self.next_tok();
        Ok(Node::new(
            NodeKind::Set {
                name: name_str,
                val: Box::new(val),
            },
            span,
        ))
    }

    fn parse_bracket_proc_form(&mut self, span: Span) -> Result<Node, String> {
        let name_node = self.parse_sdatum()?;
        let name = match &name_node.kind {
            NodeKind::Sym(s) => s.clone(),
            _ => return Err(self.err("proc: expects a name")),
        };
        let mut ret = None;
        if self.peek().kind == TokenKind::Pipe {
            self.next_tok();
            ret = Some(self.parse_type()?);
        }
        self.expect(
            TokenKind::LBracket,
            "a parameter list, e.g. [proc f [x y] ...]",
        )?;
        let mut params = Vec::new();
        while self.peek().kind != TokenKind::RBracket {
            if self.at_end() {
                return Err(self.err("unexpected end of input: missing ']'"));
            }
            let head = self.parse_sdatum_head()?;
            let pname = match &head.kind {
                NodeKind::Sym(s) => s.clone(),
                _ => return Err(self.err("proc: expects a parameter name")),
            };
            if ["int", "float", "num", "bool", "string", "any"].contains(&pname.as_str()) {
                return Err(self.err(format!(
                    "parameter '{pname}' looks like a type; write [name|{pname}]"
                )));
            }
            let ty = if self.peek().kind == TokenKind::Pipe {
                self.next_tok();
                Some(self.parse_type()?)
            } else {
                None
            };
            params.push((pname, ty));
        }
        self.next_tok(); // ']'
        let mut body = Vec::new();
        while self.peek().kind != TokenKind::RBracket {
            if self.at_end() {
                return Err(self.err("unexpected end of input: missing ']'"));
            }
            body.push(self.parse_sdatum()?);
        }
        self.next_tok();
        Ok(Node::new(
            NodeKind::Proc {
                name,
                ret,
                params,
                body,
            },
            span,
        ))
    }
}

/// Parse CDirac source text into top-level AST nodes.
pub fn parse_source(src: &str) -> Result<Vec<Node>, String> {
    let toks = super::lexer::tokenize(src)?;
    let mut p = Parser {
        toks,
        pos: 0,
        tmp: 0,
    };
    p.parse_program()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::NodeKind;

    fn one(src: &str) -> Node {
        let prog = parse_source(src).unwrap();
        assert_eq!(prog.len(), 1, "expected 1 form in {src:?}");
        prog.into_iter().next().unwrap()
    }

    fn is_var(n: &Node, name: &str) -> bool {
        matches!(&n.kind, NodeKind::Var { name: n, .. } if n == name)
    }

    #[test]
    fn spans_point_at_leading_tokens() {
        let n = one("var x = 5;");
        assert_eq!(n.span, Span::new(1, 1));
        assert!(is_var(&n, "x"));
        let n = one("\n\n  set y = 1;");
        assert_eq!(n.span, Span::new(3, 3));
    }

    #[test]
    fn var_set_const() {
        let n = one("var x|int = 5;");
        match &n.kind {
            NodeKind::Var { name, ty, val } => {
                assert_eq!(name, "x");
                assert_eq!(*ty, Some(TypeExpr::Int));
                assert!(matches!(val.kind, NodeKind::Int(5)));
            }
            other => panic!("expected var, got {other:?}"),
        }
        assert!(matches!(one("set x = 5;").kind, NodeKind::Set { .. }));
        assert!(matches!(one("const PI = 3;").kind, NodeKind::Const { .. }));
        assert!(parse_source("x = 5;").is_err());
    }

    #[test]
    fn calls_dot_operators() {
        assert_eq!(one("displayln(\"hi\");").print(), "[displayln \"hi\"]");
        assert_eq!(one("sc.nextInt();").print(), "[sc nextInt]");
        assert_eq!(one("System.in;").print(), "System.in");
        assert_eq!(one("set p.x = 9;").print(), "[set p.x 9]");
        assert_eq!(one("a + b;").print(), "[+ a b]");
        assert_eq!(one("a == b;").print(), "[= a b]");
        assert_eq!(one("a != b;").print(), "[not [= a b]]");
        assert_eq!(one("machine.+(20, 22);").print(), "[machine + 20 22]");
        assert_eq!(one("zero?(x);").print(), "[zero? x]");
        assert_eq!(one("string-append(a, b);").print(), "[string-append a b]");
    }

    #[test]
    fn proc_if_while_for() {
        let n = one("fn fact(n|int) | int { if (n <= 1) { return 1; } return n * fact(n - 1); }");
        match &n.kind {
            NodeKind::Proc {
                name,
                ret,
                params,
                body,
            } => {
                assert_eq!(name, "fact");
                assert_eq!(*ret, Some(TypeExpr::Int));
                assert_eq!(params.len(), 1);
                assert_eq!(body.len(), 2);
            }
            other => panic!("expected proc, got {other:?}"),
        }
        assert!(
            one("for (var i = 0; i < 10; set i = i + 1) { displayln(i); }")
                .print()
                .starts_with("[for [var i 0]")
        );
    }

    #[test]
    fn for_in_desugar() {
        let n = one("for x in xs { displayln(x); }");
        let printed = n.print();
        assert!(
            printed.starts_with("[if true [var __for_xs_1 xs]"),
            "{printed}"
        );
        assert!(printed.contains("[for [var __for_i_2 0]"), "{printed}");
        assert!(
            printed.contains("[var x [get __for_xs_1 __for_i_2]]"),
            "{printed}"
        );
        let n = one("for x|int in xs { displayln(x); }");
        assert!(
            n.print().contains("[var x|int [get __for_xs_1 __for_i_2]]"),
            "{}",
            n.print()
        );
        let prog =
            parse_source("for a in xs { displayln(a); } for b in ys { displayln(b); }").unwrap();
        assert!(prog[0].print().contains("__for_xs_1"));
        assert!(prog[1].print().contains("__for_xs_3"));
        assert!(parse_source("var __for_xs_1 = 1;").is_err());
        assert!(parse_source("for x of xs { displayln(x); }").is_err());
        assert!(parse_source("for (x in xs) { displayln(x); }").is_err());
    }

    #[test]
    fn migration_errors() {
        for src in [
            "x = 5;",
            "new Scanner(\"x\");",
            "sc:nextInt();",
            "if (a) then b; end;",
            "if (a) elif (b) { c; }",
            "displayln [1 2;",
        ] {
            assert!(parse_source(src).is_err(), "should fail: {src}");
        }
    }

    #[test]
    fn print_is_canonical_sdirac() {
        // Spot goldens: new-AST print must equal reference canonical forms.
        for (src, want) in [
            ("var x|int = 5;", "[var x|int 5]"),
            ("set x = x + 1;", "[set x [+ x 1]]"),
            ("const PI = 3;", "[const PI 3]"),
            (
                "fn adder(x|int, y|int) | int { return x + y; }",
                "[proc adder|int [x|int y|int] [return [+ x y]]]",
            ),
            ("if (a) { b; } else { c; }", "[if a b else c]"),
            ("while (a) { b; }", "[while a b]"),
            ("sc.nextInt();", "[sc nextInt]"),
        ] {
            assert_eq!(one(src).print(), want, "golden mismatch for {src}");
        }
    }
}
