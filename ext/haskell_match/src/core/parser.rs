//! Recursive-descent parsers for Haskell patterns and `data` declarations.
//!
//! Pattern grammar (a faithful subset of Haskell 2010 plus a few extensions):
//!
//! ```text
//! pattern := lpat (':' pattern)?                 -- cons, right associative
//! lpat    := '-' number
//!          | ConId apat+                          -- constructor application
//!          | apat
//! apat    := var ('@' apat)?                      -- variable / as-pattern
//!          | '~' apat | '!' apat                  -- lazy / bang
//!          | ConId                                -- nullary constructor
//!          | ConId '{' fpat, ... [..] '}'         -- record pattern
//!          | literal                              -- 1  1.5  'c'  :sym
//!          | string                               -- "abc" = ['a', 'b', 'c']
//!          | '_'
//!          | '(' ')' | '(' pattern ')' | '(' pattern ',' pattern ... ')'
//!          | '[' ']' | '[' pattern ',' ... ']'
//! fpat    := var '=' pattern | var                -- NamedFieldPuns
//! ```

use super::ast::{Lit, RawPat};
use super::error::{CoreError, ErrorKind, Result};
use super::lexer::{tokenize, Tok, Token};

struct P {
    toks: Vec<Token>,
    i: usize,
    src: String,
}

impl P {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|t| &t.tok)
    }
    fn peek_at(&self, n: usize) -> Option<&Tok> {
        self.toks.get(self.i + n).map(|t| &t.tok)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).map(|t| t.tok.clone());
        if t.is_some() {
            self.i += 1;
        }
        t
    }
    fn pos(&self) -> usize {
        self.toks
            .get(self.i)
            .map(|t| t.pos + 1)
            .unwrap_or_else(|| self.src.chars().count() + 1)
    }
    fn expect(&mut self, want: Tok, what: &str) -> Result<()> {
        match self.next() {
            Some(ref t) if *t == want => Ok(()),
            Some(t) => Err(self.err(format!("expected {} but found {}", what, describe(&t)))),
            None => Err(self.err(format!("expected {} but reached end of pattern", what))),
        }
    }
    fn err(&self, msg: String) -> CoreError {
        let at = if self.i > 0 && self.i <= self.toks.len() {
            self.toks[self.i - 1].pos + 1
        } else {
            self.pos()
        };
        CoreError::syntax(format!("{} (column {} in {:?})", msg, at, self.src))
    }
    fn err_here(&self, msg: String) -> CoreError {
        CoreError::syntax(format!("{} (column {} in {:?})", msg, self.pos(), self.src))
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::LParen => "'('".into(),
        Tok::RParen => "')'".into(),
        Tok::LBracket => "'['".into(),
        Tok::RBracket => "']'".into(),
        Tok::LBrace => "'{'".into(),
        Tok::RBrace => "'}'".into(),
        Tok::Comma => "','".into(),
        Tok::Colon => "':'".into(),
        Tok::DoubleColon => "'::'".into(),
        Tok::Equals => "'='".into(),
        Tok::At => "'@'".into(),
        Tok::Tilde => "'~'".into(),
        Tok::Bang => "'!'".into(),
        Tok::Minus => "'-'".into(),
        Tok::Pipe => "'|'".into(),
        Tok::Arrow => "'->'".into(),
        Tok::DotDot => "'..'".into(),
        Tok::Underscore => "'_'".into(),
        Tok::VarId(v) => format!("variable '{}'", v),
        Tok::ConId(c) => format!("constructor '{}'", c),
        Tok::Int(s) | Tok::Float(s) => format!("literal {}", s),
        Tok::Str(s) => format!("literal {:?}", s),
        Tok::Char(s) => format!("literal '{}'", s),
        Tok::Sym(s) => format!("literal :{}", s),
    }
}

