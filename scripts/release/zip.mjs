import fs from "node:fs/promises";
import { createReadStream } from "node:fs";
import { createHash } from "node:crypto";
import { createDeflateRaw, createInflateRaw, crc32 } from "node:zlib";
import { Transform, Writable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { safeName, walkFiles } from "./paths.mjs";

// Deliberate ZIP32 limits; fail before promotion rather than truncate or buffer large files.
export const MAX_FILE = 2 ** 31 - 1;
const MAX_ZIP = 0xffffffff - 1;
const utf8 = new TextDecoder("utf-8", { fatal: true });
const check = (condition, message) => {
  if (!condition) throw new Error(`ZIP校验失败：${message}`);
};

export async function createZip(root, directory, zipPath) {
  const files = await walkFiles(root, directory);
  check(files.length > 0 && files.length < 65535, "文件数必须在1至65534之间");
  const handle = await fs.open(zipPath, "wx");
  let position = 0;
  const entries = [];
  async function append(buffer) {
    check(position + buffer.length <= MAX_ZIP, "归档超过ZIP32上限");
    let written = 0;
    while (written < buffer.length) {
      const result = await handle.write(
        buffer,
        written,
        buffer.length - written,
        position,
      );
      check(result.bytesWritten > 0, "文件写入无进展");
      written += result.bytesWritten;
      position += result.bytesWritten;
    }
  }
  try {
    for (const file of files) {
      check(file.bytes <= MAX_FILE, `${file.name}超过2GiB单文件限额`);
      const name = Buffer.from("FluxDL/" + file.name, "utf8");
      check(name.length <= 65535, "文件名过长");
      const offset = position;
      const header = Buffer.alloc(30);
      header.writeUInt32LE(0x04034b50);
      header.writeUInt16LE(20, 4);
      header.writeUInt16LE(0x800, 6); // UTF-8, no encryption/data descriptors
      header.writeUInt16LE(8, 8);
      header.writeUInt16LE(33, 12); // 1980-01-01, deterministic DOS date
      header.writeUInt16LE(name.length, 26);
      await append(header);
      await append(name);
      let crc = 0;
      let bytes = 0;
      const start = position;
      await pipeline(
        createReadStream(file.full),
        new Transform({
          transform(chunk, _, done) {
            bytes += chunk.length;
            if (bytes > MAX_FILE)
              return done(new Error("ZIP输入超过单文件限额"));
            crc = crc32(chunk, crc);
            done(null, chunk);
          },
        }),
        createDeflateRaw(),
        new Writable({
          write(chunk, _, done) {
            append(chunk).then(() => done(), done);
          },
        }),
      );
      check(bytes === file.bytes, `打包时源文件大小变化：${file.name}`);
      const compressed = position - start;
      const patch = Buffer.alloc(12);
      patch.writeUInt32LE(crc);
      patch.writeUInt32LE(compressed, 4);
      patch.writeUInt32LE(bytes, 8);
      check(
        (await handle.write(patch, 0, patch.length, offset + 14))
          .bytesWritten === patch.length,
        "头部写入不完整",
      );
      entries.push({ name, offset, crc, compressed, bytes });
    }
    const centralStart = position;
    for (const entry of entries) {
      const header = Buffer.alloc(46);
      header.writeUInt32LE(0x02014b50);
      header.writeUInt16LE(20, 4);
      header.writeUInt16LE(20, 6);
      header.writeUInt16LE(0x800, 8);
      header.writeUInt16LE(8, 10);
      header.writeUInt16LE(33, 14);
      header.writeUInt32LE(entry.crc, 16);
      header.writeUInt32LE(entry.compressed, 20);
      header.writeUInt32LE(entry.bytes, 24);
      header.writeUInt16LE(entry.name.length, 28);
      header.writeUInt32LE(entry.offset, 42);
      await append(header);
      await append(entry.name);
    }
    const end = Buffer.alloc(22);
    end.writeUInt32LE(0x06054b50);
    end.writeUInt16LE(entries.length, 8);
    end.writeUInt16LE(entries.length, 10);
    end.writeUInt32LE(position - centralStart, 12);
    end.writeUInt32LE(centralStart, 16);
    await append(end);
    await handle.sync();
  } finally {
    await handle.close();
  }
}

// Decompress every entry through a bounded stream, checking CRC32, size and SHA256.
// Strictly accept this workflow's ZIP profile; no extraction paths are followed.
export async function verifyZip(zipPath, expected) {
  const handle = await fs.open(zipPath, "r");
  const size = (await handle.stat()).size;
  async function read(position, length) {
    check(
      position >= 0 && length >= 0 && position + length <= size,
      "数据越界",
    );
    const buffer = Buffer.alloc(length);
    check(
      (await handle.read(buffer, 0, length, position)).bytesRead === length,
      "数据截断",
    );
    return buffer;
  }
  try {
    check(size >= 22 && size <= MAX_ZIP, "不支持的归档大小");
    const end = await read(size - 22, 22);
    check(
      end.readUInt32LE() === 0x06054b50 && end.readUInt16LE(20) === 0,
      "结束记录无效",
    );
    check(end.readUInt16LE(4) === 0 && end.readUInt16LE(6) === 0, "不允许分卷");
    const count = end.readUInt16LE(10);
    check(
      count === end.readUInt16LE(8) && count === expected.size && count < 65535,
      "条目数量不同",
    );
    const centralSize = end.readUInt32LE(12);
    const centralStart = end.readUInt32LE(16);
    check(centralStart + centralSize === size - 22, "中央目录位置无效");
    let pos = centralStart;
    let nextLocal = 0;
    const seen = new Set();
    for (let i = 0; i < count; i++) {
      const header = await read(pos, 46);
      check(header.readUInt32LE() === 0x02014b50, "中央目录头无效");
      const flags = header.readUInt16LE(8);
      const method = header.readUInt16LE(10);
      const crc = header.readUInt32LE(16);
      const compressed = header.readUInt32LE(20);
      const bytes = header.readUInt32LE(24);
      const nameLength = header.readUInt16LE(28);
      check(
        flags === 0x800 && method === 8 && bytes <= MAX_FILE && compressed > 0,
        "不支持的条目格式或大小",
      );
      check(
        header.readUInt16LE(30) === 0 &&
          header.readUInt16LE(32) === 0 &&
          header.readUInt16LE(34) === 0 &&
          header.readUInt32LE(38) === 0,
        "拒绝extra、注释、分卷或链接属性",
      );
      const nameBuffer = await read(pos + 46, nameLength);
      const fullName = safeName(utf8.decode(nameBuffer));
      check(fullName.startsWith("FluxDL/"), "顶层必须为FluxDL/");
      const name = fullName.slice(7);
      check(
        expected.has(name) && !seen.has(name.toLowerCase()),
        `额外或重复条目：${name}`,
      );
      seen.add(name.toLowerCase());
      check(
        header.readUInt32LE(42) === nextLocal,
        "本地条目重叠、缺失或夹带数据",
      );
      const local = await read(nextLocal, 30);
      check(
        local.readUInt32LE() === 0x04034b50 &&
          local.readUInt16LE(6) === flags &&
          local.readUInt16LE(8) === method &&
          local.readUInt32LE(14) === crc &&
          local.readUInt32LE(18) === compressed &&
          local.readUInt32LE(22) === bytes &&
          local.readUInt16LE(26) === nameLength &&
          local.readUInt16LE(28) === 0,
        "本地头与中央目录不一致",
      );
      check(
        (await read(nextLocal + 30, nameLength)).equals(nameBuffer),
        "条目名不一致",
      );
      const start = nextLocal + 30 + nameLength;
      nextLocal = start + compressed;
      check(nextLocal <= centralStart, "压缩数据越界");
      let actualCrc = 0;
      let actualSize = 0;
      const hash = createHash("sha256");
      await pipeline(
        createReadStream(zipPath, { start, end: nextLocal - 1 }),
        createInflateRaw(),
        new Writable({
          write(chunk, _, done) {
            actualSize += chunk.length;
            if (actualSize > bytes)
              return done(new Error("ZIP解压数据超过声明大小"));
            actualCrc = crc32(chunk, actualCrc);
            hash.update(chunk);
            done();
          },
        }),
      );
      check(
        actualCrc === crc && actualSize === bytes,
        `CRC或大小错误：${name}`,
      );
      const expectedFile = expected.get(name);
      check(
        expectedFile.bytes === bytes &&
          expectedFile.sha256 === hash.digest("hex"),
        `SHA256不一致：${name}`,
      );
      pos += 46 + nameLength;
    }
    check(
      nextLocal === centralStart && pos === size - 22,
      "存在未校验的归档数据",
    );
    return {
      files: count,
      crc32: "passed",
      sha256: "passed",
      topLevel: "FluxDL/",
    };
  } finally {
    await handle.close();
  }
}
