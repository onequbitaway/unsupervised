//! Tokens: random secrets that are shown once and stored as hashes.

use std::fmt;

use sha2::{Digest, Sha256};

use crate::error::StoreError;

/// Number of random bytes in a token.
const TOKEN_BYTES: usize = 32;
/// Number of characters of the random part that a hint shows.
const HINT_CHARS: usize = 6;

/// What a token gives access to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TokenKind {
    /// The token of an owner: it manages the residents of the owner.
    Owner,
    /// The key of one resident: it acts for the resident.
    Resident,
}

impl TokenKind {
    /// Returns the text that every token of this kind starts with.
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            TokenKind::Owner => "unsup_own_",
            TokenKind::Resident => "unsup_res_",
        }
    }

    /// Returns the name of the kind, as the store writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            TokenKind::Owner => "owner",
            TokenKind::Resident => "resident",
        }
    }

    /// Reads the kind from the prefix of a token. Returns `None` when the
    /// text does not look like a token of this server.
    #[must_use]
    pub fn of(text: &str) -> Option<TokenKind> {
        [TokenKind::Owner, TokenKind::Resident].into_iter().find(|kind| {
            text.strip_prefix(kind.prefix()).is_some_and(|rest| {
                rest.len() == TOKEN_BYTES * 2 && rest.bytes().all(|b| b.is_ascii_hexdigit())
            })
        })
    }
}

/// Fills a buffer with random bytes from the operating system.
fn random_bytes<const N: usize>() -> Result<[u8; N], StoreError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|e| StoreError::Random(e.to_string()))?;
    Ok(bytes)
}

/// Returns `bytes` random bytes from the operating system, as lowercase
/// hexadecimal text.
///
/// # Errors
///
/// Fails when the operating system gives no random bytes.
pub fn random_hex(bytes: usize) -> Result<String, StoreError> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).map_err(|e| StoreError::Random(e.to_string()))?;
    Ok(hex::encode(buffer))
}

/// A new token in clear text. It exists only between the moment the server
/// makes it and the moment the server shows it to its holder.
///
/// `Debug` does not print the secret.
#[derive(Clone, PartialEq, Eq)]
pub struct Token {
    kind: TokenKind,
    text: String,
}

impl Token {
    /// Makes a new token from 32 random bytes of the operating system.
    ///
    /// # Errors
    ///
    /// Fails when the operating system gives no random bytes.
    pub fn generate(kind: TokenKind) -> Result<Token, StoreError> {
        let bytes = random_bytes::<TOKEN_BYTES>()?;
        Ok(Token { kind, text: format!("{}{}", kind.prefix(), hex::encode(bytes)) })
    }

    /// Returns the kind of the token.
    #[must_use]
    pub const fn kind(&self) -> TokenKind {
        self.kind
    }

    /// Returns the secret text. Show it to the holder once and do not keep
    /// or log it.
    #[must_use]
    pub fn reveal(&self) -> &str {
        &self.text
    }

    /// Returns the hash that the store keeps.
    #[must_use]
    pub fn hash(&self) -> TokenHash {
        TokenHash::of(&self.text)
    }

    /// Returns a short text that lets the holder recognise the token in a
    /// list: the prefix and the first characters of the random part.
    #[must_use]
    pub fn hint(&self) -> String {
        let shown = self.kind.prefix().len() + HINT_CHARS;
        format!("{}...", &self.text[..shown])
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token").field("kind", &self.kind).field("text", &"<hidden>").finish()
    }
}

/// The SHA-256 hash of a token. The store keeps this and never the token.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TokenHash([u8; 32]);

impl TokenHash {
    /// Hashes the text of a token.
    #[must_use]
    pub fn of(text: &str) -> TokenHash {
        TokenHash(Sha256::digest(text.as_bytes()).into())
    }

    /// Makes the value from the bytes that a store kept. Returns `None`
    /// when the length is not 32.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<TokenHash> {
        <[u8; 32]>::try_from(bytes).ok().map(TokenHash)
    }

    /// Returns the 32 bytes of the hash.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Compares two hashes in constant time.
    #[must_use]
    pub fn matches(&self, other: &TokenHash) -> bool {
        constant_time_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for TokenHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A hash is not a secret, but it has no place in a log either.
        f.write_str("TokenHash(..)")
    }
}

/// Compares two byte strings. The time depends on the length only, never
/// on the place of the first difference.
#[must_use]
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (x, y) in a.iter().zip(b) {
        difference |= x ^ y;
    }
    std::hint::black_box(difference) == 0
}

/// A secret text that the server must keep as it is, for example the
/// signing secret of a webhook. `Debug` does not print it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Text that every webhook secret starts with.
    pub const WEBHOOK_PREFIX: &'static str = "unsup_whs_";

    /// Makes a new signing secret for a webhook from 32 random bytes of
    /// the operating system.
    ///
    /// # Errors
    ///
    /// Fails when the operating system gives no random bytes.
    pub fn generate_webhook() -> Result<Secret, StoreError> {
        let bytes = random_bytes::<TOKEN_BYTES>()?;
        Ok(Secret(format!("{}{}", Secret::WEBHOOK_PREFIX, hex::encode(bytes))))
    }

    /// Wraps a secret that a store kept.
    #[must_use]
    pub fn from_stored(text: String) -> Secret {
        Secret(text)
    }

    /// Returns the secret text. Do not log it.
    #[must_use]
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<hidden>)")
    }
}
