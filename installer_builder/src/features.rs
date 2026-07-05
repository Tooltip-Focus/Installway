// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Feature-pack tagging: map manifest files to a feature id by glob. The mapping
//! rides in the signed manifest, so the valid feature subsets are fixed at build
//! time — a plugin picks among them, it can't invent files.

use crate::args::ResolvedFeature;
use anyhow::{Result, bail};
use common::model::manifest::Manifest;

/// `*`/`?` must not cross `/` (only `**` does) — matches this module's
/// documented semantics, unlike `glob`'s default of treating them the same.
const MATCH_OPTIONS: glob::MatchOptions = glob::MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: false,
};

/// Compiled form of a feature's path pattern, parsed once rather than once per
/// manifest file. Glob match over a `/`-separated relative path: `*` (any run
/// within a path segment, not crossing `/`), `**` (any run including `/`), `?`
/// (one non-`/` char). A bare prefix with no wildcard matches the whole subtree
/// (so `Folder1` covers `Folder1/**`) — the common "this folder is feature X".
enum CompiledPattern {
    Prefix(String),
    Glob(glob::Pattern),
}

impl CompiledPattern {
    fn new(pattern: &str) -> Self {
        if pattern.contains(['*', '?']) {
            // Falls back to no-match on a malformed pattern; `apply` already
            // reports "matched no files" for a feature whose patterns never hit.
            match glob::Pattern::new(pattern) {
                Ok(g) => CompiledPattern::Glob(g),
                Err(_) => CompiledPattern::Prefix(String::new()),
            }
        } else {
            CompiledPattern::Prefix(pattern.trim_end_matches('/').to_string())
        }
    }

    fn matches(&self, path: &str) -> bool {
        match self {
            CompiledPattern::Prefix(p) => path == p || path.starts_with(&format!("{p}/")),
            CompiledPattern::Glob(g) => g.matches_with(path, MATCH_OPTIONS),
        }
    }
}

