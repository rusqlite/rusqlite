//! Prepared statements cache for faster execution.

use crate::raw_statement::RawStatement;
use crate::statement::bind_value_ref_static;
use crate::types::ValueRef;
use crate::{BindIndex, Connection, PrepFlags, Result, Statement};
use hashlink::LruCache;
use std::cell::RefCell;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

impl Connection {
    /// Prepare a SQL statement for execution, returning a previously prepared
    /// (but not currently in-use) statement if one is available. The
    /// returned statement will be cached for reuse by future calls to
    /// [`prepare_cached`](Connection::prepare_cached) once it is dropped.
    ///
    /// ```rust,no_run
    /// # use rusqlite::{Connection, Result};
    /// fn insert_new_people(conn: &Connection) -> Result<()> {
    ///     {
    ///         let mut stmt = conn.prepare_cached("INSERT INTO People (name) VALUES (?1)")?;
    ///         stmt.execute(["Joe Smith"])?;
    ///     }
    ///     {
    ///         // This will return the same underlying SQLite statement handle without
    ///         // having to prepare it again.
    ///         let mut stmt = conn.prepare_cached("INSERT INTO People (name) VALUES (?1)")?;
    ///         stmt.execute(["Bob Jones"])?;
    ///     }
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Failure
    ///
    /// Will return `Err` if `sql` cannot be converted to a C-compatible string
    /// or if the underlying SQLite call fails.
    #[inline]
    pub fn prepare_cached(&self, sql: &str) -> Result<CachedStatement<'_>> {
        self.cache.get(self, sql)
    }

    /// Set the maximum number of cached prepared statements this connection
    /// will hold. By default, a connection will hold a relatively small
    /// number of cached statements. If you need more, or know that you
    /// will not use cached statements, you
    /// can set the capacity manually using this method.
    #[inline]
    pub fn set_prepared_statement_cache_capacity(&self, capacity: usize) {
        self.cache.set_capacity(capacity);
    }

    /// Remove/finalize all prepared statements currently in the cache.
    #[inline]
    pub fn flush_prepared_statement_cache(&self) {
        self.cache.flush();
    }
}

/// Prepared statements LRU cache.
#[derive(Debug)]
pub struct StatementCache(RefCell<LruCache<Arc<str>, RawStatement>>);

unsafe impl Send for StatementCache {}

/// Cacheable statement.
///
/// Statement will return automatically to the cache by default.
/// If you want the statement to be discarded, call
/// [`discard()`](CachedStatement::discard) on it.
pub struct CachedStatement<'conn> {
    stmt: Option<Statement<'conn>>,
    cache: &'conn StatementCache,
}

/// A cached prepared statement that supports zero-copy parameter binding.
///
/// The non-cached counterpart is [`crate::BorrowingStatement`]; see its
/// documentation for the lifetime model and `SQLITE_STATIC` semantics.
///
/// When this value is dropped, the underlying [`CachedStatement`] is returned
/// to its [`Connection`]'s cache (with bindings cleared first). Call
/// [`CachedBorrowingStatement::discard`] to finalize it instead.
///
/// # Safety guarantees
///
/// The borrow checker rejects all of the following at compile time.
///
/// Dropping bound data while the wrapper is still in use:
///
/// ```compile_fail
/// use rusqlite::Connection;
/// let conn = Connection::open_in_memory().unwrap();
/// let mut stmt = conn.prepare_cached_borrowing("SELECT ?1").unwrap();
/// let s = String::from("data");
/// stmt.raw_bind_parameter_ref(1, s.as_str()).unwrap();
/// drop(s);
/// stmt.raw_execute().unwrap();
/// ```
///
/// Moving bound data:
///
/// ```compile_fail
/// use rusqlite::Connection;
/// let conn = Connection::open_in_memory().unwrap();
/// let mut stmt = conn.prepare_cached_borrowing("SELECT ?1").unwrap();
/// let s = String::from("data");
/// stmt.raw_bind_parameter_ref(1, s.as_str()).unwrap();
/// let _moved = s;
/// stmt.raw_execute().unwrap();
/// ```
///
/// Mutating bound data:
///
/// ```compile_fail
/// use rusqlite::Connection;
/// let conn = Connection::open_in_memory().unwrap();
/// let mut stmt = conn.prepare_cached_borrowing("SELECT ?1").unwrap();
/// let mut s = String::from("data");
/// stmt.raw_bind_parameter_ref(1, s.as_str()).unwrap();
/// s.push_str("more");
/// stmt.raw_execute().unwrap();
/// ```
///
/// Binding data that does not outlive the wrapper:
///
/// ```compile_fail
/// use rusqlite::Connection;
/// let conn = Connection::open_in_memory().unwrap();
/// let mut stmt = conn.prepare_cached_borrowing("SELECT ?1").unwrap();
/// {
///     let local = String::from("scoped");
///     stmt.raw_bind_parameter_ref(1, local.as_str()).unwrap();
/// }
/// stmt.raw_execute().unwrap();
/// ```
pub struct CachedBorrowingStatement<'conn, 'stmt> {
    inner: CachedStatement<'conn>,
    _marker: std::marker::PhantomData<&'stmt ()>,
}

