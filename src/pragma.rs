//! Pragma helpers

use {Connection, DatabaseName, Result, Row};
use error::Error;
use ffi;
use types::{ToSql, ToSqlOutput, ValueRef};

impl Connection {
    pub fn pragma_query<F>(&self,
                           schema_name: Option<DatabaseName>,
                           pragma_name: &str,
                           mut f: F)
                           -> Result<()>
        where F: FnMut(&Row) -> Result<()>
    {
        if pragma_name.is_empty() || !is_identifier(pragma_name) {
            return Err(Error::SqliteFailure(ffi::Error::new(ffi::SQLITE_MISUSE),
                                            Some(format!("Invalid pragma \"{}\"", pragma_name))));
        }
        let mut query = String::new();
        query.push_str("PRAGMA ");
        if let Some(schema_name) = schema_name {
            double_quote(&mut query, schema_name);
            query.push('.');
        }
        query.push_str(pragma_name);
        let mut stmt = try!(self.prepare(&query));
        let mut rows = try!(stmt.query(&[]));
        while let Some(result_row) = rows.next() {
            let row = try!(result_row);
            try!(f(&row));
        }
        Ok(())
    }

    pub fn pragma_update<F>(&self,
                            schema_name: Option<DatabaseName>,
                            pragma_name: &str,
                            pragma_value: &ToSql,
                            f: Option<F>)
                            -> Result<()>
        where F: FnMut(&Row) -> Result<()>
    {
        if pragma_name.is_empty() || !is_identifier(pragma_name) {
            return Err(Error::SqliteFailure(ffi::Error::new(ffi::SQLITE_MISUSE),
                                            Some(format!("Invalid pragma \"{}\"", pragma_name))));
        }
        let mut sql = String::new();
        sql.push_str("PRAGMA ");
        if let Some(schema_name) = schema_name {
            double_quote(&mut sql, schema_name);
            sql.push('.');
        }
        sql.push_str(pragma_name);
        // The argument is may be either in parentheses
        // or it may be separated from the pragma name by an equal sign.
        // The two syntaxes yield identical results.
        sql.push('(');
        let pragma_value = try!(pragma_value.to_sql());
        let pragma_value = match pragma_value {
            ToSqlOutput::Borrowed(v) => v,
            ToSqlOutput::Owned(ref v) => ValueRef::from(v),
            #[cfg(feature = "blob")]
            ToSqlOutput::ZeroBlob(_) => {
                return Err(Error::SqliteFailure(ffi::Error::new(ffi::SQLITE_MISUSE),
                                                Some(format!("Invalid pragma value \"{:?}\"",
                                                             pragma_value))));
            }
        };
        match pragma_value {
            ValueRef::Integer(i) => {
                sql.push_str(&i.to_string());
            }
            ValueRef::Real(r) => {
                sql.push_str(&r.to_string());
            }
            ValueRef::Text(s) => {
                sql.push_str(s);
            }
            _ => {
                return Err(Error::SqliteFailure(ffi::Error::new(ffi::SQLITE_MISUSE),
                                                Some(format!("Invalid pragma value \"{:?}\"",
                                                             pragma_value))))
            }
        };
        sql.push(')');
        if f.is_none() {
            return self.execute_batch(&sql);
        }
        let mut f = f.unwrap();
        let mut stmt = try!(self.prepare(&sql));
        let mut rows = try!(stmt.query(&[]));
        while let Some(result_row) = rows.next() {
            let row = try!(result_row);
            try!(f(&row));
        }
        Ok(())
    }
}

fn double_quote(query: &mut String, schema_name: DatabaseName) {
    match schema_name {
        DatabaseName::Main => query.push_str("main"),
        DatabaseName::Temp => query.push_str("temp"),
        DatabaseName::Attached(ref s) => {
            if is_identifier(s) {
                query.push_str(s)
            } else {
                wrap_and_escape(query, s, '"')
            }
        }
    }
}

fn wrap_and_escape(query: &mut String, s: &str, quote: char) {
    query.push(quote);
    let chars = s.chars();
    for ch in chars {
        // escape `quote` by doubling it
        if ch == quote {
            query.push(ch);
        }
        query.push(ch)
    }
    query.push(quote);
}

fn is_identifier(s: &str) -> bool {
    let chars = s.char_indices();
    for (i, ch) in chars {
        if i == 0 {
            if !is_identifier_start(ch) {
                return false;
            }
        } else if !is_identifier_continue(ch) {
            return false;
        }
    }
    true
}

fn is_identifier_start(c: char) -> bool {
    (c >= 'A' && c <= 'Z') || c == '_' || (c >= 'a' && c <= 'z') || c > '\x7F'
}

fn is_identifier_continue(c: char) -> bool {
    c == '$' || (c >= '0' && c <= '9') || (c >= 'A' && c <= 'Z') || c == '_' ||
    (c >= 'a' && c <= 'z') || c > '\x7F'
}