/// Tag each manifest file with the single feature whose patterns it matches and
/// record the declared feature ids on the manifest. Errors if a file matches
/// more than one feature (ambiguous) or a feature matches no file (likely typo).
pub fn apply(manifest: &mut Manifest, features: &[ResolvedFeature]) -> Result<()> {
    if features.is_empty() {
        return Ok(());
    }

    let mut matched_per_feature: std::collections::HashMap<&str, usize> =
        features.iter().map(|f| (f.id.as_str(), 0usize)).collect();

    let compiled: Vec<(&str, Vec<CompiledPattern>)> = features
        .iter()
        .map(|f| {
            (
                f.id.as_str(),
                f.paths.iter().map(|p| CompiledPattern::new(p)).collect(),
            )
        })
        .collect();

    for (rel, entry) in manifest.files.iter_mut() {
        let norm = rel.replace('\\', "/");
        let mut hit: Option<&str> = None;
        for (fid, patterns) in &compiled {
            if patterns.iter().any(|p| p.matches(&norm)) {
                if let Some(prev) = hit {
                    bail!(
                        "file '{}' matches two features ('{}' and '{}'); a file can belong to at most one feature",
                        rel,
                        prev,
                        fid
                    );
                }
                hit = Some(fid);
            }
        }
        if let Some(id) = hit {
            entry.feature = Some(id.to_string());
            *matched_per_feature.get_mut(id).unwrap() += 1;
        }
    }

    for f in features {
        let n = matched_per_feature[f.id.as_str()];
        if n == 0 {
            bail!(
                "feature '{}' matched no files (patterns: {:?}); check the paths",
                f.id,
                f.paths
            );
        }
        println!("Feature: {} <- {} file(s)", f.id, n);
    }

    let mut ids: Vec<String> = features.iter().map(|f| f.id.clone()).collect();
    ids.sort();
    ids.dedup();
    manifest.features = ids;

    let mut defaults: Vec<String> = features
        .iter()
        .filter(|f| f.default_enabled)
        .map(|f| f.id.clone())
        .collect();
    defaults.sort();
    defaults.dedup();
    manifest.default_features = defaults;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::model::file_entry::FileEntry;
    use std::collections::HashMap;

    fn glob_match(pattern: &str, path: &str) -> bool {
        CompiledPattern::new(pattern).matches(path)
    }

    #[test]
    fn glob_prefix_covers_subtree() {
        assert!(glob_match("Dossier1", "Dossier1/a.txt"));
        assert!(glob_match("Dossier1", "Dossier1/sub/b.txt"));
        assert!(glob_match("Dossier1", "Dossier1")); // the dir entry itself
        assert!(!glob_match("Dossier1", "Dossier10/a.txt")); // not a prefix segment
        assert!(!glob_match("Dossier1", "Other/a.txt"));
    }

    #[test]
    fn glob_star_and_doublestar() {
        assert!(glob_match("data/**", "data/a/b/c.bin"));
        assert!(glob_match("data/*.txt", "data/readme.txt"));
        assert!(!glob_match("data/*.txt", "data/sub/readme.txt")); // * stops at /
        assert!(glob_match("**/*.dll", "a/b/x.dll"));
        assert!(glob_match("*.exe", "app.exe"));
        assert!(!glob_match("*.exe", "bin/app.exe"));
    }

    #[test]
    fn glob_question_and_nested_doublestar() {
        assert!(glob_match("v?.dat", "v1.dat"));
        assert!(!glob_match("v?.dat", "v12.dat")); // ? is exactly one char
        assert!(!glob_match("v?.dat", "v/.dat")); // ? never crosses /
        // ** embedded between fixed segments.
        assert!(glob_match("a/**/z.bin", "a/z.bin")); // ** matches zero segments
        assert!(glob_match("a/**/z.bin", "a/b/c/z.bin"));
        assert!(!glob_match("a/**/z.bin", "a/b/c/y.bin"));
        // trailing-slash prefix is normalized.
        assert!(glob_match("data/", "data/x"));
    }

    fn entry() -> FileEntry {
        FileEntry {
            hash: "h".into(),
            size: 1,
            patch: None,
            feature: None,
        }
    }

    fn manifest_with(files: &[&str]) -> Manifest {
        let mut m: HashMap<String, FileEntry> = HashMap::new();
        for f in files {
            m.insert((*f).to_string(), entry());
        }
        Manifest {
            version: "1".into(),
            exe: None,
            files: m,
            deleted_files: vec![],
            full_size: 0,
            total_patch_size: 0,
            features: vec![],
            default_features: vec![],
        }
    }

    fn feat(id: &str, paths: &[&str]) -> ResolvedFeature {
        feat_def(id, paths, false)
    }

    fn feat_def(id: &str, paths: &[&str], default_enabled: bool) -> ResolvedFeature {
        ResolvedFeature {
            id: id.into(),
            paths: paths.iter().map(|s| s.to_string()).collect(),
            default_enabled,
        }
    }

    #[test]
    fn apply_tags_and_records_ids() {
        let mut m = manifest_with(&["base.exe", "D1/a.txt", "D1/b.txt", "D2/x.dat"]);
        apply(&mut m, &[feat("D1", &["D1"]), feat("D2", &["D2"])]).unwrap();
        assert_eq!(m.files["base.exe"].feature, None);
        assert_eq!(m.files["D1/a.txt"].feature.as_deref(), Some("D1"));
        assert_eq!(m.files["D2/x.dat"].feature.as_deref(), Some("D2"));
        assert_eq!(m.features, vec!["D1".to_string(), "D2".to_string()]);
        assert!(m.default_features.is_empty()); // none default-enabled
    }

    #[test]
    fn apply_records_default_enabled_subset() {
        let mut m = manifest_with(&["D1/a.txt", "D2/x.dat"]);
        apply(
            &mut m,
            &[
                feat_def("D1", &["D1"], true),
                feat_def("D2", &["D2"], false),
            ],
        )
        .unwrap();
        assert_eq!(m.features, vec!["D1".to_string(), "D2".to_string()]);
        assert_eq!(m.default_features, vec!["D1".to_string()]); // only the default-on one
    }

    #[test]
    fn apply_rejects_overlap() {
        let mut m = manifest_with(&["shared/a.txt"]);
        let err = apply(&mut m, &[feat("A", &["shared"]), feat("B", &["shared/**"])]).unwrap_err();
        assert!(err.to_string().contains("two features"), "{err}");
    }

    #[test]
    fn apply_rejects_unmatched_feature() {
        let mut m = manifest_with(&["base.exe"]);
        let err = apply(&mut m, &[feat("Ghost", &["nope/**"])]).unwrap_err();
        assert!(err.to_string().contains("matched no files"), "{err}");
    }
}
