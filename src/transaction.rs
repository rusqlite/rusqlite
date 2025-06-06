use crate::{Connection, Result};
use std::ops::Deref;

/// Options for transaction behavior. See [BEGIN
/// TRANSACTION](http://www.sqlite.org/lang_transaction.html) for details.
#[derive(Copy, Clone)]
#[non_exhaustive]
pub enum TransactionBehavior {
    /// DEFERRED means that the transaction does not actually start until the
    /// database is first accessed.
    Deferred,
    /// IMMEDIATE cause the database connection to start a new write
    /// immediately, without waiting for a writes statement.
    Immediate,
    /// EXCLUSIVE prevents other database connections from reading the database
    /// while the transaction is underway.
    Exclusive,
}

/// Options for how a Transaction or Savepoint should behave when it is dropped.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropBehavior {
    /// Roll back the changes. This is the default.
    Rollback,

    /// Commit the changes.
    Commit,

    /// Do not commit or roll back changes - this will leave the transaction or
    /// savepoint open, so should be used with care.
    Ignore,

    /// Panic. Used to enforce intentional behavior during development.
    Panic,
}

/// Represents a transaction on a database connection.
///
/// ## Note
///
/// Transactions will roll back by default. Use `commit` method to explicitly
/// commit the transaction, or use `set_drop_behavior` to change what happens
/// when the transaction is dropped.
///
/// ## Example
///
/// ```rust,no_run
/// # use rusqlite::{Connection, Result};
/// # fn do_queries_part_1(_conn: &Connection) -> Result<()> { Ok(()) }
/// # fn do_queries_part_2(_conn: &Connection) -> Result<()> { Ok(()) }
/// fn perform_queries(conn: &mut Connection) -> Result<()> {
///     let tx = conn.transaction()?;
///
///     do_queries_part_1(&tx)?; // tx causes rollback if this fails
///     do_queries_part_2(&tx)?; // tx causes rollback if this fails
///
///     tx.commit()
/// }
/// ```
#[derive(Debug)]
pub struct Transaction<'conn> {
    conn: &'conn Connection,
    drop_behavior: DropBehavior,
}

/// Represents a savepoint on a database connection.
///
/// ## Note
///
/// Savepoints will roll back by default. Use `commit` method to explicitly
/// commit the savepoint, or use `set_drop_behavior` to change what happens
/// when the savepoint is dropped.
///
/// ## Example
///
/// ```rust,no_run
/// # use rusqlite::{Connection, Result};
/// # fn do_queries_part_1(_conn: &Connection) -> Result<()> { Ok(()) }
/// # fn do_queries_part_2(_conn: &Connection) -> Result<()> { Ok(()) }
/// fn perform_queries(conn: &mut Connection) -> Result<()> {
///     let sp = conn.savepoint()?;
///
///     do_queries_part_1(&sp)?; // sp causes rollback if this fails
///     do_queries_part_2(&sp)?; // sp causes rollback if this fails
///
///     sp.commit()
/// }
/// ```
#[derive(Debug)]
pub struct Savepoint<'conn> {
    conn: &'conn Connection,
    name: String,
    drop_behavior: DropBehavior,
    committed: bool,
}

