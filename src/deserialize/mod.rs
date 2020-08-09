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
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::os::raw::c_int;
use std::ptr::NonNull;
use std::sync::Mutex;
use std::{fmt, mem, ops, panic, ptr, slice};

use crate::ffi;
use crate::{inner_connection::InnerConnection, Connection, DatabaseName, Result};

mod mem_file;

impl Connection {
    /// Disconnect from database and reopen as an in-memory database based on [`MemFile`].
    pub fn deserialize(&self, schema: DatabaseName<'_>, data: MemFile) -> Result<()> {
        let result = unsafe {
            self.db.borrow_mut().deserialize_with_flags(
                schema,
                &data,
                data.capacity(),
                DeserializeFlags::FREE_ON_CLOSE | DeserializeFlags::RESIZABLE,
            )
        };
        mem::forget(data);
        result
    }

    /// Return the serialization of a database, or `None` when [`DatabaseName`] does not exist.
    pub fn serialize(&self, schema: DatabaseName<'_>) -> Result<Option<MemFile>> {
        unsafe {
            self.db
                .borrow()
                .serialize_with_flags(schema, SerializeFlags::empty())
                .map(|r| {
                    r.map(|(data, len)| {
                        let cap = ffi::sqlite3_msize(data.as_ptr() as _) as _;
                        MemFile::from_non_null(data, len, cap)
                    })
                })
        }
    }

