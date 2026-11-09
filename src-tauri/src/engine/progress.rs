use crate::diagnostics::strip_ansi;
use serde_json::Value;
#[derive(Debug, Clone, PartialEq)]
pub enum ProgressEvent {
    Download {
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        speed: Option<f64>,
        eta: Option<f64>,
        progress: Option<f64>,
    },
    Postprocessing,
    File(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedProgress {
    pub event: ProgressEvent,
    pub stream_id: Option<String>,
    pub finished: bool,
}

pub fn parse_progress_fact(line: &str) -> Option<ParsedProgress> {
    let event = parse_progress(line)?;
    let clean = strip_ansi(line);
    let object = clean
        .trim()
        .strip_prefix("__FLUX_PROGRESS__")
        .and_then(|text| serde_json::from_str::<Value>(text).ok());
    let stream_id = object.as_ref().and_then(|value| {
        let filename = progress_field(value, "filename")
            .as_str()
            .filter(|v| !v.is_empty() && *v != "NA");
        let format = value
            .get("format_id")
            .or_else(|| value.get("info.format_id"))
            .or_else(|| value.pointer("/info_dict/format_id"))
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty() && *v != "NA");
        match (filename, format) {
            (Some(name), Some(format)) => Some(format!("{format}:{name}")),
            (Some(name), None) => Some(name.to_owned()),
            (None, Some(format)) => Some(format.to_owned()),
            _ => None,
        }
        .filter(|id| id.len() <= 4096 && !id.chars().any(char::is_control))
    });
    let finished = object
        .as_ref()
        .map(|value| progress_field(value, "status"))
        .and_then(Value::as_str)
        == Some("finished");
    Some(ParsedProgress {
        event,
        stream_id,
        finished,
    })
}

pub fn parse_progress(line: &str) -> Option<ProgressEvent> {
    let clean = strip_ansi(line);
    let clean = clean.trim();
    if let Some(text) = clean.strip_prefix("__FLUX_PROGRESS__") {
        let value: Value = serde_json::from_str(text).ok()?;
        if !value.is_object() {
            return None;
        }
        let bytes = json_u64(progress_field(&value, "downloaded_bytes"));
        let known_status = matches!(
            progress_field(&value, "status").as_str(),
            Some("downloading" | "finished")
        );
        if bytes.is_none() && !known_status {
            return None;
        }
        let downloaded_bytes = bytes.unwrap_or(0);
        let total_bytes = json_u64(progress_field(&value, "total_bytes"))
            .or_else(|| json_u64(progress_field(&value, "total_bytes_estimate")))
            .filter(|n| *n > 0);
        let progress = total_bytes
            .map(|total| (downloaded_bytes as f64 / total as f64 * 100.0).clamp(0.0, 100.0));
        Some(ProgressEvent::Download {
            downloaded_bytes,
            total_bytes,
            speed: json_f64(progress_field(&value, "speed")),
            eta: json_f64(progress_field(&value, "eta")),
            progress,
        })
    } else if clean.starts_with("__FLUX_POST__") {
        Some(ProgressEvent::Postprocessing)
    } else if let Some(text) = clean.strip_prefix("__FLUX_FILE__") {
        let path: String = serde_json::from_str(text).ok()?;
        if path.is_empty() || path.chars().any(char::is_control) {
            return None;
        }
        Some(ProgressEvent::File(path))
    } else {
        None
    }
}
fn progress_field<'a>(value: &'a Value, name: &str) -> &'a Value {
    value
        .get(name)
        .or_else(|| value.get(format!("progress.{name}")))
        .unwrap_or(&Value::Null)
}
fn json_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .filter(|v| v.is_finite() && *v >= 0.0)
}
fn json_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| json_f64(value).map(|f| f as u64))
}
