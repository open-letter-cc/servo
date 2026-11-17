// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/servo/tests/javascript_evaluator.rs`.

#[allow(unused_imports)]
use super::*;

/// A function that recurses until the engine stops it, and returns
/// `[depth reached, error]`.
pub(super) const RECURSE_UNTIL_STOPPED_FUNCTION: &str = "function recurseUntilStopped() {\
    let depth = 0;\
    function recurse() { depth++; recurse(); }\
    try { recurse(); } catch (error) { return [depth, String(error)]; }\
    return [depth, 'no error'];\
}";

/// Starts an engine with `preferences` changed by `customize`, on a page served over
/// HTTP so that it may start workers.
pub(super) fn engine_on_served_page(
    customize: impl FnOnce(&mut servo::Preferences),
) -> (ServoTest, servo::WebView, net::test_util::Server) {
    let servo_test = ServoTest::new_with_builder(|builder| {
        let mut preferences = servo::Preferences::default();
        preferences.network_http_proxy_uri = String::new();
        preferences.network_https_proxy_uri = String::new();
        customize(&mut preferences);
        builder.preferences(preferences)
    });
    let handler =
        move |_: HyperRequest<Incoming>,
              response: &mut HyperResponse<BoxBody<Bytes, hyper::Error>>| {
            *response.body_mut() = make_body(b"<!DOCTYPE html><body></body>".to_vec());
        };
    let (server, url) = make_server(handler);
    let delegate = Rc::new(WebViewDelegateImpl::default());
    let webview = WebViewBuilder::new(servo_test.servo(), servo_test.rendering_context.clone())
        .delegate(delegate)
        .url(url.as_url().clone())
        .build();
    (servo_test, webview, server)
}

/// The depth reached and the error thrown by [`RECURSE_UNTIL_STOPPED_FUNCTION`] on the script
/// thread.
pub(super) fn script_recursion(servo_test: &ServoTest, webview: &servo::WebView) -> (f64, String) {
    let script = format!("({RECURSE_UNTIL_STOPPED_FUNCTION})()");
    match evaluate_javascript(servo_test, webview.clone(), script) {
        Ok(JSValue::Array(values)) => match values.as_slice() {
            [JSValue::Number(depth), JSValue::String(error)] => (*depth, error.clone()),
            other => panic!("unexpected recursion result: {other:?}"),
        },
        other => panic!("unexpected recursion result: {other:?}"),
    }
}

/// The depth reached by [`RECURSE_UNTIL_STOPPED_FUNCTION`] in a dedicated worker.
pub(super) fn worker_recursion_depth(servo_test: &ServoTest, webview: &servo::WebView) -> f64 {
    let started = evaluate_javascript(
        servo_test,
        webview.clone(),
        format!(
            "window.workerResult = null;\
             const source = 'postMessage((' + ({RECURSE_UNTIL_STOPPED_FUNCTION}).toString() + ')());';\
             const worker = new Worker(URL.createObjectURL(new Blob([source])));\
             worker.onmessage = event => {{ window.workerResult = event.data; }};\
             worker.onerror = event => {{ window.workerResult = ['error', String(event.message)]; }};\
             true"
        ),
    );
    assert_eq!(started, Ok(JSValue::Boolean(true)));
    let start = std::time::Instant::now();
    loop {
        match evaluate_javascript(servo_test, webview.clone(), "window.workerResult") {
            Ok(JSValue::Array(values)) => match values.as_slice() {
                [JSValue::Number(depth), JSValue::String(error)] => {
                    assert!(
                        error.contains("too much recursion"),
                        "the worker should have thrown, threw {error}"
                    );
                    return *depth;
                },
                other => panic!("unexpected worker result: {other:?}"),
            },
            Ok(JSValue::Null) if start.elapsed() < std::time::Duration::from_secs(30) => {
                let pause = std::time::Instant::now();
                servo_test.spin(|| pause.elapsed() < std::time::Duration::from_millis(50));
            },
            other => panic!("the worker did not answer: {other:?}"),
        }
    }
}

#[test]
pub(super) fn test_deep_recursion_throws_instead_of_overflowing() {
    let (servo_test, webview, server) = engine_on_served_page(|_| {});
    let (depth, error) = script_recursion(&servo_test, &webview);
    assert!(
        error.contains("too much recursion"),
        "deep recursion should throw a catchable error, threw {error}"
    );
    assert!(depth > 1000.0, "recursion stopped too early, at {depth}");
    // The engine is still running script afterwards.
    assert_eq!(
        evaluate_javascript(&servo_test, webview.clone(), "1 + 1"),
        Ok(JSValue::Number(2.0))
    );
    let _ = server.close();
}

