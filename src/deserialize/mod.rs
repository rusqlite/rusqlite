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
//! * Let SQLite mutably borrow a [`SetLenBytes`] using
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

use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::os::raw::{c_int, c_void};
use std::ptr::NonNull;
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
                ffi::SQLITE_DESERIALIZE_FREEONCLOSE | ffi::SQLITE_DESERIALIZE_RESIZEABLE,
            )
        };
        if result.is_ok() {
            mem::forget(data);
        }
        result
    }

    /// Return the serialization of a database, or `None` when [`DatabaseName`] does not exist.
    /// See the C Interface Specification [Serialize a database](https://www.sqlite.org/c3ref/serialize.html).
    pub fn serialize(&self, schema: DatabaseName<'_>) -> Result<Option<MemFile>> {
        // TODO: Optimize copy with hooked_io_methods
        unsafe {
            let c = self.db.borrow();
            let schema = schema.to_cstring()?;
            let mut len = 0;
            let data =
                ffi::sqlite3_serialize(c.db(), schema.as_ptr(), &mut len as *mut _ as *mut _, 0);
            Ok(NonNull::new(data).map(|data| {
                let cap = ffi::sqlite3_msize(data.as_ptr() as _) as _;
                MemFile::from_non_null(data, len, cap)
            }))
        }
    }

    /// Borrow the serialization of a database without copying the memory.
    /// This returns `Ok(None)` when [`DatabaseName`] does not exist or no in-memory serialization is present.
    pub fn serialize_no_copy(&mut self, schema: DatabaseName<'_>) -> Result<Option<&mut [u8]>> {
        let schema = schema.to_cstring()?;
        let c = self.db.borrow();
        Ok(file_ptr(&c, &schema).and_then(|file| {
            if file.pMethods != hooked_io_methods() && file.pMethods != sqlite_io_methods() {
                return None;
            }
            let data = file_buffer(file);
            let len = file_length(file);
            unsafe { Some(slice::from_raw_parts_mut(data.as_ptr(), len)) }
        }))
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
        flags: c_int,
    ) -> Result<()> {
        let schema = schema.to_cstring()?;
        let rc = ffi::sqlite3_deserialize(
            self.db(),
            schema.as_ptr(),
            data.as_ptr() as *mut _,
            data.len() as _,
            cap as _,
            flags as _,
        );
        self.decode_result(rc)
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
                ffi::SQLITE_DESERIALIZE_READONLY,
            )
        }
    }

    /// Disconnect from database and reopen as an in-memory database based on a borrowed vector
    /// (pass a `Vec<u8>`, `MemFile` or another type that implements `SetLenBytes`).
    /// If the capacity is reached, SQLite can't reallocate, so it throws [`crate::ErrorCode::DiskFull`].
    /// Before the connection drops, the slice length is updated.
    pub fn deserialize_mut<T>(&mut self, schema: DatabaseName<'a>, data: &'a mut T) -> Result<()>
    where
        T: SetLenBytes + Send,
    {
        let mut c = self.conn.db.borrow_mut();
        unsafe { c.deserialize_with_flags(schema, data, data.capacity(), 0) }.map(|_| {
            borrowing_file_hook(&mut c, &schema, FileType::SetLen(data));
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
            c.deserialize_with_flags(
                schema,
                data,
                data.capacity(),
                ffi::SQLITE_DESERIALIZE_RESIZEABLE,
            )
        }
        .map(|_| {
            borrowing_file_hook(&mut c, &schema, FileType::Resizable(data));
        })
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
            .finish()
    }
}

/// Pointer that should be updated on close.
enum FileType<'a> {
    SetLen(&'a mut dyn SetLenBytes),
    Resizable(&'a mut MemFile),
}

/// `sqlite3_file` subclass that delegates all methods except
/// `xClose` to the original/lower file defined in `memdb.c`.
/// On close, the `data` pointer gets updated.
#[repr(C)]
struct BorrowingFile<'a> {
    methods: *const ffi::sqlite3_io_methods,
    lower: *mut ffi::sqlite3_file,
    data: FileType<'a>,
}

/// Store `data: FileType` in a new `BorrowingFile`, after moving
/// the original file to a new allocation.
fn borrowing_file_hook(c: &mut InnerConnection, schema: &DatabaseName, data: FileType<'_>) {
    unsafe {
        let schema = schema.to_cstring().unwrap();
        let file = file_ptr(c, &schema).unwrap() as *mut _;
        let lower = ffi::sqlite3_malloc(MEM_VFS.1) as *mut ffi::sqlite3_file;
        ptr::copy_nonoverlapping(file as *const u8, lower as *mut u8, MEM_VFS.1 as _);
        debug_assert_eq!((*lower).pMethods, sqlite_io_methods());
        let borrowing = BorrowingFile {
            methods: hooked_io_methods(),
            lower,
            data,
        };
        ptr::write(file as *mut BorrowingFile, borrowing);
    }
}

