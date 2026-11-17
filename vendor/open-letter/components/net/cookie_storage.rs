// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/net/cookie_storage.rs`.

#[allow(unused_imports)]
use super::*;

impl CookieStorage {
    /// Add the cookies of `other`. A cookie with the same name, domain and path as one
    /// held replaces it only if it was accessed more recently. Expired cookies are
    /// dropped, and each host keeps at most `max_per_host` cookies, the most recently
    /// accessed.
    pub fn merge(&mut self, other: CookieStorage) {
        for (host, incoming_cookies) in other.cookies_map {
            let cookies = self.cookies_map.entry(host).or_default();
            for incoming in incoming_cookies {
                let same_cookie = cookies.iter().position(|cookie| {
                    cookie.cookie.name() == incoming.cookie.name() &&
                        cookie.cookie.domain() == incoming.cookie.domain() &&
                        cookie.cookie.path() == incoming.cookie.path()
                });
                match same_cookie {
                    Some(index) if cookies[index].last_access >= incoming.last_access => {},
                    Some(index) => cookies[index] = incoming,
                    None => cookies.push(incoming),
                }
            }
            cookies.retain(|cookie| !is_cookie_expired(cookie));
            if cookies.len() > self.max_per_host {
                cookies.sort_by(|a, b| b.last_access.cmp(&a.last_access));
                cookies.truncate(self.max_per_host);
            }
        }
        self.cookies_map.retain(|_, cookies| !cookies.is_empty());
    }
}
