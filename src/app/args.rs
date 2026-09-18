//! Pure parsing of the process's command-line arguments: currently only `--login`, the flag the
//! `LaunchAgent` passes on login-triggered launches (`core::login_item::launch_agent_plist`;
//! see DESIGN.md "Settings window").

/// Whether `args` (as `std::env::args()` yields them, program name included) contains the
/// `--login` flag.
#[must_use]
pub(crate) fn is_login_launch<'a>(args: impl IntoIterator<Item = &'a str>) -> bool {
    args.into_iter().any(|arg| arg == "--login")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_arguments_is_not_a_login_launch() {
        assert!(!is_login_launch(["urahafu"]));
    }

    #[test]
    fn login_flag_is_detected() {
        assert!(is_login_launch(["urahafu", "--login"]));
    }

    #[test]
    fn login_flag_is_detected_regardless_of_position() {
        assert!(is_login_launch(["urahafu", "--login", "extra"]));
    }

    #[test]
    fn unrelated_arguments_are_not_a_login_launch() {
        assert!(!is_login_launch(["urahafu", "--some-other-flag"]));
    }

    #[test]
    fn empty_argument_list_is_not_a_login_launch() {
        assert!(!is_login_launch(std::iter::empty()));
    }
}
