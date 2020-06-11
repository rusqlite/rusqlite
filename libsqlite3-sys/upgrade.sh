#!/bin/bash -e

CUR_DIR=$(pwd -P)
SCRIPT_DIR=$(cd "$(dirname "$_")" && pwd)
echo "$SCRIPT_DIR"
cd "$SCRIPT_DIR" || { echo "fatal error"; exit 1; }
cargo clean
mkdir -p "$SCRIPT_DIR/../target" "$SCRIPT_DIR/sqlite3" "$SCRIPT_DIR/sqlcipher"
export SQLITE3_LIB_DIR="$SCRIPT_DIR/sqlite3"
export SQLITE3_INCLUDE_DIR="$SQLITE3_LIB_DIR"
export SQLCIPHER_LIB_DIR="$SCRIPT_DIR/sqlcipher"
export SQLCIPHER_INCLUDE_DIR="$SQLCIPHER_LIB_DIR"

# Download and extract sqlite3 amalgamation
SQLITE=sqlite-amalgamation-3320200
curl -O "https://sqlite.org/2020/$SQLITE.zip"
unzip -p "$SQLITE.zip" "$SQLITE/sqlite3.c" > "$SQLITE3_LIB_DIR/sqlite3.c"
unzip -p "$SQLITE.zip" "$SQLITE/sqlite3.h" > "$SQLITE3_LIB_DIR/sqlite3.h"
unzip -p "$SQLITE.zip" "$SQLITE/sqlite3ext.h" > "$SQLITE3_LIB_DIR/sqlite3ext.h"
rm -f "$SQLITE.zip"

# Regenerate bindgen file for sqlite3
rm -f "$SQLITE3_LIB_DIR/bindgen_bundled_version.rs"
cargo update
# Just to make sure there is only one bindgen.rs file in target dir
find "$SCRIPT_DIR/../target" -type f -name bindgen.rs -exec rm {} \;
env LIBSQLITE3_SYS_BUNDLING=1 cargo build --features "buildtime_bindgen" --no-default-features
find "$SCRIPT_DIR/../target" -type f -name bindgen.rs -exec mv {} "$SQLITE3_LIB_DIR/bindgen_bundled_version.rs" \;

SQLCIPHER_VERSION="4.4.0"
if [ "x${1+y}" = xy ]; then
    cd "$CUR_DIR"
    cd "$1" || { echo "Not a directory: $1" >&2; exit 1; }
    printf '##### configuring in %s #####\n\n' "$(pwd -P)"
else
    # $1 unset: Download and generate sqlcipher amalgamation
    mkdir -p $SCRIPT_DIR/sqlcipher.src
    [ -e "v${SQLCIPHER_VERSION}.tar.gz" ] || curl -sfL -O "https://github.com/sqlcipher/sqlcipher/archive/v${SQLCIPHER_VERSION}.tar.gz"
    tar xzf "v${SQLCIPHER_VERSION}.tar.gz" --strip-components=1 -C "$SCRIPT_DIR/sqlcipher.src"
    cd "$SCRIPT_DIR/sqlcipher.src"
fi
./configure --with-crypto-lib=none
make sqlite3.c
cp sqlite3.c sqlite3.h sqlite3ext.h "$SCRIPT_DIR/sqlcipher/"
cp -Rp "$SCRIPT_DIR/sqlcipher" "$SCRIPT_DIR/sqlcipher.orig"
cd "$SCRIPT_DIR"
rm -rf "v${SQLCIPHER_VERSION}.tar.gz" sqlcipher.src sqlcipher.orig

# Regenerate bindgen file for sqlcipher
rm -f "$SQLCIPHER_LIB_DIR/bindgen_bundled_version.rs"
# cargo update
# find "$SCRIPT_DIR/../target" -type f -name bindgen.rs -exec rm {} \;
env LIBSQLITE3_SYS_BUNDLING=1 cargo build --features "sqlcipher buildtime_bindgen"
find "$SCRIPT_DIR/../target" -type f -name bindgen.rs -exec mv {} "$SQLCIPHER_LIB_DIR/bindgen_bundled_version.rs" \;

# Sanity checks
cd "$SCRIPT_DIR/.." || { echo "fatal error"; exit 1; }
cargo update
cargo test --features "backup blob chrono functions limits load_extension serde_json trace vtab bundled"
printf '    \e[35;1mFinished\e[0m bundled sqlite3 tests\n'
cargo test --features "backup blob chrono functions limits load_extension serde_json trace vtab bundled-libressl"
printf '    \e[35;1mFinished\e[0m bundled sqlcipher tests\n'
echo 'You should increment the version in libsqlite3-sys/Cargo.toml'
