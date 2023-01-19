//! [`ToSql`] and [`FromSql`] implementation for [`time::OffsetDateTime`].
use crate::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use crate::{Error, Result};
use time::format_description::well_known::Rfc3339;
use time::format_description::FormatItem;
use time::macros::format_description;
use time::{Date, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

const PRIMITIVE_SHORT_DATE_TIME_FORMAT: &[FormatItem<'_>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
const PRIMITIVE_DATE_TIME_Z_FORMAT: &[FormatItem<'_>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond]Z");
const OFFSET_SHORT_DATE_TIME_FORMAT: &[FormatItem<'_>] = format_description!(
    "[year]-[month]-[day] [hour]:[minute]:[second][offset_hour sign:mandatory]:[offset_minute]"
);
const OFFSET_DATE_TIME_FORMAT: &[FormatItem<'_>] = format_description!(
    "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond][offset_hour sign:mandatory]:[offset_minute]"
);
const LEGACY_DATE_TIME_FORMAT: &[FormatItem<'_>] = format_description!(
    "[year]-[month]-[day] [hour]:[minute]:[second]:[subsecond] [offset_hour sign:mandatory]:[offset_minute]"
);

const PRIMITIVE_DATE_TIME_FORMAT: &[FormatItem<'_>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond]");
const PRIMITIVE_SHORT_DATE_TIME_FORMAT_T: &[FormatItem<'_>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]");
const PRIMITIVE_DATE_TIME_FORMAT_T: &[FormatItem<'_>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond]");

const DATE_FORMAT: &[FormatItem<'_>] = format_description!("[year]-[month]-[day]");
const TIME_FORMAT: &[FormatItem<'_>] = format_description!("[hour]:[minute]");
const TIME_FORMAT_SECONDS: &[FormatItem<'_>] = format_description!("[hour]:[minute]:[second]");
const TIME_FORMAT_SECONDS_SUBSECONDS: &[FormatItem<'_>] =
    format_description!("[hour]:[minute]:[second].[subsecond]");

impl ToSql for OffsetDateTime {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        // FIXME keep original offset
        let time_string = self
            .to_offset(UtcOffset::UTC)
            .format(&PRIMITIVE_DATE_TIME_Z_FORMAT)
            .map_err(|err| Error::ToSqlConversionFailure(err.into()))?;
        Ok(ToSqlOutput::from(time_string))
    }
}

impl FromSql for OffsetDateTime {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value.as_str().and_then(|s| {
            if s.len() > 10 && s.as_bytes()[10] == b'T' {
                // YYYY-MM-DDTHH:MM:SS.SSS[+-]HH:MM
                return OffsetDateTime::parse(s, &Rfc3339)
                    .map_err(|err| FromSqlError::Other(Box::new(err)));
            }
            let s = s.strip_suffix('Z').unwrap_or(s);
            match s.len() {
                len if len <= 19 => {
                    // TODO YYYY-MM-DDTHH:MM:SS
                    PrimitiveDateTime::parse(s, &PRIMITIVE_SHORT_DATE_TIME_FORMAT)
                        .map(PrimitiveDateTime::assume_utc)
                }
                _ if s.as_bytes()[19] == b':' => {
                    // legacy
                    OffsetDateTime::parse(s, &LEGACY_DATE_TIME_FORMAT)
                }
                _ if s.as_bytes()[19] == b'.' => OffsetDateTime::parse(s, &OFFSET_DATE_TIME_FORMAT)
                    .or_else(|err| {
                        PrimitiveDateTime::parse(s, &PRIMITIVE_DATE_TIME_FORMAT)
                            .map(PrimitiveDateTime::assume_utc)
                            .map_err(|_| err)
                    }),
                _ => OffsetDateTime::parse(s, &OFFSET_SHORT_DATE_TIME_FORMAT),
            }
            .map_err(|err| FromSqlError::Other(Box::new(err)))
        })
    }
}

/// ISO 8601 calendar date without timezone => "YYYY-MM-DD"
impl ToSql for Date {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        let date_str = self
            .format(&DATE_FORMAT)
            .map_err(|err| Error::ToSqlConversionFailure(err.into()))?;
        Ok(ToSqlOutput::from(date_str))
    }
}

/// "YYYY-MM-DD" => ISO 8601 calendar date without timezone.
impl FromSql for Date {
    #[inline]
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value.as_str().and_then(|s| {
            Date::parse(s, &DATE_FORMAT).map_err(|err| FromSqlError::Other(err.into()))
        })
    }
}

/// ISO 8601 time without timezone => "HH:MM:SS.SSS"
impl ToSql for Time {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        let time_str = self
            .format(&TIME_FORMAT_SECONDS_SUBSECONDS)
            .map_err(|err| Error::ToSqlConversionFailure(err.into()))?;
        Ok(ToSqlOutput::from(time_str))
    }
}

/// "HH:MM"/"HH:MM:SS"/"HH:MM:SS.SSS" => ISO 8601 time without timezone.
impl FromSql for Time {
    #[inline]
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value.as_str().and_then(|s| {
            let fmt = match s.len() {
                5 => Ok(&TIME_FORMAT),
                8 => Ok(&TIME_FORMAT_SECONDS),
                len if len > 9 => Ok(&TIME_FORMAT_SECONDS_SUBSECONDS),
                _ => Err(FromSqlError::Other(
                    format!("Unknown time format: {}", s).into(),
                )),
            }?;

            Time::parse(s, fmt).map_err(|err| FromSqlError::Other(err.into()))
        })
    }
}

/// ISO 8601 combined date and time without timezone =>
/// "YYYY-MM-DD HH:MM:SS.SSS"
impl ToSql for PrimitiveDateTime {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        let date_time_str = self
            .format(&PRIMITIVE_DATE_TIME_FORMAT)
            .map_err(|err| Error::ToSqlConversionFailure(err.into()))?;
        Ok(ToSqlOutput::from(date_time_str))
    }
}

/// Parse a `PrimitiveDateTime` in one of the following formats:
/// YYYY-MM-DD HH:MM:SS.SSS
/// YYYY-MM-DDTHH:MM:SS.SSS
/// YYYY-MM-DD HH:MM:SS
/// YYYY-MM-DDTHH:MM:SS
impl FromSql for PrimitiveDateTime {
    #[inline]
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value.as_str().and_then(|s| {
            let has_t = s.len() > 10 && s.as_bytes()[10] == b'T';

            let fmt = match (s.len(), has_t) {
                (19, true) => Ok(&PRIMITIVE_SHORT_DATE_TIME_FORMAT_T),
                (19, false) => Ok(&PRIMITIVE_SHORT_DATE_TIME_FORMAT),
                (l, true) if l > 19 => Ok(&PRIMITIVE_DATE_TIME_FORMAT_T),
                (l, false) if l > 19 => Ok(&PRIMITIVE_DATE_TIME_FORMAT),
                _ => Err(FromSqlError::Other(
                    format!("Unknown date format: {}", s).into(),
                )),
            }?;

            PrimitiveDateTime::parse(s, fmt).map_err(|err| FromSqlError::Other(err.into()))
        })
    }
}

#[cfg(test)]
mod test {

    use crate::types::time::{PRIMITIVE_DATE_TIME_FORMAT, PRIMITIVE_DATE_TIME_FORMAT_T};

    use crate::{Connection, Result};

    use time::format_description::well_known::Rfc3339;
    use time::macros::{date, time};
    use time::{Date, OffsetDateTime, PrimitiveDateTime, Time};

    use super::{PRIMITIVE_SHORT_DATE_TIME_FORMAT, PRIMITIVE_SHORT_DATE_TIME_FORMAT_T};

    fn checked_memory_handle() -> Result<Connection> {
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE foo (t TEXT, i INTEGER, f FLOAT, b BLOB)")?;
        Ok(db)
    }

    #[test]
    fn test_offset_date_time() -> Result<()> {
        let db = checked_memory_handle()?;

        let mut ts_vec = vec![];

        let make_datetime = |secs: i128, nanos: i128| {
            OffsetDateTime::from_unix_timestamp_nanos(1_000_000_000 * secs + nanos).unwrap()
        };

        ts_vec.push(make_datetime(10_000, 0)); //January 1, 1970 2:46:40 AM
                                               // ts_vec.push(make_datetime(10_000, 1000)); //January 1, 1970 2:46:40 AM (and one microsecond)
        ts_vec.push(make_datetime(1_500_391_124, 1_000_000)); //July 18, 2017
        ts_vec.push(make_datetime(2_000_000_000, 2_000_000)); //May 18, 2033
        ts_vec.push(make_datetime(3_000_000_000, 999)); //January 24, 2065
        ts_vec.push(make_datetime(10_000_000_000, 0)); //November 20, 2286

        for ts in ts_vec {
            db.execute("INSERT INTO foo(t) VALUES (?1)", [ts])?;

            let from: OffsetDateTime = db.one_column("SELECT t FROM foo")?;

            db.execute("DELETE FROM foo", [])?;

            assert_eq!(from, ts);
        }
        Ok(())
    }

    #[test]
    fn test_offset_date_time_parsing() -> Result<()> {
        let db = checked_memory_handle()?;
        let tests = vec![
            (
                "2013-10-07 08:23:19",
                OffsetDateTime::parse("2013-10-07T08:23:19Z", &Rfc3339).unwrap(),
            ),
            (
                "2013-10-07 08:23:19Z",
                OffsetDateTime::parse("2013-10-07T08:23:19Z", &Rfc3339).unwrap(),
            ),
            (
                "2013-10-07T08:23:19Z",
                OffsetDateTime::parse("2013-10-07T08:23:19Z", &Rfc3339).unwrap(),
            ),
            (
                "2013-10-07 08:23:19.120",
                OffsetDateTime::parse("2013-10-07T08:23:19.120Z", &Rfc3339).unwrap(),
            ),
            (
                "2013-10-07 08:23:19.120Z",
                OffsetDateTime::parse("2013-10-07T08:23:19.120Z", &Rfc3339).unwrap(),
            ),
            (
                "2013-10-07T08:23:19.120Z",
                OffsetDateTime::parse("2013-10-07T08:23:19.120Z", &Rfc3339).unwrap(),
            ),
            (
                "2013-10-07 04:23:19-04:00",
                OffsetDateTime::parse("2013-10-07T04:23:19-04:00", &Rfc3339).unwrap(),
            ),
            (
                "2013-10-07 04:23:19.120-04:00",
                OffsetDateTime::parse("2013-10-07T04:23:19.120-04:00", &Rfc3339).unwrap(),
            ),
            (
                "2013-10-07T04:23:19.120-04:00",
                OffsetDateTime::parse("2013-10-07T04:23:19.120-04:00", &Rfc3339).unwrap(),
            ),
        ];

        for (s, t) in tests {
            let result: OffsetDateTime = db.query_row("SELECT ?1", [s], |r| r.get(0))?;
            assert_eq!(result, t);
        }
        Ok(())
    }

    #[test]
    fn test_date() -> Result<()> {
        let db = checked_memory_handle()?;
        let date = date!(2016 - 02 - 23);
        db.execute("INSERT INTO foo (t) VALUES (?1)", [date])?;

        let s: String = db.one_column("SELECT t FROM foo")?;
        assert_eq!("2016-02-23", s);
        let t: Date = db.one_column("SELECT t FROM foo")?;
        assert_eq!(date, t);
        Ok(())
    }

    #[test]
    fn test_time() -> Result<()> {
        let db = checked_memory_handle()?;
        let time = time!(23:56:04.00001);
        db.execute("INSERT INTO foo (t) VALUES (?1)", [time])?;

        let s: String = db.one_column("SELECT t FROM foo")?;
        assert_eq!("23:56:04.00001", s);
        let v: Time = db.one_column("SELECT t FROM foo")?;
        assert_eq!(time, v);
        Ok(())
    }

    #[test]
    fn test_primitive_date_time() -> Result<()> {
        let db = checked_memory_handle()?;
        let dt = date!(2016 - 02 - 23).with_time(time!(23:56:04));

        db.execute("INSERT INTO foo (t) VALUES (?1)", [dt])?;

        let s: String = db.one_column("SELECT t FROM foo")?;
        assert_eq!("2016-02-23 23:56:04.0", s);
        let v: PrimitiveDateTime = db.one_column("SELECT t FROM foo")?;
        assert_eq!(dt, v);

        db.execute("UPDATE foo set b = datetime(t)", [])?; // "YYYY-MM-DD HH:MM:SS"
        let hms: PrimitiveDateTime = db.one_column("SELECT b FROM foo")?;
        assert_eq!(dt, hms);
        Ok(())
    }

    #[test]
    fn test_date_parsing() -> Result<()> {
        let db = checked_memory_handle()?;
        let result: Date = db.query_row("SELECT ?1", ["2013-10-07"], |r| r.get(0))?;
        assert_eq!(result, date!(2013 - 10 - 07));
        Ok(())
    }

    #[test]
    fn test_time_parsing() -> Result<()> {
        let db = checked_memory_handle()?;
        let tests = vec![
            ("08:23", time!(08:23)),
            ("08:23:19", time!(08:23:19)),
            ("08:23:19.111", time!(08:23:19.111)),
        ];

        for (s, t) in tests {
            let result: Time = db.query_row("SELECT ?1", [s], |r| r.get(0))?;
            assert_eq!(result, t);
        }
        Ok(())
    }

    #[test]
    fn test_primitive_date_time_parsing() -> Result<()> {
        let db = checked_memory_handle()?;

        let tests = vec![
            (
                "2013-10-07T08:23:19",
                PrimitiveDateTime::parse(
                    "2013-10-07T08:23:19",
                    &PRIMITIVE_SHORT_DATE_TIME_FORMAT_T,
                )
                .unwrap(),
            ),
            (
                "2013-10-07T08:23:19.111",
                PrimitiveDateTime::parse("2013-10-07T08:23:19.111", &PRIMITIVE_DATE_TIME_FORMAT_T)
                    .unwrap(),
            ),
            (
                "2013-10-07 08:23:19",
                PrimitiveDateTime::parse("2013-10-07 08:23:19", &PRIMITIVE_SHORT_DATE_TIME_FORMAT)
                    .unwrap(),
            ),
            (
                "2013-10-07 08:23:19.111",
                PrimitiveDateTime::parse("2013-10-07 08:23:19.111", &PRIMITIVE_DATE_TIME_FORMAT)
                    .unwrap(),
            ),
        ];

        for (s, t) in tests {
            let result: PrimitiveDateTime = db.query_row("SELECT ?1", [s], |r| r.get(0))?;
            assert_eq!(result, t);
        }
        Ok(())
    }

    #[test]
    fn test_sqlite_functions() -> Result<()> {
        let db = checked_memory_handle()?;
        db.one_column::<Time>("SELECT CURRENT_TIME").unwrap();
        db.one_column::<Date>("SELECT CURRENT_DATE").unwrap();
        db.one_column::<PrimitiveDateTime>("SELECT CURRENT_TIMESTAMP")
            .unwrap();
        db.one_column::<OffsetDateTime>("SELECT CURRENT_TIMESTAMP")
            .unwrap();
        Ok(())
    }

    #[test]
    fn test_time_param() -> Result<()> {
        let db = checked_memory_handle()?;
        let now = OffsetDateTime::now_utc().time();
        let result: Result<bool> = db.query_row(
            "SELECT 1 WHERE ?1 BETWEEN time('now', '-1 minute') AND time('now', '+1 minute')",
            [now],
            |r| r.get(0),
        );
        result.unwrap();
        Ok(())
    }

    #[test]
    fn test_date_param() -> Result<()> {
        let db = checked_memory_handle()?;
        let now = OffsetDateTime::now_utc().date();
        let result: Result<bool> = db.query_row(
            "SELECT 1 WHERE ?1 BETWEEN date('now', '-1 day') AND date('now', '+1 day')",
            [now],
            |r| r.get(0),
        );
        result.unwrap();
        Ok(())
    }

    #[test]
    fn test_primitive_date_time_param() -> Result<()> {
        let db = checked_memory_handle()?;
        let now = PrimitiveDateTime::new(
            OffsetDateTime::now_utc().date(),
            OffsetDateTime::now_utc().time(),
        );
        let result: Result<bool> = db.query_row("SELECT 1 WHERE ?1 BETWEEN datetime('now', '-1 minute') AND datetime('now', '+1 minute')", [now], |r| r.get(0));
        result.unwrap();
        Ok(())
    }

    #[test]
    fn test_offset_date_time_param() -> Result<()> {
        let db = checked_memory_handle()?;
        let result: Result<bool> = db.query_row("SELECT 1 WHERE ?1 BETWEEN datetime('now', '-1 minute') AND datetime('now', '+1 minute')", [OffsetDateTime::now_utc()], |r| r.get(0));
        result.unwrap();
        Ok(())
    }
}
