#!/bin/bash -e

SCRIPT_DIR=$(cd "$(dirname "$_")" && pwd)
TARGET_DIR="$SCRIPT_DIR/../target" # ensure target dir is deterministic
CUR_DIR=$(pwd -P)
echo "$SCRIPT_DIR"
cd "$SCRIPT_DIR" || { echo "fatal error" >&2; exit 1; }
cargo clean
mkdir -p "$SCRIPT_DIR/../target" "$SCRIPT_DIR/sqlite3"
export SQLITE3_LIB_DIR="$SCRIPT_DIR/sqlite3"
export SQLITE3_INCLUDE_DIR="$SQLITE3_LIB_DIR"

# Download and extract amalgamation
SQLITE=sqlite-amalgamation-3370000
curl -sSf -O https://sqlite.org/2021/$SQLITE.zip
unzip -p "$SQLITE.zip" "$SQLITE/sqlite3.c" > "$SQLITE3_LIB_DIR/sqlite3.c"
unzip -p "$SQLITE.zip" "$SQLITE/sqlite3.h" > "$SQLITE3_LIB_DIR/sqlite3.h"
unzip -p "$SQLITE.zip" "$SQLITE/sqlite3ext.h" > "$SQLITE3_LIB_DIR/sqlite3ext.h"
rm -f "$SQLITE.zip"

# Regenerate bindgen file for sqlite3
rm -f "$SQLITE3_LIB_DIR/bindgen_bundled_version.rs"
cargo update

function generate_bindgen_binding() {
  features=$1
  target_file=$2

  rm -f "$target_file"
  # Just to make sure there is only one bindgen.rs file in target dir
  find "$TARGET_DIR" -type f -name bindgen.rs -exec rm {} \;
  env LIBSQLITE3_SYS_BUNDLING=1 cargo build --target-dir "$TARGET_DIR" --features "$features" --no-default-features
  find "$TARGET_DIR" -type f -name bindgen.rs -exec mv {} "$target_file" \;
  # rerun rustfmt after (possibly) adding wrappers
  rustfmt "$target_file"
}

# Regenerate bindgen files
generate_bindgen_binding "buildtime_bindgen session" "$SQLITE3_LIB_DIR/bindgen_bundled_version.rs"
generate_bindgen_binding "buildtime_bindgen loadable_extension" "$SQLITE3_LIB_DIR/bindgen_bundled_version-ext.rs"

# Sanity checks
cd "$SCRIPT_DIR/.." || { echo "fatal error" >&2; exit 1; }
cargo update
cargo test --features "backup blob chrono functions limits load_extension serde_json trace vtab bundled"
printf '    \e[35;1mFinished\e[0m bundled sqlite3 tests\n'
