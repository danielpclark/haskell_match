//! Parser for the Haskell subset: declarations (data, signatures, function
//! equations with guards and `where`, pattern bindings) and expressions
//! (application, infix operators with Prelude fixities, sections, lambdas,
//! `if`, `case`, `let`, lists, ranges, comprehensions, tuples).

use super::ast::*;
use super::layout::layout;
use super::lexer::{tokenize, Tok, Token};
use crate::core::ast::{Lit, RawPat};
use crate::core::error::{CoreError, ErrorKind, Result};
use crate::core::parser::{ConDecl, DataDecl};

struct P {
    toks: Vec<Token>,
    i: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Assoc {
    Left,
    Right,
    None,
}

fn fixity(op: &str) -> (u8, Assoc) {
    match op {
        "." => (9, Assoc::Right),
        "!!" => (9, Assoc::Left),
        "^" | "^^" | "**" => (8, Assoc::Right),
        "*" | "/" | "div" | "mod" | "rem" | "quot" => (7, Assoc::Left),
        "+" | "-" => (6, Assoc::Left),
        ":" | "++" => (5, Assoc::Right),
        "==" | "/=" | "<" | "<=" | ">" | ">=" | "elem" | "notElem" => (4, Assoc::None),
        "&&" => (3, Assoc::Right),
        "||" => (2, Assoc::Right),
        ">>" | ">>=" => (1, Assoc::Left),
        "$" | "$!" | "seq" => (0, Assoc::Right),
        _ => (9, Assoc::Left),
    }
}

impl P {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }
    fn peek_at(&self, n: usize) -> &Tok {
        let j = (self.i + n).min(self.toks.len() - 1);
        &self.toks[j].tok
    }
    fn here(&self) -> (usize, usize) {
        let t = &self.toks[self.i];
        (t.line, t.col)
    }
    fn next(&mut self) -> Tok {
        let t = self.toks[self.i].tok.clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }
    fn err<T>(&self, msg: impl std::fmt::Display) -> Result<T> {
        let (l, c) = self.here();
        Err(CoreError::new(
            ErrorKind::Syntax,
            format!("{}:{}: {}", l, c, msg),
        ))
    }
    fn expect(&mut self, want: Tok, what: &str) -> Result<()> {
        if *self.peek() == want {
            self.next();
            Ok(())
        } else {
            self.err(format!(
                "expected {} but found {}",
                what,
                describe(self.peek())
            ))
        }
    }
    fn at(&self, t: Tok) -> bool {
        *self.peek() == t
    }
    fn is_open(&self) -> bool {
        matches!(self.peek(), Tok::LBrace | Tok::VLBrace)
    }
    fn is_close(&self) -> bool {
        matches!(self.peek(), Tok::RBrace | Tok::VRBrace)
    }
    fn is_semi(&self) -> bool {
        matches!(self.peek(), Tok::Semi | Tok::VSemi)
    }
}

pub fn describe(t: &Tok) -> String {
    match t {
        Tok::VarId(v) => format!("'{}'", v),
        Tok::ConId(c) => format!("'{}'", c),
        Tok::VarSym(s) | Tok::ConSym(s) => format!("operator '{}'", s),
        Tok::Int(s) | Tok::Float(s) => format!("literal {}", s),
        Tok::Char(c) => format!("literal {:?}", c),
        Tok::Str(s) => format!("literal {:?}", s),
        Tok::VLBrace => "start of block".into(),
        Tok::VRBrace => "end of block".into(),
        Tok::VSemi => "end of line".into(),
        Tok::Eof => "end of input".into(),
        other => format!("{:?}", other).to_lowercase(),
    }
}

/// `Data.Map.Map` -> `Map`: a qualified constructor without its module.
fn unqualify(name: String) -> String {
    match name.rfind('.') {
        Some(i) => name[i + 1..].to_string(),
        None => name,
    }
}

/// Parse a module (a whole source text).
pub fn parse_module(src: &str) -> Result<Module> {
    let toks = layout(tokenize(src)?);
    let mut p = P { toks, i: 0 };
    let mut name = None;
    if p.at(Tok::Module) {
        p.next();
        match p.next() {
            Tok::ConId(n) => name = Some(n),
            t => return p.err(format!("expected a module name but found {}", describe(&t))),
        }
        // optional export list
        if p.at(Tok::LParen) {
            let mut depth = 0;
            loop {
                match p.next() {
                    Tok::LParen => depth += 1,
                    Tok::RParen => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    Tok::Eof => return p.err("unterminated export list"),
                    _ => {}
                }
            }
        }
        p.expect(Tok::Where, "'where' after module header")?;
    }
    let decls = p.block(|p| p.top_decl())?;
    if !p.at(Tok::Eof) {
        return p.err(format!("unexpected {}", describe(p.peek())));
    }
    Ok(Module {
        name,
        decls: group(decls, &p)?,
    })
}

