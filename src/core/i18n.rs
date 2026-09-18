//! Internationalization: every user-visible string, generated at build time from
//! `translations/*.xlf` (XLIFF 1.2) by `build.rs` (`docs/TRANSLATIONS.md`, `design/DESIGN.md`
//! §14).
//!
//! [`Text`] and [`Language`] are entirely generated: adding, removing or renaming a language is a
//! matter of adding, removing or editing one file under `translations/`, with no Rust, `Cargo.toml`
//! or `packaging/` change required. [`Text`] is exhaustive on purpose: it covers every string that
//! Urahafu shows, generated from `translations/en.xlf`'s trans-unit ids, so a new user-facing string
//! that hasn't been added to `en.xlf` simply doesn't exist as a [`Text`] variant — there is nothing
//! to forget to translate at the Rust level (the build script separately warns, per language, about
//! a missing or empty `<target>`).
//!
//! Strings that need a runtime value (a countdown digit, a remaining-seconds count, a version
//! number) are [`Text`] variants too, with `{name}` placeholders substituted by [`Text::format`].
//! A handful of purely numeric, language-agnostic formatters ([`format_time`],
//! [`hud_time_remaining`]) stay hand-written in Rust rather than living in the translation files,
//! since their output never varies by language.
//!
//! `console`/log output is explicitly out of scope: per `docs/ARCHITECTURE.md`, that stays in
//! English and untranslated, since it is read by a developer, not the end user.

include!(concat!(env!("OUT_DIR"), "/i18n_generated.rs"));

/// The countdown's keyboard-only-mode label ("Locking in {n}", DESIGN.md `countdown.locking_in`).
#[must_use]
pub fn countdown_locking_in(language: Language, n: u32) -> String {
    Text::CountdownLockingIn.format(language, &[("n", &n.to_string())])
}

/// The overlay's second line: dead-pixel test hint plus remaining auto-unlock time
/// (DESIGN.md `overlay.hint.secondary`).
#[must_use]
pub fn hint_secondary(language: Language, remaining_seconds: u32) -> String {
    Text::OverlayHintSecondary.format(language, &[("seconds", &remaining_seconds.to_string())])
}

/// Formats a duration in whole seconds as `m:ss` (e.g. 42 -> "0:42"), used by
/// [`hud_time_remaining`] and anywhere else a countdown clock is shown. Language-agnostic (plain
/// digits, no unit word), so unlike the rest of this module it isn't driven by a translation file.
#[must_use]
pub fn format_time(total_seconds: u32) -> String {
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes}:{seconds:02}")
}

/// The keyboard-only HUD's remaining-time text (DESIGN.md `hud.time_remaining`); the template is
/// identical in every language (just the `m:ss` clock), but takes `language` for API consistency.
#[must_use]
pub fn hud_time_remaining(_language: Language, total_seconds: u32) -> String {
    format_time(total_seconds)
}

/// The About alert's version line ("Version {version}", DESIGN.md §12's `about.version`).
#[must_use]
pub fn about_version(language: Language, version: &str) -> String {
    Text::AboutVersion.format(language, &[("version", version)])
}

/// Normalizes a BCP-47-ish tag for comparison: lowercase, `_` folded to `-`.
fn normalize_tag(tag: &str) -> String {
    tag.to_ascii_lowercase().replace('_', "-")
}

/// Region -> script inference for Chinese, the one primary language where a region subtag alone
/// implies a script (per BCP-47 "likely subtags"): Taiwan and Hong Kong ship as Traditional
/// Chinese; the rest fall through to the ordinary matching rules. Macao is deliberately absent
/// here — it has its own, more specific entry in [`PARENT_LOCALES`] (Macao's Traditional Chinese
/// parents to Hong Kong's before the generic `zh-Hant`). Locale data, not per-language code — the
/// same table applies regardless of which `zh-*.xlf` files are actually shipped.
const ZH_REGION_SCRIPT: &[(&str, &str)] = &[
    ("tw", "hant"),
    ("hk", "hant"),
    ("cn", "hans"),
    ("sg", "hans"),
];

