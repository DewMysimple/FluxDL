//! One safe display boundary for engine output, native failures, and IPC errors.
use regex::Regex;
use std::sync::LazyLock;

pub fn strip_ansi(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for n in chars.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(n) = chars.next() {
                        if n == '\u{7}' || (n == '\u{1b}' && chars.peek() == Some(&'\\')) {
                            if n == '\u{1b}' {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if !c.is_control() || matches!(c, '\n' | '\t') {
            out.push(c);
        }
    }
    out
}

#[derive(Default)]
pub struct LogRedactor {
    sensitive_continuation: bool,
}
impl LogRedactor {
    pub fn redact(&mut self, value: &str) -> String {
        redact_record(value, &mut self.sensitive_continuation)
    }
    pub fn omit_line(&mut self) -> &'static str {
        self.sensitive_continuation = true;
        "引擎输出单行超过 8192 字节，已省略该行并继续读取。"
    }
}
pub fn report_error(error: &crate::errors::AppError) {
    eprintln!(
        "FluxDL {:?}/{:?}: {}",
        error.code,
        error.phase,
        error.safe_text()
    );
}
pub fn redact_log(value: &str) -> String {
    LogRedactor::default().redact(value)
}
/// Raw engine output can contain arbitrary titles and filenames. Never retain it.
pub fn engine_output_summary(value: &str) -> &'static str {
    let value = value.trim_start();
    if value.starts_with("ERROR:") {
        "引擎报告错误；请按诊断中心的阶段与错误分类排查。"
    } else if value.starts_with("WARNING:") {
        "引擎报告警告；原始媒体、链接和路径详情已省略。"
    } else {
        "引擎输出已接收；原始媒体、链接和路径详情已省略。"
    }
}
fn redact_record(value: &str, sensitive_continuation: &mut bool) -> String {
    static HEADERS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)\b(?:proxy-authorization|authorization|set-cookie|cookie|x-auth-token|x-api-key|auth)\b["']?\s*[:=]\s*"#).unwrap()
    });
    static SECRETS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)\b(?:access[_-]?token|refresh[_-]?token|api[_-]?key|password|passwd|token)\b["']?\s*[:=]\s*"#).unwrap()
    });
    static URLS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)\b[a-z][a-z0-9+.-]*://[^\s<>"'`]+"#).unwrap());
    static PATHS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)(?:\b[a-z]:[\\/]|\\\\[^\\\s]+\\|/(?:home|users|tmp|var)/)[^\r\n<>\"|]*"#)
            .unwrap()
    });
    let clean = strip_ansi(value).replace("\\/", "/");
    let mut lines = Vec::new();
    for line in clean.lines() {
        if *sensitive_continuation && line.starts_with([' ', '\t']) {
            lines.push("[敏感续行已隐藏]".to_owned());
            continue;
        }
        *sensitive_continuation = false;
        let boundary = HEADERS
            .find(line)
            .into_iter()
            .chain(SECRETS.find(line))
            .min_by_key(|found| found.start());
        let safe = if let Some(found) = boundary {
            *sensitive_continuation = true;
            format!("{}[敏感值已隐藏]", &line[..found.end()])
        } else {
            line.to_owned()
        };
        let safe = URLS.replace_all(&safe, "[链接已隐藏]");
        lines.push(PATHS.replace_all(&safe, "[本地路径已隐藏]").into_owned());
    }
    lines.join("\n").chars().take(1800).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_headers_and_proxy_urls_share_one_safe_boundary() {
        for input in [
            "Authorization: Bearer SECRET\n continuation-SECRET\npublic reason",
            r#"{"Cookie":"sid=SECRET;token=SECRET","other":"SECRET"}"#,
            "pRoXy-AuThOrIzAtIoN = Basic SECRET",
            "SET-cookie: session=SECRET",
            "proxy SOCKS5://user:SECRET@localhost:1080/path?token=SECRET#SECRET",
            "HTTP://user:SECRET@example.com/a?SECRET#SECRET https://example.org/b?SECRET",
            r#"{"url":"https:\/\/user:SECRET@example.com/a?SECRET"}"#,
            "api_key=SECRET and other SECRET",
            "api_key=SECRET Cookie: visible",
            "Cookie: SECRET api_key=visible",
            "X-Auth-Token: SECRET",
            "X-API-Key: SECRET",
            "--password=SECRET",
            "auth: SECRET",
        ] {
            let safe = redact_log(input);
            assert!(!safe.contains("SECRET"), "{safe}");
        }
        assert!(!redact_log("https://example.com/path?secret").contains("example.com/path"));
        assert_eq!(redact_log("\u{1b}[31m普通错误\u{1b}[0m"), "普通错误");
    }
    #[test]
    fn split_lines_and_omitted_headers_keep_sensitive_continuations_private() {
        let mut redactor = LogRedactor::default();
        assert!(!redactor
            .redact("Authorization: Basic FIRST_PRIVATE_VALUE")
            .contains("FIRST_PRIVATE_VALUE"));
        assert!(!redactor
            .redact(" second-private-line")
            .contains("second-private-line"));
        assert_eq!(
            redactor.redact("ordinary public reason"),
            "ordinary public reason"
        );
        redactor.omit_line();
        assert!(!redactor
            .redact(" hidden-after-oversized-header")
            .contains("hidden-after-oversized-header"));
        assert_eq!(redactor.redact("public again"), "public again");
    }
}
