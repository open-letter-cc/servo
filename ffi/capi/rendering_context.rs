/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use servo_api::{DeviceIntRect, RgbaImage, SoftwareRenderingContext};

/// An opaque struct that abstracts a rendering context used
/// for managing an OpenGL or GLES rendering context.
///
/// See the documentation of the correpsonding [servo::RenderingContext]
/// trait in the Rust API for more details.
/// [servo::RenderingContext]: https://doc.servo.org/servo/trait.RenderingContext.html
///
/// Handles to this object can be created using one of the
/// [`servo_rendering_context_create_*`] functions.
///
/// # Thread safety
/// The handle must be used only from the thread that created it.
// cbindgen:opaque
pub struct RenderingContext {
    pub(crate) inner: Rc<dyn servo_api::RenderingContext>,
    /// The last presented frame, kept for [`servo_rendering_context_read_frame`].
    /// Shared by every handle to the same context (see
    /// `servo_rendering_context_clone`), since a present through any of them
    /// presents the one surface.
    pub(crate) frames: Rc<FrameCapture>,
}

impl RenderingContext {
    /// Wraps a context in a new handle. `always_capture` arms frame capture from the
    /// first present rather than from the first read; see [`FrameCapture`].
    pub(crate) fn new(inner: Rc<dyn servo_api::RenderingContext>, always_capture: bool) -> Self {
        let frames = FrameCapture::default();
        frames.armed.set(always_capture);
        RenderingContext {
            inner,
            frames: Rc::new(frames),
        }
    }

    /// A second handle to the same context, sharing its frame capture.
    pub(crate) fn clone_handle(&self) -> Self {
        RenderingContext {
            inner: self.inner.clone(),
            frames: self.frames.clone(),
        }
    }

    /// Presents the context and, if capture is armed, keeps a copy of the frame
    /// being presented so that it can be read back afterwards.
    ///
    /// The copy is taken *before* the present. The `RenderingContext` trait reads
    /// from the back buffer, so on a swap-chain context the presented frame is only
    /// readable at this moment: once the buffers are swapped the attached buffer is
    /// the next one, not the one shown.
    pub(crate) fn present_and_capture(&self) {
        if self.frames.armed.get() {
            if let Err(error) = self.inner.make_current() {
                log::error!("servo_rendering_context_present: make_current failed: {error:?}");
            }
            let size = self.inner.size();
            let rect = DeviceIntRect::from_size(
                servo_api::DeviceIntSize::new(size.width as i32, size.height as i32),
            );
            self.frames.record(self.inner.read_to_image(rect));
        }
        self.inner.present();
        self.frames.presented();
    }
}

/// The copy of the last presented frame behind a `RenderingContext` handle.
///
/// Capture costs a GPU-to-CPU read on every present, so it is not on by default on
/// contexts whose embedder normally presents to a window or a shared texture and
/// never reads back. It is armed by the first `servo_rendering_context_read_frame`
/// call, from which point every present is captured, and it is armed at creation on
/// the software context, whose only output is this read.
#[derive(Default)]
pub(crate) struct FrameCapture {
    armed: Cell<bool>,
    /// Presents through this context, whether captured or not.
    presented: Cell<u64>,
    /// `Some(Some(image))` after a captured present; `Some(None)` after a captured
    /// present whose read failed; `None` before any captured present.
    image: RefCell<Option<Option<RgbaImage>>>,
}

impl FrameCapture {
    fn record(&self, image: Option<RgbaImage>) {
        *self.image.borrow_mut() = Some(image);
    }

    fn presented(&self) {
        self.presented.set(self.presented.get() + 1);
    }

