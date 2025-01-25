extern crate rusqlite;
use rusqlite::{CachedStatement, Connection, Result, Rows};
use self_cell::{self_cell, MutBorrow};

type CachedStatementRef<'a> = CachedStatement<'a>;

// Caveat: single statement at a time for one connection.
// But if you need multiple statements, you can still create your own struct
// with multiple fields (one for each statement).
self_cell!(
    struct OwningStatement {
        owner: MutBorrow<Connection>,
        #[covariant]
        dependent: CachedStatementRef,
    }
);

fn main() -> Result<()> {
    let conn = Connection::open_in_memory()?;

    let mut os = OwningStatement::try_new(MutBorrow::new(conn), |s| {
        s.borrow_mut().prepare_cached("SELECT 1")
    })?;

    let mut rows = os.with_dependent_mut(|_conn, stmt| -> Result<Rows<'_>> { stmt.query([]) })?;
    while let Some(row) = rows.next()? {
        assert_eq!(Ok(1), row.get(0));
    }
    Ok(())
}
