// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

// Tests that a rendering context grown through `servo_webview_resize` is painted
// over its whole new area, read back through `servo_rendering_context_read_frame`.
//
// The page is horizontal bands, 10 pixels each, alternating red and blue, so a row
// that reads back black was not painted, and a band boundary away from a multiple
// of 10 would show content painted at the wrong offset. The context grows from 43
// to 100 rows: an unpainted region the size of the old height (43 rows) and one the
// size of the growth (57 rows) are told apart, which a doubling would not do.

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

#define WIDTH 64
#define OLD_HEIGHT 43
#define NEW_HEIGHT 100
#define BAND 10

typedef struct {
    int load_complete;
    int frames_ready;
} test_state_t;

static test_state_t state;
static uint8_t frame[WIDTH * NEW_HEIGHT * 4];

static void noop_wake_callback(void) { }

static void on_load_status_changed(struct WebView* webview, int32_t status, void* user_data)
{
    (void)webview;
    if (status == SERVO_LOAD_STATUS_COMPLETE) {
        ((test_state_t*)user_data)->load_complete = 1;
    }
}

static void on_new_frame_ready(struct WebView* webview, void* user_data)
{
    (void)webview;
    ((test_state_t*)user_data)->frames_ready++;
}

// One of 'R' (red), 'B' (blue), 'K' (black: unpainted) or '?' (anything else) for a
// pixel, with a tolerance for the antialiased row a band edge can produce.
static char classify(const uint8_t* p)
{
    if (p[0] > 200 && p[1] < 60 && p[2] < 60) {
        return 'R';
    }
    if (p[0] < 60 && p[1] < 60 && p[2] > 200) {
        return 'B';
    }
    if (p[0] < 16 && p[1] < 16 && p[2] < 16) {
        return 'K';
    }
    return '?';
}

// The class of a whole row: its pixels' class if they all agree, '?' otherwise.
static char classify_row(int row)
{
    char first = classify(frame + (size_t)row * WIDTH * 4);
    for (int x = 1; x < WIDTH; x++) {
        if (classify(frame + ((size_t)row * WIDTH + x) * 4) != first) {
            return '?';
        }
    }
    return first;
}

// Logs the rows of a `height`-row frame as runs, e.g. "K57 R10 B10 ...", and returns
// the number of black rows.
static int describe(const char* what, int height)
{
    char line[512];
    size_t used = 0;
    int black = 0;
    int row = 0;
    while (row < height) {
        char c = classify_row(row);
        int run = 1;
        while (row + run < height && classify_row(row + run) == c) {
            run++;
        }
        if (c == 'K') {
            black += run;
        }
        if (used < sizeof(line) - 16) {
            used += (size_t)snprintf(line + used, sizeof(line) - used, "%c%d ", c, run);
        }
        row += run;
    }
    const uint8_t* p = frame;
    log("    %s: %s(first pixel %d,%d,%d,%d)\n", what, line, p[0], p[1], p[2], p[3]);
    return black;
}

// Whether every band in a `height`-row frame starts where the page puts it: rows
// [k*BAND, (k+1)*BAND) red for even k and blue for odd k, allowing one edge row.
static int bands_in_place(int height)
{
    for (int row = 0; row < height; row++) {
        char expected = ((row / BAND) % 2 == 0) ? 'R' : 'B';
        char seen = classify_row(row);
        int at_edge = (row % BAND == 0) || (row % BAND == BAND - 1);
        if (seen != expected && !(at_edge && seen == '?')) {
            return 0;
        }
    }
    return 1;
}

static int spin_until_frame(struct Servo* servo, int timeout_ms)
{
    state.frames_ready = 0;
    for (int elapsed = 0; elapsed < timeout_ms; elapsed += 10) {
        servo_spin_event_loop(servo);
        if (state.frames_ready > 0) {
            return 1;
        }
        sleep_ms(10);
    }
    return 0;
}

// Paints, presents and reads one frame of `height` rows. Returns the read code.
static int paint_present_read(struct WebView* webview, RenderingContext* reader, int height,
    struct ServoFrameInfo* info)
{
    servo_webview_paint(webview);
    servo_rendering_context_present(reader);
    memset(frame, 0x55, sizeof(frame));
    return servo_rendering_context_read_frame(reader, frame,
        (uintptr_t)WIDTH * (uintptr_t)height * 4, info);
}

// The screenshot scenario: a 1014x43 view grown to 1014x690, then read through
// `servo_webview_take_screenshot`, painting until its callback answers.
#define SHOT_WIDTH 1014
#define SHOT_OLD_HEIGHT 43
#define SHOT_NEW_HEIGHT 690

