# Manual testing checklist

Everything below depends on real macOS behavior (permissions, event taps,
window server, hardware keys) and can't be exercised by `cargo test`. Run
through this list before tagging a release, on at least one Intel and one
Apple Silicon Mac if you have access to both.

## Accessibility permission

### Reset between passes

Every scenario below starts from "as if freshly installed", but macOS ties the
Accessibility grant to the **app's code signature**. A build signed with the
project certificate (`packaging/sign.sh`) keeps the grant across rebuilds; an
ad-hoc build loses it at every rebuild, and its stale entry stays switched on
in System Settings without working. Before
each fresh-install-style scenario, reset both halves of first-run state:

```
tccutil reset Accessibility com.beeraw.urahafu
rm ~/Library/Application\ Support/Urahafu/settings.conf
```

The first command revokes the Accessibility grant (so the next launch sees
"not trusted"); the second clears every saved setting (so the next launch is
a genuine first run) — including a leftover `welcome_seen` key from an older
Urahafu, if present, which is ignored either way.

### Scenarios

- [ ] **Fresh install, permission not yet granted**: reset both (above), then
      launch. The settings window appears, with its permission banner
      (DESIGN.md "Settings window": `window.permission.text` + "Open
      System Settings" button) visible and the Clean button disabled; no
      cleaning overlay shows at any point. The tray menu (if the icon is
      shown) also shows the "Allow Accessibility Access…" item above "Clean
      Screen" (§6).
- [ ] "Open System Settings" (the banner's button) opens Privacy & Security
      › Accessibility.
- [ ] **Fresh install, permission already granted** (e.g. granted manually in
      System Settings before ever opening the app, or a previous install's
      grant survived this build's signature): reset only the settings file
      (not the Accessibility grant), then launch. The settings window
      appears with no banner, Clean enabled — this is the bug the settings
      window fixes: previously the app started completely silently in this
      case (direct-launch mode), with nothing but a small menu bar icon and
      no explanation. The tray does **not** show the "Allow Accessibility
      Access…" item.
- [ ] **Fresh install, nothing granted, app not listed**: reset both, launch,
      click **Open System Settings** in the banner. macOS shows its own
      "allow this app?" prompt and the Accessibility pane opens with Urahafu
      already listed (switched off); switching it on is all it takes.
- [ ] **Permission survives an update**: with the permission granted to a
      build signed by `packaging/sign.sh` with the project certificate,
      install a different build signed the same way and launch it: no
      banner, Clean enabled.
- [ ] **Grant while the window is open**: reset both, launch (banner
      visible), then grant Accessibility manually in System Settings without
      touching the app. Within ~2 s the banner disappears and Clean enables
      on its own — this is the permission watcher (DESIGN.md "Settings
      window"), which polls whenever idle regardless of how System Settings
      was reached.
- [ ] **Grant from the permission-required error alert**: reset both, launch,
      close the window's banner state aside (leave the permission
      ungranted), click Clean: the permission-required error alert (§11
      case 1) appears. Click "Open System Settings" and grant access there.
      The banner disappears and Clean enables within ~2 s, exactly as in the
      previous scenario.
- [ ] Grant Accessibility while a cleaning session is in progress: no alert
      interrupts the session; the window/tray refresh only once the app is
      back at idle.
- [ ] Revoke the permission (System Settings) while the app is running, then
      try to start a cleaning session: the "Accessibility access required"
      error alert appears (§11 case 1), no overlay shows.

## Secure Input

- [ ] Open Terminal and enable Secure Keyboard Entry (Terminal menu ›
      Secure Keyboard Entry), then try to start a cleaning session: the
      "Secure Input is on" error alert appears (§11 case 3), no overlay
      shows.
- [ ] Turn Secure Keyboard Entry back off, retry: cleaning starts normally.

## Countdown

- [ ] Countdown shows 3 → 2 → 1 with the fade/scale animation described in
      §7, then the hint fades in and blocking starts.
- [ ] Pressing Esc during the countdown cancels (no blocking ever starts).
- [ ] Clicking during the countdown cancels.
- [ ] Cancelling during the countdown leaves the Dock, menu bar and all
      other apps fully responsive immediately.

## Full input blocking (once locked)

For each of the following, confirm the input has **no effect** on the rest of
the system while locked, and that holding the ✕ button for 2 seconds still
unlocks afterward:

- [ ] Regular letter/number keys
- [ ] Mouse clicks (left, right)
- [ ] Trackpad gestures (swipe between spaces/full-screen apps, pinch,
      Mission Control gesture)
- [ ] Scroll wheel / trackpad scroll
- [ ] Media keys (play/pause, next/previous track, mute)
- [ ] Brightness keys
- [ ] Volume keys
- [ ] Mission Control (F3 / Control-Up)
- [ ] Cmd-Tab (app switcher)
- [ ] Cmd-Q
- [ ] Cmd-Space (Spotlight)
- [ ] Touch ID prompt does not appear / is not triggered by stray touches
- [ ] Power button — **known limitation**: macOS reserves some hardware and
      system-level shortcuts (notably the physical power button, and on some
      configurations Control-Cmd-Q / lock screen) below the level an
      accessibility event tap can intercept. Confirm current behavior and
      record it here rather than assuming it's blocked; this is a platform
      limitation, not a bug to fix.

## Hold-to-unlock button (✕)

- [ ] Hold the ✕ button with a trackpad click for the full 2 seconds: the
      progress ring fills over 2.0 s (§8 DESIGN.md), the button briefly
      scales up, then the overlay unlocks (fade 300 ms, input released
      first).
- [ ] Same with a physical mouse click and hold: same result.
- [ ] Press the button and release before 2 seconds: the ring drains back to
      0 over 200 ms and the session stays locked.
- [ ] Press the button, then drag the pointer outside the hit target before
      2 seconds (without releasing): the hold cancels the same way as an
      early release — the ring drains and nothing unlocks.
- [ ] Wipe the trackpad with a cloth for about 30 seconds while a session is
      locked (simulating the actual cleaning motion): this does **not**
      unlock the session. Incidental contact and short taps must never
      accumulate into a completed 2-second hold.
- [ ] Move the pointer within 160 pt of the button's center while the hint
      is faded out: hint and button fade back in within 200 ms and stay
      visible as long as the pointer stays in that radius; moving away
      starts the normal 2.5 s hold before fading out again.
- [ ] Any blocked input (a key press including Esc, a click anywhere on
      screen, a scroll, a media key) re-shows the hint and the button, per
      §8.
- [ ] Enable Keyboard Only Mode and start a session: holding the small ✕ in
      the HUD pill (§9 DESIGN.md) for 2 seconds unlocks the same way as the
      full-screen button, with the same ring animation at its smaller size.
- [ ] During the countdown, the explanation line under the digit ("To
      unlock, hold the close button or Esc + Return for 2 seconds") is
      present and readable, and the ✕ button pulses gently at
      its final position but is **not** holdable yet — pressing and holding
      it during the countdown does not start a progress ring, and a click
      still cancels the countdown as usual (Esc still cancels the countdown
      too, as before — see "Countdown" above).

## Esc+Return keyboard combo (DESIGN.md §8)

- [ ] Hold Escape and Return together (physical keyboard, no other key) for
      the full 2 seconds: the same progress ring fills on the ✕ button as a
      pointer hold would, the button briefly scales up, then the overlay
      unlocks — same 300 ms fade, input released first.
- [ ] Release either key before 2 seconds: the ring drains back to 0 over
      200 ms and the session stays locked.
- [ ] Hold Escape and Return, then press a third key (e.g. a letter) without
      releasing either: the ring drains immediately. Release the third key:
      the hold restarts from 0 (Escape and Return are still down).
- [ ] Hold Escape and Return, then tap a modifier key (Shift, Control,
      Option, Command, Caps Lock, or Fn) without releasing either: the ring
      drains and — unlike the plain extra-key case above — does **not**
      restart on its own even though Escape and Return are still physically
      held down. Release and re-press Escape or Return to start a fresh
      hold.
- [ ] Substitute the numeric keypad's Enter key for Return: the combo works
      identically (keypad Enter counts as Return).
- [ ] Holding only Escape, or only Return, does not start any progress.
- [ ] Enable Keyboard Only Mode and repeat the full-hold case: the HUD's ✕
      button fills and unlocks the same way (§9 DESIGN.md), with the HUD's
      hint text reading "Hold the button or Esc + Return 2 s".
- [ ] Hold the pointer on the ✕ button partway (don't complete it), then
      also engage the Esc+Return combo: the button's ring reflects whichever
      gesture has progressed further, and either one completing unlocks the
      session.
- [ ] With Auto-Unlock set low (e.g. 30 s), hold Escape and Return but
      release before completing: confirm the fail-safe still fires at its
      normal deadline, independent of the partial combo hold.

## Fail-safe auto-unlock

- [ ] Set Auto-Unlock to 30 s, start a session, do nothing: input is released
      and the overlay closes at ~30 s.
- [ ] Same for 60 s (default).
- [ ] Same for 90 s.
- [ ] The remaining-time bar (§8) visibly counts down and gains emphasis in
      the last 10 seconds.

## Keyboard-only mode

- [ ] Enable "Keyboard Only Mode" in the menu; starting a session shows the
      HUD (§9) instead of a full-screen overlay, and the rest of the screen
      stays visible.
- [ ] HUD appearance matches System Settings › Appearance: Light.
- [ ] HUD appearance matches System Settings › Appearance: Dark.
- [ ] HUD stays on top and its own clicks are also blocked while locked.
- [ ] Countdown state in the HUD shows "Locking in 3/2/1" per §7.

## Multi-monitor

- [ ] With two or more displays connected, the cleaning color covers every
      screen.
- [ ] Hint, unlock button and remaining-time bar appear only on the main
      display, per §8.
- [ ] Input stays blocked across all displays (moving the pointer to a
      second monitor doesn't leak clicks/keys through).

## Dock and menu bar

- [ ] Dock is hidden during a full-screen (non keyboard-only) cleaning
      session.
- [ ] Menu bar is hidden during a full-screen cleaning session.
- [ ] Both come back immediately on unlock/cancel/fail-safe.

## Dead-pixel test

- [ ] Pressing Space during a session cycles Red → Green → Blue → White →
      Black → back to the chosen cleaning color, per §8.
- [ ] The step indicator ("2/5" etc.) only appears while the hint is visible.
- [ ] The remaining-time bar and wordmark are hidden during the pixel test.
- [ ] A key other than Space during the pixel test re-shows the hint as
      usual (and does not advance the pixel cycle).

## Settings window

- [ ] **Normal launch** (Dock, Finder, Spotlight, `open`): the settings
      window appears and the app activates (Dock icon + Cmd-Tab); the tray
      icon also appears iff "Show Icon in Menu Bar" is on. No overflow at any
      window width: no control is cut off, nothing pokes past the window's
      right or bottom edge.
- [ ] **Reopen from Dock/Spotlight/Finder while already running**: opening
      the app again (window closed/hidden) brings the window back and
      activates it; doing this while a cleaning session is running has no
      effect (the reopen is ignored until the session ends).
- [ ] **Close with the tray icon shown**: clicking the window's red button
      (or Cmd-W, or Esc) hides the window; the app keeps running in the menu
      bar (no Dock icon), reachable again via the tray's "Settings…" item or
      by reopening the app.
- [ ] **Close with the tray icon hidden**: with "Show Icon in Menu Bar" off,
      closing the window (red button/Cmd-W/Esc) quits the app — there would
      be no way left to reach it otherwise.
- [ ] **`--login` launch** (via the LaunchAgent, or manually:
      `open -a Urahafu --args --login`): no window appears if the tray icon
      is shown; if the icon is hidden, the window appears anyway — the app
      never runs invisibly.
- [ ] **Toggle "Show Icon in Menu Bar" live**: turning it off drops the tray
      icon immediately (no confirmation alert, no quit) and disables the
      "Open at Login" switch (it greys out and a second line,
      `window.open_at_login.needs_icon`, appears under its label — the row
      grows taller to fit it, and the window resizes to match, top-left
      corner staying put); turning it back on recreates the icon immediately
      and the note disappears (row and window shrink back).
- [ ] **Permission banner live**: with Accessibility ungranted, the orange
      banner is visible and the Clean button is disabled and dimmed (~40%
      opacity — not just greyed text); granting access (any of the ways in
      "Accessibility permission" above) makes the banner disappear and Clean
      enable/return to full opacity within ~2 s, without closing or
      reopening the window (and the window shrinks to fit now that the
      banner is gone, top-left corner staying put).
- [ ] **Color swatches**: clicking the white swatch immediately shows the
      teal selection ring around it (2 pt gap) and removes it from the black
      swatch, and vice versa; a cleaning session started right after uses the
      just-picked color. Both swatches are keyboard-focusable (Tab) and
      clickable via the keyboard (Space/Return) as well as the mouse; VoiceOver
      (or Accessibility Inspector) reads them as "Black"/"White"
      (`menu.color.black`/`menu.color.white`), not blank buttons.
- [ ] **Auto-Unlock segmented control**: shows three segments labelled
      "30 s"/"60 s"/"90 s" (`window.seconds`); clicking a segment selects it
      (single selection, like a radio group) and a session started
      afterwards auto-unlocks after that many seconds (cross-check against
      "Fail-safe auto-unlock" above).
- [ ] **Switches**: "Keyboard-Only Mode" and "Show Icon in Menu Bar" render
      as native macOS switches (not checkboxes), reflect the saved setting on
      launch, and toggling one applies immediately (same live-apply behavior
      as before).
- [ ] **Clean from the window**: clicking "Clean Screen"/"Clean Keyboard"
      hides the window and starts a session exactly like the tray's Clean;
      once the session ends (unlock, fail-safe, or cancel), the window shows
      again.
- [ ] **Clean from the tray**: starting a session from the tray's menu
      instead leaves the app at idle in the menu bar once it ends — the
      window does not pop up on its own.
- [ ] "Open at Login" toggle creates/removes the LaunchAgent plist in
      `~/Library/LaunchAgents/` and the app actually launches at the next
      login when enabled.
- [ ] **Light and dark mode**: toggle System Settings' appearance while the
      window is open (or reopen it after switching). Both group boxes' fill,
      the row separators, the permission banner's orange tint, and every
      label color stay legible and correctly themed in both appearances; the
      swatches' hairline borders (especially the white swatch's, which would
      disappear against a light background without one) and the Clean
      button's teal fill/white text look correct in both. Nothing looks like
      a light-mode color stranded on a dark background or vice versa.

## App main menu (DESIGN.md "App main menu")

- [ ] While the settings window is shown (Dock icon visible), the menu bar
      shows a real app menu (its title is the app's name, per normal macOS
      behavior) with "About Urahafu", a separator, "Settings…", a separator,
      "Quit Urahafu" — and a second "Window" menu with just "Close".
- [ ] **About Urahafu** (menu item): opens the About alert (§12) — the
      window itself no longer has its own "About Urahafu" button.
- [ ] **Settings…** (menu item, ⌘,): shows the settings window, exactly like
      clicking the tray's "Settings…" item.
- [ ] **Quit Urahafu** (menu item, ⌘Q): quits the app. Start a cleaning
      session, then press ⌘Q while locked: the blocker is dropped and input
      returns to normal before the process exits (no lingering block).
- [ ] **Window › Close** (⌘W), with the settings window key/frontmost: closes
      it exactly like the window's own red button (hides if the tray icon is
      shown, quits if it's hidden — see "Settings window" above).
- [ ] **Esc**, with the settings window key: closes it, exactly as before
      (unchanged from the earlier settings-window pass) — confirm this still
      works with the app main menu now installed.
- [ ] The app main menu does not appear, or is not reachable, while a
      cleaning session's overlay is up (the app is not `Regular` at that
      point); in any case its key equivalents have no effect on a locked
      session — the event tap swallows every key regardless of what the menu
      bar underneath it would otherwise do with it.

## Localization

- [ ] With system language set to French, menu, alerts, overlay hint and HUD
      all show French text matching §14.
- [ ] With system language set to English (or any non-French language), the
      same surfaces show English text.

## Crash safety

- [ ] Start a session, then `kill -9` the Urahafu process from Terminal while
      input is blocked: keyboard and trackpad input return to normal
      afterward (verifies the watchdog thread / OS cleanup, not just the
      graceful `Drop` path).

## Performance

- [ ] With no session active (menu bar mode, idle), CPU usage in Activity
      Monitor stays near 0% over a few minutes (no busy-polling).
- [ ] During an active session with the hint/animations settled (idle
      cleaning screen), CPU usage stays low and doesn't ramp up over time.

## Checking the settings window in dark mode

Development builds (`cargo build`, not `--release`) honor `URAHAFU_APPEARANCE=dark` (or `light`),
which forces the app's appearance without touching the system setting. Launch the bundle through
`open` so that macOS attributes the Accessibility permission to Urahafu itself rather than to your
terminal:

```sh
open --env URAHAFU_APPEARANCE=dark target/release/bundle/osx/Urahafu.app
```

(Copy a debug binary into the bundle first, since the variable is compiled out of release builds.)
Launching the binary directly from a terminal that has Accessibility access gives Urahafu that
access too: a stray Return then starts a real cleaning session.

