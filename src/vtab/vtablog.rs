//! Port of C [vtablog](http://www.sqlite.org/cgi/src/finfo?name=ext/misc/vtablog.c)
use std::ffi::c_int;
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::vtab::{
    update_module, Context, CreateVTab, Filters, IndexInfo, Inserts, UpdateVTab, Updates, VTab,
    VTabConnection, VTabCursor, VTabKind,
};
use crate::{ffi, ValueRef};
use crate::{Connection, Error, Result};

/// Register the "vtablog" module.
pub fn load_module(conn: &Connection) -> Result<()> {
    let aux: Option<()> = None;
    conn.create_module(c"vtablog", update_module::<VTabLog>(), aux)
}

/// An instance of the vtablog virtual table
#[repr(C)]
struct VTabLog {
    /// Base class. Must be first
    base: ffi::sqlite3_vtab,
    /// Associated connection
    db: *mut ffi::sqlite3,
    /// Number of rows in the table
    n_row: i64,
    /// Instance number for this vtablog table
    i_inst: usize,
    /// Number of cursors created
    n_cursor: usize,
}

impl VTabLog {
    fn connect_create(
        db: &mut VTabConnection,
        _: Option<&()>,
        args: &[&[u8]],
        is_create: bool,
    ) -> Result<(String, Self)> {
        static N_INST: AtomicUsize = AtomicUsize::new(1);
        let i_inst = N_INST.fetch_add(1, Ordering::SeqCst);
        println!(
            "VTabLog::{}(tab={}, args={:?}):",
            if is_create { "create" } else { "connect" },
            i_inst,
            args.iter().map(|b| str::from_utf8(b)).collect::<Vec<_>>(),
        );
        let mut schema = None;
        let mut n_row = None;

        let args = &args[3..];
        for c_slice in args {
            let (param, value) = super::parameter(c_slice)?;
            match param {
                "schema" => {
                    if schema.is_some() {
                        return Err(Error::ModuleError(format!(
                            "more than one '{param}' parameter"
                        )));
                    }
                    schema = Some(value.to_owned())
                }
                "rows" => {
                    if n_row.is_some() {
                        return Err(Error::ModuleError(format!(
                            "more than one '{param}' parameter"
                        )));
                    }
                    if let Ok(n) = i64::from_str(value) {
                        n_row = Some(n)
                    }
                }
                _ => {
                    return Err(Error::ModuleError(format!(
                        "unrecognized parameter '{param}'"
                    )));
                }
            }
        }
        if schema.is_none() {
            return Err(Error::ModuleError("no schema defined".to_owned()));
        }
        let vtab = Self {
            base: ffi::sqlite3_vtab::default(),
            db: unsafe { db.handle() },
            n_row: n_row.unwrap_or(10),
            i_inst,
            n_cursor: 0,
        };
        Ok((schema.unwrap(), vtab))
    }
}

impl Drop for VTabLog {
    fn drop(&mut self) {
        println!("VTabLog::drop({})", self.i_inst);
    }
}

unsafe impl<'vtab> VTab<'vtab> for VTabLog {
    type Aux = ();
    type Cursor = VTabLogCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Self::Aux>,
        args: &[&[u8]],
    ) -> Result<(String, Self)> {
        Self::connect_create(db, aux, args, false)
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<()> {
        println!(
            "VTabLog::best_index({}, num_of_order_by: {}, col_used: {}, distinct: {:?})",
            self.i_inst,
            info.num_of_order_by(),
            info.col_used(),
            info.distinct()
        );
        info.set_estimated_cost(500.);
        info.set_estimated_rows(500);
        info.set_idx_str("idx");
        info.set_idx_cstr(c"idx");
        Ok(())
    }

    fn open(&'vtab mut self) -> Result<Self::Cursor> {
        self.n_cursor += 1;
        println!(
            "VTabLog::open(tab={}, cursor={})",
            self.i_inst, self.n_cursor
        );
        Ok(VTabLogCursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            i_cursor: self.n_cursor,
            row_id: 0,
            phantom: PhantomData,
        })
    }
}

impl CreateVTab<'_> for VTabLog {
    const KIND: VTabKind = VTabKind::Default;

    fn create(
        db: &mut VTabConnection,
        aux: Option<&Self::Aux>,
        args: &[&[u8]],
    ) -> Result<(String, Self)> {
        Self::connect_create(db, aux, args, true)
    }

    fn destroy(&self) -> Result<()> {
        println!("VTabLog::destroy({})", self.i_inst);
        Ok(())
    }
}

