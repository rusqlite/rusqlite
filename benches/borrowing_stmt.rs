use bencher::{benchmark_group, benchmark_main, Bencher};
use rusqlite::{params, Connection};
use uuid::Uuid;

struct BenchData {
    name: String,
    uuid: Uuid,
    parent: Uuid,

    some_u64: u64,
    string_a: String,
    string_b: String,
    string_c: String,
}

impl BenchData {
    fn new() -> Self {
        BenchData {
            name: std::hint::black_box("test".to_string()),
            uuid: Uuid::new_v4(),
            parent: Uuid::new_v4(),
            some_u64: 42,
            string_a: std::hint::black_box("string_a".to_string()),
            string_b: std::hint::black_box("string_b".to_string()),
            string_c: std::hint::black_box("string_c".to_string()),
        }
    }
}

// efficiently build a SQL string for batch inserting values, e.g. "INSERT INTO table (col1, col2) VALUES (?,?),(?,?),(?,?)"
fn build_batch_values_sql(
    prefix: &str,
    cols_per_row: usize,
    n_rows: usize,
    suffix: &str,
) -> String {
    let per_row = 2 + 2 * cols_per_row; // "(?,?,...)"
    let mut sql = String::with_capacity(prefix.len() + 8 + n_rows * (per_row + 1) + suffix.len());

    let capacity = sql.capacity();

    sql.push_str(prefix);
    sql.push_str(" VALUES ");
    for i in 0..n_rows {
        if i > 0 {
            sql.push(',');
        }
        sql.push('(');
        for j in 0..cols_per_row {
            if j > 0 {
                sql.push(',');
            }
            sql.push('?');
        }
        sql.push(')');
    }
    sql.push(' ');
    sql.push_str(suffix);

    debug_assert_eq!(capacity, sql.capacity());
    debug_assert_eq!(capacity, sql.len());
    sql
}

const TRANSACTION_SIZE: usize = 10_000;
const BATCH_VALUES: usize = 200;
const TOTAL_ITEMS: usize = 1_000_000;
const COLS_PER_ROW: usize = 7;

fn create_data() -> Vec<BenchData> {
    (0..TOTAL_ITEMS).map(|_| BenchData::new()).collect()
}

fn init_db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE bench_data (
			name TEXT,
			uuid BLOB,
			parent BLOB,
			some_u64 INTEGER,
			string_a TEXT,
			string_b TEXT,
			string_c TEXT
		)",
    )
    .unwrap();
    db
}

fn make_statement_sql() -> String {
    build_batch_values_sql(
        "INSERT INTO bench_data (name, uuid, parent, some_u64, string_a, string_b, string_c)",
        COLS_PER_ROW,
        BATCH_VALUES,
        "",
    )
}

fn bench_borrowing(b: &mut Bencher) {
    let db = init_db();

    let data = create_data();

    b.iter(|| {
        let mut stmt = db.prepare_borrowing(&make_statement_sql()).unwrap();
        for transaction_chunk in data.chunks(TRANSACTION_SIZE) {
            let transaction = db.unchecked_transaction().unwrap();
            for insert_chunk in transaction_chunk.chunks(BATCH_VALUES) {
                for (i, row) in insert_chunk.iter().enumerate() {
                    let i = i * COLS_PER_ROW + 1;
                    stmt.raw_bind_parameter_ref(i, row.name.as_str()).unwrap();
                    stmt.raw_bind_parameter_ref(i + 1, row.uuid.as_bytes())
                        .unwrap();
                    stmt.raw_bind_parameter_ref(i + 2, row.parent.as_bytes())
                        .unwrap();
                    stmt.raw_bind_parameter_ref(i + 3, row.some_u64 as i64)
                        .unwrap();
                    stmt.raw_bind_parameter_ref(i + 4, row.string_a.as_str())
                        .unwrap();
                    stmt.raw_bind_parameter_ref(i + 5, row.string_b.as_str())
                        .unwrap();
                    stmt.raw_bind_parameter_ref(i + 6, row.string_c.as_str())
                        .unwrap();
                }
                stmt.raw_execute().unwrap();
            }
            transaction.commit().unwrap();
        }
    });
}

