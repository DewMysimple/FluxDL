import { useCallback, useEffect, useRef, useState } from "react";
import {
  Activity,
  ArrowRight,
  Copy,
  Download,
  FolderOpen,
  RefreshCw,
  Search,
} from "lucide-react";
import { isDesktop } from "../../lib/api";
import { errorText } from "../../lib/utils";
import { Button } from "../../components/ui";
import { Modal } from "../../components/overlays";
import { diagnosticsApi, taskDiagnosticSummary } from "./diagnostics-api";
import {
  stageLabels,
  phaseLabel,
  statusLabels,
  type DiagnosticEvent,
  type DiagnosticFilter,
  type DiagnosticPage,
  type Overview,
  type Health,
} from "./types";

const tabs = ["概览", "性能", "事件", "异常"] as const;
const errorAction = (code: string | null, kind?: string | null) =>
  code === "PROCESS_CLEANUP"
    ? "核对进程回收、系统权限及此Trace最终状态，避免重复启动任务。"
    : kind === "NETWORK"
      ? "检查网络及媒体可访问性，再用同一Trace定位引擎等待阶段。"
      : kind === "DISK_FULL"
        ? "检查保存磁盘空间，选择可写目录后重试。"
        : kind === "ACCESS_RESTRICTED"
          ? "核对媒体访问权限；应用不会自动读取浏览器凭据。"
          : code?.startsWith("PERSISTENCE")
            ? "检查磁盘空间、数据库权限与占用；保留现有记录后重试。"
            : code === "DEPENDENCY_UNAVAILABLE" || code === "PROCESS_SPAWN"
              ? "前往设置检测本地引擎，核对可执行文件与权限。"
              : code === "OUTPUT_VALIDATION"
                ? "检查任务保存目录与真实输出文件，勿删除历史证据。"
                : "查看完整Trace定位失败阶段，检查媒体可访问性、网络或引擎版本后重试。";
const ms = (value: number | null | undefined) =>
  value == null
    ? "暂无数据"
    : value < 1000
      ? `${value.toFixed(2)} ms`
      : `${(value / 1000).toFixed(2)} s`;
