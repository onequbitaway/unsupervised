//! Identity providers: who signs up.

use std::future::Future;
use std::pin::Pin;

use crate::token::random_hex;

/// Greatest length of the display name of an owner, in bytes.
const NAME_MAX: usize = 64;
/// Number of random bytes in the subject of a local identity.
const SUBJECT_BYTES: usize = 16;

/// What a client sends to sign up.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignUp {
    /// The display name that the client asks for.
    pub name: Option<String>,
    /// The proof that the provider needs, for example the authorization
    /// code of an OAuth service. The local provider takes none.
    pub credential: Option<String>,
}

/// Who signed up, as an identity provider reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// Name of the provider.
    pub provider: String,
    /// Stable id of the person at the provider. A person that signs up
    /// twice with the same subject gets the same owner.
    pub subject: String,
    /// Display name.
    pub name: String,
}

/// The first step of a sign-in that goes through the service of a
/// provider: where the person goes, and the value that ties the answer of
/// the service to this start.
///
/// `Debug` does not print the address and the state.
#[derive(Clone, PartialEq, Eq)]
pub struct Authorization {
    /// The address that the person opens in a browser.
    pub url: String,
    /// The value that the service sends back with its answer. The provider
    /// accepts an answer only with a state that it made, one time.
    pub state: String,
    /// Seconds for which the state is good.
    pub expires_in: u64,
}

impl std::fmt::Debug for Authorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Authorization")
            .field("expires_in", &self.expires_in)
            .finish_non_exhaustive()
    }
}

/// Why an identity provider refused a sign-up.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    /// The sign-up data is not valid. The message says what to change.
    #[error("{0}")]
    Invalid(String),
    /// The provider knows the person and refuses it.
    #[error("{0}")]
    Denied(String),
    /// The provider cannot answer now.
    #[error("{0}")]
    Unavailable(String),
}

/// The answer of [`IdentityProvider::identify`]. It is a boxed future so
/// that a provider can ask a remote service.
pub type IdentityFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Identity, IdentityError>> + Send + 'a>>;

/// A way to find out who signs up.
///
/// The gateway holds a list of providers and picks one by name. The
/// provider checks the proof in the sign-up and returns an [`Identity`].
/// The gateway then finds or makes the owner with that provider and
/// subject, and gives it a new owner token.
///
/// [`LocalSignUp`] is the only provider in this crate. A provider for an
/// OAuth service implements this trait: [`IdentityProvider::begin`] gives
/// the address of the service and a state, and
/// [`IdentityProvider::identify`] exchanges [`SignUp::credential`] for the
/// account of the person and returns the account id as the subject.
pub trait IdentityProvider: Send + Sync + 'static {
    /// The name that a client uses to pick this provider.
    fn name(&self) -> &str;

    /// Checks a sign-up and says who it is.
    fn identify<'a>(&'a self, sign_up: &'a SignUp) -> IdentityFuture<'a>;

    /// Starts a sign-in that has a step at the service of the provider.
    /// Returns `None` when the provider has no such step: the client then
    /// sends its proof directly. The default is `None`.
    ///
    /// # Errors
    ///
    /// Fails when the provider cannot start a sign-in now.
    fn begin(&self) -> Result<Option<Authorization>, IdentityError> {
        Ok(None)
    }
}

/// Sign-up without an outside service: every sign-up is a new identity.
///
/// The owner token is the only credential. An owner that loses it cannot
/// get the account back.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalSignUp;

impl LocalSignUp {
    /// The name of this provider.
    pub const NAME: &'static str = "local";

    fn check(sign_up: &SignUp) -> Result<Identity, IdentityError> {
        if sign_up.credential.is_some() {
            return Err(IdentityError::Invalid(
                "The provider `local` takes no `credential`.".into(),
            ));
        }
        let name = sign_up.name.as_deref().map(str::trim).unwrap_or_default();
        if name.is_empty() {
            return Err(IdentityError::Invalid("`name` is missing or empty.".into()));
        }
        if name.len() > NAME_MAX {
            return Err(IdentityError::Invalid(format!("`name` has at most {NAME_MAX} bytes.")));
        }
        if name.chars().any(char::is_control) {
            return Err(IdentityError::Invalid("`name` must not hold control characters.".into()));
        }
        let subject =
            random_hex(SUBJECT_BYTES).map_err(|e| IdentityError::Unavailable(e.to_string()))?;
        Ok(Identity { provider: LocalSignUp::NAME.to_string(), subject, name: name.to_string() })
    }
}

impl IdentityProvider for LocalSignUp {
    fn name(&self) -> &str {
        LocalSignUp::NAME
    }

    fn identify<'a>(&'a self, sign_up: &'a SignUp) -> IdentityFuture<'a> {
        Box::pin(std::future::ready(LocalSignUp::check(sign_up)))
    }
}
