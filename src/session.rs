//! [Session Extension](https://sqlite.org/sessionintro.html)

use std::marker::PhantomData;
use std::mem;
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;
use std::slice::from_raw_parts;

use error::{error_from_handle, error_from_sqlite_code};
use ffi;
use {errmsg_to_string, str_to_cstring, Connection, DatabaseName, Result};

// https://sqlite.org/session.html

/// An instance of this object is a session that can be used to record changes
/// to a database.
pub struct Session<'conn> {
    phantom: PhantomData<&'conn ()>,
    s: *mut ffi::sqlite3_session,
    db: *mut ffi::sqlite3, // TODO Validate: used only for error handling
    filter: Option<Box<Fn(&str) -> bool>>,
}

impl<'conn> Session<'conn> {
    /// Create a new session object
    pub fn new(db: &'conn Connection) -> Result<Session<'conn>> {
        Session::new_with_name(db, DatabaseName::Main)
    }

    /// Create a new session object
    pub fn new_with_name(db: &'conn Connection, name: DatabaseName) -> Result<Session<'conn>> {
        let name = try!(name.to_cstring());

        let db = db.db.borrow_mut().db;

        unsafe {
            let mut s: *mut ffi::sqlite3_session = mem::uninitialized();
            let r = ffi::sqlite3session_create(db, name.as_ptr(), &mut s);
            if r != ffi::SQLITE_OK {
                let e = error_from_handle(db, r);
                return Err(e);
            }

            Ok(Session {
                phantom: PhantomData,
                s,
                db,
                filter: None,
            })
        }
    }

    /// Set a table filter
    pub fn table_filter<F>(&mut self, filter: Option<F>)
    where
        F: Fn(&str) -> bool + Send + 'static,
    {
        unsafe extern "C" fn call_boxed_closure<F>(
            p_arg: *mut c_void,
            tbl_str: *const c_char,
        ) -> c_int
        where
            F: Fn(&str) -> bool,
        {
            use std::ffi::CStr;
            use std::str;

            let boxed_filter: *mut F = p_arg as *mut F;
            let tbl_name = {
                let c_slice = CStr::from_ptr(tbl_str).to_bytes();
                str::from_utf8_unchecked(c_slice)
            };
            if (*boxed_filter)(tbl_name) {
                1
            } else {
                0
            }
        }

        match filter {
            Some(filter) => {
                let boxed_filter = Box::new(filter);
                unsafe {
                    ffi::sqlite3session_table_filter(
                        self.s,
                        Some(call_boxed_closure::<F>),
                        &*boxed_filter as *const F as *mut _,
                    );
                }
                self.filter = Some(boxed_filter);
            }
            _ => {
                unsafe { ffi::sqlite3session_table_filter(self.s, None, ptr::null_mut()) }
                self.filter = None;
            }
        };
    }

    /// Attach a table. `None` means all tables.
    pub fn attach(&mut self, table: Option<&str>) -> Result<()> {
        let table = if let Some(table) = table {
            try!(str_to_cstring(table)).as_ptr()
        } else {
            ptr::null()
        };
        let r = unsafe { ffi::sqlite3session_attach(self.s, table) };
        if r != ffi::SQLITE_OK {
            let e = error_from_handle(self.db, r);
            return Err(e);
        }
        Ok(())
    }

    /// Generate a Changeset
    pub fn changeset(&mut self) -> Result<()> {
        // https://sqlite.org/session/sqlite3session_changeset.html
        unsafe {
            let mut cs: *mut c_void = mem::uninitialized();
            let mut n = 0;
            let r = ffi::sqlite3session_changeset(self.s, &mut n, &mut cs);
            if r != ffi::SQLITE_OK {
                let e = error_from_handle(self.db, r);
                return Err(e);
            }
            let _changeset = from_raw_parts(cs, n as usize); // TODO lifetime ?
                                                             // TODO must be sqlite3_free
            unimplemented!()
        }
    }

    /// Generate a Patchset
    pub fn patchset(&mut self) -> Result<()> {
        // https://sqlite.org/session/sqlite3session_patchset.html
        unsafe {
            let mut ps: *mut c_void = mem::uninitialized();
            let mut n = 0;
            let r = ffi::sqlite3session_patchset(self.s, &mut n, &mut ps);
            if r != ffi::SQLITE_OK {
                let e = error_from_handle(self.db, r);
                return Err(e);
            }
            let _patchset = from_raw_parts(ps, n as usize); // TODO lifetime ?
                                                            // TODO must be sqlite3_free
            unimplemented!()
        }
    }

    /// Load the difference between tables.
    pub fn diff(&mut self, from: DatabaseName, table: &str) -> Result<()> {
        let from = try!(from.to_cstring());
        let table = try!(str_to_cstring(table)).as_ptr();
        unsafe {
            let mut errmsg: *mut c_char = mem::uninitialized();
            let r = ffi::sqlite3session_diff(self.s, from.as_ptr(), table, &mut errmsg);
            if r != ffi::SQLITE_OK {
                let message = errmsg_to_string(&*errmsg);
                ffi::sqlite3_free(errmsg as *mut ::std::os::raw::c_void);
                return Err(error_from_sqlite_code(r, Some(message)));
            }
        }
        Ok(())
    }

    /// Test if a changeset has recorded any changes
    pub fn is_empty(&self) -> bool {
        unsafe { ffi::sqlite3session_isempty(self.s) != 0 }
    }

    /// Query the current state of the session
    pub fn is_enabled(&self) -> bool {
        unsafe { ffi::sqlite3session_enable(self.s, -1) != 0 }
    }

    /// Enable or disable the recording of changes
    pub fn set_enabled(&mut self, enabled: bool) {
        unsafe {
            ffi::sqlite3session_enable(self.s, if enabled { 1 } else { 0 });
        }
    }

    /// Query the current state of the indirect flag
    pub fn is_indirect(&self) -> bool {
        unsafe { ffi::sqlite3session_indirect(self.s, -1) != 0 }
    }

    /// Set or clear the indirect change flag
    pub fn set_indirect(&mut self, indirect: bool) {
        unsafe {
            ffi::sqlite3session_indirect(self.s, if indirect { 1 } else { 0 });
        }
    }
}

impl<'conn> Drop for Session<'conn> {
    fn drop(&mut self) {
        if self.filter.is_some() {
            self.table_filter(None::<fn(&str) -> bool>);
        }
        unsafe { ffi::sqlite3session_delete(self.s) };
    }
}

/// Cursor for iterating over the elements of a changeset or patchset.
pub struct ChangesetIter {
    it: *mut ffi::sqlite3_changeset_iter,
}

impl Drop for ChangesetIter {
    fn drop(&mut self) {
        unsafe {
            ffi::sqlite3changeset_finalize(self.it);
        }
    }
}

/// Used to combine two or more changesets or
/// patchsets
pub struct Changegroup {
    cg: *mut ffi::sqlite3_changegroup,
}

// https://sqlite.org/session/sqlite3changegroup_new.html
// https://sqlite.org/session/changegroup.html
impl Changegroup {
    pub fn new() -> Result<Self> {
        unsafe {
            let mut cg: *mut ffi::sqlite3_changegroup = mem::uninitialized();
            let r = ffi::sqlite3changegroup_new(&mut cg);
            if r != ffi::SQLITE_OK {
                return Err(error_from_sqlite_code(r, None));
            }
            Ok(Changegroup { cg })
        }
    }

    /// Add A Changeset
    pub fn add(&mut self) -> Result<()> {
        // https://sqlite.org/session/sqlite3changegroup_add.html
        let r = unsafe {
            //ffi::sqlite3changegroup_add(self.cg, )
            unimplemented!()
        };
        if r != ffi::SQLITE_OK {
            return Err(error_from_sqlite_code(r, None));
        }
        Ok(())
    }

    /// Obtain a composite Changeset
    pub fn output(&mut self) -> Result<()> {
        unsafe {
            let mut output: *mut c_void = mem::uninitialized();
            let mut n = 0;
            let r = ffi::sqlite3changegroup_output(self.cg, &mut n, &mut output);
            if r != ffi::SQLITE_OK {
                return Err(error_from_sqlite_code(r, None));
            }
            let _output = from_raw_parts(output, n as usize); // TODO lifetime ?
                                                              // TODO must be sqlite3_free
            unimplemented!()
        }
    }
}

impl Drop for Changegroup {
    fn drop(&mut self) {
        unsafe {
            ffi::sqlite3changegroup_delete(self.cg);
        }
    }
}
