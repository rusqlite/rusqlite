extern crate rusqlite;
use rusqlite::{Connection, Result};

struct Item {
    name: String,
    builtin: i32,
    type_: String,
    enc: String,
    narg: i32,
    flags: i32,
}

fn main() -> Result<()> {
    let conn = Connection::open_in_memory()?;

    let query = "SELECT name, builtin, type, enc, narg, flags
                 FROM pragma_function_list() ORDER BY name, narg";

    let mut stmt = conn.prepare(&query)?;

    println!(
        "\nSQLite function list:\n\n\
        {name:<25}  builtin  type    enc\t   narg\t   flags",
        name = "name"
    );

    let rows = stmt.query_map([], |row| {
        Ok(Item {
            name: row.get(0)?,
            builtin: row.get(1)?,
            type_: row.get(2)?,
            enc: row.get(3)?,
            narg: row.get(4)?,
            flags: row.get(5)?,
        })
    })?;

    for item in rows {
        match item {
            Ok(i) => println!(
                "{name:<25} |   {builtin}   |   {type_}\t  | {enc} |{narg:>3}\t| {flags:>7}",
                name = i.name,
                builtin = i.builtin,
                type_ = i.type_,
                enc = i.enc,
                narg = i.narg,
                flags = i.flags
            ),
            Err(e) => eprintln!("Error: {e:?}"),
        }
    }

    Ok(())
}
