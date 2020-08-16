//! `feature = "deserialize"` Serialize and deserialize interfaces, to use a `Vec<u8>`
//! as in-memory database file.
//!
//! This API is only available when SQLite is compiled with `SQLITE_ENABLE_DESERIALIZE`.
//! These functions create and read a serialized file in-memory, useful on platforms without
//! a real file system like WebAssembly or Cloud Functions. Another use case would be to encrypt
//! and decrypt databases in-memory.
//!
//! For large in-memory database files, you probably don't want to copy or reallocate
//! because that would temporarily double the required memory. Use the [`BorrowingConnection`]
//! methods to serialize and deserialize borrowed memory.
//!
//! ```
//! # use rusqlite::{Result, Connection, DatabaseName, NO_PARAMS};
//! # fn main() -> Result<()> {
//! let db = Connection::open_in_memory()?;
//! db.execute_batch("CREATE TABLE one(x INTEGER);INSERT INTO one VALUES(44)")?;
//! let mem_file: Vec<u8> = db.serialize(DatabaseName::Main)?.unwrap();
//! // This SQLite file could be send over the network and deserialized on the other side
//! // without touching the file system.
//! let mut db_clone = Connection::open_in_memory()?;
//! db_clone.deserialize(DatabaseName::Main, mem_file)?;
//! let row: u16 = db_clone.query_row("SELECT x FROM one", NO_PARAMS, |r| r.get(0))?;
//! assert_eq!(44, row);
//! # Ok(())
//! # }
//! ```
//!
//! Alternatively, consider using the [Backup API](./backup/)
//! or the [`VACUUM INTO`](https://www.sqlite.org/lang_vacuum.html) command.

use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::os::raw::{c_char, c_int, c_void};
use std::{borrow::Cow, convert::TryInto, fmt, mem, ops, panic, ptr, rc::Rc};

use crate::ffi;
use crate::{
    error::error_from_handle, inner_connection::InnerConnection, util::SmallCString, Connection,
    DatabaseName, OpenFlags, Result, NO_PARAMS,
};

impl Connection {
    /// Disconnects from database and reopen as an in-memory database based on `Vec<u8>`.
    /// The vector should contain serialized database content (a main database file).
    /// This returns an error if `DatabaseName` was not attached or other failures occurred.
    pub fn deserialize(&self, db: DatabaseName, data: Vec<u8>) -> Result<()> {
        self.deserialize_vec_db(db, MemFile::Owned(data))
    }

    /// Copies the serialization of a database to a `Vec<u8>`, or returns `Ok(None)` when
    /// `DatabaseName` does not exist.
    ///
    /// For an ordinary on-disk database file, the serialization is just a copy of the disk file.
    /// For an in-memory database or a "TEMP" database, the serialization is the same sequence of
    /// bytes which would be written to disk if that database where backed up to disk.
    ///
    /// If the database was created by one of the deserialize functions, consider
    /// [`BorrowingConnection::serialize_rc`] to read the serialization without copying.
    pub fn serialize(&self, db_name: DatabaseName) -> Result<Option<Vec<u8>>> {
        let schema = db_name.to_cstring()?;
        let file = file_ptr(&self.db.borrow(), &schema);
        file.map(|file| {
            if let Some(vec_db) = VecDbFile::try_cast(file) {
                return Ok(vec_db.data.as_slice().to_vec());
                // TODO: Optimize for pMethods == sqlite_io_methods
            }

            // sqlite3_serialize is not used because it always uses the sqlite3_malloc allocator,
            // while this function returns a Vec<u8>.

            // Query the database size with pragma to allocate a vector.
            let schema_str = schema.as_str();
            let escaped = if schema_str.contains('\'') {
                Cow::Owned(schema_str.replace("'", "''"))
            } else {
                Cow::Borrowed(schema_str)
            };
            let sql = &format!(
                "SELECT page_count * page_size FROM pragma_page_count('{0}'), pragma_page_size('{0}')",
                escaped
            );
            let db_size: i64 = self.query_row(sql, NO_PARAMS, |r| r.get(0))?;
            let db_size = db_size.try_into().unwrap();
            if db_size == 0 {
                return Ok(Vec::new())
            }
            let mut vec = Vec::with_capacity(db_size);

            // Unfortunately, sqlite3PagerGet and sqlite3PagerGetData are private APIs,
            // so the Backup API is used instead.
            backup_to_vec(&mut vec, self, db_name)?;
            assert_eq!(vec.len(), db_size, "serialize backup size mismatch");

            Ok(vec)
        })
        .transpose()
    }

