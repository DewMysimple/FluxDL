export type Stage =
  | "session_environment"
  | "download_first_output"
  | "download_before_dl"
  | "download_before_dl_to_progress"
  | "download_process_exit"
  | "preview_cache_decision"
  | "enqueue_trusted_validate"
  | "session"
  | "parse_ui_request"
  | "parse_ipc_dispatch"
  | "parse_validate"
  | "parse_engine_spawn"
  | "parse_metadata_extract"
  | "parse_response"
  | "parse_total"
  | "enqueue_ui_request"
  | "enqueue_validate"
  | "enqueue_metadata_recheck"
  | "enqueue_persist"
  | "enqueue_db_queue"
  | "enqueue_db_write"
  | "enqueue_to_progress"
  | "queue_wait"
  | "download_engine_spawn"
  | "download_first_progress"
  | "download_active"
  | "download_postprocess"
  | "download_terminal"
  | "persistence_issue"
  | "download_process_issue"
  | "download_total";
export type DiagnosticStatus =
  | "started"
  | "ok"
  | "failed"
  | "cancelled"
  | "unobserved"
  | "notapplicable"
  | "interrupted";
export interface DiagnosticEvent {
  schema_version: 1;
  event_id: string;
  session_id: string;
  trace_id: string;
  span_id: string;
  parent_span_id: string | null;
  task_id: string | null;
  timestamp_utc: string;
  elapsed_since_session_start_ms: number;
  level: "info" | "warn" | "error" | "debug" | "trace";
  source: "rust" | "frontend";
  event: "stage_started" | "stage_finished" | "fault_observed";
  phase: Stage;
  status: DiagnosticStatus;
  duration_ms: number | null;
  fields: {
    cache_outcome?: string | null;
    output_stream?: "stdout" | "stderr" | null;
    environment_id?: string | null;
    environment?: {
      build: {
        revision: string | null;
        dirty: boolean | null;
        app_version: string;
      };
      engines: Record<string, string>;
    } | null;
    related_trace_id: string | null;
    attempt_id: string | null;
    error_code: string | null;
    error_kind?: string | null;
  };
}
export interface DiagnosticFilter {
  sessionId?: string;
  traceId?: string;
  taskId?: string;
  level?: "info" | "warn" | "error" | "debug" | "trace";
  source?: "rust" | "frontend";
  phase?: Stage;
  search?: string;
  fromUtc?: string;
  toUtc?: string;
  cursor?: number;
  limit?: number;
}
export interface DiagnosticPage {
  events: DiagnosticEvent[];
  total: number;
  nextCursor: number | null;
  unreadableLines: number;
  truncated: boolean;
}
export interface Statistic {
  cacheOutcome?: string | null;
  sessionId?: string;
  environmentId?: string | null;
  codeRevision?: string | null;
  engineVersions?: Record<string, string>;
  phase: Stage;
  label: string;
  samples: number;
  p50Ms: number;
  p95Ms: number;
  maxMs: number;
  slowestTraceId: string;
}
export interface Overview {
  cacheStats?: {
    hit: number;
    missing: number;
    expired: number;
    invalidated: number;
    disabled: number;
    duplicate: number;
    fullRechecks: number;
  };
  slowTraces: DiagnosticEvent[];
  sessionId: string;
  parses: number;
  downloads: number;
  errors: number;
  total: number;
  statistics: Statistic[];
  recentErrors: DiagnosticEvent[];
  unreadableLines: number;
  truncated: boolean;
  fromUtc: string | null;
  toUtc: string | null;
}
export interface Health {
  build?: {
    revision: string | null;
    dirty: boolean | null;
    app_version: string;
  };
  elapsedSinceSessionStartMs?: number;
  mode: string;
  sessionId: string;
  directory: string | null;
  writeRestricted: boolean;
  dropped: number;
  criticalDropped: number;
  generation: number;
  level: string;
}
export const stageLabels: Record<Stage, string> = {
  session: "应用会话",
  session_environment: "会话构建与引擎版本",
  download_first_output: "等待首次引擎输出（非网络首字节）",
  download_before_dl: "启动至下载前标记",
  download_before_dl_to_progress: "下载前标记至首次有效进度",
  download_process_exit: "进程运行至回收",
  preview_cache_decision: "可信预览缓存决策",
  enqueue_trusted_validate: "后端可信预览安全校验",
  parse_ui_request: "解析响应 · 前端时钟",
  parse_ipc_dispatch: "解析命令分派",
  parse_validate: "解析参数校验",
  parse_engine_spawn: "解析进程启动",
  parse_metadata_extract: "元数据提取 · 引擎与网络",
  parse_response: "解析结果封装",
  parse_total: "首次解析 · 后端",
  enqueue_ui_request: "入队响应 · 前端时钟",
  enqueue_validate: "入队本地校验",
  enqueue_metadata_recheck: "二次安全复核 · 引擎与网络",
  enqueue_persist: "入队持久化 · 含DB排队",
  enqueue_db_queue: "SQLite写入队列等待",
  enqueue_db_write: "SQLite事务执行 · 含锁等待",
  enqueue_to_progress: "入队到首次有效进度",
  queue_wait: "串行队列等待",
  download_engine_spawn: "下载进程启动",
  download_first_progress: "进程启动到首次有效进度",
  download_active: "下载阶段",
  download_postprocess: "后处理",
  download_terminal: "终态持久化",
  persistence_issue: "持久化异常 · 时点",
  download_process_issue: "下载进程异常 · 时点",
  download_total: "下载尝试总用时",
};
export const statusLabels: Record<DiagnosticStatus, string> = {
  started: "进行中",
  ok: "已完成",
  failed: "失败",
  cancelled: "已取消",
  unobserved: "未观测到",
  notapplicable: "不适用",
  interrupted: "中断",
};

export const phaseLabel = (phase: string) =>
  stageLabels[phase as Stage] ?? "未知阶段（无有效语义记录）";
