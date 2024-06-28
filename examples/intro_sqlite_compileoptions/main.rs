extern crate rusqlite;
use rusqlite::{Connection, Result};

struct Item {
    name: String,
}

fn main() -> Result<()> {
    let conn = Connection::open_in_memory()?;

    let query = "SELECT compile_options
                 FROM pragma_compile_options()
                 ORDER BY compile_options";

    let mut stmt = conn.prepare(&query)?;

    println!("\nSQLite compile options:");

    let rows = stmt.query_map([], |row| Ok(Item { name: row.get(0)? }))?;

    for item in rows {
        match item {
            Ok(i) => println!("    {}", i.name),
            Err(e) => eprintln!("Error: {e:?}"),
        }
    }

    Ok(())
}
