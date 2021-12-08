use std::env;
use std::path::Path;

/// Tells whether we're building for Windows. This is more suitable than a plain
/// `cfg!(windows)`, since the latter does not properly handle cross-compilation
///
/// Note that there is no way to know at compile-time which system we'll be
/// targetting, and this test must be made at run-time (of the build script) See
/// https://doc.rust-lang.org/cargo/reference/environment-variables.html#environment-variables-cargo-sets-for-build-scripts
fn win_target() -> bool {
    std::env::var("CARGO_CFG_WINDOWS").is_ok()
}

/// Tells whether we're building for Android.
/// See [`win_target`]
#[cfg(any(feature = "bundled", feature = "bundled-windows"))]
fn android_target() -> bool {
    std::env::var("CARGO_CFG_TARGET_OS").map_or(false, |v| v == "android")
}

/// Tells whether a given compiler will be used `compiler_name` is compared to
/// the content of `CARGO_CFG_TARGET_ENV` (and is always lowercase)
///
/// See [`win_target`]
fn is_compiler(compiler_name: &str) -> bool {
    std::env::var("CARGO_CFG_TARGET_ENV").map_or(false, |v| v == compiler_name)
}

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir).join("bindgen.rs");
    if cfg!(feature = "in_gecko") {
        // When inside mozilla-central, we are included into the build with
        // sqlite3.o directly, so we don't want to provide any linker arguments.
        std::fs::copy("sqlite3/bindgen_bundled_version.rs", out_path)
            .expect("Could not copy bindings to output directory");
        return;
    }
    if cfg!(all(
        feature = "sqlcipher",
        not(feature = "bundled-sqlcipher")
    )) {
        if cfg!(feature = "bundled") || (win_target() && cfg!(feature = "bundled-windows")) {
            println!(
                "cargo:warning=For backwards compatibility, feature 'sqlcipher' overrides
                features 'bundled' and 'bundled-windows'. If you want a bundled build of
                SQLCipher (available for the moment only on Unix), use feature 'bundled-sqlcipher'
                or 'bundled-sqlcipher-vendored-openssl' to also bundle OpenSSL crypto."
            )
        }
        build_linked::main(&out_dir, &out_path)
    } else if cfg!(feature = "loadable_extension") {
        build_loadable_extension::main(&out_dir, &out_path)
    } else if cfg!(feature = "bundled")
        || (win_target() && cfg!(feature = "bundled-windows"))
        || cfg!(feature = "bundled-sqlcipher")
    {
        #[cfg(any(
            feature = "bundled",
            feature = "bundled-windows",
            feature = "bundled-sqlcipher"
        ))]
        build_bundled::main(&out_dir, &out_path);
        #[cfg(not(any(
            feature = "bundled",
            feature = "bundled-windows",
            feature = "bundled-sqlcipher"
        )))]
        panic!("The runtime test should not run this branch, which has not compiled any logic.")
    } else {
        build_linked::main(&out_dir, &out_path)
    }
}

#[cfg(any(
    feature = "bundled",
    feature = "bundled-windows",
    feature = "bundled-sqlcipher"
))]
mod build_bundled {
    use std::env;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use super::{is_compiler, win_target};

