/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Tests for the `request_permission` delegate: a permission requested by page
// script reaches the callback with its feature and both origins, and the answer
// given through `servo_webview_respond_permission`, during the callback or after
// it has returned, decides what the page sees.
//
// The page reports its outcome by setting `document.title`, which arrives
// through `notify_page_title_changed`: "state:" followed by the permission
// state, or "rej:" followed by the rejection's name.
//
// Permissions are denied outright in a non-secure context, before the embedder
// is asked, so the page is a `file:` URL rather than a `data:` one.

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

// How `on_request_permission` answers.
typedef enum {
    ANSWER_ALLOW = 0, // allow during the callback
    ANSWER_DENY, // deny during the callback
    ANSWER_LATER, // keep the id and answer after the callback has returned
} answer_t;

typedef struct {
    answer_t answer;
    bool later_allow;

    // What `on_request_permission` last saw.
    int requests_seen;
    uint64_t id;
    uint32_t feature;
    char requester_origin[128];
    char top_level_origin[128];
    uint64_t requester_browsing_context_id;
    uint64_t top_level_browsing_context_id;
    int webview_was_null;
    int user_data_matched;
    int inline_answer_delivered;

    char title[256];
    int load_complete;
} test_state_t;

static test_state_t state;

static void noop_wake_callback(void) { }

static void copy_string(char* dest, size_t dest_size, const uint8_t* ptr, size_t len)
{
    size_t n = len < dest_size - 1 ? len : dest_size - 1;
    if (ptr != NULL && n > 0) {
        memcpy(dest, ptr, n);
    }
    dest[n] = '\0';
}

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
    copy_string(s->title, sizeof(s->title), title_ptr, title_ptr == NULL ? 0 : title_len);
}

static void on_request_permission(struct WebView* webview,
    const struct ServoPermissionRequest* request, void* user_data)
{
    test_state_t* s = (test_state_t*)user_data;
    s->requests_seen++;
    s->user_data_matched = (s == &state);
    s->webview_was_null = (webview == NULL);
    s->id = request->id;
    s->feature = request->feature;
    copy_string(s->requester_origin, sizeof(s->requester_origin),
        request->requester_origin_ptr, request->requester_origin_len);
    copy_string(s->top_level_origin, sizeof(s->top_level_origin),
        request->top_level_origin_ptr, request->top_level_origin_len);
    s->requester_browsing_context_id = request->requester_browsing_context_id;
    s->top_level_browsing_context_id = request->top_level_browsing_context_id;

    switch (s->answer) {
    case ANSWER_ALLOW:
        s->inline_answer_delivered = servo_webview_respond_permission(request->id, true);
        break;
    case ANSWER_DENY:
        s->inline_answer_delivered = servo_webview_respond_permission(request->id, false);
        break;
    case ANSWER_LATER:
        break;
    }
}

static struct Servo* build_servo(void)
{
    ServoBuilder* builder = servo_builder_create();
    struct ServoEventLoopWaker waker = { noop_wake_callback };
    servo_builder_set_event_loop_waker(builder, waker);

    ServoOptions* options = servo_options_create();
    servo_options_set_multiprocess(options, false);
    servo_builder_set_options(builder, options);

