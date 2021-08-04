// wasm-pack test --node --features bundled

use rusqlite::{params, Connection};

#[derive(Debug)]
struct Person {
    #[allow(dead_code)]
    id: i32,
    name: String,
    data: Option<Vec<u8>>,
}

#[wasm_bindgen_test::wasm_bindgen_test]
fn node_test() {
    let conn = Connection::open_in_memory().unwrap();

    conn.execute(
        "CREATE TABLE person (
                  id              INTEGER PRIMARY KEY,
                  name            TEXT NOT NULL,
                  data            BLOB
                  )",
        [],
    )
    .unwrap();
    let me = Person {
        id: 0,
        name: "Steven".to_string(),
        data: None,
    };
    conn.execute(
        "INSERT INTO person (name, data) VALUES (?1, ?2)",
        params![me.name, me.data],
    )
    .unwrap();

    let person = conn
        .query_row("SELECT id, name, data FROM person", [], |r| {
            Ok(Person {
                id: r.get(0).unwrap(),
                name: r.get(1).unwrap(),
                data: r.get(2).unwrap(),
            })
        })
        .unwrap();

    assert_eq!(
        format!("{:?}", person),
        r#"Person { id: 1, name: "Steven", data: None }"#
    );

    let _random: i64 = conn.query_row("SELECT random()", [], |r| r.get(0)).unwrap();

    let current_year: i32 = conn
        .query_row("SELECT cast(strftime('%Y') AS decimal)", [], |r| r.get(0))
        .unwrap();
    assert!((2022..2050).contains(&current_year));
}