fn bench_borrowing_one_transaction(b: &mut Bencher) {
    let mut db = init_db();

    let data = create_data();

    b.iter(|| {
        let transaction = db.transaction().unwrap();
        let mut stmt = transaction
            .prepare_borrowing(&make_statement_sql())
            .unwrap();
        for insert_chunk in data.chunks(BATCH_VALUES) {
            for (i, row) in insert_chunk.iter().enumerate() {
                let i = i * COLS_PER_ROW + 1;
                stmt.raw_bind_parameter_ref(i, row.name.as_str()).unwrap();
                stmt.raw_bind_parameter_ref(i + 1, row.uuid.as_bytes())
                    .unwrap();
                stmt.raw_bind_parameter_ref(i + 2, row.parent.as_bytes())
                    .unwrap();
                stmt.raw_bind_parameter_ref(i + 3, row.some_u64 as i64)
                    .unwrap();
                stmt.raw_bind_parameter_ref(i + 4, row.string_a.as_str())
                    .unwrap();
                stmt.raw_bind_parameter_ref(i + 5, row.string_b.as_str())
                    .unwrap();
                stmt.raw_bind_parameter_ref(i + 6, row.string_c.as_str())
                    .unwrap();
            }
            stmt.raw_execute().unwrap();
        }
        drop(stmt);
        transaction.commit().unwrap();
    });
}

fn bench_copy(b: &mut Bencher) {
    let db = init_db();

    let data = create_data();

    b.iter(|| {
        let mut stmt = db.prepare(&make_statement_sql()).unwrap();
        for transaction_chunk in data.chunks(TRANSACTION_SIZE) {
            let transaction = db.unchecked_transaction().unwrap();
            for insert_chunk in transaction_chunk.chunks(BATCH_VALUES) {
                for (i, row) in insert_chunk.iter().enumerate() {
                    let i = i * COLS_PER_ROW + 1;
                    stmt.raw_bind_parameter(i, row.name.as_str()).unwrap();
                    stmt.raw_bind_parameter(i + 1, row.uuid.as_bytes()).unwrap();
                    stmt.raw_bind_parameter(i + 2, row.parent.as_bytes())
                        .unwrap();
                    stmt.raw_bind_parameter(i + 3, row.some_u64 as i64).unwrap();
                    stmt.raw_bind_parameter(i + 4, row.string_a.as_str())
                        .unwrap();
                    stmt.raw_bind_parameter(i + 5, row.string_b.as_str())
                        .unwrap();
                    stmt.raw_bind_parameter(i + 6, row.string_c.as_str())
                        .unwrap();
                }
                stmt.raw_execute().unwrap();
            }
            transaction.commit().unwrap();
        }
    });
}

fn bench_single_rows(b: &mut Bencher) {
    let db = init_db();

    let data = create_data();

    b.iter(|| {
        let mut stmt = db.prepare("INSERT INTO bench_data (name, uuid, parent, some_u64, string_a, string_b, string_c) VALUES (?, ?, ?, ?, ?, ?, ?)").unwrap();
        for transaction_chunk in data.chunks(TRANSACTION_SIZE) {
            let transaction = db.unchecked_transaction().unwrap();
            for row in transaction_chunk {
                      stmt.execute(params![&row.name, &row.uuid.as_bytes(), &row.parent.as_bytes(), row.some_u64 as i64, &row.string_a, &row.string_b, &row.string_c]).unwrap();       }
            transaction.commit().unwrap();
        }
    });
}

fn bench_single_rows_one_transaction(b: &mut Bencher) {
    let mut db = init_db();

    let data = create_data();

    b.iter(|| {
		let transaction = db.transaction().unwrap();
		{
			let mut stmt = transaction
				.prepare("INSERT INTO bench_data (name, uuid, parent, some_u64, string_a, string_b, string_c) VALUES (?, ?, ?, ?, ?, ?, ?)")
				.unwrap();
			for row in &data {
				stmt.execute(params![
					&row.name,
					&row.uuid.as_bytes(),
					&row.parent.as_bytes(),
					row.some_u64 as i64,
					&row.string_a,
					&row.string_b,
					&row.string_c
				])
				.unwrap();
			}
		}

		transaction.commit().unwrap();
	});
}

fn bench_single_rows_no_transaction(b: &mut Bencher) {
    let db = init_db();

    let data = create_data();

    b.iter(|| {
			let mut stmt = db
				.prepare("INSERT INTO bench_data (name, uuid, parent, some_u64, string_a, string_b, string_c) VALUES (?, ?, ?, ?, ?, ?, ?)")
				.unwrap();
			for row in &data {
				stmt.execute(params![
					&row.name,
					&row.uuid.as_bytes(),
					&row.parent.as_bytes(),
					row.some_u64 as i64,
					&row.string_a,
					&row.string_b,
					&row.string_c
				])
				.unwrap();
			}
	});
}

benchmark_group!(
    borrowing_stmt_benches,
    bench_borrowing,
    bench_borrowing_one_transaction,
    bench_copy,
    bench_single_rows,
    bench_single_rows_one_transaction,
    bench_single_rows_no_transaction
);
benchmark_main!(borrowing_stmt_benches);
