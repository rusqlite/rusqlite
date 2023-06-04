//! Serialize a database.
use std::convert::TryInto;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr::NonNull;

use crate::error::error_from_handle;
use crate::ffi;
use crate::{Connection, DatabaseName, Result};

/// Shared serialized database
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

/// Serialized database
pub enum Data<'conn> {
    /// Shared serialized database
    Shared(SharedData<'conn>),
    /// Owned serialized database
    Owned(OwnedData),
}

impl<'conn> Deref for Data<'conn> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        let (ptr, sz) = match self {
            Data::Owned(OwnedData { ptr, sz }) => (ptr.as_ptr(), *sz),
            Data::Shared(SharedData { ptr, sz, .. }) => (ptr.as_ptr(), *sz),
        };
        unsafe { std::slice::from_raw_parts(ptr, sz) }
    }
}

impl Drop for OwnedData {
    fn drop(&mut self) {
        unsafe {
            ffi::sqlite3_free(self.ptr.as_ptr().cast());
        }
    }
}

impl Connection {
    /// Serialize a database.
    pub fn serialize<'conn>(&'conn self, schema: DatabaseName<'_>) -> Result<Data<'conn>> {
        let schema = schema.as_cstring()?;
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

    /// Deserialize a database.
    pub fn deserialize(
        &mut self,
        schema: DatabaseName<'_>,
        data: Data<'_>,
        read_only: bool,
    ) -> Result<()> {
        let schema = schema.as_cstring()?;
        let (data, sz, flags) = match data {
            Data::Owned(OwnedData { ptr, sz }) => (
                ptr.as_ptr(), // FIXME double-free => mem forget
                sz.try_into().unwrap(),
                if read_only {
                    ffi::SQLITE_DESERIALIZE_FREEONCLOSE | ffi::SQLITE_DESERIALIZE_READONLY
                } else {
                    ffi::SQLITE_DESERIALIZE_FREEONCLOSE | ffi::SQLITE_DESERIALIZE_RESIZEABLE
                },
            ),
            Data::Shared(SharedData { ptr, sz, .. }) => (
                ptr.as_ptr(), // FIXME lifetime of ptr must be > lifetime self
                sz.try_into().unwrap(),
                if read_only {
                    ffi::SQLITE_DESERIALIZE_READONLY
                } else {
                    0
                },
            ),
        };
        let rc = unsafe {
            ffi::sqlite3_deserialize(self.handle(), schema.as_ptr(), data, sz, sz, flags)
        };
        if rc != ffi::SQLITE_OK {
            return Err(unsafe { error_from_handle(self.handle(), rc) });
        }
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
        Ok(())
    }
}