fn hooked_io_methods() -> *const ffi::sqlite3_io_methods {
    const HOOKED_IO_METHODS: ffi::sqlite3_io_methods = ffi::sqlite3_io_methods {
        iVersion: 3,
        xClose: Some(close),
        xRead: Some(read),
        xWrite: Some(write),
        xTruncate: Some(truncate),
        xSync: Some(sync),
        xFileSize: Some(file_size),
        xLock: Some(lock),
        xUnlock: Some(unlock),
        xCheckReservedLock: None,
        xFileControl: Some(file_control),
        xSectorSize: None,
        xDeviceCharacteristics: Some(device_characteristics),
        xShmMap: None,
        xShmLock: None,
        xShmBarrier: None,
        xShmUnmap: None,
        xFetch: Some(fetch),
        xUnfetch: Some(unfetch),
    };
    &HOOKED_IO_METHODS
}

lazy_static::lazy_static! {
    /// Get `memdb_io_methods` and `szOsFile` for the VFS defined in `memdb.c`
    static ref MEM_VFS: (&'static ffi::sqlite3_io_methods, i32) = unsafe {
        let vfs = &mut *ffi::sqlite3_vfs_find("memdb\0".as_ptr() as _);
        let file = ffi::sqlite3_malloc(vfs.szOsFile) as *mut ffi::sqlite3_file;
        assert!(!file.is_null());
        let mut out_flags = 0;
        let rc = vfs.xOpen.unwrap()(vfs, ptr::null(), file, ffi::SQLITE_OPEN_MAIN_DB, &mut out_flags);
        assert_eq!(rc, ffi::SQLITE_OK);
        let methods = &*(*file).pMethods;
        ffi::sqlite3_free(file as _);
        (methods, vfs.szOsFile)
    };
}

fn sqlite_io_methods() -> *const ffi::sqlite3_io_methods {
    MEM_VFS.0
}

/// This will be called when dropping the `Connection` or
/// when the database gets detached.
unsafe extern "C" fn close(file: *mut ffi::sqlite3_file) -> c_int {
    panic::catch_unwind(|| {
        let borrowing = &mut *(file as *mut BorrowingFile);
        let lower = &mut *borrowing.lower;
        // Update the data pointer
        match &mut borrowing.data {
            FileType::SetLen(d) => {
                d.set_len(file_length(lower));
            }
            FileType::Resizable(d) => {
                let p = file_buffer(lower);
                let cap = ffi::sqlite3_msize(p.as_ptr() as _) as _;
                let new_data = MemFile::from_non_null(p, file_length(lower), cap);
                ptr::write(*d as *mut _, new_data);
            }
        }
        ffi::sqlite3_free(lower as *mut _ as _);
        ffi::SQLITE_OK
    })
    .unwrap_or_else(|e| {
        dbg!(e); // TODO: Pass error message to caller
        ffi::SQLITE_ERROR
    })
}

fn file_ptr<'a>(
    c: &'a InnerConnection,
    schema: &crate::util::SmallCString,
) -> Option<&'a mut ffi::sqlite3_file> {
    unsafe {
        let mut file = MaybeUninit::<&mut ffi::sqlite3_file>::zeroed();
        let rc = ffi::sqlite3_file_control(
            c.db(),
            schema.as_ptr(),
            ffi::SQLITE_FCNTL_FILE_POINTER,
            file.as_mut_ptr() as _,
        );
        if rc != ffi::SQLITE_OK || file.as_ptr().is_null() {
            None
        } else {
            Some(file.assume_init())
        }
    }
}

fn file_length(file: &mut ffi::sqlite3_file) -> usize {
    unsafe {
        let mut size: i64 = 0;
        let rc = (*file.pMethods).xFileSize.map(|c| c(file, &mut size));
        debug_assert_eq!(rc, Some(ffi::SQLITE_OK));
        size as _
    }
}

