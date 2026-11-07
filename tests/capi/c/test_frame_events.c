// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

// Tests for `notify_frame_changed`: a page builds a small frame tree and the test
// checks the record an embedder could keep from the events alone.
//
// The page creates, in order:
// - frame A, a `srcdoc` frame that contains a nested frame A1;
// - frame B, a sandboxed `srcdoc` frame, whose origin is opaque ("null");
// - frame C, an empty frame whose window the page uses synchronously, straight after
//   inserting it, to request a permission - the earliest anything can come from a
//   frame, and so the hardest case for "CREATED arrives first".
// Once A has loaded the page removes it, which must report A1 removed before A.

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

#define MAX_EVENTS 128

typedef struct {
    uint8_t kind;
    uint64_t id;
    uint64_t parent;
} frame_event_t;

typedef struct {
    frame_event_t events[MAX_EVENTS];
    int event_count;
    // Set when an event or a permission request names a frame not yet created.
    int out_of_order;
    int requests_seen;
    uint64_t requester_browsing_context_id;

    char title[256];
    int load_complete;
} test_state_t;

static test_state_t state;

static void noop_wake_callback(void) { }

static int created_before(uint64_t id)
{
    for (int i = 0; i < state.event_count; i++) {
        if (state.events[i].kind == SERVO_FRAME_EVENT_CREATED && state.events[i].id == id) {
            return 1;
        }
    }
    return 0;
}

static void on_frame_changed(struct WebView* webview, uint64_t browsing_context_id,
    uint64_t parent_browsing_context_id, uint8_t kind, const uint8_t* url_ptr, size_t url_len,
    void* user_data)
{
    (void)webview;
    (void)url_ptr;
    (void)url_len;
    test_state_t* s = (test_state_t*)user_data;
    if (kind != SERVO_FRAME_EVENT_CREATED && !created_before(browsing_context_id)) {
        log("  [frame] kind %u for frame %llu before its CREATED\n", kind,
            (unsigned long long)browsing_context_id);
        s->out_of_order = 1;
    }
    if (s->event_count < MAX_EVENTS) {
        s->events[s->event_count++]
            = (frame_event_t) { kind, browsing_context_id, parent_browsing_context_id };
    }
}

static void on_request_permission(struct WebView* webview,
    const struct ServoPermissionRequest* request, void* user_data)
{
    (void)webview;
    test_state_t* s = (test_state_t*)user_data;
    s->requests_seen++;
    s->requester_browsing_context_id = request->requester_browsing_context_id;
    if (!created_before(request->requester_browsing_context_id)) {
        log("  [permission] request from frame %llu before its CREATED\n",
            (unsigned long long)request->requester_browsing_context_id);
        s->out_of_order = 1;
    }
    servo_webview_respond_permission(request->id, false);
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
    size_t len = title_ptr == NULL ? 0 : title_len;
    len = len < sizeof(s->title) - 1 ? len : sizeof(s->title) - 1;
    if (len > 0) {
        memcpy(s->title, title_ptr, len);
    }
    s->title[len] = '\0';
}

static const char PAGE[]
    = "<!doctype html><html><head><title>start</title></head><body><script>"
      "const a = document.createElement('iframe');"
      "a.srcdoc = '<iframe srcdoc=\"inner\"></iframe>';"
      "a.onload = () => setTimeout(() => { a.remove(); document.title = 'removed'; }, 300);"
      "document.body.appendChild(a);"
      "const b = document.createElement('iframe');"
      "b.sandbox = '';"
      "b.srcdoc = 'opaque';"
      "document.body.appendChild(b);"
      "const c = document.createElement('iframe');"
      "document.body.appendChild(c);"
      "c.contentWindow.navigator.permissions.request({name:'geolocation'}).catch(() => {});"
      "</script></body></html>";

