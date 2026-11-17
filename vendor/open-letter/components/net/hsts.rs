// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/net/hsts.rs`.

#[allow(unused_imports)]
use super::*;

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::num::NonZeroU64;
use std::sync::LazyLock;
use std::time::Duration;
use embedder_traits::resources::{self, Resource};
use fst::{Map, MapBuilder};
use headers::{HeaderMapExt, StrictTransportSecurity};
use http::HeaderMap;
use log::{debug, error, info};
use malloc_size_of::{MallocSizeOf, MallocSizeOfOps};
use malloc_size_of_derive::MallocSizeOf;
use net_traits::IncludeSubdomains;
use net_traits::pub_domains::reg_suffix;
use serde::{Deserialize, Serialize};
use servo_config::pref;
use servo_url::{Host, ServoUrl};
use time::UtcDateTime;

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
