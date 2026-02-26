use core::ffi::CStr;

use aes::cipher::{
    BlockDecryptMut, BlockEncryptMut, BlockSizeUser, IvSizeUser, KeyIvInit, KeySizeUser,
    block_padding::NoPadding,
};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use sha1::Sha1;
use sha2::{Sha256, Sha512};

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum HmacAlgorithm {
    HmacSha1,
    HmacSha256,
    HmacSha512,
}

impl HmacAlgorithm {
    pub const fn output_len(&self) -> usize {
        match self {
            HmacAlgorithm::HmacSha1 => 20,
            HmacAlgorithm::HmacSha256 => 32,
            HmacAlgorithm::HmacSha512 => 64,
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum KdfAlgorithm {
    Pbkdf2HmacSha1,
    Pbkdf2HmacSha256,
    Pbkdf2HmacSha512,
}

impl KdfAlgorithm {}

pub trait SqlcipherCryptoProvider {
    // TODO: should the lifetime of the return value be limited to 'static?
    fn get_provider_name(&self) -> &CStr;

    fn add_random(&mut self, buffer: &[u8]) -> i32;
    fn random(&mut self, buffer: &mut [u8]) -> i32;

    fn hmac(
        &mut self,
        algorithm: HmacAlgorithm,
        hmac_key: &[u8],
        input1: &[u8],
        input2: Option<&[u8]>,
        out: &mut [u8],
    ) -> i32;

    fn kdf(
        &mut self,
        algorithm: KdfAlgorithm,
        pass: &[u8],
        salt: &[u8],
        workfactor: i32,
        key: &mut [u8],
    ) -> i32;

    fn encrypt(&mut self, key: &[u8], iv: &[u8], input: &[u8], out: &mut [u8]) -> i32;
    fn decrypt(&mut self, key: &[u8], iv: &[u8], input: &[u8], out: &mut [u8]) -> i32;

    // TODO: should the lifetime of the return value be limited to 'static?
    fn get_cipher(&self) -> &CStr;

    fn get_key_sz(&self) -> i32;
    fn get_iv_sz(&self) -> i32;
    fn get_block_sz(&self) -> i32;
    fn get_hmac_sz(&self, algorithm: HmacAlgorithm) -> i32;

    fn fips_status(&self) -> i32;

    // TODO: should the lifetime of the return value be limited to 'static?
    fn get_provider_version(&self) -> &CStr;
}

#[derive(Default)]
pub struct RustCryptoProvider;

impl SqlcipherCryptoProvider for RustCryptoProvider {
    fn get_provider_name(&self) -> &CStr {
        c"RustCrypto"
    }

    fn add_random(&mut self, _buffer: &[u8]) -> i32 {
        // We discard the randomness.
        // `ThreadRng` from `rand` has no API to add seed data, and it's fast and secure enough on
        // its own.
        0
    }

    fn random(&mut self, buffer: &mut [u8]) -> i32 {
        rand::fill(buffer);
        0
    }

    fn hmac(
        &mut self,
        algorithm: HmacAlgorithm,
        _hmac_key: &[u8],
        input1: &[u8],
        input2: Option<&[u8]>,
        out: &mut [u8],
    ) -> i32 {
        match algorithm {
            HmacAlgorithm::HmacSha1 => {
                let mut mac = Hmac::<Sha1>::new_from_slice(_hmac_key).expect("hmac from any size");
                mac.update(input1);
                if let Some(input) = input2 {
                    mac.update(input);
                }
                let result = mac.finalize().into_bytes();
                out.copy_from_slice(&result);
            }
            HmacAlgorithm::HmacSha256 => {
                let mut mac =
                    Hmac::<Sha256>::new_from_slice(_hmac_key).expect("hmac from any size");
                mac.update(input1);
                if let Some(input) = input2 {
                    mac.update(input);
                }
                let result = mac.finalize().into_bytes();
                out.copy_from_slice(&result);
            }
            HmacAlgorithm::HmacSha512 => {
                let mut mac =
                    Hmac::<Sha512>::new_from_slice(_hmac_key).expect("hmac from any size");
                mac.update(input1);
                if let Some(input) = input2 {
                    mac.update(input);
                }
                let result = mac.finalize().into_bytes();
                out.copy_from_slice(&result);
            }
        }

        0
    }

    fn kdf(
        &mut self,
        algorithm: KdfAlgorithm,
        password: &[u8],
        salt: &[u8],
        n: i32,
        key: &mut [u8],
    ) -> i32 {
        match algorithm {
            KdfAlgorithm::Pbkdf2HmacSha1 => pbkdf2_hmac::<Sha1>(password, salt, n as u32, key),
            KdfAlgorithm::Pbkdf2HmacSha256 => pbkdf2_hmac::<Sha256>(password, salt, n as u32, key),
            KdfAlgorithm::Pbkdf2HmacSha512 => pbkdf2_hmac::<Sha512>(password, salt, n as u32, key),
        }

        0
    }

    fn decrypt(&mut self, key: &[u8], iv: &[u8], input: &[u8], out: &mut [u8]) -> i32 {
        let dec = Aes256CbcDec::new(key.into(), iv.into());
        if let Err(_e) = dec.decrypt_padded_b2b_mut::<NoPadding>(input, out) {
            out.fill(0);
            return -1;
        }

        0
    }

    fn encrypt(&mut self, key: &[u8], iv: &[u8], input: &[u8], out: &mut [u8]) -> i32 {
        let dec = Aes256CbcEnc::new(key.into(), iv.into());
        if let Err(_e) = dec.encrypt_padded_b2b_mut::<NoPadding>(input, out) {
            out.fill(0);
            return -1;
        }

        0
    }

    fn get_cipher(&self) -> &CStr {
        c"aes256-CBC"
    }

    fn get_key_sz(&self) -> i32 {
        Aes256CbcEnc::key_size() as i32
    }

    fn get_iv_sz(&self) -> i32 {
        Aes256CbcEnc::iv_size() as i32
    }

    fn get_block_sz(&self) -> i32 {
        Aes256CbcEnc::block_size() as i32
    }

    fn get_hmac_sz(&self, algorithm: HmacAlgorithm) -> i32 {
        algorithm.output_len() as i32
    }

    fn fips_status(&self) -> i32 {
        -1
    }

    fn get_provider_version(&self) -> &CStr {
        // env!("CARGO_PKG_VERSION").into()
        c"0.1.0"
    }
}