/// Parse one complete pattern.
pub fn parse_pattern(src: &str) -> Result<RawPat> {
    let toks = tokenize(src)?;
    let mut p = P {
        toks,
        i: 0,
        src: src.to_string(),
    };
    if p.peek().is_none() {
        return Err(CoreError::syntax("empty pattern"));
    }
    let pat = pattern(&mut p)?;
    if let Some(t) = p.peek() {
        return Err(p.err_here(format!("unexpected {} after pattern", describe(t))));
    }
    Ok(pat)
}

fn pattern(p: &mut P) -> Result<RawPat> {
    let head = lpat(p)?;
    if p.peek() == Some(&Tok::Colon) {
        p.next();
        if p.peek().is_none() {
            return Err(p.err("expected a pattern after ':'".into()));
        }
        let tail = pattern(p)?;
        return Ok(RawPat::Cons(Box::new(head), Box::new(tail)));
    }
    Ok(head)
}

fn starts_apat(t: Option<&Tok>) -> bool {
    matches!(
        t,
        Some(Tok::VarId(_))
            | Some(Tok::ConId(_))
            | Some(Tok::Int(_))
            | Some(Tok::Float(_))
            | Some(Tok::Str(_))
            | Some(Tok::Char(_))
            | Some(Tok::Sym(_))
            | Some(Tok::Underscore)
            | Some(Tok::LParen)
            | Some(Tok::LBracket)
            | Some(Tok::Tilde)
            | Some(Tok::Bang)
            | Some(Tok::Minus)
    )
}

fn lpat(p: &mut P) -> Result<RawPat> {
    match p.peek() {
        Some(Tok::ConId(_)) if p.peek_at(1) != Some(&Tok::LBrace) => {
            let name = match p.next() {
                Some(Tok::ConId(n)) => n,
                _ => unreachable!(),
            };
            let mut args = Vec::new();
            while starts_apat(p.peek()) {
                args.push(apat(p)?);
            }
            Ok(RawPat::Con(name, args))
        }
        _ => apat(p),
    }
}

fn negative_literal(p: &mut P) -> Result<RawPat> {
    // '-' already consumed
    match p.next() {
        Some(Tok::Int(s)) => Ok(RawPat::Lit(int_lit(&format!("-{}", s)))),
        Some(Tok::Float(s)) => {
            let f: f64 = s
                .parse()
                .map_err(|_| p.err(format!("malformed float literal {}", s)))?;
            Ok(RawPat::Lit(float_lit(-f)))
        }
        Some(t) => Err(p.err(format!(
            "expected a numeric literal after '-' but found {}",
            describe(&t)
        ))),
        None => Err(p.err("expected a numeric literal after '-'".into())),
    }
}

pub(crate) fn int_lit(text: &str) -> Lit {
    match text.parse::<i64>() {
        Ok(i) => Lit::Int(i),
        Err(_) => {
            // canonicalise: strip leading zeros / plus sign
            let (neg, digits) = match text.strip_prefix('-') {
                Some(d) => (true, d),
                None => (false, text),
            };
            let digits = digits.trim_start_matches('0');
            let digits = if digits.is_empty() { "0" } else { digits };
            if neg {
                Lit::Big(format!("-{}", digits))
            } else {
                Lit::Big(digits.to_string())
            }
        }
    }
}

pub(crate) fn float_lit(f: f64) -> Lit {
    // `0.0` and `0` denote the same value (Ruby: 0 == 0.0), so integral floats
    // that fit in an i64 are canonicalised to integers.
    if f.fract() == 0.0 && f.is_finite() && f.abs() < 9.0e15 {
        Lit::Int(f as i64)
    } else {
        Lit::Float(f)
    }
}

