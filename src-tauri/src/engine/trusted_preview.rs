//! Short-lived backend authority. Never persisted or written to diagnostics.
use crate::{
    diagnostics::CacheOutcome,
    models::{EngineDiagnostics, MediaInfo, Settings},
};
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

pub const CAPACITY: usize = 128;
pub const TTL: Duration = Duration::from_secs(180);
const ENTRY_BYTES: usize = 256 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct Fingerprint(String);
impl Fingerprint {
    pub async fn read(settings: &Settings, engines: &EngineDiagnostics) -> Option<Self> {
        let configuration = serde_json::to_string(&(
            settings.engine_path.clone(),
            settings.ffmpeg_path.clone(),
            engines.clone(),
        ))
        .ok()?;
        let paths: Vec<_> = [
            &engines.ytdlp,
            &engines.ffmpeg,
            &engines.ffprobe,
            &engines.runtime,
        ]
        .into_iter()
        .filter(|e| e.available)
        .filter_map(|e| e.path.clone())
        .collect();
        tokio::task::spawn_blocking(move || {
            let mut identity = configuration;
            for path in paths {
                let path = std::fs::canonicalize(path).ok()?;
                let meta = std::fs::metadata(&path).ok()?;
                if !meta.is_file() {
                    return None;
                }
                identity.push_str(&format!(
                    "{:?}:{}:{:?}:{:?}",
                    path,
                    meta.len(),
                    meta.modified().ok()?,
                    meta.created().ok()
                ));
            }
            Some(Self(identity))
        })
        .await
        .ok()
        .flatten()
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct Selection {
    pub mode: String,
    pub quality: String,
    pub format: Option<String>,
    pub directory: String,
}
struct Entry {
    reference: String,
    created: Instant,
    generation: u64,
    fingerprint: Fingerprint,
    mode: String,
    media: MediaInfo,
    gate: Arc<Mutex<()>>,
    accepted: Option<(Selection, String)>,
    parse_trace: Option<String>,
}
pub struct Lookup {
    pub outcome: CacheOutcome,
    pub media: Option<MediaInfo>,
    pub duplicate: Option<String>,
    pub parse_trace: Option<String>,
}
#[derive(Default)]
pub struct TrustedPreviews {
    entries: VecDeque<Entry>,
}
impl TrustedPreviews {
    pub fn store(
        &mut self,
        media: &MediaInfo,
        mode: String,
        generation: u64,
        fingerprint: Fingerprint,
    ) -> Option<String> {
        // Cache no raw engine JSON, request headers or extracted media/CDN URL.
        let mut media = media.clone();
        media.trusted_preview_ref = None;
        if media.is_playlist || serde_json::to_vec(&media).ok()?.len() > ENTRY_BYTES {
            return None;
        }
        self.entries.retain(|e| e.created.elapsed() < TTL);
        while self.entries.len() >= CAPACITY {
            self.entries.pop_front();
        }
        // Disjoint from diagnostic UUIDs: the opaque capability cannot itself
        // be accepted as a trace/task/related ID and accidentally logged.
        let reference = format!("pv_{}", uuid::Uuid::new_v4().simple());
        self.entries.push_back(Entry {
            reference: reference.clone(),
            created: Instant::now(),
            generation,
            fingerprint,
            mode,
            media,
            gate: Arc::new(Mutex::new(())),
            accepted: None,
            parse_trace: None,
        });
        Some(reference)
    }
    pub fn link_parse(&mut self, reference: &str, trace: &str) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.reference == reference) {
            e.parse_trace = Some(trace.to_owned());
        }
    }
    pub fn gate(&self, reference: Option<&str>) -> Option<Arc<Mutex<()>>> {
        let reference = reference?;
        self.entries
            .iter()
            .find(|e| e.reference == reference)
            .map(|e| e.gate.clone())
    }
    pub fn lookup(
        &mut self,
        reference: Option<&str>,
        url: &str,
        selection: &Selection,
        generation: u64,
        fingerprint: Option<&Fingerprint>,
        enabled: bool,
    ) -> Lookup {
        let miss = |outcome| Lookup {
            outcome,
            media: None,
            duplicate: None,
            parse_trace: None,
        };
        if !enabled {
            return miss(CacheOutcome::Disabled);
        }
        let Some(index) = self
            .entries
            .iter()
            .position(|e| Some(e.reference.as_str()) == reference)
        else {
            return miss(CacheOutcome::Missing);
        };
        if self.entries[index].created.elapsed() >= TTL {
            self.entries.remove(index);
            return miss(CacheOutcome::Expired);
        }
        let entry = &self.entries[index];
        if entry.generation != generation || Some(&entry.fingerprint) != fingerprint {
            return miss(CacheOutcome::ConfigChanged);
        }
        if entry.media.url != url {
            return miss(CacheOutcome::SourceMismatch);
        }
        if entry.mode != selection.mode {
            return miss(CacheOutcome::ModeMismatch);
        }
        if selection
            .format
            .as_ref()
            .is_some_and(|id| !entry.media.formats.iter().any(|f| &f.format_id == id))
        {
            return miss(CacheOutcome::FormatMismatch);
        }
        let duplicate = entry
            .accepted
            .as_ref()
            .filter(|(prior, _)| prior == selection)
            .map(|(_, id)| id.clone());
        Lookup {
            outcome: if duplicate.is_some() {
                CacheOutcome::Duplicate
            } else {
                CacheOutcome::Hit
            },
            media: Some(entry.media.clone()),
            duplicate,
            parse_trace: entry.parse_trace.clone(),
        }
    }
    pub fn accept(&mut self, reference: &str, selection: Selection, task: String) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.reference == reference) {
            e.accepted = Some((selection, task));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn media() -> MediaInfo {
        crate::engine::media_from_json(&serde_json::json!({"id":"fixture", "title":"fixture", "format_id":"direct", "ext":"mp4", "vcodec":"h264", "acodec":"aac"}), "https://fixture.invalid/original", "").unwrap()
    }
    fn selection() -> Selection {
        Selection {
            mode: "video".into(),
            quality: "1080".into(),
            format: Some("direct".into()),
            directory: "fixture-directory".into(),
        }
    }
    #[test]
    fn trusted_preview_is_random_session_local_bounded_and_expires_monotonically() {
        let mut cache = TrustedPreviews::default();
        let fingerprint = Fingerprint("fixture-engine".into());
        let first = cache
            .store(&media(), "video".into(), 1, fingerprint.clone())
            .unwrap();
        assert_eq!(
            uuid::Uuid::parse_str(first.strip_prefix("pv_").unwrap())
                .unwrap()
                .get_version_num(),
            4
        );
        assert!(!crate::diagnostics::valid_id(&first));
        assert_eq!(
            cache
                .lookup(
                    Some(&first),
                    &media().url,
                    &selection(),
                    1,
                    Some(&fingerprint),
                    true
                )
                .outcome,
            CacheOutcome::Hit
        );
        assert_eq!(
            TrustedPreviews::default()
                .lookup(
                    Some(&first),
                    &media().url,
                    &selection(),
                    1,
                    Some(&fingerprint),
                    true
                )
                .outcome,
            CacheOutcome::Missing
        );
        cache.entries[0].created = Instant::now() - TTL - Duration::from_secs(1);
        assert_eq!(
            cache
                .lookup(
                    Some(&first),
                    &media().url,
                    &selection(),
                    1,
                    Some(&fingerprint),
                    true
                )
                .outcome,
            CacheOutcome::Expired
        );
        for _ in 0..CAPACITY + 1 {
            cache
                .store(&media(), "video".into(), 1, fingerprint.clone())
                .unwrap();
        }
        assert_eq!(cache.entries.len(), CAPACITY);
        assert!(cache.entries.iter().all(|e| e.reference != first));
        let mut oversized = media();
        oversized.warnings = vec!["x".repeat(ENTRY_BYTES)];
        assert!(cache
            .store(&oversized, "video".into(), 1, fingerprint)
            .is_none());
        assert_eq!(cache.entries.len(), CAPACITY);
    }
    #[test]
    fn trusted_preview_rejects_forgery_sources_modes_formats_and_configuration_changes() {
        let mut cache = TrustedPreviews::default();
        let fp = Fingerprint("engine-a".into());
        let token = cache
            .store(&media(), "video".into(), 7, fp.clone())
            .unwrap();
        let mut check = |token: Option<&str>,
                         url: &str,
                         request: &Selection,
                         generation: u64,
                         fingerprint: Option<&Fingerprint>,
                         enabled: bool| {
            cache
                .lookup(token, url, request, generation, fingerprint, enabled)
                .outcome
        };
        assert_eq!(
            check(None, &media().url, &selection(), 7, Some(&fp), true),
            CacheOutcome::Missing
        );
        assert_eq!(
            check(
                Some(&uuid::Uuid::new_v4().to_string()),
                &media().url,
                &selection(),
                7,
                Some(&fp),
                true
            ),
            CacheOutcome::Missing
        );
        assert_eq!(
            check(
                Some(&token),
                "https://fixture.invalid/other",
                &selection(),
                7,
                Some(&fp),
                true
            ),
            CacheOutcome::SourceMismatch
        );
        assert_eq!(
            check(
                Some(&token),
                &media().url,
                &Selection {
                    mode: "audio".into(),
                    ..selection()
                },
                7,
                Some(&fp),
                true
            ),
            CacheOutcome::ModeMismatch
        );
        assert_eq!(
            check(
                Some(&token),
                &media().url,
                &Selection {
                    format: Some("direct+unsafe".into()),
                    ..selection()
                },
                7,
                Some(&fp),
                true
            ),
            CacheOutcome::FormatMismatch
        );
        assert_eq!(
            check(Some(&token), &media().url, &selection(), 8, Some(&fp), true),
            CacheOutcome::ConfigChanged
        );
        assert_eq!(
            check(
                Some(&token),
                &media().url,
                &selection(),
                7,
                Some(&Fingerprint("engine-b".into())),
                true
            ),
            CacheOutcome::ConfigChanged
        );
        assert_eq!(
            check(Some(&token), &media().url, &selection(), 7, None, true),
            CacheOutcome::ConfigChanged
        );
        assert_eq!(
            check(
                Some(&token),
                &media().url,
                &selection(),
                7,
                Some(&fp),
                false
            ),
            CacheOutcome::Disabled
        );
        cache.accept(&token, selection(), "fixture-task".into());
        assert_eq!(
            cache
                .lookup(Some(&token), &media().url, &selection(), 7, Some(&fp), true)
                .duplicate
                .as_deref(),
            Some("fixture-task")
        );
        assert!(cache
            .lookup(
                Some(&token),
                &media().url,
                &Selection {
                    quality: "720".into(),
                    ..selection()
                },
                7,
                Some(&fp),
                true
            )
            .duplicate
            .is_none());
    }
    #[tokio::test]
    async fn trusted_preview_fingerprint_detects_binary_replacement_and_missing_executables() {
        let root = std::env::temp_dir().join(format!("fluxdl-preview-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("fixture.exe");
        std::fs::write(&path, b"fixture-one").unwrap();
        let mut engines = EngineDiagnostics::default();
        engines.ytdlp.available = true;
        engines.ytdlp.path = Some(path.to_string_lossy().into_owned());
        let before = Fingerprint::read(&Settings::default(), &engines)
            .await
            .unwrap();
        std::fs::write(&path, b"fixture-two-with-different-size").unwrap();
        let after = Fingerprint::read(&Settings::default(), &engines)
            .await
            .unwrap();
        assert!(before != after);
        std::fs::remove_file(&path).unwrap();
        assert!(Fingerprint::read(&Settings::default(), &engines)
            .await
            .is_none());
        std::fs::remove_dir(&root).unwrap();
    }
}
