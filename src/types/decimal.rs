//! [`ToSql`] and [`FromSql`] implementation for [`rust_decimal::Decimal`].
use crate::types::{FromSql, FromSqlError, FromSqlResult,  ValueRef};
use crate::Result;
use rust_decimal::Decimal;
use std::convert::TryInto;
use std::str::FromStr;

fn parse_from_str(s: &str) -> Result<Decimal, FromSqlError> {
    Decimal::from_str(s).map_err(|e| FromSqlError::Other(Box::new(e)))
}

/// Deserialize to `Decimal`.
impl FromSql for Decimal {
    #[inline]
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Integer(x) => Ok(Decimal::from(x)),
            //Note: Parsing straight from f64 could cause loss of precision, when parsing from text
            // works fine like for example f64::MAX
            ValueRef::Real(x) => parse_from_str(&x.to_string()),
            ValueRef::Text(_) => value.as_str().and_then(parse_from_str),
            ValueRef::Blob(x) => match x.try_into() {
                Ok(bytes) => Ok(Decimal::deserialize(bytes)),
                Err(e) => Err(FromSqlError::Other(Box::new(e))),
            },
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::types::Type;
    use crate::{params, Connection, Error, Result};

    fn checked_memory_handle() -> Result<Connection> {
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE Decimals (i INTEGER, v DECIMAL)")?;
        Ok(db)
    }

    fn get_decimal(db: &Connection, id: i64) -> Result<Decimal> {
        db.query_row("SELECT v FROM Decimals WHERE i = ?", [id], |r| r.get(0))
    }

    #[test]
    fn test_sql_decimal() -> Result<()> {
        let db = &checked_memory_handle()?;

        let zero = Decimal::from(0);
        let max_decimal = Decimal::MAX;

        db.execute(
            "INSERT INTO Decimals (i, v) VALUES (0, ?), (1, ?), (2, ?)",
            // also insert invalid data that will fail to decode to decimal
            params![0,  max_decimal, "illegal"],
        )?;

        assert_eq!(get_decimal(db, 0)?, zero);
        assert_eq!(get_decimal(db, 1)?, max_decimal);
        //This is in fact a parsing error...
        matches!(
            get_decimal(db,2),
            Err(Error::FromSqlConversionFailure(0, Type::Text, ..))
        );

        Ok(())
    }
}
