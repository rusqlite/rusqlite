//! Ensure Virtual tables can be declared outside `rusqlite` crate.
#[cfg(all(feature = "vtab", target_family = "wasm", target_os = "unknown"))]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(feature = "vtab")]
#[test]
fn test_dummy_module() -> rusqlite::Result<()> {
    use rusqlite::vtab::{
        Context, Filters, IndexInfo, Module, VTab, VTabConnection, VTabCursor, sqlite3_vtab,
        sqlite3_vtab_cursor,
    };
    use rusqlite::{Connection, Result, version_number};
    use std::borrow::Cow;
    use std::ffi::CStr;
    use std::marker::PhantomData;
    use std::os::raw::c_int;

    const MODULE: Module<DummyTab> = Module::eponymous_only_module();

    #[repr(C)]
    struct DummyTab {
        /// Base class. Must be first
        base: sqlite3_vtab,
    }

    unsafe impl<'vtab> VTab<'vtab> for DummyTab {
        type Aux = ();
        type Cursor = DummyTabCursor<'vtab>;

        fn connect(
            _: &mut VTabConnection,
            aux: Option<&()>,
            _module_name: &[u8],
            _database_name: &[u8],
            _table_name: &[u8],
            _args: &[&[u8]],
        ) -> Result<(Cow<'static, CStr>, Self)> {
            debug_assert_eq!(aux, None);
            let vtab = Self {
                base: sqlite3_vtab::default(),
            };
            Ok((Cow::Borrowed(c"CREATE TABLE x(value)"), vtab))
        }

        fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
            info.set_estimated_cost(1.);
            Ok(true)
        }

        fn open(&'vtab mut self) -> Result<DummyTabCursor<'vtab>> {
            Ok(DummyTabCursor::default())
        }
    }

    #[derive(Default)]
    #[repr(C)]
    struct DummyTabCursor<'vtab> {
        /// Base class. Must be first
        base: sqlite3_vtab_cursor,
        /// The rowid
        row_id: i64,
        phantom: PhantomData<&'vtab DummyTab>,
    }

    unsafe impl VTabCursor for DummyTabCursor<'_> {
        fn filter(
            &mut self,
            _idx_num: c_int,
            _idx_str: Option<&CStr>,
            _args: &Filters<'_>,
        ) -> Result<()> {
            self.row_id = 1;
            Ok(())
        }

        fn next(&mut self) -> Result<()> {
            self.row_id += 1;
            Ok(())
        }

        fn eof(&self) -> bool {
            self.row_id > 1
        }

        fn column(&self, ctx: &mut Context, _: c_int) -> Result<()> {
            ctx.set_result(self.row_id)
        }

        fn rowid(&self) -> Result<i64> {
            Ok(self.row_id)
        }
    }

    let db = Connection::open_in_memory()?;

    db.create_module::<DummyTab, _>(c"dummy", &MODULE, None)?;

    let version = version_number();
    if version < 3_009_000 {
        return Ok(());
    }

    let mut s = db.prepare("SELECT * FROM dummy()")?;

    let dummy = s.query_row([], |row| row.get::<_, i32>(0))?;
    assert_eq!(1, dummy);
    Ok(())
}

/// `vtab_error` must not turn `extended_code == 0` into `SQLITE_OK`:
/// a failing xOpen would otherwise leave SQLite with a NULL cursor.
#[cfg(feature = "vtab")]
#[test]
fn test_open_error_code_zero_is_not_success() -> rusqlite::Result<()> {
    use rusqlite::ffi;
    use rusqlite::vtab::{
        Context, CreateVTab, Filters, IndexInfo, Module, VTab, VTabConnection, VTabCursor,
        VTabKind, sqlite3_vtab, sqlite3_vtab_cursor,
    };
    use rusqlite::{Connection, Error, ErrorCode, Result};
    use std::borrow::Cow;
    use std::ffi::CStr;
    use std::os::raw::c_int;

    #[repr(C)]
    struct ErrorTab {
        base: sqlite3_vtab,
        code: i32,
    }

    #[repr(C)]
    struct ErrorCursor {
        base: sqlite3_vtab_cursor,
    }

    unsafe impl<'vtab> VTab<'vtab> for ErrorTab {
        type Aux = i32;
        type Cursor = ErrorCursor;

        fn connect(
            _: &mut VTabConnection,
            aux: Option<&i32>,
            _module_name: &[u8],
            _database_name: &[u8],
            _table_name: &[u8],
            _args: &[&[u8]],
        ) -> Result<(Cow<'static, CStr>, Self)> {
            Ok((
                Cow::Borrowed(c"CREATE TABLE x(a)"),
                ErrorTab {
                    base: sqlite3_vtab::default(),
                    code: *aux.unwrap_or(&0),
                },
            ))
        }

        fn best_index(&self, _: &mut IndexInfo) -> Result<bool> {
            Ok(true)
        }

        fn open(&'vtab mut self) -> Result<ErrorCursor> {
            Err(Error::SqliteFailure(
                ffi::Error {
                    code: ErrorCode::InternalMalfunction,
                    extended_code: self.code,
                },
                Some(format!("open failed with code {}", self.code)),
            ))
        }
    }

    impl CreateVTab<'_> for ErrorTab {
        const KIND: VTabKind = VTabKind::Default;
    }

    unsafe impl VTabCursor for ErrorCursor {
        fn filter(&mut self, _: c_int, _: Option<&CStr>, _: &Filters<'_>) -> Result<()> {
            Ok(())
        }
        fn next(&mut self) -> Result<()> {
            Ok(())
        }
        fn eof(&self) -> bool {
            true
        }
        fn column(&self, _: &mut Context, _: c_int) -> Result<()> {
            Ok(())
        }
        fn rowid(&self) -> Result<i64> {
            Ok(0)
        }
    }

    const MODULE: Module<ErrorTab> = Module::read_only_module();
    let db = Connection::open_in_memory()?;
    db.create_module::<ErrorTab, _>(c"errtab", &MODULE, Some(0))?;
    db.execute("CREATE VIRTUAL TABLE t0 USING errtab", [])?;
    let err = db
        .query_row("SELECT a FROM t0", [], |_| Ok(()))
        .unwrap_err();
    let sqlite = err.sqlite_error().expect("SQLite failure");
    assert_eq!(sqlite.extended_code, ffi::SQLITE_ERROR);

    db.create_module::<ErrorTab, _>(c"errtab1", &MODULE, Some(1))?;
    db.execute("CREATE VIRTUAL TABLE t1 USING errtab1", [])?;
    let err = db
        .query_row("SELECT a FROM t1", [], |_| Ok(()))
        .unwrap_err();
    let sqlite = err.sqlite_error().expect("SQLite failure");
    assert_eq!(sqlite.extended_code, 1);
    Ok(())
}
