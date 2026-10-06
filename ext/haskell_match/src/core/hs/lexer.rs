//! Haskell lexer with line/column positions (needed for the layout rule).

use crate::core::error::{CoreError, ErrorKind, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    // identifiers
    VarId(String),
    ConId(String),
    /// Symbolic operator such as `+`, `++`, `>>=`, or a backticked identifier
    /// (`` `div` `` becomes `VarSym("div")` with `backtick = true`).
    VarSym(String),
    ConSym(String), // `:`, `:+:` ...
    // literals
    Int(String),
    Float(String),
    Char(char),
    Str(String),
    // reserved words
    Case,
    Class,
    Data,
    Deriving,
    Do,
    Else,
    If,
    Import,
    In,
    Infix,
    Infixl,
    Infixr,
    Instance,
    Let,
    Module,
    Newtype,
    Of,
    Then,
    Type,
    Where,
    // reserved operators / punctuation
    DotDot,
    DoubleColon,
    Equals,
    Backslash,
    Pipe,
    LArrow,
    RArrow,
    At,
    Tilde,
    DArrow,
    Bang,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Semi,
    Underscore,
    Backtick,
    // layout-inserted
    VLBrace,
    VRBrace,
    VSemi,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    pub col: usize,
}

pub fn lex_error(line: usize, col: usize, msg: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorKind::Syntax, format!("{}:{}: {}", line, col, msg))
}

