//! Run-Time Limits

use crate::{ffi, Connection};
use std::os::raw::c_int;

/// Run-Time limit categories, for use with [`Connection::limit`] and
/// [`Connection::set_limit`].
///
/// See the official documentation for more information:
/// - <https://www.sqlite.org/c3ref/c_limit_attached.html>
/// - <https://www.sqlite.org/limits.html>
#[repr(i32)]
#[non_exhaustive]
#[cfg_attr(docsrs, doc(cfg(feature = "limits")))]
pub enum Limit {
    /// The maximum size of any string or BLOB or table row, in bytes.
    ///
    /// This is equivalent to `SQLITE_LIMIT_LENGTH` from the C api.
    Length = ffi::SQLITE_LIMIT_LENGTH,
    /// The maximum length of an SQL statement, in bytes.
    ///
    /// This is equivalent to `SQLITE_LIMIT_SQL_LENGTH` from the C api.
    SqlLength = ffi::SQLITE_LIMIT_SQL_LENGTH,
    /// The maximum number of columns in a table definition or in the result set
    /// of a SELECT or the maximum number of columns in an index or in an
    /// ORDER BY or GROUP BY clause.
    ///
    /// This is equivalent to `SQLITE_LIMIT_COLUMN` from the C api.
    Columns = ffi::SQLITE_LIMIT_COLUMN,
    /// The maximum depth of the parse tree on any expression.
    ///
    /// This is equivalent to `SQLITE_LIMIT_EXPR_DEPTH` from the C api.
    ExprDepth = ffi::SQLITE_LIMIT_EXPR_DEPTH,
    /// The maximum number of terms in a compound SELECT statement.
    ///
    /// This is equivalent to `SQLITE_LIMIT_COMPOUND_SELECT` from the C api.
    CompoundSelectTerms = ffi::SQLITE_LIMIT_COMPOUND_SELECT,
    /// The maximum number of instructions in a virtual machine program used to
    /// implement an SQL statement.
    ///
    /// This is equivalent to `SQLITE_LIMIT_VDBE_OP` from the C api.
    VdbeInstructions = ffi::SQLITE_LIMIT_VDBE_OP,
    /// The maximum number of arguments on a function.
    ///
    /// This is equivalent to `SQLITE_LIMIT_FUNCTION_ARG` from the C api.
    FunctionArgs = ffi::SQLITE_LIMIT_FUNCTION_ARG,
    /// The maximum number of attached databases.
    ///
    /// This is equivalent to `SQLITE_LIMIT_ATTACHED` from the C api.
    AttachedDatabases = ffi::SQLITE_LIMIT_ATTACHED,
    /// The maximum length of the pattern argument to the LIKE or GLOB
    /// operators.
    ///
    /// This is equivalent to `SQLITE_LIMIT_LIKE_PATTERN_LENGTH` from the C api.
    LikePatternLength = ffi::SQLITE_LIMIT_LIKE_PATTERN_LENGTH,
    /// The maximum index number of any parameter in an SQL statement.
    ///
    /// This is equivalent to `SQLITE_LIMIT_VARIABLE_NUMBER` from the C api.
    NumParams = ffi::SQLITE_LIMIT_VARIABLE_NUMBER,
    /// The maximum depth of recursion for triggers.
    ///
    /// This is equivalent to `SQLITE_LIMIT_TRIGGER_DEPTH` from the C api.
    TriggerDepth = 10,
    /// The maximum number of auxiliary worker threads that a single prepared
    /// statement may start.
    ///
    /// This is equivalent to `SQLITE_LIMIT_WORKER_THREADS` from the C api.
    WorkerThreads = 11,
}

impl Limit {
    /// An alias for [`Limit::Length`].
    ///
    /// Provided for compatibility with the C API (as well as older versions of
    /// `rusqlite`)
    pub const SQLITE_LIMIT_LENGTH: Self = Self::Length;
    /// An alias for [`Limit::SqlLength`].
    ///
    /// Provided for compatibility with the C API (as well as older versions of
    /// `rusqlite`)
    pub const SQLITE_LIMIT_SQL_LENGTH: Self = Self::SqlLength;
    /// An alias for [`Limit::Columns`].
    ///
    /// Provided for compatibility with the C API (as well as older versions of
    /// `rusqlite`)
    pub const SQLITE_LIMIT_COLUMN: Self = Self::Columns;
    /// An alias for [`Limit::ExprDepth`].
    ///
    /// Provided for compatibility with the C API (as well as older versions of
    /// `rusqlite`)
    pub const SQLITE_LIMIT_EXPR_DEPTH: Self = Self::ExprDepth;
    /// An alias for [`Limit::CompoundSelectTerms`].
    ///
    /// Provided for compatibility with the C API (as well as older versions of
    /// `rusqlite`)
    pub const SQLITE_LIMIT_COMPOUND_SELECT: Self = Self::CompoundSelectTerms;
    /// An alias for [`Limit::VdbeInstructions`].
    ///
    /// Provided for compatibility with the C API (as well as older versions of
    /// `rusqlite`)
    pub const SQLITE_LIMIT_VDBE_OP: Self = Self::VdbeInstructions;
    /// An alias for [`Limit::FunctionArgs`].
    ///
    /// Provided for compatibility with the C API (as well as older versions of
    /// `rusqlite`)
    pub const SQLITE_LIMIT_FUNCTION_ARG: Self = Self::FunctionArgs;
    /// An alias for [`Limit::AttachedDatabases`].
    ///
    /// Provided for compatibility with the C API (as well as older versions of
    /// `rusqlite`)
    pub const SQLITE_LIMIT_ATTACHED: Self = Self::AttachedDatabases;
    /// An alias for [`Limit::LikePatternLength`].
    ///
    /// Provided for compatibility with the C API (as well as older versions of
    /// `rusqlite`)
    pub const SQLITE_LIMIT_LIKE_PATTERN_LENGTH: Self = Self::LikePatternLength;
    /// An alias for [`Limit::NumParams`].
    ///
    /// Provided for compatibility with the C API (as well as older versions of
    /// `rusqlite`)
    pub const SQLITE_LIMIT_VARIABLE_NUMBER: Self = Self::NumParams;
    /// An alias for [`Limit::TriggerDepth`].
    ///
    /// Provided for compatibility with the C API (as well as older versions of
    /// `rusqlite`)
    pub const SQLITE_LIMIT_TRIGGER_DEPTH: Self = Self::TriggerDepth;
    /// An alias for [`Limit::WorkerThreads`].
    ///
    /// Provided for compatibility with the C API (as well as older versions of
    /// `rusqlite`)
    pub const SQLITE_LIMIT_WORKER_THREADS: Self = Self::WorkerThreads;
}

