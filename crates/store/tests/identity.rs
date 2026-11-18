//! The start of a sign-in at an identity provider.

use unsup_store::{Authorization, IdentityProvider, LocalSignUp};

#[test]
fn local_sign_up_has_no_step_at_a_service() {
    assert_eq!(LocalSignUp.begin(), Ok(None));
}

#[test]
fn a_print_of_an_authorization_shows_no_address_and_no_state() {
    let authorization = Authorization {
        url: "https://provider.example/authorize?state=the-state-value".into(),
        state: "the-state-value".into(),
        expires_in: 600,
    };
    let shown = format!("{authorization:?}");
    assert_eq!(shown, "Authorization { expires_in: 600, .. }");
}
