import fs from "node:fs/promises";
import path from "node:path";
import {
  exists,
  readJson,
  safePath,
  sha256,
  walkFiles,
  writeJson,
} from "./paths.mjs";
import { buildInputs } from "./source.mjs";
import { createZip, verifyZip } from "./zip.mjs";

const HASHES = "SHA256SUMS.txt";
const fail = (message) => {
  throw new Error(message);
};

export async function inventory(root, directory) {
  return new Map(
    await Promise.all(
      (await walkFiles(root, directory, { rejectEmpty: true })).map(
        async (file) => [
          file.name,
          { bytes: file.bytes, sha256: sha256(await fs.readFile(file.full)) },
        ],
      ),
    ),
  );
}

function hashText(files) {
  return [...files]
    .filter(([name]) => name !== HASHES)
    .map(([name, file]) => `${file.sha256}  ${name}\n`)
    .join("");
}

function validateMetadata(info, fixture) {
  const keys = [
    "schema",
    "product",
    "version",
    "gitCommit",
    "gitCommitFull",
    "gitDirty",
    "sourceFingerprint",
    "buildTimeUtc",
    "finishedTimeUtc",
    "target",
    "profile",
    "packaging",
    "engineDistribution",
    "validationMode",
    "buildId",
  ];
  if (
    Object.keys(info).some((name) => !keys.includes(name)) ||
    info.schema !== 1 ||
    info.product !== "FluxDL" ||
    !/^\d+\.\d+\.\d+$/u.test(info.version) ||
    !/^[a-f0-9]{12}$/u.test(info.gitCommit) ||
    !/^[a-f0-9]{40,64}$/u.test(info.gitCommitFull) ||
    !info.gitCommitFull.startsWith(info.gitCommit) ||
    typeof info.gitDirty !== "boolean" ||
    !/^[a-f0-9]{64}$/u.test(info.sourceFingerprint) ||
    !/^[0-9a-f-]{36}$/u.test(info.buildId) ||
    info.target !== "windows-x64" ||
    info.profile !== "release" ||
    info.packaging !== "tauri2-nsis-and-raw-exe" ||
    info.engineDistribution !== "external" ||
    info.validationMode !== (fixture ? "fixture" : "production")
  )
    fail("build-info.json字段不完整、值无效或包含非正式来源");
  for (const key of ["buildTimeUtc", "finishedTimeUtc"]) {
    if (
      typeof info[key] !== "string" ||
      !/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/u.test(info[key]) ||
      !Number.isFinite(Date.parse(info[key]))
    ) {
      fail(`build-info.json时间无效：${key}`);
    }
  }
  if (info.buildTimeUtc > info.finishedTimeUtc)
    fail("构建结束时间早于开始时间");
}

export async function verifyPair(ctx, pair) {
  const { root } = ctx;
  await safePath(root, pair);
  const children = (await fs.readdir(pair)).sort();
  if (JSON.stringify(children) !== JSON.stringify(["FluxDL", "FluxDL.zip"]))
    fail(`正式输出必须且只能包含FluxDL/与FluxDL.zip：${pair}`);
  const directory = await safePath(root, path.join(pair, "FluxDL"));
  const metadata = await safePath(
    root,
    path.join(directory, "build-info.json"),
  );
  if ((await fs.stat(metadata)).size > 16384) fail("build-info.json异常大");
  const info = await readJson(metadata);
  validateMetadata(info, ctx.fixture);
  const installer = `FluxDL_${info.version}_x64-setup.exe`;
  const files = await inventory(root, directory);
  const required = [
    "FluxDL.exe",
    installer,
    "LICENSE",
    "THIRD_PARTY_NOTICES.md",
    "licenses/README.md",
    "使用说明.txt",
    "build-info.json",
    HASHES,
  ];
  for (const name of required)
    if (!files.has(name) || !files.get(name).bytes)
      fail(`缺失或为空的交付文件：${name}`);
  if (
    [...files.keys()].filter((name) => name.startsWith("licenses/")).length < 2
  )
    fail("缺少第三方许可证原文");
  for (const name of files.keys()) {
    if (!required.includes(name) && !name.startsWith("licenses/"))
      fail(`出现非交付文件或旧安装器：${name}`);
    if (
      name.startsWith("licenses/") &&
      (/(^|\/)(\.|node_modules|target|artifacts|\.git)(\/|$)/u.test(name) ||
        /\.(exe|dll|zip|mp[34]|sqlite3?|db|log|env|pem|key)$/iu.test(name))
    )
      fail(`许可证目录中出现不应分发的文件：${name}`);
  }
  if (
    (await fs.readFile(path.join(directory, HASHES), "utf8")) !==
    hashText(files)
  )
    fail("SHA256SUMS清单与全部文件不一致");
  await ctx.inspect(
    root,
    [path.join(directory, "FluxDL.exe"), path.join(directory, installer)],
    info.version,
  );
  const zip = await safePath(root, path.join(pair, "FluxDL.zip"));
  const archive = await verifyZip(zip, files);
  return {
    ...info,
    directory,
    zip,
    files: files.size,
    manifest: "passed",
    archive,
    executables: ["FluxDL.exe", installer].map((name) => ({
      name,
      bytes: files.get(name).bytes,
      sha256: files.get(name).sha256,
    })),
  };
}

