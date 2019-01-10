//! [Session Extension](https://sqlite.org/sessionintro.html)
#![allow(non_camel_case_types)]

use std::ffi::CStr;
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::mem;
use std::os::raw::{c_char, c_int, c_uchar, c_void};
use std::panic::{catch_unwind, RefUnwindSafe};
use std::ptr;
use std::slice::{from_raw_parts, from_raw_parts_mut};

use fallible_streaming_iterator::FallibleStreamingIterator;

use crate::error::error_from_sqlite_code;
use crate::ffi;
use crate::hooks::Action;
use crate::{errmsg_to_string, str_to_cstring, Connection, DatabaseName, Result};

// https://sqlite.org/session.html

/// An instance of this object is a session that can be used to record changes
/// to a database.
pub struct Session<'conn> {
    phantom: PhantomData<&'conn ()>,
    s: *mut ffi::sqlite3_session,
    filter: Option<Box<dyn Fn(&str) -> bool>>,
}

impl<'conn> Session<'conn> {
    /// Create a new session object
    pub fn new(db: &'conn Connection) -> Result<Session<'conn>> {
        Session::new_with_name(db, DatabaseName::Main)
    }

    /// Create a new session object
    pub fn new_with_name(db: &'conn Connection, name: DatabaseName<'_>) -> Result<Session<'conn>> {
        let name = name.to_cstring()?;

        let db = db.db.borrow_mut().db;

        let mut s: *mut ffi::sqlite3_session = unsafe { mem::uninitialized() };
        check!(unsafe { ffi::sqlite3session_create(db, name.as_ptr(), &mut s) });

        Ok(Session {
            phantom: PhantomData,
            s,
            filter: None,
        })
    }

