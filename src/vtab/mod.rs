//! Create virtual tables.
//!
//! Follow these steps to create your own virtual table:
//! 1. Write implementation of [`VTab`] and [`VTabCursor`] traits.
//! 2. Create an instance of the [`Module`] structure specialized for [`VTab`]
//!    impl. from step 1.
//! 3. Register your [`Module`] structure using [`Connection::create_module`].
//! 4. Run a `CREATE VIRTUAL TABLE` command that specifies the new module in the
//!    `USING` clause.
//!
//! (See [SQLite doc](http://sqlite.org/vtab.html))
//!
//! # Building a module
//!
//! Use [`Module::new()`] to create a base module, then chain `with_*` methods
//! to enable additional capabilities if needed. Each method is only available when your
//! virtual table type implements the corresponding trait.
//!
//! ```rust,ignore
//! use rusqlite::vtab::{Module, VTab, CreateVTab, VTabKind};
//!
//! // Eponymous-only read-only table (simplest case)
//! const SIMPLE: &Module<MyVTab> = &Module::new();
//!
//! // Read-only table with CREATE VIRTUAL TABLE support
//! const READ_ONLY: &Module<MyVTab> = &Module::new().with_create();
//!
//! // Writable table with transaction support
//! const WITH_TX: &Module<MyVTab> = &Module::new()
//!     .with_update()
//!     .with_transactions();
//!
//! // Table with rename support (for ALTER TABLE RENAME)
//! const RENAMEABLE: &Module<MyVTab> = &Module::new()
//!     .with_create()
//!     .with_rename();
//!
//! // Table with integrity checking (PRAGMA integrity_check support)
//! // Requires the `modern_sqlite` feature (SQLite >= 3.44.0)
//! #[cfg(feature = "modern_sqlite")]
//! const WITH_INTEGRITY: &Module<MyVTab> = &Module::new()
//!     .with_create()
//!     .with_integrity();
//! ```
//!
//! ## Available capabilities
//!
//! | Method | Trait Required | Description |
//! |--------|----------------|-------------|
//! | [`with_create()`](Module::with_create) | [`CreateVTab`] | Enable `CREATE VIRTUAL TABLE` support |
//! | [`with_update()`](Module::with_update) | [`UpdateVTab`] | Enable INSERT/UPDATE/DELETE |
//! | [`with_transactions()`](Module::with_transactions) | [`TransactionVTab`] | Enable transaction callbacks |
//! | [`with_savepoints()`](Module::with_savepoints) | [`SavepointVTab`] | Enable nested transactions |
//! | [`with_rename()`](Module::with_rename) | [`RenameVTab`] | Enable `ALTER TABLE RENAME` |
//! | [`with_find_function()`](Module::with_find_function) | [`FindFunctionVTab`] | Enable SQL function overloading |
//! | [`with_shadow_name()`](Module::with_shadow_name) | [`ShadowNameVTab`] | Identify shadow tables |
//! | [`with_integrity()`](Module::with_integrity) | [`IntegrityVTab`] | Enable `PRAGMA integrity_check` |
//!
//! ## Legacy module functions
//!
//! - [`eponymous_only_module()`] - Eponymous-only read-only table
//! - [`read_only_module()`] - Read-only table with CREATE support
//! - [`update_module()`] - Writable table
//! - [`update_module_with_tx()`] - Writable table with transactions
use std::borrow::Cow::{self, Borrowed, Owned};
use std::ffi::{c_char, c_int, c_void, CStr};
use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr;
use std::slice;

use libsqlite3_sys::sqlite3_free;

use crate::context::set_result;
use crate::error::{check, error_from_sqlite_code, to_sqlite_error};
use crate::ffi;
pub use crate::ffi::{sqlite3_vtab, sqlite3_vtab_cursor};
use crate::types::{FromSql, FromSqlError, ToSql, ValueRef};
use crate::util::{alloc, free_boxed_value};
use crate::{str_to_cstring, Connection, Error, InnerConnection, Name, Result};

// let conn: Connection = ...;
// let mod: Module = ...; // VTab builder
// conn.create_module("module", mod);
//
// conn.execute("CREATE VIRTUAL TABLE foo USING module(...)");
// \-> Module::xcreate
//  |-> let vtab: VTab = ...; // on the heap
//  \-> conn.declare_vtab("CREATE TABLE foo (...)");
// conn = Connection::open(...);
// \-> Module::xconnect
//  |-> let vtab: VTab = ...; // on the heap
//  \-> conn.declare_vtab("CREATE TABLE foo (...)");
//
// conn.close();
// \-> vtab.xdisconnect
// conn.execute("DROP TABLE foo");
// \-> vtab.xDestroy
//
// let stmt = conn.prepare("SELECT ... FROM foo WHERE ...");
// \-> vtab.xbestindex
// stmt.query().next();
// \-> vtab.xopen
//  |-> let cursor: VTabCursor = ...; // on the heap
//  |-> cursor.xfilter or xnext
//  |-> cursor.xeof
//  \-> if not eof { cursor.column or xrowid } else { cursor.xclose }
//

// db: *mut ffi::sqlite3 => VTabConnection
// module: *const ffi::sqlite3_module => Module
// aux: *mut c_void => Module::Aux
// ffi::sqlite3_vtab => VTab
// ffi::sqlite3_vtab_cursor => VTabCursor

/// Virtual table kind
pub enum VTabKind {
    /// Non-eponymous
    Default,
    /// [`create`](CreateVTab::create) == [`connect`](VTab::connect)
    ///
    /// See [SQLite doc](https://sqlite.org/vtab.html#eponymous_virtual_tables)
    Eponymous,
    /// No [`create`](CreateVTab::create) / [`destroy`](CreateVTab::destroy) or
    /// not used
    ///
    /// SQLite >= 3.9.0
    ///
    /// See [SQLite doc](https://sqlite.org/vtab.html#eponymous_only_virtual_tables)
    EponymousOnly,
}

/// Virtual table module
///
/// (See [SQLite doc](https://sqlite.org/c3ref/module.html))
#[repr(transparent)]
pub struct Module<'vtab, T: VTab<'vtab>> {
    base: ffi::sqlite3_module,
    phantom: PhantomData<&'vtab T>,
}

unsafe impl<'vtab, T: VTab<'vtab>> Send for Module<'vtab, T> {}
unsafe impl<'vtab, T: VTab<'vtab>> Sync for Module<'vtab, T> {}

union ModuleZeroHack {
    bytes: [u8; size_of::<ffi::sqlite3_module>()],
    module: ffi::sqlite3_module,
}

// Used as a trailing initializer for sqlite3_module -- this way we avoid having
// the build fail if buildtime_bindgen is on. This is safe, as bindgen-generated
// structs are allowed to be zeroed.
const ZERO_MODULE: ffi::sqlite3_module = unsafe {
    ModuleZeroHack {
        bytes: [0_u8; size_of::<ffi::sqlite3_module>()],
    }
    .module
};

impl<'vtab, T: VTab<'vtab>> Module<'vtab, T> {
    /// Create a base module with mandatory callbacks.
    ///
    /// This sets up xConnect, xBestIndex, xDisconnect, xOpen, xClose, xFilter,
    /// xNext, xEof, xColumn, and xRowid. All optional callbacks (xCreate,
    /// xDestroy, xUpdate, xRename, transaction callbacks) are left as None.
    /// xRowid is set to None if `T::WITHOUT_ROWID` is true.
    ///
    /// Use the `with_*` methods to enable additional capabilities.
    #[must_use]
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Module {
            base: ffi::sqlite3_module {
                iVersion: 1,
                xCreate: None,
                xConnect: Some(rust_connect::<T>),
                xBestIndex: Some(rust_best_index::<T>),
                xDisconnect: Some(rust_disconnect::<T>),
                xDestroy: None,
                xOpen: Some(rust_open::<T>),
                xClose: Some(rust_close::<T::Cursor>),
                xFilter: Some(rust_filter::<T::Cursor>),
                xNext: Some(rust_next::<T::Cursor>),
                xEof: Some(rust_eof::<T::Cursor>),
                xColumn: Some(rust_column::<T::Cursor>),
                xRowid: if T::WITHOUT_ROWID {
                    None
                } else {
                    Some(rust_rowid::<T::Cursor>)
                },
                xUpdate: None,
                xBegin: None,
                xSync: None,
                xCommit: None,
                xRollback: None,
                xFindFunction: None,
                xRename: None,
                ..ZERO_MODULE
            },
            phantom: PhantomData,
        }
    }
}

impl<'vtab, T: CreateVTab<'vtab>> Module<'vtab, T> {
    /// Enable xCreate/xDestroy based on [`VTabKind`].
    ///
    /// - [`VTabKind::Default`]: Uses separate create/destroy functions
    /// - [`VTabKind::Eponymous`]: xCreate == xConnect, xDestroy == xDisconnect
    /// - [`VTabKind::EponymousOnly`]: xCreate and xDestroy are None
    #[must_use]
    pub const fn with_create(self) -> Self {
        let (xcreate, xdestroy) = match T::KIND {
            VTabKind::EponymousOnly => (None, None),
            VTabKind::Eponymous => (
                Some(rust_connect::<T> as unsafe extern "C" fn(_, _, _, _, _, _) -> _),
                Some(rust_disconnect::<T> as unsafe extern "C" fn(_) -> _),
            ),
            VTabKind::Default => (
                Some(rust_create::<T> as unsafe extern "C" fn(_, _, _, _, _, _) -> _),
                Some(rust_destroy::<T> as unsafe extern "C" fn(_) -> _),
            ),
        };
        Module {
            base: ffi::sqlite3_module {
                xCreate: xcreate,
                xDestroy: xdestroy,
                ..self.base
            },
            phantom: PhantomData,
        }
    }
}

