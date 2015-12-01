extern crate pkg_config;

use std::env;

fn main() {
    let target = env::var("TARGET").unwrap();

    if target.contains("darwin") {
        println!("cargo:rustc-link-lib=sqlite3");
        println!("cargo:rustc-link-search=/usr/lib");
    } else {
        pkg_config::find_library("sqlite3").unwrap();
    }
}
