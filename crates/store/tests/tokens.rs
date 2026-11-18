//! Tokens, hashes, secrets and the local identity provider.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use unsup_store::{
    Identity, IdentityError, IdentityProvider, LocalSignUp, Secret, SignUp, Token, TokenHash,
    TokenKind, constant_time_eq, random_hex,
};

#[test]
fn a_token_has_the_prefix_of_its_kind_and_32_random_bytes() {
    let owner = Token::generate(TokenKind::Owner).expect("random bytes");
    let key = Token::generate(TokenKind::Resident).expect("random bytes");
    let random = owner.reveal().strip_prefix("unsup_own_").expect("the owner prefix");
    assert_eq!(random.len(), 64);
    assert!(random.bytes().all(|b| b.is_ascii_hexdigit()));
    assert!(key.reveal().starts_with("unsup_res_"));
    assert_eq!(key.reveal().len(), "unsup_res_".len() + 64);
    assert_eq!((owner.kind(), key.kind()), (TokenKind::Owner, TokenKind::Resident));

    // Two tokens never share the random part.
    let second = Token::generate(TokenKind::Owner).expect("random bytes");
    assert_ne!(owner.reveal(), second.reveal());
    assert_ne!(owner.hash(), second.hash());
}

#[test]
fn the_kind_is_read_from_the_prefix() {
    let owner = Token::generate(TokenKind::Owner).expect("random bytes");
    let key = Token::generate(TokenKind::Resident).expect("random bytes");
    assert_eq!(TokenKind::of(owner.reveal()), Some(TokenKind::Owner));
    assert_eq!(TokenKind::of(key.reveal()), Some(TokenKind::Resident));
    assert_eq!(TokenKind::of("unsup_own_abc"), None);
    assert_eq!(TokenKind::of(&key.reveal().replace("unsup_res_", "unsup_xyz_")), None);
    assert_eq!(TokenKind::of(&format!("{}0", key.reveal())), None);
    assert_eq!(TokenKind::of(&format!("unsup_res_{}", "g".repeat(64))), None);
    assert_eq!(TokenKind::of(""), None);
    assert_eq!(TokenKind::of("12"), None);
}

#[test]
fn the_hash_is_sha_256_of_the_text() {
    // The test vector of FIPS 180-2 for "abc".
    let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    assert_eq!(hex::encode(TokenHash::of("abc").as_bytes()), expected);
    let token = Token::generate(TokenKind::Resident).expect("random bytes");
    assert_eq!(token.hash(), TokenHash::of(token.reveal()));
    assert_eq!(TokenHash::from_bytes(token.hash().as_bytes()), Some(token.hash()));
    assert_eq!(TokenHash::from_bytes(&[0u8; 31]), None);
    assert!(token.hash().matches(&TokenHash::of(token.reveal())));
    assert!(!token.hash().matches(&TokenHash::of("abc")));
}

#[test]
fn debug_and_hint_do_not_show_the_secret() {
    let token = Token::generate(TokenKind::Resident).expect("random bytes");
    let random = &token.reveal()["unsup_res_".len()..];
    let debug = format!("{token:?} {:?}", token.hash());
    assert!(!debug.contains(random), "{debug}");
    assert!(!debug.contains(&hex::encode(token.hash().as_bytes())), "{debug}");
    // The hint shows the prefix and six characters, nothing more.
    assert_eq!(token.hint(), format!("unsup_res_{}...", &random[..6]));

    let secret = Secret::generate_webhook().expect("random bytes");
    assert!(secret.reveal().starts_with("unsup_whs_"));
    assert_eq!(secret.reveal().len(), "unsup_whs_".len() + 64);
    assert_eq!(format!("{secret:?}"), "Secret(<hidden>)");
    assert_eq!(Secret::from_stored("s3".into()).reveal(), "s3");
}

#[test]
fn constant_time_eq_compares_whole_strings() {
    assert!(constant_time_eq(b"", b""));
    assert!(constant_time_eq(b"same bytes", b"same bytes"));
    assert!(!constant_time_eq(b"same bytes", b"same byteS"));
    assert!(!constant_time_eq(b"Same bytes", b"same bytes"));
    assert!(!constant_time_eq(b"short", b"shorter"));
}

#[test]
fn random_hex_has_two_characters_for_each_byte() {
    let a = random_hex(16).expect("random bytes");
    let b = random_hex(16).expect("random bytes");
    assert_eq!(a.len(), 32);
    assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
    assert_eq!(random_hex(0).expect("no bytes"), "");
}

/// Runs a future that is ready at once.
fn now<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let mut context = Context::from_waker(Waker::noop());
    match Pin::new(&mut future).poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the local provider answers at once"),
    }
}

fn sign_up(name: Option<&str>, credential: Option<&str>) -> Result<Identity, IdentityError> {
    let request =
        SignUp { name: name.map(str::to_string), credential: credential.map(str::to_string) };
    now(LocalSignUp.identify(&request))
}

#[test]
fn local_sign_up_makes_a_new_identity_each_time() {
    assert_eq!(LocalSignUp.name(), "local");
    let first = sign_up(Some("  Ada  "), None).expect("a valid sign-up");
    let second = sign_up(Some("Ada"), None).expect("a valid sign-up");
    assert_eq!((first.provider.as_str(), first.name.as_str()), ("local", "Ada"));
    assert_eq!(first.subject.len(), 32);
    assert_ne!(first.subject, second.subject);
}

#[test]
fn local_sign_up_refuses_bad_names_and_credentials() {
    let invalid = |message: &str| Err(IdentityError::Invalid(message.to_string()));
    assert_eq!(sign_up(None, None), invalid("`name` is missing or empty."));
    assert_eq!(sign_up(Some("   "), None), invalid("`name` is missing or empty."));
    assert_eq!(sign_up(Some(&"x".repeat(65)), None), invalid("`name` has at most 64 bytes."));
    assert!(sign_up(Some(&"x".repeat(64)), None).is_ok());
    assert_eq!(
        sign_up(Some("A\u{7}da"), None),
        invalid("`name` must not hold control characters.")
    );
    assert_eq!(
        sign_up(Some("Ada"), Some("code")),
        invalid("The provider `local` takes no `credential`.")
    );
}