impl<'vtab, T: UpdateVTab<'vtab>> Module<'vtab, T> {
    /// Enable xUpdate for INSERT/UPDATE/DELETE operations.
    ///
    /// Note: This also sets xCreate/xDestroy based on [`VTabKind`].
    #[must_use]
    pub const fn with_update(self) -> Self {
        let (xcreate, xdestroy) = match T::KIND {
            VTabKind::EponymousOnly => (None, None),
            VTabKind::Eponymous => (
                Some(rust_connect::<T> as unsafe extern "C" fn(_, _, _, _, _, _) -> _),
                Some(rust_disconnect::<T> as unsafe extern "C" fn(_) -> _),
            ),
            VTabKind::Default => (
                Some(rust_create::<T> as unsafe extern "C" fn(_, _, _, _, _, _) -> _),
                Some(rust_destroy::<T> as unsafe extern "C" fn(_) -> _),
            ),
        };
        Module {
            base: ffi::sqlite3_module {
                xCreate: xcreate,
                xDestroy: xdestroy,
                xUpdate: Some(rust_update::<T>),
                ..self.base
            },
            phantom: PhantomData,
        }
    }
}

impl<'vtab, T: TransactionVTab<'vtab>> Module<'vtab, T> {
    /// Enable xBegin/xSync/xCommit/xRollback for transaction support.
    #[must_use]
    pub const fn with_transactions(self) -> Self {
        Module {
            base: ffi::sqlite3_module {
                xBegin: Some(rust_begin::<T>),
                xSync: Some(rust_sync::<T>),
                xCommit: Some(rust_commit::<T>),
                xRollback: Some(rust_rollback::<T>),
                ..self.base
            },
            phantom: PhantomData,
        }
    }
}

impl<'vtab, T: SavepointVTab<'vtab>> Module<'vtab, T> {
    /// Enable savepoint callbacks (xSavepoint, xRelease, xRollbackTo).
    ///
    /// These provide nested transaction support. The callbacks are only
    /// invoked between xBegin and xCommit/xRollback.
    ///
    /// Requires SQLite module version >= 2.
    #[must_use]
    pub const fn with_savepoints(self) -> Self {
        Module {
            base: ffi::sqlite3_module {
                iVersion: if self.base.iVersion < 2 {
                    2
                } else {
                    self.base.iVersion
                },
                xSavepoint: Some(rust_savepoint::<T>),
                xRelease: Some(rust_release_savepoint::<T>),
                xRollbackTo: Some(rust_rollback_to::<T>),
                ..self.base
            },
            phantom: PhantomData,
        }
    }
}

impl<'vtab, T: RenameVTab<'vtab>> Module<'vtab, T> {
    /// Enable xRename for ALTER TABLE RENAME support.
    #[must_use]
    pub const fn with_rename(self) -> Self {
        Module {
            base: ffi::sqlite3_module {
                xRename: Some(rust_rename::<T>),
                ..self.base
            },
            phantom: PhantomData,
        }
    }
}

impl<'vtab, T: VTab<'vtab> + ShadowNameVTab> Module<'vtab, T> {
    /// Enable xShadowName to identify shadow tables.
    ///
    /// This allows SQLite to protect shadow tables when
    /// `SQLITE_DBCONFIG_DEFENSIVE` is enabled.
    ///
    /// Requires SQLite module version >= 3.
    #[must_use]
    pub const fn with_shadow_name(self) -> Self {
        Module {
            base: ffi::sqlite3_module {
                iVersion: if self.base.iVersion < 3 {
                    3
                } else {
                    self.base.iVersion
                },
                xShadowName: Some(rust_shadow_name::<T>),
                ..self.base
            },
            phantom: PhantomData,
        }
    }
}

#[cfg(feature = "modern_sqlite")] // SQLite >= 3.44.0
impl<'vtab, T: IntegrityVTab<'vtab>> Module<'vtab, T> {
    /// Enable xIntegrity to participate in `PRAGMA integrity_check`.
    ///
    /// Requires SQLite module version >= 4 (SQLite >= 3.44.0).
    #[must_use]
    pub const fn with_integrity(self) -> Self {
        Module {
            base: ffi::sqlite3_module {
                iVersion: if self.base.iVersion < 4 {
                    4
                } else {
                    self.base.iVersion
                },
                xIntegrity: Some(rust_integrity::<T>),
                ..self.base
            },
            phantom: PhantomData,
        }
    }
}

impl<'vtab, T: FindFunctionVTab<'vtab>> Module<'vtab, T> {
    /// Enable xFindFunction to overload SQL functions for this virtual table.
    #[must_use]
    pub const fn with_find_function(self) -> Self {
        Module {
            base: ffi::sqlite3_module {
                xFindFunction: Some(rust_find_function::<T>),
                ..self.base
            },
            phantom: PhantomData,
        }
    }
}

/// Create a modifiable virtual table implementation.
///
/// Step 2 of [Creating New Virtual Table Implementations](https://sqlite.org/vtab.html#creating_new_virtual_table_implementations).
#[must_use]
pub const fn update_module<'vtab, T: UpdateVTab<'vtab>>() -> &'static Module<'vtab, T> {
    const { &Module::new().with_update() }
}

/// Create a modifiable virtual table implementation with support for transactions.
///
/// Step 2 of [Creating New Virtual Table Implementations](https://sqlite.org/vtab.html#creating_new_virtual_table_implementations).
#[must_use]
pub const fn update_module_with_tx<'vtab, T: TransactionVTab<'vtab>>() -> &'static Module<'vtab, T>
{
    const { &Module::new().with_update().with_transactions() }
}

/// Create a read-only virtual table implementation.
///
/// Step 2 of [Creating New Virtual Table Implementations](https://sqlite.org/vtab.html#creating_new_virtual_table_implementations).
#[must_use]
pub const fn read_only_module<'vtab, T: CreateVTab<'vtab>>() -> &'static Module<'vtab, T> {
    const { &Module::new().with_create() }
}

/// Create an eponymous only virtual table implementation.
///
/// Step 2 of [Creating New Virtual Table Implementations](https://sqlite.org/vtab.html#creating_new_virtual_table_implementations).
#[must_use]
pub const fn eponymous_only_module<'vtab, T: VTab<'vtab>>() -> &'static Module<'vtab, T> {
    const { &Module::new() }
}

/// Virtual table configuration options
#[repr(i32)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum VTabConfig {
    /// Equivalent to `SQLITE_VTAB_CONSTRAINT_SUPPORT`
    ConstraintSupport = 1,
    /// Equivalent to `SQLITE_VTAB_INNOCUOUS`
    Innocuous = 2,
    /// Equivalent to `SQLITE_VTAB_DIRECTONLY`
    DirectOnly = 3,
    /// Equivalent to `SQLITE_VTAB_USES_ALL_SCHEMAS`
    UsesAllSchemas = 4,
}

/// `feature = "vtab"`
pub struct VTabConnection(*mut ffi::sqlite3);

impl VTabConnection {
    /// Configure various facets of the virtual table interface
    pub fn config(&mut self, config: VTabConfig) -> Result<()> {
        check(unsafe { ffi::sqlite3_vtab_config(self.0, config as c_int) })
    }

    /// Get access to the underlying SQLite database connection handle.
    ///
    /// # Warning
    ///
    /// You should not need to use this function. If you do need to, please
    /// [open an issue on the rusqlite repository](https://github.com/rusqlite/rusqlite/issues) and describe
    /// your use case.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it gives you raw access
    /// to the SQLite connection, and what you do with it could impact the
    /// safety of this `Connection`.
    pub unsafe fn handle(&mut self) -> *mut ffi::sqlite3 {
        self.0
    }
}

/// Eponymous-only virtual table instance trait.
///
/// # Safety
///
/// The first item in a struct implementing `VTab` must be
/// `rusqlite::sqlite3_vtab`, and the struct must be `#[repr(C)]`.
///
/// ```rust,ignore
/// #[repr(C)]
/// struct MyTab {
///    /// Base class. Must be first
///    base: rusqlite::vtab::sqlite3_vtab,
///    /* Virtual table implementations will typically add additional fields */
/// }
/// ```
///
/// (See [SQLite doc](https://sqlite.org/c3ref/vtab.html))
pub unsafe trait VTab<'vtab>: Sized {
    /// Client data passed to [`Connection::create_module`].
    type Aux;
    /// Specific cursor implementation
    type Cursor: VTabCursor;

    /// Whether this is a WITHOUT ROWID virtual table.
    /// If set to true, the generated CREATE TABLE statement _must_ include WITHOUT ROWID.
    const WITHOUT_ROWID: bool = false;

    /// Establish a new connection to an existing virtual table.
    ///
    /// (See [SQLite doc](https://sqlite.org/vtab.html#the_xconnect_method))
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Self::Aux>,
        args: &[&[u8]],
    ) -> Result<(String, Self)>;

    /// Determine the best way to access the virtual table.
    /// (See [SQLite doc](https://sqlite.org/vtab.html#the_xbestindex_method))
    fn best_index(&self, info: &mut IndexInfo) -> Result<()>;

    /// Create a new cursor used for accessing a virtual table.
    /// (See [SQLite doc](https://sqlite.org/vtab.html#the_xopen_method))
    fn open(&'vtab mut self) -> Result<Self::Cursor>;
}

/// Read-only virtual table instance trait.
///
/// (See [SQLite doc](https://sqlite.org/c3ref/vtab.html))
pub trait CreateVTab<'vtab>: VTab<'vtab> {
    /// For [`EponymousOnly`](VTabKind::EponymousOnly),
    /// [`create`](CreateVTab::create) and [`destroy`](CreateVTab::destroy) are
    /// not called
    const KIND: VTabKind;
    /// Create a new instance of a virtual table in response to a CREATE VIRTUAL
    /// TABLE statement. The `db` parameter is a pointer to the SQLite
    /// database connection that is executing the CREATE VIRTUAL TABLE
    /// statement.
    ///
    /// Call [`connect`](VTab::connect) by default.
    /// (See [SQLite doc](https://sqlite.org/vtab.html#the_xcreate_method))
    fn create(
        db: &mut VTabConnection,
        aux: Option<&Self::Aux>,
        args: &[&[u8]],
    ) -> Result<(String, Self)> {
        Self::connect(db, aux, args)
    }

    /// Destroy the underlying table implementation. This method undoes the work
    /// of [`create`](CreateVTab::create).
    ///
    /// Do nothing by default.
    /// (See [SQLite doc](https://sqlite.org/vtab.html#the_xdestroy_method))
    fn destroy(&self) -> Result<()> {
        Ok(())
    }
}

