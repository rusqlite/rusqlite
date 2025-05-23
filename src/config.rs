//! Configure database connections

use std::ffi::c_int;

use crate::error::check;
use crate::ffi;
use crate::{Connection, Result};

/// Database Connection Configuration Options
/// See [Database Connection Configuration Options](https://sqlite.org/c3ref/c_dbconfig_enable_fkey.html) for details.
#[repr(i32)]
#[derive(Copy, Clone, Debug)]
#[expect(non_camel_case_types)]
#[non_exhaustive]
pub enum DbConfig {
    //SQLITE_DBCONFIG_MAINDBNAME = 1000, /* const char* */
    //SQLITE_DBCONFIG_LOOKASIDE = 1001,  /* void* int int */
    /// Enable or disable the enforcement of foreign key constraints.
    SQLITE_DBCONFIG_ENABLE_FKEY = ffi::SQLITE_DBCONFIG_ENABLE_FKEY,
    /// Enable or disable triggers.
    SQLITE_DBCONFIG_ENABLE_TRIGGER = ffi::SQLITE_DBCONFIG_ENABLE_TRIGGER,
    /// Enable or disable the `fts3_tokenizer()` function which is part of the
    /// FTS3 full-text search engine extension.
    SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER = ffi::SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER, // 3.12.0
    //SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION = 1005,
    /// In WAL mode, enable or disable the checkpoint operation before closing
    /// the connection.
    SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE = 1006, // 3.16.2
    /// Activates or deactivates the query planner stability guarantee (QPSG).
    SQLITE_DBCONFIG_ENABLE_QPSG = 1007, // 3.20.0
    /// Includes or excludes output for any operations performed by trigger
    /// programs from the output of EXPLAIN QUERY PLAN commands.
    SQLITE_DBCONFIG_TRIGGER_EQP = 1008, // 3.22.0
    /// Activates or deactivates the "reset" flag for a database connection.
    /// Run VACUUM with this flag set to reset the database.
    SQLITE_DBCONFIG_RESET_DATABASE = 1009, // 3.24.0
    /// Activates or deactivates the "defensive" flag for a database connection.
    SQLITE_DBCONFIG_DEFENSIVE = 1010, // 3.26.0
    /// Activates or deactivates the `writable_schema` flag.
    #[cfg(feature = "modern_sqlite")]
    SQLITE_DBCONFIG_WRITABLE_SCHEMA = 1011, // 3.28.0
    /// Activates or deactivates the legacy behavior of the ALTER TABLE RENAME
    /// command.
    #[cfg(feature = "modern_sqlite")]
    SQLITE_DBCONFIG_LEGACY_ALTER_TABLE = 1012, // 3.29
    /// Activates or deactivates the legacy double-quoted string literal
    /// misfeature for DML statements only.
    #[cfg(feature = "modern_sqlite")]
    SQLITE_DBCONFIG_DQS_DML = 1013, // 3.29.0
    /// Activates or deactivates the legacy double-quoted string literal
    /// misfeature for DDL statements.
    #[cfg(feature = "modern_sqlite")]
    SQLITE_DBCONFIG_DQS_DDL = 1014, // 3.29.0
    /// Enable or disable views.
    #[cfg(feature = "modern_sqlite")]
    SQLITE_DBCONFIG_ENABLE_VIEW = 1015, // 3.30.0
    /// Activates or deactivates the legacy file format flag.
    #[cfg(feature = "modern_sqlite")]
    SQLITE_DBCONFIG_LEGACY_FILE_FORMAT = 1016, // 3.31.0
    /// Tells SQLite to assume that database schemas (the contents of the
    /// `sqlite_master` tables) are untainted by malicious content.
    #[cfg(feature = "modern_sqlite")]
    SQLITE_DBCONFIG_TRUSTED_SCHEMA = 1017, // 3.31.0
    /// Sets or clears a flag that enables collection of the
    /// `sqlite3_stmt_scanstatus_v2()` statistics
    #[cfg(feature = "modern_sqlite")]
    SQLITE_DBCONFIG_STMT_SCANSTATUS = 1018, // 3.42.0
    /// Changes the default order in which tables and indexes are scanned
    #[cfg(feature = "modern_sqlite")]
    SQLITE_DBCONFIG_REVERSE_SCANORDER = 1019, // 3.42.0
    /// Enables or disables the ability of the ATTACH DATABASE SQL command
    /// to create a new database file if the database filed named in the ATTACH command does not already exist.
    #[cfg(feature = "modern_sqlite")]
    SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE = 1020, // 3.49.0
    /// Enables or disables the ability of the ATTACH DATABASE SQL command to open a database for writing.
    #[cfg(feature = "modern_sqlite")]
    SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE = 1021, // 3.49.0
    /// Enables or disables the ability to include comments in SQL text.
    #[cfg(feature = "modern_sqlite")]
    SQLITE_DBCONFIG_ENABLE_COMMENTS = 1022, // 3.49.0
}

impl Connection {
    /// Returns the current value of a `config`.
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

    /// Make configuration changes to a database connection
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
                new_val as c_int,
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
