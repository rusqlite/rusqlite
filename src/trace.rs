//! Tracing and profiling functions. Error and warning log.

use std::borrow::Cow;
use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::marker::PhantomData;
use std::panic::catch_unwind;
use std::time::Duration;

use super::ffi;
use crate::{Connection, MAIN_DB, Result, StatementStatus};

/// Set up the process-wide SQLite error logging callback.
///
/// # Safety
///
/// This function is marked unsafe for two reasons:
///
/// * The function is not threadsafe. No other SQLite calls may be made while
///   `config_log` is running, and multiple threads may not call `config_log`
///   simultaneously.
/// * The provided `callback` itself function has two requirements:
///     * It must not invoke any SQLite calls.
///     * It must be threadsafe if SQLite is used in a multithreaded way.
///
/// cf [The Error And Warning Log](http://sqlite.org/errlog.html).
#[cfg(not(feature = "loadable_extension"))]
pub unsafe fn config_log(callback: Option<fn(c_int, &str)>) -> Result<()> {
    extern "C" fn log_callback(p_arg: *mut c_void, err: c_int, msg: *const c_char) {
        let s = unsafe { CStr::from_ptr(msg).to_string_lossy() };
        let callback: fn(c_int, &str) = unsafe { std::mem::transmute(p_arg) };

        drop(catch_unwind(|| callback(err, &s)));
    }
    let rc = unsafe {
        if let Some(f) = callback {
            ffi::sqlite3_config(
                ffi::SQLITE_CONFIG_LOG,
                log_callback as extern "C" fn(_, _, _),
                f as *mut c_void,
            )
        } else {
            let nullptr: *mut c_void = std::ptr::null_mut();
            ffi::sqlite3_config(ffi::SQLITE_CONFIG_LOG, nullptr, nullptr)
        }
    };
    if rc == ffi::SQLITE_OK {
        Ok(())
    } else {
        Err(crate::error::error_from_sqlite_code(rc, None))
    }
}

/// Write a message into the error log established by
/// `config_log`.
#[inline]
pub fn log(err_code: c_int, msg: &str) {
    let msg = CString::new(msg).expect("SQLite log messages cannot contain embedded zeroes");
    unsafe { ffi::sqlite3_log(err_code, c"%s".as_ptr(), msg.as_ptr()) };
}

bitflags::bitflags! {
    /// Trace event codes
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    #[non_exhaustive]
    #[repr(C)]
    pub struct TraceEventCodes: c_uint {
        /// when a prepared statement first begins running and possibly at other times during the execution
        /// of the prepared statement, such as at the start of each trigger subprogram
        const SQLITE_TRACE_STMT = ffi::SQLITE_TRACE_STMT;
        /// when the statement finishes
        const SQLITE_TRACE_PROFILE = ffi::SQLITE_TRACE_PROFILE;
        /// whenever a prepared statement generates a single row of result
        const SQLITE_TRACE_ROW = ffi::SQLITE_TRACE_ROW;
        /// when a database connection closes
        const SQLITE_TRACE_CLOSE = ffi::SQLITE_TRACE_CLOSE;
    }
}

