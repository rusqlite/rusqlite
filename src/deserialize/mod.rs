//! `feature = "deserialize"` Serialize and deserialize interfaces.
//!
//! This API is only available when SQLite is compiled with `SQLITE_ENABLE_DESERIALIZE`.
//! These functions create and read a serialized file in-memory, useful on platforms without
//! a real file system like WebAssembly or Cloud Functions.
//!
//! For large in-memory database files, you probably don't want to copy or reallocate
//! because that would temporarily double the required memory. Consider these functions:
//!
//! * While downloading a `.sqlite` file, write the buffers directly to [`MemFile`]
//!   and pass that to [`Connection::deserialize`]
//!   (ownership is tranferred to SQLite without copying).
//! * Borrow the memory from SQLite using [`Connection::serialize_no_copy`].
//! * Let SQLite immutably borrow a large Rust-allocated vector using
//!   [`BorrowingConnection::deserialize_read_only`].
//! * Let SQLite mutably borrow a [`ResizableBytes`] using
//!   [`BorrowingConnection::deserialize_mut`].
//! * Let SQlite mutably borrow a SQLite owned memory using
//!   [`BorrowingConnection::deserialize_resizable`].
//!
//! These operations do copy memory:
//!
//! * Clone [`MemFile`].
//! * Obtain a copy of the file using [`Connection::serialize`].
//!
//! ```
//! # use rusqlite::{Result, Connection, DatabaseName, NO_PARAMS};
//! # fn main() -> Result<()> {
//! let db = Connection::open_in_memory()?;
//! db.execute_batch("CREATE TABLE foo(x INTEGER);INSERT INTO foo VALUES(44)")?;
//! let serialized = db.serialize(DatabaseName::Main)?.unwrap();
//! let mut clone = Connection::open_in_memory()?;
//! clone.deserialize(DatabaseName::Main, serialized)?;
//! let row: u16 = clone.query_row("Select x FROM foo", NO_PARAMS, |r| r.get(0))?;
//! assert_eq!(44, row);
//! # Ok(())
//! # }
//! ```
//!
//! Alternatively, consider using the [Backup API](./backup/).

pub use mem_file::MemFile;

use std::collections::HashMap;
use std::ptr::NonNull;
use std::{fmt, mem, ops, ptr, slice};

use crate::ffi;
use crate::{inner_connection::InnerConnection, Connection, DatabaseName, Result};

mod mem_file;

impl Connection {
    /// Disconnect from database and reopen as an in-memory database based on [`MemFile`].
    pub fn deserialize(&self, db: DatabaseName<'_>, data: MemFile) -> Result<()> {
        let result = unsafe {
            self.db.borrow_mut().deserialize_with_flags(
                db,
                &data,
                data.capacity(),
                DeserializeFlags::FREE_ON_CLOSE | DeserializeFlags::RESIZABLE,
            )
        };
        mem::forget(data);
        result
    }

    /// Return the serialization of a database, or `None` when [`DatabaseName`] does not exist.
    pub fn serialize(&self, db: DatabaseName<'_>) -> Result<Option<MemFile>> {
        unsafe {
            self.db
                .borrow()
                .serialize_with_flags(db, SerializeFlags::empty())
                .map(|r| {
                    r.map(|(data, len)| {
                        let cap = ffi::sqlite3_msize(data.as_ptr() as _) as _;
                        MemFile::from_non_null(data, len, cap)
                    })
                })
        }
    }

    /// Borrow the serialization of a database using the flag [`ffi::SQLITE_SERIALIZE_NOCOPY`].
    /// This returns `Ok(None)` when [`DatabaseName`] does not exist or no in-memory serialization is present.
    pub fn serialize_no_copy(&mut self, db: DatabaseName<'_>) -> Result<Option<&mut [u8]>> {
        unsafe {
            self.db
                .borrow_mut()
                .serialize_with_flags(db, SerializeFlags::NO_COPY)
                .map(|r| r.map(|(data, len)| slice::from_raw_parts_mut(data.as_ptr(), len)))
        }
    }

