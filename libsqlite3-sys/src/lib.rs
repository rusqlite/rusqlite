#![allow(non_snake_case, non_camel_case_types)]
#![cfg_attr(test, allow(deref_nullptr))] // https://github.com/rust-lang/rust-bindgen/issues/2066

// force linking to openssl
#[cfg(feature = "bundled-sqlcipher-vendored-openssl")]
extern crate openssl_sys;

#[cfg(all(windows, feature = "winsqlite3", target_pointer_width = "32"))]
compile_error!("The `libsqlite3-sys/winsqlite3` feature is not supported on 32 bit targets.");

pub use self::error::*;

use std::default::Default;
use std::mem;

mod error;

#[must_use]
pub fn SQLITE_STATIC() -> sqlite3_destructor_type {
    None
}

#[must_use]
pub fn SQLITE_TRANSIENT() -> sqlite3_destructor_type {
    Some(unsafe { mem::transmute(-1_isize) })
}

#[allow(clippy::all)]
mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindgen.rs"));
}
pub use bindings::*;

pub type sqlite3_index_constraint = sqlite3_index_info_sqlite3_index_constraint;
pub type sqlite3_index_constraint_usage = sqlite3_index_info_sqlite3_index_constraint_usage;

impl Default for sqlite3_vtab {
    fn default() -> Self {
        unsafe { mem::zeroed() }
    }
}

impl Default for sqlite3_vtab_cursor {
    fn default() -> Self {
        unsafe { mem::zeroed() }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod allocator {
    use std::alloc::{alloc, dealloc, realloc as rs_realloc, Layout};

    #[no_mangle]
    pub unsafe fn sqlite_malloc(len: usize) -> *mut u8 {
        let align = std::mem::align_of::<usize>();
        let layout = Layout::from_size_align_unchecked(len, align);

        let ptr = alloc(layout);
        ptr
    }

    const SQLITE_PTR_SIZE: usize = 8;

    #[no_mangle]
    pub unsafe fn sqlite_free(ptr: *mut u8) -> i32 {
        let mut size_a = [0; SQLITE_PTR_SIZE];

        size_a.as_mut_ptr().copy_from(ptr, SQLITE_PTR_SIZE);

        let ptr_size: u64 = u64::from_le_bytes(size_a);

        let align = std::mem::align_of::<usize>();
        let layout = Layout::from_size_align_unchecked(ptr_size as usize, align);

        dealloc(ptr, layout);

        0
    }

    #[no_mangle]
    pub unsafe fn sqlite_realloc(ptr: *mut u8, size: usize) -> *mut u8 {
        let align = std::mem::align_of::<usize>();
        let layout = Layout::from_size_align_unchecked(size, align);

        rs_realloc(ptr, layout, size)
    }
}

#[cfg(feature = "sqlite-memvfs")]
mod memvfs;

#[cfg(feature = "sqlite-memvfs")]
mod vfs {
    use log::debug;

    #[no_mangle]
    pub unsafe fn sqlite3_os_init() -> std::os::raw::c_int {
        let mut mem_vfs = Box::new(super::memvfs::get_mem_vfs());

        let mem_vfs_ptr: *mut crate::sqlite3_vfs = mem_vfs.as_mut();
        let rc = crate::sqlite3_vfs_register(mem_vfs_ptr, 1);
        debug!("sqlite3 vfs register result: {}", rc);

        std::mem::forget(mem_vfs);

        rc
    }
}
