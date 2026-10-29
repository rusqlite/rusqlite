//! Encryption
use std::ptr;

use crate::error::decode_result_raw;
use crate::ffi;
use crate::{Connection, Name, Result};

impl Connection {
    /// Specify the key for an encrypted database.
    /// Or Change the key on an open database with `rekey`.
    pub fn key<N: Name, S: AsRef<[u8]>>(
        &self,
        db_name: Option<N>,
        key: S,
        rekey: bool,
    ) -> Result<()> {
        let slice = key.as_ref();
        let n = to_c_int(slice.len())?;
        let db = self.db.borrow_mut().db;
        let cs = db_name.as_ref().map(N::as_cstr).transpose()?;
        let db_name = cs.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        unsafe {
            decode_result_raw(
                db,
                if rekey {
                    ffi::sqlite3_rekey_v2(db, db_name, slice.as_ptr().cast(), n)
                } else {
                    ffi::sqlite3_key_v2(db, db_name, slice.as_ptr().cast(), n)
                },
            )
        }
    }
}

fn to_c_int(n: usize) -> Result<std::ffi::c_int> {
    let Ok(n) = std::ffi::c_int::try_from(n) else {
        return Err(err!(ffi::SQLITE_MISUSE, "Invalid key length {n}"));
    };
    Ok(n)
}
