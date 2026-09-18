//! Build script: turns `translations/*.xlf` (XLIFF 1.2) into the generated `Text`/`Language` API
//! included by `src/core/i18n.rs`, and into the `CFBundleLocalizations` list merged into the
//! bundled `Info.plist` (`docs/TRANSLATIONS.md` explains the file format and the "just drop a
//! file" contract this script implements).
//!
//! Adding or removing a shipped language never touches this file, `Cargo.toml` or the plist
//! template: it is entirely a function of which `translations/*.xlf` files exist. This script is
//! the only thing that reads them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "this build script's only job is to validate translations/*.xlf and the target \
              directory layout, then fail the build loudly and immediately if either is wrong; \
              unwrap/expect/panic (surfaced to cargo as a build error) are the correct tool for \
              that, not something to avoid the way application code would"
)]

use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use roxmltree::Document;

/// One `<trans-unit>` parsed out of an XLIFF file, before cross-file validation.
struct Unit {
    id: String,
    source: String,
    target: String,
}

/// One parsed `translations/<tag>.xlf` file.
struct XliffFile {
    /// BCP-47 tag, taken from the file name (the source of truth for "which languages exist").
    tag: String,
    language_name: String,
    direction: String,
    units: Vec<Unit>,
}

/// Every `{name}` placeholder appearing in `text`, in first-seen order, deduplicated.
fn placeholders(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (start, ch) in text.char_indices() {
        if ch != '{' {
            continue;
        }
        if let Some(end) = text[start + 1..].find('}') {
            let name = &text[start + 1..start + 1 + end];
            if !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !found.iter().any(|f| f == name)
            {
                found.push(name.to_owned());
            }
        }
    }
    found
}

/// Reads and parses one `translations/<file_name>` XLIFF file. `cargo:warning` and hard `panic!`s
/// below are how this script reports problems to `cargo build`'s output.
fn parse_xliff(dir: &Path, file_name: &str) -> XliffFile {
    let tag = file_name
        .strip_suffix(".xlf")
        .unwrap_or_else(|| panic!("translations/{file_name}: expected a .xlf extension"))
        .to_owned();
    let path = dir.join(file_name);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("translations/{file_name}: cannot read file: {err}"));
    let doc = Document::parse(&text)
        .unwrap_or_else(|err| panic!("translations/{file_name}: malformed XML: {err}"));

    let file_node = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "file")
        .unwrap_or_else(|| panic!("translations/{file_name}: missing <file> element"));

    let target_language = file_node.attribute("target-language").unwrap_or_else(|| {
        panic!("translations/{file_name}: <file> is missing a target-language attribute")
    });
    assert!(
        target_language.eq_ignore_ascii_case(&tag),
        "translations/{file_name}: target-language=\"{target_language}\" does not match the \
         file name (expected \"{tag}\"); rename the file or fix the attribute"
    );

    let mut language_name = None;
    let mut direction = "ltr".to_owned();
    for note in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "note")
    {
        match note.attribute("from") {
            Some("urahafu:language-name") => {
                language_name = note.text().map(str::trim).filter(|s| !s.is_empty());
            }
            Some("urahafu:direction") => {
                let value = note.text().map(str::trim).unwrap_or_default();
                assert!(
                    value == "ltr" || value == "rtl",
                    "translations/{file_name}: urahafu:direction must be \"ltr\" or \"rtl\", \
                     found {value:?}"
                );
                value.clone_into(&mut direction);
            }
            _ => {}
        }
    }
    let language_name = language_name.unwrap_or_else(|| {
        panic!(
            "translations/{file_name}: missing a <note from=\"urahafu:language-name\"> header \
             declaring the language's native name"
        )
    });

    let mut units = Vec::new();
    for unit in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "trans-unit")
    {
        let id = unit
            .attribute("id")
            .unwrap_or_else(|| panic!("translations/{file_name}: <trans-unit> is missing an id"))
            .to_owned();
        let source = unit
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "source")
            .and_then(|n| n.text())
            .unwrap_or_default()
            .to_owned();
        let target = unit
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "target")
            .and_then(|n| n.text())
            .unwrap_or_default()
            .to_owned();
        units.push(Unit { id, source, target });
    }

    XliffFile {
        tag,
        language_name: language_name.to_owned(),
        direction,
        units,
    }
}

