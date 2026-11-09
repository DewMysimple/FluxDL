import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { spawn } from "node:child_process";
import { pathToFileURL } from "node:url";
import { once } from "node:events";
import {
  OWNER,
  WORK,
  OUTPUT,
  assertNoRecovery,
  createTransaction,
  exists,
  initWork,
  readJson,
  readLock,
  safeName,
  safePath,
  sha256,
  transactions,
  withLock,
  writeJson,
} from "./release/paths.mjs";
import {
  RECEIPT,
  buildDesktop,
  buildInputs,
  configuration,
  context,
  inspectBinaries,
  run,
  sourceState,
} from "./release/source.mjs";
import { inventory, verifyPair } from "./release/package.mjs";
import { createZip } from "./release/zip.mjs";
import {
  releaseBuild,
  releaseClean,
  releaseStage,
  releaseVerify,
} from "./release/workflow.mjs";

// Fixture injection is exclusively an internal test API. Production CLIs never expose it.
async function fixture(t) {
  const root = await fs.realpath(
    await fs.mkdtemp(path.join(os.tmpdir(), "fluxdl-release-测试 (空格)-")),
  );
  await fs.writeFile(path.join(root, ".fixture-owner"), OWNER, { flag: "wx" });
  t.after(async () => {
    await safePath(await fs.realpath(os.tmpdir()), root);
    assert.equal(
      await fs.readFile(path.join(root, ".fixture-owner"), "utf8"),
      OWNER,
    );
    await fs.rm(root, { recursive: true });
  });
  for (const name of [
    "src",
    "src-tauri",
    "licenses/nested 中文",
    "node_modules/@tauri-apps/cli",
  ])
    await fs.mkdir(path.join(root, name), { recursive: true });
  await fs.writeFile(
    path.join(root, ".gitignore"),
    "/src-tauri/target/\n/release-output/\n/.fluxdl-release-work/\n/node_modules/\n",
  );
  await fs.writeFile(
    path.join(root, "src/example.ts"),
    "export const value = 1;\n",
  );
  await fs.writeFile(path.join(root, "LICENSE"), "Fixture MIT license\n");
  await fs.writeFile(
    path.join(root, "THIRD_PARTY_NOTICES.md"),
    "External engines\n",
  );
  await fs.writeFile(
    path.join(root, "licenses/README.md"),
    "Fixture license inventory\n",
  );
  await fs.writeFile(
    path.join(root, "licenses/nested 中文/LICENSE.txt"),
    "Unicode nested license\n",
  );
  await writeJson(
    path.join(root, "node_modules/@tauri-apps/cli/package.json"),
    { version: "2.12.4" },
  );
  const ctx = {
    ...context(root),
    fixture: true,
    runningLegacy: async () => false,
    inspect: async (_root, files, version) => {
      for (const file of files) {
        const data = await fs.readFile(file);
        assert.ok(
          data.length >= 4096 &&
            data
              .subarray(128, 160)
              .toString()
              .startsWith("fixture FluxDL " + version),
          "fixture PE版本/大小无效",
        );
      }
    },
  };
  await setVersion(ctx, "0.2.0");
  await run(root, "git", ["init", "-q"], { capture: true });
  await run(root, "git", ["add", "."], { capture: true });
  await run(
    root,
    "git",
    [
      "-c",
      "user.name=FluxDL Fixture",
      "-c",
      "user.email=fixture@example.invalid",
      "commit",
      "-qm",
      "fixture",
    ],
    { capture: true },
  );
  await seedBuild(ctx, "first");
  return ctx;
}

async function setVersion(ctx, version) {
  await writeJson(path.join(ctx.root, "package.json"), {
    name: "fluxdl",
    version,
  });
  await writeJson(path.join(ctx.root, "package-lock.json"), {
    version,
    packages: { "": { version } },
  });
  await writeJson(path.join(ctx.root, "src-tauri/tauri.conf.json"), {
    version,
    productName: "FluxDL",
    bundle: {
      targets: ["nsis"],
      resources: ["../THIRD_PARTY_NOTICES.md", "../LICENSE", "../licenses/"],
    },
  });
  await fs.writeFile(
    path.join(ctx.root, "src-tauri/Cargo.toml"),
    `[package]\nname = "fluxdl"\nversion = "${version}"\n`,
  );
  await fs.writeFile(
    path.join(ctx.root, "src-tauri/Cargo.lock"),
    `[[package]]\nname = "fluxdl"\nversion = "${version}"\n`,
  );
}