/// Legacy/grandfathered primary-language subtag aliases (CLDR/IANA language-subtag registry): a
/// preferred tag using one of these older primary subtags is treated as if it used the modern
/// subtag instead, keeping any script/region that followed it. `no` is the macrolanguage code for
/// Norwegian, which macOS's own preferred-languages list reports for Bokmål; it resolves to `nb`
/// (Bokmål, the common written standard) rather than `nn` (Nynorsk). `in` and `iw` are the
/// pre-1989 ISO 639 codes for Indonesian and Hebrew, still reported by some older systems/browsers.
const LANGUAGE_ALIASES: &[(&str, &str)] = &[("no", "nb"), ("in", "id"), ("iw", "he")];

/// CLDR parent-locale data for language+region combinations that should fall back to a specific
/// regional standard before the language's bare/generic form. Each entry is `(primary language,
/// region subtags it applies to, ordered parent tags to try)`:
/// - Spanish spoken in Latin America (plus the US, which CLDR groups with it) falls back to the
///   `es-419` regional standard before generic `es` (which in practice means European Spanish).
/// - Portuguese varieties spoken outside Brazil and Portugal — mostly Lusophone Africa, plus a
///   handful of European-diaspora regions CLDR treats the same way — fall back to `pt-PT`.
/// - Chinese as spoken in Macao falls back to Hong Kong's Traditional Chinese before the generic
///   `zh-Hant` (Macao and Hong Kong share far more written-Chinese convention with each other than
///   either does with Taiwan).
///
/// Locale data, not per-language code: entries apply purely by primary-language + region subtag
/// match, independent of which `translations/*.xlf` files are actually shipped.
const PARENT_LOCALES: &[(&str, &[&str], &[&str])] = &[
    (
        "es",
        &[
            "419", "mx", "ar", "co", "cl", "pe", "ve", "ec", "gt", "cu", "bo", "do", "hn", "py",
            "sv", "ni", "cr", "pa", "uy", "pr", "us",
        ],
        &["es-419", "es"],
    ),
    (
        "pt",
        &["ao", "mz", "cv", "gw", "st", "tl", "mo", "lu", "ch", "gq"],
        &["pt-pt"],
    ),
    ("zh", &["mo"], &["zh-hk", "zh-hant"]),
];

/// Applies [`LANGUAGE_ALIASES`] to a normalized tag's primary subtag, leaving any script/region
/// subtag untouched (e.g. `no-no` -> `nb-no`).
fn apply_language_alias(norm: &str) -> String {
    let mut parts = norm.splitn(2, '-');
    let primary = parts.next().unwrap_or(norm);
    let rest = parts.next();
    let aliased = LANGUAGE_ALIASES
        .iter()
        .find(|(from, _)| *from == primary)
        .map_or(primary, |(_, to)| *to);
    match rest {
        Some(rest) => format!("{aliased}-{rest}"),
        None => aliased.to_string(),
    }
}

/// The region (or numeric area, e.g. `419`) subtag of a normalized, hyphen-split tag, skipping
/// over a script subtag if present (a script is always exactly four ASCII letters; a region is
/// two letters or three digits, per BCP-47) — so `zh-hant-mo` and `zh-mo` both yield `mo`.
fn region_subtag<'a>(parts: &[&'a str]) -> Option<&'a str> {
    parts
        .iter()
        .skip(1)
        .find(|p| !(p.len() == 4 && p.chars().all(|c| c.is_ascii_alphabetic())))
        .copied()
}

