//! Create virtual file systems.
//!
//! Follow these steps to create your own VFS:
//!
//! 1. Write an implementation of the [`Vfs`] and [`VfsFile`] traits.
//! 2. Optionally write implementations of the [`VfsWalFile`] and [`VfsFetchFile`] traits.
//! 3. Create [`VfsRegistration`] for the VFS from step 1 and configure it.
//! 4. Call [`VfsRegistration::register`].
//! 5. Open a connection with [`Connection::open_with_flags_and_vfs`].
//!
//! (See [SQLite doc](https://sqlite.org/vfs.html))

#[cfg(all(feature = "memvfs", unix))]
pub mod memvfs;

use crate::ffi as sqlite3;
use crate::ffi::{
    sqlite3_file, sqlite3_filename, sqlite3_int64, sqlite3_io_methods, sqlite3_vfs, Error,
    IntoResultCodeExt,
};
use rand::RngCore;
use std::borrow::Cow;
use std::error;
use std::ffi::{c_char, c_int, CStr, CString, OsStr};
use std::fmt::{self, Display};
use std::marker::PhantomData;
use std::num::NonZero;
use std::ops::Deref;
use std::os::raw::c_void;
use std::ptr::{self, NonNull};
use std::sync::atomic::{self, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};
use std::{mem, slice};

use crate::{Connection, OpenFlags};

/// A specialised result type for [`Vfs`] operations.
pub type Result<T, E = Error> = core::result::Result<T, E>;

/// Extension trait to write results to output parameters, consuming the result and returning an appropriate [`Result`].
pub trait WriteOutputResultExt<T> {
    /// Converts `self` into the `sqlite`-expected `out` param + return code form.
    ///
    /// If `self` is:
    /// - `Ok(value)`, then `value` is written to `*output` and [`Result::Ok`] is returned.
    /// - `Err(err)`, then `*output` is unchanged and `err` is returned.
    fn write_to_output(self, output: &mut impl From<T>) -> Result<()>;
}