    pub fn main(out_dir: &str, out_path: &Path) {
        let lib_name = super::lib_name();

        // This is just a sanity check, the top level `main` should ensure this.
        assert!(!(cfg!(feature = "bundled-windows") && !cfg!(feature = "bundled") && !win_target()),
            "This module should not be used: we're not on Windows and the bundled feature has not been enabled");

        #[cfg(feature = "buildtime_bindgen")]
        {
            use super::{bindings, header_file, HeaderLocation};
            let header_path = format!("{}/{}", lib_name, header_file());
            let header = HeaderLocation::FromPath(header_path.clone());
            bindings::write_to_out_dir(header, out_path);
            println!("cargo:rerun-if-changed={}", &header_path);
        }
        #[cfg(not(feature = "buildtime_bindgen"))]
        {
            use std::fs;
            fs::copy(format!("{}/bindgen_bundled_version.rs", lib_name), out_path)
                .expect("Could not copy bindings to output directory");
            println!("cargo:rerun-if-changed=sqlite3/bindgen_bundled_version.rs");
        }
        // println!("cargo:rerun-if-changed=sqlite3/sqlite3.c");
        // println!("cargo:rerun-if-changed=sqlcipher/sqlite3.c");
        println!("cargo:rerun-if-changed={}/sqlite3.c", lib_name);
        println!("cargo:rerun-if-changed=sqlite3/wasm32-wasi-vfs.c");
        let mut cfg = cc::Build::new();
        cfg.file(format!("{}/sqlite3.c", lib_name))
            .flag("-DSQLITE_CORE")
            .flag("-DSQLITE_DEFAULT_FOREIGN_KEYS=1")
            .flag("-DSQLITE_ENABLE_API_ARMOR")
            .flag("-DSQLITE_ENABLE_COLUMN_METADATA")
            .flag("-DSQLITE_ENABLE_DBSTAT_VTAB")
            .flag("-DSQLITE_ENABLE_FTS3")
            .flag("-DSQLITE_ENABLE_FTS3_PARENTHESIS")
            .flag("-DSQLITE_ENABLE_FTS5")
            .flag("-DSQLITE_ENABLE_JSON1")
            .flag("-DSQLITE_ENABLE_LOAD_EXTENSION=1")
            .flag("-DSQLITE_ENABLE_MEMORY_MANAGEMENT")
            .flag("-DSQLITE_ENABLE_RTREE")
            .flag("-DSQLITE_ENABLE_STAT2")
            .flag("-DSQLITE_ENABLE_STAT4")
            .flag("-DSQLITE_SOUNDEX")
            .flag("-DSQLITE_THREADSAFE=1")
            .flag("-DSQLITE_USE_URI")
            .flag("-DHAVE_USLEEP=1")
            .flag("-D_POSIX_THREAD_SAFE_FUNCTIONS") // cross compile with MinGW
            .warnings(false);

        if cfg!(feature = "bundled-sqlcipher") {
            cfg.flag("-DSQLITE_HAS_CODEC").flag("-DSQLITE_TEMP_STORE=2");

            let target = env::var("TARGET").unwrap();
            let host = env::var("HOST").unwrap();

            let is_windows = host.contains("windows") && target.contains("windows");
            let is_apple = host.contains("apple") && target.contains("apple");

            let lib_dir = env("OPENSSL_LIB_DIR").map(PathBuf::from);
            let inc_dir = env("OPENSSL_INCLUDE_DIR").map(PathBuf::from);
            let mut use_openssl = false;

            let (lib_dir, inc_dir) = match (lib_dir, inc_dir) {
                (Some(lib_dir), Some(inc_dir)) => {
                    use_openssl = true;
                    (lib_dir, inc_dir)
                }
                (lib_dir, inc_dir) => match find_openssl_dir(&host, &target) {
                    None => {
                        if is_windows && !cfg!(feature = "bundled-sqlcipher-vendored-openssl") {
                            panic!("Missing environment variable OPENSSL_DIR or OPENSSL_DIR is not set")
                        } else {
                            (PathBuf::new(), PathBuf::new())
                        }
                    }
                    Some(openssl_dir) => {
                        let lib_dir = lib_dir.unwrap_or_else(|| openssl_dir.join("lib"));
                        let inc_dir = inc_dir.unwrap_or_else(|| openssl_dir.join("include"));

                        assert!(
                            Path::new(&lib_dir).exists(),
                            "OpenSSL library directory does not exist: {}",
                            lib_dir.to_string_lossy()
                        );

                        if !Path::new(&inc_dir).exists() {
                            panic!(
                                "OpenSSL include directory does not exist: {}",
                                inc_dir.to_string_lossy()
                            )
                        }

                        use_openssl = true;
                        (lib_dir, inc_dir)
                    }
                },
            };

            if cfg!(feature = "bundled-sqlcipher-vendored-openssl") {
                cfg.include(std::env::var("DEP_OPENSSL_INCLUDE").unwrap());
                // cargo will resolve downstream to the static lib in
                // openssl-sys
            } else if is_windows {
                // Windows without `-vendored-openssl` takes this to link against a prebuilt
                // OpenSSL lib
                cfg.include(inc_dir.to_string_lossy().as_ref());
                let lib = lib_dir.join("libcrypto.lib");
                cfg.flag(lib.to_string_lossy().as_ref());
            } else if use_openssl {
                cfg.include(inc_dir.to_string_lossy().as_ref());
                // branch not taken on Windows, just `crypto` is fine.
                println!("cargo:rustc-link-lib=dylib=crypto");
                println!("cargo:rustc-link-search={}", lib_dir.to_string_lossy());
            } else if is_apple {
                cfg.flag("-DSQLCIPHER_CRYPTO_CC");
                println!("cargo:rustc-link-lib=framework=Security");
                println!("cargo:rustc-link-lib=framework=CoreFoundation");
            } else {
                // branch not taken on Windows, just `crypto` is fine.
                println!("cargo:rustc-link-lib=dylib=crypto");
            }
        }

        // on android sqlite can't figure out where to put the temp files.
        // the bundled sqlite on android also uses `SQLITE_TEMP_STORE=3`.
        // https://android.googlesource.com/platform/external/sqlite/+/2c8c9ae3b7e6f340a19a0001c2a889a211c9d8b2/dist/Android.mk
        if super::android_target() {
            cfg.flag("-DSQLITE_TEMP_STORE=3");
        }

        if cfg!(feature = "with-asan") {
            cfg.flag("-fsanitize=address");
        }

        // If explicitly requested: enable static linking against the Microsoft Visual
        // C++ Runtime to avoid dependencies on vcruntime140.dll and similar libraries.
        if cfg!(target_feature = "crt-static") && is_compiler("msvc") {
            cfg.static_crt(true);
        }

        // Older versions of visual studio don't support c99 (including isnan), which
        // causes a build failure when the linker fails to find the `isnan`
        // function. `sqlite` provides its own implementation, using the fact
        // that x != x when x is NaN.
        //
        // There may be other platforms that don't support `isnan`, they should be
        // tested for here.
        if is_compiler("msvc") {
            use cc::windows_registry::{find_vs_version, VsVers};
            let vs_has_nan = match find_vs_version() {
                Ok(ver) => ver != VsVers::Vs12,
                Err(_msg) => false,
            };
            if vs_has_nan {
                cfg.flag("-DHAVE_ISNAN");
            }
        } else {
            cfg.flag("-DHAVE_ISNAN");
        }
        if !win_target() {
            cfg.flag("-DHAVE_LOCALTIME_R");
        }
        // Target wasm32-wasi can't compile the default VFS
        if is_compiler("wasm32-wasi") {
            cfg.flag("-DSQLITE_OS_OTHER")
                // https://github.com/rust-lang/rust/issues/74393
                .flag("-DLONGDOUBLE_TYPE=double");
            if cfg!(feature = "wasm32-wasi-vfs") {
                cfg.file("sqlite3/wasm32-wasi-vfs.c");
            }
        }
        if cfg!(feature = "unlock_notify") {
            cfg.flag("-DSQLITE_ENABLE_UNLOCK_NOTIFY");
        }
        if cfg!(feature = "preupdate_hook") {
            cfg.flag("-DSQLITE_ENABLE_PREUPDATE_HOOK");
        }
        if cfg!(feature = "session") {
            cfg.flag("-DSQLITE_ENABLE_SESSION");
        }

        if let Ok(limit) = env::var("SQLITE_MAX_VARIABLE_NUMBER") {
            cfg.flag(&format!("-DSQLITE_MAX_VARIABLE_NUMBER={}", limit));
        }
        println!("cargo:rerun-if-env-changed=SQLITE_MAX_VARIABLE_NUMBER");

        if let Ok(limit) = env::var("SQLITE_MAX_EXPR_DEPTH") {
            cfg.flag(&format!("-DSQLITE_MAX_EXPR_DEPTH={}", limit));
        }
        println!("cargo:rerun-if-env-changed=SQLITE_MAX_EXPR_DEPTH");

        if let Ok(extras) = env::var("LIBSQLITE3_FLAGS") {
            for extra in extras.split_whitespace() {
                if extra.starts_with("-D") || extra.starts_with("-U") {
                    cfg.flag(extra);
                } else if extra.starts_with("SQLITE_") {
                    cfg.flag(&format!("-D{}", extra));
                } else {
                    panic!("Don't understand {} in LIBSQLITE3_FLAGS", extra);
                }
            }
        }
        println!("cargo:rerun-if-env-changed=LIBSQLITE3_FLAGS");

        cfg.compile(lib_name);

        println!("cargo:lib_dir={}", out_dir);
    }

