// Generated loopback payloads prove runner mechanics only, never Internet throughput.
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import http from "node:http";
import { fileURLToPath } from "node:url";
import { setTimeout as wait } from "node:timers/promises";
import {
  DownloadBudget,
  TransferMetrics,
  runSafeChild,
  safeRoot,
} from "./network-core.mjs";

const workspace = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const fixture = path.join(
  workspace,
  "scripts/fixtures/network-transfer-child.mjs",
);
const privateToken = "FIXTURE_PRIVATE_SIGNED_QUERY";
const alive = (pid) => {
  if (!Number.isInteger(pid)) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};

async function ownedRun(run) {
  const root = safeRoot(
    workspace,
    path.join(
      workspace,
      "artifacts/network-diagnostics",
      `transfer-test-${crypto.randomUUID()}`,
    ),
  );
  fs.mkdirSync(root, { recursive: true });
  try {
    return await run(root);
  } finally {
    safeRoot(workspace, root);
    // These fixtures never create media files; preserve anything unexpected.
    assert.deepEqual(fs.readdirSync(root), []);
    fs.rmdirSync(root);
  }
}

async function withServer(handler, run) {
  const timers = new Set();
  let sentBytes = 0;
  function stream(
    response,
    { chunks, size = 4096, interval = 100, interrupt = false },
  ) {
    const payload = Buffer.alloc(size, 0x5a);
    let count = 0;
    response.writeHead(200, {
      "Content-Type": "application/octet-stream",
      "X-Fixture-Private": privateToken,
    });
    const timer = setInterval(() => {
      if (response.destroyed) return;
      response.write(payload);
      sentBytes += payload.length;
      if (++count >= chunks) {
        clearInterval(timer);
        timers.delete(timer);
        if (interrupt) response.destroy();
        else response.end();
      }
    }, interval);
    timers.add(timer);
    response.once("close", () => {
      clearInterval(timer);
      timers.delete(timer);
    });
  }
  const server = http.createServer((request, response) =>
    handler(request, response, stream),
  );
  server.listen(0, "127.0.0.1");
  await new Promise((resolve, reject) => {
    server.once("listening", resolve);
    server.once("error", reject);
  });
  const endpoint = `http://127.0.0.1:${server.address().port}`;
  try {
    return await run(endpoint, () => sentBytes);
  } finally {
    for (const timer of timers) clearInterval(timer);
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  }
}

async function transfer(
  root,
  url,
  { mode = "ordinary", seconds = 18, budget, hardSeconds = 25 } = {},
) {
  const metrics = new TransferMetrics();
  const records = [];
  let bytes = 0;
  const child = await runSafeChild(process.execPath, [fixture], {
    cwd: root,
    input: { mode, url },
    hardSeconds,
    onRecord(record) {
      records.push(record);
      if (record.type === "progress") {
        bytes = record.bytes;
        metrics.add(record.elapsed_s, record.bytes, record.phase);
        if (budget && record.bytes >= budget.roundLimit())
          return budget.reason(0, record.bytes);
      }
      return null;
    },
    onTick(elapsed) {
      return budget && elapsed >= seconds ? "round_time_budget" : null;
    },
  });
  metrics.stop(child.duration_s);
  if (budget) budget.account(bytes);
  assert.equal(
    JSON.stringify({ records, child, metrics: metrics.summary() }).includes(
      privateToken,
    ),
    false,
  );
  assert.equal(JSON.stringify(records).includes("http://"), false);
  return { child, metrics: metrics.summary(), bytes, records };
}

test(
  "real slow HTTP transfer supplies full 5s/10s windows and no private query/header output",
  { timeout: 30_000 },
  async () => {
    await ownedRun((root) =>
      withServer(
        (_request, response, stream) => stream(response, { chunks: 112 }),
        async (endpoint, sent) => {
          const result = await transfer(
            root,
            `${endpoint}/slow?token=${privateToken}`,
          );
          assert.equal(result.child.code, 0);
          assert.equal(result.child.killed, false);
          assert.equal(result.bytes, sent());
          assert.equal(result.bytes, 112 * 4096);
          assert(result.metrics.duration_s >= 10);
          assert(result.metrics.five_second_windows >= 2);
          assert(
            result.metrics.mean_Bps > 20 * 1024 &&
              result.metrics.mean_Bps < 60 * 1024,
          );
          assert(result.metrics.median_5s_Bps > 20 * 1024);
          assert(result.metrics.last_10s_Bps > 20 * 1024);
          assert.equal(result.records.at(-1).terminal, "completed");
        },
      ),
    );
  },
);

test(
  "real socket interruption produces one bounded retry and a truthful aggregate",
  { timeout: 15_000 },
  async () => {
    let requests = 0;
    await ownedRun((root) =>
      withServer(
        (_request, response, stream) => {
          requests++;
          stream(response, {
            chunks: requests === 1 ? 4 : 8,
            interval: 30,
            interrupt: requests === 1,
          });
        },
        async (endpoint, sent) => {
          const result = await transfer(
            root,
            `${endpoint}/interrupted?signature=${privateToken}`,
          );
          assert.equal(result.child.code, 0);
          assert.equal(requests, 2);
          assert.equal(
            result.records.filter((r) => r.type === "retry").length,
            1,
          );
          assert.equal(result.records.at(-1).retries, 1);
          assert(result.bytes > 8 * 4096 && result.bytes <= sent());
          assert.equal(result.metrics.last_10s_Bps, null);
        },
      ),
    );
  },
);

