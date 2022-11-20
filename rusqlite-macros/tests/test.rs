use rusqlite_macros::__bind;

#[test]
fn test_literal() {
    let stmt = ();
    __bind!(stmt, "SELECT $name");
}

/* FIXME
#[test]
fn test_raw_string() {
    let stmt = ();
    __bind!((), r#"SELECT 1"#);
}

#[test]
fn test_const() {
    const SQL: &str = "SELECT 1";
    let stmt = ();
    __bind!((), SQL);
}
*/