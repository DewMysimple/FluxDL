import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { exists, readJson, safePath, sha256, writeJson } from "./paths.mjs";

const helper = fileURLToPath(new URL("./inspect-pe.ps1", import.meta.url));
export const ROOT = path.resolve(
  fileURLToPath(new URL("../../", import.meta.url)),
);
export const RECEIPT = "src-tauri/target/release/.fluxdl-build.json";
const fail = (message) => {
  throw new Error(message);
};

export function run(root, executable, args, { capture = false, input } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, {
      cwd: root,
      shell: false,
      windowsHide: true,
      stdio: capture
        ? ["pipe", "pipe", "pipe"]
        : ["ignore", "inherit", "inherit"],
    });
    let stdout = "";
    let stderr = "";
    if (capture) {
      child.stdout.setEncoding("utf8").on("data", (chunk) => {
        stdout += chunk;
      });
      child.stderr.setEncoding("utf8").on("data", (chunk) => {
        stderr += chunk;
      });
      child.stdin.on("error", () => {});
      child.stdin.end(input);
    }
    child.once("error", reject);
    child.once("exit", (code, signal) => {
      if (code === 0) resolve(stdout.trim());
      else
        reject(
          new Error(
            `${path.basename(executable)} ${args.join(" ")} 失败（${signal ?? code}）${capture ? ": " + stderr : ""}`,
          ),
        );
    });
  });
}

export async function configuration(root) {
  const pkg = await readJson(path.join(root, "package.json"));
  const lock = await readJson(path.join(root, "package-lock.json"));
  const tauri = await readJson(path.join(root, "src-tauri/tauri.conf.json"));
  const cargo = await fs.readFile(
    path.join(root, "src-tauri/Cargo.toml"),
    "utf8",
  );
  const cargoLock = await fs.readFile(
    path.join(root, "src-tauri/Cargo.lock"),
    "utf8",
  );
  const cargoVersion = cargo.match(
    /\[package\][\s\S]*?\bversion\s*=\s*"([^"]+)"/u,
  )?.[1];
  const cargoLockVersion = cargoLock.match(
    /\[\[package\]\]\s*name = "fluxdl"\s*version = "([^"]+)"/u,
  )?.[1];
  const versions = [
    pkg.version,
    lock.version,
    lock.packages?.[""]?.version,
    tauri.version,
    cargoVersion,
    cargoLockVersion,
  ];
  if (
    !/^\d+\.\d+\.\d+$/u.test(pkg.version) ||
    versions.some((v) => v !== pkg.version)
  ) {
    fail(
      `版本不一致或不支持的版本：${JSON.stringify(versions)}；请同步package/lock、Tauri和Cargo版本。`,
    );
  }
  if (
    pkg.name !== "fluxdl" ||
    tauri.productName !== "FluxDL" ||
    !tauri.bundle?.targets?.includes("nsis")
  ) {
    fail("仅支持本项目FluxDL的NSIS构建配置");
  }
  // New external resource requirements must be designed explicitly, not silently omitted.
  if (
    JSON.stringify(tauri.bundle.resources) !==
    JSON.stringify(["../THIRD_PARTY_NOTICES.md", "../LICENSE", "../licenses/"])
  ) {
    fail("Tauri bundle资源配置发生变化；请先更新并验证交付文件契约。");
  }
  return {
    version: pkg.version,
    installer: `FluxDL_${pkg.version}_x64-setup.exe`,
  };
}

export const git = (root, args) => run(root, "git", args, { capture: true });

