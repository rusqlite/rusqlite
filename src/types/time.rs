//! [`ToSql`] and [`FromSql`] implementation for [`time::OffsetDateTime`].
use crate::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use crate::Result;
use std::ops::Neg;
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

const SQLITE_DATETIME_FMT: &str = "%Y-%m-%d %H:%M:%S.%NZ";
const SQLITE_DATETIME_FMT_LEGACY: &str = "%Y-%m-%d %H:%M:%S:%N %z";
const TIMESTAMP_FMT: &str = "%Y-%m-%d %H:%M";
const TIMESTAMPTFMT: &str = "%Y-%m-%dT%H:%M";
const TIMESTAMP_FMT_SEC: &str = "%Y-%m-%d %H:%M:%S";
const TIMESTAMPTFMT_SEC: &str = "%Y-%m-%dT%H:%M:%S";
const TIMESTAMP_FMT_SEC_FRAC: &str = "%Y-%m-%d %H:%M:%S.%N";
const TIMESTAMPTFMT_SEC_FRAC: &str = "%Y-%m-%dT%H:%M:%S.%N";

impl ToSql for OffsetDateTime {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        let mut time_string = self
            .to_offset(UtcOffset::UTC)
            .format(TIMESTAMP_FMT_SEC_FRAC);
        /* Truncate nanosecond precision */
        time_string.truncate(time_string.len() - 6);
        Ok(ToSqlOutput::from(time_string))
    }
}

impl FromSql for OffsetDateTime {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value.as_str().and_then(|mut s| {
            let original_s = s;
            let mut offset = time::UtcOffset::UTC;
            if s.len() == 10 {
                return PrimitiveDateTime::parse(s, "%F")
                    .map(|d| d.assume_offset(offset))
                    .map_err(|err| FromSqlError::Other(Box::new(err)));
            }
            /* Detect timezone specifiers */
            if s.ends_with('Z') {
                /* Z means UTC, so this is useless for parsing since our
                 * default is UTC */
                s = &s[..s.len() - 1];
            }
            let has_t: bool = s.as_bytes().get(10) == Some(&b'T');
            /* Check if s ends with "[+-]HH:MM" */
            if s.len() > "+00:00".len() {
                let time_suffix: &str = &s[s.len() - "+00:00".len()..];
                if time_suffix.starts_with('-') || time_suffix.starts_with('+') {
                    s = &s[..s.len() - "+00:00".len()];
                    let is_neg: bool = time_suffix.starts_with('-');
                    let mut hr_b_1 = time_suffix.as_bytes()[1];
                    let mut hr_b_2 = time_suffix.as_bytes()[2];
                    let mut mn_b_1 = time_suffix.as_bytes()[4];
                    let mut mn_b_2 = time_suffix.as_bytes()[5];
                    if ![hr_b_1, hr_b_2, mn_b_1, mn_b_2]
                        .iter()
                        .all(u8::is_ascii_digit)
                    {
                        return Err(FromSqlError::InvalidDatetime(format!(
                            "Value {:?} contains non-digit characters in the timezone indicator.",
                            original_s
                        )));
                    }
                    hr_b_1 -= b'0';
                    hr_b_2 -= b'0';
                    mn_b_1 -= b'0';
                    mn_b_2 -= b'0';
                    let hours = (hr_b_1 * 10 + hr_b_2) as i16;
                    let minutes = hours * 60 + (mn_b_1 as i16) * 10 + mn_b_2 as i16;
                    offset = if is_neg {
                        time::UtcOffset::minutes(minutes.neg())
                    } else {
                        time::UtcOffset::minutes(minutes)
                    };
                }
            }
            match s.len() {
                16 if has_t => {
                    PrimitiveDateTime::parse(s, TIMESTAMPTFMT).map(|d| d.assume_offset(offset))
                }
                16 => PrimitiveDateTime::parse(s, TIMESTAMP_FMT).map(|d| d.assume_offset(offset)),
                19 if has_t => {
                    PrimitiveDateTime::parse(s, TIMESTAMPTFMT_SEC).map(|d| d.assume_offset(offset))
                }
                19 => {
                    PrimitiveDateTime::parse(s, TIMESTAMP_FMT_SEC).map(|d| d.assume_offset(offset))
                }
                23 if has_t => {
                    PrimitiveDateTime::parse(format!("{}000000", s), TIMESTAMPTFMT_SEC_FRAC)
                        .map(|d| d.assume_offset(offset))
                }
                23 => PrimitiveDateTime::parse(format!("{}000000", s), TIMESTAMP_FMT_SEC_FRAC)
                    .map(|d| d.assume_offset(offset)),
                _ => PrimitiveDateTime::parse(s, SQLITE_DATETIME_FMT)
                    .map(|d| d.assume_offset(offset))
                    .or_else(|err| {
                        OffsetDateTime::parse(original_s, SQLITE_DATETIME_FMT_LEGACY)
                            .map_err(|_| err)
                    }),
            }
            .map_err(|err| FromSqlError::Other(Box::new(err)))
        })
    }
}

#[cfg(test)]
mod test {
    use crate::{Connection, Result};
    use std::time::Duration;
    use time::OffsetDateTime;

    fn checked_memory_handle() -> Result<Connection> {
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE foo (t TEXT, i INTEGER, f FLOAT)")?;
        Ok(db)
    }

