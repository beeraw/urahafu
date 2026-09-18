# Translations

Urahafu is translated into 47 languages. Each language is **a single file** under `translations/`
(`en.xlf`, `fr.xlf`, `de.xlf`, `zh-Hant.xlf`, …) in XLIFF 1.2 format, an open standard supported by
most translation tools (Weblate, Crowdin, Poedit, Xcode, …).

**Translating Urahafu requires no Rust knowledge.** Everything else — the `Text` enum, the
`Language` enum, system-language detection, integration into the macOS bundle — is generated
automatically at build time (`build.rs`) from these files. Adding, removing or editing a language
happens entirely within `translations/`, without touching Rust code, `Cargo.toml`, or
`packaging/`.

## Languages

| Tag | File | Language (autonym) | Direction | Native review |
| --- | --- | --- | --- | --- |
| `en` | `en.xlf` | English | ltr | source of truth |
| `fr` | `fr.xlf` | Français | ltr | needs native review |
| `es` | `es.xlf` | Español | ltr | needs native review |
| `it` | `it.xlf` | Italiano | ltr | needs native review |
| `pt-BR` | `pt-BR.xlf` | Português (Brasil) | ltr | needs native review |
| `pt-PT` | `pt-PT.xlf` | Português (Portugal) | ltr | needs native review |
| `ar` | `ar.xlf` | العربية | **rtl** | needs native review |
| `ja` | `ja.xlf` | 日本語 | ltr | needs native review |
| `zh-Hans` | `zh-Hans.xlf` | 中文（简体） | ltr | needs native review |
| `zh-Hant` | `zh-Hant.xlf` | 中文（繁體） | ltr | needs native review |
| `de` | `de.xlf` | Deutsch | ltr | needs native review |
| `nl` | `nl.xlf` | Nederlands | ltr | needs native review |
| `sv` | `sv.xlf` | Svenska | ltr | needs native review |
| `da` | `da.xlf` | Dansk | ltr | needs native review |
| `nb` | `nb.xlf` | Norsk bokmål | ltr | needs native review |
| `fi` | `fi.xlf` | Suomi | ltr | needs native review |
| `pl` | `pl.xlf` | Polski | ltr | needs native review |
| `cs` | `cs.xlf` | Čeština | ltr | needs native review |
| `sk` | `sk.xlf` | Slovenčina | ltr | needs native review |
| `hu` | `hu.xlf` | Magyar | ltr | needs native review |
| `ro` | `ro.xlf` | Română | ltr | needs native review |
| `hr` | `hr.xlf` | Hrvatski | ltr | needs native review |
| `sl` | `sl.xlf` | Slovenščina | ltr | needs native review |
| `el` | `el.xlf` | Ελληνικά | ltr | needs native review |
| `tr` | `tr.xlf` | Türkçe | ltr | needs native review |
| `ru` | `ru.xlf` | Русский | ltr | needs native review |
| `uk` | `uk.xlf` | Українська | ltr | needs native review |
| `ca` | `ca.xlf` | Català | ltr | needs native review |
| `es-419` | `es-419.xlf` | Español (Latinoamérica) | ltr | needs native review |
| `zh-HK` | `zh-HK.xlf` | 繁體中文（香港） | ltr | needs native review |
| `ko` | `ko.xlf` | 한국어 | ltr | needs native review |
| `th` | `th.xlf` | ไทย | ltr | needs native review |
| `vi` | `vi.xlf` | Tiếng Việt | ltr | needs native review |
| `id` | `id.xlf` | Bahasa Indonesia | ltr | needs native review |
| `ms` | `ms.xlf` | Bahasa Melayu | ltr | needs native review |
| `hi` | `hi.xlf` | हिन्दी | ltr | needs native review |
| `bn` | `bn.xlf` | বাংলা | ltr | needs native review |
| `ur` | `ur.xlf` | اردو | **rtl** | needs native review |
| `he` | `he.xlf` | עברית | **rtl** | needs native review |
| `mr` | `mr.xlf` | मराठी | ltr | needs native review |
| `gu` | `gu.xlf` | ગુજરાતી | ltr | needs native review |
| `pa` | `pa.xlf` | ਪੰਜਾਬੀ | ltr | needs native review |
| `ta` | `ta.xlf` | தமிழ் | ltr | needs native review |
| `te` | `te.xlf` | తెలుగు | ltr | needs native review |
| `kn` | `kn.xlf` | ಕನ್ನಡ | ltr | needs native review |
| `ml` | `ml.xlf` | മലയാളം | ltr | needs native review |
| `or` | `or.xlf` | ଓଡ଼ିଆ | ltr | needs native review |