export async function sourceState(root) {
  const commit = await git(root, ["rev-parse", "HEAD"]);
  if (!/^[a-f0-9]{40,64}$/u.test(commit)) fail("无法确认Git HEAD");
  const dirty = Boolean(
    await git(root, ["status", "--porcelain=v1", "--untracked-files=all"]),
  );
  const list = await git(root, [
    "ls-files",
    "--cached",
    "--others",
    "--exclude-standard",
    "-z",
    "--",
    "src",
    "src-tauri",
    "public",
    "licenses",
    "scripts",
    "package.json",
    "package-lock.json",
    "index.html",
    "vite.config.ts",
    "tsconfig.json",
    "tsconfig.app.json",
    "tsconfig.node.json",
    "LICENSE",
    "THIRD_PARTY_NOTICES.md",
  ]);
  const names = [...new Set(list.split("\0").filter(Boolean))].sort();
  const hash = createHash("sha256");
  for (const name of names) {
    const full = await safePath(root, path.join(root, name));
    hash.update(name + "\0");
    hash.update(sha256(await fs.readFile(full)) + "\0");
  }
  return { commit, dirty, fingerprint: hash.digest("hex") };
}

export async function inspectBinaries(root, files, version) {
  for (const [index, file] of files.entries()) {
    await safePath(root, file);
    const stat = await fs.stat(file);
    if (!stat.isFile() || stat.size < 4096)
      fail(`EXE缺失、为空或过小：${file}`);
    const handle = await fs.open(file, "r");
    try {
      const dos = Buffer.alloc(64);
      await handle.read(dos, 0, dos.length, 0);
      const offset = dos.readUInt32LE(60);
      if (
        dos.readUInt16LE() !== 0x5a4d ||
        offset < 64 ||
        offset + 26 > stat.size
      )
        fail(`无效PE文件：${file}`);
      const pe = Buffer.alloc(26);
      await handle.read(pe, 0, pe.length, offset);
      const machine = pe.readUInt16LE(4);
      const magic = pe.readUInt16LE(24);
      if (
        pe.readUInt32LE() !== 0x4550 ||
        pe.readUInt16LE(6) < 1 ||
        !(pe.readUInt16LE(22) & 2) ||
        (index === 0 && (machine !== 0x8664 || magic !== 0x20b)) ||
        (index === 1 &&
          !(
            (machine === 0x14c && magic === 0x10b) ||
            (machine === 0x8664 && magic === 0x20b)
          ))
      ) {
        fail(`PE架构/可执行标记不符合配置：${file}`);
      }
    } finally {
      await handle.close();
    }
  }
  const details = JSON.parse(
    await run(
      root,
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-File", helper],
      { capture: true, input: JSON.stringify(files) },
    ),
  );
  if (
    details.length !== 2 ||
    details.some(
      (v) =>
        v.product !== "FluxDL" ||
        v.productVersion !== version ||
        v.fileVersion !== version,
    )
  ) {
    fail("EXE/安装器的PE产品名或文件版本与构建配置不一致");
  }
}

export function context(root = ROOT) {
  return {
    root: path.resolve(root),
    fixture: false,
    inspect: inspectBinaries,
    run,
  };
}

export async function preflight(ctx) {
  if (process.platform !== "win32" || process.arch !== "x64")
    fail("正式构建需要Windows x64 Node及MSVC工具链");
  if (Number(process.versions.node.split(".")[0]) < 24)
    fail("发布脚本需要Node.js 24或更新版本（内置CRC32）");
  for (const name of [
    "CARGO_TARGET_DIR",
    "CARGO_BUILD_TARGET",
    "TAURI_CONFIG",
    "TAURI_TARGET_TRIPLE",
  ]) {
    if (process.env[name])
      fail(
        `请先清除${name}覆盖项；此流水线只接受仓库配置的Windows x64默认输出。`,
      );
  }
  await configuration(ctx.root);
  for (const [tool, args] of [
    ["rustc", ["--version"]],
    ["cargo", ["--version"]],
  ]) {
    console.log(await ctx.run(ctx.root, tool, args, { capture: true }));
  }
  const rust = await ctx.run(ctx.root, "rustc", ["-vV"], { capture: true });
  if (!rust.includes("host: x86_64-pc-windows-msvc"))
    fail("Rust host必须为x86_64-pc-windows-msvc");
  const tauri = await readJson(
    path.join(ctx.root, "node_modules/@tauri-apps/cli/package.json"),
  );
  console.log(
    `Node ${process.versions.node}; Tauri ${tauri.version}; npm ${await npmRun(ctx, ["--version"], true)}`,
  );
}

