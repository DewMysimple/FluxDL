// Summarize real desktop event observations without copying task data or paths.
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const source = path.resolve(root, process.argv[2] || "artifacts/desktop-smoke.json");
const output = path.resolve(root, process.argv[3] || "docs/engineering/ipc-after.json");
const label = process.argv[4] || "after";
if (!output.startsWith(root + path.sep)) throw new Error("Output must remain in this project");
const raw = fs.readFileSync(source);
const report = JSON.parse(raw);
if (report.errors?.length || !report.checks?.length) throw new Error("Smoke did not pass; refusing to present it as performance evidence");
const grouped = new Map();
for (const segment of report.events || []) {
  for (const record of segment.records) {
    const key = `${segment.window}:${record.event}:${record.type}`;
    if (!grouped.has(key)) grouped.set(key, { window: segment.window, event: record.event, type: record.type, records: [], segments: [] });
    const group = grouped.get(key);
    group.records.push(record);
    group.segments.push(`${segment.segment}:${record.sessionId || "legacy"}`);
  }
}
if (!grouped.size) throw new Error("No actual events observed; a type definition is not a measurement");
const observations = [...grouped.values()].map(({ window, event, type, records, segments }) => {
  const sizes = records.map(record => record.bytes).sort((a, b) => a - b);
  const quantile = p => sizes[Math.max(0, Math.ceil(sizes.length * p) - 1)];
  const bySegment = new Map();
  for (let index = 0; index < records.length; index++) {
    const key = segments[index];
    if (!bySegment.has(key)) bySegment.set(key, []);
    bySegment.get(key).push(records[index].at);
  }
  let peakOneSecondCount = 0;
  for (const times of bySegment.values()) {
    times.sort((a, b) => a - b);
    for (let start = 0, end = 0; end < times.length; end++) {
      while (times[end] - times[start] >= 1000) start++;
      peakOneSecondCount = Math.max(peakOneSecondCount, end - start + 1);
    }
  }
  return { window, event, type, count: records.length, payloadBytes: { p50: quantile(0.5), p95: quantile(0.95), min: sizes[0], max: sizes.at(-1) }, totalPayloadBytes: sizes.reduce((sum, value) => sum + value, 0), peakOneSecondCount };
});
const result = {
  schemaVersion: 1, label, generatedAt: new Date().toISOString(), runId: report.runId,
  originalReportSha256: createHash("sha256").update(raw).digest("hex"),
  passedChecks: report.checks.length, errors: [], observations,
  limitations: [
    "Real Tauri event payload UTF-8 sizes measured inside each WebView; excludes transport framing and encoding overhead.",
    "Event listener installed before smoke actions; early application startup events are not included.",
    "Peak count includes semantic events for that type; legacy snapshots do not identify progress versus semantic changes.",
    "The smoke has a small number of tasks; large-history payload is measured separately by engineering_bench.",
    "Windows background scheduling may batch delivery; observed peak is not the configured backend tick rate.",
  ],
};
fs.mkdirSync(path.dirname(output), { recursive: true });
fs.writeFileSync(output, JSON.stringify(result, null, 2) + "\n");
console.log(JSON.stringify({ output: path.relative(root, output), observations }, null, 2));
