/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Tests for the shared surface, driven through the C ABI exactly as an embedder
//! would call it. A second D3D11 device stands in for the consuming process; S8
//! runs that consumer in a real child process.
//!
//! Only one `Servo` may exist per process, so every engine-side case runs in order
//! inside one test.

use std::ffi::{CString, c_void};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::ptr;
use std::time::{Duration, Instant};

use servo_api::WebView;
use winapi::Interface;
use winapi::shared::dxgi::{
    CreateDXGIFactory1, DXGI_ADAPTER_DESC1, IDXGIAdapter1, IDXGIFactory1, IDXGIKeyedMutex,
};
use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
use winapi::shared::winerror::{S_OK, SUCCEEDED};
use winapi::um::d3d11::{
    D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
};
use winapi::um::d3d11_1::ID3D11Device1;
use winapi::um::d3dcommon::D3D_DRIVER_TYPE_UNKNOWN;
use winapi::um::winnt::HANDLE;
use wio::com::ComPtr;

use super::{SERVO_SHARED_SURFACE_OK, ServoSharedSurfaceInfo};
use crate::native::{
    SERVO_INPUT_MOUSE_BUTTON, SERVO_MOUSE_BUTTON_PRIMARY, SERVO_MOUSE_DOWN, SERVO_MOUSE_UP,
    ServoInputEvent,
};
use crate::rendering_context::RenderingContext;
use crate::webview_delegate::{
    SERVO_EMBEDDER_CONTROL_PAGE_REQUEST, ServoEmbedderControl, ServoEmbedderController,
    ServoWebViewDelegate,
};

const CHILD_ENV: &str = "SERVO_SHARED_SURFACE_CHILD";

// ---------------------------------------------------------------------------
// The consumer: a separate D3D11 device on the same adapter.
// ---------------------------------------------------------------------------

fn consumer_device(adapter_luid: u64) -> ComPtr<ID3D11Device1> {
    unsafe {
        let mut factory: *mut c_void = ptr::null_mut();
        assert_eq!(
            CreateDXGIFactory1(&IDXGIFactory1::uuidof(), &mut factory),
            S_OK
        );
        let factory = ComPtr::from_raw(factory as *mut IDXGIFactory1);
        let mut index = 0;
        loop {
            let mut adapter: *mut IDXGIAdapter1 = ptr::null_mut();
            assert!(
                SUCCEEDED(factory.EnumAdapters1(index, &mut adapter)),
                "no adapter with LUID {adapter_luid:#x}"
            );
            let adapter = ComPtr::from_raw(adapter);
            let mut desc: DXGI_ADAPTER_DESC1 = std::mem::zeroed();
            adapter.GetDesc1(&mut desc);
            let luid =
                ((desc.AdapterLuid.HighPart as u32 as u64) << 32) | desc.AdapterLuid.LowPart as u64;
            if luid == adapter_luid {
                let mut device: *mut ID3D11Device = ptr::null_mut();
                let hr = D3D11CreateDevice(
                    adapter.as_raw() as *mut _,
                    D3D_DRIVER_TYPE_UNKNOWN,
                    ptr::null_mut(),
                    0,
                    ptr::null(),
                    0,
                    D3D11_SDK_VERSION,
                    &mut device,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                assert_eq!(hr, S_OK, "D3D11CreateDevice");
                return ComPtr::from_raw(device).cast::<ID3D11Device1>().unwrap();
            }
            index += 1;
        }
    }
}

fn open_nt(device: &ComPtr<ID3D11Device1>, handle: u64) -> Result<ComPtr<ID3D11Texture2D>, i32> {
    let mut texture: *mut c_void = ptr::null_mut();
    let hr = unsafe {
        device.OpenSharedResource1(
            handle as usize as HANDLE,
            &ID3D11Texture2D::uuidof(),
            &mut texture,
        )
    };
    if SUCCEEDED(hr) && !texture.is_null() {
        Ok(unsafe { ComPtr::from_raw(texture as *mut ID3D11Texture2D) })
    } else {
        Err(hr)
    }
}

fn open_legacy(device: &ComPtr<ID3D11Device1>, handle: u64) -> bool {
    let mut texture: *mut c_void = ptr::null_mut();
    let hr = unsafe {
        device.OpenSharedResource(
            handle as usize as HANDLE,
            &ID3D11Texture2D::uuidof(),
            &mut texture,
        )
    };
    if SUCCEEDED(hr) && !texture.is_null() {
        unsafe { ComPtr::from_raw(texture as *mut ID3D11Texture2D) };
        return true;
    }
    false
}

/// Acquire a finished frame, read the given pixels, and hand the texture back.
/// Returns `None` if no frame was ready within `timeout_ms`.
fn consume(
    device: &ComPtr<ID3D11Device1>,
    texture: &ComPtr<ID3D11Texture2D>,
    points: &[(u32, u32)],
    timeout_ms: u32,
) -> Option<Vec<[u8; 4]>> {
    unsafe {
        let keyed_mutex = texture.cast::<IDXGIKeyedMutex>().unwrap();
        if keyed_mutex.AcquireSync(1, timeout_ms) != S_OK {
            return None;
        }
        let mut desc: D3D11_TEXTURE2D_DESC = std::mem::zeroed();
        texture.GetDesc(&mut desc);
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ,
            MiscFlags: 0,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            ..desc
        };
        let mut staging: *mut ID3D11Texture2D = ptr::null_mut();
        assert_eq!(
            device.CreateTexture2D(&staging_desc, ptr::null(), &mut staging),
            S_OK
        );
        let staging = ComPtr::from_raw(staging);
        let mut context: *mut ID3D11DeviceContext = ptr::null_mut();
        device.GetImmediateContext(&mut context);
        let context = ComPtr::from_raw(context);
        context.CopyResource(
            staging.as_raw() as *mut ID3D11Resource,
            texture.as_raw() as *mut ID3D11Resource,
        );
        keyed_mutex.ReleaseSync(0);

        let mut mapped: D3D11_MAPPED_SUBRESOURCE = std::mem::zeroed();
        assert_eq!(
            context.Map(
                staging.as_raw() as *mut ID3D11Resource,
                0,
                D3D11_MAP_READ,
                0,
                &mut mapped
            ),
            S_OK
        );
        let pixels = points
            .iter()
            .map(|&(x, y)| {
                let row = (mapped.pData as *const u8).add((y * mapped.RowPitch) as usize);
                let pixel = row.add((x * 4) as usize);
                [*pixel, *pixel.add(1), *pixel.add(2), *pixel.add(3)]
            })
            .collect();
        context.Unmap(staging.as_raw() as *mut ID3D11Resource, 0);
        Some(pixels)
    }
}

