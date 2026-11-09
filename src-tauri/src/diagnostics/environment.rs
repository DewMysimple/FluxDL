use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildIdentity {
    pub revision: Option<String>,
    pub dirty: Option<bool>,
    pub app_version: String,
}
impl BuildIdentity {
    pub fn compiled() -> Self {
        Self {
            revision: option_env!("FLUXDL_CODE_REVISION")
                .filter(|s| *s != "unknown")
                .map(str::to_owned),
            dirty: match option_env!("FLUXDL_CODE_DIRTY") {
                Some("true") => Some(true),
                Some("false") => Some(false),
                _ => None,
            },
            app_version: env!("CARGO_PKG_VERSION").into(),
        }
    }
    pub fn valid(&self) -> bool {
        self.revision
            .as_ref()
            .is_none_or(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            && numeric_version(&self.app_version)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    pub build: BuildIdentity,
    pub engines: BTreeMap<String, String>,
}
impl Environment {
    pub fn valid(&self) -> bool {
        self.build.valid()
            && self.engines.len() <= 3
            && self.engines.iter().all(|(k, v)| {
                matches!(k.as_str(), "yt-dlp" | "ffmpeg" | "ffprobe") && numeric_version(v)
            })
    }
}
fn numeric_version(s: &str) -> bool {
    s.len() <= 24
        && (2..=4).contains(&s.split('.').count())
        && s.split('.')
            .all(|p| !p.is_empty() && p.len() <= 4 && p.bytes().all(|b| b.is_ascii_digit()))
}