impl Transaction<'_> {
    /// Begin a new transaction. Cannot be nested; see `savepoint` for nested
    /// transactions.
    ///
    /// Even though we don't mutate the connection, we take a `&mut Connection`
    /// to prevent nested transactions on the same connection. For cases
    /// where this is unacceptable, [`Transaction::new_unchecked`] is available.
    #[inline]
    pub fn new(conn: &mut Connection, behavior: TransactionBehavior) -> Result<Transaction<'_>> {
        Self::new_unchecked(conn, behavior)
    }

    /// Begin a new transaction, failing if a transaction is open.
    ///
    /// If a transaction is already open, this will return an error. Where
    /// possible, [`Transaction::new`] should be preferred, as it provides a
    /// compile-time guarantee that transactions are not nested.
    #[inline]
    pub fn new_unchecked(
        conn: &Connection,
        behavior: TransactionBehavior,
    ) -> Result<Transaction<'_>> {
        let query = match behavior {
            TransactionBehavior::Deferred => "BEGIN DEFERRED",
            TransactionBehavior::Immediate => "BEGIN IMMEDIATE",
            TransactionBehavior::Exclusive => "BEGIN EXCLUSIVE",
        };
        conn.execute_batch(query).map(move |()| Transaction {
            conn,
            drop_behavior: DropBehavior::Rollback,
        })
    }

    /// Starts a new [savepoint](http://www.sqlite.org/lang_savepoint.html), allowing nested
    /// transactions.
    ///
    /// ## Note
    ///
    /// Just like outer level transactions, savepoint transactions rollback by
    /// default.
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::{Connection, Result};
    /// # fn perform_queries_part_1_succeeds(_conn: &Connection) -> bool { true }
    /// fn perform_queries(conn: &mut Connection) -> Result<()> {
    ///     let mut tx = conn.transaction()?;
    ///
    ///     {
    ///         let sp = tx.savepoint()?;
    ///         if perform_queries_part_1_succeeds(&sp) {
    ///             sp.commit()?;
    ///         }
    ///         // otherwise, sp will rollback
    ///     }
    ///
    ///     tx.commit()
    /// }
    /// ```
    #[inline]
    pub fn savepoint(&mut self) -> Result<Savepoint<'_>> {
        Savepoint::new_(self.conn)
    }

    /// Create a new savepoint with a custom savepoint name. See `savepoint()`.
    #[inline]
    pub fn savepoint_with_name<T: Into<String>>(&mut self, name: T) -> Result<Savepoint<'_>> {
        Savepoint::with_name_(self.conn, name)
    }

    /// Get the current setting for what happens to the transaction when it is
    /// dropped.
    #[inline]
    #[must_use]
    pub fn drop_behavior(&self) -> DropBehavior {
        self.drop_behavior
    }

    /// Configure the transaction to perform the specified action when it is
    /// dropped.
    #[inline]
    pub fn set_drop_behavior(&mut self, drop_behavior: DropBehavior) {
        self.drop_behavior = drop_behavior;
    }

    /// A convenience method which consumes and commits a transaction.
    #[inline]
    pub fn commit(mut self) -> Result<()> {
        self.commit_()
    }

    #[inline]
    fn commit_(&mut self) -> Result<()> {
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    /// A convenience method which consumes and rolls back a transaction.
    #[inline]
    pub fn rollback(mut self) -> Result<()> {
        self.rollback_()
    }

    #[inline]
    fn rollback_(&mut self) -> Result<()> {
        self.conn.execute_batch("ROLLBACK")?;
        Ok(())
    }

    /// Consumes the transaction, committing or rolling back according to the
    /// current setting (see `drop_behavior`).
    ///
    /// Functionally equivalent to the `Drop` implementation, but allows
    /// callers to see any errors that occur.
    #[inline]
    pub fn finish(mut self) -> Result<()> {
        self.finish_()
    }

    #[inline]
    fn finish_(&mut self) -> Result<()> {
        if self.conn.is_autocommit() {
            return Ok(());
        }
        match self.drop_behavior() {
            DropBehavior::Commit => self.commit_().or_else(|_| self.rollback_()),
            DropBehavior::Rollback => self.rollback_(),
            DropBehavior::Ignore => Ok(()),
            DropBehavior::Panic => panic!("Transaction dropped unexpectedly."),
        }
    }
}

impl Deref for Transaction<'_> {
    type Target = Connection;

    #[inline]
    fn deref(&self) -> &Connection {
        self.conn
    }
}

#[expect(unused_must_use)]
impl Drop for Transaction<'_> {
    #[inline]
    fn drop(&mut self) {
        self.finish_();
    }
}