    fn env(name: &str) -> Option<OsString> {
        let prefix = env::var("TARGET").unwrap().to_uppercase().replace("-", "_");
        let prefixed = format!("{}_{}", prefix, name);
        let var = env::var_os(&prefixed);

        match var {
            None => env::var_os(name),
            _ => var,
        }
    }

    fn find_openssl_dir(_host: &str, _target: &str) -> Option<PathBuf> {
        let openssl_dir = env("OPENSSL_DIR");
        openssl_dir.map(PathBuf::from)
    }
}

fn env_prefix() -> &'static str {
    if cfg!(any(feature = "sqlcipher", feature = "bundled-sqlcipher")) {
        "SQLCIPHER"
    } else {
        "SQLITE3"
    }
}

fn header_file() -> &'static str {
    if cfg!(feature = "loadable_extension") {
        "sqlite3ext.h"
    } else {
        "sqlite3.h"
    }
}

fn wrapper_file() -> &'static str {
    if cfg!(feature = "loadable_extension") {
        "wrapper-ext.h"
    } else {
        "wrapper.h"
    }
}

fn lib_name() -> &'static str {
    if cfg!(any(feature = "sqlcipher", feature = "bundled-sqlcipher")) {
        "sqlcipher"
    } else if cfg!(all(windows, feature = "winsqlite3")) {
        "winsqlite3"
    } else {
        "sqlite3"
    }
}

pub enum HeaderLocation {
    FromEnvironment,
    Wrapper,
    FromPath(String),
}

impl From<HeaderLocation> for String {
    fn from(header: HeaderLocation) -> String {
        match header {
            HeaderLocation::FromEnvironment => {
                let prefix = env_prefix();
                let mut header = env::var(format!("{}_INCLUDE_DIR", prefix)).unwrap_or_else(|_| {
                    panic!(
                        "{}_INCLUDE_DIR must be set if {}_LIB_DIR is set",
                        prefix, prefix
                    )
                });
                header.push('/');
                header.push_str(header_file());
                header
            }
            HeaderLocation::Wrapper => wrapper_file().into(),
            HeaderLocation::FromPath(path) => path,
        }
    }
}

mod build_linked {
    #[cfg(feature = "vcpkg")]
    extern crate vcpkg;

    use super::{bindings, env_prefix, header_file, is_compiler, lib_name, win_target, HeaderLocation};
    use std::env;
    use std::path::Path;

    pub fn main(_out_dir: &str, out_path: &Path) {
        let header = find_sqlite();
        if (cfg!(any(
            feature = "bundled_bindings",
            feature = "bundled",
            feature = "bundled-sqlcipher"
        )) || (win_target() && cfg!(feature = "bundled-windows")))
            && !cfg!(feature = "buildtime_bindgen")
        {
            // Generally means the `bundled_bindings` feature is enabled.
            // Most users are better off with turning
            // on buildtime_bindgen instead, but this is still supported as we
            // have runtime version checks and there are good reasons to not
            // want to run bindgen.
            std::fs::copy(
                format!("{}/bindgen_bundled_version.rs", lib_name()),
                out_path,
            )
            .expect("Could not copy bindings to output directory");
        } else {
            bindings::write_to_out_dir(header, out_path);
        }
    }