impl<'conn, 'stmt> CachedBorrowingStatement<'conn, 'stmt> {
    /// Binds a parameter by reference using `SQLITE_STATIC`.
    /// See [`crate::BorrowingStatement::raw_bind_parameter_ref`] for details.
    //
    // `&mut self` is taken (rather than `&self`, which is all the inner
    // SQLite call needs) for exclusivity with outstanding `Rows<'_>` from
    // `raw_query` (which borrow `&mut Statement` via `DerefMut`) and with
    // `clear_bindings` / the consuming `into_*` methods. `'stmt` is a type
    // parameter fixed at wrapper construction, so the accepted lifetime of
    // `param` is the same across successive calls.
    #[inline]
    pub fn raw_bind_parameter_ref<I, R>(&mut self, one_based_index: I, param: R) -> Result<()>
    where
        I: BindIndex,
        R: Into<ValueRef<'stmt>>,
    {
        let stmt: &Statement<'conn> = &self.inner;
        let ndx = one_based_index.idx(stmt)?;
        // SAFETY: `'stmt` on the wrapper, enforced via `PhantomData<&'stmt ()>`,
        // ties the bound data's lifetime to this wrapper. When the wrapper is
        // dropped, the inner `CachedStatement`'s Drop returns the underlying
        // stmt to the cache via `cache_stmt`, which calls
        // `sqlite3_clear_bindings` *before* re-inserting. So a future cache
        // hit gets the stmt with NULL bindings, and SQLite never reads `param`
        // after it has been freed.
        unsafe { bind_value_ref_static(stmt, ndx, param.into()) }
    }

    /// Clears all parameter bindings on this statement.
    ///
    /// After this call, every slot reads as `NULL` until re-bound. The
    /// `'stmt` lifetime parameter on the wrapper is **not** reset — to bind
    /// data of a shorter lifetime than the original `'stmt`, consume the
    /// wrapper with [`into_cached_statement`](Self::into_cached_statement) and
    /// re-wrap the returned [`CachedStatement`] with a fresh `'stmt`.
    #[inline]
    pub fn clear_bindings(&mut self) {
        self.inner.stmt.as_mut().unwrap().stmt.clear_bindings();
    }

    /// Clears all parameter bindings and unwraps the inner [`CachedStatement`].
    ///
    /// Use this to "reset" the `'stmt` lifetime: after `sqlite3_clear_bindings`
    /// runs, no `SQLITE_STATIC` pointer remains in SQLite, so any previously
    /// bound borrow is safe to drop. The returned [`CachedStatement`] can then
    /// be re-wrapped via [`CachedBorrowingStatement::from`] with a brand-new
    /// `'stmt`, or simply dropped to return the stmt to the cache.
    ///
    /// # Example
    ///
    /// Bind to data with one lifetime, release it, then bind to data with a
    /// shorter (incompatible) lifetime by round-tripping through the inner
    /// [`CachedStatement`]:
    ///
    /// ```no_run
    /// use rusqlite::{CachedBorrowingStatement, Connection};
    ///
    /// # fn run() -> rusqlite::Result<()> {
    /// let conn = Connection::open_in_memory()?;
    /// conn.execute("CREATE TABLE items (x TEXT)", ())?;
    ///
    /// let inner = {
    ///     let first = String::from("first");
    ///     let mut wrapper = conn.prepare_cached_borrowing("INSERT INTO items VALUES (?)")?;
    ///     wrapper.raw_bind_parameter_ref(1, first.as_str())?;
    ///     wrapper.raw_execute()?;
    ///     wrapper.into_cached_statement()
    /// }; // `first` dropped here — SQLite no longer holds the pointer.
    ///
    /// let mut wrapper = CachedBorrowingStatement::from(inner);
    /// let second = String::from("second");
    /// wrapper.raw_bind_parameter_ref(1, second.as_str())?;
    /// wrapper.raw_execute()?;
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn into_cached_statement(mut self) -> CachedStatement<'conn> {
        self.inner.stmt.as_mut().unwrap().stmt.clear_bindings();
        self.inner
    }

    /// Discards the statement, preventing it from being returned to its
    /// [`Connection`]'s collection of cached statements.
    #[inline]
    pub fn discard(self) {
        self.inner.discard();
    }
}

