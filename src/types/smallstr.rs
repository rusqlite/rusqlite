#![warn(clippy::pedantic)]
//! Implementation of [`FromSql`] and [`ToSql`] for [`SmallString`]

use smallstr::SmallString;
use smallvec::Array;

use crate::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use crate::Result;

/// Deserialize a TEXT SQL value to a `SmallString`.
///
/// The behaviour of conversion should be identical to that of `String`, `Box<str>`, etc
impl<A: Array<Item = u8>> FromSql for SmallString<A> {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value.as_str().map(From::from)
    }
}

/// Serialize a TEXT SQL value from a `SmallString`.
///
/// The behaviour of conversion should be identical to that of `String`, `Box<str>`, `&str`, etc
impl<A: Array<Item = u8>> ToSql for SmallString<A> {
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        self.as_str().to_sql()
    }
}

#[cfg(test)]
mod tests {

    use smallstr::SmallString;

    use crate::{Connection, Result};

    #[test]
    fn round_trip() -> Result<()> {
        let db = Connection::open_in_memory()?;

        let slug = SmallString::<[u8; 16]>::from("apple");

        db.execute_batch(
            "CREATE TABLE strings (id INTEGER PRIMARY KEY AUTOINCREMENT, data TEXT) STRICT",
        )?;

        let id = db
            .prepare("INSERT INTO strings (data) VALUES (?1)")?
            .insert([slug])?;

        let stored: SmallString<[u8; 16]> =
            db.query_row("SELECT * FROM strings WHERE id=?1", [id], |row| {
                row.get("data")
            })?;

        assert_eq!(stored, "apple");

        Ok(())
    }
}
