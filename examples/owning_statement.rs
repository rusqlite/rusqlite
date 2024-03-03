extern crate rusqlite;
use ouroboros::self_referencing;
use rusqlite::{CachedStatement, Connection, Result, Rows};

#[self_referencing]
struct OwningStatement {
    conn: Connection,
    #[borrows(conn)]
    #[covariant]
    stmt: CachedStatement<'this>,
}

fn main() -> Result<()> {
    let conn = Connection::open_in_memory()?;

    let mut os = OwningStatementTryBuilder {
        conn,
        stmt_builder: |c| c.prepare_cached("SELECT 1"),
    }
    .try_build()?;

    let mut rows = os.with_stmt_mut(|stmt| -> Result<Rows<'_>> { stmt.query([]) })?;
    while let Some(row) = rows.next()? {
        assert_eq!(Ok(1), row.get(0));
    }
    Ok(())
}
