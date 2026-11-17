// Copyright 2026 Open Letter, Inc.
// SPDX-License-Identifier: LicenseRef-OpenLetter-Tools

//! Open Letter additions to `ffi/capi/lib.rs`.

#[allow(unused_imports)]
use super::*;

/// The ABI version of this capi surface.
///
/// This is incremented whenever a struct layout, an enum's numbering, or a
/// function signature exported by this crate changes in a way an embedder
/// built against an older header could get wrong. Changing an existing
/// field's meaning, reordering fields and removing one all qualify.
///
/// **So does a struct passed or returned by value growing at all.** Appending
/// a field is source-compatible — recompile against the new header and a
/// positional initializer zero-fills what it does not mention — but it is not
/// binary-compatible, and the distinction is the whole point of this number.
/// `servo_webview_builder_set_delegate` takes a `ServoWebViewDelegate` *by
/// value*, so a caller built against a smaller header pushes fewer bytes than
/// this crate reads, and the difference is read out of whatever happened to be
/// next to it: a function pointer assembled from adjacent stack. Nothing
/// diagnoses that, which is why appending to such a struct bumps this.
///
/// A struct only ever reached behind a pointer can grow without a bump, since
/// the callee reads only fields the caller also knows about.
///
/// Embedders should call [`servo_capi_abi_version`] once, before constructing
/// any struct defined by this crate, and refuse to proceed if it disagrees
/// with the version their own headers were generated against, rather than
/// reading, or passing, a struct whose shape they have guessed wrong.
///
/// # History
///
/// - **5** — `ServoWebViewDelegate` grew from 144 to 152 bytes with
///   `notify_frame_changed`. `ServoPermissionRequest` (48 to 64) and
///   `ServoEmbedderControl` (40 to 56) gained requester and top-level browsing
///   context ids; both are passed by pointer, so they alone would not have moved it.
/// - **4** — `ServoWebViewDelegate` grew from 136 to 144 bytes with
///   `request_clipboard_access`.
/// - **3** — `ServoWebViewDelegate` grew from 128 to 136 bytes with
///   `request_permission`.
/// - **2** — `ServoWebViewDelegate` grew from 64 to 128 bytes across several
///   additions. Version 1 was still reported throughout, which left a
///   by-value caller nothing to check against; that is the bug this policy
///   now exists to prevent.
/// - **1** — the first version reported.
pub const SERVO_CAPI_ABI_VERSION: u32 = 5;

/// Returns the ABI version of this capi build. See [`SERVO_CAPI_ABI_VERSION`].
#[unsafe(no_mangle)]
pub extern "C" fn servo_capi_abi_version() -> u32 {
    SERVO_CAPI_ABI_VERSION
}

/// This build has upstream's multiprocess support compiled in, so
/// `servo_options_set_multiprocess` has an effect and
/// [`servo_content_process_main`](crate::servo::servo_content_process_main) can serve
/// a content process rather than reporting that it cannot.
pub const SERVO_CAPI_FEATURE_MULTIPROCESS: u64 = 1 << 0;

/// This build recognises `dom_embedder_request_enabled` as a preference name, so
/// passing it to `servo_preferences_set_bool` configures the page-request channel
/// (`EmbedderRequest.send` on the page side, delivered on `on_control` tagged
/// `SERVO_EMBEDDER_CONTROL_TAG_PAGE_REQUEST`) rather than panicking.
///
/// Every preference name `servo_preferences_set_*` does not recognise is a panic,
/// not a no-op — there is no graceful way to probe a preference's existence except
/// by setting it, which is exactly the call this bit exists to make safe to skip.
/// Unlike [`SERVO_CAPI_FEATURE_MULTIPROCESS`] this is not behind a Cargo feature; it
/// is unconditionally set in every build from the commit that added it onward, so
/// its only job is distinguishing those builds from older ones that predate it.
pub const SERVO_CAPI_FEATURE_EMBEDDER_REQUEST: u64 = 1 << 1;

/// Returns a bitmask of the `SERVO_CAPI_FEATURE_*` bits this build was compiled with.
///
/// Two payloads built from the same source with different features are the same set of
/// filenames and cannot be told apart by inspecting them. This is how an embedder asks
/// which one it has, at load, rather than finding out when the behaviour it depends on
/// turns out to be absent — for multiprocess, that would be the first content process
/// failing to start.
///
/// Bits outside the `SERVO_CAPI_FEATURE_*` set this build knows about are zero, and new
/// bits may be defined later. An embedder should test the bits it cares about and ignore
/// the rest rather than comparing the whole mask.
///
/// This is a function and defines no struct, so it does not participate in
/// [`SERVO_CAPI_ABI_VERSION`]: a payload too old to export it at all cannot be
/// distinguished from one reporting no features, and an embedder that resolves it
/// optionally should treat its absence as "unknown" rather than as "no features".
#[unsafe(no_mangle)]
pub extern "C" fn servo_capi_build_features() -> u64 {
    #[allow(unused_mut)]
    let mut features: u64 = 0;

    #[cfg(feature = "multiprocess")]
    {
        features |= SERVO_CAPI_FEATURE_MULTIPROCESS;
    }

    features |= SERVO_CAPI_FEATURE_EMBEDDER_REQUEST;

    features
}

