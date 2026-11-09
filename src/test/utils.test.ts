import { describe, expect, it } from "vitest";
import {
  bytes,
  duration,
  isActive,
  isRetryable,
  validateUrl,
} from "../lib/utils";

describe("user input and lifecycle presentation", () => {
  it("accepts only complete HTTP links without embedded credentials", () => {
    expect(validateUrl("https://example.org/video.mp4?x=1&y=2")).toBeNull();
    for (const url of [
      "",
      "--exec calc.exe",
      "file:///C:/private",
      "javascript:alert(1)",
      "https://user:secret@example.org/video",
    ])
      expect(validateUrl(url)).not.toBeNull();
  });
  it("does not present unknown byte counts or durations as measured values", () => {
    expect(bytes(null)).toBe("—");
    expect(bytes(Number.NaN)).toBe("—");
    expect(bytes(1024)).toBe("1.0 KB");
    expect(duration(null)).toBe("时长未知");
    expect(duration(3661)).toBe("1:01:01");
  });
  it("keeps cancelling active and interrupted retryable", () => {
    expect(isActive("cancelling")).toBe(true);
    expect(isActive("completed")).toBe(false);
    expect(isRetryable("interrupted")).toBe(true);
    expect(isRetryable("downloading")).toBe(false);
  });
});
