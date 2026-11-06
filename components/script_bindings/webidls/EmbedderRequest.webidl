/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// This interface is internal to Servo. It is not a web standard and is exposed only
// when the embedder turns on `dom_embedder_request_enabled`.
//
// `send()` delivers a copy of `data` to the embedder as a page-request embedder control
// and resolves with the embedder's answer, which may be empty. It rejects if the
// embedder declines, does not take page requests, or drops the request unanswered.
// Servo never parses either side's bytes.
//
// WebAssembly reaches it as an ordinary JavaScript import; there is no
// WebAssembly-specific path.

[Exposed=Window, Pref="dom_embedder_request_enabled"]
namespace EmbedderRequest {
  Promise<Uint8Array> send(BufferSource data);
};