    /// Answers a read: fills `info` for the context's current `size` and the last
    /// captured frame, and copies the frame into `buffer` when it fits. Returns the
    /// `SERVO_READ_FRAME_*` code.
    ///
    /// # Safety
    ///
    /// `buffer` is either null or valid for writes of `buffer_len` bytes; `info` is
    /// either null or valid for a write of a `ServoFrameInfo`.
    unsafe fn read(
        &self,
        size: dpi::PhysicalSize<u32>,
        buffer: *mut u8,
        buffer_len: usize,
        info: *mut ServoFrameInfo,
    ) -> i32 {
        self.armed.set(true);

        let image = self.image.borrow();
        let (width, height, pixels) = match image.as_ref() {
            Some(Some(image)) => (image.width(), image.height(), Some(image.as_raw())),
            Some(None) => (size.width, size.height, None),
            None => (size.width, size.height, None),
        };
        let stride = width * 4;
        let frame_info = ServoFrameInfo {
            width,
            height,
            stride,
            format: SERVO_PIXEL_FORMAT_RGBA8_PREMULTIPLIED,
            flags: 0,
            frame: self.presented.get(),
        };
        if !info.is_null() {
            // SAFETY: non-null and valid for a write, as documented above.
            unsafe { info.write(frame_info) };
        }

        let pixels = match image.as_ref() {
            None => return SERVO_READ_FRAME_NO_FRAME,
            Some(None) => return SERVO_READ_FRAME_UNREADABLE,
            Some(Some(_)) => pixels.expect("captured image has pixels"),
        };
        if buffer.is_null() || buffer_len < pixels.len() {
            return SERVO_READ_FRAME_BUFFER_TOO_SMALL;
        }
        // SAFETY: `buffer` is non-null and valid for `buffer_len` bytes, which is at
        // least `pixels.len()`; the two allocations cannot overlap since `pixels` is
        // owned by this crate.
        unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), buffer, pixels.len()) };
        SERVO_READ_FRAME_OK
    }
}

/// `ServoFrameInfo::format`: four bytes per pixel, in memory order red, green,
/// blue, alpha, with the colour channels premultiplied by alpha.
pub const SERVO_PIXEL_FORMAT_RGBA8_PREMULTIPLIED: u32 = 1;

/// `ServoFrameInfo::flags` bit: rows run bottom to top, so row 0 is the bottom of
/// the page. Never set by this build, whose frames are always top-down; defined so
/// that a later backend that cannot flip for free can say so instead of flipping.
// Part of the C header's vocabulary; nothing in this crate sets it yet.
#[allow(dead_code)]
pub const SERVO_FRAME_FLAG_BOTTOM_UP: u32 = 1 << 0;

/// Returned by [`servo_rendering_context_read_frame`] when the frame was copied
/// into `buffer` and `info` describes it.
pub const SERVO_READ_FRAME_OK: i32 = 0;
/// Returned by [`servo_rendering_context_read_frame`] when `buffer` is `NULL` or
/// `buffer_len` is smaller than the frame. `info` describes the frame so the caller
/// can size a buffer; `buffer` is untouched.
pub const SERVO_READ_FRAME_BUFFER_TOO_SMALL: i32 = -1;
/// Returned by [`servo_rendering_context_read_frame`] when no frame has been
/// captured yet: nothing has been presented since capture was armed. `info` carries
/// the context's size and the number of presents so far.
pub const SERVO_READ_FRAME_NO_FRAME: i32 = -2;
/// Returned by [`servo_rendering_context_read_frame`] when the context could not be
/// read back at the last present.
pub const SERVO_READ_FRAME_UNREADABLE: i32 = -3;

/// What [`servo_rendering_context_read_frame`] fills in about a frame.
///
/// Only ever reached through a pointer, so it may grow by appending fields without
/// an ABI version bump.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ServoFrameInfo {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Bytes per row. Rows are packed: `width * 4`.
    pub stride: u32,
    /// The pixel format; always [`SERVO_PIXEL_FORMAT_RGBA8_PREMULTIPLIED`].
    pub format: u32,
    /// `SERVO_FRAME_FLAG_*` bits. 0 means rows run top to bottom.
    pub flags: u32,
    /// Presents through this context since it was created, counted whether or not
    /// they were captured. 0 means nothing has been presented yet. A caller that
    /// remembers the last value it saw can skip a read when it has not moved.
    pub frame: u64,
}

const _: () = assert!(
    size_of::<ServoFrameInfo>() == 32,
    "ServoFrameInfo must stay 32 bytes wide"
);

/// Creates a new software rendering context for headless rendering.
///
/// Its frames are read with [`servo_rendering_context_read_frame`] after each
/// `servo_rendering_context_present`.
///
/// Returns a newly allocated `RenderingContext` handle, or `NULL` on
/// failure.
/// The ownership of the returned handle is transferred to the caller,
/// who must free it with [`servo_rendering_context_free`] or consume it
/// by passing it to `servo_webview_builder_create`.
#[unsafe(no_mangle)]
pub extern "C" fn servo_rendering_context_create_software(
    width: u32,
    height: u32,
) -> *mut RenderingContext {
    let size = dpi::PhysicalSize::new(width, height);
    match SoftwareRenderingContext::new(size) {
        Ok(context) => Box::into_raw(Box::new(RenderingContext::new(Rc::new(context), true))),
        Err(error) => {
            log::error!("Failed to create SoftwareRenderingContext: {error:?}");
            std::ptr::null_mut()
        },
    }
}