    /// Set a table filter
    pub fn table_filter<F>(&mut self, filter: Option<F>)
    where
        F: Fn(&str) -> bool + Send + RefUnwindSafe + 'static,
    {
        unsafe extern "C" fn call_boxed_closure<F>(
            p_arg: *mut c_void,
            tbl_str: *const c_char,
        ) -> c_int
        where
            F: Fn(&str) -> bool + RefUnwindSafe,
        {
            use std::ffi::CStr;
            use std::str;

            let boxed_filter: *mut F = p_arg as *mut F;
            let tbl_name = {
                let c_slice = CStr::from_ptr(tbl_str).to_bytes();
                str::from_utf8_unchecked(c_slice)
            };
            if let Ok(true) = catch_unwind(|| (*boxed_filter)(tbl_name)) {
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
            str_to_cstring(table)?.as_ptr()
        } else {
            ptr::null()
        };
        unsafe { check!(ffi::sqlite3session_attach(self.s, table)) };
        Ok(())
    }

    /// Generate a Changeset
    pub fn changeset(&mut self) -> Result<Changeset> {
        let mut n = 0;
        let mut cs: *mut c_void = unsafe { mem::uninitialized() };
        check!(unsafe { ffi::sqlite3session_changeset(self.s, &mut n, &mut cs) });
        Ok(Changeset { cs, n })
    }

    // sqlite3session_changeset_strm

    /// Generate a Patchset
    pub fn patchset(&mut self) -> Result<Changeset> {
        let mut n = 0;
        let mut ps: *mut c_void = unsafe { mem::uninitialized() };
        check!(unsafe { ffi::sqlite3session_patchset(self.s, &mut n, &mut ps) });
        // TODO Validate: same struct
        Ok(Changeset { cs: ps, n })
    }

    // sqlite3session_patchset_strm

    /// Load the difference between tables.
    pub fn diff(&mut self, from: DatabaseName<'_>, table: &str) -> Result<()> {
        let from = from.to_cstring()?;
        let table = str_to_cstring(table)?.as_ptr();
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

// sqlite3changeset_invert_strm
// sqlite3changeset_start_strm
// sqlite3changeset_concat_strm

/// Changeset or Patchset
pub struct Changeset {
    cs: *mut c_void,
    n: c_int,
}

impl Changeset {
    /// Invert a changeset
    pub fn invert(&self) -> Result<Changeset> {
        let mut n = 0;
        let mut cs: *mut c_void = unsafe { mem::uninitialized() };
        check!(unsafe { ffi::sqlite3changeset_invert(self.n, self.cs, &mut n, &mut cs) });
        Ok(Changeset { cs, n })
    }

    /// Create an iterator to traverse a changeset
    pub fn iter<'changeset>(&'changeset self) -> Result<ChangesetIter<'changeset>> {
        let mut it: *mut ffi::sqlite3_changeset_iter = unsafe { mem::uninitialized() };
        check!(unsafe { ffi::sqlite3changeset_start(&mut it, self.n, self.cs) });
        Ok(ChangesetIter {
            phantom: PhantomData,
            it,
            item: None,
        })
    }

    /// Concatenate two changeset objects
    pub fn concat(a: &Changeset, b: &Changeset) -> Result<Changeset> {
        let mut n = 0;
        let mut cs: *mut c_void = unsafe { mem::uninitialized() };
        check!(unsafe { ffi::sqlite3changeset_concat(a.n, a.cs, b.n, b.cs, &mut n, &mut cs) });
        Ok(Changeset { cs, n })
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
pub struct ChangesetIter<'changeset> {
    phantom: PhantomData<&'changeset ()>,
    it: *mut ffi::sqlite3_changeset_iter,
    item: Option<ChangesetItem>,
}

impl<'changeset> ChangesetIter<'changeset> {
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

    /// Advance a changeset iterator
    // https://sqlite.org/session/sqlite3changeset_next.html
    pub fn next(&mut self) -> Result<bool> {
        let rc = unsafe { ffi::sqlite3changeset_next(self.it) };
        match rc {
            ffi::SQLITE_ROW => Ok(true),
            ffi::SQLITE_DONE => Ok(false),
            code => Err(error_from_sqlite_code(code, None)),
        }
    }

    // The pIter argument passed to this function may either be an iterator passed
    // to a conflict-handler by sqlite3changeset_apply(), or an iterator created
    // by sqlite3changeset_start(). In the latter case, the most recent call to
    // sqlite3changeset_next() must have returned SQLITE_ROW. Furthermore, it
    // may only be called if the type of change that the iterator currently points
    // to is either SQLITE_DELETE or SQLITE_UPDATE. Otherwise, this function
    // returns SQLITE_MISUSE and sets *ppValue to NULL.
    //sqlite3changeset_old

    /// Obtain the current operation
    pub fn op(&self) -> Result<Operation> {
        let mut number_of_columns = 0;
        let mut code = 0;
        let mut indirect = 0;
        let tab = unsafe {
            let mut pz_tab: *const c_char = mem::uninitialized();
            check!(ffi::sqlite3changeset_op(
                self.it,
                &mut pz_tab,
                &mut number_of_columns,
                &mut code,
                &mut indirect
            ));
            CStr::from_ptr(pz_tab)
        };
        let table_name = tab.to_str()?.to_owned();
        Ok(Operation {
            table_name,
            number_of_columns,
            code: Action::from(code),
            indirect: indirect != 0,
        })
    }

    /// Obtain the primary key definition of a table
    pub fn pk(&self) -> Result<Vec<bool>> {
        let mut number_of_columns = 0;
        let pks = unsafe {
            let mut pks: *mut c_uchar = mem::uninitialized();
            check!(ffi::sqlite3changeset_pk(
                self.it,
                &mut pks,
                &mut number_of_columns
            ));
            from_raw_parts(pks, number_of_columns as usize)
        };
        Ok(pks.iter().map(|pk| *pk != 0).collect())
    }
}

impl<'changeset> FallibleStreamingIterator for ChangesetIter<'changeset> {
    type Error = crate::error::Error;
    type Item = ChangesetItem;

    fn advance(&mut self) -> Result<()> {
        if self.next()? {
            self.item = Some(ChangesetItem { it: self.it });
        } else {
            self.item = None;
        }
        Ok(())
    }

    fn get(&self) -> Option<&ChangesetItem> {
        self.item.as_ref()
    }
}

pub struct Operation {
    table_name: String,
    number_of_columns: i32,
    code: Action,
    indirect: bool,
}

impl Operation {
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    pub fn number_of_columns(&self) -> i32 {
        self.number_of_columns
    }

    pub fn code(&self) -> Action {
        self.code
    }

    pub fn indirect(&self) -> bool {
        self.indirect
    }
}

impl<'changeset> Drop for ChangesetIter<'changeset> {
    fn drop(&mut self) {
        unsafe {
            // This function should only be called on iterators created using the
            // sqlite3changeset_start() function.
            ffi::sqlite3changeset_finalize(self.it);
        }
    }
}

pub struct ChangesetItem {
    it: *mut ffi::sqlite3_changeset_iter,
}

/// Used to combine two or more changesets or
/// patchsets
pub struct Changegroup {
    cg: *mut ffi::sqlite3_changegroup,
}

// https://sqlite.org/session/changegroup.html
impl Changegroup {
    pub fn new() -> Result<Self> {
        let mut cg: *mut ffi::sqlite3_changegroup = unsafe { mem::uninitialized() };
        check!(unsafe { ffi::sqlite3changegroup_new(&mut cg) });
        Ok(Changegroup { cg })
    }

    /// Add a changeset
    pub fn add(&mut self, cs: &Changeset) -> Result<()> {
        check!(unsafe { ffi::sqlite3changegroup_add(self.cg, cs.n, cs.cs) });
        Ok(())
    }

    // sqlite3changegroup_add_strm

    /// Obtain a composite Changeset
    pub fn output(&mut self) -> Result<Changeset> {
        let mut n = 0;
        let mut output: *mut c_void = unsafe { mem::uninitialized() };
        check!(unsafe { ffi::sqlite3changegroup_output(self.cg, &mut n, &mut output) });
        Ok(Changeset { cs: output, n })
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

impl Connection {
    /// Apply a changeset to a database
    pub fn apply<F, C>(&self, cs: &Changeset, filter: Option<F>, conflict: C) -> Result<()>
    where
        F: Fn(&str) -> bool + Send + RefUnwindSafe + 'static,
        C: Fn(ConflictType, ChangesetItem) -> ConflictAction + Send + RefUnwindSafe + 'static,
    {
        let db = self.db.borrow_mut().db;

        unsafe extern "C" fn call_filter<F, C>(p_ctx: *mut c_void, tbl_str: *const c_char) -> c_int
        where
            F: Fn(&str) -> bool + Send + RefUnwindSafe + 'static,
            C: Fn(ConflictType, ChangesetItem) -> ConflictAction + Send + RefUnwindSafe + 'static,
        {
            use std::ffi::CStr;
            use std::str;

            let tuple: *mut (Option<F>, C) = p_ctx as *mut (Option<F>, C);
            let tbl_name = {
                let c_slice = CStr::from_ptr(tbl_str).to_bytes();
                str::from_utf8_unchecked(c_slice)
            };
            match *tuple {
                (Some(ref filter), _) => {
                    if let Ok(true) = catch_unwind(|| filter(tbl_name)) {
                        1
                    } else {
                        0
                    }
                }
                _ => unimplemented!(),
            }
        }

        unsafe extern "C" fn call_conflict<F, C>(
            p_ctx: *mut c_void,
            e_conflict: c_int,
            p: *mut ffi::sqlite3_changeset_iter,
        ) -> c_int
        where
            F: Fn(&str) -> bool + Send + RefUnwindSafe + 'static,
            C: Fn(ConflictType, ChangesetItem) -> ConflictAction + Send + RefUnwindSafe + 'static,
        {
            let tuple: *mut (Option<F>, C) = p_ctx as *mut (Option<F>, C);
            let conflict_type = ConflictType::from(e_conflict);
            let item = ChangesetItem { it: p };
            if let Ok(action) = catch_unwind(|| (*tuple).1(conflict_type, item)) {
                action as c_int
            } else {
                ffi::SQLITE_CHANGESET_ABORT
            }
        }

        let filtered = filter.is_some();
        let tuple = &mut (filter, conflict);
        check!(unsafe {
            if filtered {
                ffi::sqlite3changeset_apply(
                    db,
                    cs.n,
                    cs.cs,
                    Some(call_filter::<F, C>),
                    Some(call_conflict::<F, C>),
                    tuple as *mut (Option<F>, C) as *mut c_void,
                )
            } else {
                ffi::sqlite3changeset_apply(
                    db,
                    cs.n,
                    cs.cs,
                    None,
                    Some(call_conflict::<F, C>),
                    tuple as *mut (Option<F>, C) as *mut c_void,
                )
            }
        });
        Ok(())
    }

    // sqlite3changeset_apply_strm
}

/// Constants passed to the conflict handler
#[derive(Debug, PartialEq)]
pub enum ConflictType {
    UNKNOWN = -1,
    SQLITE_CHANGESET_DATA = ffi::SQLITE_CHANGESET_DATA as isize,
    SQLITE_CHANGESET_NOTFOUND = ffi::SQLITE_CHANGESET_NOTFOUND as isize,
    SQLITE_CHANGESET_CONFLICT = ffi::SQLITE_CHANGESET_CONFLICT as isize,
    SQLITE_CHANGESET_CONSTRAINT = ffi::SQLITE_CHANGESET_CONSTRAINT as isize,
    SQLITE_CHANGESET_FOREIGN_KEY = ffi::SQLITE_CHANGESET_FOREIGN_KEY as isize,
}
impl From<i32> for ConflictType {
    fn from(code: i32) -> ConflictType {
        match code {
            ffi::SQLITE_CHANGESET_DATA => ConflictType::SQLITE_CHANGESET_DATA,
            ffi::SQLITE_CHANGESET_NOTFOUND => ConflictType::SQLITE_CHANGESET_NOTFOUND,
            ffi::SQLITE_CHANGESET_CONFLICT => ConflictType::SQLITE_CHANGESET_CONFLICT,
            ffi::SQLITE_CHANGESET_CONSTRAINT => ConflictType::SQLITE_CHANGESET_CONSTRAINT,
            ffi::SQLITE_CHANGESET_FOREIGN_KEY => ConflictType::SQLITE_CHANGESET_FOREIGN_KEY,
            _ => ConflictType::UNKNOWN,
        }
    }
}

/// Constants returned by the conflict handler
#[derive(Debug, PartialEq)]
pub enum ConflictAction {
    SQLITE_CHANGESET_OMIT = ffi::SQLITE_CHANGESET_OMIT as isize,
    SQLITE_CHANGESET_REPLACE = ffi::SQLITE_CHANGESET_REPLACE as isize,
    SQLITE_CHANGESET_ABORT = ffi::SQLITE_CHANGESET_ABORT as isize,
}

unsafe extern "C" fn x_input(p_in: *mut c_void, data: *mut c_void, len: *mut c_int) -> c_int {
    if p_in.is_null() {
        return ffi::SQLITE_MISUSE;
    }
    let bytes: &mut [u8] = from_raw_parts_mut(data as *mut u8, len as usize);
    //let reader: &mut Read = &mut *p_in;
    let reader: &mut Read = &mut std::io::stdin(); // FIXME
    match reader.read(bytes) {
        Ok(n) => {
            *len = n as i32; // TODO Validate: n = 0 may not mean the reader will always no longer be able to
                             // produce bytes.
            ffi::SQLITE_OK
        }
        Err(_) => ffi::SQLITE_IOERR_READ, // TODO check if err is a (ru)sqlite Error => propagate
    }
}

// The sessions module never invokes an xOutput callback with the third
// parameter set to a value less than or equal to zero.
unsafe extern "C" fn x_output(p_out: *mut c_void, data: *const c_void, len: c_int) -> c_int {
    if p_out.is_null() {
        return ffi::SQLITE_MISUSE;
    }
    let bytes: &[u8] = from_raw_parts(data as *const u8, len as usize);
    //let writer: &mut Write = &mut *p_out;
    let writer: &mut Write = &mut std::io::stdout(); // FIXME
    match writer.write_all(bytes) {
        Ok(_) => ffi::SQLITE_OK,
        Err(_) => ffi::SQLITE_IOERR_WRITE, // TODO check if err is a (ru)sqlite Error => propagate
    }
}

#[cfg(test)]
mod test {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::{ConflictAction, Session};
    use crate::Connection;

    #[test]
    fn test_changeset() {
        let changeset = {
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch("CREATE TABLE foo(t TEXT PRIMARY KEY NOT NULL);")
                .unwrap();

            let mut session = Session::new(&db).unwrap();
            assert!(session.is_empty());

            session.attach(None).unwrap();
            db.execute("INSERT INTO foo (t) VALUES (?);", &["bar"])
                .unwrap();

            session.changeset().unwrap()
        };
        let mut iter = changeset.iter().unwrap();
        assert_eq!(Ok(true), iter.next());
    }

    #[test]
    fn test_changeset_apply() {
        let changeset = {
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch("CREATE TABLE foo(t TEXT PRIMARY KEY NOT NULL);")
                .unwrap();

            let mut session = Session::new(&db).unwrap();
            assert!(session.is_empty());

            session.attach(None).unwrap();
            db.execute("INSERT INTO foo (t) VALUES (?);", &["bar"])
                .unwrap();

            session.changeset().unwrap()
        };

        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE foo(t TEXT PRIMARY KEY NOT NULL);")
            .unwrap();

        lazy_static! {
            static ref called: AtomicBool = AtomicBool::new(false);
        }
        db.apply(
            &changeset,
            None::<fn(&str) -> bool>,
            |_conflict_type, _item| {
                called.store(true, Ordering::Relaxed);
                ConflictAction::SQLITE_CHANGESET_OMIT
            },
        )
        .unwrap();

        assert!(!called.load(Ordering::Relaxed));
        let check = db
            .query_row("SELECT 1 FROM foo WHERE t = ?", &["bar"], |row| row.get(0))
            .unwrap();
        assert_eq!(1, check);
    }

    #[test]
    fn test_session_empty() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE foo(t TEXT PRIMARY KEY NOT NULL);")
            .unwrap();

        let mut session = Session::new(&db).unwrap();
        assert!(session.is_empty());

        session.attach(None).unwrap();
        db.execute("INSERT INTO foo (t) VALUES (?);", &["bar"])
            .unwrap();

        assert!(!session.is_empty());
    }

    #[test]
    fn test_session_set_enabled() {
        let db = Connection::open_in_memory().unwrap();

        let mut session = Session::new(&db).unwrap();
        assert!(session.is_enabled());
        session.set_enabled(false);
        assert!(!session.is_enabled());
    }

    #[test]
    fn test_session_set_indirect() {
        let db = Connection::open_in_memory().unwrap();

        let mut session = Session::new(&db).unwrap();
        assert!(!session.is_indirect());
        session.set_indirect(true);
        assert!(session.is_indirect());
    }
}
