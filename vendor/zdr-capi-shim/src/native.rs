/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The twelve C entry points an embedder needs to host a `WebView` inside a
//! window it already owns, and which `ffi/capi` does not define.
//!
//! `ffi/capi` can load a page and paint it into a software buffer. It cannot
//! be given a window, resized, focused, clicked, typed into or scrolled. These
//! are the functions that close that gap. Every one of them is an ordinary
//! capi entry point in the same style as the rest of the surface - there is no
//! new embedding layer here and no new concept.
//!
//! # ABI note, please read before changing anything in this file
//!
//! `ServoNativeWindowHandle`, `ServoKeyboardEvent`, `ServoInputEvent` and
//! `ServoScroll` are the contract with the embedder, transcribed field for
//! field. The ABI is positional, so a reordered or retyped field is silently
//! wrong rather than a compile error, and it corrupts every call rather than
//! failing to link - a mismatch here delivers clicks at the wrong coordinates.
//!
//! Every size, alignment and field offset is therefore asserted at compile
//! time further down, mirroring the embedder's own assertions on its side of
//! the same structs. Those two sets of assertions *are* the contract. Changing
//! a field here without changing it there compiles cleanly and breaks input at
//! runtime, so change both or neither.

// The `SERVO_*` constants are the vocabulary of the C header and of every
// embedder binding against it. Several of them name a default that this file
// reaches by falling through a `match`, so they are written down but never
// read here; that is the point of them, not an oversight.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CStr;
use std::mem::offset_of;
use std::os::raw::c_char;
use std::rc::{Rc, Weak};

use euclid::Scale;
use raw_window_handle::{DisplayHandle, WindowHandle};
use servo_api::{
    Code, DeviceIndependentPixel, DevicePixel, DevicePoint, DeviceVector2D, InputEvent, Key,
    KeyState, KeyboardEvent, Location, Modifiers, MouseButton, MouseButtonAction, MouseButtonEvent,
    MouseLeftViewportEvent, MouseMoveEvent, NamedKey, Scroll, WebView, WebViewPoint, WebViewVector,
    WheelDelta, WheelEvent, WheelMode, WindowRenderingContext,
};

use crate::rendering_context::RenderingContext;

/// Returned by the `-> i32` entry points on success.
const OK: i32 = 0;
/// Returned by the `-> i32` entry points when the call could not be carried
/// out. The reason is always logged at `error` level.
const ERR: i32 = -1;

// -------------------------------------------------------------------------
// Native window handles
// -------------------------------------------------------------------------

/// The windowing-system handle of a window the embedder already owns, in the
/// shape `raw-window-handle` needs to describe it.
///
/// Only [`SERVO_WINDOW_HANDLE_WIN32`] is understood today. Any other `kind` is
/// refused rather than guessed at, because getting this wrong hands `surfman`
/// a pointer of the wrong sort.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ServoNativeWindowHandle {
    /// Which windowing system `window` and `display` come from.
    pub kind: i32,
    /// Screen number, for windowing systems that have one. Unused for Win32.
    pub screen: i32,
    /// Win32: the `HWND`.
    pub window: usize,
    /// Win32: the `HINSTANCE` (`GWLP_HINSTANCE`), or 0 if the embedder does
    /// not have one to hand.
    pub display: usize,
}

/// `window` is an `HWND` and `display` is an `HINSTANCE`.
pub const SERVO_WINDOW_HANDLE_WIN32: i32 = 0;
/// Declared so that the `kind` space is the same on both sides of the ABI.
/// Not implemented: a call carrying it is refused, not guessed at.
pub const SERVO_WINDOW_HANDLE_WAYLAND: i32 = 1;
/// See [`SERVO_WINDOW_HANDLE_WAYLAND`].
pub const SERVO_WINDOW_HANDLE_XLIB: i32 = 2;
/// See [`SERVO_WINDOW_HANDLE_WAYLAND`].
pub const SERVO_WINDOW_HANDLE_APP_KIT: i32 = 3;

/// Turns a `ServoNativeWindowHandle` into the borrowed `raw-window-handle`
/// pair that `WindowRenderingContext` wants, or `None` if it cannot.
///
/// # Safety
///
/// The caller must ensure that `handle.window` is a live window handle owned
/// by the calling thread, and that the window outlives every
/// `RenderingContext` built from it.
#[cfg(target_os = "windows")]
unsafe fn borrow_handles(
    handle: ServoNativeWindowHandle,
) -> Option<(DisplayHandle<'static>, WindowHandle<'static>)> {
    // Scoped rather than at the top of the file so the non-Windows build does
    // not import names it has no use for.
    use std::num::NonZeroIsize;

    use raw_window_handle::{
        RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
    };

    if handle.kind != SERVO_WINDOW_HANDLE_WIN32 {
        log::error!(
            "ServoNativeWindowHandle kind {} is not supported (expected {} = Win32)",
            handle.kind,
            SERVO_WINDOW_HANDLE_WIN32
        );
        return None;
    }

    let Some(hwnd) = NonZeroIsize::new(handle.window as isize) else {
        log::error!("ServoNativeWindowHandle.window is a null HWND");
        return None;
    };

    let mut win32 = Win32WindowHandle::new(hwnd);
    win32.hinstance = NonZeroIsize::new(handle.display as isize);

    // SAFETY: the caller is assumed to uphold the requirements documented
    // above; `borrow_raw` only records that assumption in the type.
    let window = unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(win32)) };
    // SAFETY: a Windows display handle carries no borrowed data.
    let display = unsafe {
        DisplayHandle::borrow_raw(RawDisplayHandle::Windows(WindowsDisplayHandle::new()))
    };

    Some((display, window))
}

