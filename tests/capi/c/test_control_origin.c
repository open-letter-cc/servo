// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

// Tests that an embedder control carries the requesting document's own origin and
// frame, not the top-level document's: a cross-origin frame's control must not be
// mistaken for its parent's.
//
// The two documents need distinguishable origins, and every origin a `file:`,
// `data:` or sandboxed page has serializes as "null", so the pages are served over
// HTTP by the Rust side of this test: the top-level document from 127.0.0.1 and its
// frame from localhost, two different tuple origins on one loopback server.
//
// Driven through page requests (`EmbedderRequest.send`) because a select, file
// picker or context menu only opens on a trusted user gesture, which nothing in
// this API can produce. All four tags reach the embedder through the same code, so
// what is checked here holds for each of them.

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

// What one control carried. The page sends one byte, 1 from the top-level
// document and 2 from its frame, which is how the two are told apart here.
typedef struct {
    int seen;
    char origin[128];
    uint64_t requester_browsing_context_id;
    uint64_t top_level_browsing_context_id;
} control_seen_t;

typedef struct {
    control_seen_t top;
    control_seen_t frame;
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
    size_t len = title_ptr == NULL ? 0 : title_len;
    len = len < sizeof(s->title) - 1 ? len : sizeof(s->title) - 1;
    if (len > 0) {
        memcpy(s->title, title_ptr, len);
    }
    s->title[len] = '\0';
}

static bool on_control(struct WebView* webview, const struct ServoEmbedderControl* control,
    void* user_data)
{
    (void)webview;
    test_state_t* s = (test_state_t*)user_data;
    if (control->tag != (uint32_t)SERVO_EMBEDDER_CONTROL_TAG_PAGE_REQUEST
        || control->payload_len != 1) {
        return false;
    }

    control_seen_t* seen = control->payload_ptr[0] == 1 ? &s->top : &s->frame;
    seen->seen++;
    size_t len = control->origin_len < sizeof(seen->origin) - 1 ? control->origin_len
                                                                : sizeof(seen->origin) - 1;
    if (len > 0) {
        memcpy(seen->origin, control->origin_ptr, len);
    }
    seen->origin[len] = '\0';
    seen->requester_browsing_context_id = control->requester_browsing_context_id;
    seen->top_level_browsing_context_id = control->top_level_browsing_context_id;

    servo_webview_send_response(control, NULL, 0);
    return true;
}

static struct ServoEmbedderController controller = {
    SERVO_EMBEDDER_CONTROL_PAGE_REQUEST,
    on_control,
};

static struct Servo* build_servo(void)
{
    ServoBuilder* builder = servo_builder_create();
    struct ServoEventLoopWaker waker = { noop_wake_callback };
    servo_builder_set_event_loop_waker(builder, waker);

    ServoOptions* options = servo_options_create();
    servo_options_set_multiprocess(options, false);
    servo_builder_set_options(builder, options);

    ServoPreferences* preferences = servo_preferences_create();
    servo_preferences_set_bool(preferences, "dom_embedder_request_enabled", true);
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
        .embedder_control = &controller,
        .notify_page_title_changed = on_page_title_changed,
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

static int check_control(const char* name, const control_seen_t* seen,
    const char* expected_origin)
{
    if (seen->seen != 1) {
        log("  FAIL %s: seen %d times\n", name, seen->seen);
        return 1;
    }
    if (strcmp(seen->origin, expected_origin) != 0) {
        log("  FAIL %s: origin '%s', expected '%s'\n", name, seen->origin, expected_origin);
        return 1;
    }
    if (seen->requester_browsing_context_id == 0 || seen->top_level_browsing_context_id == 0) {
        log("  FAIL %s: requester frame %llu, top-level frame %llu\n", name,
            (unsigned long long)seen->requester_browsing_context_id,
            (unsigned long long)seen->top_level_browsing_context_id);
        return 1;
    }
    return 0;
}

// `port` serves `/top` and `/frame`, written by the Rust side of this test.
int run_c_control_origin_tests(uint16_t port)
{
    log("running control origin tests\n");
    memset(&state, 0, sizeof(state));

    char top_url[128];
    char top_origin[64];
    char frame_origin[64];
    snprintf(top_url, sizeof(top_url), "http://127.0.0.1:%u/top", (unsigned)port);
    snprintf(top_origin, sizeof(top_origin), "http://127.0.0.1:%u", (unsigned)port);
    snprintf(frame_origin, sizeof(frame_origin), "http://localhost:%u", (unsigned)port);

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
    state.title[0] = '\0';
    if (servo_webview_load(webview, top_url) != 0) {
        log("  FAIL: servo_webview_load failed for '%s'\n", top_url);
        failed = 1;
    } else {
        for (int elapsed_ms = 0; elapsed_ms < 20000 && strcmp(state.title, "done") != 0;
            elapsed_ms += 10) {
            servo_spin_event_loop(servo);
            sleep_ms(10);
        }
        if (strcmp(state.title, "done") != 0) {
            log("  FAIL: timed out; last title '%s', top seen %d, frame seen %d\n", state.title,
                state.top.seen, state.frame.seen);
            failed = 1;
        }
    }

    if (!failed) {
        failed += check_control("top_level_control", &state.top, top_origin);
        if (state.top.requester_browsing_context_id
            != state.top.top_level_browsing_context_id) {
            log("  FAIL top_level_control: the top-level document's requester frame is not "
                "the top-level frame\n");
            failed++;
        }
        if (!failed) {
            log("  PASS top_level_control\n");
        }

        // The case this exists for: the frame's own origin and frame, not its parent's.
        int frame_failed = check_control("cross_origin_frame_control", &state.frame,
            frame_origin);
        if (!frame_failed
            && (state.frame.requester_browsing_context_id
                    == state.frame.top_level_browsing_context_id
                || state.frame.top_level_browsing_context_id
                    != state.top.top_level_browsing_context_id)) {
            log("  FAIL cross_origin_frame_control: requester frame %llu, top-level frame "
                "%llu, the top-level document's frame %llu\n",
                (unsigned long long)state.frame.requester_browsing_context_id,
                (unsigned long long)state.frame.top_level_browsing_context_id,
                (unsigned long long)state.top.top_level_browsing_context_id);
            frame_failed = 1;
        }
        if (!frame_failed) {
            log("  PASS cross_origin_frame_control\n");
        }
        failed += frame_failed;
    }

    servo_webview_free(webview);
    servo_free(servo);
    log("%s (%d failed)\n", failed == 0 ? "PASS" : "FAIL", failed);
    return failed;
}