    /// Wraps the `Connection` in [`BorrowingConnection`] to serialize and deserialize within the
    /// lifetime of a connection.
    pub fn into_borrowing<'a>(self) -> BorrowingConnection<'a> {
        BorrowingConnection {
            conn: self,
            phantom: PhantomData,
        }
    }

    /// Deserialize using a [`VecDbFile`].
    /// # Safety
    /// The caller must make sure that `data` outlives the connection.
    fn deserialize_vec_db(&self, schema: DatabaseName, data: MemFile) -> Result<()> {
        let schema = schema.to_cstring()?;
        let mut c = self.db.borrow_mut();
        unsafe {
            let rc = ffi::sqlite3_deserialize(c.db(), schema.as_ptr(), ptr::null_mut(), 0, 0, 0);
            c.decode_result(rc)?;
            let file = file_ptr(&c, &schema).unwrap();
            assert_eq!(file.pMethods, MEMDB_VFS.0);
            let mut size_max: ffi::sqlite3_int64 = -1;
            let rc = (*file.pMethods).xFileControl.unwrap()(
                file,
                ffi::SQLITE_FCNTL_SIZE_LIMIT,
                &mut size_max as *mut _ as _,
            );
            assert_eq!(rc, ffi::SQLITE_OK);
            let size_max = size_max.try_into().unwrap();
            let file = file as *mut _ as _;
            ptr::write(file, VecDbFile::new(data, size_max));
            Ok(())
        }
    }
}

fn backup_to_vec(vec: &mut Vec<u8>, src: &Connection, db_name: DatabaseName<'_>) -> Result<()> {
    let mut temp_db = Connection::open_with_flags_and_vfs("0", OpenFlags::default(), "memdb")?;
    unsafe {
        let temp_file = file_ptr(&temp_db.db.borrow_mut(), &SmallCString::new("main")?).unwrap();
        assert_eq!(temp_file.pMethods, MEMDB_VFS.0);
        // At this point, MemFile->aData is null
        ptr::write(
            temp_file as *mut _ as _,
            VecDbFile::new(MemFile::Resizable(vec), 0),
        );
    };

    use crate::backup::{
        Backup,
        StepResult::{Busy, Done, Locked, More},
    };
    let backup = Backup::new_with_names(src, db_name, &mut temp_db, DatabaseName::Main)?;
    let mut r = More;
    while r == More {
        r = backup.step(100)?;
    }
    match r {
        Done => Ok(()),
        Busy => Err(unsafe { error_from_handle(ptr::null_mut(), ffi::SQLITE_BUSY) }),
        Locked => Err(unsafe { error_from_handle(ptr::null_mut(), ffi::SQLITE_LOCKED) }),
        More => unreachable!(),
    }
}

/// Wrapper around [`Connection`] with lifetime constraint to serialize/deserialize borrowed memory,
/// returned from [`Connection::into_borrowing`].
pub struct BorrowingConnection<'a> {
    conn: Connection,
    // This RefCell phantom protects against user-after-frees.
    phantom: PhantomData<std::cell::RefCell<&'a [u8]>>,
}