typedef struct {
    int done;
    int error;
    uint32_t width;
    uint32_t height;
} shot_t;

static uint8_t shot_pixels[SHOT_WIDTH * SHOT_NEW_HEIGHT * 4];

static void on_screenshot(const uint8_t* data, uint32_t width, uint32_t height, int32_t error,
    void* user_data)
{
    shot_t* shot = (shot_t*)user_data;
    shot->done = 1;
    shot->error = error;
    shot->width = width;
    shot->height = height;
    if (data != NULL && width == SHOT_WIDTH && height <= SHOT_NEW_HEIGHT) {
        memcpy(shot_pixels, data, (size_t)width * height * 4);
    }
}

// The page's colour, rgb(0,128,255), within a tolerance; black is (0,0,0).
static int is_page_colour(const uint8_t* p)
{
    return p[0] < 16 && p[1] > 112 && p[1] < 144 && p[2] > 239;
}

static const uint8_t* shot_pixel(uint32_t x, uint32_t y)
{
    return shot_pixels + ((size_t)y * SHOT_WIDTH + x) * 4;
}

// Runs the screenshot scenario on `servo` and returns the number of failures.
static int run_screenshot_after_grow(struct Servo* servo)
{
    test_state_t shot_state;
    memset(&shot_state, 0, sizeof(shot_state));
    RenderingContext* context
        = servo_rendering_context_create_software(SHOT_WIDTH, SHOT_OLD_HEIGHT);
    RenderingContext* reader = servo_rendering_context_clone(context);
    ServoWebViewBuilder* builder = servo_webview_builder_create(servo, context);
    ServoWebViewDelegate delegate = {
        .user_data = &shot_state,
        .notify_load_status_changed = on_load_status_changed,
        .notify_new_frame_ready = on_new_frame_ready,
    };
    servo_webview_builder_set_delegate(builder, delegate);
    servo_webview_builder_set_url(builder,
        "data:text/html,<html><body style='margin:0;background:rgb(0,128,255)'></body></html>");
    struct WebView* webview = servo_webview_builder_build(builder);

    for (int elapsed = 0; elapsed < 20000 && !shot_state.load_complete; elapsed += 10) {
        servo_spin_event_loop(servo);
        sleep_ms(10);
    }
    for (int i = 0; i < 30; i++) {
        servo_spin_event_loop(servo);
        servo_webview_paint(webview);
        servo_rendering_context_present(reader);
        sleep_ms(10);
    }

    servo_webview_resize(webview, SHOT_WIDTH, SHOT_NEW_HEIGHT);
    shot_t shot;
    memset(&shot, 0, sizeof(shot));
    memset(shot_pixels, 0x55, sizeof(shot_pixels));
    servo_webview_take_screenshot(webview, on_screenshot, &shot);
    int paints = 0;
    for (int elapsed = 0; elapsed < 20000 && !shot.done; elapsed += 10) {
        servo_spin_event_loop(servo);
        servo_webview_paint(webview);
        servo_rendering_context_present(reader);
        paints++;
        sleep_ms(10);
    }

    int failed = 0;
    if (!shot.done || shot.error != 0 || shot.width != SHOT_WIDTH
        || shot.height != SHOT_NEW_HEIGHT) {
        log("  FAIL screenshot_after_grow: done %d, error %d, %ux%u\n", shot.done, shot.error,
            shot.width, shot.height);
        failed = 1;
    } else {
        // The rows the report sampled, and the boundary between them.
        const uint32_t rows[] = { 0, 20, 42, 43, 44, 100, 345, 646, 647, 688, 689 };
        char line[512];
        size_t used = 0;
        int bad = 0;
        for (size_t i = 0; i < sizeof(rows) / sizeof(rows[0]); i++) {
            const uint8_t* p = shot_pixel(SHOT_WIDTH / 2, rows[i]);
            int ok = is_page_colour(p);
            bad += !ok;
            used += (size_t)snprintf(line + used, sizeof(line) - used, "y%u=%s(%d,%d,%d) ",
                rows[i], ok ? "page" : "OTHER", p[0], p[1], p[2]);
        }
        // Every row, not just the samples, at the left edge and the middle.
        int leading_black = 0;
        while (leading_black < SHOT_NEW_HEIGHT) {
            const uint8_t* p = shot_pixel(SHOT_WIDTH / 2, (uint32_t)leading_black);
            if (!(p[0] < 16 && p[1] < 16 && p[2] < 16)) {
                break;
            }
            leading_black++;
        }
        int page_rows = 0;
        for (uint32_t y = 0; y < SHOT_NEW_HEIGHT; y++) {
            page_rows += is_page_colour(shot_pixel(0, y)) && is_page_colour(shot_pixel(SHOT_WIDTH / 2, y));
        }
        log("    screenshot after %d paint(s): %s\n", paints, line);
        log("    leading black rows %d, rows in the page's colour %d of %d\n", leading_black,
            page_rows, SHOT_NEW_HEIGHT);
        if (bad == 0 && page_rows == SHOT_NEW_HEIGHT) {
            log("  PASS screenshot_after_grow (%dx%d to %dx%d)\n", SHOT_WIDTH, SHOT_OLD_HEIGHT,
                SHOT_WIDTH, SHOT_NEW_HEIGHT);
        } else {
            log("  FAIL screenshot_after_grow\n");
            failed = 1;
        }
    }

    servo_webview_free(webview);
    servo_rendering_context_free(reader);
    return failed;
}