/// Non-Windows stub.
///
/// The native-window entry points are exported on every target so that the
/// header and the export list do not change shape per platform, but only Win32
/// handles are described by the embedder ABI, so everything else is refused
/// rather than guessed at.
///
/// # Safety
///
/// Trivially safe; `unsafe` only to match the Windows signature.
#[cfg(not(target_os = "windows"))]
unsafe fn borrow_handles(
    handle: ServoNativeWindowHandle,
) -> Option<(DisplayHandle<'static>, WindowHandle<'static>)> {
    log::error!(
        "native rendering contexts are only implemented for Win32 handles; got \
         ServoNativeWindowHandle kind {} on a non-Windows target",
        handle.kind
    );
    None
}

// -------------------------------------------------------------------------
// Keeping hold of the concrete WindowRenderingContext
// -------------------------------------------------------------------------

// `RenderingContext` erases its payload to `Rc<dyn servo::RenderingContext>`,
// and `set_window` / `take_window` are inherent methods on the concrete
// `WindowRenderingContext` rather than trait methods - so once erased there is
// no way back. Rather than change `RenderingContext`, which would be an edit
// to `ffi/capi`, keep a side table from the erased `Rc`'s data address to the
// concrete value.
//
// The entries are `Weak`, so this never keeps a context alive, and a dead
// entry is dropped the next time its address is looked up. Contexts are
// per-viewport and single-threaded, so the table is a thread-local and is
// never contended.
thread_local! {
    static NATIVE_CONTEXTS: RefCell<HashMap<usize, Weak<WindowRenderingContext>>> =
        RefCell::new(HashMap::new());
}

/// The identity of an erased rendering context: the address of the value
/// inside the `Rc`, which is stable for as long as the `Rc` is alive and is
/// shared by every clone of it.
fn erased_key(inner: &Rc<dyn servo_api::RenderingContext>) -> usize {
    Rc::as_ptr(inner) as *const () as usize
}

fn remember_native(context: &Rc<WindowRenderingContext>) {
    let erased: Rc<dyn servo_api::RenderingContext> = context.clone();
    let key = erased_key(&erased);
    NATIVE_CONTEXTS.with(|contexts| {
        contexts.borrow_mut().insert(key, Rc::downgrade(context));
    });
}

/// Looks up the concrete `WindowRenderingContext` behind an erased handle.
/// Returns `None` for a software context, or for a native one whose last
/// handle has already been freed.
fn lookup_native(
    inner: &Rc<dyn servo_api::RenderingContext>,
) -> Option<Rc<WindowRenderingContext>> {
    let key = erased_key(inner);
    NATIVE_CONTEXTS.with(|contexts| {
        let mut contexts = contexts.borrow_mut();
        match contexts.get(&key).and_then(Weak::upgrade) {
            Some(context) => Some(context),
            None => {
                contexts.remove(&key);
                None
            },
        }
    })
}

/// Shared prologue for the two entry points that need the concrete context.
///
/// # Safety
///
/// `context` must satisfy the requirements documented on the calling entry
/// point.
unsafe fn native_context_of(
    context: *mut RenderingContext,
    what: &str,
) -> Option<Rc<WindowRenderingContext>> {
    assert!(!context.is_null(), "context pointer must not be null");

    // SAFETY: the caller is assumed to uphold the safety requirements for
    // `context` documented on the calling entry point.
    let inner = unsafe { &(*context).inner };

    let found = lookup_native(inner);
    if found.is_none() {
        log::error!("{what} needs a rendering context from servo_rendering_context_create_native");
    }
    found
}

// -------------------------------------------------------------------------
// Rendering context entry points
// -------------------------------------------------------------------------

/// Creates a rendering context that draws directly into a window the embedder
/// already owns, rather than into a software buffer.
///
/// `window` describes that window; see [`ServoNativeWindowHandle`].
///
/// `width` and `height` are the window's inner size in physical pixels. Both
/// must be non-zero.
///
/// Returns a newly allocated `RenderingContext` handle, or `NULL` on failure.
/// The ownership of the returned handle is transferred to the caller, who must
/// free it with `servo_rendering_context_free` or consume it by passing it to
/// `servo_webview_builder_create`.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `window` describes a live window owned by the calling thread, which is
///   not destroyed before the returned `RenderingContext` and every `WebView`
///   built from it have been freed.
/// - The call is made from the thread that will drive the event loop.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_rendering_context_create_native(
    window: ServoNativeWindowHandle,
    width: u32,
    height: u32,
) -> *mut RenderingContext {
    // SAFETY: the caller is assumed to uphold the requirements documented
    // above.
    let Some((display_handle, window_handle)) = (unsafe { borrow_handles(window) }) else {
        return std::ptr::null_mut();
    };

    let size = dpi::PhysicalSize::new(width, height);
    match WindowRenderingContext::new(display_handle, window_handle, size) {
        Ok(context) => {
            let context = Rc::new(context);
            remember_native(&context);
            Box::into_raw(Box::new(RenderingContext { inner: context }))
        },
        Err(error) => {
            log::error!("Failed to create WindowRenderingContext: {error:?}");
            std::ptr::null_mut()
        },
    }
}