/// Writable virtual table instance trait.
///
/// (See [SQLite doc](https://sqlite.org/vtab.html#xupdate))
pub trait UpdateVTab<'vtab>: CreateVTab<'vtab> {
    /// Delete rowid or PK
    fn delete(&mut self, arg: ValueRef<'_>) -> Result<()>;
    /// Insert: `args[0] == NULL: old rowid or PK, args[1]: new rowid or PK,
    /// args[2]: ...`
    ///
    /// Return the new rowid.
    /// If the VTab is a WITHOUT_ROWID table, then the returned "rowid" is ignored.
    // TODO Make the distinction between argv[1] == NULL and argv[1] != NULL ?
    fn insert(&mut self, args: &Inserts<'_>) -> Result<i64>;
    /// Update: `args[0] != NULL: old rowid or PK, args[1]: new row id or PK,
    /// args[2]: ...`
    fn update(&mut self, args: &Updates<'_>) -> Result<()>;
}

/// Virtual table that supports renaming via ALTER TABLE RENAME.
///
/// See [SQLite doc](https://sqlite.org/vtab.html#the_xrename_method)
pub trait RenameVTab<'vtab>: CreateVTab<'vtab> {
    /// Notify the virtual table that it will be given a new name.
    ///
    /// If this method returns `Ok(())`, SQLite renames the table.
    /// If this method returns an error, the renaming is prevented.
    fn rename(&mut self, new_name: &str) -> Result<()>;
}

/// Writable virtual table instance with transaction support trait.
///
/// See [SQLite doc](https://sqlite.org/vtab.html#the_xbegin_method)
pub trait TransactionVTab<'vtab>: UpdateVTab<'vtab> {
    /// Start a new transaction
    fn begin(&mut self) -> Result<()> {
        Ok(())
    }
    /// Begin two-phase commit
    fn sync(&mut self) -> Result<()> {
        Ok(())
    }
    /// Commit the current transaction
    fn commit(&mut self) -> Result<()> {
        Ok(())
    }
    /// Abandon the current transaction
    fn rollback(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Virtual table with savepoint (nested transaction) support.
///
/// These methods are only called between [`TransactionVTab::begin`] and
/// [`TransactionVTab::commit`]/[`TransactionVTab::rollback`].
///
/// See [SQLite doc](https://sqlite.org/vtab.html#xsavepoint)
pub trait SavepointVTab<'vtab>: TransactionVTab<'vtab> {
    /// Save current state as savepoint N.
    ///
    /// A subsequent call to [`rollback_to`](Self::rollback_to) with the same N
    /// means the virtual table state should return to what it was when this
    /// method was called.
    fn savepoint(&mut self, savepoint_id: c_int) -> Result<()>;

    /// Invalidate all savepoints where N >= `savepoint_id`.
    fn release(&mut self, savepoint_id: c_int) -> Result<()>;

    /// Return to the state when [`savepoint`](Self::savepoint) was called with
    /// `savepoint_id`.
    ///
    /// This invalidates all savepoints with N > `savepoint_id`.
    fn rollback_to(&mut self, savepoint_id: c_int) -> Result<()>;
}

/// Virtual table that uses shadow tables.
///
/// Implement this trait to allow SQLite to identify shadow tables belonging
/// to this virtual table. When `SQLITE_DBCONFIG_DEFENSIVE` is enabled,
/// shadow tables become read-only for ordinary SQL statements.
///
/// See [SQLite doc](https://sqlite.org/vtab.html#the_xshadowname_method)
pub trait ShadowNameVTab {
    /// Returns `true` if the given suffix identifies a shadow table.
    ///
    /// For example, if your virtual table "foo" uses shadow tables named
    /// "foo_content" and "foo_index", this should return `true` for
    /// "content" and "index".
    fn shadow_name(suffix: &str) -> bool;
}

/// Virtual table that supports integrity checking.
///
/// Implement this trait to participate in `PRAGMA integrity_check` and
/// `PRAGMA quick_check`.
///
/// Requires SQLite >= 3.44.0.
///
/// See [SQLite doc](https://sqlite.org/vtab.html#the_xintegrity_method)
#[cfg(feature = "modern_sqlite")]
pub trait IntegrityVTab<'vtab>: VTab<'vtab> {
    /// Check the integrity of the virtual table content.
    ///
    /// - `schema`: The schema name ("main", "temp", etc.)
    /// - `table`: The virtual table name
    /// - `flags`: 0 for `integrity_check`, 1 for `quick_check`
    ///
    /// Return `Ok(None)` if no problems are found.
    /// Return `Ok(Some(message))` to report an integrity problem.
    /// Return `Err(...)` only if the integrity check itself fails (e.g., OOM).
    fn integrity(&self, schema: &str, table: &str, flags: c_int) -> Result<Option<String>>;
}

/// A wrapper for SQL functions that can be returned from [`FindFunctionVTab::find_function`].
///
/// This type stores a closure and provides the raw function pointer and user data
/// needed for xFindFunction. Store instances of this type in your virtual table
/// struct to ensure they remain valid for the vtab's lifetime.
///
/// Requires the `functions` feature.
///
/// # Example
///
/// ```rust,ignore
/// use rusqlite::vtab::{VTabFunc, FindFunctionResult};
///
/// #[repr(C)]
/// struct MyVTab {
///     base: sqlite3_vtab,
///     // Store the function to keep it alive
///     my_func: VTabFunc<fn(&functions::Context<'_>) -> Result<i64>>,
/// }
///
/// impl MyVTab {
///     fn new() -> Self {
///         Self {
///             base: Default::default(),
///             my_func: VTabFunc::new(|ctx| {
///                 let val: i64 = ctx.get(0)?;
///                 Ok(val * 2)
///             }),
///         }
///     }
/// }
/// ```
#[cfg(feature = "functions")]
pub struct VTabFunc<F> {
    func: F,
}

#[cfg(feature = "functions")]
impl<F, T> VTabFunc<F>
where
    F: Fn(&crate::functions::Context<'_>) -> Result<T>,
    T: crate::functions::SqlFnOutput,
{
    /// Create a new virtual table function wrapper.
    ///
    /// The closure receives a [`functions::Context`](crate::functions::Context)
    /// and should return a [`Result<T>`] where `T` implements
    /// [`SqlFnOutput`](crate::functions::SqlFnOutput).
    pub fn new(func: F) -> Self {
        Self { func }
    }

    /// Get a [`FindFunctionResult::Overload`] that can be returned from
    /// [`FindFunctionVTab::find_function`].
    pub fn as_overload(&self) -> FindFunctionResult {
        FindFunctionResult::Overload {
            func: vtab_func_trampoline::<F, T>,
            user_data: (&self.func as *const F).cast_mut().cast(),
        }
    }

    /// Get a [`FindFunctionResult::Indexable`] that can be returned from
    /// [`FindFunctionVTab::find_function`].
    ///
    /// The `constraint_op` value will appear in `sqlite3_index_info.aConstraint[].op`
    /// during [`VTab::best_index`], allowing query optimization.
    /// Must use [`SQLITE_INDEX_CONSTRAINT_FUNCTION`].
    pub fn as_indexable(&self, constraint_op: IndexConstraintOp) -> FindFunctionResult {
        FindFunctionResult::Indexable {
            func: vtab_func_trampoline::<F, T>,
            user_data: (&self.func as *const F).cast_mut().cast(),
            constraint_op,
        }
    }
}

/// Trampoline function that bridges FFI to the Rust closure.
#[cfg(feature = "functions")]
unsafe extern "C" fn vtab_func_trampoline<F, T>(
    ctx: *mut ffi::sqlite3_context,
    argc: c_int,
    argv: *mut *mut ffi::sqlite3_value,
) where
    F: Fn(&crate::functions::Context<'_>) -> Result<T>,
    T: crate::functions::SqlFnOutput,
{
    use std::panic::catch_unwind;

    let args = slice::from_raw_parts(argv, argc as usize);
    let r = catch_unwind(std::panic::AssertUnwindSafe(|| {
        let func_ptr = ffi::sqlite3_user_data(ctx).cast::<F>();
        assert!(
            !func_ptr.is_null(),
            "Internal error - null function pointer"
        );
        let fn_ctx = crate::functions::Context { ctx, args };
        (*func_ptr)(&fn_ctx)
    }));

    match r {
        Err(_) => {
            ffi::sqlite3_result_error_code(ctx, ffi::SQLITE_ERROR);
            if let Ok(cstr) = str_to_cstring("Rust panic in vtab function") {
                ffi::sqlite3_result_error(ctx, cstr.as_ptr(), -1);
            }
        }
        Ok(Ok(value)) => match crate::functions::SqlFnOutput::to_sql(&value) {
            Ok((ref output, sub_type)) => {
                set_result(ctx, args, output);
                if let Some(st) = sub_type {
                    ffi::sqlite3_result_subtype(ctx, st);
                }
            }
            Err(err) => {
                ffi::sqlite3_result_error_code(ctx, ffi::SQLITE_ERROR);
                if let Ok(cstr) = str_to_cstring(&err.to_string()) {
                    ffi::sqlite3_result_error(ctx, cstr.as_ptr(), -1);
                }
            }
        },
        Ok(Err(err)) => {
            if let Error::SqliteFailure(ref e, ref s) = err {
                ffi::sqlite3_result_error_code(ctx, e.extended_code);
                if let Some(Ok(cstr)) = s.as_ref().map(|s| str_to_cstring(s)) {
                    ffi::sqlite3_result_error(ctx, cstr.as_ptr(), -1);
                }
            } else {
                ffi::sqlite3_result_error_code(ctx, ffi::SQLITE_ERROR);
                if let Ok(cstr) = str_to_cstring(&err.to_string()) {
                    ffi::sqlite3_result_error(ctx, cstr.as_ptr(), -1);
                }
            }
        }
    }
}

/// Result of [`FindFunctionVTab::find_function`].
///
/// Specifies how to overload a function within a virtual table query.
///
/// For a high-level API, use [`VTabFunc`] (requires `functions` feature) to
/// create these values from closures. The raw variants are available for
/// advanced use cases or when the `functions` feature is not enabled.
#[derive(Clone, Copy)]
pub enum FindFunctionResult {
    /// No function overload; use the default function.
    None,
    /// Overload the function with the provided implementation.
    ///
    /// The function pointer and user data will be used instead of
    /// the default SQL function.
    ///
    /// For a safer API, use [`VTabFunc::as_overload`].
    Overload {
        /// The function implementation (same signature as scalar functions).
        func: unsafe extern "C" fn(
            ctx: *mut ffi::sqlite3_context,
            argc: c_int,
            argv: *mut *mut ffi::sqlite3_value,
        ),
        /// User data passed to the function as `sqlite3_user_data()`.
        ///
        /// Must remain valid for the lifetime of the virtual table.
        user_data: *mut c_void,
    },
    /// Overload the function and mark it as indexable.
    ///
    /// This enables the function to be used in WHERE clauses with
    /// optimization support (e.g., `WHERE geopoly_overlap(col, ?)`).
    ///
    /// The `constraint_op` must be `SQLITE_INDEX_CONSTRAINT_FUNCTION`.
    /// This value will appear in `sqlite3_index_info.aConstraint[].op` during
    /// [`VTab::best_index`].
    ///
    /// For a safer API, use [`VTabFunc::as_indexable`].
    ///
    /// Requires SQLite >= 3.25.0.
    Indexable {
        /// The function implementation.
        func: unsafe extern "C" fn(
            ctx: *mut ffi::sqlite3_context,
            argc: c_int,
            argv: *mut *mut ffi::sqlite3_value,
        ),
        /// User data passed to the function.
        user_data: *mut c_void,
        /// The constraint operator code later passed to [`VTab::best_index`].
        constraint_op: IndexConstraintOp,
    },
}

/// Virtual table with function overloading support.
///
/// Implement this trait to allow the virtual table to provide custom
/// implementations of SQL functions when used with this table's columns.
///
/// This is called during `sqlite3_prepare()` to check if the virtual table
/// wants to overload a function. The function is only considered for
/// overloading when a column from this virtual table is the first argument.
///
/// # Example
///
/// ```rust,ignore
/// use rusqlite::vtab::{VTab, FindFunctionVTab, VTabFunc, FindFunctionResult};
///
/// #[repr(C)]
/// struct MyVTab {
///     base: sqlite3_vtab,
///     double_func: VTabFunc<fn(&functions::Context<'_>) -> Result<i64>>,
/// }
///
/// impl FindFunctionVTab<'_> for MyVTab {
///     fn find_function(&self, _n_arg: c_int, name: &str) -> FindFunctionResult {
///         if name.eq_ignore_ascii_case("double") {
///             self.double_func.as_overload()
///         } else {
///             FindFunctionResult::None
///         }
///     }
/// }
/// ```
///
/// See [SQLite doc](https://sqlite.org/vtab.html#the_xfindfunction_method)
pub trait FindFunctionVTab<'vtab>: VTab<'vtab> {
    /// Check if the virtual table wants to overload a function.
    ///
    /// - `n_arg`: Number of arguments the function is called with
    /// - `name`: Name of the function being looked up
    ///
    /// Return [`FindFunctionResult::None`] if no overloading is desired.
    /// Return [`FindFunctionResult::Overload`] to provide a custom implementation.
    /// Return [`FindFunctionResult::Indexable`] to provide a custom implementation
    /// that can also be used for query optimization (requires SQLite >= 3.25.0).
    fn find_function(&self, n_arg: c_int, name: &str) -> FindFunctionResult;
}

/// Index constraint operator.
/// See [Virtual Table Constraint Operator Codes](https://sqlite.org/c3ref/c_index_constraint_eq.html) for details.
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
#[allow(missing_docs)]
#[expect(non_camel_case_types)]
pub enum IndexConstraintOp {
    SQLITE_INDEX_CONSTRAINT_EQ,
    SQLITE_INDEX_CONSTRAINT_GT,
    SQLITE_INDEX_CONSTRAINT_LE,
    SQLITE_INDEX_CONSTRAINT_LT,
    SQLITE_INDEX_CONSTRAINT_GE,
    SQLITE_INDEX_CONSTRAINT_MATCH,
    SQLITE_INDEX_CONSTRAINT_LIKE,         // 3.10.0
    SQLITE_INDEX_CONSTRAINT_GLOB,         // 3.10.0
    SQLITE_INDEX_CONSTRAINT_REGEXP,       // 3.10.0
    SQLITE_INDEX_CONSTRAINT_NE,           // 3.21.0
    SQLITE_INDEX_CONSTRAINT_ISNOT,        // 3.21.0
    SQLITE_INDEX_CONSTRAINT_ISNOTNULL,    // 3.21.0
    SQLITE_INDEX_CONSTRAINT_ISNULL,       // 3.21.0
    SQLITE_INDEX_CONSTRAINT_IS,           // 3.21.0
    SQLITE_INDEX_CONSTRAINT_LIMIT,        // 3.38.0
    SQLITE_INDEX_CONSTRAINT_OFFSET,       // 3.38.0
    /// Value must be >=150.
    SQLITE_INDEX_CONSTRAINT_FUNCTION(u8), // 3.25.0
}

impl From<u8> for IndexConstraintOp {
    fn from(code: u8) -> Self {
        match code {
            2 => Self::SQLITE_INDEX_CONSTRAINT_EQ,
            4 => Self::SQLITE_INDEX_CONSTRAINT_GT,
            8 => Self::SQLITE_INDEX_CONSTRAINT_LE,
            16 => Self::SQLITE_INDEX_CONSTRAINT_LT,
            32 => Self::SQLITE_INDEX_CONSTRAINT_GE,
            64 => Self::SQLITE_INDEX_CONSTRAINT_MATCH,
            65 => Self::SQLITE_INDEX_CONSTRAINT_LIKE,
            66 => Self::SQLITE_INDEX_CONSTRAINT_GLOB,
            67 => Self::SQLITE_INDEX_CONSTRAINT_REGEXP,
            68 => Self::SQLITE_INDEX_CONSTRAINT_NE,
            69 => Self::SQLITE_INDEX_CONSTRAINT_ISNOT,
            70 => Self::SQLITE_INDEX_CONSTRAINT_ISNOTNULL,
            71 => Self::SQLITE_INDEX_CONSTRAINT_ISNULL,
            72 => Self::SQLITE_INDEX_CONSTRAINT_IS,
            73 => Self::SQLITE_INDEX_CONSTRAINT_LIMIT,
            74 => Self::SQLITE_INDEX_CONSTRAINT_OFFSET,
            v => Self::SQLITE_INDEX_CONSTRAINT_FUNCTION(v),
        }
    }
}

impl From<IndexConstraintOp> for u8 {
    fn from(value: IndexConstraintOp) -> u8 {
        match value {
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ => 2,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GT => 4,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LE => 8,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LT => 16,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GE => 32,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_MATCH => 64,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LIKE => 65,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_GLOB => 66,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_REGEXP => 67,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_NE => 68,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_ISNOT => 69,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_ISNOTNULL => 70,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_ISNULL => 71,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_IS => 72,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_LIMIT => 73,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_OFFSET => 74,
            IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_FUNCTION(v) => v,
        }
    }
}

bitflags::bitflags! {
    /// Virtual table scan flags
    /// See [Function Flags](https://sqlite.org/c3ref/c_index_scan_unique.html) for details.
    #[repr(C)]
    #[derive(Copy, Clone, Debug)]
    pub struct IndexFlags: c_int {
        /// Default
        const NONE     = 0;
        /// Scan visits at most 1 row.
        const SQLITE_INDEX_SCAN_UNIQUE  = ffi::SQLITE_INDEX_SCAN_UNIQUE;
        /// Display idxNum as hex in EXPLAIN QUERY PLAN
        const SQLITE_INDEX_SCAN_HEX  = 0x0000_0002; // 3.47.0
    }
}

/// Pass information into and receive the reply from the
/// [`VTab::best_index`] method.
///
/// (See [SQLite doc](http://sqlite.org/c3ref/index_info.html))
#[derive(Debug)]
pub struct IndexInfo(*mut ffi::sqlite3_index_info);

impl IndexInfo {
    /// Iterate on index constraint and its associated usage.
    #[inline]
    pub fn constraints_and_usages(&mut self) -> IndexConstraintAndUsageIter<'_> {
        let constraints =
            unsafe { slice::from_raw_parts((*self.0).aConstraint, (*self.0).nConstraint as usize) };
        let constraint_usages = unsafe {
            slice::from_raw_parts_mut((*self.0).aConstraintUsage, (*self.0).nConstraint as usize)
        };
        IndexConstraintAndUsageIter {
            iter: constraints.iter().zip(constraint_usages.iter_mut()),
        }
    }

    /// Record WHERE clause constraints.
    #[inline]
    #[must_use]
    pub fn constraints(&self) -> IndexConstraintIter<'_> {
        let constraints =
            unsafe { slice::from_raw_parts((*self.0).aConstraint, (*self.0).nConstraint as usize) };
        IndexConstraintIter {
            iter: constraints.iter(),
        }
    }

    /// Information about the ORDER BY clause.
    #[inline]
    #[must_use]
    pub fn order_bys(&self) -> OrderByIter<'_> {
        let order_bys =
            unsafe { slice::from_raw_parts((*self.0).aOrderBy, (*self.0).nOrderBy as usize) };
        OrderByIter {
            iter: order_bys.iter(),
        }
    }

    /// Number of terms in the ORDER BY clause
    #[inline]
    #[must_use]
    pub fn num_of_order_by(&self) -> usize {
        unsafe { (*self.0).nOrderBy as usize }
    }

    /// Information about what parameters to pass to [`VTabCursor::filter`].
    #[inline]
    pub fn constraint_usage(&mut self, constraint_idx: usize) -> IndexConstraintUsage<'_> {
        let constraint_usages = unsafe {
            slice::from_raw_parts_mut((*self.0).aConstraintUsage, (*self.0).nConstraint as usize)
        };
        IndexConstraintUsage(&mut constraint_usages[constraint_idx])
    }

    /// Number used to identify the index
    #[inline]
    pub fn set_idx_num(&mut self, idx_num: c_int) {
        unsafe {
            (*self.0).idxNum = idx_num;
        }
    }

    /// String used to identify the index
    pub fn set_idx_str(&mut self, idx_str: &str) {
        unsafe {
            if (*self.0).needToFreeIdxStr == 1 {
                sqlite3_free((*self.0).idxStr as _);
            }
            (*self.0).idxStr = alloc(idx_str);
            (*self.0).needToFreeIdxStr = 1;
        }
    }
    /// String used to identify the index
    pub fn set_idx_cstr(&mut self, idx_str: &'static CStr) {
        unsafe {
            if (*self.0).needToFreeIdxStr == 1 {
                sqlite3_free((*self.0).idxStr as _);
            }
            (*self.0).idxStr = idx_str.as_ptr() as _;
            (*self.0).needToFreeIdxStr = 0;
        }
    }

    /// True if output is already ordered
    #[inline]
    pub fn set_order_by_consumed(&mut self, order_by_consumed: bool) {
        unsafe {
            (*self.0).orderByConsumed = order_by_consumed as c_int;
        }
    }

    /// Estimated cost of using this index
    #[inline]
    pub fn set_estimated_cost(&mut self, estimated_ost: f64) {
        unsafe {
            (*self.0).estimatedCost = estimated_ost;
        }
    }

    /// Estimated number of rows returned.
    #[inline]
    pub fn set_estimated_rows(&mut self, estimated_rows: i64) {
        unsafe {
            (*self.0).estimatedRows = estimated_rows;
        }
    }

    /// Mask of `SQLITE_INDEX_SCAN_*` flags.
    #[inline]
    pub fn set_idx_flags(&mut self, flags: IndexFlags) {
        unsafe { (*self.0).idxFlags = flags.bits() };
    }

    /// Mask of columns used by statement
    #[inline]
    pub fn col_used(&self) -> u64 {
        unsafe { (*self.0).colUsed }
    }

    /// Determine the collation for a virtual table constraint
    pub fn collation(&self, constraint_idx: usize) -> Result<&str> {
        let idx = constraint_idx as c_int;
        let collation = unsafe { ffi::sqlite3_vtab_collation(self.0, idx) };
        if collation.is_null() {
            return Err(err!(ffi::SQLITE_MISUSE, "{constraint_idx} is out of range"));
        }
        Ok(unsafe { CStr::from_ptr(collation) }.to_str()?)
    }

    /// Determine if a virtual table query is DISTINCT
    #[must_use]
    #[cfg(feature = "modern_sqlite")] // SQLite >= 3.38.0
    pub fn distinct(&self) -> DistinctMode {
        match unsafe { ffi::sqlite3_vtab_distinct(self.0) } {
            0 => DistinctMode::Ordered,
            1 => DistinctMode::Grouped,
            2 => DistinctMode::Distinct,
            3 => DistinctMode::DistinctOrdered,
            _ => DistinctMode::Ordered,
        }
    }

    /// Constraint value
    #[cfg(feature = "modern_sqlite")] // SQLite >= 3.38.0
    pub fn rhs_value(&self, constraint_idx: usize) -> Result<Option<ValueRef<'_>>> {
        let idx = constraint_idx as c_int;
        let mut p_value: *mut ffi::sqlite3_value = ptr::null_mut();
        let rc = unsafe { ffi::sqlite3_vtab_rhs_value(self.0, idx, &mut p_value) };
        if rc == ffi::SQLITE_NOTFOUND {
            return Ok(None);
        }
        check(rc)?;
        assert!(!p_value.is_null());
        Ok(Some(unsafe { ValueRef::from_value(p_value) }))
    }

    /// Identify IN constraints
    #[cfg(feature = "modern_sqlite")] // SQLite >= 3.38.0
    pub fn is_in_constraint(&self, constraint_idx: usize) -> Result<bool> {
        self.check_constraint_index(constraint_idx)?;
        let idx = constraint_idx as c_int;
        Ok(unsafe { ffi::sqlite3_vtab_in(self.0, idx, -1) != 0 })
    }
    /// Handle IN constraints
    #[cfg(feature = "modern_sqlite")] // SQLite >= 3.38.0
    pub fn set_in_constraint(&mut self, constraint_idx: usize, filter_all: bool) -> Result<bool> {
        self.check_constraint_index(constraint_idx)?;
        let idx = constraint_idx as c_int;
        Ok(unsafe { ffi::sqlite3_vtab_in(self.0, idx, filter_all as c_int) != 0 })
    }

    #[cfg(feature = "modern_sqlite")] // SQLite >= 3.38.0
    fn check_constraint_index(&self, idx: usize) -> Result<()> {
        if idx >= unsafe { (*self.0).nConstraint } as usize {
            return Err(err!(ffi::SQLITE_MISUSE, "{idx} is out of range"));
        }
        Ok(())
    }
}

/// Determine if a virtual table query is DISTINCT
#[non_exhaustive]
#[derive(Debug, Eq, PartialEq)]
pub enum DistinctMode {
    /// This is the default expectation.
    Ordered,
    /// This mode is used when the query planner is doing a GROUP BY.
    Grouped,
    /// This mode is used for a DISTINCT query.
    Distinct,
    /// This mode is used for queries that have both DISTINCT and ORDER BY clauses.
    DistinctOrdered,
}

/// Iterate on index constraint and its associated usage.
pub struct IndexConstraintAndUsageIter<'a> {
    iter: std::iter::Zip<
        slice::Iter<'a, ffi::sqlite3_index_constraint>,
        slice::IterMut<'a, ffi::sqlite3_index_constraint_usage>,
    >,
}

impl<'a> Iterator for IndexConstraintAndUsageIter<'a> {
    type Item = (IndexConstraint<'a>, IndexConstraintUsage<'a>);

