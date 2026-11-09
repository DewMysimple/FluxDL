import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  OWNER,
  OUTPUT,
  WORK,
  assertNoRecovery,
  createTransaction,
  exists,
  initWork,
  clearStaleLock,
  readJson,
  readLock,
  removeTree,
  safePath,
  sha256,
  transactions,
  walkFiles,
  withLock,
  writeJson,
} from "./paths.mjs";
import { buildDesktop, npmRun, preflight, run } from "./source.mjs";
import { optionalVerify, stageCandidate, verifyPair } from "./package.mjs";

const rename = (ctx, from, to) => (ctx.rename ?? fs.rename)(from, to);
const hook = (ctx, phase) => ctx.hook?.(phase);
const journalPhase = async (directory) => {
  try {
    return (await readJson(path.join(directory, "journal.json"))).phase;
  } catch {
    return "unknown (journal损坏或缺失，不能据此删除备份)";
  }
};
const recoveryMessage = (transaction, output) =>
  `旧版恢复失败，备份必须保留：${path.join(transaction, "previous-output")}\n` +
  `正式输出：${output}\n停止其他发布任务；运行 npm run release:clean 预览，然后 npm run release:clean -- --apply --recover。` +
  "若手工恢复，先将正式输出移至另一个安全位置，再把上述previous-output整体重命名回release-output；不要合并复制或删除备份。";

export async function publish(ctx, base) {
  const { root } = ctx;
  await assertNoRecovery(root, base);
  if (!ctx.fixture && (ctx.hook || ctx.rename || ctx.createZip))
    throw new Error("生产发布不允许测试故障注入");
  const output = await safePath(root, path.join(root, OUTPUT));
  await optionalVerify(ctx, output);
  const transaction = await createTransaction(root, base);
  const backup = path.join(transaction, "previous-output");
  let promoted = false;
  let backedUp = false;
  const journal = async (phase) =>
    writeJson(path.join(transaction, "journal.json"), {
      phase,
      updatedAt: new Date().toISOString(),
    });
  try {
    const candidate = await stageCandidate(ctx, transaction);
    await journal("ready");
    await hook(ctx, "ready");
    if (await exists(output)) {
      await rename(ctx, output, backup);
      backedUp = true;
      await journal("backup-moved");
    }
    await hook(ctx, "backup-moved");
    await rename(ctx, candidate, output);
    promoted = true;
    await journal("promoted");
    await hook(ctx, "promoted");
    const result = await verifyPair(ctx, output);
    await journal("verified");
    // Cleanup errors cannot trigger rollback after a verified backup was already removed.
    try {
      await removeTree(root, transaction);
    } catch (e) {
      console.warn(
        `新产物已验证，临时目录未能全部清理：${transaction}；${e.message}。请用release:clean预览。`,
      );
    }
    return result;
  } catch (error) {
    await writeJson(
      await safePath(root, path.join(base, "last-failure.json")),
      {
        owner: OWNER,
        timeUtc: new Date().toISOString(),
        transaction: path.basename(transaction),
        message: error.message,
      },
    ).catch((e) => console.warn(`无法写入故障记录：${e.message}`));
    try {
      if (promoted)
        await rename(ctx, output, path.join(transaction, "rejected-output"));
      if (backedUp) {
        await rename(ctx, backup, output);
        await verifyPair(ctx, output);
      }
      await removeTree(root, transaction);
    } catch (rollbackError) {
      await journal("rollback-failed").catch(() => {});
      throw new Error(
        `${error.message}\n${recoveryMessage(transaction, output)}\n回滚错误：${rollbackError.message}`,
        { cause: rollbackError },
      );
    }
    throw new Error(`发布失败，上一版正式产物已保留或恢复：${error.message}`, {
      cause: error,
    });
  }
}

