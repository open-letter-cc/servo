// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

// Tests for the page-request embedder control: bytes passed to
// `EmbedderRequest.send()` in page script reach `on_control`, and the bytes
// answered through `servo_webview_send_response` resolve the page's promise.
//
// Each page reports its outcome by setting `document.title`, which arrives
// through `notify_page_title_changed`: "ok:" followed by the resolved bytes as
// comma-separated decimals, or "rej:" followed by the rejection's name.

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

// How `on_control` answers a page request.
typedef enum {
    ANSWER_RESPOND = 0, // answer with `response`/`response_len`
    ANSWER_DECLINE, // return false without answering
    ANSWER_DROP, // return true without answering
} answer_t;

typedef struct {
    answer_t answer;
    const uint8_t* response;
    size_t response_len;

    // What `on_control` last saw.
    int controls_seen;
    uint32_t tag;
    uint8_t payload[64];
    size_t payload_len;
    size_t origin_len;
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

static void on_page_title_changed(struct WebView* webview, const uint8_t* title_ptr,
    size_t title_len, void* user_data)
{
    (void)webview;
    test_state_t* s = (test_state_t*)user_data;
    if (title_ptr == NULL) {
        s->title[0] = '\0';
        return;
    }
    size_t len = title_len < sizeof(s->title) - 1 ? title_len : sizeof(s->title) - 1;
    memcpy(s->title, title_ptr, len);
    s->title[len] = '\0';
}

static bool on_control(struct WebView* webview, const struct ServoEmbedderControl* control,
    void* user_data)
{
    test_state_t* s = (test_state_t*)user_data;
    s->controls_seen++;
    s->user_data_matched = (s == &state);
    s->webview_was_null = (webview == NULL);
    s->tag = control->tag;
    s->origin_len = control->origin_len;
    s->payload_len = control->payload_len;
    if (control->payload_len <= sizeof(s->payload) && control->payload_len > 0) {
        memcpy(s->payload, control->payload_ptr, control->payload_len);
    }

    if (control->tag != (uint32_t)SERVO_EMBEDDER_CONTROL_TAG_PAGE_REQUEST) {
        return false;
    }

    switch (s->answer) {
    case ANSWER_RESPOND:
        if (!servo_webview_send_response(control, s->response, s->response_len)) {
            log("  [on_control] servo_webview_send_response returned false\n");
        }
        return true;
    case ANSWER_DECLINE:
        return false;
    case ANSWER_DROP:
        return true;
    }
    return false;
}

static struct ServoEmbedderController controller = {
    SERVO_EMBEDDER_CONTROL_PAGE_REQUEST,
    on_control,
};

static struct Servo* build_servo(bool pref_on)
{
    ServoBuilder* builder = servo_builder_create();
    struct ServoEventLoopWaker waker = { noop_wake_callback };
    servo_builder_set_event_loop_waker(builder, waker);

    ServoOptions* options = servo_options_create();
    servo_options_set_multiprocess(options, false);
    servo_builder_set_options(builder, options);

    if (pref_on) {
        ServoPreferences* preferences = servo_preferences_create();
        servo_preferences_set_bool(preferences, "dom_embedder_request_enabled", true);
        servo_builder_set_preferences(builder, preferences);
    }

    struct Servo* servo = servo_builder_build(builder);
    if (servo != NULL) {
        servo_setup_logging(servo);
    }
    return servo;
}

static struct WebView* build_webview(struct Servo* servo)
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
        .embedder_control = &controller,
        .notify_page_title_changed = on_page_title_changed,
    };
    servo_webview_builder_set_delegate(builder, delegate);
    struct WebView* webview = servo_webview_builder_build(builder);
    if (webview == NULL) {
        return NULL;
    }

    // A load issued before the webview's browsing context exists is dropped, so
    // wait for the initial document to finish loading before driving it.
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