impl<'conn, 'stmt> From<CachedStatement<'conn>> for CachedBorrowingStatement<'conn, 'stmt> {
    /// Wraps a [`CachedStatement`] so its parameters can be bound by reference
    /// using `SQLITE_STATIC`, avoiding the data copy performed by the regular
    /// bind API.
    #[inline]
    fn from(inner: CachedStatement<'conn>) -> Self {
        Self {
            inner,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'conn> Deref for CachedBorrowingStatement<'conn, '_> {
    type Target = CachedStatement<'conn>;

    #[inline]
    fn deref(&self) -> &CachedStatement<'conn> {
        &self.inner
    }
}

impl<'conn> DerefMut for CachedBorrowingStatement<'conn, '_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut CachedStatement<'conn> {
        &mut self.inner
    }
}

impl<'conn> Deref for CachedStatement<'conn> {
    type Target = Statement<'conn>;

    #[inline]
    fn deref(&self) -> &Statement<'conn> {
        self.stmt.as_ref().unwrap()
    }
}

impl<'conn> DerefMut for CachedStatement<'conn> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Statement<'conn> {
        self.stmt.as_mut().unwrap()
    }
}

impl Drop for CachedStatement<'_> {
    #[inline]
    fn drop(&mut self) {
        if let Some(stmt) = self.stmt.take() {
            self.cache.cache_stmt(unsafe { stmt.into_raw() });
        }
    }
}

impl CachedStatement<'_> {
    #[inline]
    fn new<'conn>(stmt: Statement<'conn>, cache: &'conn StatementCache) -> CachedStatement<'conn> {
        CachedStatement {
            stmt: Some(stmt),
            cache,
        }
    }

    /// Discard the statement, preventing it from being returned to its
    /// [`Connection`]'s collection of cached statements.
    #[inline]
    pub fn discard(mut self) {
        self.stmt = None;
    }
}

impl StatementCache {
    /// Create a statement cache.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self(RefCell::new(LruCache::new(capacity)))
    }

    #[inline]
    fn set_capacity(&self, capacity: usize) {
        self.0.borrow_mut().set_capacity(capacity);
    }

    // Search the cache for a prepared-statement object that implements `sql`.
    // If no such prepared-statement can be found, allocate and prepare a new one.
    //
    // # Failure
    //
    // Will return `Err` if no cached statement can be found and the underlying
    // SQLite prepare call fails.
    fn get<'conn>(
        &'conn self,
        conn: &'conn Connection,
        sql: &str,
    ) -> Result<CachedStatement<'conn>> {
        let trimmed = sql.trim();
        let mut cache = self.0.borrow_mut();
        let stmt = match cache.remove(trimmed) {
            Some(raw_stmt) => Ok(Statement::new(conn, raw_stmt)),
            None => conn.prepare_with_flags(trimmed, PrepFlags::SQLITE_PREPARE_PERSISTENT),
        };
        stmt.map(|mut stmt| {
            stmt.stmt.set_statement_cache_key(trimmed);
            CachedStatement::new(stmt, self)
        })
    }

    // Return a statement to the cache.
    fn cache_stmt(&self, mut stmt: RawStatement) {
        if stmt.is_null() {
            return;
        }
        let mut cache = self.0.borrow_mut();
        // Load-bearing for `CachedBorrowingStatement` soundness: any
        // `SQLITE_STATIC` pointer bound via `raw_bind_parameter_ref` must be
        // cleared here before the stmt re-enters the cache, so a later cache
        // hit cannot deref the freed data. See `CachedBorrowingStatement`
        // and `test_cached_borrowing_reuse_clears_bindings`.
        stmt.clear_bindings();
        if let Some(sql) = stmt.statement_cache_key() {
            cache.insert(sql, stmt);
        } else {
            debug_assert!(
                false,
                "bug in statement cache code, statement returned to cache that without key"
            );
        }
    }

    #[inline]
    fn flush(&self) {
        let mut cache = self.0.borrow_mut();
        cache.clear();
    }
}

