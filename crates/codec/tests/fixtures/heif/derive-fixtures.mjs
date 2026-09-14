// Container-only fixture transformations; the HEVC bitstreams are never encoded.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root = path.dirname(fileURLToPath(import.meta.url));
const box = (type, data) => {
  const header = Buffer.alloc(8);
  header.writeUInt32BE(data.length + 8);
  header.write(type, 4);
  return Buffer.concat([header, data]);
};
const children = data => {
  const result = [];
  for (let offset = 0; offset < data.length;) {
    const size = data.readUInt32BE(offset);
    if (size < 8 || offset + size > data.length) throw Error('invalid source box');
    result.push({type: data.toString('ascii', offset + 4, offset + 8), data: Buffer.from(data.subarray(offset + 8, offset + size))});
    offset += size;
  }
  return result;
};
const portraitExifTiff = Buffer.from(
  '49492a00080000000200120103000100000006000000698704000100000026000000' +
  '00000000010003900200140000003800000000000000323032343a30333a303420' +
  '30353a30363a303700',
  'hex',
);
function addExifMetadata(boxes, primaryItemId, tiff) {
  const itemId = 3;
  const raw = Buffer.concat([Buffer.alloc(4), tiff]);
  const idat = boxes.find(b => b.type === 'idat');
  const extentOffset = idat.data.length;
  idat.data = Buffer.concat([idat.data, raw]);

  const iloc = boxes.find(b => b.type === 'iloc');
  const location = Buffer.alloc(20);
  location.writeUInt16BE(itemId, 0);
  location.writeUInt16BE(1, 2); // Version 1 construction method: idat.
  location.writeUInt16BE(1, 10);
  location.writeUInt32BE(extentOffset, 12);
  location.writeUInt32BE(raw.length, 16);
  iloc.data.writeUInt16BE(iloc.data.readUInt16BE(6) + 1, 6);
  iloc.data = Buffer.concat([iloc.data, location]);

  const iinf = boxes.find(b => b.type === 'iinf');
  const info = Buffer.concat([
    Buffer.from([2, 0, 0, 0]),
    Buffer.from([0, itemId, 0, 0]),
    Buffer.from('Exif\0'),
  ]);
  iinf.data.writeUInt16BE(iinf.data.readUInt16BE(4) + 1, 4);
  iinf.data = Buffer.concat([iinf.data, box('infe', info)]);

  const iref = boxes.find(b => b.type === 'iref');
  const reference = Buffer.alloc(6);
  reference.writeUInt16BE(itemId, 0);
  reference.writeUInt16BE(1, 2);
  reference.writeUInt16BE(primaryItemId, 4);
  iref.data = Buffer.concat([iref.data, box('cdsc', reference)]);
}
function derive(bytes, mode) {
  const top = children(bytes);
  const meta = top.find(b => b.type === 'meta');
  const originalMetaLength = meta.data.length;
  const boxes = children(meta.data.subarray(4));
  const iprp = boxes.find(b => b.type === 'iprp');
  const properties = children(iprp.data);
  const ipco = properties.find(b => b.type === 'ipco');
  const ipma = properties.find(b => b.type === 'ipma');
  const props = children(ipco.data);
  if (mode === 'grid') {
    const grid = boxes.find(b => b.type === 'idat').data;
    grid[3] = 1; // Two columns, with distinct items sharing the coded bytes.
    grid.writeUInt16BE(93, 4);
    props[2].data.writeUInt32BE(93, 4);
    const iref = boxes.find(b => b.type === 'iref');
    const refs = children(iref.data.subarray(4));
    refs[0].data.writeUInt16BE(2, 2);
    refs[0].data = Buffer.concat([refs[0].data.subarray(0,6), Buffer.from([0, 3])]);
    iref.data = Buffer.concat([iref.data.subarray(0,4), ...refs.map(b => box(b.type,b.data))]);
    const iloc = boxes.find(b => b.type === 'iloc');
    const tileLocation = Buffer.from(iloc.data.subarray(8,28));
    tileLocation.writeUInt16BE(3);
    iloc.data.writeUInt16BE(3,6);
    iloc.data = Buffer.concat([iloc.data,tileLocation]);
    const iinf = boxes.find(b => b.type === 'iinf');
    const entries = children(iinf.data.subarray(6));
    const tileInfo = Buffer.from(entries[0].data);
    tileInfo.writeUInt16BE(3,4);
    iinf.data.writeUInt16BE(3,4);
    iinf.data = Buffer.concat([iinf.data,box('infe',tileInfo)]);
    const tileProperties = Buffer.from(ipma.data.subarray(8,13));
    tileProperties.writeUInt16BE(3);
    ipma.data.writeUInt32BE(3,4);
    ipma.data = Buffer.concat([ipma.data,tileProperties]);
  } else {
    if (mode === 'rotate') {
      boxes.find(b => b.type === 'idat').data.writeUInt16BE(28,4);
      props[2].data.writeUInt32BE(28,4);
    }
    props.push(mode === 'rotate'
      ? {type:'irot', data:Buffer.from([1])}
      : {type:'colr', data:Buffer.from([110,99,108,120,0,12,0,13,0,6,128])});
    // The final association is the primary grid item (ID 2).
    ipma.data[ipma.data.length - 3] += 1;
    ipma.data = Buffer.concat([ipma.data, Buffer.from([props.length | 128])]);
    if (mode === 'rotate') addExifMetadata(boxes, 2, portraitExifTiff);
  }
  ipco.data = Buffer.concat(props.map(b => box(b.type,b.data)));
  iprp.data = Buffer.concat(properties.map(b => box(b.type,b.data)));
  meta.data = Buffer.concat([meta.data.subarray(0,4), ...boxes.map(b => box(b.type,b.data))]);
  const delta = meta.data.length - originalMetaLength;
  const iloc = boxes.find(b => b.type === 'iloc');
  // These pinned sources use iloc v1: the first item's 32-bit base offset
  // points into mdat, while the second item uses idat-relative construction.
  iloc.data.writeUInt32BE(iloc.data.readUInt32BE(14) + delta, 14);
  if (mode === 'grid') iloc.data.writeUInt32BE(iloc.data.readUInt32BE(54) + delta, 54);
  meta.data = Buffer.concat([meta.data.subarray(0,4), ...boxes.map(b => box(b.type,b.data))]);
  return Buffer.concat(top.map(b => box(b.type,b.data)));
}
const eight = fs.readFileSync(path.join(root, 'iphone-8bit.heic'));
if (process.argv[2]) {
  const ten = fs.readFileSync(process.argv[2]);
  fs.writeFileSync(path.join(root, 'iphone-10bit-grid.heic'), derive(ten,'grid'));
}
fs.writeFileSync(path.join(root, 'portrait-rotated.heic'), derive(eight,'rotate'));
fs.writeFileSync(path.join(root, 'display-p3.heic'), derive(eight,'p3'));
fs.writeFileSync(path.join(root, 'truncated.heic'), eight.subarray(0,256));