export async function stageCandidate(ctx, transaction) {
  const { root } = ctx;
  const { config, receipt, binaries } = await buildInputs(ctx);
  const pair = await safePath(root, path.join(transaction, "candidate"));
  const directory = path.join(pair, "FluxDL");
  await fs.mkdir(directory, { recursive: true });
  for (const [index, name] of ["FluxDL.exe", config.installer].entries())
    await fs.copyFile(binaries[index], path.join(directory, name));
  for (const name of ["LICENSE", "THIRD_PARTY_NOTICES.md"]) {
    await safePath(root, path.join(root, name));
    if (!(await fs.stat(path.join(root, name))).size)
      fail(`许可证为空：${name}`);
    await fs.copyFile(path.join(root, name), path.join(directory, name));
  }
  const licenses = path.join(root, "licenses");
  await walkFiles(root, licenses);
  await fs.cp(licenses, path.join(directory, "licenses"), {
    recursive: true,
    errorOnExist: true,
    force: false,
  });
  await fs.writeFile(
    path.join(directory, "使用说明.txt"),
    `FluxDL ${config.version} · Windows x64\r\n\r\n` +
      `${config.installer} 是NSIS安装程序，支持当前用户安装；不是便携版。\r\n` +
      "FluxDL.exe是原始Release程序，需要Windows 10/11 x64及系统Edge WebView2；并非完全无依赖绿色单文件。\r\n" +
      "设置和SQLite持久化在系统AppData的io.fluxdl.desktop；此交付不包含用户数据，也不设置便携数据目录。\r\n" +
      "在设置中检测或选择本机yt-dlp.exe；MP3及音视频合并需要FFmpeg和FFprobe。\r\n" +
      "本版不附带或自动下载yt-dlp、FFmpeg、FFprobe、EJS或JavaScript运行时；按站点要求自行配置。\r\n" +
      "关闭窗口默认进入托盘，真正退出请使用托盘菜单。程序未代码签名。\r\n" +
      "请保留相邻许可证。此本地包不等于GitHub Release；完整说明见项目README及docs/build-and-release.md。\r\n",
    "utf8",
  );
  await writeJson(path.join(directory, "build-info.json"), {
    schema: 1,
    product: "FluxDL",
    version: config.version,
    gitCommit: receipt.gitCommit,
    gitCommitFull: receipt.gitCommitFull,
    gitDirty: receipt.gitDirty,
    sourceFingerprint: receipt.sourceFingerprint,
    buildTimeUtc: receipt.buildTimeUtc,
    finishedTimeUtc: receipt.finishedTimeUtc,
    target: "windows-x64",
    profile: "release",
    packaging: "tauri2-nsis-and-raw-exe",
    engineDistribution: "external",
    validationMode: ctx.fixture ? "fixture" : "production",
    buildId: path.basename(transaction).slice(12),
  });
  const files = await inventory(root, directory);
  await fs.writeFile(path.join(directory, HASHES), hashText(files), "utf8");
  // Compare copied binaries to their recorded input hashes before creating the archive.
  for (const [index, name] of ["FluxDL.exe", config.installer].entries()) {
    if (files.get(name).sha256 !== receipt.files[index].sha256)
      fail("暂存时构建输出发生变化");
  }
  const zip = path.join(pair, "FluxDL.zip");
  await (ctx.createZip ?? createZip)(root, directory, zip);
  await verifyPair(ctx, pair);
  // Reject source/license edits racing the copy, even if the resulting candidate is self-consistent.
  await buildInputs(ctx);
  return pair;
}

export async function optionalVerify(ctx, pair) {
  return (await exists(pair)) ? verifyPair(ctx, pair) : null;
}