/// Trace event
#[non_exhaustive]
pub enum TraceEvent<'s> {
    /// when a prepared statement first begins running and possibly at other times during the execution
    /// of the prepared statement, such as at the start of each trigger subprogram
    Stmt(StmtRef<'s>, &'s str),
    /// when the statement finishes
    Profile(StmtRef<'s>, Duration),
    /// whenever a prepared statement generates a single row of result
    Row(StmtRef<'s>),
    /// when a database connection closes
    Close(ConnRef<'s>),
}

/// Statement reference
pub struct StmtRef<'s> {
    ptr: *mut ffi::sqlite3_stmt,
    phantom: PhantomData<&'s ()>,
}

impl StmtRef<'_> {
    fn new(ptr: *mut ffi::sqlite3_stmt) -> Self {
        StmtRef {
            ptr,
            phantom: PhantomData,
        }
    }

    /// SQL text
    #[must_use]
    pub fn sql(&self) -> Cow<'_, str> {
        let sql = unsafe { ffi::sqlite3_sql(self.ptr) };

        if sql.is_null() {
            return Cow::default();
        }

        // Safety: sql is a valid pointer to a cstr returned by sqlite3
        unsafe { CStr::from_ptr(sql).to_string_lossy() }
    }

    /// Expanded SQL text
    #[must_use]
    pub fn expanded_sql(&self) -> Option<String> {
        unsafe {
            crate::raw_statement::expanded_sql(self.ptr).map(|s| s.to_string_lossy().to_string())
        }
    }

    /// Get the value for one of the status counters for this statement.
    #[must_use]
    pub fn get_status(&self, status: StatementStatus) -> i32 {
        unsafe { crate::raw_statement::stmt_status(self.ptr, status, false) }
    }
}

/// Connection reference
pub struct ConnRef<'s> {
    ptr: *mut ffi::sqlite3,
    phantom: PhantomData<&'s ()>,
}

impl ConnRef<'_> {
    /// Test for auto-commit mode.
    #[must_use]
    pub fn is_autocommit(&self) -> bool {
        unsafe { crate::inner_connection::get_autocommit(self.ptr) }
    }

    /// the path to the database file, if one exists and is known.
    #[must_use]
    pub fn db_filename(&self) -> Option<&str> {
        unsafe { crate::inner_connection::db_filename(self.phantom, self.ptr, MAIN_DB) }
    }
}

