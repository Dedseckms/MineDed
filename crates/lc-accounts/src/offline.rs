//! Offline accounts: nickname validation, Mojang-compatible offline UUID, throwaway tokens.

use chacha20poly1305::aead::Generate;
use lc_types::{AppError, AppResult, ErrorCode};
use md5::{Digest, Md5};
use uuid::{Builder, Uuid};

/// Minimum nickname length accepted by vanilla servers.
pub const NAME_MIN: usize = 3;
/// Maximum nickname length accepted by vanilla servers.
pub const NAME_MAX: usize = 16;

/// Validates an offline nickname: 3–16 chars of `[A-Za-z0-9_]`.
///
/// Leading/trailing whitespace is trimmed; the trimmed name is returned.
pub fn validate_offline_name(name: &str) -> AppResult<&str> {
    let name = name.trim();
    let len_ok = (NAME_MIN..=NAME_MAX).contains(&name.len());
    let chars_ok = name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if len_ok && chars_ok {
        Ok(name)
    } else {
        Err(AppError::new(
            ErrorCode::InvalidInput,
            "offline name must be 3-16 characters of [A-Za-z0-9_]",
        )
        .detail("field", "name"))
    }
}

/// Offline UUID exactly like Mojang's server: Java's
/// `UUID.nameUUIDFromBytes(("OfflinePlayer:" + name).getBytes(UTF_8))`,
/// i.e. MD5 of the string with version 3 / RFC 4122 variant bits set.
pub fn offline_uuid(name: &str) -> Uuid {
    let digest = Md5::digest(format!("OfflinePlayer:{name}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest);
    Builder::from_md5_bytes(bytes).into_uuid()
}

/// Random hex access token for offline launches (the game only needs a non-empty value).
pub fn random_access_token() -> AppResult<String> {
    let bytes = <[u8; 16]>::try_generate()
        .map_err(|_| AppError::new(ErrorCode::Internal, "access token generation failed"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Independent reference: manual bit twiddling as in `java.util.UUID.nameUUIDFromBytes`.
    fn java_name_uuid(input: &str) -> String {
        let mut b: [u8; 16] = Md5::digest(input.as_bytes()).into();
        b[6] = (b[6] & 0x0f) | 0x30;
        b[8] = (b[8] & 0x3f) | 0x80;
        let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
        format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        )
    }

    #[test]
    fn offline_uuid_known_value() {
        // Well-known offline UUID of "Notch".
        assert_eq!(
            offline_uuid("Notch").to_string(),
            "b50ad385-829d-3141-a216-7e7d7539ba7f"
        );
    }

    #[test]
    fn offline_uuid_matches_java_algorithm() {
        for name in ["Steve", "Alex", "abc", "Player_123", "a_b_c_d_e_f_g_h_"] {
            let u = offline_uuid(name);
            assert_eq!(
                u.to_string(),
                java_name_uuid(&format!("OfflinePlayer:{name}"))
            );
            assert_eq!(u.get_version_num(), 3);
        }
    }

    #[test]
    fn offline_uuid_is_case_sensitive() {
        assert_ne!(offline_uuid("steve"), offline_uuid("Steve"));
    }

    #[test]
    fn name_validation() {
        for ok in ["abc", "Steve", "Player_123", "0123456789abcdef", "___"] {
            assert_eq!(validate_offline_name(ok).unwrap(), ok);
        }
        assert_eq!(validate_offline_name("  Steve ").unwrap(), "Steve");
        for bad in [
            "",
            "ab",
            "0123456789abcdefg",
            "has space",
            "dash-name",
            "Ник",
            "emoji😀",
            "dot.name",
        ] {
            let err = validate_offline_name(bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad}");
        }
    }

    #[test]
    fn access_token_is_random_hex() {
        let a = random_access_token().unwrap();
        let b = random_access_token().unwrap();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