impl Savepoint<'_> {
    #[inline]
    fn with_name_<T: Into<String>>(conn: &Connection, name: T) -> Result<Savepoint<'_>> {
        let name = name.into();
        conn.execute_batch(&format!("SAVEPOINT {name}"))
            .map(|()| Savepoint {
                conn,
                name,
                drop_behavior: DropBehavior::Rollback,
                committed: false,
            })
    }

    #[inline]
    fn new_(conn: &Connection) -> Result<Savepoint<'_>> {
        Savepoint::with_name_(conn, "_rusqlite_sp")
    }

    /// Begin a new savepoint. Can be nested.
    #[inline]
    pub fn new(conn: &mut Connection) -> Result<Savepoint<'_>> {
        Savepoint::new_(conn)
    }

    /// Begin a new savepoint with a user-provided savepoint name.
    #[inline]
    pub fn with_name<T: Into<String>>(conn: &mut Connection, name: T) -> Result<Savepoint<'_>> {
        Savepoint::with_name_(conn, name)
    }

    /// Begin a nested savepoint.
    #[inline]
    pub fn savepoint(&mut self) -> Result<Savepoint<'_>> {
        Savepoint::new_(self.conn)
    }

    /// Begin a nested savepoint with a user-provided savepoint name.
    #[inline]
    pub fn savepoint_with_name<T: Into<String>>(&mut self, name: T) -> Result<Savepoint<'_>> {
        Savepoint::with_name_(self.conn, name)
    }

    /// Get the current setting for what happens to the savepoint when it is
    /// dropped.
    #[inline]
    #[must_use]
    pub fn drop_behavior(&self) -> DropBehavior {
        self.drop_behavior
    }

    /// Configure the savepoint to perform the specified action when it is
    /// dropped.
    #[inline]
    pub fn set_drop_behavior(&mut self, drop_behavior: DropBehavior) {
        self.drop_behavior = drop_behavior;
    }

    /// A convenience method which consumes and commits a savepoint.
    #[inline]
    pub fn commit(mut self) -> Result<()> {
        self.commit_()
    }

    #[inline]
    fn commit_(&mut self) -> Result<()> {
        self.conn.execute_batch(&format!("RELEASE {}", self.name))?;
        self.committed = true;
        Ok(())
    }

    /// A convenience method which rolls back a savepoint.
    ///
    /// ## Note
    ///
    /// Unlike `Transaction`s, savepoints remain active after they have been
    /// rolled back, and can be rolled back again or committed.
    #[inline]
    pub fn rollback(&mut self) -> Result<()> {
        self.conn
            .execute_batch(&format!("ROLLBACK TO {}", self.name))
    }

    /// Consumes the savepoint, committing or rolling back according to the
    /// current setting (see `drop_behavior`).
    ///
    /// Functionally equivalent to the `Drop` implementation, but allows
    /// callers to see any errors that occur.
    #[inline]
    pub fn finish(mut self) -> Result<()> {
        self.finish_()
    }

    #[inline]
    fn finish_(&mut self) -> Result<()> {
        if self.committed {
            return Ok(());
        }
        match self.drop_behavior() {
            DropBehavior::Commit => self
                .commit_()
                .or_else(|_| self.rollback().and_then(|()| self.commit_())),
            DropBehavior::Rollback => self.rollback().and_then(|()| self.commit_()),
            DropBehavior::Ignore => Ok(()),
            DropBehavior::Panic => panic!("Savepoint dropped unexpectedly."),
        }
    }
}

impl Deref for Savepoint<'_> {
    type Target = Connection;

    #[inline]
    fn deref(&self) -> &Connection {
        self.conn
    }
}

#[expect(unused_must_use)]
impl Drop for Savepoint<'_> {
    #[inline]
    fn drop(&mut self) {
        self.finish_();
    }
}

/// Transaction state of a database
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[cfg(feature = "modern_sqlite")] // 3.37.0
pub enum TransactionState {
    /// Equivalent to `SQLITE_TXN_NONE`
    None,
    /// Equivalent to `SQLITE_TXN_READ`
    Read,
    /// Equivalent to `SQLITE_TXN_WRITE`
    Write,
}