/// Builds the ordered chain of candidate tags to try, via exact match, for a single (already
/// alias-resolved) normalized preferred tag: the tag itself, then [`PARENT_LOCALES`]' explicit
/// parents (if the tag's primary language + region has an entry), then the generic BCP-47
/// truncation steps (`lang-script-region` -> `lang-script`, and, for Chinese specifically, a
/// region-implied script via [`ZH_REGION_SCRIPT`]), then the bare primary language as a last exact
/// candidate. Matching stops at the first candidate present in `available`; if none is, the caller
/// falls back to [`primary_language_fallback`].
fn candidate_chain(norm: &str) -> Vec<String> {
    let parts: Vec<&str> = norm.split('-').collect();
    let primary = parts[0];
    let mut chain = vec![norm.to_string()];

    if let Some(region) = region_subtag(&parts) {
        for (lang, regions, parents) in PARENT_LOCALES {
            if *lang == primary && regions.contains(&region) {
                chain.extend(parents.iter().map(|p| (*p).to_string()));
            }
        }
    }

    if parts.len() >= 3 {
        chain.push(format!("{}-{}", parts[0], parts[1]));
    }

    if primary == "zh" {
        if let Some(region) = region_subtag(&parts) {
            if let Some((_, script)) = ZH_REGION_SCRIPT.iter().find(|(r, _)| *r == region) {
                chain.push(format!("zh-{script}"));
            }
        }
    }

    if parts.len() >= 2 {
        chain.push(primary.to_string());
    }

    chain
}

/// Last-resort match for a primary language that has no exact hit anywhere in
/// [`candidate_chain`]: the bare available tag if there is one (`fr` -> `fr.xlf`), else the
/// alphabetically-first available tag sharing that primary language (so `pt` picks `pt-BR` over
/// `pt-PT`, `zh` picks `zh-Hans` over `zh-Hant`, matching Apple's own convention).
fn primary_language_fallback<'a>(primary: &str, available: &[&'a str]) -> Option<&'a str> {
    let mut candidates: Vec<&str> = available
        .iter()
        .filter(|a| {
            let norm_a = normalize_tag(a);
            norm_a == primary || norm_a.starts_with(&format!("{primary}-"))
        })
        .copied()
        .collect();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|c| normalize_tag(c));
    if let Some(&bare) = candidates.iter().find(|c| normalize_tag(c) == primary) {
        return Some(bare);
    }
    Some(candidates[0])
}

/// Locale-matching core: given the caller's preferred tags (most-preferred first) and the tags
/// actually available, returns the first available tag that matches one of the preferred tags.
/// For each preferred tag, in order: (a) resolve legacy aliases ([`LANGUAGE_ALIASES`]); (b) try an
/// exact match against each tag in [`candidate_chain`] (itself/`PARENT_LOCALES`/BCP-47 truncation,
/// in that order); (c) fall back to [`primary_language_fallback`]. Only moves on to the next
/// preferred tag once a preferred tag has exhausted all of the above without a match.
///
/// A pure function over plain tag strings — it knows nothing about [`Language`] or
/// `translations/*.xlf` — so it can be unit-tested against a synthetic tag list independently of
/// which languages Urahafu actually ships.
fn match_preferred_tag<'a>(preferred: &[&str], available: &[&'a str]) -> Option<&'a str> {
    for pref in preferred {
        let norm = apply_language_alias(&normalize_tag(pref));
        let primary = norm.split('-').next().unwrap_or(&norm).to_string();

        for candidate in candidate_chain(&norm) {
            if let Some(&tag) = available.iter().find(|a| normalize_tag(a) == candidate) {
                return Some(tag);
            }
        }

        if let Some(tag) = primary_language_fallback(&primary, available) {
            return Some(tag);
        }
    }
    None
}