/// Group consecutive equations of one function into a single `Decl::Fun`.
fn group(items: Vec<Decl>, p: &P) -> Result<Vec<Decl>> {
    let mut out: Vec<Decl> = Vec::new();
    for d in items {
        match d {
            Decl::Fun(mut eqs) => {
                if let Some(Decl::Fun(prev)) = out.last_mut() {
                    if prev[0].name == eqs[0].name {
                        if prev[0].pats.len() != eqs[0].pats.len() {
                            return Err(CoreError::new(
                                ErrorKind::ClauseArity,
                                format!(
                                    "{}: equations for '{}' have different numbers of arguments",
                                    eqs[0].line, eqs[0].name
                                ),
                            ));
                        }
                        prev.append(&mut eqs);
                        continue;
                    }
                }
                // a function defined again after other declarations is an error
                for earlier in &out {
                    if let Decl::Fun(e) = earlier {
                        if e[0].name == eqs[0].name {
                            return Err(CoreError::new(
                                ErrorKind::Syntax,
                                format!(
                                    "{}: equations for '{}' are not contiguous (first defined at line {})",
                                    eqs[0].line, eqs[0].name, e[0].line
                                ),
                            ));
                        }
                    }
                }
                out.push(Decl::Fun(eqs));
            }
            other => out.push(other),
        }
    }
    let _ = p;
    Ok(out)
}

impl P {
    /// `{ item ; item ; ... }` with explicit or layout braces.
    fn block<T>(&mut self, mut item: impl FnMut(&mut P) -> Result<Option<T>>) -> Result<Vec<T>> {
        if !self.is_open() {
            return self.err(format!(
                "expected a block but found {}",
                describe(self.peek())
            ));
        }
        self.next();
        let mut items = Vec::new();
        loop {
            while self.is_semi() {
                self.next();
            }
            if self.is_close() {
                self.next();
                break;
            }
            if self.at(Tok::Eof) {
                return self.err("unexpected end of input inside a block");
            }
            if let Some(it) = item(self)? {
                items.push(it);
            }
            if self.is_semi() {
                continue;
            }
            if self.is_close() {
                self.next();
                break;
            }
            return self.err(format!("unexpected {}", describe(self.peek())));
        }
        Ok(items)
    }

    fn top_decl(&mut self) -> Result<Option<Decl>> {
        match self.peek() {
            Tok::Import => {
                // import [qualified] Name [as X] [hiding] [(..)]
                while !self.is_semi() && !self.is_close() && !self.at(Tok::Eof) {
                    self.next();
                }
                Ok(None)
            }
            Tok::Type | Tok::Class | Tok::Instance | Tok::Infix | Tok::Infixl | Tok::Infixr => {
                let what = describe(self.peek());
                if matches!(self.peek(), Tok::Class | Tok::Instance) {
                    return self.err(format!(
                        "{} declarations are not supported (type classes are out of scope)",
                        what
                    ));
                }
                while !self.is_semi() && !self.is_close() && !self.at(Tok::Eof) {
                    if self.is_open() {
                        self.skip_block()?;
                    } else {
                        self.next();
                    }
                }
                Ok(None)
            }
            Tok::Data | Tok::Newtype => self.data_decl().map(Some),
            _ => self.decl().map(Some),
        }
    }

    fn skip_block(&mut self) -> Result<()> {
        let mut depth = 0;
        loop {
            match self.peek() {
                Tok::LBrace | Tok::VLBrace => depth += 1,
                Tok::RBrace | Tok::VRBrace => {
                    depth -= 1;
                    if depth == 0 {
                        self.next();
                        return Ok(());
                    }
                }
                Tok::Eof => return self.err("unexpected end of input"),
                _ => {}
            }
            self.next();
        }
    }