// ---------------------------------------------------------------------------
// The engine side, through the C ABI.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ViewState {
    frame_ready: bool,
    load_complete: bool,
    title: String,
    controls_seen: u32,
    last_tag: u32,
    response: Vec<u8>,
}

unsafe extern "C" fn on_load_status(_: *mut WebView, status: i32, user_data: *mut c_void) {
    if status == 2 {
        unsafe { (*(user_data as *mut ViewState)).load_complete = true };
    }
}

unsafe extern "C" fn on_frame(_: *mut WebView, user_data: *mut c_void) {
    unsafe { (*(user_data as *mut ViewState)).frame_ready = true };
}

unsafe extern "C" fn on_title(_: *mut WebView, ptr: *const u8, len: usize, user_data: *mut c_void) {
    let title = if ptr.is_null() {
        String::new()
    } else {
        String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(ptr, len) }).into_owned()
    };
    unsafe { (*(user_data as *mut ViewState)).title = title };
}

unsafe extern "C" fn on_control(
    _: *mut WebView,
    control: *const ServoEmbedderControl,
    user_data: *mut c_void,
) -> bool {
    let state = unsafe { &mut *(user_data as *mut ViewState) };
    state.controls_seen += 1;
    state.last_tag = unsafe { (*control).tag };
    unsafe {
        crate::webview_delegate::servo_webview_send_response(
            control,
            state.response.as_ptr(),
            state.response.len(),
        )
    }
}

static CONTROLLER: ServoEmbedderController = ServoEmbedderController {
    flags: SERVO_EMBEDDER_CONTROL_PAGE_REQUEST,
    on_control: Some(on_control),
};

struct View {
    webview: *mut WebView,
    context: *mut RenderingContext,
    state: Box<ViewState>,
}

impl View {
    fn new(servo: *mut servo_api::Servo, context: *mut RenderingContext, url: &str) -> View {
        let mut state = Box::<ViewState>::default();
        unsafe {
            // Keep a handle of our own for make_current/present; the builder consumes one.
            let own = crate::rendering_context::servo_rendering_context_clone(context);
            let builder = crate::webview::servo_webview_builder_create(servo, context);
            let mut delegate: ServoWebViewDelegate = std::mem::zeroed();
            delegate.user_data = &mut *state as *mut ViewState as *mut c_void;
            delegate.notify_load_status_changed = Some(on_load_status);
            delegate.notify_new_frame_ready = Some(on_frame);
            delegate.notify_page_title_changed = Some(on_title);
            delegate.embedder_control = &CONTROLLER;
            crate::webview::servo_webview_builder_set_delegate(builder, delegate);
            let url = CString::new(url).unwrap();
            assert_eq!(
                crate::webview::servo_webview_builder_set_url(builder, url.as_ptr()),
                0
            );
            let webview = crate::webview::servo_webview_builder_build(builder);
            assert!(!webview.is_null());
            View {
                webview,
                context: own,
                state,
            }
        }
    }

