use super::*;
use serde::Serialize;
type StatisticKey = (Stage, String, Option<String>, Option<CacheOutcome>);
pub(super) fn engine_versions(versions: &std::collections::BTreeMap<String, String>) -> String {
    let mut text="\n## 当前检测版本与配置概要\n\n外部引擎；串行队列；无凭据采集。版本只保留数值核心，不包含路径或原始版本输出。\n\n".to_owned();
    if versions.is_empty() {
        text.push_str("尚未检测到可安全记录的版本。\n");
    }
    for (name, version) in versions {
        text.push_str(&format!("- {name}：{version}\n"));
    }
    text.push_str(
        "\n这是最近检测版本；跨会话历史样本的当时版本未逐事件保存，不能据此假设历史条件相同。\n",
    );
    text
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Statistic {
    pub session_id: String,
    pub cache_outcome: Option<CacheOutcome>,
    pub environment_id: Option<String>,
    pub code_revision: Option<String>,
    pub engine_versions: std::collections::BTreeMap<String, String>,
    pub phase: Stage,
    pub label: String,
    pub samples: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub slowest_trace_id: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub hit: usize,
    pub missing: usize,
    pub expired: usize,
    pub invalidated: usize,
    pub disabled: usize,
    pub duplicate: usize,
    pub full_rechecks: usize,
}
impl CacheStats {
    fn from_events(events: &[Event]) -> Self {
        let mut counts = Self {
            hit: 0,
            missing: 0,
            expired: 0,
            invalidated: 0,
            disabled: 0,
            duplicate: 0,
            full_rechecks: 0,
        };
        for e in events.iter().filter(|e| e.source == Source::Rust) {
            if e.phase == Stage::EnqueueMetadataRecheck && e.status == Status::Started {
                counts.full_rechecks += 1;
            }
            match e.fields.cache_outcome {
                Some(CacheOutcome::Hit) => counts.hit += 1,
                Some(CacheOutcome::Missing) => counts.missing += 1,
                Some(CacheOutcome::Expired) => counts.expired += 1,
                Some(CacheOutcome::Disabled) => counts.disabled += 1,
                Some(CacheOutcome::Duplicate) => counts.duplicate += 1,
                Some(_) => counts.invalidated += 1,
                None => {}
            }
        }
        counts
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub cache_stats: CacheStats,
    pub slow_traces: Vec<Event>,
    pub session_id: String,
    pub parses: usize,
    pub downloads: usize,
    pub errors: usize,
    pub total: usize,
    pub statistics: Vec<Statistic>,
    pub recent_errors: Vec<Event>,
    pub unreadable_lines: usize,
    pub truncated: bool,
    pub from_utc: Option<String>,
    pub to_utc: Option<String>,
}
pub fn overview(events: Vec<Event>, session: &str, unreadable: usize, truncated: bool) -> Overview {
    overview_with_context(events.clone(), session, unreadable, truncated, &events)
}
pub fn overview_with_context(
    events: Vec<Event>,
    session: &str,
    unreadable: usize,
    truncated: bool,
    context: &[Event],
) -> Overview {
    let environments: HashMap<_, _> = context
        .iter()
        .filter_map(|e| {
            Some((
                e.fields.environment_id.as_ref()?,
                e.fields.environment.as_ref()?,
            ))
        })
        .collect();
    let mut traces = HashMap::<&str, &Event>::new();
    for event in &events {
        if event.source == Source::Rust
            && event.duration_ms.is_some()
            && matches!(
                event.phase,
                Stage::ParseTotal | Stage::EnqueueToProgress | Stage::DownloadTotal
            )
        {
            traces
                .entry(&event.trace_id)
                .and_modify(|old| {
                    if event.duration_ms > old.duration_ms {
                        *old = event;
                    }
                })
                .or_insert(event);
        }
    }
    let mut slow_traces: Vec<_> = traces.into_values().cloned().collect();
    slow_traces.sort_by(|a, b| b.duration_ms.unwrap().total_cmp(&a.duration_ms.unwrap()));
    slow_traces.truncate(100);
    let policies: HashMap<_, _> = context
        .iter()
        .filter_map(|e| Some((e.trace_id.as_str(), e.fields.cache_outcome?)))
        .collect();
    let mut groups: HashMap<StatisticKey, Vec<&Event>> = HashMap::new();
    for e in &events {
        if e.status == Status::Ok && e.duration_ms.is_some() && e.phase != Stage::Session {
            groups
                .entry((
                    e.phase,
                    e.session_id.clone(),
                    e.fields.environment_id.clone(),
                    policies.get(e.trace_id.as_str()).copied(),
                ))
                .or_default()
                .push(e);
        }
    }
    let mut statistics: Vec<_> = groups
        .into_iter()
        .map(
            |((phase, session_id, environment_id, cache_outcome), mut values)| {
                values.sort_by(|a, b| a.duration_ms.unwrap().total_cmp(&b.duration_ms.unwrap()));
                let n = values.len();
                let environment = environment_id.as_ref().and_then(|id| environments.get(id));
                Statistic {
                    session_id,
                    cache_outcome,
                    code_revision: environment.and_then(|e| e.build.revision.clone()),
                    engine_versions: environment.map(|e| e.engines.clone()).unwrap_or_default(),
                    environment_id,
                    phase,
                    label: phase.label().into(),
                    samples: n,
                    p50_ms: values[(n as f64 * 0.5).ceil() as usize - 1]
                        .duration_ms
                        .unwrap(),
                    p95_ms: values[(n as f64 * 0.95).ceil() as usize - 1]
                        .duration_ms
                        .unwrap(),
                    max_ms: values[n - 1].duration_ms.unwrap(),
                    slowest_trace_id: values[n - 1].trace_id.clone(),
                }
            },
        )
        .collect();
    statistics.sort_by(|a, b| b.p95_ms.total_cmp(&a.p95_ms).then(a.label.cmp(&b.label)));
    let unique = |phase| {
        events
            .iter()
            .filter(|e| e.session_id == session && e.phase == phase && e.status == Status::Started)
            .map(|e| &e.trace_id)
            .collect::<HashSet<_>>()
            .len()
    };
    Overview {
        cache_stats: CacheStats::from_events(&events),
        slow_traces,
        session_id: session.into(),
        parses: unique(Stage::ParseTotal),
        downloads: events
            .iter()
            .filter(|e| {
                e.session_id == session
                    && e.phase == Stage::EnqueuePersist
                    && e.status == Status::Ok
            })
            .filter_map(|e| e.task_id.as_ref())
            .collect::<HashSet<_>>()
            .len(),
        errors: events
            .iter()
            .filter(|e| e.session_id == session && e.level == Level::Error)
            .count(),
        total: events.len(),
        statistics,
        recent_errors: events
            .iter()
            .rev()
            .filter(|e| e.level == Level::Error)
            .take(10)
            .cloned()
            .collect(),
        unreadable_lines: unreadable,
        truncated,
        from_utc: events
            .iter()
            .min_by_key(|e| chrono::DateTime::parse_from_rfc3339(&e.timestamp_utc).ok())
            .map(|e| e.timestamp_utc.clone()),
        to_utc: events
            .iter()
            .max_by_key(|e| chrono::DateTime::parse_from_rfc3339(&e.timestamp_utc).ok())
            .map(|e| e.timestamp_utc.clone()),
    }
}
pub fn markdown(
    events: &[Event],
    mode: &str,
    session: &str,
    unreadable: usize,
    truncated: bool,
    dropped: u64,
    critical_dropped: u64,
) -> String {
    let summary = overview(events.to_vec(), session, unreadable, truncated);
    let identity = events
        .iter()
        .rev()
        .find(|e| e.session_id == session && e.fields.environment.is_some())
        .and_then(|e| e.fields.environment.as_ref())
        .map(|e| {
            format!(
                "{} / dirty={:?}",
                e.build.revision.as_deref().unwrap_or("unknown"),
                e.build.dirty
            )
        })
        .unwrap_or_else(|| "unknown（当前筛选没有会话元数据）".into());
    let generated = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut out = format!("# FluxDL 本地诊断报告\n\n生成时间 UTC：{generated}。代码revision：{identity}。\n\n应用版本：{}；运行模式：{}；平台：{} / {}；schema：1。\n\n会话：`{}`\n\n数据区间：{} 至 {}；有效事件 {}；损坏/不兼容行 {}；读取窗口截断 {}；低优先级丢弃 {}；关键队列入队受限 {}。\n\n本会话解析 {} 次，入队 {} 次，错误事件 {} 条。统计按会话/环境分别计算，旧记录版本为unknown，不跨环境平均；仅使用成功结束且有单调耗时的真实样本，nearest-rank p50/p95。失败、取消、未观测与不适用不混入成功分位数。\n\n| 阶段 / 会话 / 环境 | n | p50 ms | p95 ms | max ms | 最慢 Trace |\n| --- | ---: | ---: | ---: | ---: | --- |\n",env!("CARGO_PKG_VERSION"),mode,std::env::consts::OS,std::env::consts::ARCH, session,summary.from_utc.as_deref().unwrap_or("无"),summary.to_utc.as_deref().unwrap_or("无"),summary.total,unreadable,truncated,dropped,critical_dropped,summary.parses,summary.downloads,summary.errors);
    for s in summary.statistics.iter().take(24) {
        out.push_str(&format!(
            "| {} | {} | {:.3} | {:.3} | {:.3} | `{}` |\n",
            format_args!(
                "{} / {} / {} / {:?}",
                s.label,
                &s.session_id[..8],
                s.environment_id
                    .as_deref()
                    .map(|s| &s[..8])
                    .unwrap_or("unknown"),
                s.cache_outcome
            ),
            s.samples,
            s.p50_ms,
            s.p95_ms,
            s.max_ms,
            s.slowest_trace_id
        ));
    }
    let cache = &summary.cache_stats;
    out.push_str(&format!("\n当前窗口可信预览决策：命中 {}；缺失 {}；过期 {}；配置/来源/模式/格式失效 {}；开发完整复核策略 {}；重复请求复用 {}。实际启动完整入队提取 {} 次。各策略分开统计，不将未执行的提取填为0ms。\n",cache.hit,cache.missing,cache.expired,cache.invalidated,cache.disabled,cache.duplicate,cache.full_rechecks));
    if summary.statistics.is_empty() {
        out.push_str("\n样本不足：尚无解析或下载阶段的成功结束样本。\n");
    }
    if summary.statistics.iter().any(|s| s.samples < 20) {
        out.push_str(
            "\n样本不足：少于20条的阶段仅显示本窗口经验分位数，单次采样的p95不代表总体性能。\n",
        );
    }
    out.push_str("\n## 慢Trace Top 5（总阶段不累加）\n\n");
    for event in summary.slow_traces.iter().take(5) {
        out.push_str(&format!(
            "- `{}`：{:?} / {}，{:?}，{:.3} ms。\n",
            event.trace_id,
            event.phase,
            event.phase.label(),
            event.status,
            event.duration_ms.unwrap_or_default()
        ));
    }
    out.push_str("\n## 已知源码入口\n\n- src/lib/api.ts：前端独立测量与Trace关联。\n- src-tauri/src/manager.rs：入队复核、排队、首次有效进度及终态确认。\n- src-tauri/src/engine/mod.rs：元数据提取与安全错误分类。\n- src-tauri/src/process/capture.rs、src-tauri/src/process/download.rs：实际spawn与进程生命周期。\n- src-tauri/src/persistence/writer.rs：DB队列与真实事务计时。\n");
    out.push_str("\n## 最近异常与未完成阶段\n\n");
    for e in events
        .iter()
        .rev()
        .filter(|e| {
            matches!(
                e.status,
                Status::Failed | Status::Cancelled | Status::Unobserved | Status::Interrupted
            )
        })
        .take(20)
    {
        out.push_str(&format!(
            "- `{}` / `{}`：{}，{:?}，错误分类 {:?}/{:?}。\n",
            e.trace_id,
            e.task_id.as_deref().unwrap_or("尚无任务"),
            e.phase.label(),
            e.status,
            e.fields.error_code,
            e.fields.error_kind
        ));
    }
    let finished: HashSet<_> = events
        .iter()
        .filter(|e| e.status != Status::Started)
        .map(|e| (&e.trace_id, &e.span_id))
        .collect();
    for e in events
        .iter()
        .rev()
        .filter(|e| e.status == Status::Started && !finished.contains(&(&e.trace_id, &e.span_id)))
        .take(20)
    {
        out.push_str(&format!(
            "- `{}`：{}，{}；没有结束证据，不补造耗时。\n",
            e.trace_id,
            e.phase.label(),
            if e.session_id == session {
                "进行中或记录缺失"
            } else {
                "上一会话中断或记录缺失"
            }
        ));
    }
    out.push_str("\n## 数据边界与本地排查\n\n解析与入队分开 Trace，通过 related_trace_id 关联；用户选择格式的停留不计入软件处理。queue_wait 是串行自然等待；metadata_extract/recheck 包含子进程、网络与解析，不能单凭该值细分外部网络；first_progress 是首个有效协议进度，非首字节。前端 performance.now() 与 Rust Instant 独立，不跨时钟相减。\n\n日志仅含枚举、UUID、错误分类和有限数值，不含URL、标题、Cookie、命令行、原始stderr、私有路径或环境变量。报告不采集任务数据库。无自动上传。\n\n本地 Codex：先读本报告，再以 Trace ID 在 sessions/*.jsonl 检索，核对 manager.rs、engine/、process/ 和前端 API。网络结果应在相同条件重复测量；不要提交本地日志至 GitHub。远程 Codex 不会自动获得本机日志。\n");
    out
}