    /// Wraps the `Connection` in `BorrowingConnection` to connect it to borrowed serialized memory
    /// using [`BorrowingConnection::deserialize_read_only`].
    pub fn into_borrowing(self) -> BorrowingConnection<'static> {
        BorrowingConnection::new(self)
    }
}

impl InnerConnection {
    /// Disconnect from database and reopen as an in-memory database based on a borrowed slice.
    /// If the `DatabaseName` does not exist, return
    /// `SqliteFailure(Error { code: Unknown, extended_code: 1 }, Some("not an error"))`.
    ///
    /// # Safety
    ///
    /// The reference `data` must last for the lifetime of this connection.
    /// `cap` must be the size of the allocation, and `cap >= data.len()`.
    ///
    /// If the data is not mutably borrowed, set [`DeserializeFlags::READ_ONLY`].
    /// If SQLite allocated the memory, consider setting [`DeserializeFlags::FREE_ON_CLOSE`]
    /// and/or [`DeserializeFlags::RESIZABLE`].
    ///
    /// See the C Interface Specification [Deserialize a database](https://www.sqlite.org/c3ref/deserialize.html).
    unsafe fn deserialize_with_flags(
        &mut self,
        db: DatabaseName<'_>,
        data: &[u8],
        cap: usize,
        flags: DeserializeFlags,
    ) -> Result<()> {
        let db = db.to_cstring()?;
        let rc = ffi::sqlite3_deserialize(
            self.db(),
            db.as_ptr(),
            data.as_ptr() as *mut _,
            data.len() as _,
            cap as _,
            flags.bits() as _,
        );
        self.decode_result(rc)
    }

    /// The serialization is the same sequence of bytes which would be written to disk
    /// if the database where backed up to disk.
    ///
    /// Returns `Ok(None)` when [`DatabaseName`] does not exist or malloc/prepare fails.
    ///
    /// # Safety
    ///
    /// If [`SerializeFlags::NO_COPY`] is set, this returns a pointer to memory that SQLite is currently using
    /// (or `Ok(None)` if this is not available).
    /// In that case, the returned `MemFile` mutably borrows `Connection`,
    /// [`std::mem::forget()`] one of them to prevent double free.
    ///
    /// See the C Interface Specification [Serialize a database](https://www.sqlite.org/c3ref/serialize.html).
    unsafe fn serialize_with_flags(
        &self,
        db: DatabaseName<'_>,
        flags: SerializeFlags,
    ) -> Result<Option<(NonNull<u8>, usize)>> {
        let db = db.to_cstring()?;
        let mut len = 0;
        let data = ffi::sqlite3_serialize(
            self.db(),
            db.as_ptr(),
            &mut len as *mut _ as *mut _,
            flags.bits() as _,
        );
        Ok(NonNull::new(data).map(|d| (d, len)))
    }
}

/// Wrap `Connection` with lifetime constraint to borrow from serialized memory.
/// Use [`Connection::into_borrowing`] to obtain one.
pub struct BorrowingConnection<'a> {
    conn: Connection,
    slice_drop: HashMap<DatabaseName<'a>, Box<dyn FnMut(&mut InnerConnection) + 'a + Send>>,
}

impl<'a> BorrowingConnection<'a> {
    fn new(conn: Connection) -> Self {
        BorrowingConnection {
            conn,
            slice_drop: HashMap::new(),
        }
    }

    /// Disconnect from database and reopen as an read-only in-memory database based on a borrowed slice
    /// (using the flag [`ffi::SQLITE_DESERIALIZE_READONLY`]).
    pub fn deserialize_read_only(&self, db: DatabaseName<'a>, data: &'a [u8]) -> Result<()> {
        unsafe {
            self.db.borrow_mut().deserialize_with_flags(
                db,
                data,
                data.len(),
                DeserializeFlags::READ_ONLY,
            )
        }
    }

