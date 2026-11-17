// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `components/servo/tests/webview.rs`.

#[allow(unused_imports)]
use super::*;

/// A solid green, 64x64 PNG.
pub(super) fn green_png() -> Vec<u8> {
    let image = RgbaImage::from_pixel(64, 64, image::Rgba([0, 128, 0, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Png)
        .expect("Should be able to encode a PNG");
    bytes.into_inner()
}

/// Names the colour of a screenshot pixel, with its value, for assertions and output.
pub(super) fn colour_name(image: &RgbaImage, x: u32, y: u32) -> String {
    let [r, g, b, _] = image.get_pixel(x, y).0;
    let near = |value: u8, target: u8| value.abs_diff(target) < 40;
    let name = match (r, g, b) {
        (r, g, b) if near(r, 0) && near(g, 128) && near(b, 0) => "green",
        (r, g, b) if near(r, 255) && near(g, 255) && near(b, 255) => "white",
        (r, g, b) if near(r, 0) && near(g, 0) && near(b, 0) => "black",
        (r, g, b) if near(r, 255) && near(g, 0) && near(b, 0) => "red",
        (r, g, b) if near(r, 0) && near(g, 0) && near(b, 255) => "blue",
        _ => "other",
    };
    format!("{name}({r},{g},{b})")
}

pub(super) fn screenshot(servo_test: &ServoTest, webview: &WebView) -> RgbaImage {
    let image = Rc::new(RefCell::new(None));
    let callback_image = image.clone();
    webview.take_screenshot(None, move |result| {
        *callback_image.borrow_mut() = Some(result.expect("Screenshot should succeed"));
    });
    servo_test.spin(|| image.borrow().is_none());
    image.borrow_mut().take().unwrap()
}

/// Takes screenshots until each `(x, y, colour)` pixel has that colour, for up to five
/// seconds, and returns the colours last seen at those pixels.
pub(super) fn wait_for_colours(
    servo_test: &ServoTest,
    webview: &WebView,
    expected: &[(u32, u32, &str)],
) -> Vec<String> {
    let start = std::time::Instant::now();
    loop {
        let image = screenshot(servo_test, webview);
        let seen: Vec<String> = expected
            .iter()
            .map(|(x, y, _)| colour_name(&image, *x, *y))
            .collect();
        let all_match = seen
            .iter()
            .zip(expected)
            .all(|(seen, (_, _, colour))| seen.starts_with(colour));
        if all_match || start.elapsed() > std::time::Duration::from_secs(5) {
            return seen;
        }
        let pause = std::time::Instant::now();
        servo_test.spin(|| pause.elapsed() < std::time::Duration::from_millis(50));
    }
}

pub(super) fn assert_colours(
    servo_test: &ServoTest,
    webview: &WebView,
    what: &str,
    expected: &[(u32, u32, &str)],
) {
    let seen = wait_for_colours(servo_test, webview, expected);
    for (seen, (x, y, colour)) in seen.iter().zip(expected) {
        assert!(
            seen.starts_with(colour),
            "{what}: pixel ({x}, {y}) should be {colour}, is {seen}"
        );
    }
}

/// Serves each page in `pages` at its path, `/avatar.png` at once and `/late.png`
/// after 1.5 seconds.
pub(super) fn serve_pages(pages: Vec<(&'static str, String)>) -> (net::test_util::Server, Url) {
    let png = green_png();
    let handler =
        move |request: HyperRequest<Incoming>,
              response: &mut HyperResponse<BoxBody<Bytes, hyper::Error>>| {
            let path = request.uri().path().to_owned();
            if path == "/avatar.png" || path == "/late.png" {
                if path == "/late.png" {
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                }
                response.headers_mut().insert(
                    http::header::CONTENT_TYPE,
                    HeaderValue::from_static("image/png"),
                );
                *response.body_mut() = make_body(png.clone());
                return;
            }
            let page = pages
                .iter()
                .find(|(page_path, _)| *page_path == path)
                .map(|(_, page)| page.clone())
                .unwrap_or_default();
            *response.body_mut() = make_body(page.into_bytes());
        };
    let (server, url) = make_server(handler);
    (server, url.as_url().clone())
}

pub(super) fn load_page(servo_test: &ServoTest, url: Url) -> (WebView, Rc<WebViewDelegateImpl>) {
    let delegate = Rc::new(WebViewDelegateImpl::default());
    let webview = WebViewBuilder::new(servo_test.servo(), servo_test.rendering_context.clone())
        .delegate(delegate.clone())
        .url(url)
        .build();
    servo_test.spin(|| {
        !matches!(
            webview.load_status(),
            LoadStatus::Complete | LoadStatus::Stopped,
        )
    });
    (webview, delegate)
}

pub(super) fn test_page(body: &str) -> String {
    format!("<!DOCTYPE html><style>html, body {{ margin: 0; background: white }}</style>{body}")
}

/// An inline `<svg>` is painted with the computed values of its elements, wherever they
/// come from: the page's stylesheet, custom properties, `currentColor` inherited from
/// outside the `<svg>`, and `:hover`. They must follow later changes, not only the first.
///
/// The hover rule is on the `<svg>`: see [`test_inline_svg_shape_hover`] for hovering
/// a shape inside it.
#[test]
pub(super) fn test_inline_svg_paint_from_page_styles() {
    let servo_test = ServoTest::new();
    let bubble = test_page(
        "<style>\
         #box { --fg: rgb(0, 128, 0); color: rgb(0, 0, 255) }\
         svg { stroke: currentColor }\
         .bubble { fill: var(--fg) }\
         svg:hover .bubble { fill: rgb(255, 0, 0) }\
         </style>\
         <div id=box><svg width=200 height=200>\
         <rect class=bubble x=20 y=20 width=160 height=160 stroke-width=20 /></svg></div>",
    );
    let current_color = test_page(
        "<style>.icon { fill: currentColor }</style>\
         <div style='color: white; background: rgb(0, 0, 255)'>\
         <svg width=100 height=100><rect width=100 height=100 fill=currentColor /></svg>\
         <svg class=icon width=100 height=100><rect width=100 height=100 /></svg></div>",
    );
    let (server, base) = serve_pages(vec![("/bubble", bubble), ("/current-color", current_color)]);

    let (webview, _delegate) = load_page(&servo_test, base.join("/bubble").unwrap());
    // The fill is the middle of the rect; the stroke straddles its edge at x = 20.
    assert_colours(
        &servo_test,
        &webview,
        "bubble",
        &[(100, 100, "green"), (20, 100, "blue"), (195, 195, "white")],
    );

    webview.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(
        DevicePoint::new(100., 100.).into(),
    )));
    assert_colours(
        &servo_test,
        &webview,
        "bubble hovered",
        &[(100, 100, "red"), (20, 100, "blue")],
    );

    webview.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(
        DevicePoint::new(400., 400.).into(),
    )));
    assert_colours(
        &servo_test,
        &webview,
        "bubble no longer hovered",
        &[(100, 100, "green"), (20, 100, "blue")],
    );

    // A custom property and a colour changed outside the `<svg>`.
    let _ = evaluate_javascript(
        &servo_test,
        webview.clone(),
        "box.style.setProperty('--fg', 'rgb(255, 255, 255)'); box.style.color = 'rgb(255, 0, 0)';",
    );
    assert_colours(
        &servo_test,
        &webview,
        "bubble after a style change outside the svg",
        &[(100, 100, "white"), (20, 100, "red")],
    );

    let (webview, _delegate) = load_page(&servo_test, base.join("/current-color").unwrap());
    assert_colours(
        &servo_test,
        &webview,
        "currentColor",
        &[(50, 50, "white"), (150, 50, "white")],
    );

    let _ = server.close();
}