    /// Paint and present if the engine has a new frame, or unconditionally.
    fn draw(&mut self, force: bool) -> bool {
        if !self.state.frame_ready && !force {
            return false;
        }
        self.state.frame_ready = false;
        unsafe {
            assert!(crate::native::servo_rendering_context_make_current(
                self.context
            ));
            crate::webview::servo_webview_paint(self.webview);
            crate::rendering_context::servo_rendering_context_present(self.context);
        }
        true
    }

    fn info(&self) -> ServoSharedSurfaceInfo {
        let mut info = ServoSharedSurfaceInfo::default();
        assert_eq!(
            unsafe { super::servo_rendering_context_shared_info(self.context, &mut info) },
            SERVO_SHARED_SURFACE_OK
        );
        info
    }

    fn load(&mut self, servo: *mut servo_api::Servo, url: &str) {
        self.state.load_complete = false;
        let url = CString::new(url).unwrap();
        assert_eq!(
            unsafe { crate::webview::servo_webview_load(self.webview, url.as_ptr()) },
            0
        );
        spin_until(servo, &mut [self], 20, |views| views[0].state.load_complete);
    }

    fn click(&self, x: f32, y: f32) {
        for action in [SERVO_MOUSE_DOWN, SERVO_MOUSE_UP] {
            let mut event: ServoInputEvent = unsafe { std::mem::zeroed() };
            event.kind = SERVO_INPUT_MOUSE_BUTTON;
            event.mouse_button_action = action;
            event.mouse_button = SERVO_MOUSE_BUTTON_PRIMARY;
            event.x = x;
            event.y = y;
            assert_eq!(
                unsafe { crate::native::servo_webview_notify_input_event(self.webview, event) },
                0
            );
        }
    }
}

/// Spin the engine, drawing every view whose frame is ready, until `done` holds.
fn spin_until(
    servo: *mut servo_api::Servo,
    views: &mut [&mut View],
    seconds: u64,
    mut done: impl FnMut(&mut [&mut View]) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        unsafe { crate::servo::servo_spin_event_loop(servo) };
        for view in views.iter_mut() {
            view.draw(false);
        }
        if done(views) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out after {seconds}s");
}

