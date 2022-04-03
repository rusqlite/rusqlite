//! Configure database connections

use std::os::raw::c_int;

use crate::error::check;
use crate::ffi;
use crate::{Connection, Result};

/// Per-Connection databse configuration options.
///
/// See [Database Connection Configuration Options][dbconfig] for details.
///
/// [dbconfig]: https://sqlite.org/c3ref/c_dbconfig_enable_fkey.html
#[repr(i32)]
#[non_exhaustive]
pub enum DbConfig {
    //SQLITE_DBCONFIG_MAINDBNAME = 1000,
    //SQLITE_DBCONFIG_LOOKASIDE = 1001,
    /// Enable or disable the enforcement of foreign key constraints.
    ///
    /// Note that by default these are off in most versions of SQLite, although
    /// if you are using `features = "bundled"`, they will be on by default
    /// (because the bundled build is performed with the compiler flag
    /// `-DSQLITE_DEFAULT_FOREIGN_KEYS=1`).
    ///
    /// It is frequently useful to temporarially disable this during migrations.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_ENABLE_FKEY` from the C API.
    ForeignKeysEnabled = 1002,

    /// Enable or disable triggers.
    ///
    /// Requires SQLite 3.12.0 or later, and will produce an error in earlier
    /// versions.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_ENABLE_TRIGGER` from the C API.
    TriggersEnabled = 1003,

    /// Enable or disable the fts3_tokenizer() function which is part of the
    /// FTS3 full-text search engine extension.
    ///
    /// Requires SQLite 3.12.0 or later, and will produce an error in earlier
    /// versions.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER` from the C API.
    Fts3TokenizerEnabled = 1004,

    // Would not be a safe API, so we omit it.
    //SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION = 1005,
    /// In WAL mode, enable or disable the checkpoint operation before closing
    /// the connection.
    ///
    /// Requires SQLite 3.16.2 or later, and will produce an error in earlier
    /// versions.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE` from the C API.
    NoCheckpointOnClose = 1006,

    /// Activates or deactivates the query planner stability guarantee (QPSG).
    ///
    /// Requires SQLite 3.20.0 or later, and will produce an error in earlier
    /// versions.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_ENABLE_QPSG` from the C API.
    QueryPlannerStabilityEnabled = 1007,

    /// Includes or excludes output for any operations performed by trigger
    /// programs from the output of `EXPLAIN QUERY PLAN` commands.
    ///
    /// Requires SQLite 3.22.0 or later, and will produce an error in earlier
    /// versions.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_TRIGGER_EQP` from the C API.
    TriggersInExplainQueryPlan = 1008,

    /// Activates or deactivates the "reset" flag for a database connection.
    /// Run VACUUM with this flag set to reset the database.
    ///
    /// Requires SQLite 3.24.0 or later, and will produce an error in earlier
    /// versions.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_RESET_DATABASE` from the C API.
    ResetDatabase = 1009,

    /// Activates or deactivates the "defensive" flag for a database connection.
    ///
    /// Requires SQLite 3.26.0 or later, and will produce an error in earlier
    /// versions.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_DEFENSIVE` from the C API.
    Defensive = 1010,

    /// Activates or deactivates the "writable_schema" flag.
    ///
    /// Requires SQLite 3.28.0 or later, and will produce an error in earlier
    /// versions.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_WRITABLE_SCHEMA` from the C API.
    WritableSchema = 1011,

    /// Activates or deactivates the legacy behavior of the `ALTER TABLE RENAME`
    /// command.
    ///
    /// Requires SQLite 3.29.0 or later, and will produce an error in earlier
    /// versions.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_LEGACY_ALTER_TABLE` from the C API.
    LegacyAlterTable = 1012,

    /// Activates or deactivates the legacy double-quoted string literal
    /// "misfeature" for data manipulation language (DML) statements only; that
    /// is, in statements which are not modifying the schema.
    ///
    /// Requires SQLite 3.29.0 or later, and will produce an error in earlier
    /// versions.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_DQS_DML` from the C API.
    DoubleQuotedStringInDML = 1013,

    /// Activates or deactivates the legacy double-quoted string literal
    /// "misfeature" for data definition language (DDL) statements; that
    /// is, in statements which are modifying the schema.
    ///
    /// Requires SQLite 3.29.0 or later, and will produce an error in earlier
    /// versions.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_DQS_DDL` from the C API.
    DoubleQuotedStringInDDL = 1014,

    /// Enable or disable views.
    ///
    /// Requires SQLite 3.30.0 or later, and will produce an error in earlier
    /// versions.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_ENABLE_VIEW` from the C API.
    EnableViews = 1015,

