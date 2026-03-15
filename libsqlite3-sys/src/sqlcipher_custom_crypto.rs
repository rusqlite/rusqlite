use alloc::boxed::Box;
use core::ffi::{c_int, c_void};

use sqlcipher_crypto_provider::{
    HmacAlgorithm, KdfAlgorithm, RustCryptoProvider, SqlcipherCryptoProvider,
};

use super::bindings::*;

#[unsafe(no_mangle)]
pub extern "C" fn rusqlite_custom_crypto_setup(_p: *mut sqlcipher_provider) {
    let p = unsafe { &mut *_p };
    // int (*init)(void);
    p.init = Some(init);
    // void (*shutdown)(void);
    p.shutdown = Some(shutdown);
    // const char* (*get_provider_name)(void *ctx);
    p.get_provider_name = Some(get_provider_name);
    // int (*add_random)(void *ctx, const void *buffer, int length);
    p.add_random = Some(add_random);
    // int (*random)(void *ctx, void *buffer, int length);
    p.random = Some(random);
    // int (*hmac)(void *ctx, int algorithm,
    //             const unsigned char *hmac_key, int key_sz,
    //             const unsigned char *in, int in_sz,
    //             const unsigned char *in2, int in2_sz,
    //             unsigned char *out);
    p.hmac = Some(hmac);
    // int (*kdf)(void *ctx, int algorithm,
    //             const unsigned char *pass, int pass_sz,
    //             const unsigned char* salt, int salt_sz,
    //             int workfactor,
    //             int key_sz, unsigned char *key);
    p.kdf = Some(kdf);
    // int (*cipher)(void *ctx, int mode,
    //             const unsigned char *key, int key_sz,
    //             const unsigned char *iv,
    //             const unsigned char *in, int in_sz,
    //             unsigned char *out);
    p.cipher = Some(cipher);
    // const char* (*get_cipher)(void *ctx);
    p.get_cipher = Some(get_cipher);
    // int (*get_key_sz)(void *ctx);
    p.get_key_sz = Some(get_key_sz);
    // int (*get_iv_sz)(void *ctx);
    p.get_iv_sz = Some(get_iv_sz);
    // int (*get_block_sz)(void *ctx);
    p.get_block_sz = Some(get_block_sz);
    // int (*get_hmac_sz)(void *ctx, int algorithm);
    p.get_hmac_sz = Some(get_hmac_sz);
    // int (*ctx_init)(void **ctx);
    p.ctx_init = Some(ctx_init);
    // int (*ctx_free)(void **ctx);
    p.ctx_free = Some(ctx_free);
    // int (*fips_status)(void *ctx);
    p.fips_status = Some(fips_status);
    // const char* (*get_provider_version)(void *ctx);
    p.get_provider_version = Some(get_provider_version);
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

fn parse_hmac_algorithm(algorithm: i32) -> HmacAlgorithm {
    match algorithm {
        SQLCIPHER_HMAC_SHA1 => HmacAlgorithm::HmacSha1,
        SQLCIPHER_HMAC_SHA256 => HmacAlgorithm::HmacSha256,
        SQLCIPHER_HMAC_SHA512 => HmacAlgorithm::HmacSha512,
        _ => unimplemented!("unimplemented SQLCIPHER_HMAC algorithm {algorithm}"),
    }
}

fn parse_kdf_algorithm(algorithm: i32) -> KdfAlgorithm {
    match algorithm {
        SQLCIPHER_PBKDF2_HMAC_SHA1 => KdfAlgorithm::Pbkdf2HmacSha1,
        SQLCIPHER_PBKDF2_HMAC_SHA256 => KdfAlgorithm::Pbkdf2HmacSha256,
        SQLCIPHER_PBKDF2_HMAC_SHA512 => KdfAlgorithm::Pbkdf2HmacSha512,
        _ => unimplemented!("unimplemented SQLCIPHER KDF algorithm {algorithm}"),
    }
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

extern "C" fn get_key_sz(ctx: *mut c_void) -> i32 {
    let obj = ctx_to_trait_mut(ctx);
    obj.get_key_sz()
}

extern "C" fn get_provider_name(ctx: *mut c_void) -> *const i8 {
    let obj = ctx_to_trait_mut(ctx);
    obj.get_provider_name().as_ptr()
}

// int (*kdf)(void *ctx, int algorithm,
//             const unsigned char *pass, int pass_sz,
//             const unsigned char* salt, int salt_sz,
//             int workfactor,
//             int key_sz, unsigned char *key);

extern "C" fn kdf(
    ctx: *mut c_void,
    algorithm: c_int,
    pass: *const u8,
    pass_sz: c_int,
    salt: *const u8,
    salt_sz: c_int,
    workfactor: c_int,
    key_sz: c_int,
    key: *mut u8,
) -> i32 {
    let obj = ctx_to_trait_mut(ctx);
    let algorithm = parse_kdf_algorithm(algorithm);
    let pass = unsafe { core::slice::from_raw_parts(pass as *const u8, pass_sz as usize) };
    let salt = unsafe { core::slice::from_raw_parts(salt as *const u8, salt_sz as usize) };
    let key = unsafe { core::slice::from_raw_parts_mut(key as *mut u8, key_sz as usize) };

    obj.kdf(algorithm, pass, salt, workfactor, key)
}

extern "C" fn hmac(
    ctx: *mut c_void,
    algorithm: c_int,
    hmac_key: *const u8,
    key_sz: c_int,
    input: *const u8,
    in_size: c_int,
    input2: *const u8,
    in2_size: c_int,
    output: *mut u8,
) -> i32 {
    let obj = ctx_to_trait_mut(ctx);

    let algorithm = parse_hmac_algorithm(algorithm);
    let hmac_key = unsafe { core::slice::from_raw_parts(hmac_key as *const u8, key_sz as usize) };
    let input = unsafe { core::slice::from_raw_parts(input as *const u8, in_size as usize) };
    let input2 = if input2.is_null() {
        None
    } else {
        Some(unsafe { core::slice::from_raw_parts(input2 as *const u8, in2_size as usize) })
    };

    let out = unsafe { core::slice::from_raw_parts_mut(output as *mut u8, algorithm.output_len()) };

    obj.hmac(algorithm, hmac_key, input, input2, out)
}

extern "C" fn cipher(
    ctx: *mut c_void,
    mode: c_int,
    key: *const u8,
    key_sz: c_int,
    iv: *const u8,
    input: *const u8,
    in_size: c_int,
    output: *mut u8,
) -> i32 {
    let obj = ctx_to_trait_mut(ctx);
    let key = unsafe { core::slice::from_raw_parts(key as *const u8, key_sz as usize) };
    let iv = unsafe { core::slice::from_raw_parts(iv as *const u8, obj.get_iv_sz() as usize) };
    let input = unsafe { core::slice::from_raw_parts(input as *const u8, in_size as usize) };

    let out = unsafe { core::slice::from_raw_parts_mut(output as *mut u8, in_size as usize) };
    match mode {
        // XXX for some reason, SQLCIPHER_ENCRYPT/SQLCIPHER_DECRYPT aren't in bindings.
        0 => {
            // Decrypt
            obj.decrypt(key, iv, input, out)
        }
        1 => {
            // Encrypt
            obj.encrypt(key, iv, input, out)
        }
        _ => {
            unimplemented!("unsupported cipher mode")
        }
    }
}
// int (*cipher)(void *ctx, int mode,
//             const unsigned char *key, int key_sz,
//             const unsigned char *iv,
//             const unsigned char *in, int in_sz,
//             unsigned char *out);

extern "C" fn get_cipher(ctx: *mut c_void) -> *const i8 {
    let obj = ctx_to_trait_mut(ctx);
    obj.get_cipher().as_ptr()
}

extern "C" fn get_iv_sz(ctx: *mut c_void) -> i32 {
    let obj = ctx_to_trait_mut(ctx);
    obj.get_iv_sz()
}

extern "C" fn get_block_sz(ctx: *mut c_void) -> i32 {
    let obj = ctx_to_trait_mut(ctx);
    obj.get_block_sz()
}

extern "C" fn get_hmac_sz(ctx: *mut c_void, algorithm: c_int) -> i32 {
    let obj = ctx_to_trait_mut(ctx);
    obj.get_hmac_sz(parse_hmac_algorithm(algorithm))
}

extern "C" fn get_provider_version(ctx: *mut c_void) -> *const i8 {
    let obj = ctx_to_trait_mut(ctx);
    obj.get_provider_version().as_ptr()
}

extern "C" fn fips_status(ctx: *mut c_void) -> i32 {
    let obj = ctx_to_trait_mut(ctx);
    obj.fips_status()
}

// int (*random)(void *ctx, void *buffer, int length);
extern "C" fn random(ctx: *mut c_void, buffer: *mut c_void, length: c_int) -> i32 {
    let obj = ctx_to_trait_mut(ctx);

    let buffer = unsafe { core::slice::from_raw_parts_mut(buffer as *mut u8, length as usize) };
    obj.random(buffer)
}

extern "C" fn add_random(ctx: *mut c_void, buffer: *const c_void, length: c_int) -> i32 {
    let obj = ctx_to_trait_mut(ctx);

    let buffer = unsafe { core::slice::from_raw_parts(buffer as *mut u8, length as usize) };
    obj.add_random(buffer)
}
