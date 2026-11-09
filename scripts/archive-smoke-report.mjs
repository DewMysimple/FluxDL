// Keep reviewable evidence without publishing local engine/data/output paths.
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const source = path.resolve(root, process.argv[2] || "artifacts/desktop-smoke.json");
const output = path.resolve(root, process.argv[3] || "docs/engineering/desktop-smoke-final.json");
if (!output.startsWith(root + path.sep)) throw new Error("Output must remain in this project");
const raw = fs.readFileSync(source);
const report = JSON.parse(raw);
if (report.errors?.length || !report.checks?.length) throw new Error("Smoke did not pass; keep the diagnostic report locally");
const summary = {
  schemaVersion: 1,
  runId: report.runId,
  originalReportSha256: createHash("sha256").update(raw).digest("hex"),
  checks: report.checks,
  errors: report.errors,
  ...(report.upgrade ? { upgrade: report.upgrade } : {}),
  privacy: "Local executable, database and output paths omitted; event sizes are reported separately.",
};
fs.mkdirSync(path.dirname(output), { recursive: true });
fs.writeFileSync(output, JSON.stringify(summary, null, 2) + "\n");
console.log(JSON.stringify({ output: path.relative(root, output), checks: summary.checks.length, upgrade: summary.upgrade }, null, 2));