impl Connection {
    /// Begin a new transaction with the default behavior (DEFERRED).
    ///
    /// The transaction defaults to rolling back when it is dropped. If you
    /// want the transaction to commit, you must call
    /// [`commit`](Transaction::commit) or
    /// [`set_drop_behavior(DropBehavior::Commit)`](Transaction::set_drop_behavior).
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::{Connection, Result};
    /// # fn do_queries_part_1(_conn: &Connection) -> Result<()> { Ok(()) }
    /// # fn do_queries_part_2(_conn: &Connection) -> Result<()> { Ok(()) }
    /// fn perform_queries(conn: &mut Connection) -> Result<()> {
    ///     let tx = conn.transaction()?;
    ///
    ///     do_queries_part_1(&tx)?; // tx causes rollback if this fails
    ///     do_queries_part_2(&tx)?; // tx causes rollback if this fails
    ///
    ///     tx.commit()
    /// }
    /// ```
    ///
    /// # Failure
    ///
    /// Will return `Err` if the underlying SQLite call fails.
    #[inline]
    pub fn transaction(&mut self) -> Result<Transaction<'_>> {
        Transaction::new(self, self.transaction_behavior)
    }

    /// Begin a new transaction with a specified behavior.
    ///
    /// See [`transaction`](Connection::transaction).
    ///
    /// # Failure
    ///
    /// Will return `Err` if the underlying SQLite call fails.
    #[inline]
    pub fn transaction_with_behavior(
        &mut self,
        behavior: TransactionBehavior,
    ) -> Result<Transaction<'_>> {
        Transaction::new(self, behavior)
    }

    /// Begin a new transaction with the default behavior (DEFERRED).
    ///
    /// Attempt to open a nested transaction will result in a SQLite error.
    /// `Connection::transaction` prevents this at compile time by taking `&mut
    /// self`, but `Connection::unchecked_transaction()` may be used to defer
    /// the checking until runtime.
    ///
    /// See [`Connection::transaction`] and [`Transaction::new_unchecked`]
    /// (which can be used if the default transaction behavior is undesirable).
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::{Connection, Result};
    /// # use std::rc::Rc;
    /// # fn do_queries_part_1(_conn: &Connection) -> Result<()> { Ok(()) }
    /// # fn do_queries_part_2(_conn: &Connection) -> Result<()> { Ok(()) }
    /// fn perform_queries(conn: Rc<Connection>) -> Result<()> {
    ///     let tx = conn.unchecked_transaction()?;
    ///
    ///     do_queries_part_1(&tx)?; // tx causes rollback if this fails
    ///     do_queries_part_2(&tx)?; // tx causes rollback if this fails
    ///
    ///     tx.commit()
    /// }
    /// ```
    ///
    /// # Failure
    ///
    /// Will return `Err` if the underlying SQLite call fails. The specific
    /// error returned if transactions are nested is currently unspecified.
    pub fn unchecked_transaction(&self) -> Result<Transaction<'_>> {
        Transaction::new_unchecked(self, self.transaction_behavior)
    }

    /// Begin a new savepoint with the default behavior (DEFERRED).
    ///
    /// The savepoint defaults to rolling back when it is dropped. If you want
    /// the savepoint to commit, you must call [`commit`](Savepoint::commit) or
    /// [`set_drop_behavior(DropBehavior::Commit)`](Savepoint::set_drop_behavior).
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::{Connection, Result};
    /// # fn do_queries_part_1(_conn: &Connection) -> Result<()> { Ok(()) }
    /// # fn do_queries_part_2(_conn: &Connection) -> Result<()> { Ok(()) }
    /// fn perform_queries(conn: &mut Connection) -> Result<()> {
    ///     let sp = conn.savepoint()?;
    ///
    ///     do_queries_part_1(&sp)?; // sp causes rollback if this fails
    ///     do_queries_part_2(&sp)?; // sp causes rollback if this fails
    ///
    ///     sp.commit()
    /// }
    /// ```
    ///
    /// # Failure
    ///
    /// Will return `Err` if the underlying SQLite call fails.
    #[inline]
    pub fn savepoint(&mut self) -> Result<Savepoint<'_>> {
        Savepoint::new(self)
    }

    /// Begin a new savepoint with a specified name.
    ///
    /// See [`savepoint`](Connection::savepoint).
    ///
    /// # Failure
    ///
    /// Will return `Err` if the underlying SQLite call fails.
    #[inline]
    pub fn savepoint_with_name<T: Into<String>>(&mut self, name: T) -> Result<Savepoint<'_>> {
        Savepoint::with_name(self, name)
    }

    /// Determine the transaction state of a database
    #[cfg(feature = "modern_sqlite")] // 3.37.0
    pub fn transaction_state<N: crate::Name>(
        &self,
        db_name: Option<N>,
    ) -> Result<TransactionState> {
        self.db.borrow().txn_state(db_name)
    }

    /// Set the default transaction behavior for the connection.
    ///
    /// ## Note
    ///
    /// This will only apply to transactions initiated by [`transaction`](Connection::transaction)
    /// or [`unchecked_transaction`](Connection::unchecked_transaction).
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use rusqlite::{Connection, Result, TransactionBehavior};
    /// # fn do_queries_part_1(_conn: &Connection) -> Result<()> { Ok(()) }
    /// # fn do_queries_part_2(_conn: &Connection) -> Result<()> { Ok(()) }
    /// fn perform_queries(conn: &mut Connection) -> Result<()> {
    ///     conn.set_transaction_behavior(TransactionBehavior::Immediate);
    ///
    ///     let tx = conn.transaction()?;
    ///
    ///     do_queries_part_1(&tx)?; // tx causes rollback if this fails
    ///     do_queries_part_2(&tx)?; // tx causes rollback if this fails
    ///
    ///     tx.commit()
    /// }
    /// ```
    pub fn set_transaction_behavior(&mut self, behavior: TransactionBehavior) {
        self.transaction_behavior = behavior;
    }
}