    /// Disconnect from database and reopen as an in-memory database based on a borrowed vector
    /// (pass a `Vec<u8>`, `MemFile` or another type that implements `ResizableBytes`).
    /// If the capacity is reached, SQLite can't reallocate, so it throws [`crate::ErrorCode::DiskFull`].
    /// Before the connection drops, the slice length is updated.
    /// If the database was detached, the slice will get length zero.
    pub fn deserialize_mut<T>(&mut self, db: DatabaseName<'a>, data: &'a mut T) -> Result<()>
    where
        T: ResizableBytes + Send,
    {
        let data_ptr = data as *mut T;
        let second_data = unsafe { &mut *data_ptr };
        let on_drop = Box::new(move |c: &mut InnerConnection| unsafe {
            let new_len =
                if let Ok(Some((_, len))) = c.serialize_with_flags(db, SerializeFlags::NO_COPY) {
                    len
                } else {
                    // On failure, the safest thing to do is setting the length to zero.
                    // This way no uninitialized memory is exposed.
                    0
                };
            second_data.set_len(new_len);
        });
        let mut c = self.conn.db.borrow_mut();
        if let Some(mut prev) = self.slice_drop.insert(db, on_drop) {
            prev(&mut *c);
        }
        unsafe { c.deserialize_with_flags(db, data, data.capacity(), DeserializeFlags::empty()) }
    }

    /// Disconnect from database and reopen as an in-memory database based on a borrowed `MemFile`.
    /// If the capacity is reached, SQLite may reallocate the borrowed memory.
    /// Before the connection drops, the `&mut MemFile` pointer, length and capacity are updated.
    /// If the database was detached, it will be pointed to a new empty `MemFile`.
    pub fn deserialize_resizable(
        &mut self,
        db: DatabaseName<'a>,
        data: &'a mut MemFile,
    ) -> Result<()> {
        let data_ptr = data as *mut MemFile;
        let second_data = unsafe { &mut *data_ptr };
        let on_drop = Box::new(move |c: &mut InnerConnection| unsafe {
            let new_data =
                if let Ok(Some((p, len))) = c.serialize_with_flags(db, SerializeFlags::NO_COPY) {
                    let cap = ffi::sqlite3_msize(p.as_ptr() as _) as _;
                    MemFile::from_non_null(p, len, cap)
                } else {
                    MemFile::new()
                };
            ptr::write(second_data as *mut _, new_data);
        });
        let mut c = self.conn.db.borrow_mut();
        if let Some(mut prev) = self.slice_drop.insert(db, on_drop) {
            prev(&mut *c);
        }
        unsafe { c.deserialize_with_flags(db, data, data.capacity(), DeserializeFlags::RESIZABLE) }
    }
}

impl Drop for BorrowingConnection<'_> {
    fn drop(&mut self) {
        for on_drop in self.slice_drop.values_mut() {
            on_drop(&mut *self.conn.db.borrow_mut());
        }
    }
}

impl ops::Deref for BorrowingConnection<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.conn
    }
}

impl ops::DerefMut for BorrowingConnection<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

