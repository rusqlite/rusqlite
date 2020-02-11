#!/bin/bash
###############################################################################
# Copyright (c) 2019 Genomics plc <info@genomicsplc.com>
#
# Permission is hereby granted, free of charge, to any person obtaining a copy
# of this software and associated documentation files (the "Software"), to deal
# in the Software without restriction, including without limitation the rights
# to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
# copies of the Software, and to permit persons to whom the Software is
# furnished to do so, subject to the following conditions:
#
# The above copyright notice and this permission notice shall be included in
# all copies or substantial portions of the Software.
#
# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
# IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
# FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
# AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
# LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
# OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
# THE SOFTWARE.
###############################################################################
set -euf -o pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" >/dev/null 2>&1 && pwd )"

build_dir=$(mktemp -d)
function cleanup {
    >&2 echo "deleting ${build_dir}"
    rm -rf "${build_dir}"
}
trap cleanup EXIT

function generate_bindgen_bindings() {
    version=$1
    url=$2
    prefix=$3
    
    >&2 echo "generating bindgen bindings for sqlite3 ${version} in ${build_dir}"
    curl -sSf -o "${build_dir}/sqlite-amalgamation-${version}.zip" "${url}"
    mkdir "${build_dir}/sqlite-${version}"
    unzip -qq -d "${build_dir}/sqlite-${version}" "${build_dir}/sqlite-amalgamation-${version}.zip"
    export SQLITE3_LIB_DIR="${build_dir}/sqlite-${version}/${prefix}"
    export SQLITE3_INCLUDE_DIR="${build_dir}/sqlite-${version}/${prefix}"
    manifest="${SCRIPT_DIR}/../Cargo.toml"
    generate_bindgen_binding "${manifest}" "${build_dir}" "${SCRIPT_DIR}" "buildtime_bindgen" "${version}"
    generate_bindgen_binding "${manifest}" "${build_dir}" "${SCRIPT_DIR}" "buildtime_bindgen,loadable_extension" "${version}-ext"
    generate_bindgen_binding "${manifest}" "${build_dir}" "${SCRIPT_DIR}" "buildtime_bindgen,loadable_extension_embedded" "${version}-ext-embed"
    >&2 echo "done generating bindings for sqlite ${version}"
}

function generate_bindgen_binding {
    manifest_path=$1
    build_dir=$2
    output_dir=$3
    features=$4
    output_base=$5

    target_dir="${build_dir}/${output_base}/target"
    output_path="${output_dir}/bindgen_${output_base}.rs"
    
    >&2 echo "calling cargo build on manifest ${manifest_path} building in ${target_dir} with features ${features}"
    cargo build --manifest-path "${manifest_path}" --target-dir "${target_dir}" --features "${features}" -p libsqlite3-sys
    bindgen_file=$(set +f ; find "${target_dir}/debug/build/libsqlite3-sys-"* -name bindgen.rs ; set -f)
    if [[ "$(echo "${bindgen_file}" | wc -l)" != 1 ]]; then
	>&2 echo "multiple bindgen files found in target directory: ${bindgen_file}"
	exit 2
    fi
    cp  "${bindgen_file}" "${output_path}"
}

generate_bindgen_bindings 3.7.16 https://sqlite.org/2013/sqlite-amalgamation-3071600.zip sqlite-amalgamation-3071600/
generate_bindgen_bindings 3.6.8 https://sqlite.org/sqlite-amalgamation-3_6_8.zip ./
