extern crate rusqlite;
use rusqlite::{version_number, Result};

fn main() -> Result<()> {
    let libver_num = version_number();

    println!("SQLite libversion number:");
    println!("Version: {libver_num}", libver_num = libver_num);

    Ok(())
}