/// `snake_case.dotted.keys` -> `UpperCamelCase` `Text` variant name, matching the convention the
/// hand-written enum used before this file existed (e.g. `menu.auto_unlock.after_30s` ->
/// `MenuAutoUnlockAfter30s`).
fn variant_name(key: &str) -> String {
    let mut out = String::new();
    for word in key.split(['.', '_']) {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.extend(chars);
        }
    }
    out
}

/// Escapes a string as a Rust string literal body (for emitting into generated source).
fn rust_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

/// Walks up from `OUT_DIR` to find the Cargo target directory (the ancestor literally named
/// `target`), so the generated plist fragment can be written to a path stable across builds
/// (`target/generated/Info.ext.plist`) rather than `OUT_DIR`'s per-build hashed path — `Cargo.toml`
/// needs a static path for `osx_info_plist_exts`.
fn find_target_dir(out_dir: &Path) -> PathBuf {
    if let Ok(dir) = env::var("CARGO_TARGET_DIR") {
        return PathBuf::from(dir);
    }
    let mut current = out_dir;
    loop {
        if current.file_name().is_some_and(|name| name == "target") {
            return current.to_path_buf();
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => panic!(
                "could not find a \"target\" ancestor directory of OUT_DIR ({}); set \
                 CARGO_TARGET_DIR if the build uses a non-standard layout",
                out_dir.display()
            ),
        }
    }
}

/// A validated `translations/en.xlf`: the ordered key list, each key's English source text, and
/// each key's canonical placeholder set (what every other language's `<target>` must match).
struct English {
    ids: Vec<String>,
    source: BTreeMap<String, String>,
    placeholders: BTreeMap<String, Vec<String>>,
}

/// Lists `translations/*.xlf` (sorted by file name) and registers `cargo:rerun-if-changed` for the
/// directory, every file in it, and the plist template.
fn discover_translation_files(manifest_dir: &Path) -> (PathBuf, Vec<String>) {
    let translations_dir = manifest_dir.join("translations");
    println!("cargo:rerun-if-changed=translations");
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("packaging/Info.ext.plist.in").display()
    );

    let mut file_names: Vec<String> = fs::read_dir(&translations_dir)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", translations_dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("xlf"))
        })
        .collect();
    file_names.sort();

    for name in &file_names {
        println!(
            "cargo:rerun-if-changed={}",
            translations_dir.join(name).display()
        );
    }
    assert!(
        file_names.iter().any(|n| n == "en.xlf"),
        "translations/en.xlf is missing: it defines the set of Text keys and must always exist"
    );

    (translations_dir, file_names)
}

/// Fails the build if two files declare the same BCP-47 tag (case-insensitively).
fn check_no_duplicate_tags(files: &[XliffFile]) {
    let mut seen_tags: HashSet<String> = HashSet::new();
    for file in files {
        let lower = file.tag.to_ascii_lowercase();
        assert!(
            seen_tags.insert(lower),
            "translations/{}.xlf: duplicate language tag (case-insensitive)",
            file.tag
        );
    }
}

/// Validates `translations/en.xlf` (unique, non-empty, `target == source` for every unit) and
/// indexes it: the result defines the set of `Text` keys and their canonical placeholder sets.
fn validate_english(files: &[XliffFile]) -> English {
    let english = files
        .iter()
        .find(|f| f.tag == "en")
        .expect("checked by discover_translation_files");

    let mut ids = Vec::new();
    let mut source = BTreeMap::new();
    let mut placeholders_by_id = BTreeMap::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    for unit in &english.units {
        assert!(
            seen_ids.insert(unit.id.clone()),
            "translations/en.xlf: duplicate trans-unit id \"{}\"",
            unit.id
        );
        assert!(
            !unit.target.trim().is_empty(),
            "translations/en.xlf: trans-unit \"{}\" has an empty <target> (en.xlf's target must \
             equal its source; en.xlf must be complete)",
            unit.id
        );
        assert_eq!(
            unit.target, unit.source,
            "translations/en.xlf: trans-unit \"{}\" has a <target> that differs from its \
             <source>; for en.xlf they must be identical",
            unit.id
        );
        placeholders_by_id.insert(unit.id.clone(), placeholders(&unit.source));
        source.insert(unit.id.clone(), unit.source.clone());
        ids.push(unit.id.clone());
    }

    English {
        ids,
        source,
        placeholders: placeholders_by_id,
    }
}