    ServoPreferences* preferences = servo_preferences_create();
    servo_preferences_set_bool(preferences, "dom_permissions_enabled", true);
    servo_builder_set_preferences(builder, preferences);

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
        .notify_page_title_changed = on_page_title_changed,
        .request_permission = on_request_permission,
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

static const char PAGE[] = "<!doctype html><html><head><title>start</title><script>"
                           "navigator.permissions.request({name:'geolocation'}).then("
                           "s=>{document.title='state:'+s.state},"
                           "e=>{document.title='rej:'+e.name});"
                           "</script></head></html>";

// The same request, made from a `srcdoc` frame. A `srcdoc` document shares its parent's
// origin, so origins cannot tell the two documents apart; browsing context ids must.
static const char FRAME_PAGE[]
    = "<!doctype html><html><head><title>start</title></head><body>"
      "<iframe srcdoc=\"<script>"
      "navigator.permissions.request({name:'geolocation'}).then("
      "s=>{parent.document.title='state:'+s.state},"
      "e=>{parent.document.title='rej:'+e.name});"
      "</script>\"></iframe></body></html>";

// Writes `contents` to the temporary directory and fills `url` with its `file:` URL.
static int write_page_named(const char* contents, const char* name, char* url, size_t url_size)
{
    char path[512];
#ifdef _WIN32
    char dir[MAX_PATH];
    DWORD len = GetTempPathA(sizeof(dir), dir);
    if (len == 0 || len >= sizeof(dir)) {
        return 1;
    }
    snprintf(path, sizeof(path), "%sservo_capi_%s_%lu.html", dir, name,
        (unsigned long)GetCurrentProcessId());
#else
    const char* dir = getenv("TMPDIR");
    snprintf(path, sizeof(path), "%s/servo_capi_%s.html", dir != NULL ? dir : "/tmp", name);
#endif
    FILE* file = fopen(path, "wb");
    if (file == NULL) {
        return 1;
    }
    fwrite(contents, 1, strlen(contents), file);
    fclose(file);

    for (char* c = path; *c != '\0'; c++) {
        if (*c == '\\') {
            *c = '/';
        }
    }
    snprintf(url, url_size, "file://%s%s", path[0] == '/' ? "" : "/", path);
    return 0;
}

static int write_page(char* url, size_t url_size)
{
    return write_page_named(PAGE, "permission", url, url_size);
}

// Load `url` and spin until the title is a final "state:" or "rej:" report, for
// at most 20 seconds, answering a kept request after `answer_after_ms` when the
// answer is deferred. Returns 0 if a report arrived.
static int load_and_wait(struct Servo* servo, struct WebView* webview, const char* url,
    int answer_after_ms)
{
    state.title[0] = '\0';
    state.requests_seen = 0;
    state.id = 0;
    state.inline_answer_delivered = 0;
    if (servo_webview_load(webview, url) != 0) {
        log("  FAIL: servo_webview_load failed for '%s'\n", url);
        return 1;
    }
    int answered_at_ms = -1;
    for (int elapsed_ms = 0; elapsed_ms < 20000; elapsed_ms += 10) {
        servo_spin_event_loop(servo);
        if (state.answer == ANSWER_LATER && state.requests_seen > 0 && answered_at_ms < 0
            && elapsed_ms >= answer_after_ms) {
            if (strncmp(state.title, "state:", 6) == 0) {
                log("  FAIL: the page settled before the deferred answer\n");
                return 1;
            }
            if (!servo_webview_respond_permission(state.id, state.later_allow)) {
                log("  FAIL: the deferred answer was not delivered\n");
                return 1;
            }
            answered_at_ms = elapsed_ms;
        }
        if (strncmp(state.title, "state:", 6) == 0 || strncmp(state.title, "rej:", 4) == 0) {
            return 0;
        }
        sleep_ms(10);
    }
    log("  FAIL: timed out; last title '%s', requests seen %d\n", state.title,
        state.requests_seen);
    return 1;
}

static int expect_title(const char* name, const char* expected)
{
    if (strcmp(state.title, expected) != 0) {
        log("  FAIL %s: expected title '%s', got '%s'\n", name, expected, state.title);
        return 1;
    }
    log("  PASS %s\n", name);
    return 0;
}

// Q1
static int test_request_reaches_the_callback(struct Servo* servo, struct WebView* webview,
    const char* url)
{
    state.answer = ANSWER_ALLOW;
    if (load_and_wait(servo, webview, url, 0) != 0) {
        return 1;
    }
    if (state.requests_seen != 1 || state.id == 0
        || state.feature != SERVO_PERMISSION_GEOLOCATION) {
        log("  FAIL request_reaches_the_callback: seen=%d id=%llu feature=%u\n",
            state.requests_seen, (unsigned long long)state.id, state.feature);
        return 1;
    }
    if (state.webview_was_null || !state.user_data_matched) {
        log("  FAIL request_reaches_the_callback: webview_null=%d user_data=%d\n",
            state.webview_was_null, state.user_data_matched);
        return 1;
    }
    // A top-level document asked, so the two origins agree.
    if (state.requester_origin[0] == '\0'
        || strcmp(state.requester_origin, state.top_level_origin) != 0) {
        log("  FAIL request_reaches_the_callback: requester '%s', top level '%s'\n",
            state.requester_origin, state.top_level_origin);
        return 1;
    }
    // ...and so do the two frame ids, which are never the "unknown" 0.
    if (state.requester_browsing_context_id == 0
        || state.requester_browsing_context_id != state.top_level_browsing_context_id) {
        log("  FAIL request_reaches_the_callback: requester frame %llu, top-level frame %llu\n",
            (unsigned long long)state.requester_browsing_context_id,
            (unsigned long long)state.top_level_browsing_context_id);
        return 1;
    }
    log("  PASS request_reaches_the_callback\n");
    return 0;
}

// Q5: a nested frame asking is told apart from the top level by frame id, even though
// a `srcdoc` frame's origin equals its parent's.
static int test_frame_request_carries_its_own_frame(struct Servo* servo,
    struct WebView* webview, const char* frame_url)
{
    state.answer = ANSWER_ALLOW;
    if (load_and_wait(servo, webview, frame_url, 0) != 0) {
        return 1;
    }
    if (state.requests_seen != 1) {
        log("  FAIL frame_request_carries_its_own_frame: seen=%d\n", state.requests_seen);
        return 1;
    }
    if (strcmp(state.requester_origin, state.top_level_origin) != 0) {
        log("  FAIL frame_request_carries_its_own_frame: expected equal origins, got '%s' and "
            "'%s'\n",
            state.requester_origin, state.top_level_origin);
        return 1;
    }
    if (state.requester_browsing_context_id == 0 || state.top_level_browsing_context_id == 0
        || state.requester_browsing_context_id == state.top_level_browsing_context_id) {
        log("  FAIL frame_request_carries_its_own_frame: requester frame %llu, top-level "
            "frame %llu\n",
            (unsigned long long)state.requester_browsing_context_id,
            (unsigned long long)state.top_level_browsing_context_id);
        return 1;
    }
    log("  PASS frame_request_carries_its_own_frame\n");
    return 0;
}

// Q2
static int test_inline_answers(struct Servo* servo, struct WebView* webview, const char* url)
{
    state.answer = ANSWER_ALLOW;
    if (load_and_wait(servo, webview, url, 0) != 0
        || expect_title("inline_answers (allow)", "state:granted") != 0) {
        return 1;
    }
    if (!state.inline_answer_delivered) {
        log("  FAIL inline_answers: the answer given during the callback was not delivered\n");
        return 1;
    }

    state.answer = ANSWER_DENY;
    if (load_and_wait(servo, webview, url, 0) != 0) {
        return 1;
    }
    return expect_title("inline_answers (deny)", "state:denied");
}

// Q3
static int test_deferred_answer(struct Servo* servo, struct WebView* webview, const char* url)
{
    state.answer = ANSWER_LATER;
    state.later_allow = true;
    if (load_and_wait(servo, webview, url, 300) != 0) {
        return 1;
    }
    return expect_title("deferred_answer", "state:granted");
}

// Q4
static int test_stale_ids_are_refused(void)
{
    // The id from Q3 has been answered; answering it again must send nothing.
    if (servo_webview_respond_permission(state.id, true)) {
        log("  FAIL stale_ids_are_refused: an answered id was accepted again\n");
        return 1;
    }
    if (servo_webview_respond_permission(0, true)) {
        log("  FAIL stale_ids_are_refused: id 0 was accepted\n");
        return 1;
    }
    log("  PASS stale_ids_are_refused\n");
    return 0;
}

int run_c_permission_tests(void)
{
    log("running permission tests\n");
    memset(&state, 0, sizeof(state));

    char url[640];
    char frame_url[640];
    if (write_page(url, sizeof(url)) != 0
        || write_page_named(FRAME_PAGE, "permission_frame", frame_url, sizeof(frame_url)) != 0) {
        log("  FAIL: could not write the test pages\n");
        return 1;
    }

    struct Servo* servo = build_servo();
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
    failed += test_request_reaches_the_callback(servo, webview, url);
    failed += test_inline_answers(servo, webview, url);
    failed += test_deferred_answer(servo, webview, url);
    failed += test_stale_ids_are_refused();
    failed += test_frame_request_carries_its_own_frame(servo, webview, frame_url);

    servo_webview_free(webview);
    servo_free(servo);
    log("%s (%d failed)\n", failed == 0 ? "PASS" : "FAIL", failed);
    return failed;
}
