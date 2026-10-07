//! The Haskell 2010 layout algorithm (report §10.3), simplified: implicit
//! blocks after `where`, `let`, `do` and `of`, closed by indentation, by
//! `in`, or when a closing bracket of an enclosing explicit construct is met.

use super::lexer::{Tok, Token};

enum Ctx {
    Explicit,
    /// column of the block, whether `let` opened it (so `in` may close it),
    /// and whether the current item of a let block has already seen its `=`
    Implicit(usize, bool, bool),
}

fn opens_block(t: &Tok) -> bool {
    matches!(t, Tok::Where | Tok::Let | Tok::Do | Tok::Of)
}

pub fn layout(tokens: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len() * 2);
    let mut ctx: Vec<Ctx> = Vec::new();
    let mut i = 0;
    let n = tokens.len();
    let mut pending_open: Option<bool> = None; // Some(is_let) after a layout keyword
    let mut last_line = 0usize;
    // the whole module is an implicit block unless it starts with `module` or `{`
    if !matches!(
        tokens.first().map(|t| &t.tok),
        Some(Tok::Module) | Some(Tok::LBrace)
    ) {
        if let Some(first) = tokens.first() {
            if first.tok != Tok::Eof {
                ctx.push(Ctx::Implicit(first.col, false, false));
                out.push(Token {
                    tok: Tok::VLBrace,
                    line: first.line,
                    col: first.col,
                });
                last_line = first.line;
            }
        }
    }
    // explicit-bracket depth tracking so a `)` / `]` / `,` can close implicit blocks
    let mut bracket_stack: Vec<usize> = Vec::new(); // ctx depth at each open bracket
    while i < n {
        let t = &tokens[i];
        if t.tok == Tok::Eof {
            // close everything
            while let Some(c) = ctx.pop() {
                if let Ctx::Implicit(..) = c {
                    out.push(Token {
                        tok: Tok::VRBrace,
                        line: t.line,
                        col: t.col,
                    });
                }
            }
            out.push(t.clone());
            break;
        }
        if let Some(is_let) = pending_open.take() {
            if t.tok == Tok::LBrace {
                ctx.push(Ctx::Explicit);
                out.push(t.clone());
                last_line = t.line;
                i += 1;
                continue;
            }
            // open an implicit block at this token's column
            let enclosing = ctx.iter().rev().find_map(|c| {
                if let Ctx::Implicit(m, _, _) = c {
                    Some(*m)
                } else {
                    None
                }
            });
            if enclosing.map(|m| t.col > m).unwrap_or(true) {
                ctx.push(Ctx::Implicit(t.col, is_let, false));
                out.push(Token {
                    tok: Tok::VLBrace,
                    line: t.line,
                    col: t.col,
                });
            } else {
                // empty block: `{}` then fall through to normal processing
                out.push(Token {
                    tok: Tok::VLBrace,
                    line: t.line,
                    col: t.col,
                });
                out.push(Token {
                    tok: Tok::VRBrace,
                    line: t.line,
                    col: t.col,
                });
            }
            last_line = t.line;
            // the token itself is processed below without the new-line rule
            emit(&mut out, &mut ctx, &mut bracket_stack, t, &mut pending_open);
            i += 1;
            continue;
        }
        // new line: compare indentation with the innermost implicit context
        if t.line > last_line {
            loop {
                match ctx.last() {
                    Some(Ctx::Implicit(m, _, _)) if t.col < *m => {
                        ctx.pop();
                        out.push(Token {
                            tok: Tok::VRBrace,
                            line: t.line,
                            col: t.col,
                        });
                    }
                    Some(Ctx::Implicit(m, _, _)) if t.col == *m => {
                        // a new item in the block, unless the token continues an expression
                        if !matches!(t.tok, Tok::Then | Tok::Else | Tok::Of | Tok::In) {
                            out.push(Token {
                                tok: Tok::VSemi,
                                line: t.line,
                                col: t.col,
                            });
                            if let Some(Ctx::Implicit(_, _, eq)) = ctx.last_mut() {
                                *eq = false;
                            }
                        }
                        break;
                    }
                    _ => break,
                }
            }
            last_line = t.line;
        }
        // `\case` (LambdaCase) opens an alternatives block like `of`
        let lambda_case =
            t.tok == Tok::Case && out.last().map(|x| x.tok == Tok::Backslash).unwrap_or(false);
        emit(&mut out, &mut ctx, &mut bracket_stack, t, &mut pending_open);
        if lambda_case {
            pending_open = Some(false);
        }
        i += 1;
    }
    out
}

