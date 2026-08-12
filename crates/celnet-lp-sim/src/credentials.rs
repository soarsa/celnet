//! **Service-credential resolution** for the simulator daemons — the seam that keeps
//! a password out of `argv`.
//!
//! # Why this exists
//!
//! A process's command line is world-readable (`/proc/<pid>/cmdline`, `ps -ef`), so a
//! `--password` flag publishes the secret to **every** local account. The simulators
//! therefore take **no** password flag at all: the credential is resolved here, from a
//! restricted file or the process environment, both of which are readable only by the
//! process owner (and root).
//!
//! # Precedence (most-explicit wins)
//!
//! 1. **`<PREFIX>_PASSWORD_FILE`** — path to a file whose *entire trimmed contents* are
//!    the password. When the variable is set the file **must** be readable: an unreadable
//!    or empty file is a hard error, never a silent fall-through to a weaker source (a
//!    silent fallback is how a rotated box quietly keeps authenticating on a stale
//!    secret). This is the deployed form — the file is `0600`, owned by the service
//!    account.
//! 2. **`<PREFIX>_PASSWORD`** — the literal secret in the environment. Convenient for a
//!    developer shell and for container secret injection.
//! 3. **Nothing** ⇒ [`CredentialError::Missing`]. There is deliberately **no** built-in
//!    default password: a daemon with no configured credential fails loudly at startup
//!    rather than silently authenticating as a seeded well-known account.
//!
//! The email follows the same shape (`<PREFIX>_USER`) but *does* carry a default — an
//! identity is not a secret, and defaulting it keeps the deployed unit self-describing.
//!
//! # Handling discipline
//!
//! [`ServiceCredentials`] deliberately implements [`Debug`] **without** the secret, so a
//! `{:?}` of a config struct that embeds it can never leak the password into a log line.

use std::fmt;

/// A resolved service login. The password is private and rendered as `<redacted>` by the
/// hand-written [`Debug`] impl, so embedding this in a config struct can never leak it.
#[derive(Clone, PartialEq, Eq)]
pub struct ServiceCredentials {
    /// The service account's login email.
    email: String,
    /// The resolved secret. Never logged, never rendered, never placed in `argv`.
    password: String,
}

impl ServiceCredentials {
    /// Construct from already-resolved parts (the test seam; production goes through
    /// [`resolve`]).
    #[must_use]
    pub fn new(email: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            password: password.into(),
        }
    }

    /// The login email.
    #[must_use]
    pub fn email(&self) -> &str {
        &self.email
    }

    /// The secret, for the one place that legitimately needs it: building the
    /// `LoginRequest` body.
    #[must_use]
    pub fn password(&self) -> &str {
        &self.password
    }
}

// The secret NEVER appears in a debug rendering — defence in depth against a config
// struct being `{:?}`-logged (mirrors the server's `Session` debug discipline).
impl fmt::Debug for ServiceCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceCredentials")
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Why credential resolution failed. Every variant names the environment variable the
/// operator must set, so the daemon's startup failure is self-explanatory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    /// Neither `<PREFIX>_PASSWORD_FILE` nor `<PREFIX>_PASSWORD` was set.
    Missing {
        /// The env-var prefix that was searched (e.g. `LPSIM`).
        prefix: String,
    },
    /// `<PREFIX>_PASSWORD_FILE` was set but the file could not be read.
    Unreadable {
        /// The configured path.
        path: String,
        /// The underlying IO error rendering.
        reason: String,
    },
    /// `<PREFIX>_PASSWORD_FILE` was set and readable but contained only whitespace.
    Empty {
        /// The configured path.
        path: String,
    },
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CredentialError::Missing { prefix } => write!(
                f,
                "no service credential configured: set {prefix}_PASSWORD_FILE to a 0600 file \
                 containing the password (preferred), or {prefix}_PASSWORD in the environment. \
                 A password is never accepted on the command line — argv is world-readable."
            ),
            CredentialError::Unreadable { path, reason } => write!(
                f,
                "password file {path:?} is not readable: {reason}. Create it with \
                 `printf %s 'SECRET' > {path} && chmod 600 {path}` owned by the service account."
            ),
            CredentialError::Empty { path } => write!(
                f,
                "password file {path:?} is empty. Write the password into it \
                 (no trailing newline required — surrounding whitespace is trimmed)."
            ),
        }
    }
}

impl std::error::Error for CredentialError {}

