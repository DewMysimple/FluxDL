import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import net from "node:net";
import { fileURLToPath } from "node:url";
import {
  DownloadBudget,
  TransferMetrics,
  localProxy,
  proxyCandidates,
  probeLocalProxy,
  safeRoot,
  speedLabel,
  runSafeChild,
  selectFormat,
  totalBudgetBytes,
  firstProgressDuration,
  safeStage,
  mergeStages,
  conditionList,
} from "./network-core.mjs";
const workspace = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

test("format selection holds default 1080 quality instead of silently preferring lower muxed media", () => {
  const formats = [
    {
      id: "18",
      has_video: true,
      has_audio: true,
      height: 360,
      protocol: "https",
    },
    {
      id: "137",
      has_video: true,
      has_audio: false,
      height: 1080,
      protocol: "https",
    },
    {
      id: "2160",
      has_video: true,
      has_audio: false,
      height: 2160,
      protocol: "https",
    },
  ];
  assert.equal(selectFormat(formats).id, "137");
  assert.equal(selectFormat(formats, "18").id, "18");
  assert.equal(selectFormat(formats, "missing"), null);
});
test("local proxy accepts only no-credential loopback endpoints; query/signature/path rejected", () => {
  assert.deepEqual(localProxy("http://127.0.0.1:7897"), {
    protocol: "http",
    port: 7897,
    host: "127.0.0.1",
  });
  assert.equal(localProxy("socks5://localhost:7897").protocol, "socks5");
  assert.equal(localProxy("http://localhost:80").port, 80);
  assert.equal(localProxy("http://localhost"), null);
  for (const value of [
    "http://user:PRIVATE_PROXY@localhost:7897",
    "http://example.com:7897",
    "http://127.0.0.1:0",
    "http://localhost:7897/?token=PRIVATE",
    "http://localhost:7897/config",
    "socks5h://localhost:7897",
  ])
    assert.equal(localProxy(value), null);
});
test("proxy candidates cross-check core process ownership and ignore Verge GUI listener", () => {
  const facts = {
    processes: [
      { name: "verge-mihomo", pid: 1 },
      { name: "clash-verge", pid: 2 },
    ],
    listeners: [
      { pid: 1, address: "127.0.0.1", port: 7897 },
      { pid: 2, address: "127.0.0.1", port: 14674 },
      { pid: 1, address: "127.0.0.1", port: 7898 },
    ],
  };
  assert.deepEqual(proxyCandidates(facts), [7897, 7898]);
  assert.deepEqual(proxyCandidates({}), []);
});
test("bounded loopback protocol probes recognize socks/http and reject incorrect service", async () => {
  const server = net.createServer((socket) =>
    socket.once("data", (b) => {
      if (b[0] === 5) {
        socket.end(Buffer.from([5, 0]));
        return;
      }
      const target = /^CONNECT 127\.0\.0\.1:(\d+) HTTP/.exec(b.toString());
      if (!target) {
        socket.end("HTTP/1.1 404 Not Found\r\n\r\n");
        return;
      }
      const tunnel = net.connect(Number(target[1]), "127.0.0.1", () => {
        socket.write("HTTP/1.1 200 Connection Established\r\n\r\n");
        tunnel.pipe(socket);
        socket.pipe(tunnel);
      });
      tunnel.on("error", () => socket.destroy());
      socket.on("error", () => tunnel.destroy());
    }),
  );
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const port = server.address().port;
  try {
    assert.equal((await probeLocalProxy(port, "socks5")).confirmed, true);
    assert.equal((await probeLocalProxy(port, "http")).confirmed, true);
  } finally {
    await new Promise((r) => server.close(r));
  }
  assert.equal((await probeLocalProxy(port, "http", 100)).confirmed, false);
  for (const status of [404, 407]) {
    const wrong = net.createServer((s) =>
      s.once("data", () => s.end(`HTTP/1.1 ${status} Unavailable\r\n\r\n`)),
    );
    await new Promise((r) => wrong.listen(0, "127.0.0.1", r));
    const result = await probeLocalProxy(wrong.address().port, "http");
    assert.equal(result.confirmed, false);
    assert.equal(result.authentication_required, status === 407);
    await new Promise((r) => wrong.close(r));
  }
});
test("5s/10s metrics use real byte deltas, distinguish kb/s units, stop during postprocessing", () => {
  const m = new TransferMetrics();
  for (let t = 0; t <= 15; t++) m.add(5 + t, 1024 + 100 * 1024 * t);
  m.add(21, 100 * 1024 * 20, "postprocessing");
  m.add(99, 1e9);
  const r = m.summary();
  assert.equal(r.downloaded_bytes, 100 * 1024 * 15);
  assert.equal(r.duration_s, 15);
  assert.equal(r.mean_Bps, 102400);
  assert.equal(r.median_5s_Bps, 102400);
  assert.equal(r.last_10s_Bps, 102400);
  assert.equal(r.first_progress_s, 5);
  assert.equal(speedLabel(102400), "100.00 KiB/s");
  assert.equal((102400 * 8) / 1000, 819.2);
});
test("missing progress/short sample/unknown total stays unmeasured; real stalls and counters are bounded", () => {
  const m = new TransferMetrics();
  m.add(0, 0);
  assert.equal(m.summary().mean_Bps, null);
  m.add(1, 1024);
  m.add(2, 2048);
  for (let t = 3; t <= 7; t++) m.add(t, 2048);
  m.add(8, 4096);
  const r = m.summary();
  assert.equal(r.stall_count, 1);
  assert.equal(r.stall_seconds, 5);
  assert.equal(r.last_10s_Bps, null);
  m.add(9, 1);
  assert.equal(m.summary().observed_total_bytes, 4096);
  const final = new TransferMetrics();
  final.add(0, 100);
  final.add(1, 200);
  final.add(2, 400, "finished");
  assert.equal(final.summary().downloaded_bytes, 300);
  assert.equal(final.summary().duration_s, 2);
  final.add(3, 1000);
  assert.equal(final.summary().downloaded_bytes, 300);
});
test("time, per-round and total byte budgets clamp caller inputs", () => {
  const b = new DownloadBudget({ seconds: 999, bytes: 1e12, total: 1e12 });
  assert.equal(b.seconds, 45);
  assert.equal(b.roundLimit(), 40 * 1024 * 1024);
  assert.equal(b.reason(45, 0), "round_time_budget");
  assert.equal(b.reason(0, b.roundLimit()), "round_byte_budget");
  b.account(299 * 1024 * 1024);
  assert.equal(b.roundLimit(), 1024 * 1024);
  assert.equal(b.reason(0, 1024 * 1024), "total_byte_budget");
  for (const x of [NaN, Infinity, -Infinity])
    for (const key of ["seconds", "bytes", "total"])
      assert.throws(() => new DownloadBudget({ [key]: x }));
  assert.throws(() => b.reason(NaN, 1));
  assert.throws(() => b.account(Infinity));
  assert.throws(() => runSafeChild(process.execPath, [], { hardSeconds: NaN }));
});
test("cross-run remaining media budget is an explicit finite integer bounded by 300MiB", () => {
  assert.equal(totalBudgetBytes(), 300 * 1024 * 1024);
  const remaining = 300 * 1024 * 1024 - 20448639;
  const b = new DownloadBudget({ total: totalBudgetBytes(remaining) });
  assert.equal(b.remaining, remaining);
  b.account(remaining - 1);
  assert.equal(b.roundLimit(), 1);
  assert.equal(b.reason(0, 1), "total_byte_budget");
  for (const value of [
    0,
    -1,
    1.5,
    NaN,
    Infinity,
    300 * 1024 * 1024 + 1,
    null,
    "100",
    {},
  ])
    assert.throws(() => totalBudgetBytes(value));
});
test("100ms progress yields a bounded observed maximum rather than an always-null rate", () => {
  const m = new TransferMetrics();
  for (let n = 0; n <= 20; n++) m.add(n / 10, 1024 + n * 1024);
  assert(m.summary().max_Bps > 10000 && m.summary().max_Bps < 11000);
});
test("precise first-progress evidence survives a real owned cutoff before any final result", async () => {
  const root = safeRoot(
    workspace,
    path.join(
      workspace,
      "artifacts/network-diagnostics",
      `first-progress-test-${crypto.randomUUID()}`,
    ),
  );
  fs.mkdirSync(root, { recursive: true });
  let first = null;
  const result = await runSafeChild(
    process.execPath,
    [
      "-e",
      "console.log(JSON.stringify({type:'first_progress',duration_s:2.5}));setInterval(()=>{},1000)",
    ],
    {
      cwd: root,
      hardSeconds: 1.5,
      onRecord: (r) => {
        const value = firstProgressDuration(r);
        if (value !== null) first ??= value;
      },
    },
  );
  assert.equal(result.killed, true);
  assert.equal(first, 2.5);
  for (const value of [null, -1, NaN, Infinity, "2.5"])
    assert.equal(
      firstProgressDuration({ type: "first_progress", duration_s: value }),
      null,
    );
  assert.equal(
    firstProgressDuration({ type: "progress", duration_s: 1 }),
    null,
  );
  assert.deepEqual(fs.readdirSync(root), []);
  fs.rmdirSync(root);
});
test("only owned artifact descendant accepted; unrelated/user downloads roots rejected", () => {
  assert.equal(
    safeRoot(
      workspace,
      path.join(workspace, "artifacts/network-diagnostics/test-owned"),
    ),
    path.join(workspace, "artifacts/network-diagnostics/test-owned"),
  );
  for (const p of [
    os.tmpdir(),
    path.join(workspace, "artifacts/network-diagnostics"),
    path.join(workspace, "artifacts/network-diagnostics/../../src"),
  ])
    assert.throws(() => safeRoot(workspace, p));
});
test("condition selection is nonempty A/B/C only, deduplicated, and defaults to original matrix", () => {
  assert.deepEqual(conditionList(), ["A", "B", "C"]);
  assert.deepEqual(conditionList(["A", "A"]), ["A"]);
  assert.deepEqual(conditionList(["C", "A", "C"]), ["C", "A"]);
  for (const value of [null, [], ["D"], ["A", null], "A", [1], ["a"]])
    assert.throws(() => conditionList(value));
});
test("stage evidence is a bounded enum/numeric whitelist and excludes unknown private fields", () => {
  const fixture = {
    phase: "parse_total",
    status: "ok",
    duration_ms: 1234,
    raw_url: "https://private.invalid/?sig=PRIVATE_SIG",
    title: "PRIVATE_TITLE",
  };
  assert.deepEqual(safeStage(fixture), {
    phase: "parse_total",
    status: "ok",
    duration_ms: 1234,
  });
  assert.equal(
    JSON.stringify(mergeStages([fixture, fixture])).includes("PRIVATE_"),
    false,
  );
  assert.equal(mergeStages([fixture, fixture]).length, 1);
  for (const value of [
    { phase: "PRIVATE_TITLE", status: "ok", duration_ms: 1 },
    { phase: "parse_total", status: "PRIVATE_SIG", duration_ms: 1 },
    ...[-1, NaN, Infinity, "1234"].map((duration_ms) => ({
      phase: "parse_total",
      status: "ok",
      duration_ms,
    })),
  ])
    assert.equal(safeStage(value), null);
  assert.equal(
    mergeStages(
      Array.from({ length: 200 }, (_, duration_ms) => ({
        phase: "parse_total",
        status: "ok",
        duration_ms,
      })),
    ).length,
    128,
  );
});
test("live completed and open stages survive a real deadline without a final record", async () => {
  const root = safeRoot(
    workspace,
    path.join(
      workspace,
      "artifacts/network-diagnostics",
      `stage-test-${crypto.randomUUID()}`,
    ),
  );
  fs.mkdirSync(root, { recursive: true });
  let evidence = [];
  const result = await runSafeChild(
    process.execPath,
    [
      "-e",
      "for(const stage of [{phase:'parse_total',status:'ok',duration_ms:7000},{phase:'enqueue_metadata_recheck',status:'ok',duration_ms:8000},{phase:'download_first_progress',status:'started',duration_ms:null}])console.log(JSON.stringify({type:'stage',stage}));setInterval(()=>{},1000)",
    ],
    {
      cwd: root,
      hardSeconds: 1.5,
      onRecord: (r) => {
        if (r.type === "stage") evidence = mergeStages(evidence, [r.stage]);
      },
    },
  );
  assert.equal(result.killed, true);
  assert.equal(evidence.length, 3);
  assert.equal(evidence[0].duration_ms, 7000);
  assert.equal(evidence[2].duration_ms, null);
  assert.deepEqual(fs.readdirSync(root), []);
  fs.rmdirSync(root);
});
test("real child byte cutoff and deadline terminate owned process without leaking raw output", async () => {
  const root = path.join(
    workspace,
    "artifacts/network-diagnostics",
    `self-test-${crypto.randomUUID()}`,
  );
  fs.mkdirSync(root, { recursive: true });
  const code =
    "let n=0;setInterval(()=>console.log(JSON.stringify({type:'progress',bytes:++n*1024})),20);console.error('PRIVATE_SIGNED_URL token=PRIVATE_PROXY');";
  let bytes = 0;
  const result = await runSafeChild(process.execPath, ["-e", code], {
    cwd: root,
    hardSeconds: 3,
    onRecord: (r) => {
      bytes = r.bytes;
      return bytes >= 4096 ? "round_byte_budget" : null;
    },
  });
  assert.equal(result.killed, true);
  assert.equal(result.stop_reason, "round_byte_budget");
  assert.equal(JSON.stringify(result).includes("PRIVATE_"), false);
  const timed = await runSafeChild(
    process.execPath,
    ["-e", "setInterval(()=>{},1000)"],
    { cwd: root, hardSeconds: 0.15 },
  );
  assert.equal(timed.stop_reason, "hard_time_budget");
  assert(timed.duration_s < 8);
  // This fixture created no files, and the validated root is removed non-recursively.
  assert.deepEqual(fs.readdirSync(root), []);
  fs.rmdirSync(root);
});
