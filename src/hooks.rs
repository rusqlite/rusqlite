//! `feature = "hooks"` Commit, Data Change and Rollback Notification Callbacks
#![allow(non_camel_case_types)]

use std::os::raw::{c_char, c_int, c_void};
use std::panic::{catch_unwind, RefUnwindSafe};
use std::ptr;

use crate::ffi;

use crate::{Connection, InnerConnection};

/// `feature = "hooks"` Action Codes
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(i32)]
#[non_exhaustive]
#[allow(clippy::upper_case_acronyms)]
pub enum Action {
    /// Unsupported / unexpected action
    UNKNOWN = -1,
    /// DELETE command
    SQLITE_DELETE = ffi::SQLITE_DELETE,
    /// INSERT command
    SQLITE_INSERT = ffi::SQLITE_INSERT,
    /// UPDATE command
    SQLITE_UPDATE = ffi::SQLITE_UPDATE,
}

impl From<i32> for Action {
    #[inline]
    fn from(code: i32) -> Action {
        match code {
            ffi::SQLITE_DELETE => Action::SQLITE_DELETE,
            ffi::SQLITE_INSERT => Action::SQLITE_INSERT,
            ffi::SQLITE_UPDATE => Action::SQLITE_UPDATE,
            _ => Action::UNKNOWN,
        }
    }
}

/// `feature = "hooks"` The context recieved by an authorizer hook.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuthContext<'c> {
    // The action to be authorized.
    pub action: AuthAction<'c>,

    /// The database name, if applicable.
    pub database_name: Option<&'c str>,

    // The inner-most trigger or view responsible for the access attempt.
    pub accessor: Option<&'c str>,
}

/// `feature = "hooks"` Actions and arguments
/// found within a statement during preparation.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[allow(missing_docs)] // This is self-documenting.
pub enum AuthAction<'c> {
    /// This variant is not normally produced by SQLite. You may encounter it
    // if you're using a different version than what's supported by this library.
    Unknown {
        /// The unknown authorization action code.
        code: i32,
        /// The third arg to the authorizer callback.
        arg1: Option<&'c str>,
        /// The fourth arg to the authorizer callback.
        arg2: Option<&'c str>,
    },
    CreateIndex {
        index_name: &'c str,
        table_name: &'c str,
    },
    CreateTable {
        table_name: &'c str,
    },
    CreateTempIndex {
        index_name: &'c str,
        table_name: &'c str,
    },
    CreateTempTable {
        table_name: &'c str,
    },
    CreateTempTrigger {
        trigger_name: &'c str,
        table_name: &'c str,
    },
    CreateTempView {
        view_name: &'c str,
    },
    CreateTrigger {
        trigger_name: &'c str,
        table_name: &'c str,
    },
    CreateView {
        view_name: &'c str,
    },
    Delete {
        table_name: &'c str,
    },
    DropIndex {
        index_name: &'c str,
        table_name: &'c str,
    },
    DropTable {
        table_name: &'c str,
    },
    DropTempIndex {
        index_name: &'c str,
        table_name: &'c str,
    },
    DropTempTable {
        table_name: &'c str,
    },
    DropTempTrigger {
        trigger_name: &'c str,
        table_name: &'c str,
    },
    DropTempView {
        view_name: &'c str,
    },
    DropTrigger {
        trigger_name: &'c str,
        table_name: &'c str,
    },
    DropView {
        view_name: &'c str,
    },
    Insert {
        table_name: &'c str,
    },
    Pragma {
        pragma_name: &'c str,
        /// The pragma value, if present (e.g., `PRAGMA name = value;`).
        pragma_value: Option<&'c str>,
    },
    Read {
        table_name: &'c str,
        column_name: &'c str,
    },
    Select,
    Transaction {
        operation: TransactionOperation,
    },
    Update {
        table_name: &'c str,
        column_name: &'c str,
    },
    Attach {
        filename: &'c std::path::Path,
    },
    Detach {
        database_name: &'c str,
    },
    AlterTable {
        database_name: &'c str,
        table_name: &'c str,
    },
    Reindex {
        index_name: &'c str,
    },
    Analyze {
        table_name: &'c str,
    },
    CreateVtable {
        table_name: &'c str,
        module_name: &'c str,
    },
    DropVtable {
        table_name: &'c str,
        module_name: &'c str,
    },
    Function {
        function_name: &'c str,
    },
    Savepoint {
        operation: TransactionOperation,
        savepoint_name: &'c str,
    },
    #[cfg(feature = "modern_sqlite")]
    Recursive,
}

