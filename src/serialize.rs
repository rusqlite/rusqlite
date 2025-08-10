//! Serialize a database.
use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr::NonNull;

use crate::error::{error_from_handle, error_from_sqlite_code};
use crate::ffi;
use crate::{Connection, Error, Name, Result};

/// Shared (SQLITE_SERIALIZE_NOCOPY) serialized database
pub struct SharedData<'conn> {
    phantom: PhantomData<&'conn Connection>,
    ptr: NonNull<u8>,
    sz: usize,
}

/// Owned serialized database
pub struct OwnedData {
    ptr: NonNull<u8>,
    sz: usize,
}

impl OwnedData {
    /// # Safety
    ///
    /// Caller must be certain that `ptr` is allocated by `sqlite3_malloc`.
    pub unsafe fn from_raw_nonnull(ptr: NonNull<u8>, sz: usize) -> Self {
        Self { ptr, sz }
    }

    fn into_raw(self) -> (*mut u8, usize) {
        let raw = (self.ptr.as_ptr(), self.sz);
        std::mem::forget(self);
        raw
    }
}

impl Drop for OwnedData {
    fn drop(&mut self) {
        unsafe {
            ffi::sqlite3_free(self.ptr.as_ptr().cast());
        }
    }
}

/// Serialized database
pub enum Data<'conn> {
    /// Shared (SQLITE_SERIALIZE_NOCOPY) serialized database
    Shared(SharedData<'conn>),
    /// Owned serialized database
    Owned(OwnedData),
}

impl Deref for Data<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        let (ptr, sz) = match self {
            Data::Owned(OwnedData { ptr, sz }) => (ptr.as_ptr(), *sz),
            Data::Shared(SharedData { ptr, sz, .. }) => (ptr.as_ptr(), *sz),
        };
        unsafe { std::slice::from_raw_parts(ptr, sz) }
    }
}

impl Connection {
    /// Serialize a database.
    pub fn serialize<N: Name>(&self, schema: N) -> Result<Data<'_>> {
        let schema = schema.as_cstr()?;
        let mut sz = 0;
        let mut ptr: *mut u8 = unsafe {
            ffi::sqlite3_serialize(
                self.handle(),
                schema.as_ptr(),
                &mut sz,
                ffi::SQLITE_SERIALIZE_NOCOPY,
            )
        };
        Ok(if ptr.is_null() {
            ptr = unsafe { ffi::sqlite3_serialize(self.handle(), schema.as_ptr(), &mut sz, 0) };
            if ptr.is_null() {
                return Err(unsafe { error_from_handle(self.handle(), ffi::SQLITE_NOMEM) });
            }
            Data::Owned(OwnedData {
                ptr: NonNull::new(ptr).unwrap(),
                sz: sz.try_into().unwrap(),
            })
        } else {
            // shared buffer
            Data::Shared(SharedData {
                ptr: NonNull::new(ptr).unwrap(),
                sz: sz.try_into().unwrap(),
                phantom: PhantomData,
            })
        })
    }

    /// Deserialize from stream
    pub fn deserialize_read_exact<N: Name, R: std::io::Read>(
        &mut self,
        schema: N,
        mut read: R,
        sz: usize,
        read_only: bool,
    ) -> Result<()> {
        let ptr = unsafe { ffi::sqlite3_malloc(sz.try_into().unwrap()) }.cast::<u8>();
        if ptr.is_null() {
            return Err(error_from_sqlite_code(ffi::SQLITE_NOMEM, None));
        }
        let buf = unsafe { std::slice::from_raw_parts_mut(ptr, sz) };
        read.read_exact(buf).map_err(|e| {
            Error::SqliteFailure(
                ffi::Error {
                    code: ffi::ErrorCode::CannotOpen,
                    extended_code: ffi::SQLITE_IOERR,
                },
                Some(format!("{e}")),
            )
        })?;
        let ptr = NonNull::new(ptr).unwrap();
        let data = unsafe { OwnedData::from_raw_nonnull(ptr, sz) };
        self.deserialize(schema, data, read_only)
    }

    /// Deserialize `include_bytes` as a read only database
    pub fn deserialize_bytes<N: Name>(&mut self, schema: N, data: &'static [u8]) -> Result<()> {
        let sz = data.len().try_into().unwrap();
        self.deserialize_(
            schema,
            data.as_ptr() as *mut _,
            sz,
            ffi::SQLITE_DESERIALIZE_READONLY,
        )
    }

    /// Deserialize a database.
    pub fn deserialize<N: Name>(
        &mut self,
        schema: N,
        data: OwnedData,
        read_only: bool,
    ) -> Result<()> {
        let (data, sz) = data.into_raw();
        let sz = sz.try_into().unwrap();
        let flags = if read_only {
            ffi::SQLITE_DESERIALIZE_FREEONCLOSE | ffi::SQLITE_DESERIALIZE_READONLY
        } else {
            ffi::SQLITE_DESERIALIZE_FREEONCLOSE | ffi::SQLITE_DESERIALIZE_RESIZEABLE
        };
        self.deserialize_(schema, data, sz, flags)
        /* TODO
        if let Some(mxSize) = mxSize {
            unsafe {
                ffi::sqlite3_file_control(
                    self.handle(),
                    schema.as_ptr(),
                    ffi::SQLITE_FCNTL_SIZE_LIMIT,
                    &mut mxSize,
                )
            };
        }*/
    }

    fn deserialize_<N: Name>(
        &mut self,
        schema: N,
        data: *mut u8,
        sz: ffi::sqlite_int64,
        flags: std::ffi::c_uint,
    ) -> Result<()> {
        let schema = schema.as_cstr()?;
        let rc = unsafe {
            ffi::sqlite3_deserialize(self.handle(), schema.as_ptr(), data, sz, sz, flags)
        };
        if rc != ffi::SQLITE_OK {
            return Err(unsafe { error_from_handle(self.handle(), rc) });
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::MAIN_DB;

    #[test]
    fn serialize() -> Result<()> {
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE x AS SELECT 'data'")?;
        let data = db.serialize(MAIN_DB)?;
        let Data::Owned(data) = data else {
            panic!("expected OwnedData")
        };
        assert!(data.sz > 0);
        Ok(())
    }

    #[test]
    fn deserialize_read_exact() -> Result<()> {
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE x AS SELECT 'data'")?;
        let data = db.serialize(MAIN_DB)?;

        let mut dst = Connection::open_in_memory()?;
        let read = data.deref();
        dst.deserialize_read_exact(MAIN_DB, read, read.len(), false)?;
        dst.execute("DELETE FROM x", [])?;
        Ok(())
    }

    #[test]
    fn deserialize_bytes() -> Result<()> {
        let data = b"";
        let mut dst = Connection::open_in_memory()?;
        dst.deserialize_bytes(MAIN_DB, data)?;
        Ok(())
    }

    #[test]
    fn deserialize() -> Result<()> {
        let src = Connection::open_in_memory()?;
        src.execute_batch("CREATE TABLE x AS SELECT 'data'")?;
        let data = src.serialize(MAIN_DB)?;
        let Data::Owned(data) = data else {
            panic!("expected OwnedData")
        };

        let mut dst = Connection::open_in_memory()?;
        dst.deserialize(MAIN_DB, data, false)?;
        dst.execute("DELETE FROM x", [])?;
        Ok(())
    }
}
