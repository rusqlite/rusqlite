extern crate rusqlite;
use rusqlite::{Connection, Result};

struct Item {
    application_id: i64,
    user_version: i64,
    schema_version: i64,
}

fn main() -> Result<()> {
    let conn = Connection::open_in_memory()?;

    let query = "SELECT application_id, user_version, schema_version
                 FROM pragma_application_id(), pragma_user_version(),
                 pragma_schema_version()";

    let mut stmt = conn.prepare(&query)?;

    println!("\nSQLite ID numbers for a new database (should be all zeroes):");

    let rows = stmt.query_map([], |row| {
        Ok(Item {
            application_id: row.get(0)?,
            user_version: row.get(1)?,
            schema_version: row.get(2)?,
        })
    })?;

    for item in rows {
        match item {
            Ok(i) => println!(
                "application_id: {}\nuser_version: {}\nschema_version: {}\n",
                i.application_id, i.user_version, i.schema_version
            ),
            Err(e) => eprintln!("Error: {e:?}"),
        }
    }

    // Update IDs and verify that the new returned values match.
    // application_id and user_version are set via PRAGMAs;
    // schema_version is changed by changing the schema.

    let pragmas = "PRAGMA application_id = 1;\n\
                   PRAGMA user_version   = 1;\n\
                   CREATE TABLE foo(x INTEGER)";

    conn.execute_batch(&pragmas)?;

    println!("\nSQLite ID numbers after update (should be all ones):");

    let rows = stmt.query_map([], |row| {
        Ok(Item {
            application_id: row.get(0)?,
            user_version: row.get(1)?,
            schema_version: row.get(2)?,
        })
    })?;

    for item in rows {
        match item {
            Ok(i) => println!(
                "application_id: {}\nuser_version: {}\nschema_version: {}\n",
                i.application_id, i.user_version, i.schema_version
            ),
            Err(e) => eprintln!("Error: {e:?}"),
        }
    }

    Ok(())
}