impl<'c> AuthAction<'c> {
    fn from_raw(code: i32, arg1: Option<&'c str>, arg2: Option<&'c str>) -> Self {
        match (code, arg1, arg2) {
            (ffi::SQLITE_CREATE_INDEX, Some(index_name), Some(table_name)) => Self::CreateIndex {
                index_name,
                table_name,
            },
            (ffi::SQLITE_CREATE_TABLE, Some(table_name), _) => Self::CreateTable { table_name },
            (ffi::SQLITE_CREATE_TEMP_INDEX, Some(index_name), Some(table_name)) => {
                Self::CreateTempIndex {
                    index_name,
                    table_name,
                }
            }
            (ffi::SQLITE_CREATE_TEMP_TABLE, Some(table_name), _) => {
                Self::CreateTempTable { table_name }
            }
            (ffi::SQLITE_CREATE_TEMP_TRIGGER, Some(trigger_name), Some(table_name)) => {
                Self::CreateTempTrigger {
                    trigger_name,
                    table_name,
                }
            }
            (ffi::SQLITE_CREATE_TEMP_VIEW, Some(view_name), _) => {
                Self::CreateTempView { view_name }
            }
            (ffi::SQLITE_CREATE_TRIGGER, Some(trigger_name), Some(table_name)) => {
                Self::CreateTrigger {
                    trigger_name,
                    table_name,
                }
            }
            (ffi::SQLITE_CREATE_VIEW, Some(view_name), _) => Self::CreateView { view_name },
            (ffi::SQLITE_DELETE, Some(table_name), None) => Self::Delete { table_name },
            (ffi::SQLITE_DROP_INDEX, Some(index_name), Some(table_name)) => Self::DropIndex {
                index_name,
                table_name,
            },
            (ffi::SQLITE_DROP_TABLE, Some(table_name), _) => Self::DropTable { table_name },
            (ffi::SQLITE_DROP_TEMP_INDEX, Some(index_name), Some(table_name)) => {
                Self::DropTempIndex {
                    index_name,
                    table_name,
                }
            }
            (ffi::SQLITE_DROP_TEMP_TABLE, Some(table_name), _) => {
                Self::DropTempTable { table_name }
            }
            (ffi::SQLITE_DROP_TEMP_TRIGGER, Some(trigger_name), Some(table_name)) => {
                Self::DropTempTrigger {
                    trigger_name,
                    table_name,
                }
            }
            (ffi::SQLITE_DROP_TEMP_VIEW, Some(view_name), _) => Self::DropTempView { view_name },
            (ffi::SQLITE_DROP_TRIGGER, Some(trigger_name), Some(table_name)) => Self::DropTrigger {
                trigger_name,
                table_name,
            },
            (ffi::SQLITE_DROP_VIEW, Some(view_name), _) => Self::DropView { view_name },
            (ffi::SQLITE_INSERT, Some(table_name), _) => Self::Insert { table_name },
            (ffi::SQLITE_PRAGMA, Some(pragma_name), pragma_value) => Self::Pragma {
                pragma_name,
                pragma_value,
            },
            (ffi::SQLITE_READ, Some(table_name), Some(column_name)) => Self::Read {
                table_name,
                column_name,
            },
            (ffi::SQLITE_SELECT, _, _) => Self::Select,
            (ffi::SQLITE_TRANSACTION, Some(operation_str), _) => Self::Transaction {
                operation: TransactionOperation::from_str(operation_str),
            },
            (ffi::SQLITE_UPDATE, Some(table_name), Some(column_name)) => Self::Update {
                table_name,
                column_name,
            },
            (ffi::SQLITE_ATTACH, Some(filename_str), _) => Self::Attach {
                filename: std::path::Path::new(filename_str),
            },
            (ffi::SQLITE_DETACH, Some(database_name), _) => Self::Detach { database_name },
            (ffi::SQLITE_ALTER_TABLE, Some(database_name), Some(table_name)) => Self::AlterTable {
                database_name,
                table_name,
            },
            (ffi::SQLITE_REINDEX, Some(index_name), _) => Self::Reindex { index_name },
            (ffi::SQLITE_ANALYZE, Some(table_name), _) => Self::Analyze { table_name },
            (ffi::SQLITE_CREATE_VTABLE, Some(table_name), Some(module_name)) => {
                Self::CreateVtable {
                    table_name,
                    module_name,
                }
            }
            (ffi::SQLITE_DROP_VTABLE, Some(table_name), Some(module_name)) => Self::DropVtable {
                table_name,
                module_name,
            },
            (ffi::SQLITE_FUNCTION, _, Some(function_name)) => Self::Function { function_name },
            (ffi::SQLITE_SAVEPOINT, Some(operation_str), Some(savepoint_name)) => Self::Savepoint {
                operation: TransactionOperation::from_str(operation_str),
                savepoint_name,
            },
            #[cfg(feature = "modern_sqlite")]
            (ffi::SQLITE_RECURSIVE, _, _) => Self::Recursive,
            (code, arg1, arg2) => Self::Unknown { code, arg1, arg2 },
        }
    }
}

