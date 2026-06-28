// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Multi-language string lookup.
//!
//! Locales are TOML files under `common/locales/`, embedded at compile time
//! via `include_str!`. Each locale is a flat key -> string map after parsing
//!
//! Detection on Windows uses `GetUserDefaultLocaleName` and takes the first
//! 2 ISO-639 chars (`"en-US"` -> `"en"`). A `--lang <code>` CLI flag and the
//! `INSTALLWAY_LANG` env var both override detection. Unknown languages
//! fall back to English; missing keys fall back to English, then to the key
//! literal so nothing ever returns an empty string.

use std::collections::HashMap;
use std::sync::OnceLock;
use windows::Win32::Globalization::GetUserDefaultLocaleName;

const EN: &str = include_str!("../locales/en.toml");
const FR: &str = include_str!("../locales/fr.toml");

const SUPPORTED: &[(&str, &str)] = &[("en", EN), ("fr", FR)];
const DEFAULT_LANG: &str = "en";

static TABLES: OnceLock<HashMap<&'static str, HashMap<String, String>>> = OnceLock::new();

/// Process-wide resolved UI language, set once at startup via
/// [`Translator::set_global`]. Lets code that has no `Translator` in hand (e.g.
/// plugin-context construction) read the same language the host UI uses.
static CURRENT_LANG: OnceLock<&'static str> = OnceLock::new();

/// The process-wide UI language code (2 ISO-639 chars, e.g. `"fr"`). Falls back
/// to the default language until a translator calls [`Translator::set_global`].
/// Passed to plugins as `ctx.lang` so they can localize to match the host.
pub fn current_lang() -> &'static str {
    CURRENT_LANG.get().copied().unwrap_or(DEFAULT_LANG)
}

fn tables() -> &'static HashMap<&'static str, HashMap<String, String>> {
    TABLES.get_or_init(|| {
        let mut all = HashMap::new();
        for (code, src) in SUPPORTED {
            let mut flat = HashMap::new();
            if let Ok(v) = src.parse::<toml::Value>() {
                flatten(&v, "", &mut flat);
            }
            all.insert(*code, flat);
        }
        all
    })
}

fn flatten(v: &toml::Value, prefix: &str, out: &mut HashMap<String, String>) {
    if let Some(tbl) = v.as_table() {
        for (k, val) in tbl {
            let full = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{}.{}", prefix, k)
            };
            if val.is_table() {
                flatten(val, &full, out);
            } else if let Some(s) = val.as_str() {
                out.insert(full, s.to_string());
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct Translator {
    lang: &'static str,
}

impl Translator {
    /// Build a translator for an explicit language code (2 chars).
    /// Unknown codes return the default-language translator.
    pub fn for_lang(code: &str) -> Self {
        let two = code
            .split(['-', '_'])
            .next()
            .unwrap_or(code)
            .to_ascii_lowercase();
        for (c, _) in SUPPORTED {
            if *c == two {
                return Self { lang: c };
            }
        }
        Self { lang: DEFAULT_LANG }
    }

    /// Detect from CLI args (`--lang <code>`), env (`INSTALLWAY_LANG`),
    /// then OS user locale, then default.
    pub fn detect(args: &[String]) -> Self {
        if let Some(idx) = args.iter().position(|a| a == "--lang")
            && let Some(c) = args.get(idx + 1)
        {
            return Self::for_lang(c);
        }
        if let Ok(c) = std::env::var("INSTALLWAY_LANG")
            && !c.is_empty()
        {
            return Self::for_lang(&c);
        }
        if let Some(c) = os_user_locale() {
            return Self::for_lang(&c);
        }
        Self { lang: DEFAULT_LANG }
    }

    pub fn lang(&self) -> &'static str {
        self.lang
    }

    /// Record this translator's language as the process-wide UI language so
    /// [`current_lang`] (and thus the plugin context) reflects it. First call
    /// wins; later calls are no-ops. Call once at each process entry point,
    /// right after the translator is resolved.
    pub fn set_global(self) {
        let _ = CURRENT_LANG.set(self.lang);
    }

    /// Look up a key. Missing -> fall back to English -> key literal.
    pub fn get(&self, key: &str) -> String {
        let t = tables();
        if let Some(s) = t.get(self.lang).and_then(|m| m.get(key)) {
            return s.clone();
        }
        if self.lang != DEFAULT_LANG
            && let Some(s) = t.get(DEFAULT_LANG).and_then(|m| m.get(key))
        {
            return s.clone();
        }
        key.to_string()
    }

    /// Look up with `{placeholder}` substitution.
    pub fn fmt(&self, key: &str, vars: &[(&str, &str)]) -> String {
        let mut s: String = self.get(key);
        for (k, v) in vars {
            s = s.replace(&format!("{{{}}}", k), v);
        }
        s
    }
}

impl Default for Translator {
    fn default() -> Self {
        Self { lang: DEFAULT_LANG }
    }
}

fn os_user_locale() -> Option<String> {
    let mut buf = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
    let n = unsafe { GetUserDefaultLocaleName(&mut buf) };
    if n <= 0 {
        return None;
    }
    let end = (n as usize).saturating_sub(1).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..end]);
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_lang_known_unknown_and_region() {
        assert_eq!(Translator::for_lang("fr").lang(), "fr");
        assert_eq!(Translator::for_lang("fr-FR").lang(), "fr");
        assert_eq!(Translator::for_lang("EN").lang(), "en");
        assert_eq!(Translator::for_lang("de").lang(), "en"); // unknown -> default
    }

    #[test]
    fn get_falls_back_en_then_key() {
        let fr = Translator::for_lang("fr");
        assert!(!fr.get("install.install").is_empty()); // present in fr
        assert_eq!(fr.get("totally.absent.key"), "totally.absent.key");
    }

    #[test]
    fn fmt_substitutes_placeholders() {
        let en = Translator::for_lang("en");
        let s = en.fmt(
            "install.window_title",
            &[("product", "Foo"), ("version", "1.0")],
        );
        assert!(s.contains("Foo") && s.contains("1.0"));
    }

    #[test]
    fn detect_arg_overrides_everything() {
        let args = vec!["--lang".to_string(), "fr".to_string()];
        assert_eq!(Translator::detect(&args).lang(), "fr");
    }
}