/// Points an existing native rendering context at a different window, with a
/// new size.
///
/// `context` is a handle to a `RenderingContext` object. The ownership of
/// `context` remains with the caller after the call.
///
/// Returns 0 on success, or -1 if `context` is not a native rendering context
/// or the window could not be bound.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `context` is a non-null pointer to a `RenderingContext` previously
///   returned by [`servo_rendering_context_create_native`] and has not yet
///   been freed nor passed to another API that takes ownership of it.
/// - `window` describes a live window owned by the calling thread, as for
///   [`servo_rendering_context_create_native`].
/// - The call is made from the same thread that created `context`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_rendering_context_set_window(
    context: *mut RenderingContext,
    window: ServoNativeWindowHandle,
    width: u32,
    height: u32,
) -> i32 {
    // SAFETY: see the safety requirements documented above.
    let Some(native) =
        (unsafe { native_context_of(context, "servo_rendering_context_set_window") })
    else {
        return ERR;
    };

    // SAFETY: as above.
    let Some((_display_handle, window_handle)) = (unsafe { borrow_handles(window) }) else {
        return ERR;
    };

    match native.set_window(window_handle, dpi::PhysicalSize::new(width, height)) {
        Ok(()) => OK,
        Err(error) => {
            log::error!("Failed to set window on WindowRenderingContext: {error:?}");
            ERR
        },
    }
}

/// Stops an existing native rendering context drawing into its window.
///
/// `context` is a handle to a `RenderingContext` object. The ownership of
/// `context` remains with the caller after the call; the handle stays valid
/// and can be given a new window with [`servo_rendering_context_set_window`].
///
/// Returns 0 on success, or -1 if `context` is not a native rendering context
/// or the window could not be released.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `context` is a non-null pointer to a `RenderingContext` previously
///   returned by [`servo_rendering_context_create_native`] and has not yet
///   been freed nor passed to another API that takes ownership of it.
/// - The call is made from the same thread that created `context`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_rendering_context_take_window(
    context: *mut RenderingContext,
) -> i32 {
    // SAFETY: see the safety requirements documented above.
    let Some(native) =
        (unsafe { native_context_of(context, "servo_rendering_context_take_window") })
    else {
        return ERR;
    };

    match native.take_window() {
        Ok(()) => OK,
        Err(error) => {
            log::error!("Failed to take window from WindowRenderingContext: {error:?}");
            ERR
        },
    }
}

/// Makes a second handle to the same underlying rendering context, so that
/// more than one viewport can share it.
///
/// This is a handle copy, not a new context: both handles refer to the same
/// surface and the same GL context, and freeing one does not disturb the
/// other. It works for software contexts as well as native ones.
///
/// `context` is a handle to a `RenderingContext` object. The ownership of
/// `context` remains with the caller after the call.
///
/// Returns a newly allocated `RenderingContext` handle. The ownership of the
/// returned handle is transferred to the caller, who must free it with
/// `servo_rendering_context_free` or consume it by passing it to
/// `servo_webview_builder_create`.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `context` is a non-null pointer to a `RenderingContext` previously
///   returned by one of the `servo_rendering_context_create_*` functions and
///   has not yet been freed nor passed to another API that takes ownership of
///   it.
/// - The call is made from the same thread that created `context`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_rendering_context_clone(
    context: *mut RenderingContext,
) -> *mut RenderingContext {
    assert!(!context.is_null(), "context pointer must not be null");

    // SAFETY: the caller is assumed to uphold the safety requirements for
    // `context` documented above.
    let inner = unsafe { (*context).inner.clone() };

    Box::into_raw(Box::new(RenderingContext { inner }))
}

// -------------------------------------------------------------------------
// Input
// -------------------------------------------------------------------------

/// A mouse button went down or came up. Reads `x`, `y`, `mouse_button` and
/// `mouse_button_action`.
pub const SERVO_INPUT_MOUSE_BUTTON: i32 = 0;
/// The mouse moved. Reads `x` and `y`.
pub const SERVO_INPUT_MOUSE_MOVE: i32 = 1;
/// The mouse left the viewport. Reads no other field.
pub const SERVO_INPUT_MOUSE_LEFT_VIEWPORT: i32 = 2;
/// A wheel turned. Reads `x`, `y`, `wheel_mode` and the three
/// `wheel_delta_*`.
pub const SERVO_INPUT_WHEEL: i32 = 3;
/// A key went down or came up. Reads `keyboard`.
pub const SERVO_INPUT_KEYBOARD: i32 = 4;

/// A mouse button went down.
pub const SERVO_MOUSE_DOWN: i32 = 0;
/// A mouse button came up.
pub const SERVO_MOUSE_UP: i32 = 1;