/// `:hover` on a shape inside an inline `<svg>`. The `<svg>` is laid out as one replaced
/// box, rasterized as a whole, so hit testing finds the `<svg>` and never a shape in it.
#[test]
#[ignore = "shapes inside an inline <svg> are not hit-testable"]
pub(super) fn test_inline_svg_shape_hover() {
    let servo_test = ServoTest::new();
    let page = test_page(
        "<style>.bubble { fill: rgb(0, 128, 0) } .bubble:hover { fill: rgb(255, 0, 0) }</style>         <svg width=200 height=200><rect class=bubble x=20 y=20 width=160 height=160 /></svg>",
    );
    let (server, base) = serve_pages(vec![("/hover", page)]);
    let (webview, _delegate) = load_page(&servo_test, base.join("/hover").unwrap());
    assert_colours(&servo_test, &webview, "shape", &[(100, 100, "green")]);
    webview.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(
        DevicePoint::new(100., 100.).into(),
    )));
    assert_colours(&servo_test, &webview, "shape hovered", &[(100, 100, "red")]);
    let _ = server.close();
}

/// An `<image>` in an inline `<svg>` loads its `href`, including under a mask or a clip
/// path, and is painted when it arrives after the first paint.
#[test]
pub(super) fn test_inline_svg_image_loads_href() {
    let servo_test = ServoTest::new();
    let mask = test_page(
        "<svg width=100 height=100><mask id=m><circle cx=50 cy=50 r=50 fill=white /></mask>\
         <image href='/avatar.png' width=100 height=100 mask='url(#m)' /></svg>",
    );
    let clip = test_page(
        "<svg width=100 height=100><clipPath id=c><circle cx=50 cy=50 r=50 /></clipPath>\
         <image href='/avatar.png' width=100 height=100 clip-path='url(#c)' /></svg>",
    );
    let plain = test_page(
        "<svg width=100 height=100><image href='/avatar.png' width=100 height=100 /></svg>",
    );
    let control = test_page(
        "<img src='/avatar.png' style='width: 100px; height: 100px; border-radius: 50%'>",
    );
    let late = test_page(
        "<svg id=svg width=100 height=100><mask id=m><circle cx=50 cy=50 r=50 fill=white />\
         </mask></svg>",
    );
    let (server, base) = serve_pages(vec![
        ("/mask", mask),
        ("/clip", clip),
        ("/plain", plain),
        ("/control", control),
        ("/late", late),
    ]);

    for path in ["/mask", "/clip", "/control"] {
        let (webview, _delegate) = load_page(&servo_test, base.join(path).unwrap());
        // Inside the circle, and outside it in the corner.
        assert_colours(
            &servo_test,
            &webview,
            path,
            &[(50, 50, "green"), (3, 3, "white")],
        );
    }
    let (webview, _delegate) = load_page(&servo_test, base.join("/plain").unwrap());
    assert_colours(
        &servo_test,
        &webview,
        "/plain",
        &[(50, 50, "green"), (3, 3, "green")],
    );

    // Insert the image after the first paint; it is served 1.5 seconds later.
    let (webview, _delegate) = load_page(&servo_test, base.join("/late").unwrap());
    assert_colours(&servo_test, &webview, "/late before", &[(50, 50, "white")]);
    let _ = evaluate_javascript(
        &servo_test,
        webview.clone(),
        "window.loaded = false;\
         const image = document.createElementNS('http://www.w3.org/2000/svg', 'image');\
         image.onload = () => { window.loaded = true; };\
         image.setAttribute('width', '100');\
         image.setAttribute('height', '100');\
         image.setAttribute('mask', 'url(#m)');\
         image.setAttribute('href', '/late.png');\
         svg.appendChild(image);",
    );
    assert_colours(
        &servo_test,
        &webview,
        "/late after the image arrived",
        &[(50, 50, "green"), (3, 3, "white")],
    );
    assert_eq!(
        evaluate_javascript(&servo_test, webview.clone(), "window.loaded"),
        Ok(JSValue::Boolean(true)),
        "the image should have fired load"
    );

    let _ = server.close();
}

/// CSS counters: `counter()` in generated content should count, so the tenth line's
/// number is wider than the first's.
#[test]
#[ignore = "CSS counters are not implemented: counter() always formats 0"]
pub(super) fn test_css_counters_count() {
    let servo_test = ServoTest::new();
    let lines = "<span></span>\n".repeat(8);
    let page = test_page(&format!(
        "<style>pre {{ counter-reset: line }}\
         span {{ display: inline-block; counter-increment: line }}\
         span::before {{ content: counter(line) }}</style>\
         <pre><span id=first></span>\n{lines}<span id=tenth></span></pre>"
    ));
    let (server, base) = serve_pages(vec![("/counters", page)]);
    let (webview, _delegate) = load_page(&servo_test, base.join("/counters").unwrap());
    let result = evaluate_javascript(
        &servo_test,
        webview,
        "tenth.getBoundingClientRect().width > first.getBoundingClientRect().width",
    );
    assert_eq!(result, Ok(JSValue::Boolean(true)));
    let _ = server.close();
}