    /// Activates or deactivates the legacy file format flag.
    ///
    /// Requires SQLite 3.31.0 or later, and will produce an error in earlier
    /// versions.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_LEGACY_FILE_FORMAT` from the C API.
    LegacyFileFormat = 1016,

    /// Tells SQLite to assume that database schemas (the contents of the
    /// sqlite_master tables) are untainted by malicious content.
    ///
    /// Requires SQLite 3.31.0 or later, and will produce an error in earlier
    /// versions.
    ///
    /// Equivalent to `SQLITE_DBCONFIG_TRUSTED_SCHEMA` from the C API.
    TrustedSchema = 1017,
}

impl DbConfig {
    /// Alias for [`DbConfig::ForeignKeysEnabled`], provided for compatibility
    /// with the C API.
    pub const SQLITE_DBCONFIG_ENABLE_FKEY: Self = Self::ForeignKeysEnabled;
    /// Alias for [`DbConfig::TriggersEnabled`], provided for compatibility with
    /// the C API.
    pub const SQLITE_DBCONFIG_ENABLE_TRIGGER: Self = Self::TriggersEnabled;
    /// Alias for [`DbConfig::Fts3TokenizerEnabled`], provided for compatibility
    /// with the C API.
    pub const SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER: Self = Self::Fts3TokenizerEnabled;
    /// Alias for [`DbConfig::NoCheckpointOnClose`], provided for compatibility
    /// with the C API.
    pub const SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE: Self = Self::NoCheckpointOnClose;
    /// Alias for [`DbConfig::QueryPlannerStabilityEnabled`], provided for
    /// compatibility with the C API.
    pub const SQLITE_DBCONFIG_ENABLE_QPSG: Self = Self::QueryPlannerStabilityEnabled;
    /// Alias for [`DbConfig::TriggersInExplainQueryPlan`], provided for
    /// compatibility with the C API.
    pub const SQLITE_DBCONFIG_TRIGGER_EQP: Self = Self::TriggersInExplainQueryPlan;
    /// Alias for [`DbConfig::ResetDatabase`], provided for compatibility with
    /// the C API.
    pub const SQLITE_DBCONFIG_RESET_DATABASE: Self = Self::ResetDatabase;
    /// Alias for [`DbConfig::Defensive`], provided for compatibility with the C
    /// API.
    pub const SQLITE_DBCONFIG_DEFENSIVE: Self = Self::Defensive;
    /// Alias for [`DbConfig::WritableSchema`], provided for compatibility with
    /// the C API.
    pub const SQLITE_DBCONFIG_WRITABLE_SCHEMA: Self = Self::WritableSchema;
    /// Alias for [`DbConfig::LegacyAlterTable`], provided for compatibility
    /// with the C API.
    pub const SQLITE_DBCONFIG_LEGACY_ALTER_TABLE: Self = Self::LegacyAlterTable;
    /// Alias for [`DbConfig::DoubleQuotedStringInDML`], provided for
    /// compatibility with the C API.
    pub const SQLITE_DBCONFIG_DQS_DML: Self = Self::DoubleQuotedStringInDML;
    /// Alias for [`DbConfig::DoubleQuotedStringInDDL`], provided for
    /// compatibility with the C API.
    pub const SQLITE_DBCONFIG_DQS_DDL: Self = Self::DoubleQuotedStringInDDL;
    /// Alias for [`DbConfig::EnableViews`], provided for compatibility with the
    /// C API.
    pub const SQLITE_DBCONFIG_ENABLE_VIEW: Self = Self::EnableViews;
    /// Alias for [`DbConfig::LegacyFileFormat`], provided for compatibility
    /// with the C API.
    pub const SQLITE_DBCONFIG_LEGACY_FILE_FORMAT: Self = Self::LegacyFileFormat;
    /// Alias for [`DbConfig::TrustedSchema`], provided for compatibility with
    /// the C API.
    pub const SQLITE_DBCONFIG_TRUSTED_SCHEMA: Self = Self::TrustedSchema;
}

impl Connection {
    /// Returns the current value of a [`DbConfig`].
    ///
    /// - `SQLITE_DBCONFIG_ENABLE_FKEY`: return `false` or `true` to indicate
    ///   whether FK enforcement is off or on
    /// - `SQLITE_DBCONFIG_ENABLE_TRIGGER`: return `false` or `true` to indicate
    ///   whether triggers are disabled or enabled
    /// - `SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER`: return `false` or `true` to
    ///   indicate whether `fts3_tokenizer` are disabled or enabled
    /// - `SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE`: return `false` to indicate
    ///   checkpoints-on-close are not disabled or `true` if they are
    /// - `SQLITE_DBCONFIG_ENABLE_QPSG`: return `false` or `true` to indicate
    ///   whether the QPSG is disabled or enabled
    /// - `SQLITE_DBCONFIG_TRIGGER_EQP`: return `false` to indicate
    ///   output-for-trigger are not disabled or `true` if it is
    #[inline]
    pub fn db_config(&self, config: DbConfig) -> Result<bool> {
        let c = self.db.borrow();
        unsafe {
            let mut val = 0;
            check(ffi::sqlite3_db_config(
                c.db(),
                config as c_int,
                -1,
                &mut val,
            ))?;
            Ok(val != 0)
        }
    }