    fn find_link_mode() -> &'static str {
        // If the user specifies SQLITE3_STATIC (or SQLCIPHER_STATIC), do static
        // linking, unless it's explicitly set to 0.
        match &env::var(format!("{}_STATIC", env_prefix())) {
            Ok(v) if v != "0" => "static",
            _ => "dylib",
        }
    }
    // Prints the necessary cargo link commands and returns the path to the header.
    fn find_sqlite() -> HeaderLocation {
        let link_lib = lib_name();

        println!("cargo:rerun-if-env-changed={}_INCLUDE_DIR", env_prefix());
        println!("cargo:rerun-if-env-changed={}_LIB_DIR", env_prefix());
        println!("cargo:rerun-if-env-changed={}_STATIC", env_prefix());
        if cfg!(feature = "vcpkg") && is_compiler("msvc") {
            println!("cargo:rerun-if-env-changed=VCPKGRS_DYNAMIC");
        }

        // dependents can access `DEP_SQLITE3_LINK_TARGET` (`sqlite3` being the
        // `links=` value in our Cargo.toml) to get this value. This might be
        // useful if you need to ensure whatever crypto library sqlcipher relies
        // on is available, for example.
        println!("cargo:link-target={}", link_lib);

        if win_target() && cfg!(feature = "winsqlite3") {
            println!("cargo:rustc-link-lib=dylib={}", link_lib);
            return HeaderLocation::Wrapper;
        }

        // Allow users to specify where to find SQLite.
        if let Ok(dir) = env::var(format!("{}_LIB_DIR", env_prefix())) {
            // Try to use pkg-config to determine link commands
            let pkgconfig_path = Path::new(&dir).join("pkgconfig");
            env::set_var("PKG_CONFIG_PATH", pkgconfig_path);
            if pkg_config::Config::new().probe(link_lib).is_err() {
                // Otherwise just emit the bare minimum link commands.
                println!("cargo:rustc-link-lib={}={}", find_link_mode(), link_lib);
                println!("cargo:rustc-link-search={}", dir);
            }
            return HeaderLocation::FromEnvironment;
        }

        if let Some(header) = try_vcpkg() {
            return header;
        }

        // See if pkg-config can do everything for us.
        match pkg_config::Config::new()
            .print_system_libs(false)
            .probe(link_lib)
        {
            Ok(mut lib) => {
                if let Some(mut header) = lib.include_paths.pop() {
                    header.push(header_file());
                    HeaderLocation::FromPath(header.to_string_lossy().into())
                } else {
                    HeaderLocation::Wrapper
                }
            }
            Err(_) => {
                // No env var set and pkg-config couldn't help; just output the link-lib
                // request and hope that the library exists on the system paths. We used to
                // output /usr/lib explicitly, but that can introduce other linking problems;
                // see https://github.com/rusqlite/rusqlite/issues/207.
                println!("cargo:rustc-link-lib={}={}", find_link_mode(), link_lib);
                HeaderLocation::Wrapper
            }
        }
    }

    fn try_vcpkg() -> Option<HeaderLocation> {
        if cfg!(feature = "vcpkg") && is_compiler("msvc") {
            // See if vcpkg can find it.
            if let Ok(mut lib) = vcpkg::Config::new().probe(lib_name()) {
                if let Some(mut header) = lib.include_paths.pop() {
                    header.push(header_file());
                    return Some(HeaderLocation::FromPath(header.to_string_lossy().into()));
                }
            }
            None
        } else {
            None
        }
    }
}

mod build_loadable_extension {
    use super::{bindings, env_prefix, header_file, HeaderLocation};
    use std::env;
    use std::path::Path;

    pub fn main(_out_dir: &str, out_path: &Path) {
        let header = find_sqlite();
        if cfg!(feature = "session") {
            panic!("The session feature is not available when building a loadable extension since the sqlite API routines for loadable extensions do not include session methods");
        }
        bindings::write_to_out_dir(header, out_path);
    }

    // Prints the necessary cargo link commands and returns the path to the header.
    fn find_sqlite() -> HeaderLocation {
        let link_lib = "sqlite3";
        println!("cargo:rerun-if-env-changed={}_INCLUDE_DIR", env_prefix());
        println!("cargo:rerun-if-env-changed={}_LIB_DIR", env_prefix());
        println!("cargo:rerun-if-env-changed={}_STATIC", env_prefix());
        if cfg!(all(feature = "vcpkg", target_env = "msvc")) {
            println!("cargo:rerun-if-env-changed=VCPKGRS_DYNAMIC");
        }
        // Allow users to specify where to find SQLite.
        if let Ok(dir) = env::var(format!("{}_LIB_DIR", env_prefix())) {
            // Try to use pkg-config to determine link commands
            let pkgconfig_path = Path::new(&dir).join("pkgconfig");
            env::set_var("PKG_CONFIG_PATH", pkgconfig_path);
            return HeaderLocation::FromEnvironment;
        }

        if let Some(header) = try_vcpkg() {
            return header;
        }

        // See if pkg-config can do everything for us.
        match pkg_config::Config::new()
            .print_system_libs(false)
            .probe(link_lib)
        {
            Ok(mut lib) => {
                if let Some(mut header) = lib.include_paths.pop() {
                    header.push(header_file());
                    HeaderLocation::FromPath(header.to_string_lossy().into())
                } else {
                    HeaderLocation::Wrapper
                }
            }
            Err(_) => HeaderLocation::Wrapper,
        }
    }

