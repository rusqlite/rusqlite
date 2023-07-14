//! [`ToSql`] and [`FromSql`] implementation for [`camino::Utf8PathBuf`].

use camino::Utf8PathBuf;

use crate::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use crate::Result;

/// Serialize `Utf8PathBuf` to text.
impl ToSql for Utf8PathBuf {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

/// Deserialize text to `Utf8PathBuf`.
impl FromSql for Utf8PathBuf {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Text(text) => {
                let s = std::str::from_utf8(text).map_err(|e| FromSqlError::Other(Box::new(e)))?;
                Ok(Utf8PathBuf::from(s))
            }
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

#[cfg(test)]
mod test {
    use camino::Utf8PathBuf;

    use crate::{params, types::FromSql, Connection, Result, ToSql, ValueRef};

    #[test]
    fn utf8pathbuf_to_sql() -> Result<()> {
        let path = "/usr/bin/bash";
        let buf = Utf8PathBuf::from(path);
        let sql = buf.to_sql()?;
        assert_eq!(path.to_sql()?, sql);
        Ok(())
    }

    #[test]
    fn utf8pathbuf_roundtrip() -> Result<()> {
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE paths (id INTEGER, path TEXT)")?;

        db.execute(
            "INSERT INTO paths (id, path) VALUES (0, ?1)",
            params![Utf8PathBuf::from("/usr/bin/bash")],
        )?;
        let buf: Utf8PathBuf =
            db.query_row("SELECT path FROM paths WHERE id = 0", [], |r| r.get(0))?;
        assert_eq!(Utf8PathBuf::from("/usr/bin/bash"), buf);

        Ok(())
    }

    #[test]
    fn non_utf8_field() -> Result<()> {
        let inval = b"foo\xFF\xFFbar";
        let path = Utf8PathBuf::column_result(ValueRef::Text(inval));
        assert!(path.is_err());
        Ok(())
    }
}