/// Draw and consume frames until the pixels at `points` match `expected`, returning
/// what was last seen.
fn wait_for_pixels(
    servo: *mut servo_api::Servo,
    view: &mut View,
    consumer: &ComPtr<ID3D11Device1>,
    texture: &ComPtr<ID3D11Texture2D>,
    points: &[(u32, u32)],
    expected: &[[u8; 4]],
) -> Result<(), Vec<[u8; 4]>> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last = Vec::new();
    while Instant::now() < deadline {
        unsafe { crate::servo::servo_spin_event_loop(servo) };
        view.draw(true);
        if let Some(seen) = consume(consumer, texture, points, 100) {
            if seen == expected {
                return Ok(());
            }
            last = seen;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(last)
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

#[test]
fn shared_surface_end_to_end() {
    unsafe {
        let builder = crate::servo_builder_create();
        let options = crate::options::servo_options_create();
        crate::options::servo_options_set_multiprocess(options, false);
        crate::servo_builder_set_options(builder, options);
        let preferences = crate::preferences::servo_preferences_create();
        let name = CString::new("dom_embedder_request_enabled").unwrap();
        crate::preferences::servo_preferences_set_bool(preferences, name.as_ptr(), true);
        crate::servo_builder_set_preferences(builder, preferences);
        let servo = crate::servo_builder_build(builder);
        assert!(!servo.is_null());

        // S1: a surface at a size no window has, with nothing created on screen.
        let (width, height) = (1600, 1200);
        let context =
            super::servo_rendering_context_create_shared(width, height, std::process::id());
        assert!(!context.is_null(), "S1: create_shared returned NULL");
        let top_half = "<style>body{margin:0}div{height:600px}</style>\
                        <div style='background:rgb(255,0,0)'></div>\
                        <div style='background:rgb(0,0,255)'></div>";
        let mut view = View::new(servo, context, &format!("data:text/html,{top_half}"));
        let info = view.info();
        assert_eq!((info.width, info.height), (width, height), "S1: size");
        assert_eq!(info.generation, 1);
        assert_eq!(info.frame, 0);
        assert_eq!(info.dxgi_format, 28);
        assert_eq!(info.flags, super::SERVO_SHARED_SURFACE_FLAG_BOTTOM_UP);
        assert_ne!(info.handle, 0);
        println!("PASS S1 create_shared_returns_context_at_requested_size");

        // S2: an NT handle, opened as one, and not as a legacy shared handle.
        let consumer = consumer_device(info.adapter_luid);
        let texture = open_nt(&consumer, info.handle).expect("S2: OpenSharedResource1 failed");
        assert!(
            !open_legacy(&consumer, info.handle),
            "S2: legacy OpenSharedResource succeeded"
        );
        println!("PASS S2 handle_is_nt_and_unnamed");

        // S4: a frame reaches the consumer through the keyed mutex. Rows are bottom
        // up, as the flag says, so texture row 0 holds the bottom of the page.
        spin_until(servo, &mut [&mut view], 20, |views| {
            views[0].state.load_complete
        });
        let points = [(10, height - 10), (10, 10)];
        match wait_for_pixels(servo, &mut view, &consumer, &texture, &points, &[RED, BLUE]) {
            Ok(()) => println!("PASS S4 frame_ready_then_pixels_on_gpu (bottom-up)"),
            Err(seen) => {
                panic!("S4: expected the page's red top at the texture's last row, saw {seen:?}")
            },
        }
        assert!(view.info().frame > 0, "S4: frame counter did not advance");

        // S6: a click far outside any window-sized region reaches the document.
        view.load(
            servo,
            "data:text/html,<title>start</title><body style='margin:0;height:1200px'></body>\
             <script>onclick=e=>{document.title='click:'+e.clientX+','+e.clientY}</script>",
        );
        spin_until(servo, &mut [&mut view], 5, |views| {
            views[0].draw(true);
            true
        });
        view.click(1500.0, 1100.0);
        spin_until(servo, &mut [&mut view], 20, |views| {
            views[0].state.title.starts_with("click:")
        });
        assert_eq!(view.state.title, "click:1500,1100", "S6: click position");
        println!("PASS S6 input_beyond_visible_part");

        // C1: one view on a shared surface makes a page request, gets its answer,
        // and the frame that answer produces arrives on the surface.
        view.state.response = vec![200];
        view.load(
            servo,
            "data:text/html,<title>start</title><body style='margin:0'></body><script>\
             EmbedderRequest.send(new Uint8Array([7])).then(r=>{\
             document.body.style.background='rgb(0,'+r[0]+',0)';document.title='done'})\
             </script>",
        );
        spin_until(servo, &mut [&mut view], 20, |views| {
            views[0].state.title == "done"
        });
        assert_eq!(view.state.controls_seen, 1, "C1: page request not seen");
        assert_eq!(view.state.last_tag, 4, "C1: tag");
        let green = [0, 200, 0, 255];
        wait_for_pixels(
            servo,
            &mut view,
            &consumer,
            &texture,
            &[(800, 600)],
            &[green],
        )
        .unwrap_or_else(|seen| panic!("C1: expected the answer's colour, saw {seen:?}"));
        println!("PASS C1 both_on_one_webview");

        // S5: a resize replaces the texture and issues a new handle.
        let before = view.info();
        crate::native::servo_rendering_context_resize(view.context, 800, 600);
        crate::webview::servo_webview_resize(view.webview, 800, 600);
        let after = view.info();
        assert_eq!((after.width, after.height), (800, 600), "S5: size");
        assert_eq!(after.generation, before.generation + 1, "S5: generation");
        assert_ne!(after.handle, before.handle, "S5: handle reused");
        let resized = open_nt(&consumer, after.handle).expect("S5: new handle does not open");
        wait_for_pixels(
            servo,
            &mut view,
            &consumer,
            &resized,
            &[(400, 300)],
            &[green],
        )
        .unwrap_or_else(|seen| panic!("S5: no frame at the new size, saw {seen:?}"));
        println!("PASS S5 resize_reissues_handle");

        // S8 and S3: share with a child process, which opens and reads the frame;
        // the handle value means nothing in this process.
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "shared_surface::tests::consumer_child",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let child_context = super::servo_rendering_context_create_shared(256, 256, child.id());
        assert!(
            !child_context.is_null(),
            "S8: create_shared for the child returned NULL"
        );
        let mut child_view = View::new(
            servo,
            child_context,
            "data:text/html,<body style='margin:0;background:rgb(0,128,255)'></body>",
        );
        let child_info = child_view.info();
        assert!(
            open_nt(&consumer, child_info.handle).is_err(),
            "S3: the child's handle opened in this process"
        );
        println!("PASS S3 handle_not_in_other_process");

        writeln!(
            child.stdin.as_mut().unwrap(),
            "{} {}",
            child_info.handle,
            child_info.adapter_luid
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            crate::servo::servo_spin_event_loop(servo);
            child_view.draw(true);
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "S8: the child timed out");
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut output = String::new();
        for line in BufReader::new(child.stdout.take().unwrap()).lines() {
            output.push_str(&line.unwrap());
            output.push('\n');
        }
        assert!(
            status.success() && output.contains("CHILD PIXEL 0 128 255 255"),
            "S8: child failed ({status}):\n{output}"
        );
        println!("PASS S8 presented_in_another_process");

        consumer_cases(servo, &consumer, &mut view);

        crate::webview::servo_webview_free(child_view.webview);
        crate::rendering_context::servo_rendering_context_free(child_view.context);
        crate::webview::servo_webview_free(view.webview);
        crate::rendering_context::servo_rendering_context_free(view.context);
        crate::servo::servo_free(servo);
    }
}

/// The consumer side of S8, run by `shared_surface_end_to_end` in a child process.
/// It reads `<handle> <adapter luid>` on stdin, opens the handle it was given, and
/// reports the first frame it reads.
#[test]
#[ignore]
fn consumer_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    let mut fields = line
        .split_whitespace()
        .map(|field| field.parse::<u64>().unwrap());
    let (handle, luid) = (fields.next().unwrap(), fields.next().unwrap());

    let device = consumer_device(luid);
    let texture = open_nt(&device, handle).expect("child: OpenSharedResource1 failed");
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(pixels) = consume(&device, &texture, &[(128, 128)], 200) {
            let [r, g, b, a] = pixels[0];
            if [r, g, b, a] == [0, 128, 255, 255] {
                println!("CHILD PIXEL {r} {g} {b} {a}");
                std::io::stdout().flush().unwrap();
                return;
            }
        }
    }
    panic!("child: no frame with the expected colour");
}

