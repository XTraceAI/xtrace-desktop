// Builds small synthetic .DS_Store files for the disk image layout tests.
export const u32 = (value) => {
  const bytes = Buffer.alloc(4);
  bytes.writeUInt32BE(value);
  return bytes;
};
export const record = (name, code, type, value) =>
  Buffer.concat([
    u32(name.length),
    Buffer.from(name, 'utf16le').swap16(),
    Buffer.from(code, 'latin1'),
    Buffer.from(type, 'latin1'),
    value,
  ]);
export const blob = (bytes) => Buffer.concat([u32(bytes.length), bytes]);
export const iloc = (name, x, y) =>
  record(name, 'Iloc', 'blob', blob(Buffer.concat([u32(x), u32(y), Buffer.alloc(8, 0xff)])));

// Writes a .DS_Store the way Finder lays it out: a buddy-allocated file whose
// "DSDB" tree either holds every record in one leaf or splits them under a root.
export function dsStore(records, { split = false } = {}) {
  const size = 4096;
  const nodes = split
    ? [
        Buffer.concat([u32(4), u32(1), u32(3), records[1]]),
        Buffer.concat([u32(0), u32(1), records[0]]),
        Buffer.concat([u32(0), u32(records.length - 2), ...records.slice(2)]),
      ]
    : [Buffer.concat([u32(0), u32(records.length), ...records])];
  const blocks = 2 + nodes.length;
  const offsets = Array.from({ length: 256 }, (_, i) => (i < blocks ? (size * (i + 1)) | 12 : 0));
  const root = Buffer.concat([
    u32(blocks),
    u32(0),
    ...offsets.map(u32),
    u32(1),
    Buffer.from([4]),
    Buffer.from('DSDB'),
    u32(1),
  ]);
  const tree = Buffer.concat([
    u32(2),
    u32(split ? 1 : 0),
    u32(records.length),
    u32(nodes.length),
    u32(size),
  ]);
  const file = Buffer.alloc(4 + size * (blocks + 1));
  file.writeUInt32BE(1, 0);
  file.write('Bud1', 4, 'latin1');
  file.writeUInt32BE(size, 8);
  file.writeUInt32BE(root.length, 12);
  file.writeUInt32BE(size, 16);
  [root, tree, ...nodes].forEach((block, i) => block.copy(file, 4 + size * (i + 1)));
  return file;
}