export function DiagnosticsPage({
  initialTask = "",
  onTask,
}: {
  initialTask?: string;
  onTask: (id: string) => void;
}) {
  const [tab, setTab] = useState<(typeof tabs)[number]>(
    initialTask ? "性能" : "概览",
  );
  const [filter, setFilter] = useState<DiagnosticFilter>(
    initialTask ? { taskId: initialTask } : {},
  );
  const [draft, setDraft] = useState<DiagnosticFilter>(filter);
  const [page, setPage] = useState<DiagnosticPage>();
  const [overview, setOverview] = useState<Overview>();
  const [health, setHealth] = useState<Health>();
  const [trace, setTrace] = useState<string>("");
  const [traceOrder, setTraceOrder] = useState("recent");
  const [traceEvents, setTraceEvents] = useState<DiagnosticEvent[]>([]);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [exporting, setExporting] = useState(false);
  const [reading, setReading] = useState(isDesktop);
  const [confirm, setConfirm] = useState(false);
  const generation = useRef({ value: 0 });
  const update = useCallback(async () => {
    if (!isDesktop) return;
    const request = ++generation.current.value;
    setReading(true);
    try {
      const effective = {
        ...filter,
        ...(tab === "异常" ? { level: "error" as const } : {}),
        limit: 100,
      };
      const [next, summary, status, timeline] = await Promise.all([
        diagnosticsApi.query(effective),
        diagnosticsApi.overview(filter),
        diagnosticsApi.health(),
        trace
          ? diagnosticsApi.query({ traceId: trace, limit: 200 })
          : Promise.resolve(null),
      ]);
      if (request !== generation.current.value) return;
      setPage(next);
      if (initialTask && !trace)
        setTrace(
          (
            next.events.find(
              (e) =>
                e.source === "rust" &&
                e.fields.attempt_id &&
                e.session_id === status.sessionId,
            ) ??
            next.events.find((e) => e.source === "rust" && e.fields.attempt_id)
          )?.trace_id ?? "",
        );
      setOverview(summary);
      setHealth(status);
      setTraceEvents(timeline?.events ?? []);
      setError("");
      setReading(false);
    } catch (failure) {
      if (request === generation.current.value) {
        setError(errorText(failure));
        setReading(false);
      }
    }
  }, [filter, tab, trace, initialTask]);
  useEffect(() => {
    const requestState = generation.current;
    let disposed = false;
    let busy = false;
    const run = async () => {
      if (disposed || busy) return;
      busy = true;
      await update();
      busy = false;
    };
    void run();
    const timer = window.setInterval(() => {
      if (!filter.cursor) void run();
    }, 2000);
    return () => {
      disposed = true;
      ++requestState.value;
      window.clearInterval(timer);
    };
  }, [update, filter.cursor]);
  const action = async (fn: () => Promise<unknown>, message: string) => {
    try {
      await fn();
      setNotice(message);
    } catch (failure) {
      setError(errorText(failure));
    }
  };
  const selectTrace = (id: string) => {
    setTrace(id);
    setTab("性能");
  };
  const stateText = (event: DiagnosticEvent) =>
    event.status === "started" &&
    health &&
    event.session_id !== health.sessionId
      ? "旧会话未闭合，当前状态未确认"
      : (statusLabels[event.status] ?? "未知状态");
  const traces = [
    ...new Map((page?.events ?? []).map((e) => [e.trace_id, e])).values(),
  ].filter((e) => e.phase !== "session");
  const visibleTraces =
    traceOrder === "slow" ? (overview?.slowTraces ?? []) : traces;
  const starts = new Map(
    traceEvents
      .filter((e) => e.status === "started")
      .map((e) => [e.span_id, e]),
  );
  const startMs = (e: DiagnosticEvent) =>
    starts.get(e.span_id)?.elapsed_since_session_start_ms ??
    e.elapsed_since_session_start_ms;
  const stages = [
    ...new Map([...traceEvents].reverse().map((e) => [e.span_id, e])).values(),
  ].sort((a, b) => startMs(a) - startMs(b));
  const longest = Math.max(1, ...stages.map((e) => e.duration_ms ?? 0));
  const origin = Math.min(
    ...stages.filter((e) => e.source === "rust").map(startMs),
  );
  const end = Math.max(
    origin + 1,
    ...stages
      .filter((e) => e.source === "rust")
      .map((e) => e.elapsed_since_session_start_ms),
  );
  const totalError = (health?.dropped ?? 0) + (health?.criticalDropped ?? 0);
  return (
    <section className="diagnostics-page">
      <div className="page-heading">
        <div>
          <span className="eyebrow">本地运行诊断</span>
          <h1>
            诊断中心<span className="heading-dot">.</span>
          </h1>
          <p>用真实阶段记录定位等待与异常，诊断资料仅保存在本机。</p>
        </div>
        <Activity size={26} />
      </div>
      <div className="diagnostic-health" role="status">
        {reading && <span>正在读取本地诊断…</span>}
        <span
          className={`engine-dot ${health && !health.writeRestricted ? "ready" : ""}`}
        />
        <strong>
          {!isDesktop
            ? "浏览器预览 · 无后端采样"
            : health?.writeRestricted
              ? "日志写入受限 · 下载继续运行"
              : page?.total
                ? "记录正常"
                : "暂无诊断数据"}
        </strong>
        <span>
          {health?.mode === "development" ? "开发版" : health ? "正式版" : ""}{" "}
          {health?.level}
        </span>
      </div>
      {health?.build && (
        <p className="muted small-text">
          当前会话代码 {health.build.revision?.slice(0, 12) ?? "unknown"} ·
          dirty {String(health.build.dirty ?? "unknown")} · v
          {health.build.app_version}
        </p>
      )}
      {health?.directory && (
        <details className="diagnostic-directory">
          <summary>实际日志目录</summary>
          <code>{health.directory}</code>
          <p>目录位置只在此显示，不写入事件或诊断包。</p>
        </details>
      )}
      <div className="diagnostic-actions">
        <Button
          disabled={!isDesktop}
          onClick={() =>
            void action(
              () => diagnosticsApi.open(),
              "已请求资源管理器打开日志目录",
            )
          }
        >
          <FolderOpen size={14} />
          打开日志目录
        </Button>
        <Button
          disabled={!isDesktop}
          onClick={() =>
            void action(async () => {
              await diagnosticsApi.refresh();
              await update();
            }, "latest-report.md 已刷新")
          }
        >
          <RefreshCw size={14} />
          刷新报告
        </Button>
        <Button
          disabled={!isDesktop || exporting}
          onClick={() => setConfirm(true)}
        >
          <Download size={14} />
          导出诊断包
        </Button>
        <Button
          disabled={!isDesktop}
          onClick={() =>
            void action(() => diagnosticsApi.reveal(), "已请求定位最近导出文件")
          }
        >
          定位导出文件
        </Button>
      </div>
      {error && (
        <div className="notice danger" role="alert">
          {error}
        </div>
      )}
      {notice && (
        <p className="notice" role="status">
          {notice}
        </p>
      )}
      {page?.unreadableLines || page?.truncated || totalError > 0 ? (
        <div className="notice warning">
          损坏或不兼容记录 {page?.unreadableLines ?? 0} 条；
          {page?.truncated
            ? "仅显示最近20,000条可分析事件，历史可能不完整；"
            : ""}
          低优先级丢弃 {health?.dropped ?? 0}，关键队列入队受限{" "}
          {health?.criticalDropped ?? 0}。缺失记录不视为0耗时。
        </div>
      ) : null}
      <div className="diagnostic-tabs" role="tablist" aria-label="诊断视图">
        {tabs.map((t) => (
          <button
            key={t}
            role="tab"
            aria-selected={tab === t}
            onClick={() => {
              setTab(t);
              setFilter((f) => ({ ...f, cursor: 0 }));
            }}
          >
            {t}
          </button>
        ))}
      </div>
      <form
        className="diagnostic-filters"
        onSubmit={(e) => {
          e.preventDefault();
          setFilter({ ...draft, cursor: 0 });
        }}
      >
        <label>
          <span>搜索阶段 / 错误分类</span>
          <input
            aria-label="搜索诊断"
            maxLength={120}
            value={draft.search ?? ""}
            onChange={(e) => setDraft({ ...draft, search: e.target.value })}
            placeholder="如 二次安全复核 / PROCESS_EXIT"
          />
        </label>
        <label>
          <span>Trace ID</span>
          <input
            aria-label="筛选 Trace ID"
            value={draft.traceId ?? ""}
            onChange={(e) =>
              setDraft({ ...draft, traceId: e.target.value || undefined })
            }
          />
        </label>
        <label>
          <span>任务 ID</span>
          <input
            aria-label="筛选任务 ID"
            value={draft.taskId ?? ""}
            onChange={(e) =>
              setDraft({ ...draft, taskId: e.target.value || undefined })
            }
          />
        </label>
        <label>
          <span>等级</span>
          <select
            aria-label="日志等级"
            value={draft.level ?? ""}
            onChange={(e) =>
              setDraft({
                ...draft,
                level: (e.target.value ||
                  undefined) as DiagnosticFilter["level"],
              })
            }
          >
            <option value="">全部</option>
            <option value="info">信息</option>
            <option value="warn">警告</option>
            <option value="error">错误</option>
          </select>
        </label>
        <label>
          <span>来源</span>
          <select
            aria-label="日志来源"
            value={draft.source ?? ""}
            onChange={(e) =>
              setDraft({
                ...draft,
                source: (e.target.value ||
                  undefined) as DiagnosticFilter["source"],
              })
            }
          >
            <option value="">全部</option>
            <option value="rust">Rust</option>
            <option value="frontend">前端</option>
          </select>
        </label>
        <label>
          <span>开始时间</span>
          <input
            aria-label="开始时间"
            type="datetime-local"
            onChange={(e) =>
              setDraft({
                ...draft,
                fromUtc: e.target.value
                  ? new Date(e.target.value).toISOString()
                  : undefined,
              })
            }
          />
        </label>
        <label>
          <span>结束时间</span>
          <input
            aria-label="结束时间"
            type="datetime-local"
            onChange={(e) =>
              setDraft({
                ...draft,
                toUtc: e.target.value
                  ? new Date(e.target.value).toISOString()
                  : undefined,
              })
            }
          />
        </label>
        <Button type="submit" disabled={!isDesktop}>
          <Search size={14} />
          应用筛选
        </Button>
        <Button
          onClick={() => {
            setDraft({});
            setFilter({});
            setTrace("");
          }}
        >
          清除筛选
        </Button>
      </form>
      {(filter.taskId || initialTask) && (
        <Button
          disabled={!isDesktop}
          onClick={() =>
            void action(
              async () =>
                navigator.clipboard.writeText(
                  await taskDiagnosticSummary(filter.taskId ?? initialTask),
                ),
              "本任务安全诊断摘要已复制",
            )
          }
        >
          <Copy size={14} />
          复制本任务诊断摘要
        </Button>
      )}
      {tab === "概览" && (
        <>
          {overview?.cacheStats && (
            <p className="muted">
              当前范围可信预览：命中 {overview.cacheStats.hit} 次；缺失{" "}
              {overview.cacheStats.missing} 次；过期{" "}
              {overview.cacheStats.expired} 次；失效{" "}
              {overview.cacheStats.invalidated} 次；实际完整重新提取{" "}
              {overview.cacheStats.fullRechecks} 次。重复请求复用{" "}
              {overview.cacheStats.duplicate} 次，开发完整复核对照{" "}
              {overview.cacheStats.disabled} 次。
            </p>
          )}
          <div className="diagnostic-metrics">
            {[
              ["本会话解析", overview?.parses],
              ["本会话入队", overview?.downloads],
              ["本会话错误事件", overview?.errors],
            ].map(([label, value]) => (
              <div key={label}>
                <span>{label}</span>
                <strong>{value ?? "—"}</strong>
              </div>
            ))}
          </div>
          <div className="diagnostic-metrics">
            {["parse_ui_request", "enqueue_to_progress"].map((phase) => {
              const s = overview?.statistics.find(
                (s) =>
                  s.phase === phase &&
                  (!s.sessionId || s.sessionId === health?.sessionId),
              );
              return (
                <div key={phase}>
                  <span>
                    {stageLabels[phase as keyof typeof stageLabels]} · p50
                  </span>
                  <strong>{ms(s?.p50Ms)}</strong>
                  <small>
                    {s
                      ? `${s.samples} 个成功样本 · p95 ${ms(s.p95Ms)}`
                      : "仅展示实际采集样本"}
                  </small>
                </div>
              );
            })}
          </div>
          <h2>慢步骤 Top 5</h2>
          <Statistics
            values={overview?.statistics.slice(0, 5) ?? []}
            select={selectTrace}
          />
          <h2>最近异常</h2>
          {overview?.recentErrors.length ? (
            overview.recentErrors.map((e) => (
              <button
                className="diagnostic-error"
                key={e.event_id}
                onClick={() => selectTrace(e.trace_id)}
              >
                <span>
                  {phaseLabel(e.phase)} · {e.fields.error_code ?? "阶段失败"}
                </span>
                <code>{e.trace_id}</code>
                <ArrowRight size={14} />
              </button>
            ))
          ) : (
            <p className="diagnostic-empty">当前范围没有异常记录。</p>
          )}
        </>
      )}
      {tab === "性能" && (
        <>
          <p className="muted">
            用户选择格式的停留不计入应用耗时。排队等待单列；引擎提取含网络，无法据此细分网络耗时。首次进度不等于首字节。
          </p>
          <Statistics
            values={overview?.statistics ?? []}
            select={selectTrace}
          />
          <h2>选择 Trace 查看阶段</h2>
          <label className="diagnostic-trace-sort">
            Trace排序{" "}
            <select
              aria-label="Trace排序"
              value={traceOrder}
              onChange={(e) => setTraceOrder(e.target.value)}
            >
              <option value="recent">最近事件页</option>
              <option value="slow">当前范围内最慢100个Trace</option>
            </select>
          </label>
          <p className="muted">
            慢Trace按已观测总阶段耗时排序，最多分析最近20,000条事件；嵌套阶段不累加，未结束阶段不填估计值。
          </p>
          <div className="diagnostic-trace-list">
            {visibleTraces.map((e) => (
              <button
                key={e.trace_id}
                className={trace === e.trace_id ? "selected" : ""}
                onClick={() => selectTrace(e.trace_id)}
              >
                <code>{e.trace_id}</code>
                <span>{phaseLabel(e.phase)}</span>
                {traceOrder === "slow" && <span>{ms(e.duration_ms)}</span>}
              </button>
            ))}
          </div>
          {!trace ? (
            <p className="diagnostic-empty">
              请选择一个真实 Trace；无样本时不生成时间轴。
            </p>
          ) : (
            <>
              <p className="muted small-text">
                尝试 ID：
                {traceEvents.find((e) => e.fields.attempt_id)?.fields
                  .attempt_id ?? "预览解析或无有效记录"}
                。未结束、跨会话和超出保留窗口的阶段不补零。
              </p>
              <div className="diagnostic-trace-heading">
                <h2>阶段时间轴</h2>
                <code>{trace}</code>
                <Button
                  onClick={() =>
                    void action(
                      () => navigator.clipboard.writeText(trace),
                      "Trace ID 已复制",
                    )
                  }
                >
                  <Copy size={13} />
                  复制 Trace ID
                </Button>
              </div>
              {stages.length ? (
                stages.map((e) => (
                  <details
                    className={`diagnostic-span ${e.status === "failed" ? "failed" : ""}`}
                    key={e.span_id}
                  >
                    <summary>
                      <span>{phaseLabel(e.phase)}</span>
                      <i>
                        <b
                          style={{
                            width:
                              e.duration_ms == null
                                ? "0%"
                                : `${Math.min(100, (e.duration_ms / (e.source === "frontend" ? longest : end - origin)) * 100)}%`,
                            marginLeft:
                              e.source === "frontend"
                                ? 0
                                : `${Math.max(0, ((startMs(e) - origin) / (end - origin)) * 100)}%`,
                          }}
                        />
                      </i>
                      <strong>
                        {e.status === "started" &&
                        e.session_id !== health?.sessionId
                          ? "上一会话中断 / 记录缺失"
                          : stateText(e)}{" "}
                        ·{" "}
                        {e.event === "fault_observed"
                          ? "瞬时异常，不计跨度耗时"
                          : e.duration_ms == null
                            ? "未观测到结束耗时"
                            : ms(e.duration_ms)}
                      </strong>
                    </summary>
                    <dl>
                      <dt>开始 UTC</dt>
                      <dd>
                        {starts.get(e.span_id)?.timestamp_utc ??
                          (e.source === "frontend"
                            ? "前端测量不跨时钟对齐"
                            : "瞬时事件或缺少开始记录")}
                      </dd>
                      <dt>结束 / 发生 UTC</dt>
                      <dd>{e.timestamp_utc}</dd>
                      <dt>会话单调时点</dt>
                      <dd>{ms(e.elapsed_since_session_start_ms)}</dd>
                      <dt>结果 / 错误分类</dt>
                      <dd>
                        {stateText(e)} / {e.fields.error_code ?? "无"} /{" "}
                        {e.fields.error_kind ?? "未分类"}
                      </dd>
                      {e.level === "error" && (
                        <>
                          <dt>排查建议</dt>
                          <dd>
                            {errorAction(
                              e.fields.error_code,
                              e.fields.error_kind,
                            )}
                          </dd>
                        </>
                      )}
                      <dt>尝试 ID</dt>
                      <dd>{e.fields.attempt_id ?? "尚未创建任务"}</dd>
                      {e.fields.cache_outcome && (
                        <>
                          <dt>缓存决策</dt>
                          <dd>
                            {e.fields.cache_outcome}
                            （仅表示准入决策，下载仍以真实终态为准）
                          </dd>
                        </>
                      )}
                      {e.fields.output_stream && (
                        <>
                          <dt>首次输出管线</dt>
                          <dd>{e.fields.output_stream}（不代表网络首字节）</dd>
                        </>
                      )}
                    </dl>
                    {e.task_id && (
                      <Button onClick={() => onTask(e.task_id!)}>
                        定位任务
                      </Button>
                    )}
                    {e.fields.related_trace_id && (
                      <Button
                        onClick={() => selectTrace(e.fields.related_trace_id!)}
                      >
                        查看关联解析 / 上次尝试
                      </Button>
                    )}
                  </details>
                ))
              ) : (
                <p className="diagnostic-empty">
                  此 Trace 没有保留记录，可能超出轮转或查询窗口。
                </p>
              )}
            </>
          )}
        </>
      )}
      {(tab === "事件" || tab === "异常") && (
        <>
          <p className="muted">
            {page?.total ?? 0} 条匹配记录 · 每页最多100条。
            {filter.cursor
              ? "查看较早记录时停止自动翻动；手动翻页继续。"
              : "每2秒读取真实事件。"}
          </p>
          <div className="diagnostic-event-table">
            <table>
              <thead>
                <tr>
                  <th>时间 / 等级</th>
                  <th>阶段 / 事件</th>
                  <th>Trace / 任务</th>
                  <th>状态 / 耗时</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {page?.events.map((e) => (
                  <tr key={e.event_id}>
                    <td>
                      <time>
                        {new Date(e.timestamp_utc).toLocaleTimeString()}
                      </time>
                      <small className={`level-${e.level}`}>
                        {e.level} · {e.source}
                      </small>
                    </td>
                    <td>
                      {phaseLabel(e.phase)}
                      <small>
                        {e.event} {e.fields.error_code} {e.fields.error_kind}
                      </small>
                      {e.level === "error" && (
                        <small>
                          {errorAction(
                            e.fields.error_code,
                            e.fields.error_kind,
                          )}
                        </small>
                      )}
                    </td>
                    <td>
                      <button
                        className="text-button"
                        onClick={() => selectTrace(e.trace_id)}
                      >
                        <code>{e.trace_id}</code>
                      </button>
                      {e.task_id && (
                        <button
                          className="text-button"
                          onClick={() => onTask(e.task_id!)}
                        >
                          <code>{e.task_id}</code>
                        </button>
                      )}
                    </td>
                    <td>
                      {stateText(e)}
                      <small>
                        {e.duration_ms == null ? "—" : ms(e.duration_ms)}
                      </small>
                    </td>
                    <td>
                      <Button
                        aria-label="复制事件"
                        onClick={() =>
                          void action(
                            () =>
                              navigator.clipboard.writeText(
                                JSON.stringify(e, null, 2),
                              ),
                            "安全事件已复制",
                          )
                        }
                      >
                        <Copy size={13} />
                      </Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {!page?.events.length && (
            <p className="diagnostic-empty">
              当前筛选没有事件。请先解析或下载，再检查筛选范围。
            </p>
          )}
        </>
      )}
      <div className="diagnostic-pagination">
        <Button
          disabled={!filter.cursor}
          onClick={() =>
            setFilter((f) => ({
              ...f,
              cursor: Math.max(0, (f.cursor ?? 0) - 100),
            }))
          }
        >
          上一页
        </Button>
        <span>第 {Math.floor((filter.cursor ?? 0) / 100) + 1} 页</span>
        <Button
          disabled={page?.nextCursor == null}
          onClick={() =>
            setFilter((f) => ({ ...f, cursor: page?.nextCursor ?? 0 }))
          }
        >
          下一页
        </Button>
      </div>
      {confirm && (
        <Modal title="导出本地诊断包" onClose={() => setConfirm(false)}>
          <p className="modal-description">
            将导出当前筛选范围内的
            summary.md、events.jsonl、errors.jsonl、environment.json、privacy-manifest.json
            和 README.md。
          </p>
          <p className="notice">
            仅含净化后的阶段、耗时、ID、错误分类和安全版本。无媒体链接、标题、Cookie、路径、原始
            stderr 或数据库。请检查后自行决定分享；不会自动上传。
          </p>
          <Button
            className="primary"
            disabled={exporting}
            onClick={() => {
              setExporting(true);
              void diagnosticsApi
                .export(filter)
                .then((result) => {
                  if (result) setNotice(result);
                  setConfirm(false);
                })
                .catch((failure) => setError(errorText(failure)))
                .finally(() => setExporting(false));
            }}
          >
            {exporting ? "请选择保存位置…" : "选择文件名并导出"}
          </Button>
        </Modal>
      )}
    </section>
  );
}
function Statistics({
  values,
  select,
}: {
  values: NonNullable<Overview["statistics"]>;
  select: (id: string) => void;
}) {
  return values.length ? (
    <div className="diagnostic-event-table">
      <table>
        <thead>
          <tr>
            <th>阶段</th>
            <th>成功样本 n</th>
            <th>样本说明</th>
            <th>p50</th>
            <th>p95</th>
            <th>最大</th>
            <th>最慢实例</th>
          </tr>
        </thead>
        <tbody>
          {values.map((s) => (
            <tr
              key={`${s.phase}-${s.sessionId ?? ""}-${s.environmentId ?? "unknown"}-${s.cacheOutcome ?? "none"}`}
            >
              <td>
                {s.label}
                <small className="diagnostic-environment">
                  策略 {s.cacheOutcome ?? "非预览入队或未知"} · 会话{" "}
                  {s.sessionId?.slice(0, 8) ?? "unknown"} · revision{" "}
                  {s.codeRevision?.slice(0, 8) ?? "unknown"}
                  <br />
                  {Object.entries(s.engineVersions ?? {})
                    .map(([k, v]) => `${k} ${v}`)
                    .join(" · ") || "引擎环境未知"}
                </small>
              </td>
              <td>{s.samples}</td>
              <td>
                {s.samples < 20 ? "样本不足；仅经验分位数" : "当前窗口样本"}
              </td>
              <td>{ms(s.p50Ms)}</td>
              <td>{ms(s.p95Ms)}</td>
              <td>{ms(s.maxMs)}</td>
              <td>
                <button
                  className="text-button"
                  onClick={() => select(s.slowestTraceId)}
                  title={s.slowestTraceId}
                >
                  <code>{s.slowestTraceId.slice(0, 8)}…</code>
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  ) : (
    <p className="diagnostic-empty">
      暂无可计算的成功阶段样本。失败或未观测到的阶段不会补成0。
    </p>
  );
}