/// The DOM `MouseEvent.button` values. Anything outside this range is passed
/// through to the page as an "other" button rather than refused.
pub const SERVO_MOUSE_BUTTON_PRIMARY: i16 = 0;
/// See [`SERVO_MOUSE_BUTTON_PRIMARY`].
pub const SERVO_MOUSE_BUTTON_AUXILIARY: i16 = 1;
/// See [`SERVO_MOUSE_BUTTON_PRIMARY`].
pub const SERVO_MOUSE_BUTTON_SECONDARY: i16 = 2;
/// See [`SERVO_MOUSE_BUTTON_PRIMARY`].
pub const SERVO_MOUSE_BUTTON_BACK: i16 = 3;
/// See [`SERVO_MOUSE_BUTTON_PRIMARY`].
pub const SERVO_MOUSE_BUTTON_FORWARD: i16 = 4;

/// The `wheel_delta_*` fields are in pixels. Matches the DOM's
/// `WheelEvent.DOM_DELTA_PIXEL`.
pub const SERVO_WHEEL_MODE_PIXEL: i32 = 0;
/// The `wheel_delta_*` fields are in lines.
pub const SERVO_WHEEL_MODE_LINE: i32 = 1;
/// The `wheel_delta_*` fields are in pages.
pub const SERVO_WHEEL_MODE_PAGE: i32 = 2;

/// The key is down.
pub const SERVO_KEY_DOWN: i32 = 0;
/// The key is up.
pub const SERVO_KEY_UP: i32 = 1;

/// The DOM `KeyboardEvent.location` values.
pub const SERVO_KEY_LOCATION_STANDARD: i32 = 0;
/// See [`SERVO_KEY_LOCATION_STANDARD`].
pub const SERVO_KEY_LOCATION_LEFT: i32 = 1;
/// See [`SERVO_KEY_LOCATION_STANDARD`].
pub const SERVO_KEY_LOCATION_RIGHT: i32 = 2;
/// See [`SERVO_KEY_LOCATION_STANDARD`].
pub const SERVO_KEY_LOCATION_NUMPAD: i32 = 3;

/// Bit flags for [`ServoKeyboardEvent::modifiers`]. OR them together; bits
/// outside this set are ignored.
pub const SERVO_MODIFIER_ALT: u32 = 0x001;
/// See [`SERVO_MODIFIER_ALT`].
pub const SERVO_MODIFIER_ALT_GRAPH: u32 = 0x002;
/// See [`SERVO_MODIFIER_ALT`].
pub const SERVO_MODIFIER_CAPS_LOCK: u32 = 0x004;
/// See [`SERVO_MODIFIER_ALT`].
pub const SERVO_MODIFIER_CONTROL: u32 = 0x008;
/// See [`SERVO_MODIFIER_ALT`].
pub const SERVO_MODIFIER_FN: u32 = 0x010;
/// See [`SERVO_MODIFIER_ALT`].
pub const SERVO_MODIFIER_FN_LOCK: u32 = 0x020;
/// See [`SERVO_MODIFIER_ALT`].
pub const SERVO_MODIFIER_META: u32 = 0x040;
/// See [`SERVO_MODIFIER_ALT`].
pub const SERVO_MODIFIER_NUM_LOCK: u32 = 0x080;
/// See [`SERVO_MODIFIER_ALT`].
pub const SERVO_MODIFIER_SCROLL_LOCK: u32 = 0x100;
/// See [`SERVO_MODIFIER_ALT`].
pub const SERVO_MODIFIER_SHIFT: u32 = 0x200;
/// See [`SERVO_MODIFIER_ALT`].
pub const SERVO_MODIFIER_SYMBOL: u32 = 0x400;
/// See [`SERVO_MODIFIER_ALT`].
pub const SERVO_MODIFIER_SYMBOL_LOCK: u32 = 0x800;

// These are the `keyboard-types` `Modifiers` bits, which is what
// `Modifiers::from_bits_truncate` below reads them as. If that crate ever
// renumbers them, this is where it shows up.
const _: () = {
    assert!(SERVO_MODIFIER_ALT == Modifiers::ALT.bits());
    assert!(SERVO_MODIFIER_ALT_GRAPH == Modifiers::ALT_GRAPH.bits());
    assert!(SERVO_MODIFIER_CAPS_LOCK == Modifiers::CAPS_LOCK.bits());
    assert!(SERVO_MODIFIER_CONTROL == Modifiers::CONTROL.bits());
    assert!(SERVO_MODIFIER_FN == Modifiers::FN.bits());
    assert!(SERVO_MODIFIER_FN_LOCK == Modifiers::FN_LOCK.bits());
    assert!(SERVO_MODIFIER_META == Modifiers::META.bits());
    assert!(SERVO_MODIFIER_NUM_LOCK == Modifiers::NUM_LOCK.bits());
    assert!(SERVO_MODIFIER_SCROLL_LOCK == Modifiers::SCROLL_LOCK.bits());
    assert!(SERVO_MODIFIER_SHIFT == Modifiers::SHIFT.bits());
    assert!(SERVO_MODIFIER_SYMBOL == Modifiers::SYMBOL.bits());
    assert!(SERVO_MODIFIER_SYMBOL_LOCK == Modifiers::SYMBOL_LOCK.bits());
};

