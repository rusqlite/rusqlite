//! Private implementation details of `rusqlite`.

use litrs::StringLit;
use proc_macro::{Delimiter, Group, Literal, Span, TokenStream, TokenTree};

use fallible_iterator::FallibleIterator;
use sqlite3_parser::ast::fmt::ToTokens;
use sqlite3_parser::ast::{Cmd, ParameterInfo};
use sqlite3_parser::lexer::sql::Parser;

// https://internals.rust-lang.org/t/custom-error-diagnostics-with-procedural-macros-on-almost-stable-rust/8113

#[doc(hidden)]
#[proc_macro]
pub fn __single(input: TokenStream) -> TokenStream {
    try_single(input).unwrap_or_else(|msg| parse_ts(&format!("compile_error!({msg:?})")))
}

fn try_single(input: TokenStream) -> Result<TokenStream> {
    let literal = {
        let mut iter = input.clone().into_iter();
        let literal = iter.next().unwrap();
        assert!(iter.next().is_none());
        literal
    };

    let literal = match into_literal(&literal) {
        Some(it) => it,
        None => return Err("expected a plain string literal".to_string()),
    };
    let string_lit = match StringLit::try_from(literal) {
        Ok(string_lit) => string_lit,
        Err(e) => return Ok(e.to_compile_error()),
    };
    parse(string_lit.value())?;
    // TODO DDL vs DML (without RETURNING) vs DQL
    Ok(input)
}

#[doc(hidden)]
#[proc_macro]
pub fn __bind(input: TokenStream) -> TokenStream {
    try_bind(input).unwrap_or_else(|msg| parse_ts(&format!("compile_error!({msg:?})")))
}

type Result<T> = std::result::Result<T, String>;

fn try_bind(input: TokenStream) -> Result<TokenStream> {
    let (stmt, literal) = {
        let mut iter = input.into_iter();
        let stmt = iter.next().unwrap();
        let literal = iter.next().unwrap();
        assert!(iter.next().is_none());
        (stmt, literal)
    };

    let Some(literal) = into_literal(&literal) else {
        return Err("expected a plain string literal".to_string());
    };
    let call_site = literal.span();
    let string_lit = match StringLit::try_from(literal) {
        Ok(string_lit) => string_lit,
        Err(e) => return Ok(e.to_compile_error()),
    };
    let ast = parse(string_lit.value())?;

    let mut info = ParameterInfo::default();
    if let Err(err) = ast.to_tokens(&mut info) {
        return Err(err.to_string());
    }
    if info.count == 0 {
        return Ok(TokenStream::new());
    }
    if info.count as usize != info.names.len() {
        return Err("Mixing named and numbered parameters is not supported.".to_string());
    }

    let mut res = TokenStream::new();
    for (i, name) in info.names.iter().enumerate() {
        res.extend(Some(stmt.clone()));
        res.extend(respan(
            parse_ts(&format!(
                ".raw_bind_parameter({}, &{})?;",
                i + 1,
                &name[1..]
            )),
            call_site,
        ));
    }

    Ok(res)
}

fn parse(sql: &str) -> Result<Cmd> {
    let mut parser = Parser::new(sql.as_bytes());
    let ast = match parser.next() {
        Ok(None) => return Err("Invalid input".to_owned()),
        Err(err) => return Err(err.to_string()),
        Ok(Some(ast)) => ast,
    };
    match parser.next() {
        Err(err) => return Err(err.to_string()),
        Ok(Some(_)) => return Err("Multiple statements".to_owned()),
        _ => {}
    };
    Ok(ast)
}

fn into_literal(ts: &TokenTree) -> Option<Literal> {
    match ts {
        TokenTree::Literal(l) => Some(l.clone()),
        TokenTree::Group(g) => match g.delimiter() {
            Delimiter::None => match g.stream().into_iter().collect::<Vec<_>>().as_slice() {
                [TokenTree::Literal(l)] => Some(l.clone()),
                _ => None,
            },
            Delimiter::Parenthesis | Delimiter::Brace | Delimiter::Bracket => None,
        },
        _ => None,
    }
}

fn respan(ts: TokenStream, span: Span) -> TokenStream {
    let mut res = TokenStream::new();
    for tt in ts {
        let tt = match tt {
            TokenTree::Ident(mut ident) => {
                ident.set_span(ident.span().resolved_at(span).located_at(span));
                TokenTree::Ident(ident)
            }
            TokenTree::Group(group) => {
                TokenTree::Group(Group::new(group.delimiter(), respan(group.stream(), span)))
            }
            _ => tt,
        };
        res.extend(Some(tt))
    }
    res
}

fn parse_ts(s: &str) -> TokenStream {
    s.parse().unwrap()
}
