use rusqlite_macros::__bind;

type Result = std::result::Result<(), String>;

struct Stmt;

impl Stmt {
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
    __bind!(stmt "SELECT $first_name, {last_name}");
    Ok(())
}

#[test]
fn test_tuple() -> Result {
    let names = ("El", "Barto");
    let mut stmt = Stmt;
    __bind!(stmt "SELECT {names.0}, {names.1}");
    Ok(())
}

#[test]
fn test_struct() -> Result {
    struct Person<'s> {
        first_name: &'s str,
        last_name: &'s str,
    }
    let p = Person {
        first_name: "El",
        last_name: "Barto",
    };
    let mut stmt = Stmt;
    __bind!(stmt "SELECT {p.first_name}, {p.last_name}");
    Ok(())
}

/* FIXME
#[test]
fn test_raw_string() {
    let stmt = ();
    __bind!(stmt r#"SELECT 1"#);
}

#[test]
fn test_const() {
    const SQL: &str = "SELECT 1";
    let stmt = ();
    __bind!(stmt SQL);
}
*/