/// The keyboard half of a [`ServoInputEvent`], occupying exactly the 32 bytes
/// from offset 48 to the end of that struct.
///
/// `key` and `code` must be genuine DOM values - `"a"`, `"Enter"`, `"KeyA"` -
/// and **not** platform scancodes or platform key names. `NULL` or the empty
/// string means unidentified. Any other value that does not parse refuses the
/// whole event, with the reason logged at `error` level; that is the loudest
/// this layer can be about it, so check the log if keystrokes appear to go
/// nowhere.
///
/// Both pointers are borrowed for the duration of the call only; everything
/// needed is copied out before it returns.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ServoKeyboardEvent {
    /// The DOM `KeyboardEvent.key` value, as a NUL terminated UTF-8 string.
    pub key: *const c_char,
    /// The DOM `KeyboardEvent.code` value, as a NUL terminated UTF-8 string.
    pub code: *const c_char,
    /// [`SERVO_KEY_DOWN`] or [`SERVO_KEY_UP`].
    pub state: i32,
    /// One of the `SERVO_KEY_LOCATION_*` constants. Anything else is read as
    /// [`SERVO_KEY_LOCATION_STANDARD`].
    pub location: i32,
    /// The active modifiers; see the `SERVO_MODIFIER_*` constants.
    pub modifiers: u32,
    /// Whether this is an auto-repeat of a held key.
    pub repeat: bool,
    /// Whether an IME composition is in progress.
    pub is_composing: bool,
}

/// One input event on its way from the embedder to a `WebView`.
///
/// `kind` selects which of the other fields are read; the rest are ignored,
/// but must still be initialised - zero them rather than leaving them
/// uninitialised, because every byte of this struct crosses the ABI.
///
/// `x`/`y` are device pixels relative to the `WebView`'s top-left, not the
/// host window's.
///
/// A note that otherwise costs people an afternoon: a wheel event and a scroll
/// event describe the same gesture with **opposite** signs. `wheel_delta_y` is
/// positive when the view scrolls *up*, revealing content above;
/// [`ServoScroll::delta_y`] is positive when the view scrolls *down*,
/// revealing content below. An embedder that feeds one gesture to both must
/// negate on the way.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ServoInputEvent {
    /// Which kind of event this is; one of the `SERVO_INPUT_*` constants.
    pub kind: i32,
    /// [`SERVO_MOUSE_DOWN`] or [`SERVO_MOUSE_UP`].
    pub mouse_button_action: i32,
    /// One of the `SERVO_MOUSE_BUTTON_*` constants.
    pub mouse_button: i16,
    /// How to read the three `wheel_delta_*` fields; one of the
    /// `SERVO_WHEEL_MODE_*` constants.
    pub wheel_mode: i32,
    /// Where in the `WebView` the event happened, in device pixels.
    pub x: f32,
    /// Where in the `WebView` the event happened, in device pixels.
    pub y: f32,
    /// Wheel delta in the left/right direction. Positive scrolls the view
    /// left, revealing content to the left.
    pub wheel_delta_x: f64,
    /// Wheel delta in the up/down direction. Positive scrolls the view up,
    /// revealing content above.
    pub wheel_delta_y: f64,
    /// Wheel delta going into and out of the screen.
    pub wheel_delta_z: f64,
    /// The keyboard payload, read only when `kind` is
    /// [`SERVO_INPUT_KEYBOARD`].
    pub keyboard: ServoKeyboardEvent,
}

/// Where a scroll should end up.
///
/// `kind` selects the meaning: with [`SERVO_SCROLL_DELTA`], `delta_x`/`delta_y`
/// are an offset to scroll by; with [`SERVO_SCROLL_START`] or
/// [`SERVO_SCROLL_END`] they are ignored.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ServoScroll {
    /// One of the `SERVO_SCROLL_*` constants.
    pub kind: i32,
    /// Horizontal offset in device pixels. Positive reveals content to the
    /// right.
    pub delta_x: f32,
    /// Vertical offset in device pixels. Positive reveals content below.
    pub delta_y: f32,
}

/// Scroll by `delta_x`/`delta_y`.
pub const SERVO_SCROLL_DELTA: i32 = 0;
/// Scroll to the start of the scrollable area, ignoring the deltas.
pub const SERVO_SCROLL_START: i32 = 1;
/// Scroll to the end of the scrollable area, ignoring the deltas.
pub const SERVO_SCROLL_END: i32 = 2;