    #[inline]
    fn next(&mut self) -> Option<(IndexConstraint<'a>, IndexConstraintUsage<'a>)> {
        self.iter
            .next()
            .map(|raw| (IndexConstraint(raw.0), IndexConstraintUsage(raw.1)))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

/// `feature = "vtab"`
pub struct IndexConstraintIter<'a> {
    iter: slice::Iter<'a, ffi::sqlite3_index_constraint>,
}

impl<'a> Iterator for IndexConstraintIter<'a> {
    type Item = IndexConstraint<'a>;

    #[inline]
    fn next(&mut self) -> Option<IndexConstraint<'a>> {
        self.iter.next().map(IndexConstraint)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

/// WHERE clause constraint.
pub struct IndexConstraint<'a>(&'a ffi::sqlite3_index_constraint);

impl IndexConstraint<'_> {
    /// Column constrained.  -1 for ROWID
    #[inline]
    #[must_use]
    pub fn column(&self) -> c_int {
        self.0.iColumn
    }

    /// Constraint operator
    #[inline]
    #[must_use]
    pub fn operator(&self) -> IndexConstraintOp {
        IndexConstraintOp::from(self.0.op)
    }

    /// True if this constraint is usable
    #[inline]
    #[must_use]
    pub fn is_usable(&self) -> bool {
        self.0.usable != 0
    }
}

/// Information about what parameters to pass to
/// [`VTabCursor::filter`].
pub struct IndexConstraintUsage<'a>(&'a mut ffi::sqlite3_index_constraint_usage);