#[cfg(test)]
mod test {
    use super::DropBehavior;
    use crate::{Connection, Error, Result};

    fn checked_memory_handle() -> Result<Connection> {
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE foo (x INTEGER)")?;
        Ok(db)
    }

    #[test]
    fn test_drop() -> Result<()> {
        let mut db = checked_memory_handle()?;
        {
            let tx = db.transaction()?;
            tx.execute_batch("INSERT INTO foo VALUES(1)")?;
            // default: rollback
        }
        {
            let mut tx = db.transaction()?;
            tx.execute_batch("INSERT INTO foo VALUES(2)")?;
            tx.set_drop_behavior(DropBehavior::Commit)
        }
        {
            let tx = db.transaction()?;
            assert_eq!(2, tx.one_column::<i32, _>("SELECT SUM(x) FROM foo", [])?);
        }
        Ok(())
    }
    fn assert_nested_tx_error(e: Error) {
        if let Error::SqliteFailure(e, Some(m)) = &e {
            assert_eq!(e.extended_code, crate::ffi::SQLITE_ERROR);
            // FIXME: Not ideal...
            assert_eq!(e.code, crate::ErrorCode::Unknown);
            assert!(m.contains("transaction"));
        } else {
            panic!("Unexpected error type: {e:?}");
        }
    }

    #[test]
    fn test_unchecked_nesting() -> Result<()> {
        let db = checked_memory_handle()?;

        {
            let tx = db.unchecked_transaction()?;
            let e = tx.unchecked_transaction().unwrap_err();
            assert_nested_tx_error(e);
            // default: rollback
        }
        {
            let tx = db.unchecked_transaction()?;
            tx.execute_batch("INSERT INTO foo VALUES(1)")?;
            // Ensure this doesn't interfere with ongoing transaction
            let e = tx.unchecked_transaction().unwrap_err();
            assert_nested_tx_error(e);

            tx.execute_batch("INSERT INTO foo VALUES(1)")?;
            tx.commit()?;
        }

        assert_eq!(2, db.one_column::<i32, _>("SELECT SUM(x) FROM foo", [])?);
        Ok(())
    }

