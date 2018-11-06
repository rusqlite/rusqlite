//! [Session Extension](https://sqlite.org/sessionintro.html)
#![allow(non_camel_case_types)]

use std::marker::PhantomData;
use std::mem;
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;

use error::error_from_sqlite_code;
use ffi;
use {errmsg_to_string, str_to_cstring, Connection, DatabaseName, Result};

// https://sqlite.org/session.html

/// An instance of this object is a session that can be used to record changes
/// to a database.
pub struct Session<'conn> {
    phantom: PhantomData<&'conn ()>,
    s: *mut ffi::sqlite3_session,
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
            check!(ffi::sqlite3session_create(db, name.as_ptr(), &mut s));

            Ok(Session {
                phantom: PhantomData,
                s,
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
        unsafe { check!(ffi::sqlite3session_attach(self.s, table)) };
        Ok(())
    }

    /// Generate a Changeset
    pub fn changeset(&mut self) -> Result<Changeset> {
        unsafe {
            let mut cs: *mut c_void = mem::uninitialized();
            let mut n = 0;
            check!(ffi::sqlite3session_changeset(self.s, &mut n, &mut cs));
            Ok(Changeset { cs, n })
        }
    }

    // sqlite3session_changeset_strm

    /// Generate a Patchset
    pub fn patchset(&mut self) -> Result<Changeset> {
        unsafe {
            let mut ps: *mut c_void = mem::uninitialized();
            let mut n = 0;
            check!(ffi::sqlite3session_patchset(self.s, &mut n, &mut ps));
            // TODO Validate: same struct
            Ok(Changeset { cs: ps, n })
        }
    }

    // sqlite3session_patchset_strm

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

// sqlite3changeset_apply_strm
// sqlite3changeset_invert_strm
// sqlite3changeset_start_strm
// sqlite3changeset_concat_strm

/// Changeset or Patchset
pub struct Changeset {
    cs: *mut c_void,
    n: c_int,
}

impl Changeset {
    /// Apply a changeset to a database
    pub fn apply(&self) -> Result<()> {
        // https://sqlite.org/session/sqlite3changeset_apply.html
        /*unsafe {
            check!(ffi::sqlite3changeset_apply());
        }*/
        Ok(())
    }

    /// Invert a changeset
    pub fn invert(&self) -> Result<Changeset> {
        unsafe {
            let mut cs: *mut c_void = mem::uninitialized();
            let mut n = 0;
            check!(ffi::sqlite3changeset_invert(
                self.n, self.cs, &mut n, &mut cs
            ));
            Ok(Changeset { cs, n })
        }
    }

    /// Create an iterator to traverse a changeset
    pub fn iter(&self) -> Result<ChangesetIter> {
        unsafe {
            let mut it: *mut ffi::sqlite3_changeset_iter = mem::uninitialized();
            check!(ffi::sqlite3changeset_start(&mut it, self.n, self.cs));
            Ok(ChangesetIter { it })
        }
    }

    /// Concatenate two changeset objects
    pub fn concat(a: &Changeset, b: &Changeset) -> Result<Changeset> {
        unsafe {
            let mut cs: *mut c_void = mem::uninitialized();
            let mut n = 0;
            check!(ffi::sqlite3changeset_concat(
                a.n, a.cs, b.n, b.cs, &mut n, &mut cs
            ));
            Ok(Changeset { cs, n })
        }
    }
}

impl Drop for Changeset {
    fn drop(&mut self) {
        unsafe {
            ffi::sqlite3_free(self.cs);
        }
    }
}

/// Cursor for iterating over the elements of a changeset or patchset.
pub struct ChangesetIter {
    it: *mut ffi::sqlite3_changeset_iter,
}

impl ChangesetIter {
    // This function should only be used with iterator objects passed to a
    // conflict-handler callback by sqlite3changeset_apply() with either
    // SQLITE_CHANGESET_DATA or SQLITE_CHANGESET_CONFLICT
    //sqlite3changeset_conflict

    // This function may only be called with an iterator passed to an
    // SQLITE_CHANGESET_FOREIGN_KEY conflict handler callback.
    //sqlite3changeset_fk_conflicts

    // The pIter argument passed to this function may either be an iterator passed
    // to a conflict-handler by sqlite3changeset_apply(), or an iterator created
    // by sqlite3changeset_start(). In the latter case, the most recent call to
    // sqlite3changeset_next() must have returned SQLITE_ROW. Furthermore, it
    // may only be called if the type of change that the iterator currently points
    // to is either SQLITE_UPDATE or SQLITE_INSERT. Otherwise, this function
    // returns SQLITE_MISUSE and sets *ppValue to NULL.
    //sqlite3changeset_new

    pub fn next(&mut self) -> Result<()> {
        unsafe {
            check!(ffi::sqlite3changeset_next(self.it));
        }
        // TODO Validate: ()
        Ok(())
    }
    //

    // The pIter argument passed to this function may either be an iterator passed
    // to a conflict-handler by sqlite3changeset_apply(), or an iterator created
    // by sqlite3changeset_start(). In the latter case, the most recent call to
    // sqlite3changeset_next() must have returned SQLITE_ROW. Furthermore, it
    // may only be called if the type of change that the iterator currently points
    // to is either SQLITE_DELETE or SQLITE_UPDATE. Otherwise, this function
    // returns SQLITE_MISUSE and sets *ppValue to NULL.
    //sqlite3changeset_old

    // The pIter argument passed to this function may either be an iterator passed
    // to a conflict-handler by sqlite3changeset_apply(), or an iterator created
    // by sqlite3changeset_start(). In the latter case, the most recent call to
    // sqlite3changeset_next() must have returned SQLITE_ROW. If this is not the
    // case, this function returns SQLITE_MISUSE.
    //sqlite3changeset_op

    //
    //sqlite3changeset_pk
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

// https://sqlite.org/session/changegroup.html
impl Changegroup {
    pub fn new() -> Result<Self> {
        unsafe {
            let mut cg: *mut ffi::sqlite3_changegroup = mem::uninitialized();
            check!(ffi::sqlite3changegroup_new(&mut cg));
            Ok(Changegroup { cg })
        }
    }

    /// Add a changeset
    pub fn add(&mut self, cs: &Changeset) -> Result<()> {
        unsafe {
            check!(ffi::sqlite3changegroup_add(self.cg, cs.n, cs.cs));
        }
        Ok(())
    }

    // sqlite3changegroup_add_strm

    /// Obtain a composite Changeset
    pub fn output(&mut self) -> Result<Changeset> {
        unsafe {
            let mut output: *mut c_void = mem::uninitialized();
            let mut n = 0;
            check!(ffi::sqlite3changegroup_output(self.cg, &mut n, &mut output));
            Ok(Changeset { cs: output, n })
        }
    }
    // sqlite3changegroup_output_strm
}

impl Drop for Changegroup {
    fn drop(&mut self) {
        unsafe {
            ffi::sqlite3changegroup_delete(self.cg);
        }
    }
}

/// Constants passed to the conflict handler
#[derive(Debug, PartialEq)]
pub enum ConflictType {
    SQLITE_CHANGESET_DATA = ffi::SQLITE_CHANGESET_DATA as isize,
    SQLITE_CHANGESET_NOTFOUND = ffi::SQLITE_CHANGESET_NOTFOUND as isize,
    SQLITE_CHANGESET_CONFLICT = ffi::SQLITE_CHANGESET_CONFLICT as isize,
    SQLITE_CHANGESET_CONSTRAINT = ffi::SQLITE_CHANGESET_CONSTRAINT as isize,
    SQLITE_CHANGESET_FOREIGN_KEY = ffi::SQLITE_CHANGESET_FOREIGN_KEY as isize,
}

/// Constants returned by the conflict handler
#[derive(Debug, PartialEq)]
pub enum ConflictAction {
    SQLITE_CHANGESET_OMIT = ffi::SQLITE_CHANGESET_OMIT as isize,
    SQLITE_CHANGESET_REPLACE = ffi::SQLITE_CHANGESET_REPLACE as isize,
    SQLITE_CHANGESET_ABORT = ffi::SQLITE_CHANGESET_ABORT as isize,
}