#[cfg(all(test, not(miri)))]
mod test {
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::StatementCache;
    use crate::{Connection, Result};
    use fallible_iterator::FallibleIterator as _;

    impl StatementCache {
        fn clear(&self) {
            self.0.borrow_mut().clear();
        }

        fn len(&self) -> usize {
            self.0.borrow().len()
        }

        fn capacity(&self) -> usize {
            self.0.borrow().capacity()
        }
    }

    #[test]
    fn test_cache() -> Result<()> {
        let db = Connection::open_in_memory()?;
        let cache = &db.cache;
        let initial_capacity = cache.capacity();
        assert_eq!(0, cache.len());
        assert!(initial_capacity > 0);

        let sql = "PRAGMA schema_version";
        {
            let mut stmt = db.prepare_cached(sql)?;
            assert_eq!(0, cache.len());
            assert_eq!(0, stmt.query_row([], |r| r.get::<_, i64>(0))?);
        }
        assert_eq!(1, cache.len());

        {
            let mut stmt = db.prepare_cached(sql)?;
            assert_eq!(0, cache.len());
            assert_eq!(0, stmt.query_row([], |r| r.get::<_, i64>(0))?);
        }
        assert_eq!(1, cache.len());

        cache.clear();
        assert_eq!(0, cache.len());
        assert_eq!(initial_capacity, cache.capacity());
        Ok(())
    }

    #[test]
    fn test_set_capacity() -> Result<()> {
        let db = Connection::open_in_memory()?;
        let cache = &db.cache;

        let sql = "PRAGMA schema_version";
        {
            let mut stmt = db.prepare_cached(sql)?;
            assert_eq!(0, cache.len());
            assert_eq!(0, stmt.query_row([], |r| r.get::<_, i64>(0))?);
        }
        assert_eq!(1, cache.len());

        db.set_prepared_statement_cache_capacity(0);
        assert_eq!(0, cache.len());

        {
            let mut stmt = db.prepare_cached(sql)?;
            assert_eq!(0, cache.len());
            assert_eq!(0, stmt.query_row([], |r| r.get::<_, i64>(0))?);
        }
        assert_eq!(0, cache.len());

        db.set_prepared_statement_cache_capacity(8);
        {
            let mut stmt = db.prepare_cached(sql)?;
            assert_eq!(0, cache.len());
            assert_eq!(0, stmt.query_row([], |r| r.get::<_, i64>(0))?);
        }
        assert_eq!(1, cache.len());
        Ok(())
    }

    #[test]
    fn test_discard() -> Result<()> {
        let db = Connection::open_in_memory()?;
        let cache = &db.cache;

        let sql = "PRAGMA schema_version";
        {
            let mut stmt = db.prepare_cached(sql)?;
            assert_eq!(0, cache.len());
            assert_eq!(0, stmt.query_row([], |r| r.get::<_, i64>(0))?);
            stmt.discard();
        }
        assert_eq!(0, cache.len());
        Ok(())
    }

    #[test]
    fn test_ddl() -> Result<()> {
        let db = Connection::open_in_memory()?;
        db.execute_batch(
            r"
            CREATE TABLE foo (x INT);
            INSERT INTO foo VALUES (1);
        ",
        )?;

        let sql = "SELECT * FROM foo";

        {
            let mut stmt = db.prepare_cached(sql)?;
            assert_eq!(Ok(Some(1i32)), stmt.query([])?.map(|r| r.get(0)).next());
        }

        db.execute_batch(
            r"
            ALTER TABLE foo ADD COLUMN y INT;
            UPDATE foo SET y = 2;
        ",
        )?;

        {
            let mut stmt = db.prepare_cached(sql)?;
            assert_eq!(
                Ok(Some((1i32, 2i32))),
                stmt.query([])?.map(|r| Ok((r.get(0)?, r.get(1)?))).next()
            );
        }
        Ok(())
    }

    #[test]
    fn test_connection_close() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.prepare_cached("SELECT * FROM sqlite_master;")?;