    #[test]
    fn test_explicit_rollback_commit() -> Result<()> {
        let mut db = checked_memory_handle()?;
        {
            let mut tx = db.transaction()?;
            {
                let mut sp = tx.savepoint()?;
                sp.execute_batch("INSERT INTO foo VALUES(1)")?;
                sp.rollback()?;
                sp.execute_batch("INSERT INTO foo VALUES(2)")?;
                sp.commit()?;
            }
            tx.commit()?;
        }
        {
            let tx = db.transaction()?;
            tx.execute_batch("INSERT INTO foo VALUES(4)")?;
            tx.commit()?;
        }
        {
            let tx = db.transaction()?;
            assert_eq!(6, tx.one_column::<i32, _>("SELECT SUM(x) FROM foo", [])?);
        }
        Ok(())
    }

    #[test]
    fn test_savepoint() -> Result<()> {
        let mut db = checked_memory_handle()?;
        {
            let mut tx = db.transaction()?;
            tx.execute_batch("INSERT INTO foo VALUES(1)")?;
            assert_current_sum(1, &tx)?;
            tx.set_drop_behavior(DropBehavior::Commit);
            {
                let mut sp1 = tx.savepoint()?;
                sp1.execute_batch("INSERT INTO foo VALUES(2)")?;
                assert_current_sum(3, &sp1)?;
                // will roll back sp1
                {
                    let mut sp2 = sp1.savepoint()?;
                    sp2.execute_batch("INSERT INTO foo VALUES(4)")?;
                    assert_current_sum(7, &sp2)?;
                    // will roll back sp2
                    {
                        let sp3 = sp2.savepoint()?;
                        sp3.execute_batch("INSERT INTO foo VALUES(8)")?;
                        assert_current_sum(15, &sp3)?;
                        sp3.commit()?;
                        // committed sp3, but will be erased by sp2 rollback
                    }
                    assert_current_sum(15, &sp2)?;
                }
                assert_current_sum(3, &sp1)?;
            }
            assert_current_sum(1, &tx)?;
        }
        assert_current_sum(1, &db)?;
        Ok(())
    }

    #[test]
    fn test_ignore_drop_behavior() -> Result<()> {
        let mut db = checked_memory_handle()?;

        let mut tx = db.transaction()?;
        {
            let mut sp1 = tx.savepoint()?;
            insert(1, &sp1)?;
            sp1.rollback()?;
            insert(2, &sp1)?;
            {
                let mut sp2 = sp1.savepoint()?;
                sp2.set_drop_behavior(DropBehavior::Ignore);
                insert(4, &sp2)?;
            }
            assert_current_sum(6, &sp1)?;
            sp1.commit()?;
        }
        assert_current_sum(6, &tx)?;
        Ok(())
    }

    #[test]
    fn test_savepoint_drop_behavior_releases() -> Result<()> {
        let mut db = checked_memory_handle()?;

        {
            let mut sp = db.savepoint()?;
            sp.set_drop_behavior(DropBehavior::Commit);
        }
        assert!(db.is_autocommit());
        {
            let mut sp = db.savepoint()?;
            sp.set_drop_behavior(DropBehavior::Rollback);
        }
        assert!(db.is_autocommit());

        Ok(())
    }

    #[test]
    fn test_savepoint_release_error() -> Result<()> {
        let mut db = checked_memory_handle()?;

        db.pragma_update(None, "foreign_keys", true)?;
        db.execute_batch("CREATE TABLE r(n INTEGER PRIMARY KEY NOT NULL); CREATE TABLE f(n REFERENCES r(n) DEFERRABLE INITIALLY DEFERRED);")?;
        {
            let mut sp = db.savepoint()?;
            sp.execute("INSERT INTO f VALUES (0)", [])?;
            sp.set_drop_behavior(DropBehavior::Commit);
        }
        assert!(db.is_autocommit());

        Ok(())
    }

