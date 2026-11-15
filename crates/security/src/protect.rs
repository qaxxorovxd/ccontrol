//! At-rest protection for the device private key.
//!
//! Format: 1 header byte followed by the payload.
//! * `0x01` — Windows DPAPI (CurrentUser scope) blob.
//! * `0x00` — plaintext PKCS#8; only written on platforms without a native
//!   secret store wired up yet, and always with owner-only (0600) permissions.

use crate::SecurityError;
use std::path::Path;

const TAG_PLAIN: u8 = 0x00;
const TAG_DPAPI: u8 = 0x01;

pub fn protect(secret: &[u8]) -> Result<Vec<u8>, SecurityError> {
    #[cfg(windows)]
    {
        let mut out = vec![TAG_DPAPI];
        out.extend(dpapi::protect(secret)?);
        Ok(out)
    }
    #[cfg(not(windows))]
    {
        let mut out = vec![TAG_PLAIN];
        out.extend_from_slice(secret);
        Ok(out)
    }
}

pub fn unprotect(blob: &[u8]) -> Result<Vec<u8>, SecurityError> {
    match blob.split_first() {
        Some((&TAG_PLAIN, rest)) => Ok(rest.to_vec()),
        #[cfg(windows)]
        Some((&TAG_DPAPI, rest)) => dpapi::unprotect(rest),
        #[cfg(not(windows))]
        Some((&TAG_DPAPI, _)) => Err(SecurityError::Protect(
            "DPAPI blob cannot be opened on this OS".into(),
        )),
        _ => Err(SecurityError::Protect(
            "unknown key container format".into(),
        )),
    }
}

/// Write a file readable only by the current user (best effort on Windows,
/// where the per-user profile ACL already applies).
pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, data)?;
    std::fs::rename(tmp, path)
}

#[cfg(windows)]
mod dpapi {
    use super::SecurityError;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    const ENTROPY: &[u8] = b"ccontrol-identity-v1";

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut core::ffi::c_void)));
        v
    }

    pub fn protect(data: &[u8]) -> Result<Vec<u8>, SecurityError> {
        let input = blob(data);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptProtectData(
                &input,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .map_err(|e| SecurityError::Protect(format!("CryptProtectData: {e}")))?;
            Ok(take(out))
        }
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, SecurityError> {
        let input = blob(data);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .map_err(|e| SecurityError::Protect(format!("CryptUnprotectData: {e}")))?;
            Ok(take(out))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let secret = b"top secret key material";
        let sealed = protect(secret).unwrap();
        // DPAPI encrypts on Windows; elsewhere the key is stored as-is in a 0600 file.
        #[cfg(windows)]
        {
            assert_eq!(sealed[0], TAG_DPAPI);
            assert!(!sealed.windows(secret.len()).any(|w| w == secret));
        }
        #[cfg(not(windows))]
        assert_eq!(sealed[0], TAG_PLAIN);
        assert_eq!(unprotect(&sealed).unwrap(), secret);
    }

    #[test]
    fn rejects_unknown_container() {
        assert!(unprotect(&[0x7f, 1, 2]).is_err());
        assert!(unprotect(&[]).is_err());
    }
}
