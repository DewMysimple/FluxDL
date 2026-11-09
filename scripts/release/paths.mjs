import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { randomUUID, createHash } from "node:crypto";

export const OWNER = "fluxdl-release-workflow-v1";
export const WORK = ".fluxdl-release-work";
export const OUTPUT = "release-output";
export const exists = async (p) =>
  fs.lstat(p).then(
    () => true,
    (e) => {
      if (e.code === "ENOENT") return false;
      throw e;
    },
  );
export const readJson = async (p) => JSON.parse(await fs.readFile(p, "utf8"));
export const writeJson = (p, value) =>
  fs.writeFile(p, JSON.stringify(value, null, 2) + "\n", { flag: "w" });
export const sha256 = (buffer) =>
  createHash("sha256").update(buffer).digest("hex");

// Validate every existing ancestor. Junctions and symbolic links are never followed.
export async function safePath(root, target) {
  root = path.resolve(root);
  target = path.resolve(target);
  const relative = path.relative(root, target);
  if (
    !relative ||
    relative.startsWith(".." + path.sep) ||
    relative === ".." ||
    path.isAbsolute(relative) ||
    target === path.parse(target).root ||
    target === path.resolve(os.homedir())
  ) {
    throw new Error(`拒绝不在项目安全范围内的路径：${target}`);
  }
  if (
    path.normalize(await fs.realpath(root)).toLowerCase() !== root.toLowerCase()
  ) {
    throw new Error(`项目根目录不能通过符号链接或junction访问：${root}`);
  }
  let current = root;
  for (const part of relative.split(path.sep)) {
    current = path.join(current, part);
    if (!(await exists(current))) continue;
    const stat = await fs.lstat(current);
    if (stat.isSymbolicLink() || (!stat.isDirectory() && !stat.isFile())) {
      throw new Error(`拒绝链接或特殊文件：${current}`);
    }
  }
  return target;
}

export function safeName(name) {
  if (
    !name ||
    name.includes("\\") ||
    name.startsWith("/") ||
    name.includes(":") ||
    [...name].some((char) => char.charCodeAt(0) < 32) ||
    name
      .split("/")
      .some(
        (part) =>
          !part ||
          part === "." ||
          part === ".." ||
          /[. ]$/u.test(part) ||
          /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/iu.test(part),
      )
  ) {
    throw new Error(`非法归档相对路径：${JSON.stringify(name)}`);
  }
  return name;
}

export async function walkFiles(root, directory, { rejectEmpty = false } = {}) {
  await safePath(root, directory);
  const files = [];
  const folded = new Set();
  async function walk(dir) {
    const children = await fs.readdir(dir, { withFileTypes: true });
    if (rejectEmpty && !children.length)
      throw new Error(`交付目录包含无法在文件清单中表示的空目录：${dir}`);
    for (const item of children) {
      const full = await safePath(root, path.join(dir, item.name));
      const name = safeName(
        path.relative(directory, full).split(path.sep).join("/"),
      );
      if (item.isDirectory()) await walk(full);
      else if (item.isFile()) {
        if (folded.has(name.toLowerCase()))
          throw new Error(`Windows大小写重复路径：${name}`);
        folded.add(name.toLowerCase());
        files.push({ name, full, bytes: (await fs.stat(full)).size });
      } else throw new Error(`拒绝特殊文件：${full}`);
    }
  }
  await walk(directory);
  return files.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
}

export async function removeTree(root, directory) {
  await safePath(root, directory);
  await walkFiles(root, directory);
  await fs.rm(directory, { recursive: true });
}

export async function initWork(root) {
  const base = await safePath(root, path.join(root, WORK));
  try {
    await fs.mkdir(base);
    await fs.writeFile(
      path.join(base, ".owner.json"),
      JSON.stringify({ owner: OWNER }),
      { flag: "wx" },
    );
  } catch (e) {
    if (e.code !== "EEXIST") throw e;
  }
  await safePath(root, path.join(base, ".owner.json"));
  if ((await fs.stat(root)).dev !== (await fs.stat(base)).dev)
    throw new Error("暂存区必须与正式输出位于同一文件系统");
  if ((await readJson(path.join(base, ".owner.json"))).owner !== OWNER) {
    throw new Error(`临时目录没有有效所有权标记，保留不动：${base}`);
  }
  await safePath(root, path.join(base, ".owner.json"));
  return base;
}

