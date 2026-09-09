"""Reproduce static fonts from the pinned, licensed sources in fonts/manifest.json."""

import hashlib
import io
import json
import sys
import urllib.request
from pathlib import Path

import fontTools
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

if fontTools.__version__ != "4.64.0":
    raise SystemExit("Use fonttools[woff]==4.64.0; see public/fonts/README.md")
if sys.argv[1:] not in ([], ["--check"]):
    raise SystemExit("Usage: vendor-fonts.py [--check]")
check = sys.argv[1:] == ["--check"]
destination = Path(__file__).resolve().parent.parent / "public" / "fonts"
manifest = json.loads((destination / "manifest.json").read_text())

for family in manifest.values():
    source = urllib.request.urlopen(family["source"], timeout=30).read()
    if hashlib.sha256(source).hexdigest() != family["sourceSha256"]:
        raise SystemExit("Pinned source hash does not match")
    for filename, expected in family["files"].items():
        weight = int(filename.split(".")[0].split("-")[-1])
        font = TTFont(io.BytesIO(source), recalcTimestamp=False)
        static = instantiateVariableFont(font, {"wght": weight}, updateFontNames=True)
        static.recalcTimestamp = False
        static.flavor = "woff2" if filename.endswith(".woff2") else None
        output = io.BytesIO()
        static.save(output)
        data = output.getvalue()
        if hashlib.sha256(data).hexdigest() != expected:
            raise SystemExit(f"Generated hash does not match: {filename}")
        path = destination / filename
        if check:
            if path.read_bytes() != data:
                raise SystemExit(f"Vendored bytes differ: {filename}")
        else:
            path.write_bytes(data)
print("All 16 static font files match their pinned hashes.")
