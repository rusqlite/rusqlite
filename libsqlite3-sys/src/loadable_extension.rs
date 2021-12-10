use crate::sqlite3_api_routines;
use std::sync::Once;

#[cfg(feature = "loadable_extension_embedded")]
// bindings were built with loadable_extension_embedded:
// define sqlite3_api as an extern since this code will be embedded
// within a loadable extension that defines and exports this itself.
//
// this extern static is immutable so it MUST be set by the embedding
// extension before any of the rust code executes.
extern "C" {
    pub static sqlite3_api: *const sqlite3_api_routines;
}

#[cfg(not(feature = "loadable_extension_embedded"))]
// bindings were built with (non-embedded) loadable_extension:
// we define our own (i.e. not extern) sqlite_api static
// variable and export it publicly so that it is included in
// our FFI (C) interface, in case we are the host to any
// embedded extensions.
//
// This is public only so that it is exported in our library
// for use by any extensions that we embed.
//
// It should not be accessed by any rust code.
#[no_mangle]
pub static mut sqlite3_api: *mut sqlite3_api_routines = std::ptr::null_mut();

// Sqlite3ApiRoutines wraps access to the sqlite3_api_routines provided
// to the loadable extension.
struct Sqlite3ApiRoutines {
    p_api: *mut sqlite3_api_routines,
}

impl Sqlite3ApiRoutines {
    const UNINIT: Self = Sqlite3ApiRoutines {
        p_api: std::ptr::null_mut(),
    };

    #[inline]
    fn get(&self) -> *const sqlite3_api_routines {
        self.get_mut() as *const sqlite3_api_routines
    }

    #[inline]
    fn get_mut(&self) -> *mut sqlite3_api_routines {
        if self.p_api.is_null() {
            panic!("attempted to access Sqlite3ApiRoutines that was not initialized, please ensure you have called loadable_extension_init prior to attempting to use any API functions from a loadable extension");
        }
        self.p_api
    }
}

static SQLITE3_API_ONCE: Once = Once::new();
static mut SQLITE3_API: Sqlite3ApiRoutines = Sqlite3ApiRoutines::UNINIT;

/// Access the raw pointer to the sqlite3_api_routines after it has been
/// initialized by a call to `loadable_extension_init`.`
///
/// Will panic if an attempt is made to access it prior to initialization.
///
/// # Safety
///
/// This function accesses the mutable static SQLITE3_API which is unsafe,
/// but it is safe provided that SQLITE3_API is only mutated by the
/// `loadable_extension_init` function, which includes a sync::Once guard.
///
/// A call to this function will panic if it is called prior to the
/// extension being initialized by a call to `loadable_extension_init`.
pub fn loadable_extension_sqlite3_api() -> *const sqlite3_api_routines {
    unsafe { SQLITE3_API.get() }
}

#[cfg(not(feature = "loadable_extension_embedded"))]
/// Initialize a (non-embedded) loadable extension
///
/// This function has essentially the same role as the `SQLITE_EXTENSION_INIT2`
/// macro when building a sqlite loadable extension in C.
///
/// It is only available when the `loadable_extension` feature is enabled and
/// the `loadable_extension_embedded` feature is not.
///
/// In general, a sqlite extension that is not embedded should declare a pub
/// extern C function that implements the sqlite extension loading entry point
/// interface (see: https://www.sqlite.org/loadext.html), and that entry point
/// should arrange to call this function before any other rust code is executed.
///
/// An example minimal sqlite extension entrypoint function might be:
/// ```
/// #[no_mangle]
/// pub unsafe extern "C" fn sqlite3_extension_init(
///     db: *mut libsqlite3_sys::sqlite3,
///     pz_err_msg: *mut *mut std::os::raw::c_char,
///     p_api: *mut libsqlite3_sys::sqlite3_api_routines,
/// ) -> std::os::raw::c_int {
///     // SQLITE_EXTENSION_INIT2 equivalent
///     libsqlite3_sys::loadable_extension_init(p_api);
///
///     libsqlite3_sys::SQLITE_OK
/// }
/// ```
///
/// # Safety
///
/// The raw pointer passed in to `p_api` must point to a valid `sqlite3_api_routines`
/// struct.
///
/// The function will panic if a null pointer is provided.
///
/// This function is thread-safe, but only the first invocation will have any effect.
pub unsafe fn loadable_extension_init(init_p_api: *mut sqlite3_api_routines) {
    if init_p_api.is_null() {
        panic!("loadable_extension_init was passed a null pointer");
    }

    // protect the setting of SQLITE3_API with a sync::Once so that it is thread-safe.
    // only the first invocation will have any effect.
    SQLITE3_API_ONCE.call_once(|| {
        SQLITE3_API.p_api = init_p_api;
        // also set sqlite3_api to the provided value to support hosting of embedded extensions of our own
        sqlite3_api = init_p_api;
    });
}

#[cfg(feature = "loadable_extension_embedded")]
/// Initialize an embedded loadable extension
///
/// It is only available when the `loadable_extension_embedded` feature is enabled.
///
/// We rely on the host extension who embeds us (i.e. by linking us in) to
/// implement the sqlite extension entry point itself and set the global symbol
/// `sqlite3_api` to the `sqlite3_api_routines`struct that it was passed from
/// sqlite.  With a C extension, that can be done as usual by invoking the
/// `SQLITE_EXTENSION_INIT2` macro from the entry point as recommended in the
/// sqlite loadable extension docs. If the host extension is another rusqlite
/// loadable extension, it will also set the `sqlite3_api` symbol in this case
/// so that extensions that it embeds will have access to the API routines.
///
/// # Safety
///
/// The host extension must have already populated `sqlite3_api` to point to
/// a valid `sqlite3_api_routines` struct populated by sqlite and passed to it
/// during its own initialisation.
///
/// This function will panic if `sqlite3_api` is a null pointer.
///
/// This function is thread-safe, but only the first invocation will have any effect.
pub unsafe fn loadable_extension_embedded_init() {
    if sqlite3_api.is_null() {
        panic!("loadable_extension_embedded_init was called with a null `sqlite3_api` - the host extension should have set this (for example by invoking `SQLITE_EXTENSION_INIT2` prior to this function being called)");
    }

    // protect the setting of SQLITE3_API with a sync::Once so that it is thread-safe.
    // only the first invocation will have any effect.
    SQLITE3_API_ONCE.call_once(|| {
        SQLITE3_API.p_api = sqlite3_api as *mut sqlite3_api_routines;
    });
}
