use core::ffi::CStr;

pub trait SqlcipherCryptoProvider {
    fn get_provider_name(&self) -> &CStr;

    fn add_random(&mut self, _buffer: &[u8]) -> i32;
    fn random(&mut self, _buffer: &mut [u8]) -> i32;

    fn hmac(
        &mut self,
        algorithm: i32,
        hmac_key: &[u8],
        input1: &[u8],
        input2: &[u8],
        out: &mut [u8],
    ) -> i32;

    fn kdf(
        &mut self,
        algorithm: i32,
        pass: &[u8],
        salt: &[u8],
        workfactor: i32,
        key: &mut [u8],
    ) -> i32;

    fn cipher(&mut self, mode: i32, key: &[u8], iv: &[u8], input: &[u8], out: &mut [u8]) -> i32;

    fn get_cipher(&self) -> &CStr;

    fn get_key_sz(&self) -> i32;
    fn get_iv_sz(&self) -> i32;
    fn get_block_sz(&self) -> i32;
    fn get_hmac_sz(&self, algorithm: i32) -> i32;

    fn fips_status(&self) -> i32;

    fn get_provider_version(&self) -> &CStr;
}

#[derive(Default)]
pub struct RustCryptoProvider;

impl SqlcipherCryptoProvider for RustCryptoProvider {
    fn get_provider_name(&self) -> &CStr {
        todo!()
    }

    fn add_random(&mut self, _buffer: &[u8]) -> i32 {
        todo!()
    }

    fn random(&mut self, buffer: &mut [u8]) -> i32 {
        rand::fill(buffer);
        0
    }

    fn hmac(
        &mut self,
        _algorithm: i32,
        _hmac_key: &[u8],
        _input1: &[u8],
        _input2: &[u8],
        _out: &mut [u8],
    ) -> i32 {
        todo!()
    }

    fn kdf(
        &mut self,
        _algorithm: i32,
        _pass: &[u8],
        _salt: &[u8],
        _workfactor: i32,
        _key: &mut [u8],
    ) -> i32 {
        todo!()
    }

    fn cipher(
        &mut self,
        _mode: i32,
        _key: &[u8],
        _iv: &[u8],
        _input: &[u8],
        _out: &mut [u8],
    ) -> i32 {
        todo!()
    }

    fn get_cipher(&self) -> &CStr {
        todo!()
    }

    fn get_key_sz(&self) -> i32 {
        todo!()
    }

    fn get_iv_sz(&self) -> i32 {
        todo!()
    }

    fn get_block_sz(&self) -> i32 {
        todo!()
    }

    fn get_hmac_sz(&self, _algorithm: i32) -> i32 {
        todo!()
    }

    fn fips_status(&self) -> i32 {
        todo!()
    }

    fn get_provider_version(&self) -> &CStr {
        todo!()
    }
}
