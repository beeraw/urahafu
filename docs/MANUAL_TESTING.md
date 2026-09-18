# Manual testing checklist

Everything below depends on real macOS behavior (permissions, event taps,
window server, hardware keys) and can't be exercised by `cargo test`. Run
through this list before tagging a release, on at least one Intel and one
Apple Silicon Mac if you have access to both.

## Accessibility permission

### Reset between passes

Every scenario below starts from "as if freshly installed", but macOS ties the
Accessibility grant to the **app's code signature**. Urahafu is ad-hoc-signed
in dev builds, so each rebuild can get a new signature — the grant survives
some rebuilds and silently disappears after others, inconsistently. Before
each fresh-install-style scenario, reset both halves of first-run state:

```
tccutil reset Accessibility com.beeraw.urahafu
rm ~/Library/Application\ Support/Urahafu/settings.conf
```

The first command revokes the Accessibility grant (so the next launch sees
"not trusted"); the second clears `welcome_seen` and every other saved
setting (so the next launch is a genuine first run). Without the second
command, the welcome alert (§10) won't reappear even though the permission
alert might.

### Scenarios

- [ ] **Fresh install, permission not yet granted**: reset both (above), then
      launch. The welcome alert (§10 "Welcome to Urahafu") appears first,
      with a single "Continue" button. Clicking it immediately shows the
      "needs Accessibility access" alert (§10, second table), no cleaning
      overlay shows at any point. The menu (behind the alerts) already shows
      the "Allow Accessibility Access…" item above "Clean Screen" (§6).
- [ ] "Open System Settings" (on the permission alert) opens Privacy &
      Security › Accessibility.
- [ ] **Fresh install, permission already granted** (e.g. granted manually in
      System Settings before ever opening the app, or a previous install's
      grant survived this build's signature): reset only the settings file
      (not the Accessibility grant), then launch. Only the welcome alert
      appears, this time with a single "OK" button (no permission alert
      chained after it) — this is the bug this feature fixes: previously the
      app started completely silently in this case, with nothing but a small
      menu bar icon and no explanation. The menu does **not** show the
      "Allow Accessibility Access…" item.
- [ ] Relaunching after the welcome alert was already shown (`welcome_seen`
      now true) never shows it again, regardless of permission state.
- [ ] **"Later" then manual grant**: reset both, launch, click "Later" on the
      permission alert (no System Settings window opens). Without touching
      the app again, grant Accessibility manually in System Settings. Within
      ~2 s the "You're all set!" confirmation (§10, third table) appears on
      its own and the "Allow Accessibility Access…" menu item disappears —
      this is the permission watcher (§10 "Permission monitoring"), which
      polls regardless of how the alert was dismissed.
- [ ] **Grant from the permission-required error alert**: reset both, launch,
      dismiss the welcome and first-launch alerts however you like while
      leaving the permission ungranted. From the menu, click "Clean Screen"
      (or the new "Allow Accessibility Access…" item): the permission-required
      error alert (§11 case 1) appears. Click
      "Open System Settings" and grant access there. The "You're all set!"
      confirmation appears within ~2 s, exactly as in the previous scenario.
- [ ] Grant Accessibility while a cleaning session is in progress (started
      via direct-launch mode, or already unlocked with the permission
      re-granted mid-session): no alert interrupts the session; the "You're
      all set!" confirmation appears only once the app is back at idle
      (menu-bar, between sessions).
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
      unlock, hold the close button for 2 seconds") is present and readable,
      and the ✕ button pulses gently at
      its final position but is **not** holdable yet — pressing and holding
      it during the countdown does not start a progress ring, and a click
      still cancels the countdown as usual.

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

## Launch modes

- [ ] Normal launch (menu bar icon visible): clicking the icon shows the
      menu described in §6.
- [ ] Direct-launch mode (menu bar icon hidden via the confirmation dialog
      in §6): opening the app starts cleaning immediately, no menu.
- [ ] Holding Option while opening the app in direct-launch mode brings back
      the menu bar icon/menu for that launch.
- [ ] "Open at Login" toggle creates/removes the LaunchAgent plist in
      `~/Library/LaunchAgents/` and the app actually launches at the next
      login when enabled.
- [ ] "Open at Login" is greyed out when the menu bar icon is hidden
      (direct-launch mode), per §6.

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