export async function releaseBuild(ctx) {
  return withLock(ctx.root, async (base) => {
    await assertNoRecovery(ctx.root, base);
    await preflight(ctx);
    for (const script of [
      "typecheck",
      "lint",
      "format:check",
      "test",
      "test:release",
      "check:rust",
      "format:rust",
      "lint:rust",
      "test:rust",
    ]) {
      await npmRun(ctx, ["run", script]);
    }
    await ctx.run(ctx.root, "powershell.exe", [
      "-NoProfile",
      "-NonInteractive",
      "-File",
      path.join(ctx.root, "scripts/test-queue.ps1"),
    ]);
    await ctx.run(ctx.root, "python", [
      path.join(ctx.root, "scripts/collect-licenses.py"),
    ]);
    await buildDesktop(ctx);
    return publish(ctx, base);
  });
}

export async function desktopBuild(ctx) {
  return withLock(ctx.root, async (base) => {
    await assertNoRecovery(ctx.root, base);
    await preflight(ctx);
    return buildDesktop(ctx);
  });
}

export async function releaseStage(ctx) {
  return withLock(ctx.root, async (base) => {
    const legacy = path.join(ctx.root, "release");
    if (await exists(legacy))
      console.log(
        `发现历史release目录，保留不动：${legacy}。用npm run release:clean -- --legacy预览。`,
      );
    return publish(ctx, base);
  });
}

export async function releaseVerify(ctx) {
  const lock = await readLock(ctx.root);
  if (lock)
    throw new Error(
      `发布锁尚在（PID ${lock.pid}，alive=${lock.alive}），只读验证不修改或接管它：${lock.file}`,
    );
  return verifyPair(ctx, path.join(ctx.root, OUTPUT));
}

async function legacyCandidate(ctx) {
  const { root } = ctx;
  const directory = await safePath(root, path.join(root, "release"));
  if (!(await exists(directory))) return null;
  const files = await walkFiles(root, directory);
  const names = files.map((file) => file.name);
  const installers = names.filter((name) =>
    /^FluxDL_\d+\.\d+\.\d+_x64-setup\.exe$/u.test(name),
  );
  let reason = "历史stage脚本文件白名单与原二进制哈希匹配";
  let eligible = true;
  const allowed = [
    "FluxDL.exe",
    "LICENSE",
    "THIRD_PARTY_NOTICES.md",
    "使用说明.txt",
    "SHA256SUMS.txt",
  ];
  if (
    !installers.length ||
    !allowed.every((name) => names.includes(name)) ||
    names.some(
      (name) =>
        !allowed.includes(name) &&
        !installers.includes(name) &&
        !name.startsWith("licenses/"),
    )
  ) {
    eligible = false;
    reason = "包含未知文件或缺失历史stage必需文件，禁止删除整个旧目录";
  }
  if (eligible) {
    for (const file of files.filter(
      (file) =>
        file.name.startsWith("licenses/") ||
        ["LICENSE", "THIRD_PARTY_NOTICES.md"].includes(file.name),
    )) {
      const known = await safePath(root, path.join(root, file.name));
      if (
        !(await exists(known)) ||
        sha256(await fs.readFile(known)) !==
          sha256(await fs.readFile(file.full))
      ) {
        eligible = false;
        reason = "许可证存在不能与当前仓库确认的文件，保留旧目录";
        break;
      }
    }
    const lines = (
      await fs.readFile(path.join(directory, "SHA256SUMS.txt"), "utf8")
    )
      .trim()
      .split(/\r?\n/u);
    const covered = new Set();
    for (const line of lines) {
      const match =
        /^([a-f0-9]{64}) {2}(FluxDL(?:_\d+\.\d+\.\d+_x64-setup)?\.exe)$/u.exec(
          line,
        );
      if (
        !match ||
        !names.includes(match[2]) ||
        sha256(await fs.readFile(path.join(directory, match[2]))) !== match[1]
      ) {
        eligible = false;
        reason = "历史二进制SHA256不能确认，保留旧目录";
        break;
      }
      covered.add(match[2]);
    }
    if (
      !covered.has("FluxDL.exe") ||
      installers.some((name) => !covered.has(name))
    ) {
      eligible = false;
      reason = "历史二进制哈希清单不完整";
    }
  }
  if (eligible) {
    const running = ctx.runningLegacy
      ? await ctx.runningLegacy(directory)
      : JSON.parse(
          await run(
            root,
            "powershell.exe",
            [
              "-NoProfile",
              "-NonInteractive",
              "-File",
              fileURLToPath(new URL("./running-legacy.ps1", import.meta.url)),
            ],
            { capture: true, input: path.join(directory, "FluxDL.exe") },
          ),
        );
    if (running) {
      eligible = false;
      reason = "旧FluxDL.exe仍在运行，不停止或删除用户实例";
    }
  }
  return {
    path: directory,
    bytes: files.reduce((n, file) => n + file.bytes, 0),
    eligible,
    reason,
    kind: "legacy",
  };
}