`en.xlf` (English) is the source of truth for the whole set of keys; `fr.xlf` is the project's
original translation, written directly by the team rather than machine-translated — but like every
other non-English language, it is still marked as needing native-speaker review above before being
considered final. All 45 remaining languages were produced by assisted machine translation and
**still need review by a native speaker** before being considered final — in particular the System
Settings panel names and the overall register (see "Translation notes" below). Three languages read
right-to-left (**rtl**): Arabic, Urdu and Hebrew.

## File format

Every `translations/<tag>.xlf` file has the same structure. Annotated excerpt from `fr.xlf`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<xliff version="1.2" xmlns="urn:oasis:names:tc:xliff:document:1.2">
  <file source-language="en" target-language="fr" datatype="plaintext" original="urahafu">
    <header>
      <!-- Language name, in the language itself. Required. -->
      <note from="urahafu:language-name">Français</note>
      <!-- "ltr" (default) or "rtl". Omitted = ltr. -->
      <note from="urahafu:direction">ltr</note>
    </header>
    <body>
      <trans-unit id="menu.clean_screen">
        <source>Clean Screen</source>
        <target>Nettoyer l'écran</target>
        <note>Menu item shown when keyboard-only mode is off. Keep short: appears in the macOS
        menu bar dropdown.</note>
      </trans-unit>
      <!-- … one <trans-unit> per string … -->
    </body>
  </file>