    /// Borrow the serialization of a database without copying the memory.
    /// This returns `Ok(None)` when [`DatabaseName`] does not exist or no in-memory serialization is present.
    pub fn serialize_no_copy(&mut self, schema: DatabaseName<'_>) -> Result<Option<&mut [u8]>> {
        let schema = schema.to_cstring()?;
        let c = self.db.borrow();
        let file = get_file_ptr(&c, &schema)?;
        if file.pMethods != hooked_io_methods() && file.pMethods != sqlite_io_methods() {
            return Ok(None)
        }
        let data = get_file_buf(file);
        let len = get_file_len(file);
        Ok(Some(unsafe { slice::from_raw_parts_mut(data.as_ptr(), len) }))
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
        schema: DatabaseName<'_>,
        data: &[u8],
        cap: usize,
        flags: DeserializeFlags,
    ) -> Result<()> {
        let schema = schema.to_cstring()?;
        let rc = ffi::sqlite3_deserialize(
            self.db(),
            schema.as_ptr(),
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
        schema: DatabaseName<'_>,
        flags: SerializeFlags,
    ) -> Result<Option<(NonNull<u8>, usize)>> {
        let schema = schema.to_cstring()?;
        let mut len = 0;
        let data = ffi::sqlite3_serialize(
            self.db(),
            schema.as_ptr(),
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
    phantom: PhantomData<&'a [u8]>,
}

impl<'a> BorrowingConnection<'a> {
    fn new(conn: Connection) -> Self {
        BorrowingConnection {
            conn,
            phantom: PhantomData,
        }
    }

    /// Disconnect from database and reopen as an read-only in-memory database based on a borrowed slice
    /// (using the flag [`ffi::SQLITE_DESERIALIZE_READONLY`]).
    pub fn deserialize_read_only(&self, schema: DatabaseName<'a>, data: &'a [u8]) -> Result<()> {
        unsafe {
            self.db.borrow_mut().deserialize_with_flags(
                schema,
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
    pub fn deserialize_mut<T>(&mut self, schema: DatabaseName<'a>, data: &'a mut T) -> Result<()>
    where
        T: ResizableBytes + Send,
    {
        let mut c = self.conn.db.borrow_mut();
        unsafe {
            c.deserialize_with_flags(schema, data, data.capacity(), DeserializeFlags::empty())
        }
        .and_then(|_| {
            set_close_hook(
                &mut c,
                &schema,
                Box::new(move |file: &mut ffi::sqlite3_file| unsafe {
                    data.set_len(get_file_len(file));
                }),
            )?;
            Ok(())
        })
    }

    /// Disconnect from database and reopen as an in-memory database based on a borrowed `MemFile`.
    /// If the capacity is reached, SQLite may reallocate the borrowed memory.
    /// Before the connection drops, the `&mut MemFile` pointer, length and capacity are updated.
    pub fn deserialize_resizable(
        &mut self,
        schema: DatabaseName<'a>,
        data: &'a mut MemFile,
    ) -> Result<()> {
        let mut c = self.conn.db.borrow_mut();
        unsafe {
            c.deserialize_with_flags(schema, data, data.capacity(), DeserializeFlags::RESIZABLE)
        }
        .and_then(|_| {
            set_close_hook(
                &mut c,
                &schema,
                Box::new(move |file: &mut ffi::sqlite3_file| unsafe {
                    let p = get_file_buf(file);
                    let cap = ffi::sqlite3_msize(p.as_ptr() as _) as _;
                    let new_data = MemFile::from_non_null(p, get_file_len(file), cap);
                    ptr::write(data as *mut _, new_data);
                }),
            )?;
            Ok(())
        })
    }
}

static mut SQLITE_IO_METHODS: *const ffi::sqlite3_io_methods = ptr::null();
static mut HOOKED_IO_METHODS: Option<ffi::sqlite3_io_methods> = None;
type OnClose = Box<dyn FnOnce(&mut ffi::sqlite3_file) + Send + Sync>;
lazy_static::lazy_static! {
    static ref FILE_BORROW: Mutex<HashMap<usize, OnClose>> = Mutex::new(HashMap::new());
}

fn hooked_io_methods() -> *const ffi::sqlite3_io_methods {
    unsafe { HOOKED_IO_METHODS.as_ref().map_or(ptr::null(), |f| f as *const _) }
}
fn sqlite_io_methods() -> *const ffi::sqlite3_io_methods {
    unsafe { SQLITE_IO_METHODS }
}

fn set_close_hook<'a>(
    c: &mut InnerConnection,
    schema: &DatabaseName,
    on_close: Box<dyn FnOnce(&mut ffi::sqlite3_file) + Send + 'a>,
) -> Result<()> {
    unsafe {
        let schema = schema.to_cstring().unwrap();
        let file = get_file_ptr(c, &schema)?;
        // This is thread-safe because memdb_io_methods in memdb.c
        // is always the same, so it does not matter who wins a
        // potential data race.
        if SQLITE_IO_METHODS.is_null() {
            SQLITE_IO_METHODS = file.pMethods;
        }
        if HOOKED_IO_METHODS.is_none() {
            HOOKED_IO_METHODS = Some(ffi::sqlite3_io_methods {
                xClose: Some(close_fork),
                ..*SQLITE_IO_METHODS
            });
        }
        file.pMethods = HOOKED_IO_METHODS.as_ref().unwrap();
        // TODO: I'm not proud of this HashMap hack fix, maybe we should
        //       register a wrapper VFS so the &mut MemFile can be
        //       stored inside a sqlite3_file subclass.
        // Transmute is needed for the lifetime and Sync differences.
        FILE_BORROW
            .lock()
            .unwrap()
            .insert(file as *const _ as _, mem::transmute(on_close));
        Ok(())
    }
}

unsafe extern "C" fn close_fork(file: *mut ffi::sqlite3_file) -> c_int {
    // This function is called by C, so panicing would be UB.
    panic::catch_unwind(|| {
        FILE_BORROW
            .lock()
            .map(|mut l| {
                debug_assert_eq!(
                    (*file).pMethods,
                    HOOKED_IO_METHODS.as_ref().map_or(ptr::null(), |h| h)
                );
                l.remove(&(file as usize)).map_or(ffi::SQLITE_ERROR, |f| {
                    f(&mut *file);
                    ffi::SQLITE_OK
                })
            })
            .unwrap_or(ffi::SQLITE_ERROR)
    })
    .unwrap_or_else(|_e| {
        // TODO: Don't discard the error message
        ffi::SQLITE_ERROR
    })
}

fn get_file_ptr<'a>(
    c: &'a InnerConnection,
    schema: &crate::util::SmallCString,
) -> Result<&'a mut ffi::sqlite3_file> {
    unsafe {
        let mut file = MaybeUninit::<&mut ffi::sqlite3_file>::zeroed();
        let rc = ffi::sqlite3_file_control(
            c.db(),
            schema.as_ptr(),
            ffi::SQLITE_FCNTL_FILE_POINTER,
            file.as_mut_ptr() as _,
        );
        if rc != ffi::SQLITE_OK {
            return Err(crate::error::error_from_sqlite_code(rc, None));
        }
        debug_assert!(!file.as_ptr().is_null());
        Ok(file.assume_init())
    }
}

fn get_file_len(file: &mut ffi::sqlite3_file) -> usize {
    unsafe {
        let mut size: i64 = 0;
        let rc = (*file.pMethods).xFileSize.map(|f| f(file, &mut size));
        debug_assert_eq!(rc, Some(ffi::SQLITE_OK));
        size as _
    }
}

fn get_file_buf(file: &mut ffi::sqlite3_file) -> NonNull<u8> {
    let fetch: *mut u8 = unsafe {
        // Unfortunately, serialize_no_copy does not work here as the db is already
        // detached, but the sqlite3_file is not yet freed. Because the aData field
        // is private, this hack is needed to get the buffer.
        let mut fetch = MaybeUninit::zeroed();
        let rc = (*file.pMethods).xFetch.unwrap()(file, 0, 0, fetch.as_mut_ptr() as _);
        debug_assert_eq!(rc, ffi::SQLITE_OK);
        let rc = (*file.pMethods).xUnfetch.unwrap()(file, 0, ptr::null_mut());
        debug_assert_eq!(rc, ffi::SQLITE_OK);
        fetch.assume_init()
    };
    NonNull::new(fetch).unwrap()
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
        assert_ne!(0, serialized1.capacity());
        assert_eq!(new_len, serialized1.len());

        // test detach error handling for deserialize_mut
        assert_ne!(0, serialized2.capacity());
        assert!(serialized1[..] == serialized2[..]);
        let mut db4 = Connection::open_in_memory()?.into_borrowing();
        db4.execute_batch("ATTACH DATABASE ':memory:' AS hello")?;
        db4.deserialize_mut(DatabaseName::Attached("hello"), &mut serialized2)?;
        db4.execute_batch("DETACH DATABASE hello")?;
        let debug = format!("{:?}", db4);
        mem::drop(db4);
        assert_ne!(0, serialized2.capacity());
        assert!(serialized1[..] == serialized2[..]);

        // Debug impl
        assert_eq!(
            &debug,
            "BorrowingConnection { conn: Connection { path: Some(\":memory:\") } }"
        );

        Ok(())
    }

    fn serialize_len(conn: &mut Connection) -> usize {
        conn.serialize_no_copy(DatabaseName::Main)
            .unwrap()
            .unwrap()
            .len()
    }
}