async function seedBuild(ctx, payload) {
  const { version, installer } = await configuration(ctx.root);
  const directory = path.join(ctx.root, "src-tauri/target/release");
  await fs.mkdir(path.join(directory, "bundle/nsis"), { recursive: true });
  const binaries = [
    path.join(directory, "fluxdl.exe"),
    path.join(directory, "bundle/nsis", installer),
  ];
  for (const [index, file] of binaries.entries()) {
    const pe = Buffer.alloc(8192);
    pe.writeUInt16LE(0x5a4d);
    pe.writeUInt32LE(64, 60);
    pe.writeUInt32LE(0x4550, 64);
    pe.writeUInt16LE(index === 0 ? 0x8664 : 0x14c, 68);
    pe.writeUInt16LE(1, 70);
    pe.writeUInt16LE(2, 86);
    pe.writeUInt16LE(index === 0 ? 0x20b : 0x10b, 88);
    pe.write(`fixture FluxDL ${version}`, 128);
    pe.write(payload, 256);
    await fs.writeFile(file, pe);
  }
  const state = await sourceState(ctx.root);
  const now = new Date().toISOString();
  await writeJson(path.join(ctx.root, RECEIPT), {
    schema: 1,
    validationMode: "fixture",
    version,
    gitCommit: state.commit.slice(0, 12),
    gitCommitFull: state.commit,
    gitDirty: state.dirty,
    sourceFingerprint: state.fingerprint,
    buildTimeUtc: now,
    finishedTimeUtc: now,
    files: await Promise.all(
      binaries.map(async (file) => ({
        name: path.basename(file),
        sha256: sha256(await fs.readFile(file)),
      })),
    ),
  });
}

const output = (ctx) => path.join(ctx.root, OUTPUT);
const snapshot = (ctx) => inventory(ctx.root, output(ctx));

test("首次发布；中文/空格/括号路径、ZIP独立解压与全部哈希一致", async (t) => {
  const ctx = await fixture(t);
  await fs.writeFile(
    path.join(ctx.root, "licenses/nested 中文/large.txt"),
    Buffer.alloc(2 * 1024 * 1024, 123),
  );
  await seedBuild(ctx, "unicode");
  const result = await releaseStage(ctx);
  assert.deepEqual((await fs.readdir(output(ctx))).sort(), [
    "FluxDL",
    "FluxDL.zip",
  ]);
  assert.equal(result.archive.crc32, "passed");
  const extracted = path.join(ctx.root, "extracted");
  await run(ctx.root, "python", [
    "-c",
    "import zipfile,sys; z=zipfile.ZipFile(sys.argv[1]); assert z.testzip() is None; z.extractall(sys.argv[2])",
    result.zip,
    extracted,
  ]);
  assert.deepEqual(
    await inventory(ctx.root, path.join(extracted, "FluxDL")),
    await inventory(ctx.root, result.directory),
  );
  assert.deepEqual(await transactions(ctx.root, path.join(ctx.root, WORK)), []);
  await run(
    ctx.root,
    "git",
    ["check-ignore", path.join(output(ctx), "FluxDL.zip")],
    { capture: true },
  );
});

test("同版本覆盖、不同版本覆盖、重复stage均仅保留最新一对产物", async (t) => {
  const ctx = await fixture(t);
  const first = await releaseStage(ctx);
  await seedBuild(ctx, "second-build");
  const second = await releaseStage(ctx);
  assert.notEqual(first.executables[0].sha256, second.executables[0].sha256);
  await setVersion(ctx, "0.3.0");
  await seedBuild(ctx, "new-version");
  await releaseStage(ctx);
  await releaseStage(ctx);
  const result = await releaseVerify(ctx);
  assert.equal(result.version, "0.3.0");
  assert.equal(
    await exists(path.join(result.directory, "FluxDL_0.2.0_x64-setup.exe")),
    false,
  );
  assert.deepEqual(await transactions(ctx.root, path.join(ctx.root, WORK)), []);
});

