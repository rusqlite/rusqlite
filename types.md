# Rust vs SQLite types

## From SQLite to Rust

### From SQLite statement

- `sqlite3_column_type`: `SQLITE_NULL` | `SQLITE_INTEGER` | `SQLITE_FLOAT` | `SQLITE_TEXT` | `SQLITE_BLOB`
- `sqlite3_column_int64`
- `sqlite3_column_double`
- `sqlite3_column_bytes` / `sqlite3_column_text` / `sqlite3_column_blob`

```
sqlite3_column_* => ValueRef => FromSql => * Rust
```

### From SQLite functions / virtual tables

- `sqlite3_value`
- `sqlite3_value_type`: `SQLITE_NULL` | `SQLITE_INTEGER` | `SQLITE_FLOAT` | `SQLITE_TEXT` | `SQLITE_BLOB`
- `sqlite3_value_int64`
- `sqlite3_value_double`
- `sqlite3_value_bytes` / `sqlite3_value_text` / `sqlite3_value_blob`
- `sqlite3_value_pointer`

```
sqlite3_value => ValueRef => FromSql => * Rust
```

### From Rust to SQLite

## For SQLite statements

- `sqlite3_bind_null`
- `sqlite3_bind_int64`
- `sqlite3_bind_double`
- `sqlite3_bind_text64`
- `sqlite3_bind_blob64` / `sqlite3_bind_zeroblob64`
- `sqlite3_bind_pointer`

```
* Rust => ToSql => ToSqlOutput => sqlite3_bind_*
```

### For SQLite functions / virtual tables

- `sqlite3_result_null`
- `sqlite3_result_int64`
- `sqlite3_result_double`
- `sqlite3_result_text64`
- `sqlite3_result_blob64` / `sqlite3_result_zeroblob64`
- `sqlite3_result_pointer`
- `sqlite3_result_value`

```
* Rust => ToSql => ToSqlOutput => sqlite3_result_*
```
