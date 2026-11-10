//! Encryption for data at rest with Windows DPAPI (CryptProtectData).
//! Only the signed-in Windows account on this PC can decrypt it; no keys
//! are stored by Atoll. Used for clipboard history when "Encrypt history" is on.
//! https://learn.microsoft.com/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata

use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

/// Suffix of encrypted files.
pub const EXT: &str = "enc";

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
}

unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    unsafe {
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        v
    }
}

pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
    let input = blob(data);
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    let ok = unsafe {
        CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
    };
    if ok == 0 {
        return Err("Windows could not encrypt the data.".into());
    }
    Ok(unsafe { take(out) })
}

pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    let input = blob(data);
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    let ok = unsafe {
        CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
    };
    if ok == 0 {
        return Err("Windows could not decrypt the data (was it saved by another account?).".into());
    }
    Ok(unsafe { take(out) })
}

pub fn is_encrypted(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == EXT)
}

/// Read a file, decrypting it if it is an encrypted one.
pub fn read(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if is_encrypted(path) { unprotect(&bytes) } else { Ok(bytes) }
}

/// Encrypt `path` into `path.enc` and delete the plain file. Returns the new path.
pub fn encrypt_file(path: &Path) -> Result<PathBuf, String> {
    let plain = std::fs::read(path).map_err(|e| e.to_string())?;
    let target = PathBuf::from(format!("{}.{EXT}", path.display()));
    std::fs::write(&target, protect(&plain)?).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(path);
    Ok(target)
}

/// Decrypt `x.png.enc` back to `x.png` and delete the encrypted file. Returns the new path.
pub fn decrypt_file(path: &Path) -> Result<PathBuf, String> {
    let plain = read(path)?;
    let target = path.with_extension("");
    std::fs::write(&target, plain).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(path);
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let secret = b"clipboard history";
        let sealed = protect(secret).unwrap();
        assert_ne!(sealed, secret);
        assert_eq!(unprotect(&sealed).unwrap(), secret);
    }
}
