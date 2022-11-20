//! Private implementation details of `rusqlite`.

use proc_macro::{Delimiter, Literal, TokenStream, TokenTree};

use fallible_iterator::FallibleIterator;
use sqlite3_parser::lexer::sql::Parser;
use sqlite3_parser::ast::{ParameterInfo, ToTokens};

// https://internals.rust-lang.org/t/custom-error-diagnostics-with-procedural-macros-on-almost-stable-rust/8113

#[doc(hidden)]
#[proc_macro]
pub fn __bind(input: TokenStream) -> TokenStream {
    try_bind(input).unwrap_or_else(|msg| parse_ts(&format!("compile_error!({:?})", msg)))
}

type Result<T> = std::result::Result<T, String>;

fn try_bind(input: TokenStream) -> Result<TokenStream> {
    //eprintln!("INPUT: {:#?}", input);
    let (stmt, literal) = {
        let mut iter = input.into_iter();
        let stmt = iter.next().unwrap();
        let _punct = iter.next().unwrap();
        let literal = iter.next().unwrap();
        assert!(iter.next().is_none());
        (stmt, literal)
    };

    let literal = match into_literal(&literal) {
        Some(it) => it,
        None => return Err("expected a plain string literal".to_string()),
    };
    let sql = literal.to_string();
    if !sql.starts_with('"') {
        return Err("expected a plain string literal".to_string());
    }
    let sql = strip_matches(&sql, "\"");
    //eprintln!("SQL: {}", sql);

    let mut parser = Parser::new(sql.as_bytes());
    let ast = match parser.next() {
        Ok(None) => return Err("Invalid input".to_owned()),
        Err(err) => {
            return Err(err.to_string());
        }
        Ok(Some(ast)) => ast
    };
    let mut info = ParameterInfo::default();
    if let Err(err) = ast.to_tokens(&mut info) {
        return Err(err.to_string());
    }
    //eprintln!("ParameterInfo.count: {:#?}", info.count);
    //eprintln!("ParameterInfo.names: {:#?}", info.names);

    let mut res = TokenStream::new();
    Ok(res)
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

fn strip_matches<'a>(s: &'a str, pattern: &str) -> &'a str {
    s.strip_prefix(pattern).unwrap_or(s).strip_suffix(pattern).unwrap_or(s)
}

fn parse_ts(s: &str) -> TokenStream {
    s.parse().unwrap()
}