pub(crate) type BoxedAuthorizer =
    Box<dyn for<'c> FnMut(AuthContext<'c>) -> Authorization + Send + 'static>;

/// `feature = "hooks"` A transaction operation.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum TransactionOperation {
    Unknown,
    Begin,
    Release,
    Rollback,
}

impl TransactionOperation {
    fn from_str(op_str: &str) -> Self {
        match op_str {
            "BEGIN" => Self::Begin,
            "RELEASE" => Self::Release,
            "ROLLBACK" => Self::Rollback,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Authorization {
    /// Authorize the action.
    Allow,
    /// Don't allow access, but don't trigger an error either.
    Ignore,
    /// Trigger an error.
    Deny,
}

impl Authorization {
    fn into_raw(self) -> c_int {
        match self {
            Self::Allow => ffi::SQLITE_OK,
            Self::Ignore => ffi::SQLITE_IGNORE,
            Self::Deny => ffi::SQLITE_DENY,
        }
    }
}

impl Connection {
    /// `feature = "hooks"` Register a callback function to be invoked whenever
    /// a transaction is committed.
    ///
    /// The callback returns `true` to rollback.
    #[inline]
    pub fn commit_hook<'c, F>(&'c self, hook: Option<F>)
    where
        F: FnMut() -> bool + Send + 'c,
    {
        self.db.borrow_mut().commit_hook(hook);
    }

    /// `feature = "hooks"` Register a callback function to be invoked whenever
    /// a transaction is committed.
    ///
    /// The callback returns `true` to rollback.
    #[inline]
    pub fn rollback_hook<'c, F>(&'c self, hook: Option<F>)
    where
        F: FnMut() + Send + 'c,
    {
        self.db.borrow_mut().rollback_hook(hook);
    }

    /// `feature = "hooks"` Register a callback function to be invoked whenever
    /// a row is updated, inserted or deleted in a rowid table.
    ///
    /// The callback parameters are:
    ///
    /// - the type of database update (SQLITE_INSERT, SQLITE_UPDATE or
    /// SQLITE_DELETE),
    /// - the name of the database ("main", "temp", ...),
    /// - the name of the table that is updated,
    /// - the ROWID of the row that is updated.
    #[inline]
    pub fn update_hook<'c, F>(&'c self, hook: Option<F>)
    where
        F: FnMut(Action, &str, &str, i64) + Send + 'c,
    {
        self.db.borrow_mut().update_hook(hook);
    }

    /// `feature = "hooks"` Register a query progress callback.
    ///
    /// The parameter `num_ops` is the approximate number of virtual machine
    /// instructions that are evaluated between successive invocations of the
    /// `handler`. If `num_ops` is less than one then the progress handler
    /// is disabled.
    ///
    /// If the progress callback returns `true`, the operation is interrupted.
    pub fn progress_handler<F>(&self, num_ops: c_int, handler: Option<F>)
    where
        F: FnMut() -> bool + Send + RefUnwindSafe + 'static,
    {
        self.db.borrow_mut().progress_handler(num_ops, handler);
    }