impl Connection {
    /// Returns the current value of a [`Limit`].
    #[inline]
    #[cfg_attr(docsrs, doc(cfg(feature = "limits")))]
    pub fn limit(&self, limit: Limit) -> i32 {
        let c = self.db.borrow();
        unsafe { ffi::sqlite3_limit(c.db(), limit as c_int, -1) }
    }

    /// Changes the [`Limit`]'s value to `new_val`, returning the prior
    /// value of the limit.
    #[inline]
    #[cfg_attr(docsrs, doc(cfg(feature = "limits")))]
    pub fn set_limit(&self, limit: Limit, new_val: i32) -> i32 {
        let c = self.db.borrow_mut();
        unsafe { ffi::sqlite3_limit(c.db(), limit as c_int, new_val) }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{Connection, Result};

    #[test]
    #[cfg(feature = "bundled")]
    fn test_limit_values() {
        macro_rules! check_all {
            ($(($RustName:ident, $SQLITE_NAME:ident)),+ $(,)?) => {{
                // Hack to force a compile failure if we add a DbConfig variant
                // without updating this test.
                const _: fn(Limit) = |r| match r { $(Limit::$RustName => {}),+ };
                $({
                    assert_eq!(Limit::$RustName as i32, crate::ffi::$SQLITE_NAME as i32);
                    assert_eq!(
                        Limit::$SQLITE_NAME as i32,
                        crate::ffi::$SQLITE_NAME as i32,
                    );
                })+
            }};
        }
        check_all![
            (Length, SQLITE_LIMIT_LENGTH),
            (SqlLength, SQLITE_LIMIT_SQL_LENGTH),
            (Columns, SQLITE_LIMIT_COLUMN),
            (ExprDepth, SQLITE_LIMIT_EXPR_DEPTH),
            (CompoundSelectTerms, SQLITE_LIMIT_COMPOUND_SELECT),
            (VdbeInstructions, SQLITE_LIMIT_VDBE_OP),
            (FunctionArgs, SQLITE_LIMIT_FUNCTION_ARG),
            (AttachedDatabases, SQLITE_LIMIT_ATTACHED),
            (LikePatternLength, SQLITE_LIMIT_LIKE_PATTERN_LENGTH),
            (NumParams, SQLITE_LIMIT_VARIABLE_NUMBER),
            (TriggerDepth, SQLITE_LIMIT_TRIGGER_DEPTH),
            (WorkerThreads, SQLITE_LIMIT_WORKER_THREADS),
        ];
    }

    #[test]
    fn test_limit() -> Result<()> {
        let db = Connection::open_in_memory()?;
        db.set_limit(Limit::Length, 1024);
        assert_eq!(1024, db.limit(Limit::Length));

        db.set_limit(Limit::SqlLength, 1024);
        assert_eq!(1024, db.limit(Limit::SqlLength));

        db.set_limit(Limit::Columns, 64);
        assert_eq!(64, db.limit(Limit::Columns));

        db.set_limit(Limit::ExprDepth, 256);
        assert_eq!(256, db.limit(Limit::ExprDepth));

        db.set_limit(Limit::CompoundSelectTerms, 32);
        assert_eq!(32, db.limit(Limit::CompoundSelectTerms));

        db.set_limit(Limit::NumParams, 32);
        assert_eq!(32, db.limit(Limit::NumParams));

        db.set_limit(Limit::AttachedDatabases, 2);
        assert_eq!(2, db.limit(Limit::AttachedDatabases));

        db.set_limit(Limit::LikePatternLength, 128);
        assert_eq!(128, db.limit(Limit::LikePatternLength));

        db.set_limit(Limit::NumParams, 99);
        assert_eq!(99, db.limit(Limit::NumParams));

        // SQLITE_LIMIT_TRIGGER_DEPTH was added in SQLite 3.6.18.
        if crate::version_number() >= 3_006_018 {
            db.set_limit(Limit::TriggerDepth, 32);
            assert_eq!(32, db.limit(Limit::TriggerDepth));
        }

        // SQLITE_LIMIT_WORKER_THREADS was added in SQLite 3.8.7.
        if crate::version_number() >= 3_008_007 {
            db.set_limit(Limit::WorkerThreads, 2);
            assert_eq!(2, db.limit(Limit::WorkerThreads));
        }
        Ok(())
    }
}
