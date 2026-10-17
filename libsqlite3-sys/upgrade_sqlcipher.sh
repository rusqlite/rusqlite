#!/bin/sh -e

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
echo "$SCRIPT_DIR"
cd "$SCRIPT_DIR" || { echo "fatal error" >&2; exit 1; }
cargo clean -p libsqlite3-sys
mkdir -p "$SCRIPT_DIR/../target" "$SCRIPT_DIR/sqlcipher"
SQLCIPHER_LIB_DIR="$SCRIPT_DIR/sqlcipher"

# Regenerate bindgen file for sqlcipher from the sqlcipher-src header
rm -f "$SQLCIPHER_LIB_DIR/bindgen_bundled_version.rs"

# cargo update
find "$SCRIPT_DIR/../target" -type f -name bindgen.rs -exec rm {} \;
env LIBSQLITE3_SYS_BUNDLING=1 cargo build --features "bundled-sqlcipher buildtime_bindgen session"
find "$SCRIPT_DIR/../target" -type f -name bindgen.rs -exec mv {} "$SQLCIPHER_LIB_DIR/bindgen_bundled_version.rs" \;

# Sanity checks
cd "$SCRIPT_DIR/.." || { echo "fatal error" >&2; exit 1; }
cargo update --quiet
cargo test --features "backup blob chrono functions limits load_extension serde_json trace vtab bundled-sqlcipher-vendored-openssl"
printf '    \e[35;1mFinished\e[0m bundled-sqlcipher-vendored-openssl/sqlcipher tests\n'
