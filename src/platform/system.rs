//! Small system queries that don't deserve their own module: the user's preferred languages (for
//! `core::i18n::Language::from_preferred`), opening a URL, and locating the home directory.

use std::env;
use std::path::PathBuf;
use std::process::Command;

use crate::platform::ffi::locale;

/// Errors from the small system operations in this module.
#[derive(Debug, thiserror::Error)]
pub enum SystemError {
    /// `/usr/bin/open` could not be spawned for `url`.
    #[error("failed to open {url}: {source}")]
    Open {
        /// The URL or path that was being opened.
        url: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// `$HOME` is not set, or set to an empty string.
    #[error("could not determine the home directory: $HOME is not set")]
    NoHomeDirectory,
}

/// The user's preferred languages, most preferred first (e.g. `["fr-FR", "en-US"]`), straight
/// from `CFLocaleCopyPreferredLanguages`.
#[must_use]
pub fn preferred_languages() -> Vec<String> {
    locale::preferred_languages()
}

/// Opens `url` with the default handler via `/usr/bin/open`.
///
/// # Errors
///
/// Returns [`SystemError::Open`] if `/usr/bin/open` could not be spawned.
pub fn open_url(url: &str) -> Result<(), SystemError> {
    Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .map_err(|source| SystemError::Open {
            url: url.to_owned(),
            source,
        })?;
    Ok(())
}

/// The current user's home directory, from `$HOME`.
///
/// # Errors
///
/// Returns [`SystemError::NoHomeDirectory`] if `$HOME` is unset or empty.
pub fn home_dir() -> Result<PathBuf, SystemError> {
    match env::var_os("HOME") {
        Some(home) if !home.is_empty() => Ok(PathBuf::from(home)),
        _ => Err(SystemError::NoHomeDirectory),
    }
}