fn emit(
    out: &mut Vec<Token>,
    ctx: &mut Vec<Ctx>,
    brackets: &mut Vec<usize>,
    t: &Token,
    pending_open: &mut Option<bool>,
) {
    match &t.tok {
        Tok::Equals => {
            // Inside a `let` block each binding has one `=`; a second one
            // before the next binding means the block was a guard qualifier
            // (`| let y = f x = y`) and the block ends here (the report's
            // parse-error(t) rule, for this one case).
            let inside_block = brackets.last().map(|d| ctx.len() > *d).unwrap_or(true);
            if let Some(Ctx::Implicit(_, true, eq_seen)) = ctx.last_mut() {
                if *eq_seen && inside_block {
                    ctx.pop();
                    out.push(Token {
                        tok: Tok::VRBrace,
                        line: t.line,
                        col: t.col,
                    });
                } else {
                    *eq_seen = true;
                }
            }
            out.push(t.clone());
        }
        Tok::Semi => {
            if let Some(Ctx::Implicit(_, _, eq)) = ctx.last_mut() {
                *eq = false;
            }
            out.push(t.clone());
        }
        Tok::In => {
            // `let ... in`: close the implicit let block if it is still open
            // (the indentation rule may already have closed it)
            if let Some(Ctx::Implicit(_, true, _)) = ctx.last() {
                if brackets.last().map(|d| ctx.len() > *d).unwrap_or(true) {
                    ctx.pop();
                    out.push(Token {
                        tok: Tok::VRBrace,
                        line: t.line,
                        col: t.col,
                    });
                }
            }
            out.push(t.clone());
        }
        Tok::LParen | Tok::LBracket => {
            brackets.push(ctx.len());
            out.push(t.clone());
        }
        Tok::RParen | Tok::RBracket => {
            if let Some(depth) = brackets.pop() {
                while ctx.len() > depth {
                    if let Some(Ctx::Implicit(..)) = ctx.pop() {
                        out.push(Token {
                            tok: Tok::VRBrace,
                            line: t.line,
                            col: t.col,
                        });
                    }
                }
            }
            out.push(t.clone());
        }
        Tok::Comma => {
            // a comma belonging to an enclosing bracket closes implicit blocks opened inside it
            if let Some(depth) = brackets.last() {
                while ctx.len() > *depth {
                    if let Some(Ctx::Implicit(..)) = ctx.pop() {
                        out.push(Token {
                            tok: Tok::VRBrace,
                            line: t.line,
                            col: t.col,
                        });
                    }
                }
            } else if let Some(Ctx::Implicit(_, true, _)) = ctx.last() {
                // `| let y = e, cond`: a comma ends a let block used as a guard
                ctx.pop();
                out.push(Token {
                    tok: Tok::VRBrace,
                    line: t.line,
                    col: t.col,
                });
            }
            out.push(t.clone());
        }
        Tok::LBrace => {
            ctx.push(Ctx::Explicit);
            out.push(t.clone());
        }
        Tok::RBrace => {
            // close implicit blocks opened inside the explicit one
            while let Some(Ctx::Implicit(..)) = ctx.last() {
                ctx.pop();
                out.push(Token {
                    tok: Tok::VRBrace,
                    line: t.line,
                    col: t.col,
                });
            }
            if let Some(Ctx::Explicit) = ctx.last() {
                ctx.pop();
            }
            out.push(t.clone());
        }
        Tok::Then | Tok::Else | Tok::Of => {
            // `case e of` inside a `let`/`where` item on the same line is fine;
            // `then`/`else` never close blocks here (handled by indentation)
            out.push(t.clone());
            if t.tok == Tok::Of {
                *pending_open = Some(false);
            }
        }
        tok => {
            out.push(t.clone());
            if opens_block(tok) {
                *pending_open = Some(matches!(tok, Tok::Let));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::hs::lexer::tokenize;

    fn show(src: &str) -> String {
        layout(tokenize(src).unwrap())
            .into_iter()
            .map(|t| match t.tok {
                Tok::VLBrace => "{".to_string(),
                Tok::VRBrace => "}".to_string(),
                Tok::VSemi => ";".to_string(),
                Tok::Eof => "".to_string(),
                Tok::VarId(v) => v,
                Tok::ConId(c) => c,
                Tok::VarSym(s) | Tok::ConSym(s) => s,
                Tok::Int(s) => s,
                Tok::Equals => "=".into(),
                Tok::Where => "where".into(),
                Tok::Let => "let".into(),
                Tok::In => "in".into(),
                Tok::Case => "case".into(),
                Tok::Of => "of".into(),
                Tok::RArrow => "->".into(),
                Tok::LParen => "(".into(),
                Tok::RParen => ")".into(),
                Tok::Pipe => "|".into(),
                other => format!("{:?}", other),
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn top_level_items_and_where() {
        let src = "f x = y\n  where y = x\n        z = 1\ng = 2\n";
        assert_eq!(show(src), "{ f x = y where { y = x ; z = 1 } ; g = 2 }");
    }

    #[test]
    fn let_in_and_case() {
        assert_eq!(show("f = let a = 1 in a"), "{ f = let { a = 1 } in a }");
        assert_eq!(
            show("f x = case x of\n  0 -> 1\n  n -> n\ng = 3"),
            "{ f x = case x of { 0 -> 1 ; n -> n } ; g = 3 }"
        );
        assert_eq!(
            show("f = (let a = 1 in a) + 1"),
            "{ f = ( let { a = 1 } in a ) + 1 }"
        );
    }

    #[test]
    fn multi_line_let_with_in_on_its_own_line() {
        let src = "f x = let y = x\n          z = y\n      in y + z\ng = 1\n";
        assert_eq!(
            show(src),
            "{ f x = let { y = x ; z = y } in y + z ; g = 1 }"
        );
    }

    #[test]
    fn let_guards_close_at_comma_or_second_equals() {
        assert_eq!(
            show("f x\n  | let y = x, y > 1 = y\n  | let z = x = z\n"),
            "{ f x | let { y = x } Comma y > 1 = y | let { z = x } = z }"
        );
    }

    #[test]
    fn guards_continue_an_equation() {
        let src = "sign n\n  | n > 0 = 1\n  | otherwise = 0\n";
        assert_eq!(show(src), "{ sign n | n > 0 = 1 | otherwise = 0 }");
    }
}
