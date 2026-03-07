use rusqlite::{Connection, Result};
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
use wasm_bindgen::prelude::wasm_bindgen;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
macro_rules! println {
    ($($t:tt)*) => (log(&format_args!($($t)*).to_string()))
}

struct Person {
    id: i32,
    name: String,
}

#[cfg_attr(all(target_family = "wasm", target_os = "unknown"), wasm_bindgen(main))]
fn main() -> Result<()> {
    let conn = Connection::open_in_memory()?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS persons (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL
        )",
        (), // empty list of parameters.
    )?;

    conn.execute(
        "INSERT INTO persons (name) VALUES (?1), (?2), (?3)",
        ["Steven", "John", "Alex"].map(|n| n.to_string()),
    )?;

    let mut stmt = conn.prepare("SELECT id, name FROM persons")?;
    let rows = stmt.query_map([], |row| {
        Ok(Person {
            id: row.get(0)?,
            name: row.get(1)?,
        })
    })?;

    println!("Found persons:");

    for person in rows {
        match person {
            Ok(p) => println!("ID: {}, Name: {}", p.id, p.name),
            Err(e) => eprintln!("Error: {e:?}"),
        }
    }

    Ok(())
}
