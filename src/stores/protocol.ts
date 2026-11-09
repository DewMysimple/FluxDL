import type {
  AppEvent,
  DownloadTask,
  EngineDiagnostics,
  EngineInfo,
  FormatOption,
  MediaInfo,
  Settings,
  TaskStatus,
} from "../lib/types";

const record = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const number = (value: unknown): value is number =>
  typeof value === "number" && Number.isFinite(value);
const nullableNumber = (value: unknown) => value === null || number(value);
const nullableString = (value: unknown) =>
  value === null || typeof value === "string";
const strings = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every((item) => typeof item === "string");
const status = (value: unknown): value is TaskStatus =>
  typeof value === "string" &&
  [
    "queued",
    "resolving",
    "downloading",
    "postprocessing",
    "completed",
    "failed",
    "cancelling",
    "cancelled",
    "interrupted",
  ].includes(value);
function format(value: unknown): value is FormatOption {
  return (
    record(value) &&
    ["formatId", "ext", "label"].every(
      (key) => typeof value[key] === "string",
    ) &&
    nullableNumber(value.height) &&
    nullableNumber(value.filesize) &&
    ["hasVideo", "hasAudio", "requiresFfmpeg"].every(
      (key) => typeof value[key] === "boolean",
    )
  );
}
function media(value: unknown): value is MediaInfo {
  return (
    record(value) &&
    ["url", "title", "mediaId", "source"].every(
      (key) => typeof value[key] === "string",
    ) &&
    nullableNumber(value.duration) &&
    nullableString(value.thumbnail) &&
    typeof value.isPlaylist === "boolean" &&
    Array.isArray(value.formats) &&
    value.formats.every(format) &&
    strings(value.warnings)
  );
}
function task(value: unknown): value is DownloadTask {
  return (
    record(value) &&
    ["id", "mode", "quality", "outputDir", "createdAt", "updatedAt"].every(
      (key) => typeof value[key] === "string",
    ) &&
    media(value.media) &&
    status(value.status) &&
    nullableString(value.formatId) &&
    nullableString(value.finalPath) &&
    nullableString(value.error) &&
    nullableNumber(value.progress) &&
    nullableNumber(value.speed) &&
    nullableNumber(value.eta) &&
    number(value.downloadedBytes) &&
    nullableNumber(value.totalBytes) &&
    strings(value.logs) &&
    typeof value.fileExists === "boolean"
  );
}
function settings(value: unknown): value is Settings {
  return (
    record(value) &&
    [
      "downloadDir",
      "enginePath",
      "ffmpegPath",
      "defaultMode",
      "quality",
      "theme",
      "density",
      "closeBehavior",
      "completionAction",
    ].every((key) => typeof value[key] === "string") &&
    typeof value.reducedMotion === "boolean"
  );
}
function engine(value: unknown): value is EngineInfo {
  return (
    record(value) &&
    nullableString(value.path) &&
    nullableString(value.version) &&
    typeof value.source === "string" &&
    typeof value.available === "boolean" &&
    typeof value.message === "string"
  );
}
function diagnostics(value: unknown): value is EngineDiagnostics {
  return (
    record(value) &&
    engine(value.ytdlp) &&
    engine(value.ffmpeg) &&
    engine(value.ffprobe) &&
    engine(value.runtime) &&
    strings(value.warnings)
  );
}

/** Validate the small transport boundary instead of asserting unknown JSON. */
export function isAppEvent(value: unknown): value is AppEvent {
  if (
    !record(value) ||
    typeof value.sessionId !== "string" ||
    !value.sessionId.length ||
    value.sessionId.length > 256 ||
    !Number.isSafeInteger(value.revision) ||
    !number(value.revision) ||
    value.revision < 0 ||
    !record(value.change)
  )
    return false;
  const change = value.change;
  switch (change.type) {
    case "checkpoint":
      return true;
    case "taskUpsert":
      return task(change.task) && Number.isSafeInteger(change.index);
    case "taskProgress":
      return (
        typeof change.taskId === "string" &&
        status(change.status) &&
        nullableNumber(change.progress) &&
        nullableNumber(change.speed) &&
        nullableNumber(change.eta) &&
        number(change.downloadedBytes) &&
        nullableNumber(change.totalBytes) &&
        typeof change.updatedAt === "string" &&
        strings(change.logsAppend)
      );
    case "taskRemoved":
      return typeof change.taskId === "string";
    case "settingsUpdated":
      return settings(change.settings);
    case "diagnosticsUpdated":
      return diagnostics(change.engines);
    case "fileAvailabilityUpdated":
      return (
        Array.isArray(change.files) &&
        change.files.every(
          (file) =>
            record(file) &&
            typeof file.taskId === "string" &&
            typeof file.fileExists === "boolean",
        )
      );
    default:
      return false;
  }
}