test(
  "local HLS fixture follows a real manifest and two fragments without confusing protocol evidence",
  { timeout: 15_000 },
  async () => {
    const manifest =
      "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\n/segment-1.ts\n#EXTINF:1,\n/segment-2.ts\n#EXT-X-ENDLIST\n";
    const paths = [];
    await ownedRun((root) =>
      withServer(
        (request, response, stream) => {
          const pathname = new URL(request.url, "http://127.0.0.1").pathname;
          paths.push(pathname);
          if (pathname === "/playlist.m3u8")
            response
              .writeHead(200, {
                "Content-Type": "application/vnd.apple.mpegurl",
              })
              .end(manifest);
          else stream(response, { chunks: 4, interval: 30 });
        },
        async (endpoint, sent) => {
          const result = await transfer(
            root,
            `${endpoint}/playlist.m3u8?token=${privateToken}`,
            { mode: "hls" },
          );
          assert.equal(result.child.code, 0);
          assert.deepEqual(paths, [
            "/playlist.m3u8",
            "/segment-1.ts",
            "/segment-2.ts",
          ]);
          assert.equal(result.bytes, sent());
          assert.equal(result.bytes, 8 * 4096);
          assert.equal(result.records.at(-1).segments, 2);
          assert.deepEqual(result.records[0], {
            type: "protocol",
            protocol: "fixture_hls",
            segmented: true,
          });
          assert.equal(result.metrics.median_5s_Bps, null);
        },
      ),
    );
  },
);

test(
  "actual bytes enforce per-round and total budgets and terminate only owned requests",
  { timeout: 20_000 },
  async () => {
    await ownedRun((root) =>
      withServer(
        (_request, response, stream) =>
          stream(response, { chunks: 1000, size: 1024, interval: 30 }),
        async (endpoint, sent) => {
          const budget = new DownloadBudget({
            seconds: 8,
            bytes: 64 * 1024,
            total: 128 * 1024,
          });
          const first = await transfer(
            root,
            `${endpoint}/budget-a?token=${privateToken}`,
            { budget },
          );
          assert.equal(first.child.killed, true);
          assert.equal(first.child.stop_reason, "round_byte_budget");
          assert(
            first.bytes >= 64 * 1024 && first.bytes < 96 * 1024,
            `observed cutoff bytes: ${first.bytes}`,
          );
          assert(budget.remaining > 0 && budget.remaining <= 64 * 1024);
          const second = await transfer(
            root,
            `${endpoint}/budget-b?token=${privateToken}`,
            { budget },
          );
          assert.equal(second.child.killed, true);
          assert.equal(second.child.stop_reason, "total_byte_budget");
          assert.equal(budget.remaining, 0);
          // Approximate guard allows only bounded in-flight data between last progress and cleanup.
          assert(sent() <= 192 * 1024, `total served bytes: ${sent()}`);
          const settled = sent();
          await wait(150);
          assert.equal(sent(), settled);
        },
      ),
    );
  },
);

test(
  "actual slow request is stopped at its deadline and downloaded bytes stop increasing",
  { timeout: 15_000 },
  async () => {
    await ownedRun((root) =>
      withServer(
        (_request, response, stream) =>
          stream(response, { chunks: 1000, size: 1024, interval: 30 }),
        async (endpoint, sent) => {
          const budget = new DownloadBudget({
            seconds: 2.5,
            bytes: 1024 * 1024,
          });
          const result = await transfer(
            root,
            `${endpoint}/deadline?token=${privateToken}`,
            { budget, seconds: 2.5, hardSeconds: 8 },
          );
          assert.equal(result.child.killed, true);
          assert.equal(result.child.stop_reason, "round_time_budget");
          assert(result.child.duration_s < 7);
          assert(result.bytes > 0 && result.bytes < 1024 * 1024);
          const settled = sent();
          await wait(150);
          assert.equal(sent(), settled);
        },
      ),
    );
  },
);

for (const mode of ["parent_exits", "parent_waits"]) {
  test(
    `Windows owned Job reaps descendant with inherited pipes: ${mode}`,
    { timeout: 15_000, skip: process.platform !== "win32" },
    async () => {
      await ownedRun(async (root) => {
        let pid = null;
        let wasAlive = false;
        let seenAt = null;
        const result = await runSafeChild(process.execPath, [fixture], {
          cwd: root,
          input: { mode },
          hardSeconds: 8,
          onRecord(record) {
            if (record.type === "descendant") {
              pid = record.pid;
              wasAlive = alive(pid);
              seenAt = performance.now();
            }
            return null;
          },
          onTick() {
            return mode === "parent_waits" &&
              seenAt !== null &&
              performance.now() - seenAt > 300
              ? "fixture_tree_cutoff"
              : null;
          },
        });
        assert.equal(wasAlive, true);
        if (mode === "parent_exits") {
          assert.equal(result.code, 0);
          assert.equal(result.killed, false);
        } else {
          assert.equal(result.killed, true);
          assert.equal(result.stop_reason, "fixture_tree_cutoff");
        }
        for (let i = 0; i < 20 && alive(pid); i++) await wait(50);
        assert.equal(alive(pid), false);
        assert(result.duration_s < 7);
      });
    },
  );
}
