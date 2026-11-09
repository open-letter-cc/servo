// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

// Tests for the resource request callback.
//
// The page tree, written to the temp directory:
// - `rr_main.html`, the top-level document, which contains an iframe and makes three
//   `fetch()` calls: a GET carrying a custom header, a POST, and one the callback
//   refuses;
// - `rr_frame.html`, the iframe's `src`, which navigates itself by script to
// - `rr_frame2.html`, a nested-frame navigation with a `document` destination.
//
// The first `WebView` has the callback set on its builder. The second has a delegate
// but no callback and must behave as before the callback existed: the fetch the first
// one refuses succeeds there. A third, built with neither, must refuse a live setter.

#define _POSIX_C_SOURCE 200809L // required by `nanosleep` in `<time.h>`
#include "servo/servo_capi.h"
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <windows.h>
#else
#include <time.h>
#endif

#define log(fmt, ...)               \
    do {                            \
        printf(fmt, ##__VA_ARGS__); \
        fflush(stdout);             \
    } while (0)

static void sleep_ms(long period)
{
#ifdef _WIN32
    Sleep(period);
#else
    struct timespec delay;
    delay.tv_sec = period / 1000;
    delay.tv_nsec = (period % 1000) * 1000000;
    nanosleep(&delay, NULL);
#endif
}

#define MAX_REQUESTS 64

typedef struct {
    char url[512];
    char method[16];
    char destination[32];
    char probe[64];
    uint8_t kind;
    bool is_redirect;
} seen_request_t;

typedef struct {
    seen_request_t requests[MAX_REQUESTS];
    int request_count;
    char title[256];
} test_state_t;

static test_state_t vetted;
static test_state_t plain;

static void noop_wake_callback(void) { }

static void copy_bytes(char* dest, size_t dest_size, const uint8_t* ptr, size_t len)
{
    size_t n = ptr == NULL ? 0 : len;
    n = n < dest_size - 1 ? n : dest_size - 1;
    if (n > 0) {
        memcpy(dest, ptr, n);
    }
    dest[n] = '\0';
}

static int ends_with(const char* s, const char* suffix)
{
    size_t s_len = strlen(s), suffix_len = strlen(suffix);
    return s_len >= suffix_len && strcmp(s + s_len - suffix_len, suffix) == 0;
}

static bool on_resource_request(struct WebView* webview, const struct ServoResourceRequest* request,
    void* user_data)
{
    (void)webview;
    test_state_t* s = (test_state_t*)user_data;
    if (s->request_count >= MAX_REQUESTS) {
        return true;
    }
    seen_request_t* seen = &s->requests[s->request_count++];
    copy_bytes(seen->url, sizeof(seen->url), request->url_ptr, request->url_len);
    copy_bytes(seen->method, sizeof(seen->method), request->method_ptr, request->method_len);
    copy_bytes(seen->destination, sizeof(seen->destination), request->destination_ptr,
        request->destination_len);
    seen->probe[0] = '\0';
    for (size_t i = 0; i < request->headers_len; i++) {
        const struct ServoHttpHeader* header = &request->headers[i];
        if (header->name_len == 7 && memcmp(header->name_ptr, "x-probe", 7) == 0) {
            copy_bytes(seen->probe, sizeof(seen->probe), header->value_ptr, header->value_len);
        }
    }
    seen->kind = request->kind;
    seen->is_redirect = request->is_redirect;

    return strstr(seen->url, "deny-me") == NULL;
}

static void on_page_title_changed(struct WebView* webview, const uint8_t* title_ptr,
    size_t title_len, void* user_data)
{
    (void)webview;
    test_state_t* s = (test_state_t*)user_data;
    copy_bytes(s->title, sizeof(s->title), title_ptr, title_len);
}

static const char MAIN_PAGE[]
    = "<!doctype html><html><head><title>start</title></head><body>"
      "<iframe src=\"rr_frame.html\"></iframe><script>"
      "const outcome = p => p.then(r => r.text()).then(() => 'ok', () => 'rejected');"
      "Promise.all(["
      "  outcome(fetch('data:text/plain,allow-me', { headers: { 'X-Probe': 'probe-value' } })),"
      "  outcome(fetch('data:text/plain,post-me', { method: 'POST', body: 'x' })),"
      "  outcome(fetch('data:text/plain,deny-me')),"
      "]).then(([allowed, posted, denied]) => {"
      "  document.title = 'allowed:' + allowed + ' denied:' + denied;"
      "});"
      "</script></body></html>";

static const char FRAME_PAGE[]
    = "<!doctype html><script>location.replace('rr_frame2.html');</script>";

static const char FRAME2_PAGE[] = "<!doctype html><p>nested</p>";

// Writes `contents` as `name` in the temp directory and leaves its `file://` URL in
// `url`.
static int write_page(const char* name, const char* contents, size_t contents_len, char* url,
    size_t url_size)
{
    char path[512];
#ifdef _WIN32
    char dir[MAX_PATH];
    DWORD len = GetTempPathA(sizeof(dir), dir);
    if (len == 0 || len >= sizeof(dir)) {
        return 1;
    }
    snprintf(path, sizeof(path), "%s%s", dir, name);
#else
    const char* dir = getenv("TMPDIR");
    snprintf(path, sizeof(path), "%s/%s", dir != NULL ? dir : "/tmp", name);
#endif
    FILE* file = fopen(path, "wb");
    if (file == NULL) {
        return 1;
    }
    fwrite(contents, 1, contents_len, file);
    fclose(file);

    for (char* c = path; *c != '\0'; c++) {
        if (*c == '\\') {
            *c = '/';
        }
    }
    snprintf(url, url_size, "file://%s%s", path[0] == '/' ? "" : "/", path);
    return 0;
}

static const seen_request_t* find(const test_state_t* s, const char* url_suffix)
{
    for (int i = 0; i < s->request_count; i++) {
        if (ends_with(s->requests[i].url, url_suffix)) {
            return &s->requests[i];
        }
    }
    return NULL;
}

// Spins until `s` has a final title and, if `frame2_expected`, the nested navigation
// has been offered, then a little longer for stragglers.
static void spin_until_done(struct Servo* servo, const test_state_t* s, int frame2_expected)
{
    int done_at_ms = -1;
    for (int elapsed_ms = 0; elapsed_ms < 20000; elapsed_ms += 10) {
        servo_spin_event_loop(servo);
        if (done_at_ms < 0 && strncmp(s->title, "allowed:", 8) == 0
            && (!frame2_expected || find(s, "rr_frame2.html") != NULL)) {
            done_at_ms = elapsed_ms;
        }
        if (done_at_ms >= 0 && elapsed_ms - done_at_ms > 500) {
            return;
        }
        sleep_ms(10);
    }
}

static int expect(int ok, const char* name, int* failed)
{
    if (ok) {
        log("  PASS %s\n", name);
    } else {
        log("  FAIL %s\n", name);
        *failed += 1;
    }
    return ok;
}

int run_c_resource_request_tests(void)
{
    log("running resource request tests\n");
    memset(&vetted, 0, sizeof(vetted));
    memset(&plain, 0, sizeof(plain));

    char url[640], unused[640];
    if (write_page("rr_main.html", MAIN_PAGE, sizeof(MAIN_PAGE) - 1, url, sizeof(url)) != 0
        || write_page("rr_frame.html", FRAME_PAGE, sizeof(FRAME_PAGE) - 1, unused,
               sizeof(unused))
            != 0
        || write_page("rr_frame2.html", FRAME2_PAGE, sizeof(FRAME2_PAGE) - 1, unused,
               sizeof(unused))
            != 0) {
        log("  FAIL: could not write the test pages\n");
        return 1;
    }

    ServoBuilder* servo_builder = servo_builder_create();
    struct ServoEventLoopWaker waker = { noop_wake_callback };
    servo_builder_set_event_loop_waker(servo_builder, waker);
    ServoOptions* options = servo_options_create();
    servo_options_set_multiprocess(options, false);
    servo_builder_set_options(servo_builder, options);
    struct Servo* servo = servo_builder_build(servo_builder);
    if (servo == NULL) {
        log("  FAIL: servo_builder_build returned NULL\n");
        return 1;
    }

    int failed = 0;

    // The vetted `WebView`: callback set on the builder, so it sees the initial
    // navigation too.
    ServoWebViewBuilder* builder
        = servo_webview_builder_create(servo, servo_rendering_context_create_software(64, 64));
    ServoWebViewDelegate delegate = {
        .user_data = &vetted,
        .notify_page_title_changed = on_page_title_changed,
    };
    servo_webview_builder_set_delegate(builder, delegate);
    servo_webview_builder_set_resource_request_callback(builder, on_resource_request, &vetted);
    servo_webview_builder_set_url(builder, url);
    struct WebView* webview = servo_webview_builder_build(builder);
    spin_until_done(servo, &vetted, 1);

    const seen_request_t* main = find(&vetted, "rr_main.html");
    expect(main != NULL && main->kind == SERVO_RESOURCE_REQUEST_TOP_LEVEL_NAVIGATION
            && strcmp(main->method, "GET") == 0 && strcmp(main->destination, "document") == 0
            && !main->is_redirect,
        "top_level_navigation", &failed);

    const seen_request_t* frame = find(&vetted, "rr_frame.html");
    expect(frame != NULL && frame->kind == SERVO_RESOURCE_REQUEST_FRAME_NAVIGATION
            && strcmp(frame->destination, "iframe") == 0,
        "frame_src_navigation", &failed);

    // A navigation inside the frame has a `document` destination like a top-level one;
    // it must still be reported as a frame navigation.
    const seen_request_t* frame2 = find(&vetted, "rr_frame2.html");
    expect(frame2 != NULL && frame2->kind == SERVO_RESOURCE_REQUEST_FRAME_NAVIGATION
            && strcmp(frame2->destination, "document") == 0,
        "nested_document_navigation", &failed);

    const seen_request_t* allowed = find(&vetted, "allow-me");
    expect(allowed != NULL && allowed->kind == SERVO_RESOURCE_REQUEST_SUBRESOURCE
            && strcmp(allowed->method, "GET") == 0 && strcmp(allowed->probe, "probe-value") == 0,
        "subresource_with_headers", &failed);

    const seen_request_t* posted = find(&vetted, "post-me");
    expect(posted != NULL && strcmp(posted->method, "POST") == 0, "method", &failed);

    expect(find(&vetted, "deny-me") != NULL
            && strcmp(vetted.title, "allowed:ok denied:rejected") == 0,
        "deny_is_network_error", &failed);

    expect(servo_webview_set_resource_request_callback(webview, NULL, NULL), "live_clear",
        &failed);

    // The plain `WebView`: a delegate, no callback. Nothing is refused, and no callback
    // hears of its requests: not the vetted one's, which was just cleared, nor any other.
    int vetted_count_before = vetted.request_count;
    builder = servo_webview_builder_create(servo, servo_rendering_context_create_software(64, 64));
    ServoWebViewDelegate plain_delegate = {
        .user_data = &plain,
        .notify_page_title_changed = on_page_title_changed,
    };
    servo_webview_builder_set_delegate(builder, plain_delegate);
    servo_webview_builder_set_url(builder, url);
    struct WebView* plain_webview = servo_webview_builder_build(builder);
    spin_until_done(servo, &plain, 0);
    expect(strcmp(plain.title, "allowed:ok denied:ok") == 0, "unset_is_unchanged", &failed);
    expect(plain.request_count == 0 && vetted.request_count == vetted_count_before,
        "cleared_and_unset_are_silent", &failed);
    if (strcmp(plain.title, "allowed:ok denied:ok") != 0) {
        log("    plain title '%s'\n", plain.title);
    }

    // Neither delegate nor callback: Servo's own default delegate, which cannot take one.
    builder = servo_webview_builder_create(servo, servo_rendering_context_create_software(64, 64));
    struct WebView* bare_webview = servo_webview_builder_build(builder);
    expect(!servo_webview_set_resource_request_callback(bare_webview, on_resource_request, &plain),
        "live_set_refused_without_capi_delegate", &failed);

    if (failed) {
        log("    vetted title '%s'\n", vetted.title);
        for (int i = 0; i < vetted.request_count; i++) {
            const seen_request_t* r = &vetted.requests[i];
            log("    request %d: kind %u %s dest '%s' redirect %d probe '%s' %s\n", i, r->kind,
                r->method, r->destination, r->is_redirect, r->probe, r->url);
        }
    }

    servo_webview_free(bare_webview);
    servo_webview_free(plain_webview);
    servo_webview_free(webview);
    servo_free(servo);
    log("%s (%d failed)\n", failed == 0 ? "PASS" : "FAIL", failed);
    return failed;
}
