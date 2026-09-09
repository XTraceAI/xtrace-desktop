# Dependency license obligations

The dependency policy accepts Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause,
ISC, OFL-1.1, Unicode-DFS-2016, Unicode-3.0, Zlib and MPL-2.0. This covers
the current locked graph; unknown licenses, unsupported exceptions and
expressions requiring a prohibited license still fail. An allowed alternative
in an OR expression is sufficient; every obligation in an AND expression must
be allowed. Keep the Rust and npm policy lists synchronized.

Run `pnpm notices` after dependency changes, then `pnpm notices:check`.
The generated root `THIRD_PARTY_NOTICES.md` preserves upstream license and
copyright texts from the locked Rust graph and production npm packages.
Font notices remain alongside the bundled fonts.

## MPL source access

Each MPL-covered Rust package in the generated notices includes a versioned
crates.io source-archive link. `Cargo.lock` records the archive checksum.
These dependencies are consumed from their unmodified registry archives;
changes to MPL-covered files require corresponding modified source availability.
MPL obligations apply to covered files, including modifications, and do not
require unrelated application files to use MPL.

Before distributing a binary, ship these notices and this document with it,
make source-access information visible to recipients, verify that each linked
archive is available and matches the candidate lockfile, and retain a copy for
source fulfillment. If a covered dependency is patched or its source becomes
unavailable, provide the actual corresponding source, including modifications,
and update the source location before distribution. New MPL npm or non-registry
Rust dependencies require an explicit corresponding-source location before
release; the current automatic links cover registry Rust packages only.
Release acceptance must inspect the packaged notices and source information,
not just the checkout. This CI policy does not complete release packaging.

See the [Mozilla MPL FAQ](https://www.mozilla.org/en-US/MPL/2.0/FAQ/) for
binary distribution and modified-file obligations.