    /// `feature = "hooks"` Register an authorizer callback that's invoked
    /// as a statement is being prepared.
    #[inline]
    pub fn authorizer<'c, F>(&self, hook: Option<F>) -> crate::Result<()>
    where
        F: for<'r> FnMut(AuthContext<'r>) -> Authorization + Send + RefUnwindSafe + 'static,
    {
        self.db.borrow_mut().authorizer(hook)
    }
}

impl InnerConnection {
    #[inline]
    pub fn remove_hooks(&mut self) {
        self.update_hook(None::<fn(Action, &str, &str, i64)>);
        self.commit_hook(None::<fn() -> bool>);
        self.rollback_hook(None::<fn()>);
        self.progress_handler(0, None::<fn() -> bool>);
    }

    fn commit_hook<'c, F>(&'c mut self, hook: Option<F>)
    where
        F: FnMut() -> bool + Send + 'c,
    {
        unsafe extern "C" fn call_boxed_closure<F>(p_arg: *mut c_void) -> c_int
        where
            F: FnMut() -> bool,
        {
            let r = catch_unwind(|| {
                let boxed_hook: *mut F = p_arg as *mut F;
                (*boxed_hook)()
            });
            if let Ok(true) = r {
                1
            } else {
                0
            }
        }

        // unlike `sqlite3_create_function_v2`, we cannot specify a `xDestroy` with
        // `sqlite3_commit_hook`. so we keep the `xDestroy` function in
        // `InnerConnection.free_boxed_hook`.
        let free_commit_hook = if hook.is_some() {
            Some(free_boxed_hook::<F> as unsafe fn(*mut c_void))
        } else {
            None
        };

        let previous_hook = match hook {
            Some(hook) => {
                let boxed_hook: *mut F = Box::into_raw(Box::new(hook));
                unsafe {
                    ffi::sqlite3_commit_hook(
                        self.db(),
                        Some(call_boxed_closure::<F>),
                        boxed_hook as *mut _,
                    )
                }
            }
            _ => unsafe { ffi::sqlite3_commit_hook(self.db(), None, ptr::null_mut()) },
        };
        if !previous_hook.is_null() {
            if let Some(free_boxed_hook) = self.free_commit_hook {
                unsafe { free_boxed_hook(previous_hook) };
            }
        }
        self.free_commit_hook = free_commit_hook;
    }

    fn rollback_hook<'c, F>(&'c mut self, hook: Option<F>)
    where
        F: FnMut() + Send + 'c,
    {
        unsafe extern "C" fn call_boxed_closure<F>(p_arg: *mut c_void)
        where
            F: FnMut(),
        {
            let _ = catch_unwind(|| {
                let boxed_hook: *mut F = p_arg as *mut F;
                (*boxed_hook)();
            });
        }

        let free_rollback_hook = if hook.is_some() {
            Some(free_boxed_hook::<F> as unsafe fn(*mut c_void))
        } else {
            None
        };

        let previous_hook = match hook {
            Some(hook) => {
                let boxed_hook: *mut F = Box::into_raw(Box::new(hook));
                unsafe {
                    ffi::sqlite3_rollback_hook(
                        self.db(),
                        Some(call_boxed_closure::<F>),
                        boxed_hook as *mut _,
                    )
                }
            }
            _ => unsafe { ffi::sqlite3_rollback_hook(self.db(), None, ptr::null_mut()) },
        };
        if !previous_hook.is_null() {
            if let Some(free_boxed_hook) = self.free_rollback_hook {
                unsafe { free_boxed_hook(previous_hook) };
            }
        }
        self.free_rollback_hook = free_rollback_hook;
    }

    fn update_hook<'c, F>(&'c mut self, hook: Option<F>)
    where
        F: FnMut(Action, &str, &str, i64) + Send + 'c,
    {
        unsafe extern "C" fn call_boxed_closure<F>(
            p_arg: *mut c_void,
            action_code: c_int,
            db_str: *const c_char,
            tbl_str: *const c_char,
            row_id: i64,
        ) where
            F: FnMut(Action, &str, &str, i64),
        {
            use std::ffi::CStr;
            use std::str;

            let action = Action::from(action_code);
            let db_name = {
                let c_slice = CStr::from_ptr(db_str).to_bytes();
                str::from_utf8(c_slice)
            };
            let tbl_name = {
                let c_slice = CStr::from_ptr(tbl_str).to_bytes();
                str::from_utf8(c_slice)
            };

            let _ = catch_unwind(|| {
                let boxed_hook: *mut F = p_arg as *mut F;
                (*boxed_hook)(
                    action,
                    db_name.expect("illegal db name"),
                    tbl_name.expect("illegal table name"),
                    row_id,
                );
            });
        }

        let free_update_hook = if hook.is_some() {
            Some(free_boxed_hook::<F> as unsafe fn(*mut c_void))
        } else {
            None
        };

        let previous_hook = match hook {
            Some(hook) => {
                let boxed_hook: *mut F = Box::into_raw(Box::new(hook));
                unsafe {
                    ffi::sqlite3_update_hook(
                        self.db(),
                        Some(call_boxed_closure::<F>),
                        boxed_hook as *mut _,
                    )
                }
            }
            _ => unsafe { ffi::sqlite3_update_hook(self.db(), None, ptr::null_mut()) },
        };
        if !previous_hook.is_null() {
            if let Some(free_boxed_hook) = self.free_update_hook {
                unsafe { free_boxed_hook(previous_hook) };
            }
        }
        self.free_update_hook = free_update_hook;
    }

    fn progress_handler<F>(&mut self, num_ops: c_int, handler: Option<F>)
    where
        F: FnMut() -> bool + Send + RefUnwindSafe + 'static,
    {
        unsafe extern "C" fn call_boxed_closure<F>(p_arg: *mut c_void) -> c_int
        where
            F: FnMut() -> bool,
        {
            let r = catch_unwind(|| {
                let boxed_handler: *mut F = p_arg as *mut F;
                (*boxed_handler)()
            });
            if let Ok(true) = r {
                1
            } else {
                0
            }
        }

        match handler {
            Some(handler) => {
                let boxed_handler = Box::new(handler);
                unsafe {
                    ffi::sqlite3_progress_handler(
                        self.db(),
                        num_ops,
                        Some(call_boxed_closure::<F>),
                        &*boxed_handler as *const F as *mut _,
                    )
                }
                self.progress_handler = Some(boxed_handler);
            }
            _ => {
                unsafe { ffi::sqlite3_progress_handler(self.db(), num_ops, None, ptr::null_mut()) }
                self.progress_handler = None;
            }
        };
    }

    pub fn authorizer<'c, F>(&'c mut self, authorizer: Option<F>) -> crate::Result<()>
    where
        F: for<'r> FnMut(AuthContext<'r>) -> Authorization + Send + RefUnwindSafe + 'static,
    {
        unsafe extern "C" fn call_boxed_closure<'c, F>(
            p_arg: *mut c_void,
            action_code: c_int,
            action_arg1_str: *const c_char,
            action_arg2_str: *const c_char,
            db_str: *const c_char,
            accessor_str: *const c_char,
        ) -> c_int
        where
            F: FnMut(AuthContext<'c>) -> Authorization + Send + 'static,
        {
            use std::ffi::CStr;
            use std::str;

            let optional_str = |p_str: *const c_char| {
                if p_str.is_null() {
                    None
                } else {
                    let c_slice = CStr::from_ptr(p_str).to_bytes();
                    str::from_utf8(c_slice).ok()
                }
            };
            let action = AuthAction::from_raw(
                action_code,
                optional_str(action_arg1_str),
                optional_str(action_arg2_str),
            );
            let auth_ctx = AuthContext {
                action,
                database_name: optional_str(db_str),
                accessor: optional_str(accessor_str),
            };

            let r = catch_unwind(|| {
                let boxed_hook: *mut F = p_arg as *mut F;
                (*boxed_hook)(auth_ctx)
            });
            match r {
                Ok(auth) => auth.into_raw(),
                Err(_) => ffi::SQLITE_ERROR,
            }
        }

        let callback_fn = authorizer
            .as_ref()
            .map(|_| call_boxed_closure::<'c, F> as unsafe extern "C" fn(_, _, _, _, _, _) -> _);
        let boxed_authorizer = authorizer.map(Box::new);

        match unsafe {
            ffi::sqlite3_set_authorizer(
                self.db(),
                callback_fn,
                boxed_authorizer
                    .as_ref()
                    .map(|f| &**f as *const F as *mut _)
                    .unwrap_or_else(ptr::null_mut),
            )
        } {
            ffi::SQLITE_OK => {
                self.authorizer = boxed_authorizer.map(|ba| ba as BoxedAuthorizer);
                Ok(())
            }
            err_code => Err(unsafe { crate::error::error_from_handle(self.db(), err_code) }),
        }
    }
}

