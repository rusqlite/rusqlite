use crate::ffi::loadable_extension_embedded_init; // required feature `loadable_extension_embedded`
use std::marker::PhantomData;
use std::os::raw::{c_char, c_int};

use rusqlite::vtab::{
    eponymous_only_module, sqlite3_vtab, sqlite3_vtab_cursor, Context, IndexInfo, VTab,
    VTabConnection, VTabCursor, Values,
};
use rusqlite::{
    ffi,
    functions::FunctionFlags,
    types::{ToSqlOutput, Value},
};
use rusqlite::{to_sqlite_error, Connection, Result};

/// example_embedded_extension_init is the entry point for this library.
///
/// This crate produces a cdylib that is intended to be embedded within
/// (i.e. linked into) another library that implements the sqlite loadable
/// extension entrypoint.
///
/// In the case of this example code, refer to the `example-c-host-extension`
/// C code to find where this entry point is invoked.
///
/// Note that this interface is private between the host extension and this
/// library - it can have any signature as long as it passes the *sqlite3 db
/// pointer so we can use it to initialize our rusqlite::Connection.
///
/// It does *not* have to return sqlite status codes (such as SQLITE_OK), we
/// just do that here to keep the C extension simple.
///
/// # Safety
///
/// The C host extension must pass a pointer to a valid `sqlite3` struct in
/// `db`` and either null or a pointer to a char* in `pz_err_msg`.
#[no_mangle]
pub unsafe extern "C" fn example_embedded_extension_init(
    db: *mut ffi::sqlite3,
    pz_err_msg: *mut *mut c_char,
) -> c_int {
    loadable_extension_embedded_init();

    let res = example_embedded_init(db);
    if let Err(err) = res {
        return to_sqlite_error(&err, pz_err_msg);
    }

    ffi::SQLITE_OK
}

#[repr(C)]
struct ExampleEmbeddedTab {
    /// Base class. Must be first
    base: sqlite3_vtab,
}

unsafe impl<'vtab> VTab<'vtab> for ExampleEmbeddedTab {
    type Aux = ();
    type Cursor = ExampleEmbeddedTabCursor<'vtab>;

    fn connect(
        _: &mut VTabConnection,
        _aux: Option<&()>,
        _args: &[&[u8]],
    ) -> Result<(String, ExampleEmbeddedTab)> {
        let vtab = ExampleEmbeddedTab {
            base: sqlite3_vtab::default(),
        };
        Ok(("CREATE TABLE x(value TEXT)".to_owned(), vtab))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<()> {
        info.set_estimated_cost(1.);
        Ok(())
    }

    fn open(&'vtab self) -> Result<ExampleEmbeddedTabCursor<'vtab>> {
        Ok(ExampleEmbeddedTabCursor::default())
    }
}

#[derive(Default)]
#[repr(C)]
struct ExampleEmbeddedTabCursor<'vtab> {
    /// Base class. Must be first
    base: sqlite3_vtab_cursor,
    /// The rowid
    row_id: i64,
    phantom: PhantomData<&'vtab ExampleEmbeddedTab>,
}

unsafe impl VTabCursor for ExampleEmbeddedTabCursor<'_> {
    fn filter(
        &mut self,
        _idx_num: c_int,
        _idx_str: Option<&str>,
        _args: &Values<'_>,
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
        ctx.set_result(&"example_embedded_test_value".to_string())
    }

    fn rowid(&self) -> Result<i64> {
        Ok(self.row_id)
    }
}

fn example_embedded_init(db: *mut ffi::sqlite3) -> Result<()> {
    let conn = unsafe { Connection::from_handle(db)? };
    eprintln!("inited example embedded extension module {:?}", db);
    conn.create_scalar_function(
        "example_embedded_test_function",
        0,
        FunctionFlags::SQLITE_DETERMINISTIC,
        |_ctx| {
            Ok(ToSqlOutput::Owned(Value::Text(
                "Example embedded extension loaded correctly!".to_string(),
            )))
        },
    )?;
    conn.create_module::<ExampleEmbeddedTab>("example_embedded", eponymous_only_module::<ExampleEmbeddedTab>(), None)
}
