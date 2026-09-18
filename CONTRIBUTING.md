# Contributing to Urahafu

Thanks for considering a contribution. A few rules keep the codebase
consistent and the app safe to run:

- **Code is written in English.** Identifiers, comments, docblocks and commit
  messages — no French in source code. Translations of user-facing text live
  in `core::i18n`, not in identifiers.
- **`cargo fmt --all`, `cargo clippy --all-targets -- -D warnings` and
  `cargo test` must pass** before you open a pull request. CI enforces the
  same checks (see `.github/workflows/ci.yml`), plus `cargo doc`,
  `cargo audit` and `cargo deny check`.
- **No `unwrap()`/`expect()` in production code paths.** Handle errors
  explicitly and surface them through `crate::error::Error`. `unwrap()` is
  acceptable in tests.
- **`unsafe` only lives in `src/platform/ffi/`.**
  The rest of the crate builds with `#![deny(unsafe_code)]`. Every `unsafe`
  block needs a `// SAFETY:` comment explaining why it's sound.
- **Every user-visible string goes through `core::i18n::Text`.** `Text` and
  `Language` are generated at build time from `translations/*.xlf`
  (`build.rs`); a new string is added by adding a `<trans-unit>` to
  `translations/en.xlf`, not by editing Rust. See `docs/TRANSLATIONS.md`.

**Translators don't need any of the above.** Adding or improving a
translation is purely editing a `translations/<tag>.xlf` file (XLIFF 1.2, a
format Poedit/Weblate/Crowdin open natively) — no Rust toolchain, no build,
no pull request checklist beyond the file itself. `docs/TRANSLATIONS.md` is
the self-contained guide for that: file format, adding a language, translator
tooling, and the rules the build enforces (placeholders, fallback warnings).

See `docs/ARCHITECTURE.md` for the module layout and the contract between
`core/` (pure, cross-platform, tested) and `platform/` (macOS-only, holds all
the `unsafe`), and `design/DESIGN.md` for the behavior and visuals any change
to the cleaning overlay should match.

For anything that touches the input-blocking safety net specifically, please
say so in the pull request description — those changes get extra scrutiny
given the threat model in `SECURITY.md`.
