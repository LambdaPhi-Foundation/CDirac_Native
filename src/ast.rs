//! AST: span-carrying nodes shared by the CDirac surface and (next) the
//! LLVM backend in this repo.
//!
//! Shapes mirror the reference pipeline's `Node`/`TypeExpr` one for one
//! (see `../bracket-jvm/bkt/src/parser.rs`), including the surface
//! desugars the parser performs (`for-in` to index loops, `==` to `[=]`,
//! dot calls to space forms). Every node carries its source `Span` so
//! the backend reports errors at the right line. `print()` renders the
//! canonical SDirac spelling — `parse(print(n)) == n` is the
//! compatibility gate against the reference corpus.

/// Source position (1-based line/column of the node's first token).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

impl Span {
    pub fn new(line: usize, col: usize) -> Self {
        Span { line, col }
    }

    /// Placeholder for synthesized nodes (desugars with no source token).
    pub fn synthetic() -> Self {
        Span { line: 0, col: 0 }
    }
}

/// Static type expression (opt-in annotations ride inline on nodes).
#[derive(Clone, Debug, PartialEq)]
pub enum TypeExpr {
    Int,
    Float,
    Num,
    Bool,
    True,
    False,
    Str,
    Void,
    Nil,
    Any,
    Ptr,
    Array(Box<TypeExpr>),
    Queue(Box<TypeExpr>),
    Union(Vec<TypeExpr>),
}

impl TypeExpr {
    pub fn print(&self) -> String {
        match self {
            TypeExpr::Int => "int".to_string(),
            TypeExpr::Float => "float".to_string(),
            TypeExpr::Num => "num".to_string(),
            TypeExpr::Bool => "bool".to_string(),
            TypeExpr::True => "true".to_string(),
            TypeExpr::False => "false".to_string(),
            TypeExpr::Str => "string".to_string(),
            TypeExpr::Void => "void".to_string(),
            TypeExpr::Nil => "nil".to_string(),
            TypeExpr::Any => "any".to_string(),
            TypeExpr::Ptr => "ptr".to_string(),
            TypeExpr::Array(e) => format!("[array {}]", e.print()),
            TypeExpr::Queue(e) => format!("[queue {}]", e.print()),
            TypeExpr::Union(ms) => ms.iter().map(|m| m.print()).collect::<Vec<_>>().join("|"),
        }
    }
}

/// AST node. `List` carries an optional dotted tail; `Nil` is `[]` /
/// `null` / `nil`. `Var` declares a mutable binding, `Const` an
/// immutable one; everything else is a generic call plus head-string
/// dispatch downstream — exactly like the reference AST.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Int(i32),
    Float(f64),
    Bool(bool),
    Str(String),
    Sym(String),
    Nil,
    List(Vec<Node>, Option<Box<Node>>),
    Var {
        name: String,
        ty: Option<TypeExpr>,
        val: Box<Node>,
    },
    Const {
        name: String,
        ty: Option<TypeExpr>,
        val: Box<Node>,
    },
    Set {
        name: String,
        val: Box<Node>,
    },
    Proc {
        name: String,
        ret: Option<TypeExpr>,
        params: Vec<(String, Option<TypeExpr>)>,
        body: Vec<Node>,
    },
}

impl Node {
    pub fn new(kind: NodeKind, span: Span) -> Self {
        Node { kind, span }
    }

