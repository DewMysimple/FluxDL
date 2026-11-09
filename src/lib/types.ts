export type TaskStatus =
  | "queued"
  | "resolving"
  | "downloading"
  | "postprocessing"
  | "completed"
  | "failed"
  | "cancelling"
  | "cancelled"
  | "interrupted";
export interface Settings {
  downloadDir: string;
  enginePath: string;
  ffmpegPath: string;
  defaultMode: string;
  quality: string;
  theme: string;
  density: string;
  reducedMotion: boolean;
  closeBehavior: string;
  completionAction: string;
}
export interface EngineInfo {
  path: string | null;
  version: string | null;
  source: string;
  available: boolean;
  message: string;
}
export interface EngineDiagnostics {
  ytdlp: EngineInfo;
  ffmpeg: EngineInfo;
  ffprobe: EngineInfo;
  runtime: EngineInfo;
  warnings: string[];
}
export interface FormatOption {
  formatId: string;
  ext: string;
  label: string;
  height: number | null;
  hasVideo: boolean;
  hasAudio: boolean;
  filesize: number | null;
  requiresFfmpeg: boolean;
}
export interface MediaInfo {
  url: string;
  title: string;
  mediaId: string;
  source: string;
  duration: number | null;
  thumbnail: string | null;
  isPlaylist: boolean;
  formats: FormatOption[];
  warnings: string[];
}
export interface EnqueueInput {
  media: MediaInfo;
  mode: string;
  quality: string;
  formatId: string | null;
  outputDir: string | null;
}
export interface DownloadTask {
  id: string;
  media: MediaInfo;
  mode: string;
  quality: string;
  formatId: string | null;
  outputDir: string;
  status: TaskStatus;
  progress: number | null;
  speed: number | null;
  eta: number | null;
  downloadedBytes: number;
  totalBytes: number | null;
  finalPath: string | null;
  error: string | null;
  logs: string[];
  createdAt: string;
  updatedAt: string;
  fileExists: boolean;
}
export interface AppSnapshot {
  settings: Settings;
  engines: EngineDiagnostics;
  tasks: DownloadTask[];
}
