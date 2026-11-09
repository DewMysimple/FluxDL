import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const option = (name, fallback) => {
  const index = args.indexOf(name);
  return index < 0 ? fallback : args[index + 1];
};
const phase = option("--phase", "after-refactor");
const output = option("--output", "docs/engineering/frontend-after.json");
if (!phase || !output) throw new Error("--phase and --output require values");
const report = path.resolve(root, output);
if (!report.startsWith(root + path.sep)) {
  throw new Error("Benchmark output must stay inside this project");
}
const child = spawnSync(
  process.execPath,
  [path.join(root, "node_modules/vitest/vitest.mjs"), "run", "src/test/frontend-baseline.test.tsx"],
  {
    cwd: root,
    env: {
      ...process.env,
      FLUXDL_FRONTEND_BASELINE: "1",
      FLUXDL_FRONTEND_PHASE: phase,
      FLUXDL_FRONTEND_REPORT: report,
    },
    windowsHide: true,
    stdio: "inherit",
  },
);
if (child.error) throw child.error;
process.exitCode = child.status ?? 1;