/// One language's string table, aligned to `english.ids`'s order: `<target>` where present and
/// non-empty with a matching placeholder set (a mismatch is a hard error), English otherwise (with
/// a `cargo:warning` unless this *is* `en.xlf`). Also warns about any id in `file` that isn't a
/// known English key.
fn build_language_strings(file: &XliffFile, english: &English) -> Vec<String> {
    let by_id: BTreeMap<&str, &Unit> = file.units.iter().map(|u| (u.id.as_str(), u)).collect();

    for unit in &file.units {
        if !english.placeholders.contains_key(&unit.id) {
            println!(
                "cargo:warning=translations/{}.xlf: unknown trans-unit id \"{}\" (not in \
                 en.xlf); ignored",
                file.tag, unit.id
            );
        }
    }

    english
        .ids
        .iter()
        .map(|id| {
            let expected = &english.placeholders[id];
            match by_id.get(id.as_str()) {
                Some(unit) if !unit.target.trim().is_empty() => {
                    let found = placeholders(&unit.target);
                    assert!(
                        found.iter().collect::<HashSet<_>>()
                            == expected.iter().collect::<HashSet<_>>(),
                        "translations/{}.xlf: trans-unit \"{}\" has placeholders {found:?} but \
                         its English source has {expected:?} (file: translations/{}.xlf, id: \
                         \"{}\")",
                        file.tag,
                        id,
                        file.tag,
                        id
                    );
                    unit.target.clone()
                }
                Some(_) if file.tag == "en" => {
                    // en.xlf already validated (non-empty, target == source) above.
                    english.source[id].clone()
                }
                _ => {
                    if file.tag != "en" {
                        println!(
                            "cargo:warning=translations/{}.xlf: missing or empty target for \
                             \"{}\"; falling back to English",
                            file.tag, id
                        );
                    }
                    english.source[id].clone()
                }
            }
        })
        .collect()
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let (translations_dir, file_names) = discover_translation_files(&manifest_dir);

    let mut files: Vec<XliffFile> = file_names
        .iter()
        .map(|name| parse_xliff(&translations_dir, name))
        .collect();
    check_no_duplicate_tags(&files);

    let english = validate_english(&files);

    // (lang_tag, native_name, direction, [text per english.ids order]), sorted by tag.
    files.sort_by(|a, b| a.tag.cmp(&b.tag));
    let per_language: Vec<(String, String, String, Vec<String>)> = files
        .iter()
        .map(|file| {
            let strings = build_language_strings(file, &english);
            (
                file.tag.clone(),
                file.language_name.clone(),
                file.direction.clone(),
                strings,
            )
        })
        .collect();

    let english_position = per_language
        .iter()
        .position(|(tag, ..)| tag == "en")
        .expect("en.xlf is always present");

    generate_rust(
        &english.ids,
        &english.source,
        &per_language,
        english_position,
    );
    generate_plist(&manifest_dir, &per_language);
}