/// Destroys `context` and frees its memory.
///
/// `context` is a handle to a `RenderingContext` object.
/// The ownership of `context` is transferred to the function. The caller
/// must not use or free `context` again.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `context` was previously returned by one of the
///   `servo_rendering_context_create_*` functions and has not yet been
///   freed nor passed to another API that takes ownership of it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_rendering_context_free(context: *mut RenderingContext) {
    assert!(!context.is_null(), "context pointer must not be null");

    // SAFETY: The caller is assumed to uphold the safety requirements
    // for `context` documented above.
    unsafe {
        let _ = Box::from_raw(context);
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
    let handle = unsafe { (*context).clone_handle() };

    Box::into_raw(Box::new(handle))
}

/// Presents this context's rendered frame, swapping buffers in a
/// double-buffered context.
///
/// `context` is a handle to a `RenderingContext` object. The ownership of
/// `context` remains with the caller after the call.
///
/// **This is the second half of drawing.** `servo_webview_paint` renders into
/// this context's surface; without this call that surface is never shown, so the
/// window keeps whatever it had while the engine correctly reports new frames.
/// On a software context it is what makes a frame readable with
/// [`servo_rendering_context_read_frame`].
///
/// The rendering context is the window-level object and webviews are sub-objects
/// of it, so the relationship is one context to N webviews: paint every webview
/// on a context, then present that context **once**. Presenting per webview would
/// present N times a frame.
///
/// Requires this context to be current; see
/// `servo_rendering_context_make_current`.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `context` was previously returned by one of the
///   `servo_rendering_context_create_*` functions, or by
///   `servo_rendering_context_clone`, and has not yet been freed.
/// - The call is made from the same thread that created `context`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_rendering_context_present(context: *mut RenderingContext) {
    assert!(!context.is_null(), "context pointer must not be null");

    // SAFETY: the caller is assumed to uphold the safety requirements for
    // `context` documented above.
    unsafe { (*context).present_and_capture() };
}

