use crate::{ffi, Error, Result, Statement};
use std::ffi::CStr;

mod sealed {
    use std::ffi::CStr;
    /// This trait exists just to ensure that the only impls of `trait BindIndex`
    /// that are allowed are ones in this crate.
    pub trait Sealed {}
    impl Sealed for usize {}
    impl Sealed for &str {}
    impl Sealed for &CStr {}
}

/// A trait implemented by types that can index into parameters of a statement.
///
/// It is only implemented for `usize` and `&str` and `&CStr`.
pub trait BindIndex: sealed::Sealed {
    /// Returns the index of the associated parameter, or `Error` if no such
    /// parameter exists.
    fn idx(&self, stmt: &Statement<'_>) -> Result<usize>;
}

impl BindIndex for usize {
    #[inline]
    fn idx(&self, _: &Statement<'_>) -> Result<usize> {
        // No validation
        Ok(*self)
    }
}

impl BindIndex for &'_ str {
    fn idx(&self, stmt: &Statement<'_>) -> Result<usize> {
        match stmt.parameter_index(self)? {
            Some(idx) => Ok(idx),
            None => Err(Error::InvalidParameterName(self.to_string())),
        }
    }
}
/// C-string literal to avoid alloc
impl BindIndex for &CStr {
    fn idx(&self, stmt: &Statement<'_>) -> Result<usize> {
        let r = unsafe { ffi::sqlite3_bind_parameter_index(stmt.ptr(), self.as_ptr()) };
        match r {
            0 => Err(Error::InvalidParameterName(
                self.to_string_lossy().to_string(),
            )),
            i => Ok(i as usize),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::{ffi, Connection, Error, Result};

    #[test]
    fn invalid_name() -> Result<()> {
        let db = Connection::open_in_memory()?;
        let mut stmt = db.prepare("SELECT 1")?;
        let err = stmt.raw_bind_parameter(1, 1).unwrap_err();
        assert_eq!(
            err.sqlite_error_code(),
            Some(ffi::ErrorCode::ParameterOutOfRange),
        );
        let err = stmt.raw_bind_parameter(":p1", 1).unwrap_err();
        assert_eq!(err, Error::InvalidParameterName(":p1".to_owned()));
        let err = stmt.raw_bind_parameter(c"x", 1).unwrap_err();
        assert_eq!(err, Error::InvalidParameterName("x".to_owned()));
        Ok(())
    }
}