impl IndexConstraintUsage<'_> {
    /// if `argv_index` > 0, constraint is part of argv to
    /// [`VTabCursor::filter`]
    #[inline]
    pub fn set_argv_index(&mut self, argv_index: c_int) {
        self.0.argvIndex = argv_index;
    }

    /// if `omit`, do not code a test for this constraint
    #[inline]
    pub fn set_omit(&mut self, omit: bool) {
        self.0.omit = omit as std::ffi::c_uchar;
    }
}

/// `feature = "vtab"`
pub struct OrderByIter<'a> {
    iter: slice::Iter<'a, ffi::sqlite3_index_orderby>,
}

impl<'a> Iterator for OrderByIter<'a> {
    type Item = OrderBy<'a>;

    #[inline]
    fn next(&mut self) -> Option<OrderBy<'a>> {
        self.iter.next().map(OrderBy)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

/// A column of the ORDER BY clause.
pub struct OrderBy<'a>(&'a ffi::sqlite3_index_orderby);

impl OrderBy<'_> {
    /// Column number
    #[inline]
    #[must_use]
    pub fn column(&self) -> c_int {
        self.0.iColumn
    }

    /// True for DESC.  False for ASC.
    #[inline]
    #[must_use]
    pub fn is_order_by_desc(&self) -> bool {
        self.0.desc != 0
    }
}

/// Virtual table cursor trait.
///
/// # Safety
///
/// Implementations must be like:
/// ```rust,ignore
/// #[repr(C)]
/// struct MyTabCursor {
///    /// Base class. Must be first
///    base: rusqlite::vtab::sqlite3_vtab_cursor,
///    /* Virtual table implementations will typically add additional fields */
/// }
/// ```
///
/// (See [SQLite doc](https://sqlite.org/c3ref/vtab_cursor.html))
pub unsafe trait VTabCursor: Sized {
    /// Begin a search of a virtual table.
    /// (See [SQLite doc](https://sqlite.org/vtab.html#the_xfilter_method))
    fn filter(&mut self, idx_num: c_int, idx_str: Option<&str>, args: &Filters<'_>) -> Result<()>;
    /// Advance cursor to the next row of a result set initiated by
    /// [`filter`](VTabCursor::filter). (See [SQLite doc](https://sqlite.org/vtab.html#the_xnext_method))
    fn next(&mut self) -> Result<()>;
    /// Must return `false` if the cursor currently points to a valid row of
    /// data, or `true` otherwise.
    /// (See [SQLite doc](https://sqlite.org/vtab.html#the_xeof_method))
    fn eof(&self) -> bool;
    /// Find the value for the `i`-th column of the current row.
    /// `i` is zero-based so the first column is numbered 0.
    /// May return its result back to SQLite using one of the specified `ctx`.
    /// (See [SQLite doc](https://sqlite.org/vtab.html#the_xcolumn_method))
    fn column(&self, ctx: &mut Context, i: c_int) -> Result<()>;
    /// Return the rowid of row that the cursor is currently pointing at.
    /// Will not be called if the vtab is WITHOUT ROWID.
    /// (See [SQLite doc](https://sqlite.org/vtab.html#the_xrowid_method))
    fn rowid(&self) -> Result<i64>;
}

/// Context is used by [`VTabCursor::column`] to specify the
/// cell value.
pub struct Context(*mut ffi::sqlite3_context);

impl Context {
    /// Set current cell value
    #[inline]
    pub fn set_result<T: ToSql>(&mut self, value: &T) -> Result<()> {
        let t = value.to_sql()?;
        unsafe { set_result(self.0, &[], &t) };
        Ok(())
    }