fn file_buffer(file: &mut ffi::sqlite3_file) -> NonNull<u8> {
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

unsafe fn file_lower(
    file: *mut ffi::sqlite3_file,
) -> (&'static ffi::sqlite3_io_methods, *mut ffi::sqlite3_file) {
    let file = &mut *(*(file as *mut BorrowingFile)).lower;
    (&*file.pMethods, file)
}

// Below are 11 sqlite3_io_methods functions that delegate to the lower file.
unsafe extern "C" fn read(
    file: *mut ffi::sqlite3_file,
    buf: *mut c_void,
    amt: c_int,
    ofst: i64,
) -> c_int {
    let (m, f) = file_lower(file);
    m.xRead.map_or(ffi::SQLITE_ERROR, |c| c(f, buf, amt, ofst))
}
unsafe extern "C" fn write(
    file: *mut ffi::sqlite3_file,
    buf: *const c_void,
    amt: c_int,
    ofst: i64,
) -> c_int {
    let (m, f) = file_lower(file);
    m.xWrite.map_or(ffi::SQLITE_ERROR, |c| c(f, buf, amt, ofst))
}
unsafe extern "C" fn truncate(file: *mut ffi::sqlite3_file, size: i64) -> c_int {
    let (m, f) = file_lower(file);
    m.xTruncate.map_or(ffi::SQLITE_ERROR, |c| c(f, size))
}
unsafe extern "C" fn sync(file: *mut ffi::sqlite3_file, flags: c_int) -> c_int {
    let (m, f) = file_lower(file);
    m.xSync.map_or(ffi::SQLITE_ERROR, |c| c(f, flags))
}
unsafe extern "C" fn file_size(file: *mut ffi::sqlite3_file, size: *mut i64) -> c_int {
    let (m, f) = file_lower(file);
    m.xFileSize.map_or(ffi::SQLITE_ERROR, |c| c(f, size))
}
unsafe extern "C" fn lock(file: *mut ffi::sqlite3_file, lock: c_int) -> c_int {
    let (m, f) = file_lower(file);
    m.xLock.map_or(ffi::SQLITE_ERROR, |c| c(f, lock))
}
unsafe extern "C" fn unlock(file: *mut ffi::sqlite3_file, lock: c_int) -> c_int {
    let (m, f) = file_lower(file);
    m.xUnlock.map_or(ffi::SQLITE_ERROR, |c| c(f, lock))
}
unsafe extern "C" fn file_control(
    file: *mut ffi::sqlite3_file,
    op: c_int,
    arg: *mut c_void,
) -> c_int {
    let (m, f) = file_lower(file);
    m.xFileControl.map_or(ffi::SQLITE_ERROR, |c| c(f, op, arg))
}
unsafe extern "C" fn device_characteristics(file: *mut ffi::sqlite3_file) -> c_int {
    let (m, f) = file_lower(file);
    m.xDeviceCharacteristics.map_or(0, |c| c(f))
}
unsafe extern "C" fn fetch(
    file: *mut ffi::sqlite3_file,
    ofst: i64,
    amt: c_int,
    p: *mut *mut c_void,
) -> c_int {
    let (m, f) = file_lower(file);
    m.xFetch.map_or(ffi::SQLITE_ERROR, |c| c(f, ofst, amt, p))
}
unsafe extern "C" fn unfetch(file: *mut ffi::sqlite3_file, ofst: i64, p: *mut c_void) -> c_int {
    let (m, f) = file_lower(file);
    m.xUnfetch.map_or(ffi::SQLITE_ERROR, |c| c(f, ofst, p))
}

/// Vector of bytes where the length can be modified.
/// [`BorrowingConnection`] functions use this to borrow memory from arbitrary allocators.
pub trait SetLenBytes: ops::Deref<Target = [u8]> + fmt::Debug {
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
impl SetLenBytes for Vec<u8> {
    unsafe fn set_len(&mut self, new_len: usize) {
        self.set_len(new_len);
    }
    fn capacity(&self) -> usize {
        self.capacity()
    }
}
impl SetLenBytes for MemFile {
    unsafe fn set_len(&mut self, new_len: usize) {
        self.set_len(new_len);
    }
    fn capacity(&self) -> usize {
        self.capacity()
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
                    ffi::SQLITE_DESERIALIZE_READONLY,
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

    #[test]
    fn test_serialize_non_existing_db_name() {
        let mut db = Connection::open_in_memory().unwrap();
        let sql = "BEGIN;
        CREATE TABLE hello(x INTEGER);
        INSERT INTO hello VALUES(1);
        INSERT INTO hello VALUES(2);
        INSERT INTO hello VALUES(3);
        END;";
        db.execute_batch(sql).unwrap();
        assert!(db
            .serialize_no_copy(DatabaseName::Attached("does not exist"))
            .unwrap()
            .is_none());
        assert!(db
            .serialize(DatabaseName::Attached("does not exist"))
            .unwrap()
            .is_none());
        let file = db.serialize(DatabaseName::Main).unwrap().unwrap();
        db.deserialize(DatabaseName::Attached("does not exist"), file)
            .unwrap_err();
    }

    fn serialize_len(conn: &mut Connection) -> usize {
        conn.serialize_no_copy(DatabaseName::Main)
            .unwrap()
            .unwrap()
            .len()
    }
}
