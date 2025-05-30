use super::{Null, Value, ValueRef};
#[cfg(feature = "array")]
use crate::vtab::array::Array;
use crate::{Error, Result};
use std::borrow::Cow;

/// `ToSqlOutput` represents the possible output types for implementers of the
/// [`ToSql`] trait.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ToSqlOutput<'a> {
    /// A borrowed SQLite-representable value.
    Borrowed(ValueRef<'a>),

    /// An owned SQLite-representable value.
    Owned(Value),

    /// A BLOB of the given length that is filled with
    /// zeroes.
    #[cfg(feature = "blob")]
    ZeroBlob(i32),

    /// n-th arg of an SQL scalar function
    #[cfg(feature = "functions")]
    Arg(usize),

    /// `feature = "array"`
    #[cfg(feature = "array")]
    Array(Array),
}

// Generically allow any type that can be converted into a ValueRef
// to be converted into a ToSqlOutput as well.
impl<'a, T: ?Sized> From<&'a T> for ToSqlOutput<'a>
where
    &'a T: Into<ValueRef<'a>>,
{
    #[inline]
    fn from(t: &'a T) -> Self {
        ToSqlOutput::Borrowed(t.into())
    }
}

// We cannot also generically allow any type that can be converted
// into a Value to be converted into a ToSqlOutput because of
// coherence rules (https://github.com/rust-lang/rust/pull/46192),
// so we'll manually implement it for all the types we know can
// be converted into Values.
macro_rules! from_value(
    ($t:ty) => (
        impl From<$t> for ToSqlOutput<'_> {
            #[inline]
            fn from(t: $t) -> Self { ToSqlOutput::Owned(t.into())}
        }
    );
    (non_zero $t:ty) => (
        impl From<$t> for ToSqlOutput<'_> {
            #[inline]
            fn from(t: $t) -> Self { ToSqlOutput::Owned(t.get().into())}
        }
    )
);
from_value!(String);
from_value!(Null);
from_value!(bool);
from_value!(i8);
from_value!(i16);
from_value!(i32);
from_value!(i64);
from_value!(isize);
from_value!(u8);
from_value!(u16);
from_value!(u32);
from_value!(f32);
from_value!(f64);
from_value!(Vec<u8>);

from_value!(non_zero std::num::NonZeroI8);
from_value!(non_zero std::num::NonZeroI16);
from_value!(non_zero std::num::NonZeroI32);
from_value!(non_zero std::num::NonZeroI64);
from_value!(non_zero std::num::NonZeroIsize);
from_value!(non_zero std::num::NonZeroU8);
from_value!(non_zero std::num::NonZeroU16);
from_value!(non_zero std::num::NonZeroU32);

// It would be nice if we could avoid the heap allocation (of the `Vec`) that
// `i128` needs in `Into<Value>`, but it's probably fine for the moment, and not
// worth adding another case to Value.
#[cfg(feature = "i128_blob")]
from_value!(i128);

#[cfg(feature = "i128_blob")]
from_value!(non_zero std::num::NonZeroI128);

#[cfg(feature = "uuid")]
from_value!(uuid::Uuid);

impl ToSql for ToSqlOutput<'_> {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        Ok(match *self {
            ToSqlOutput::Borrowed(v) => ToSqlOutput::Borrowed(v),
            ToSqlOutput::Owned(ref v) => ToSqlOutput::Borrowed(ValueRef::from(v)),

            #[cfg(feature = "blob")]
            ToSqlOutput::ZeroBlob(i) => ToSqlOutput::ZeroBlob(i),
            #[cfg(feature = "functions")]
            ToSqlOutput::Arg(i) => ToSqlOutput::Arg(i),
            #[cfg(feature = "array")]
            ToSqlOutput::Array(ref a) => ToSqlOutput::Array(a.clone()),
        })
    }
}

/// A trait for types that can be converted into SQLite values. Returns
/// [`Error::ToSqlConversionFailure`] if the conversion fails.
pub trait ToSql {
    /// Converts Rust value to SQLite value
    fn to_sql(&self) -> Result<ToSqlOutput<'_>>;
}