    /// Determine if column access is for UPDATE
    #[inline]
    #[must_use]
    pub fn no_change(&self) -> bool {
        unsafe { ffi::sqlite3_vtab_nochange(self.0) != 0 }
    }

    /// Get the db connection handle via [sqlite3_context_db_handle](https://www.sqlite.org/c3ref/context_db_handle.html)
    ///
    /// # Safety
    ///
    /// This function is unsafe because improper use may impact the Connection.
    pub unsafe fn get_connection(&self) -> Result<ConnectionRef<'_>> {
        let handle = ffi::sqlite3_context_db_handle(self.0);
        Ok(ConnectionRef {
            conn: Connection::from_handle(handle)?,
            phantom: PhantomData,
        })
    }
}

/// A reference to a connection handle with a lifetime bound to context.
pub struct ConnectionRef<'ctx> {
    // comes from Connection::from_handle(sqlite3_context_db_handle(...))
    // and is non-owning
    conn: Connection,
    phantom: PhantomData<&'ctx Context>,
}

impl Deref for ConnectionRef<'_> {
    type Target = Connection;

    #[inline]
    fn deref(&self) -> &Connection {
        &self.conn
    }
}

/// Wrapper to [`VTabCursor::filter`] arguments, the values
/// requested by [`VTab::best_index`].
pub struct Filters<'a> {
    values: Values<'a>,
}
impl<'a> Deref for Filters<'a> {
    type Target = Values<'a>;

    fn deref(&self) -> &Self::Target {
        &self.values
    }
}
#[cfg(feature = "modern_sqlite")] // SQLite >= 3.38.0
impl<'a> Filters<'a> {
    /// Find all elements on the right-hand side of an IN constraint
    pub fn in_values(&self, idx: usize) -> Result<InValues<'_>> {
        let list = self.args[idx];
        Ok(InValues {
            list,
            phantom: PhantomData,
            first: true,
        })
    }
}

/// IN values
#[cfg(feature = "modern_sqlite")] // SQLite >= 3.38.0
pub struct InValues<'a> {
    list: *mut ffi::sqlite3_value,
    phantom: PhantomData<Filters<'a>>,
    first: bool,
}
#[cfg(feature = "modern_sqlite")] // SQLite >= 3.38.0
impl<'a> fallible_iterator::FallibleIterator for InValues<'a> {
    type Error = Error;
    type Item = ValueRef<'a>;

    fn next(&mut self) -> Result<Option<Self::Item>> {
        let mut val: *mut ffi::sqlite3_value = ptr::null_mut();
        let rc = unsafe {
            if self.first {
                self.first = false;
                ffi::sqlite3_vtab_in_first(self.list, &mut val)
            } else {
                ffi::sqlite3_vtab_in_next(self.list, &mut val)
            }
        };
        match rc {
            ffi::SQLITE_OK => Ok(Some(unsafe { ValueRef::from_value(val) })),
            ffi::SQLITE_DONE => Ok(None),
            _ => Err(error_from_sqlite_code(rc, None)),
        }
    }
}

/// Wrapper to [ffi::sqlite3_value]s
pub struct Values<'a> {
    args: &'a [*mut ffi::sqlite3_value],
}

impl Values<'_> {
    /// Returns the number of values.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.args.len()
    }

    /// Returns `true` if there is no value.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.args.is_empty()
    }

    /// Returns value at `idx`
    pub fn get<T: FromSql>(&self, idx: usize) -> Result<T> {
        let arg = self.args[idx];
        let value = unsafe { ValueRef::from_value(arg) };
        FromSql::column_result(value).map_err(|err| match err {
            FromSqlError::InvalidType => Error::InvalidFilterParameterType(idx, value.data_type()),
            FromSqlError::Other(err) => {
                Error::FromSqlConversionFailure(idx, value.data_type(), err)
            }
            FromSqlError::InvalidBlobSize { .. } => {
                Error::FromSqlConversionFailure(idx, value.data_type(), Box::new(err))
            }
            FromSqlError::OutOfRange(i) => Error::IntegralValueOutOfRange(idx, i),
        })
    }

    // `sqlite3_value_type` returns `SQLITE_NULL` for pointer.
    // So it seems not possible to enhance `ValueRef::from_value`.
    #[cfg(feature = "array")]
    fn get_array(&self, idx: usize) -> Option<array::Array> {
        use crate::types::Value;
        let arg = self.args[idx];
        let ptr = unsafe { ffi::sqlite3_value_pointer(arg, array::ARRAY_TYPE) };
        if ptr.is_null() {
            None
        } else {
            Some(unsafe {
                let ptr = ptr as *const Vec<Value>;
                array::Array::increment_strong_count(ptr); // don't consume it
                array::Array::from_raw(ptr)
            })
        }
    }

    /// Turns `Values` into an iterator.
    #[inline]
    #[must_use]
    pub fn iter(&self) -> ValueIter<'_> {
        ValueIter {
            iter: self.args.iter(),
        }
    }
}

impl<'a> IntoIterator for &'a Values<'a> {
    type IntoIter = ValueIter<'a>;
    type Item = ValueRef<'a>;

    #[inline]
    fn into_iter(self) -> ValueIter<'a> {
        self.iter()
    }
}

/// [`Values`] iterator.
pub struct ValueIter<'a> {
    iter: slice::Iter<'a, *mut ffi::sqlite3_value>,
}

impl<'a> Iterator for ValueIter<'a> {
    type Item = ValueRef<'a>;

    #[inline]
    fn next(&mut self) -> Option<ValueRef<'a>> {
        self.iter
            .next()
            .map(|&raw| unsafe { ValueRef::from_value(raw) })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

/// Wrapper to [`UpdateVTab::insert`] arguments
pub struct Inserts<'a> {
    values: Values<'a>,
}
impl<'a> Deref for Inserts<'a> {
    type Target = Values<'a>;

    fn deref(&self) -> &Self::Target {
        &self.values
    }
}
impl Inserts<'_> {
    /// Determine the virtual table conflict policy
    ///
    /// # Safety
    /// This function is unsafe because it uses raw pointer
    #[must_use]
    pub unsafe fn on_conflict(&self, db: *mut ffi::sqlite3) -> ConflictMode {
        ConflictMode::from(unsafe { ffi::sqlite3_vtab_on_conflict(db) })
    }
}

/// Wrapper to [`UpdateVTab::update`] arguments
pub struct Updates<'a> {
    values: Values<'a>,
}
impl<'a> Deref for Updates<'a> {
    type Target = Values<'a>;

    fn deref(&self) -> &Self::Target {
        &self.values
    }
}
impl Updates<'_> {
    /// Returns `true` if and only
    /// - if the column corresponding to `idx` is unchanged by the UPDATE operation that the [`UpdateVTab::update`] method call was invoked to implement
    /// - and if and the prior [`VTabCursor::column`] method call that was invoked to extracted the value for that column returned without setting a result.
    #[inline]
    #[must_use]
    pub fn no_change(&self, idx: usize) -> bool {
        unsafe { ffi::sqlite3_value_nochange(self.values.args[idx]) != 0 }
    }

    /// Determine the virtual table conflict policy
    ///
    /// # Safety
    /// This function is unsafe because it uses raw pointer
    #[must_use]
    pub unsafe fn on_conflict(&self, db: *mut ffi::sqlite3) -> ConflictMode {
        ConflictMode::from(unsafe { ffi::sqlite3_vtab_on_conflict(db) })
    }
}

/// Conflict resolution modes
#[non_exhaustive]
#[derive(Debug, Eq, PartialEq)]
pub enum ConflictMode {
    /// SQLITE_ROLLBACK
    Rollback,
    /// SQLITE_IGNORE
    Ignore,
    /// SQLITE_FAIL
    Fail,
    /// SQLITE_ABORT
    Abort,
    /// SQLITE_REPLACE
    Replace,
}
impl From<c_int> for ConflictMode {
    fn from(value: c_int) -> Self {
        match value {
            ffi::SQLITE_ROLLBACK => ConflictMode::Rollback,
            ffi::SQLITE_IGNORE => ConflictMode::Ignore,
            ffi::SQLITE_FAIL => ConflictMode::Fail,
            ffi::SQLITE_ABORT => ConflictMode::Abort,
            ffi::SQLITE_REPLACE => ConflictMode::Replace,
            _ => unreachable!("sqlite3_vtab_on_conflict returned invalid value"),
        }
    }
}

impl Connection {
    /// Register a virtual table implementation.
    ///
    /// Step 3 of [Creating New Virtual Table
    /// Implementations](https://sqlite.org/vtab.html#creating_new_virtual_table_implementations).
    #[inline]
    pub fn create_module<'vtab, T: VTab<'vtab>, M: Name>(
        &self,
        module_name: M,
        module: &'static Module<'vtab, T>,
        aux: Option<T::Aux>,
    ) -> Result<()> {
        self.db.borrow_mut().create_module(module_name, module, aux)
    }
}

