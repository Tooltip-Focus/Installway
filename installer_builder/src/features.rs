// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

//! Feature-pack tagging: map manifest files to a feature id by glob. The mapping
//! rides in the signed manifest, so the valid feature subsets are fixed at build
//! time — a plugin picks among them, it can't invent files.

use crate::args::ResolvedFeature;
use anyhow::{Result, bail};
use common::model::manifest::Manifest;

/// Glob match over a `/`-separated relative path. Supports `*` (any run of
/// chars within a path segment, i.e. not crossing `/`), `**` (any run including
/// `/`), and `?` (one non-`/` char). A bare prefix with no wildcard also matches
/// the whole subtree (so `Dossier1` covers `Dossier1/**`), which is the common
/// "this folder is feature X" case.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    // Plain prefix (no wildcards) → exact file or directory subtree.
    if !pattern.contains(['*', '?']) {
        let p = pattern.trim_end_matches('/');
        return path == p || path.starts_with(&format!("{p}/"));
    }
    let pat: Vec<&str> = pattern.split('/').collect();
    let txt: Vec<&str> = path.split('/').collect();
    seg_match(&pat, &txt)
}

/// Match path segments. A segment that is exactly `**` matches zero or more
/// segments; any other segment matches exactly one via [`seg_one`] (where `*`
/// and `?` stay within that segment, never crossing `/`).
fn seg_match(pat: &[&str], txt: &[&str]) -> bool {
    if pat.is_empty() {
        return txt.is_empty();
    }
    if pat[0] == "**" {
        // Consume 0..=txt.len() segments, then match the rest.
        return (0..=txt.len()).any(|skip| seg_match(&pat[1..], &txt[skip..]));
    }
    if txt.is_empty() {
        return false;
    }
    seg_one(pat[0].as_bytes(), txt[0].as_bytes()) && seg_match(&pat[1..], &txt[1..])
}

/// Wildcard match within one path segment: `*` matches any run of chars, `?` one
/// char. (No `/` can appear here — segments are split on it.)
fn seg_one(pat: &[u8], txt: &[u8]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None; // (pat_idx_of_star, txt_idx)
    while ti < txt.len() {
        if pi < pat.len() && (pat[pi] == txt[ti] || pat[pi] == b'?') {
            pi += 1;
            ti += 1;
        } else if pi < pat.len() && pat[pi] == b'*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, ref mut st)) = star {
            *st += 1;
            ti = *st;
            pi = sp + 1;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }
    pi == pat.len()
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

    for (rel, entry) in manifest.files.iter_mut() {
        let norm = rel.replace('\\', "/");
        let mut hit: Option<&str> = None;
        for f in features {
            if f.paths.iter().any(|p| glob_match(p, &norm)) {
                if let Some(prev) = hit {
                    bail!(
                        "file '{}' matches two features ('{}' and '{}'); a file can belong to at most one feature",
                        rel,
                        prev,
                        f.id
                    );
                }
                hit = Some(&f.id);
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
            exe: String::new(),
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