export async function releaseClean(ctx, options = {}) {
  const { root } = ctx;
  const basePath = await safePath(root, path.join(root, WORK));
  const lock = await readLock(root);
  if (lock && options.apply && (!options.staleLock || lock.alive))
    throw new Error(`不能接管发布锁：${lock.file}（alive=${lock.alive}）`);
  const candidates = [];
  if (await exists(basePath)) {
    await initWork(root);
    if (!lock?.alive) {
      for (const transaction of await transactions(root, basePath)) {
        const files = await walkFiles(root, transaction);
        const hasBackup = await exists(
          path.join(transaction, "previous-output"),
        );
        const phase = await journalPhase(transaction);
        candidates.push({
          path: transaction,
          bytes: files.reduce((n, file) => n + file.bytes, 0),
          kind: "transaction",
          eligible: !hasBackup || phase === "verified",
          reason: hasBackup
            ? phase === "verified"
              ? "新产物已验证，清理前将再次核实正式输出身份和哈希"
              : "含旧版备份；仅--recover显式核实并恢复后才可清理"
            : `脚本所有的残留事务（${phase}），无旧版备份`,
        });
      }
    }
  }
  if (options.legacy) {
    const legacy = await legacyCandidate(ctx);
    if (legacy) candidates.push(legacy);
  }
  if (!options.apply)
    return { mode: "dry-run", lock, candidates, deletedBytes: 0 };
  if (lock) {
    await clearStaleLock(root, lock);
  }
  return withLock(root, async () => {
    let deletedBytes = 0;
    for (const item of candidates) {
      if (item.kind === "legacy") {
        const fresh = await legacyCandidate(ctx);
        if (!fresh?.eligible) continue;
      }
      const backup = path.join(item.path, "previous-output");
      if (item.kind === "transaction" && (await exists(backup))) {
        const phase = await journalPhase(item.path);
        if (phase === "verified" && !options.recover) {
          const current = await verifyPair(ctx, path.join(root, OUTPUT));
          if (current.buildId !== path.basename(item.path).slice(12))
            throw new Error("已验证输出不再属于该事务，保留备份");
          deletedBytes += (await walkFiles(root, item.path)).reduce(
            (n, file) => n + file.bytes,
            0,
          );
          await removeTree(root, item.path);
          continue;
        }
        if (!options.recover) continue;
        await verifyPair(ctx, backup);
        const output = path.join(root, OUTPUT);
        if (await exists(output)) {
          const info = await readJson(
            await safePath(root, path.join(output, "FluxDL/build-info.json")),
          );
          if (info.buildId !== path.basename(item.path).slice(12))
            throw new Error(`正式输出不属于该事务，不能自动挪动：${output}`);
          await fs.rename(
            output,
            path.join(item.path, "rejected-output-recovery"),
          );
        }
        await fs.rename(backup, output);
        await verifyPair(ctx, output);
      } else if (!item.eligible) continue;
      // Recalculate after restoration; restored final files are not counted as deleted.
      deletedBytes += (await walkFiles(root, item.path)).reduce(
        (n, file) => n + file.bytes,
        0,
      );
      await removeTree(root, item.path);
    }
    return {
      mode: "apply",
      candidates,
      deletedBytes,
      lockRemoved: Boolean(lock),
    };
  });
}
