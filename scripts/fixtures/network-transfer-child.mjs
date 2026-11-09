// Local fixture only: transfers generated loopback payloads and emits numeric records.
// It is not yt-dlp and supplies no evidence of YouTube performance.
import fs from "node:fs";
import http from "node:http";
import { spawn } from "node:child_process";
import { setTimeout as wait } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const emit = (record) => process.stdout.write(`${JSON.stringify(record)}\n`);
if (process.argv.includes("--grandchild")) {
  emit({ type: "grandchild_ready", pid: process.pid });
  setInterval(
    () => emit({ type: "grandchild_heartbeat", pid: process.pid }),
    100,
  );
} else {
  const raw = fs.readFileSync(0, "utf8");
  if (raw.length > 8192) process.exit(2);
  const input = JSON.parse(raw);
  if (["parent_exits", "parent_waits"].includes(input.mode)) {
    const child = spawn(
      process.execPath,
      [fileURLToPath(import.meta.url), "--grandchild"],
      { windowsHide: true, stdio: ["ignore", "inherit", "inherit"] },
    );
    child.unref();
    emit({ type: "descendant", pid: child.pid });
    if (input.mode === "parent_exits") setTimeout(() => process.exit(0), 500);
    else setInterval(() => emit({ type: "parent_heartbeat" }), 100);
  } else {
    const parsed = new URL(input.url);
    if (
      parsed.protocol !== "http:" ||
      parsed.hostname !== "127.0.0.1" ||
      !parsed.port ||
      parsed.username ||
      parsed.password ||
      !["ordinary", "hls"].includes(input.mode)
    )
      process.exit(2);
    const start = performance.now();
    let bytes = 0;
    let phase = "downloading";
    let retries = 0;
    let segments = 0;
    const elapsed = () => (performance.now() - start) / 1000;
    const progress = () =>
      emit({ type: "progress", elapsed_s: elapsed(), bytes, phase });
    const timer = setInterval(progress, 100);
    function request(url, metadata = false) {
      return new Promise((resolve, reject) => {
        const parts = [];
        let metadataBytes = 0;
        const req = http.get(url, (response) => {
          if (response.statusCode !== 200) {
            response.resume();
            reject(new Error("fixture_status"));
            return;
          }
          response.on("data", (chunk) => {
            if (metadata) {
              metadataBytes += chunk.length;
              if (metadataBytes > 16 * 1024) {
                req.destroy();
                reject(new Error("fixture_manifest_bound"));
                return;
              }
              parts.push(chunk);
            } else {
              bytes += chunk.length;
              progress();
            }
          });
          response.once("aborted", () => reject(new Error("fixture_abort")));
          response.once("error", () => reject(new Error("fixture_read")));
          response.once("end", () =>
            resolve(metadata ? Buffer.concat(parts).toString("utf8") : ""),
          );
        });
        req.setTimeout(5000, () => req.destroy(new Error("fixture_timeout")));
        req.once("error", () => reject(new Error("fixture_request")));
      });
    }
    async function transfer(url) {
      for (let attempt = 0; ; attempt++) {
        try {
          await request(url);
          segments++;
          return;
        } catch {
          if (attempt >= 1) throw new Error("fixture_retry_bound");
          retries++;
          emit({
            type: "retry",
            count: retries,
            error_code: "FIXTURE_INTERRUPTED",
          });
          await wait(30);
        }
      }
    }
    try {
      if (input.mode === "hls") {
        emit({ type: "protocol", protocol: "fixture_hls", segmented: true });
        const manifest = await request(parsed, true);
        if (!manifest.startsWith("#EXTM3U"))
          throw new Error("fixture_manifest");
        const paths = manifest
          .split(/\r?\n/)
          .filter((line) => line && !line.startsWith("#"));
        if (!paths.length || paths.length > 4)
          throw new Error("fixture_segment_bound");
        for (const part of paths) {
          const segment = new URL(part, parsed);
          if (segment.origin !== parsed.origin)
            throw new Error("fixture_foreign_origin");
          await transfer(segment);
        }
      } else {
        emit({ type: "protocol", protocol: "fixture_http", segmented: false });
        await transfer(parsed);
      }
      phase = "finished";
      progress();
      emit({ type: "result", terminal: "completed", bytes, retries, segments });
    } catch {
      emit({ type: "failure", error_code: "FIXTURE_NETWORK_FAILURE" });
      process.exitCode = 1;
    } finally {
      clearInterval(timer);
    }
  }
}
