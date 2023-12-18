//! Adaptation of https://sqlite.org/loadext.html#programming_loadable_extensions
//!
//! # build
//! ```sh
//! cargo build --example loadable_extension --features "loadable_extension modern_sqlite functions vtab trace"
//! ```
//!
//! # test
//! ```sh
//! sqlite> .log on
//! sqlite> .load target/debug/examples/libloadable_extension.so
//! (28) Rusqlite extension initialized
//! sqlite> SELECT rusqlite_test_function();
//! Rusqlite extension loaded correctly!
//! ```

use rusqlite::functions::FunctionFlags;
use rusqlite::trace::log;
use rusqlite::types::{ToSqlOutput, Value};
use rusqlite::{ffi, sqlite3_extension_init};
use rusqlite::{Connection, Result};

sqlite3_extension_init!(extension_init);

fn extension_init(db: Connection) -> Result<()> {
    db.create_scalar_function(
        "rusqlite_test_function",
        0,
        FunctionFlags::SQLITE_DETERMINISTIC,
        |_ctx| {
            Ok(ToSqlOutput::Owned(Value::Text(
                "Rusqlite extension loaded correctly!".to_string(),
            )))
        },
    )?;
    log(ffi::SQLITE_WARNING, "Rusqlite extension initialized");
    Ok(())
}