impl<'a> BorrowingConnection<'a> {
    /// Obtains a reference counted serialization of a database, or returns `Ok(None)` when
    /// [`DatabaseName`] does not exist or no in-memory file is present.
    /// The database is read-only while there are `Rc` or `Weak` pointers.
    /// Once the database is detached, the reference count held by this connection is released.
    pub fn serialize_rc(&self, db: DatabaseName) -> Result<Option<Rc<MemFile<'a>>>> {
        let schema = db.to_cstring()?;
        let c = self.conn.db.borrow_mut();
        Ok(file_ptr(&c, &schema)
            .and_then(|file| VecDbFile::try_cast(file).map(|f| Rc::clone(&f.data))))
    }

    /// Disconnects database and reopens it as an read-only in-memory database based on a slice.
    pub fn deserialize_read_only(&self, db: DatabaseName, slice: &'a [u8]) -> Result<()> {
        self.deserialize_vec_db(db, MemFile::ReadOnly(slice))
    }

    /// Disconnects database and reopens it as an in-memory database based on a borrowed vector.
    pub fn deserialize_resizable(&self, db: DatabaseName, vec: &'a mut Vec<u8>) -> Result<()> {
        self.deserialize_vec_db(db, MemFile::Resizable(vec))
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

/// Byte array storing an in-memory database file.
#[non_exhaustive]
pub enum MemFile<'a> {
    /// Owned vector.
    Owned(Vec<u8>),
    /// Mutable borrowed vector that can be resized.
    Resizable(&'a mut Vec<u8>),
    /// Immutably borrowed slice for a read-only database.
    ReadOnly(&'a [u8]),
}

impl MemFile<'_> {
    fn as_slice(&self) -> &[u8] {
        match self {
            MemFile::Owned(d) => d,
            MemFile::Resizable(d) => d,
            MemFile::ReadOnly(d) => d,
        }
    }

    fn as_ptr(&self) -> *const u8 {
        self.as_slice().as_ptr()
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        match self {
            MemFile::Owned(d) => &mut d[..],
            MemFile::Resizable(d) => &mut d[..],
            MemFile::ReadOnly(_) => unreachable!("ReadOnly.as_mut_slice"),
        }
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_slice().as_mut_ptr()
    }

    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn set_len(&mut self, new_len: usize) {
        unsafe {
            match self {
                MemFile::Owned(d) => d.set_len(new_len),
                MemFile::Resizable(d) => d.set_len(new_len),
                MemFile::ReadOnly(_) => unreachable!("ReadOnly.set_len"),
            }
        }
    }

    fn cap(&self) -> usize {
        match self {
            MemFile::Owned(d) => d.capacity(),
            MemFile::Resizable(d) => d.capacity(),
            MemFile::ReadOnly(_) => unreachable!("ReadOnly.cap"),
        }
    }

    fn reserve_additional(&mut self, additional: usize) {
        match self {
            MemFile::Owned(d) => d.reserve(additional),
            MemFile::Resizable(d) => d.reserve(additional),
            MemFile::ReadOnly(_) => unreachable!("ReadOnly.reserve_additional"),
        }
    }

    // Write-protected/read-only or not
    fn writable(&self) -> bool {
        match self {
            MemFile::Owned(_) => true,
            MemFile::Resizable(_) => true,
            MemFile::ReadOnly(_) => false,
        }
    }
}

impl ops::Deref for MemFile<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

/// `sqlite3_file` subclass for the `vec_db` Virtual File System. It's inspired by
/// the `memdb` VFS but uses the Rust allocator instead of `sqlite3_malloc`.
/// The database is stored in an owned or borrowed `Vec<u8>`.
#[repr(C)]
struct VecDbFile<'a> {
    methods: *const ffi::sqlite3_io_methods,
    data: Rc<MemFile<'a>>,
    size_max: usize,
    memory_mapped: u16,
}

impl<'a> VecDbFile<'a> {
    fn new(data: MemFile<'a>, size_max: usize) -> Self {
        VecDbFile {
            size_max,
            data: Rc::new(data),
            methods: &VEC_DB_IO_METHODS,
            memory_mapped: 0,
        }
    }

    fn try_cast(file: &mut ffi::sqlite3_file) -> Option<&mut Self> {
        if file.pMethods == &VEC_DB_IO_METHODS {
            unsafe { Some(&mut *(file as *mut _ as *mut VecDbFile)) }
        } else {
            None
        }
    }
}

/// IO Methods for the `vec_db` Virtual File System.
/// This can't be a const because the pointers are compared.
static VEC_DB_IO_METHODS: ffi::sqlite3_io_methods = ffi::sqlite3_io_methods {
    iVersion: 3,
    xClose: Some(c_close),
    xRead: Some(c_read),
    xWrite: Some(c_write),
    xTruncate: Some(c_truncate),
    xSync: Some(c_sync),
    xFileSize: Some(c_size),
    xLock: Some(c_lock),
    xUnlock: Some(c_lock),
    xCheckReservedLock: None,
    xFileControl: Some(c_file_control),
    xSectorSize: None,
    xDeviceCharacteristics: Some(c_device_characteristics),
    xShmMap: None,
    xShmLock: None,
    xShmBarrier: None,
    xShmUnmap: None,
    xFetch: Some(c_fetch),
    xUnfetch: Some(c_unfetch),
};

lazy_static::lazy_static! {
    /// Get `memdb_io_methods` and `szOsFile` for the VFS defined in `memdb.c`
    static ref MEMDB_VFS: (&'static ffi::sqlite3_io_methods, i32) = unsafe {
        let vfs = &mut *ffi::sqlite3_vfs_find("memdb\0".as_ptr() as _);
        let sz = vfs.szOsFile;
        assert!(mem::size_of::<VecDbFile>() <= sz as _, "VecDbFile doesn't fit in allocation");
        let file = ffi::sqlite3_malloc(sz) as *mut ffi::sqlite3_file;
        assert!(!file.is_null());
        let mut out_flags = 0;
        let rc = vfs.xOpen.unwrap()(vfs, ptr::null(), file, ffi::SQLITE_OPEN_MAIN_DB, &mut out_flags);
        assert_eq!(rc, ffi::SQLITE_OK);
        let methods = &*(*file).pMethods;
        ffi::sqlite3_free(file as _);
        (methods, sz)
    };
}

