use lazy_static::lazy_static;
use libsqlite3_sys::{
    sqlite3, sqlite3_context, sqlite3_create_function_v2, sqlite3_result_error, sqlite3_result_int,
    sqlite3_user_data, sqlite3_value, sqlite3_value_text, SQLITE_DETERMINISTIC, SQLITE_UTF8,
};
use regex::Regex;
use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::{c_void, CStr};

type RegexCache = HashMap<Cow<'static, str>, Regex>;
type RegexCacheOwned = Box<RegexCache>;
type RegexCacheRef = &'static mut RegexCache;

lazy_static! {
    static ref REGEX_NAME: &'static CStr =
        unsafe { CStr::from_bytes_with_nul_unchecked(b"REGEXP\0") };
}

pub fn init(db: *mut sqlite3) -> crate::Result<()> {
    unsafe {
        // TODO does this need to be thread-safe?
        let regex_cache: RegexCacheOwned = Box::new(HashMap::new());

        check!(sqlite3_create_function_v2(
            db,
            REGEX_NAME.as_ptr(),
            2,
            SQLITE_UTF8 | SQLITE_DETERMINISTIC,
            Box::into_raw(regex_cache) as *mut c_void,
            Some(run_regex),
            None,
            None,
            Some(destroy),
        ));
    }
    Ok(())
}

unsafe extern "C" fn run_regex(
    ctx: *mut sqlite3_context,
    args: i32,
    argv: *mut *mut sqlite3_value,
) {
    fn run_regex_impl(
        cache: &mut HashMap<Cow<str>, Regex>,
        pattern: &str,
        haystack: &str,
    ) -> Result<bool, String> {
        if let Some(regex) = cache.get(pattern) {
            return Ok(regex.is_match(haystack));
        }
        let regex = match Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return Err(e.to_string()),
        };
        let result = regex.is_match(haystack);
        cache.insert(Cow::Owned(pattern.into()), regex);
        Ok(result)
    }

    if args != 2 {
        let msg = format!("expected 2 args, found {}\0", args);
        sqlite3_result_error(ctx, msg.as_ptr() as *const i8, msg.len() as i32)
    }
    // neither of these should allocate because sqlite3 returns valid utf8
    let pattern = CStr::from_ptr(sqlite3_value_text(*argv) as *const i8).to_string_lossy();
    let haystack =
        CStr::from_ptr(sqlite3_value_text(*argv.offset(1)) as *const i8).to_string_lossy();
    let cache: RegexCacheRef = &mut *(sqlite3_user_data(ctx) as *const _ as *mut RegexCache);

    let result = run_regex_impl(cache, &pattern, &haystack);
    match result {
        Ok(res) => sqlite3_result_int(ctx, if res { 1 } else { 0 }),
        Err(e) => sqlite3_result_error(ctx, e.as_ptr() as *const i8, e.len() as i32),
    }
}

unsafe extern "C" fn destroy(data: *mut c_void) {
    let _: RegexCacheOwned = Box::from_raw(data as *mut _);
}

#[cfg(test)]
mod test {
    use crate::{params, Connection, Result, Row, NO_PARAMS};

    #[derive(Debug, PartialEq, Clone)]
    struct Record {
        t: String,
        i: i32,
        f: f64,
        b: Vec<u8>,
    }

    impl Record {
        fn from_row(row: &Row) -> Result<Self> {
            Ok(Record {
                t: row.get(0)?,
                i: row.get(1)?,
                f: row.get(2)?,
                b: row.get(3)?,
            })
        }
    }

    fn checked_memory_handle() -> Result<Connection> {
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE foo (t TEXT, i INTEGER, f FLOAT, b BLOB)")?;
        let mut stmt = db.prepare("INSERT INTO foo (t, i, f, b) VALUES (?, ?, ?, ?)")?;
        stmt.execute(params!["first person", 1, 1.0, &b"a blob"[..]])?;
        stmt.execute(params!["a n other", 100, 32.1, &b"some blob-like text"[..]])?;
        drop(stmt);
        Ok(db)
    }

    #[test]
    fn regex() {
        fn check_query(db: &Connection, query: &str, expected: &Record) {
            let ans = db
                .query_row_and_then(query, NO_PARAMS, Record::from_row)
                .unwrap();
            assert_eq!(ans, *expected);
        }
        let db = checked_memory_handle().unwrap();
        let first_expected = Record {
            t: "first person".into(),
            i: 1,
            f: 1.0,
            b: b"a blob".to_vec(),
        };
        let second_expected = Record {
            t: "a n other".into(),
            i: 100,
            f: 32.1,
            b: b"some blob-like text".to_vec(),
        };
        check_query(
            &db,
            r#"SELECT t, i, f, b FROM foo WHERE t REGEXP "f[aei]r[st]{2}""#,
            &first_expected,
        );
        check_query(
            &db,
            r#"SELECT t, i, f, b FROM foo WHERE f REGEXP "\d+\.1""#,
            &second_expected,
        );
        check_query(
            &db,
            r#"SELECT t, i, f, b FROM foo WHERE b REGEXP "\w{4} \w{4}-\w{4} \w{4}""#,
            &second_expected,
        );
    }
}