impl<T> WriteOutputResultExt<T> for Result<T> {
    fn write_to_output(self, output: &mut impl From<T>) -> Result<()> {
        match self {
            Ok(value) => {
                *output = value.into();
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

/// Represents a sqlite virtual file system.
///
/// This trait abstracts [`sqlite3_vfs`](https://www.sqlite.org/c3ref/vfs.html).
pub trait Vfs: Sync {
    /// The type of files stored within this VFS.
    type File: VfsFile;

    /// Opens a file. Returns the file and the actual flags used.
    ///
    /// See [`xOpen`](https://www.sqlite.org/c3ref/vfs.html).
    fn open(&self, file: FileType<'_>, flags: VfsOpenFlags) -> Result<OpenFile<Self::File>>;
    /// Deletes a file, optionally syncing the directory afterward.
    ///
    /// See [`xDelete`](https://www.sqlite.org/c3ref/vfs.html).
    fn delete(&self, name: VfsPath<'_>, sync_dir: bool) -> Result<()>;

    /// Checks if a file exists.
    ///
    /// See [`xAccess`](https://www.sqlite.org/c3ref/vfs.html).
    fn exists(&self, name: VfsPath<'_>) -> Result<bool>;

    /// Checks if a file is readable.
    ///
    /// See [`xAccess`](https://www.sqlite.org/c3ref/vfs.html).
    fn can_read(&self, name: VfsPath<'_>) -> Result<bool>;

    /// Checks if a file is writable.
    ///
    /// See [`xAccess`](https://www.sqlite.org/c3ref/vfs.html).
    fn can_write(&self, name: VfsPath<'_>) -> Result<bool>;

    /// Writes the full pathname of a file to the output buffer.
    ///
    /// See [`xFullPathname`](https://www.sqlite.org/c3ref/vfs.html).
    fn write_full_path(&self, name: VfsPath<'_>, out: &mut [u8]) -> Result<usize>;

    /// Returns the last error code.
    ///
    /// See [`xGetLastError`](https://www.sqlite.org/c3ref/vfs.html).
    fn last_error(&self) -> i32;

    /// Fills a buffer with random bytes.
    ///
    /// See [`xRandomness`](https://www.sqlite.org/c3ref/vfs.html).
    fn fill_random_bytes(&self, out: &mut [u8]) -> Result<()> {
        let mut rng = rand::rng();
        rng.fill_bytes(out);
        Ok(())
    }

    /// Sleeps for the given duration.
    ///
    /// See [`xSleep`](https://www.sqlite.org/c3ref/vfs.html).
    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }

    /// Returns the current system time.
    ///
    /// See [`xCurrentTimeInt64`](https://www.sqlite.org/c3ref/vfs.html).
    fn now(&self) -> Result<SystemTime> {
        Ok(SystemTime::now())
    }
}

/// The type of file being opened.
///
/// See [`xOpen`](https://sqlite.org/c3ref/vfs.html#sqlite3vfsxopen)
#[derive(Debug)]
pub enum FileType<'a> {
    /// Mirrors SQLITE_OPEN_MAIN_DB.
    MainDb(VfsPath<'a>),
    /// Mirrors SQLITE_OPEN_MAIN_JOURNAL.
    MainJournal(VfsPath<'a>),
    /// Mirrors SQLITE_OPEN_TEMP_DB.
    TempDb,
    /// Mirrors SQLITE_OPEN_TEMP_JOURNAL.
    TempJournal,
    /// Mirrors SQLITE_OPEN_TRANSIENT_DB.
    TransientDb,
    /// Mirrors SQLITE_OPEN_SUBJOURNAL.
    Subjournal(VfsPath<'a>),
    /// Mirrors SQLITE_OPEN_SUPER_JOURNAL.
    SuperJournal(VfsPath<'a>),
    /// Mirrors SQLITE_OPEN_WAL.
    Wal(VfsPath<'a>),
}

impl<'a> FileType<'a> {
    /// Returns the path associated with this file type, if available.
    pub fn path(&self) -> Option<&VfsPath<'a>> {
        match self {
            FileType::MainDb(path)
            | FileType::MainJournal(path)
            | FileType::Subjournal(path)
            | FileType::SuperJournal(path)
            | FileType::Wal(path) => Some(path),
            FileType::TempDb | FileType::TempJournal | FileType::TransientDb => None,
        }
    }
}

bitflags::bitflags! {
    /// Flags used by VFS-backed opens, extending [`crate::OpenFlags`].
    pub struct VfsOpenFlags: c_int {
        /// Mirrors [`OpenFlags::SQLITE_OPEN_READ_ONLY`].
        const SQLITE_OPEN_READ_ONLY = sqlite3::SQLITE_OPEN_READONLY;
        /// Mirrors [`OpenFlags::SQLITE_OPEN_READ_WRITE`].
        const SQLITE_OPEN_READ_WRITE = sqlite3::SQLITE_OPEN_READWRITE;
        /// Deletes the file when it is closed. See <https://sqlite.org/c3ref/vfs.html#sqlite3vfsxopen>.
        const SQLITE_OPEN_DELETE_ON_CLOSE = sqlite3::SQLITE_OPEN_DELETEONCLOSE;
        /// Opens the file in exclusive mode. See <https://sqlite.org/c3ref/vfs.html#sqlite3vfsxopen>.
        const SQLITE_OPEN_EXCLUSIVE = sqlite3::SQLITE_OPEN_EXCLUSIVE;
        /// Enables auto-proxy handling for VFS layering. See <https://sqlite.org/c3ref/vfs.html#sqlite3vfsxopen>.
        const SQLITE_OPEN_AUTOPROXY = sqlite3::SQLITE_OPEN_AUTOPROXY;
        /// Mirrors [`OpenFlags::SQLITE_OPEN_CREATE`].
        const SQLITE_OPEN_CREATE = sqlite3::SQLITE_OPEN_CREATE;
        /// Mirrors [`OpenFlags::SQLITE_OPEN_URI`].
        const SQLITE_OPEN_URI = sqlite3::SQLITE_OPEN_URI;
        /// Mirrors [`OpenFlags::SQLITE_OPEN_MEMORY`].
        const SQLITE_OPEN_MEMORY = sqlite3::SQLITE_OPEN_MEMORY;
        /// Mirrors [`OpenFlags::SQLITE_OPEN_NO_MUTEX`].
        const SQLITE_OPEN_NO_MUTEX = sqlite3::SQLITE_OPEN_NOMUTEX;
        /// Mirrors [`OpenFlags::SQLITE_OPEN_FULL_MUTEX`].
        const SQLITE_OPEN_FULL_MUTEX = sqlite3::SQLITE_OPEN_FULLMUTEX;
        /// Mirrors [`OpenFlags::SQLITE_OPEN_SHARED_CACHE`].
        const SQLITE_OPEN_SHARED_CACHE = 0x0002_0000;
        /// Mirrors [`OpenFlags::SQLITE_OPEN_PRIVATE_CACHE`].
        const SQLITE_OPEN_PRIVATE_CACHE = 0x0004_0000;
        /// Mirrors [`OpenFlags::SQLITE_OPEN_NOFOLLOW`].
        const SQLITE_OPEN_NOFOLLOW = 0x0100_0000;
        /// Mirrors [`OpenFlags::SQLITE_OPEN_EXRESCODE`].
        const SQLITE_OPEN_EXRESCODE = 0x0200_0000;
    }
}

impl From<OpenFlags> for VfsOpenFlags {
    fn from(flags: OpenFlags) -> Self {
        VfsOpenFlags::from_bits_retain(flags.bits())
    }
}

impl From<VfsOpenFlags> for OpenFlags {
    fn from(flags: VfsOpenFlags) -> Self {
        OpenFlags::from_bits_retain(flags.bits())
    }
}

/// A file path passed to VFS operations.
#[derive(Debug)]
pub struct VfsPath<'a>(&'a CStr);

impl<'a> VfsPath<'a> {
    /// Creates a new `VfsPath`.
    pub fn new(path: &'a CStr) -> Self {
        Self(path)
    }

    /// Returns the inner path as a OsStr.
    #[cfg(unix)]
    pub fn as_os_str(&self) -> &OsStr {
        use std::os::unix::ffi::OsStrExt;
        OsStr::from_bytes(self.0.to_bytes())
    }

    /// Returns the inner path as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.to_bytes()
    }

    /// Returns the inner path as a CStr.
    pub fn as_c_str(&self) -> &CStr {
        self.0
    }
}

/// Represents a VFS opened file along with its additional flags.
pub struct OpenFile<F> {
    file: F,
    readonly: bool,
}

impl<T> OpenFile<T> {
    /// Creates a new `OpenFile`.
    pub fn new(file: T) -> Self {
        OpenFile {
            file,
            readonly: false,
        }
    }

    /// Marks the file as readonly. See [`SQLITE_OPEN_READONLY`](https://sqlite.org/c3ref/vfs.html#sqlite3vfsxopen).
    pub fn readonly(mut self) -> Self {
        self.readonly = true;
        self
    }
}

/// Represents the most basic file I/O bahaviours required by a [`Vfs`].
///
/// This trait is optional and corresponds to [`sqlite3_io_methods` v1](https://www.sqlite.org/c3ref/io_methods.html).
#[allow(clippy::len_without_is_empty)]
pub trait VfsFile {
    /// Reads from the file at an offset.
    ///
    /// See [`xRead`](https://www.sqlite.org/c3ref/io_methods.html#xRead).
    fn read_at(&mut self, buf: &mut [u8], offset: u64) -> Result<usize>;

    /// Writes to the file at an offset.
    ///
    /// See [`xWrite`](https://www.sqlite.org/c3ref/io_methods.html#xWrite).
    fn write_at(&mut self, buf: &[u8], offset: u64) -> Result<()>;

    /// Truncates the file to a size.
    ///
    /// See [`xTruncate`](https://www.sqlite.org/c3ref/io_methods.html#xTruncate).
    fn truncate(&mut self, size: u64) -> Result<()>;

    /// Syncs the file to disk.
    ///
    /// See [`xSync`](https://www.sqlite.org/c3ref/io_methods.html#xSync).
    fn sync(&mut self, op: SyncOptions) -> Result<()>;

    /// Gets the file size.
    ///
    /// See [`xFileSize`](https://www.sqlite.org/c3ref/io_methods.html#xFileSize).
    fn len(&self) -> Result<u64>;

    /// Acquires a file lock at the given `level`.
    ///
    /// See [`xLock`](https://www.sqlite.org/c3ref/io_methods.html#xLock).
    fn lock(&mut self, level: LockLevel) -> Result<()>;

    /// Releases a file lock at the given `level`.
    ///
    /// See [`xUnlock`](https://www.sqlite.org/c3ref/io_methods.html#xUnlock).
    fn unlock(&mut self, level: LockLevel) -> Result<()>;

    /// Checks if a write lock is held.
    ///
    /// See [`xCheckReservedLock`](https://www.sqlite.org/c3ref/io_methods.html#xCheckReservedLock).
    fn is_write_locked(&self) -> Result<bool>;

    /// Gets the sector size.
    ///
    /// See [`xSectorSize`](https://www.sqlite.org/c3ref/io_methods.html#xSectorSize).
    fn sector_len(&self) -> u32;

    /// Gets I/O characteristics.
    ///
    /// See [`xDeviceCharacteristics`](https://www.sqlite.org/c3ref/io_methods.html#xDeviceCharacteristics).
    fn io_capabilities(&self) -> IoCapabilities;

    /// Gets the current lock state.
    ///
    /// See [`SQLITE_FCNTL_LOCKSTATE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntllockstate).
    fn lock_level(&self) -> LockLevel;

    /// Gets the last OS error number.
    ///
    /// See [`SQLITE_FCNTL_LAST_ERRNO`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntllasterrno).
    fn last_errno(&self) -> i32;

    /// Handles the size hint for a transaction.
    ///
    /// See [`SQLITE_FCNTL_SIZE_HINT`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlsizehint).
    fn hint_size(&mut self, size: i64) -> Result<()> {
        let _ = size;
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }

    /// Hints that subsequent writes overwrite existing content.
    ///
    /// See [`SQLITE_FCNTL_OVERWRITE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntloverwrite).
    fn hint_overwrite(&mut self, size: u64) -> Result<()> {
        let _ = size;
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }

    /// Sets the database chunk size.
    ///
    /// See [`SQLITE_FCNTL_CHUNK_SIZE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlchunksize).
    fn set_chunk_size(&mut self, size: u32) -> Result<()> {
        let _ = size;
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }

    /// Handles PRAGMA forwarding.
    ///
    /// See [`SQLITE_FCNTL_PRAGMA`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlpragma).
    fn pragma(&mut self, name: &str, arg: Option<&str>) -> PragmaResult {
        let _ = name;
        let _ = arg;
        Err(PragmaError::from(Error::new(sqlite3::SQLITE_NOTFOUND)))
    }

    /// Sets the max mmap size and returns the previous value.
    ///
    /// See [`SQLITE_FCNTL_MMAP_SIZE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlmmapsize).
    fn set_mmap_size(&mut self, size: u64) -> Result<u64> {
        let _ = size;
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }

    /// Gets the max mmap size.
    ///
    /// See [`SQLITE_FCNTL_MMAP_SIZE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlmmapsize).
    fn mmap_size(&self) -> Result<u64> {
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }

    /// Reports whether the file has moved.
    ///
    /// See [`SQLITE_FCNTL_HAS_MOVED`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcnthasmoved).
    fn has_moved(&self) -> bool {
        false
    }

    /// Pre-sync hook for a single database.
    ///
    /// See [`SQLITE_FCNTL_SYNC`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlsync).
    fn pre_sync_single_db(&mut self) -> Result<()> {
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }

    /// Pre-sync hook for multiple databases (with super-journal).
    ///
    /// See [`SQLITE_FCNTL_SYNC`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlsync).
    fn pre_sync_multiple_db(&mut self, super_journal: VfsPath<'_>) -> Result<()> {
        let _ = super_journal;
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }

    /// Completes commit phase two.
    ///
    /// See [`SQLITE_FCNTL_COMMIT_PHASETWO`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlcommitphasetwo).
    fn commit_phase_two(&mut self) -> Result<()> {
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }

    /// Sets the parent connection.
    ///
    /// See [`SQLITE_FCNTL_PDB`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlpdb).
    fn set_parent_connection(&mut self, conn: Connection) {
        let _ = conn;
    }

    /// Begins an atomic-write sequence.
    ///
    /// See [`SQLITE_FCNTL_BEGIN_ATOMIC_WRITE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlbeginatomicwrite).
    fn begin_atomic(&mut self) -> Result<()> {
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }

    /// Commits an atomic-write sequence.
    ///
    /// See [`SQLITE_FCNTL_COMMIT_ATOMIC_WRITE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlcommitatomicwrite).
    fn commit_atomic(&mut self) -> Result<()> {
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }

    /// Rolls back an atomic-write sequence.
    ///
    /// See [`SQLITE_FCNTL_ROLLBACK_ATOMIC_WRITE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlrollbackatomicwrite).
    fn rollback_atomic(&mut self) {}

    /// Sets the busy handler.
    ///
    /// See [`SQLITE_FCNTL_BUSYHANDLER`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlbusyhandler).
    fn set_busy_handler(&mut self, handler: BusyHandler) {
        let _ = handler;
    }

    /// Sets the lock timeout and returns the previous value.
    ///
    /// See [`SQLITE_FCNTL_LOCK_TIMEOUT`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntllocktimeout).
    fn set_lock_timeout(&mut self, timeout: Duration) -> Result<Duration> {
        let _ = timeout;
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }

    /// Gets WAL persistence.
    ///
    /// See [`SQLITE_FCNTL_PERSIST_WAL`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlpersistwal).
    fn is_wal_persistent(&self) -> bool {
        false
    }

    /// Sets WAL persistence.
    ///
    /// See [`SQLITE_FCNTL_PERSIST_WAL`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlpersistwal).
    fn set_wal_persistent(&mut self, persist: bool) {
        let _ = persist;
    }

    /// Gets powersafe overwrite property for the filesystem.
    ///
    /// See [`SQLITE_FCNTL_POWERSAFE_OVERWRITE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlpowersafeoverwrite).
    fn is_powersafe_overwrite(&self) -> bool {
        false
    }

    /// Sets powersafe overwrite property for the filesystem.
    ///
    /// See [`SQLITE_FCNTL_POWERSAFE_OVERWRITE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlpowersafeoverwrite).
    fn set_powersafe_overwrite(&mut self, powersafe: bool) -> Result<()> {
        let _ = powersafe;
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }

    /// Hints WAL lock behavior.
    ///
    /// See [`SQLITE_FCNTL_WAL_BLOCK`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlwalblock).
    fn hint_wal_lock(&mut self) {}

    /// Controls blocking behavior during connect.
    ///
    /// See [`SQLITE_FCNTL_BLOCK_ON_CONNECT`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlblockonconnect).
    fn hint_block_on_connect(&mut self, block: bool) {
        let _ = block;
    }

    /// Signals checkpoint start.
    ///
    /// See [`SQLITE_FCNTL_CKPT_START`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlckptstart).
    fn on_checkpoint_start(&mut self) {}

    /// Signals checkpoint completion.
    ///
    /// See [`SQLITE_FCNTL_CKPT_DONE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlckptdone).
    fn on_checkpoint_done(&mut self) {}

    /// Gets the lock proxy file path. This is only used on MacOS by the unix VFS.
    ///
    /// See [`SQLITE_FCNTL_LOCK_PROXYFILE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html).
    #[cfg(unix)]
    fn lock_proxy_file_path(&self) -> Result<Option<&'static CStr>> {
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }

    /// Gets the lock proxy file path. This is only used on MacOS by the unix VFS.
    ///
    /// See [`SQLITE_FCNTL_LOCK_PROXYFILE`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html).
    #[cfg(unix)]
    fn set_lock_proxy_file_path(&mut self, path: Option<&'static CStr>) -> Result<()> {
        let _ = path;
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }

    /// Sets the size limit, checking for validity, and returns the value set. This is only used by the memory VFS.
    ///
    /// See [`SQLITE_FCNTL_SIZE_LIMIT`](https://www.sqlite.org/c3ref/c_fcntl_begin_atomic_write.html#sqlitefcntlsizelimit).
    fn set_size_limit(&mut self, size: Option<u64>) -> Result<u64> {
        let _ = size;
        Err(Error::new(sqlite3::SQLITE_NOTFOUND))
    }
}

/// Options for syncing a file.
pub struct SyncOptions {
    /// True for Mac OS X style fullsync, false for Unix style fsync.
    pub full: bool,
    /// True to sync only the data of the file and not its inode (fdatasync).
    pub data_only: bool,
}

impl SyncOptions {
    /// Converts to raw SQLite flags.
    pub fn to_raw(&self) -> c_int {
        let mut flags = 0;
        if self.full {
            flags |= sqlite3::SQLITE_SYNC_FULL;
        }
        if self.data_only {
            flags |= sqlite3::SQLITE_SYNC_DATAONLY;
        }
        flags
    }
}

/// Represents pragma operation results.
pub type PragmaResult = std::result::Result<Option<Cow<'static, str>>, PragmaError>;

/// Represents errors in pragma operations.
#[derive(Debug)]
pub struct PragmaError {
    /// Error code.
    pub code: Error,
    /// Optional error message.
    pub message: Option<Cow<'static, str>>,
}

impl PragmaError {
    /// Constructs a pragma error with an explicit message.
    pub fn new(code: Error, message: impl Into<Cow<'static, str>>) -> Self {
        PragmaError {
            code,
            message: Some(message.into()),
        }
    }
}

impl From<Error> for PragmaError {
    fn from(code: Error) -> Self {
        PragmaError {
            code,
            message: None,
        }
    }
}

impl Display for PragmaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { code, message } = self;
        match message {
            Some(msg) => write!(f, "{code}: {msg}"),
            None => write!(f, "{}", code),
        }
    }
}

impl error::Error for PragmaError {}

/// Represents the connection busy handler callback.
///
/// See [`sqlite3_busy_handler`](https://sqlite.org/c3ref/busy_handler.html)
pub struct BusyHandler {
    handler: extern "C" fn(*mut c_void) -> c_int,
    arg: *mut c_void,
}

impl BusyHandler {
    /// Calls the busy handler. Returns true to retry, false to give up.
    pub fn call(&self) -> bool {
        let rc = (self.handler)(self.arg);
        rc != 0
    }
}

/// Represents file I/O behaviors required to use a write-ahead log with shared-memory support
/// with a [`Vfs`].
///
/// This trait corresponds to [`sqlite3_io_methods` v2](https://www.sqlite.org/c3ref/io_methods.html).
pub trait VfsWalFile: VfsFile {
    /// Maps a shared-memory region.
    ///
    /// See [`xShmMap`](https://www.sqlite.org/c3ref/io_methods.html#xShmMap).
    fn map_shm(&mut self, region_index: u32, region_size: usize, extend: bool)
        -> Result<&mut [u8]>;

    /// Acquires a shared-memory lock.
    ///
    /// See [`xShmLock`](https://www.sqlite.org/c3ref/io_methods.html#xShmLock).
    fn lock_shm(&mut self, locks: WalLock, mode: WalLockMode) -> Result<()>;

    /// Releases a shared-memory lock.
    ///
    /// See [`xShmLock`](https://www.sqlite.org/c3ref/io_methods.html#xShmLock).
    fn unlock_shm(&mut self, locks: WalLock, mode: WalLockMode) -> Result<()>;

    /// Unmaps the shared-memory, optionally deleting.
    ///
    /// See [`xShmUnmap`](https://www.sqlite.org/c3ref/io_methods.html#xShmUnmap).
    fn unmap_shm(&mut self, delete: bool) -> Result<()>;

    /// Issues a memory barrier.
    ///
    /// See [`xShmBarrier`](https://www.sqlite.org/c3ref/io_methods.html#xShmBarrier).
    fn barrier(&mut self) {
        atomic::fence(Ordering::SeqCst);
    }
}

/// Lock mode for WAL shared-memory operations.
///
/// See [Flags for `xShmLock`](https://sqlite.org/c3ref/c_shm_exclusive.html).
#[derive(Copy, Clone, Debug)]
pub enum WalLockMode {
    /// Mirrors [`SQLITE_SHM_SHARED`](https://sqlite.org/c3ref/c_shm_exclusive.html).
    Shared,
    /// Mirrors [`SQLITE_SHM_EXCLUSIVE`](https://sqlite.org/c3ref/c_shm_exclusive.html).
    Exclusive,
}

impl WalLockMode {
    /// Converts from raw SQLite flags.
    pub fn try_from_raw(raw: c_int) -> Result<Self> {
        if (raw & sqlite3::SQLITE_SHM_SHARED) != 0 {
            Ok(WalLockMode::Shared)
        } else if (raw & sqlite3::SQLITE_SHM_EXCLUSIVE) != 0 {
            Ok(WalLockMode::Exclusive)
        } else {
            Err(Error::new(sqlite3::SQLITE_MISUSE))
        }
    }

    /// Converts to raw SQLite flags.
    pub fn to_raw(&self) -> c_int {
        match self {
            WalLockMode::Shared => sqlite3::SQLITE_SHM_SHARED,
            WalLockMode::Exclusive => sqlite3::SQLITE_SHM_EXCLUSIVE,
        }
    }
}

/// A set representing WAL locks.
///
/// See [WAL Locks](https://sqlite.org/walformat.html#locks).
pub struct WalLock {
    mask: u16,
}

impl WalLock {
    /// Slot index reserved for the WAL write lock.
    pub const WAL_WRITE_LOCK: usize = 0;
    /// Slot index reserved for the WAL checkpoint lock.
    pub const WAL_CKPT_LOCK: usize = 1;
    /// Slot index reserved for the WAL recovery lock.
    pub const WAL_RECOVER_LOCK: usize = 2;
    /// Slot index of the first WAL read lock.
    pub const WAL_READ_LOCK_0: usize = 3;

    /// Creates a set from an offset and count.
    pub const fn new(offset: usize, n: usize) -> Self {
        let mask: u16 = (1 << (offset + n)) - (1 << offset);
        WalLock { mask }
    }

    /// Creates a set from a raw u16 mask.
    pub const fn from_mask(mask: u16) -> Self {
        WalLock { mask }
    }

    /// Returns true if the write lock is included.
    pub fn write(&self) -> bool {
        self.mask & (1 << Self::WAL_WRITE_LOCK) != 0
    }

    /// Returns true if the checkpoint lock is included.
    pub fn checkpoint(&self) -> bool {
        self.mask & (1 << Self::WAL_CKPT_LOCK) != 0
    }

    /// Returns true if the recover lock is included.
    pub fn recover(&self) -> bool {
        self.mask & (1 << Self::WAL_RECOVER_LOCK) != 0
    }

    /// Returns true if the given read lock index is included.
    /// index must be in 0..5.
    pub fn read(&self, index: usize) -> Result<bool> {
        if index >= 5 {
            return Err(Error::new(sqlite3::SQLITE_MISUSE));
        }
        Ok(self.mask & (1 << (Self::WAL_READ_LOCK_0 + index)) != 0)
    }
}

/// Represents file I/O behaviors for in-memory page access with a [`Vfs`].
///
/// This trait is optional and corresponds to ([`sqlite3_io_methods` v3](https://www.sqlite.org/c3ref/io_methods.html)).
pub trait VfsFetchFile: VfsFile {
    /// Fetches a page region into memory.
    ///
    /// See [`xFetch`](https://www.sqlite.org/c3ref/io_methods.html#xFetch).
    fn fetch(&mut self, offset: i64, amount: NonZero<usize>) -> Result<Option<&mut [u8]>>;

    /// Releases a previously fetched region.
    ///
    /// See [`xUnfetch`](https://www.sqlite.org/c3ref/io_methods.html#xUnfetch).
    fn unfetch(&mut self, offset: i64, ptr: NonNull<u8>) -> Result<()>;

    /// Releases all previously fetched regions.
    ///
    /// See [`xUnfetch`](https://www.sqlite.org/c3ref/io_methods.html#xUnfetch).
    fn unfetch_all(&mut self) -> Result<()>;
}

/// File locking levels. See [File Locking](https://www.sqlite.org/lockingv3.html).
#[derive(Copy, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LockLevel {
    /// Mirrors SQLITE_LOCK_NONE.
    None,
    /// Mirrors SQLITE_LOCK_SHARED.
    Shared,
    /// Mirrors SQLITE_LOCK_RESERVED.
    Reserved,
    /// Mirrors SQLITE_LOCK_PENDING.
    Pending,
    /// Mirrors SQLITE_LOCK_EXCLUSIVE.
    Exclusive,
}

impl LockLevel {
    /// Converts SQLite lock constants into [`LockLevel`].
    pub fn from_raw(level: c_int) -> Self {
        match level {
            sqlite3::SQLITE_LOCK_NONE => LockLevel::None,
            sqlite3::SQLITE_LOCK_SHARED => LockLevel::Shared,
            sqlite3::SQLITE_LOCK_RESERVED => LockLevel::Reserved,
            sqlite3::SQLITE_LOCK_PENDING => LockLevel::Pending,
            sqlite3::SQLITE_LOCK_EXCLUSIVE => LockLevel::Exclusive,
            _ => panic!("invalid lock level"),
        }
    }

    /// Converts [`LockLevel`] back into SQLite lock constants.
    pub fn to_raw(&self) -> c_int {
        match self {
            LockLevel::None => sqlite3::SQLITE_LOCK_NONE,
            LockLevel::Shared => sqlite3::SQLITE_LOCK_SHARED,
            LockLevel::Reserved => sqlite3::SQLITE_LOCK_RESERVED,
            LockLevel::Pending => sqlite3::SQLITE_LOCK_PENDING,
            LockLevel::Exclusive => sqlite3::SQLITE_LOCK_EXCLUSIVE,
        }
    }
}

/// I/O characteristics reported by [`VfsFile::io_capabilities`].
///
/// See [Device Characteristics](https://sqlite.org/c3ref/c_iocap_atomic.html).
#[derive(Clone, Debug, Default)]
pub struct IoCapabilities {
    /// Mirrors `SQLITE_IOCAP_ATOMIC*`; captures the atomic write guarantee.
    pub atomic_write: AtomicWrite,
    /// Mirrors `SQLITE_IOCAP_SAFE_APPEND`; data grows before the file length.
    pub safe_append: bool,
    /// Mirrors `SQLITE_IOCAP_SEQUENTIAL`; writes reach storage in call order.
    pub sequential: bool,
    /// Mirrors `SQLITE_IOCAP_UNDELETABLE_WHEN_OPEN`; prevents unlink while open.
    pub undeletable_when_open: bool,
    /// Mirrors `SQLITE_IOCAP_POWERSAFE_OVERWRITE`; crashes leave neighbors intact.
    pub powersafe_overwrite: bool,
    /// Mirrors `SQLITE_IOCAP_IMMUTABLE`; indicates read-only backing media.
    pub immutable: bool,
    /// Mirrors `SQLITE_IOCAP_BATCH_ATOMIC`; honors begin/commit atomic writes.
    pub batch_atomic: bool,
    /// Mirrors `SQLITE_IOCAP_SUBPAGE_READ`; permits unaligned reads beyond header.
    pub subpage_read: bool,
}

impl IoCapabilities {
    /// Builds the structured capabilities from raw SQLite flag bits.
    pub fn from_raw(raw: c_int) -> Self {
        let write_cap = if raw == 0 {
            AtomicWrite::Never
        } else if (raw & sqlite3::SQLITE_IOCAP_ATOMIC) != 0 {
            AtomicWrite::Always
        } else {
            AtomicWrite::Block {
                size_512: (raw & sqlite3::SQLITE_IOCAP_ATOMIC512) != 0,
                size_1k: (raw & sqlite3::SQLITE_IOCAP_ATOMIC1K) != 0,
                size_2k: (raw & sqlite3::SQLITE_IOCAP_ATOMIC2K) != 0,
                size_4k: (raw & sqlite3::SQLITE_IOCAP_ATOMIC4K) != 0,
                size_8k: (raw & sqlite3::SQLITE_IOCAP_ATOMIC8K) != 0,
                size_16k: (raw & sqlite3::SQLITE_IOCAP_ATOMIC16K) != 0,
                size_32k: (raw & sqlite3::SQLITE_IOCAP_ATOMIC32K) != 0,
                size_64k: (raw & sqlite3::SQLITE_IOCAP_ATOMIC64K) != 0,
            }
        };

        IoCapabilities {
            atomic_write: write_cap,
            safe_append: (raw & sqlite3::SQLITE_IOCAP_SAFE_APPEND) != 0,
            sequential: (raw & sqlite3::SQLITE_IOCAP_SEQUENTIAL) != 0,
            undeletable_when_open: (raw & sqlite3::SQLITE_IOCAP_UNDELETABLE_WHEN_OPEN) != 0,
            powersafe_overwrite: (raw & sqlite3::SQLITE_IOCAP_POWERSAFE_OVERWRITE) != 0,
            immutable: (raw & sqlite3::SQLITE_IOCAP_IMMUTABLE) != 0,
            batch_atomic: (raw & sqlite3::SQLITE_IOCAP_BATCH_ATOMIC) != 0,
            subpage_read: (raw & sqlite3::SQLITE_IOCAP_SUBPAGE_READ) != 0,
        }
    }

    /// Converts the structured capabilities to raw SQLite flag bits.
    pub fn to_raw(&self) -> c_int {
        let mut flags = 0;

        let IoCapabilities {
            atomic_write: write_cap,
            safe_append,
            sequential,
            undeletable_when_open,
            powersafe_overwrite,
            immutable,
            batch_atomic,
            subpage_read,
        } = self;

        match *write_cap {
            AtomicWrite::Never => {}
            AtomicWrite::Block {
                size_512,
                size_1k,
                size_2k,
                size_4k,
                size_8k,
                size_16k,
                size_32k,
                size_64k,
            } => {
                if size_512 {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC512;
                }
                if size_1k {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC1K;
                }
                if size_2k {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC2K;
                }
                if size_4k {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC4K;
                }
                if size_8k {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC8K;
                }
                if size_16k {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC16K;
                }
                if size_32k {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC32K;
                }
                if size_64k {
                    flags |= sqlite3::SQLITE_IOCAP_ATOMIC64K;
                }
            }
            AtomicWrite::Always => {
                flags |= sqlite3::SQLITE_IOCAP_ATOMIC;
            }
        }
        if *safe_append {
            flags |= sqlite3::SQLITE_IOCAP_SAFE_APPEND;
        }
        if *sequential {
            flags |= sqlite3::SQLITE_IOCAP_SEQUENTIAL;
        }
        if *undeletable_when_open {
            flags |= sqlite3::SQLITE_IOCAP_UNDELETABLE_WHEN_OPEN;
        }
        if *powersafe_overwrite {
            flags |= sqlite3::SQLITE_IOCAP_POWERSAFE_OVERWRITE;
        }
        if *immutable {
            flags |= sqlite3::SQLITE_IOCAP_IMMUTABLE;
        }
        if *batch_atomic {
            flags |= sqlite3::SQLITE_IOCAP_BATCH_ATOMIC;
        }
        if *subpage_read {
            flags |= sqlite3::SQLITE_IOCAP_SUBPAGE_READ;
        }
        flags
    }
}

/// Atomic write capabilities.
#[derive(Clone, Debug, Default)]
pub enum AtomicWrite {
    /// No SQLITE_IOCAP_ATOMIC* bits are set.
    #[default]
    Never,
    /// Mirrors size-specific `SQLITE_IOCAP_ATOMICnnn` bitfields.
    Block {
        /// Mirrors to `SQLITE_IOCAP_ATOMIC512`.
        size_512: bool,
        /// Mirrors to `SQLITE_IOCAP_ATOMIC1K`.
        size_1k: bool,
        /// Mirrors to `SQLITE_IOCAP_ATOMIC2K`.
        size_2k: bool,
        /// Mirrors to `SQLITE_IOCAP_ATOMIC4K`.
        size_4k: bool,
        /// Mirrors to `SQLITE_IOCAP_ATOMIC8K`.
        size_8k: bool,
        /// Mirrors to `SQLITE_IOCAP_ATOMIC16K`.
        size_16k: bool,
        /// Mirrors to `SQLITE_IOCAP_ATOMIC32K`.
        size_32k: bool,
        /// Mirrors to `SQLITE_IOCAP_ATOMIC64K`.
        size_64k: bool,
    },
    /// Mirrors SQLITE_IOCAP_ATOMIC.
    Always,
}

/// RAII guard that unregisters a VFS on drop.
#[must_use]
pub struct VfsRegistrationGuard<V>(Arc<VfsStorage<V>>);

impl<V> Deref for VfsRegistrationGuard<V> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        &self.0.vfs
    }
}

impl<V> Drop for VfsRegistrationGuard<V> {
    fn drop(&mut self) {
        let rc = unsafe {
            sqlite3::sqlite3_vfs_unregister(&self.0.base as *const sqlite3_vfs as *mut _)
        };
        if rc != sqlite3::SQLITE_OK {
            panic!("cannot unregister VFS: {}", Error::new(rc));
        }
    }
}

/// Represents a lack of support for a category of file I/O methods.
pub struct NoSupport;

/// Stores info the I/O method categories supported by a [`Vfs`].
pub struct VfsSupport<S> {
    _support: PhantomData<S>,
}

/// Extension trait to provide a pre-computed [`Vfs`] method table.
/// This is only necessary due to Rust's current lack of const generics over types.
#[doc(hidden)]
pub trait VfsMethodTableExt {
    /// The available methods derived from a [`VfsSupport`].
    const METHODS: sqlite3_io_methods;
}

// Base implementation without WAL and Fetch support.
impl<T> VfsSupport<(T, NoSupport, NoSupport)>
where
    T: Vfs,
{
    const fn methods() -> sqlite3_io_methods {
        sqlite3_io_methods {
            iVersion: 1,
            xClose: Some(x_close::<T>),
            xRead: Some(x_read::<T>),
            xWrite: Some(x_write::<T>),
            xTruncate: Some(x_truncate::<T>),
            xSync: Some(x_sync::<T>),
            xFileSize: Some(x_file_size::<T>),
            xLock: Some(x_lock::<T>),
            xUnlock: Some(x_unlock::<T>),
            xCheckReservedLock: Some(x_check_reserved_lock::<T>),
            xFileControl: Some(x_file_control::<T>),
            xSectorSize: Some(x_sector_size::<T>),
            xDeviceCharacteristics: Some(x_device_characteristics::<T>),

            // No WAL support
            xShmMap: None,
            xShmLock: None,
            xShmBarrier: None,
            xShmUnmap: None,

            // No Fetch support
            xFetch: None,
            xUnfetch: None,
        }
    }
}

impl<T> VfsMethodTableExt for VfsSupport<(T, NoSupport, NoSupport)>
where
    T: Vfs,
{
    const METHODS: sqlite3_io_methods = Self::methods();
}

// Wal support implementation
impl<T, F> VfsSupport<(T, F, NoSupport)>
where
    T: Vfs<File = F>,
    F: VfsWalFile,
{
    const fn methods() -> sqlite3_io_methods {
        let mut methods = VfsSupport::<(T, NoSupport, NoSupport)>::methods();
        methods.iVersion = 2;
        methods.xShmMap = Some(x_shm_map::<T, F>);
        methods.xShmLock = Some(x_shm_lock::<T, F>);
        methods.xShmBarrier = Some(x_shm_barrier::<T, F>);
        methods.xShmUnmap = Some(x_shm_unmap::<T, F>);
        methods
    }
}

impl<T, F> VfsMethodTableExt for VfsSupport<(T, F, NoSupport)>
where
    T: Vfs<File = F>,
    F: VfsWalFile,
{
    const METHODS: sqlite3_io_methods = Self::methods();
}

// Fetch support implementation
impl<T, F> VfsSupport<(T, NoSupport, F)>
where
    T: Vfs<File = F>,
    F: VfsFetchFile,
{
    const fn methods() -> sqlite3_io_methods {
        let mut methods = VfsSupport::<(T, NoSupport, NoSupport)>::methods();
        methods.iVersion = 3;
        methods.xFetch = Some(x_fetch::<T, F>);
        methods.xUnfetch = Some(x_unfetch::<T, F>);
        methods
    }
}

impl<T, F> VfsMethodTableExt for VfsSupport<(T, NoSupport, F)>
where
    T: Vfs<File = F>,
    F: VfsFetchFile,
{
    const METHODS: sqlite3_io_methods = Self::methods();
}

impl<T, F> VfsSupport<(T, F, F)>
where
    T: Vfs<File = F>,
    F: VfsFetchFile + VfsWalFile,
{
    const fn methods() -> sqlite3_io_methods {
        let mut methods = VfsSupport::<(T, F, NoSupport)>::methods();
        methods.iVersion = 3;
        methods.xFetch = Some(x_fetch::<T, F>);
        methods.xUnfetch = Some(x_unfetch::<T, F>);
        methods
    }
}

impl<T, F> VfsMethodTableExt for VfsSupport<(T, F, F)>
where
    T: Vfs<File = F>,
    F: VfsFetchFile + VfsWalFile,
{
    const METHODS: sqlite3_io_methods = Self::methods();
}

/// Builder for VFS registration.
pub struct VfsRegistration<T, M> {
    vfs: T,
    max_pathlen: usize,
    make_default: bool,
    method_table: PhantomData<M>,
}

impl<T: Vfs> VfsRegistration<T, VfsSupport<(T, NoSupport, NoSupport)>> {
    /// Creates a new VFS registration builder.
    pub fn new(vfs: T) -> Self {
        Self {
            vfs,
            max_pathlen: 512,
            make_default: false,
            method_table: PhantomData,
        }
    }
}

impl<T: Vfs, M: VfsMethodTableExt> VfsRegistration<T, M> {
    /// Registers the VFS with SQLite.
    pub fn register(self, name: &str) -> Result<VfsRegistrationGuard<T>> {
        if name.is_empty() {
            return Err(Error::new(sqlite3::SQLITE_MISUSE));
        }

        let Self {
            vfs,
            max_pathlen,
            make_default,
            method_table: _,
        } = self;

        let storage = Arc::new_cyclic(move |storage| {
            let name = match CString::new(name) {
                Ok(name) => name,
                Err(_) => unreachable!(), // `&str` cannot contain '\0'.
            };

            let base = sqlite3_vfs {
                iVersion: 2,
                szOsFile: mem::size_of::<VfsFileStorage<T>>() as c_int,
                mxPathname: max_pathlen as c_int,
                pNext: ptr::null_mut(),
                zName: name.as_ptr(),
                pAppData: storage.as_ptr() as *mut c_void,
                xOpen: Some(x_open::<T, M>),
                xDelete: Some(x_delete::<T>),
                xAccess: Some(x_access::<T>),
                xFullPathname: Some(x_full_pathname::<T>),

                // FIXME: support for non-unix systems
                xDlOpen: Some(x_dlopen),
                xDlError: Some(x_dlerror),
                xDlSym: Some(x_dlsym),
                xDlClose: Some(x_dlclose),

                xRandomness: Some(x_randomness::<T>),
                xSleep: Some(x_sleep::<T>),
                xCurrentTime: Some(x_get_current_time_deprecated),
                xGetLastError: Some(x_get_last_error::<T>),
                xCurrentTimeInt64: Some(x_get_current_time::<T>),

                // NOTE: nice to have, but not strictly needed
                xSetSystemCall: None,
                xGetSystemCall: None,
                xNextSystemCall: None,
            };
            VfsStorage { base, name, vfs }
        });

        let rc = unsafe {
            sqlite3::sqlite3_vfs_register(
                &storage.base as *const sqlite3_vfs as *mut _,
                make_default as c_int,
            )
        };
        if rc != sqlite3::SQLITE_OK {
            return Err(Error::new(rc));
        }
        Ok(VfsRegistrationGuard(storage))
    }
}

impl<T, M> VfsRegistration<T, M> {
    /// Sets the maximum path length supported by the VFS.
    pub fn max_pathlen(mut self, len: usize) -> Self {
        self.max_pathlen = len;
        self
    }

    /// Makes this VFS the default one.
    pub fn make_default(mut self) -> Self {
        self.make_default = true;
        self
    }
}

impl<T: Vfs, Wal> VfsRegistration<T, VfsSupport<(T, Wal, NoSupport)>>
where
    T::File: VfsFetchFile,
{
    /// Enables fetch support (io_methods v3).
    pub fn with_fetch(self) -> VfsRegistration<T, VfsSupport<(T, Wal, T::File)>> {
        let Self {
            vfs,
            max_pathlen,
            make_default,
            method_table: _,
        } = self;
        VfsRegistration {
            vfs,
            max_pathlen,
            make_default,
            method_table: PhantomData,
        }
    }
}

impl<T: Vfs, Fetch> VfsRegistration<T, VfsSupport<(T, NoSupport, Fetch)>>
where
    T::File: VfsWalFile,
{
    /// Enables WAL support (io_methods v2).
    pub fn with_wal(self) -> VfsRegistration<T, VfsSupport<(T, T::File, Fetch)>> {
        let Self {
            vfs,
            max_pathlen,
            make_default,
            method_table: _,
        } = self;
        VfsRegistration {
            vfs,
            max_pathlen,
            make_default,
            method_table: PhantomData,
        }
    }
}

/// Stores a registered VFS instance with its SQLite metadata and lifecycle management.
struct VfsStorage<V> {
    base: sqlite3_vfs,
    name: CString,
    vfs: V,
}

impl<T> VfsStorage<T> {
    unsafe fn from_raw(ptr: *mut sqlite3_vfs) -> Arc<Self> {
        let vfs = unsafe { ptr.as_ref() }.expect("cannot get reference to empty vfs storage");
        let storage_ptr = vfs.pAppData.cast::<VfsStorage<T>>();
        if storage_ptr.is_null() {
            panic!("cannot get reference to empty vfs storage");
        }
        unsafe {
            Arc::increment_strong_count(storage_ptr);
            Arc::from_raw(storage_ptr)
        }
    }
}

#[repr(C)]
struct VfsFileStorage<T: Vfs> {
    base: sqlite3_file,
    state: FileStorageState<T>,
}

enum FileStorageState<T: Vfs> {
    Open {
        vfs: Arc<VfsStorage<T>>,
        file: T::File,
    },
    Closed,
}

impl<T: Vfs> VfsFileStorage<T> {
    /// Returns a mutable reference to the VfsFileStorage from a raw pointer.
    /// SAFETY: The reference is valid as long as the underlying pointer is valid,
    /// and should generally be used only within the scope of a function called by SQLite.
    unsafe fn from_raw<'sqlite>(ptr: *mut sqlite3_file) -> &'sqlite mut Self {
        unsafe {
            ptr.cast::<VfsFileStorage<T>>()
                .as_mut()
                .expect("cannot get reference to empty file storage")
        }
    }

    fn file(&mut self) -> &mut T::File {
        match &mut self.state {
            FileStorageState::Open { file, .. } => file,
            FileStorageState::Closed => panic!("internal error: file already closed"),
        }
    }

    fn vfs(&self) -> &VfsStorage<T> {
        match &self.state {
            FileStorageState::Open { vfs, .. } => vfs,
            FileStorageState::Closed => panic!("internal error: file already closed"),
        }
    }
}

unsafe extern "C" fn x_open<T: Vfs, M: VfsMethodTableExt>(
    vfs: *mut sqlite3_vfs,
    filename: sqlite3_filename,
    out: *mut sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    let path = if filename.is_null() {
        if (flags & sqlite3::SQLITE_OPEN_DELETEONCLOSE) == 0 {
            return sqlite3::SQLITE_MISUSE;
        }
        None
    } else {
        Some(VfsPath::new(unsafe { CStr::from_ptr(filename) }))
    };

    let vfs_storage = unsafe { VfsStorage::<T>::from_raw(vfs) };

    const SQLITE_FILE_TYPE_MASK: c_int = 0x0FFF00;
    let file_type = match flags & SQLITE_FILE_TYPE_MASK {
        sqlite3::SQLITE_OPEN_MAIN_DB => {
            FileType::MainDb(path.expect("internal error: NULL database path"))
        }
        sqlite3::SQLITE_OPEN_MAIN_JOURNAL => {
            FileType::MainJournal(path.expect("internal error: NULL database path"))
        }
        sqlite3::SQLITE_OPEN_TEMP_DB => FileType::TempDb,
        sqlite3::SQLITE_OPEN_TEMP_JOURNAL => FileType::TempJournal,
        sqlite3::SQLITE_OPEN_TRANSIENT_DB => FileType::TransientDb,
        sqlite3::SQLITE_OPEN_SUBJOURNAL => {
            FileType::Subjournal(path.expect("internal error: NULL database path"))
        }
        sqlite3::SQLITE_OPEN_SUPER_JOURNAL => {
            FileType::SuperJournal(path.expect("internal error: NULL database path"))
        }
        sqlite3::SQLITE_OPEN_WAL => {
            FileType::Wal(path.expect("internal error: NULL database path"))
        }
        _ => panic!("internal error: invalid file type"),
    };
    let vfs_flags = VfsOpenFlags::from_bits_retain(flags);
    let open_file = match vfs_storage.vfs.open(file_type, vfs_flags) {
        Ok(r) => r,
        Err(e) => return e.extended_code,
    };
    if !out_flags.is_null() {
        unsafe {
            out_flags.write(if open_file.readonly {
                flags | sqlite3::SQLITE_OPEN_READONLY
            } else {
                flags
            });
        }
    }
    let methods: &'static sqlite3_io_methods = &M::METHODS;
    let storage = VfsFileStorage {
        base: sqlite3_file {
            pMethods: methods as *const _,
        },
        state: FileStorageState::Open {
            vfs: vfs_storage,
            file: open_file.file,
        },
    };
    unsafe {
        out.cast::<VfsFileStorage<T>>().write(storage);
    }
    sqlite3::SQLITE_OK
}

unsafe extern "C" fn x_delete<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    filename: *const c_char,
    sync: c_int,
) -> c_int {
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    let name = unsafe { CStr::from_ptr(filename) };
    storage.vfs.delete(VfsPath(name), sync != 0).into_rc()
}

unsafe extern "C" fn x_access<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    filename: *const c_char,
    flags: c_int,
    outcome: *mut c_int,
) -> c_int {
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    let name = unsafe { CStr::from_ptr(filename) };
    let out = unsafe {
        outcome
            .as_mut()
            .expect("internal error: invalid output pointer for xAccess")
    };

    let result = match flags {
        sqlite3::SQLITE_ACCESS_EXISTS => storage.vfs.exists(VfsPath(name)),
        sqlite3::SQLITE_ACCESS_READ => storage.vfs.can_read(VfsPath(name)),
        sqlite3::SQLITE_ACCESS_READWRITE => storage.vfs.can_write(VfsPath(name)),
        _ => return sqlite3::SQLITE_MISUSE,
    };

    result.write_to_output(out).into_rc()
}

unsafe extern "C" fn x_full_pathname<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    name: *const c_char,
    n_out: c_int,
    out: *mut c_char,
) -> c_int {
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    let name = unsafe { CStr::from_ptr(name) };
    let out_len = mem::size_of::<c_char>() * n_out as usize;
    let out_slice = unsafe { slice::from_raw_parts_mut(out as *mut u8, out_len) };

    storage
        .vfs
        .write_full_path(VfsPath(name), &mut out_slice[..(out_len - 1)])
        .map(|len| {
            // Null-terminate
            out_slice[len] = 0;
        })
        .into_rc()
}

// On linux, these function are available by default in libc. On other platforms `-ldl` is probably needed.
// Also, this code is unix-only and does not work on windows.
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *mut c_char;
}