// The ABI is positional: a reordered field is silently wrong rather than a
// compile error, and a mismatch delivers clicks at the wrong coordinates
// instead of failing to link. Every offset is pinned here, not just the sizes,
// so that an edit to any of the structs above has to be deliberate. These
// mirror the embedder's own assertions; the two sets are the contract.
const _: () = {
    assert!(size_of::<ServoNativeWindowHandle>() == 24);
    assert!(align_of::<ServoNativeWindowHandle>() == 8);
    assert!(offset_of!(ServoNativeWindowHandle, kind) == 0);
    assert!(offset_of!(ServoNativeWindowHandle, screen) == 4);
    assert!(offset_of!(ServoNativeWindowHandle, window) == 8);
    assert!(offset_of!(ServoNativeWindowHandle, display) == 16);

    assert!(size_of::<ServoKeyboardEvent>() == 32);
    assert!(align_of::<ServoKeyboardEvent>() == 8);
    assert!(offset_of!(ServoKeyboardEvent, key) == 0);
    assert!(offset_of!(ServoKeyboardEvent, code) == 8);
    assert!(offset_of!(ServoKeyboardEvent, state) == 16);
    assert!(offset_of!(ServoKeyboardEvent, location) == 20);
    assert!(offset_of!(ServoKeyboardEvent, modifiers) == 24);
    assert!(offset_of!(ServoKeyboardEvent, repeat) == 28);
    assert!(offset_of!(ServoKeyboardEvent, is_composing) == 29);

    assert!(size_of::<ServoInputEvent>() == 80);
    assert!(align_of::<ServoInputEvent>() == 8);
    assert!(offset_of!(ServoInputEvent, kind) == 0);
    assert!(offset_of!(ServoInputEvent, mouse_button_action) == 4);
    assert!(offset_of!(ServoInputEvent, mouse_button) == 8);
    // 2 bytes of padding at offset 10 realign `wheel_mode`.
    assert!(offset_of!(ServoInputEvent, wheel_mode) == 12);
    assert!(offset_of!(ServoInputEvent, x) == 16);
    assert!(offset_of!(ServoInputEvent, y) == 20);
    assert!(offset_of!(ServoInputEvent, wheel_delta_x) == 24);
    assert!(offset_of!(ServoInputEvent, wheel_delta_y) == 32);
    assert!(offset_of!(ServoInputEvent, wheel_delta_z) == 40);
    assert!(offset_of!(ServoInputEvent, keyboard) == 48);

    assert!(size_of::<ServoScroll>() == 12);
    assert!(align_of::<ServoScroll>() == 4);
    assert!(offset_of!(ServoScroll, kind) == 0);
    assert!(offset_of!(ServoScroll, delta_x) == 4);
    assert!(offset_of!(ServoScroll, delta_y) == 8);
};

/// # Safety
///
/// `string` must be null, or point to a NUL terminated string that is not
/// modified for the duration of the call.
unsafe fn borrowed_str<'a>(string: *const c_char) -> Option<&'a str> {
    if string.is_null() {
        return None;
    }

    // SAFETY: the caller is assumed to uphold the requirements documented
    // above.
    match unsafe { CStr::from_ptr(string) }.to_str() {
        Ok(string) => Some(string),
        Err(error) => {
            log::error!("keyboard event string is not valid UTF-8: {error}");
            None
        },
    }
}

/// # Safety
///
/// `event.key` and `event.code` must satisfy the requirements of
/// [`borrowed_str`].
unsafe fn keyboard_event_of(event: ServoKeyboardEvent) -> Option<KeyboardEvent> {
    // SAFETY: the caller is assumed to uphold the requirements documented
    // above.
    // NULL and the empty string both mean "unidentified", per the ABI; only a
    // non-empty string that fails to parse is an error worth refusing over.
    let key = match unsafe { borrowed_str(event.key) }.filter(|key| !key.is_empty()) {
        Some(key) => match key.parse::<Key>() {
            Ok(key) => key,
            Err(_) => {
                log::error!(
                    "{key:?} is not a DOM KeyboardEvent.key value, so the event was dropped; \
                     expected values such as \"a\", \"Enter\" or \"ArrowLeft\""
                );
                return None;
            },
        },
        None => Key::Named(NamedKey::Unidentified),
    };

    // SAFETY: as above.
    let code = match unsafe { borrowed_str(event.code) }.filter(|code| !code.is_empty()) {
        Some(code) => match code.parse::<Code>() {
            Ok(code) => code,
            Err(_) => {
                log::error!(
                    "{code:?} is not a DOM KeyboardEvent.code value, so the event was dropped; \
                     expected values such as \"KeyA\", \"Enter\" or \"ArrowLeft\""
                );
                return None;
            },
        },
        None => Code::Unidentified,
    };

    let state = match event.state {
        SERVO_KEY_UP => KeyState::Up,
        _ => KeyState::Down,
    };

    let location = match event.location {
        SERVO_KEY_LOCATION_LEFT => Location::Left,
        SERVO_KEY_LOCATION_RIGHT => Location::Right,
        SERVO_KEY_LOCATION_NUMPAD => Location::Numpad,
        _ => Location::Standard,
    };

    Some(KeyboardEvent::new_without_event(
        state,
        key,
        code,
        location,
        Modifiers::from_bits_truncate(event.modifiers),
        event.repeat,
        event.is_composing,
    ))
}

