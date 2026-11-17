// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/servo/tests/site_data_manager.rs`.

#[allow(unused_imports)]
use super::*;

#[test]
pub(super) fn test_site_state_export_and_import_cookies() {
    use servo::{SiteStateImportMode, SiteStateKind};

    let servo_test = ServoTest::new();
    let site_data_manager = servo_test.servo().site_data_manager();
    let url = url::Url::parse("http://example.com/").unwrap();
    let names = |site_data_manager: &servo::SiteDataManager| {
        let mut names: Vec<String> = site_data_manager
            .cookies_for_url(url.clone(), CookieSource::HTTP)
            .iter()
            .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
            .collect();
        names.sort();
        names
    };

    site_data_manager.set_cookie_for_url(
        url.clone(),
        Cookie::build(("exported", "1")).path("/").build(),
        None,
    );
    let exported = site_data_manager.export_site_state(SiteStateKind::Cookies);
    assert!(
        String::from_utf8_lossy(&exported).contains("exported"),
        "the export should hold the cookie"
    );

    // Replace: the held jar becomes the imported one.
    site_data_manager.clear_cookies(None);
    site_data_manager.set_cookie_for_url(
        url.clone(),
        Cookie::build(("discarded", "1")).path("/").build(),
        None,
    );
    assert_eq!(
        site_data_manager.import_site_state(
            SiteStateKind::Cookies,
            exported.clone(),
            SiteStateImportMode::Replace
        ),
        Ok(())
    );
    assert_eq!(names(site_data_manager), vec!["exported=1".to_owned()]);

    // Merge: what is held is kept and the import is added.
    site_data_manager.clear_cookies(None);
    site_data_manager.set_cookie_for_url(
        url.clone(),
        Cookie::build(("held", "1")).path("/").build(),
        None,
    );
    assert_eq!(
        site_data_manager.import_site_state(
            SiteStateKind::Cookies,
            exported,
            SiteStateImportMode::Merge
        ),
        Ok(())
    );
    assert_eq!(
        names(site_data_manager),
        vec!["exported=1".to_owned(), "held=1".to_owned()]
    );

    // Data that does not parse is refused, and nothing changes.
    assert!(
        site_data_manager
            .import_site_state(
                SiteStateKind::Cookies,
                b"not json".to_vec(),
                SiteStateImportMode::Replace
            )
            .is_err()
    );
    assert_eq!(
        names(site_data_manager),
        vec!["exported=1".to_owned(), "held=1".to_owned()]
    );

    // The other kinds export and import too.
    for kind in [SiteStateKind::Hsts, SiteStateKind::AuthCache] {
        let exported = site_data_manager.export_site_state(kind);
        assert!(!exported.is_empty(), "{kind:?} should export");
        assert_eq!(
            site_data_manager.import_site_state(kind, exported, SiteStateImportMode::Merge),
            Ok(())
        );
    }
}

/// Starts an engine on a new config directory with `write_on_exit`, sets a cookie, shuts
/// the engine down, and returns the names of the files left in the directory.
pub(super) fn files_left_after_exit(write_on_exit: bool) -> Vec<String> {
    let config_dir = std::env::temp_dir().join(format!(
        "site-state-exit-{}-{write_on_exit}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&config_dir);
    std::fs::create_dir_all(&config_dir).unwrap();

    let servo_test = ServoTest::new_with_builder(|builder| {
        let mut opts = servo_config::opts::Opts::default();
        opts.config_dir = Some(config_dir.clone());
        let mut preferences = servo::Preferences::default();
        preferences.network_http_proxy_uri = String::new();
        preferences.network_https_proxy_uri = String::new();
        preferences.network_site_state_write_on_exit = write_on_exit;
        builder.opts(opts).preferences(preferences)
    });
    servo_test.servo().site_data_manager().set_cookie_for_url(
        url::Url::parse("http://example.com/").unwrap(),
        Cookie::build(("persisted", "1"))
            .path("/")
            .max_age(cookie::time::Duration::hours(1))
            .build(),
        None,
    );
    let _ = servo_test.servo().site_data_manager().cookies_for_url(
        url::Url::parse("http://example.com/").unwrap(),
        CookieSource::HTTP,
    );
    // Dropping the engine waits for it to shut down.
    drop(servo_test);

    let mut files: Vec<String> = std::fs::read_dir(&config_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    let _ = std::fs::remove_dir_all(&config_dir);
    files
}

#[test]
pub(super) fn test_site_state_written_on_exit() {
    let files = files_left_after_exit(true);
    assert!(files.contains(&"cookie_jar.json".to_owned()), "{files:?}");
    assert!(
        !files.iter().any(|file| file.ends_with(".tmp")),
        "no temporary file should be left: {files:?}"
    );
}

#[test]
pub(super) fn test_site_state_not_written_on_exit_when_turned_off() {
    let files = files_left_after_exit(false);
    assert!(
        !files.iter().any(|file| file.starts_with("cookie_jar")),
        "{files:?}"
    );
}