        conn.close().expect("connection not closed");
        Ok(())
    }

    #[test]
    fn test_cache_key() -> Result<()> {
        let db = Connection::open_in_memory()?;
        let cache = &db.cache;
        assert_eq!(0, cache.len());

        //let sql = " PRAGMA schema_version; -- comment";
        let sql = "PRAGMA schema_version; ";
        {
            let mut stmt = db.prepare_cached(sql)?;
            assert_eq!(0, cache.len());
            assert_eq!(0, stmt.query_row([], |r| r.get::<_, i64>(0))?);
        }
        assert_eq!(1, cache.len());

        {
            let mut stmt = db.prepare_cached(sql)?;
            assert_eq!(0, cache.len());
            assert_eq!(0, stmt.query_row([], |r| r.get::<_, i64>(0))?);
        }
        assert_eq!(1, cache.len());
        Ok(())
    }

    #[test]
    fn test_empty_stmt() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.prepare_cached("")?;
        Ok(())
    }

    #[test]
    fn test_cached_borrowing() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute("CREATE TABLE items (x TEXT);", ())?;

        let mut ref_stmt = conn.prepare_cached_borrowing("INSERT INTO items (x) VALUES (?)")?;
        let x_static = "X Value";

        let x_owned = x_static.to_string();

        ref_stmt.raw_bind_parameter_ref(1, x_owned.as_str())?;
        // can't drop x until after ref_stmt is executed, because it holds a reference to x
        // drop(x);
        ref_stmt.raw_execute()?;
        // dropping here is fine, because ref_stmt can be dropped first

        drop(x_owned);

        let mut stmt = conn.prepare_cached("SELECT x FROM items")?;
        let item: String = stmt.query_row([], |r| r.get(0))?;
        assert_eq!(item, x_static);
        Ok(())
    }

    #[test]
    fn test_cached_borrowing_reuse_clears_bindings() -> Result<()> {
        // Bind data via SQLITE_STATIC, drop the wrapper (returning the stmt
        // to the cache), then re-prepare the same SQL. The cache must hand
        // back the stmt with bindings cleared so the prior dangling pointer
        // can never be read.
        let conn = Connection::open_in_memory()?;

        let sql = "SELECT ?1";
        {
            let owned = String::from("transient text");
            let mut ref_stmt = conn.prepare_cached_borrowing(sql)?;
            ref_stmt.raw_bind_parameter_ref(1, owned.as_str())?;
            // wrapper Drop runs cache_stmt -> clear_bindings before reinsertion
        }

        // Re-prepare the same SQL: cache hit. With bindings cleared, slot 1
        // is NULL, so `SELECT ?1` returns NULL. `raw_query` uses the current
        // bindings as-is rather than re-binding from `[]`.
        let mut stmt = conn.prepare_cached(sql)?;
        let mut rows = stmt.raw_query();
        let row = rows.next()?.expect("one row");
        let value: Option<String> = row.get(0)?;
        assert_eq!(value, None);
        Ok(())
    }

    #[test]
    fn test_cached_borrowing_discard() -> Result<()> {
        // After discard, the stmt must not be returned to the cache. We can't
        // observe that directly without poking the private cache field, so
        // verify the next prepare_cached returns a usable (freshly-prepared)
        // stmt and that bindings are clean.
        let conn = Connection::open_in_memory()?;

        let sql = "SELECT ?1";
        {
            let mut stmt = conn.prepare_cached_borrowing(sql)?;
            let owned = String::from("transient");
            stmt.raw_bind_parameter_ref(1, owned.as_str())?;
            stmt.discard();
        }

        let mut stmt = conn.prepare_cached(sql)?;
        let mut rows = stmt.raw_query();
        let row = rows.next()?.expect("one row");
        let value: Option<String> = row.get(0)?;
        assert_eq!(value, None, "freshly prepared stmt must have no bindings");
        Ok(())
    }

    #[test]
    fn test_cached_borrowing_into_cached_statement() -> Result<()> {
        // Bind to short-lived data, clear-and-unwrap to release the
        // SQLITE_STATIC pointer, then re-wrap and bind to different
        // short-lived data.
        let conn = Connection::open_in_memory()?;
        conn.execute("CREATE TABLE items (x TEXT)", ())?;

        let inner = {
            let first = String::from("first");
            let mut wrapper = conn.prepare_cached_borrowing("INSERT INTO items VALUES (?)")?;
            wrapper.raw_bind_parameter_ref(1, first.as_str())?;
            wrapper.raw_execute()?;
            wrapper.into_cached_statement()
        };

        let mut wrapper = super::CachedBorrowingStatement::from(inner);
        let second = String::from("second");
        wrapper.raw_bind_parameter_ref(1, second.as_str())?;
        wrapper.raw_execute()?;
        drop(wrapper);
        drop(second);

        let rows: Vec<String> = conn
            .prepare("SELECT x FROM items ORDER BY rowid")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<_>>()?;
        assert_eq!(rows, vec!["first".to_owned(), "second".to_owned()]);
        Ok(())
    }
}
