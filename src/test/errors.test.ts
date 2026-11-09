import { describe, expect, it } from "vitest";
import {
  errorText,
  isStructuredIpcError,
  unknownErrorMessage,
} from "../lib/errors";
import type { StructuredIpcError } from "../lib/types";

describe("structured and legacy IPC error boundary", () => {
  it("keeps legacy string and native Error messages unchanged", () => {
    expect(errorText("旧版错误，请重试")).toBe("旧版错误，请重试");
    expect(errorText(new Error("原生错误说明"))).toBe("原生错误说明");
    expect(errorText(Object("旧版字符串对象"))).toBe("旧版字符串对象");
  });
  it("accepts the exact v1 DTO and exposes only its public message", () => {
    const error: StructuredIpcError = {
      version: 1,
      code: "ENGINE_NOT_FOUND",
      phase: "engine_detection",
      message: "找不到下载引擎，请在设置中选择路径",
      details: "private-diagnostic-context",
    };
    expect(isStructuredIpcError(error)).toBe(true);
    expect(errorText(error)).toBe(error.message);
    expect(errorText(error)).not.toContain(error.details);
    expect(
      isStructuredIpcError({
        version: 1,
        code: "NETWORK",
        phase: "probe",
        message: "请检查网络",
      }),
    ).toBe(true);
  });
  it.each([
    null,
    undefined,
    12,
    true,
    {},
    [],
    { message: "unversioned" },
    { version: 2, code: "NETWORK", phase: "probe", message: "wrong version" },
    { version: 1, code: 12, phase: "probe", message: "wrong code" },
    { version: 1, code: "NETWORK", phase: "probe", message: " " },
    {
      version: 1,
      code: "NETWORK",
      phase: "probe",
      message: "invalid details",
      details: null,
    },
  ])("uses a readable fallback for malformed input %#", (value) => {
    expect(isStructuredIpcError(value)).toBe(false);
    expect(errorText(value)).toBe(unknownErrorMessage);
    expect(errorText(value)).not.toBe("[object Object]");
  });
  it("contains property getter failures rather than throwing during error rendering", () => {
    const error = {
      get version() {
        throw new Error("getter failure");
      },
    };
    expect(() => errorText(error)).not.toThrow();
    expect(errorText(error)).toBe(unknownErrorMessage);
  });
});
