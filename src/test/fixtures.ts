import { previewSnapshot } from "../lib/api";
import { useApp } from "../stores/app";
import { createTaskDictionary } from "../stores/cache";
import type {
  AppSnapshot,
  AppChange,
  AppEvent,
  DownloadTask,
  MediaInfo,
} from "../lib/types";

export function mediaFixture(overrides: Partial<MediaInfo> = {}): MediaInfo {
  return {
    url: "https://example.org/authorized-test.mp4",
    title: "测试媒体",
    mediaId: "fixture",
    source: "Example",
    duration: 42,
    thumbnail: null,
    isPlaylist: false,
    formats: [
      {
        formatId: "mp4",
        ext: "mp4",
        label: "720p MP4",
        height: 720,
        hasVideo: true,
        hasAudio: true,
        filesize: null,
        requiresFfmpeg: false,
      },
    ],
    warnings: [],
    ...overrides,
  };
}

export function taskFixture(
  id: string,
  overrides: Partial<DownloadTask> = {},
): DownloadTask {
  return {
    id,
    media: mediaFixture({ mediaId: id, title: `媒体 ${id}` }),
    mode: "video",
    quality: "720",
    formatId: null,
    outputDir: "C:\\Downloads",
    status: "queued",
    progress: null,
    speed: null,
    eta: null,
    downloadedBytes: 0,
    totalBytes: null,
    finalPath: null,
    error: null,
    logs: [],
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    fileExists: false,
    ...overrides,
  };
}

export function snapshotFixture(tasks: DownloadTask[] = []): AppSnapshot {
  const engine = {
    path: "fixture.exe",
    version: "fixture",
    source: "fixture",
    available: true,
    message: "",
  };
  return {
    sessionId: "fixture-session",
    revision: 0,
    settings: { ...previewSnapshot.settings, downloadDir: "C:\\Downloads" },
    engines: {
      ytdlp: { ...engine },
      ffmpeg: { ...engine },
      ffprobe: { ...engine },
      runtime: { ...engine },
      warnings: [],
    },
    tasks,
  };
}

export function installMatchMedia() {
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    value: () => ({
      matches: false,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
    }),
  });
}

/** Reset the singleton's lifecycle/cache between independent component tests. */
export function seedApp(snapshot: AppSnapshot, loading = false) {
  useApp.getState().beginSync(async () => undefined);
  useApp.setState({
    sessionId: null,
    revision: 0,
    tasksById: createTaskDictionary(),
    taskIds: [],
    listingRevision: 0,
  });
  useApp.getState().setSnapshot(snapshot);
  useApp.setState({ loading, error: null, errorSource: null });
}

export function eventFixture(
  revision: number,
  change: AppChange,
  sessionId = "fixture-session",
): AppEvent {
  return { sessionId, revision, change };
}