/// Resolve the service credential for the daemon whose env-var prefix is `prefix`
/// (`LPSIM`, `FIXSIM`, …), defaulting the identity to `default_email`.
///
/// See the module docs for the precedence. Reads only the process environment and the
/// filesystem — never `argv`.
///
/// # Errors
/// [`CredentialError`] when no source is configured, or when a configured
/// `<PREFIX>_PASSWORD_FILE` is unreadable or empty.
pub fn resolve(prefix: &str, default_email: &str) -> Result<ServiceCredentials, CredentialError> {
    let email = non_empty_var(&format!("{prefix}_USER")).unwrap_or_else(|| default_email.to_string());
    let password = resolve_password(prefix)?;
    Ok(ServiceCredentials { email, password })
}

/// The password half of [`resolve`], split out so the precedence is unit-testable
/// independently of the identity.
///
/// # Errors
/// As [`resolve`].
fn resolve_password(prefix: &str) -> Result<String, CredentialError> {
    // (1) An explicitly configured file wins, and MUST work — no silent fall-through.
    if let Some(path) = non_empty_var(&format!("{prefix}_PASSWORD_FILE")) {
        return read_file_password(&path);
    }

    // (2) The environment variable.
    if let Some(pw) = non_empty_var(&format!("{prefix}_PASSWORD")) {
        return Ok(pw);
    }

    // (3) Deliberately no default — fail loud rather than try a well-known secret.
    Err(CredentialError::Missing {
        prefix: prefix.to_string(),
    })
}

/// Read a password out of a file: the entire contents, trimmed. A missing/unreadable
/// file and a whitespace-only file are both hard errors (module docs, precedence step 1).
///
/// # Errors
/// [`CredentialError::Unreadable`] / [`CredentialError::Empty`].
fn read_file_password(path: &str) -> Result<String, CredentialError> {
    let raw = std::fs::read_to_string(path).map_err(|e| CredentialError::Unreadable {
        path: path.to_string(),
        reason: e.to_string(),
    })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CredentialError::Empty {
            path: path.to_string(),
        });
    }
    Ok(trimmed.to_string())
}

/// Read an environment variable, treating "set but blank/whitespace" as absent — a
/// blank `LPSIM_PASSWORD=` in a generated unit file must not count as a configured
/// secret (it would otherwise mask the file source and then fail login opaquely).
fn non_empty_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// The Debug rendering NEVER contains the secret — the property that keeps a
    /// `{:?}`-logged config from leaking the credential.
    #[test]
    fn debug_redacts_the_password() {
        let creds = ServiceCredentials::new("svc@example.test", "hunter2-not-in-logs");
        let rendered = format!("{creds:?}");
        assert!(
            !rendered.contains("hunter2-not-in-logs"),
            "password leaked into Debug: {rendered}"
        );
        assert!(rendered.contains("<redacted>"));
        assert!(rendered.contains("svc@example.test"), "identity is not secret");
        // The accessor still returns the real secret for the login call.
        assert_eq!(creds.password(), "hunter2-not-in-logs");
    }

    /// A password file's contents are used, with surrounding whitespace trimmed (so an
    /// operator's `echo` trailing newline does not corrupt the secret).
    #[test]
    fn password_file_is_read_and_trimmed() {
        let dir = std::env::temp_dir().join(format!("celnet-cred-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pw");
        let mut f = std::fs::File::create(&path).unwrap();
        // Deliberately written the way `echo` would: with a trailing newline.
        writeln!(f, "  s3cret-from-file  ").unwrap();
        drop(f);

        let pw = read_file_password(path.to_str().unwrap()).unwrap();
        assert_eq!(pw, "s3cret-from-file");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A configured-but-unreadable file is a HARD error — never a silent fall-through to
    /// a weaker source, which is how a rotated box would keep using a stale secret.
    #[test]
    fn missing_password_file_is_an_error_not_a_fallback() {
        let err = read_file_password("/nonexistent/celnet/pw").unwrap_err();
        assert!(matches!(err, CredentialError::Unreadable { .. }));
        assert!(err.to_string().contains("not readable"));
    }

    /// A whitespace-only file is rejected rather than yielding an empty password.
    #[test]
    fn empty_password_file_is_rejected() {
        let dir = std::env::temp_dir().join(format!("celnet-cred-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pw");
        std::fs::write(&path, "   \n\t ").unwrap();
        let err = read_file_password(path.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, CredentialError::Empty { .. }));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The "nothing configured" error names BOTH variables an operator can set, and
    /// states the argv rule — the message is the daemon's only startup documentation.
    #[test]
    fn missing_credential_error_is_actionable() {
        let err = CredentialError::Missing {
            prefix: "LPSIM".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("LPSIM_PASSWORD_FILE"));
        assert!(msg.contains("LPSIM_PASSWORD"));
        assert!(
            msg.contains("argv is world-readable"),
            "the error must explain WHY there is no flag"
        );
    }
}