impl<T: ToSql + ToOwned + ?Sized> ToSql for Cow<'_, T> {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        self.as_ref().to_sql()
    }
}

impl<T: ToSql + ?Sized> ToSql for Box<T> {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        self.as_ref().to_sql()
    }
}

impl<T: ToSql + ?Sized> ToSql for std::rc::Rc<T> {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        self.as_ref().to_sql()
    }
}

impl<T: ToSql + ?Sized> ToSql for std::sync::Arc<T> {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        self.as_ref().to_sql()
    }
}

// We should be able to use a generic impl like this:
//
// impl<T: Copy> ToSql for T where T: Into<Value> {
//     fn to_sql(&self) -> Result<ToSqlOutput> {
//         Ok(ToSqlOutput::from((*self).into()))
//     }
// }
//
// instead of the following macro, but this runs afoul of
// https://github.com/rust-lang/rust/issues/30191 and reports conflicting
// implementations even when there aren't any.

macro_rules! to_sql_self(
    ($t:ty) => (
        impl ToSql for $t {
            #[inline]
            fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::from(*self))
            }
        }
    )
);

to_sql_self!(Null);
to_sql_self!(bool);
to_sql_self!(i8);
to_sql_self!(i16);
to_sql_self!(i32);
to_sql_self!(i64);
to_sql_self!(isize);
to_sql_self!(u8);
to_sql_self!(u16);
to_sql_self!(u32);
to_sql_self!(f32);
to_sql_self!(f64);

to_sql_self!(std::num::NonZeroI8);
to_sql_self!(std::num::NonZeroI16);
to_sql_self!(std::num::NonZeroI32);
to_sql_self!(std::num::NonZeroI64);
to_sql_self!(std::num::NonZeroIsize);
to_sql_self!(std::num::NonZeroU8);
to_sql_self!(std::num::NonZeroU16);
to_sql_self!(std::num::NonZeroU32);

#[cfg(feature = "i128_blob")]
to_sql_self!(i128);

#[cfg(feature = "i128_blob")]
to_sql_self!(std::num::NonZeroI128);

#[cfg(feature = "uuid")]
to_sql_self!(uuid::Uuid);

macro_rules! to_sql_self_fallible(
    ($t:ty) => (
        impl ToSql for $t {
            #[inline]
            fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::Owned(Value::Integer(
                    i64::try_from(*self).map_err(
                        // TODO: Include the values in the error message.
                        |err| Error::ToSqlConversionFailure(err.into())
                    )?
                )))
            }
        }
    );
    (non_zero $t:ty) => (
        impl ToSql for $t {
            #[inline]
            fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::Owned(Value::Integer(
                    i64::try_from(self.get()).map_err(
                        // TODO: Include the values in the error message.
                        |err| Error::ToSqlConversionFailure(err.into())
                    )?
                )))
            }
        }
    )
);

// Special implementations for usize and u64 because these conversions can fail.
to_sql_self_fallible!(u64);
to_sql_self_fallible!(usize);
to_sql_self_fallible!(non_zero std::num::NonZeroU64);
to_sql_self_fallible!(non_zero std::num::NonZeroUsize);

impl<T: ?Sized> ToSql for &'_ T
where
    T: ToSql,
{
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        (*self).to_sql()
    }
}

impl ToSql for String {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl ToSql for str {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self))
    }
}

impl ToSql for Vec<u8> {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_slice()))
    }
}

impl<const N: usize> ToSql for [u8; N] {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(&self[..]))
    }
}

impl ToSql for [u8] {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self))
    }
}

impl ToSql for Value {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self))
    }
}

impl<T: ToSql> ToSql for Option<T> {
    #[inline]
    fn to_sql(&self) -> Result<ToSqlOutput<'_>> {
        match *self {
            None => Ok(ToSqlOutput::from(Null)),
            Some(ref t) => t.to_sql(),
        }
    }
}