    #[test]
    fn test_offset_date_time() -> Result<()> {
        let db = checked_memory_handle()?;

        let mut ts_vec = vec![];

        let make_datetime = |secs, millis| {
            OffsetDateTime::from_unix_timestamp(secs) + Duration::from_millis(millis)
        };

        ts_vec.push(make_datetime(10_000, 0)); //January 1, 1970 2:46:40 AM
        ts_vec.push(make_datetime(10_000, 1000)); //January 1, 1970 2:46:40 AM (and one second)
        ts_vec.push(make_datetime(1_500_391_124, 100)); //July 18, 2017
        ts_vec.push(make_datetime(2_000_000_000, 200)); //May 18, 2033
        ts_vec.push(make_datetime(3_000_000_000, 999)); //January 24, 2065
        ts_vec.push(make_datetime(10_000_000_000, 0)); //November 20, 2286

        for ts in ts_vec {
            db.execute("INSERT INTO foo(t) VALUES (?)", [ts])?;

            let from: OffsetDateTime = db.query_row("SELECT t FROM foo", [], |r| r.get(0))?;

            db.execute("DELETE FROM foo", [])?;

            assert_eq!(from, ts);
        }

        let inputs = &[
            "2021-06-06 15:01",
            "2021-06-06 15:01-04:00",
            "2021-06-06 15:01Z",
            "2021-06-06 15:01:11",
            "2021-06-06 15:01:11Z",
            "2021-06-06 15:01:11.697",
            "2021-06-06 15:01:11.697Z",
            "2021-06-06T15:01",
            "2021-06-06T15:01Z",
            "2021-06-06T15:01:11",
            "2021-06-06T15:01:11+02:00",
            "2021-06-06T15:01:11Z",
            "2021-06-06T15:01:11.697",
            "2021-06-06T15:01:11.697Z",
            "2013-10-07 08:23:19.120",
            "2013-10-07T08:23:19.120Z",
            "2013-10-07 04:23:19.120+06:30",
            "2013-10-07 04:23:19.120-04:00",
        ];

        for ts in inputs.as_ref() {
            db.execute("INSERT INTO foo(t) VALUES (?)", [ts])?;

            let from: OffsetDateTime = db.query_row("SELECT t FROM foo", [], |r| r.get(0))?;
            db.execute("DELETE FROM foo", [])?;

            let from: String = from.format("%Y-%m-%d %H:%M:%S.%N %z");
            let mut timezone_indicator_range = None;
            let mut ptr: usize;
            /*
             * Byte indexes:
             *
             * "2013-10-07 08:23:19.120000000 +0000"
             *  000000000011111111112222222222333333
             *  012345678901234567890123456789012345
             */
            /* first 10 bytes must be identical: */
            assert_eq!(&from[..10], &ts[..10]);
            /* HH:MM  must be equal: */
            assert_eq!(&from[12..16], &ts[12..16]);
            if ts.len() > 18 {
                /* Includes second information and/or timezone indicator. */

                ptr = 16;
                /* timezone indicator. */
                if [b'-', b'+'].contains(&ts.as_bytes()[ptr]) {
                    timezone_indicator_range = Some((ptr..ptr + 3, ptr + 4..ptr + 6));
                } else if ts.as_bytes()[16] == b':' {
                    /* Has SS seconds part. */
                    ptr = 17;
                    assert_eq!(&ts.as_bytes()[ptr..ptr + 2], &from.as_bytes()[17..19]);
                    ptr += 2;
                    if ts.as_bytes().get(ptr) == Some(&b'.') {
                        /* Has .SSS fractional second (milliseconds) part. */
                        ptr += 1;
                        assert_eq!(&ts.as_bytes()[ptr..ptr + 3], &from.as_bytes()[20..23]);
                        ptr += 3;
                    }
                    if ts.as_bytes().get(ptr) == Some(&b'Z') {
                        ptr += 1;
                    }
                    if ts.as_bytes().get(ptr).is_some()
                        && [b'-', b'+'].contains(&ts.as_bytes()[ptr])
                    {
                        timezone_indicator_range = Some((ptr..ptr + 3, ptr + 4..ptr + 6));
                    }
                }
            }

            if let Some((hr, mn)) = timezone_indicator_range {
                /* Check timezone indicator. */
                /* (time crate formats timezone info as [-+]HHMM and sqlite as [-+]HH:MM) */
                /* Compare sign and hour parts */
                assert_eq!(&ts.as_bytes()[hr], &from.as_bytes()[30..33]);

                /* Compare minute parts */
                assert_eq!(&ts.as_bytes()[mn], &from.as_bytes()[33..35]);
            }
        }
        let invalid_inputs = &[
            "2021-06-06 15:0",
            "2021-06-06 15:00      ",
            "2021-06-06 15:01ZZ",
            "2021-06-06 15:01-04T00",
            "2013-10-07 04:23:19.12",
            "2013-10-07 04:23:19.120-0400",
            "2013-10-07 04:23:19.120000000",
        ];
        for ts in invalid_inputs.as_ref() {
            db.execute("INSERT INTO foo(t) VALUES (CAST(? AS DATETIME))", [ts])?;

            let from: Result<OffsetDateTime> = db.query_row("SELECT t FROM foo", [], |r| r.get(0));
            assert!(from.is_err());
            db.execute("DELETE FROM foo", [])?;
        }
        Ok(())
    }

    #[test]
    fn test_sqlite_functions() -> Result<()> {
        let db = checked_memory_handle()?;
        let result: Result<OffsetDateTime> =
            db.query_row("SELECT CURRENT_TIMESTAMP", [], |r| r.get(0));
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_param() -> Result<()> {
        let db = checked_memory_handle()?;
        let result: Result<bool> = db.query_row("SELECT 1 WHERE ? BETWEEN datetime('now', '-1 minute') AND datetime('now', '+1 minute')", [OffsetDateTime::now_utc()], |r| r.get(0));
        assert!(result.is_ok());
        Ok(())
    }
}