#[cfg(unix)]
unsafe extern "C" fn x_dlopen(_: *mut sqlite3_vfs, filename: *const c_char) -> *mut c_void {
    // Linux only, but that's ok as dqlite-utils is for linux only
    const RTLD_NOW: c_int = 1;
    const RTLD_GLOBAL: c_int = 4;

    unsafe { dlopen(filename, RTLD_NOW | RTLD_GLOBAL) }
}

#[cfg(windows)]
unsafe extern "C" fn x_dlopen(_: *mut sqlite3_vfs, filename: *const c_char) -> *mut c_void {
    todo!();
}

#[cfg(unix)]
unsafe extern "C" fn x_dlerror(_: *mut sqlite3_vfs, n: c_int, out: *mut c_char) {
    unsafe {
        let err = dlerror();
        if !err.is_null() {
            sqlite3::sqlite3_snprintf(n, out, c"%s".as_ptr() as *const c_char, err);
        }
    }
}

#[cfg(windows)]
unsafe extern "C" fn x_dlerror(_: *mut sqlite3_vfs, n: c_int, out: *mut c_char) {
    todo!();
}

// FIXME: the return type of this function is wrong:
//  - either it should be a pointer to a function with generic signature in C like void(*)()
//  - or it should be the only actual use this function is used for:
//      unsafe extern "C" fn(*mut sqlite3, *mut *mut char, *const sqlite3_api_routines) -> c_int.
// See https://github.com/rust-lang/rust-bindgen/issues/2713
#[cfg(unix)]
unsafe extern "C" fn x_dlsym(
    _: *mut sqlite3_vfs,
    p: *mut c_void,
    sym: *const c_char,
) -> Option<unsafe extern "C" fn(*mut sqlite3_vfs, *mut c_void, *const i8)> {
    Some(unsafe {
        mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut sqlite3_vfs, *mut c_void, *const i8)>(
            dlsym(p, sym),
        )
    })
}