    #[cfg(all(feature = "vcpkg", target_env = "msvc"))]
    fn try_vcpkg() -> Option<HeaderLocation> {
        let link_lib = "sqlite3";
        // See if vcpkg can find it.
        if let Ok(mut lib) = vcpkg::Config::new().probe(link_lib) {
            if let Some(mut header) = lib.include_paths.pop() {
                header.push(header_file());
                return Some(HeaderLocation::FromPath(header.to_string_lossy().into()));
            }
        }
        None
    }

    #[cfg(not(all(feature = "vcpkg", target_env = "msvc")))]
    fn try_vcpkg() -> Option<HeaderLocation> {
        None
    }
}

#[cfg(not(feature = "buildtime_bindgen"))]
mod bindings {
    #![allow(dead_code)]
    use super::HeaderLocation;

    use std::fs;
    use std::path::Path;

    static PREBUILT_BINDGEN_PATHS: &[&str] = &[
        "bindgen-bindings/bindgen_3.6.8",
        #[cfg(feature = "min_sqlite_version_3_6_23")]
        "bindgen-bindings/bindgen_3.6.23",
        #[cfg(feature = "min_sqlite_version_3_7_7")]
        "bindgen-bindings/bindgen_3.7.7",
        #[cfg(feature = "min_sqlite_version_3_7_16")]
        "bindgen-bindings/bindgen_3.7.16",
        #[cfg(any(
            feature = "bundled_bindings",
            feature = "bundled",
            all(windows, feature = "bundled-windows")
        ))]
        "sqlite3/bindgen_bundled_version",
    ];

    pub fn write_to_out_dir(_header: HeaderLocation, out_path: &Path) {
        let in_path = format!(
            "{}{}.rs",
            PREBUILT_BINDGEN_PATHS[PREBUILT_BINDGEN_PATHS.len() - 1],
            prebuilt_bindgen_ext()
        );
        fs::copy(in_path.to_owned(), out_path).unwrap_or_else(|_| {
            panic!(
                "Could not copy bindings to output directory from {}",
                in_path
            )
        });
    }

    fn prebuilt_bindgen_ext() -> &'static str {
        if cfg!(feature = "loadable_extension") {
            "-ext"
        } else {
            ""
        }
    }
}

#[cfg(feature = "buildtime_bindgen")]
mod bindings {
    use super::HeaderLocation;
    use bindgen::callbacks::{IntKind, ParseCallbacks};

    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::Path;

    use super::win_target;

    #[derive(Debug)]
    struct SqliteTypeChooser;

    impl ParseCallbacks for SqliteTypeChooser {
        fn int_macro(&self, _name: &str, value: i64) -> Option<IntKind> {
            if value >= i32::min_value() as i64 && value <= i32::max_value() as i64 {
                Some(IntKind::I32)
            } else {
                None
            }
        }
    }

    // Are we generating the bundled bindings? Used to avoid emitting things
    // that would be problematic in bundled builds. This env var is set by
    // `upgrade.sh`.
    fn generating_bundled_bindings() -> bool {
        // Hacky way to know if we're generating the bundled bindings
        println!("cargo:rerun-if-env-changed=LIBSQLITE3_SYS_BUNDLING");
        match std::env::var("LIBSQLITE3_SYS_BUNDLING") {
            Ok(v) => v != "0",
            Err(_) => false,
        }
    }

