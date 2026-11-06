/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Content-process containment: with multiprocess on, a content process stuck
// in an infinite loop must not block the process that embeds Servo, nor any
// other webview it hosts, and freeing the stuck webview must return.
//
// Driven by an infinite loop because nothing in the engine interrupts one.
// Unbounded JS recursion does not work for this: SpiderMonkey stops it at its
// own depth limit long before the native stack overflows, so the page simply
// finishes loading and the content process never fails.
//
// What is checked is responsiveness while the hang is still ongoing: an
// unrelated webview, sharing the embedder's event loop, must load a page to
// completion while the first webview's content process is stuck. Whether the
// stuck webview itself reports anything is not asserted; a hang raises no
// crash notification today.
//
// Also checked: servo_webview_content_process_id names a process other than
// this one once the webview's first document is active.
//
// Then a content process is killed outright, from outside, the way an operating
// system or a job object would. The engine must notice on its own: notify_crashed
// fires for the webview it served, servo_webview_content_process_id stops naming
// the dead process, a reload recovers the webview in a fresh process, and freeing
// every webview and the engine afterwards returns. The last point is what a
// watchdog in the test binary enforces.

#define _POSIX_C_SOURCE 200809L // required by `nanosleep` in `<time.h>`
#include "servo/servo_capi.h"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <windows.h>
#else
#include <signal.h>
#include <time.h>
#include <unistd.h>
#endif

static uint32_t current_process_id(void)
{
#ifdef _WIN32
    return (uint32_t)GetCurrentProcessId();
#else
    return (uint32_t)getpid();
#endif
}

// Kills `pid` without any cooperation from it. Returns 0 on success.
static int terminate_process(uint32_t pid)
{
#ifdef _WIN32
    HANDLE handle = OpenProcess(PROCESS_TERMINATE, FALSE, (DWORD)pid);
    if (handle == NULL) {
        return 1;
    }
    int failed = TerminateProcess(handle, 1) ? 0 : 1;
    CloseHandle(handle);
    return failed;
#else
    return kill((pid_t)pid, SIGKILL) == 0 ? 0 : 1;
#endif
}

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
    int load_complete;
    int crashed;
    char crash_reason[256];
} webview_state_t;

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
        ((webview_state_t*)user_data)->load_complete = 1;
    }
}

static void on_notify_crashed(struct WebView* webview, const uint8_t* reason_ptr,
    size_t reason_len, const uint8_t* backtrace_ptr, size_t backtrace_len, void* user_data)
{
    (void)webview;
    (void)backtrace_ptr;
    (void)backtrace_len;
    webview_state_t* s = (webview_state_t*)user_data;
    s->crashed = 1;
    copy_string(s->crash_reason, sizeof(s->crash_reason), reason_ptr, reason_len);
}

static struct Servo* build_servo(bool multiprocess)
{
    ServoBuilder* builder = servo_builder_create();
    struct ServoEventLoopWaker waker = { noop_wake_callback };
    servo_builder_set_event_loop_waker(builder, waker);

    ServoOptions* options = servo_options_create();
    servo_options_set_multiprocess(options, multiprocess);
    // The default `hard_fail` behaviour is "crash the whole process on a script
    // panic" - built for test runners, per its own doc comment, and exactly the
    // behaviour this test exists to rule out for a real embedder.
    servo_options_set_hard_fail(options, false);
    // Sandboxing is an orthogonal concern to process containment; off by default
    // so a sandboxing failure cannot be mistaken for a containment failure. A CI
    // job that exists to prove a platform's sandbox arms sets
    // SERVO_CAPI_TEST_SANDBOX=1, and then the content process must also survive
    // entering the sandbox for this test to pass.
    const char* sandbox = getenv("SERVO_CAPI_TEST_SANDBOX");
    bool sandboxed = sandbox != NULL && sandbox[0] == '1';
    log(sandboxed ? "  content processes: sandboxed\n" : "  content processes: not sandboxed\n");
    servo_options_set_sandbox(options, sandboxed);
    servo_builder_set_options(builder, options);

    struct Servo* servo = servo_builder_build(builder);
    if (servo != NULL) {
        servo_setup_logging(servo);
    }
    return servo;
}

static struct WebView* build_webview(struct Servo* servo, webview_state_t* state)
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
        .user_data = state,
        .notify_load_status_changed = on_load_status_changed,
        .notify_crashed = on_notify_crashed,
    };
    servo_webview_builder_set_delegate(builder, delegate);
    return servo_webview_builder_build(builder);
}

