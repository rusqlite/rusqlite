//! [`ToSql`] and [`FromSql`] implementation for [`std::path::PathBuf`].

use std::fmt;
use std::path::PathBuf;

use crate::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use crate::{Error, Result};

#[derive(Debug)]
pub struct PathError {
    path: PathBuf,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path.display())
    }
}

impl std::error::Error for PathError {}

/// Serialize `PathBuf` to UTF-8 text.
impl ToSql for PathBuf {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        match self.to_str() {
            Some(s) => Ok(ToSqlOutput::from(s)),
            None => {
                let err = PathError { path: self.clone() };
                Err(Error::ToSqlConversionFailure(Box::new(err)))
            }
        }
    }
}

/// Deserialize UTF-8 text to `PathBuf`.
impl FromSql for PathBuf {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Text(s) => {
                let s = std::str::from_utf8(s).map_err(|e| FromSqlError::Other(Box::new(e)))?;
                Ok(PathBuf::from(s))
            }
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use crate::{params, Connection, Result, ToSql};

    // `from_bytes` extension doesn't exist on Windows
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    #[test]
    fn non_utf8_to_sql() -> Result<()> {
        use std::{ffi::OsStr, os::unix::prelude::OsStrExt};

        let inval = OsStr::from_bytes(b"foo\xFF\xFFbar");
        let oss = OsStr::new(inval);
        let path = PathBuf::from(oss);
        let sql = path.to_sql();
        assert!(sql.is_err());
        Ok(())
    }

    #[test]
    fn pathbuf_to_sql() -> Result<()> {
        let path = "/usr/bin/bash";
        let buf = PathBuf::from(path);
        let sql = buf.to_sql()?;
        assert_eq!(path.to_sql()?, sql);
        Ok(())
    }

    #[test]
    fn pathbuf_roundtrip() -> Result<()> {
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE paths (id INTEGER, path TEXT)")?;

        db.execute(
            "INSERT INTO paths (id, path) VALUES (0, ?1)",
            params![PathBuf::from("/usr/bin/bash")],
        )?;
        let buf: PathBuf = db.query_row("SELECT path FROM paths WHERE id = 0", [], |r| r.get(0))?;
        assert_eq!(PathBuf::from("/usr/bin/bash"), buf);

        Ok(())
    }
}
