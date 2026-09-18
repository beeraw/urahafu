# Security Policy

## Threat model

Urahafu intercepts every keystroke, click, scroll and trackpad gesture on the
machine while a cleaning session is active, using a macOS event tap
(`CGEventTap`). That is a powerful capability, so the project treats it as the
core of its threat model:

- **No keystroke is ever stored, logged or transmitted — and none is even
  read.** Unlocking works by holding an on-screen ✕ button for 2 seconds, not
  by typing a sequence, so the app has no reason to know which key was
  pressed at all: forwarded key events carry no character, only the fact
  that some key (or Space, or Escape) was blocked. There is no network code
  anywhere in the crate.
- **The user must never get stuck.** Blocking only starts after the app has
  proven it can remove itself again: the input blocker refuses to start if
  Accessibility isn't granted, if Secure Input is active, or if the event tap
  fails to install. A fail-safe timer (30/60/90 s, independent watchdog
  thread) releases input even if the main thread hangs or panics.
- **`unsafe` is confined to `src/platform/ffi/`.** The rest of the crate
  builds with `#![deny(unsafe_code)]`. Every `unsafe` block is documented with
  a `// SAFETY:` comment explaining the invariant it relies on.
- **The release binary is ad-hoc signed, not notarized.** Users install it
  from a GitHub Release zip; see the README for the Gatekeeper workaround.
  This is a known, documented trade-off of shipping an unsigned open-source
  binary, not an oversight.

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