export function pidAlive(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) throw new Error("锁中的PID无效");
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    if (e.code === "ESRCH") return false;
    if (e.code === "EPERM") return true;
    throw e;
  }
}

export async function readLock(root) {
  const file = await safePath(root, path.join(root, WORK, "lock.json"));
  if (!(await exists(file))) return null;
  const lock = await readJson(file);
  if (lock.owner !== OWNER || typeof lock.token !== "string")
    throw new Error(`未知发布锁，保留：${file}`);
  return { ...lock, alive: pidAlive(lock.pid), file };
}

// Serialize explicit stale-lock removal too; a second cleaner must not unlink a new lock.
export async function clearStaleLock(root, expected) {
  const guard = await safePath(
    root,
    path.join(root, WORK, "stale-lock-guard.json"),
  );
  let handle;
  try {
    handle = await fs.open(guard, "wx");
  } catch (e) {
    throw new Error(
      `锁恢复正在执行或留有恢复锁，先核实该文件中的PID并按文档恢复：${guard}`,
      { cause: e },
    );
  }
  try {
    await handle.writeFile(
      JSON.stringify({ owner: OWNER, pid: process.pid, token: expected.token }),
    );
    await handle.close();
    handle = null;
    if (
      pidAlive(expected.pid) ||
      (await readJson(expected.file)).token !== expected.token
    ) {
      throw new Error("锁的身份或存活状态变化，拒绝清理");
    }
    await fs.unlink(expected.file);
  } finally {
    await handle?.close();
    await fs.unlink(guard);
  }
}

export async function withLock(root, action) {
  const base = await initWork(root);
  const file = path.join(base, "lock.json");
  const lock = {
    owner: OWNER,
    pid: process.pid,
    token: randomUUID(),
    createdAt: new Date().toISOString(),
  };
  let handle;
  try {
    handle = await fs.open(file, "wx");
  } catch (e) {
    if (e.code !== "EEXIST") throw e;
    const current = await readLock(root);
    throw new Error(
      `发布锁已存在（PID ${current.pid}，${current.alive ? "仍在运行" : "已退出"}）：${file}。` +
        (current.alive
          ? "等待该任务结束。"
          : "确认后运行 npm run release:clean -- --apply --stale-lock；不会自动删除锁。"),
      { cause: e },
    );
  }
  try {
    await handle.writeFile(JSON.stringify(lock));
    await handle.close();
    handle = null;
    return await action(base);
  } finally {
    await handle?.close();
    if ((await exists(file)) && (await readJson(file)).token === lock.token)
      await fs.unlink(file);
  }
}

export async function transactions(root, base) {
  const result = [];
  for (const name of await fs.readdir(base)) {
    if (
      name === ".owner.json" ||
      name === "lock.json" ||
      name === "last-failure.json"
    )
      continue;
    const directory = await safePath(root, path.join(base, name));
    if (!/^transaction-[0-9a-f-]{36}$/u.test(name))
      throw new Error(`发现未知临时文件，保留：${directory}`);
    const marker = await readJson(
      await safePath(root, path.join(directory, ".owner.json")),
    );
    if (marker.owner !== OWNER || marker.id !== name.slice(12))
      throw new Error(`无效事务所有权：${directory}`);
    const allowed = [
      ".owner.json",
      "journal.json",
      "candidate",
      "previous-output",
      "rejected-output",
      "rejected-output-recovery",
    ];
    if ((await fs.readdir(directory)).some((child) => !allowed.includes(child)))
      throw new Error(`事务中含未知文件，保留：${directory}`);
    await walkFiles(root, directory);
    result.push(directory);
  }
  return result;
}

export async function assertNoRecovery(root, base) {
  const pending = await transactions(root, base);
  if (pending.length)
    throw new Error(
      `发现异常退出残留，禁止覆盖。先运行 npm run release:clean 预览；` +
        `有previous-output备份时使用 --apply --recover，否则使用 --apply。恢复位置：\n${pending.join("\n")}`,
    );
}

export async function createTransaction(root, base) {
  const id = randomUUID();
  const directory = await safePath(root, path.join(base, `transaction-${id}`));
  await fs.mkdir(directory);
  await writeJson(path.join(directory, ".owner.json"), { owner: OWNER, id });
  await writeJson(path.join(directory, "journal.json"), {
    phase: "staging",
    startedAt: new Date().toISOString(),
  });
  return directory;
}
