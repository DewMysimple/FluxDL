use super::*;
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
};

fn regular(path: &Path, directory: bool) -> std::io::Result<()> {
    let m = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(std::io::Error::other("reparse point"));
        }
    }
    if m.file_type().is_symlink() || (directory && !m.is_dir()) || (!directory && !m.is_file()) {
        return Err(std::io::Error::other("unexpected path type"));
    }
    Ok(())
}
pub(super) fn check_directory(path: &Path) -> std::io::Result<PathBuf> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(std::io::Error::other("absolute path required"));
    }
    for ancestor in path.ancestors() {
        regular(ancestor, true)?;
    }
    fs::canonicalize(path)
}
fn prepare(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        check_directory(path)?;
    } else {
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("parent required"))?;
        if !parent.exists() {
            prepare(parent)?;
        }
        check_directory(parent)?;
        match fs::create_dir(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    check_directory(path)?;
    Ok(())
}
fn known(name: &str) -> bool {
    let Some(stem) = name
        .strip_prefix("session-")
        .and_then(|s| s.strip_suffix(".jsonl"))
    else {
        return false;
    };
    let Some((stamp, rest)) = stem.split_once('-') else {
        return false;
    };
    stamp.len() == 13
        && stamp.bytes().all(|b| b.is_ascii_digit())
        && rest.is_ascii()
        && rest.len() == 43
        && valid_id(&rest[..36])
        && rest.as_bytes()[36] == b'-'
        && rest[37..].bytes().all(|b| b.is_ascii_digit())
}
fn files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    check_directory(root)?;
    let sessions = root.join("sessions");
    check_directory(&sessions)?;
    let mut files = Vec::new();
    for entry in fs::read_dir(sessions)? {
        let entry = entry?;
        let path = entry.path();
        if known(&entry.file_name().to_string_lossy()) && regular(&path, false).is_ok() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}
