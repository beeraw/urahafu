//! Preferred-language lookup (`CFLocaleCopyPreferredLanguages`), used by
//! [`crate::platform::system::preferred_languages`] to pick the UI language
//! (`core::i18n::Language::from_preferred`).

use std::ffi::c_void;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};

/// The user's preferred languages, most preferred first, as BCP-47-ish tags (e.g. `"fr-FR"`,
/// `"en-US"`) — exactly what `core::i18n::Language::from_preferred` expects.
#[must_use]
pub fn preferred_languages() -> Vec<String> {
    // SAFETY: `CFLocaleCopyPreferredLanguages` takes no arguments and returns either null or a
    // new, owned ("copy rule") `CFArrayRef` of `CFStringRef`s.
    let raw = unsafe { CFLocaleCopyPreferredLanguages() };
    if raw.is_null() {
        return Vec::new();
    }
    // SAFETY: `raw` was just returned non-null by a "copy" function, so we own one reference.
    let array: CFArray<*const c_void> = unsafe { CFArray::wrap_under_create_rule(raw) };

    array
        .get_all_values()
        .into_iter()
        .map(|ptr| {
            // SAFETY: every element of the array returned by `CFLocaleCopyPreferredLanguages` is
            // a `CFStringRef`; the array (still owned by `array` above) keeps it alive, so we
            // take a borrowed reference ("get rule") rather than an owned one.
            let s = unsafe { CFString::wrap_under_get_rule(ptr.cast::<c_void>() as CFStringRef) };
            s.to_string()
        })
        .collect()
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFLocaleCopyPreferredLanguages() -> CFArrayRef;
}
