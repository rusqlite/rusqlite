use std::ffi::{CStr, c_char, c_int, c_void};
#[cfg(feature = "load_extension")]
use std::path::Path;
use std::ptr;
use std::str;
use std::sync::{Arc, Mutex};

use super::ffi;
use super::{Connection, InterruptHandle, Name, OpenFlags, PrepFlags, Result};
use crate::error::{Error, decode_result_raw, error_from_handle, error_with_offset};
use crate::raw_statement::RawStatement;
use crate::statement::Statement;
use crate::version_number;

pub struct InnerConnection {
    pub db: *mut ffi::sqlite3,
    // It's unsafe to call `sqlite3_close` while another thread is performing
    // a `sqlite3_interrupt`, and vice versa, so we take this mutex during
    // those functions. This protects a copy of the `db` pointer (which is
    // cleared on closing), however the main copy, `db`, is unprotected.
    // Otherwise, a long-running query would prevent calling interrupt, as
    // interrupt would only acquire the lock after the query's completion.
    interrupt_lock: Arc<Mutex<*mut ffi::sqlite3>>,
    owned: bool,
}

unsafe impl Send for InnerConnection {}

impl InnerConnection {
    #[expect(clippy::arc_with_non_send_sync)] // See unsafe impl Send / Sync for InterruptHandle
    #[inline]
    pub unsafe fn new(db: *mut ffi::sqlite3, owned: bool) -> Self {
        Self {
            db,
            interrupt_lock: Arc::new(Mutex::new(if owned { db } else { ptr::null_mut() })),
            owned,
        }
    }

    pub fn open_with_flags(
        c_path: &CStr,
        mut flags: OpenFlags,
        vfs: Option<&CStr>,
    ) -> Result<Self> {
        ensure_safe_sqlite_threading_mode()?;

        let z_vfs = match vfs {
            Some(c_vfs) => c_vfs.as_ptr(),
            None => ptr::null(),
        };

        // turn on extended results code before opening database to have a better diagnostic if a failure happens
        let exrescode = if version_number() >= 3_037_000 {
            flags |= OpenFlags::SQLITE_OPEN_EXRESCODE;
            true
        } else {
            false // flag SQLITE_OPEN_EXRESCODE is ignored by SQLite version < 3.37.0
        };

        unsafe {
            let mut db: *mut ffi::sqlite3 = ptr::null_mut();
            let r = ffi::sqlite3_open_v2(c_path.as_ptr(), &raw mut db, flags.bits(), z_vfs);
            if r != ffi::SQLITE_OK {
                let e = if db.is_null() {
                    err!(r, "{}", c_path.to_string_lossy())
                } else {
                    let mut e = error_from_handle(db, r);
                    if let Error::SqliteFailure(
                        ffi::Error {
                            code: ffi::ErrorCode::CannotOpen,
                            ..
                        },
                        Some(msg),
                    ) = e
                    {
                        e = err!(r, "{msg}: {}", c_path.to_string_lossy());
                    }
                    ffi::sqlite3_close(db);
                    e
                };

                return Err(e);
            }

            // attempt to turn on extended results code; don't fail if we can't.
            if !exrescode {
                ffi::sqlite3_extended_result_codes(db, 1);
            }

            let r = ffi::sqlite3_busy_timeout(db, 5000);
            if r != ffi::SQLITE_OK {
                let e = error_from_handle(db, r);
                ffi::sqlite3_close(db);
                return Err(e);
            }

            Ok(Self::new(db, true))
        }
    }

    #[inline]
    pub fn db(&self) -> *mut ffi::sqlite3 {
        self.db
    }

    #[inline]
    pub fn decode_result(&self, code: c_int) -> Result<()> {
        unsafe { decode_result_raw(self.db(), code) }
    }

    pub fn close(&mut self) -> Result<()> {
        if self.db.is_null() {
            return Ok(());
        }
        let mut shared_handle = self.interrupt_lock.lock().unwrap();
        assert!(
            !self.owned || !shared_handle.is_null(),
            "Bug: Somehow interrupt_lock was cleared before the DB was closed"
        );
        if !self.owned {
            self.db = ptr::null_mut();
            return Ok(());
        }
        unsafe {
            let r = ffi::sqlite3_close(self.db);
            // Need to use _raw because _guard has a reference out, and
            // decode_result takes &mut self.
            let r = decode_result_raw(self.db, r);
            if r.is_ok() {
                *shared_handle = ptr::null_mut();
                self.db = ptr::null_mut();
            }
            r
        }
    }

    #[inline]
    pub fn get_interrupt_handle(&self) -> InterruptHandle {
        InterruptHandle {
            db_lock: Arc::clone(&self.interrupt_lock),
        }
    }

    #[inline]
    #[cfg(feature = "load_extension")]
    pub unsafe fn enable_load_extension(&mut self, onoff: c_int) -> Result<()> {
        let r = unsafe { ffi::sqlite3_enable_load_extension(self.db, onoff) };
        self.decode_result(r)
    }

