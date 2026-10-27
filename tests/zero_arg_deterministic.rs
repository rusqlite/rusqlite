//! A deterministic function of no arguments, compared against an indexed
//! column after ANALYZE, is called by SQLite's query planner (STAT4) with a
//! null `argv`. `call_boxed_closure` builds `slice::from_raw_parts(argv, 0)`
//! from it, which is undefined behaviour: a debug build aborts on the
//! null-pointer precondition.
#![cfg(all(feature = "bundled", feature = "functions"))]

use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection, Result};

#[test]
fn zero_arg_deterministic_function_called_while_planning() -> Result<()> {
    let db = Connection::open_in_memory()?;
    db.create_scalar_function(
        "zero_args",
        0,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |_| Ok(1),
    )?;
    db.execute_batch(
        "CREATE TABLE t(x INTEGER);
         CREATE INDEX t_x ON t(x);
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100)
         INSERT INTO t SELECT i FROM n;
         ANALYZE;",
    )?;
    let count: i64 = db.query_row("SELECT count(*) FROM t WHERE x = zero_args()", [], |r| {
        r.get(0)
    })?;
    assert_eq!(count, 1);
    Ok(())
}