const _: () = assert!(
    size_of::<ServoEventLoopWaker>() == 16,
    "ServoEventLoopWaker must stay 16 bytes wide"
);

#[cfg(test)]
mod tests {
    use super::ServoEventLoopWaker;
    use crate::rendering_context::ServoFrameInfo;
    use crate::webview_delegate::{
        ServoEmbedderControl, ServoEmbedderController, ServoHttpHeader, ServoPermissionRequest,
        ServoResourceRequest, ServoWebViewDelegate,
    };

    /// Generic embedder control extension, no domain logic.
    ///
    /// These sizes are part of the ABI that embedders compile against, so they are
    /// pinned here as well as by the `const` assertions next to each struct.
    #[test]
    fn abi_struct_sizes() {
        println!(
            "waker={} delegate={} controller={} control={} permission={}",
            size_of::<ServoEventLoopWaker>(),
            size_of::<ServoWebViewDelegate>(),
            size_of::<ServoEmbedderController>(),
            size_of::<ServoEmbedderControl>(),
            size_of::<ServoPermissionRequest>()
        );

        assert_eq!(size_of::<ServoEventLoopWaker>(), 16);
        assert_eq!(size_of::<ServoWebViewDelegate>(), 152);
        assert_eq!(size_of::<ServoEmbedderController>(), 16);
        assert_eq!(size_of::<ServoEmbedderControl>(), 56);
        assert_eq!(size_of::<ServoPermissionRequest>(), 64);
        assert_eq!(size_of::<ServoFrameInfo>(), 32);
        assert_eq!(size_of::<ServoHttpHeader>(), 32);
        assert_eq!(size_of::<ServoResourceRequest>(), 72);
    }

    /// The feature mask must say what this build actually is, in both arms: an
    /// embedder refusing to run multiprocess on a single-process payload depends on
    /// the bit being absent, and that is the arm a default build never exercises.
    #[test]
    fn build_features_match_this_build() {
        let features = crate::servo_capi_build_features();
        println!("build features = {features:#x}");

        let multiprocess = features & crate::SERVO_CAPI_FEATURE_MULTIPROCESS != 0;
        assert_eq!(multiprocess, cfg!(feature = "multiprocess"));

        // Unconditional in every build that knows about it at all, unlike
        // multiprocess: see SERVO_CAPI_FEATURE_EMBEDDER_REQUEST's own doc comment.
        assert_ne!(features & crate::SERVO_CAPI_FEATURE_EMBEDDER_REQUEST, 0);

        // No bit outside the defined set.
        let known = crate::SERVO_CAPI_FEATURE_MULTIPROCESS | crate::SERVO_CAPI_FEATURE_EMBEDDER_REQUEST;
        assert_eq!(features & !known, 0);
    }

    /// Pins the ABI version, so that it moves only with a by-value struct change. The
    /// page-request control changed no struct and left it at 2; `request_permission`
    /// grew the delegate and moved it to 3; `request_clipboard_access` moved it to 4;
    /// `notify_frame_changed` moved it to 5.
    #[test]
    fn abi_version_pinned() {
        assert_eq!(crate::servo_capi_abi_version(), 5);
    }

    /// Renaming tags 4 and 5 kept their numbers and their flag bits.
    #[test]
    fn renamed_tags_keep_their_numbers() {
        use crate::webview_delegate::{
            SERVO_EMBEDDER_CONTROL_HOST_REQUEST, SERVO_EMBEDDER_CONTROL_PAGE_REQUEST,
            ServoEmbedderControlTag,
        };

        assert_eq!(ServoEmbedderControlTag::PageRequest as u32, 4);
        assert_eq!(ServoEmbedderControlTag::HostRequest as u32, 5);
        assert_eq!(SERVO_EMBEDDER_CONTROL_PAGE_REQUEST, 1 << (4 - 1));
        assert_eq!(SERVO_EMBEDDER_CONTROL_HOST_REQUEST, 1 << (5 - 1));
    }
}