for (const [name, mutation] of [
  [
    "主EXE缺失",
    (ctx) =>
      fs.unlink(path.join(ctx.root, "src-tauri/target/release/fluxdl.exe")),
  ],
  [
    "安装器缺失",
    (ctx) =>
      fs.unlink(
        path.join(
          ctx.root,
          "src-tauri/target/release/bundle/nsis/FluxDL_0.2.0_x64-setup.exe",
        ),
      ),
  ],
  ["许可证缺失", (ctx) => fs.unlink(path.join(ctx.root, "LICENSE"))],
  [
    "版本不一致",
    (ctx) =>
      writeJson(path.join(ctx.root, "package.json"), {
        name: "fluxdl",
        version: "9.0.0",
      }),
  ],
  [
    "零字节EXE",
    (ctx) =>
      fs.writeFile(
        path.join(ctx.root, "src-tauri/target/release/fluxdl.exe"),
        "",
      ),
  ],
  [
    "源码已变更",
    (ctx) =>
      fs.appendFile(path.join(ctx.root, "src/example.ts"), "\n// changed"),
  ],
  [
    "遗留安装器被替换",
    (ctx) =>
      fs.appendFile(
        path.join(
          ctx.root,
          "src-tauri/target/release/bundle/nsis/FluxDL_0.2.0_x64-setup.exe",
        ),
        "other build",
      ),
  ],
  ["成功收据缺失", (ctx) => fs.unlink(path.join(ctx.root, RECEIPT))],
]) {
  test(`${name}：发布失败且旧目录和ZIP逐文件不变`, async (t) => {
    const ctx = await fixture(t);
    await releaseStage(ctx);
    const before = await snapshot(ctx);
    await mutation(ctx);
    await assert.rejects(releaseStage(ctx));
    assert.deepEqual(await snapshot(ctx), before);
  });
}

test("生产模式拒绝fixture收据、fixture交付和伪PE版本资源", async (t) => {
  const ctx = await fixture(t);
  await assert.rejects(buildInputs(context(ctx.root)), /fixture/u);
  await releaseStage(ctx);
  await assert.rejects(
    verifyPair(context(ctx.root), output(ctx)),
    /非正式来源/u,
  );
  if (process.platform === "win32") {
    await assert.rejects(
      inspectBinaries(
        ctx.root,
        [
          path.join(ctx.root, "src-tauri/target/release/fluxdl.exe"),
          path.join(
            ctx.root,
            "src-tauri/target/release/bundle/nsis/FluxDL_0.2.0_x64-setup.exe",
          ),
        ],
        "0.2.0",
      ),
      /PE产品名/u,
    );
  }
});

test("ZIP生成失败不能覆盖旧产物", async (t) => {
  const ctx = await fixture(t);
  await releaseStage(ctx);
  const before = await snapshot(ctx);
  await seedBuild(ctx, "new");
  await assert.rejects(
    releaseStage({
      ...ctx,
      createZip: async () => {
        throw new Error("injected archive failure");
      },
    }),
    /archive failure/u,
  );
  assert.deepEqual(await snapshot(ctx), before);
});

test("ZIP篡改/目录篡改/额外旧安装器均被只读verify拒绝", async (t) => {
  const ctx = await fixture(t);
  await releaseStage(ctx);
  const zip = path.join(output(ctx), "FluxDL.zip");
  const original = await fs.readFile(zip);
  const damaged = Buffer.from(original);
  damaged[80] ^= 0xff;
  await fs.writeFile(zip, damaged);
  const before = await snapshot(ctx);
  await assert.rejects(releaseVerify(ctx));
  assert.deepEqual(await snapshot(ctx), before);
  await fs.writeFile(zip, original);
  const license = path.join(output(ctx), "FluxDL/LICENSE");
  await fs.appendFile(license, "tampered");
  await assert.rejects(releaseVerify(ctx), /SHA256SUMS/u);
  await fs.writeFile(license, "Fixture MIT license\n");
  await fs.writeFile(
    path.join(output(ctx), "FluxDL/FluxDL_0.1.0_x64-setup.exe"),
    "old",
  );
  await assert.rejects(releaseVerify(ctx), /旧安装器/u);
});

for (const phase of ["backup-moved", "promoted"]) {
  test(`${phase}后失败：回滚旧版并保留last-failure证据`, async (t) => {
    const ctx = await fixture(t);
    await releaseStage(ctx);
    const before = await snapshot(ctx);
    await seedBuild(ctx, "new");
    await assert.rejects(
      releaseStage({
        ...ctx,
        hook: async (current) => {
          if (current === phase) throw new Error(`injected ${phase}`);
        },
      }),
      /已保留或恢复/u,
    );
    assert.deepEqual(await snapshot(ctx), before);
    assert.ok(
      (
        await readJson(path.join(ctx.root, WORK, "last-failure.json"))
      ).message.includes(phase),
    );
    await releaseVerify(ctx);
  });
}

