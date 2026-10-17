#!/bin/sh -e

# Regenerates sqlcipher/bindgen_bundled_version.rs from the header of the pinned sqlcipher-src.

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
echo "$SCRIPT_DIR"
cd "$SCRIPT_DIR" || { echo "fatal error" >&2; exit 1; }

cargo clean -p libsqlite3-sys
mkdir -p "$SCRIPT_DIR/../target" "$SCRIPT_DIR/sqlcipher"

# Remove the stale bindings so a fresh one is copied in.
rm -f "$SCRIPT_DIR/sqlcipher/bindgen_bundled_version.rs"
find "$SCRIPT_DIR/../target" -type f -name bindgen.rs -delete

# Build with buildtime_bindgen to regenerate bindings from the crate header.
env LIBSQLITE3_SYS_BUNDLING=1 cargo build \
  --features "bundled-sqlcipher buildtime_bindgen session"
find "$SCRIPT_DIR/../target" -type f -name bindgen.rs \
  -exec cp {} "$SCRIPT_DIR/sqlcipher/bindgen_bundled_version.rs" \;

# Sanity checks
cd "$SCRIPT_DIR/.." || { echo "fatal error" >&2; exit 1; }
cargo update --quiet
cargo test --features "backup blob chrono functions limits load_extension serde_json trace vtab bundled-sqlcipher-vendored-openssl"
printf '    \e[35;1mFinished\e[0m bundled-sqlcipher-vendored-openssl/sqlcipher tests\n'
