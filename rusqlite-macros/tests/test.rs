use rusqlite_macros::__bind;

type Result = std::result::Result<(), String>;

struct Stmt;

impl Stmt {
    #[expect(clippy::unused_self)]
    pub fn raw_bind_parameter(&mut self, one_based_col_index: usize, param: &str) -> Result {
        let (..) = (one_based_col_index, param);
        Ok(())
    }
}

#[test]
fn test_literal() -> Result {
    let first_name = "El";
    let last_name = "Barto";
    let mut stmt = Stmt;
    __bind!(stmt "SELECT $first_name, $last_name");
    Ok(())
}

#[test]
fn test_no_placeholder() {
    #[expect(clippy::no_effect_underscore_binding)]
    let _stmt = Stmt;
    __bind!(_stmt "SELECT 1");
}

#[test]
fn test_raw_string() {
    #[expect(clippy::no_effect_underscore_binding)]
    let _stmt = Stmt;
    __bind!(_stmt r"SELECT 1");
    __bind!(_stmt r#"SELECT 1"#);
}

/* FIXME
#[test]
fn test_const() {
    const SQL: &str = "SELECT 1";
    let stmt = ();
    __bind!(stmt SQL);
}
*/