#[cfg(test)]
mod test {
    use super::{ToSql, ToSqlOutput};
    use crate::{types::Value, types::ValueRef, Result};

    fn is_to_sql<T: ToSql>() {}

    #[test]
    fn to_sql() -> Result<()> {
        assert_eq!(
            ToSqlOutput::Borrowed(ValueRef::Null).to_sql()?,
            ToSqlOutput::Borrowed(ValueRef::Null)
        );
        assert_eq!(
            ToSqlOutput::Owned(Value::Null).to_sql()?,
            ToSqlOutput::Borrowed(ValueRef::Null)
        );
        Ok(())
    }

    #[test]
    fn test_integral_types() {
        is_to_sql::<i8>();
        is_to_sql::<i16>();
        is_to_sql::<i32>();
        is_to_sql::<i64>();
        is_to_sql::<isize>();
        is_to_sql::<u8>();
        is_to_sql::<u16>();
        is_to_sql::<u32>();
        is_to_sql::<u64>();
        is_to_sql::<usize>();
    }

    #[test]
    fn test_nonzero_types() {
        is_to_sql::<std::num::NonZeroI8>();
        is_to_sql::<std::num::NonZeroI16>();
        is_to_sql::<std::num::NonZeroI32>();
        is_to_sql::<std::num::NonZeroI64>();
        is_to_sql::<std::num::NonZeroIsize>();
        is_to_sql::<std::num::NonZeroU8>();
        is_to_sql::<std::num::NonZeroU16>();
        is_to_sql::<std::num::NonZeroU32>();
        is_to_sql::<std::num::NonZeroU64>();
        is_to_sql::<std::num::NonZeroUsize>();
    }

    #[test]
    fn test_u8_array() {
        let a: [u8; 99] = [0u8; 99];
        let _a: &[&dyn ToSql] = crate::params![a];
        let r = ToSql::to_sql(&a);

        r.unwrap();
    }

    #[test]
    fn test_cow_str() {
        use std::borrow::Cow;
        let s = "str";
        let cow: Cow<str> = Cow::Borrowed(s);
        let r = cow.to_sql();
        r.unwrap();
        let cow: Cow<str> = Cow::Owned::<str>(String::from(s));
        let r = cow.to_sql();
        r.unwrap();
        // Ensure this compiles.
        let _p: &[&dyn ToSql] = crate::params![cow];
    }

    #[test]
    fn test_box_dyn() {
        let s: Box<dyn ToSql> = Box::new("Hello world!");
        let _s: &[&dyn ToSql] = crate::params![s];
        let r = ToSql::to_sql(&s);

        r.unwrap();
    }

    #[test]
    fn test_box_deref() {
        let s: Box<str> = "Hello world!".into();
        let _s: &[&dyn ToSql] = crate::params![s];
        let r = s.to_sql();

        r.unwrap();
    }

    #[test]
    fn test_box_direct() {
        let s: Box<str> = "Hello world!".into();
        let _s: &[&dyn ToSql] = crate::params![s];
        let r = ToSql::to_sql(&s);

        r.unwrap();
    }

    #[test]
    fn test_cells() {
        use std::{rc::Rc, sync::Arc};

        let source_str: Box<str> = "Hello world!".into();

        let s: Rc<Box<str>> = Rc::new(source_str.clone());
        let _s: &[&dyn ToSql] = crate::params![s];
        let r = s.to_sql();
        r.unwrap();

        let s: Arc<Box<str>> = Arc::new(source_str.clone());
        let _s: &[&dyn ToSql] = crate::params![s];
        let r = s.to_sql();
        r.unwrap();

        let s: Arc<str> = Arc::from(&*source_str);
        let _s: &[&dyn ToSql] = crate::params![s];
        let r = s.to_sql();
        r.unwrap();

        let s: Arc<dyn ToSql> = Arc::new(source_str.clone());
        let _s: &[&dyn ToSql] = crate::params![s];
        let r = s.to_sql();
        r.unwrap();

        let s: Rc<str> = Rc::from(&*source_str);
        let _s: &[&dyn ToSql] = crate::params![s];
        let r = s.to_sql();
        r.unwrap();

        let s: Rc<dyn ToSql> = Rc::new(source_str);
        let _s: &[&dyn ToSql] = crate::params![s];
        let r = s.to_sql();
        r.unwrap();
    }

