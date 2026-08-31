//! A read-only in-memory VFS implementation for SQLite databases.

use std::{
    collections::HashMap,
    ffi::OsString,
    io::Write,
    sync::{Arc, RwLock},
};

use crate::ffi::Error;
use crate::vfs::{
    FileType, IoCapabilities, LockLevel, OpenFile, Result, SyncOptions, Vfs, VfsFile, VfsOpenFlags,
    VfsPath,
};

#[cfg(feature = "serialize")]
use crate::Connection;

/// An in-memory VFS implementation. This VFS allows you to read files entirely in memory.
/// It is useful for testing or for applications that require fast access to temporary data.
#[derive(Default)]
pub struct MemVfs {
    files: RwLock<HashMap<OsString, Arc<[u8]>>>,
}

impl MemVfs {
    /// Create a new in-memory VFS instance.
    pub fn new() -> Self {
        Self {
            files: RwLock::new(HashMap::new()),
        }
    }

    /// Add a file with the given name and data to the VFS. If a file with the same name already exists, it will be overwritten.
    pub fn add_file(&self, name: impl Into<OsString>, data: impl Into<Vec<u8>>) {
        let mut files = self.files.write().unwrap();
        files.insert(name.into(), Arc::from(data.into()));
    }

    /// Create a new file in the VFS by serializing an in-memory database.
    #[cfg(feature = "serialize")]
    pub fn create_file(
        &self,
        name: impl Into<OsString>,
        f: impl FnOnce(&mut Connection) -> crate::Result<()>,
    ) -> crate::Result<()> {
        let mut conn = Connection::open_in_memory()?;
        f(&mut conn)?;
        let data = conn.serialize(crate::MAIN_DB)?;
        self.add_file(name, data.to_vec());
        Ok(())
    }

    /// Remove a file with the given name from the VFS. If no such file exists, this method does nothing.
    pub fn remove_file(&self, name: impl Into<OsString>) {
        let mut files = self.files.write().unwrap();
        let name = name.into();
        files.remove(&name);
    }
}

impl Vfs for MemVfs {
    type File = MemFile;

    fn open(&self, file: FileType<'_>, _flags: VfsOpenFlags) -> Result<OpenFile<Self::File>> {
        let name = match file {
            FileType::MainDb(name) => name,
            _ => return Err(Error::new(libsqlite3_sys::SQLITE_CANTOPEN)),
        };

        let files = self.files.read().unwrap();
        if let Some(data) = files.get(name.as_os_str()) {
            Ok(OpenFile::new(MemFile { data: data.clone() }).readonly())
        } else {
            Err(Error::new(libsqlite3_sys::SQLITE_CANTOPEN))
        }
    }

    fn delete(&self, _name: VfsPath<'_>, _sync_dir: bool) -> Result<()> {
        Ok(())
    }

    fn exists(&self, name: VfsPath<'_>) -> Result<bool> {
        let files = self.files.read().unwrap();
        Ok(files.contains_key(name.as_os_str()))
    }

    fn can_read(&self, name: VfsPath<'_>) -> Result<bool> {
        let files = self.files.read().unwrap();
        Ok(files.contains_key(name.as_os_str()))
    }

    fn can_write(&self, _name: VfsPath<'_>) -> Result<bool> {
        Ok(false)
    }

    fn write_full_path(&self, name: VfsPath<'_>, mut out: &mut [u8]) -> Result<usize> {
        out.write(name.as_bytes())
            .map_err(|_| Error::new(libsqlite3_sys::SQLITE_CANTOPEN))
    }

    fn last_error(&self) -> i32 {
        0
    }
}

/// A read-only file in the in-memory VFS.
pub struct MemFile {
    data: Arc<[u8]>,
}

impl VfsFile for MemFile {
    fn read_at(&mut self, buf: &mut [u8], offset: u64) -> Result<usize> {
        if offset >= self.data.len() as u64 {
            return Ok(0);
        }
        let end = std::cmp::min(offset as usize + buf.len(), self.data.len());
        let bytes_read = end - offset as usize;
        buf[..bytes_read].copy_from_slice(&self.data[offset as usize..end]);
        Ok(bytes_read)
    }