</xliff>
```

What matters in each `<trans-unit>`:

- **`id`** — the string's key (`menu.clean_screen`, `alert.tap_failed.title`, …). At build time this
  becomes a variant of the Rust `Text` enum (`Text::MenuCleanScreen`, …): `en.xlf` defines the
  complete set of keys, and every other file must reuse the same `id`s.
- **`<source>`** — the English text. In `en.xlf`, `<source>` and `<target>` are identical; in every
  other file, `<source>` stays the reference English text (so the translator knows what they're
  translating) and `<target>` holds the translation.
- **`<target>`** — the translation, in the file's language.
- **`<note>`** — context for the translator: where the string appears on screen, a length
  constraint ("HUD pill: keep it short"), the meaning of a `{parameter}`. This text never appears in
  the app; it's written in English in the shipped files, but can be translated or expanded without
  consequence — only `id`, `<source>` and `<target>` are read by the build.

## `{placeholder}` parameters

Some strings contain one or more parameters in curly braces, for example `countdown.locking_in` →
"Locking in {n}". These are values computed at display time (a countdown digit, a number of
seconds, the version number): the name inside the braces isn't translated, but its **position** in
the sentence is — place `{n}` wherever the number belongs in your language, even if the order
differs from English. Language-specific formatting (a space or not before a unit, word order, etc.)
is entirely up to the translator: for example, Japanese and Chinese traditionally use no space
between a number and its unit (`{n}秒後`), unlike French (`{n} s`).

Every file must use **exactly the same parameters** as `en.xlf` for a given key (same names, any
number of occurrences, in whatever order suits the language). A parameter that's missing, added, or
misspelled in a `<target>` fails the build with a precise message (file, key, expected vs. found
parameters) — a broken string can never ship by mistake.

## Adding a language

1. Copy `translations/en.xlf` to `translations/<tag>.xlf`, where `<tag>` is the language's BCP-47
   tag (`de`, `nl`, `ko`, `zh-Hant`, `pt-PT`, …) — this file name is what becomes the language's
   tag, nothing else.
2. In the new file, change `target-language="en"` to `target-language="<tag>"` (must exactly match
   the file name).
3. Fill in `<note from="urahafu:language-name">` with the language's name, written in the language
   itself ("Deutsch", "Nederlands", "한국어", …).
4. If the language reads right-to-left, add `<note from="urahafu:direction">rtl</note>`.
5. Translate each `<target>` (they currently hold the English text, copied from `en.xlf`).
6. Build (`cargo build`): the new language automatically appears in `Language::all()`, in
   system-language detection, and in the macOS bundle's `CFBundleLocalizations`. Nothing else to
   do.

Removing a language is the reverse: delete `translations/<tag>.xlf` and rebuild.

### Incomplete translation

A file doesn't need to be finished to be picked up. For every `<trans-unit>` whose `<target>` is
empty or missing, the build prints a warning (`cargo:warning`, visible with `cargo build -vv` or in
the build logs) naming the file and the key, and Urahafu shows the English text in its place until
the translation is completed. A string whose `id` doesn't exist in `en.xlf` (a stale key, a typo)
also produces a warning and is ignored.

Only two things fail the build instead of just warning: malformed XML or a missing
`target-language`/`urahafu:language-name` (the file is then unusable), and a `<target>` whose
`{…}` parameters don't match those in `en.xlf` (a string silently losing an `{n}` would be a real
bug, not a translation in progress).

## Using Poedit, Weblate or Crowdin

These are standard XLIFF 1.2 files; all three tools open them natively:

- **Poedit**: "File › Open…", pick `translations/<tag>.xlf`. Poedit shows
  `<source>`/`<target>`/`<note>` in its usual interface and saves in the same format — no special
  setup needed.
- **Weblate** / **Crowdin**: configure `translations/en.xlf` as the source file and
  `translations/<tag>.xlf` as the per-language target file ("XLIFF 1.2" format). Both tools
  natively treat `<note>` as translator context and recognize `{parameters}` as placeholders not to
  translate.
- **Xcode**: the same files import as-is via *Editor › Import Localizations…* should Urahafu ever
  adopt an Xcode-based translation workflow; this is one of the reasons XLIFF 1.2 was chosen over a
  custom format.

## System-language detection

`Language::from_preferred` (`src/core/i18n.rs`) receives the user's ordered list of preferred
languages as macOS reports it (`CFLocaleCopyPreferredLanguages`, e.g. `["fr-FR", "en-US"]`) and
picks the first language **actually present under `translations/`** that matches, in the user's own
order of preference — never a fixed order on the app's side. For each preferred tag, in order:

1. **legacy alias**: a few old primary-language subtags (CLDR/IANA registry) are first rewritten to
   their modern equivalent, keeping the script/region — `no` (Norwegian macrolanguage) → `nb`
   (Bokmål), `in` → `id` (Indonesian), `iw` → `he` (Hebrew); for example `no-NO` becomes `nb-NO`
   before the rest of the algorithm runs;
2. **exact match** with an available tag (case-insensitive, `_`/`-` equivalent);
3. **CLDR parent chain**, a small fixed table of language+region → parent tag(s) to try before the
   generic truncation:
   - Latin American and US Spanish (`419`, `MX`, `AR`, `CO`, `CL`, `PE`, `VE`, `EC`, `GT`, `CU`,
     `BO`, `DO`, `HN`, `PY`, `SV`, `NI`, `CR`, `PA`, `UY`, `PR`, `US`) → `es-419` then `es`;
   - Portuguese as spoken outside Brazil/Portugal (mainly Lusophone Africa, plus a few European
     diaspora regions CLDR treats the same way: Angola, Mozambique, Cape Verde, Guinea-Bissau, São
     Tomé and Príncipe, East Timor, Macau, Luxembourg, Switzerland, Equatorial Guinea) → `pt-PT`;
   - Chinese as spoken in Macau → `zh-HK` then `zh-Hant` (Macau shares more written conventions
     with Hong Kong than with Taiwan);
4. **generic BCP-47 truncation**: `language-script-region` → `language-script` → `language` (for
   example `zh-Hant-TW` matches an available `zh-Hant.xlf`); for Chinese specifically, a region can
   also imply a script without explicit script truncation — `zh-TW`, `zh-HK` resolve to `zh-Hant`
   (or the more specific `zh-HK` tag if it matches), `zh-CN`, `zh-SG` resolve to `zh-Hans` — a small
   fixed BCP-47 mapping table, not per-language code;
5. **bare language**, as a last resort if nothing above found an exact match: the bare tag is
   preferred if it exists (`fr` → `fr.xlf` if present), otherwise the first available tag in
   alphabetical order among those sharing the same primary language — this is why `pt` resolves to
   `pt-BR` (not `pt-PT`) and `zh` resolves to `zh-Hans` (not `zh-Hant`), matching Apple's own
   conventions.

If no preferred tag matches → English, the fallback language. The core of this algorithm
(`match_preferred_tag`, `src/core/i18n.rs`) knows nothing about the current 47 languages: the alias
and parent-chain tables above are general linguistic data (CLDR), not code written for any specific
shipped language — it operates on the list of tags actually present under `translations/`, so
adding or removing a file is enough to change what it can recognize, without touching the
algorithm. Unit tests (`src/core/i18n.rs`, `tests` module) cover this mechanism against a synthetic
list of tags, independent of the files actually shipped.

## Per-app macOS language setting

macOS lets you choose an app's language individually, independent of the system's overall language:
**System Settings › General › Language & Region › Applications**, then add Urahafu and pick its
language from the list (which reflects `CFBundleLocalizations`, i.e. the languages actually present
under `translations/` at build time). Handy for testing a translation in progress, or for a
translator who wants to see their work in context without changing the language of their whole
session.

## Recent keys

Added with the settings window that replaced menu-bar/direct-launch mode (DESIGN.md's "Settings
window" section): `menu.settings` (the tray and app menu item opening the window),
`window.permission.text` (the window's permission banner), `window.open_at_login.needs_icon` (the
note under a disabled "Open at Login" checkbox), `window.pixel_test_hint` (a small label explaining
the dead-pixel test), and `menu.window`/`menu.close` (the app's Window menu and its Close item).

Reworded when the Esc + Return unlock combo was added: `overlay.hint.hold_to_unlock`,
`countdown.unlock_explanation` and `hud.hold_to_unlock` now mention the key combo.

The settings window replaced several alerts outright: `welcome.*`, `first_launch.title`,
`first_launch.text`, `first_launch.later`, `first_launch.ready.*` and `alert.hide_icon.*` have been
removed from every `translations/*.xlf` file (`first_launch.open_settings` survives — reused by the
window's permission banner button).

Earlier: `menu.grant_access` — the "Allow Accessibility Access…" menu item, only visible while the
permission is missing (§6). `countdown.starting` — replaces `countdown.label` ("Cleaning in"): text
that no longer depends on the digit shown below it (§7).

## Translation notes

- "urahafu" is **never** translated, in any language: it's the app's proper name ("cleanliness" in
  Shimaore), not a term to localize — see the note on `<trans-unit id="overlay.wordmark">` in
  `en.xlf`.
- Paths to System Settings ("System Settings › Privacy & Security › Accessibility" and its
  equivalents) must use Apple's official names for macOS 13 and later in each language; the
  provided translations were verified at publication time, but any future revision of these panels
  by Apple calls for a review.
- **Digits**: every language, including Arabic, uses Western digits (`0`-`9`), never Indo-Arabic
  digits (٠-٩) — this also matches what the Arabic macOS interface most commonly shows. A
  parameterized Arabic string can be written in normal logical order; macOS's text-rendering engine
  (CoreText) applies the Unicode bidirectional algorithm and places the number correctly within the
  Arabic text — nothing special needed on the translation side.
- **The ✕ (or ×) glyph must never appear in a translated string.** Strings related to unlocking by
  holding the ✕ button refer to that button in words ("the close button", "la croix", "閉じるボタン",
  …): next to a number ("✕ 2 seconds"), the glyph reads as a multiplication sign, not as a reference
  to the button. A test (`no_text_contains_the_cross_or_multiplication_glyph`, `src/core/i18n.rs`)
  checks that no generated string, in any language, contains ✕ or ×.
- **Register**: informal and consistent with the original English — formality of address, how
  imperatives are phrased, etc. is up to the translator's judgment for their language, as long as
  the tone stays consistent from one string to the next within the same file.
- **Esc and Return key names** follow the name Apple prints for these keys in that language's
  macOS keyboard-shortcuts documentation. Most languages keep "Esc" and "Return" in Latin script;
  known exceptions are French (Échap + Retour), German (Esc + Eingabetaste), Italian (Esc + Invio),
  Spanish and Portuguese (Esc + Retorno), Danish (Retur), Swedish (Retur), Hungarian and Norwegian
  (Enter), Arabic (رجوع), Thai (รีเทิร์น), and Hong Kong Chinese (返回, where Taiwan keeps
  "Return"). Key names worth a native check first: Latin American Spanish, Catalan, Romanian,
  Finnish (shortened in the HUD), Malay, and every language macOS itself isn't localized into.
- The keyboard-only mode HUD (`hud.*`) is a compact pill: favor short translations that won't
  overflow, as the `<note>`s on these keys in `en.xlf` point out.
