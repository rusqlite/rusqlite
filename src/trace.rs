//! Tracing and profiling functions. Error and warning log.

use std::borrow::Cow;
use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::marker::PhantomData;
use std::mem;
use std::panic::catch_unwind;
use std::panic::AssertUnwindSafe;
use std::ptr;
use std::time::Duration;

use super::ffi;
use crate::util::ThinBoxAny;
use crate::{Connection, StatementStatus, MAIN_DB};

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
pub unsafe fn config_log<F>(callback: Option<F>) -> crate::Result<()>
where
    F: Fn(c_int, &str) + Send + Sync + 'static,
{
    extern "C" fn log_callback<F>(p_arg: *mut c_void, err: c_int, msg: *const c_char)
    where
        F: Fn(c_int, &str) + Send + Sync + 'static,
    {
        let s = unsafe { CStr::from_ptr(msg).to_string_lossy() };
        let callback: &F = unsafe { &*p_arg.cast::<F>() };

        drop(catch_unwind(AssertUnwindSafe(|| callback(err, &s))));
    }

    let callback = callback.map(|f| ptr::NonNull::from(Box::leak(Box::new(f))));

    let rc = if let Some(f) = callback {
        ffi::sqlite3_config(
            ffi::SQLITE_CONFIG_LOG,
            log_callback::<F> as extern "C" fn(_, _, _),
            f.as_ptr() as *mut c_void,
        )
    } else {
        let nullptr: *mut c_void = ptr::null_mut();
        ffi::sqlite3_config(ffi::SQLITE_CONFIG_LOG, nullptr, nullptr)
    };

    if rc == ffi::SQLITE_OK {
        // Remember the previous pointer so we can free it. It is safe to access this `static mut`
        // because this function is not thread-safe. We use `NonNull` instead of `Box` because the
        // latter has too strict aliasing requirements for this use case.
        #[expect(clippy::type_complexity)]
        static mut PREVIOUS: Option<ptr::NonNull<dyn Fn(c_int, &str) + Send + Sync>> = None;

        #[expect(static_mut_refs)]
        if let Some(ptr) = unsafe { PREVIOUS.take() } {
            drop(unsafe { Box::from_raw(ptr.as_ptr()) });
        }
        if let Some(callback) = callback {
            unsafe { PREVIOUS = Some(callback) };
        }

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
    unsafe {
        ffi::sqlite3_log(err_code, b"%s\0" as *const _ as *const c_char, msg.as_ptr());
    }
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
    pub fn sql(&self) -> Cow<'_, str> {
        unsafe { CStr::from_ptr(ffi::sqlite3_sql(self.ptr)).to_string_lossy() }
    }
    /// Expanded SQL text
    pub fn expanded_sql(&self) -> Option<String> {
        unsafe {
            crate::raw_statement::expanded_sql(self.ptr).map(|s| s.to_string_lossy().to_string())
        }
    }
    /// Get the value for one of the status counters for this statement.
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
    pub fn is_autocommit(&self) -> bool {
        unsafe { crate::inner_connection::get_autocommit(self.ptr) }
    }
    /// the path to the database file, if one exists and is known.
    pub fn db_filename(&self) -> Option<&str> {
        unsafe { crate::inner_connection::db_filename(self.phantom, self.ptr, MAIN_DB) }
    }
}

impl Connection {
    /// Register or clear a callback function that can be
    /// used for tracing the execution of SQL statements.
    ///
    /// Prepared statement placeholders are replaced/logged with their assigned
    /// values. There can only be a single tracer defined for each database
    /// connection. Setting a new tracer clears the old one.
    #[deprecated(since = "0.33.0", note = "use trace_v2 instead")]
    pub fn trace(&mut self, trace_fn: Option<fn(&str)>) {
        unsafe extern "C" fn trace_callback(p_arg: *mut c_void, z_sql: *const c_char) {
            let trace_fn: fn(&str) = mem::transmute(p_arg);
            let s = CStr::from_ptr(z_sql).to_string_lossy();
            drop(catch_unwind(|| trace_fn(&s)));
        }

        let c = self.db.borrow_mut();
        match trace_fn {
            Some(f) => unsafe {
                ffi::sqlite3_trace(c.db(), Some(trace_callback), f as *mut c_void);
            },
            None => unsafe {
                ffi::sqlite3_trace(c.db(), None, ptr::null_mut());
            },
        }
    }

    /// Register or clear a callback function that can be
    /// used for profiling the execution of SQL statements.
    ///
    /// There can only be a single profiler defined for each database
    /// connection. Setting a new profiler clears the old one.
    #[deprecated(since = "0.33.0", note = "use trace_v2 instead")]
    pub fn profile(&mut self, profile_fn: Option<fn(&str, Duration)>) {
        unsafe extern "C" fn profile_callback(
            p_arg: *mut c_void,
            z_sql: *const c_char,
            nanoseconds: u64,
        ) {
            let profile_fn: fn(&str, Duration) = mem::transmute(p_arg);
            let s = CStr::from_ptr(z_sql).to_string_lossy();

            let duration = Duration::from_nanos(nanoseconds);
            drop(catch_unwind(|| profile_fn(&s, duration)));
        }

        let c = self.db.borrow_mut();
        match profile_fn {
            Some(f) => unsafe {
                ffi::sqlite3_profile(c.db(), Some(profile_callback), f as *mut c_void)
            },
            None => unsafe { ffi::sqlite3_profile(c.db(), None, ptr::null_mut()) },
        };
    }

