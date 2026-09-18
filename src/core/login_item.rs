//! `LaunchAgent` plist generation for "Open at Login" (DESIGN.md §6, `docs/ARCHITECTURE.md`
//! "Settings").
//!
//! This module only builds the plist's XML text — a pure string operation. Login item state is
//! not a setting: per the architecture contract, it *is* whether this plist file exists on disk,
//! so the platform layer (`src/platform/login_item.rs`) is responsible for writing/removing it
//! at [`launch_agent_path`] and reading back its presence; nothing here touches the filesystem.

use std::path::{Path, PathBuf};

/// Reverse-DNS bundle identifier used as both the plist file name and the `Label` key.
pub const BUNDLE_ID: &str = "com.beeraw.urahafu";

/// The path a login item's `LaunchAgent` plist belongs at, given the user's home directory:
/// `~/Library/LaunchAgents/<bundle id>.plist`.
#[must_use]
pub fn launch_agent_path(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{BUNDLE_ID}.plist"))
}

/// Builds the `LaunchAgent` plist XML that launches `executable_path` at login.
///
/// `RunAtLoad` starts the app immediately when the agent is loaded (i.e. at the next login);
/// `KeepAlive` is deliberately omitted since Urahafu is a menu bar app the user may quit anytime,
/// not a daemon that should be relaunched.
#[must_use]
pub fn launch_agent_plist(executable_path: &Path) -> String {
    let path_str = executable_path.to_string_lossy();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{executable}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
        label = escape_xml(BUNDLE_ID),
        executable = escape_xml(&path_str),
    )
}

/// Escapes the five XML special characters for use in text content or attribute values.
fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_agent_path_uses_bundle_id() {
        let home = Path::new("/Users/example");
        let path = launch_agent_path(home);
        assert_eq!(
            path,
            Path::new("/Users/example/Library/LaunchAgents/com.beeraw.urahafu.plist")
        );
    }

    #[test]
    fn plist_is_well_formed_xml_shell() {
        let plist = launch_agent_plist(Path::new(
            "/Applications/Urahafu.app/Contents/MacOS/urahafu",
        ));
        assert!(plist.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(plist.contains("<!DOCTYPE plist PUBLIC"));
        assert!(plist.contains("<plist version=\"1.0\">"));
        assert!(plist.trim_end().ends_with("</plist>"));
        // Every opening tag we emit has a matching closing tag.
        for tag in ["dict", "array"] {
            let opens = plist.matches(&format!("<{tag}>")).count();
            let closes = plist.matches(&format!("</{tag}>")).count();
            assert_eq!(opens, closes, "mismatched <{tag}> tags");
        }
        assert_eq!(
            plist.matches("<plist").count(),
            plist.matches("</plist>").count(),
            "mismatched <plist> tags"
        );
    }

    #[test]
    fn plist_contains_label_and_program_arguments() {
        let plist = launch_agent_plist(Path::new(
            "/Applications/Urahafu.app/Contents/MacOS/urahafu",
        ));
        assert!(plist.contains("<key>Label</key>"));
        assert!(plist.contains(&format!("<string>{BUNDLE_ID}</string>")));
        assert!(plist.contains("<key>ProgramArguments</key>"));
        assert!(
            plist.contains("<string>/Applications/Urahafu.app/Contents/MacOS/urahafu</string>")
        );
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<true/>"));
    }

    #[test]
    fn plist_escapes_ampersand_in_path() {
        let plist = launch_agent_plist(Path::new("/Users/A & B/Urahafu.app/urahafu"));
        assert!(plist.contains("/Users/A &amp; B/Urahafu.app/urahafu"));
        assert!(!plist.contains("A & B"));
    }

    #[test]
    fn plist_escapes_angle_brackets_and_quotes_in_path() {
        let plist = launch_agent_plist(Path::new("/Users/<weird>\"'/urahafu"));
        assert!(plist.contains("&lt;weird&gt;&quot;&apos;"));
        assert!(!plist.contains("<weird>"));
    }

    #[test]
    fn escape_xml_handles_all_five_special_characters() {
        assert_eq!(escape_xml("&<>\"'"), "&amp;&lt;&gt;&quot;&apos;");
    }

    #[test]
    fn escape_xml_leaves_plain_text_untouched() {
        assert_eq!(
            escape_xml("/Applications/Urahafu.app"),
            "/Applications/Urahafu.app"
        );
    }
}
