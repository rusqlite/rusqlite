//! Code related to `sqlite3_context` common to `functions` and `vtab` modules.

use std::ffi::c_void;

use crate::ffi::{self, sqlite3_context, sqlite3_value};
use crate::{Result, str_for_sqlite};

use crate::types::{ToSqlOutput, ValueRef};

// This function is inline despite it's size because what's in the ToSqlOutput
// is often known to the compiler, and thus const prop/DCE can substantially
// simplify the function.
#[inline]
pub(super) unsafe fn set_result(
    ctx: *mut sqlite3_context,
    #[allow(unused_variables)] args: &[*mut sqlite3_value],
    result: ToSqlOutput<'_>,
) -> Result<()> {
    unsafe {
        let value = match result {
            ToSqlOutput::Borrowed(v) => v,
            ToSqlOutput::Owned(ref v) => ValueRef::from(v),

            #[cfg(feature = "blob")]
            ToSqlOutput::ZeroBlob(len) => {
                let code = ffi::sqlite3_result_zeroblob64(ctx, len);
                return if code != ffi::SQLITE_OK {
                    Err(crate::error::error_from_handle(
                        ffi::sqlite3_context_db_handle(ctx),
                        code,
                    ))
                } else {
                    Ok(())
                };
            }
            #[cfg(feature = "functions")]
            ToSqlOutput::Arg(i) => {
                ffi::sqlite3_result_value(ctx, args[i]);
                return Ok(());
            }
            #[cfg(feature = "pointer")]
            ToSqlOutput::Pointer(ref p) => {
                ffi::sqlite3_result_pointer(ctx, p.0 as _, p.1.as_ptr(), p.2);
                return Ok(());
            }
        };

        match value {
            ValueRef::Null => ffi::sqlite3_result_null(ctx),
            ValueRef::Integer(i) => ffi::sqlite3_result_int64(ctx, i),
            ValueRef::Real(r) => ffi::sqlite3_result_double(ctx, r),
            ValueRef::Text(s) => {
                let (c_str, len, destructor) = str_for_sqlite(s);
                ffi::sqlite3_result_text64(ctx, c_str, len, destructor, ffi::SQLITE_UTF8 as _);
                // TODO SQLITE_UTF8_ZT
            }
            ValueRef::Blob(b) => {
                let length = b.len();
                if length == 0 {
                    ffi::sqlite3_result_zeroblob64(ctx, 0);
                } else {
                    ffi::sqlite3_result_blob64(
                        ctx,
                        b.as_ptr().cast::<c_void>(),
                        length as ffi::sqlite3_uint64,
                        ffi::SQLITE_TRANSIENT(),
                    );
                }
            }
        }
        Ok(())
    }
}