static int write_page(char* url, size_t url_size)
{
    char path[512];
#ifdef _WIN32
    char dir[MAX_PATH];
    DWORD len = GetTempPathA(sizeof(dir), dir);
    if (len == 0 || len >= sizeof(dir)) {
        return 1;
    }
    snprintf(path, sizeof(path), "%sservo_capi_frame_events_%lu.html", dir,
        (unsigned long)GetCurrentProcessId());
#else
    const char* dir = getenv("TMPDIR");
    snprintf(path, sizeof(path), "%s/servo_capi_frame_events.html", dir != NULL ? dir : "/tmp");
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

// The id of the `n`th CREATED event whose parent is `parent`, or 0.
static uint64_t nth_child(uint64_t parent, int n)
{
    for (int i = 0; i < state.event_count; i++) {
        if (state.events[i].kind == SERVO_FRAME_EVENT_CREATED
            && state.events[i].parent == parent && n-- == 0) {
            return state.events[i].id;
        }
    }
    return 0;
}

static int index_of(uint8_t kind, uint64_t id)
{
    for (int i = 0; i < state.event_count; i++) {
        if (state.events[i].kind == kind && state.events[i].id == id) {
            return i;
        }
    }
    return -1;
}

static int count_of(uint8_t kind, uint64_t id)
{
    int count = 0;
    for (int i = 0; i < state.event_count; i++) {
        if (state.events[i].kind == kind && state.events[i].id == id) {
            count++;
        }
    }
    return count;
}

int run_c_frame_events_tests(void)
{
    log("running frame event tests\n");
    memset(&state, 0, sizeof(state));

    char url[640];
    if (write_page(url, sizeof(url)) != 0) {
        log("  FAIL: could not write the test page\n");
        return 1;
    }

    ServoBuilder* servo_builder = servo_builder_create();
    struct ServoEventLoopWaker waker = { noop_wake_callback };
    servo_builder_set_event_loop_waker(servo_builder, waker);
    ServoOptions* options = servo_options_create();
    servo_options_set_multiprocess(options, false);
    servo_builder_set_options(servo_builder, options);
    ServoPreferences* preferences = servo_preferences_create();
    servo_preferences_set_bool(preferences, "dom_permissions_enabled", true);
    servo_builder_set_preferences(servo_builder, preferences);
    struct Servo* servo = servo_builder_build(servo_builder);
    if (servo == NULL) {
        log("  FAIL: servo_builder_build returned NULL\n");
        return 1;
    }

    RenderingContext* context = servo_rendering_context_create_software(64, 64);
    ServoWebViewBuilder* builder = servo_webview_builder_create(servo, context);
    ServoWebViewDelegate delegate = {
        .user_data = &state,
        .notify_load_status_changed = on_load_status_changed,
        .notify_page_title_changed = on_page_title_changed,
        .request_permission = on_request_permission,
        .notify_frame_changed = on_frame_changed,
    };
    servo_webview_builder_set_delegate(builder, delegate);
    servo_webview_builder_set_url(builder, url);
    struct WebView* webview = servo_webview_builder_build(builder);
    if (webview == NULL) {
        log("  FAIL: could not build a webview\n");
        servo_free(servo);
        return 1;
    }

    // Spin until the page has removed frame A, then a little longer for the removals.
    int removed_at_ms = -1;
    for (int elapsed_ms = 0; elapsed_ms < 20000; elapsed_ms += 10) {
        servo_spin_event_loop(servo);
        if (removed_at_ms < 0 && strcmp(state.title, "removed") == 0) {
            removed_at_ms = elapsed_ms;
        }
        if (removed_at_ms >= 0 && elapsed_ms - removed_at_ms > 1000) {
            break;
        }
        sleep_ms(10);
    }

    int failed = 0;
    if (removed_at_ms < 0) {
        log("  FAIL: the page never removed frame A; last title '%s'\n", state.title);
        failed = 1;
    }

    // The top-level frame: one CREATED with no parent.
    uint64_t top = nth_child(0, 0);
    if (!failed && (top == 0 || nth_child(0, 1) != 0)) {
        log("  FAIL tree: expected exactly one top-level frame\n");
        failed = 1;
    }

    // A is the child of the top-level frame that has a child of its own.
    uint64_t a = 0, a1 = 0, others[2] = { 0, 0 };
    int other_count = 0;
    for (int n = 0; !failed && n < 3; n++) {
        uint64_t child = nth_child(top, n);
        if (child == 0) {
            log("  FAIL tree: expected three child frames, found %d\n", n);
            failed = 1;
        } else if (nth_child(child, 0) != 0) {
            a = child;
            a1 = nth_child(child, 0);
        } else if (other_count < 2) {
            others[other_count++] = child;
        }
    }
    if (!failed && (a == 0 || a1 == 0 || other_count != 2)) {
        log("  FAIL tree: could not find the nested frame A1 under A\n");
        failed = 1;
    }
    if (!failed) {
        log("  PASS tree (top %llu, A %llu, A1 %llu)\n", (unsigned long long)top,
            (unsigned long long)a, (unsigned long long)a1);
    }

    // B and C: their own distinct ids, the sandboxed "null"-origin frame included.
    if (!failed
        && (others[0] == others[1] || others[0] == a || others[1] == a || others[0] == top
            || others[1] == top)) {
        log("  FAIL distinct_ids: frames share an id\n");
        failed = 1;
    } else if (!failed) {
        log("  PASS distinct_ids (including the opaque-origin frame)\n");
    }

    // C asked for a permission synchronously from the parent's script.
    if (!failed && (state.requests_seen != 1 || !created_before(state.requester_browsing_context_id)
            || (state.requester_browsing_context_id != others[0]
                && state.requester_browsing_context_id != others[1]))) {
        log("  FAIL created_before_request: seen %d, requester frame %llu\n",
            state.requests_seen, (unsigned long long)state.requester_browsing_context_id);
        failed = 1;
    } else if (!failed) {
        log("  PASS created_before_request\n");
    }

    if (!failed && state.out_of_order) {
        log("  FAIL created_first: an event or request named a frame before its CREATED\n");
        failed = 1;
    } else if (!failed) {
        log("  PASS created_first\n");
    }

    // A's own document became active: at least one NAVIGATED for A.
    if (!failed && count_of(SERVO_FRAME_EVENT_NAVIGATED, a) == 0) {
        log("  FAIL navigated: no NAVIGATED for frame A\n");
        failed = 1;
    } else if (!failed) {
        log("  PASS navigated\n");
    }

    // Removing A reports A1 first, then A, each once; nothing else is removed.
    int removed_a1 = index_of(SERVO_FRAME_EVENT_REMOVED, a1);
    int removed_a = index_of(SERVO_FRAME_EVENT_REMOVED, a);
    if (!failed
        && (removed_a1 < 0 || removed_a < 0 || removed_a1 > removed_a
            || count_of(SERVO_FRAME_EVENT_REMOVED, a) != 1
            || count_of(SERVO_FRAME_EVENT_REMOVED, a1) != 1
            || count_of(SERVO_FRAME_EVENT_REMOVED, top) != 0)) {
        log("  FAIL removed_children_first: A1 at %d, A at %d\n", removed_a1, removed_a);
        failed = 1;
    } else if (!failed) {
        log("  PASS removed_children_first\n");
    }

    if (failed) {
        for (int i = 0; i < state.event_count; i++) {
            log("    event %d: kind %u frame %llu parent %llu\n", i, state.events[i].kind,
                (unsigned long long)state.events[i].id,
                (unsigned long long)state.events[i].parent);
        }
    }

    servo_webview_free(webview);
    servo_free(servo);
    log("%s (%d failed)\n", failed == 0 ? "PASS" : "FAIL", failed);
    return failed;
}
