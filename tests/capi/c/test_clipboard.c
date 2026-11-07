// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

// Tests for the `request_clipboard_access` delegate: a copy or paste gesture
// against the OS clipboard asks the embedder first, synchronously, before
// Servo's own default clipboard behaviour runs.
//
// Driven through `navigator.clipboard.writeText`/`readText` rather than
// `document.execCommand('copy'/'paste')`: execCommand's copy/cut/paste command
// names exist for feature-detection purposes but have no execution step wired
// to the engine's actual clipboard-action handler in this engine, so they
// never reach the gate at all - only a real input gesture (native key event or
// context menu action, neither reachable from this capi surface without a
// native window) or the async Clipboard API does.
//
// What the gate's answer changes is still not observable from page script:
// `writeText`'s promise resolves unconditionally once the DOM-level steps run,
// regardless of what the embedder decides about the underlying OS write. So
// these tests assert what crosses the ABI - that the callback fires once per
// gesture, with the right webview and user_data - rather than reading the OS
// clipboard back.

#define _POSIX_C_SOURCE 200809L // required by `nanosleep` in `<time.h>`
#include "servo/servo_capi.h"
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

typedef struct {
    bool answer; // what `on_request_clipboard_access` returns
    int requests_seen;
    int webview_was_null;
    int user_data_matched;

    char title[256];
    int load_complete;
} test_state_t;

static test_state_t state;

static void noop_wake_callback(void) { }

static void on_load_status_changed(struct WebView* webview, int32_t load_status,
    void* user_data)
{
    (void)webview;
    if (load_status == SERVO_LOAD_STATUS_COMPLETE) {
        ((test_state_t*)user_data)->load_complete = 1;
    }
}

static void copy_string(char* dest, size_t dest_size, const uint8_t* ptr, size_t len)
{
    size_t n = len < dest_size - 1 ? len : dest_size - 1;
    if (ptr != NULL && n > 0) {
        memcpy(dest, ptr, n);
    }
    dest[n] = '\0';
}

static void on_page_title_changed(struct WebView* webview, const uint8_t* title_ptr,
    size_t title_len, void* user_data)
{
    (void)webview;
    test_state_t* s = (test_state_t*)user_data;
    copy_string(s->title, sizeof(s->title), title_ptr, title_ptr == NULL ? 0 : title_len);
}

static bool on_request_clipboard_access(struct WebView* webview, void* user_data)
{
    test_state_t* s = (test_state_t*)user_data;
    s->requests_seen++;
    s->webview_was_null = (webview == NULL);
    s->user_data_matched = (s == &state);
    return s->answer;
}

static struct Servo* build_servo(void)
{
    ServoBuilder* builder = servo_builder_create();
    struct ServoEventLoopWaker waker = { noop_wake_callback };
    servo_builder_set_event_loop_waker(builder, waker);

    ServoOptions* options = servo_options_create();
    servo_options_set_multiprocess(options, false);
    servo_builder_set_options(builder, options);

    // The async Clipboard API is off by default; this test drives copy and
    // paste through it.
    ServoPreferences* preferences = servo_preferences_create();
    servo_preferences_set_bool(preferences, "dom_async_clipboard_enabled", true);
    servo_builder_set_preferences(builder, preferences);

    struct Servo* servo = servo_builder_build(builder);
    if (servo != NULL) {
        servo_setup_logging(servo);
    }
    return servo;
}

// Builds a `WebView` whose delegate sets `request_clipboard_access` only when
// `gate_the_clipboard` is true, so the NULL-callback default path (clipboard
// access allowed without ever calling out) can be exercised too.
static struct WebView* build_webview(struct Servo* servo, bool gate_the_clipboard)
{
    RenderingContext* context = servo_rendering_context_create_software(64, 64);
    if (context == NULL) {
        return NULL;
    }
    ServoWebViewBuilder* builder = servo_webview_builder_create(servo, context);
    if (builder == NULL) {
        return NULL;
    }
    ServoWebViewDelegate delegate = {
        .user_data = &state,
        .notify_load_status_changed = on_load_status_changed,
        .notify_page_title_changed = on_page_title_changed,
        .request_clipboard_access = gate_the_clipboard ? on_request_clipboard_access : NULL,
    };
    servo_webview_builder_set_delegate(builder, delegate);
    struct WebView* webview = servo_webview_builder_build(builder);
    if (webview == NULL) {
        return NULL;
    }

    state.load_complete = 0;
    for (int elapsed_ms = 0; elapsed_ms < 20000 && !state.load_complete; elapsed_ms += 10) {
        servo_spin_event_loop(servo);
        sleep_ms(10);
    }
    if (!state.load_complete) {
        log("  FAIL: the initial document never finished loading\n");
        servo_webview_free(webview);
        return NULL;
    }
    return webview;
}

// Writes a string to the clipboard, then reads it back, on load - one copy
// gesture (`set_text`) and one paste gesture (`get_text`), each going through
// the gate once. The final title only reports that both promises settled, not
// what the gate decided; see the note above.
static const char PAGE[] = "<!doctype html><html><head><title>start</title><script>"
                            "navigator.clipboard.writeText('clip-gate-test').then("
                            "  function () { return navigator.clipboard.readText(); },"
                            "  function (e) { document.title = 'error:write:' + e; throw e; }"
                            ").then("
                            "  function () { document.title = 'copy:done'; },"
                            "  function (e) { document.title = 'error:read:' + e; }"
                            ");"
                            "</script></head></html>";