int run_c_resize_tests(void)
{
    log("running resize tests\n");
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

    RenderingContext* context = servo_rendering_context_create_software(WIDTH, OLD_HEIGHT);
    RenderingContext* reader = servo_rendering_context_clone(context);
    ServoWebViewBuilder* builder = servo_webview_builder_create(servo, context);
    ServoWebViewDelegate delegate = {
        .user_data = &state,
        .notify_load_status_changed = on_load_status_changed,
        .notify_new_frame_ready = on_new_frame_ready,
    };
    servo_webview_builder_set_delegate(builder, delegate);
    servo_webview_builder_set_url(builder,
        "data:text/html,<html><body style='margin:0;height:400px;background:"
        "repeating-linear-gradient(to bottom,rgb(255,0,0) 0px,rgb(255,0,0) 10px,"
        "rgb(0,0,255) 10px,rgb(0,0,255) 20px)'></body></html>");
    struct WebView* webview = servo_webview_builder_build(builder);

    int failed = 0;
    struct ServoFrameInfo info;

    // Before the grow: wait for the page, then present until the bands are in place.
    for (int elapsed = 0; elapsed < 20000 && !state.load_complete; elapsed += 10) {
        servo_spin_event_loop(servo);
        sleep_ms(10);
    }
    int ready = 0;
    for (int attempt = 0; attempt < 200 && !ready; attempt++) {
        servo_spin_event_loop(servo);
        if (paint_present_read(webview, reader, OLD_HEIGHT, &info) == SERVO_READ_FRAME_OK
            && info.height == OLD_HEIGHT && bands_in_place(OLD_HEIGHT)) {
            ready = 1;
        }
        sleep_ms(10);
    }
    if (!ready) {
        describe("before grow", OLD_HEIGHT);
        log("  FAIL: the page never rendered at %dx%d\n", WIDTH, OLD_HEIGHT);
        failed = 1;
    } else {
        log("  PASS painted_before_grow (%dx%d)\n", WIDTH, OLD_HEIGHT);
    }

    // Grow, then read the first frames painted at the new size.
    if (!failed) {
        servo_webview_resize(webview, WIDTH, NEW_HEIGHT);
        int reported = spin_until_frame(servo, 5000);
        int first_black = -1;
        int healed_after = -1;
        for (int n = 1; n <= 8; n++) {
            int code = paint_present_read(webview, reader, NEW_HEIGHT, &info);
            if (code != SERVO_READ_FRAME_OK || info.height != NEW_HEIGHT) {
                log("  FAIL: read after grow returned %d at %ux%u\n", code, info.width,
                    info.height);
                failed = 1;
                break;
            }
            char what[32];
            snprintf(what, sizeof(what), "frame %d after grow", n);
            int black = describe(what, NEW_HEIGHT);
            if (n == 1) {
                first_black = black;
            }
            if (black == 0 && bands_in_place(NEW_HEIGHT)) {
                healed_after = n;
                break;
            }
            servo_spin_event_loop(servo);
            sleep_ms(20);
        }
        log("    (a frame was %sreported after the grow)\n", reported ? "" : "NOT ");
        if (!failed && first_black == 0 && healed_after == 1) {
            log("  PASS grown_area_painted_on_first_frame\n");
        } else if (!failed) {
            log("  FAIL grown_area_painted_on_first_frame: %d black rows on the first frame "
                "(old height %d, growth %d); fully painted after %d frame(s)\n",
                first_black, OLD_HEIGHT, NEW_HEIGHT - OLD_HEIGHT, healed_after);
            failed = 1;
        }
    }

    servo_webview_free(webview);
    servo_rendering_context_free(reader);

    failed += run_screenshot_after_grow(servo);

    servo_free(servo);
    log("%s (%d failed)\n", failed == 0 ? "PASS" : "FAIL", failed);
    return failed;
}