    /// Canonical SDirac spelling; `parse(print(n)) == n` for all corpus ASTs.
    pub fn print(&self) -> String {
        match &self.kind {
            NodeKind::Int(n) => n.to_string(),
            NodeKind::Float(x) => float_repr(*x),
            NodeKind::Bool(b) => {
                if *b {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            NodeKind::Str(s) => format!("\"{}\"", escape_str(s)),
            NodeKind::Sym(s) => s.clone(),
            NodeKind::Nil => "null".to_string(),
            NodeKind::List(elems, tail) => {
                let mut parts: Vec<String> = elems.iter().map(|e| e.print()).collect();
                if let Some(t) = tail {
                    parts.push(".".to_string());
                    parts.push(t.print());
                }
                format!("[{}]", parts.join(" "))
            }
            NodeKind::Var { name, ty, val } => match ty {
                Some(t) => format!("[var {}|{} {}]", name, t.print(), val.print()),
                None => format!("[var {} {}]", name, val.print()),
            },
            NodeKind::Const { name, ty, val } => match ty {
                Some(t) => format!("[const {}|{} {}]", name, t.print(), val.print()),
                None => format!("[const {} {}]", name, val.print()),
            },
            NodeKind::Set { name, val } => format!("[set {} {}]", name, val.print()),
            NodeKind::Proc {
                name,
                ret,
                params,
                body,
            } => {
                let ps: Vec<String> = params
                    .iter()
                    .map(|(n, t)| match t {
                        Some(t) => format!("{}|{}", n, t.print()),
                        None => n.clone(),
                    })
                    .collect();
                let mut parts = vec![
                    "proc".to_string(),
                    match ret {
                        Some(t) => format!("{}|{}", name, t.print()),
                        None => name.clone(),
                    },
                    format!("[{}]", ps.join(" ")),
                ];
                parts.extend(body.iter().map(|b| b.print()));
                format!("[{}]", parts.join(" "))
            }
        }
    }
}

pub fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// Java `Double.toString` rules: `NaN`, `Infinity`/`-Infinity`, decimal
/// for `[1e-3, 1e7)`, otherwise scientific `d.dEexp` with uppercase `E`
/// and a mandatory `.0` for integral mantissas. Digits come from Rust's
/// shortest-round-trip `{:e}` formatting, reframed to Java's threshold
/// and mantissa style (same algorithm and known limitation as the
/// reference pipeline).
pub fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }
    if x == 0.0 {
        return if x.is_sign_negative() {
            "-0.0".to_string()
        } else {
            "0.0".to_string()
        };
    }
    let s = format!("{:e}", x);
    let (mant, exp_s) = s.split_once('e').expect("scientific format");
    let exp: i32 = exp_s.parse().expect("exponent parses");
    let neg = mant.starts_with('-');
    let mant = if neg { &mant[1..] } else { mant };
    let (lead, frac) = match mant.split_once('.') {
        Some((a, b)) => (a, b),
        None => (mant, ""),
    };
    let digits = format!("{lead}{frac}");
    if (-3..7).contains(&exp) {
        let pos = (exp + 1) as i64;
        let n = digits.len() as i64;
        let body = if pos <= 0 {
            format!("0.{}{}", "0".repeat((-pos) as usize), digits)
        } else if pos as usize >= digits.len() {
            format!("{}{}.0", digits, "0".repeat((pos - n) as usize))
        } else {
            format!("{}.{}", &digits[..pos as usize], &digits[pos as usize..])
        };
        if neg {
            format!("-{body}")
        } else {
            body
        }
    } else {
        let mant_j = if digits.len() == 1 {
            format!("{digits}.0")
        } else {
            format!("{}.{}", &digits[..1], &digits[1..])
        };
        if neg {
            format!("-{mant_j}E{exp}")
        } else {
            format!("{mant_j}E{exp}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(kind: NodeKind) -> Node {
        Node::new(kind, Span::synthetic())
    }

    #[test]
    fn canonical_spellings() {
        assert_eq!(node(NodeKind::Int(42)).print(), "42");
        assert_eq!(node(NodeKind::Int(i32::MIN)).print(), "-2147483648");
        assert_eq!(node(NodeKind::Float(3.5)).print(), "3.5");
        assert_eq!(node(NodeKind::Float(1e21)).print(), "1.0E21");
        assert_eq!(node(NodeKind::Float(f64::NAN)).print(), "NaN");
        assert_eq!(node(NodeKind::Float(f64::INFINITY)).print(), "Infinity");
        assert_eq!(node(NodeKind::Bool(true)).print(), "true");
        assert_eq!(node(NodeKind::Nil).print(), "null");
        assert_eq!(
            node(NodeKind::Str("a\n\"b".to_string())).print(),
            "\"a\\n\\\"b\""
        );
        assert_eq!(
            TypeExpr::Union(vec![TypeExpr::Int, TypeExpr::Str]).print(),
            "int|string"
        );
    }
}