// ---------------------------------------------------------------------------
// Consumer textures: M1-M7.
// ---------------------------------------------------------------------------

/// A consumer process driven over stdin, reporting over stdout. See
/// `consumer_sequence_child` for its commands.
struct Consumer {
    child: std::process::Child,
    lines: std::sync::mpsc::Receiver<String>,
}

impl Consumer {
    fn spawn() -> Consumer {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "shared_surface::tests::consumer_sequence_child",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Consumer { child, lines }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn send(&mut self, command: &str) {
        writeln!(self.child.stdin.as_mut().unwrap(), "{command}").unwrap();
    }

    /// The next reply line, if one has arrived; the harness's own output is skipped.
    fn poll(&self) -> Option<String> {
        while let Ok(line) = self.lines.try_recv() {
            if let Some(reply) = line.strip_prefix("CHILD ") {
                return Some(reply.to_owned());
            }
        }
        None
    }
}

/// One turn of the engine: spin, then paint and present, which publishes to every
/// consumer. Returns how long the paint and present took.
fn pump(servo: *mut servo_api::Servo, view: &mut View) -> Duration {
    unsafe { crate::servo::servo_spin_event_loop(servo) };
    let start = Instant::now();
    view.draw(true);
    start.elapsed()
}

/// Pump until every consumer in `waiting` has replied, returning the replies.
fn replies(
    servo: *mut servo_api::Servo,
    view: &mut View,
    waiting: &[&Consumer],
    what: &str,
) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut got: Vec<Option<String>> = vec![None; waiting.len()];
    while got.iter().any(Option::is_none) {
        assert!(Instant::now() < deadline, "{what}: timed out, got {got:?}");
        pump(servo, view);
        for (consumer, slot) in waiting.iter().zip(got.iter_mut()) {
            if slot.is_none() {
                *slot = consumer.poll();
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    got.into_iter().map(Option::unwrap).collect()
}

fn set_colour(view: &View, [r, g, b, _]: [u8; 4]) {
    let script = format!("document.body.style.background='rgb({r},{g},{b})'");
    assert!(unsafe {
        crate::webview::servo_webview_evaluate_javascript(
            view.webview,
            script.as_ptr(),
            script.len(),
        )
    });
}

fn colour(step: u8) -> [u8; 4] {
    [step * 10, 60, 250 - step * 10, 255]
}

fn rgb([r, g, b, _]: [u8; 4]) -> String {
    format!("{r} {g} {b}")
}

fn sentinel() -> ServoSharedSurfaceInfo {
    ServoSharedSurfaceInfo {
        handle: 0xdead,
        width: 7,
        ..Default::default()
    }
}

fn untouched(info: &ServoSharedSurfaceInfo) -> bool {
    info.handle == 0xdead && info.width == 7
}

fn add_consumer(context: *mut RenderingContext, pid: u32) -> (i32, ServoSharedSurfaceInfo) {
    let mut info = sentinel();
    let status =
        unsafe { super::servo_rendering_context_shared_add_consumer(context, pid, &mut info) };
    (status, info)
}

fn consumer_info(context: *mut RenderingContext, pid: u32) -> (i32, ServoSharedSurfaceInfo) {
    let mut info = sentinel();
    let status =
        unsafe { super::servo_rendering_context_shared_consumer_info(context, pid, &mut info) };
    (status, info)
}

/// M1-M7, on `view` after the cases above have run. Every call in it crosses the C
/// ABI, as in `shared_surface_end_to_end`.
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn consumer_cases(
    servo: *mut servo_api::Servo,
    device: &ComPtr<ID3D11Device1>,
    view: &mut View,
) {
    use super::{
        SERVO_SHARED_SURFACE_CONSUMER_EXISTS, SERVO_SHARED_SURFACE_CONSUMER_FAILED,
        SERVO_SHARED_SURFACE_NO_CONSUMER, SERVO_SHARED_SURFACE_NOT_SHARED,
        servo_rendering_context_shared_add_consumer,
        servo_rendering_context_shared_remove_consumer,
    };

    view.load(
        servo,
        "data:text/html,<body style='margin:0;background:rgb(0,60,250)'></body>",
    );
    // Settle, so that no present happens between the two reads of the primary below.
    for _ in 0..20 {
        pump(servo, view);
    }
    let primary_before = view.info();
    let mut a = Consumer::spawn();
    let mut b = Consumer::spawn();

    // M7, first half, and M5: adding consumers leaves the primary alone, and no
    // consumer handle opens in this process.
    let (status, a_info) = add_consumer(view.context, a.pid());
    assert_eq!(status, SERVO_SHARED_SURFACE_OK, "M1: add_consumer(A)");
    let (status, b_info) = add_consumer(view.context, b.pid());
    assert_eq!(status, SERVO_SHARED_SURFACE_OK, "M1: add_consumer(B)");
    let primary_after = view.info();
    assert_eq!(
        (
            primary_after.handle,
            primary_after.generation,
            primary_after.frame
        ),
        (
            primary_before.handle,
            primary_before.generation,
            primary_before.frame
        ),
        "M7: adding consumers changed the primary"
    );
    for info in [&a_info, &b_info] {
        assert_ne!(info.handle, 0, "M1: null consumer handle");
        assert_eq!(
            (info.width, info.height),
            (primary_after.width, primary_after.height),
            "M1: consumer size"
        );
        assert_eq!(info.dxgi_format, 28);
        assert_eq!(info.flags, super::SERVO_SHARED_SURFACE_FLAG_BOTTOM_UP);
        assert_eq!(info.generation, primary_after.generation);
        assert_eq!(info.adapter_luid, primary_after.adapter_luid);
        assert!(
            open_nt(device, info.handle).is_err(),
            "M5: a consumer handle opened in the engine's process"
        );
    }
    let (status, again) = consumer_info(view.context, b.pid());
    assert_eq!(status, SERVO_SHARED_SURFACE_OK);
    assert_eq!(
        again.handle, b_info.handle,
        "consumer_info disagrees with add"
    );

    // M6: errors, each leaving *out untouched.
    let (status, info) = add_consumer(view.context, b.pid());
    assert_eq!(
        status, SERVO_SHARED_SURFACE_CONSUMER_EXISTS,
        "M6: pid twice"
    );
    assert!(untouched(&info), "M6: out written for a pid twice");
    let (status, info) = add_consumer(view.context, 0);
    assert_eq!(status, SERVO_SHARED_SURFACE_CONSUMER_FAILED, "M6: pid 0");
    assert!(untouched(&info), "M6: out written for pid 0");
    let software = crate::rendering_context::servo_rendering_context_create_software(64, 64);
    let (status, info) = add_consumer(software, std::process::id());
    assert_eq!(status, SERVO_SHARED_SURFACE_NOT_SHARED, "M6: software add");
    assert!(untouched(&info), "M6: out written for a software context");
    let (status, info) = consumer_info(software, a.pid());
    assert_eq!(status, SERVO_SHARED_SURFACE_NOT_SHARED, "M6: software info");
    assert!(untouched(&info));
    assert_eq!(
        servo_rendering_context_shared_remove_consumer(software, a.pid()),
        SERVO_SHARED_SURFACE_NOT_SHARED,
        "M6: software remove"
    );
    crate::rendering_context::servo_rendering_context_free(software);
    assert_eq!(
        servo_rendering_context_shared_add_consumer(view.context, a.pid(), ptr::null_mut()),
        SERVO_SHARED_SURFACE_NOT_SHARED,
        "M6: null out"
    );
    let (status, info) = consumer_info(view.context, 1);
    assert_eq!(status, SERVO_SHARED_SURFACE_NO_CONSUMER, "M6: unknown pid");
    assert!(untouched(&info));
    println!("PASS M5 consumer_handle_not_in_engine_process");
    println!("PASS M6 consumer_errors");

    // M1: both consumers receive every step of a counted sequence, in order. The
    // engine moves on only once both have acknowledged the current step.
    a.send(&format!("open {} {}", a_info.handle, a_info.adapter_luid));
    b.send(&format!("open {} {}", b_info.handle, b_info.adapter_luid));
    let opened = replies(servo, view, &[&a, &b], "M1: open");
    let size = format!("OPENED {} {}", a_info.width, a_info.height);
    assert_eq!(opened, [size.clone(), size], "M1: open");
    for step in 1..=10 {
        set_colour(view, colour(step));
        let expected = rgb(colour(step));
        a.send(&format!("read {expected}"));
        b.send(&format!("read {expected}"));
        let want = format!("SEEN {expected}");
        assert_eq!(
            replies(servo, view, &[&a, &b], &format!("M1: step {step}")),
            [want.clone(), want],
            "M1: step {step}"
        );
    }
    println!("PASS M1 every_consumer_every_frame");

    // M2: a consumer holding its texture stalls neither the engine nor the other
    // consumer, and is handed the newest frame once it lets go.
    a.send("hold 5000");
    assert_eq!(replies(servo, view, &[&a], "M2: hold"), ["HELD"]);
    let mut slowest = Duration::ZERO;
    for step in 11..=20 {
        set_colour(view, colour(step));
        let expected = rgb(colour(step));
        b.send(&format!("read {expected}"));
        let deadline = Instant::now() + Duration::from_secs(10);
        let reply = loop {
            assert!(Instant::now() < deadline, "M2: B starved at step {step}");
            slowest = slowest.max(pump(servo, view));
            if let Some(reply) = b.poll() {
                break reply;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(reply, format!("SEEN {expected}"), "M2: B at step {step}");
    }
    assert!(
        slowest < Duration::from_millis(100),
        "M2: a present took {slowest:?} while a consumer held its texture"
    );
    assert_eq!(a.poll(), None, "M2: A let go before B's steps were done");
    assert_eq!(replies(servo, view, &[&a], "M2: release"), ["RELEASED"]);
    a.send("first");
    assert_eq!(
        replies(servo, view, &[&a], "M2: first after release"),
        [format!("FIRST {}", rgb(colour(20)))],
        "M2: A was handed a stale frame"
    );
    println!("PASS M2 busy_consumer_never_stalls (slowest present {slowest:?})");

    // M3: a removed consumer's texture stops receiving frames; the other carries on.
    assert_eq!(
        servo_rendering_context_shared_remove_consumer(view.context, a.pid()),
        SERVO_SHARED_SURFACE_OK,
        "M3: remove"
    );
    // A frame published before the removal may still be waiting; take it.
    a.send("noframe 0");
    replies(servo, view, &[&a], "M3: drain");
    for step in 21..=25 {
        set_colour(view, colour(step));
        let expected = rgb(colour(step));
        b.send(&format!("read {expected}"));
        assert_eq!(
            replies(servo, view, &[&b], &format!("M3: B step {step}")),
            [format!("SEEN {expected}")],
            "M3: B step {step}"
        );
    }
    a.send("noframe 500");
    assert_eq!(replies(servo, view, &[&a], "M3: noframe"), ["NOFRAME"]);
    a.send("stale");
    assert_eq!(
        replies(servo, view, &[&a], "M3: stale"),
        [format!("STALE {}", rgb(colour(20)))],
        "M3: a removed consumer's texture changed"
    );
    assert_eq!(
        servo_rendering_context_shared_remove_consumer(view.context, a.pid()),
        SERVO_SHARED_SURFACE_NO_CONSUMER,
        "M3: second remove"
    );
    let (status, info) = consumer_info(view.context, a.pid());
    assert_eq!(
        status, SERVO_SHARED_SURFACE_NO_CONSUMER,
        "M3: info after remove"
    );
    assert!(untouched(&info));
    a.send("exit");
    println!("PASS M3 remove_consumer");

    // M4 and M5: a resize issues the consumer a new texture at the new size.
    crate::native::servo_rendering_context_resize(view.context, 640, 480);
    crate::webview::servo_webview_resize(view.webview, 640, 480);
    let (status, resized) = consumer_info(view.context, b.pid());
    assert_eq!(status, SERVO_SHARED_SURFACE_OK, "M4: info after resize");
    assert_eq!((resized.width, resized.height), (640, 480), "M4: size");
    assert_eq!(resized.generation, b_info.generation + 1, "M4: generation");
    assert_ne!(resized.handle, b_info.handle, "M4: handle reused");
    assert!(
        open_nt(device, resized.handle).is_err(),
        "M5: a resized consumer handle opened in the engine's process"
    );
    b.send(&format!("open {} {}", resized.handle, resized.adapter_luid));
    assert_eq!(
        replies(servo, view, &[&b], "M4: reopen"),
        ["OPENED 640 480"]
    );
    set_colour(view, colour(1));
    b.send(&format!("read {}", rgb(colour(1))));
    assert_eq!(
        replies(servo, view, &[&b], "M4: frame at new size"),
        [format!("SEEN {}", rgb(colour(1)))]
    );
    println!("PASS M4 consumer_follows_resize");

    // M7, second half: the primary still delivers frames.
    let primary = open_nt(device, view.info().handle).expect("M7: primary no longer opens");
    wait_for_pixels(servo, view, device, &primary, &[(8, 8)], &[colour(1)])
        .unwrap_or_else(|seen| panic!("M7: primary saw {seen:?}"));
    println!("PASS M7 primary_unaffected");

    b.send("exit");
    for mut consumer in [a, b] {
        let status = consumer.child.wait().unwrap();
        assert!(status.success(), "a consumer exited with {status}");
    }
}

/// A consumer process for M1-M4. It reads commands on stdin and answers each with
/// one line beginning `CHILD `:
///
/// - `open <handle> <luid>`: open a texture; `OPENED <width> <height>`.
/// - `read <r g b>`: take frames until one has that colour; `SEEN <r g b>`. Any
///   colour other than that one or the previous target fails the child.
/// - `first`: take one frame; `FIRST <r g b>`.
/// - `hold <ms>`: take one frame; `HELD`; keep it `ms`; `RELEASED`.
/// - `noframe <ms>`: `NOFRAME` if no frame arrives within `ms`, else `FRAME`.
/// - `stale`: read what the texture holds without waiting for a frame; `STALE <r g b>`.
/// - `exit`.
#[test]
#[ignore]
fn consumer_sequence_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let reply = |line: String| {
        println!("CHILD {line}");
        std::io::stdout().flush().unwrap();
    };
    let rgb_of = |pixel: [u8; 4]| format!("{} {} {}", pixel[0], pixel[1], pixel[2]);
    let point = [(8, 8)];
    let mut device: Option<ComPtr<ID3D11Device1>> = None;
    let mut texture: Option<ComPtr<ID3D11Texture2D>> = None;
    let mut previous = String::new();
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let mut words = line.split_whitespace();
        match words.next().unwrap() {
            "open" => {
                let handle: u64 = words.next().unwrap().parse().unwrap();
                let luid: u64 = words.next().unwrap().parse().unwrap();
                let device = device.get_or_insert_with(|| consumer_device(luid));
                let opened = open_nt(device, handle).expect("child: open failed");
                let mut desc: D3D11_TEXTURE2D_DESC = unsafe { std::mem::zeroed() };
                unsafe { opened.GetDesc(&mut desc) };
                texture = Some(opened);
                reply(format!("OPENED {} {}", desc.Width, desc.Height));
            },
            "read" => {
                let target = words.collect::<Vec<_>>().join(" ");
                let deadline = Instant::now() + Duration::from_secs(20);
                loop {
                    assert!(Instant::now() < deadline, "child: never saw {target}");
                    let device = device.as_ref().unwrap();
                    let Some(pixels) = consume(device, texture.as_ref().unwrap(), &point, 100)
                    else {
                        continue;
                    };
                    let seen = rgb_of(pixels[0]);
                    if seen == target {
                        break;
                    }
                    // Before the first step, whatever the page started with.
                    if !previous.is_empty() {
                        assert_eq!(seen, previous, "child: a colour between steps");
                    }
                }
                reply(format!("SEEN {target}"));
                previous = target;
            },
            "first" => {
                let device = device.as_ref().unwrap();
                let pixels = consume(device, texture.as_ref().unwrap(), &point, 10_000)
                    .expect("child: no frame");
                previous = rgb_of(pixels[0]);
                reply(format!("FIRST {previous}"));
            },
            "hold" => {
                let ms: u64 = words.next().unwrap().parse().unwrap();
                let keyed_mutex = texture.as_ref().unwrap().cast::<IDXGIKeyedMutex>().unwrap();
                assert_eq!(unsafe { keyed_mutex.AcquireSync(1, 10_000) }, S_OK);
                reply("HELD".into());
                std::thread::sleep(Duration::from_millis(ms));
                unsafe { keyed_mutex.ReleaseSync(0) };
                reply("RELEASED".into());
            },
            "noframe" => {
                let ms: u32 = words.next().unwrap().parse().unwrap();
                let keyed_mutex = texture.as_ref().unwrap().cast::<IDXGIKeyedMutex>().unwrap();
                if unsafe { keyed_mutex.AcquireSync(1, ms) } == S_OK {
                    unsafe { keyed_mutex.ReleaseSync(0) };
                    reply("FRAME".into());
                } else {
                    reply("NOFRAME".into());
                }
            },
            "stale" => {
                // A removed consumer's texture rests at key 0. Hand it to key 1 so
                // `consume` can read it as it would a frame.
                let texture = texture.as_ref().unwrap();
                let keyed_mutex = texture.cast::<IDXGIKeyedMutex>().unwrap();
                assert_eq!(unsafe { keyed_mutex.AcquireSync(0, 1_000) }, S_OK);
                unsafe { keyed_mutex.ReleaseSync(1) };
                let pixels = consume(device.as_ref().unwrap(), texture, &point, 1_000)
                    .expect("child: stale read");
                reply(format!("STALE {}", rgb_of(pixels[0])));
            },
            "exit" => return,
            other => panic!("child: unknown command {other}"),
        }
    }
}
