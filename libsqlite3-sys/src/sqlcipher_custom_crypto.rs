use alloc::boxed::Box;
use core::ffi::{c_int, c_void};

use sqlcipher_crypto_provider::{RustCryptoProvider, SqlcipherCryptoProvider};

use super::bindings::*;

#[unsafe(no_mangle)]
pub extern "C" fn rusqlite_custom_crypto_setup(_p: *mut sqlcipher_provider) {
    let p = unsafe { &mut *_p };
    // int (*init)(void);
    p.init = Some(init);
    // void (*shutdown)(void);
    p.shutdown = Some(shutdown);
    // const char* (*get_provider_name)(void *ctx);
    // int (*add_random)(void *ctx, const void *buffer, int length);
    // int (*random)(void *ctx, void *buffer, int length);
    p.random = Some(random);
    // int (*hmac)(void *ctx, int algorithm,
    //             const unsigned char *hmac_key, int key_sz,
    //             const unsigned char *in, int in_sz,
    //             const unsigned char *in2, int in2_sz,
    //             unsigned char *out);
    // int (*kdf)(void *ctx, int algorithm,
    //             const unsigned char *pass, int pass_sz,
    //             const unsigned char* salt, int salt_sz,
    //             int workfactor,
    //             int key_sz, unsigned char *key);
    // int (*cipher)(void *ctx, int mode,
    //             const unsigned char *key, int key_sz,
    //             const unsigned char *iv,
    //             const unsigned char *in, int in_sz,
    //             unsigned char *out);
    // const char* (*get_cipher)(void *ctx);
    // int (*get_key_sz)(void *ctx);
    // int (*get_iv_sz)(void *ctx);
    // int (*get_block_sz)(void *ctx);
    // int (*get_hmac_sz)(void *ctx, int algorithm);
    // int (*ctx_init)(void **ctx);
    p.ctx_init = Some(ctx_init);
    // int (*ctx_free)(void **ctx);
    p.ctx_free = Some(ctx_free);
    // int (*fips_status)(void *ctx);
    // const char* (*get_provider_version)(void *ctx);
    // sqlcipher_provider *next;
    p.next = core::ptr::null_mut();
}

extern "C" fn init() -> i32 {
    0
}
extern "C" fn shutdown() {}

fn ctx_to_trait_mut<'a>(ctx: *mut c_void) -> &'a mut Box<dyn SqlcipherCryptoProvider> {
    let obj = ctx as *mut Box<dyn SqlcipherCryptoProvider>;

    unsafe { obj.as_mut().expect("non-null crypto provider") }
}

extern "C" fn ctx_init(ctx: *mut *mut c_void) -> i32 {
    let ctx_ref: &mut *mut c_void = unsafe { &mut *ctx };
    let obj = Box::new(Box::new(RustCryptoProvider::default()) as Box<dyn SqlcipherCryptoProvider>);
    let obj: *mut Box<dyn SqlcipherCryptoProvider> = Box::into_raw(obj);
    *ctx_ref = obj as *mut c_void;

    0
}

extern "C" fn ctx_free(ctx: *mut *mut c_void) -> i32 {
    let _obj = unsafe {
        let obj = *ctx as *mut Box<dyn SqlcipherCryptoProvider>;

        Box::<_>::from_raw(obj)
    };
    drop(_obj);

    0
}

// int (*random)(void *ctx, void *buffer, int length);
extern "C" fn random(ctx: *mut c_void, buffer: *mut c_void, length: c_int) -> i32 {
    let obj = ctx_to_trait_mut(ctx);

    let buffer = unsafe { core::slice::from_raw_parts_mut(buffer as *mut u8, length as usize) };
    obj.random(buffer)
}
