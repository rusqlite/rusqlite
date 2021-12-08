#!/bin/bash

set -euf -o pipefail

# the crate dir is where this script is located
crate_dir="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# location of the cdylib embedded library within the target dir to be embedded within the c host extension
example_embedded_extension_lib_dir="${crate_dir}/target/debug"
example_embedded_extension_lib="example_embedded_extension"

# location of the c host extension to be loaded by sqlite
example_c_host_extension_dir="${crate_dir}/example-c-host-extension"
example_c_host_extension="${example_c_host_extension_dir}/libexample_c_host_extension" # sqlite will try adding .so, .dll, .dylib to this on its own

# expected output from vtable query
expected_vtable_output="example_embedded_test_value"

# expected output from function query
expected_function_output="Example embedded extension loaded correctly!"

# sqlite3 include dir (location of sqlite3ext.h) - can be set by SQLITE3_INCLUDE_DIR env var or defaults to bundled version
sqlite3_include_dir=${SQLITE3_INCLUDE_DIR:-${crate_dir}/../libsqlite3-sys/sqlite3}

>&2 echo "checking for sqlite3 shell"
sqlite3_cmd=$(which sqlite3)
>&2 echo "sqlite3 found: ${sqlite3_cmd}"

# build the example-embedded-extension crate
>&2 echo "building the example-embedded-extension crate in ${crate_dir}"
(cd "${crate_dir}" && cargo build --all-targets --verbose)
>&2 echo "successfully built the example-embedded-extension crate"

# build the C-based host extension
>&2 echo "building the example-c-host-extension"
clang -g -fPIC -O2 -shared -I${sqlite3_include_dir} -I${crate_dir} -L${example_embedded_extension_lib_dir} -Wl,-rpath,${example_embedded_extension_lib_dir} -l${example_embedded_extension_lib} ${example_c_host_extension_dir}/example_c_host_extension.c -o ${example_c_host_extension}.so
>&2 echo "successfully built the example-c-host-extension"

>&2 echo "running sqlite3 (${sqlite3_cmd}) to test loadable_extension_embedded ${example_c_host_extension} vtable (embedded within C-based extension)"
actual_vtable_output=$(${sqlite3_cmd} -cmd ".load ${example_c_host_extension}" :memory: "SELECT value FROM example_embedded LIMIT 1;")
>&2 echo "sqlite3 command returned successfully from vtable test, checking output is as expected"
test "${actual_vtable_output}" = "${expected_vtable_output}" && echo "OK" || (echo "vtable output '${actual_vtable_output}' was not as expected '${expected_vtable_output}'"; echo "FAIL"; exit 1)

>&2 echo "running sqlite3 (${sqlite3_cmd}) to test loadable_extension_embedded ${example_c_host_extension} function (embedded within C-based extension)"
actual_function_output=$(${sqlite3_cmd} -cmd ".load ${example_c_host_extension}" :memory: "SELECT example_embedded_test_function();")
>&2 echo "sqlite3 command returned successfully from function test, checking output is as expected"
test  "${actual_function_output}" = "${expected_function_output}" && echo "OK" || (echo "function output '${actual_function_output}' was not as expected '${expected_function_output}'"; echo "FAIL"; exit 1)

>&2 echo "All tests passed."
exit 0
