//! Content hashes recorded in the ledger (`grill confirm`, `plan approve`): `sha256:` and the
//! 64 lowercase hex digits of the SHA-256 of the bytes.

use sha2::{Digest, Sha256};

/// `sha256:` and the 64 lowercase hex digits of the SHA-256 of `bytes`.
pub(crate) fn sha256(bytes: &[u8]) -> String {
    let hex = Sha256::digest(bytes)
        .into_iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16));
    "sha256:".chars().chain(hex).collect()
}

#[cfg(test)]
mod tests {
    use super::sha256;

    #[test]
    fn the_hash_is_sha256_in_lowercase_hex() {
        assert_eq!(
            sha256(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "the FIPS 180-2 test vector"
        );
    }
}
