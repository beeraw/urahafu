# Urahafu — architecture

This document is the contract between modules. Design decisions (visuals, texts, timings) live in
[`design/DESIGN.md`](../design/DESIGN.md); read it for every user-visible detail.

## Goals that shape the code

1. **The user never gets stuck.** Blocking only starts once it is proven possible; the fail-safe
   unlock works even if the main thread hangs or panics.
2. **Keystrokes are never stored, logged or sent.** Unlocking is a hold gesture — on the ✕ button,
   or holding Escape and Return together — not typing: no character is ever decoded,
   kept or sent. The input blocker maps key events to `InputEvent::KeyDown`/`KeyUp` (carrying a
   `KeyKind` of `Escape`/`Return`/`Space`/`Other`) and `ModifierChange` by virtual key code alone.
   No network code at all.
3. **Pure logic is testable without macOS.** Everything under `src/core/` compiles and is tested on
   any platform, with an injectable clock and no real waiting.
4. **`unsafe` lives in one module** (`src/platform/ffi/`), each
   block documented with `// SAFETY:`. The crate root has `#![deny(unsafe_code)]`; only the ffi
   module opts out with `#[allow(unsafe_code)]`.

## Crate layout

One package `urahafu` (edition 2024), library + thin binary.

```
build.rs                 generates Text/Language from translations/*.xlf; see "Strings and translations"
translations/            *.xlf (XLIFF 1.2), one file per language — source of truth for user-visible strings
src/main.rs              calls urahafu::run(); prints a clear error and exits non-zero on failure
src/lib.rs               module tree, `run()`, crate-level lints
src/error.rs             top-level `Error` (thiserror)
src/core/                PURE — no macOS crate, no I/O except settings file helpers
  clock.rs               `Clock` trait (`fn now(&self) -> Instant`), `SystemClock`, `ManualClock` (tests)
  easing.rs              ease_out_cubic, ease_in_out_cubic, linear fade helpers
  countdown.rs           `Countdown` — 3 s, digit + per-digit animation phase
  failsafe.rs            `FailsafeDelay` (30/60/90 s) and `Deadline` (remaining, fraction, expired)
  hint.rs                `HintFader` — opacity over time (initial show, re-show on input)
  hold_ring.rs            `HoldRing` — shared 2.0 s fill / 200 ms drain / 150 ms completion-pulse
                         state machine, used by both the pointer hold and the Esc+Return combo
  combo.rs                `ComboTracker` — the Esc+Return keyboard hold-to-unlock combo (DESIGN.md
                         §8): held-key bookkeeping driving its own `HoldRing`
  pixel_test.rs          `PixelTest` — Off → Red → Green → Blue → White → Black → Off
  color.rs               `Rgb`, `CleaningColor { Black, White }`, `ink_for(background)` (luminance rule)
  settings.rs            `Settings` + parse/serialize (`key = value`), validation, file load/save
  i18n.rs                `Language`/`Text`: generated at build time from `translations/*.xlf` by
                         `build.rs` (see "Strings" below); hand-written glue for
                         `Language::from_preferred(&[&str])` and the numeric-only formatters
  layout.rs              positions/sizes (in points) of hint, unlock button, bar, countdown, HUD for a screen size
  login_item.rs          LaunchAgent plist generation (pure string)
  session.rs             `Session` state machine → `SessionView` + `SessionCommand`s
  canvas.rs              software drawing into `&mut [u32]` (0RGB): fill, AA circle, rounded rect,
                         alpha-mask blit with opacity. Pure, testable pixel math.
src/platform/            macOS only (`#[cfg(target_os = "macos")]`)
  ffi/                   every unsafe call: CGEventTap, CFRunLoop, IsSecureEventInputEnabled,
                         AXIsProcessTrustedWithOptions, CoreText → bitmap,
                         CFLocaleCopyPreferredLanguages, and the settings window's NSObject
                         subclass (action_target.rs: target/action, window delegate, the
                         kAEReopenApplication Apple Event handler); menu_item.rs: the app main
                         menu's NSMenuItem target/action wiring (DESIGN.md "App main menu")
  input_blocker.rs       `InputBlocker::start(..) -> Result<BlockerGuard, BlockError>`; Drop removes the tap
  permission.rs          accessibility trust check, open System Settings pane
  text.rs                CoreText rasterizer: (string, size, weight, tabular) → alpha mask, cached
  overlay.rs             winit windows (one per monitor, or one HUD), softbuffer surfaces, draws SessionView
  tray.rs                tray-icon menu built from Settings + Language (reduced: grant access,
                         clean, settings, quit); menu ids → `TrayCommand`
  settings_window.rs     the native settings window (NSWindow + NSStackViews, safe code only):
                         builds/owns every control, `show`/`hide`/`is_visible`/`update`; decodes
                         control actions (via ffi/action_target.rs) into a tag the app layer maps
                         to a `WindowCommand`; exposes its own `ActionTarget` for reuse by
                         `app_menu.rs`
  app_menu.rs             the app main menu bar (safe code only, DESIGN.md "App main menu"): builds and
                         installs the App/Window menus once at startup, reusing the settings
                         window's `ActionTarget`
  alert.rs               native alerts (the three Safety-net error alerts, about)
  login_item.rs          writes/removes ~/Library/LaunchAgents/<bundle id>.plist
  system.rs              preferred languages, open URL (`/usr/bin/open`), home directory
src/app/                 winit `ApplicationHandler`: args.rs (parses `--login`), startup.rs,
                         clean.rs, menu.rs, window.rs, handler.rs — see that module's own docs
tests/                   integration tests of core flows (full session with ManualClock, settings files)
```

## Core types (contract)

Unlocking is a **hold**, not a typed sequence: the user presses and holds an on-screen ✕ button
for 2 seconds, or (DESIGN.md §8) holds Escape and Return together for the same duration.
`session.rs` has no character matcher at all — `core::sequence`/`core::keymap` were deleted along
with the typed "urahafu" path.

```rust
// session.rs
//
// Coordinate convention for every pointer variant: main-screen logical points, origin at the
// top-left (documented on the type itself). The input-blocker's tap events already are in this
// space (`CGEventGetLocation`); winit's `CursorMoved` (window-local) needs its window's origin
// added first — see `src/app/coords.rs`.
pub enum KeyKind { Escape, Return, Space, Other }
pub enum InputEvent {
    KeyDown { kind: KeyKind, repeat: bool }, KeyUp(KeyKind), ModifierChange,
    PointerDown { x: f32, y: f32 }, PointerDragged { x: f32, y: f32 }, PointerUp { x: f32, y: f32 },
    PointerMoved { x: f32, y: f32 }, Scroll,
}
pub enum SessionPhase { Countdown, Locked, Unlocking, Finished(EndReason) }
pub enum EndReason { Unlocked, Failsafe, Cancelled }
pub enum SessionCommand { BlockInputs, ReleaseInputs, Close }
pub struct SessionConfig { pub color: CleaningColor, pub failsafe: FailsafeDelay, pub keyboard_only: bool, pub language: Language }
impl Session {
    pub fn new(config: SessionConfig, now: Instant) -> Self;          // starts in Countdown
    pub fn set_unlock_target(&mut self, center: Point, radius: f32, hit_radius: f32); // where the ✕ button currently is
    pub fn handle_input(&mut self, event: InputEvent, now: Instant) -> Vec<SessionCommand>;
    pub fn tick(&mut self, now: Instant) -> Vec<SessionCommand>;       // countdown end → BlockInputs; hold completion/fail-safe → ReleaseInputs; fade end → Close
    pub fn view(&self, now: Instant) -> SessionView;                  // everything the renderer needs, no logic in renderer
    pub fn next_wake(&self, now: Instant) -> Option<Instant>;         // when to redraw next (animation ~60 fps, or next timer)
    pub fn phase(&self) -> SessionPhase;
}
pub struct UnlockButtonView {
    pub center: Point, pub radius: f32, pub hit_radius: f32,
    pub opacity: f32, pub hold_progress: f32, pub scale: f32,
}
pub struct SessionView {
    pub background: Rgb, pub ink: Rgb, pub keyboard_only: bool,
    pub countdown: Option<CountdownView>,   // digit, opacity, scale
    pub hint_opacity: f32, pub wordmark_opacity: f32,
    pub unlock_button: Option<UnlockButtonView>,
    pub countdown_unlock_hint: bool,        // the countdown screen's explanation line
    pub bar: Option<BarView>,               // fraction remaining, opacity (None during pixel test)
    pub pixel_step: Option<(u8, u8)>,       // (2, 5)
    pub remaining_secs: u32,
    pub overlay_opacity: f32,               // unlock fade
}
```

Rules (see DESIGN.md for numbers): `KeyDown{Escape,..}` or `PointerDown` during Countdown →
Cancelled; the button is shown pulsing there but not holdable. A hold starts only on a
`PointerDown` inside the button's hit target while Locked, and progresses linearly to completion
over 2.0 s (`core::hold_ring::HoldRing`); releasing early or leaving the hit target drains the
progress ring back to 0 over 200 ms. Completion → a 150 ms scale pulse (1.0→1.1), then the normal
unlock sequence: `ReleaseInputs`, entering `Unlocking`, a 300 ms overlay fade, `Close`.
`ReleaseInputs` is emitted **when entering Unlocking** (inputs are freed before the fade), whether
that transition came from a completed hold, a completed keyboard combo, or the fail-safe deadline.
Hovering within 160 pt of the button re-shows the hint+button and keeps them visible for as long as
the pointer stays there; any other blocked input (`KeyDown`/`ModifierChange`/`Scroll`/`PointerDown`
outside the target) re-shows them on the hint's normal fade timeline. Space during Locked toggles
through the pixel test (outside keyboard-only mode) without affecting either hold; it also counts
as "another key" for the keyboard combo below.

**Esc+Return keyboard combo** (`core::combo::ComboTracker`, DESIGN.md §8): while Locked,
every `KeyDown`/`KeyUp`/`ModifierChange` event is fed to a `ComboTracker`, which tracks whether
Escape and Return, and only Escape and Return, are currently down and drives its own `HoldRing`
with the identical 2.0 s/200 ms/150 ms timing as the pointer hold. `Session::view`'s
`UnlockButtonView::hold_progress`/`scale` are the **max** of the pointer `HoldRing` and the combo's
own, so either gesture — or both at once — drives the same on-screen button. Either `HoldRing`
completing (`HoldRing::completion_elapsed`) transitions to `Unlocking` exactly like the pointer
hold does today; the fail-safe deadline is still checked independently every tick a hold isn't
mid-completion-pulse, so a partial combo hold never delays it. A key-down for another key (Space
included) or a `ModifierChange` while the combo is engaged resets its progress; see
`core::combo`'s own module docs for the full state machine and, in particular, why a
`ModifierChange` additionally blocks the combo from restarting until Escape or Return is actually
released and re-pressed (a modifier flag change carries no down/up information to track a
persistent "held" state the way a real key's down/up pair can). Key autorepeat
(`InputEvent::KeyDown.repeat`) is filtered out of the combo's own start/reset decisions in
`session.rs`'s `combo_event_for`, though it still reshows the hint like any other key-down,
unchanged from before this event carried the distinction.

## Safety net (input blocker)

- Refuse to start (typed `BlockError`) when: accessibility not trusted, Secure Input enabled, tap
  creation fails. The app then shows the matching alert and never shows the lock screen.
- The tap runs on its own thread with its own CFRunLoop. Shared state: `Arc<AtomicBool> active` and
  the deadline (`Instant` captured at start + fail-safe delay, read-only).
- The callback **passes events through** as soon as `active` is false or the deadline is past; it
  re-enables the tap on `kCGEventTapDisabledByTimeout`/`ByUserInput`.
- A watchdog thread wakes at deadline + 2 s and disables/removes the tap itself, independently of
  the main thread.
- A panic hook (installed once) flips every live `active` flag to false before the default hook.
- `BlockerGuard::drop` disables the tap, stops the run loop, joins the thread.
- The callback maps key-down/key-up events to `InputEvent::KeyDown`/`KeyUp` (a `KeyKind` of
  `Escape`/`Return`/`Space`/`Other` — the numeric keypad's Enter key maps to `Return` too — plus,
  for `KeyDown`, whether it's an OS autorepeat) by virtual key code alone, and modifier-flag
  changes to `InputEvent::ModifierChange` — no character is ever decoded, logged, or kept (neither
  the hold-to-unlock button nor the Esc+Return combo needs one). Pointer down/dragged/up events
  carry their location via `CGEventGetLocation` (global points, origin top-left of the main
  display — already the exact coordinate space `InputEvent`'s pointer variants document); scroll
  events map to `InputEvent::Scroll`. Every event is still swallowed regardless of what it maps to.
- Plain pointer movement (`mouseMoved`, no button held) is deliberately not part of the tap's mask
  at all: the hover reveal instead reads winit's own `CursorMoved` in the app layer, converted into
  the session's coordinate space by `src/app/coords.rs`'s pure helper.

## Settings

File `~/Library/Application Support/Urahafu/settings.conf`, `key = value`, unknown keys ignored,
invalid values replaced by defaults (per key). Keys: `color` (black|white), `failsafe_seconds`
(30|60|90), `keyboard_only` (true|false), `show_menu_bar_icon` (true|false). Login item state is
not stored: it is the presence of the LaunchAgent plist. A settings file written by an older
Urahafu may still have a `welcome_seen` key (the now-removed welcome alert, DESIGN.md's old §10);
the loader keeps silently ignoring it like any other unknown key, and nothing writes it anymore.

## Strings and translations

All user-visible strings come from `core::i18n::Text`. Unlike before, `Text` and `Language` are not
hand-written: `build.rs` reads `translations/*.xlf` (XLIFF 1.2, one file per language) at compile
time and generates `$OUT_DIR/i18n_generated.rs`, which `src/core/i18n.rs` brings in with
`include!`. `translations/en.xlf` is the source of truth for the set of keys — a `Text` variant
exists if and only if it has a `<trans-unit>` in `en.xlf` — and for each key's canonical
`{placeholder}` set, which every other language's `<target>` must match exactly (a mismatch is a
build error; a missing or empty `<target>` falls back to English with a `cargo:warning`). Adding or
removing a shipped language is exactly adding or removing one `translations/<tag>.xlf` file — no
Rust, `Cargo.toml` or `packaging/` change. `docs/TRANSLATIONS.md` is the full guide (file format,
adding a language, translator tooling, the language-detection algorithm). `build.rs` also renders
`target/generated/Info.ext.plist` from `packaging/Info.ext.plist.in`, filling in
`CFBundleLocalizations` from the same discovered files; `Cargo.toml`'s `osx_info_plist_exts` points
at that generated, stable path. Console/log output stays in English and untranslated.

## Dependencies (keep minimal, justify each in README)

thiserror · winit · softbuffer · tray-icon · core-foundation · core-graphics · core-text ·
objc2 / objc2-foundation / objc2-app-kit for `NSAlert` presentation, the settings window
(`NSWindow`, `NSStackView` and friends, plus the `kAEReopenApplication` Apple Event via
objc2-core-services) and the app main menu (`NSMenu`/`NSMenuItem`, DESIGN.md "App main menu") — align
versions with the ones winit and tray-icon already pull in, to avoid duplicates.
No serde, no image decoder (the menu bar icon is embedded as raw RGBA generated by the design script).
roxmltree is a build-dependency only (parses `translations/*.xlf` in `build.rs`); it is never
compiled into or linked with the shipped binary.
