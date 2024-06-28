extern crate rusqlite;
use rusqlite::{Connection, Result};

struct Item {
    name: String,
}

fn main() -> Result<()> {
    let conn = Connection::open_in_memory()?;

    let query = "SELECT name FROM pragma_pragma_list() ORDER BY name";

    let mut stmt = conn.prepare(&query)?;

    println!("\nSQLite pragma names:");

    let rows = stmt.query_map([], |row| Ok(Item { name: row.get(0)? }))?;

    for item in rows {
        match item {
            Ok(i) => println!("    {}", i.name),
            Err(e) => eprintln!("Error: {e:?}"),
        }
    }

    Ok(())
}