test("替换重命名失败回滚；回滚失败保留唯一备份并可显式恢复", async (t) => {
  const ctx = await fixture(t);
  await releaseStage(ctx);
  const before = await snapshot(ctx);
  await seedBuild(ctx, "next");
  const failure = {
    ...ctx,
    rename: async (from, to) => {
      if (
        path.basename(from) === "candidate" ||
        path.basename(from) === "previous-output"
      )
        throw new Error("injected locked rename");
      return fs.rename(from, to);
    },
  };
  await assert.rejects(releaseStage(failure), (error) => {
    assert.ok(
      error.message.includes("previous-output") &&
        error.message.includes(ctx.root) &&
        error.message.includes("--recover"),
    );
    return true;
  });
  const preview = await releaseClean(ctx);
  assert.equal(preview.candidates.length, 1);
  assert.equal(preview.candidates[0].eligible, false);
  const backup = path.join(preview.candidates[0].path, "previous-output");
  assert.deepEqual(await inventory(ctx.root, backup), before);
  await releaseClean(ctx, { apply: true });
  assert.equal(await exists(backup), true);
  await assert.rejects(releaseStage(ctx), /异常退出残留/u);
  await releaseClean(ctx, { apply: true, recover: true });
  assert.deepEqual(await snapshot(ctx), before);
  await releaseVerify(ctx);
});

test("异常退出staging：默认clean只预览，apply幂等清理且不修改正式输出", async (t) => {
  const ctx = await fixture(t);
  await releaseStage(ctx);
  const before = await snapshot(ctx);
  const base = await initWork(ctx.root);
  const transaction = await createTransaction(ctx.root, base);
  await fs.mkdir(path.join(transaction, "candidate"));
  await fs.writeFile(
    path.join(transaction, "candidate/partial.zip"),
    "interrupted",
  );
  await assert.rejects(assertNoRecovery(ctx.root, base), /残留/u);
  const preview = await releaseClean(ctx);
  assert.equal(preview.mode, "dry-run");
  assert.ok(await exists(transaction));
  assert.equal(preview.deletedBytes, 0);
  const cleaned = await releaseClean(ctx, { apply: true });
  assert.ok(cleaned.deletedBytes > 0);
  assert.equal(await exists(transaction), false);
  assert.deepEqual(await snapshot(ctx), before);
  assert.equal((await releaseClean(ctx, { apply: true })).deletedBytes, 0);
});

test("两个独立进程的发布锁互斥；活锁不能被clean接管", async (t) => {
  const ctx = await fixture(t);
  const moduleUrl = pathToFileURL(
    path.resolve("scripts/release/paths.mjs"),
  ).href;
  const code = `import {withLock} from ${JSON.stringify(moduleUrl)}; await withLock(process.argv[1], async () => { console.log('ready'); await new Promise(r => process.stdin.once('data', r)); });`;
  const child = spawn(
    process.execPath,
    ["--input-type=module", "-e", code, ctx.root],
    { stdio: ["pipe", "pipe", "pipe"], windowsHide: true },
  );
  try {
    await once(child.stdout, "data");
    assert.ok((await readLock(ctx.root)).alive);
    await assert.rejects(releaseStage(ctx), /仍在运行/u);
    await assert.rejects(
      releaseClean(ctx, { apply: true, staleLock: true }),
      /不能接管/u,
    );
  } finally {
    child.stdin.end("done");
    await once(child, "exit");
  }
  await releaseStage(ctx);
});

test("已退出的发布锁必须显式stale-lock；未知所有权不删除", async (t) => {
  const ctx = await fixture(t);
  const base = await initWork(ctx.root);
  const file = path.join(base, "lock.json");
  await writeJson(file, { owner: OWNER, token: "dead-lock", pid: 2147483647 });
  await assert.rejects(
    withLock(ctx.root, async () => {}),
    /已退出/u,
  );
  await releaseClean(ctx);
  assert.ok(await exists(file));
  await releaseClean(ctx, { apply: true, staleLock: true });
  assert.equal(await exists(file), false);
  await fs.mkdir(path.join(base, "unknown-directory"));
  await assert.rejects(releaseClean(ctx, { apply: true }), /未知临时文件/u);
  assert.ok(await exists(path.join(base, "unknown-directory")));
});

