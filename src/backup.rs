//! Online backup API
use std::path::{Path};

use super::ffi;
use {SqliteError, SqliteResult, SqliteConnection};

impl SqliteConnection {
    /// Open or create a database file named `dest_path`.  Transfer the
    /// content of local database `name` into the `dest_path` database.
    pub fn backup<P: AsRef<Path>>(&self, name: &str, dest_path: &P) -> SqliteResult<()> {
        let dest = try!(SqliteConnection::open(dest_path)); // "cannot open target database: "

        let mut dest_db = dest.db.borrow_mut();
        let dest_name = try!(super::str_to_cstring("main"));
        let src_db = self.db.borrow_mut();
        let src_name = try!(super::str_to_cstring(name));
        let backup = unsafe { ffi::sqlite3_backup_init(dest_db.db(), dest_name.as_ptr(), src_db.db(), src_name.as_ptr()) };
        if backup.is_null() {
            let msg = unsafe { super::errmsg_to_string(ffi::sqlite3_errmsg(dest_db.db())) };
            let err = Err(SqliteError{code: unsafe { ffi::sqlite3_errcode(dest_db.db()) }, message: "backup failed: ".to_string() + &msg });
            let _ = dest_db.close();
            return err;
        }

        let mut rc;
        loop {
            rc = unsafe { ffi::sqlite3_backup_step(backup, 100) };
            if rc != ffi::SQLITE_OK {
                break;
            }
        }
        unsafe { ffi::sqlite3_backup_finish(backup) };
        let result = if rc == ffi::SQLITE_DONE {
            Ok(())
        } else {
            let msg = unsafe { super::errmsg_to_string(ffi::sqlite3_errmsg(dest_db.db())) };
            Err(SqliteError{code: rc, message: "backup failed: ".to_string() + &msg })
        };
        let _ = dest_db.close();
        result
    }

    /// Open a database file named `src_path`.  Transfer the content
    /// of `src_path` into the local database `name`.
    pub fn restore<P: AsRef<Path>>(&self, name: &str, src_path: &P) -> SqliteResult<()> {
        let src = try!(SqliteConnection::open_with_flags(src_path, super::SQLITE_OPEN_READ_ONLY | super::SQLITE_OPEN_URI)); // "cannot open source database: "

        let dest_db = self.db.borrow_mut();
        let dest_name = try!(super::str_to_cstring(name));
        let mut src_db = src.db.borrow_mut();
        let src_name = try!(super::str_to_cstring("main"));
        let backup = unsafe { ffi::sqlite3_backup_init(dest_db.db(), dest_name.as_ptr(), src_db.db(), src_name.as_ptr()) };
        if backup.is_null() {
            let msg = unsafe { super::errmsg_to_string(ffi::sqlite3_errmsg(dest_db.db())) };
            let err = Err(SqliteError{code: unsafe { ffi::sqlite3_errcode(dest_db.db()) }, message: "restore failed: ".to_string() + &msg });
            let _ = src_db.close();
            return err;
        }
        let mut rc;
        let mut n_timeout = 0;
        loop {
            rc = unsafe { ffi::sqlite3_backup_step(backup, 100) };
            if rc == ffi::SQLITE_OK {
            } else if rc == ffi::SQLITE_BUSY {
                n_timeout += 1;
                if n_timeout >= 3 {
                    break;
                }
                unsafe { ffi::sqlite3_sleep(100) };
            } else {
                break;
            }
        }
        unsafe { ffi::sqlite3_backup_finish(backup) };
        let result = if rc == ffi::SQLITE_DONE {
            Ok(())
        } else if rc == ffi::SQLITE_BUSY || rc == ffi::SQLITE_LOCKED {
            Err(SqliteError{code: rc, message: "restore failed: source database busy".to_string() })
        } else {
            let msg = unsafe { super::errmsg_to_string(ffi::sqlite3_errmsg(dest_db.db())) };
            Err(SqliteError{code: rc, message: "restore failed: ".to_string() + &msg })
        };
        let _ = src_db.close();
        result
    }
}

#[cfg(test)]
mod test {
    use SqliteConnection;
    use {SQLITE_OPEN_URI, SQLITE_OPEN_CREATE, SQLITE_OPEN_READ_WRITE};

    #[test]
    fn test_backup() {
        let db = SqliteConnection::open_in_memory().unwrap();
        let sql = "BEGIN;
                CREATE TABLE foo(x INTEGER);
                INSERT INTO foo VALUES(42);
                END;";
        db.execute_batch(sql).unwrap();
        db.backup("main", &":memory:").unwrap();
        assert!(db.close().is_ok());
    }

    #[test]
    fn test_restore() {
        let src_path = "file:dummy.db?mode=memory&cache=shared";
        let src = SqliteConnection::open_with_flags(&src_path, SQLITE_OPEN_URI | SQLITE_OPEN_CREATE | SQLITE_OPEN_READ_WRITE).unwrap();
        let sql = "BEGIN;
               CREATE TABLE foo(x INTEGER);
               INSERT INTO foo VALUES(42);
               END;";
        src.execute_batch(sql).unwrap();

        let db = SqliteConnection::open_in_memory().unwrap();
        db.restore("main", &src_path).unwrap();

        let the_answer = db.query_row("SELECT x FROM foo",
                                           &[],
                                           |r| r.get::<i64>(0));
        assert_eq!(42i64, the_answer.unwrap());

        assert!(db.close().is_ok());
        assert!(src.close().is_ok());
    }
}