fn apat(p: &mut P) -> Result<RawPat> {
    let tok = match p.next() {
        Some(t) => t,
        None => return Err(p.err("expected a pattern but reached end of input".into())),
    };
    match tok {
        Tok::Underscore => Ok(RawPat::Wild),
        Tok::VarId(v) => {
            if p.peek() == Some(&Tok::At) {
                p.next();
                if !starts_apat(p.peek()) {
                    return Err(p.err(format!("expected a pattern after '{}@'", v)));
                }
                let inner = apat(p)?;
                Ok(RawPat::As(v, Box::new(inner)))
            } else {
                Ok(RawPat::Var(v))
            }
        }
        Tok::Tilde => {
            if !starts_apat(p.peek()) {
                return Err(p.err("expected a pattern after '~'".into()));
            }
            Ok(RawPat::Lazy(Box::new(apat(p)?)))
        }
        Tok::Bang => {
            if !starts_apat(p.peek()) {
                return Err(p.err("expected a pattern after '!'".into()));
            }
            Ok(RawPat::Bang(Box::new(apat(p)?)))
        }
        Tok::Minus => negative_literal(p),
        Tok::ConId(name) => {
            if p.peek() == Some(&Tok::LBrace) {
                p.next();
                record_fields(p, name)
            } else {
                Ok(RawPat::Con(name, vec![]))
            }
        }
        Tok::Int(s) => Ok(RawPat::Lit(int_lit(&s))),
        Tok::Float(s) => {
            let f: f64 = s
                .parse()
                .map_err(|_| p.err(format!("malformed float literal {}", s)))?;
            Ok(RawPat::Lit(float_lit(f)))
        }
        // `"abc"` is `['a', 'b', 'c']`, as `String = [Char]` in Haskell
        Tok::Str(s) => Ok(RawPat::List(
            s.chars().map(|c| RawPat::Lit(Lit::Char(c))).collect(),
        )),
        Tok::Char(s) => Ok(RawPat::Lit(Lit::Char(
            s.chars().next().expect("lexer checked length"),
        ))),
        Tok::Sym(s) => Ok(RawPat::Lit(Lit::Sym(s))),
        Tok::LParen => {
            if p.peek() == Some(&Tok::RParen) {
                p.next();
                return Ok(RawPat::Tuple(vec![]));
            }
            let first = pattern(p)?;
            if p.peek() == Some(&Tok::Comma) {
                let mut items = vec![first];
                while p.peek() == Some(&Tok::Comma) {
                    p.next();
                    items.push(pattern(p)?);
                }
                p.expect(Tok::RParen, "')' to close tuple pattern")?;
                Ok(RawPat::Tuple(items))
            } else {
                p.expect(Tok::RParen, "')'")?;
                Ok(first)
            }
        }
        Tok::LBracket => {
            let mut items = Vec::new();
            if p.peek() == Some(&Tok::RBracket) {
                p.next();
                return Ok(RawPat::List(items));
            }
            loop {
                items.push(pattern(p)?);
                match p.next() {
                    Some(Tok::Comma) => continue,
                    Some(Tok::RBracket) => break,
                    Some(t) => {
                        return Err(p.err(format!(
                            "expected ',' or ']' in list pattern but found {}",
                            describe(&t)
                        )))
                    }
                    None => return Err(p.err("unterminated list pattern".into())),
                }
            }
            Ok(RawPat::List(items))
        }
        other => Err(p.err(format!("unexpected {} in pattern", describe(&other)))),
    }
}