    fn data_decl(&mut self) -> Result<Decl> {
        let (line, _) = self.here();
        self.next(); // data / newtype
        let name = match self.next() {
            Tok::ConId(n) => unqualify(n),
            t => return self.err(format!("expected a type name but found {}", describe(&t))),
        };
        let mut tyvars = Vec::new();
        while let Tok::VarId(v) = self.peek() {
            tyvars.push(v.clone());
            self.next();
        }
        let mut cons = Vec::new();
        if self.at(Tok::Equals) {
            self.next();
            loop {
                cons.push(self.con_decl()?);
                if self.at(Tok::Pipe) {
                    self.next();
                } else {
                    break;
                }
            }
        }
        let mut deriving = Vec::new();
        if self.at(Tok::Deriving) {
            self.next();
            if self.at(Tok::LParen) {
                self.next();
                loop {
                    match self.next() {
                        Tok::ConId(c) => deriving.push(unqualify(c)),
                        Tok::RParen => break,
                        Tok::Comma => {}
                        t => {
                            return self
                                .err(format!("unexpected {} in deriving clause", describe(&t)))
                        }
                    }
                }
            } else {
                match self.next() {
                    Tok::ConId(c) => deriving.push(unqualify(c)),
                    t => {
                        return self.err(format!("unexpected {} in deriving clause", describe(&t)))
                    }
                }
            }
        }
        if cons.is_empty() {
            return self.err(format!("type '{}' has no constructors", name));
        }
        for (i, c) in cons.iter().enumerate() {
            if cons[..i].iter().any(|d| d.name == c.name) {
                return Err(CoreError::new(
                    ErrorKind::DataDeclaration,
                    format!("{}: constructor '{}' is declared twice", line, c.name),
                ));
            }
        }
        Ok(Decl::Data {
            decl: DataDecl { name, tyvars, cons },
            deriving,
            line,
        })
    }

