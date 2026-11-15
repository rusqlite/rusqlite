#[cfg(feature = "hooks")]
#[test]
fn test_update_hook_no_mem() -> rusqlite::Result<()> {
    use rusqlite::{Connection, ffi};

    let mut db = Connection::open_in_memory()?;
    db.execute_batch("CREATE TABLE t(x)")?;

    // make the next SQLite allocation fail
    unsafe { ffi::sqlite3_hard_heap_limit64(ffi::sqlite3_memory_used()) };
    let s = String::with_capacity(5);
    let r = db.update_hook(Some(move |_, _: &str, _: &str, _| {
        std::hint::black_box(s.len());
    }));
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
