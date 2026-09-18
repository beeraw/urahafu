//! Renders a fixed set of DESIGN.md snapshots (countdown, black/white cleaning screens, the
//! dead-pixel test, and both keyboard-only HUD appearances) to PNG files, using the real
//! `core::session::Session` state machine (driven by hand-picked `Instant`s, exactly like
//! `tests/session_flow.rs` — no real waiting) and the real CoreText-backed text rasterizer. No
//! window, no winit event loop, no event tap: everything is drawn offscreen into a `Vec<u32>`
//! buffer and converted to RGBA for the PNG encoder.
//!
//! This is a dev-only example (`png` is a `[dev-dependency]`, never a normal one) for visually
//! comparing the renderer's output against `design/mockups/`.
//!
//! # How to run
//!
//! ```sh
//! cargo run --example render_preview -- <output-directory>
//! ```
//!
//! `<output-directory>` is any writable directory path; it is created if it does not already
//! exist. The base set (English) plus a handful of script-coverage snapshots are written there:
//! - `countdown.png`, `cleaning-black.png`, `cleaning-white.png`, `pixel-test.png` — English,
//!   DESIGN.md's core states.
//! - `keyboard-only-dark.png`, `keyboard-only-light.png` — English HUD, dark/light appearance.
//! - `countdown-<tag>.png` / `cleaning-black-<tag>.png` for `tag` in `ja`, `hi`, `ta`, `th`, `ko`,
//!   `or` — CoreText's system-font fallback exercised against non-Latin scripts (Japanese/CJK,
//!   Devanagari, Tamil, Thai, Hangul, Odia): these must render real glyphs, never tofu/empty boxes,
//!   and the cleaning-black hint's text box must not clip the tall ascenders/descenders some of
//!   these scripts have.
//! - `keyboard-only-dark-ar.png` / `keyboard-only-dark-ur.png` / `keyboard-only-dark-he.png` — RTL
//!   mirroring (right-to-left layout, Arabic, Urdu and Hebrew).
//! - `cleaning-black-he.png` — Hebrew cleaning-black state, RTL text shaping check.
//! - `keyboard-only-dark-de.png` — German, whose compound words run long, to check the HUD pill
//!   still fits them.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "this example converts constantly between pixel counts (u32/usize/i64) and point \
              positions (f32); every value is a fixed, small preview-canvas dimension (1440x900 \
              @2x or smaller), far below any of these types' precision-loss or overflow ranges"
)]
use std::env;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use urahafu::core::color::CleaningColor;
use urahafu::core::countdown::Countdown;
use urahafu::core::failsafe::FailsafeDelay;
use urahafu::core::i18n::Language;
use urahafu::core::layout::{self, Layout, ScreenSize};
use urahafu::core::session::{InputEvent, KeyKind, Session, SessionConfig};
use urahafu::platform::render::{self, Appearance, ScreenRole};
use urahafu::platform::text::TextRasterizer;

/// Reference screen size (DESIGN.md §5: "relative to a reference screen of 1440x900 pt").
const SCREEN_WIDTH_PT: f32 = 1440.0;
/// Reference screen height.
const SCREEN_HEIGHT_PT: f32 = 900.0;
/// `HiDPI` rasterization scale (2x).
const SCALE: f32 = 2.0;

fn main() {
    let Some(out_dir) = env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: cargo run --example render_preview -- <output-directory>");
        std::process::exit(1);
    };
    if let Err(err) = std::fs::create_dir_all(&out_dir) {
        eprintln!(
            "urahafu render_preview: failed to create {}: {err}",
            out_dir.display()
        );
        std::process::exit(1);
    }

    let mut rasterizer = TextRasterizer::new();

    render_countdown(&mut rasterizer, &out_dir);
    render_cleaning_black(&mut rasterizer, &out_dir);
    render_cleaning_white(&mut rasterizer, &out_dir);
    render_pixel_test(&mut rasterizer, &out_dir);
    render_keyboard_only(
        &mut rasterizer,
        &out_dir,
        Appearance::Dark,
        Language::ENGLISH,
        "keyboard-only-dark.png",
        true,
    );
    render_keyboard_only(
        &mut rasterizer,
        &out_dir,
        Appearance::Light,
        Language::ENGLISH,
        "keyboard-only-light.png",
        false,
    );
    render_keyboard_only_rtl(&mut rasterizer, &out_dir, "ar");
    render_keyboard_only_rtl(&mut rasterizer, &out_dir, "ur");
    render_keyboard_only_rtl(&mut rasterizer, &out_dir, "he");
    render_cleaning_black_for_language(&mut rasterizer, &out_dir, "he");
    render_keyboard_only(
        &mut rasterizer,
        &out_dir,
        Appearance::Dark,
        language("de"),
        "keyboard-only-dark-de.png",
        true,
    );

    // Script-coverage snapshots: CoreText's system-font fallback exercised against non-Latin
    // scripts, and the cleaning-black hint's text box checked against tall Indic ascenders /
    // descenders (Devanagari, Tamil, Odia).
    for tag in ["ja", "hi", "ta", "th", "ko", "or"] {
        render_countdown_for_language(&mut rasterizer, &out_dir, tag);
        render_cleaning_black_for_language(&mut rasterizer, &out_dir, tag);
    }

    println!("wrote PNGs to {}", out_dir.display());
}