fn record_fields(p: &mut P, con: String) -> Result<RawPat> {
    // '{' already consumed
    let mut fields: Vec<(String, RawPat)> = Vec::new();
    let mut wildcard = false;
    if p.peek() == Some(&Tok::RBrace) {
        p.next();
        return Ok(RawPat::Record(con, fields, false));
    }
    loop {
        match p.next() {
            Some(Tok::DotDot) => {
                wildcard = true;
                match p.next() {
                    Some(Tok::RBrace) => break,
                    _ => return Err(p.err("'..' must be the last item in a record pattern".into())),
                }
            }
            Some(Tok::VarId(f)) => {
                if fields.iter().any(|(g, _)| *g == f) {
                    return Err(CoreError::new(
                        ErrorKind::Field,
                        format!("duplicate field '{}' in record pattern for '{}'", f, con),
                    ));
                }
                if p.peek() == Some(&Tok::Equals) {
                    p.next();
                    let pat = pattern(p)?;
                    fields.push((f, pat));
                } else {
                    // NamedFieldPuns: `Con { field }` binds `field`
                    let v = f.clone();
                    fields.push((f, RawPat::Var(v)));
                }
                match p.next() {
                    Some(Tok::Comma) => continue,
                    Some(Tok::RBrace) => break,
                    Some(t) => {
                        return Err(p.err(format!(
                            "expected ',' or '}}' in record pattern but found {}",
                            describe(&t)
                        )))
                    }
                    None => return Err(p.err("unterminated record pattern".into())),
                }
            }
            Some(t) => {
                return Err(p.err(format!(
                    "expected a field name in record pattern for '{}' but found {}",
                    con,
                    describe(&t)
                )))
            }
            None => return Err(p.err("unterminated record pattern".into())),
        }
    }
    Ok(RawPat::Record(con, fields, wildcard))
}

