//! JSON serialisation of the AST for the Ruby code generator.

use super::ast::*;
use crate::core::ast::RawPat;

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn arr(items: Vec<String>) -> String {
    format!("[{}]", items.join(","))
}

fn pat(p: &RawPat) -> String {
    let mut vars = Vec::new();
    pat_vars(p, &mut vars);
    format!(
        "{{\"text\":{},\"vars\":{}}}",
        esc(&render_pat(p)),
        arr(vars.iter().map(|v| esc(v)).collect())
    )
}

fn literal(l: &Literal) -> String {
    match l {
        Literal::Int(s) => format!("[\"int\",{}]", esc(s)),
        Literal::Float(s) => format!("[\"float\",{}]", esc(s)),
        Literal::Char(c) => format!("[\"char\",{}]", esc(&c.to_string())),
        Literal::Str(s) => format!("[\"str\",{}]", esc(s)),
    }
}

pub fn expr(e: &Expr) -> String {
    match e {
        Expr::Var(v) => format!("[\"var\",{}]", esc(v)),
        Expr::Con(c) => format!("[\"con\",{}]", esc(c)),
        Expr::Lit(l) => format!("[\"lit\",{}]", literal(l)),
        Expr::App(f, args) => format!(
            "[\"app\",{},{}]",
            expr(f),
            arr(args.iter().map(expr).collect())
        ),
        Expr::BinOp(op, l, r) => format!("[\"op\",{},{},{}]", esc(op), expr(l), expr(r)),
        Expr::Neg(x) => format!("[\"neg\",{}]", expr(x)),
        Expr::If(c, t, f) => format!("[\"if\",{},{},{}]", expr(c), expr(t), expr(f)),
        Expr::Case(s, alts) => format!(
            "[\"case\",{},{}]",
            expr(s),
            arr(alts.iter().map(alt).collect())
        ),
        Expr::Let(decls, body) => format!(
            "[\"let\",{},{}]",
            arr(decls.iter().map(decl).collect()),
            expr(body)
        ),
        Expr::Lambda(pats, body) => format!(
            "[\"lam\",{},{}]",
            arr(pats.iter().map(pat).collect()),
            expr(body)
        ),
        Expr::List(items) => format!("[\"list\",{}]", arr(items.iter().map(expr).collect())),
        Expr::Tuple(items) => format!("[\"tuple\",{}]", arr(items.iter().map(expr).collect())),
        Expr::Range { from, then, to } => format!(
            "[\"range\",{},{},{}]",
            expr(from),
            then.as_ref()
                .map(|t| expr(t))
                .unwrap_or_else(|| "null".into()),
            to.as_ref()
                .map(|t| expr(t))
                .unwrap_or_else(|| "null".into())
        ),
        Expr::Comp(body, quals) => format!(
            "[\"comp\",{},{}]",
            expr(body),
            arr(quals.iter().map(qual).collect())
        ),
        Expr::SectionL(op, x) => format!("[\"section_l\",{},{}]", esc(op), expr(x)),
        Expr::SectionR(op, x) => format!("[\"section_r\",{},{}]", esc(op), expr(x)),
        Expr::OpFun(op) => format!("[\"opfun\",{}]", esc(op)),
    }
}

fn qual(q: &Qual) -> String {
    match q {
        Qual::Gen(p, e) => format!("[\"gen\",{},{}]", pat(p), expr(e)),
        Qual::Guard(e) => format!("[\"guard\",{}]", expr(e)),
        Qual::Let(ds) => format!("[\"let\",{}]", arr(ds.iter().map(decl).collect())),
    }
}

fn rhs(r: &Rhs) -> String {
    match r {
        Rhs::Plain(e) => format!("{{\"body\":{}}}", expr(e)),
        Rhs::Guarded(gs) => format!(
            "{{\"guards\":{}}}",
            arr(gs
                .iter()
                .map(|(g, e)| format!("[{},{}]", expr(g), expr(e)))
                .collect())
        ),
    }
}

fn alt(a: &Alt) -> String {
    format!(
        "{{\"pat\":{},\"rhs\":{},\"where\":{},\"line\":{}}}",
        pat(&a.pat),
        rhs(&a.rhs),
        arr(a.wheres.iter().map(decl).collect()),
        a.line
    )
}

fn equation(e: &Equation) -> String {
    format!(
        "{{\"pats\":{},\"rhs\":{},\"where\":{},\"line\":{}}}",
        arr(e.pats.iter().map(pat).collect()),
        rhs(&e.rhs),
        arr(e.wheres.iter().map(decl).collect()),
        e.line
    )
}

pub fn decl(d: &Decl) -> String {
    match d {
        Decl::Data { decl, deriving, line } => format!(
            "{{\"kind\":\"data\",\"name\":{},\"tyvars\":{},\"cons\":{},\"deriving\":{},\"line\":{}}}",
            esc(&decl.name),
            arr(decl.tyvars.iter().map(|v| esc(v)).collect()),
            arr(decl
                .cons
                .iter()
                .map(|c| format!(
                    "{{\"name\":{},\"arity\":{},\"fields\":{}}}",
                    esc(&c.name),
                    c.arity,
                    c.fields.as_ref().map(|fs| arr(fs.iter().map(|f| esc(f)).collect())).unwrap_or_else(|| "null".into())
                ))
                .collect()),
            arr(deriving.iter().map(|v| esc(v)).collect()),
            line
        ),
        Decl::Sig { names, arity, line } => format!(
            "{{\"kind\":\"sig\",\"names\":{},\"arity\":{},\"line\":{}}}",
            arr(names.iter().map(|v| esc(v)).collect()),
            arity,
            line
        ),
        Decl::Fun(eqs) => format!(
            "{{\"kind\":\"fun\",\"name\":{},\"arity\":{},\"equations\":{},\"line\":{}}}",
            esc(&eqs[0].name),
            eqs[0].pats.len(),
            arr(eqs.iter().map(equation).collect()),
            eqs[0].line
        ),
        Decl::PatBind { pat: p, rhs: r, wheres, line } => format!(
            "{{\"kind\":\"bind\",\"pat\":{},\"rhs\":{},\"where\":{},\"line\":{}}}",
            pat(p),
            rhs(r),
            arr(wheres.iter().map(decl).collect()),
            line
        ),
    }
}

pub fn module(m: &Module) -> String {
    format!(
        "{{\"name\":{},\"decls\":{}}}",
        m.name
            .as_ref()
            .map(|n| esc(n))
            .unwrap_or_else(|| "null".into()),
        arr(m.decls.iter().map(decl).collect())
    )
}
