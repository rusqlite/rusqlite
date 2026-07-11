//! [`ToSql`] and [`FromSql`] implementation for [`Ulid`].
use crate::Result;
use crate::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use ulid::Ulid;

/// Serialize `Ulid` to text.
impl ToSql for Ulid {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.to_string()))
    }
}

/// Deserialize text to `Ulid`.
impl FromSql for Ulid {
    #[inline]
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()
            .and_then(|s| Ulid::from_string(s).map_err(FromSqlError::other))
    }
}

#[cfg(all(test, not(miri)))]
mod test {
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use std::error::Error as _;
    use ulid::Ulid;

    use crate::types::{FromSql as _, ToSql as _, ToSqlOutput, Value, ValueRef};

    #[test]
    fn to_sql() {
        let ulid = Ulid::new();
        assert_eq!(
            ulid.to_sql(),
            Ok(ToSqlOutput::Owned(Value::Text(ulid.to_string())))
        );
    }

    #[test]
    fn from_sql() {
        let ulid = Ulid::new();
        let str = ulid.to_string();
        let value = ValueRef::Text(str.as_bytes());
        assert_eq!(Ulid::column_result(value), Ok(ulid));
        let value = ValueRef::Text("invalid".as_bytes());
        let err = Ulid::column_result(value).unwrap_err();
        let source = err.source().unwrap();
        assert_eq!(
            format!("{}", source),
            format!("{}", ulid::DecodeError::InvalidLength)
        );
    }
}
