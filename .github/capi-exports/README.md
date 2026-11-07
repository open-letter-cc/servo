<!-- Copyright 2026 Open Letter, Inc. -->
<!-- SPDX-License-Identifier: LicenseRef-OpenLetter-Tools -->
# C ABI export lists

The names the Motor Controller workflow checks in every built payload, shared by
each platform's export-surface step so the lists cannot drift apart. One symbol
per line; blank lines and lines starting with `#` are ignored.

- `required.txt`: absence fails the embedder's library load outright. Naming
  these is pure correctness.
- `optional.txt`: resolved if present, skipped if absent, so an older payload
  still loads. Naming one is a promise to ship it forever, made deliberately.
- `planned.txt`: a sequencing tripwire. A symbol listed here that appears in
  the payload without being in `required.txt` or `optional.txt` fails the job,
  so the commit that implements an entry point is the commit that classifies it.

`check.sh` is the gate for ELF and Mach-O payloads. The Windows job applies the
same logic in PowerShell against the PE export directory. The floor (a minimum
count, never an equality) is passed in by each job.
