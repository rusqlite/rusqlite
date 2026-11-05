//! Ensure that a callback is not left installed with a dangling pointer when
//! `sqlite3_set_clientdata` fails to allocate.
//! Run in its own process because the heap limit is global.
#[cfg(feature = "hooks")]
#[test]
fn test_update_hook_oom() -> rusqlite::Result<()> {
    use rusqlite::{Connection, ffi};

    let mut db = Connection::open_in_memory()?;
    db.execute_batch("CREATE TABLE t(x)")?;

    // make the next SQLite allocation fail
    unsafe { ffi::sqlite3_hard_heap_limit64(ffi::sqlite3_memory_used()) };
    let r = db.update_hook(Some(|_, _: &str, _: &str, _| {}));
    unsafe { ffi::sqlite3_hard_heap_limit64(0) };
    if r.is_ok() {
        // memory status disabled: allocation did not fail
        return Ok(());
    }
    // no hook must be installed
    let previous = unsafe { ffi::sqlite3_update_hook(db.handle(), None, std::ptr::null_mut()) };
    assert!(previous.is_null());
    db.execute_batch("INSERT INTO t VALUES (1)")?;
    Ok(())
}
