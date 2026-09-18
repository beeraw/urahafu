# Security Policy

## Threat model

Urahafu intercepts every keystroke, click, scroll and trackpad gesture on the
machine while a cleaning session is active, using a macOS event tap
(`CGEventTap`). That is a powerful capability, so the project treats it as the
core of its threat model:

- **No keystroke is ever stored, logged or transmitted — and no character is
  ever read.** Unlocking works by holding an on-screen ✕ button for 2
  seconds, or by holding Escape and Return together for the same duration,
  not by typing a sequence, so the app has no reason to know what was typed
  at all: forwarded key events carry no character, only the physical virtual
  key code reduced to "Escape", "Return", "Space" or "some other key" (plus,
  for a key-down, whether it's an OS autorepeat) and, for a modifier key,
  only the fact that its flags changed. There is no network code anywhere in
  the crate.
- **The user must never get stuck.** Blocking only starts after the app has
  proven it can remove itself again: the input blocker refuses to start if
  Accessibility isn't granted, if Secure Input is active, or if the event tap
  fails to install. A fail-safe timer (30/60/90 s, independent watchdog
  thread) releases input even if the main thread hangs or panics.
- **`unsafe` is confined to `src/platform/ffi/`.** The rest of the crate
  builds with `#![deny(unsafe_code)]`. Every `unsafe` block is documented with
  a `// SAFETY:` comment explaining the invariant it relies on.
- **Releases are signed with the project's self-signed certificate, not
  notarized.** Users install them from a GitHub Release zip; see the README
  for the Gatekeeper workaround and for how to check the certificate
  fingerprint. This is a known, documented trade-off of shipping an
  open-source binary without an Apple Developer ID, not an oversight.

## Code signing

macOS ties the Accessibility permission to the app's signing identity. With
the project certificate ("Urahafu Code Signing", SHA-1
`11700E5903A230D75A389FFD99928A689F5FA11A`), that identity is the bundle
identifier plus the certificate, so the permission survives updates.

This also means that **whoever holds the certificate's private key can ship a
build that inherits the Accessibility permission of existing users**. The key
is therefore handled as follows:

- It exists in the maintainer's login keychain and in an offline backup, and
  in the repository's GitHub Actions secrets (`MACOS_CERT_P12`,
  `MACOS_CERT_PASSWORD`), used only by the release workflow on version tags.
- The release workflow imports it into a temporary keychain created for the
  job and deleted at the end of it, and checks that the signed app carries
  the expected certificate.
- A self-signed certificate can't be revoked. If the key is ever believed to
  be compromised, the project will say so prominently in the README and the
  releases, switch to a new certificate (published with its fingerprint), and
  users will be asked to grant the permission again, once.

Out of scope: attacks that require an attacker who already has Accessibility
permission or root on the machine, and physical access attacks (e.g. a
hardware keylogger) that no userspace app can defend against.

## Reporting a vulnerability

Please report security issues privately through GitHub's
[private vulnerability reporting](https://github.com/beeraw/urahafu/security/advisories/new)
for this repository, rather than opening a public issue.

Include what you found, how to reproduce it, and its impact if you can. We'll
acknowledge the report and work with you on a fix and, where relevant, a
coordinated disclosure timeline.
