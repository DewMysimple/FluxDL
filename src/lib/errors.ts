import type { StructuredIpcError } from "./types";

export const unknownErrorMessage = "操作失败，未能读取错误信息，请重试。";
const nonemptyString = (value: unknown): value is string =>
  typeof value === "string" && value.trim().length > 0;

export function isStructuredIpcError(
  value: unknown,
): value is StructuredIpcError {
  try {
    if (typeof value !== "object" || value === null || Array.isArray(value))
      return false;
    return (
      "version" in value &&
      value.version === 1 &&
      "code" in value &&
      nonemptyString(value.code) &&
      "phase" in value &&
      nonemptyString(value.phase) &&
      "message" in value &&
      nonemptyString(value.message) &&
      (!("details" in value) ||
        value.details === undefined ||
        typeof value.details === "string")
    );
  } catch {
    // A thrown object can contain failing getters; formatting must remain safe.
    return false;
  }
}

/** Show only the public summary. Diagnostic details never enter product toasts. */
export function errorText(error: unknown): string {
  try {
    if (typeof error === "string") return error;
    if (error instanceof Error) {
      const message = error.message;
      return nonemptyString(message) ? message : unknownErrorMessage;
    }
    if (isStructuredIpcError(error)) {
      const message = error.message;
      return nonemptyString(message) ? message : unknownErrorMessage;
    }
    if (error instanceof String) {
      const message = error.valueOf();
      return typeof message === "string" ? message : unknownErrorMessage;
    }
    return unknownErrorMessage;
  } catch {
    return unknownErrorMessage;
  }
}