    pub fn write_to_out_dir(header: HeaderLocation, out_path: &Path) {
        let header: String = header.into();
        let mut output = Vec::new();
        println!("cargo:rerun-if-env-changed=SQLITE3_INCLUDE_DIR");
        println!("cargo:rerun-if-env-changed=SQLITE3_LIB_DIR");
        let mut bindings = bindgen::builder()
            .trust_clang_mangling(false)
            .header(header.clone())
            .parse_callbacks(Box::new(SqliteTypeChooser))
            .rustfmt_bindings(true);

        if cfg!(any(feature = "sqlcipher", feature = "bundled-sqlcipher")) {
            bindings = bindings.clang_arg("-DSQLITE_HAS_CODEC");
        }
        if cfg!(feature = "unlock_notify") {
            bindings = bindings.clang_arg("-DSQLITE_ENABLE_UNLOCK_NOTIFY");
        }
        if cfg!(feature = "preupdate_hook") {
            bindings = bindings.clang_arg("-DSQLITE_ENABLE_PREUPDATE_HOOK");
        }
        if cfg!(feature = "session") {
            bindings = bindings.clang_arg("-DSQLITE_ENABLE_SESSION");
        }
        if win_target() && cfg!(feature = "winsqlite3") {
            bindings = bindings
                .clang_arg("-DBINDGEN_USE_WINSQLITE3")
                .blocklist_item("NTDDI_.+")
                .blocklist_item("WINAPI_FAMILY.*")
                .blocklist_item("_WIN32_.+")
                .blocklist_item("_VCRT_COMPILER_PREPROCESSOR")
                .blocklist_item("_SAL_VERSION")
                .blocklist_item("__SAL_H_VERSION")
                .blocklist_item("_USE_DECLSPECS_FOR_SAL")
                .blocklist_item("_USE_ATTRIBUTES_FOR_SAL")
                .blocklist_item("_CRT_PACKING")
                .blocklist_item("_HAS_EXCEPTIONS")
                .blocklist_item("_STL_LANG")
                .blocklist_item("_HAS_CXX17")
                .blocklist_item("_HAS_CXX20")
                .blocklist_item("_HAS_NODISCARD")
                .blocklist_item("WDK_NTDDI_VERSION")
                .blocklist_item("OSVERSION_MASK")
                .blocklist_item("SPVERSION_MASK")
                .blocklist_item("SUBVERSION_MASK")
                .blocklist_item("WINVER")
                .blocklist_item("__security_cookie")
                .blocklist_type("size_t")
                .blocklist_type("__vcrt_bool")
                .blocklist_type("wchar_t")
                .blocklist_function("__security_init_cookie")
                .blocklist_function("__report_gsfailure")
                .blocklist_function("__va_start");
        }

        // When cross compiling unless effort is taken to fix the issue, bindgen
        // will find the wrong headers. There's only one header included by the
        // amalgamated `sqlite.h`: `stdarg.h`.
        //
        // Thankfully, there's almost no case where rust code needs to use
        // functions taking `va_list` (It's nearly impossible to get a `va_list`
        // in Rust unless you get passed it by C code for some reason).
        //
        // Arguably, we should never be including these, but we include them for
        // the cases where they aren't totally broken...
        let target_arch = std::env::var("TARGET").unwrap();
        let host_arch = std::env::var("HOST").unwrap();
        let is_cross_compiling = target_arch != host_arch;
        let blocklist_va_list_functions = &vec![
            "sqlite3_vmprintf",
            "sqlite3_vsnprintf",
            "sqlite3_xvsnprintf",
            "sqlite3_str_vappendf",
        ];
        // Note that when generating the bundled file, we're essentially always
        // cross compiling.
        if generating_bundled_bindings() || is_cross_compiling {
            // get rid of blocklisted functions that use va_list
            for fn_name in blocklist_va_list_functions {
                bindings = bindings.blocklist_function(fn_name)
            }
            // Get rid of va_list
            bindings = bindings
                .blocklist_type("va_list")
                .blocklist_type("__builtin_va_list")
                .blocklist_type("__gnuc_va_list")
                .blocklist_item("__GNUC_VA_LIST");

            // handle __va_list_tag specially as it is referenced from sqlite3_api_routines
            // so if it is blocklisted, those references will be broken for loadable extensions.
            #[cfg(not(feature = "loadable_extension"))]
            {
                bindings = bindings.blocklist_type("__va_list_tag");
            }
            // when building as a loadable_extension, make __va_list_tag opaque instead of omitting it
            #[cfg(feature = "loadable_extension")]
            {
                bindings = bindings.opaque_type("__va_list_tag");
            }
        }

        // rust-bindgen does not handle CPP macros that alias functions, so
        // when using sqlite3ext.h to support loadable extensions, the macros
        // that attempt to redefine sqlite3 API routines to be redirected through
        // the global sqlite3_api instance of the sqlite3_api_routines structure
        // do not result in any code production.
        //
        // Before defining wrappers to take their place, we need to blocklist
        // all sqlite3 API functions since none of their symbols will be
        // available directly when being loaded as an extension.
        #[cfg(feature = "loadable_extension")]
        {
            // some api functions do not have an implementation in sqlite3_api_routines
            // (for example: sqlite3_config, sqlite3_initialize, sqlite3_interrupt, ...).
            // while this isn't a problem for shared libraries (unless we actually try to
            // call them, it is better to blocklist them all so that the build will fail
            // if an attempt is made to call an extern function that we know won't exist
            // and to avoid undefined symbol issues when linking the loadable extension
            // rust code with other (e.g. non-rust) code
            bindings = bindings.blocklist_function(".*")
        }

        bindings
            .generate()
            .unwrap_or_else(|_| panic!("could not run bindgen on header {}", header))
            .write(Box::new(&mut output))
            .expect("could not write output of bindgen");

        #[allow(unused_mut)]
        let mut output_string = String::from_utf8(output).expect("bindgen output was not UTF-8?!");

        // Get the list of API functions supported by sqlite3_api_routines,
        // set the corresponding sqlite3 api routine to be blocklisted in the
        // final bindgen run, and add wrappers for each of the API functions to
        // dispatch the API call through a sqlite3_api global, which is defined
        // outside the generated bindings in lib.rs, either as a built-in static
        // or an extern symbol in the case of loadable_extension_embedded (i.e.
        // when the rust code will be a part of an extension but not implement
        // the extension entrypoint itself).
        #[cfg(feature = "loadable_extension")]
        {
            let api_routines_struct_name = "sqlite3_api_routines".to_owned();

            let api_routines_struct =
                match get_struct_by_name(&output_string, &api_routines_struct_name) {
                    Some(s) => s,
                    None => {
                        panic!(
                            "Failed to find struct {} in early bindgen output",
                            &api_routines_struct_name
                        );
                    }
                };

            output_string.push_str(
                r#"

// sqlite3_api is defined in lib.rs as either a static or an extern when compiled as a loadable_extension
use crate::sqlite3_api;

// sqlite3 API wrappers to support loadable extensions (Note: these were generated from build.rs - not by rust-bindgen)

"#,
            );

            // create wrapper for each field in api routines struct
            for field in &api_routines_struct.fields {
                let ident = match &field.ident {
                    Some(ident) => ident,
                    None => {
                        panic!("Unexpected anonymous field in sqlite");
                    }
                };
                let field_type = &field.ty;

                // construct global sqlite api function identifier from field identifier
                let api_fn_name = format!("sqlite3_{}", ident);

                if (generating_bundled_bindings() || is_cross_compiling)
                    && blocklist_va_list_functions
                        .iter()
                        .any(|fn_name| *fn_name == api_fn_name)
                {
                    // skip this function as it is blocklisted when generating bundled bindings or cross compiling
                    continue;
                }

                // generate wrapper function and push it to output string
                let wrapper = generate_wrapper(ident, field_type, &api_fn_name);
                output_string.push_str(&wrapper);
            }

            output_string.push('\n');
        }

        #[allow(unused_mut)]
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(out_path)
            .unwrap_or_else(|_| panic!("Could not write to {:?}", out_path));

        #[cfg(not(feature = "loadable_extension"))]
        // the generated bindings have already been through rustfmt, just write them out
        file.write_all(output_string.as_bytes())
            .unwrap_or_else(|_| panic!("Could not write to {:?}", out_path));
        #[cfg(feature = "loadable_extension")]
        write_with_rustfmt(file, output_string) // if we have generated loadable_extension bindings, pipe them through rustfmt as we write them out
            .unwrap_or_else(|e| panic!("Could not rustfmt output to {:?}: {:?}", out_path, e));
    }

