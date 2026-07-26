//! Code related to `sqlite3_context` common to `functions` and `vtab` modules.

use std::ffi::c_void;

use crate::ffi::{self, sqlite3_context, sqlite3_value};
use crate::{Result, str_for_sqlite};

use crate::types::{ToSqlOutput, Value, ValueRef};

// This function is inline despite it's size because what's in the ToSqlOutput
// is often known to the compiler, and thus const prop/DCE can substantially
// simplify the function.
#[inline]
pub(super) unsafe fn set_result(
    ctx: *mut sqlite3_context,
    #[allow(unused_variables)] args: &[*mut sqlite3_value],
    result: ToSqlOutput<'_>,
) -> Result<()> {
    match result {
        ToSqlOutput::Borrowed(ValueRef::Null) | ToSqlOutput::Owned(Value::Null) => {
            unsafe { ffi::sqlite3_result_null(ctx) };
        }
        ToSqlOutput::Borrowed(ValueRef::Integer(i)) | ToSqlOutput::Owned(Value::Integer(i)) => {
            unsafe { ffi::sqlite3_result_int64(ctx, i) };
        }
        ToSqlOutput::Borrowed(ValueRef::Real(r)) | ToSqlOutput::Owned(Value::Real(r)) => {
            unsafe { ffi::sqlite3_result_double(ctx, r) };
        }
        ToSqlOutput::Borrowed(ValueRef::Text(s)) => result_text(ctx, s),
        ToSqlOutput::Owned(Value::Text(s)) => result_text(ctx, s.as_bytes()),
        ToSqlOutput::Borrowed(ValueRef::Blob(b)) => result_blob(ctx, b),
        ToSqlOutput::Owned(Value::Blob(b)) => result_blob(ctx, b.as_slice()),
        #[cfg(feature = "blob")]
        ToSqlOutput::ZeroBlob(len) => {
            let code = unsafe { ffi::sqlite3_result_zeroblob64(ctx, len) };
            if code != ffi::SQLITE_OK {
                return Err(unsafe {
                    crate::error::error_from_handle(ffi::sqlite3_context_db_handle(ctx), code)
                });
            }
        }
        #[cfg(feature = "functions")]
        ToSqlOutput::Arg(i) => {
            unsafe { ffi::sqlite3_result_value(ctx, args[i]) };
        }
        #[cfg(feature = "pointer")]
        ToSqlOutput::Pointer(ref p) => {
            unsafe { ffi::sqlite3_result_pointer(ctx, p.0 as _, p.1.as_ptr(), p.2) };
        }
    }
    Ok(())
}

fn result_blob(ctx: *mut sqlite3_context, b: &[u8]) {
    let length = b.len();
    if length == 0 {
        unsafe { ffi::sqlite3_result_zeroblob64(ctx, 0) };
    } else {
        unsafe {
            ffi::sqlite3_result_blob64(
                ctx,
                b.as_ptr().cast::<c_void>(),
                length as ffi::sqlite3_uint64,
                ffi::SQLITE_TRANSIENT(),
            );
        };
    }
}

fn result_text(ctx: *mut sqlite3_context, s: &[u8]) {
    let (c_str, len, destructor) = str_for_sqlite(s);
    unsafe { ffi::sqlite3_result_text64(ctx, c_str, len, destructor, ffi::SQLITE_UTF8 as _) };
    // TODO SQLITE_UTF8_ZT
}