unsafe fn free_boxed_hook<F>(p: *mut c_void) {
    drop(Box::from_raw(p as *mut F));
}

#[cfg(test)]
mod test {
    use super::Action;
    use crate::{Connection, Result};
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn test_commit_hook() -> Result<()> {
        let db = Connection::open_in_memory()?;

        let mut called = false;
        db.commit_hook(Some(|| {
            called = true;
            false
        }));
        db.execute_batch("BEGIN; CREATE TABLE foo (t TEXT); COMMIT;")?;
        assert!(called);
        Ok(())
    }

    #[test]
    fn test_fn_commit_hook() -> Result<()> {
        let db = Connection::open_in_memory()?;

        fn hook() -> bool {
            true
        }

        db.commit_hook(Some(hook));
        db.execute_batch("BEGIN; CREATE TABLE foo (t TEXT); COMMIT;")
            .unwrap_err();
        Ok(())
    }

    #[test]
    fn test_rollback_hook() -> Result<()> {
        let db = Connection::open_in_memory()?;

        let mut called = false;
        db.rollback_hook(Some(|| {
            called = true;
        }));
        db.execute_batch("BEGIN; CREATE TABLE foo (t TEXT); ROLLBACK;")?;
        assert!(called);
        Ok(())
    }

    #[test]
    fn test_update_hook() -> Result<()> {
        let db = Connection::open_in_memory()?;

        let mut called = false;
        db.update_hook(Some(|action, db: &str, tbl: &str, row_id| {
            assert_eq!(Action::SQLITE_INSERT, action);
            assert_eq!("main", db);
            assert_eq!("foo", tbl);
            assert_eq!(1, row_id);
            called = true;
        }));
        db.execute_batch("CREATE TABLE foo (t TEXT)")?;
        db.execute_batch("INSERT INTO foo VALUES ('lisa')")?;
        assert!(called);
        Ok(())
    }

