// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools-DAP

//! Open Letter additions to `components/net/hsts.rs`.

#[allow(unused_imports)]
use super::*;

impl HstsList {
    /// Add the entries of `other`. An entry for a host already held replaces it only if
    /// it expires later; an entry with no expiry never expires.
    pub fn merge(&mut self, other: HstsList) {
        let expiry = |entry: &HstsEntry| entry.expires_at.map_or(u64::MAX, NonZeroU64::get);
        for (base_domain, incoming_entries) in other.entries_map {
            let entries = self.entries_map.entry(base_domain).or_default();
            for incoming in incoming_entries {
                match entries.iter().position(|entry| entry.host == incoming.host) {
                    Some(index) if expiry(&entries[index]) >= expiry(&incoming) => {},
                    Some(index) => entries[index] = incoming,
                    None => entries.push(incoming),
                }
            }
        }
    }
}