test("路径防越界、保留名、Windows junction和ZIP路径逃逸被拒绝", async (t) => {
  const ctx = await fixture(t);
  for (const file of [
    ctx.root,
    path.dirname(ctx.root),
    os.homedir(),
    path.parse(ctx.root).root,
  ])
    await assert.rejects(safePath(ctx.root, file));
  for (const name of [
    "../escape",
    "C:/data",
    "/root",
    "a\\b",
    "NUL.txt",
    "a/../b",
    "trailing.",
  ])
    assert.throws(() => safeName(name));
  const external = await fs.realpath(
    await fs.mkdtemp(path.join(os.tmpdir(), "fluxdl-external-")),
  );
  try {
    await fs.writeFile(path.join(external, "keep.txt"), "untouched");
    const link = path.join(ctx.root, "licenses/external-link");
    await fs.symlink(
      external,
      link,
      process.platform === "win32" ? "junction" : "dir",
    );
    await assert.rejects(
      safePath(ctx.root, path.join(link, "keep.txt")),
      /链接/u,
    );
    await assert.rejects(releaseStage(ctx));
    assert.equal(
      await fs.readFile(path.join(external, "keep.txt"), "utf8"),
      "untouched",
    );
    await fs.unlink(link);
  } finally {
    await safePath(await fs.realpath(os.tmpdir()), external);
    await fs.rm(external, { recursive: true });
  }
});

test("历史release未知文件不删；白名单目录也不删除运行中的程序", async (t) => {
  const ctx = await fixture(t);
  await releaseStage(ctx);
  const legacy = path.join(ctx.root, "release");
  await fs.cp(path.join(output(ctx), "FluxDL"), legacy, { recursive: true });
  await fs.writeFile(path.join(legacy, "keep.txt"), "user file");
  let result = await releaseClean(ctx, { legacy: true, apply: true });
  assert.equal(
    result.candidates.find((item) => item.kind === "legacy").eligible,
    false,
  );
  assert.equal(
    await fs.readFile(path.join(legacy, "keep.txt"), "utf8"),
    "user file",
  );
  for (const name of ["keep.txt", "build-info.json"])
    await fs.unlink(path.join(legacy, name));
  const names = ["FluxDL.exe", "FluxDL_0.2.0_x64-setup.exe"];
  await fs.writeFile(
    path.join(legacy, "SHA256SUMS.txt"),
    (
      await Promise.all(
        names.map(
          async (name) =>
            `${sha256(await fs.readFile(path.join(legacy, name)))}  ${name}`,
        ),
      )
    ).join("\n") + "\n",
  );
  result = await releaseClean(
    { ...ctx, runningLegacy: async () => true },
    { legacy: true, apply: true },
  );
  assert.equal(
    result.candidates.find((item) => item.kind === "legacy").eligible,
    false,
  );
  assert.ok(await exists(legacy));
  await releaseClean(ctx, { legacy: true, apply: true });
  assert.equal(await exists(legacy), false);
  await releaseVerify(ctx);
});

test("完整构建失败不破坏正式输出，且无成功收据可供后续误用", async (t) => {
  const ctx = await fixture(t);
  await releaseStage(ctx);
  const before = await snapshot(ctx);
  const failed = {
    ...ctx,
    run: async (_root, executable, args) => {
      if (args.some((arg) => arg.endsWith("tauri.js")))
        throw new Error("injected Tauri build failure");
      if (executable === "rustc" && args[0] === "-vV")
        return "host: x86_64-pc-windows-msvc";
      return "fixture tool";
    },
  };
  await assert.rejects(releaseBuild(failed), /Tauri build failure/u);
  assert.deepEqual(await snapshot(ctx), before);
  assert.equal(await exists(path.join(ctx.root, RECEIPT)), false);
  await assert.rejects(releaseStage(ctx), /成功构建收据/u);
});

test("desktop:build只更新构建目录和收据，不写正式输出", async (t) => {
  const ctx = await fixture(t);
  await releaseStage(ctx);
  const before = await snapshot(ctx);
  const buildCtx = {
    ...ctx,
    run: async (_root, executable, args) => {
      if (args.some((arg) => arg.endsWith("tauri.js"))) {
        await seedBuild(ctx, "rebuilt");
        return "";
      }
      if (executable === "rustc" && args[0] === "-vV")
        return "host: x86_64-pc-windows-msvc";
      return "fixture tool";
    },
  };
  await buildDesktop(buildCtx);
  assert.deepEqual(await snapshot(ctx), before);
  await releaseStage(ctx);
  assert.notDeepEqual(await snapshot(ctx), before);
});

