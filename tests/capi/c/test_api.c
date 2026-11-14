/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

#define _POSIX_C_SOURCE 200809L // required by `nanosleep` in `<time.h>`
#include "servo/servo_capi.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <windows.h>
#else
#include <time.h>
#endif

// The same sizes `tests::abi_struct_sizes` pins on the Rust side, checked here
// against the generated header as a C caller compiles it.
_Static_assert(sizeof(ServoWebViewDelegate) == 152, "ServoWebViewDelegate size");
_Static_assert(sizeof(struct ServoPermissionRequest) == 64, "ServoPermissionRequest size");
_Static_assert(sizeof(struct ServoEmbedderControl) == 56, "ServoEmbedderControl size");
_Static_assert(sizeof(struct ServoFrameInfo) == 32, "ServoFrameInfo size");
_Static_assert(sizeof(struct ServoHttpHeader) == 32, "ServoHttpHeader size");
_Static_assert(sizeof(struct ServoResourceRequest) == 72, "ServoResourceRequest size");

// The test runner expects us to return 0 for success and
// non-zero for failure.

static int tests_run;
static int tests_failed;

#define RUN_TEST(name)                                          \
    do {                                                        \
        tests_run++;                                            \
        printf("  RUN  %s\n", #name);                           \
        fflush(stdout);                                         \
        int result = name();                                    \
        if (result != 0) {                                      \
            printf("  FAIL %s (returned %d)\n", #name, result); \
            tests_failed++;                                     \
        }                                                       \
        fflush(stdout);                                         \
    } while (0)

int test_builder_create_and_free(void)
{
    ServoBuilder* builder = servo_builder_create();
    if (builder == NULL)
        return 1;
    servo_builder_free(builder);
    return 0;
}

int test_builder_set_options(void)
{
    ServoBuilder* builder = servo_builder_create();
    ServoOptions* options = servo_options_create();
    if (builder == NULL || options == NULL)
        return 1;
    servo_builder_set_options(builder, options);
    // builder now owns options, so don't free it
    servo_builder_free(builder);
    return 0;
}

int test_builder_set_preferences(void)
{
    ServoBuilder* builder = servo_builder_create();
    ServoPreferences* preferences = servo_preferences_create();
    if (builder == NULL || preferences == NULL)
        return 1;
    servo_builder_set_preferences(builder, preferences);
    // builder now owns preferences, so don't free it
    servo_builder_free(builder);
    return 0;
}

static void noop_wake_callback(void) { }

int test_builder_set_event_loop_waker(void)
{
    ServoBuilder* builder = servo_builder_create();
    if (builder == NULL)
        return 1;

    struct ServoEventLoopWaker waker = { noop_wake_callback };
    servo_builder_set_event_loop_waker(builder, waker);
    servo_builder_free(builder);
    return 0;
}

int test_options_create_and_free(void)
{
    ServoOptions* options = servo_options_create();
    if (options == NULL)
        return 1;
    servo_options_free(options);
    return 0;
}

int test_options_setters(void)
{
    ServoOptions* options = servo_options_create();
    if (options == NULL)
        return 1;
    servo_options_set_hard_fail(options, true);
    servo_options_set_multiprocess(options, false);
    servo_options_set_force_ipc(options, true);
    servo_options_set_background_hang_monitor(options, false);
    servo_options_set_sandbox(options, true);
    servo_options_set_temporary_storage(options, false);
    servo_options_set_ignore_certificate_errors(options, true);
    servo_options_set_unminify_js(options, false);
    servo_options_set_unminify_css(options, true);
    servo_options_set_debug_option(
        options, SERVO_DIAGNOSTICS_LOGGING_OPTION_STYLE_TREE, true);
    servo_options_enable_time_profiling_to_stdout(options, 0.5);
    servo_options_set_random_pipeline_closure_probability(options, 0.25f);
    servo_options_free(options);
    return 0;
}

int test_preferences_create_and_free(void)
{
    ServoPreferences* preferences = servo_preferences_create();
    if (preferences == NULL)
        return 1;
    servo_preferences_free(preferences);
    return 0;
}

int test_preferences_bool_roundtrip(void)
{
    ServoPreferences* preferences = servo_preferences_create();
    if (preferences == NULL)
        return 1;

    servo_preferences_set_bool(preferences, "dom_gamepad_enabled", false);
    if (servo_preferences_get_bool(preferences, "dom_gamepad_enabled") != false) {
        servo_preferences_free(preferences);
        return 1;
    }

    servo_preferences_set_bool(preferences, "dom_gamepad_enabled", true);
    if (servo_preferences_get_bool(preferences, "dom_gamepad_enabled") != true) {
        servo_preferences_free(preferences);
        return 1;
    }

    servo_preferences_free(preferences);
    return 0;
}

int test_preferences_i64_roundtrip(void)
{
    ServoPreferences* preferences = servo_preferences_create();
    if (preferences == NULL)
        return 1;

    servo_preferences_set_i64(preferences, "layout_threads", 4);
    if (servo_preferences_get_i64(preferences, "layout_threads") != 4) {
        servo_preferences_free(preferences);
        return 1;
    }

    servo_preferences_set_i64(preferences, "layout_threads", -1);
    if (servo_preferences_get_i64(preferences, "layout_threads") != -1) {
        servo_preferences_free(preferences);
        return 1;
    }

    servo_preferences_free(preferences);
    return 0;
}

int test_preferences_u64_roundtrip(void)
{
    ServoPreferences* preferences = servo_preferences_create();
    if (preferences == NULL)
        return 1;

    servo_preferences_set_u64(preferences, "network_http_cache_size", 12345);
    if (servo_preferences_get_u64(preferences, "network_http_cache_size") != 12345) {
        servo_preferences_free(preferences);
        return 1;
    }

    servo_preferences_free(preferences);
    return 0;
}

int test_preferences_string_roundtrip(void)
{
    ServoPreferences* preferences = servo_preferences_create();
    if (preferences == NULL)
        return 1;

    servo_preferences_set_string(preferences, "user_agent", "ServoTest/1.0");

    char* user_agent = servo_preferences_get_string(preferences, "user_agent");
    if (user_agent == NULL) {
        servo_preferences_free(preferences);
        return 1;
    }
    if (strcmp(user_agent, "ServoTest/1.0") != 0) {
        free(user_agent);
        servo_preferences_free(preferences);
        return 1;
    }
    free(user_agent);
    servo_preferences_free(preferences);
    return 0;
}

int test_preferences_f64_roundtrip(void)
{
    ServoPreferences* preferences = servo_preferences_create();
    if (preferences == NULL)
        return 1;

    servo_preferences_set_f64(preferences, "dom_geolocation_test_latitude", 52.5);
    if (servo_preferences_get_f64(preferences, "dom_geolocation_test_latitude") != 52.5) {
        servo_preferences_free(preferences);
        return 1;
    }

    servo_preferences_free(preferences);
    return 0;
}

int test_preferences_f64_array_roundtrip(void)
{
    ServoPreferences* preferences = servo_preferences_create();
    if (preferences == NULL)
        return 1;

    size_t count = servo_preferences_get_f64_array(preferences, "shell_background_color_rgba", NULL, 0);
    if (count != 4) {
        servo_preferences_free(preferences);
        return 1;
    }

    const double color[4] = { 0.0, 0.25, 0.5, 0.75 };
    servo_preferences_set_f64_array(preferences, "shell_background_color_rgba", color, 4);

    double read[4] = { 0 };
    count = servo_preferences_get_f64_array(preferences, "shell_background_color_rgba", read, 4);
    if (count != 4 || memcmp(read, color, sizeof(color)) != 0) {
        servo_preferences_free(preferences);
        return 1;
    }

    servo_preferences_free(preferences);
    return 0;
}

int test_rendering_context_create_and_free(void)
{
    RenderingContext* context = servo_rendering_context_create_software(100, 100);
    if (context == NULL)
        return 1;
    servo_rendering_context_free(context);
    return 0;
}

int test_rendering_context_zero_size_returns_null(void)
{
    RenderingContext* context = servo_rendering_context_create_software(0, 0);
    if (context != NULL) {
        servo_rendering_context_free(context);
        return 1;
    }
    return 0;
}

/* A singleton Servo instance shared by all WebView tests.
 * Servo has global state that can only be initialized once per process. */
static struct Servo* test_servo = NULL;

// Helper function that creates a minimal Servo instance.
static struct Servo* create_test_servo(void)
{
    if (test_servo != NULL)
        return test_servo;

    ServoBuilder* builder = servo_builder_create();
    if (builder == NULL)
        return NULL;

    struct ServoEventLoopWaker waker = { noop_wake_callback };
    servo_builder_set_event_loop_waker(builder, waker);

    ServoOptions* options = servo_options_create();
    if (options != NULL) {
        servo_options_set_multiprocess(options, false);
        servo_builder_set_options(builder, options);
    }

    test_servo = servo_builder_build(builder);
    if (test_servo != NULL) {
        servo_setup_logging(test_servo);
    }
    return test_servo;
}

static void free_test_servo(void)
{
    if (test_servo != NULL) {
        servo_free(test_servo);
        test_servo = NULL;
    }
}

int test_webview_builder_create_and_free(void)
{
    struct Servo* servo = create_test_servo();
    if (servo == NULL)
        return 1;

    RenderingContext* context = servo_rendering_context_create_software(100, 100);
    if (context == NULL)
        return 1;

    ServoWebViewBuilder* webview_builder = servo_webview_builder_create(servo, context);
    if (webview_builder == NULL) {
        servo_rendering_context_free(context);
        return 1;
    }

    servo_webview_builder_free(webview_builder);
    return 0;
}

int test_webview_builder_set_url_valid(void)
{
    struct Servo* servo = create_test_servo();
    if (servo == NULL)
        return 1;

    RenderingContext* context = servo_rendering_context_create_software(100, 100);
    if (context == NULL)
        return 1;

    ServoWebViewBuilder* webview_builder = servo_webview_builder_create(servo, context);
    int result = servo_webview_builder_set_url(webview_builder, "https://example.com");
    servo_webview_builder_free(webview_builder);
    return (result == 0) ? 0 : 1;
}

int test_webview_builder_set_url_invalid(void)
{
    struct Servo* servo = create_test_servo();
    if (servo == NULL)
        return 1;

    RenderingContext* context = servo_rendering_context_create_software(100, 100);
    if (context == NULL)
        return 1;

    ServoWebViewBuilder* webview_builder = servo_webview_builder_create(servo, context);
    int result = servo_webview_builder_set_url(webview_builder, "not a valid url !!!");
    servo_webview_builder_free(webview_builder);
    return (result == -1) ? 0 : 1;
}

int test_webview_builder_build(void)
{
    struct Servo* servo = create_test_servo();
    if (servo == NULL)
        return 1;

    RenderingContext* context = servo_rendering_context_create_software(100, 100);
    if (context == NULL)
        return 1;

    ServoWebViewBuilder* webview_builder = servo_webview_builder_create(servo, context);
    int result = servo_webview_builder_set_url(webview_builder, "about:blank");
    if (result != 0) {
        servo_webview_builder_free(webview_builder);
        return 1;
    }

    struct WebView* webview = servo_webview_builder_build(webview_builder);
    if (webview == NULL)
        return 1;

    servo_webview_free(webview);
    return 0;
}

int test_webview_hidpi_scale_factor(void)
{
    struct Servo* servo = create_test_servo();
    if (servo == NULL)
        return 1;

    RenderingContext* context = servo_rendering_context_create_software(100, 100);
    if (context == NULL)
        return 1;

    ServoWebViewBuilder* webview_builder = servo_webview_builder_create(servo, context);
    struct WebView* webview = servo_webview_builder_build(webview_builder);
    if (webview == NULL)
        return 1;

    servo_webview_set_hidpi_scale_factor(webview, 2.0f);
    if (servo_webview_get_hidpi_scale_factor(webview) != 2.0f) {
        servo_webview_free(webview);
        return 1;
    }

    servo_webview_set_hidpi_scale_factor(webview, 1.5f);
    if (servo_webview_get_hidpi_scale_factor(webview) != 1.5f) {
        servo_webview_free(webview);
        return 1;
    }

    servo_webview_free(webview);
    return 0;
}

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

/* What a page reports through `console.log`, for the visibility tests. Pages log
 * one line per observation, e.g. "vis frame hidden" or "ticks 42". */
#define MAX_MESSAGES 256
#define MAX_MESSAGE_LEN 96

typedef struct {
    int load_complete;
    int message_count;
    char messages[MAX_MESSAGES][MAX_MESSAGE_LEN];
    long last_ticks;
    int ticks_reports;
} page_log_t;

static void on_page_load_status_changed(struct WebView* webview, int32_t status, void* user_data)
{
    (void)webview;
    if (status == SERVO_LOAD_STATUS_COMPLETE) {
        ((page_log_t*)user_data)->load_complete = 1;
    }
}

static void on_page_console_message(struct WebView* webview, int32_t level,
    const uint8_t* message_ptr, uintptr_t message_len, void* user_data)
{
    (void)webview;
    (void)level;
    page_log_t* page = (page_log_t*)user_data;
    char message[MAX_MESSAGE_LEN];
    size_t len = message_len < MAX_MESSAGE_LEN - 1 ? message_len : MAX_MESSAGE_LEN - 1;
    memcpy(message, message_ptr, len);
    message[len] = '\0';
    long ticks;
    if (sscanf(message, "ticks %ld", &ticks) == 1) {
        page->last_ticks = ticks;
        page->ticks_reports++;
        return;
    }
    if (page->message_count < MAX_MESSAGES) {
        memcpy(page->messages[page->message_count], message, len + 1);
        page->message_count++;
    }
}

static int page_message_count(const page_log_t* page, const char* message)
{
    int count = 0;
    for (int i = 0; i < page->message_count; i++) {
        count += strcmp(page->messages[i], message) == 0;
    }
    return count;
}

/* The number of "vis ..." messages, i.e. `visibilitychange` events seen. */
static int page_visibility_events(const page_log_t* page)
{
    int count = 0;
    for (int i = 0; i < page->message_count; i++) {
        count += strncmp(page->messages[i], "vis ", 4) == 0;
    }
    return count;
}

static void spin_for(struct Servo* servo, int period_ms)
{
    for (int elapsed = 0; elapsed < period_ms; elapsed += 10) {
        servo_spin_event_loop(servo);
        sleep_ms(10);
    }
}

/* Spins until every message in the NULL-terminated `expected` has been logged. */
static int spin_until_logged(struct Servo* servo, const page_log_t* page,
    const char* const* expected, int timeout_ms)
{
    for (int elapsed = 0; elapsed <= timeout_ms; elapsed += 10) {
        int missing = 0;
        for (const char* const* message = expected; *message != NULL; message++) {
            missing += page_message_count(page, *message) == 0;
        }
        if (missing == 0) {
            return 1;
        }
        servo_spin_event_loop(servo);
        sleep_ms(10);
    }
    for (const char* const* message = expected; *message != NULL; message++) {
        if (page_message_count(page, *message) == 0) {
            printf("    never logged: \"%s\"\n", *message);
        }
    }
    return 0;
}

static void print_page_log(const page_log_t* page)
{
    for (int i = 0; i < page->message_count; i++) {
        printf("    logged: \"%s\"\n", page->messages[i]);
    }
    fflush(stdout);
}

static int evaluate(struct WebView* webview, const char* script)
{
    return servo_webview_evaluate_javascript(webview, (const uint8_t*)script, strlen(script))
        ? 0
        : 1;
}

/* Builds a `WebView` on a blank page that logs to `page`, and waits for it to load. */
static struct WebView* build_logging_webview(struct Servo* servo, page_log_t* page)
{
    memset(page, 0, sizeof(*page));
    RenderingContext* context = servo_rendering_context_create_software(100, 100);
    if (context == NULL)
        return NULL;
    ServoWebViewBuilder* webview_builder = servo_webview_builder_create(servo, context);
    ServoWebViewDelegate delegate = {
        .user_data = page,
        .notify_load_status_changed = on_page_load_status_changed,
        .show_console_message = on_page_console_message,
    };
    servo_webview_builder_set_delegate(webview_builder, delegate);
    servo_webview_builder_set_url(webview_builder, "data:text/html,<body></body>");
    struct WebView* webview = servo_webview_builder_build(webview_builder);
    if (webview == NULL)
        return NULL;
    for (int elapsed = 0; elapsed < 20000 && !page->load_complete; elapsed += 10) {
        servo_spin_event_loop(servo);
        sleep_ms(10);
    }
    if (!page->load_complete) {
        printf("    the page never loaded\n");
        servo_webview_free(webview);
        return NULL;
    }
    return webview;
}

/* Defines `addFrame(name, sandboxed)`, which appends an iframe that logs
 * "init <name> <state>" when its script runs and "vis <name> <state>" on each of its
 * `visibilitychange` events, then does the same for the top-level document and adds
 * two iframes: one same-origin, one sandboxed with an opaque origin. */
static const char* const ADD_FRAMES_SCRIPT
    = "window.addFrame = function (name, sandboxed) {"
      "  var frame = document.createElement('iframe');"
      "  if (sandboxed) frame.setAttribute('sandbox', 'allow-scripts');"
      "  frame.srcdoc = '<script>'"
      "    + 'console.log(\"init ' + name + ' \" + document.visibilityState);'"
      "    + 'document.addEventListener(\"visibilitychange\", function () {'"
      "    + '  console.log(\"vis ' + name + ' \" + document.visibilityState);'"
      "    + '});'"
      "    + '<\\/script>';"
      "  document.body.appendChild(frame);"
      "};"
      "document.addEventListener('visibilitychange', function () {"
      "  console.log('vis top ' + document.visibilityState);"
      "});"
      "console.log('init top ' + document.visibilityState);"
      "addFrame('frame', false);"
      "addFrame('sandboxed', true);";

/* Hiding a `WebView` makes all of its documents hidden, nested ones included, each
 * with one `visibilitychange`; a document created while hidden starts hidden; and
 * showing makes them all visible again. */
int test_webview_hide_show_visibility(void)
{
    struct Servo* servo = create_test_servo();
    if (servo == NULL)
        return 1;
    static page_log_t page;
    struct WebView* webview = build_logging_webview(servo, &page);
    if (webview == NULL)
        return 1;

    int failed = evaluate(webview, ADD_FRAMES_SCRIPT);
    const char* const loaded[]
        = { "init top visible", "init frame visible", "init sandboxed visible", NULL };
    if (!failed && !spin_until_logged(servo, &page, loaded, 10000)) {
        printf("    documents not visible before hiding\n");
        failed = 1;
    }

    if (!failed) {
        servo_webview_hide(webview);
        const char* const hidden[]
            = { "vis top hidden", "vis frame hidden", "vis sandboxed hidden", NULL };
        if (!spin_until_logged(servo, &page, hidden, 10000)) {
            printf("    documents not hidden after servo_webview_hide\n");
            failed = 1;
        }
    }

    if (!failed) {
        // Hiding again changes nothing and fires nothing.
        servo_webview_hide(webview);
        failed = evaluate(webview, "addFrame('late', false);");
        const char* const late[] = { "init late hidden", NULL };
        if (!failed && !spin_until_logged(servo, &page, late, 10000)) {
            printf("    an iframe created while hidden did not start hidden\n");
            failed = 1;
        }
    }

    if (!failed) {
        servo_webview_show(webview);
        const char* const shown[] = { "vis top visible", "vis frame visible",
            "vis sandboxed visible", "vis late visible", NULL };
        if (!spin_until_logged(servo, &page, shown, 10000)) {
            printf("    documents not visible after servo_webview_show\n");
            failed = 1;
        }
    }

    if (!failed) {
        // Let any stray event arrive, then require exactly one per document and change.
        spin_for(servo, 300);
        if (page_visibility_events(&page) != 7) {
            printf("    expected 7 visibilitychange events, saw %d\n",
                page_visibility_events(&page));
            failed = 1;
        }
    }

    if (failed)
        print_page_log(&page);
    servo_webview_free(webview);
    return failed;
}

/* Reads the page's tick counter, or -1 if it does not answer. */
static long read_ticks(struct Servo* servo, struct WebView* webview, page_log_t* page)
{
    int reports = page->ticks_reports;
    if (evaluate(webview, "console.log('ticks ' + window.ticks);") != 0)
        return -1;
    for (int elapsed = 0; elapsed < 10000; elapsed += 10) {
        if (page->ticks_reports > reports)
            return page->last_ticks;
        servo_spin_event_loop(servo);
        sleep_ms(10);
    }
    return -1;
}

static long ticks_over(struct Servo* servo, struct WebView* webview, page_log_t* page,
    int period_ms)
{
    long start = read_ticks(servo, webview, page);
    spin_for(servo, period_ms);
    long end = read_ticks(servo, webview, page);
    return (start < 0 || end < 0) ? -1 : end - start;
}

/* A regression test for throttling a hidden `WebView`: a 10ms `setTimeout` chain
 * that ticks freely while shown runs at the throttled rate (one timer per second at
 * most) while hidden, and recovers when shown again. */
int test_webview_hidden_timer_cadence(void)
{
    struct Servo* servo = create_test_servo();
    if (servo == NULL)
        return 1;
    static page_log_t page;
    struct WebView* webview = build_logging_webview(servo, &page);
    if (webview == NULL)
        return 1;

    const int window_ms = 1500;
    long shown_ticks = -1;
    long hidden_ticks = -1;
    long shown_again_ticks = -1;
    int failed = evaluate(webview,
        "window.ticks = 0;"
        "(function tick() { window.ticks++; setTimeout(tick, 10); })();"
        "document.addEventListener('visibilitychange', function () {"
        "  console.log('vis top ' + document.visibilityState);"
        "});");
    if (!failed)
        shown_ticks = ticks_over(servo, webview, &page, window_ms);

    if (!failed) {
        servo_webview_hide(webview);
        const char* const hidden[] = { "vis top hidden", NULL };
        failed = !spin_until_logged(servo, &page, hidden, 10000);
    }
    if (!failed)
        hidden_ticks = ticks_over(servo, webview, &page, window_ms);

    if (!failed) {
        servo_webview_show(webview);
        const char* const shown[] = { "vis top visible", NULL };
        failed = !spin_until_logged(servo, &page, shown, 10000);
    }
    if (!failed) {
        // The chain may still be waiting out one throttled timer when shown.
        spin_for(servo, 1100);
        shown_again_ticks = ticks_over(servo, webview, &page, window_ms);
    }

    printf("    ticks in %dms: shown %ld, hidden %ld, shown again %ld\n", window_ms,
        shown_ticks, hidden_ticks, shown_again_ticks);
    if (!failed
        && !(shown_ticks >= 20 && hidden_ticks >= 0 && hidden_ticks <= 3
            && shown_again_ticks >= 20)) {
        failed = 1;
    }

    if (failed)
        print_page_log(&page);
    servo_webview_free(webview);
    return failed;
}

typedef struct {
    int calls;
    size_t len;
    char data[65536];
} exported_t;

static void on_site_state_exported(const uint8_t* data_ptr, uintptr_t data_len, void* user_data)
{
    exported_t* exported = (exported_t*)user_data;
    exported->calls++;
    exported->len = data_len < sizeof(exported->data) ? data_len : sizeof(exported->data);
    memcpy(exported->data, data_ptr, exported->len);
}

/* Each kind of site state exports as JSON and imports again, merged or replacing; data
 * that does not parse, and an unknown kind or mode, are refused. */
int test_site_state_export_import(void)
{
    struct Servo* servo = create_test_servo();
    if (servo == NULL)
        return 1;

    const int32_t kinds[] = { SERVO_SITE_STATE_COOKIES, SERVO_SITE_STATE_HSTS,
        SERVO_SITE_STATE_AUTH_CACHE };
    static exported_t exported;
    for (size_t i = 0; i < sizeof(kinds) / sizeof(kinds[0]); i++) {
        memset(&exported, 0, sizeof(exported));
        if (servo_site_state_export(servo, kinds[i], on_site_state_exported, &exported) != 0
            || exported.calls != 1 || exported.len == 0 || exported.data[0] != '{') {
            printf("    kind %d did not export JSON (calls %d, %zu bytes)\n", kinds[i],
                exported.calls, exported.len);
            return 1;
        }
        if (servo_site_state_import(servo, kinds[i], SERVO_SITE_STATE_MERGE,
                (const uint8_t*)exported.data, exported.len)
                != 0
            || servo_site_state_import(servo, kinds[i], SERVO_SITE_STATE_REPLACE,
                   (const uint8_t*)exported.data, exported.len)
                != 0) {
            printf("    kind %d did not import its own export\n", kinds[i]);
            return 1;
        }
    }

    const char* garbage = "not json";
    if (servo_site_state_import(servo, SERVO_SITE_STATE_COOKIES, SERVO_SITE_STATE_MERGE,
            (const uint8_t*)garbage, strlen(garbage))
        != -2) {
        printf("    unparsable data was not refused with -2\n");
        return 1;
    }
    if (servo_site_state_import(servo, SERVO_SITE_STATE_COOKIES, SERVO_SITE_STATE_MERGE, NULL, 0)
        != -2) {
        printf("    empty data was not refused with -2\n");
        return 1;
    }
    memset(&exported, 0, sizeof(exported));
    if (servo_site_state_export(servo, 99, on_site_state_exported, &exported) != -1
        || exported.calls != 0) {
        printf("    an unknown kind was not refused with -1, without a callback\n");
        return 1;
    }
    if (servo_site_state_export(servo, SERVO_SITE_STATE_COOKIES, NULL, NULL) != -1) {
        printf("    a null callback was not refused with -1\n");
        return 1;
    }
    if (servo_site_state_import(servo, 99, SERVO_SITE_STATE_MERGE, NULL, 0) != -1
        || servo_site_state_import(servo, SERVO_SITE_STATE_COOKIES, 99, NULL, 0) != -1) {
        printf("    an unknown kind or mode was not refused with -1\n");
        return 1;
    }
    return 0;
}

/* Each named resource-limit setter sets its preference and refuses a value out of range,
 * leaving the preference as it was. */
int test_preferences_resource_limits(void)
{
    ServoPreferences* preferences = servo_preferences_create();
    if (preferences == NULL)
        return 1;

    struct {
        const char* name;
        int32_t (*set)(ServoPreferences*, int64_t);
        int64_t good;
        int64_t bad;
    } limits[] = {
        { "thread_script_stack_size_kb", servo_preferences_set_script_thread_stack_size_kb, 16384,
            512 },
        { "thread_worker_stack_size_kb", servo_preferences_set_worker_thread_stack_size_kb, 4096,
            0 },
        { "thread_layout_stack_size_kb", servo_preferences_set_layout_thread_stack_size_kb, 65536,
            -1 },
        { "layout_threads", servo_preferences_set_layout_threads, 1, 100 },
    };
    int failed = 0;
    for (size_t i = 0; i < sizeof(limits) / sizeof(limits[0]); i++) {
        if (limits[i].set(preferences, limits[i].good) != 0
            || servo_preferences_get_i64(preferences, limits[i].name) != limits[i].good) {
            printf("    %s was not set to %lld\n", limits[i].name, (long long)limits[i].good);
            failed = 1;
        }
        if (limits[i].set(preferences, limits[i].bad) != -1
            || servo_preferences_get_i64(preferences, limits[i].name) != limits[i].good) {
            printf("    %s accepted %lld\n", limits[i].name, (long long)limits[i].bad);
            failed = 1;
        }
    }
    servo_preferences_free(preferences);
    return failed;
}

int run_c_api_tests(void)
{
    tests_run = 0;
    tests_failed = 0;

    RUN_TEST(test_builder_create_and_free);
    RUN_TEST(test_builder_set_options);
    RUN_TEST(test_builder_set_preferences);
    RUN_TEST(test_builder_set_event_loop_waker);
    RUN_TEST(test_options_create_and_free);
    RUN_TEST(test_options_setters);
    RUN_TEST(test_preferences_create_and_free);
    RUN_TEST(test_preferences_bool_roundtrip);
    RUN_TEST(test_preferences_i64_roundtrip);
    RUN_TEST(test_preferences_u64_roundtrip);
    RUN_TEST(test_preferences_string_roundtrip);
    RUN_TEST(test_preferences_f64_roundtrip);
    RUN_TEST(test_preferences_f64_array_roundtrip);
    RUN_TEST(test_preferences_resource_limits);
    RUN_TEST(test_rendering_context_create_and_free);
    RUN_TEST(test_rendering_context_zero_size_returns_null);
    RUN_TEST(test_webview_builder_create_and_free);
    RUN_TEST(test_webview_builder_set_url_valid);
    RUN_TEST(test_webview_builder_set_url_invalid);
    RUN_TEST(test_webview_builder_build);
    RUN_TEST(test_webview_hidpi_scale_factor);
    RUN_TEST(test_webview_hide_show_visibility);
    RUN_TEST(test_webview_hidden_timer_cadence);
    RUN_TEST(test_site_state_export_import);

    free_test_servo();

    printf("\n%d tests, %d passed, %d failed\n", tests_run,
        tests_run - tests_failed, tests_failed);
    return tests_failed > 0 ? 1 : 0;
}
