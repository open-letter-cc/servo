// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

// Tests for `servo_rendering_context_read_frame` on the software context.
//
// The point under test is which buffer the read sees. A present publishes a frame;
// a paint since then draws the next one. The read must return the published frame
// until the next present, and never the one being drawn. So the test paints a red
// page, presents, reads red; then loads a blue page, paints it without presenting,
// and reads red still; then presents and reads blue.

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

_Static_assert(sizeof(struct ServoFrameInfo) == 32, "ServoFrameInfo size");

#define WIDTH 32
#define HEIGHT 16
#define FRAME_BYTES (WIDTH * HEIGHT * 4)

typedef struct {
    int load_complete;
    int frames_ready;
} test_state_t;

static test_state_t state;

static void noop_wake_callback(void) { }

static void on_load_status_changed(struct WebView* webview, ServoLoadStatus status,
    void* user_data)
{
    (void)webview;
    test_state_t* s = user_data;
    if (status == SERVO_LOAD_STATUS_COMPLETE) {
        s->load_complete = 1;
    }
}

static void on_new_frame_ready(struct WebView* webview, void* user_data)
{
    (void)webview;
    test_state_t* s = user_data;
    s->frames_ready++;
}


// Spins until the page has loaded and reported a frame, then paints it. Returns 0
// on success.
static int load_and_paint(struct Servo* servo, struct WebView* webview, const char* url)
{
    // A NULL `url` waits for the page the webview was built with.
    state.load_complete = 0;
    state.frames_ready = 0;
    if (url != NULL && servo_webview_load(webview, url) != 0) {
        log("  FAIL: servo_webview_load failed for '%s'\n", url);
        return 1;
    }
    if (url == NULL) {
        url = "(initial page)";
    }
    for (int elapsed_ms = 0; elapsed_ms < 20000; elapsed_ms += 10) {
        servo_spin_event_loop(servo);
        if (state.load_complete && state.frames_ready > 0) {
            break;
        }
        sleep_ms(10);
    }
    if (!state.load_complete || state.frames_ready == 0) {
        log("  FAIL: page '%s' never became ready (complete=%d, frames=%d)\n", url,
            state.load_complete, state.frames_ready);
        return 1;
    }
    // A frame reported before load completion can predate the page's paint; spin a
    // little more so the paint below is of the finished page.
    for (int i = 0; i < 20; i++) {
        servo_spin_event_loop(servo);
        sleep_ms(10);
    }
    servo_webview_paint(webview);
    return 0;
}

// Checks every pixel of `frame` against the colour; returns 0 if they all match.
static int expect_colour(const uint8_t* frame, uint8_t r, uint8_t g, uint8_t b, const char* what)
{
    for (int i = 0; i < WIDTH * HEIGHT; i++) {
        const uint8_t* p = frame + i * 4;
        if (p[0] != r || p[1] != g || p[2] != b || p[3] != 255) {
            log("  FAIL %s: pixel %d is (%d,%d,%d,%d), expected (%d,%d,%d,255)\n", what, i,
                p[0], p[1], p[2], p[3], r, g, b);
            return 1;
        }
    }
    return 0;
}

// Paints, presents and reads until every pixel is the colour, or the attempts run
// out. A page's first frame-ready can predate its layout, so one paint is not
// enough to know the page has rendered; presenting repeatedly is fine because what
// is under test is which buffer a read sees, not how many presents it took.
static int present_until_colour(struct Servo* servo, struct WebView* webview,
    RenderingContext* reader, uint8_t* frame, struct ServoFrameInfo* info, uint8_t r,
    uint8_t g, uint8_t b)
{
    for (int attempt = 0; attempt < 200; attempt++) {
        servo_spin_event_loop(servo);
        servo_webview_paint(webview);
        servo_rendering_context_present(reader);
        memset(frame, 0x55, FRAME_BYTES);
        int code = servo_rendering_context_read_frame(reader, frame, FRAME_BYTES, info);
        if (code != SERVO_READ_FRAME_OK) {
            log("  FAIL present_until_colour: code %d\n", code);
            return 1;
        }
        int matches = 1;
        for (int i = 0; i < WIDTH * HEIGHT; i++) {
            const uint8_t* p = frame + i * 4;
            if (p[0] != r || p[1] != g || p[2] != b) {
                matches = 0;
                break;
            }
        }
        if (matches) {
            return 0;
        }
        sleep_ms(10);
    }
    log("  FAIL present_until_colour: never saw (%d,%d,%d)\n", r, g, b);
    return 1;
}