/// Appends the `Text` enum and its `impl` block to `out`.
fn generate_text_impl(
    out: &mut String,
    english_ids: &[String],
    english_source: &BTreeMap<String, String>,
) {
    // `Text` enum.
    out.push_str("/// Every user-visible string, generated from `translations/en.xlf`.\n");
    out.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n");
    out.push_str(
        "#[allow(\n    clippy::doc_markdown,\n    reason = \"variant docs are the raw English \
         source text from translations/en.xlf, not hand-written prose; words like GitHub or \
         Urahafu in there are not meant to be backtick-quoted\"\n)]\n",
    );
    out.push_str("pub enum Text {\n");
    for id in english_ids {
        writeln!(out, "    /// {}", english_source[id]).unwrap();
        writeln!(out, "    {},", variant_name(id)).unwrap();
    }
    out.push_str("}\n\n");

    // `Text::key`.
    out.push_str("impl Text {\n");
    out.push_str("    /// The translation key, as used in `translations/*.xlf`.\n");
    out.push_str("    #[must_use]\n");
    out.push_str("    pub fn key(self) -> &'static str {\n");
    out.push_str("        match self {\n");
    for id in english_ids {
        writeln!(out, "            Self::{} => {:?},", variant_name(id), id).unwrap();
    }
    out.push_str("        }\n    }\n\n");

    // `Text::placeholders`.
    out.push_str(
        "    /// The `{name}` placeholders this string's English source uses, in source order.\n",
    );
    out.push_str("    #[must_use]\n");
    out.push_str(
        "    #[allow(\n        clippy::match_same_arms,\n        reason = \"several distinct \
         keys legitimately take no placeholder at all; merging those arms would obscure which \
         key is which\"\n    )]\n",
    );
    out.push_str("    pub fn placeholders(self) -> &'static [&'static str] {\n");
    out.push_str("        match self {\n");
    for id in english_ids {
        let names = placeholders(&english_source[id]);
        let list = names
            .iter()
            .map(|n| format!("{n:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(out, "            Self::{} => &[{list}],", variant_name(id)).unwrap();
    }
    out.push_str("        }\n    }\n\n");

    // `Text::all`.
    out.push_str("    /// Every [`Text`] variant, in `translations/en.xlf` order.\n");
    out.push_str("    #[must_use]\n");
    out.push_str("    pub fn all() -> &'static [Text] {\n");
    out.push_str("        &ALL_TEXTS\n");
    out.push_str("    }\n\n");

    // `Text::get`.
    out.push_str("    /// The translated string for `language` (English if `language` had no\n");
    out.push_str("    /// translation for this key and a build warning was emitted).\n");
    out.push_str("    #[must_use]\n");
    out.push_str("    pub fn get(self, language: Language) -> &'static str {\n");
    out.push_str("        STRINGS[language.0][self as usize]\n");
    out.push_str("    }\n\n");

    // `Text::format`.
    out.push_str(
        "    /// [`Text::get`], with every `{name}` placeholder replaced by the matching\n",
    );
    out.push_str(
        "    /// entry of `args` (unmatched placeholders are left as literal `{name}` text;\n",
    );
    out.push_str(
        "    /// [`Text::placeholders`] plus a test are how call sites catch a missing one).\n",
    );
    out.push_str("    #[must_use]\n");
    out.push_str(
        "    pub fn format(self, language: Language, args: &[(&str, &str)]) -> String {\n",
    );
    out.push_str("        let mut result = self.get(language).to_owned();\n");
    out.push_str("        for (name, value) in args {\n");
    out.push_str("            result = result.replace(&format!(\"{{{name}}}\"), value);\n");
    out.push_str("        }\n");
    out.push_str("        result\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");
}

/// Appends the `Language` newtype and its `impl` block to `out`.
fn generate_language_impl(out: &mut String, english_position: usize) {
    writeln!(
        out,
        "/// One of the languages Urahafu ships translations for; a `translations/<tag>.xlf` file \
         each. Order matches the sorted (by tag) discovery order in `build.rs`."
    )
    .unwrap();
    out.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n");
    out.push_str("pub struct Language(usize);\n\n");

    out.push_str("impl Language {\n");
    writeln!(
        out,
        "    /// English, the fallback language and the source of truth for [`Text`]'s keys."
    )
    .unwrap();
    writeln!(
        out,
        "    pub const ENGLISH: Language = Language({english_position});"
    )
    .unwrap();
    out.push('\n');
    out.push_str("    /// Every shipped language, in ascending tag order.\n");
    out.push_str("    #[must_use]\n");
    out.push_str("    pub fn all() -> &'static [Language] {\n");
    writeln!(out, "        &LANGUAGES").unwrap();
    out.push_str("    }\n\n");

    out.push_str(
        "    /// This language's BCP-47 tag, exactly as its file name in `translations/`.\n",
    );
    out.push_str("    #[must_use]\n");
    out.push_str("    pub fn tag(self) -> &'static str {\n");
    out.push_str("        TAGS[self.0]\n");
    out.push_str("    }\n\n");

    out.push_str("    /// This language's name, in its own language (e.g. \"Fran\\u{e7}ais\").\n");
    out.push_str("    #[must_use]\n");
    out.push_str("    pub fn native_name(self) -> &'static str {\n");
    out.push_str("        NATIVE_NAMES[self.0]\n");
    out.push_str("    }\n\n");

    out.push_str("    /// Whether this language reads right-to-left.\n");
    out.push_str("    #[must_use]\n");
    out.push_str("    pub fn is_rtl(self) -> bool {\n");
    out.push_str("        IS_RTL[self.0]\n");
    out.push_str("    }\n\n");

    out.push_str("    /// Looks up a language by its exact BCP-47 tag (case-insensitive).\n");
    out.push_str("    #[must_use]\n");
    out.push_str("    pub fn from_tag(tag: &str) -> Option<Language> {\n");
    out.push_str("        TAGS.iter().position(|t| t.eq_ignore_ascii_case(tag)).map(Language)\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");
}

/// Appends the private data tables (`ALL_TEXTS`, `TAGS`, `NATIVE_NAMES`, `IS_RTL`, `LANGUAGES`,
/// `STRINGS`) that back the generated `impl` blocks.
fn generate_data_tables(
    out: &mut String,
    english_ids: &[String],
    per_language: &[(String, String, String, Vec<String>)],
) {
    let n_langs = per_language.len();
    let n_keys = english_ids.len();

    writeln!(out, "const ALL_TEXTS: [Text; {n_keys}] = [").unwrap();
    for id in english_ids {
        writeln!(out, "    Text::{},", variant_name(id)).unwrap();
    }
    out.push_str("];\n\n");

    writeln!(out, "const TAGS: [&str; {n_langs}] = [").unwrap();
    for (tag, ..) in per_language {
        writeln!(out, "    {tag:?},").unwrap();
    }
    out.push_str("];\n\n");

    writeln!(out, "const NATIVE_NAMES: [&str; {n_langs}] = [").unwrap();
    for (_, name, ..) in per_language {
        writeln!(out, "    {name:?},").unwrap();
    }
    out.push_str("];\n\n");

    writeln!(out, "const IS_RTL: [bool; {n_langs}] = [").unwrap();
    for (_, _, direction, _) in per_language {
        writeln!(out, "    {},", direction == "rtl").unwrap();
    }
    out.push_str("];\n\n");

    writeln!(out, "const LANGUAGES: [Language; {n_langs}] = [").unwrap();
    for i in 0..n_langs {
        writeln!(out, "    Language({i}),").unwrap();
    }
    out.push_str("];\n\n");

    // `static`, not `const`: with dozens of languages this table is large enough that
    // clippy::large_const_arrays flags a `const` (which would duplicate the whole table at every
    // use site) — a `static` has one fixed storage location instead.
    writeln!(out, "static STRINGS: [[&str; {n_keys}]; {n_langs}] = [").unwrap();
    for (tag, _, _, strings) in per_language {
        writeln!(out, "    // {tag}").unwrap();
        out.push_str("    [\n");
        for s in strings {
            writeln!(out, "        \"{}\",", rust_escape(s)).unwrap();
        }
        out.push_str("    ],\n");
    }
    out.push_str("];\n");
}

/// Renders `$OUT_DIR/i18n_generated.rs`: the `Text`/`Language` API, included by `src/core/i18n.rs`.
fn generate_rust(
    english_ids: &[String],
    english_source: &BTreeMap<String, String>,
    per_language: &[(String, String, String, Vec<String>)],
    english_position: usize,
) {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("set by cargo"));
    let mut out = String::new();
    out.push_str("// @generated by build.rs from translations/*.xlf. Do not edit by hand.\n\n");

    generate_text_impl(&mut out, english_ids, english_source);
    generate_language_impl(&mut out, english_position);
    generate_data_tables(&mut out, english_ids, per_language);

    fs::write(out_dir.join("i18n_generated.rs"), out).expect("failed to write generated i18n.rs");
}

fn generate_plist(manifest_dir: &Path, per_language: &[(String, String, String, Vec<String>)]) {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("set by cargo"));
    let target_dir = find_target_dir(&out_dir);
    let generated_dir = target_dir.join("generated");
    fs::create_dir_all(&generated_dir).expect("failed to create target/generated");

    let template_path = manifest_dir.join("packaging/Info.ext.plist.in");
    let template = fs::read_to_string(&template_path)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", template_path.display()));

    let mut localizations = String::new();
    localizations.push_str("  <array>\n");
    for (tag, ..) in per_language {
        writeln!(localizations, "    <string>{tag}</string>").unwrap();
    }
    localizations.push_str("  </array>");

    assert!(
        template.contains("@@CFBUNDLE_LOCALIZATIONS@@"),
        "packaging/Info.ext.plist.in is missing the @@CFBUNDLE_LOCALIZATIONS@@ placeholder"
    );
    let rendered = template.replace("@@CFBUNDLE_LOCALIZATIONS@@", localizations.trim_end());

    fs::write(generated_dir.join("Info.ext.plist"), rendered)
        .expect("failed to write target/generated/Info.ext.plist");
}
