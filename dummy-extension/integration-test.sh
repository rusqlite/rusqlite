#!/bin/bash

set -euf -o pipefail

sqlite3_cmd=$(which sqlite3)
dummy_extension="dummy-extension/target/debug/libdummy_extension" # sqlite will try adding .so, .dll, .dylib to this on its own

>&2 echo "running sqlite3 (${sqlite3_cmd}) to test loadable extension ${dummy_extension}"
output=$(sqlite3 -cmd ".load ${dummy_extension}" :memory: "SELECT value FROM dummy")

>&2 echo "sqlite3 command returned output, checking it is as expected"
test "1" = "${output}"

>&2 echo "OK"
exit 0
