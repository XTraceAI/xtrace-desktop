// Derive the macOS menu-bar template icon from the committed source mark.
// A template image is black with alpha; AppKit tints it for the menu bar's
// appearance. The mark's colored tiles become opaque and its white shapes
// transparent, box-filtered down to 36x36 (18 points at 2x).
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { deflateSync, inflateSync } from 'node:zlib';

const root = fileURLToPath(new URL('../', import.meta.url));
const SIZE = 36;
const source = await readFile(join(root, 'apps/desktop/ui/public/mark.png'));

function chunks(png) {
  if (png.toString('latin1', 1, 4) !== 'PNG') throw new Error('Not a PNG.');
  const out = [];
  for (let offset = 8; offset < png.length;) {
    const length = png.readUInt32BE(offset);
    out.push({
      type: png.toString('latin1', offset + 4, offset + 8),
      data: png.subarray(offset + 8, offset + 8 + length),
    });
    offset += 12 + length;
  }
  return out;
}

function decode(png) {
  const list = chunks(png);
  const header = list.find((chunk) => chunk.type === 'IHDR').data;
  const width = header.readUInt32BE(0);
  const height = header.readUInt32BE(4);
  const [depth, colorType, , , interlace] = header.subarray(8, 13);
  if (depth !== 8 || interlace !== 0) throw new Error('Unsupported PNG layout.');
  const channels = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 }[colorType];
  const palette = list.find((chunk) => chunk.type === 'PLTE')?.data;
  const alphas = list.find((chunk) => chunk.type === 'tRNS')?.data;
  const raw = inflateSync(Buffer.concat(list.filter((c) => c.type === 'IDAT').map((c) => c.data)));
  const stride = width * channels;
  const pixels = new Uint8Array(width * height * 4);
  let previous = new Uint8Array(stride);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const row = new Uint8Array(stride);
    for (let i = 0; i < stride; i++) {
      const a = i >= channels ? row[i - channels] : 0;
      const b = previous[i];
      const c = i >= channels ? previous[i - channels] : 0;
      const p = a + b - c;
      const paeth =
        Math.abs(p - a) <= Math.abs(p - b) && Math.abs(p - a) <= Math.abs(p - c)
          ? a
          : Math.abs(p - b) <= Math.abs(p - c)
            ? b
            : c;
      row[i] = (line[i] + [0, a, b, (a + b) >> 1, paeth][filter]) & 255;
    }
    for (let x = 0; x < width; x++) {
      const at = (y * width + x) * 4;
      if (colorType === 3) {
        const index = row[x];
        pixels.set(palette.subarray(index * 3, index * 3 + 3), at);
        pixels[at + 3] = alphas && index < alphas.length ? alphas[index] : 255;
      } else if (colorType === 6) pixels.set(row.subarray(x * 4, x * 4 + 4), at);
      else if (colorType === 2) pixels.set([...row.subarray(x * 3, x * 3 + 3), 255], at);
      else throw new Error('Unsupported PNG color type.');
    }
    previous = row;
  }
  return { width, height, pixels };
}

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc = (bytes) => {
  let c = 0xffffffff;
  for (const byte of bytes) c = crcTable[(c ^ byte) & 255] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
function chunk(type, data) {
  const body = Buffer.concat([Buffer.from(type, 'latin1'), data]);
  const out = Buffer.alloc(body.length + 8);
  out.writeUInt32BE(data.length, 0);
  body.copy(out, 4);
  out.writeUInt32BE(crc(body), body.length + 4);
  return out;
}

const { width, height, pixels } = decode(source);
const scanlines = Buffer.alloc(SIZE * (SIZE * 4 + 1));
for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    // Average the tile coverage of every source pixel inside this output pixel.
    const [x0, x1] = [Math.floor((x * width) / SIZE), Math.floor(((x + 1) * width) / SIZE)];
    const [y0, y1] = [Math.floor((y * height) / SIZE), Math.floor(((y + 1) * height) / SIZE)];
    let sum = 0;
    for (let sy = y0; sy < y1; sy++)
      for (let sx = x0; sx < x1; sx++) {
        const at = (sy * width + sx) * 4;
        // White shapes have a full green channel; the tiles' is at most ~150.
        const tile = Math.min(1, Math.max(0, (255 - pixels[at + 1]) / 105));
        sum += (pixels[at + 3] / 255) * tile;
      }
    const at = y * (SIZE * 4 + 1) + 1 + x * 4;
    scanlines[at + 3] = Math.round((sum / ((x1 - x0) * (y1 - y0))) * 255);
  }
}
const header = Buffer.alloc(13);
header.writeUInt32BE(SIZE, 0);
header.writeUInt32BE(SIZE, 4);
header.set([8, 6, 0, 0, 0], 8);
await writeFile(
  join(root, 'apps/desktop/src-tauri/icons/tray-template.png'),
  Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', deflateSync(scanlines, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]),
);
