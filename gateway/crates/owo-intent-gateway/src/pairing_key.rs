//! User-scope DPAPI key wrapping primitives. No persistence or enrollment endpoint.
//! DPAPI does not protect against arbitrary code already running as the same user.
use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{
        BCryptGenRandom, CryptProtectData, CryptUnprotectData, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    },
};
use zeroize::{Zeroize, Zeroizing};
const MAX_WRAPPED_BYTES: usize = 16 * 1024;

/// Process-local random key. Never use model/API credentials as pairing keys.
pub fn generate_key() -> io::Result<Zeroizing<[u8; 32]>> {
    let mut key = Zeroizing::new([0u8; 32]);
    if unsafe {
        BCryptGenRandom(
            ptr::null_mut(),
            key.as_mut_ptr(),
            32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    } < 0
        || *key == [0; 32]
    {
        return Err(io::Error::other("pairing randomness unavailable"));
    }
    Ok(key)
}
// No Debug/Serialize: encrypted material is not normal diagnostic output either.
pub struct WrappedPairingKey(Vec<u8>);
struct Output(CRYPT_INTEGER_BLOB);
impl Drop for Output {
    fn drop(&mut self) {
        if !self.0.pbData.is_null() {
            unsafe {
                std::slice::from_raw_parts_mut(self.0.pbData, self.0.cbData as usize).zeroize();
                LocalFree(self.0.pbData.cast());
            }
        }
    }
}
fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr().cast_mut(),
    }
}
fn entropy(id: &[u8; 32]) -> io::Result<Vec<u8>> {
    if id == &[0; 32] {
        return Err(io::Error::other("invalid pairing identifier"));
    }
    let mut value = b"OwO/IME/pairing-key/v1\0".to_vec();
    value.extend_from_slice(id);
    Ok(value)
}
impl WrappedPairingKey {
    /// User scope only: never set CRYPTPROTECT_LOCAL_MACHINE. Caller must enforce
    /// file ACLs and trusted enrollment when eventually persisting this blob.
    pub fn protect(key: &Zeroizing<[u8; 32]>, id: &[u8; 32]) -> io::Result<Self> {
        if **key == [0; 32] {
            return Err(io::Error::other("invalid pairing key"));
        }
        let input = blob(key.as_ref());
        let context = entropy(id)?;
        let extra = blob(&context);
        let mut out = Output(CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        });
        if unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                &extra,
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out.0,
            )
        } == 0
        {
            return Err(io::Error::other("pairing key protection failed"));
        }
        if out.0.pbData.is_null() || out.0.cbData == 0 || out.0.cbData as usize > MAX_WRAPPED_BYTES
        {
            return Err(io::Error::other("invalid protected pairing key"));
        }
        Ok(Self(
            unsafe { std::slice::from_raw_parts(out.0.pbData, out.0.cbData as usize) }.to_vec(),
        ))
    }
    pub fn from_encrypted_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_WRAPPED_BYTES {
            return Err(io::Error::other("invalid protected pairing key size"));
        }
        Ok(Self(bytes.to_vec()))
    }
    pub fn encrypted_bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn unprotect(&self, id: &[u8; 32]) -> io::Result<Zeroizing<[u8; 32]>> {
        let input = blob(&self.0);
        let context = entropy(id)?;
        let extra = blob(&context);
        let mut out = Output(CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        });
        if unsafe {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                &extra,
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out.0,
            )
        } == 0
        {
            return Err(io::Error::other("pairing key unprotection failed"));
        }
        if out.0.pbData.is_null() || out.0.cbData != 32 {
            return Err(io::Error::other("invalid unprotected pairing key"));
        }
        let mut key = Zeroizing::new([0u8; 32]);
        unsafe {
            ptr::copy_nonoverlapping(out.0.pbData, key.as_mut_ptr(), 32);
        }
        if *key == [0; 32] {
            return Err(io::Error::other("invalid pairing key"));
        }
        Ok(key)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn random_ephemeral_keys_use_os_entropy_without_printing_secrets() {
        let a = generate_key().unwrap();
        let b = generate_key().unwrap();
        assert!(*a != [0; 32]);
        assert!(*a != *b);
    }
    #[test]
    fn user_scope_roundtrip_and_pairing_swap_tampering_rejected() {
        let key = Zeroizing::new([7; 32]);
        let id = [1; 32];
        let wrapped = WrappedPairingKey::protect(&key, &id).unwrap();
        assert!(wrapped.encrypted_bytes() != key.as_ref());
        let restored = WrappedPairingKey::from_encrypted_bytes(wrapped.encrypted_bytes()).unwrap();
        let result = restored.unprotect(&id).unwrap();
        assert!(*result == *key);
        assert!(restored.unprotect(&[2; 32]).is_err());
        let mut corrupt = wrapped.encrypted_bytes().to_vec();
        let end = corrupt.len() - 1;
        corrupt[end] ^= 1;
        assert!(WrappedPairingKey::from_encrypted_bytes(&corrupt)
            .unwrap()
            .unprotect(&id)
            .is_err());
    }
    #[test]
    fn invalid_material_refused_before_os_calls() {
        assert!(WrappedPairingKey::from_encrypted_bytes(&[]).is_err());
        assert!(WrappedPairingKey::from_encrypted_bytes(&vec![0; MAX_WRAPPED_BYTES + 1]).is_err());
        assert!(WrappedPairingKey::protect(&Zeroizing::new([0; 32]), &[1; 32]).is_err());
        assert!(WrappedPairingKey::protect(&Zeroizing::new([7; 32]), &[0; 32]).is_err());
        assert!(
            WrappedPairingKey::from_encrypted_bytes(b"synthetic invalid blob")
                .unwrap()
                .unprotect(&[1; 32])
                .is_err()
        );
    }
}
