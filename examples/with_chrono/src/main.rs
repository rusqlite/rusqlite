use std::io::stdin;

use rusqlite::NO_PARAMS;
use rusqlite::{params, Connection, Result};

// // https://docs.rs/time/0.2.9/time/struct.Instant.html
// use time::Instant;

// https://github.com/jgallagher/rusqlite#optional-features - chrono
// Refer to Cargo.toml
use chrono::naive::NaiveDateTime;

// 1. created_at: NaiveDateTime
#[derive(Debug)]
struct Message {
    query: String, // Query is unique so use it as id.
    used: i64, // See how many time it was used.
    created_at: NaiveDateTime,
}

// Message {
//     query: "rust",
//     used: 1,
//     created_at: 2020-03-24T11:01:34,
// }

// 2. created_at: String, // without Chrono

// Message {
//     query: "rust",
//     used: 1,
//     created_at: "2020-03-24 10:54:05",
// }

pub fn from_stdin() -> String {
    let mut input = String::new();
    stdin().read_line(&mut input).unwrap();
    let input = input[..(input.len() - 1)].to_string();

    input
}

// time::Instant::now
fn main() -> Result<()> {
    let conn = Connection::open("messages.db")?;

    conn.execute(
        "create table if not exists messages (
            query text not null unique,
            used integer default 1,
            created_at DATE DEFAULT (datetime('now','localtime'))
        )",
        NO_PARAMS,
    )?;

    loop {
        println!("What do you want?[c, r, u, d, l, e]"); // create, update(used +1), read, delete, list

        let action = from_stdin();
        match action.as_ref() {
            "c" => {
                // rust, golang
                println!("What do you want to save in messages?");
                let query = from_stdin();
                conn.execute("INSERT INTO messages (query) values (?1)", &[&query])?;
                println!("{:#?} is included in messages.", query)
            }
            "r" => {
                println!("Which query you want to read?");
                let query = from_stdin();

                let mut stmt = conn.prepare("SELECT * FROM messages WHERE query = (?1);")?;

                // You can use others instead of query_map.
                // https://docs.rs/rusqlite/0.21.0/rusqlite/struct.Statement.html#method.query
                let message = stmt.query_map(params![&query], |row| {
                    Ok(Message {
                        query: row.get(0)?,
                        used: row.get(1)?,
                        created_at: row.get(2)?,
                    })
                })?;

                for row in message {
                    println!("{:#?}", row?);
                }
            }
            "u" => {
                println!("What query you want to increment its used number?");
                let query = from_stdin();
                // https://stackoverflow.com/questions/744289/sqlite-increase-value-by-a-certain-number/744290
                // Find I can do the same with Postgresql.
                // https://www.sqlitetutorial.net/sqlite-update/
                conn.execute("UPDATE messages SET used = used + 1 WHERE query = (?1);", &[&query])?;
                println!("{:#?} is used one more time.", &query)
            }
            "d" => {
                println!("What query you want to delete?");
                let query = from_stdin();
                conn.execute("DELETE FROM messages WHERE query = (?1);", &[&query])?;
                println!("{:#?} is deleted from the messages", &query)
            }
            "l" => {
                let mut stmt = conn.prepare("SELECT * FROM messages;")?;

                let messages = stmt.query_map(NO_PARAMS, |row| {
                    Ok(Message {
                        query: row.get(0)?,
                        used: row.get(1)?,
                        created_at: row.get(2)?,
                    })
                })?;

                for message in messages {
                    println!("{:#?}", message?);
                }
            }
            "e" => {
                println!("You want to end the SQLite CLI example.");
                break
            }
            _ => {
                println!("You should use [c, r, u, d, l, e] to create, read, update, delete, list messages and e to end.");println!("You should use [c, r, u, l, e] to create, read, update, list messages and e to end.");
            }
        };
    }

    Ok(())
}