    fn write_at(&mut self, _buf: &[u8], _offset: u64) -> Result<()> {
        Err(Error::new(libsqlite3_sys::SQLITE_IOERR_WRITE))
    }

    fn truncate(&mut self, _size: u64) -> Result<()> {
        Err(Error::new(libsqlite3_sys::SQLITE_IOERR_WRITE))
    }

    fn sync(&mut self, _op: SyncOptions) -> Result<()> {
        Ok(())
    }

    fn len(&self) -> Result<u64> {
        Ok(self.data.len() as u64)
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

    fn sector_len(&self) -> u32 {
        0
    }

    fn io_capabilities(&self) -> IoCapabilities {
        IoCapabilities {
            immutable: true,
            subpage_read: true,
            ..Default::default()
        }
    }

    fn lock_level(&self) -> LockLevel {
        LockLevel::None
    }

    fn last_errno(&self) -> i32 {
        0
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused)]
    use super::*;

    #[test]
    #[cfg(feature = "serialize")]
    fn test_memvfs() -> crate::Result<()> {
        use crate::{params, vfs, OpenFlags};

        let vfs = vfs::VfsRegistration::new(MemVfs::new()).register("memvfs")?;
        vfs.create_file("test.db", |conn| {
            conn.execute_batch(
                "
                    CREATE TABLE test (
                        id INTEGER PRIMARY KEY,
                        value TEXT
                    );
                    INSERT INTO test (value) VALUES ('hello'), ('world');
                ",
            )?;
            Ok(())
        })?;

        let conn1 = Connection::open_with_flags_and_vfs("test.db", OpenFlags::default(), "memvfs")?;
        let values: Vec<String> = conn1
            .prepare(
                "
                    SELECT value FROM test ORDER BY id
                ",
            )?
            .query_map(params![], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        assert_eq!(
            values,
            ["hello", "world"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );

        // Make sure that the file can be read from multiple connections
        // without issues.
        {
            let conn2 =
                Connection::open_with_flags_and_vfs("test.db", OpenFlags::default(), "memvfs")?;
            let count: i64 = conn2.query_one(
                "
                    SELECT COUNT(*)
                    FROM test
                ",
                params![],
                |row| row.get(0),
            )?;
            assert_eq!(count, 2);
        }

        // Make sure that you can remove the file from the VFS and still read it.
        vfs.remove_file("test.db");

        // Can't open now.
        let result = Connection::open_with_flags_and_vfs("test.db", OpenFlags::default(), "memvfs");
        assert!(result.is_err());

        // But existing connections still work.
        let count: i64 = conn1.query_one(
            "
                SELECT COUNT(*)
                FROM test
            ",
            params![],
            |row| row.get(0),
        )?;
        assert_eq!(count, 2);

        // Can add the file again with different content.
        vfs.create_file("test.db", |conn| {
            conn.execute_batch(
                "
                    CREATE TABLE test2 (
                        id INTEGER PRIMARY KEY,
                        value TEXT
                    );
                    INSERT INTO test2 (value) VALUES ('foo'), ('bar'), ('baz');
                ",
            )?;
            Ok(())
        })?;

        let conn3 = Connection::open_with_flags_and_vfs("test.db", OpenFlags::default(), "memvfs")?;
        let values: Vec<String> = conn3
            .prepare(
                "
                    SELECT value FROM test2 ORDER BY id
                ",
            )?
            .query_map(params![], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        assert_eq!(
            values,
            ["foo", "bar", "baz"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );

        // Make sure existing connections still work when the VFS is unregistered.
        drop(vfs);

        let count: i64 = conn1.query_one(
            "
                SELECT COUNT(*)
                FROM test
            ",
            params![],
            |row| row.get(0),
        )?;
        assert_eq!(count, 2);

        let count: i64 = conn3.query_one(
            "
                SELECT COUNT(*)
                FROM test2
            ",
            params![],
            |row| row.get(0),
        )?;
        assert_eq!(count, 3);

        Ok(())
    }
}