fn is_symbol_char(c: char) -> bool {
    "!#$%&*+./<=>?@\\^|-~:".contains(c)
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

pub fn tokenize(src: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1usize;
    let mut col = 1usize;
    let advance = |i: &mut usize, line: &mut usize, col: &mut usize, chars: &[char]| {
        if chars[*i] == '\n' {
            *line += 1;
            *col = 1;
        } else if chars[*i] == '\t' {
            *col += 8 - ((*col - 1) % 8);
        } else {
            *col += 1;
        }
        *i += 1;
    };
    while i < chars.len() {
        let c = chars[i];
        let (tl, tc) = (line, col);
        // whitespace
        if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
            advance(&mut i, &mut line, &mut col, &chars);
            continue;
        }
        // line comment: `--` followed by non-symbol
        if c == '-' && i + 1 < chars.len() && chars[i + 1] == '-' {
            let mut j = i;
            while j < chars.len() && chars[j] == '-' {
                j += 1;
            }
            if j >= chars.len() || !is_symbol_char(chars[j]) {
                while i < chars.len() && chars[i] != '\n' {
                    advance(&mut i, &mut line, &mut col, &chars);
                }
                continue;
            }
        }
        // block comment (nested)
        if c == '{' && i + 1 < chars.len() && chars[i + 1] == '-' {
            let mut depth = 0;
            loop {
                if i + 1 >= chars.len() {
                    return Err(lex_error(tl, tc, "unterminated block comment"));
                }
                if chars[i] == '{' && chars[i + 1] == '-' {
                    depth += 1;
                    advance(&mut i, &mut line, &mut col, &chars);
                    advance(&mut i, &mut line, &mut col, &chars);
                } else if chars[i] == '-' && chars[i + 1] == '}' {
                    depth -= 1;
                    advance(&mut i, &mut line, &mut col, &chars);
                    advance(&mut i, &mut line, &mut col, &chars);
                    if depth == 0 {
                        break;
                    }
                } else {
                    advance(&mut i, &mut line, &mut col, &chars);
                }
            }
            continue;
        }
        let mut push = |tok: Tok| {
            out.push(Token {
                tok,
                line: tl,
                col: tc,
            })
        };
        match c {
            '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '`' => {
                push(match c {
                    '(' => Tok::LParen,
                    ')' => Tok::RParen,
                    '[' => Tok::LBracket,
                    ']' => Tok::RBracket,
                    '{' => Tok::LBrace,
                    '}' => Tok::RBrace,
                    ',' => Tok::Comma,
                    ';' => Tok::Semi,
                    _ => Tok::Backtick,
                });
                advance(&mut i, &mut line, &mut col, &chars);
            }
            '"' => {
                let (s, next) = lex_string(&chars, i, '"', tl, tc)?;
                push(Tok::Str(s));
                while i < next {
                    advance(&mut i, &mut line, &mut col, &chars);
                }
            }
            '\'' if is_char_literal_start(&chars, i) => {
                let (s, next) = lex_string(&chars, i, '\'', tl, tc)?;
                let mut it = s.chars();
                let ch = it.next();
                if ch.is_none() || it.next().is_some() {
                    return Err(lex_error(
                        tl,
                        tc,
                        "character literal must contain exactly one character",
                    ));
                }
                push(Tok::Char(ch.unwrap()));
                while i < next {
                    advance(&mut i, &mut line, &mut col, &chars);
                }
            }
            c if c.is_ascii_digit() => {
                let (tok, next) = lex_number(&chars, i, tl, tc)?;
                push(tok);
                while i < next {
                    advance(&mut i, &mut line, &mut col, &chars);
                }
            }
            '_' if !(i + 1 < chars.len() && is_ident_char(chars[i + 1])) => {
                push(Tok::Underscore);
                advance(&mut i, &mut line, &mut col, &chars);
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                let mut j = i;
                while j < chars.len() && is_ident_char(chars[j]) {
                    j += 1;
                }
                let mut name: String = chars[start..j].iter().collect();
                let upper = c.is_uppercase();
                // qualified names: Mod.Sub.name / Mod.Con / Mod.+
                if upper {
                    while j + 1 < chars.len() && chars[j] == '.' {
                        let n = chars[j + 1];
                        if n.is_alphabetic() || n == '_' {
                            let seg_start = j + 1;
                            let mut k = seg_start;
                            while k < chars.len() && is_ident_char(chars[k]) {
                                k += 1;
                            }
                            let seg: String = chars[seg_start..k].iter().collect();
                            if seg.starts_with(|ch: char| ch.is_uppercase()) {
                                // Mod.Con keeps its qualifier (module headers need it);
                                // the parser drops it where a constructor is meant
                                name.push('.');
                                name.push_str(&seg);
                            } else {
                                name = seg;
                            }
                            j = k;
                        } else if is_symbol_char(n) {
                            let seg_start = j + 1;
                            let mut k = seg_start;
                            while k < chars.len() && is_symbol_char(chars[k]) {
                                k += 1;
                            }
                            let sym: String = chars[seg_start..k].iter().collect();
                            push(if sym.starts_with(':') {
                                Tok::ConSym(sym)
                            } else {
                                Tok::VarSym(sym)
                            });
                            while i < k {
                                advance(&mut i, &mut line, &mut col, &chars);
                            }
                            name.clear();
                            break;
                        } else {
                            break;
                        }
                    }
                    if name.is_empty() {
                        continue;
                    }
                }
                let tok = match name.as_str() {
                    "case" => Tok::Case,
                    "class" => Tok::Class,
                    "data" => Tok::Data,
                    "deriving" => Tok::Deriving,
                    "do" => Tok::Do,
                    "else" => Tok::Else,
                    "if" => Tok::If,
                    "import" => Tok::Import,
                    "in" => Tok::In,
                    "infix" => Tok::Infix,
                    "infixl" => Tok::Infixl,
                    "infixr" => Tok::Infixr,
                    "instance" => Tok::Instance,
                    "let" => Tok::Let,
                    "module" => Tok::Module,
                    "newtype" => Tok::Newtype,
                    "of" => Tok::Of,
                    "then" => Tok::Then,
                    "type" => Tok::Type,
                    "where" => Tok::Where,
                    _ => {
                        if name
                            .chars()
                            .next()
                            .map(|c| c.is_uppercase())
                            .unwrap_or(false)
                        {
                            Tok::ConId(name)
                        } else {
                            Tok::VarId(name)
                        }
                    }
                };
                push(tok);
                while i < j {
                    advance(&mut i, &mut line, &mut col, &chars);
                }
            }
            c if is_symbol_char(c) => {
                let start = i;
                let mut j = i;
                while j < chars.len() && is_symbol_char(chars[j]) {
                    j += 1;
                }
                let sym: String = chars[start..j].iter().collect();
                let tok = match sym.as_str() {
                    ".." => Tok::DotDot,
                    "::" => Tok::DoubleColon,
                    "=" => Tok::Equals,
                    "\\" => Tok::Backslash,
                    "|" => Tok::Pipe,
                    "<-" => Tok::LArrow,
                    "->" => Tok::RArrow,
                    "@" => Tok::At,
                    "~" => Tok::Tilde,
                    "=>" => Tok::DArrow,
                    "!" => Tok::Bang,
                    _ => {
                        if sym.starts_with(':') {
                            Tok::ConSym(sym)
                        } else {
                            Tok::VarSym(sym)
                        }
                    }
                };
                push(tok);
                while i < j {
                    advance(&mut i, &mut line, &mut col, &chars);
                }
            }
            other => {
                return Err(lex_error(
                    tl,
                    tc,
                    format!("unexpected character {:?}", other),
                ))
            }
        }
    }
    out.push(Token {
        tok: Tok::Eof,
        line,
        col: 0,
    });
    Ok(out)
}

/// A `'` starts a char literal unless it is part of an identifier (`x'`);
/// identifiers are lexed before we get here, so any `'` seen is a literal.
fn is_char_literal_start(chars: &[char], i: usize) -> bool {
    // 'x' or '\n' etc.
    if i + 2 < chars.len() && chars[i + 1] != '\\' && chars[i + 2] == '\'' {
        return true;
    }
    if i + 1 < chars.len() && chars[i + 1] == '\\' {
        return true;
    }
    false
}

fn lex_number(chars: &[char], start: usize, line: usize, col: usize) -> Result<(Tok, usize)> {
    let mut i = start;
    if chars[i] == '0'
        && i + 1 < chars.len()
        && matches!(chars[i + 1], 'x' | 'X' | 'o' | 'O' | 'b' | 'B')
    {
        let radix = match chars[i + 1] {
            'x' | 'X' => 16,
            'o' | 'O' => 8,
            _ => 2,
        };
        let mut j = i + 2;
        let ds = j;
        while j < chars.len() && (chars[j].is_digit(radix) || chars[j] == '_') {
            j += 1;
        }
        let digits: String = chars[ds..j].iter().filter(|c| **c != '_').collect();
        if digits.is_empty() {
            return Err(lex_error(line, col, "malformed numeric literal"));
        }
        let v = u128::from_str_radix(&digits, radix)
            .map_err(|_| lex_error(line, col, "numeric literal too large"))?;
        return Ok((Tok::Int(v.to_string()), j));
    }
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
        i += 1;
    }
    let mut is_float = false;
    if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
        is_float = true;
        i += 1;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
            i += 1;
        }
    }
    if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
        let mut j = i + 1;
        if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
            j += 1;
        }
        if j < chars.len() && chars[j].is_ascii_digit() {
            is_float = true;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    let text: String = chars[start..i].iter().filter(|c| **c != '_').collect();
    Ok((
        if is_float {
            Tok::Float(text)
        } else {
            Tok::Int(text)
        },
        i,
    ))
}