    /// Make [configuration](DbConfig) changes to a database connection
    ///
    /// - `SQLITE_DBCONFIG_ENABLE_FKEY`: `false` to disable FK enforcement,
    ///   `true` to enable FK enforcement
    /// - `SQLITE_DBCONFIG_ENABLE_TRIGGER`: `false` to disable triggers, `true`
    ///   to enable triggers
    /// - `SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER`: `false` to disable
    ///   `fts3_tokenizer()`, `true` to enable `fts3_tokenizer()`
    /// - `SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE`: `false` (the default) to enable
    ///   checkpoints-on-close, `true` to disable them
    /// - `SQLITE_DBCONFIG_ENABLE_QPSG`: `false` to disable the QPSG, `true` to
    ///   enable QPSG
    /// - `SQLITE_DBCONFIG_TRIGGER_EQP`: `false` to disable output for trigger
    ///   programs, `true` to enable it
    #[inline]
    pub fn set_db_config(&self, config: DbConfig, new_val: bool) -> Result<bool> {
        let c = self.db.borrow_mut();
        unsafe {
            let mut val = 0;
            check(ffi::sqlite3_db_config(
                c.db(),
                config as c_int,
                if new_val { 1 } else { 0 },
                &mut val,
            ))?;
            Ok(val != 0)
        }
    }
}

#[cfg(test)]
mod test {
    use super::DbConfig;
    use crate::{Connection, Result};

    #[test]
    #[cfg(feature = "bundled")]
    fn test_dbconfig_values() {
        macro_rules! check_all {
            ($(($RustName:ident, $SQLITE_NAME:ident)),+ $(,)?) => {{
                // Hack to force a compile failure if we add a DbConfig variant
                // without updating this test.
                const _: fn(DbConfig) = |r| match r { $(DbConfig::$RustName => {}),+ };
                $({
                    assert_eq!(DbConfig::$RustName as i32, crate::ffi::$SQLITE_NAME as i32);
                    assert_eq!(
                        DbConfig::$SQLITE_NAME as i32,
                        crate::ffi::$SQLITE_NAME as i32,
                    );
                })+
            }};
        }
        check_all![
            (ForeignKeysEnabled, SQLITE_DBCONFIG_ENABLE_FKEY),
            (TriggersEnabled, SQLITE_DBCONFIG_ENABLE_TRIGGER),
            (Fts3TokenizerEnabled, SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER),
            (NoCheckpointOnClose, SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE),
            (QueryPlannerStabilityEnabled, SQLITE_DBCONFIG_ENABLE_QPSG),
            (TriggersInExplainQueryPlan, SQLITE_DBCONFIG_TRIGGER_EQP),
            (ResetDatabase, SQLITE_DBCONFIG_RESET_DATABASE),
            (Defensive, SQLITE_DBCONFIG_DEFENSIVE),
            (WritableSchema, SQLITE_DBCONFIG_WRITABLE_SCHEMA),
            (LegacyAlterTable, SQLITE_DBCONFIG_LEGACY_ALTER_TABLE),
            (DoubleQuotedStringInDML, SQLITE_DBCONFIG_DQS_DML),
            (DoubleQuotedStringInDDL, SQLITE_DBCONFIG_DQS_DDL),
            (EnableViews, SQLITE_DBCONFIG_ENABLE_VIEW),
            (LegacyFileFormat, SQLITE_DBCONFIG_LEGACY_FILE_FORMAT),
            (TrustedSchema, SQLITE_DBCONFIG_TRUSTED_SCHEMA),
        ];
    }

    #[test]
    fn test_db_config() -> Result<()> {
        let db = Connection::open_in_memory()?;

        let opposite = !db.db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY)?;
        assert_eq!(
            db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, opposite),
            Ok(opposite)
        );
        assert_eq!(
            db.db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY),
            Ok(opposite)
        );

        let opposite = !db.db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)?;
        assert_eq!(
            db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, opposite),
            Ok(opposite)
        );
        assert_eq!(
            db.db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER),
            Ok(opposite)
        );
        Ok(())
    }
}