export async function npmRun(ctx, args, capture = false) {
  const cli = process.env.npm_execpath;
  if (!cli || !(await exists(cli)))
    fail("请通过npm run调用构建命令，以便定位当前npm CLI。");
  return ctx.run(ctx.root, process.execPath, [cli, ...args], { capture });
}

export async function buildDesktop(ctx) {
  const { root } = ctx;
  const config = await configuration(root);
  const receiptFile = await safePath(root, path.join(root, RECEIPT));
  if (await exists(receiptFile)) await fs.unlink(receiptFile);
  const state = await sourceState(root);
  const buildTimeUtc = new Date().toISOString();
  const installerPath = await safePath(
    root,
    path.join(root, "src-tauri/target/release/bundle/nsis", config.installer),
  );
  // Only the exact intermediate installer is invalidated, never final output or caches.
  if (await exists(installerPath)) await fs.unlink(installerPath);
  await ctx.run(root, process.execPath, [
    path.join(root, "node_modules/@tauri-apps/cli/tauri.js"),
    "build",
    "--bundles",
    "nsis",
  ]);
  const files = [
    path.join(root, "src-tauri/target/release/fluxdl.exe"),
    installerPath,
  ];
  await ctx.inspect(root, files, config.version);
  const after = await sourceState(root);
  if (
    state.commit !== after.commit ||
    state.fingerprint !== after.fingerprint ||
    state.dirty !== after.dirty
  ) {
    fail(
      "构建期间HEAD、工作树状态或构建输入发生变化；不生成来源收据，不发布。",
    );
  }
  await configuration(root);
  const receipt = {
    schema: 1,
    validationMode: ctx.fixture ? "fixture" : "production",
    version: config.version,
    gitCommit: state.commit.slice(0, 12),
    gitCommitFull: state.commit,
    gitDirty: state.dirty,
    sourceFingerprint: state.fingerprint,
    buildTimeUtc,
    finishedTimeUtc: new Date().toISOString(),
    files: await Promise.all(
      files.map(async (file) => ({
        name: path.basename(file),
        sha256: sha256(await fs.readFile(file)),
      })),
    ),
  };
  await writeJson(receiptFile, receipt);
  return receipt;
}

export async function buildInputs(ctx) {
  const { root } = ctx;
  const config = await configuration(root);
  const file = await safePath(root, path.join(root, RECEIPT));
  if (!(await exists(file)))
    fail(
      "缺少可核实的成功构建收据。运行 npm run desktop:build 或 npm run release:build；不会静默复用历史EXE。",
    );
  const receipt = await readJson(file);
  if (
    receipt.schema !== 1 ||
    receipt.validationMode !== (ctx.fixture ? "fixture" : "production") ||
    receipt.version !== config.version
  ) {
    fail("构建收据版本/来源模式不匹配；测试fixture不能用于正式发布。");
  }
  const state = await sourceState(root);
  if (state.fingerprint !== receipt.sourceFingerprint)
    fail(
      "构建输入已过期：源码、配置、脚本或许可证有变化。请重新执行 npm run desktop:build。",
    );
  const binaries = [
    path.join(root, "src-tauri/target/release/fluxdl.exe"),
    path.join(root, "src-tauri/target/release/bundle/nsis", config.installer),
  ];
  await ctx.inspect(root, binaries, config.version);
  for (const [index, binary] of binaries.entries()) {
    if (
      receipt.files?.[index]?.name !== path.basename(binary) ||
      receipt.files[index].sha256 !== sha256(await fs.readFile(binary))
    ) {
      fail(`构建输出与成功收据不同，不能发布：${binary}`);
    }
  }
  if (!ctx.fixture) {
    if (state.commit !== receipt.gitCommitFull)
      console.log(
        `来源说明：产物实际构建于${receipt.gitCommit}；当前HEAD不同，但构建输入指纹完全相同。元数据保留实际构建Commit。`,
      );
    console.log(
      `复用已确认构建：${receipt.version} / ${receipt.gitCommit} / dirty=${receipt.gitDirty} / ${receipt.buildTimeUtc}`,
    );
  }
  return { config, receipt, binaries };
}