    #[cfg(feature = "load_extension")]
    pub unsafe fn load_extension<N: Name>(
        &self,
        dylib_path: &Path,
        entry_point: Option<N>,
    ) -> Result<()> {
        let dylib_str = super::path_to_cstring(dylib_path)?;
        let mut errmsg: *mut c_char = ptr::null_mut();
        let cs = entry_point.as_ref().map(N::as_cstr).transpose()?;
        let c_entry = cs.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        unsafe {
            let r =
                ffi::sqlite3_load_extension(self.db, dylib_str.as_ptr(), c_entry, &raw mut errmsg);
            if r == ffi::SQLITE_OK {
                Ok(())
            } else {
                let message = super::errmsg_to_string(errmsg);
                ffi::sqlite3_free(errmsg.cast::<c_void>());
                Err(crate::error::error_from_sqlite_code(r, Some(message)))
            }
        }
    }

    #[inline]
    pub fn last_insert_rowid(&self) -> i64 {
        unsafe { ffi::sqlite3_last_insert_rowid(self.db()) }
    }

    pub fn prepare<'a>(
        &mut self,
        conn: &'a Connection,
        sql: &str,
        flags: PrepFlags,
    ) -> Result<(Statement<'a>, usize)> {
        let mut c_stmt: *mut ffi::sqlite3_stmt = ptr::null_mut();
        let Ok(len) = c_int::try_from(sql.len()) else {
            return Err(err!(ffi::SQLITE_TOOBIG));
        };
        let c_sql = sql.as_bytes().as_ptr().cast::<c_char>();
        let mut c_tail: *const c_char = ptr::null();
        let r = cfg_select! {
            feature = "unlock_notify" => {
                // no_fmt
                unsafe {
                    use crate::unlock_notify;
                    let mut rc;
                    loop {
                        rc = ffi::sqlite3_prepare_v3(
                            self.db(),
                            c_sql,
                            len,
                            flags.bits(),
                            &raw mut c_stmt,
                            &raw mut c_tail,
                        );
                        if !unlock_notify::is_locked(self.db, rc) {
                            break;
                        }
                        rc = unlock_notify::wait_for_unlock_notify(self.db);
                        if rc != ffi::SQLITE_OK {
                            break;
                        }
                    }
                    rc
                }
            }
            _ => {
                // no_fmt
                unsafe {
                    ffi::sqlite3_prepare_v3(
                        self.db(),
                        c_sql,
                        len,
                        flags.bits(),
                        &mut c_stmt,
                        &mut c_tail,
                    )
                }
            }
        };
        // If there is an error, *ppStmt is set to NULL.
        if r != ffi::SQLITE_OK {
            return Err(unsafe { error_with_offset(self.db, r, sql) });
        }
        // If the input text contains no SQL (if the input is an empty string or a
        // comment) then *ppStmt is set to NULL.
        let tail = if c_tail.is_null() {
            0
        } else {
            let n = (c_tail as isize) - (c_sql as isize);
            if n <= 0 || n >= len as isize {
                0
            } else {
                n as usize
            }
        };
        Ok((
            Statement::new(conn, unsafe { RawStatement::new(c_stmt) }),
            tail,
        ))
    }

    #[inline]
    pub fn changes(&self) -> u64 {
        unsafe { ffi::sqlite3_changes64(self.db()) as u64 }
    }

    #[inline]
    pub fn total_changes(&self) -> u64 {
        unsafe { ffi::sqlite3_total_changes64(self.db()) as u64 }
    }

    #[inline]
    pub fn is_autocommit(&self) -> bool {
        unsafe { get_autocommit(self.db()) }
    }

    pub fn is_busy(&self) -> bool {
        let db = self.db();
        unsafe {
            let mut stmt = ffi::sqlite3_next_stmt(db, ptr::null_mut());
            while !stmt.is_null() {
                if ffi::sqlite3_stmt_busy(stmt) != 0 {
                    return true;
                }
                stmt = ffi::sqlite3_next_stmt(db, stmt);
            }
        }
        false
    }

    pub fn cache_flush(&mut self) -> Result<()> {
        crate::error::check(unsafe { ffi::sqlite3_db_cacheflush(self.db()) })
    }

    pub fn db_readonly<N: Name>(&self, db_name: N) -> Result<bool> {
        let name = db_name.as_cstr()?;
        let r = unsafe { ffi::sqlite3_db_readonly(self.db, name.as_ptr()) };
        match r {
            0 => Ok(false),
            1 => Ok(true),
            -1 => Err(err!(
                ffi::SQLITE_MISUSE,
                "{db_name:?} is not the name of a database"
            )),
            _ => Err(err!(r, "Unexpected result")),
        }
    }

    pub fn txn_state<N: Name>(
        &self,
        db_name: Option<N>,
    ) -> Result<super::transaction::TransactionState> {
        let cs = db_name.as_ref().map(N::as_cstr).transpose()?;
        let name = cs.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        let r = unsafe { ffi::sqlite3_txn_state(self.db, name) };
        match r {
            0 => Ok(super::transaction::TransactionState::None),
            1 => Ok(super::transaction::TransactionState::Read),
            2 => Ok(super::transaction::TransactionState::Write),
            -1 => Err(err!(
                ffi::SQLITE_MISUSE,
                "{db_name:?} is not the name of a valid schema"
            )),
            _ => Err(err!(r, "Unexpected result")),
        }
    }

    #[inline]
    pub fn release_memory(&self) -> Result<()> {
        self.decode_result(unsafe { ffi::sqlite3_db_release_memory(self.db) })
    }

    pub fn is_interrupted(&self) -> bool {
        unsafe { ffi::sqlite3_is_interrupted(self.db) == 1 }
    }

    pub unsafe fn file_control<N: Name>(
        &self,
        db_name: Option<N>,
        op: c_int,
        arg: *mut c_void,
    ) -> Result<()> {
        let cs = db_name.as_ref().map(N::as_cstr).transpose()?;
        let cn = cs.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        // error code is not remembered and will not be recalled by sqlite3_errcode() or sqlite3_errmsg()
        crate::error::check(unsafe { ffi::sqlite3_file_control(self.db, cn, op, arg) })
    }

    pub fn set_clientdata<
        T: Send + 'static,
        N: Name,
        F: Fn(*mut ffi::sqlite3, *mut c_void) -> c_int,
    >(
        &mut self,
        name: N,
        data: Option<T>,
        preset: F,
    ) -> Result<*mut T> {
        let name = name.as_cstr()?;
        let ptr = data.map_or(ptr::null_mut(), |d| Box::into_raw(Box::new(d)));
        let res = self.decode_result(preset(self.db, ptr.cast()));
        if res.is_err() {
            if !ptr.is_null() {
                unsafe { crate::util::free_boxed_value::<T>(ptr.cast()) };
            }
            res?;
        }
        self.decode_result(unsafe {
            ffi::sqlite3_set_clientdata(
                self.db,
                name.as_ptr(),
                ptr.cast::<c_void>(),
                if ptr.is_null() {
                    None
                } else {
                    Some(crate::util::free_boxed_value::<T>)
                },
            )
        })?;
        Ok(ptr)
    }
    pub unsafe fn get_clientdata<T, N: Name>(&self, name: N) -> Result<*mut T> {
        let name = name.as_cstr()?;
        Ok(unsafe { ffi::sqlite3_get_clientdata(self.db, name.as_ptr()).cast::<T>() })
    }
}