// ---------------------------------------------------------------------------
// data declarations
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct ConDecl {
    pub name: String,
    pub arity: usize,
    /// Field names for record syntax; `None` for positional constructors.
    pub fields: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DataDecl {
    pub name: String,
    pub tyvars: Vec<String>,
    pub cons: Vec<ConDecl>,
}

fn derr(p: &P, msg: String) -> CoreError {
    CoreError::new(
        ErrorKind::DataDeclaration,
        format!("{} (column {} in {:?})", msg, p.pos(), p.src),
    )
}

/// Parse `data T a b = C1 a | C2 { f :: b } | C3 deriving (...)`.
/// The leading `data`/`newtype` keyword and the `deriving` clause are optional.
pub fn parse_data(src: &str) -> Result<DataDecl> {
    let toks = tokenize(src).map_err(|e| CoreError::new(ErrorKind::DataDeclaration, e.message))?;
    let mut p = P {
        toks,
        i: 0,
        src: src.to_string(),
    };
    if matches!(p.peek(), Some(Tok::VarId(k)) if k == "data" || k == "newtype") {
        p.next();
    }
    let name = match p.next() {
        Some(Tok::ConId(n)) => n,
        Some(t) => {
            return Err(derr(
                &p,
                format!("expected a type name but found {}", describe(&t)),
            ))
        }
        None => return Err(derr(&p, "empty data declaration".into())),
    };
    let mut tyvars = Vec::new();
    while let Some(Tok::VarId(v)) = p.peek() {
        let v = v.clone();
        p.next();
        tyvars.push(v);
    }
    match p.next() {
        Some(Tok::Equals) => {}
        Some(t) => {
            return Err(derr(
                &p,
                format!("expected '=' after type name but found {}", describe(&t)),
            ))
        }
        None => {
            return Err(derr(
                &p,
                format!("type '{}' has no constructors (expected '=')", name),
            ))
        }
    }
    let mut cons = Vec::new();
    loop {
        cons.push(con_decl(&mut p)?);
        match p.peek() {
            Some(Tok::Pipe) => {
                p.next();
            }
            Some(Tok::VarId(k)) if k == "deriving" => {
                // skip the rest
                p.i = p.toks.len();
                break;
            }
            None => break,
            Some(t) => {
                return Err(derr(
                    &p,
                    format!("unexpected {} in data declaration", describe(t)),
                ));
            }
        }
    }
    // duplicate constructor names
    for (i, c) in cons.iter().enumerate() {
        if cons[..i].iter().any(|d| d.name == c.name) {
            return Err(CoreError::new(
                ErrorKind::DataDeclaration,
                format!(
                    "constructor '{}' is declared twice in type '{}'",
                    c.name, name
                ),
            ));
        }
    }
    Ok(DataDecl { name, tyvars, cons })
}

fn con_decl(p: &mut P) -> Result<ConDecl> {
    let name = match p.next() {
        Some(Tok::ConId(n)) => n,
        Some(t) => {
            return Err(derr(
                p,
                format!("expected a constructor name but found {}", describe(&t)),
            ))
        }
        None => return Err(derr(p, "expected a constructor name".into())),
    };
    if p.peek() == Some(&Tok::LBrace) {
        p.next();
        let mut fields: Vec<String> = Vec::new();
        if p.peek() == Some(&Tok::RBrace) {
            p.next();
            return Ok(ConDecl {
                name,
                arity: 0,
                fields: Some(fields),
            });
        }
        loop {
            // f1, f2 :: Type
            let mut names = Vec::new();
            loop {
                match p.next() {
                    Some(Tok::VarId(f)) => names.push(f),
                    Some(t) => {
                        return Err(derr(
                            p,
                            format!("expected a field name but found {}", describe(&t)),
                        ))
                    }
                    None => return Err(derr(p, "unterminated record declaration".into())),
                }
                match p.next() {
                    Some(Tok::Comma) => continue,
                    Some(Tok::DoubleColon) => break,
                    Some(t) => {
                        return Err(derr(
                            p,
                            format!("expected '::' after field name but found {}", describe(&t)),
                        ))
                    }
                    None => return Err(derr(p, "unterminated record declaration".into())),
                }
            }
            type_expr(p)?;
            for f in names {
                if fields.contains(&f) {
                    return Err(CoreError::new(
                        ErrorKind::DataDeclaration,
                        format!("field '{}' is declared twice in constructor '{}'", f, name),
                    ));
                }
                fields.push(f);
            }
            match p.next() {
                Some(Tok::Comma) => continue,
                Some(Tok::RBrace) => break,
                Some(t) => {
                    return Err(derr(
                        p,
                        format!("expected ',' or '}}' but found {}", describe(&t)),
                    ))
                }
                None => return Err(derr(p, "unterminated record declaration".into())),
            }
        }
        let arity = fields.len();
        return Ok(ConDecl {
            name,
            arity,
            fields: Some(fields),
        });
    }
    let mut arity = 0;
    while starts_atype(p.peek()) {
        atype(p)?;
        arity += 1;
    }
    Ok(ConDecl {
        name,
        arity,
        fields: None,
    })
}

fn starts_atype(t: Option<&Tok>) -> bool {
    matches!(
        t,
        Some(Tok::ConId(_))
            | Some(Tok::VarId(_))
            | Some(Tok::LParen)
            | Some(Tok::LBracket)
            | Some(Tok::Bang)
    ) && !matches!(t, Some(Tok::VarId(k)) if k == "deriving")
}

fn type_expr(p: &mut P) -> Result<()> {
    if !starts_atype(p.peek()) {
        return Err(derr(p, "expected a type".into()));
    }
    while starts_atype(p.peek()) {
        atype(p)?;
    }
    if p.peek() == Some(&Tok::Arrow) {
        p.next();
        type_expr(p)?;
    }
    Ok(())
}

fn atype(p: &mut P) -> Result<()> {
    match p.next() {
        Some(Tok::ConId(_)) | Some(Tok::VarId(_)) => Ok(()),
        Some(Tok::Bang) => atype(p),
        Some(Tok::LParen) => {
            if p.peek() == Some(&Tok::RParen) {
                p.next();
                return Ok(());
            }
            type_expr(p)?;
            while p.peek() == Some(&Tok::Comma) {
                p.next();
                type_expr(p)?;
            }
            match p.next() {
                Some(Tok::RParen) => Ok(()),
                _ => Err(derr(p, "expected ')' in type".into())),
            }
        }
        Some(Tok::LBracket) => {
            type_expr(p)?;
            match p.next() {
                Some(Tok::RBracket) => Ok(()),
                _ => Err(derr(p, "expected ']' in type".into())),
            }
        }
        Some(t) => Err(derr(p, format!("unexpected {} in type", describe(&t)))),
        None => Err(derr(p, "unexpected end of type".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RawPat::*;

    fn v(s: &str) -> RawPat {
        Var(s.into())
    }
    fn con(s: &str, args: Vec<RawPat>) -> RawPat {
        Con(s.into(), args)
    }

    #[test]
    fn simple_patterns() {
        assert_eq!(parse_pattern("_").unwrap(), Wild);
        assert_eq!(parse_pattern("x").unwrap(), v("x"));
        assert_eq!(parse_pattern("Nothing").unwrap(), con("Nothing", vec![]));
        assert_eq!(parse_pattern("Just x").unwrap(), con("Just", vec![v("x")]));
        assert_eq!(
            parse_pattern("Just (Just x)").unwrap(),
            con("Just", vec![con("Just", vec![v("x")])])
        );
        assert_eq!(
            parse_pattern("Pair a Nothing").unwrap(),
            con("Pair", vec![v("a"), con("Nothing", vec![])])
        );
    }

    #[test]
    fn lists_and_cons() {
        assert_eq!(parse_pattern("[]").unwrap(), List(vec![]));
        assert_eq!(parse_pattern("[a, b]").unwrap(), List(vec![v("a"), v("b")]));
        assert_eq!(
            parse_pattern("(x:xs)").unwrap(),
            Cons(Box::new(v("x")), Box::new(v("xs")))
        );
        assert_eq!(
            parse_pattern("x:y:rest").unwrap(),
            Cons(
                Box::new(v("x")),
                Box::new(Cons(Box::new(v("y")), Box::new(v("rest"))))
            )
        );
        assert_eq!(
            parse_pattern("(Just x : xs)").unwrap(),
            Cons(Box::new(con("Just", vec![v("x")])), Box::new(v("xs")))
        );
    }

    #[test]
    fn tuples_unit_as_lazy_bang() {
        assert_eq!(parse_pattern("()").unwrap(), Tuple(vec![]));
        assert_eq!(
            parse_pattern("(a, b, _)").unwrap(),
            Tuple(vec![v("a"), v("b"), Wild])
        );
        assert_eq!(parse_pattern("(a)").unwrap(), v("a"));
        assert_eq!(
            parse_pattern("all@(x:_)").unwrap(),
            As(
                "all".into(),
                Box::new(Cons(Box::new(v("x")), Box::new(Wild)))
            )
        );
        assert_eq!(
            parse_pattern("~(a, b)").unwrap(),
            Lazy(Box::new(Tuple(vec![v("a"), v("b")])))
        );
        assert_eq!(parse_pattern("!x").unwrap(), Bang(Box::new(v("x"))));
        assert_eq!(
            parse_pattern("Just !x").unwrap(),
            con("Just", vec![Bang(Box::new(v("x")))])
        );
    }

    #[test]
    fn literals() {
        assert_eq!(parse_pattern("0").unwrap(), Lit(super::Lit::Int(0)));
        assert_eq!(parse_pattern("-1").unwrap(), Lit(super::Lit::Int(-1)));
        assert_eq!(parse_pattern("(-1)").unwrap(), Lit(super::Lit::Int(-1)));
        assert_eq!(
            parse_pattern("Just (-2)").unwrap(),
            con("Just", vec![Lit(super::Lit::Int(-2))])
        );
        assert_eq!(parse_pattern("1.5").unwrap(), Lit(super::Lit::Float(1.5)));
        assert_eq!(parse_pattern("2.0").unwrap(), Lit(super::Lit::Int(2)));
        assert_eq!(
            parse_pattern("\"hi\"").unwrap(),
            List(vec![Lit(super::Lit::Char('h')), Lit(super::Lit::Char('i'))])
        );
        assert_eq!(parse_pattern("\"\"").unwrap(), List(vec![]));
        assert_eq!(parse_pattern("'c'").unwrap(), Lit(super::Lit::Char('c')));
        assert_eq!(
            parse_pattern(":ok").unwrap(),
            Lit(super::Lit::Sym("ok".into()))
        );
        assert_eq!(
            parse_pattern("123456789012345678901234567890").unwrap(),
            Lit(super::Lit::Big("123456789012345678901234567890".into()))
        );
        assert_eq!(parse_pattern("0x10").unwrap(), Lit(super::Lit::Int(16)));
    }

    #[test]
    fn records() {
        assert_eq!(
            parse_pattern("Person { name = n, age }").unwrap(),
            Record(
                "Person".into(),
                vec![("name".into(), v("n")), ("age".into(), v("age"))],
                false
            )
        );
        assert_eq!(
            parse_pattern("Person {}").unwrap(),
            Record("Person".into(), vec![], false)
        );
        assert_eq!(
            parse_pattern("Person { name = _, .. }").unwrap(),
            Record("Person".into(), vec![("name".into(), Wild)], true)
        );
        assert_eq!(
            parse_pattern("Just Person { .. }").unwrap(),
            con("Just", vec![Record("Person".into(), vec![], true)])
        );
    }

    #[test]
    fn syntax_errors() {
        for bad in [
            "",
            "(",
            ")",
            "Just (",
            "[a,",
            "x@",
            "(a,",
            "Just x)",
            "a b",
            "~",
            "P { name }}",
            "P { .. , x }",
            "P { = x }",
            "-",
            "-x",
            "x :",
            "1 2",
        ] {
            let r = parse_pattern(bad);
            assert!(r.is_err(), "expected error for {:?}, got {:?}", bad, r);
            assert_eq!(r.unwrap_err().kind, ErrorKind::Syntax, "kind for {:?}", bad);
        }
        assert_eq!(
            parse_pattern("P { a = 1, a = 2 }").unwrap_err().kind,
            ErrorKind::Field
        );
    }

    #[test]
    fn data_declarations() {
        let d = parse_data("data Maybe a = Nothing | Just a").unwrap();
        assert_eq!(d.name, "Maybe");
        assert_eq!(d.tyvars, vec!["a".to_string()]);
        assert_eq!(
            d.cons,
            vec![
                ConDecl {
                    name: "Nothing".into(),
                    arity: 0,
                    fields: None
                },
                ConDecl {
                    name: "Just".into(),
                    arity: 1,
                    fields: None
                },
            ]
        );

        let d = parse_data("Shape = Circle Double | Rect Double Double | Poly [(Double, Double)] deriving (Show, Eq)").unwrap();
        assert_eq!(
            d.cons.iter().map(|c| c.arity).collect::<Vec<_>>(),
            vec![1, 2, 1]
        );

        let d = parse_data("Tree a = Leaf | Node (Tree a) a (Tree a)").unwrap();
        assert_eq!(d.cons[1].arity, 3);

        let d =
            parse_data("data Person = Person { name :: String, age, shoe :: Int } | Anon").unwrap();
        assert_eq!(
            d.cons[0].fields,
            Some(vec!["name".into(), "age".into(), "shoe".into()])
        );
        assert_eq!(d.cons[0].arity, 3);
        assert_eq!(d.cons[1].arity, 0);

        let d = parse_data("newtype Wrap = Wrap { unwrap :: Int -> Int }").unwrap();
        assert_eq!(d.cons[0].arity, 1);

        let d = parse_data("data Strict = S !Int {-# UNPACK #-} !Double").unwrap();
        assert_eq!(d.cons[0].arity, 2);

        let d = parse_data("Unit = Unit ()").unwrap();
        assert_eq!(d.cons[0].arity, 1);
    }

    #[test]
    fn data_errors() {
        for bad in [
            "",
            "data",
            "Maybe a",
            "Maybe = ",
            "Maybe = Just a |",
            "maybe = X",
            "T = A | A",
            "T = R { a :: Int, a :: Int }",
            "T = A ) B",
            "T = R { a Int }",
        ] {
            let r = parse_data(bad);
            assert!(r.is_err(), "expected error for {:?}, got {:?}", bad, r);
            assert_eq!(
                r.unwrap_err().kind,
                ErrorKind::DataDeclaration,
                "kind for {:?}",
                bad
            );
        }
    }
}