/// Looks up a shipped [`Language`] by BCP-47 tag, panicking with a clear message if it isn't
/// shipped — every tag this example passes here is one of Urahafu's `translations/*.xlf` files, so
/// a lookup failure means the file was renamed or removed, not a normal runtime condition.
#[allow(
    clippy::panic,
    reason = "this dev-only example looks up a handful of BCP-47 tags that Urahafu always ships \
              translations for; if one were ever removed, failing loudly here (not in production \
              code) is exactly what should happen"
)]
fn language(tag: &str) -> Language {
    Language::from_tag(tag)
        .unwrap_or_else(|| panic!("urahafu render_preview: no shipped language for tag {tag:?}"))
}

fn base_config(color: CleaningColor, keyboard_only: bool) -> SessionConfig {
    SessionConfig {
        color,
        failsafe: FailsafeDelay::Seconds60,
        keyboard_only,
        language: Language::ENGLISH,
    }
}

/// The reference-screen layout (DESIGN.md §5), shared by every full-screen snapshot to compute
/// the hold-to-unlock button's target geometry via the real `core::layout` functions — the same
/// ones the app layer feeds into `Session::set_unlock_target`.
fn reference_layout() -> Layout {
    Layout::new(ScreenSize::new(SCREEN_WIDTH_PT, SCREEN_HEIGHT_PT))
}

/// Snapshot 1: countdown showing "3" (mid-hold of the first digit, fully faded in), with the
/// hold-to-unlock explanation and the pulsing (not yet holdable) ✕ button.
fn render_countdown(rasterizer: &mut TextRasterizer, out_dir: &std::path::Path) {
    let start = Instant::now();
    let mut session = Session::new(base_config(CleaningColor::Black, false), start);
    let layout = reference_layout();
    session.set_unlock_target(
        layout.unlock_button(),
        layout::UNLOCK_BUTTON_RADIUS,
        layout::UNLOCK_BUTTON_HIT_RADIUS,
    );
    let at = start + Duration::from_millis(500);
    let view = session.view(at);

    let mut buffer = pixel_buffer();
    render::render_overlay(
        &mut buffer,
        pixel_width(),
        pixel_height(),
        SCALE,
        ScreenRole::Main,
        &view,
        Language::ENGLISH,
        rasterizer,
    );
    write_png(&buffer, &out_dir.join("countdown.png"));
}

/// Snapshot 2: black overlay, locked, initial hint still visible (no input yet), ✕ button idle.
fn render_cleaning_black(rasterizer: &mut TextRasterizer, out_dir: &std::path::Path) {
    let start = Instant::now();
    let mut session = Session::new(base_config(CleaningColor::Black, false), start);
    let layout = reference_layout();
    session.set_unlock_target(
        layout.unlock_button(),
        layout::UNLOCK_BUTTON_RADIUS,
        layout::UNLOCK_BUTTON_HIT_RADIUS,
    );
    let locked_at = start + Countdown::TOTAL;
    session.tick(locked_at);
    let view = session.view(locked_at);

    let mut buffer = pixel_buffer();
    render::render_overlay(
        &mut buffer,
        pixel_width(),
        pixel_height(),
        SCALE,
        ScreenRole::Main,
        &view,
        Language::ENGLISH,
        rasterizer,
    );
    write_png(&buffer, &out_dir.join("cleaning-black.png"));
}

/// Like [`render_countdown`], but in the given shipped language, written to `countdown-<tag>.png`
/// — used for script-coverage snapshots (CoreText's system-font fallback to non-Latin scripts).
fn render_countdown_for_language(
    rasterizer: &mut TextRasterizer,
    out_dir: &std::path::Path,
    tag: &str,
) {
    let start = Instant::now();
    let mut session = Session::new(
        SessionConfig {
            language: language(tag),
            ..base_config(CleaningColor::Black, false)
        },
        start,
    );
    let layout = reference_layout();
    session.set_unlock_target(
        layout.unlock_button(),
        layout::UNLOCK_BUTTON_RADIUS,
        layout::UNLOCK_BUTTON_HIT_RADIUS,
    );
    let at = start + Duration::from_millis(500);
    let view = session.view(at);

    let mut buffer = pixel_buffer();
    render::render_overlay(
        &mut buffer,
        pixel_width(),
        pixel_height(),
        SCALE,
        ScreenRole::Main,
        &view,
        language(tag),
        rasterizer,
    );
    write_png(&buffer, &out_dir.join(format!("countdown-{tag}.png")));
}

