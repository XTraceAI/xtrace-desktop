# Original upstream notices

Some locked Rust crates omit their upstream license files from the package.
`manifest.json` associates each affected package/version with its source commit
from `.cargo_vcs_info.json`, original URL and SHA-256. The generator verifies
that revision and the vendored text before including it alongside cargo-about's
full selected license text. It never substitutes an invented copyright notice.

Files are unmodified upstream notices, except the explicitly identified leading
MPL notice from `selectors/lib.rs`; that entry records the full source hash and
the exact excerpt. The objc2 notice links to license texts, so cargo-about's
full license text is also necessary. Runtime generation is offline for these
vendored notices; updating a dependency requires reviewing its new provenance.
