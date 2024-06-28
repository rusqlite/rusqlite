extern crate rusqlite;
use rusqlite::{version, Result};

fn main() -> Result<()> {
    let libversion = version();

    println!("SQLite libversion:");
    println!("Version: {libversion}", libversion = libversion);

    Ok(())
}