impl UpdateVTab<'_> for VTabLog {
    fn delete(&mut self, arg: ValueRef<'_>) -> Result<()> {
        println!("VTabLog::delete({}, {arg:?})", self.i_inst);
        Ok(())
    }

    fn insert(&mut self, args: &Inserts<'_>) -> Result<i64> {
        println!(
            "VTabLog::insert({}, on_conflict:{:?}, {:?})",
            self.i_inst,
            unsafe { args.on_conflict(self.db) },
            args.iter().collect::<Vec<ValueRef<'_>>>()
        );
        Ok(self.n_row)
    }

    fn update(&mut self, args: &Updates<'_>) -> Result<()> {
        println!(
            "VTabLog::update({}, on_conflict:{:?}, {:?})",
            self.i_inst,
            unsafe { args.on_conflict(self.db) },
            args.iter()
                .enumerate()
                .map(|(i, v)| (v, args.no_change(i)))
                .collect::<Vec<(ValueRef<'_>, bool)>>()
        );
        Ok(())
    }
}

/// A cursor for the Series virtual table
#[repr(C)]
struct VTabLogCursor<'vtab> {
    /// Base class. Must be first
    base: ffi::sqlite3_vtab_cursor,
    /// Cursor number
    i_cursor: usize,
    /// The rowid
    row_id: i64,
    phantom: PhantomData<&'vtab VTabLog>,
}

impl VTabLogCursor<'_> {
    fn vtab(&self) -> &VTabLog {
        unsafe { &*(self.base.pVtab as *const VTabLog) }
    }
}

impl Drop for VTabLogCursor<'_> {
    fn drop(&mut self) {
        println!(
            "VTabLogCursor::drop(tab={}, cursor={})",
            self.vtab().i_inst,
            self.i_cursor
        );
    }
}

unsafe impl VTabCursor for VTabLogCursor<'_> {
    fn filter(&mut self, idx_num: c_int, idx_str: Option<&str>, args: &Filters<'_>) -> Result<()> {
        println!(
            "VTabLogCursor::filter(tab={}, cursor={}, idx_num={idx_num}, idx_str={idx_str:?}, args={})",
            self.vtab().i_inst,
            self.i_cursor,
            args.len()
        );
        self.row_id = 0;
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        println!(
            "VTabLogCursor::next(tab={}, cursor={}): rowid {} -> {}",
            self.vtab().i_inst,
            self.i_cursor,
            self.row_id,
            self.row_id + 1
        );
        self.row_id += 1;
        Ok(())
    }

    fn eof(&self) -> bool {
        let eof = self.row_id >= self.vtab().n_row;
        println!(
            "VTabLogCursor::eof(tab={}, cursor={}): {}",
            self.vtab().i_inst,
            self.i_cursor,
            eof,
        );
        eof
    }

    fn column(&self, ctx: &mut Context, i: c_int) -> Result<()> {
        if ctx.no_change() {
            println!(
                "VTabLogCursor::column(tab={}, cursor={}, i={}): no change",
                self.vtab().i_inst,
                self.i_cursor,
                i,
            );
            return Ok(());
        }
        let value = if i < 26 {
            format!(
                "{}{}",
                "abcdefghijklmnopqrstuvwyz".chars().nth(i as usize).unwrap(),
                self.row_id
            )
        } else {
            format!("{i}{}", self.row_id)
        };
        println!(
            "VTabLogCursor::column(tab={}, cursor={}, i={}): {}",
            self.vtab().i_inst,
            self.i_cursor,
            i,
            value,
        );
        ctx.set_result(&value)
    }

    fn rowid(&self) -> Result<i64> {
        println!(
            "VTabLogCursor::rowid(tab={}, cursor={}): {}",
            self.vtab().i_inst,
            self.i_cursor,
            self.row_id,
        );
        Ok(self.row_id)
    }
}

#[cfg(test)]
mod test {
    use crate::{Connection, Result};
    #[test]
    fn test_module() -> Result<()> {
        let db = Connection::open_in_memory()?;
        super::load_module(&db)?;

        db.execute_batch(
            "CREATE VIRTUAL TABLE temp.log USING vtablog(
                    schema='CREATE TABLE x(a,b,c)',
                    rows=25
                );",
        )?;
        let mut stmt = db.prepare("SELECT * FROM log;")?;
        let mut rows = stmt.query([])?;
        while rows.next()?.is_some() {}
        db.execute("DELETE FROM log WHERE a = ?1", ["a1"])?;
        db.execute(
            "INSERT INTO log (a, b, c) VALUES (?1, ?2, ?3)",
            ["a", "b", "c"],
        )?;
        db.execute(
            "UPDATE log SET b = ?1, c = ?2 WHERE a = ?3",
            ["bn", "cn", "a1"],
        )?;
        Ok(())
    }
}