/// # Safety
///
/// The string fields of `event.keyboard` must satisfy the requirements of
/// [`borrowed_str`].
unsafe fn input_event_of(event: ServoInputEvent) -> Option<InputEvent> {
    let point = WebViewPoint::Device(DevicePoint::new(event.x, event.y));

    match event.kind {
        SERVO_INPUT_MOUSE_BUTTON => {
            let action = match event.mouse_button_action {
                SERVO_MOUSE_UP => MouseButtonAction::Up,
                _ => MouseButtonAction::Down,
            };
            Some(InputEvent::MouseButton(MouseButtonEvent::new(
                action,
                MouseButton::from(event.mouse_button),
                point,
            )))
        },
        SERVO_INPUT_MOUSE_MOVE => Some(InputEvent::MouseMove(MouseMoveEvent::new(point))),
        SERVO_INPUT_MOUSE_LEFT_VIEWPORT => {
            Some(InputEvent::MouseLeftViewport(MouseLeftViewportEvent {
                focus_moving_to_another_iframe: false,
            }))
        },
        SERVO_INPUT_WHEEL => {
            let mode = match event.wheel_mode {
                SERVO_WHEEL_MODE_LINE => WheelMode::DeltaLine,
                SERVO_WHEEL_MODE_PAGE => WheelMode::DeltaPage,
                _ => WheelMode::DeltaPixel,
            };
            Some(InputEvent::Wheel(WheelEvent::new(
                WheelDelta {
                    x: event.wheel_delta_x,
                    y: event.wheel_delta_y,
                    z: event.wheel_delta_z,
                    mode,
                },
                point,
            )))
        },
        // SAFETY: the caller is assumed to uphold the requirements documented
        // above.
        SERVO_INPUT_KEYBOARD => {
            unsafe { keyboard_event_of(event.keyboard) }.map(InputEvent::Keyboard)
        },
        other => {
            log::error!("{other} is not a known ServoInputEvent kind, so the event was dropped");
            None
        },
    }
}

/// Delivers a mouse, wheel or keyboard event to `webview`.
///
/// `webview` is a handle to a `WebView` object. The ownership of `webview`
/// remains with the caller after the call.
///
/// Returns 0 if the event was handed to Servo, or -1 if it could not be
/// understood - an unknown `kind`, or a `key`/`code` that is not a DOM value.
/// A -1 always logs the reason.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` is a non-null pointer to a `WebView` previously returned by
///   `servo_webview_builder_build` and has not yet been freed nor passed to
///   another API that takes ownership of it.
/// - If `event.kind` is [`SERVO_INPUT_KEYBOARD`], `event.keyboard.key` and
///   `event.keyboard.code` are either null or point to NUL terminated strings
///   that remain unmodified for the duration of the call.
/// - The call is made from the same thread that created `webview`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_notify_input_event(
    webview: *mut WebView,
    event: ServoInputEvent,
) -> i32 {
    assert!(!webview.is_null(), "webview pointer must not be null");

    // SAFETY: the caller is assumed to uphold the safety requirements for
    // `event` documented above.
    let Some(event) = (unsafe { input_event_of(event) }) else {
        return ERR;
    };

    // SAFETY: the caller is assumed to uphold the safety requirements for
    // `webview` documented above.
    let webview = unsafe { &*webview };
    webview.notify_input_event(event);
    OK
}

/// Scrolls the scrollable area under `x`, `y` to the given destination.
///
/// `webview` is a handle to a `WebView` object. The ownership of `webview`
/// remains with the caller after the call.
///
/// `x` and `y` are in physical device pixels.
///
/// Returns 0 on success, or -1 if `scroll.kind` is not a known
/// `SERVO_SCROLL_*` constant.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` is a non-null pointer to a `WebView` previously returned by
///   `servo_webview_builder_build` and has not yet been freed nor passed to
///   another API that takes ownership of it.
/// - The call is made from the same thread that created `webview`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_notify_scroll_event(
    webview: *mut WebView,
    scroll: ServoScroll,
    x: f32,
    y: f32,
) -> i32 {
    assert!(!webview.is_null(), "webview pointer must not be null");

    let scroll = match scroll.kind {
        SERVO_SCROLL_DELTA => Scroll::Delta(WebViewVector::Device(DeviceVector2D::new(
            scroll.delta_x,
            scroll.delta_y,
        ))),
        SERVO_SCROLL_START => Scroll::Start,
        SERVO_SCROLL_END => Scroll::End,
        other => {
            log::error!("{other} is not a known ServoScroll kind, so the event was dropped");
            return ERR;
        },
    };

    // SAFETY: the caller is assumed to uphold the safety requirements for
    // `webview` documented above.
    let webview = unsafe { &*webview };
    webview.notify_scroll_event(scroll, WebViewPoint::Device(DevicePoint::new(x, y)));
    OK
}

// -------------------------------------------------------------------------
// Size, focus and HiDPI
// -------------------------------------------------------------------------

/// Resizes `webview`'s rendering context.
///
/// Every `WebView` sharing that rendering context is resized with it, since a
/// `WebView` is always as big as its context. The minimum is 1x1; a smaller
/// request is clamped rather than refused.
///
/// `webview` is a handle to a `WebView` object. The ownership of `webview`
/// remains with the caller after the call.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` is a non-null pointer to a `WebView` previously returned by
///   `servo_webview_builder_build` and has not yet been freed nor passed to
///   another API that takes ownership of it.
/// - The call is made from the same thread that created `webview`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_resize(webview: *mut WebView, width: u32, height: u32) {
    assert!(!webview.is_null(), "webview pointer must not be null");

    // SAFETY: the caller is assumed to uphold the safety requirements for
    // `webview` documented above.
    let webview = unsafe { &*webview };
    webview.resize(dpi::PhysicalSize::new(width, height));
}