    #[cfg(feature = "loadable_extension")]
    fn write_with_rustfmt(mut file: std::fs::File, output: String) -> Result<(), String> {
        // pipe generated bindings through rustfmt
        let rustfmt =
            which::which("rustfmt").map_err(|e| format!("rustfmt not on PATH: {:?}", e))?;
        let mut cmd = std::process::Command::new(rustfmt);
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        let mut rustfmt_child = cmd
            .spawn()
            .map_err(|e| format!("failed to execute rustfmt: {:?}", e))?;
        let mut rustfmt_child_stdin = rustfmt_child
            .stdin
            .take()
            .ok_or("failed to take rustfmt stdin")?;
        let mut rustfmt_child_stdout = rustfmt_child
            .stdout
            .take()
            .ok_or("failed to take rustfmt stdout")?;

        // spawn a thread to write output string to rustfmt stdin
        let stdin_handle = ::std::thread::spawn(move || {
            let _ = rustfmt_child_stdin.write_all(output.as_bytes());
            output
        });

        // read stdout of rustfmt and write it to bindings file at out_path
        std::io::copy(&mut rustfmt_child_stdout, &mut file)
            .map_err(|e| format!("failed to write to rustfmt stdin: {:?}", e))?;

        let status = rustfmt_child
            .wait()
            .map_err(|e| format!("failed to wait for rustfmt to complete: {:?}", e))?;
        stdin_handle
            .join()
            .map_err(|e| format!("unexpected error: failed to join rustfmt stdin: {:?}", e))?;

        match status.code() {
            Some(0) => {}
            Some(2) => {
                return Err("rustfmt parsing error".to_string());
            }
            Some(3) => {
                return Err("rustfmt could not format some lines.".to_string());
            }
            _ => {
                return Err("Internal rustfmt error".to_string());
            }
        }
        Ok(())
    }

    #[cfg(feature = "loadable_extension")]
    fn get_struct_by_name(bindgen_sources: &str, name: &str) -> Option<syn::ItemStruct> {
        let file = syn::parse_file(&bindgen_sources).expect("unable to parse early bindgen output");

        for item in &file.items {
            if let syn::Item::Struct(s) = item {
                if s.ident == name {
                    return Some(s.to_owned());
                }
            }
        }
        None
    }

    #[cfg(feature = "loadable_extension")]
    fn bare_fn_from_type_path(t: &syn::Type) -> syn::TypeBareFn {
        let path = match t {
            syn::Type::Path(tp) => &tp.path,
            _ => {
                panic!("type was not a type path");
            }
        };

        let mut path_args: Option<syn::PathArguments> = None;
        for segment in &path.segments {
            if segment.arguments.is_empty() {
                continue;
            }
            path_args = Some(segment.arguments.to_owned());
            break;
        }
        match path_args {
            Some(syn::PathArguments::AngleBracketed(p)) => {
                for gen_arg in p.args {
                    match gen_arg {
                        syn::GenericArgument::Type(syn::Type::BareFn(bf)) => {
                            return bf;
                        }
                        _ => {
                            panic!("parsed type was not a bare function as expected");
                        }
                    };
                }
            }
            _ => {
                panic!("parsed path args were not angle bracketed as expected");
            }
        };
        panic!("unexpected failure to parse bare function");
    }

