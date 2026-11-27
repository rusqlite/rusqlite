//! Regression tests for panic containment in the `xUpdate` trampoline.
#![cfg(feature = "vtab")]

use rusqlite::vtab::{
    CreateVTab, Filters, IndexInfo, Inserts, Module, UpdateVTab, Updates, VTab, VTabConnection,
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

    fn best_index(&self, _: &mut IndexInfo) -> Result<bool> {
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

    fn insert(&mut self, _: &Inserts<'_>) -> Result<i64> {
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
    fn filter(&mut self, _: c_int, _: Option<&CStr>, _: &Filters<'_>) -> Result<()> {
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

    fn column(&self, ctx: &mut rusqlite::vtab::Context, _: c_int) -> Result<()> {
        ctx.set_result(1)
    }

    fn rowid(&self) -> Result<i64> {
        Ok(1)
    }
}

const MODULE: Module<PanicTab> = Module::update_module();

fn connection(mode: i32) -> Result<Connection> {
    let db = Connection::open_in_memory()?;
    let name = format!("panic{mode}");
    db.create_module::<PanicTab, _>(name.as_str(), &MODULE, Some(mode))?;
    db.execute(&format!("CREATE VIRTUAL TABLE t{mode} USING {name}"), [])?;
    Ok(db)
}

/// Catching callback panics requires unwinding.
#[cfg(panic = "unwind")]
#[test]
fn test_update_panic_is_contained() -> Result<()> {
    let db = connection(2)?;
    db.execute("INSERT INTO t2 VALUES (1)", [])?;
    let err = db.execute("UPDATE t2 SET a = 2", []).unwrap_err();
    assert!(err.to_string().contains("update panicked"), "{err}");
    Ok(())
}

#[test]
fn test_update_in_range_index() -> Result<()> {
    let db = connection(5)?;
    db.execute("INSERT INTO t5 VALUES (1)", [])?;
    db.execute("UPDATE t5 SET a = 2", [])?;
    Ok(())
}