impl Connection {
    /// Register or clear a trace callback function
    pub fn trace_v2<F>(&mut self, mask: TraceEventCodes, trace_fn: Option<F>) -> Result<()>
    where
        F: FnMut(TraceEvent<'_>) + Send + 'static,
    {
        unsafe extern "C" fn trace_callback<F>(
            evt: c_uint,
            ctx: *mut c_void,
            p: *mut c_void,
            x: *mut c_void,
        ) -> c_int
        where
            F: FnMut(TraceEvent<'_>),
        {
            unsafe {
                drop(catch_unwind(|| {
                    let trace_fn: *mut F = ctx.cast::<F>();
                    match evt {
                        ffi::SQLITE_TRACE_STMT => {
                            let str = CStr::from_ptr(x as *const c_char).to_string_lossy();
                            (*trace_fn)(TraceEvent::Stmt(
                                StmtRef::new(p.cast::<ffi::sqlite3_stmt>()),
                                &str,
                            ));
                        }
                        ffi::SQLITE_TRACE_PROFILE => {
                            let ns = *(x as *const i64);
                            (*trace_fn)(TraceEvent::Profile(
                                StmtRef::new(p.cast::<ffi::sqlite3_stmt>()),
                                Duration::from_nanos(u64::try_from(ns).unwrap_or_default()),
                            ));
                        }
                        ffi::SQLITE_TRACE_ROW => {
                            (*trace_fn)(TraceEvent::Row(StmtRef::new(
                                p.cast::<ffi::sqlite3_stmt>(),
                            )));
                        }
                        ffi::SQLITE_TRACE_CLOSE => (*trace_fn)(TraceEvent::Close(ConnRef {
                            ptr: p.cast::<ffi::sqlite3>(),
                            phantom: PhantomData,
                        })),
                        _ => {}
                    }
                }));
                // The integer return value from the callback is currently ignored, though this may change in future releases.
                // Callback implementations should return zero to ensure future compatibility.
                ffi::SQLITE_OK
            }
        }
        let mut c = self.db.borrow_mut();
        let x = trace_fn.as_ref().map(|_| trace_callback::<F> as _);
        c.set_clientdata(c"sqlite3_trace_v2", trace_fn, |db, bh| unsafe {
            ffi::sqlite3_trace_v2(db, mask.bits(), x, bh);
            ffi::SQLITE_OK
        })?;
        Ok(())
    }
}

#[cfg(all(test, not(miri)))]
mod test {
    use std::cell::RefCell;
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use std::time::Duration;

    use super::{TraceEvent, TraceEventCodes};
    use crate::{Connection, Result};

    #[test]
    pub fn trace_v2() -> Result<()> {
        use std::borrow::Borrow as _;
        use std::cmp::Ordering;

        let mut db = Connection::open_in_memory()?;
        db.trace_v2(
            TraceEventCodes::all(),
            Some(|e: TraceEvent<'_>| match e {
                TraceEvent::Stmt(s, sql) => {
                    assert_eq!(s.sql(), sql);
                }
                TraceEvent::Profile(s, d) => {
                    assert_eq!(s.get_status(crate::StatementStatus::Sort), 0);
                    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
                    assert_eq!(d.cmp(&Duration::ZERO), Ordering::Greater);
                    // Timers on the web are not very accurate
                    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
                    std::assert_matches!(
                        d.cmp(&Duration::ZERO),
                        Ordering::Equal | Ordering::Greater
                    );
                }
                TraceEvent::Row(s) => {
                    assert_eq!(s.expanded_sql().as_deref(), Some(s.sql().borrow()));
                }
                TraceEvent::Close(db) => {
                    assert!(db.is_autocommit());
                    // https://www.sqlite.org/c3ref/db_filename.html
                    // if database N is a temporary or in-memory database,
                    // then this function will return either a NULL pointer or an empty string.
                    assert!(db.db_filename().is_none_or(|s| s.is_empty()));
                }
            }),
        )?;

        db.one_column::<u32, _>("PRAGMA user_version", [])?;
        drop(db);

        let mut db = Connection::open_in_memory()?;
        db.trace_v2(TraceEventCodes::empty(), None::<fn(TraceEvent<'_>)>)
    }

    #[test]
    #[cfg(feature = "blob")]
    pub fn null_sql() -> Result<()> {
        let mut db = Connection::open_in_memory()?;
        let sql = "CREATE TABLE test (content BLOB);
                   INSERT INTO test VALUES (ZEROBLOB(10));";
        db.execute_batch(sql)?;
        let rowid = db.last_insert_rowid();

        db.trace_v2(
            TraceEventCodes::SQLITE_TRACE_ROW,
            Some(|e: TraceEvent<'_>| {
                if let TraceEvent::Row(s) = e {
                    assert_eq!(s.sql(), "");
                }
            }),
        )?;
        db.blob_open(crate::MAIN_DB, c"test", c"content", rowid, true)?;

        Ok(())
    }

    #[test]
    #[cfg_attr(all(target_family = "wasm", target_os = "unknown"), ignore)]
    pub fn already_borrowed() {
        thread_local! {
            static CONN: RefCell<Option<Connection>> = const { RefCell::new(None) };
        }
        CONN.with_borrow_mut(|c| c.replace(Connection::open_in_memory().unwrap()));
        let budget: Vec<u8> = vec![0];
        CONN.with_borrow_mut(|c| {
            c.as_mut()
                .unwrap()
                .trace_v2(
                    TraceEventCodes::SQLITE_TRACE_STMT,
                    Some(move |e: TraceEvent<'_>| {
                        if let TraceEvent::Stmt(_, sql) = e {
                            println!("{}", sql);
                        }
                        CONN.with_borrow_mut(|c| {
                            c.as_mut()
                                .unwrap()
                                .trace_v2(
                                    TraceEventCodes::SQLITE_TRACE_STMT,
                                    None::<fn(TraceEvent)>,
                                )
                                .unwrap();
                        });
                        println!("{}", budget[0]);
                    }),
                )
                .unwrap();
            c.as_ref()
                .unwrap()
                .query_one("SELECT 1;", [], |r| r.get::<_, bool>(0))
                .unwrap();
        });
    }
}