#[cfg(windows)]
unsafe extern "C" fn x_dlsym(
    _: *mut sqlite3_vfs,
    p: *mut c_void,
    sym: *const c_char,
) -> Option<unsafe extern "C" fn(*mut sqlite3_vfs, *mut c_void, *const i8)> {
    todo!();
}

#[cfg(unix)]
unsafe extern "C" fn x_dlclose(_: *mut sqlite3_vfs, handle: *mut core::ffi::c_void) {
    unsafe { dlclose(handle) };
}

#[cfg(windows)]
unsafe extern "C" fn x_dlclose(_: *mut sqlite3_vfs, handle: *mut core::ffi::c_void) {
    todo!();
}

unsafe extern "C" fn x_randomness<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    n_out: c_int,
    out: *mut c_char,
) -> c_int {
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    storage
        .vfs
        .fill_random_bytes(unsafe { slice::from_raw_parts_mut(out as *mut u8, n_out as usize) })
        .into_rc()
}

unsafe extern "C" fn x_sleep<T: Vfs>(vfs: *mut sqlite3_vfs, microseconds: c_int) -> c_int {
    if microseconds <= 0 {
        return 0;
    }
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    storage
        .vfs
        .sleep(Duration::from_micros(microseconds as u64));
    microseconds
}

unsafe extern "C" fn x_get_current_time_deprecated(_: *mut sqlite3_vfs, _: *mut f64) -> c_int {
    panic!("deprecated xCurrentTime called");
}