fn file_ptr<'a>(c: &InnerConnection, schema: &SmallCString) -> Option<&'a mut ffi::sqlite3_file> {
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

unsafe fn catch_unwind_sqlite_error(
    file: *mut ffi::sqlite3_file,
    f: impl FnOnce(&mut VecDbFile) -> c_int + panic::UnwindSafe,
) -> c_int {
    panic::catch_unwind(|| f(&mut *(file as *mut VecDbFile))).unwrap_or_else(|e| {
        dbg!(e);
        ffi::SQLITE_ERROR
    })
}

/// This will be called when dropping the `Connection` or
/// when the database gets detached.
unsafe extern "C" fn c_close(file: *mut ffi::sqlite3_file) -> c_int {
    catch_unwind_sqlite_error(file, |file| {
        ptr::drop_in_place(file);
        ffi::SQLITE_OK
    })
}

/// Read data from a memory file.
unsafe extern "C" fn c_read(
    file: *mut ffi::sqlite3_file,
    buf: *mut c_void,
    amt: c_int,
    ofst: i64,
) -> c_int {
    catch_unwind_sqlite_error(file, |file| {
        let data = &file.data;
        let buf = buf as *mut u8;
        let amt: usize = amt.try_into().unwrap();
        let ofst: usize = ofst.try_into().unwrap();
        if ofst + amt > data.len() {
            ptr::write_bytes(buf, 0, amt);
            if ofst < data.len() {
                ptr::copy_nonoverlapping(data.as_ptr().add(ofst), buf, data.len() - ofst);
            }
            return ffi::SQLITE_IOERR_SHORT_READ;
        }
        ptr::copy_nonoverlapping(data.as_ptr().add(ofst), buf, amt);
        ffi::SQLITE_OK
    })
}

/// Write data to a memory file.
unsafe extern "C" fn c_write(
    file: *mut ffi::sqlite3_file,
    buf: *const c_void,
    amt: c_int,
    ofst: i64,
) -> c_int {
    catch_unwind_sqlite_error(file, |file| {
        let data = match Rc::get_mut(&mut file.data) {
            Some(d) if d.writable() => d,
            _ => return ffi::SQLITE_READONLY,
        };
        let sz = data.len();
        let sz_alloc = data.cap();
        let amt = amt as usize;
        let ofst = ofst as usize;
        if ofst + amt > sz {
            if ofst + amt > sz_alloc {
                if file.memory_mapped > 0 {
                    return ffi::SQLITE_FULL;
                }
                data.reserve_additional(ofst + amt - sz_alloc);
                if data.cap() > file.size_max {
                    return ffi::SQLITE_FULL;
                }
            }
            if ofst > sz {
                ptr::write_bytes(data.as_mut_ptr().add(sz), 0, ofst - sz);
            }
            data.set_len(ofst + amt);
        }
        ptr::copy_nonoverlapping(buf, data.as_mut_ptr().add(ofst).cast(), amt);
        ffi::SQLITE_OK
    })
}

/// Truncate a memory file.
///
/// In rollback mode (which is always the case for memdb, as it does not
/// support WAL mode) the truncate() method is only used to reduce
/// the size of a file, never to increase the size.
unsafe extern "C" fn c_truncate(file: *mut ffi::sqlite3_file, size: i64) -> c_int {
    catch_unwind_sqlite_error(file, |file| {
        if let Some(data) = Rc::get_mut(&mut file.data) {
            let size = size.try_into().unwrap();
            if size > data.len() || !data.writable() {
                ffi::SQLITE_FULL
            } else {
                data.set_len(size);
                ffi::SQLITE_OK
            }
        } else {
            ffi::SQLITE_FULL
        }
    })
}

/// Sync a memory file.
unsafe extern "C" fn c_sync(_file: *mut ffi::sqlite3_file, _flags: c_int) -> c_int {
    ffi::SQLITE_OK
}

/// Return the current file-size of a memory file.
unsafe extern "C" fn c_size(file: *mut ffi::sqlite3_file, size: *mut i64) -> c_int {
    catch_unwind_sqlite_error(file, |file| {
        *size = file.data.len() as _;
        ffi::SQLITE_OK
    })
}

/// Lock a memory file.
unsafe extern "C" fn c_lock(file: *mut ffi::sqlite3_file, lock: c_int) -> c_int {
    if lock > ffi::SQLITE_LOCK_SHARED && !(*(file as *mut VecDbFile)).data.writable() {
        ffi::SQLITE_READONLY
    } else {
        // TODO: Why stores memdb.c the lock in the struct but never uses it
        ffi::SQLITE_OK
    }
}

/// File control method.
unsafe extern "C" fn c_file_control(
    file: *mut ffi::sqlite3_file,
    op: c_int,
    arg: *mut c_void,
) -> c_int {
    catch_unwind_sqlite_error(file, |file| {
        let data = &file.data;
        match op {
            ffi::SQLITE_FCNTL_VFSNAME => {
                *(arg as *mut *const c_char) = ffi::sqlite3_mprintf(
                    "vec_db(%p,%llu)\0".as_ptr() as _,
                    data.as_ptr(),
                    data.len() as ffi::sqlite3_uint64,
                );
                ffi::SQLITE_OK
            }
            ffi::SQLITE_FCNTL_SIZE_LIMIT => {
                let arg = arg as *mut ffi::sqlite3_int64;
                let mut limit = *arg;
                if limit < data.len() as _ {
                    if limit < 0 {
                        limit = file.size_max as _;
                    } else {
                        limit = data.len() as _;
                    }
                }
                file.size_max = limit.try_into().expect("overflow size_max");
                *arg = limit;
                ffi::SQLITE_OK
            }
            _ => ffi::SQLITE_NOTFOUND,
        }
    })
}

/// Return the device characteristic flags supported.
unsafe extern "C" fn c_device_characteristics(_file: *mut ffi::sqlite3_file) -> c_int {
    ffi::SQLITE_IOCAP_ATOMIC
        | ffi::SQLITE_IOCAP_POWERSAFE_OVERWRITE
        | ffi::SQLITE_IOCAP_SAFE_APPEND
        | ffi::SQLITE_IOCAP_SEQUENTIAL
}

/// Fetch a page of a memory-mapped file.
unsafe extern "C" fn c_fetch(
    file: *mut ffi::sqlite3_file,
    ofst: i64,
    amt: c_int,
    p: *mut *mut c_void,
) -> c_int {
    catch_unwind_sqlite_error(file, |file| {
        let data = &file.data;
        let amt: usize = amt.try_into().unwrap();
        let ofst: usize = ofst.try_into().unwrap();
        if ofst + amt > data.len() as _ {
            *p = ptr::null_mut();
        } else {
            // Safety: SQLite uses a read-only memory map <https://www.sqlite.org/mmap.html>,
            // so it is safe to cast this *const to *mut.
            *p = data.as_ptr() as *mut u8 as _;
            file.memory_mapped += 1;
        }
        ffi::SQLITE_OK
    })
}

/// Release a memory-mapped page.
unsafe extern "C" fn c_unfetch(file: *mut ffi::sqlite3_file, _ofst: i64, _p: *mut c_void) -> c_int {
    catch_unwind_sqlite_error(file, |file| {
        file.memory_mapped -= 1;
        ffi::SQLITE_OK
    })
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{Connection, DatabaseName, Error, Result, NO_PARAMS};
    use std::ffi::CStr;

    #[test]
    pub fn test_serialize_deserialize() {
        let db = Connection::open_in_memory().unwrap().into_borrowing();
        let sql = "BEGIN;
            CREATE TABLE foo(x INTEGER);
            INSERT INTO foo VALUES(1);
            INSERT INTO foo VALUES(2);
            INSERT INTO foo VALUES(3);
            END;";
        db.execute_batch(sql).unwrap();
        let serialized = db.serialize(DatabaseName::Main).unwrap().unwrap();

        // create a new db and import the serialized data
        let db2 = Connection::open_in_memory().unwrap().into_borrowing();
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
        assert!(db.serialize_rc(DatabaseName::Main).unwrap().is_none());
        let borrowed_serialized = db2.serialize_rc(DatabaseName::Main).unwrap().unwrap();
        let mut serialized = Vec::new();
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
    #[allow(clippy::redundant_clone)]
    pub fn test_serialize_rc() {
        // prepare two distinct files: a & b
        let db1 = Connection::open_in_memory().unwrap().into_borrowing();
        db1.execute_batch("CREATE TABLE a(x INTEGER);INSERT INTO a VALUES(1);")
            .unwrap();
        let file_a = db1.serialize(DatabaseName::Main).unwrap().unwrap();
        db1.execute_batch("INSERT INTO a VALUES(2);").unwrap();
        let file_b = db1.serialize(DatabaseName::Main).unwrap().unwrap();

        let db2 = Connection::open_in_memory().unwrap().into_borrowing();
        db2.deserialize(DatabaseName::Main, file_a.clone()).unwrap();
        let file_c = db2.serialize_rc(DatabaseName::Main).unwrap().unwrap();
        let sql = "INSERT INTO a VALUES(3)";
        db2.execute_batch(sql)
            .expect_err("should be write protected");
        mem::drop(file_c);
        db2.execute_batch(sql)
            .expect("should succeed after file_c is dropped");
        assert_eq!(
            2,
            db2.query_row("SELECT COUNT(x) FROM a", NO_PARAMS, |r| r.get::<_, i32>(0))
                .unwrap()
        );

        let name_d = DatabaseName::Attached("d");
        let err = db2
            .deserialize_read_only(name_d, &file_a)
            .expect_err("name does not exist");
        assert_eq!(
            err,
            Error::SqliteFailure(
                ffi::Error {
                    code: ffi::ErrorCode::Unknown,
                    extended_code: 1
                },
                Some("not an error".to_string())
            )
        );
        db2.execute_batch("ATTACH DATABASE ':memory:' AS d")
            .unwrap();
        db2.deserialize(name_d, file_a.clone()).unwrap();
        let file_d = db2.serialize_rc(name_d).unwrap().unwrap();
        // detach and attach other db, this should call xClose and decrease reference count
        assert_eq!(2, Rc::strong_count(&file_d));
        db2.deserialize(name_d, file_b).unwrap();
        assert_eq!(1, Rc::strong_count(&file_d));
        // test whether file_d stayed intact
        db2.deserialize_read_only(DatabaseName::Main, &file_d)
            .unwrap();
        assert_eq!(
            1,
            db2.query_row("SELECT MAX(x) FROM main.a", NO_PARAMS, |r| r
                .get::<_, i32>(0))
                .unwrap()
        );
        assert_eq!(
            2,
            db2.query_row("SELECT MAX(x) FROM d.a", NO_PARAMS, |r| r.get::<_, i32>(0))
                .unwrap()
        );
        mem::drop(db2);
        // mem::drop(file_a); // uncommenting this line should not compile
        file_d.len();
    }

    #[test]
    pub fn test_deserialize_read_only_1() {
        let db = Connection::open_in_memory().unwrap();
        let sql = "BEGIN;
            CREATE TABLE foo(x INTEGER);
            INSERT INTO foo VALUES(1);
            INSERT INTO foo VALUES(2);
            INSERT INTO foo VALUES(3);
            END;";
        db.execute_batch(sql).unwrap();
        let serialized = db.serialize(DatabaseName::Main).unwrap().unwrap();
        // copy to Vec and create new Vec
        let serialized_vec = Vec::from(&serialized[..]);
        let mut serialized = Vec::new();
        serialized.extend(serialized_vec);

        // create a new db and import the serialized data
        let db2 = Connection::open_in_memory().unwrap().into_borrowing();
        db2.deserialize_read_only(DatabaseName::Main, &serialized)
            .unwrap();
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
    #[allow(clippy::redundant_clone)]
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
        let serialized_three = serialized_two.clone();
        {
            let name_bar = "bar".to_string();
            db.deserialize_read_only(DatabaseName::Attached(&name_bar), &serialized_three)?;
        }
        // mem::drop(serialized_three); // uncommenting this should not compile
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

    //noinspection RsAssertEqual
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
        let db1 = db1.into_borrowing();
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
        let db4 = Connection::open_in_memory()?.into_borrowing();
        db4.execute_batch("ATTACH DATABASE ':memory:' AS hello")?;
        db4.deserialize_resizable(DatabaseName::Attached("hello"), &mut serialized2)?;
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
        let db = Connection::open_in_memory().unwrap().into_borrowing();
        let sql = "BEGIN;
        CREATE TABLE hello(x INTEGER);
        INSERT INTO hello VALUES(1);
        INSERT INTO hello VALUES(2);
        INSERT INTO hello VALUES(3);
        END;";
        db.execute_batch(sql).unwrap();
        assert!(db
            .serialize_rc(DatabaseName::Attached("does not exist"))
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

    fn serialize_len(conn: &mut BorrowingConnection) -> usize {
        conn.serialize_rc(DatabaseName::Main)
            .unwrap()
            .unwrap()
            .len()
    }

    #[test]
    fn test_vec_db_vfs_name() {
        unsafe {
            let db = Connection::open_in_memory().unwrap();
            let vec = vec![1, 2, 3];
            let vec_ptr = vec.as_ptr();
            db.deserialize(DatabaseName::Main, vec).unwrap();
            let mut name: *const c_char = ptr::null();
            let rc = ffi::sqlite3_file_control(
                db.db.borrow().db(),
                "main\0".as_ptr() as _,
                ffi::SQLITE_FCNTL_VFSNAME,
                &mut name as *mut _ as _,
            );
            assert_eq!(ffi::SQLITE_OK, rc);
            assert!(!name.is_null());
            let name_str = CStr::from_ptr(name).to_str().unwrap();
            assert_eq!(name_str, &format!("vec_db({:X},3)", vec_ptr as usize));
            ffi::sqlite3_free(name as _);
        }
    }

    #[test]
    fn test_serialize_zero_pages() {
        let db = Connection::open_in_memory().unwrap().into_borrowing();
        let vec = db.serialize(DatabaseName::Main).unwrap().unwrap();
        assert_eq!(vec.len(), 0);
    }

    #[test]
    fn test_serialize_vec_db() -> Result<()> {
        let db = Connection::open_in_memory().unwrap().into_borrowing();
        db.execute_batch("CREATE TABLE a(x INTEGER); ATTACH DATABASE ':memory:' AS a")?;
        let vec = db.serialize(DatabaseName::Main)?.unwrap();
        let name_a = DatabaseName::Attached("a");
        db.deserialize_read_only(name_a, &vec)?;
        // code coverage reports shows this uses the optimized path
        let copy = db.serialize(name_a)?.unwrap();
        assert_eq!(vec, copy);
        Ok(())
    }

    #[test]
    fn test_serialize_quoted() -> Result<()> {
        let db = Connection::open_in_memory().unwrap().into_borrowing();
        db.execute_batch(
            r#"ATTACH DATABASE ':memory:' AS "q'u""o'te";
            CREATE TABLE "q'u""o'te".a(x INTEGER);
            INSERT INTO "q'u""o'te".a VALUES (1);"#,
        )?;
        let name_a = DatabaseName::Attached(r#"q'u"o'te"#);
        let count: i64 = db.pragma_query_value(Some(name_a), "page_count", |r| r.get(0))?;
        assert_eq!(count, 2);
        let count: i64 = db.query_row(
            r#"SELECT page_count FROM pragma_page_count("q'u""o'te")"#,
            NO_PARAMS,
            |r| r.get(0),
        )?;
        assert_eq!(count, 2);
        let vec = db.serialize(name_a)?.unwrap();
        assert_eq!(vec.len(), 8192);
        Ok(())
    }

    #[test]
    fn test_serialize_page_size() -> Result<()> {
        let db = Connection::open_in_memory().unwrap().into_borrowing();
        db.execute_batch(r#"PRAGMA page_size = 512;CREATE TABLE a(x INTEGER);"#)?;
        let vec = db.serialize(DatabaseName::Main)?.unwrap();
        assert_eq!(vec.len(), 512 * 2);
        Ok(())
    }

    #[test]
    fn test_vec_db_fetch() -> Result<()> {
        let db = Connection::open_in_memory().unwrap().into_borrowing();
        db.execute_batch("CREATE TABLE a(x INTEGER)")?;
        let mut vec = db.serialize(DatabaseName::Main)?.unwrap();
        let size = vec.len();
        assert_ne!(0, size);
        db.deserialize_resizable(DatabaseName::Main, &mut vec)?;
        let file = file_ptr(&db.db.borrow(), &DatabaseName::Main.to_cstring()?).unwrap();
        // fetch returns null on overflow
        assert!(file_fetch(file, 0, size + 1)?.is_null());
        assert!(file_fetch(file, 1, size)?.is_null());
        file_fetch(file, -1, 1).expect_err("should catch panic because of negative offset");
        let p = file_fetch(file, 0, size)?;
        assert!(!p.is_null());
        assert_eq!(p, db.serialize_rc(DatabaseName::Main)?.unwrap().as_ptr());
        // Won't resize unit unfetch
        let sql = "WITH RECURSIVE cnt(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM cnt WHERE x<100000) INSERT INTO a SELECT x FROM cnt";
        db.execute_batch(sql).expect_err("enlarge should fail");
        file_unfetch(file);
        assert_eq!(size, db.serialize_rc(DatabaseName::Main)?.unwrap().len());
        db.execute_batch(sql).expect("enlarge should succeed");
        assert_ne!(size, db.serialize_rc(DatabaseName::Main)?.unwrap().len());
        Ok(())
    }

    fn file_fetch(file: &mut ffi::sqlite3_file, ofst: i64, amt: usize) -> Result<*const u8> {
        unsafe {
            let mut fetch = MaybeUninit::zeroed();
            let rc =
                (*file.pMethods).xFetch.unwrap()(file, ofst, amt as _, fetch.as_mut_ptr() as _);
            if rc != ffi::SQLITE_OK {
                Err(error_from_handle(ptr::null_mut(), ffi::SQLITE_LOCKED))
            } else {
                Ok(fetch.assume_init())
            }
        }
    }

    fn file_unfetch(file: &mut ffi::sqlite3_file) {
        let rc = unsafe { (*file.pMethods).xUnfetch.unwrap()(file, 0, ptr::null_mut()) };
        assert_eq!(rc, ffi::SQLITE_OK);
    }

    #[test]
    fn test_vec_db_read_short() -> Result<()> {
        let db = Connection::open_in_memory().unwrap().into_borrowing();
        db.execute_batch("CREATE TABLE a(x INTEGER)")?;
        db.deserialize(
            DatabaseName::Main,
            db.serialize(DatabaseName::Main)?.unwrap(),
        )?;
        let file = file_ptr(&db.db.borrow(), &DatabaseName::Main.to_cstring()?).unwrap();

        // when reading past end, the buffer should be filled with zeros
        let mut buf = [1; 16];
        let end = file_len(file);
        let rc = file_read(file, &mut buf, end);
        assert_eq!(rc, ffi::SQLITE_IOERR_SHORT_READ);
        assert_eq!(&buf, &[0; 16]);

        // when reading partly past the end, the buffer should be filled with content
        let vec_db = VecDbFile::try_cast(file).unwrap();
        Rc::get_mut(&mut vec_db.data).unwrap().as_mut_slice()[end as usize - 1] = 0xab;
        let mut buf = [1; 16];
        let rc = file_read(file, &mut buf, end - 8);
        assert_eq!(rc, ffi::SQLITE_IOERR_SHORT_READ);
        assert_eq!(&buf, b"\0\0\0\0\0\0\0\xab\0\0\0\0\0\0\0\0");
        Ok(())
    }

    fn file_read(file: &mut ffi::sqlite3_file, buf: &mut [u8], ofst: i64) -> c_int {
        unsafe {
            (*file.pMethods).xRead.unwrap()(file, buf.as_mut_ptr() as _, buf.len() as _, ofst)
        }
    }

    fn file_len(file: &mut ffi::sqlite3_file) -> i64 {
        unsafe {
            let mut size = 0;
            let rc = (*file.pMethods).xFileSize.unwrap()(file, &mut size);
            assert_eq!(rc, ffi::SQLITE_OK);
            size
        }
    }

    #[test]
    fn test_vec_db_size_max() -> Result<()> {
        let db = Connection::open_in_memory().unwrap().into_borrowing();
        db.execute_batch("CREATE TABLE a(x INTEGER)")?;
        let mut vec = db.serialize(DatabaseName::Main)?.unwrap();
        let size = vec.len() as i64;
        assert_ne!(0, size);
        db.deserialize_resizable(DatabaseName::Main, &mut vec)?;
        let file = file_ptr(&db.db.borrow(), &DatabaseName::Main.to_cstring()?).unwrap();
        let cap = file_cap(file, -1);
        assert_eq!(cap, 1073741824, "default SQLITE_CONFIG_MEMDB_MAXSIZE");
        assert_eq!(size, file_cap(file, 200));
        assert_eq!(size, file_cap(file, -1));

        // trigger enlarge
        let sql = "WITH RECURSIVE cnt(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM cnt WHERE x<500) INSERT INTO a SELECT x FROM cnt";
        db.execute_batch(sql).expect_err("enlarge should fail");

        let new_cap = size * 2;
        assert_eq!(new_cap, file_cap(file, new_cap));
        assert_eq!(new_cap, file_cap(file, -1));
        db.execute_batch(sql).expect("enlarge should succeed");

        // truncate
        assert_eq!(new_cap, file_len(file));
        db.execute_batch("DELETE FROM a; VACUUM;")?;
        assert_eq!(size, file_len(file));
        assert_eq!(new_cap, file_cap(file, -1));
        db.execute_batch("DROP TABLE a; VACUUM;")?;
        assert_eq!(4096, file_len(file));

        Ok(())
    }

    fn file_cap(file: &mut ffi::sqlite3_file, mut size_max: i64) -> i64 {
        unsafe {
            let rc = (*file.pMethods).xFileControl.unwrap()(
                file,
                ffi::SQLITE_FCNTL_SIZE_LIMIT,
                &mut size_max as *mut _ as _,
            );
            assert_eq!(rc, ffi::SQLITE_OK);
            size_max
        }
    }

    #[test]
    fn test_vec_db_write_zero_past_len() -> Result<()> {
        unsafe {
            let mut vec_db = VecDbFile::new(MemFile::Owned(Vec::new()), usize::MAX);
            let file = &mut vec_db as *mut _ as *mut ffi::sqlite3_file;
            let write = (*vec_db.methods).xWrite.unwrap();
            let buf = &[11u8, 22, 33];
            write(file, buf.as_ptr() as *const _, buf.len() as _, 4);
            assert_eq!(vec_db.data.as_slice(), &[0, 0, 0, 0, 11, 22, 33]);
        }
        Ok(())
    }
}