/// Copies the last frame presented on `context` into a caller-owned buffer.
///
/// This is the slow path of reading a context: a CPU copy of the frame, available
/// on every kind of context. An embedder that can read the context's surface
/// directly, as it can a shared surface, does not need it.
///
/// `buffer` is `width * height * 4` bytes, allocated and owned by the caller, or
/// `NULL` to only fill `info`. `info` is filled on every return but
/// [`SERVO_READ_FRAME_UNREADABLE`] and may be `NULL`.
///
/// Returns:
///
/// - [`SERVO_READ_FRAME_OK`]: the frame is in `buffer` and `info` describes it.
/// - [`SERVO_READ_FRAME_BUFFER_TOO_SMALL`]: `buffer` is `NULL` or `buffer_len` is
///   less than `info.stride * info.height`; `buffer` is untouched, `info` is filled.
/// - [`SERVO_READ_FRAME_NO_FRAME`]: nothing has been presented since capture was
///   armed (see below); `info.frame` is the number of presents so far.
/// - [`SERVO_READ_FRAME_UNREADABLE`]: the context could not be read back at the
///   last present.
///
/// # What a frame is here
///
/// The frame is the one most recently published by `servo_rendering_context_present`.
/// A `servo_webview_paint` since then does not change what this returns until the
/// next present: paint draws the next frame, present publishes it. Call this on
/// the thread that drives the context, between a present and the next paint.
///
/// # Capture is armed, not free
///
/// Keeping the frame costs a GPU-to-CPU read on every present, so a context does
/// not pay it until asked. The software context is armed from creation. Any other
/// context is armed by the first call to this function, which therefore returns
/// [`SERVO_READ_FRAME_NO_FRAME`] with a non-zero `info.frame` if frames were
/// presented before it; the next present is captured and the call after it
/// succeeds.
///
/// # Safety
///
/// The caller must ensure that:
///
/// - `context` is a non-null pointer to a `RenderingContext` previously returned by
///   one of the `servo_rendering_context_create_*` functions, or by
///   `servo_rendering_context_clone`, and not yet freed nor passed to another API
///   that takes ownership of it.
/// - `buffer` is either `NULL` or valid for writes of `buffer_len` bytes.
/// - `info` is either `NULL` or valid for a write of a `ServoFrameInfo`.
/// - The call is made from the same thread that created `context`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn servo_rendering_context_read_frame(
    context: *mut RenderingContext,
    buffer: *mut u8,
    buffer_len: usize,
    info: *mut ServoFrameInfo,
) -> i32 {
    assert!(!context.is_null(), "context pointer must not be null");

    // SAFETY: the caller is assumed to uphold the safety requirements documented
    // above.
    let context = unsafe { &*context };
    // SAFETY: as above, for `buffer` and `info`.
    unsafe {
        context
            .frames
            .read(context.inner.size(), buffer, buffer_len, info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, fill: u8) -> RgbaImage {
        RgbaImage::from_raw(width, height, [fill, fill, fill, 255].repeat((width * height) as usize))
            .expect("test image")
    }

    fn read(
        capture: &FrameCapture,
        buffer: Option<&mut [u8]>,
    ) -> (i32, ServoFrameInfo) {
        let mut info = ServoFrameInfo::default();
        let (ptr, len) = match buffer {
            Some(buffer) => (buffer.as_mut_ptr(), buffer.len()),
            None => (std::ptr::null_mut(), 0),
        };
        let code = unsafe { capture.read(dpi::PhysicalSize::new(4, 2), ptr, len, &mut info) };
        (code, info)
    }

    #[test]
    fn abi_struct_size() {
        assert_eq!(size_of::<ServoFrameInfo>(), 32);
    }

    #[test]
    fn no_frame_until_a_captured_present() {
        let capture = FrameCapture::default();
        let (code, info) = read(&capture, None);
        assert_eq!(code, SERVO_READ_FRAME_NO_FRAME);
        assert_eq!((info.width, info.height, info.frame), (4, 2, 0));
        // The read armed capture. An uncaptured present still counts.
        assert!(capture.armed.get());
        capture.presented();
        let (code, info) = read(&capture, None);
        assert_eq!(code, SERVO_READ_FRAME_NO_FRAME);
        assert_eq!(info.frame, 1);
    }

    #[test]
    fn sizes_then_copies_the_presented_frame() {
        let capture = FrameCapture::default();
        capture.record(Some(image(4, 2, 7)));
        capture.presented();

        let (code, info) = read(&capture, None);
        assert_eq!(code, SERVO_READ_FRAME_BUFFER_TOO_SMALL);
        assert_eq!((info.width, info.height, info.stride), (4, 2, 16));
        assert_eq!(info.format, SERVO_PIXEL_FORMAT_RGBA8_PREMULTIPLIED);
        assert_eq!(info.flags, 0);
        assert_eq!(info.frame, 1);

        let mut small = [0xaa; 31];
        let (code, _) = read(&capture, Some(&mut small));
        assert_eq!(code, SERVO_READ_FRAME_BUFFER_TOO_SMALL);
        assert!(small.iter().all(|byte| *byte == 0xaa), "buffer untouched");

        let mut exact = [0; 32];
        let (code, _) = read(&capture, Some(&mut exact));
        assert_eq!(code, SERVO_READ_FRAME_OK);
        assert_eq!(exact, [7, 7, 7, 255].repeat(8).as_slice());

        // A larger buffer is fine; only the frame's bytes are written.
        let mut large = [0xaa; 40];
        let (code, _) = read(&capture, Some(&mut large));
        assert_eq!(code, SERVO_READ_FRAME_OK);
        assert_eq!(&large[..32], [7, 7, 7, 255].repeat(8).as_slice());
        assert!(large[32..].iter().all(|byte| *byte == 0xaa));
    }

    #[test]
    fn unreadable_present_reports_itself() {
        let capture = FrameCapture::default();
        capture.record(Some(image(4, 2, 1)));
        capture.presented();
        capture.record(None);
        capture.presented();
        let mut buffer = [0; 32];
        let (code, info) = read(&capture, Some(&mut buffer));
        assert_eq!(code, SERVO_READ_FRAME_UNREADABLE);
        assert_eq!(info.frame, 2);
    }
}