unsafe extern "C" fn x_get_current_time<T: Vfs>(vfs: *mut sqlite3_vfs, out_ptr: *mut i64) -> c_int {
    const UNIX_EPOCH: i64 = 24405875i64 * 8640000i64;

    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    let out = unsafe {
        out_ptr
            .as_mut()
            .expect("internal error: invalid output pointer for xCurrentTimeInt64")
    };
    storage
        .vfs
        .now()
        .map(|time| {
            time.duration_since(SystemTime::UNIX_EPOCH)
                .expect("internal error: now is before unix epoch")
                .as_millis() as i64
                + UNIX_EPOCH
        })
        .write_to_output(out)
        .into_rc()
}

unsafe extern "C" fn x_get_last_error<T: Vfs>(
    vfs: *mut sqlite3_vfs,
    _: c_int,
    _: *mut c_char,
) -> i32 {
    let storage = unsafe { VfsStorage::<T>::from_raw(vfs) };
    storage.vfs.last_error()
}

unsafe extern "C" fn x_close<T: Vfs>(file: *mut sqlite3_file) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    storage.state = FileStorageState::Closed;
    sqlite3::SQLITE_OK
}

unsafe extern "C" fn x_read<T: Vfs>(
    file: *mut sqlite3_file,
    data: *mut c_void,
    amount: i32,
    offset: i64,
) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let buf = unsafe { slice::from_raw_parts_mut(data as *mut u8, amount as usize) };
    file.read_at(buf, offset as u64)
        .and_then(|size| {
            if size < buf.len() {
                // Zero-fill the rest of the buffer
                buf[size..].fill(0);
                Err(Error::new(sqlite3::SQLITE_IOERR_SHORT_READ))
            } else {
                Ok(())
            }
        })
        .into_rc()
}