/// Tells Servo that `webview` has gained keyboard focus.
///
/// `webview` is a handle to a `WebView` object. The ownership of `webview`
/// remains with the caller after the call.
///
/// The change is asynchronous: [`servo_webview_focused`] does not report the
/// new value until Servo has processed the request, which happens while the
/// embedder is inside `servo_spin_event_loop`.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` is a non-null pointer to a `WebView` previously returned by
///   `servo_webview_builder_build` and has not yet been freed nor passed to
///   another API that takes ownership of it.
/// - The call is made from the same thread that created `webview`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_focus(webview: *mut WebView) {
    assert!(!webview.is_null(), "webview pointer must not be null");

    // SAFETY: the caller is assumed to uphold the safety requirements for
    // `webview` documented above.
    let webview = unsafe { &*webview };
    webview.focus();
}

/// Tells Servo that `webview` has lost keyboard focus.
///
/// `webview` is a handle to a `WebView` object. The ownership of `webview`
/// remains with the caller after the call.
///
/// As with [`servo_webview_focus`], the change is asynchronous.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` is a non-null pointer to a `WebView` previously returned by
///   `servo_webview_builder_build` and has not yet been freed nor passed to
///   another API that takes ownership of it.
/// - The call is made from the same thread that created `webview`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_blur(webview: *mut WebView) {
    assert!(!webview.is_null(), "webview pointer must not be null");

    // SAFETY: the caller is assumed to uphold the safety requirements for
    // `webview` documented above.
    let webview = unsafe { &*webview };
    webview.blur();
}

/// Whether `webview` currently has the keyboard focus.
///
/// `webview` is a handle to a `WebView` object. The ownership of `webview`
/// remains with the caller after the call.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` is a non-null pointer to a `WebView` previously returned by
///   `servo_webview_builder_build` and has not yet been freed nor passed to
///   another API that takes ownership of it.
/// - The call is made from the same thread that created `webview`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_focused(webview: *mut WebView) -> bool {
    assert!(!webview.is_null(), "webview pointer must not be null");

    // SAFETY: the caller is assumed to uphold the safety requirements for
    // `webview` documented above.
    let webview = unsafe { &*webview };
    webview.focused()
}

/// Sets the HiDPI scale factor of `webview`: how many physical device pixels
/// there are to a device-independent pixel on the display showing it.
///
/// `webview` is a handle to a `WebView` object. The ownership of `webview`
/// remains with the caller after the call.
///
/// Returns 0 on success, or -1 if `scale_factor` is not a positive, finite
/// number.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `webview` is a non-null pointer to a `WebView` previously returned by
///   `servo_webview_builder_build` and has not yet been freed nor passed to
///   another API that takes ownership of it.
/// - The call is made from the same thread that created `webview`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_set_hidpi_scale_factor(
    webview: *mut WebView,
    scale_factor: f32,
) -> i32 {
    assert!(!webview.is_null(), "webview pointer must not be null");

    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        log::error!("{scale_factor} is not a usable HiDPI scale factor");
        return ERR;
    }

    // SAFETY: the caller is assumed to uphold the safety requirements for
    // `webview` documented above.
    let webview = unsafe { &*webview };
    webview.set_hidpi_scale_factor(Scale::<f32, DeviceIndependentPixel, DevicePixel>::new(
        scale_factor,
    ));
    OK
}

/// Sets the HiDPI scale factor a `WebView` should open with.
///
/// `builder` is a handle to a `ServoWebViewBuilder` object. The ownership of
/// `builder` remains with the caller after the call.
///
/// Returns 0 if `scale_factor` is a positive, finite number, or -1 otherwise.
///
/// # A limitation worth knowing about
///
/// `ServoWebViewBuilder` is defined in `ffi/capi` and has no field to put this
/// in, and this crate adds to that surface without editing it. So the value is
/// **accepted and not applied**: an embedder must follow
/// `servo_webview_builder_build` with a call to
/// [`servo_webview_set_hidpi_scale_factor`] carrying the same value, before
/// the first paint, to get the intended result. Every call logs that reminder
/// at `warn` level.
///
/// This function exists so that the export surface is complete and so that the
/// ordering requirement has somewhere to be written down. Giving `ffi/capi`'s
/// builder a real `hidpi_scale_factor` field is the proper fix, and is a
/// one-field change there.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `builder` is a non-null pointer to a `ServoWebViewBuilder` previously
///   returned by `servo_webview_builder_create` and has not yet been freed nor
///   passed to another API that takes ownership of it.
/// - The call is made from the same thread that created `builder`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_webview_builder_set_hidpi_scale_factor(
    builder: *mut crate::webview::ServoWebViewBuilder,
    scale_factor: f32,
) -> i32 {
    assert!(!builder.is_null(), "builder pointer must not be null");

    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        log::error!("{scale_factor} is not a usable HiDPI scale factor");
        return ERR;
    }

    log::warn!(
        "servo_webview_builder_set_hidpi_scale_factor({scale_factor}) was accepted but cannot be \
         applied to the builder; call servo_webview_set_hidpi_scale_factor({scale_factor}) after \
         servo_webview_builder_build and before the first paint"
    );
    OK
}