static int write_page(char* url, size_t url_size)
{
    char path[512];
#ifdef _WIN32
    char dir[MAX_PATH];
    DWORD len = GetTempPathA(sizeof(dir), dir);
    if (len == 0 || len >= sizeof(dir)) {
        return 1;
    }
    snprintf(path, sizeof(path), "%sservo_capi_clipboard_%lu.html", dir,
        (unsigned long)GetCurrentProcessId());
#else
    const char* dir = getenv("TMPDIR");
    snprintf(path, sizeof(path), "%s/servo_capi_clipboard.html", dir != NULL ? dir : "/tmp");
#endif
    FILE* file = fopen(path, "wb");
    if (file == NULL) {
        return 1;
    }
    fwrite(PAGE, 1, sizeof(PAGE) - 1, file);
    fclose(file);

    for (char* c = path; *c != '\0'; c++) {
        if (*c == '\\') {
            *c = '/';
        }
    }
    snprintf(url, url_size, "file://%s%s", path[0] == '/' ? "" : "/", path);
    return 0;
}

static int load_and_wait_for_copy_report(struct Servo* servo, struct WebView* webview,
    const char* url)
{
    state.title[0] = '\0';
    state.requests_seen = 0;
    if (servo_webview_load(webview, url) != 0) {
        log("  FAIL: servo_webview_load failed for '%s'\n", url);
        return 1;
    }
    for (int elapsed_ms = 0; elapsed_ms < 20000; elapsed_ms += 10) {
        servo_spin_event_loop(servo);
        if (strncmp(state.title, "copy:", 5) == 0 || strncmp(state.title, "error:", 6) == 0) {
            return 0;
        }
        sleep_ms(10);
    }
    log("  FAIL: timed out waiting for the copy report; last title '%s'\n", state.title);
    return 1;
}

// Q1: the gate fires once per copy or paste gesture - twice per page load here,
// `writeText` then `readText` - with the webview and user_data the embedder
// expects, whichever way it answers.
static int test_gate_reaches_the_callback(struct Servo* servo, const char* url)
{
    int failed = 0;
    for (int trial = 0; trial < 2 && failed == 0; trial++) {
        state.answer = (trial == 0);
        struct WebView* webview = build_webview(servo, true);
        if (webview == NULL) {
            log("  FAIL gate_reaches_the_callback: could not build a webview\n");
            return 1;
        }
        if (load_and_wait_for_copy_report(servo, webview, url) != 0) {
            failed = 1;
        } else if (state.requests_seen != 2) {
            log("  FAIL gate_reaches_the_callback: requests_seen=%d (answer=%d)\n",
                state.requests_seen, state.answer);
            failed = 1;
        } else if (state.webview_was_null || !state.user_data_matched) {
            log("  FAIL gate_reaches_the_callback: webview_null=%d user_data=%d (answer=%d)\n",
                state.webview_was_null, state.user_data_matched, state.answer);
            failed = 1;
        } else if (strcmp(state.title, "copy:done") != 0) {
            // Both promises settle regardless of the gate's answer - only the
            // OS-clipboard access that follows is gated - so this reports the
            // same way either way.
            log("  FAIL gate_reaches_the_callback: title '%s' (answer=%d)\n", state.title,
                state.answer);
            failed = 1;
        }
        servo_webview_free(webview);
    }
    if (failed == 0) {
        log("  PASS gate_reaches_the_callback (allow and deny)\n");
    }
    return failed;
}

// Q2: a `WebView` whose delegate never sets `request_clipboard_access` keeps
// Servo's default, which allows clipboard access without ever calling out.
static int test_null_callback_keeps_the_default(struct Servo* servo, const char* url)
{
    state.answer = false; // irrelevant: on_request_clipboard_access is not installed
    struct WebView* webview = build_webview(servo, false);
    if (webview == NULL) {
        log("  FAIL null_callback_keeps_the_default: could not build a webview\n");
        return 1;
    }
    int failed = 0;
    if (load_and_wait_for_copy_report(servo, webview, url) != 0) {
        failed = 1;
    } else if (state.requests_seen != 0) {
        log("  FAIL null_callback_keeps_the_default: callback fired %d times\n",
            state.requests_seen);
        failed = 1;
    } else if (strcmp(state.title, "copy:done") != 0) {
        log("  FAIL null_callback_keeps_the_default: title '%s'\n", state.title);
        failed = 1;
    }
    servo_webview_free(webview);
    if (failed == 0) {
        log("  PASS null_callback_keeps_the_default\n");
    }
    return failed;
}

int run_c_clipboard_tests(void)
{
    log("running clipboard tests\n");
    memset(&state, 0, sizeof(state));

    char url[640];
    if (write_page(url, sizeof(url)) != 0) {
        log("  FAIL: could not write the test page\n");
        return 1;
    }

    struct Servo* servo = build_servo();
    if (servo == NULL) {
        log("  FAIL: servo_builder_build returned NULL\n");
        return 1;
    }

    int failed = 0;
    failed += test_gate_reaches_the_callback(servo, url);
    failed += test_null_callback_keeps_the_default(servo, url);

    servo_free(servo);
    log("%s (%d failed)\n", failed == 0 ? "PASS" : "FAIL", failed);
    return failed;
}