unsafe extern "C" fn x_write<T: Vfs>(
    file: *mut sqlite3_file,
    data: *const c_void,
    amount: i32,
    offset: i64,
) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let buf = unsafe { slice::from_raw_parts(data as *const u8, amount as usize) };
    file.write_at(buf, offset as u64).into_rc()
}

unsafe extern "C" fn x_truncate<T: Vfs>(file: *mut sqlite3_file, size: i64) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.truncate(size as u64).into_rc()
}

unsafe extern "C" fn x_sync<T: Vfs>(file: *mut sqlite3_file, flags: c_int) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let options = SyncOptions {
        full: (flags & sqlite3::SQLITE_SYNC_FULL) != 0,
        data_only: (flags & sqlite3::SQLITE_SYNC_DATAONLY) != 0,
    };
    file.sync(options).into_rc()
}

unsafe extern "C" fn x_file_size<T: Vfs>(
    file: *mut sqlite3_file,
    out_ptr: *mut sqlite3_int64,
) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let out = unsafe {
        out_ptr
            .as_mut()
            .expect("internal error: invalid output pointer for xFileSize")
    };
    file.len()
        .map(|size| size as i64)
        .write_to_output(out)
        .into_rc()
}

unsafe extern "C" fn x_lock<T: Vfs>(file: *mut sqlite3_file, level: c_int) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let lock_level = LockLevel::from_raw(level);
    file.lock(lock_level).into_rc()
}

unsafe extern "C" fn x_unlock<T: Vfs>(file: *mut sqlite3_file, level: c_int) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let lock_level = LockLevel::from_raw(level);
    file.unlock(lock_level).into_rc()
}

unsafe extern "C" fn x_check_reserved_lock<T: Vfs>(
    file: *mut sqlite3_file,
    out_ptr: *mut c_int,
) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let out = unsafe {
        out_ptr
            .as_mut()
            .expect("internal error: invalid output pointer for xCheckReservedLock")
    };
    file.is_write_locked().write_to_output(out).into_rc()
}