    #[cfg(feature = "loadable_extension")]
    fn generate_varargs_input_idents(
        field_ident: &syn::Ident,
        bare_fn: &syn::TypeBareFn,
        var_arg_types: &[&syn::Type],
    ) -> syn::punctuated::Punctuated<syn::BareFnArg, syn::token::Comma> {
        use syn::Token;
        let mut api_fn_inputs = bare_fn.inputs.clone();
        for (index, var_arg_type) in var_arg_types.iter().enumerate() {
            let mut input = api_fn_inputs[api_fn_inputs.len() - 1].clone();
            let input_ident = syn::Ident::new(&format!("vararg{}", index + 1), field_ident.span());
            let colon = Token![:](field_ident.span());
            input.name = Some((input_ident, colon));
            input.ty = (*var_arg_type).to_owned();
            api_fn_inputs.push(input);
        }
        api_fn_inputs
    }

    #[cfg(feature = "loadable_extension")]
    fn generate_wrapper(
        field_ident: &syn::Ident,
        syn_type: &syn::Type,
        api_fn_name: &str,
    ) -> String {
        use quote::quote;
        use std::collections::BTreeMap;

        let field_name = field_ident.to_string();

        // add wrapper macro invocation to be appended to the generated bindings
        let bare_fn = bare_fn_from_type_path(syn_type);
        let api_fn_output = &bare_fn.output;

        // a map of wrapper function names to function inputs vectors
        let mut wrapper_fn_inputs_map: BTreeMap<
            String,
            syn::punctuated::Punctuated<syn::BareFnArg, syn::token::Comma>,
        > = BTreeMap::new();

        // always generate a wrapper function of the same name as the api function name with no variadic arguments
        wrapper_fn_inputs_map.insert(
            api_fn_name.to_string(),
            generate_varargs_input_idents(field_ident, &bare_fn, &[]),
        );

        // handle variadic api functions by generating additional bindings for specific sets of method arguments that we support
        if bare_fn.variadic.is_some() {
            let const_c_char_type: syn::Type = syn::parse2(quote!(*const ::std::os::raw::c_char))
                .expect("failed to parse c_char type");
            let mut_void_type: syn::Type =
                syn::parse2(quote!(*mut ::core::ffi::c_void)).expect("failed to parse c_char type");
            let c_int_type: syn::Type =
                syn::parse2(quote!(::std::os::raw::c_int)).expect("failed to parse c_int type");
            let mut_c_int_type: syn::Type = syn::parse2(quote!(*mut ::std::os::raw::c_int))
                .expect("failed to parse mutable c_int reference");
            // until rust c_variadic support exists, we can't
            // transparently wrap variadic api functions.
            // generate specific set of args in place of
            // variadic for each function we care about.
            // functions we don't handle will have
            match api_fn_name {
                "sqlite3_db_config" => {
                    // https://sqlite.org/c3ref/c_dbconfig_defensive.html
                    wrapper_fn_inputs_map.insert(
                        "sqlite3_db_config_constchar".to_string(),
                        generate_varargs_input_idents(field_ident, &bare_fn, &[&const_c_char_type]),
                    ); // used for SQLITE_DBCONFIG_MAINDBNAME
                    wrapper_fn_inputs_map.insert(
                        "sqlite3_db_config_void_int_mutint".to_string(),
                        generate_varargs_input_idents(
                            field_ident,
                            &bare_fn,
                            &[&mut_void_type, &c_int_type, &mut_c_int_type],
                        ),
                    ); // used for SQLITE_DBCONFIG_LOOKASIDE
                    wrapper_fn_inputs_map.insert(
                        "sqlite3_db_config_int_mutint".to_string(),
                        generate_varargs_input_idents(
                            field_ident,
                            &bare_fn,
                            &[&c_int_type, &mut_c_int_type],
                        ),
                    ); // used for all other configuration verbs
                }
                "sqlite3_vtab_config" => {
                    // https://sqlite.org/c3ref/c_vtab_constraint_support.html
                    wrapper_fn_inputs_map.insert(
                        "sqlite3_vtab_config_int".to_string(),
                        generate_varargs_input_idents(field_ident, &bare_fn, &[&c_int_type]),
                    ); // used for SQLITE_VTAB_CONSTRAINT_SUPPORT
                }
                _ => {}
            };
        }

        let mut wrappers = String::new();
        for (api_fn_name, api_fn_inputs) in wrapper_fn_inputs_map {
            let api_fn_ident = syn::Ident::new(&api_fn_name, field_ident.span());

            // get identifiers for each of the inputs to use in the api call
            let api_fn_input_idents: Vec<syn::Ident> = (&api_fn_inputs)
                .into_iter()
                .map(|input| match &input.name {
                    Some((ident, _)) => ident.to_owned(),
                    _ => {
                        panic!("Input has no name {:#?}", input);
                    }
                })
                .collect();

            // generate wrapper and return it as a string
            let wrapper_tokens = quote! {
                pub unsafe fn #api_fn_ident(#api_fn_inputs) #api_fn_output {
                    if sqlite3_api.is_null() {
                        panic!("sqlite3_api is null");
                    }
                    ((*sqlite3_api).#field_ident
                        .expect(stringify!("sqlite3_api contains null pointer for ", #field_name, " function")))(
                            #(#api_fn_input_idents),*
                    )
                }
            };
            wrappers.push_str(&format!("{}\n\n", wrapper_tokens.to_string()));
        }
        wrappers
    }
}