impl fmt::Debug for BorrowingConnection<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BorrowingConnection")
            .field("conn", &self.conn)
            .field("slice_drop", &self.slice_drop.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Resizable vector of bytes.
/// [`BorrowingConnection`] functions use this to borrow memory from arbitrary allocators.
pub trait ResizableBytes: ops::Deref<Target = [u8]> + fmt::Debug {
    /// Forces the length of the vector to new_len.
    ///
    /// # Safety
    /// - `new_len` must be less than or equal to `capacity()`.
    /// - The elements at `old_len..new_len` must be initialized.
    unsafe fn set_len(&mut self, new_len: usize);
    /// The size of the allocation.
    /// It must be safe to write this number of bytes at `as_ptr()`.
    fn capacity(&self) -> usize;
}
impl ResizableBytes for Vec<u8> {
    unsafe fn set_len(&mut self, new_len: usize) {
        self.set_len(new_len);
    }
    fn capacity(&self) -> usize {
        self.capacity()
    }
}
impl ResizableBytes for MemFile {
    unsafe fn set_len(&mut self, new_len: usize) {
        self.set_len(new_len);
    }
    fn capacity(&self) -> usize {
        self.capacity()
    }
}

bitflags::bitflags! {
    /// Flags for [`Connection::deserialize_with_flags`].
    #[repr(C)]
    struct DeserializeFlags: ::std::os::raw::c_int {
        /// The deserialized database should be treated as read-only
        /// [`ffi::SQLITE_DESERIALIZE_READONLY`].
        const READ_ONLY = ffi::SQLITE_DESERIALIZE_READONLY;
        /// SQLite should take ownership of this memory and automatically free it when it has finished using it
        /// [`ffi::SQLITE_DESERIALIZE_FREEONCLOSE`].
        const FREE_ON_CLOSE = ffi::SQLITE_DESERIALIZE_FREEONCLOSE;
        /// SQLite is allowed to grow the size of the database using `sqlite3_realloc64()`
        /// [`ffi::SQLITE_DESERIALIZE_RESIZEABLE].
        const RESIZABLE = ffi::SQLITE_DESERIALIZE_RESIZEABLE;
    }
}

bitflags::bitflags! {
    /// Flags for [`Connection::serialize_with_flags`].
    #[repr(C)]
    struct SerializeFlags: ::std::os::raw::c_int {
        /// Return a reference to the contiguous in-memory database that the connection
        /// currently uses instead of making a copy of the database.
        const NO_COPY = ffi::SQLITE_SERIALIZE_NOCOPY;
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{Connection, DatabaseName, Error, ErrorCode, Result, NO_PARAMS};

    #[test]
    pub fn test_serialize() {
        let mut db = Connection::open_in_memory().unwrap();
        let sql = "BEGIN;
            CREATE TABLE foo(x INTEGER);
            INSERT INTO foo VALUES(1);
            INSERT INTO foo VALUES(2);
            INSERT INTO foo VALUES(3);
            END;";
        db.execute_batch(sql).unwrap();
        let serialized = db.serialize(DatabaseName::Main).unwrap().unwrap();

        // create a new db and import the serialized data
        let mut db2 = Connection::open_in_memory().unwrap();
        db2.deserialize(DatabaseName::Main, serialized).unwrap();
        let mut query = db2.prepare("SELECT x FROM foo").unwrap();
        let results: Result<Vec<u16>> = query
            .query_map(NO_PARAMS, |row| row.get(0))
            .unwrap()
            .collect();
        std::mem::drop(query);
        assert_eq!(vec![1, 2, 3], results.unwrap());
        // should not be read-only
        let sql = "INSERT INTO foo VALUES(4)";
        db2.execute_batch(sql).unwrap();

        // NO_COPY only works on db2
        assert!(db.serialize_no_copy(DatabaseName::Main).unwrap().is_none());
        let borrowed_serialized = db2.serialize_no_copy(DatabaseName::Main).unwrap().unwrap();
        let mut serialized = MemFile::new();
        serialized.extend(borrowed_serialized.iter().cloned());

        // create a third db and import the serialized data
        let db3 = Connection::open_in_memory().unwrap();
        db3.deserialize(DatabaseName::Main, serialized).unwrap();
        let mut query = db3.prepare("SELECT x FROM foo").unwrap();
        let results: Result<Vec<u16>> = query
            .query_map(NO_PARAMS, |row| row.get(0))
            .unwrap()
            .collect();
        assert_eq!(vec![1, 2, 3, 4], results.unwrap());
    }

    #[test]
    pub fn test_deserialize_with_flags() {
        let db = Connection::open_in_memory().unwrap();
        let sql = "BEGIN;
            CREATE TABLE foo(x INTEGER);
            INSERT INTO foo VALUES(1);
            INSERT INTO foo VALUES(2);
            INSERT INTO foo VALUES(3);
            END;";
        db.execute_batch(sql).unwrap();
        let serialized = db.serialize(DatabaseName::Main).unwrap().unwrap();
        // copy to Vec and create new MemFile
        let serialized_vec = Vec::from(&serialized[..]);
        let mut serialized = MemFile::new();
        serialized.extend(serialized_vec);

        // create a new db and import the serialized data
        let db2 = Connection::open_in_memory().unwrap();
        unsafe {
            db2.db
                .borrow_mut()
                .deserialize_with_flags(
                    DatabaseName::Main,
                    &serialized,
                    serialized.capacity(),
                    DeserializeFlags::READ_ONLY,
                )
                .unwrap()
        };
        let mut query = db2.prepare("SELECT x FROM foo").unwrap();
        let results: Result<Vec<u16>> = query
            .query_map(NO_PARAMS, |row| row.get(0))
            .unwrap()
            .collect();
        assert_eq!(vec![1, 2, 3], results.unwrap());
        // should be read-only
        let sql = "INSERT INTO foo VALUES(4)";
        db2.execute_batch(sql).unwrap_err();
    }

    #[test]
    pub fn test_deserialize_read_only() -> Result<()> {
        let sql = "BEGIN;
            CREATE TABLE hello(x INTEGER);
            INSERT INTO hello VALUES(1);
            INSERT INTO hello VALUES(2);
            INSERT INTO hello VALUES(3);
            END;";

        // prepare two named databases
        let one = Connection::open_in_memory()?;
        one.execute_batch(sql)?;
        let serialized_one = one.serialize(DatabaseName::Main)?.unwrap();

        let two = Connection::open_in_memory()?;
        two.execute_batch(sql)?;
        let serialized_two = two.serialize(DatabaseName::Main)?.unwrap();

        // create a new db and import the serialized data
        let db = Connection::open_in_memory()?.into_borrowing();
        db.execute_batch("ATTACH DATABASE ':memory:' AS foo; ATTACH DATABASE ':memory:' AS bar")?;
        db.deserialize_read_only(DatabaseName::Attached("foo"), &serialized_one)?;
        db.deserialize_read_only(DatabaseName::Attached("bar"), &serialized_two)?;
        let mut query = db.prepare("SELECT x FROM foo.hello")?;
        let results: Result<Vec<u16>> = query.query_map(NO_PARAMS, |row| row.get(0))?.collect();
        assert_eq!(vec![1, 2, 3], results?);
        let mut query = db.prepare("SELECT x FROM bar.hello")?;
        let results: Result<Vec<u16>> = query.query_map(NO_PARAMS, |row| row.get(0))?.collect();
        assert_eq!(vec![1, 2, 3], results?);
        // should be read-only
        let sql = "INSERT INTO foo VALUES(4)";
        db.execute_batch(sql).unwrap_err();
        Ok(())
    }

    #[test]
    pub fn test_deserialize_mutable() -> Result<()> {
        let sql = "BEGIN;
            CREATE TABLE hello(x INTEGER);
            INSERT INTO hello VALUES(1);
            INSERT INTO hello VALUES(2);
            INSERT INTO hello VALUES(3);
            END;";
        let db1 = Connection::open_in_memory()?;
        db1.execute_batch(sql)?;
        let mut serialized1 = db1.serialize(DatabaseName::Main)?.unwrap();
        let initial_len = serialized1.len();
        serialized1.reserve(8192);

        // create a new db and mutably borrow the serialized data
        let mut db3 = Connection::open_in_memory()?.into_borrowing();
        db3.deserialize_mut(DatabaseName::Main, &mut serialized1)?;
        // update should not affect length
        db3.execute_batch("UPDATE hello SET x = 44 WHERE x = 3")?;
        let mut query = db3.prepare("SELECT x FROM hello")?;
        let results: Result<Vec<u16>> = query.query_map(NO_PARAMS, |row| row.get(0))?.collect();
        assert_eq!(vec![1, 2, 44], results?);
        mem::drop(query);
        assert_eq!(initial_len, serialize_len(&mut db3));

        // insert data until the length needs to grow
        let count_until_resize = std::iter::repeat(())
            .take_while(|_| {
                db3.execute_batch("INSERT INTO hello VALUES(44);").unwrap();
                serialize_len(&mut db3) == initial_len
            })
            .count();
        assert_eq!(524, count_until_resize);

        // after some time, DiskFull is thrown
        let sql = "INSERT INTO hello VALUES(55);";
        for _i in 0..=509 {
            db3.execute_batch(sql)?;
        }
        if let Err(Error::SqliteFailure(
            ffi::Error {
                code: ErrorCode::DiskFull,
                extended_code: _,
            },
            _,
        )) = db3.execute_batch(sql)
        {
        } else {
            panic!("should return SqliteFailure");
        }
        // connection close should update length of serialized1
        let new_len = serialize_len(&mut db3);
        assert!(new_len > initial_len);
        mem::drop(db3);
        assert_eq!(new_len, serialized1.len());

        Ok(())
    }

    #[test]
    pub fn test_deserialize_resizable() -> Result<()> {
        let sql = "BEGIN;
            CREATE TABLE hello(x INTEGER);
            INSERT INTO hello VALUES(1);
            INSERT INTO hello VALUES(2);
            INSERT INTO hello VALUES(3);
            END;";
        let db1 = Connection::open_in_memory()?;
        db1.execute_batch(sql)?;
        let mut serialized1 = db1.serialize(DatabaseName::Main)?.unwrap();
        let initial_cap = serialized1.capacity();
        let initial_len = serialized1.len();

        // create a new db and mutably borrow the serialized data
        let mut db3 = Connection::open_in_memory()?.into_borrowing();
        db3.deserialize_resizable(DatabaseName::Main, &mut serialized1)?;
        // update should not affect length
        db3.execute_batch("UPDATE hello SET x = 44 WHERE x = 3")?;
        let mut query = db3.prepare("SELECT x FROM hello")?;
        let results: Result<Vec<u16>> = query.query_map(NO_PARAMS, |row| row.get(0))?.collect();
        assert_eq!(vec![1, 2, 44], results?);
        mem::drop(query);
        assert_eq!(initial_len, serialize_len(&mut db3));

        // insert data until the length needs to grow
        let count_until_resize = std::iter::repeat(())
            .take_while(|_| {
                db3.execute_batch("INSERT INTO hello VALUES(44);").unwrap();
                serialize_len(&mut db3) == initial_len
            })
            .count();
        assert_eq!(524, count_until_resize);

        // connection close should update ptr, capacity, length of serialized1
        let new_len = serialize_len(&mut db3);
        assert!(new_len > initial_len);
        mem::drop(db3);
        assert_eq!(new_len, serialized1.len());
        assert!(serialized1.capacity() > initial_cap);
        // serialized1.as_ptr() may differ, but it could also have grown in place
        let mut serialized2 = serialized1.clone();

        // serializing again should work
        db1.execute_batch("ATTACH DATABASE ':memory:' AS three;")?;
        let mut db1 = db1.into_borrowing();
        db1.deserialize_resizable(DatabaseName::Attached("three"), &mut serialized1)?;
        let count: u16 = db1.query_row("SELECT COUNT(*) FROM hello", NO_PARAMS, |r| r.get(0))?;
        assert_eq!(3, count);
        let count: u16 =
            db1.query_row("SELECT COUNT(*) FROM three.hello", NO_PARAMS, |r| r.get(0))?;
        assert_eq!(528, count);

        // test detach error handling for deserialize_resizable
        db1.execute_batch("DETACH DATABASE three")?;
        mem::drop(db1);
        assert_eq!(0, serialized1.capacity());
        assert_eq!(0, serialized1.len());
        assert_eq!(
            std::ptr::NonNull::dangling().as_ptr(),
            serialized1.as_mut_ptr()
        );

        // test detach error handling for deserialize_mut
        assert_ne!(0, serialized2.capacity());
        assert_ne!(0, serialized2.len());
        let mut db4 = Connection::open_in_memory()?.into_borrowing();
        db4.execute_batch("ATTACH DATABASE ':memory:' AS hello")?;
        db4.deserialize_mut(DatabaseName::Attached("hello"), &mut serialized2)?;
        db4.execute_batch("DETACH DATABASE hello")?;
        let debug = format!("{:?}", db4);
        mem::drop(db4);
        assert_ne!(0, serialized2.capacity());
        assert_eq!(0, serialized2.len());

        // Debug impl
        assert_eq!(&debug, "BorrowingConnection { conn: Connection { path: Some(\":memory:\") }, slice_drop: [Attached(\"hello\")] }");

        Ok(())
    }

    fn serialize_len(conn: &mut Connection) -> usize {
        conn.serialize_no_copy(DatabaseName::Main)
            .unwrap()
            .unwrap()
            .len()
    }
}