impl Language {
    /// Picks a language from an ordered list of preferred language tags (as macOS's
    /// `CFLocaleCopyPreferredLanguages` would report them, e.g. `["fr-FR", "en-US"]`), using this
    /// module's private tag matcher against every shipped [`Language::all`] tag. Falls back to
    /// [`Language::ENGLISH`] if nothing matches.
    #[must_use]
    pub fn from_preferred(preferred: &[&str]) -> Self {
        let available: Vec<&str> = Self::all().iter().map(|l| l.tag()).collect();
        match_preferred_tag(preferred, &available)
            .and_then(Self::from_tag)
            .unwrap_or(Self::ENGLISH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Words allowed to appear unchanged in any language: proper nouns and other strings that are
    /// legitimately identical everywhere (the app's name, GitHub, standalone "OK" buttons, the MIT
    /// license name).
    const ALLOWED_UNCHANGED_WORDS: &[&str] = &["urahafu", "github", "ok", "mit"];

    /// `text` with every `{placeholder}` span removed — placeholder names (e.g. `version` in
    /// `"Version {version}"`) are not translatable content and must not count as words.
    fn strip_placeholders(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut in_placeholder = false;
        for ch in text.chars() {
            match ch {
                '{' => in_placeholder = true,
                '}' => in_placeholder = false,
                _ if !in_placeholder => out.push(ch),
                _ => {}
            }
        }
        out
    }

    /// Whether `source` (with any `{placeholder}` removed) contains a word that isn't in
    /// [`ALLOWED_UNCHANGED_WORDS`] — used to tell a legitimate coincidence (a proper noun, or a
    /// short word that's spelled the same in both languages) from a real untranslated leak.
    fn contains_untranslated_word(source: &str) -> bool {
        strip_placeholders(source).split_whitespace().any(|word| {
            let letters: String = word.chars().filter(|c| c.is_alphabetic()).collect();
            !letters.is_empty()
                && !ALLOWED_UNCHANGED_WORDS.contains(&letters.to_ascii_lowercase().as_str())
        })
    }

    #[test]
    fn language_all_includes_english() {
        assert!(Language::all().contains(&Language::ENGLISH));
        assert_eq!(Language::ENGLISH.tag(), "en");
    }

    #[test]
    fn language_from_tag_round_trips_every_shipped_language() {
        for &language in Language::all() {
            assert_eq!(Language::from_tag(language.tag()), Some(language));
        }
    }

    #[test]
    fn language_from_tag_is_case_insensitive() {
        assert_eq!(Language::from_tag("EN"), Some(Language::ENGLISH));
        assert_eq!(Language::from_tag("en"), Some(Language::ENGLISH));
    }

    #[test]
    fn language_from_tag_unknown_tag_is_none() {
        assert_eq!(Language::from_tag("xx"), None);
    }

    #[test]
    fn every_text_has_a_non_empty_translation_in_every_language() {
        for &text in Text::all() {
            for &language in Language::all() {
                assert!(
                    !text.get(language).is_empty(),
                    "{text:?} has an empty translation for {language:?}"
                );
            }
        }
    }

    #[test]
    fn every_key_is_unique_and_snake_case_ish() {
        let keys: std::collections::HashSet<&str> = Text::all().iter().map(|t| t.key()).collect();
        assert_eq!(
            keys.len(),
            Text::all().len(),
            "duplicate translation keys found"
        );
        for &text in Text::all() {
            let key = text.key();
            assert!(!key.is_empty());
            assert!(
                key.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_')
            );
        }
    }

    /// The hold-to-unlock strings must spell out "close button"/"croix"/etc. in words rather than
    /// using the ✕ glyph inline: next to a number ("✕ 2 secondes"), the glyph reads as a
    /// multiplication sign, not a button reference. No [`Text`] string (in any language) may
    /// contain either character.
    #[test]
    fn no_text_contains_the_cross_or_multiplication_glyph() {
        for &text in Text::all() {
            for &language in Language::all() {
                let translated = text.get(language);
                assert!(
                    !translated.contains('\u{2715}') && !translated.contains('\u{d7}'),
                    "{text:?} ({language:?}) must spell out the button in words, not use \u{2715}/\u{d7}: {translated:?}"
                );
            }
        }
    }

    #[test]
    fn no_untranslated_english_leaks_into_other_languages() {
        // Heuristic: a non-trivial (> 15 chars) string that is byte-for-byte identical to the
        // English text, for a language other than English, is almost certainly an untranslated
        // string rather than a legitimate coincidence — unless every "real" word in it is a
        // proper noun that legitimately doesn't translate (Urahafu, GitHub, OK, MIT).
        for &text in Text::all() {
            let en = text.get(Language::ENGLISH);
            if strip_placeholders(en).chars().count() <= 15 {
                continue;
            }
            for &language in Language::all() {
                if language == Language::ENGLISH {
                    continue;
                }
                let translated = text.get(language);
                if translated == en {
                    assert!(
                        !contains_untranslated_word(en),
                        "{text:?} looks untranslated (identical to English) for {language:?}: {translated:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn format_substitutes_every_placeholder_in_every_language() {
        for &text in Text::all() {
            let placeholders = text.placeholders();
            if placeholders.is_empty() {
                continue;
            }
            let args: Vec<(&str, &str)> = placeholders.iter().map(|&name| (name, "42")).collect();
            for &language in Language::all() {
                let formatted = text.format(language, &args);
                assert!(
                    !formatted.contains('{') && !formatted.contains('}'),
                    "{text:?} ({language:?}) still has an unsubstituted placeholder after \
                     format() with args for {placeholders:?}: {formatted:?}"
                );
            }
        }
    }

    #[test]
    fn is_rtl_is_true_only_for_right_to_left_languages() {
        const RTL_TAGS: &[&str] = &["ar", "ur", "he"];
        for &language in Language::all() {
            assert_eq!(
                language.is_rtl(),
                RTL_TAGS.contains(&language.tag()),
                "{language:?}"
            );
        }
    }

    #[test]
    fn countdown_locking_in_includes_the_digit() {
        for &language in Language::all() {
            assert!(countdown_locking_in(language, 3).contains('3'));
        }
    }

    #[test]
    fn hint_secondary_includes_seconds() {
        for &language in Language::all() {
            assert!(hint_secondary(language, 42).contains("42"));
        }
    }

    #[test]
    fn format_time_pads_seconds() {
        assert_eq!(format_time(42), "0:42");
        assert_eq!(format_time(5), "0:05");
        assert_eq!(format_time(90), "1:30");
        assert_eq!(format_time(0), "0:00");
    }

    #[test]
    fn hud_time_remaining_matches_format_time() {
        for &language in Language::all() {
            assert_eq!(hud_time_remaining(language, 42), "0:42");
        }
    }

    #[test]
    fn about_version_includes_version_string() {
        for &language in Language::all() {
            assert!(about_version(language, "1.0.0").contains("1.0.0"));
        }
    }

    #[test]
    fn language_from_preferred_picks_first_matching_language() {
        assert_eq!(
            Language::from_preferred(&["fr-FR", "en-US"]),
            Language::from_tag("fr").unwrap()
        );
        assert_eq!(
            Language::from_preferred(&["en-US", "fr-FR"]),
            Language::ENGLISH
        );
    }

    #[test]
    fn language_from_preferred_picks_pt_br_for_generic_pt() {
        let pt_br = Language::from_tag("pt-BR").unwrap();
        assert_eq!(Language::from_preferred(&["pt-BR"]), pt_br);
        assert_eq!(Language::from_preferred(&["pt"]), pt_br);
    }

    #[test]
    fn language_from_preferred_picks_pt_pt_for_portugal_and_lusophone_africa() {
        let pt_pt = Language::from_tag("pt-PT").unwrap();
        assert_eq!(Language::from_preferred(&["pt-PT"]), pt_pt);
        assert_eq!(Language::from_preferred(&["PT-PT"]), pt_pt);
        assert_eq!(Language::from_preferred(&["pt_PT"]), pt_pt);
        // Lusophone Africa parents to pt-PT, not pt-BR, per the CLDR parent-locale table.
        assert_eq!(Language::from_preferred(&["pt-AO"]), pt_pt);
        assert_eq!(Language::from_preferred(&["pt-MZ"]), pt_pt);
    }

    #[test]
    fn language_from_preferred_skips_unrelated_tags() {
        // "xx" is reserved by ISO 639 for "no linguistic content" and is never shipped, so it's a
        // safe stand-in for "some language Urahafu doesn't have" that stays unrelated regardless of
        // how many `translations/*.xlf` files are added over time.
        assert_eq!(
            Language::from_preferred(&["xx-XX", "fr-CH"]),
            Language::from_tag("fr").unwrap()
        );
    }

    #[test]
    fn language_from_preferred_falls_back_to_english() {
        assert_eq!(
            Language::from_preferred(&["xx-XX", "zz-ZZ"]),
            Language::ENGLISH
        );
        assert_eq!(Language::from_preferred(&[]), Language::ENGLISH);
    }

    #[test]
    fn language_from_preferred_picks_zh_hant_for_traditional_tags() {
        let zh_hant = Language::from_tag("zh-Hant").unwrap();
        assert_eq!(Language::from_preferred(&["zh-Hant"]), zh_hant);
        assert_eq!(Language::from_preferred(&["zh-Hant-TW"]), zh_hant);
        assert_eq!(Language::from_preferred(&["zh-TW"]), zh_hant);
        assert_eq!(Language::from_preferred(&["ZH-hant"]), zh_hant);
        assert_eq!(Language::from_preferred(&["zh_Hant_TW"]), zh_hant);
    }

    #[test]
    fn language_from_preferred_picks_zh_hk_for_hong_kong_and_macao() {
        // zh-HK ships as its own file (Cantonese-influenced Hong Kong conventions); Macao parents
        // to it before falling further back to generic zh-Hant.
        let zh_hk = Language::from_tag("zh-HK").unwrap();
        assert_eq!(Language::from_preferred(&["zh-HK"]), zh_hk);
        assert_eq!(Language::from_preferred(&["zh-MO"]), zh_hk);
        assert_eq!(Language::from_preferred(&["zh-Hant-MO"]), zh_hk);
    }

    #[test]
    fn language_from_preferred_picks_zh_hans_for_other_chinese_tags() {
        let zh_hans = Language::from_tag("zh-Hans").unwrap();
        assert_eq!(Language::from_preferred(&["zh"]), zh_hans);
        assert_eq!(Language::from_preferred(&["zh-Hans"]), zh_hans);
        assert_eq!(Language::from_preferred(&["zh-CN"]), zh_hans);
        assert_eq!(Language::from_preferred(&["zh-SG"]), zh_hans);
    }

    #[test]
    fn language_from_preferred_respects_ordering() {
        assert_eq!(
            Language::from_preferred(&["xx-XX", "it-IT"]),
            Language::from_tag("it").unwrap()
        );
        assert_eq!(
            Language::from_preferred(&["zh-TW", "ar-SA"]),
            Language::from_tag("zh-Hant").unwrap()
        );
    }

    // -- `match_preferred_tag` itself, on a synthetic tag list, independent of which
    // -- translations/*.xlf files Urahafu actually ships.

    const SYNTHETIC: &[&str] = &["en", "fr", "es", "pt-BR", "pt-PT", "zh-Hans", "zh-Hant"];

    #[test]
    fn match_preferred_tag_exact_match() {
        assert_eq!(match_preferred_tag(&["fr-FR"], SYNTHETIC), Some("fr"));
        assert_eq!(match_preferred_tag(&["FR"], SYNTHETIC), Some("fr"));
    }

    #[test]
    fn match_preferred_tag_language_and_script() {
        assert_eq!(
            match_preferred_tag(&["zh-Hant-HK"], SYNTHETIC),
            Some("zh-Hant")
        );
    }

    #[test]
    fn match_preferred_tag_region_implies_chinese_script() {
        assert_eq!(match_preferred_tag(&["zh-TW"], SYNTHETIC), Some("zh-Hant"));
        assert_eq!(match_preferred_tag(&["zh-CN"], SYNTHETIC), Some("zh-Hans"));
    }

    #[test]
    fn match_preferred_tag_primary_language_prefers_bare_tag() {
        assert_eq!(match_preferred_tag(&["fr-CA"], SYNTHETIC), Some("fr"));
    }

    #[test]
    fn match_preferred_tag_primary_language_prefers_alphabetical_without_bare_tag() {
        assert_eq!(match_preferred_tag(&["pt"], SYNTHETIC), Some("pt-BR"));
        assert_eq!(match_preferred_tag(&["zh"], SYNTHETIC), Some("zh-Hans"));
    }

    #[test]
    fn match_preferred_tag_falls_through_to_next_preferred() {
        assert_eq!(
            match_preferred_tag(&["de-DE", "es-419"], SYNTHETIC),
            Some("es")
        );
    }

    #[test]
    fn match_preferred_tag_no_match_is_none() {
        assert_eq!(match_preferred_tag(&["de-DE", "ko-KR"], SYNTHETIC), None);
        assert_eq!(match_preferred_tag(&[], SYNTHETIC), None);
    }

    // -- Extended synthetic tag list covering the CLDR parent-locale/alias data added on top of
    // -- the plain truncation/region-script rules above.
    const SYNTHETIC_EXT: &[&str] = &[
        "en", "fr", "es", "es-419", "pt-BR", "pt-PT", "zh-Hans", "zh-Hant", "zh-HK", "nb", "ur",
        "de",
    ];

    #[test]
    fn match_preferred_tag_spanish_latam_prefers_es_419_when_present() {
        assert_eq!(
            match_preferred_tag(&["es-MX"], SYNTHETIC_EXT),
            Some("es-419")
        );
    }

    #[test]
    fn match_preferred_tag_spanish_latam_falls_back_to_es_without_es_419() {
        assert_eq!(match_preferred_tag(&["es-MX"], SYNTHETIC), Some("es"));
    }

    #[test]
    fn match_preferred_tag_spain_spanish_is_plain_es() {
        assert_eq!(match_preferred_tag(&["es-ES"], SYNTHETIC_EXT), Some("es"));
    }

    #[test]
    fn match_preferred_tag_zh_mo_prefers_zh_hk_when_present() {
        assert_eq!(
            match_preferred_tag(&["zh-MO"], SYNTHETIC_EXT),
            Some("zh-HK")
        );
        assert_eq!(
            match_preferred_tag(&["zh-Hant-MO"], SYNTHETIC_EXT),
            Some("zh-HK")
        );
    }

    #[test]
    fn match_preferred_tag_zh_mo_falls_back_to_zh_hant_without_zh_hk() {
        assert_eq!(match_preferred_tag(&["zh-MO"], SYNTHETIC), Some("zh-Hant"));
    }

    #[test]
    fn match_preferred_tag_zh_tw_is_zh_hant() {
        assert_eq!(
            match_preferred_tag(&["zh-TW"], SYNTHETIC_EXT),
            Some("zh-Hant")
        );
    }

    #[test]
    fn match_preferred_tag_norwegian_alias_prefers_bokmal() {
        assert_eq!(match_preferred_tag(&["no-NO"], SYNTHETIC_EXT), Some("nb"));
    }

    #[test]
    fn match_preferred_tag_portuguese_africa_falls_back_to_pt_pt() {
        assert_eq!(
            match_preferred_tag(&["pt-AO"], SYNTHETIC_EXT),
            Some("pt-PT")
        );
    }

    #[test]
    fn match_preferred_tag_bare_pt_prefers_pt_br() {
        assert_eq!(match_preferred_tag(&["pt"], SYNTHETIC_EXT), Some("pt-BR"));
    }

    #[test]
    fn match_preferred_tag_de_ch_falls_back_to_bare_de() {
        assert_eq!(match_preferred_tag(&["de-CH"], SYNTHETIC_EXT), Some("de"));
    }

    #[test]
    fn match_preferred_tag_ur_pk_falls_back_to_bare_ur() {
        assert_eq!(match_preferred_tag(&["ur-PK"], SYNTHETIC_EXT), Some("ur"));
    }
}
