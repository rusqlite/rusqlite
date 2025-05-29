//! Convert some `jiff` types.

use jiff::{
    civil::{Date, DateTime, Time},
    Timestamp,
};

use crate::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use crate::Result;

/// Gregorian calendar date => "YYYY-MM-DD"
impl ToSql for Date {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        let s = self.to_string();
        Ok(ToSqlOutput::from(s))
    }
}

/// "YYYY-MM-DD" => Gregorian calendar date.
impl FromSql for Date {
    #[inline]
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()
            .and_then(|s| s.parse().map_err(FromSqlError::other))
    }
}
/// time => "HH:MM:SS.SSS"
impl ToSql for Time {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        let date_str = self.to_string();
        Ok(ToSqlOutput::from(date_str))
    }
}

/// "HH:MM:SS.SSS" => time.
impl FromSql for Time {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()
            .and_then(|s| s.parse().map_err(FromSqlError::other))
    }
}

/// Gregorian datetime => "YYYY-MM-DDTHH:MM:SS.SSS"
impl ToSql for DateTime {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        let s = self.to_string();
        Ok(ToSqlOutput::from(s))
    }
}

/// "YYYY-MM-DDTHH:MM:SS.SSS" => Gregorian datetime.
impl FromSql for DateTime {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()
            .and_then(|s| s.parse().map_err(FromSqlError::other))
    }
}

/// UTC time => UTC RFC3339 timestamp
/// ("YYYY-MM-DDTHH:MM:SS.SSSZ").
impl ToSql for Timestamp {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.to_string()))
    }
}

/// RFC3339 ("YYYY-MM-DD HH:MM:SS.SSS[+-]HH:MM") into `Timestamp`.
impl FromSql for Timestamp {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .parse::<Timestamp>()
            .map_err(FromSqlError::other)
    }
}

#[cfg(test)]
mod test {
    use crate::{Connection, Result};
    use jiff::{
        civil::{Date, DateTime, Time},
        Timestamp,
    };

    fn checked_memory_handle() -> Result<Connection> {
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE foo (t TEXT, b BLOB)")?;
        Ok(db)
    }

    #[test]
    fn test_date() -> Result<()> {
        let db = checked_memory_handle()?;
        let date = Date::constant(2016, 2, 23);
        db.execute("INSERT INTO foo (t) VALUES (?1)", [date])?;

        let s: String = db.one_column("SELECT t FROM foo", [])?;
        assert_eq!("2016-02-23", s);
        let t: Date = db.one_column("SELECT t FROM foo", [])?;
        assert_eq!(date, t);

        db.execute("UPDATE foo set b = date(t)", [])?;
        let t: Date = db.one_column("SELECT b FROM foo", [])?;
        assert_eq!(date, t);

        let r: Result<Date> = db.one_column("SELECT '2023-02-29'", []);
        assert!(r.is_err());
        Ok(())
    }

    #[test]
    fn test_time() -> Result<()> {
        let db = checked_memory_handle()?;
        let time = Time::constant(23, 56, 4, 0);
        db.execute("INSERT INTO foo (t) VALUES (?1)", [time])?;

        let s: String = db.one_column("SELECT t FROM foo", [])?;
        assert_eq!("23:56:04", s);
        let v: Time = db.one_column("SELECT t FROM foo", [])?;
        assert_eq!(time, v);

        db.execute("UPDATE foo set b = time(t)", [])?;
        let v: Time = db.one_column("SELECT b FROM foo", [])?;
        assert_eq!(time, v);

        let r: Result<Time> = db.one_column("SELECT '25:22:45'", []);
        assert!(r.is_err());
        Ok(())
    }

    #[test]
    fn test_date_time() -> Result<()> {
        let db = checked_memory_handle()?;
        let dt = DateTime::constant(2016, 2, 23, 23, 56, 4, 0);

        db.execute("INSERT INTO foo (t) VALUES (?1)", [dt])?;

        let s: String = db.one_column("SELECT t FROM foo", [])?;
        assert_eq!("2016-02-23T23:56:04", s);
        let v: DateTime = db.one_column("SELECT t FROM foo", [])?;
        assert_eq!(dt, v);

        db.execute("UPDATE foo set b = datetime(t)", [])?;
        let v: DateTime = db.one_column("SELECT b FROM foo", [])?;
        assert_eq!(dt, v);

        let r: Result<DateTime> = db.one_column("SELECT '2023-02-29T00:00:00'", []);
        assert!(r.is_err());
        Ok(())
    }

    #[test]
    fn test_timestamp() -> Result<()> {
        let db = checked_memory_handle()?;
        let ts: Timestamp = "2016-02-23 23:56:04Z".parse().unwrap();

        db.execute("INSERT INTO foo (t) VALUES (?1)", [ts])?;

        let s: String = db.one_column("SELECT t FROM foo", [])?;
        assert_eq!("2016-02-23T23:56:04Z", s);
        let v: Timestamp = db.one_column("SELECT t FROM foo", [])?;
        assert_eq!(ts, v);

        let r: Result<Timestamp> = db.one_column("SELECT '2023-02-29T00:00:00Z'", []);
        assert!(r.is_err());

        Ok(())
    }

    #[test]
    fn test_timestamp_various_formats() -> Result<()> {
        let db = checked_memory_handle()?;
        // Copied over from a test in `src/types/time.rs`. The format numbers
        // come from <https://sqlite.org/lang_datefunc.html>.
        let tests = vec![
            // Rfc3339
            "2013-10-07T08:23:19.123456789Z",
            "2013-10-07 08:23:19.123456789Z",
            // Format 2
            "2013-10-07 08:23Z",
            "2013-10-07 08:23+04:00",
            // Format 3
            "2013-10-07 08:23:19Z",
            "2013-10-07 08:23:19+04:00",
            // Format 4
            "2013-10-07 08:23:19.123Z",
            "2013-10-07 08:23:19.123+04:00",
            // Format 5
            "2013-10-07T08:23Z",
            "2013-10-07T08:23+04:00",
            // Format 6
            "2013-10-07T08:23:19Z",
            "2013-10-07T08:23:19+04:00",
            // Format 7
            "2013-10-07T08:23:19.123Z",
            "2013-10-07T08:23:19.123+04:00",
        ];

        for string in tests {
            let expected: Timestamp = string.parse().unwrap();
            let result: Timestamp = db.one_column("SELECT ?1", [string])?;
            assert_eq!(result, expected);
        }
        Ok(())
    }
}
