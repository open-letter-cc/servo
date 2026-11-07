<!-- Copyright 2026 Open Letter, Inc. -->
<!-- SPDX-License-Identifier: LicenseRef-OpenLetter-Tools -->
# Notice for the full-surface C ABI library

The shared library built from this crate (`servo.dll`, `libservo.so`,
`libservo.dylib`) is a Larger Work in the sense of the Mozilla Public
License 2.0, section 3.3. It combines two kinds of source:

- **Covered Software under MPL-2.0**: the engine under `components/`, the C
  API crate under `ffi/capi/` (except the file named below), and every other
  file in this tree that carries the MPL header, including the modifications
  this fork makes to those files. Their Source Code Form is this repository.
  `src/lib.rs` in this crate compiles that C API crate's source into this
  library and is itself MPL-2.0.
- **Files under LicenseRef-OpenLetter-Tools** (text in `LICENSES/` at the
  root of this tree): this crate's other files (`src/native.rs`,
  `src/shared_surface.rs`, `src/shared_surface_tests.rs`, `build.rs`,
  `Cargo.toml`, `README.md`), `ffi/capi/clipboard.rs`,
  `components/script/dom/embedderrequest.rs`,
  `components/script_bindings/webidls/EmbedderRequest.webidl`, the C suite
  files under `tests/capi/` that carry that identifier, the workflow and
  export lists under `.github/`, and the vendor module under
  `vendor/motor-controller/` with the IPC crate it carries (MIT/Apache with
  tagged local changes).

No file under one licence contains code from a file under the other. The
MPL-2.0 terms apply to the Covered Software in full and are not altered by
the other licence; the other licence applies to its own files and to the
combination as distributed.