test(
  "Windows文件占用：不结束持有进程；失败保留旧版或完成可验证替换",
  { skip: process.platform !== "win32" },
  async (t) => {
    const ctx = await fixture(t);
    await releaseStage(ctx);
    const before = await snapshot(ctx);
    const script = path.join(ctx.root, "hold-file.ps1");
    await fs.writeFile(
      script,
      "$f=[IO.File]::Open($env:FLUXDL_FIXTURE_FILE,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read); [Console]::WriteLine('ready'); [Console]::ReadLine() | Out-Null; $f.Dispose()\n",
    );
    const child = spawn(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-File", script],
      {
        env: {
          ...process.env,
          FLUXDL_FIXTURE_FILE: path.join(output(ctx), "FluxDL/FluxDL.exe"),
        },
        stdio: ["pipe", "pipe", "pipe"],
        windowsHide: true,
      },
    );
    try {
      await once(child.stdout, "data");
      await seedBuild(ctx, "next");
      try {
        await releaseStage(ctx);
        await releaseVerify(ctx);
      } catch {
        assert.deepEqual(await snapshot(ctx), before);
      }
      assert.equal(child.exitCode, null);
    } finally {
      child.stdin.end("\n");
      await once(child, "exit");
    }
  },
);

test("ZIP32限额和链接输入在生成阶段拒绝", async (t) => {
  const ctx = await fixture(t);
  const directory = path.join(ctx.root, "large-input");
  await fs.mkdir(directory);
  const file = await fs.open(path.join(directory, "large.bin"), "w");
  await file.truncate(2 ** 31);
  await file.close();
  await assert.rejects(
    createZip(ctx.root, directory, path.join(ctx.root, "large.zip")),
    /2GiB/u,
  );
});

test("已promote后回滚也失败：当前候选和旧备份均保留，显式恢复不合并", async (t) => {
  const ctx = await fixture(t);
  await releaseStage(ctx);
  const before = await snapshot(ctx);
  await seedBuild(ctx, "next-build");
  await assert.rejects(
    releaseStage({
      ...ctx,
      hook: async (phase) => {
        if (phase === "promoted") throw new Error("post-promotion fault");
      },
      rename: async (from, to) => {
        if (path.basename(to) === "rejected-output")
          throw new Error("rollback blocked");
        return fs.rename(from, to);
      },
    }),
    /旧版恢复失败/u,
  );
  assert.notDeepEqual(await snapshot(ctx), before);
  const pending = await transactions(ctx.root, path.join(ctx.root, WORK));
  assert.equal(pending.length, 1);
  assert.deepEqual(
    await inventory(ctx.root, path.join(pending[0], "previous-output")),
    before,
  );
  await releaseClean(ctx, { apply: true, recover: true });
  assert.deepEqual(await snapshot(ctx), before);
});

test("ZIP生成期间源码变化：候选校验虽通过也不得发布", async (t) => {
  const ctx = await fixture(t);
  await releaseStage(ctx);
  const before = await snapshot(ctx);
  await assert.rejects(
    releaseStage({
      ...ctx,
      createZip: async (...args) => {
        await createZip(...args);
        await fs.appendFile(
          path.join(ctx.root, "src/example.ts"),
          "// concurrent edit",
        );
      },
    }),
    /已过期/u,
  );
  assert.deepEqual(await snapshot(ctx), before);
});

test("build-info缺字段或dirty伪造，以及事务内未知文件，均拒绝静默处理", async (t) => {
  const ctx = await fixture(t);
  await releaseStage(ctx);
  const metadata = path.join(output(ctx), "FluxDL/build-info.json");
  const info = await readJson(metadata);
  assert.equal(typeof info.gitDirty, "boolean");
  await writeJson(metadata, { ...info, gitDirty: "false" });
  await assert.rejects(releaseVerify(ctx), /字段不完整/u);
  const transaction = await createTransaction(
    ctx.root,
    await initWork(ctx.root),
  );
  await fs.writeFile(path.join(transaction, "user-note.txt"), "do not delete");
  await assert.rejects(releaseClean(ctx, { apply: true }), /未知文件/u);
  assert.equal(
    await fs.readFile(path.join(transaction, "user-note.txt"), "utf8"),
    "do not delete",
  );
});