fn lex_string(
    chars: &[char],
    start: usize,
    quote: char,
    line: usize,
    col: usize,
) -> Result<(String, usize)> {
    let mut i = start + 1;
    let mut out = String::new();
    loop {
        if i >= chars.len() || chars[i] == '\n' {
            return Err(lex_error(
                line,
                col,
                "unterminated string or character literal",
            ));
        }
        let c = chars[i];
        if c == quote {
            return Ok((out, i + 1));
        }
        if c == '\\' {
            i += 1;
            if i >= chars.len() {
                return Err(lex_error(line, col, "unterminated escape sequence"));
            }
            let e = chars[i];
            match e {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0' => out.push('\0'),
                'a' => out.push('\u{7}'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'v' => out.push('\u{b}'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                '\'' => out.push('\''),
                '&' => {}
                'x' => {
                    let mut j = i + 1;
                    let hs = j;
                    while j < chars.len() && chars[j].is_ascii_hexdigit() {
                        j += 1;
                    }
                    let hex: String = chars[hs..j].iter().collect();
                    let cp = u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| lex_error(line, col, "invalid hexadecimal escape"))?;
                    out.push(cp);
                    i = j;
                    continue;
                }
                d if d.is_ascii_digit() => {
                    let mut j = i;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                    let dec: String = chars[i..j].iter().collect();
                    let cp = dec
                        .parse::<u32>()
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| lex_error(line, col, "invalid decimal escape"))?;
                    out.push(cp);
                    i = j;
                    continue;
                }
                other => {
                    return Err(lex_error(
                        line,
                        col,
                        format!("unknown escape sequence \\{}", other),
                    ))
                }
            }
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        tokenize(s).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn lexes_declarations() {
        assert_eq!(
            toks("f (x:xs) = x + sum xs `div` 2 -- c\n"),
            vec![
                Tok::VarId("f".into()),
                Tok::LParen,
                Tok::VarId("x".into()),
                Tok::ConSym(":".into()),
                Tok::VarId("xs".into()),
                Tok::RParen,
                Tok::Equals,
                Tok::VarId("x".into()),
                Tok::VarSym("+".into()),
                Tok::VarId("sum".into()),
                Tok::VarId("xs".into()),
                Tok::Backtick,
                Tok::VarId("div".into()),
                Tok::Backtick,
                Tok::Int("2".into()),
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn positions_and_literals() {
        let ts = tokenize("x = 'a'\n  y = \"s\\n\" 1.5").unwrap();
        assert_eq!((ts[0].line, ts[0].col), (1, 1));
        assert_eq!(ts[2].tok, Tok::Char('a'));
        assert_eq!((ts[3].line, ts[3].col), (2, 3));
        assert_eq!(ts[5].tok, Tok::Str("s\n".into()));
        assert_eq!(ts[6].tok, Tok::Float("1.5".into()));
        assert_eq!(
            toks("x' y_1 Just Data.Map.lookup ->  <- :: .. => \\ | @ ~ !")[..],
            [
                Tok::VarId("x'".into()),
                Tok::VarId("y_1".into()),
                Tok::ConId("Just".into()),
                Tok::VarId("lookup".into()),
                Tok::RArrow,
                Tok::LArrow,
                Tok::DoubleColon,
                Tok::DotDot,
                Tok::DArrow,
                Tok::Backslash,
                Tok::Pipe,
                Tok::At,
                Tok::Tilde,
                Tok::Bang,
                Tok::Eof
            ]
        );
        assert_eq!(toks("a --> b")[1], Tok::VarSym("-->".into())); // not a comment
        assert_eq!(toks("{- {- nested -} -} z")[0], Tok::VarId("z".into()));
    }

    #[test]
    fn errors() {
        assert!(tokenize("\"abc").is_err());
        assert!(tokenize("'ab'").is_err());
        assert!(tokenize("{- open").is_err());
        assert!(tokenize("§").is_err());
    }
}