pub(super) fn stamp(root: &Path) -> std::io::Result<super::LogStamp> {
    files(root)?
        .into_iter()
        .map(|path| {
            let meta = fs::metadata(&path)?;
            Ok((path, meta.len(), meta.modified()?))
        })
        .collect()
}
pub(super) fn retain(root: &Path, current: Option<&Path>, policy: &Policy) -> std::io::Result<()> {
    let mut all = files(root)?;
    all.reverse();
    let allowance =
        policy.max_files.max(1) - usize::from(current.is_some_and(|p| all.iter().any(|f| f == p)));
    let mut kept = 0;
    for path in &all {
        if current == Some(path) {
            continue;
        }
        let expired = fs::metadata(path)?
            .modified()?
            .elapsed()
            .is_ok_and(|age| age > policy.max_age);
        if expired || kept >= allowance {
            check_directory(&root.join("sessions"))?;
            regular(path, false)?;
            fs::remove_file(path)?;
        } else {
            kept += 1;
        }
    }
    Ok(())
}
pub(super) fn read(root: &Path) -> ReadResult {
    let mut result = ReadResult {
        events: vec![],
        unreadable: 0,
        truncated: false,
    };
    let paths = match files(root) {
        Ok(p) => p,
        Err(_) => {
            result.unreadable += 1;
            return result;
        }
    };
    let mut queue = VecDeque::new();
    let mut ids = HashSet::new();
    // Never let externally dropped files turn a UI query into an unbounded read.
    if paths.len() > 5 {
        result.truncated = true;
    }
    for path in paths
        .into_iter()
        .rev()
        .take(5)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let Ok(meta) = fs::metadata(&path) else {
            result.unreadable += 1;
            continue;
        };
        if meta.len() > 10 * 1024 * 1024 + 16_384 {
            result.unreadable += 1;
            continue;
        }
        let Ok(file) = File::open(&path) else {
            result.unreadable += 1;
            continue;
        };
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) if line.len() <= 16_384 && line.ends_with(b"\n") => {
                    match serde_json::from_slice::<Event>(&line) {
                        Ok(event) if event.valid() => {
                            if ids.insert(event.event_id.clone()) {
                                if queue.len() == READ_LIMIT {
                                    if let Some(old) = queue.pop_front() {
                                        let old: Event = old;
                                        ids.remove(&old.event_id);
                                    }
                                    result.truncated = true;
                                }
                                queue.push_back(event);
                            }
                        }
                        _ => result.unreadable += 1,
                    }
                }
                Ok(_) => result.unreadable += 1,
                Err(_) => {
                    result.unreadable += 1;
                    break;
                }
            }
        }
    }
    result.events = queue.into_iter().collect();
    sort_events(&mut result.events);
    result
}
struct Sink {
    written: HashSet<String>,
    written_order: VecDeque<String>,
    root: PathBuf,
    policy: Policy,
    session: String,
    stamp: i64,
    sequence: usize,
    current: Option<(PathBuf, File, u64)>,
}
impl Sink {
    fn new(root: PathBuf, session: String, policy: Policy) -> std::io::Result<Self> {
        prepare(&root)?;
        prepare(&root.join("sessions"))?;
        retain(&root, None, &policy)?;
        Ok(Self {
            written: HashSet::new(),
            written_order: VecDeque::new(),
            root,
            session,
            policy,
            stamp: chrono::Utc::now().timestamp_millis(),
            sequence: 0,
            current: None,
        })
    }
    fn write(&mut self, event: &Event) -> std::io::Result<()> {
        if self.written.contains(&event.event_id) {
            return Ok(());
        }
        let mut bytes = serde_json::to_vec(event)?;
        bytes.push(b'\n');
        if self
            .current
            .as_ref()
            .is_none_or(|(_, _, size)| *size + bytes.len() as u64 > self.policy.max_bytes)
        {
            self.current = None;
            check_directory(&self.root.join("sessions"))?;
            let path = self.root.join("sessions").join(format!(
                "session-{}-{}-{:06}.jsonl",
                self.stamp, self.session, self.sequence
            ));
            self.sequence += 1;
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            self.current = Some((path.clone(), file, 0));
            retain(&self.root, Some(&path), &self.policy)?;
        }
        let (path, file, size) = self.current.as_mut().expect("sink opened");
        check_directory(&self.root.join("sessions"))?;
        regular(path, false)?;
        write_record(file, &bytes)?;
        *size += bytes.len() as u64;
        self.written.insert(event.event_id.clone());
        self.written_order.push_back(event.event_id.clone());
        if self.written_order.len() > MEMORY_LIMIT * 2 {
            if let Some(old) = self.written_order.pop_front() {
                self.written.remove(&old);
            }
        }
        Ok(())
    }
    fn report(&mut self, mode: &str, shared: &Shared) -> std::io::Result<()> {
        if let Some((_, file, _)) = &mut self.current {
            file.flush()?;
        }
        let read = read(&self.root);
        let mut summary = report::markdown(
            &read.events,
            mode,
            &self.session,
            read.unreadable,
            read.truncated,
            shared.dropped.load(Ordering::Relaxed),
            shared.critical_dropped.load(Ordering::Relaxed),
        );
        summary.push_str(&report::engine_versions(
            &shared.versions.lock().unwrap_or_else(|e| e.into_inner()),
        ));
        summary.push_str("\n## Codex建议读取的本地文件\n\n");
        for path in files(&self.root)?.iter().rev().take(5) {
            if let Some(name) = path.file_name().and_then(|v| v.to_str()) {
                summary.push_str(&format!("- sessions/{name}\n"));
            }
        }
        check_directory(&self.root)?;
        let dest = self.root.join("latest-report.md");
        if fs::symlink_metadata(&dest).is_ok() {
            regular(&dest, false)?;
        }
        let temporary = self
            .root
            .join(format!("latest-report-{}.tmp", uuid::Uuid::new_v4()));
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let written = output
            .write_all(summary.as_bytes())
            .and_then(|_| output.flush());
        drop(output);
        if let Err(error) = written {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        if let Err(e) = fs::rename(&temporary, &dest) {
            let _ = fs::remove_file(&temporary);
            return Err(e);
        }
        retain(
            &self.root,
            self.current.as_ref().map(|(p, _, _)| p.as_path()),
            &self.policy,
        )?;
        Ok(())
    }
}
pub(super) fn write_record(writer: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
    writer.write_all(bytes)
}
#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    root: Option<PathBuf>,
    mode: &'static str,
    session: String,
    policy: Policy,
    shared: Arc<Shared>,
    low: mpsc::Receiver<Event>,
    high: mpsc::Receiver<Event>,
    controls: mpsc::Receiver<mpsc::Sender<Result<(), String>>>,
) {
    let mut sink = root
        .as_ref()
        .and_then(|root| Sink::new(root.clone(), session.clone(), policy.clone()).ok());
    if root.is_some() && sink.is_none() {
        shared.warning.store(true, Ordering::Relaxed);
    }
    let mut dirty = true;
    let mut last_generation = 0;
    let mut last = Instant::now();
    let mut retry = Instant::now();
    loop {
        let generation = shared.generation.load(Ordering::Relaxed);
        if generation != last_generation {
            dirty = true;
            last_generation = generation;
        }
        #[cfg(test)]
        if shared.paused.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        let mut pending = Vec::new();
        for _ in 0..256 {
            if let Ok(e) = high.try_recv() {
                pending.push(e);
            } else {
                break;
            }
        }
        for _ in 0..1024 {
            if let Ok(e) = low.try_recv() {
                pending.push(e);
            } else {
                break;
            }
        }
        if sink.is_none() && root.is_some() && retry.elapsed() >= Duration::from_secs(2) {
            sink = Sink::new(root.clone().unwrap(), session.clone(), policy.clone()).ok();
            retry = Instant::now();
        }
        // Reserved error delivery also has a bounded in-memory fallback. The
        // pipe/queue producer never waits for the writer, even under pressure.
        if let Some(sink) = &sink {
            pending.extend(
                shared
                    .events
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .iter()
                    .filter(|e| {
                        (e.level == Level::Error || e.phase == Stage::DownloadTerminal)
                            && !sink.written.contains(&e.event_id)
                    })
                    .cloned(),
            );
        }
        for event in pending {
            dirty = true;
            if let Some(sink) = &mut sink {
                if sink.write(&event).is_err() {
                    shared.warning.store(true, Ordering::Relaxed);
                }
            } else if root.is_some() {
                shared.warning.store(true, Ordering::Relaxed);
            }
        }
        let control = controls.try_recv();
        let exiting = matches!(control, Err(mpsc::TryRecvError::Disconnected));
        if (dirty && last.elapsed() >= Duration::from_secs(2)) || control.is_ok() || exiting {
            let result = if let Some(sink) = &mut sink {
                sink.report(mode, &shared)
                    .map_err(|_| "日志写入受限；可查看内存事件，下载继续运行。".to_owned())
            } else if root.is_none() && mode == "test-disabled" {
                Ok(())
            } else {
                Err("日志目录不可写；下载继续运行。".into())
            };
            if result.is_err() {
                shared.warning.store(true, Ordering::Relaxed);
            }
            if let Ok(tx) = control {
                let _ = tx.send(result);
            }
            dirty = false;
            last = Instant::now();
        }
        if exiting {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
pub(super) fn expire(path: &Path) {
    let file = OpenOptions::new().write(true).open(path).unwrap();
    file.set_times(
        std::fs::FileTimes::new()
            .set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1)),
    )
    .unwrap();
}
