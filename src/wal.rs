//! Write-Ahead Logging

use std::ffi::c_int;
use std::ptr;

use crate::error::decode_result_raw;
use crate::ffi;
use crate::inner_connection::InnerConnection;
use crate::{Connection, Name, Result};

/// Checkpoint mode
#[derive(Clone, Copy, Default)]
#[repr(i32)]
#[non_exhaustive]
pub enum CheckpointMode {
    /// Do as much as possible w/o blocking
    #[default]
    PASSIVE = ffi::SQLITE_CHECKPOINT_PASSIVE,
    /// Wait for writers, then checkpoint
    FULL = ffi::SQLITE_CHECKPOINT_FULL,
    /// Like FULL but wait for readers
    RESTART = ffi::SQLITE_CHECKPOINT_RESTART,
    /// Like RESTART but also truncate WAL
    TRUNCATE = ffi::SQLITE_CHECKPOINT_TRUNCATE,
    /// Do no work at all
    #[cfg(feature = "modern_sqlite")] // 3.51.0
    NOOP = -1, //ffi::SQLITE_CHECKPOINT_NOOP,
}

impl Connection {
    /// Runs a checkpoint operation on database `db_name` in the specified `mode`. Status information is returned back into integers:
    /// - the total number of frames in the log file
    ///   or to -1 if the checkpoint could not run because of an error or because the database is not in WAL mode.
    /// - the total number of checkpointed frames in the log file (including any that were already checkpointed before the function was called)
    ///   or to -1 if the checkpoint could not run due to an error or because the database is not in WAL mode.
    ///
    /// If parameter `db_name` is `None`, then the specified operation is attempted on all WAL databases attached to database connection.
    pub fn wal_checkpoint_v2<N: Name>(
        &self,
        db_name: Option<N>,
        mode: CheckpointMode,
    ) -> Result<(c_int, c_int)> {
        self.db.borrow_mut().wal_checkpoint_v2(db_name, mode)
    }
}

impl InnerConnection {
    pub(crate) fn wal_checkpoint_v2<N: Name>(
        &mut self,
        db_name: Option<N>,
        mode: CheckpointMode,
    ) -> Result<(c_int, c_int)> {
        let cs = db_name.as_ref().map(N::as_cstr).transpose()?;
        let db_name = cs.as_ref().map_or(ptr::null(), |s| s.as_ptr());
        let mut n_log = 0;
        let mut n_ckpt = 0;
        unsafe {
            decode_result_raw(
                self.db,
                ffi::sqlite3_wal_checkpoint_v2(
                    self.db,
                    db_name,
                    mode as c_int,
                    &raw mut n_log,
                    &raw mut n_ckpt,
                ),
            )?;
        };
        Ok((n_log, n_ckpt))
    }
}

#[cfg(all(test, not(miri)))]
mod test {
    use super::CheckpointMode;
    use crate::{Connection, DEFAULT_NAME, Result};
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[cfg_attr(
        all(target_family = "wasm", target_os = "unknown"),
        ignore = "no filesystem on this platform"
    )]
    #[test]
    fn wal_checkpoint_v2() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("wal.db3");
        let db = Connection::open(&path)?;
        let (n_log, n_ckpt) = db.wal_checkpoint_v2(DEFAULT_NAME, CheckpointMode::default())?;
        assert_eq!(n_log, -1);
        assert_eq!(n_ckpt, -1);
        db.pragma_update(None, "journal_mode", "wal")?;
        let (n_log, n_ckpt) = db.wal_checkpoint_v2(DEFAULT_NAME, CheckpointMode::default())?;
        assert_eq!(n_log, 0);
        assert_eq!(n_ckpt, 0);
        Ok(())
    }
}