    #[cfg(feature = "i128_blob")]
    #[test]
    fn test_i128() -> Result<()> {
        use crate::Connection;
        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE foo (i128 BLOB, desc TEXT)")?;
        db.execute(
            "
            INSERT INTO foo(i128, desc) VALUES
                (?1, 'zero'),
                (?2, 'neg one'), (?3, 'neg two'),
                (?4, 'pos one'), (?5, 'pos two'),
                (?6, 'min'), (?7, 'max')",
            [0i128, -1i128, -2i128, 1i128, 2i128, i128::MIN, i128::MAX],
        )?;

        let mut stmt = db.prepare("SELECT i128, desc FROM foo ORDER BY i128 ASC")?;

        let res = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i128>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        assert_eq!(
            res,
            &[
                (i128::MIN, "min".to_owned()),
                (-2, "neg two".to_owned()),
                (-1, "neg one".to_owned()),
                (0, "zero".to_owned()),
                (1, "pos one".to_owned()),
                (2, "pos two".to_owned()),
                (i128::MAX, "max".to_owned()),
            ]
        );
        Ok(())
    }

    #[cfg(feature = "i128_blob")]
    #[test]
    fn test_non_zero_i128() -> Result<()> {
        use std::num::NonZeroI128;
        macro_rules! nz {
            ($x:expr) => {
                NonZeroI128::new($x).unwrap()
            };
        }

        let db = crate::Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE foo (i128 BLOB, desc TEXT)")?;
        db.execute(
            "INSERT INTO foo(i128, desc) VALUES
                (?1, 'neg one'), (?2, 'neg two'),
                (?3, 'pos one'), (?4, 'pos two'),
                (?5, 'min'), (?6, 'max')",
            [
                nz!(-1),
                nz!(-2),
                nz!(1),
                nz!(2),
                nz!(i128::MIN),
                nz!(i128::MAX),
            ],
        )?;
        let mut stmt = db.prepare("SELECT i128, desc FROM foo ORDER BY i128 ASC")?;

        let res = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<(NonZeroI128, String)>, _>>()?;

        assert_eq!(
            res,
            &[
                (nz!(i128::MIN), "min".to_owned()),
                (nz!(-2), "neg two".to_owned()),
                (nz!(-1), "neg one".to_owned()),
                (nz!(1), "pos one".to_owned()),
                (nz!(2), "pos two".to_owned()),
                (nz!(i128::MAX), "max".to_owned()),
            ]
        );
        let err = db.query_row("SELECT ?1", [0i128], |row| row.get::<_, NonZeroI128>(0));
        assert_eq!(err, Err(crate::Error::IntegralValueOutOfRange(0, 0)));
        Ok(())
    }

    #[cfg(feature = "uuid")]
    #[test]
    fn test_uuid() -> Result<()> {
        use crate::{params, Connection};
        use uuid::Uuid;

        let db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE foo (id BLOB CHECK(length(id) = 16), label TEXT);")?;

        let id = Uuid::new_v4();

        db.execute(
            "INSERT INTO foo (id, label) VALUES (?1, ?2)",
            params![id, "target"],
        )?;

        let mut stmt = db.prepare("SELECT id, label FROM foo WHERE id = ?1")?;

        let mut rows = stmt.query(params![id])?;
        let row = rows.next()?.unwrap();

        let found_id: Uuid = row.get_unwrap(0);
        let found_label: String = row.get_unwrap(1);

        assert_eq!(found_id, id);
        assert_eq!(found_label, "target");
        Ok(())
    }
}
