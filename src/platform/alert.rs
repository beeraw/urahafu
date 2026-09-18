//! Native alerts (`NSAlert`), built from [`crate::core::i18n::Text`] (DESIGN.md §11-§12).
//!
//! `objc2`/`objc2-app-kit`'s generated `NSAlert` bindings are already safe for everything used
//! here (see the crate docs for why no `ffi/alert.rs` is needed: the only genuinely `unsafe`
//! parts of the Objective-C bridge, like custom icon setters, are not needed — an `NSAlert`
//! defaults to the app icon on its own, matching every mockup in DESIGN.md §11-§12).
//!
//! The first-launch/welcome/hide-icon-confirmation alerts are gone: the settings window
//! (DESIGN.md "Settings window") replaced them. Only the three Safety-net error alerts and the
//! About alert remain here.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication};
use objc2_foundation::NSString;

use crate::core::i18n::{Language, Text, about_version};

/// Severity of a native alert, mirroring `NSAlertStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlertStyle {
    /// Informational, no particular urgency (About).
    #[default]
    Informational,
    /// A critical condition (the three Safety-net error alerts: permission missing, tap failed,
    /// Secure Input active).
    Critical,
}

impl AlertStyle {
    fn to_ns(self) -> NSAlertStyle {
        match self {
            Self::Informational => NSAlertStyle::Informational,
            Self::Critical => NSAlertStyle::Critical,
        }
    }
}

/// Everything needed to show one native alert. `buttons[0]` is the default button (Return key,
/// shown in the most prominent position); every other button is a secondary action.
#[derive(Debug, Clone)]
pub struct AlertSpec {
    /// The alert's prominent message (`NSAlert.messageText`).
    pub title: String,
    /// The alert's secondary, less prominent text (`NSAlert.informativeText`).
    pub message: String,
    /// Button titles, in add order; `buttons[0]` is the default button. Must not be empty (an
    /// `NSAlert` with no buttons added at all gets an implicit "OK", which this type never
    /// relies on).
    pub buttons: Vec<String>,
    /// The alert's severity.
    pub style: AlertStyle,
}

/// Shows `spec` as a native, modal `NSAlert` and returns the index into `spec.buttons` of the
/// button the user clicked. Must run on the main thread (`mtm` is proof of that).
///
/// Activates the app first so the alert reliably comes to the front — necessary for an accessory
/// app (no Dock icon), which otherwise has no natural way to get focus.
#[must_use]
pub fn show(spec: &AlertSpec, mtm: MainThreadMarker) -> usize {
    let app = NSApplication::sharedApplication(mtm);
    app.activate();

    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&spec.title));
    alert.setInformativeText(&NSString::from_str(&spec.message));
    alert.setAlertStyle(spec.style.to_ns());
    for button in &spec.buttons {
        alert.addButtonWithTitle(&NSString::from_str(button));
    }

    let response = alert.runModal();
    usize::try_from(response - NSAlertFirstButtonReturn).unwrap_or(0)
}

impl AlertSpec {
    /// Error alert: Accessibility permission missing or refused (DESIGN.md §11, case 1).
    #[must_use]
    pub fn permission_required(language: Language) -> Self {
        Self {
            title: Text::AlertPermissionMissingTitle.get(language).to_owned(),
            message: Text::AlertPermissionMissingText.get(language).to_owned(),
            buttons: vec![
                Text::AlertPermissionMissingOpenSettings
                    .get(language)
                    .to_owned(),
                Text::AlertCancel.get(language).to_owned(),
            ],
            style: AlertStyle::Critical,
        }
    }

    /// Error alert: the event tap failed to create (DESIGN.md §11, case 2).
    #[must_use]
    pub fn tap_failed(language: Language) -> Self {
        Self {
            title: Text::AlertTapFailedTitle.get(language).to_owned(),
            message: Text::AlertTapFailedText.get(language).to_owned(),
            buttons: vec![
                Text::AlertRetry.get(language).to_owned(),
                Text::AlertCancel.get(language).to_owned(),
            ],
            style: AlertStyle::Critical,
        }
    }

    /// Error alert: Secure Input is active (DESIGN.md §11, case 3).
    #[must_use]
    pub fn secure_input(language: Language) -> Self {
        Self {
            title: Text::AlertSecureInputTitle.get(language).to_owned(),
            message: Text::AlertSecureInputText.get(language).to_owned(),
            buttons: vec![
                Text::AlertRetry.get(language).to_owned(),
                Text::AlertCancel.get(language).to_owned(),
            ],
            style: AlertStyle::Critical,
        }
    }

    /// The About alert (DESIGN.md §12): name, version, tagline and license in the informative
    /// text, "View on GitHub" and "OK" (default) as buttons.
    #[must_use]
    pub fn about(language: Language, version: &str) -> Self {
        let message = format!(
            "{}\n{}\n{}",
            about_version(language, version),
            Text::AboutTagline.get(language),
            Text::AboutLicense.get(language),
        );
        Self {
            title: "Urahafu".to_owned(),
            message,
            buttons: vec![
                Text::AboutOk.get(language).to_owned(),
                Text::AboutViewOnGithub.get(language).to_owned(),
            ],
            style: AlertStyle::Informational,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_required_has_two_buttons_open_settings_default() {
        let spec = AlertSpec::permission_required(Language::from_tag("fr").unwrap());
        assert_eq!(spec.buttons[0], "Ouvrir les Réglages Système");
        assert_eq!(spec.buttons[1], "Annuler");
    }

    #[test]
    fn tap_failed_default_button_is_retry() {
        let spec = AlertSpec::tap_failed(Language::ENGLISH);
        assert_eq!(spec.buttons[0], "Try Again");
    }

    #[test]
    fn secure_input_default_button_is_retry() {
        let spec = AlertSpec::secure_input(Language::ENGLISH);
        assert_eq!(spec.buttons[0], "Try Again");
    }

    #[test]
    fn about_includes_version_tagline_and_license() {
        let spec = AlertSpec::about(Language::ENGLISH, "1.0.0");
        assert!(spec.message.contains("Version 1.0.0"));
        assert!(spec.message.contains("Clean your screen and keyboard"));
        assert!(spec.message.contains("MIT License"));
        assert_eq!(spec.buttons[0], "OK");
    }

    #[test]
    fn no_alert_spec_has_empty_buttons() {
        let language = Language::from_tag("fr").unwrap();
        let specs = [
            AlertSpec::permission_required(language),
            AlertSpec::tap_failed(language),
            AlertSpec::secure_input(language),
            AlertSpec::about(language, "1.0.0"),
        ];
        for spec in specs {
            assert!(!spec.buttons.is_empty());
        }
    }
}