impl InnerConnection {
    fn create_module<'vtab, T: VTab<'vtab>, M: Name>(
        &mut self,
        module_name: M,
        module: &'static Module<'vtab, T>,
        aux: Option<T::Aux>,
    ) -> Result<()> {
        use crate::version;
        if version::version_number() < 3_009_000 && module.base.xCreate.is_none() {
            return Err(Error::ModuleError(format!(
                "Eponymous-only virtual table not supported by SQLite version {}",
                version::version()
            )));
        }
        let c_name = module_name.as_cstr()?;
        let r = match aux {
            Some(aux) => {
                let boxed_aux: *mut T::Aux = Box::into_raw(Box::new(aux));
                unsafe {
                    ffi::sqlite3_create_module_v2(
                        self.db(),
                        c_name.as_ptr(),
                        &module.base,
                        boxed_aux.cast::<c_void>(),
                        Some(free_boxed_value::<T::Aux>),
                    )
                }
            }
            None => unsafe {
                ffi::sqlite3_create_module_v2(
                    self.db(),
                    c_name.as_ptr(),
                    &module.base,
                    ptr::null_mut(),
                    None,
                )
            },
        };
        self.decode_result(r)
    }
}

/// Escape double-quote (`"`) character occurrences by
/// doubling them (`""`).
#[must_use]
pub fn escape_double_quote(identifier: &str) -> Cow<'_, str> {
    if identifier.contains('"') {
        // escape quote by doubling them
        Owned(identifier.replace('"', "\"\""))
    } else {
        Borrowed(identifier)
    }
}
/// Dequote string
#[must_use]
pub fn dequote(s: &str) -> &str {
    if s.len() < 2 {
        return s;
    }
    match s.bytes().next() {
        Some(b) if b == b'"' || b == b'\'' => match s.bytes().next_back() {
            Some(e) if e == b => &s[1..s.len() - 1], // FIXME handle inner escaped quote(s)
            _ => s,
        },
        _ => s,
    }
}
/// The boolean can be one of:
/// ```text
/// 1 yes true on
/// 0 no false off
/// ```
#[must_use]
pub fn parse_boolean(s: &str) -> Option<bool> {
    if s.eq_ignore_ascii_case("yes")
        || s.eq_ignore_ascii_case("on")
        || s.eq_ignore_ascii_case("true")
        || s.eq("1")
    {
        Some(true)
    } else if s.eq_ignore_ascii_case("no")
        || s.eq_ignore_ascii_case("off")
        || s.eq_ignore_ascii_case("false")
        || s.eq("0")
    {
        Some(false)
    } else {
        None
    }
}

/// `<param_name>=['"]?<param_value>['"]?` => `(<param_name>, <param_value>)`
pub fn parameter(c_slice: &[u8]) -> Result<(&str, &str)> {
    let arg = std::str::from_utf8(c_slice)?.trim();
    match arg.split_once('=') {
        Some((key, value)) => {
            let param = key.trim();
            let value = dequote(value.trim());
            Ok((param, value))
        }
        _ => Err(Error::ModuleError(format!("illegal argument: '{arg}'"))),
    }
}

unsafe extern "C" fn rust_create<'vtab, T>(
    db: *mut ffi::sqlite3,
    aux: *mut c_void,
    argc: c_int,
    argv: *const *const c_char,
    pp_vtab: *mut *mut sqlite3_vtab,
    err_msg: *mut *mut c_char,
) -> c_int
where
    T: CreateVTab<'vtab>,
{
    let mut conn = VTabConnection(db);
    let aux = aux.cast::<T::Aux>();
    let args = slice::from_raw_parts(argv, argc as usize);
    let vec = args
        .iter()
        .map(|&cs| CStr::from_ptr(cs).to_bytes()) // FIXME .to_str() -> Result<&str, Utf8Error>
        .collect::<Vec<_>>();
    match T::create(&mut conn, aux.as_ref(), &vec[..]) {
        Ok((sql, vtab)) => match std::ffi::CString::new(sql) {
            Ok(c_sql) => {
                let rc = ffi::sqlite3_declare_vtab(db, c_sql.as_ptr());
                if rc == ffi::SQLITE_OK {
                    let boxed_vtab: *mut T = Box::into_raw(Box::new(vtab));
                    *pp_vtab = boxed_vtab.cast::<sqlite3_vtab>();
                    ffi::SQLITE_OK
                } else {
                    let err = error_from_sqlite_code(rc, None);
                    to_sqlite_error(&err, err_msg)
                }
            }
            Err(err) => {
                *err_msg = alloc(&err.to_string());
                ffi::SQLITE_ERROR
            }
        },
        Err(err) => to_sqlite_error(&err, err_msg),
    }
}

unsafe extern "C" fn rust_connect<'vtab, T>(
    db: *mut ffi::sqlite3,
    aux: *mut c_void,
    argc: c_int,
    argv: *const *const c_char,
    pp_vtab: *mut *mut sqlite3_vtab,
    err_msg: *mut *mut c_char,
) -> c_int
where
    T: VTab<'vtab>,
{
    let mut conn = VTabConnection(db);
    let aux = aux.cast::<T::Aux>();
    let args = slice::from_raw_parts(argv, argc as usize);
    let vec = args
        .iter()
        .map(|&cs| CStr::from_ptr(cs).to_bytes()) // FIXME .to_str() -> Result<&str, Utf8Error>
        .collect::<Vec<_>>();
    match T::connect(&mut conn, aux.as_ref(), &vec[..]) {
        Ok((sql, vtab)) => match std::ffi::CString::new(sql) {
            Ok(c_sql) => {
                let rc = ffi::sqlite3_declare_vtab(db, c_sql.as_ptr());
                if rc == ffi::SQLITE_OK {
                    let boxed_vtab: *mut T = Box::into_raw(Box::new(vtab));
                    *pp_vtab = boxed_vtab.cast::<sqlite3_vtab>();
                    ffi::SQLITE_OK
                } else {
                    let err = error_from_sqlite_code(rc, None);
                    to_sqlite_error(&err, err_msg)
                }
            }
            Err(err) => {
                *err_msg = alloc(&err.to_string());
                ffi::SQLITE_ERROR
            }
        },
        Err(err) => to_sqlite_error(&err, err_msg),
    }
}

unsafe extern "C" fn rust_best_index<'vtab, T>(
    vtab: *mut sqlite3_vtab,
    info: *mut ffi::sqlite3_index_info,
) -> c_int
where
    T: VTab<'vtab>,
{
    let vt = vtab.cast::<T>();
    let mut idx_info = IndexInfo(info);
    vtab_error(vtab, (*vt).best_index(&mut idx_info))
}

unsafe extern "C" fn rust_disconnect<'vtab, T>(vtab: *mut sqlite3_vtab) -> c_int
where
    T: VTab<'vtab>,
{
    if vtab.is_null() {
        return ffi::SQLITE_OK;
    }
    let vtab = vtab.cast::<T>();
    drop(Box::from_raw(vtab));
    ffi::SQLITE_OK
}

unsafe extern "C" fn rust_destroy<'vtab, T>(vtab: *mut sqlite3_vtab) -> c_int
where
    T: CreateVTab<'vtab>,
{
    if vtab.is_null() {
        return ffi::SQLITE_OK;
    }
    let vt = vtab.cast::<T>();
    match (*vt).destroy() {
        Ok(_) => {
            drop(Box::from_raw(vt));
            ffi::SQLITE_OK
        }
        err => vtab_error(vtab, err),
    }
}

unsafe extern "C" fn rust_open<'vtab, T>(
    vtab: *mut sqlite3_vtab,
    pp_cursor: *mut *mut sqlite3_vtab_cursor,
) -> c_int
where
    T: VTab<'vtab> + 'vtab,
{
    let vt = vtab.cast::<T>();
    match (*vt).open() {
        Ok(cursor) => {
            let boxed_cursor: *mut T::Cursor = Box::into_raw(Box::new(cursor));
            *pp_cursor = boxed_cursor.cast::<sqlite3_vtab_cursor>();
            ffi::SQLITE_OK
        }
        err => vtab_error(vtab, err),
    }
}

unsafe extern "C" fn rust_close<C>(cursor: *mut sqlite3_vtab_cursor) -> c_int
where
    C: VTabCursor,
{
    let cr = cursor.cast::<C>();
    drop(Box::from_raw(cr));
    ffi::SQLITE_OK
}

unsafe extern "C" fn rust_filter<C>(
    cursor: *mut sqlite3_vtab_cursor,
    idx_num: c_int,
    idx_str: *const c_char,
    argc: c_int,
    argv: *mut *mut ffi::sqlite3_value,
) -> c_int
where
    C: VTabCursor,
{
    use std::str;
    let idx_name = if idx_str.is_null() {
        None
    } else {
        let c_slice = CStr::from_ptr(idx_str).to_bytes();
        Some(str::from_utf8_unchecked(c_slice))
    };
    let args = slice::from_raw_parts_mut(argv, argc as usize);
    let values = Values { args };
    let cr = cursor as *mut C;
    cursor_error(cursor, (*cr).filter(idx_num, idx_name, &Filters { values }))
}

unsafe extern "C" fn rust_next<C>(cursor: *mut sqlite3_vtab_cursor) -> c_int
where
    C: VTabCursor,
{
    let cr = cursor as *mut C;
    cursor_error(cursor, (*cr).next())
}

unsafe extern "C" fn rust_eof<C>(cursor: *mut sqlite3_vtab_cursor) -> c_int
where
    C: VTabCursor,
{
    let cr = cursor.cast::<C>();
    (*cr).eof() as c_int
}

unsafe extern "C" fn rust_column<C>(
    cursor: *mut sqlite3_vtab_cursor,
    ctx: *mut ffi::sqlite3_context,
    i: c_int,
) -> c_int
where
    C: VTabCursor,
{
    let cr = cursor.cast::<C>();
    let mut ctxt = Context(ctx);
    result_error(ctx, (*cr).column(&mut ctxt, i))
}

unsafe extern "C" fn rust_rowid<C>(
    cursor: *mut sqlite3_vtab_cursor,
    p_rowid: *mut ffi::sqlite3_int64,
) -> c_int
where
    C: VTabCursor,
{
    let cr = cursor.cast::<C>();
    match (*cr).rowid() {
        Ok(rowid) => {
            *p_rowid = rowid;
            ffi::SQLITE_OK
        }
        err => cursor_error(cursor, err),
    }
}