#[inline]
pub(crate) unsafe fn get_autocommit(ptr: *mut ffi::sqlite3) -> bool {
    unsafe { ffi::sqlite3_get_autocommit(ptr) != 0 }
}

#[inline]
pub(crate) unsafe fn db_filename<N: Name>(
    _: std::marker::PhantomData<&()>,
    ptr: *mut ffi::sqlite3,
    db_name: N,
) -> Option<&str> {
    let db_name = db_name.as_cstr().unwrap();
    unsafe {
        let db_filename = ffi::sqlite3_db_filename(ptr, db_name.as_ptr());
        if db_filename.is_null() {
            None
        } else {
            CStr::from_ptr(db_filename).to_str().ok()
        }
    }
}

impl Drop for InnerConnection {
    #[expect(unused_must_use)]
    #[inline]
    fn drop(&mut self) {
        self.close();
    }
}

// threading mode checks are not necessary (and do not work) on target
// platforms that do not have threading (such as webassembly)
#[cfg(target_arch = "wasm32")]
fn ensure_safe_sqlite_threading_mode() -> Result<()> {
    Ok(())
}

#[cfg(not(any(target_arch = "wasm32")))]
fn ensure_safe_sqlite_threading_mode() -> Result<()> {
    // Ensure SQLite was compiled in threadsafe mode.
    if unsafe { ffi::sqlite3_threadsafe() == 0 } {
        return Err(Error::SqliteSingleThreadedMode);
    }

    // Now we know SQLite is _capable_ of being in Multi-thread of Serialized mode,
    // but it's possible someone configured it to be in Single-thread mode
    // before calling into us. That would mean we're exposing an unsafe API via
    // a safe one (in Rust terminology).
    //
    // We can ask SQLite for a mutex and check for
    // the magic value 8. This isn't documented, but it's what SQLite
    // returns for its mutex allocation function in Single-thread mode.
    const SQLITE_SINGLETHREADED_MUTEX_MAGIC: usize = 8;
    let is_singlethreaded = unsafe {
        let mutex_ptr = ffi::sqlite3_mutex_alloc(0);
        let is_singlethreaded = mutex_ptr as usize == SQLITE_SINGLETHREADED_MUTEX_MAGIC;
        ffi::sqlite3_mutex_free(mutex_ptr);
        is_singlethreaded
    };
    if is_singlethreaded {
        Err(Error::SqliteSingleThreadedMode)
    } else {
        Ok(())
    }
}