// Load `url` and spin until the title is a final "ok:" or "rej:" report, or a
// fixed title is reached, for at most 20 seconds. Returns 0 if one arrived.
static int load_and_wait(struct Servo* servo, struct WebView* webview, const char* url,
    const char* until)
{
    state.title[0] = '\0';
    state.controls_seen = 0;
    state.payload_len = 0;
    if (servo_webview_load(webview, url) != 0) {
        log("  FAIL: servo_webview_load failed for '%s'\n", url);
        return 1;
    }
    for (int elapsed_ms = 0; elapsed_ms < 20000; elapsed_ms += 10) {
        servo_spin_event_loop(servo);
        if (until != NULL ? strcmp(state.title, until) == 0
                          : strncmp(state.title, "ok:", 3) == 0 || strncmp(state.title, "rej:", 4) == 0) {
            return 0;
        }
        sleep_ms(10);
    }
    log("  FAIL: timed out; last title '%s'\n", state.title);
    return 1;
}

#define SEND_PAGE(bytes)                                                      \
    "data:text/html,<html><head><title>start</title><script>"                 \
    "EmbedderRequest.send(new Uint8Array([" bytes "])).then("                 \
    "r=>{document.title='ok:'+Array.from(r).join(',')},"                      \
    "e=>{document.title='rej:'+e.name});"                                     \
    "</script></head></html>"

static int expect_title(const char* name, const char* expected)
{
    if (strcmp(state.title, expected) != 0) {
        log("  FAIL %s: expected title '%s', got '%s'\n", name, expected, state.title);
        return 1;
    }
    log("  PASS %s\n", name);
    return 0;
}

// P1
static int test_page_bytes_reach_on_control(struct Servo* servo, struct WebView* webview)
{
    static const uint8_t response[] = { 1 };
    state.answer = ANSWER_RESPOND;
    state.response = response;
    state.response_len = sizeof(response);
    if (load_and_wait(servo, webview, SEND_PAGE("1,2,3,0,255"), NULL) != 0) {
        return 1;
    }
    static const uint8_t expected[] = { 1, 2, 3, 0, 255 };
    if (state.controls_seen != 1 || state.tag != (uint32_t)SERVO_EMBEDDER_CONTROL_TAG_PAGE_REQUEST
        || state.payload_len != sizeof(expected)
        || memcmp(state.payload, expected, sizeof(expected)) != 0) {
        log("  FAIL page_bytes_reach_on_control: seen=%d tag=%u len=%zu\n",
            state.controls_seen, state.tag, state.payload_len);
        return 1;
    }
    if (state.webview_was_null || !state.user_data_matched || state.origin_len == 0) {
        log("  FAIL page_bytes_reach_on_control: webview_null=%d user_data=%d origin_len=%zu\n",
            state.webview_was_null, state.user_data_matched, state.origin_len);
        return 1;
    }
    log("  PASS page_bytes_reach_on_control\n");
    return 0;
}

// P2
static int test_response_bytes_resolve_the_promise(struct Servo* servo, struct WebView* webview)
{
    static const uint8_t response[] = { 9, 8, 7 };
    state.answer = ANSWER_RESPOND;
    state.response = response;
    state.response_len = sizeof(response);
    if (load_and_wait(servo, webview, SEND_PAGE("5"), NULL) != 0
        || expect_title("response_bytes_resolve_the_promise", "ok:9,8,7") != 0) {
        return 1;
    }

    state.response = NULL;
    state.response_len = 0;
    if (load_and_wait(servo, webview, SEND_PAGE("5"), NULL) != 0) {
        return 1;
    }
    return expect_title("response_bytes_resolve_the_promise (empty)", "ok:");
}

// P3
static int test_declined_rejects(struct Servo* servo, struct WebView* webview)
{
    state.answer = ANSWER_DECLINE;
    if (load_and_wait(servo, webview, SEND_PAGE("5"), NULL) != 0) {
        return 1;
    }
    return expect_title("declined_rejects", "rej:NotAllowedError");
}