/// Like [`render_cleaning_black`], but in the given shipped language, written to
/// `cleaning-black-<tag>.png` — used for script-coverage snapshots. The cleaning-black hint is the
/// tallest piece of translated text shown at once (dead-pixel-test hint plus auto-unlock
/// countdown, `overlay.hint.secondary`), which makes it the right snapshot to eyeball for clipping
/// on scripts with tall ascenders/descenders (Devanagari, Tamil, Odia).
fn render_cleaning_black_for_language(
    rasterizer: &mut TextRasterizer,
    out_dir: &std::path::Path,
    tag: &str,
) {
    let start = Instant::now();
    let mut session = Session::new(
        SessionConfig {
            language: language(tag),
            ..base_config(CleaningColor::Black, false)
        },
        start,
    );
    let layout = reference_layout();
    session.set_unlock_target(
        layout.unlock_button(),
        layout::UNLOCK_BUTTON_RADIUS,
        layout::UNLOCK_BUTTON_HIT_RADIUS,
    );
    let locked_at = start + Countdown::TOTAL;
    session.tick(locked_at);
    let view = session.view(locked_at);

    let mut buffer = pixel_buffer();
    render::render_overlay(
        &mut buffer,
        pixel_width(),
        pixel_height(),
        SCALE,
        ScreenRole::Main,
        &view,
        language(tag),
        rasterizer,
    );
    write_png(&buffer, &out_dir.join(format!("cleaning-black-{tag}.png")));
}

/// Snapshot 3: white overlay, hint re-shown after a blocked key, ✕ button mid-hold (~60 % of the
/// 2 s hold, driven through the real `PointerDown` + `tick` API rather than a faked view — DESIGN.md
/// §8's hold-progress ring).
fn render_cleaning_white(rasterizer: &mut TextRasterizer, out_dir: &std::path::Path) {
    let start = Instant::now();
    let mut session = Session::new(base_config(CleaningColor::White, false), start);
    let layout = reference_layout();
    let button_center = layout.unlock_button();
    session.set_unlock_target(
        button_center,
        layout::UNLOCK_BUTTON_RADIUS,
        layout::UNLOCK_BUTTON_HIT_RADIUS,
    );
    let locked_at = start + Countdown::TOTAL;
    session.tick(locked_at);
    session.handle_input(
        InputEvent::KeyDown {
            kind: KeyKind::Other,
            repeat: false,
        },
        locked_at,
    );
    // A blocked key re-shows the hint from opacity 0 (DESIGN.md §8: 200 ms fade-in on every
    // blocked input, `HintFader::on_input`); viewing at `locked_at` itself would catch that
    // fade-in at its very first frame (opacity 0, hint invisible) even though every other
    // on-screen element is already in its final state. View a moment later, once the fade-in has
    // completed, so this snapshot shows the hint at its steady-state opacity like the other
    // snapshots do.
    let after_reshow = locked_at + Duration::from_millis(500);
    session.handle_input(
        InputEvent::PointerDown {
            x: button_center.x,
            y: button_center.y,
        },
        after_reshow,
    );
    // 60% of the 2.0s hold duration (DESIGN.md §8/§13).
    let mid_hold = after_reshow + Duration::from_millis(1200);
    session.tick(mid_hold);
    let view = session.view(mid_hold);

    let mut buffer = pixel_buffer();
    render::render_overlay(
        &mut buffer,
        pixel_width(),
        pixel_height(),
        SCALE,
        ScreenRole::Main,
        &view,
        Language::ENGLISH,
        rasterizer,
    );
    write_png(&buffer, &out_dir.join("cleaning-white.png"));
}

