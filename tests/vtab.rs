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

/// The three vtab callback trampolines must contain a user callback panic and
/// return `SQLITE_ERROR` instead of aborting at the non-unwinding C ABI boundary.
#[cfg(feature = "vtab")]
#[test]
fn test_vtab_callback_panics_are_contained() -> rusqlite::Result<()> {
    use rusqlite::vtab::{
        Context, CreateVTab, Filters, IndexInfo, Module, UpdateVTab, Updates, VTab, VTabConnection,
        VTabCursor, VTabKind, sqlite3_vtab, sqlite3_vtab_cursor,
    };
    use rusqlite::{Connection, Result};
    use std::borrow::Cow;
    use std::ffi::CStr;
    use std::os::raw::c_int;

    #[repr(C)]
    struct PanicTab {
        base: sqlite3_vtab,
        mode: i32,
    }

    #[repr(C)]
    struct PanicCursor {
        base: sqlite3_vtab_cursor,
        mode: i32,
        done: bool,
    }

    unsafe impl<'vtab> VTab<'vtab> for PanicTab {
        type Aux = i32;
        type Cursor = PanicCursor;

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
                PanicTab {
                    base: sqlite3_vtab::default(),
                    mode: *aux.unwrap_or(&0),
                },
            ))
        }

        fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
            match self.mode {
                0 => {
                    let _ = info.constraint_usage(5);
                }
                3 => {
                    let _ = info.constraint_usage(0);
                }
                4 => {
                    info.constraint_usage(0).set_argv_index(1);
                }
                _ => {}
            }
            Ok(true)
        }

        fn open(&'vtab mut self) -> Result<PanicCursor> {
            Ok(PanicCursor {
                base: sqlite3_vtab_cursor::default(),
                mode: self.mode,
                done: false,
            })
        }
    }

    impl CreateVTab<'_> for PanicTab {
        const KIND: VTabKind = VTabKind::Default;
    }

    impl UpdateVTab<'_> for PanicTab {
        fn delete(&mut self, _: rusqlite::types::ValueRef<'_>) -> Result<()> {
            Ok(())
        }
        fn insert(&mut self, _: &rusqlite::vtab::Inserts<'_>) -> Result<i64> {
            Ok(0)
        }
        fn update(&mut self, updates: &Updates<'_>) -> Result<()> {
            match self.mode {
                2 => {
                    let _ = updates.no_change(999);
                }
                5 => {
                    let _ = updates.no_change(0);
                }
                _ => {}
            }
            Ok(())
        }
    }

    unsafe impl VTabCursor for PanicCursor {
        fn filter(&mut self, _: c_int, _: Option<&CStr>, args: &Filters<'_>) -> Result<()> {
            match self.mode {
                1 => {
                    let _ = args.in_values(1);
                }
                4 => {
                    let _ = args.in_values(0);
                }
                _ => {}
            }
            self.done = false;
            Ok(())
        }
        fn next(&mut self) -> Result<()> {
            self.done = true;
            Ok(())
        }
        fn eof(&self) -> bool {
            self.done
        }
        fn column(&self, ctx: &mut Context, _: c_int) -> Result<()> {
            ctx.set_result(1)
        }
        fn rowid(&self) -> Result<i64> {
            Ok(1)
        }
    }

    const MODULE: Module<PanicTab> = Module::update_module();
    let db = Connection::open_in_memory()?;

    db.create_module::<PanicTab, _>(c"panic0", &MODULE, Some(0))?;
    db.execute("CREATE VIRTUAL TABLE t0 USING panic0", [])?;
    let err = db
        .query_row("SELECT a FROM t0", [], |_| Ok(()))
        .unwrap_err();
    assert!(err.to_string().contains("best_index panicked"), "{err}");

    db.create_module::<PanicTab, _>(c"panic1", &MODULE, Some(1))?;
    db.execute("CREATE VIRTUAL TABLE t1 USING panic1", [])?;
    let err = db
        .query_row("SELECT a FROM t1 WHERE a = 1", [], |_| Ok(()))
        .unwrap_err();
    assert!(err.to_string().contains("filter panicked"), "{err}");

    db.create_module::<PanicTab, _>(c"panic2", &MODULE, Some(2))?;
    db.execute("CREATE VIRTUAL TABLE t2 USING panic2", [])?;
    db.execute("INSERT INTO t2 VALUES (1)", [])?;
    let err = db.execute("UPDATE t2 SET a = 2", []).unwrap_err();
    assert!(err.to_string().contains("update panicked"), "{err}");

    for (name, mode) in [("panic3", 3), ("panic4", 4), ("panic5", 5)] {
        db.create_module::<PanicTab, _>(name, &MODULE, Some(mode))?;
        db.execute(&format!("CREATE VIRTUAL TABLE t{mode} USING {name}"), [])?;
    }
    let row: i64 = db.query_row("SELECT a FROM t3 WHERE a = 1", [], |r| r.get(0))?;
    assert_eq!(row, 1);
    let row: i64 = db.query_row("SELECT a FROM t4 WHERE a = 1", [], |r| r.get(0))?;
    assert_eq!(row, 1);
    db.execute("INSERT INTO t5 VALUES (1)", [])?;
    db.execute("UPDATE t5 SET a = 2", [])?;

    Ok(())
}