    #[test]
    fn test_progress_handler() -> Result<()> {
        let db = Connection::open_in_memory()?;

        static CALLED: AtomicBool = AtomicBool::new(false);
        db.progress_handler(
            1,
            Some(|| {
                CALLED.store(true, Ordering::Relaxed);
                false
            }),
        );
        db.execute_batch("BEGIN; CREATE TABLE foo (t TEXT); COMMIT;")?;
        assert!(CALLED.load(Ordering::Relaxed));
        Ok(())
    }

    #[test]
    fn test_progress_handler_interrupt() -> Result<()> {
        let db = Connection::open_in_memory()?;

        fn handler() -> bool {
            true
        }

        db.progress_handler(1, Some(handler));
        db.execute_batch("BEGIN; CREATE TABLE foo (t TEXT); COMMIT;")
            .unwrap_err();
        Ok(())
    }

    #[test]
    fn test_authorizer() -> Result<()> {
        use super::{AuthAction, AuthContext, Authorization};

        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE foo (public TEXT, private TEXT)")
            .unwrap();

        let authorizer = move |ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Read { column_name, .. } if column_name == "private" => {
                Authorization::Ignore
            }
            AuthAction::DropTable { .. } => Authorization::Deny,
            _ => Authorization::Allow,
        };

        db.authorizer(Some(authorizer)).unwrap();
        db.execute_batch(
            "BEGIN TRANSACTION; INSERT INTO foo VALUES ('pub txt', 'priv txt'); COMMIT;",
        )
        .unwrap();
        db.query_row_and_then("SELECT * FROM foo", [], |row| -> Result<()> {
            assert_eq!(row.get::<_, String>("public")?, "pub txt");
            assert!(row.get::<_, Option<String>>("private")?.is_none());
            Ok(())
        })
        .unwrap();
        db.execute_batch("DROP TABLE foo").unwrap_err();

        Ok(())
    }
}