int run_c_read_frame_tests(void)
{
    log("running read frame tests\n");
    memset(&state, 0, sizeof(state));

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

    RenderingContext* context = servo_rendering_context_create_software(WIDTH, HEIGHT);
    if (context == NULL) {
        log("  FAIL: servo_rendering_context_create_software returned NULL\n");
        servo_free(servo);
        return 1;
    }

    struct ServoFrameInfo info;
    uint8_t frame[FRAME_BYTES];
    int failed = 0;

    // Nothing presented yet: sized, but no frame.
    memset(&info, 0xaa, sizeof(info));
    int code = servo_rendering_context_read_frame(context, NULL, 0, &info);
    if (code != SERVO_READ_FRAME_NO_FRAME || info.width != WIDTH || info.height != HEIGHT
        || info.frame != 0) {
        log("  FAIL no_frame: code %d, %ux%u, frame %llu\n", code, info.width, info.height,
            (unsigned long long)info.frame);
        failed = 1;
    } else {
        log("  PASS no_frame_before_present\n");
    }

    // The builder consumes the handle it is given, so keep a second one for
    // presenting and reading.
    RenderingContext* reader = servo_rendering_context_clone(context);
    ServoWebViewBuilder* builder = servo_webview_builder_create(servo, context);
    ServoWebViewDelegate delegate = {
        .user_data = &state,
        .notify_load_status_changed = on_load_status_changed,
        .notify_new_frame_ready = on_new_frame_ready,
    };
    servo_webview_builder_set_delegate(builder, delegate);
    const char* red = "data:text/html,<html><body style='margin:0;background:red'></body></html>";
    const char* blue = "data:text/html,<html><body style='margin:0;background:blue'></body></html>";
    // The first page goes on the builder: a load issued before the initial page has
    // settled is not what this test is about.
    servo_webview_builder_set_url(builder, red);
    struct WebView* webview = servo_webview_builder_build(builder);
    if (webview == NULL) {
        log("  FAIL: could not build a webview\n");
        servo_free(servo);
        return 1;
    }

    if (!failed && load_and_paint(servo, webview, NULL) != 0) {
        failed = 1;
    }

    // Painted but not presented: still no frame.
    if (!failed) {
        code = servo_rendering_context_read_frame(reader, frame, sizeof(frame), &info);
        if (code != SERVO_READ_FRAME_NO_FRAME || info.frame != 0) {
            log("  FAIL paint_is_not_present: code %d, frame %llu\n", code,
                (unsigned long long)info.frame);
            failed = 1;
        } else {
            log("  PASS paint_is_not_present\n");
        }
    }

    // Present until the frame is red, and check it is sized as described.
    if (!failed) {
        if (present_until_colour(servo, webview, reader, frame, &info, 255, 0, 0) != 0) {
            failed = 1;
        } else if (info.width != WIDTH || info.height != HEIGHT || info.stride != WIDTH * 4
            || info.format != SERVO_PIXEL_FORMAT_RGBA8_PREMULTIPLIED || info.flags != 0
            || info.frame == 0) {
            log("  FAIL red_after_present info: %ux%u stride %u format %u flags %u frame %llu\n",
                info.width, info.height, info.stride, info.format, info.flags,
                (unsigned long long)info.frame);
            failed = 1;
        } else {
            log("  PASS red_after_present (after %llu presents)\n",
                (unsigned long long)info.frame);
        }
    }

    // Sizing and a too-short_buffer buffer leave the buffer alone.
    if (!failed) {
        uint8_t short_buffer[FRAME_BYTES - 1];
        memset(short_buffer, 0x77, sizeof(short_buffer));
        code = servo_rendering_context_read_frame(reader, short_buffer, sizeof(short_buffer), &info);
        int untouched = 1;
        for (size_t i = 0; i < sizeof(short_buffer); i++) {
            if (short_buffer[i] != 0x77) {
                untouched = 0;
                break;
            }
        }
        int sized = servo_rendering_context_read_frame(reader, NULL, 0, &info);
        if (code != SERVO_READ_FRAME_BUFFER_TOO_SMALL || !untouched
            || sized != SERVO_READ_FRAME_BUFFER_TOO_SMALL || info.stride * info.height != FRAME_BYTES) {
            log("  FAIL buffer_too_small: code %d, untouched %d, sized %d, bytes %u\n", code,
                untouched, sized, info.stride * info.height);
            failed = 1;
        } else {
            log("  PASS buffer_too_small\n");
        }
    }

    // Load and paint blue without presenting: the read is still red.
    uint64_t red_presents = info.frame;
    if (!failed && load_and_paint(servo, webview, blue) != 0) {
        failed = 1;
    }
    if (!failed) {
        memset(frame, 0x55, sizeof(frame));
        code = servo_rendering_context_read_frame(reader, frame, sizeof(frame), &info);
        if (code != SERVO_READ_FRAME_OK || info.frame != red_presents) {
            log("  FAIL still_red_until_present: code %d, frame %llu\n", code,
                (unsigned long long)info.frame);
            failed = 1;
        } else if (expect_colour(frame, 255, 0, 0, "still_red_until_present") != 0) {
            failed = 1;
        } else {
            log("  PASS still_red_until_present\n");
        }
    }

    // Present: now blue, and the present count has moved on.
    if (!failed) {
        if (present_until_colour(servo, webview, reader, frame, &info, 0, 0, 255) != 0
            || info.frame <= red_presents) {
            log("  FAIL blue_after_present (frame %llu)\n", (unsigned long long)info.frame);
            failed = 1;
        } else {
            log("  PASS blue_after_present\n");
        }
    }

    servo_webview_free(webview);
    servo_rendering_context_free(reader);
    servo_free(servo);
    return failed;
}
