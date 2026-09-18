# zdr-capi-shim

The Servo C ABI cdylib with the full embedding surface on it.

This crate is `ffi/capi` plus the twelve entry points an embedder needs to put a
`WebView` inside a window it already owns. It builds `servo_capi_full.dll`, which
`.github/workflows/motor-controller.yaml` ships as `servo.dll`.

## Why it exists

`ffi/capi` exports thirty of the forty-two entry points a single-window embedder
binds. It can build a `Servo`, build a `WebView`, load a URL, paint into a
software buffer and take a screenshot. It cannot be handed a window, resized,
focused, clicked, typed into or scrolled — so an embedder on that surface gets a
picture of a page rather than a page.

The twelve that are missing:

| Entry point | What it unblocks |
| :--- | :--- |
| `servo_rendering_context_create_native` | rendering into the embedder's own window at all |
| `servo_rendering_context_set_window` | rebinding a context to a different window |
| `servo_rendering_context_take_window` | detaching a context from its window |
| `servo_rendering_context_clone` | sharing one context across viewports |
| `servo_webview_resize` | following the window as it resizes |
| `servo_webview_notify_input_event` | any mouse or keyboard input |
| `servo_webview_notify_scroll_event` | scrolling |
| `servo_webview_focus` | moving focus into a viewport |
| `servo_webview_blur` | moving focus out of a viewport |
| `servo_webview_focused` | knowing which viewport has focus |
| `servo_webview_builder_set_hidpi_scale_factor` | opening correctly on a scaled display |
| `servo_webview_set_hidpi_scale_factor` | following a DPI change |

None of these is a new embedding layer. They are ordinary capi entry points in
the same style as the thirty, over `WebView` and `WindowRenderingContext` methods
that `components/` has had all along.

## Why it is a separate crate and not a patch to `ffi/capi`

Adding them to `ffi/capi` directly would be the smaller change, and if you are
reading this with the freedom to make it, make it there instead and delete this
crate. This crate exists for the case where `ffi/capi` and `components/` are
off-limits — an embedder tracking upstream who cannot carry core patches.

Given that constraint, a *linking* shim is not possible:

- `ffi/capi` is `crate-type = ["cdylib"]`. A cdylib cannot be a Rust dependency,
  so no crate can link to it and re-export from it.
- A second cdylib that forwarded to `servo_capi.dll` at the PE level would
  statically link its own copy of libservo. Two copies means two allocators, two
  `Rc` universes and two sets of type identities, and a `RenderingContext`
  constructed in one is not a `RenderingContext` the other can use.
- The internals the new entry points need — `RenderingContext::inner` in
  particular — are `pub(crate)` to `ffi/capi`.

So the only shape left is a cdylib that *contains* `ffi/capi`. `src/lib.rs` is:

```rust
include!("../../../ffi/capi/lib.rs");

mod native;
```

That is `include!` and not `#[path] mod`, because the included file's items have
to land at *this* crate's root: `ffi/capi`'s modules refer to each other as
`crate::rendering_context`, `crate::webview_delegate` and so on. It is not a copy
or a fork — it is the same source file `servo-capi` builds, so the thirty stay in
step with upstream by construction, and `src/native.rs` holds every line this
crate adds.

## Known limitation

`servo_webview_builder_set_hidpi_scale_factor` validates its argument, logs, and
returns 0 without applying anything. `ServoWebViewBuilder` is defined in
`ffi/capi` and has no field to put a scale factor in, and this crate does not
edit `ffi/capi`. An embedder must follow `servo_webview_builder_build` with
`servo_webview_set_hidpi_scale_factor`, before the first paint, to get the
intended result. Giving `ffi/capi`'s builder a real `hidpi_scale_factor` field is
the proper fix and is a one-field change there.

## The ABI

`ServoNativeWindowHandle` (24 bytes), `ServoKeyboardEvent` (32),
`ServoInputEvent` (80, `keyboard` at offset 48) and `ServoScroll` (12) are
transcribed field for field from the embedder's binding. Every size, alignment
and field offset is asserted at compile time in `src/native.rs`, mirroring the
embedder's assertions on its side. **Those two sets of assertions are the
contract** — a field changed on one side and not the other compiles cleanly and
delivers clicks at the wrong coordinates at runtime.

The modifier bits are additionally asserted equal to `keyboard-types`'
`Modifiers` bits, so a renumbering upstream fails this crate's build rather than
silently dropping Ctrl.

Two traps, both silent, both documented on the items in `src/native.rs`:

- A wheel event and a scroll event describe the same gesture with **opposite**
  signs. An embedder feeding one gesture to both must negate on the way.
- `key` and `code` must be genuine DOM values (`"a"`, `"Enter"`, `"KeyA"`). NULL
  and the empty string mean unidentified; any other value that does not parse
  drops the whole event, with the reason logged at `error` level.

## Two ABI mismatches this crate does not fix

Both are in the thirty entry points `ffi/capi` already shipped, both are structs
passed **by value**, and neither can be corrected from here without editing
`ffi/capi` — a duplicate `#[no_mangle]` would not link.

- **`ServoEventLoopWaker`.** The embedder's binding is
  `{ wake: Option<extern "C" fn(*mut c_void)>, user_data: *mut c_void }`, 16
  bytes. `ffi/capi/lib.rs` defines `{ wake_callback: extern "C" fn() }`, 8 bytes,
  with no `user_data` at all. On the Windows x64 ABI a 16-byte struct is passed
  by hidden pointer and an 8-byte one in a register, so a call to
  `servo_builder_set_event_loop_waker` does not merely lose `user_data` — the
  callee reads a pointer as a function pointer. Fixing this means adding
  `user_data` to the engine's struct and widening `wake_callback`.
- **`ServoWebViewDelegate`.** The embedder's binding is a single
  `{ user_data: *mut c_void }` placeholder, 8 bytes;
  `ffi/capi/webview_delegate.rs` has `user_data` plus
  `notify_load_status_changed` and `notify_new_frame_ready`, 24 bytes. The
  embedder already marks its version as not field-for-field verified, so this one
  is known — but `servo_webview_builder_set_delegate` is in the *required* set,
  and calling it as declared reads 16 bytes past the caller's struct.

## Building

`servo_capi_full.dll` is a superset of `servo_capi.dll` built from the same
source, so there is no reason to build both:

```sh
cargo build -p zdr-capi-shim --release
```

The lib is named `servo_capi_full` rather than `servo_capi` because `ffi/capi`
already claims that name in this workspace, and two cdylibs writing
`target/<profile>/servo_capi.dll` is an output-filename collision that breaks
`cargo build --workspace`. Embedders load the shipped DLL by file name, not by
the name in its PE export directory.

## Licence

MPL-2.0, as the rest of Servo.
