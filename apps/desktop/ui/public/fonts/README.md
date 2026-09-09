# Bundled fonts

Manrope 400/500/600/700/800 and Geist Mono 400/500/600 are bundled as static
WOFF2 (UI) and TTF (future SVG renderer). Both formats contain the source's full
glyph coverage. They require no runtime font provider or network access.

Sources are Google Fonts' official repository, pinned by commit and SHA-256 in
`manifest.json`. Copyright and SIL Open Font License 1.1 text accompany each
family as `Manrope-OFL.txt` and `GeistMono-OFL.txt`. No reserved font names are
declared in these licenses. The transformation instantiates only the weight
axis, updates static weight/style names, and encodes WOFF2; it does not redraw
glyphs. The 16 binaries total about 1 MB.

Reproduce or verify from the UI directory using an isolated Python environment:

```sh
python3 -m venv /tmp/xtrace-font-tools
/tmp/xtrace-font-tools/bin/pip install 'fonttools[woff]==4.64.0' 'brotli==1.2.0' 'zopfli==0.4.3'
/tmp/xtrace-font-tools/bin/python scripts/vendor-fonts.py --check
```

Omit `--check` to restore matching binaries. Source and generated hashes are
verified before a file is written. `recalcTimestamp=False` keeps the upstream
font timestamps, making repeated generation byte-identical.