unsafe extern "C" fn rust_update<'vtab, T>(
    vtab: *mut sqlite3_vtab,
    argc: c_int,
    argv: *mut *mut ffi::sqlite3_value,
    p_rowid: *mut ffi::sqlite3_int64,
) -> c_int
where
    T: UpdateVTab<'vtab> + 'vtab,
{
    assert!(argc >= 1);
    let args = slice::from_raw_parts_mut(argv, argc as usize);
    let vt = vtab.cast::<T>();
    let r = if args.len() == 1 {
        (*vt).delete(ValueRef::from_value(args[0]))
    } else if ffi::sqlite3_value_type(args[0]) == ffi::SQLITE_NULL {
        // TODO Make the distinction between argv[1] == NULL and argv[1] != NULL ?
        let values = Values { args };
        match (*vt).insert(&Inserts { values }) {
            Ok(rowid) => {
                *p_rowid = rowid;
                Ok(())
            }
            Err(e) => Err(e),
        }
    } else {
        let values = Values { args };
        (*vt).update(&Updates { values })
    };
    vtab_error(vtab, r)
}

unsafe extern "C" fn rust_begin<'vtab, T>(vtab: *mut sqlite3_vtab) -> c_int
where
    T: TransactionVTab<'vtab>,
{
    let vt = vtab.cast::<T>();
    vtab_error(vtab, (*vt).begin())
}
unsafe extern "C" fn rust_sync<'vtab, T>(vtab: *mut sqlite3_vtab) -> c_int
where
    T: TransactionVTab<'vtab>,
{
    let vt = vtab.cast::<T>();
    vtab_error(vtab, (*vt).sync())
}
unsafe extern "C" fn rust_commit<'vtab, T>(vtab: *mut sqlite3_vtab) -> c_int
where
    T: TransactionVTab<'vtab>,
{
    let vt = vtab.cast::<T>();
    vtab_error(vtab, (*vt).commit())
}
unsafe extern "C" fn rust_rollback<'vtab, T>(vtab: *mut sqlite3_vtab) -> c_int
where
    T: TransactionVTab<'vtab>,
{
    let vt = vtab.cast::<T>();
    vtab_error(vtab, (*vt).rollback())
}

unsafe extern "C" fn rust_savepoint<'vtab, T>(vtab: *mut sqlite3_vtab, n: c_int) -> c_int
where
    T: SavepointVTab<'vtab>,
{
    let vt = vtab.cast::<T>();
    vtab_error(vtab, (*vt).savepoint(n))
}

unsafe extern "C" fn rust_release_savepoint<'vtab, T>(vtab: *mut sqlite3_vtab, n: c_int) -> c_int
where
    T: SavepointVTab<'vtab>,
{
    let vt = vtab.cast::<T>();
    vtab_error(vtab, (*vt).release(n))
}

unsafe extern "C" fn rust_rollback_to<'vtab, T>(vtab: *mut sqlite3_vtab, n: c_int) -> c_int
where
    T: SavepointVTab<'vtab>,
{
    let vt = vtab.cast::<T>();
    vtab_error(vtab, (*vt).rollback_to(n))
}

unsafe extern "C" fn rust_rename<'vtab, T>(
    vtab: *mut sqlite3_vtab,
    new_name: *const c_char,
) -> c_int
where
    T: RenameVTab<'vtab>,
{
    let vt = vtab.cast::<T>();
    let name = match CStr::from_ptr(new_name).to_str() {
        Ok(s) => s,
        Err(e) => return vtab_error::<()>(vtab, Err(Error::Utf8Error(e))),
    };
    vtab_error(vtab, (*vt).rename(name))
}

unsafe extern "C" fn rust_shadow_name<T>(suffix: *const c_char) -> c_int
where
    T: ShadowNameVTab,
{
    let suffix_str = match CStr::from_ptr(suffix).to_str() {
        Ok(s) => s,
        Err(_) => return 0,
    };
    T::shadow_name(suffix_str) as c_int
}

#[cfg(feature = "modern_sqlite")]
unsafe extern "C" fn rust_integrity<'vtab, T>(
    vtab: *mut sqlite3_vtab,
    schema: *const c_char,
    table: *const c_char,
    flags: c_int,
    pz_err: *mut *mut c_char,
) -> c_int
where
    T: IntegrityVTab<'vtab>,
{
    let vt = vtab.cast::<T>();
    let schema_str = match CStr::from_ptr(schema).to_str() {
        Ok(s) => s,
        Err(e) => return vtab_error::<()>(vtab, Err(Error::Utf8Error(e))),
    };
    let table_str = match CStr::from_ptr(table).to_str() {
        Ok(s) => s,
        Err(e) => return vtab_error::<()>(vtab, Err(Error::Utf8Error(e))),
    };
    match (*vt).integrity(schema_str, table_str, flags) {
        Ok(None) => ffi::SQLITE_OK,
        Ok(Some(msg)) => {
            *pz_err = alloc(&msg);
            ffi::SQLITE_OK
        }
        Err(Error::SqliteFailure(err, _)) => err.extended_code,
        Err(_) => ffi::SQLITE_ERROR,
    }
}

unsafe extern "C" fn rust_find_function<'vtab, T>(
    vtab: *mut sqlite3_vtab,
    n_arg: c_int,
    z_name: *const c_char,
    px_func: *mut Option<
        unsafe extern "C" fn(
            ctx: *mut ffi::sqlite3_context,
            argc: c_int,
            argv: *mut *mut ffi::sqlite3_value,
        ),
    >,
    pp_arg: *mut *mut c_void,
) -> c_int
where
    T: FindFunctionVTab<'vtab>,
{
    let vt = vtab.cast::<T>();
    let name = match CStr::from_ptr(z_name).to_str() {
        Ok(s) => s,
        Err(_) => return 0,
    };
    match (*vt).find_function(n_arg, name) {
        FindFunctionResult::None => 0,
        FindFunctionResult::Overload { func, user_data } => {
            *px_func = Some(func);
            *pp_arg = user_data;
            1
        }
        FindFunctionResult::Indexable {
            func,
            user_data,
            constraint_op,
        } => {
            *px_func = Some(func);
            *pp_arg = user_data;
            u8::from(constraint_op) as c_int
        }
    }
}

/// Virtual table cursors can set an error message by assigning a string to
/// `zErrMsg`.
unsafe fn cursor_error<T>(cursor: *mut sqlite3_vtab_cursor, result: Result<T>) -> c_int {
    vtab_error((*cursor).pVtab, result)
}

/// Virtual tables can set an error message by assigning a string to
/// `zErrMsg`.
unsafe fn vtab_error<T>(vtab: *mut sqlite3_vtab, result: Result<T>) -> c_int {
    match result {
        Ok(_) => ffi::SQLITE_OK,
        Err(Error::SqliteFailure(err, s)) => {
            if let Some(err_msg) = s {
                set_err_msg(vtab, &err_msg);
            }
            err.extended_code
        }
        Err(err) => {
            set_err_msg(vtab, &err.to_string());
            ffi::SQLITE_ERROR
        }
    }
}

/// Virtual tables methods can set an error message by assigning a string to
/// `zErrMsg`.
#[cold]
unsafe fn set_err_msg(vtab: *mut sqlite3_vtab, err_msg: &str) {
    if !(*vtab).zErrMsg.is_null() {
        ffi::sqlite3_free((*vtab).zErrMsg.cast::<c_void>());
    }
    (*vtab).zErrMsg = alloc(err_msg);
}

/// To raise an error, the `column` method should use this method to set the
/// error message and return the error code.
#[cold]
unsafe fn result_error<T>(ctx: *mut ffi::sqlite3_context, result: Result<T>) -> c_int {
    match result {
        Ok(_) => ffi::SQLITE_OK,
        Err(Error::SqliteFailure(err, s)) => {
            match err.extended_code {
                ffi::SQLITE_TOOBIG => {
                    ffi::sqlite3_result_error_toobig(ctx);
                }
                ffi::SQLITE_NOMEM => {
                    ffi::sqlite3_result_error_nomem(ctx);
                }
                code => {
                    ffi::sqlite3_result_error_code(ctx, code);
                    if let Some(Ok(cstr)) = s.map(|s| str_to_cstring(&s)) {
                        ffi::sqlite3_result_error(ctx, cstr.as_ptr(), -1);
                    }
                }
            };
            err.extended_code
        }
        Err(err) => {
            ffi::sqlite3_result_error_code(ctx, ffi::SQLITE_ERROR);
            if let Ok(cstr) = str_to_cstring(&err.to_string()) {
                ffi::sqlite3_result_error(ctx, cstr.as_ptr(), -1);
            }
            ffi::SQLITE_ERROR
        }
    }
}

#[cfg(feature = "array")]
pub mod array;
#[cfg(feature = "csvtab")]
pub mod csvtab;
#[cfg(feature = "series")]
pub mod series; // SQLite >= 3.9.0
#[cfg(all(test, feature = "modern_sqlite"))]
mod vtablog;

#[cfg(test)]
mod test {
    #[test]
    fn test_dequote() {
        assert_eq!("", super::dequote(""));
        assert_eq!("'", super::dequote("'"));
        assert_eq!("\"", super::dequote("\""));
        assert_eq!("'\"", super::dequote("'\""));
        assert_eq!("", super::dequote("''"));
        assert_eq!("", super::dequote("\"\""));
        assert_eq!("x", super::dequote("'x'"));
        assert_eq!("x", super::dequote("\"x\""));
        assert_eq!("x", super::dequote("x"));
    }
    #[test]
    fn test_parse_boolean() {
        assert_eq!(None, super::parse_boolean(""));
        assert_eq!(Some(true), super::parse_boolean("1"));
        assert_eq!(Some(true), super::parse_boolean("yes"));
        assert_eq!(Some(true), super::parse_boolean("on"));
        assert_eq!(Some(true), super::parse_boolean("true"));
        assert_eq!(Some(false), super::parse_boolean("0"));
        assert_eq!(Some(false), super::parse_boolean("no"));
        assert_eq!(Some(false), super::parse_boolean("off"));
        assert_eq!(Some(false), super::parse_boolean("false"));
    }
    #[test]
    fn test_parse_parameters() {
        assert_eq!(Ok(("key", "value")), super::parameter(b"key='value'"));
        assert_eq!(Ok(("key", "foo=bar")), super::parameter(b"key='foo=bar'"));
    }
}