    fn con_decl(&mut self) -> Result<ConDecl> {
        let name = match self.next() {
            Tok::ConId(n) => unqualify(n),
            t => {
                return self.err(format!(
                    "expected a constructor name but found {}",
                    describe(&t)
                ))
            }
        };
        if self.at(Tok::LBrace) {
            self.next();
            let mut fields = Vec::new();
            if self.at(Tok::RBrace) {
                self.next();
                return Ok(ConDecl {
                    name,
                    arity: 0,
                    fields: Some(fields),
                });
            }
            loop {
                let mut names = Vec::new();
                loop {
                    match self.next() {
                        Tok::VarId(f) => names.push(f),
                        t => {
                            return self
                                .err(format!("expected a field name but found {}", describe(&t)))
                        }
                    }
                    match self.next() {
                        Tok::Comma => {}
                        Tok::DoubleColon => break,
                        t => return self.err(format!("expected '::' but found {}", describe(&t))),
                    }
                }
                self.skip_type_until(&[Tok::Comma, Tok::RBrace])?;
                for f in names {
                    if fields.contains(&f) {
                        return self.err(format!("field '{}' is declared twice", f));
                    }
                    fields.push(f);
                }
                match self.next() {
                    Tok::Comma => {}
                    Tok::RBrace => break,
                    t => {
                        return self.err(format!("expected ',' or '}}' but found {}", describe(&t)))
                    }
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
        while self.starts_atype() {
            self.atype()?;
            arity += 1;
        }
        Ok(ConDecl {
            name,
            arity,
            fields: None,
        })
    }

    fn starts_atype(&self) -> bool {
        matches!(
            self.peek(),
            Tok::ConId(_) | Tok::VarId(_) | Tok::LParen | Tok::LBracket | Tok::Bang
        )
    }

    /// Skip one atomic type (we keep no type information).
    fn atype(&mut self) -> Result<()> {
        match self.next() {
            Tok::ConId(_) | Tok::VarId(_) => Ok(()),
            Tok::Bang => self.atype(),
            Tok::LParen => {
                let mut depth = 1;
                while depth > 0 {
                    match self.next() {
                        Tok::LParen => depth += 1,
                        Tok::RParen => depth -= 1,
                        Tok::Eof => return self.err("unterminated type"),
                        _ => {}
                    }
                }
                Ok(())
            }
            Tok::LBracket => {
                let mut depth = 1;
                while depth > 0 {
                    match self.next() {
                        Tok::LBracket => depth += 1,
                        Tok::RBracket => depth -= 1,
                        Tok::Eof => return self.err("unterminated type"),
                        _ => {}
                    }
                }
                Ok(())
            }
            t => self.err(format!("unexpected {} in type", describe(&t))),
        }
    }

    /// Skip a type until one of `stops` at depth 0; returns the number of
    /// top-level `->` arrows seen (the arity of a signature).
    fn skip_type_until(&mut self, stops: &[Tok]) -> Result<usize> {
        let mut depth = 0;
        let mut arrows = 0;
        loop {
            let t = self.peek().clone();
            if depth == 0
                && (stops.contains(&t) || self.is_semi() || self.is_close() || t == Tok::Eof)
            {
                return Ok(arrows);
            }
            match t {
                Tok::LParen | Tok::LBracket => depth += 1,
                Tok::RParen | Tok::RBracket => depth -= 1,
                Tok::RArrow if depth == 0 => arrows += 1,
                _ => {}
            }
            self.next();
        }
    }

    /// A declaration inside any block: signature, equation or pattern binding.
    fn decl(&mut self) -> Result<Decl> {
        let (line, _) = self.here();
        // signature: var (, var)* :: type
        if let Tok::VarId(_) = self.peek() {
            let mut j = 1;
            while matches!(self.peek_at(j), Tok::Comma)
                && matches!(self.peek_at(j + 1), Tok::VarId(_))
            {
                j += 2;
            }
            if matches!(self.peek_at(j), Tok::DoubleColon) {
                let mut names = Vec::new();
                loop {
                    match self.next() {
                        Tok::VarId(v) => names.push(v),
                        Tok::Comma => {}
                        Tok::DoubleColon => break,
                        t => return self.err(format!("unexpected {} in signature", describe(&t))),
                    }
                }
                let arity = self.skip_type_until(&[])?;
                return Ok(Decl::Sig { names, arity, line });
            }
        }
        // function equation: var apat* rhs   |  infix: pat varop pat rhs
        if let Tok::VarId(name) = self.peek().clone() {
            let next = self.peek_at(1).clone();
            if matches!(next, Tok::Equals | Tok::Pipe) {
                // `x = e` : a simple pattern binding (a value)
                self.next();
                let (rhs, wheres) = self.rhs(Tok::Equals)?;
                return Ok(Decl::PatBind {
                    pat: RawPat::Var(name),
                    rhs,
                    wheres,
                    line,
                });
            }
            if self.starts_apat_at(1) {
                self.next();
                let mut pats = Vec::new();
                while self.starts_apat() {
                    pats.push(self.apat()?);
                }
                if let Tok::VarSym(op) | Tok::ConSym(op) = self.peek().clone() {
                    if !matches!(op.as_str(), "=") {
                        // infix definition with a left operand pattern: (x `op` y) handled below
                        return self
                            .err(format!("operator definitions ('{}') are not supported", op));
                    }
                }
                let (rhs, wheres) = self.rhs(Tok::Equals)?;
                return Ok(Decl::Fun(vec![Equation {
                    name,
                    pats,
                    rhs,
                    wheres,
                    line,
                }]));
            }
        }
        // pattern binding: pat = e
        let pat = self.pattern()?;
        if let Tok::VarSym(op) = self.peek().clone() {
            return self.err(format!("operator definitions ('{}') are not supported", op));
        }
        if self.at(Tok::Backtick) {
            return self.err("infix function definitions are not supported; write `f x y = ...`");
        }
        let (rhs, wheres) = self.rhs(Tok::Equals)?;
        Ok(Decl::PatBind {
            pat,
            rhs,
            wheres,
            line,
        })
    }

    /// `= e [where decls]` or guarded `| g = e ... [where decls]`; `sep` is
    /// `=` for equations and `->` for case alternatives.
    fn rhs(&mut self, sep: Tok) -> Result<(Rhs, Vec<Decl>)> {
        let rhs = if self.at(Tok::Pipe) {
            let mut guards = Vec::new();
            while self.at(Tok::Pipe) {
                self.next();
                let g = self.expr()?;
                self.expect(sep.clone(), if sep == Tok::Equals { "'='" } else { "'->'" })?;
                let e = self.expr()?;
                guards.push((g, e));
            }
            Rhs::Guarded(guards)
        } else {
            self.expect(sep.clone(), if sep == Tok::Equals { "'='" } else { "'->'" })?;
            Rhs::Plain(self.expr()?)
        };
        let mut wheres = Vec::new();
        if self.at(Tok::Where) {
            self.next();
            let items = self.block(|p| p.decl().map(Some))?;
            wheres = group(items, self)?;
        }
        Ok((rhs, wheres))
    }

    // ----------------------------------------------------------------- patterns

    fn starts_apat(&self) -> bool {
        self.starts_apat_at(0)
    }

    fn starts_apat_at(&self, n: usize) -> bool {
        matches!(
            self.peek_at(n),
            Tok::VarId(_)
                | Tok::ConId(_)
                | Tok::Int(_)
                | Tok::Float(_)
                | Tok::Char(_)
                | Tok::Str(_)
                | Tok::Underscore
                | Tok::LParen
                | Tok::LBracket
                | Tok::Tilde
                | Tok::Bang
        )
    }

    /// pattern := lpat (':' pattern)?
    fn pattern(&mut self) -> Result<RawPat> {
        let head = self.lpat()?;
        if let Tok::ConSym(s) = self.peek() {
            if s == ":" {
                self.next();
                let tail = self.pattern()?;
                return Ok(RawPat::Cons(Box::new(head), Box::new(tail)));
            }
        }
        Ok(head)
    }

    fn lpat(&mut self) -> Result<RawPat> {
        match self.peek().clone() {
            Tok::ConId(name) if !matches!(self.peek_at(1), Tok::LBrace) => {
                let name = unqualify(name);
                self.next();
                let mut args = Vec::new();
                while self.starts_apat() {
                    args.push(self.apat()?);
                }
                Ok(RawPat::Con(name, args))
            }
            Tok::VarSym(s) if s == "-" => {
                self.next();
                match self.next() {
                    Tok::Int(n) => Ok(RawPat::Lit(crate::core::parser::int_lit(&format!(
                        "-{}",
                        n
                    )))),
                    Tok::Float(f) => {
                        let v: f64 = f
                            .parse()
                            .map_err(|_| self.err::<()>("bad float").unwrap_err())?;
                        Ok(RawPat::Lit(crate::core::parser::float_lit(-v)))
                    }
                    t => self.err(format!(
                        "expected a number after '-' in pattern, found {}",
                        describe(&t)
                    )),
                }
            }
            _ => self.apat(),
        }
    }

    fn apat(&mut self) -> Result<RawPat> {
        match self.next() {
            Tok::Underscore => Ok(RawPat::Wild),
            Tok::VarId(v) => {
                if self.at(Tok::At) {
                    self.next();
                    let inner = self.apat()?;
                    Ok(RawPat::As(v, Box::new(inner)))
                } else {
                    Ok(RawPat::Var(v))
                }
            }
            Tok::Tilde => Ok(RawPat::Lazy(Box::new(self.apat()?))),
            Tok::Bang => Ok(RawPat::Bang(Box::new(self.apat()?))),
            Tok::ConId(name) => {
                let name = unqualify(name);
                if self.at(Tok::LBrace) {
                    self.next();
                    self.record_pat(name)
                } else {
                    Ok(RawPat::Con(name, vec![]))
                }
            }
            Tok::Int(s) => Ok(RawPat::Lit(crate::core::parser::int_lit(&s))),
            Tok::Float(s) => {
                let v: f64 = s
                    .parse()
                    .map_err(|_| self.err::<()>("bad float").unwrap_err())?;
                Ok(RawPat::Lit(crate::core::parser::float_lit(v)))
            }
            Tok::Char(c) => Ok(RawPat::Lit(Lit::Char(c))),
            Tok::Str(s) => Ok(RawPat::List(
                s.chars().map(|c| RawPat::Lit(Lit::Char(c))).collect(),
            )),
            Tok::LParen => {
                if self.at(Tok::RParen) {
                    self.next();
                    return Ok(RawPat::Tuple(vec![]));
                }
                let first = self.pattern()?;
                if self.at(Tok::Comma) {
                    let mut items = vec![first];
                    while self.at(Tok::Comma) {
                        self.next();
                        items.push(self.pattern()?);
                    }
                    self.expect(Tok::RParen, "')'")?;
                    Ok(RawPat::Tuple(items))
                } else {
                    self.expect(Tok::RParen, "')'")?;
                    Ok(first)
                }
            }
            Tok::LBracket => {
                let mut items = Vec::new();
                if self.at(Tok::RBracket) {
                    self.next();
                    return Ok(RawPat::List(items));
                }
                loop {
                    items.push(self.pattern()?);
                    match self.next() {
                        Tok::Comma => {}
                        Tok::RBracket => break,
                        t => {
                            return self.err(format!(
                                "expected ',' or ']' in list pattern, found {}",
                                describe(&t)
                            ))
                        }
                    }
                }
                Ok(RawPat::List(items))
            }
            t => self.err(format!("unexpected {} in pattern", describe(&t))),
        }
    }

    fn record_pat(&mut self, con: String) -> Result<RawPat> {
        let mut fields = Vec::new();
        let mut rest = false;
        if self.at(Tok::RBrace) {
            self.next();
            return Ok(RawPat::Record(con, fields, false));
        }
        loop {
            match self.next() {
                Tok::DotDot => {
                    rest = true;
                    self.expect(Tok::RBrace, "'}' after '..'")?;
                    break;
                }
                Tok::VarId(f) => {
                    if self.at(Tok::Equals) {
                        self.next();
                        let p = self.pattern()?;
                        fields.push((f, p));
                    } else {
                        let v = f.clone();
                        fields.push((f, RawPat::Var(v)));
                    }
                    match self.next() {
                        Tok::Comma => {}
                        Tok::RBrace => break,
                        t => {
                            return self.err(format!(
                                "expected ',' or '}}' in record pattern, found {}",
                                describe(&t)
                            ))
                        }
                    }
                }
                t => return self.err(format!("unexpected {} in record pattern", describe(&t))),
            }
        }
        Ok(RawPat::Record(con, fields, rest))
    }

    // -------------------------------------------------------------- expressions

    pub fn expr(&mut self) -> Result<Expr> {
        let e = self.infix_expr(0)?;
        if self.at(Tok::DoubleColon) {
            // type annotation: skip
            self.next();
            self.skip_type_until(&[
                Tok::RParen,
                Tok::Comma,
                Tok::RBracket,
                Tok::Then,
                Tok::Else,
                Tok::Of,
                Tok::In,
            ])?;
        }
        Ok(e)
    }

    fn peek_op(&self) -> Option<(String, usize)> {
        match self.peek() {
            Tok::VarSym(s) | Tok::ConSym(s) => Some((s.clone(), 1)),
            Tok::Backtick => {
                if let Tok::VarId(v) = self.peek_at(1) {
                    if matches!(self.peek_at(2), Tok::Backtick) {
                        return Some((v.clone(), 3));
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Precedence climbing over `lexp (op lexp)*`.
    fn infix_expr(&mut self, min_prec: u8) -> Result<Expr> {
        let mut lhs = if let Tok::VarSym(s) = self.peek() {
            if s == "-" {
                self.next();
                let operand = self.infix_expr(7)?;
                Expr::Neg(Box::new(operand))
            } else {
                self.lexp()?
            }
        } else {
            self.lexp()?
        };
        while let Some((op, width)) = self.peek_op() {
            let (prec, assoc) = fixity(&op);
            if prec < min_prec {
                break;
            }
            // consume the operator tokens
            for _ in 0..width {
                self.next();
            }
            let next_min = match assoc {
                Assoc::Left | Assoc::None => prec + 1,
                Assoc::Right => prec,
            };
            let rhs = self.infix_expr(next_min)?;
            lhs = Expr::BinOp(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn lexp(&mut self) -> Result<Expr> {
        match self.peek() {
            Tok::Backslash => {
                self.next();
                let mut pats = Vec::new();
                while self.starts_apat() {
                    pats.push(self.apat()?);
                }
                if pats.is_empty() {
                    return self.err("a lambda needs at least one parameter");
                }
                self.expect(Tok::RArrow, "'->' in lambda")?;
                let body = self.expr()?;
                Ok(Expr::Lambda(pats, Box::new(body)))
            }
            Tok::Let => {
                self.next();
                let items = self.block(|p| p.decl().map(Some))?;
                let decls = group(items, self)?;
                self.expect(Tok::In, "'in' after let bindings")?;
                let body = self.expr()?;
                Ok(Expr::Let(decls, Box::new(body)))
            }
            Tok::If => {
                self.next();
                let c = self.expr()?;
                while self.is_semi() {
                    self.next();
                }
                self.expect(Tok::Then, "'then'")?;
                let t = self.expr()?;
                while self.is_semi() {
                    self.next();
                }
                self.expect(Tok::Else, "'else'")?;
                let e = self.expr()?;
                Ok(Expr::If(Box::new(c), Box::new(t), Box::new(e)))
            }
            Tok::Case => {
                self.next();
                let scrut = self.expr()?;
                self.expect(Tok::Of, "'of'")?;
                let alts = self.block(|p| p.alt().map(Some))?;
                if alts.is_empty() {
                    return self.err("a case expression needs at least one alternative");
                }
                Ok(Expr::Case(Box::new(scrut), alts))
            }
            Tok::Do => self.err("'do' blocks are not supported (monads are out of scope)"),
            _ => self.fexp(),
        }
    }

    fn alt(&mut self) -> Result<Alt> {
        let (line, _) = self.here();
        let pat = self.pattern()?;
        let (rhs, wheres) = self.rhs(Tok::RArrow)?;
        Ok(Alt {
            pat,
            rhs,
            wheres,
            line,
        })
    }

    fn starts_aexp(&self) -> bool {
        matches!(
            self.peek(),
            Tok::VarId(_)
                | Tok::ConId(_)
                | Tok::Int(_)
                | Tok::Float(_)
                | Tok::Char(_)
                | Tok::Str(_)
                | Tok::LParen
                | Tok::LBracket
        )
    }

    /// Function application: `aexp aexp*`.
    fn fexp(&mut self) -> Result<Expr> {
        let f = self.aexp()?;
        let mut args = Vec::new();
        while self.starts_aexp() {
            args.push(self.aexp()?);
        }
        if args.is_empty() {
            Ok(f)
        } else {
            Ok(Expr::App(Box::new(f), args))
        }
    }

    fn aexp(&mut self) -> Result<Expr> {
        match self.next() {
            Tok::VarId(v) => Ok(Expr::Var(v)),
            Tok::ConId(c) => Ok(Expr::Con(unqualify(c))),
            Tok::Int(s) => Ok(Expr::Lit(Literal::Int(s))),
            Tok::Float(s) => Ok(Expr::Lit(Literal::Float(s))),
            Tok::Char(c) => Ok(Expr::Lit(Literal::Char(c))),
            Tok::Str(s) => Ok(Expr::Lit(Literal::Str(s))),
            Tok::LParen => self.paren(),
            Tok::LBracket => self.bracket(),
            Tok::Underscore => self.err("'_' is not an expression"),
            t => self.err(format!("unexpected {} in expression", describe(&t))),
        }
    }

    /// After `(`: unit, parenthesised expression, tuple, section, or `(op)`.
    fn paren(&mut self) -> Result<Expr> {
        if self.at(Tok::RParen) {
            self.next();
            return Ok(Expr::Tuple(vec![]));
        }
        // (op) or right section (op e), but `(- e)` is negation
        if let Some((op, width)) = self.peek_op() {
            let is_minus = op == "-";
            let after_op = self.peek_at(width).clone();
            if after_op == Tok::RParen {
                for _ in 0..width {
                    self.next();
                }
                self.next();
                return Ok(Expr::OpFun(op));
            }
            if !is_minus {
                for _ in 0..width {
                    self.next();
                }
                let (prec, _) = fixity(&op);
                let e = self.infix_expr(prec)?;
                self.expect(Tok::RParen, "')' to close section")?;
                return Ok(Expr::SectionR(op, Box::new(e)));
            }
        }
        let first = self.expr()?;
        // left section (e op)
        if let Some((op, width)) = self.peek_op() {
            if self.peek_at(width) == &Tok::RParen {
                for _ in 0..width {
                    self.next();
                }
                self.next();
                return Ok(Expr::SectionL(op, Box::new(first)));
            }
        }
        if self.at(Tok::Comma) {
            let mut items = vec![first];
            while self.at(Tok::Comma) {
                self.next();
                items.push(self.expr()?);
            }
            self.expect(Tok::RParen, "')' to close tuple")?;
            return Ok(Expr::Tuple(items));
        }
        self.expect(Tok::RParen, "')'")?;
        Ok(first)
    }

    /// After `[`: list, range or comprehension.
    fn bracket(&mut self) -> Result<Expr> {
        if self.at(Tok::RBracket) {
            self.next();
            return Ok(Expr::List(vec![]));
        }
        let first = self.expr()?;
        match self.peek() {
            Tok::DotDot => {
                self.next();
                let to = if self.at(Tok::RBracket) {
                    None
                } else {
                    Some(Box::new(self.expr()?))
                };
                self.expect(Tok::RBracket, "']'")?;
                Ok(Expr::Range {
                    from: Box::new(first),
                    then: None,
                    to,
                })
            }
            Tok::Comma => {
                self.next();
                let second = self.expr()?;
                if self.at(Tok::DotDot) {
                    self.next();
                    let to = if self.at(Tok::RBracket) {
                        None
                    } else {
                        Some(Box::new(self.expr()?))
                    };
                    self.expect(Tok::RBracket, "']'")?;
                    return Ok(Expr::Range {
                        from: Box::new(first),
                        then: Some(Box::new(second)),
                        to,
                    });
                }
                let mut items = vec![first, second];
                while self.at(Tok::Comma) {
                    self.next();
                    items.push(self.expr()?);
                }
                self.expect(Tok::RBracket, "']'")?;
                Ok(Expr::List(items))
            }
            Tok::Pipe => {
                self.next();
                let mut quals = Vec::new();
                loop {
                    quals.push(self.qual()?);
                    if self.at(Tok::Comma) {
                        self.next();
                    } else {
                        break;
                    }
                }
                self.expect(Tok::RBracket, "']' to close comprehension")?;
                Ok(Expr::Comp(Box::new(first), quals))
            }
            _ => {
                self.expect(Tok::RBracket, "']'")?;
                Ok(Expr::List(vec![first]))
            }
        }
    }

    fn qual(&mut self) -> Result<Qual> {
        if self.at(Tok::Let) {
            self.next();
            let items = self.block(|p| p.decl().map(Some))?;
            return Ok(Qual::Let(group(items, self)?));
        }
        // generator `pat <- e`: try to find `<-` before the next `,` / `]` at depth 0
        let save = self.i;
        if self.starts_apat() {
            if let Ok(pat) = self.pattern() {
                if self.at(Tok::LArrow) {
                    self.next();
                    let e = self.expr()?;
                    return Ok(Qual::Gen(pat, e));
                }
            }
        }
        self.i = save;
        Ok(Qual::Guard(self.expr()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Module {
        parse_module(src).unwrap_or_else(|e| panic!("{}: {}", e.kind as u8, e.message))
    }

    fn fun<'a>(m: &'a Module, name: &str) -> &'a Vec<Equation> {
        m.decls
            .iter()
            .find_map(|d| match d {
                Decl::Fun(eqs) if eqs[0].name == name => Some(eqs),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no function {}", name))
    }

    #[test]
    fn equations_guards_where() {
        let m = parse(
            "module Geometry where\n\
             data Shape = Circle Double | Rect Double Double deriving (Show, Eq)\n\
             area :: Shape -> Double\n\
             area (Circle r) = pi * r * r\n\
             area (Rect w h) = w * h\n\
             describe s\n  | area s > 100 = \"big\"\n  | otherwise = \"small\"\n\
             sumTo n = go n 0\n  where go 0 acc = acc\n        go k acc = go (k - 1) (acc + k)\n",
        );
        assert_eq!(m.name.as_deref(), Some("Geometry"));
        assert!(
            matches!(&m.decls[0], Decl::Data { deriving, .. } if deriving == &vec!["Show".to_string(), "Eq".to_string()])
        );
        assert!(
            matches!(&m.decls[1], Decl::Sig { names, arity: 1, .. } if names == &vec!["area".to_string()])
        );
        let area = fun(&m, "area");
        assert_eq!(area.len(), 2);
        assert_eq!(render_pat(&area[0].pats[0]), "Circle hs_r");
        let describe = fun(&m, "describe");
        assert!(matches!(describe[0].rhs, Rhs::Guarded(ref g) if g.len() == 2));
        let sum_to = fun(&m, "sumTo");
        assert_eq!(sum_to[0].wheres.len(), 1);
        if let Decl::Fun(go) = &sum_to[0].wheres[0] {
            assert_eq!(go.len(), 2);
            assert_eq!(render_pat(&go[1].pats[0]), "hs_k");
        } else {
            panic!("where should hold a function");
        }
    }

    #[test]
    fn expressions() {
        let m = parse("f x = x + 2 * 3 - 1\ng = (+ 1) . (* 2) $ 3\nh = [1, 2] ++ [x * 2 | x <- [1..10], even x]\nk = \\a b -> if a then (a, b) else (b, a)\nl = let y = 1; z = 2 in y + z\nm xs = case xs of\n  [] -> 0\n  (y:_) | y > 0 -> y\n        | otherwise -> 0\nn = negate (-5) `div` 2\no = [1, 3 ..]\np = 'c' : \"ab\"\nq = (subtract 1) 5\n");
        let f = fun(&m, "f");
        // x + ((2 * 3) - 1)? No: + and - are both infixl 6: ((x + (2*3)) - 1)
        assert_eq!(
            f[0].rhs,
            Rhs::Plain(Expr::BinOp(
                "-".into(),
                Box::new(Expr::BinOp(
                    "+".into(),
                    Box::new(Expr::Var("x".into())),
                    Box::new(Expr::BinOp(
                        "*".into(),
                        Box::new(Expr::Lit(Literal::Int("2".into()))),
                        Box::new(Expr::Lit(Literal::Int("3".into())))
                    ))
                )),
                Box::new(Expr::Lit(Literal::Int("1".into())))
            ))
        );
        let g = m
            .decls
            .iter()
            .find_map(|d| {
                if let Decl::PatBind {
                    pat: RawPat::Var(n),
                    rhs,
                    ..
                } = d
                {
                    (n == "g").then_some(rhs)
                } else {
                    None
                }
            })
            .unwrap();
        // `.` binds tighter than `$`: ((+1) . (*2)) $ 3
        assert!(matches!(g, Rhs::Plain(Expr::BinOp(op, _, _)) if op == "$"));
        let h = m
            .decls
            .iter()
            .find_map(|d| {
                if let Decl::PatBind {
                    pat: RawPat::Var(n),
                    rhs,
                    ..
                } = d
                {
                    (n == "h").then_some(rhs)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(
            matches!(h, Rhs::Plain(Expr::BinOp(op, _, r)) if op == "++" && matches!(**r, Expr::Comp(_, ref q) if q.len() == 2))
        );
        let m_fn = fun(&m, "m");
        assert!(
            matches!(&m_fn[0].rhs, Rhs::Plain(Expr::Case(_, alts)) if alts.len() == 2 && matches!(alts[1].rhs, Rhs::Guarded(ref g) if g.len() == 2))
        );
        let o = m
            .decls
            .iter()
            .find_map(|d| {
                if let Decl::PatBind {
                    pat: RawPat::Var(n),
                    rhs,
                    ..
                } = d
                {
                    (n == "o").then_some(rhs)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(matches!(
            o,
            Rhs::Plain(Expr::Range {
                then: Some(_),
                to: None,
                ..
            })
        ));
    }

    #[test]
    fn errors() {
        assert!(parse_module("f x = ").is_err());
        assert!(parse_module("f x = case x of").is_err());
        assert!(parse_module("x <+> y = 1").is_err());
        assert!(parse_module("class Foo a where\n  foo :: a").is_err());
        assert!(parse_module("f = do\n  x").is_err());
        assert!(parse_module("f 0 = 1\nf x y = 2").is_err());
        assert!(parse_module("f 0 = 1\ng = 2\nf x = 3").is_err());
        let e = parse_module("f = (1 +").unwrap_err();
        assert!(e.message.starts_with("1:"), "{}", e.message);
    }
}
