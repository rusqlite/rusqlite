#!/bin/sh -e

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
echo "$SCRIPT_DIR"
cd "$SCRIPT_DIR" || { echo "fatal error" >&2; exit 1; }
cargo clean -p libsqlite3-sys
TARGET_DIR="$SCRIPT_DIR/../target"
export SQLITE3_LIB_DIR="$SCRIPT_DIR/sqlite3mc"
mkdir -p "$TARGET_DIR" "$SQLITE3_LIB_DIR"

# Download and extract amalgamation
SQLITEMC=2.2.4
SQLITE="sqlite3mc-${SQLITEMC}-sqlite-3.50.4-amalgamation"
wget -O "$SQLITE.zip" "https://github.com/utelle/SQLite3MultipleCiphers/releases/download/v${SQLITEMC}/$SQLITE.zip"
unzip -p "$SQLITE.zip" "sqlite3mc_amalgamation.c" > "$SQLITE3_LIB_DIR/sqlite3.c"
unzip -p "$SQLITE.zip" "sqlite3mc_amalgamation.h" > "$SQLITE3_LIB_DIR/sqlite3.h"
rm -f "$SQLITE.zip"

export SQLITE3_INCLUDE_DIR="$SQLITE3_LIB_DIR"

# Regenerate bindgen file for sqlcipher
rm -f "$SQLITE3_LIB_DIR/bindgen_bundled_version.rs"

# cargo update
find "$SCRIPT_DIR/../target" -type f -name bindgen.rs -exec rm {} \;
env LIBSQLITE3_SYS_BUNDLING=1 cargo build --features "sqlite3mc buildtime_bindgen session"
find "$SCRIPT_DIR/../target" -type f -name bindgen.rs -exec mv {} "$SQLITE3_LIB_DIR/bindgen_bundled_version.rs" \;

# Sanity checks
cd "$SCRIPT_DIR/.." || { echo "fatal error" >&2; exit 1; }
cargo update --quiet
cargo test --features "backup blob chrono functions limits load_extension serde_json trace vtab bundled-sqlite3mc"
printf '    \e[35;1mFinished\e[0m bundled-sqlite3mc tests\n'