/// The script thread's stack size preference sets how deep script recurses, and a small
/// worker stack still throws rather than overflowing.
#[test]
pub(super) fn test_script_thread_stack_size_preference() {
    let (servo_test, webview, server) = engine_on_served_page(|preferences| {
        preferences.thread_script_stack_size_kb = 16 * 1024;
        preferences.thread_worker_stack_size_kb = 1024;
    });
    let (script_depth, error) = script_recursion(&servo_test, &webview);
    assert!(error.contains("too much recursion"), "{error}");
    let worker_depth = worker_recursion_depth(&servo_test, &webview);
    assert!(
        script_depth > worker_depth * 4.0,
        "a 16 MiB script stack should recurse far deeper than a 1 MiB worker stack: \
         script {script_depth}, worker {worker_depth}"
    );
    let _ = server.close();
}

/// The worker stack size preference sets how deep a worker recurses, and a small script
/// stack still throws rather than overflowing.
#[test]
pub(super) fn test_worker_thread_stack_size_preference() {
    let (servo_test, webview, server) = engine_on_served_page(|preferences| {
        preferences.thread_script_stack_size_kb = 1024;
        preferences.thread_worker_stack_size_kb = 16 * 1024;
    });
    let (script_depth, error) = script_recursion(&servo_test, &webview);
    assert!(error.contains("too much recursion"), "{error}");
    let worker_depth = worker_recursion_depth(&servo_test, &webview);
    assert!(
        worker_depth > script_depth * 4.0,
        "a 16 MiB worker stack should recurse far deeper than a 1 MiB script stack: \
         script {script_depth}, worker {worker_depth}"
    );
    let _ = server.close();
}

/// The JavaScript heap limit stops an allocation that exceeds it, and the engine carries on.
///
/// Reaching the limit panics the script thread today: reporting the out-of-memory error
/// creates an `ErrorEvent`, whose prototype cannot be allocated in the full heap, and the
/// bindings assert that object creation succeeds.
#[test]
#[ignore = "reaching js_mem_max panics the script thread (object creation asserts)"]
pub(super) fn test_js_heap_limit_preference() {
    let (servo_test, webview, server) = engine_on_served_page(|preferences| {
        preferences.js_mem_max = 64;
    });
    let allocate = |count: u32| {
        evaluate_javascript(
            &servo_test,
            webview.clone(),
            format!(
                "(() => {{ const kept = []; for (let i = 0; i < {count}; i++) \
                 kept.push({{ i, text: 'item ' + i }}); return kept.length; }})()"
            ),
        )
    };
    assert_eq!(allocate(10_000), Ok(JSValue::Number(10_000.0)));
    assert!(
        allocate(4_000_000).is_err(),
        "allocating past a 64 MiB heap should fail"
    );
    assert_eq!(
        evaluate_javascript(&servo_test, webview.clone(), "1 + 1"),
        Ok(JSValue::Number(2.0))
    );
    let _ = server.close();
}

/// A page `width` columns wide, each nested `depth` elements deep, laid out: returns the
/// body's height.
pub(super) fn lay_out_wide_and_deep_page(
    servo_test: &ServoTest,
    webview: &servo::WebView,
    width: u32,
    depth: u32,
) -> Result<JSValue, JavaScriptEvaluationError> {
    evaluate_javascript(
        servo_test,
        webview.clone(),
        format!(
            "(() => {{\
               for (let column = 0; column < {width}; column++) {{\
                 let node = document.body.appendChild(document.createElement('div'));\
                 for (let level = 0; level < {depth}; level++) {{\
                   node = node.appendChild(document.createElement('div'));\
                 }}\
                 node.textContent = 'leaf';\
               }}\
               return document.body.offsetHeight;\
             }})()"
        ),
    )
}

/// The layout stack size preference gives parallel layout stacks deep enough for a page
/// that overflows the default ones. A page 8 columns wide and 1000 elements deep
/// overflows the default style threads' stacks (and so the process) in a debug build;
/// with 64 MiB layout stacks it lays out.
#[test]
pub(super) fn test_layout_thread_stack_size_preference() {
    let (servo_test, webview, server) = engine_on_served_page(|preferences| {
        preferences.thread_layout_stack_size_kb = 64 * 1024;
    });
    let result = lay_out_wide_and_deep_page(&servo_test, &webview, 8, 1000);
    assert!(
        matches!(result, Ok(JSValue::Number(height)) if height > 0.0),
        "the page should lay out: {result:?}"
    );
    let _ = server.close();
}
