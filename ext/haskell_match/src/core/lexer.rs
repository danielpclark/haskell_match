//! Tokeniser shared by the pattern parser and the `data` declaration parser.

use super::error::{CoreError, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// An infix constructor operator such as `:+:` (always starts with `:`).
    ConSym(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    DoubleColon,
    Equals,
    At,
    Tilde,
    Bang,
    Minus,
    Pipe,
    Arrow,
    DotDot,
    Underscore,
    /// Lower-case identifier (variable / field / type variable).
    VarId(String),
    /// Upper-case identifier (constructor / type), possibly qualified
    /// (`Data.Maybe.Just` is reported as `Just`; qualification is ignored).
    ConId(String),
    Int(String),
    Float(String),
    Str(String),
    Char(String),
    Sym(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub pos: usize,
}

pub fn tokenize(src: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let pos = i;
        let push = |out: &mut Vec<Token>, tok: Tok| out.push(Token { tok, pos });
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                i += 1;
            }
            '-' if i + 1 < chars.len() && chars[i + 1] == '-' => {
                // line comment
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '{' if i + 1 < chars.len() && chars[i + 1] == '-' => {
                // block comment (nested)
                let mut depth = 0;
                loop {
                    if i + 1 >= chars.len() {
                        return Err(CoreError::syntax(format!(
                            "unterminated block comment starting at column {}",
                            pos + 1
                        )));
                    }
                    if chars[i] == '{' && chars[i + 1] == '-' {
                        depth += 1;
                        i += 2;
                    } else if chars[i] == '-' && chars[i + 1] == '}' {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            '(' => {
                push(&mut out, Tok::LParen);
                i += 1;
            }
            ')' => {
                push(&mut out, Tok::RParen);
                i += 1;
            }
            '[' => {
                push(&mut out, Tok::LBracket);
                i += 1;
            }
            ']' => {
                push(&mut out, Tok::RBracket);
                i += 1;
            }
            '{' => {
                push(&mut out, Tok::LBrace);
                i += 1;
            }
            '}' => {
                push(&mut out, Tok::RBrace);
                i += 1;
            }
            ',' => {
                push(&mut out, Tok::Comma);
                i += 1;
            }
            '@' => {
                push(&mut out, Tok::At);
                i += 1;
            }
            '~' => {
                push(&mut out, Tok::Tilde);
                i += 1;
            }
            '!' => {
                push(&mut out, Tok::Bang);
                i += 1;
            }
            '|' => {
                push(&mut out, Tok::Pipe);
                i += 1;
            }
            '=' => {
                push(&mut out, Tok::Equals);
                i += 1;
            }
            '-' if i + 1 < chars.len() && chars[i + 1] == '>' => {
                push(&mut out, Tok::Arrow);
                i += 2;
            }
            '-' => {
                push(&mut out, Tok::Minus);
                i += 1;
            }
            '.' if i + 1 < chars.len() && chars[i + 1] == '.' => {
                push(&mut out, Tok::DotDot);
                i += 2;
            }
            ':' if i + 1 < chars.len() && chars[i + 1] == ':' => {
                push(&mut out, Tok::DoubleColon);
                i += 2;
            }
            ':' if i + 1 < chars.len()
                && (chars[i + 1].is_alphabetic() || chars[i + 1] == '_')
                && symbol_position(&chars, i) =>
            {
                // Ruby symbol literal extension: `:foo`, `:foo?`, `:foo!`, `:Foo`
                let start = i + 1;
                i += 1;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                if i < chars.len() && (chars[i] == '?' || chars[i] == '!' || chars[i] == '=') {
                    // `=` is part of a symbol only when not followed by another `=`
                    if chars[i] != '=' || i + 1 >= chars.len() || chars[i + 1] != '=' {
                        i += 1;
                    }
                }
                let name: String = chars[start..i].iter().collect();
                push(&mut out, Tok::Sym(name));
            }
            ':' if i + 1 < chars.len() && chars[i + 1] == '"' && symbol_position(&chars, i) => {
                // `:"quoted symbol"`
                let (s, next) = lex_string(&chars, i + 1, '"')?;
                push(&mut out, Tok::Sym(s));
                i = next;
            }
            ':' if i + 1 < chars.len() && is_consym_char(chars[i + 1]) => {
                // an infix constructor such as `:+:` or `:|`
                let start = i;
                i += 1;
                while i < chars.len() && is_consym_char(chars[i]) {
                    i += 1;
                }
                let sym: String = chars[start..i].iter().collect();
                push(&mut out, Tok::ConSym(sym));
            }
            ':' => {
                push(&mut out, Tok::Colon);
                i += 1;
            }
            '"' => {
                let (s, next) = lex_string(&chars, i, '"')?;
                push(&mut out, Tok::Str(s));
                i = next;
            }
            '\'' => {
                let (s, next) = lex_string(&chars, i, '\'')?;
                if s.chars().count() != 1 {
                    return Err(CoreError::syntax(format!(
                        "character literal at column {} must contain exactly one character",
                        pos + 1
                    )));
                }
                push(&mut out, Tok::Char(s));
                i = next;
            }
            '_' if !(i + 1 < chars.len() && is_ident_char(chars[i + 1])) => {
                push(&mut out, Tok::Underscore);
                i += 1;
            }
            c if c.is_ascii_digit() => {
                let (tok, next) = lex_number(&chars, i)?;
                push(&mut out, tok);
                i = next;
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (is_ident_char(chars[i]) || chars[i] == '\'') {
                    i += 1;
                }
                let mut name: String = chars[start..i].iter().collect();
                if c.is_uppercase() {
                    // qualified name: `Mod.Sub.Con` — consume dotted segments
                    while i + 1 < chars.len()
                        && chars[i] == '.'
                        && (chars[i + 1].is_alphabetic() || chars[i + 1] == '_')
                    {
                        i += 1;
                        let seg_start = i;
                        while i < chars.len() && (is_ident_char(chars[i]) || chars[i] == '\'') {
                            i += 1;
                        }
                        let seg: String = chars[seg_start..i].iter().collect();
                        if seg
                            .chars()
                            .next()
                            .map(|c| c.is_uppercase())
                            .unwrap_or(false)
                        {
                            name = seg;
                        } else {
                            return Err(CoreError::syntax(format!(
                                "qualified name at column {} must end in a constructor",
                                pos + 1
                            )));
                        }
                    }
                    push(&mut out, Tok::ConId(name));
                } else {
                    push(&mut out, Tok::VarId(name));
                }
            }
            other => {
                return Err(CoreError::syntax(format!(
                    "unexpected character {:?} at column {}",
                    other,
                    pos + 1
                )));
            }
        }
    }
    Ok(out)
}

/// A `:` starts a Ruby symbol literal (an extension) only when it is not
/// directly attached to a preceding operand: `(x:xs)` and `x : xs` are cons,
/// `:ok`, `[:a, :b]`, `Just :ok` and `(Pair :a :b)` are symbols.
/// Characters that may follow the leading `:` of an infix constructor.
fn is_consym_char(c: char) -> bool {
    "!#$%&*+./<=>?@\\^|-~:".contains(c)
}

fn symbol_position(chars: &[char], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    matches!(
        chars[i - 1],
        ' ' | '\t' | '\n' | '\r' | '(' | '[' | ',' | '=' | '|' | '{'
    )
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn lex_number(chars: &[char], start: usize) -> Result<(Tok, usize)> {
    let mut i = start;
    // hexadecimal / octal / binary integers
    if chars[i] == '0' && i + 1 < chars.len() {
        let radix = match chars[i + 1] {
            'x' | 'X' => Some(16),
            'o' | 'O' => Some(8),
            'b' | 'B' => Some(2),
            _ => None,
        };
        if let Some(radix) = radix {
            let mut j = i + 2;
            let digits_start = j;
            while j < chars.len() && (chars[j].is_digit(radix) || chars[j] == '_') {
                j += 1;
            }
            let digits: String = chars[digits_start..j]
                .iter()
                .filter(|c| **c != '_')
                .collect();
            if digits.is_empty() {
                return Err(CoreError::syntax(format!(
                    "malformed numeric literal at column {}",
                    start + 1
                )));
            }
            let value = u128::from_str_radix(&digits, radix).map_err(|_| {
                CoreError::syntax(format!(
                    "numeric literal at column {} is too large",
                    start + 1
                ))
            })?;
            return Ok((Tok::Int(value.to_string()), j));
        }
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
    if is_float {
        Ok((Tok::Float(text), i))
    } else {
        Ok((Tok::Int(text), i))
    }
}

/// Lex a quoted literal starting at `start` (which holds the opening quote).
/// Returns the unescaped contents and the index just past the closing quote.
fn lex_string(chars: &[char], start: usize, quote: char) -> Result<(String, usize)> {
    let mut i = start + 1;
    let mut out = String::new();
    loop {
        if i >= chars.len() {
            return Err(CoreError::syntax(format!(
                "unterminated {} literal starting at column {}",
                if quote == '"' { "string" } else { "character" },
                start + 1
            )));
        }
        let c = chars[i];
        if c == quote {
            return Ok((out, i + 1));
        }
        if c == '\\' {
            i += 1;
            if i >= chars.len() {
                return Err(CoreError::syntax("unterminated escape sequence"));
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
                '&' => {} // Haskell empty escape
                'x' | 'u' => {
                    // hex escape: \xHH.. or \uHHHH / \u{...}
                    let mut j = i + 1;
                    let braces = j < chars.len() && chars[j] == '{';
                    if braces {
                        j += 1;
                    }
                    let hs = j;
                    while j < chars.len() && chars[j].is_ascii_hexdigit() {
                        j += 1;
                    }
                    let hex: String = chars[hs..j].iter().collect();
                    if braces {
                        if j < chars.len() && chars[j] == '}' {
                            j += 1;
                        } else {
                            return Err(CoreError::syntax("unterminated \\u{...} escape"));
                        }
                    }
                    let cp = u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| CoreError::syntax("invalid hexadecimal escape"))?;
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
                        .ok_or_else(|| CoreError::syntax("invalid decimal escape"))?;
                    out.push(cp);
                    i = j;
                    continue;
                }
                other => {
                    return Err(CoreError::syntax(format!(
                        "unknown escape sequence \\{}",
                        other
                    )))
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
    fn punctuation_and_identifiers() {
        assert_eq!(
            toks("(x:xs) _ Just a@(y, _) [] ~p !q"),
            vec![
                Tok::LParen,
                Tok::VarId("x".into()),
                Tok::Colon,
                Tok::VarId("xs".into()),
                Tok::RParen,
                Tok::Underscore,
                Tok::ConId("Just".into()),
                Tok::VarId("a".into()),
                Tok::At,
                Tok::LParen,
                Tok::VarId("y".into()),
                Tok::Comma,
                Tok::Underscore,
                Tok::RParen,
                Tok::LBracket,
                Tok::RBracket,
                Tok::Tilde,
                Tok::VarId("p".into()),
                Tok::Bang,
                Tok::VarId("q".into()),
            ]
        );
    }

    #[test]
    fn literals() {
        assert_eq!(
            toks(r#"0 42 1.5 2e3 0x1F "hi\n" 'c' :sym :"quoted sym" -7 _x x'"#),
            vec![
                Tok::Int("0".into()),
                Tok::Int("42".into()),
                Tok::Float("1.5".into()),
                Tok::Float("2e3".into()),
                Tok::Int("31".into()),
                Tok::Str("hi\n".into()),
                Tok::Char("c".into()),
                Tok::Sym("sym".into()),
                Tok::Sym("quoted sym".into()),
                Tok::Minus,
                Tok::Int("7".into()),
                Tok::VarId("_x".into()),
                Tok::VarId("x'".into()),
            ]
        );
    }

    #[test]
    fn data_decl_tokens() {
        assert_eq!(
            toks("data Maybe a = Nothing | Just a -- comment\n{- block {- nested -} -} deriving (Show)"),
            vec![
                Tok::VarId("data".into()),
                Tok::ConId("Maybe".into()),
                Tok::VarId("a".into()),
                Tok::Equals,
                Tok::ConId("Nothing".into()),
                Tok::Pipe,
                Tok::ConId("Just".into()),
                Tok::VarId("a".into()),
                Tok::VarId("deriving".into()),
                Tok::LParen,
                Tok::ConId("Show".into()),
                Tok::RParen,
            ]
        );
        assert_eq!(
            toks("P { name :: String, age :: Int } -> ..")[..],
            [
                Tok::ConId("P".into()),
                Tok::LBrace,
                Tok::VarId("name".into()),
                Tok::DoubleColon,
                Tok::ConId("String".into()),
                Tok::Comma,
                Tok::VarId("age".into()),
                Tok::DoubleColon,
                Tok::ConId("Int".into()),
                Tok::RBrace,
                Tok::Arrow,
                Tok::DotDot,
            ]
        );
    }

    #[test]
    fn colon_versus_symbol() {
        assert_eq!(
            toks("x:xs"),
            vec![Tok::VarId("x".into()), Tok::Colon, Tok::VarId("xs".into())]
        );
        assert_eq!(
            toks("x : xs"),
            vec![Tok::VarId("x".into()), Tok::Colon, Tok::VarId("xs".into())]
        );
        assert_eq!(
            toks("Just :ok"),
            vec![Tok::ConId("Just".into()), Tok::Sym("ok".into())]
        );
        assert_eq!(
            toks("[:a,:b]"),
            vec![
                Tok::LBracket,
                Tok::Sym("a".into()),
                Tok::Comma,
                Tok::Sym("b".into()),
                Tok::RBracket
            ]
        );
        assert_eq!(
            toks("x:_"),
            vec![Tok::VarId("x".into()), Tok::Colon, Tok::Underscore]
        );
    }

    #[test]
    fn qualified_constructor() {
        assert_eq!(
            toks("Data.Maybe.Just x"),
            vec![Tok::ConId("Just".into()), Tok::VarId("x".into())]
        );
    }

    #[test]
    fn errors() {
        assert!(tokenize("\"abc").is_err());
        assert!(tokenize("'ab'").is_err());
        assert!(tokenize("x $ y").is_err());
        assert!(tokenize("{- never closed").is_err());
        assert!(tokenize("\"\\q\"").is_err());
        assert!(tokenize("0x").is_err());
    }
}
