// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/net/tests/cookie.rs`.

#[allow(unused_imports)]
use super::*;

#[test]
pub(super) fn test_cookie_storage_merge() {
    let url = ServoUrl::parse("http://example.com/").unwrap();
    let other_url = ServoUrl::parse("http://example.org/").unwrap();

    let mut held = CookieStorage::new(150);
    add_cookie_to_storage(&mut held, &url, "kept=held; Path=/");
    add_cookie_to_storage(&mut held, &url, "shared=old; Path=/");

    delay_to_ensure_different_timestamp();
    let mut imported = CookieStorage::new(150);
    add_cookie_to_storage(&mut imported, &url, "shared=new; Path=/");
    add_cookie_to_storage(&mut imported, &other_url, "added=imported; Path=/");

    held.merge(imported);

    // The more recently accessed `shared` wins; nothing held is removed.
    let cookies = held.cookies_for_url(&url, CookieSource::HTTP).unwrap();
    assert!(cookies.contains("kept=held"), "{cookies}");
    assert!(cookies.contains("shared=new"), "{cookies}");
    assert!(!cookies.contains("shared=old"), "{cookies}");
    assert_eq!(
        held.cookies_for_url(&other_url, CookieSource::HTTP),
        Some("added=imported".to_owned())
    );
}

#[test]
pub(super) fn test_cookie_storage_merge_keeps_the_more_recent_held_cookie() {
    let url = ServoUrl::parse("http://example.com/").unwrap();

    let mut imported = CookieStorage::new(150);
    add_cookie_to_storage(&mut imported, &url, "shared=old; Path=/");
    delay_to_ensure_different_timestamp();
    let mut held = CookieStorage::new(150);
    add_cookie_to_storage(&mut held, &url, "shared=new; Path=/");

    held.merge(imported);

    assert_eq!(
        held.cookies_for_url(&url, CookieSource::HTTP),
        Some("shared=new".to_owned())
    );
}

#[test]
pub(super) fn test_cookie_storage_merge_drops_expired_cookies_and_respects_the_host_limit() {
    let url = ServoUrl::parse("http://example.com/").unwrap();

    let mut held = CookieStorage::new(2);
    add_cookie_to_storage(&mut held, &url, "first=1; Path=/");
    delay_to_ensure_different_timestamp();
    let mut imported = CookieStorage::new(150);
    add_cookie_to_storage(&mut imported, &url, "second=2; Path=/");
    add_cookie_to_storage(&mut imported, &url, "third=3; Path=/");
    add_cookie_to_storage(&mut imported, &url, "gone=1; Max-Age=0; Path=/");

    held.merge(imported);

    let cookies = held.cookies_for_url(&url, CookieSource::HTTP).unwrap();
    assert!(!cookies.contains("gone"), "{cookies}");
    assert!(
        !cookies.contains("first"),
        "the least recently accessed goes: {cookies}"
    );
    assert!(
        cookies.contains("second") && cookies.contains("third"),
        "{cookies}"
    );
}
