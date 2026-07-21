//! Code related to `sqlite3_context` common to `functions` and `vtab` modules.

use crate::Result;
use crate::ffi::{self, SQLITE_STATIC, SQLITE_UTF8, sqlite3_context, sqlite3_value};

use crate::types::RawValue;

// This function is inline despite it's size because what's in the ToSqlOutput
// is often known to the compiler, and thus const prop/DCE can substantially
// simplify the function.
#[inline]
pub(super) unsafe fn set_result(
    ctx: *mut sqlite3_context,
    #[allow(unused_variables)] args: &[*mut sqlite3_value],
    result: RawValue,
) -> Result<()> {
    match result {
        RawValue::Null => {
            unsafe { ffi::sqlite3_result_null(ctx) };
        }
        RawValue::Integer(i) => {
            unsafe { ffi::sqlite3_result_int64(ctx, i) };
        }
        RawValue::Real(r) => {
            unsafe { ffi::sqlite3_result_double(ctx, r) };
        }
        RawValue::Text {
            ptr,
            bytes,
            destructor,
            flags,
        } => unsafe { ffi::sqlite3_result_text64(ctx, ptr, bytes.get() as _, destructor, flags) },
        RawValue::EmptyText => unsafe {
            ffi::sqlite3_result_text64(ctx, "".as_ptr() as _, 0, SQLITE_STATIC(), SQLITE_UTF8 as _);
        },
        RawValue::Blob {
            ptr,
            bytes,
            destructor,
        } => unsafe { ffi::sqlite3_result_blob64(ctx, ptr, bytes.get() as _, destructor) },
        RawValue::ZeroBlob(len) => {
            let code = unsafe { ffi::sqlite3_result_zeroblob64(ctx, len) };
            if code != ffi::SQLITE_OK {
                return Err(unsafe {
                    crate::error::error_from_handle(ffi::sqlite3_context_db_handle(ctx), code)
                });
            }
        }
        #[cfg(feature = "functions")]
        RawValue::Arg(i) => {
            unsafe { ffi::sqlite3_result_value(ctx, args[i]) };
        }
        #[cfg(feature = "pointer")]
        RawValue::Pointer {
            ptr,
            ptr_type,
            destructor,
        } => {
            unsafe { ffi::sqlite3_result_pointer(ctx, ptr as _, ptr_type.as_ptr(), destructor) };
        }
    }
    Ok(())
}