    #[test]
    fn test_savepoint_names() -> Result<()> {
        let mut db = checked_memory_handle()?;

        {
            let mut sp1 = db.savepoint_with_name("my_sp")?;
            insert(1, &sp1)?;
            assert_current_sum(1, &sp1)?;
            {
                let mut sp2 = sp1.savepoint_with_name("my_sp")?;
                sp2.set_drop_behavior(DropBehavior::Commit);
                insert(2, &sp2)?;
                assert_current_sum(3, &sp2)?;
                sp2.rollback()?;
                assert_current_sum(1, &sp2)?;
                insert(4, &sp2)?;
            }
            assert_current_sum(5, &sp1)?;
            sp1.rollback()?;
            {
                let mut sp2 = sp1.savepoint_with_name("my_sp")?;
                sp2.set_drop_behavior(DropBehavior::Ignore);
                insert(8, &sp2)?;
            }
            assert_current_sum(8, &sp1)?;
            sp1.commit()?;
        }
        assert_current_sum(8, &db)?;
        Ok(())
    }

    #[test]
    fn test_rc() -> Result<()> {
        use std::rc::Rc;
        let mut conn = Connection::open_in_memory()?;
        let rc_txn = Rc::new(conn.transaction()?);

        // This will compile only if Transaction is Debug
        Rc::try_unwrap(rc_txn).unwrap();
        Ok(())
    }

    fn insert(x: i32, conn: &Connection) -> Result<usize> {
        conn.execute("INSERT INTO foo VALUES(?1)", [x])
    }

    fn assert_current_sum(x: i32, conn: &Connection) -> Result<()> {
        assert_eq!(x, conn.one_column::<i32, _>("SELECT SUM(x) FROM foo", [])?);
        Ok(())
    }

    #[test]
    #[cfg(feature = "modern_sqlite")]
    fn txn_state() -> Result<()> {
        use super::TransactionState;
        use crate::MAIN_DB;
        let db = Connection::open_in_memory()?;
        assert_eq!(TransactionState::None, db.transaction_state(Some(MAIN_DB))?);
        assert_eq!(TransactionState::None, db.transaction_state::<&str>(None)?);
        db.execute_batch("BEGIN")?;
        assert_eq!(TransactionState::None, db.transaction_state::<&str>(None)?);
        let _: i32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        assert_eq!(TransactionState::Read, db.transaction_state::<&str>(None)?);
        db.pragma_update(None, "user_version", 1)?;
        assert_eq!(TransactionState::Write, db.transaction_state::<&str>(None)?);
        db.execute_batch("ROLLBACK")?;
        Ok(())
    }

    #[test]
    #[cfg(feature = "modern_sqlite")]
    fn auto_commit() -> Result<()> {
        use super::TransactionState;
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE t(i UNIQUE);")?;
        assert!(db.is_autocommit());
        let mut stmt = db.prepare("SELECT name FROM sqlite_master")?;
        assert_eq!(TransactionState::None, db.transaction_state::<&str>(None)?);
        {
            let mut rows = stmt.query([])?;
            assert!(rows.next()?.is_some()); // start reading
            assert_eq!(TransactionState::Read, db.transaction_state::<&str>(None)?);
            db.execute("INSERT INTO t VALUES (1)", [])?; // auto-commit
            assert_eq!(TransactionState::Read, db.transaction_state::<&str>(None)?);
            assert!(rows.next()?.is_some()); // still reading
            assert_eq!(TransactionState::Read, db.transaction_state::<&str>(None)?);
            assert!(rows.next()?.is_none()); // end
            assert_eq!(TransactionState::None, db.transaction_state::<&str>(None)?);
        }
        Ok(())
    }
}
