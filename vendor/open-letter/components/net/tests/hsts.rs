// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/net/tests/hsts.rs`.

#[allow(unused_imports)]
use super::*;

#[test]
pub(super) fn test_hsts_list_merge_keeps_the_later_expiry() {
    let entry = |host: &str, expires_at: Option<u64>| HstsEntry {
        host: host.to_owned(),
        include_subdomains: false,
        expires_at: expires_at.map(|seconds| NonZeroU64::new(seconds).unwrap()),
    };
    let list = |entries: Vec<HstsEntry>| {
        let mut entries_map = HashMap::new();
        entries_map.insert("example.com".to_owned(), entries);
        HstsList { entries_map }
    };

    let far = 4_000_000_000;
    let near = 3_000_000_000;
    let mut held = list(vec![
        entry("example.com", Some(near)),
        entry("a.example.com", Some(far)),
        entry("b.example.com", None),
    ]);
    held.merge(list(vec![
        entry("example.com", Some(far)),
        entry("a.example.com", Some(near)),
        entry("b.example.com", Some(far)),
        entry("c.example.com", Some(near)),
    ]));

    let entries = &held.entries_map["example.com"];
    let expiry = |host: &str| {
        entries
            .iter()
            .find(|entry| entry.host == host)
            .map(|entry| entry.expires_at.map(NonZeroU64::get))
    };
    assert_eq!(entries.len(), 4);
    assert_eq!(expiry("example.com"), Some(Some(far)));
    assert_eq!(expiry("a.example.com"), Some(Some(far)));
    assert_eq!(expiry("b.example.com"), Some(None));
    assert_eq!(expiry("c.example.com"), Some(Some(near)));
}