// P4
static int test_dropped_rejects(struct Servo* servo, struct WebView* webview)
{
    state.answer = ANSWER_DROP;
    if (load_and_wait(servo, webview, SEND_PAGE("5"), NULL) != 0
        || expect_title("dropped_rejects (handled, unanswered)", "rej:NotAllowedError") != 0) {
        return 1;
    }

    controller.flags = SERVO_EMBEDDER_CONTROL_NONE;
    state.answer = ANSWER_RESPOND;
    int result = load_and_wait(servo, webview, SEND_PAGE("5"), NULL);
    controller.flags = SERVO_EMBEDDER_CONTROL_PAGE_REQUEST;
    if (result != 0) {
        return 1;
    }
    if (state.controls_seen != 0) {
        log("  FAIL dropped_rejects: on_control saw a control without the tag bit\n");
        return 1;
    }
    return expect_title("dropped_rejects (tag bit not set)", "rej:NotAllowedError");
}

// P9. A hand-assembled module whose exported `run` calls its import `m.f`,
// which is an ordinary JavaScript function that calls `EmbedderRequest.send()`.
static int test_wasm_reaches_it_by_js_import(struct Servo* servo, struct WebView* webview)
{
    static const uint8_t response[] = { 4, 2 };
    state.answer = ANSWER_RESPOND;
    state.response = response;
    state.response_len = sizeof(response);
    const char* url = "data:text/html,<html><head><title>start</title><script>"
                      "const f=()=>{EmbedderRequest.send(new Uint8Array([42])).then("
                      "r=>{document.title='ok:'+Array.from(r).join(',')},"
                      "e=>{document.title='rej:'+e.name})};"
                      "WebAssembly.instantiate(new Uint8Array(["
                      "0,97,115,109,1,0,0,0,"
                      "1,4,1,96,0,0,"
                      "2,7,1,1,109,1,102,0,0,"
                      "3,2,1,0,"
                      "7,7,1,3,114,117,110,0,1,"
                      "10,6,1,4,0,16,0,11"
                      "]),{m:{f}}).then(({instance})=>instance.exports.run(),"
                      "e=>{document.title='rej:wasm '+e})"
                      "</script></head></html>";
    if (load_and_wait(servo, webview, url, NULL) != 0) {
        return 1;
    }
    if (state.payload_len != 1 || state.payload[0] != 42) {
        log("  FAIL wasm_reaches_it_by_js_import: payload_len=%zu\n", state.payload_len);
        return 1;
    }
    return expect_title("wasm_reaches_it_by_js_import", "ok:4,2");
}

int run_c_page_request_tests(void)
{
    log("running page request tests\n");
    memset(&state, 0, sizeof(state));
    struct Servo* servo = build_servo(true);
    if (servo == NULL) {
        log("  FAIL: servo_builder_build returned NULL\n");
        return 1;
    }
    struct WebView* webview = build_webview(servo);
    if (webview == NULL) {
        log("  FAIL: could not build a webview\n");
        servo_free(servo);
        return 1;
    }

    int failed = 0;
    failed += test_page_bytes_reach_on_control(servo, webview);
    failed += test_response_bytes_resolve_the_promise(servo, webview);
    failed += test_declined_rejects(servo, webview);
    failed += test_dropped_rejects(servo, webview);
    failed += test_wasm_reaches_it_by_js_import(servo, webview);

    servo_webview_free(webview);
    servo_free(servo);
    log("%s (%d failed)\n", failed == 0 ? "PASS" : "FAIL", failed);
    return failed;
}

// P5. Run in its own process, since preferences are process-wide.
int run_c_page_request_pref_off_tests(void)
{
    log("running page request pref-off tests\n");
    memset(&state, 0, sizeof(state));
    struct Servo* servo = build_servo(false);
    if (servo == NULL) {
        log("  FAIL: servo_builder_build returned NULL\n");
        return 1;
    }
    struct WebView* webview = build_webview(servo);
    if (webview == NULL) {
        log("  FAIL: could not build a webview\n");
        servo_free(servo);
        return 1;
    }

    const char* url = "data:text/html,<html><head><title>start</title><script>"
                      "document.title='typeof:'+typeof EmbedderRequest;"
                      "</script></head></html>";
    int failed = load_and_wait(servo, webview, url, "typeof:undefined");
    log("  %s pref_off_hides_the_function\n", failed == 0 ? "PASS" : "FAIL");

    servo_webview_free(webview);
    servo_free(servo);
    return failed;
}