/// Snapshot 4: dead-pixel test on its green step (2/5), hint still visible (still within the
/// initial 4 s hold, so the pixel-test Space presses alone do not hide it), with the ✕ button
/// (ink chosen for legibility against green, per DESIGN.md §8's luminance rule).
fn render_pixel_test(rasterizer: &mut TextRasterizer, out_dir: &std::path::Path) {
    let start = Instant::now();
    let mut session = Session::new(base_config(CleaningColor::Black, false), start);
    let layout = reference_layout();
    session.set_unlock_target(
        layout.unlock_button(),
        layout::UNLOCK_BUTTON_RADIUS,
        layout::UNLOCK_BUTTON_HIT_RADIUS,
    );
    let locked_at = start + Countdown::TOTAL;
    session.tick(locked_at);
    // Off -> Red -> Green: two Space presses.
    session.handle_input(
        InputEvent::KeyDown {
            kind: KeyKind::Space,
            repeat: false,
        },
        locked_at,
    );
    session.handle_input(
        InputEvent::KeyDown {
            kind: KeyKind::Space,
            repeat: false,
        },
        locked_at,
    );
    let view = session.view(locked_at);

    let mut buffer = pixel_buffer();
    render::render_overlay(
        &mut buffer,
        pixel_width(),
        pixel_height(),
        SCALE,
        ScreenRole::Main,
        &view,
        Language::ENGLISH,
        rasterizer,
    );
    write_png(&buffer, &out_dir.join("pixel-test.png"));
}

/// Snapshots 5/6: the keyboard-only HUD pill, dark and light appearance, rendered at its
/// DESIGN.md §9 position onto a full reference-screen canvas so it can be compared to the
/// mockups in context (the pill itself is only as large as its content — see `render_hud`'s doc
/// comment on the opaque-backing transparency fallback). `mid_hold` drives an actual ~50 % hold
/// through the real `PointerDown` + `tick` API (rather than faking the view) for one of the two
/// appearances, so the HUD's accent hold-progress ring (DESIGN.md §9) shows up in at least one
/// snapshot.
#[allow(
    clippy::too_many_arguments,
    reason = "a preview snapshot function inherently needs the shared rasterizer/output dir plus \
              the handful of knobs (appearance, language, file name, hold state) that vary \
              between the snapshots calling it"
)]
fn render_keyboard_only(
    rasterizer: &mut TextRasterizer,
    out_dir: &std::path::Path,
    appearance: Appearance,
    language: Language,
    file_name: &str,
    mid_hold: bool,
) {
    let rtl = language.is_rtl();
    let start = Instant::now();
    let mut session = Session::new(
        SessionConfig {
            language,
            ..base_config(CleaningColor::Black, true)
        },
        start,
    );
    let locked_at = start + Countdown::TOTAL;
    session.tick(locked_at);
    session.handle_input(
        InputEvent::KeyDown {
            kind: KeyKind::Other,
            repeat: false,
        },
        locked_at,
    );

    // The HUD button's target depends on the pill's width, which in turn depends on the HUD's
    // own content (language, remaining time) — compute it from a first view, exactly as the app
    // layer's `update_unlock_target` does, before telling the session where the button is.
    let unsized_view = session.view(locked_at);
    let content_width =
        render::hud_content_width_points(rasterizer, &unsized_view, language).max(160.0);
    let layout = reference_layout();
    let pill_rect = layout.hud_pill(content_width);
    session.set_unlock_target(
        layout::hud_unlock_button(pill_rect, rtl),
        layout::HUD_UNLOCK_BUTTON_RADIUS,
        layout::HUD_UNLOCK_BUTTON_HIT_RADIUS,
    );

    let view_at = if mid_hold {
        let button_center = layout::hud_unlock_button(pill_rect, rtl);
        session.handle_input(
            InputEvent::PointerDown {
                x: button_center.x,
                y: button_center.y,
            },
            locked_at,
        );
        // ~50% of the 2.0s hold duration (DESIGN.md §8/§13).
        let mid = locked_at + Duration::from_millis(1000);
        session.tick(mid);
        mid
    } else {
        locked_at
    };
    let view = session.view(view_at);

    // A neutral backdrop stands in for the desktop the HUD would float over in a real
    // compositor; the pill itself is opaque (see the transparency-fallback note in
    // `render.rs`/`overlay.rs`), so this backdrop is only for visual context in the preview.
    let mut buffer = pixel_buffer();
    let backdrop = urahafu::core::color::Rgb::new(0x3a, 0x3a, 0x3c);
    for pixel in &mut buffer {
        *pixel = backdrop.to_u32();
    }

    let pill_width_px = (content_width * SCALE).round() as u32;
    let pill_height_px = (layout::HUD_PILL_HEIGHT * SCALE).round() as u32;
    let mut pill_buffer = vec![0u32; (pill_width_px * pill_height_px) as usize];
    render::render_hud(
        &mut pill_buffer,
        pill_width_px,
        pill_height_px,
        SCALE,
        appearance,
        &view,
        language,
        rasterizer,
    );

    let offset_x = (pill_rect.x * SCALE).round() as i64;
    let offset_y = (pill_rect.y * SCALE).round() as i64;
    blit(
        &mut buffer,
        pixel_width(),
        pixel_height(),
        &pill_buffer,
        pill_width_px,
        pill_height_px,
        offset_x,
        offset_y,
    );

    write_png(&buffer, &out_dir.join(file_name));
}