test("dev命令没有正式发布生命周期钩子", async () => {
  const pkg = await readJson(path.resolve("package.json"));
  assert.equal(pkg.scripts["desktop:dev"], "tauri dev");
  assert.equal(pkg.scripts.dev, "vite --host 127.0.0.1");
  for (const hook of [
    "predev",
    "postdev",
    "pretest",
    "posttest",
    "postbuild",
    "predesktop:dev",
    "postdesktop:dev",
  ]) {
    assert.equal(pkg.scripts[hook], undefined);
  }
});

test("异常退出损坏journal：仍保留备份，核实哈希后可显式恢复", async (t) => {
  const ctx = await fixture(t);
  await releaseStage(ctx);
  const before = await snapshot(ctx);
  const transaction = await createTransaction(
    ctx.root,
    await initWork(ctx.root),
  );
  await fs.rename(output(ctx), path.join(transaction, "previous-output"));
  await fs.writeFile(path.join(transaction, "journal.json"), "{");
  const preview = await releaseClean(ctx);
  assert.equal(preview.candidates[0].eligible, false);
  await releaseClean(ctx, { apply: true });
  assert.ok(await exists(path.join(transaction, "previous-output")));
  await releaseClean(ctx, { apply: true, recover: true });
  assert.deepEqual(await snapshot(ctx), before);
});

test("故障日志路径变为junction时，记录失败也必须完成旧版回滚", async (t) => {
  const ctx = await fixture(t);
  await releaseStage(ctx);
  const before = await snapshot(ctx);
  const sink = path.join(ctx.root, "diagnostic-sink");
  await fs.mkdir(sink);
  await fs.writeFile(path.join(sink, "keep.txt"), "untouched");
  const link = path.join(ctx.root, WORK, "last-failure.json");
  try {
    await assert.rejects(
      releaseStage({
        ...ctx,
        hook: async (phase) => {
          if (phase !== "backup-moved") return;
          await fs.symlink(
            sink,
            link,
            process.platform === "win32" ? "junction" : "dir",
          );
          throw new Error("injected promotion fault");
        },
      }),
      /已保留或恢复/u,
    );
    assert.deepEqual(await snapshot(ctx), before);
    assert.equal(
      await fs.readFile(path.join(sink, "keep.txt"), "utf8"),
      "untouched",
    );
  } finally {
    if (await exists(link)) await fs.unlink(link);
  }
});

test("空许可证目录不能被ZIP静默遗漏，旧版保持不变", async (t) => {
  const ctx = await fixture(t);
  await releaseStage(ctx);
  const before = await snapshot(ctx);
  await fs.mkdir(path.join(ctx.root, "licenses/empty-license"));
  await assert.rejects(releaseStage(ctx), /空目录/u);
  assert.deepEqual(await snapshot(ctx), before);
});

test("暂存期间旧正式目录出现未知文件：不挪动、不删除用户添加的文件", async (t) => {
  const ctx = await fixture(t);
  const first = await releaseStage(ctx);
  const note = path.join(output(ctx), "FluxDL/keep.txt");
  await seedBuild(ctx, "new-build");
  await assert.rejects(
    releaseStage({
      ...ctx,
      hook: async (phase) => {
        if (phase === "ready") await fs.writeFile(note, "user-added");
      },
    }),
    /非交付文件/u,
  );
  assert.equal(await fs.readFile(note, "utf8"), "user-added");
  assert.equal(
    sha256(await fs.readFile(path.join(first.directory, "FluxDL.exe"))),
    first.executables[0].sha256,
  );
});

test("已备份目录被加入未知文件：成功的新包保留，备份和未知文件不得清理", async (t) => {
  const ctx = await fixture(t);
  await releaseStage(ctx);
  await seedBuild(ctx, "new-build");
  let note;
  const result = await releaseStage({
    ...ctx,
    hook: async (phase) => {
      if (phase !== "backup-moved") return;
      const pending = await transactions(ctx.root, path.join(ctx.root, WORK));
      note = path.join(pending[0], "previous-output/FluxDL/keep.txt");
      await fs.writeFile(note, "user-added");
    },
  });
  assert.equal(result.manifest, "passed");
  assert.equal(await fs.readFile(note, "utf8"), "user-added");
  await assert.rejects(releaseClean(ctx, { apply: true }), /非交付文件/u);
  assert.equal(await fs.readFile(note, "utf8"), "user-added");
});