static int write_page(const char* contents, char* url, size_t url_size, const char* name)
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

// Blocks before its own load ever completes - that is the point. Nothing in
// this script yields, awaits, or returns.
static const char HANG_PAGE[] = "<!doctype html><html><head><title>start</title><script>"
                                 "while (true) { }"
                                 "</script></head></html>";

static const char ORDINARY_PAGE[] = "<!doctype html><html><head><title>ok</title></head></html>";

int run_c_multiprocess_tests(void)
{
    log("running multiprocess containment tests\n");

    char hang_url[640];
    char ordinary_url[640];
    if (write_page(HANG_PAGE, hang_url, sizeof(hang_url), "hang") != 0
        || write_page(ORDINARY_PAGE, ordinary_url, sizeof(ordinary_url), "ordinary") != 0) {
        log("  FAIL: could not write the test pages\n");
        return 1;
    }

    struct Servo* servo = build_servo(true);
    if (servo == NULL) {
        log("  FAIL: servo_builder_build returned NULL\n");
        return 1;
    }

    webview_state_t hang_state = { 0 };
    struct WebView* hang_webview = build_webview(servo, &hang_state);
    if (hang_webview == NULL) {
        log("  FAIL: could not build the webview that will hang\n");
        servo_free(servo);
        return 1;
    }

    // Wait for the initial blank document before driving a real load, same as
    // every other test in this suite.
    for (int elapsed_ms = 0; elapsed_ms < 20000 && !hang_state.load_complete; elapsed_ms += 10) {
        servo_spin_event_loop(servo);
        sleep_ms(10);
    }
    if (!hang_state.load_complete) {
        log("  FAIL: the initial document never finished loading\n");
        servo_webview_free(hang_webview);
        servo_free(servo);
        return 1;
    }

    // The webview's document runs out of process, so it must report a content
    // process, and one that is not this one. An embedder containing a dead content
    // process to the webviews it served depends on this being non-zero.
    uint32_t content_pid = servo_webview_content_process_id(hang_webview);
    if (content_pid == 0 || content_pid == current_process_id()) {
        log("  FAIL: servo_webview_content_process_id returned %u (this process is %u)\n",
            (unsigned)content_pid, (unsigned)current_process_id());
        servo_webview_free(hang_webview);
        servo_free(servo);
        return 1;
    }
    log("  webview's content process: pid %u\n", (unsigned)content_pid);

    hang_state.load_complete = 0;
    if (servo_webview_load(hang_webview, hang_url) != 0) {
        log("  FAIL: servo_webview_load failed for the hang page\n");
        servo_webview_free(hang_webview);
        servo_free(servo);
        return 1;
    }

    // Give the content process a head start into its infinite loop before
    // checking anything else, so the ordinary webview's load below genuinely
    // overlaps the hang rather than racing it.
    log("  letting the content process enter its infinite loop...\n");
    for (int elapsed_ms = 0; elapsed_ms < 2000; elapsed_ms += 10) {
        servo_spin_event_loop(servo);
        sleep_ms(10);
    }
    if (hang_state.load_complete) {
        log("  FAIL: the hang page's load completed - it was not actually hung, so this run "
            "does not test what it means to\n");
        servo_webview_free(hang_webview);
        servo_free(servo);
        return 1;
    }

    // The decisive check, made *while hang_webview is still stuck*: build a
    // second, unrelated webview and load an ordinary page in it, sharing the
    // same host event loop. If the host or its event loop were blocked by the
    // hung content process, this would never complete either.
    log("  loading an unrelated page concurrently, with the hang still ongoing "
        "(timeout 20s)...\n");
    webview_state_t ordinary_state = { 0 };
    struct WebView* ordinary_webview = build_webview(servo, &ordinary_state);
    int host_stayed_responsive = 0;
    if (ordinary_webview == NULL) {
        log("  FAIL: could not build the second webview while the first was hung\n");
    } else if (servo_webview_load(ordinary_webview, ordinary_url) != 0) {
        log("  FAIL: servo_webview_load failed for the ordinary page while the first webview "
            "was hung\n");
    } else {
        for (int elapsed_ms = 0; elapsed_ms < 20000 && !ordinary_state.load_complete;
            elapsed_ms += 10) {
            servo_spin_event_loop(servo);
            sleep_ms(10);
        }
        host_stayed_responsive = ordinary_state.load_complete;
    }

    int failed = 0;
    if (host_stayed_responsive) {
        log("  PASS: an unrelated webview loaded normally while the first webview's content "
            "process was still stuck in its infinite loop - the hang did not freeze the host\n");
    } else {
        log("  FAIL: the host (or its event loop) did not make progress on an unrelated "
            "webview while one content process was hung - containment did not hold\n");
        failed = 1;
    }

    if (hang_state.crashed) {
        log("  additionally observed: notify_crashed fired for the hung webview, reason "
            "'%s'\n",
            hang_state.crash_reason);
    }

    // Content-process death. The ordinary webview is the subject because it is
    // known to be responsive, so a missing notification cannot be blamed on a
    // stuck script thread.
    if (ordinary_webview != NULL && host_stayed_responsive) {
        uint32_t dead_pid = servo_webview_content_process_id(ordinary_webview);
        if (dead_pid == 0 || dead_pid == current_process_id()) {
            log("  FAIL: the ordinary webview reports content process %u\n", (unsigned)dead_pid);
            failed = 1;
        } else if (terminate_process(dead_pid) != 0) {
            log("  FAIL: could not terminate content process %u\n", (unsigned)dead_pid);
            failed = 1;
        } else {
            log("  terminated the ordinary webview's content process (pid %u); waiting for "
                "notify_crashed (timeout 10s)...\n",
                (unsigned)dead_pid);
            ordinary_state.crashed = 0;
            for (int elapsed_ms = 0; elapsed_ms < 10000 && !ordinary_state.crashed;
                elapsed_ms += 10) {
                servo_spin_event_loop(servo);
                sleep_ms(10);
            }
            if (!ordinary_state.crashed) {
                log("  FAIL: notify_crashed did not fire after the content process was "
                    "killed\n");
                failed = 1;
            } else {
                log("  PASS: notify_crashed fired, reason '%s'\n", ordinary_state.crash_reason);
            }

            uint32_t after_pid = servo_webview_content_process_id(ordinary_webview);
            if (after_pid == dead_pid) {
                log("  FAIL: servo_webview_content_process_id still names the dead process "
                    "%u\n",
                    (unsigned)dead_pid);
                failed = 1;
            } else {
                log("  PASS: servo_webview_content_process_id no longer names the dead "
                    "process (now %u)\n",
                    (unsigned)after_pid);
            }

            // Reload is the recovery. It must complete, in a process that is not
            // the dead one.
            ordinary_state.load_complete = 0;
            if (servo_webview_load(ordinary_webview, ordinary_url) != 0) {
                log("  FAIL: servo_webview_load failed after the content process died\n");
                failed = 1;
            } else {
                for (int elapsed_ms = 0; elapsed_ms < 20000 && !ordinary_state.load_complete;
                    elapsed_ms += 10) {
                    servo_spin_event_loop(servo);
                    sleep_ms(10);
                }
                uint32_t reloaded_pid = servo_webview_content_process_id(ordinary_webview);
                if (!ordinary_state.load_complete) {
                    log("  FAIL: the reload after the content process died never "
                        "completed\n");
                    failed = 1;
                } else if (reloaded_pid == 0 || reloaded_pid == dead_pid) {
                    log("  FAIL: after reload the webview reports content process %u "
                        "(dead one was %u)\n",
                        (unsigned)reloaded_pid, (unsigned)dead_pid);
                    failed = 1;
                } else {
                    log("  PASS: reload recovered the webview in content process %u\n",
                        (unsigned)reloaded_pid);
                }
            }
        }
    }

    // Freeing a webview whose content process is permanently busy-looping is
    // itself a containment question: if this call never returns, so does this
    // whole test process, which the test runner's own timeout will surface.
    log("  freeing the hung webview...\n");
    servo_webview_free(hang_webview);
    log("  freed without hanging\n");

    if (ordinary_webview != NULL) {
        servo_webview_free(ordinary_webview);
    }
    // The engine has had a content process die under it. This must return; the
    // test binary's watchdog is what turns a hang here into a failure.
    log("  freeing the engine...\n");
    servo_free(servo);
    log("  engine freed\n");
    log("%s (%d failed)\n", failed == 0 ? "PASS" : "FAIL", failed);
    return failed;
}