/// The keyboard-only HUD pill, dark appearance, in a right-to-left shipped language named by
/// `tag` (`ar`, `ur`) — to eyeball the RTL mirroring described in `render_hud`'s doc comment (✕
/// button on the left end, title + glyph on the right, hold-progress ring still clockwise)
/// against a real CoreText-shaped run, which a synthetic/fake rasterizer can't exercise. Written
/// to `keyboard-only-dark-<tag>.png`.
fn render_keyboard_only_rtl(rasterizer: &mut TextRasterizer, out_dir: &std::path::Path, tag: &str) {
    render_keyboard_only(
        rasterizer,
        out_dir,
        Appearance::Dark,
        language(tag),
        &format!("keyboard-only-dark-{tag}.png"),
        true,
    );
}

/// Copies `src` (opaque, no alpha) onto `dst` at `(dest_x, dest_y)`, clipped to `dst`'s bounds.
#[allow(
    clippy::too_many_arguments,
    reason = "a plain opaque pixel-rect copy inherently needs both buffers' dimensions plus a \
              destination offset"
)]
#[allow(
    clippy::similar_names,
    reason = "dst_x/dst_y and src_x/src_y are deliberately paired coordinate names; renaming \
              them apart would make the copy geometry harder to follow, not easier"
)]
fn blit(
    dst: &mut [u32],
    dst_width: u32,
    dst_height: u32,
    src: &[u32],
    src_width: u32,
    src_height: u32,
    dest_x: i64,
    dest_y: i64,
) {
    for src_y in 0..i64::from(src_height) {
        let dst_y = dest_y + src_y;
        if dst_y < 0 || dst_y >= i64::from(dst_height) {
            continue;
        }
        for src_x in 0..i64::from(src_width) {
            let dst_x = dest_x + src_x;
            if dst_x < 0 || dst_x >= i64::from(dst_width) {
                continue;
            }
            #[allow(
                clippy::cast_sign_loss,
                reason = "dst_x/dst_y/src_x/src_y are checked non-negative just above"
            )]
            let (dst_index, src_index) = (
                (dst_y as u64 * u64::from(dst_width) + dst_x as u64) as usize,
                (src_y as u64 * u64::from(src_width) + src_x as u64) as usize,
            );
            dst[dst_index] = src[src_index];
        }
    }
}

fn pixel_width() -> u32 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "1440pt * 2.0 scale is a small, fixed, positive preview canvas size"
    )]
    let width = (SCREEN_WIDTH_PT * SCALE) as u32;
    width
}

fn pixel_height() -> u32 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "900pt * 2.0 scale is a small, fixed, positive preview canvas size"
    )]
    let height = (SCREEN_HEIGHT_PT * SCALE) as u32;
    height
}

fn pixel_buffer() -> Vec<u32> {
    vec![0u32; (pixel_width() * pixel_height()) as usize]
}

/// Writes `buffer` (`0x00RRGGBB` words, per `core::canvas`) as an opaque RGBA PNG at `path`.
fn write_png(buffer: &[u32], path: &std::path::Path) {
    let width = pixel_width();
    let height = pixel_height();
    let mut rgba = Vec::with_capacity(buffer.len() * 4);
    for &pixel in buffer {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "each channel is masked to 8 bits just below"
        )]
        let (r, g, b) = (
            ((pixel >> 16) & 0xFF) as u8,
            ((pixel >> 8) & 0xFF) as u8,
            (pixel & 0xFF) as u8,
        );
        rgba.extend_from_slice(&[r, g, b, 0xFF]);
    }

    let file = match std::fs::File::create(path) {
        Ok(file) => file,
        Err(err) => {
            eprintln!(
                "urahafu render_preview: failed to create {}: {err}",
                path.display()
            );
            std::process::exit(1);
        }
    };
    let writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = match encoder.write_header() {
        Ok(writer) => writer,
        Err(err) => {
            eprintln!(
                "urahafu render_preview: failed to write PNG header for {}: {err}",
                path.display()
            );
            std::process::exit(1);
        }
    };
    if let Err(err) = writer.write_image_data(&rgba) {
        eprintln!(
            "urahafu render_preview: failed to write PNG data for {}: {err}",
            path.display()
        );
        std::process::exit(1);
    }
}