unsafe extern "C" fn x_file_control<T: Vfs>(
    file: *mut sqlite3_file,
    op: c_int,
    arg: *mut c_void,
) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    match op {
        sqlite3::SQLITE_FCNTL_LOCKSTATE => {
            let level = file.lock_level();
            unsafe { arg.cast::<c_int>().write(level.to_raw()) };
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_LAST_ERRNO => {
            let errno = file.last_errno();
            unsafe { arg.cast::<c_int>().write(errno) };
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_SIZE_HINT => {
            let size = unsafe { arg.cast::<i64>().read() };
            file.hint_size(size).into_rc()
        }
        sqlite3::SQLITE_FCNTL_CHUNK_SIZE => {
            let size = unsafe { arg.cast::<c_int>().read() } as u32;
            file.set_chunk_size(size).into_rc()
        }
        sqlite3::SQLITE_FCNTL_OVERWRITE => {
            let size = unsafe { arg.cast::<sqlite3_int64>().read() } as u64;
            file.hint_overwrite(size).into_rc()
        }
        sqlite3::SQLITE_FCNTL_VFSNAME => {
            let name_ptr = arg.cast::<*mut c_char>();
            unsafe {
                name_ptr.write(sqlite3::sqlite3_mprintf(
                    c"%s".as_ptr() as *const c_char,
                    storage.vfs().name.as_ptr(),
                ));
            }
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_PRAGMA => {
            // The arg is a pointer to an array of 3 *char:
            //   arg[0]: output **char (either error or result)
            //   arg[1]: input *char (pragma name)
            //   arg[2]: input *char or NULL (pragma argument)
            let args = unsafe { slice::from_raw_parts_mut(arg.cast::<*mut c_char>(), 3) };
            let name = str::from_utf8(unsafe { CStr::from_ptr(args[1]) }.to_bytes())
                .expect("internal error: pragma name is not valid utf-8");
            let arg_raw = args[2];
            let arg = if !arg_raw.is_null() {
                Some(
                    str::from_utf8(unsafe { CStr::from_ptr(arg_raw) }.to_bytes())
                        .expect("internal error: pragma argument is not valid utf-8"),
                )
            } else {
                None
            };
            match file.pragma(name, arg) {
                Ok(result) => {
                    // Fun stuff: when a custom PRAGMA returns no result, but still succeeds,
                    // SQLite uses the result as both the result of the PRAGMA *and* the column name.
                    // So, if NULL, SQLite will return a column with `NULL` name and a `NULL` value,
                    // and yet the column count will be 1. This makes an assertion fail from rusqlite
                    // which expects the column name to be non-null as SQLite documentation states:
                    //
                    //   If sqlite3_malloc() fails during the processing of either routine (for
                    //   example during a conversion from UTF-8 to UTF-16) then a NULL pointer is returned.
                    //
                    // Admittedly, the above does not explicitly say that the column name cannot be NULL,
                    // but it is still unexpected IMHO.
                    // Now, clearly this is a SQLite quirk/documentation bug, but to work around it, we
                    // can just use
                    //  - the argument, if present (like `PRAGMA journal_mode = XXX`` does)
                    //  - or the pragma name
                    // as the column name.
                    let result_string = result.as_deref().or(arg).unwrap_or(name);
                    args[0] = unsafe {
                        sqlite3::sqlite3_mprintf(
                            c"%.*s".as_ptr() as *const c_char,
                            result_string.len(),
                            result_string.as_bytes(),
                        )
                    };
                    sqlite3::SQLITE_OK
                }
                Err(PragmaError { code, message }) => {
                    if let Some(result) = message {
                        args[0] = unsafe {
                            sqlite3::sqlite3_mprintf(
                                c"%.*s".as_ptr() as *const c_char,
                                result.len(),
                                result.as_bytes(),
                            )
                        };
                    }
                    code.extended_code
                }
            }
        }
        sqlite3::SQLITE_FCNTL_MMAP_SIZE => {
            let size = unsafe { arg.cast::<sqlite3_int64>().as_mut() }.expect(
                "internal error: arg for SQLITE_FCNTL_MMAP_SIZE must point to an sqlite3_int64",
            );
            let new_size = *size;
            let result = if new_size < 0 {
                file.mmap_size()
            } else {
                file.set_mmap_size(new_size as u64)
            };
            result
                .map(|size| size as sqlite3_int64)
                .write_to_output(size)
                .into_rc()
        }
        sqlite3::SQLITE_FCNTL_HAS_MOVED => {
            unsafe { arg.cast::<c_int>().write(file.has_moved() as c_int) };
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_SYNC => {
            let super_journal_raw = arg.cast::<c_char>();
            if super_journal_raw.is_null() {
                return file.pre_sync_single_db().into_rc();
            }
            file.pre_sync_multiple_db(VfsPath(unsafe { CStr::from_ptr(super_journal_raw) }))
                .into_rc()
        }
        sqlite3::SQLITE_FCNTL_COMMIT_PHASETWO => file.commit_phase_two().into_rc(),
        sqlite3::SQLITE_FCNTL_PDB => {
            let pdb = unsafe { arg.cast::<*mut sqlite3::sqlite3>().read() };
            let connection = unsafe { Connection::from_handle(pdb) }
                .expect("internal error: invalid sqlite3 handle");
            file.set_parent_connection(connection);
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_BEGIN_ATOMIC_WRITE => file.begin_atomic().into_rc(),
        sqlite3::SQLITE_FCNTL_COMMIT_ATOMIC_WRITE => file.commit_atomic().into_rc(),
        sqlite3::SQLITE_FCNTL_ROLLBACK_ATOMIC_WRITE => {
            file.rollback_atomic();
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_LOCK_TIMEOUT => {
            let timeout = unsafe { arg.cast::<i32>().as_mut() }
                .expect("internal error: arg for SQLITE_FCNTL_LOCK_TIMEOUT must point to an i32");
            let new_timeout = Duration::from_millis(*timeout as u64);
            file.set_lock_timeout(new_timeout)
                .map(|old| old.as_millis() as i32)
                .write_to_output(timeout)
                .into_rc()
        }
        sqlite3::SQLITE_FCNTL_BUSYHANDLER => {
            let args = unsafe { slice::from_raw_parts(arg.cast::<*mut c_void>(), 2) };
            let handler: extern "C" fn(*mut c_void) -> c_int = unsafe { mem::transmute(args[0]) };
            let arg = args[1];
            file.set_busy_handler(BusyHandler { handler, arg });
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_NULL_IO => {
            storage.state = FileStorageState::Closed;
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_PERSIST_WAL => {
            let persist = unsafe { arg.cast::<i32>().as_mut() }
                .expect("internal error: arg for SQLITE_FCNTL_PERSIST_WAL must point to an i32");
            if *persist < 0 {
                *persist = file.is_wal_persistent() as i32;
                return sqlite3::SQLITE_OK;
            }
            file.set_wal_persistent(*persist != 0);
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_POWERSAFE_OVERWRITE => {
            let psow = unsafe { arg.cast::<c_int>().as_mut() }.expect(
                "internal error: arg for SQLITE_FCNTL_POWERSAFE_OVERWRITE must point to an c_int",
            );
            if *psow < 0 {
                *psow = file.is_powersafe_overwrite() as c_int;
                return sqlite3::SQLITE_OK;
            }
            file.set_powersafe_overwrite(*psow != 0).into_rc()
        }
        sqlite3::SQLITE_FCNTL_WAL_BLOCK => {
            file.hint_wal_lock();
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_BLOCK_ON_CONNECT => {
            let block = unsafe { arg.cast::<i32>().as_mut() }.expect(
                "internal error: arg for SQLITE_FCNTL_BLOCK_ON_CONNECT must point to an i32",
            );
            file.hint_block_on_connect(*block != 0);
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_CKPT_START => {
            file.on_checkpoint_start();
            sqlite3::SQLITE_OK
        }
        sqlite3::SQLITE_FCNTL_CKPT_DONE => {
            file.on_checkpoint_done();
            sqlite3::SQLITE_OK
        }

        // Not available as they are specific VFS detail
        #[cfg(unix)]
        sqlite3::SQLITE_FCNTL_GET_LOCKPROXYFILE => {
            let out = unsafe {
                arg.cast::<*const c_char>().as_mut().expect(
                    "internal error: arg for SQLITE_FCNTL_GET_LOCKPROXYFILE must point to a *mut c_char",
                )
            };
            file.lock_proxy_file_path()
                .map(|path| path.map_or(ptr::null(), |p| p.as_ptr()))
                .write_to_output(out)
                .into_rc()
        }
        #[cfg(not(unix))]
        sqlite3::SQLITE_FCNTL_GET_LOCKPROXYFILE => sqlite3::SQLITE_NOTFOUND,

        #[cfg(unix)]
        sqlite3::SQLITE_FCNTL_SET_LOCKPROXYFILE => {
            let path_ptr = arg.cast::<c_char>();
            let path = if path_ptr.is_null() {
                None
            } else {
                Some(unsafe { CStr::from_ptr(path_ptr) })
            };
            file.set_lock_proxy_file_path(path).into_rc()
        }
        #[cfg(not(unix))]
        sqlite3::SQLITE_FCNTL_SET_LOCKPROXYFILE => sqlite3::SQLITE_NOTFOUND,

        sqlite3::SQLITE_FCNTL_SIZE_LIMIT => {
            let limit = unsafe { arg.cast::<sqlite3_int64>().as_mut() }.expect(
                "internal error: arg for SQLITE_FCNTL_SIZE_LIMIT must point to an sqlite3_int64",
            );
            let new_limit = if *limit < 0 {
                None
            } else {
                Some(*limit as u64)
            };
            file.set_size_limit(new_limit)
                .map(|size| size as sqlite3_int64)
                .write_to_output(limit)
                .into_rc()
        }
        // TODO: I don't have a Windows system to test/implement this.
        sqlite3::SQLITE_FCNTL_WIN32_GET_HANDLE
        | sqlite3::SQLITE_FCNTL_WIN32_SET_HANDLE
        | sqlite3::SQLITE_FCNTL_WIN32_AV_RETRY => todo!(),

        // FIXME: this can't be implemented right now as it requires understanding
        // what zipvfs is doing internally. But that is proprietary software and there
        // is not public documentation about it. The only usage I could find is in
        // rbu vfs where the argument passed is a *mut c_void that is expected to be
        // filled with zipvfs-specific data (Or a pointer to the file? Or to the vfs?)
        // And is used as a check. The only way to implement this properly is to pass
        // something like `&mut ()` or a `*mut c_void` from the caller side, but I
        // think that is not useful for now.
        sqlite3::SQLITE_FCNTL_ZIPVFS => sqlite3::SQLITE_NOTFOUND,

        // FIXME: this requires us to provide RBU support, which we don't have right now.
        // Mapping here the RBU datastructures is non-trivial and not useful without
        // proper safe wrappers for it.
        sqlite3::SQLITE_FCNTL_RBU => sqlite3::SQLITE_NOTFOUND,

        // This has been removed in newer SQLite versions (0e77c3fa4d4c3445600869b6f32ecddc31d82c3d)
        // as it was actively harmful (it was preventing recovery in WAL mode).
        sqlite3::SQLITE_FCNTL_CKSM_FILE => sqlite3::SQLITE_NOTFOUND,

        // FIXME: This is experimental. I think it's best to wait until there is a
        // more concrete use case for it and the interface is stabilized.
        sqlite3::SQLITE_FCNTL_EXTERNAL_READER => sqlite3::SQLITE_NOTFOUND,

        // Should be implemented by SQLite core
        sqlite3::SQLITE_FCNTL_DATA_VERSION
        | sqlite3::SQLITE_FCNTL_RESERVE_BYTES
        | sqlite3::SQLITE_FCNTL_FILE_POINTER
        | sqlite3::SQLITE_FCNTL_JOURNAL_POINTER
        | sqlite3::SQLITE_FCNTL_VFS_POINTER
        | sqlite3::SQLITE_FCNTL_SYNC_OMITTED
        | sqlite3::SQLITE_FCNTL_RESET_CACHE => sqlite3::SQLITE_MISUSE,

        // Not supported.
        sqlite3::SQLITE_FCNTL_TRACE | sqlite3::SQLITE_FCNTL_TEMPFILENAME => sqlite3::SQLITE_OK,

        // Newer codes that we don't need to handle yet
        fcntl if fcntl <= 100 => sqlite3::SQLITE_NOTFOUND,

        // TODO: allow extensions to handle custom opcodes
        _ => sqlite3::SQLITE_NOTFOUND,
    }
}

unsafe extern "C" fn x_sector_size<T: Vfs>(file: *mut sqlite3_file) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.sector_len() as c_int
}

unsafe extern "C" fn x_device_characteristics<T: Vfs>(file: *mut sqlite3_file) -> c_int {
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.io_capabilities().to_raw()
}

unsafe extern "C" fn x_shm_map<T, F>(
    file: *mut sqlite3_file,
    region: c_int,
    size: c_int,
    extend: c_int,
    out_ptr: *mut *mut c_void,
) -> c_int
where
    F: VfsWalFile,
    T: Vfs<File = F>,
{
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let out = unsafe {
        out_ptr
            .cast::<*mut u8>()
            .as_mut()
            .expect("internal error: invalid output pointer for xShmMap")
    };
    if region < 0 {
        return sqlite3::SQLITE_MISUSE;
    }
    file.map_shm(region as u32, size as usize, extend != 0)
        .map(|s| s.as_mut_ptr())
        .write_to_output(out)
        .into_rc()
}

unsafe extern "C" fn x_shm_lock<T, F>(
    file: *mut sqlite3_file,
    offset: c_int,
    n: c_int,
    flags: c_int,
) -> c_int
where
    F: VfsWalFile,
    T: Vfs<File = F>,
{
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let lock_mode = match WalLockMode::try_from_raw(flags) {
        Ok(mode) => mode,
        Err(_) => return sqlite3::SQLITE_MISUSE,
    };
    let wal_lock = WalLock::new(offset as usize, n as usize);

    if (flags & sqlite3::SQLITE_SHM_LOCK) != 0 {
        file.lock_shm(wal_lock, lock_mode).into_rc()
    } else if (flags & sqlite3::SQLITE_SHM_UNLOCK) != 0 {
        file.unlock_shm(wal_lock, lock_mode).into_rc()
    } else {
        sqlite3::SQLITE_MISUSE
    }
}

unsafe extern "C" fn x_shm_barrier<T, F>(file: *mut sqlite3_file)
where
    F: VfsWalFile,
    T: Vfs<File = F>,
{
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.barrier();
}

unsafe extern "C" fn x_shm_unmap<T, F>(file: *mut sqlite3_file, delete: c_int) -> c_int
where
    F: VfsWalFile,
    T: Vfs<File = F>,
{
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    file.unmap_shm(delete != 0).into_rc()
}

unsafe extern "C" fn x_fetch<T, F>(
    file: *mut sqlite3_file,
    offset: i64,
    amount: i32,
    out_ptr: *mut *mut c_void,
) -> c_int
where
    F: VfsFetchFile,
    T: Vfs<File = F>,
{
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    let out = unsafe {
        out_ptr
            .cast::<*mut u8>()
            .as_mut()
            .expect("internal error: invalid output pointer for xFetch")
    };
    if amount <= 0 {
        panic!("internal error: amount passed to xFetch must be above 0");
    }
    file.fetch(offset, NonZero::new(amount as usize).unwrap())
        .map(|s| s.map_or(ptr::null_mut(), |slice| slice.as_mut_ptr()))
        .write_to_output(out)
        .into_rc()
}

unsafe extern "C" fn x_unfetch<T, F>(
    file: *mut sqlite3_file,
    offset: i64,
    ptr: *mut c_void,
) -> c_int
where
    F: VfsFetchFile,
    T: Vfs<File = F>,
{
    let storage = unsafe { VfsFileStorage::<T>::from_raw(file) };
    let file = storage.file();
    if let Some(ptr) = NonNull::new(ptr as *mut u8) {
        file.unfetch(offset, ptr).into_rc()
    } else {
        file.unfetch_all().into_rc()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::CStr,
        io::Write,
        num::NonZero,
        ptr::NonNull,
        time::{Duration, SystemTime},
    };

    use libsqlite3_sys as sqlite3;

    use crate::Connection;

    use super::*;

    struct DummyVfs;

    impl Vfs for DummyVfs {
        type File = DummyFile;

        fn open(&self, _path: FileType<'_>, _flags: VfsOpenFlags) -> Result<OpenFile<Self::File>> {
            Ok(OpenFile::new(DummyFile))
        }

        fn delete(&self, _path: VfsPath<'_>, _sync: bool) -> Result<()> {
            Ok(())
        }

        fn write_full_path(&self, path: VfsPath<'_>, mut out: &mut [u8]) -> Result<usize> {
            Ok(out.write(path.as_bytes()).unwrap())
        }

        fn fill_random_bytes(&self, _out: &mut [u8]) -> Result<()> {
            Ok(())
        }

        fn sleep(&self, _duration: Duration) {}

        fn now(&self) -> Result<SystemTime> {
            Ok(SystemTime::now())
        }

        fn last_error(&self) -> i32 {
            0
        }

        fn exists(&self, _name: VfsPath<'_>) -> Result<bool> {
            Ok(false)
        }

        fn can_read(&self, _name: VfsPath<'_>) -> Result<bool> {
            Ok(true)
        }

        fn can_write(&self, _name: VfsPath<'_>) -> Result<bool> {
            Ok(true)
        }
    }

    struct DummyFile;

    impl VfsFile for DummyFile {
        fn read_at(&mut self, buf: &mut [u8], _offset: u64) -> Result<usize> {
            buf.fill(0);
            Err(Error::new(sqlite3::SQLITE_IOERR_SHORT_READ))
        }

        fn write_at(&mut self, _buf: &[u8], _offset: u64) -> Result<()> {
            Ok(())
        }

        fn truncate(&mut self, _size: u64) -> Result<()> {
            Ok(())
        }

        fn sync(&mut self, _op: SyncOptions) -> Result<()> {
            Ok(())
        }

        fn len(&self) -> Result<u64> {
            Ok(0)
        }

        fn lock(&mut self, _level: LockLevel) -> Result<()> {
            Ok(())
        }

        fn unlock(&mut self, _level: LockLevel) -> Result<()> {
            Ok(())
        }

        fn is_write_locked(&self) -> Result<bool> {
            Ok(false)
        }

        fn lock_level(&self) -> LockLevel {
            LockLevel::None
        }

        fn last_errno(&self) -> i32 {
            0
        }

        fn sector_len(&self) -> u32 {
            4096
        }

        fn io_capabilities(&self) -> IoCapabilities {
            IoCapabilities::default()
        }
    }

    impl VfsWalFile for DummyFile {
        fn map_shm(
            &mut self,
            _region_index: u32,
            _region_size: usize,
            _extend: bool,
        ) -> Result<&mut [u8]> {
            unimplemented!()
        }

        fn lock_shm(&mut self, _locks: WalLock, _mode: WalLockMode) -> Result<()> {
            Ok(())
        }

        fn unlock_shm(&mut self, _locks: WalLock, _mode: WalLockMode) -> Result<()> {
            Err(Error::new(sqlite3::SQLITE_ERROR))
        }

        fn unmap_shm(&mut self, _delete: bool) -> Result<()> {
            Err(Error::new(sqlite3::SQLITE_ERROR))
        }
    }

    impl VfsFetchFile for DummyFile {
        fn fetch(&mut self, _offset: i64, _amount: NonZero<usize>) -> Result<Option<&mut [u8]>> {
            Err(Error::new(sqlite3::SQLITE_ERROR))
        }

        fn unfetch(&mut self, _offset: i64, _ptr: NonNull<u8>) -> Result<()> {
            Err(Error::new(sqlite3::SQLITE_ERROR))
        }

        fn unfetch_all(&mut self) -> Result<()> {
            Err(Error::new(sqlite3::SQLITE_ERROR))
        }
    }

    #[test]
    fn test_registration() {
        let token = VfsRegistration::new(DummyVfs)
            .make_default()
            .max_pathlen(16)
            .register("dummy")
            .unwrap();

        let default_vfs_ptr = unsafe { sqlite3::sqlite3_vfs_find(std::ptr::null()) };
        assert!(!default_vfs_ptr.is_null());
        let vfs_ptr = unsafe { sqlite3::sqlite3_vfs_find(c"dummy".as_ptr()) };

        assert!(!vfs_ptr.is_null());
        assert!(vfs_ptr == default_vfs_ptr);
        assert!(unsafe {
            CStr::from_ptr((*vfs_ptr).zName)
                .to_str()
                .unwrap()
                .eq("dummy")
        });
        assert!(unsafe { (*vfs_ptr).mxPathname } == 16);
        assert!(unsafe { (*vfs_ptr).iVersion } == 2);
        drop(token);

        let vfs_ptr = unsafe { sqlite3::sqlite3_vfs_find(c"dummy".as_ptr()) };
        assert!(vfs_ptr.is_null());
    }

    #[test]
    fn test_base_file_methods() {
        let token = VfsRegistration::new(DummyVfs).register("base").unwrap();

        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("test.db");
        let conn = Connection::open_with_flags_and_vfs(
            db_path.to_str().unwrap(),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
            "base",
        )
        .unwrap();

        let methods = unsafe {
            let db_handle = conn.handle();
            let mut file_ptr: *mut sqlite3::sqlite3_file = std::ptr::null_mut();
            let rc = sqlite3::sqlite3_file_control(
                db_handle,
                std::ptr::null(),
                sqlite3::SQLITE_FCNTL_FILE_POINTER,
                &mut file_ptr as *mut _ as *mut std::ffi::c_void,
            );
            assert_eq!(rc, sqlite3::SQLITE_OK);
            assert!(!file_ptr.is_null());
            *(*file_ptr).pMethods
        };
        assert_eq!(methods.iVersion, 1);
        assert!(methods.xClose.is_some());
        assert!(methods.xRead.is_some());
        assert!(methods.xWrite.is_some());
        assert!(methods.xTruncate.is_some());
        assert!(methods.xSync.is_some());
        assert!(methods.xFileSize.is_some());
        assert!(methods.xLock.is_some());
        assert!(methods.xUnlock.is_some());
        assert!(methods.xCheckReservedLock.is_some());
        assert!(methods.xFileControl.is_some());
        assert!(methods.xSectorSize.is_some());
        assert!(methods.xDeviceCharacteristics.is_some());
        assert!(methods.xShmMap.is_none());
        assert!(methods.xShmLock.is_none());
        assert!(methods.xShmBarrier.is_none());
        assert!(methods.xShmUnmap.is_none());
        assert!(methods.xFetch.is_none());
        assert!(methods.xUnfetch.is_none());
        drop(token);
    }

    #[test]
    fn test_fetch_file_methods() {
        let token = VfsRegistration::new(DummyVfs)
            .with_fetch()
            .register("fetch")
            .unwrap();

        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("test.db");
        let conn = Connection::open_with_flags_and_vfs(
            db_path.to_str().unwrap(),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
            "fetch",
        )
        .unwrap();

        let methods = unsafe {
            let db_handle = conn.handle();
            let mut file_ptr: *mut sqlite3::sqlite3_file = std::ptr::null_mut();
            let rc = sqlite3::sqlite3_file_control(
                db_handle,
                std::ptr::null(),
                sqlite3::SQLITE_FCNTL_FILE_POINTER,
                &mut file_ptr as *mut _ as *mut std::ffi::c_void,
            );
            assert_eq!(rc, sqlite3::SQLITE_OK);
            assert!(!file_ptr.is_null());
            *(*file_ptr).pMethods
        };
        assert_eq!(methods.iVersion, 3);
        assert!(methods.xClose.is_some());
        assert!(methods.xRead.is_some());
        assert!(methods.xWrite.is_some());
        assert!(methods.xTruncate.is_some());
        assert!(methods.xSync.is_some());
        assert!(methods.xFileSize.is_some());
        assert!(methods.xLock.is_some());
        assert!(methods.xUnlock.is_some());
        assert!(methods.xCheckReservedLock.is_some());
        assert!(methods.xFileControl.is_some());
        assert!(methods.xSectorSize.is_some());
        assert!(methods.xDeviceCharacteristics.is_some());
        assert!(methods.xShmMap.is_none());
        assert!(methods.xShmLock.is_none());
        assert!(methods.xShmBarrier.is_none());
        assert!(methods.xShmUnmap.is_none());
        assert!(methods.xFetch.is_some());
        assert!(methods.xUnfetch.is_some());
        drop(token);
    }

    #[test]
    fn test_wal_file_methods() {
        let token = VfsRegistration::new(DummyVfs)
            .with_wal()
            .register("wal")
            .unwrap();

        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("test.db");
        let conn = Connection::open_with_flags_and_vfs(
            db_path.to_str().unwrap(),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
            "wal",
        )
        .unwrap();

        let methods = unsafe {
            let db_handle = conn.handle();
            let mut file_ptr: *mut sqlite3::sqlite3_file = std::ptr::null_mut();
            let rc = sqlite3::sqlite3_file_control(
                db_handle,
                std::ptr::null(),
                sqlite3::SQLITE_FCNTL_FILE_POINTER,
                &mut file_ptr as *mut _ as *mut std::ffi::c_void,
            );
            assert_eq!(rc, sqlite3::SQLITE_OK);
            assert!(!file_ptr.is_null());
            *(*file_ptr).pMethods
        };
        assert_eq!(methods.iVersion, 2);
        assert!(methods.xClose.is_some());
        assert!(methods.xRead.is_some());
        assert!(methods.xWrite.is_some());
        assert!(methods.xTruncate.is_some());
        assert!(methods.xSync.is_some());
        assert!(methods.xFileSize.is_some());
        assert!(methods.xLock.is_some());
        assert!(methods.xUnlock.is_some());
        assert!(methods.xCheckReservedLock.is_some());
        assert!(methods.xFileControl.is_some());
        assert!(methods.xSectorSize.is_some());
        assert!(methods.xDeviceCharacteristics.is_some());
        assert!(methods.xShmMap.is_some());
        assert!(methods.xShmLock.is_some());
        assert!(methods.xShmBarrier.is_some());
        assert!(methods.xShmUnmap.is_some());
        assert!(methods.xFetch.is_none());
        assert!(methods.xUnfetch.is_none());
        drop(token);
    }

    #[test]
    fn test_complete_file_methods() {
        let token = VfsRegistration::new(DummyVfs)
            .with_wal()
            .with_fetch()
            .register("full")
            .unwrap();

        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("test.db");
        let conn = Connection::open_with_flags_and_vfs(
            db_path.to_str().unwrap(),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
            "full",
        )
        .unwrap();

        let methods = unsafe {
            let db_handle = conn.handle();
            let mut file_ptr: *mut sqlite3::sqlite3_file = std::ptr::null_mut();
            let rc = sqlite3::sqlite3_file_control(
                db_handle,
                std::ptr::null(),
                sqlite3::SQLITE_FCNTL_FILE_POINTER,
                &mut file_ptr as *mut _ as *mut std::ffi::c_void,
            );
            assert_eq!(rc, sqlite3::SQLITE_OK);
            assert!(!file_ptr.is_null());
            *(*file_ptr).pMethods
        };
        assert_eq!(methods.iVersion, 3);
        assert!(methods.xClose.is_some());
        assert!(methods.xRead.is_some());
        assert!(methods.xWrite.is_some());
        assert!(methods.xTruncate.is_some());
        assert!(methods.xSync.is_some());
        assert!(methods.xFileSize.is_some());
        assert!(methods.xLock.is_some());
        assert!(methods.xUnlock.is_some());
        assert!(methods.xCheckReservedLock.is_some());
        assert!(methods.xFileControl.is_some());
        assert!(methods.xSectorSize.is_some());
        assert!(methods.xDeviceCharacteristics.is_some());
        assert!(methods.xShmMap.is_some());
        assert!(methods.xShmLock.is_some());
        assert!(methods.xShmBarrier.is_some());
        assert!(methods.xShmUnmap.is_some());
        assert!(methods.xFetch.is_some());
        assert!(methods.xUnfetch.is_some());
        drop(token);
    }
}