    /// Register or clear a trace callback function
    pub fn trace_v2<F>(&self, mask: TraceEventCodes, trace_fn: Option<F>)
    where
        F: Fn(TraceEvent<'_>) + Send + Sync + 'static,
    {
        unsafe extern "C" fn trace_callback<F: Fn(TraceEvent<'_>) + Send + Sync + 'static>(
            evt: c_uint,
            ctx: *mut c_void,
            p: *mut c_void,
            x: *mut c_void,
        ) -> c_int {
            let trace_fn: &F = unsafe { &*ctx.cast::<F>() };
            drop(catch_unwind(AssertUnwindSafe(|| match evt {
                ffi::SQLITE_TRACE_STMT => {
                    let str = CStr::from_ptr(x as *const c_char).to_string_lossy();
                    trace_fn(TraceEvent::Stmt(
                        StmtRef::new(p as *mut ffi::sqlite3_stmt),
                        &str,
                    ))
                }
                ffi::SQLITE_TRACE_PROFILE => {
                    let ns = *(x as *const i64);
                    trace_fn(TraceEvent::Profile(
                        StmtRef::new(p as *mut ffi::sqlite3_stmt),
                        Duration::from_nanos(u64::try_from(ns).unwrap_or_default()),
                    ))
                }
                ffi::SQLITE_TRACE_ROW => {
                    trace_fn(TraceEvent::Row(StmtRef::new(p as *mut ffi::sqlite3_stmt)))
                }
                ffi::SQLITE_TRACE_CLOSE => trace_fn(TraceEvent::Close(ConnRef {
                    ptr: p as *mut ffi::sqlite3,
                    phantom: PhantomData,
                })),
                _ => {}
            })));
            // The integer return value from the callback is currently ignored, though this may change in future releases.
            // Callback implementations should return zero to ensure future compatibility.
            ffi::SQLITE_OK
        }

        let mut c = self.db.borrow_mut();

        let (boxed, trace_fn) = ThinBoxAny::new_option(trace_fn);
        if let Some(f) = trace_fn {
            unsafe {
                ffi::sqlite3_trace_v2(
                    c.db(),
                    mask.bits(),
                    Some(trace_callback::<F>),
                    f as *mut c_void,
                );
            }
        } else {
            unsafe {
                ffi::sqlite3_trace_v2(c.db(), 0, None, ptr::null_mut());
            }
        }
        c.trace_v2 = boxed;
    }
}

#[cfg(test)]
mod test {
    use std::sync::{LazyLock, Mutex};
    use std::time::Duration;

    use crate::{Connection, Result};

    #[test]
    #[allow(deprecated)]
    fn test_trace() -> Result<()> {
        static TRACED_STMTS: LazyLock<Mutex<Vec<String>>> =
            LazyLock::new(|| Mutex::new(Vec::new()));
        fn tracer(s: &str) {
            let mut traced_stmts = TRACED_STMTS.lock().unwrap();
            traced_stmts.push(s.to_owned());
        }

        let mut db = Connection::open_in_memory()?;
        db.trace(Some(tracer));
        {
            let _ = db.query_row("SELECT ?1", [1i32], |_| Ok(()));
            let _ = db.query_row("SELECT ?1", ["hello"], |_| Ok(()));
        }
        db.trace(None);
        {
            let _ = db.query_row("SELECT ?1", [2i32], |_| Ok(()));
            let _ = db.query_row("SELECT ?1", ["goodbye"], |_| Ok(()));
        }

        let traced_stmts = TRACED_STMTS.lock().unwrap();
        assert_eq!(traced_stmts.len(), 2);
        assert_eq!(traced_stmts[0], "SELECT 1");
        assert_eq!(traced_stmts[1], "SELECT 'hello'");
        Ok(())
    }

    #[test]
    #[allow(deprecated)]
    fn test_profile() -> Result<()> {
        static PROFILED: LazyLock<Mutex<Vec<(String, Duration)>>> =
            LazyLock::new(|| Mutex::new(Vec::new()));
        fn profiler(s: &str, d: Duration) {
            let mut profiled = PROFILED.lock().unwrap();
            profiled.push((s.to_owned(), d));
        }

        let mut db = Connection::open_in_memory()?;
        db.profile(Some(profiler));
        db.execute_batch("PRAGMA application_id = 1")?;
        db.profile(None);
        db.execute_batch("PRAGMA application_id = 2")?;

        let profiled = PROFILED.lock().unwrap();
        assert_eq!(profiled.len(), 1);
        assert_eq!(profiled[0].0, "PRAGMA application_id = 1");
        Ok(())
    }

    #[test]
    pub fn trace_v2() -> Result<()> {
        use super::{TraceEvent, TraceEventCodes};
        use std::borrow::Borrow;
        use std::cmp::Ordering;

        let db = Connection::open_in_memory()?;
        db.trace_v2(
            TraceEventCodes::all(),
            Some(|e: TraceEvent<'_>| match e {
                TraceEvent::Stmt(s, sql) => {
                    assert_eq!(s.sql(), sql);
                }
                TraceEvent::Profile(s, d) => {
                    assert_eq!(s.get_status(crate::StatementStatus::Sort), 0);
                    assert_eq!(d.cmp(&Duration::ZERO), Ordering::Greater)
                }
                TraceEvent::Row(s) => {
                    assert_eq!(s.expanded_sql().as_deref(), Some(s.sql().borrow()));
                }
                TraceEvent::Close(db) => {
                    assert!(db.is_autocommit());
                    assert!(db.db_filename().is_none());
                }
            }),
        );

        db.one_column::<u32>("PRAGMA user_version")?;
        drop(db);

        let db = Connection::open_in_memory()?;
        db.trace_v2(TraceEventCodes::empty(), None::<fn(TraceEvent<'_>)>);
        Ok(())
    }
}
