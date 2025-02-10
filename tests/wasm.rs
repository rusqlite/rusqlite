#[cfg(all(target_family = "wasm", target_os = "unknown"))]
mod wasm {
    // Running wasm tests on dedicated_worker
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

    use wasm_bindgen_test::{console_log, wasm_bindgen_test};

    use rusqlite::{ffi::install_opfs_sahpool, Connection};

    #[derive(Debug)]
    struct Person {
        #[allow(unused)]
        id: i32,
        name: String,
        data: Option<Vec<u8>>,
    }

    #[wasm_bindgen_test]
    #[allow(unused)]
    fn test_memory() {
        let conn = Connection::open("mem.db").unwrap();

        conn.execute(
            "CREATE TABLE person (
            id    INTEGER PRIMARY KEY,
            name  TEXT NOT NULL,
            data  BLOB
        )",
            (), // empty list of parameters.
        )
        .unwrap();
        let me = Person {
            id: 0,
            name: "Steven".to_string(),
            data: None,
        };
        conn.execute(
            "INSERT INTO person (name, data) VALUES (?1, ?2)",
            (&me.name, &me.data),
        )
        .unwrap();

        let mut stmt = conn.prepare("SELECT id, name, data FROM person").unwrap();
        let person_iter = stmt
            .query_map([], |row| {
                Ok(Person {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    data: row.get(2)?,
                })
            })
            .unwrap();

        for person in person_iter {
            console_log!("Found person {:?}", person.unwrap());
        }
    }

    #[wasm_bindgen_test]
    #[allow(unused)]
    async fn test_persistence_vfs() {
        install_opfs_sahpool(None, false).await.unwrap();
        let conn = Connection::open("file:persistence.db?vfs=opfs-sahpool").unwrap();

        if conn.execute("DROP table person;", ()).is_ok() {
            console_log!("opfs-sahpool: table exist");
        }
        conn.execute(
            "CREATE TABLE person (
            id    INTEGER PRIMARY KEY,
            name  TEXT NOT NULL,
            data  BLOB
        )",
            (), // empty list of parameters.
        )
        .unwrap();
        let me = Person {
            id: 0,
            name: "Steven".to_string(),
            data: None,
        };
        conn.execute(
            "INSERT INTO person (name, data) VALUES (?1, ?2)",
            (&me.name, &me.data),
        )
        .unwrap();

        let mut stmt = conn.prepare("SELECT id, name, data FROM person").unwrap();
        let person_iter = stmt
            .query_map([], |row| {
                Ok(Person {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    data: row.get(2)?,
                })
            })
            .unwrap();

        for person in person_iter {
            console_log!("Found person {:?}", person.unwrap());
        }
    }
}
