use std::time::{SystemTime, UNIX_EPOCH};

pub fn info(event: &str, fields: &[(&str, String)]) {
    log("INFO", event, fields);
}

pub fn warn(event: &str, fields: &[(&str, String)]) {
    log("WARN", event, fields);
}

pub fn error(event: &str, fields: &[(&str, String)]) {
    log("ERROR", event, fields);
}

fn log(level: &str, event: &str, fields: &[(&str, String)]) {
    let mut line = format!(
        "ts_ms={} level={} event={}",
        now_ms(),
        level,
        sanitize_token(event)
    );
    for (key, value) in fields {
        line.push(' ');
        line.push_str(&sanitize_token(key));
        line.push('=');
        line.push_str(&sanitize_value(value));
    }
    println!("{line}");
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn sanitize_token(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn sanitize_value(value: &str) -> String {
    let redacted = redact_sensitive_text(value);
    let value = redacted.as_str();
    if value.is_empty() {
        return "\"\"".to_owned();
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':'))
    {
        value.to_owned()
    } else {
        format!("{value:?}")
    }
}

pub(crate) fn redact_sensitive_text(text: &str) -> String {
    text.split_inclusive(char::is_whitespace)
        .map(redact_token_segment)
        .collect::<String>()
}

fn redact_token_segment(segment: &str) -> String {
    let token = segment.trim_end_matches(char::is_whitespace);
    let whitespace = &segment[token.len()..];
    format!("{}{}", redact_token(token), whitespace)
}

fn redact_token(token: &str) -> String {
    let key = token
        .split_once('=')
        .or_else(|| token.split_once(':'))
        .map(|(key, _)| key)
        .unwrap_or(token);

    if contains_sensitive_marker(key) {
        if let Some((key, _)) = token.split_once('=') {
            return format!("{key}=<redacted>");
        }
        if let Some((key, _)) = token.split_once(':') {
            return format!("{key}:<redacted>");
        }
        return "<redacted>".to_owned();
    }

    token.to_owned()
}

fn contains_sensitive_marker(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    const SUBSTRING_MARKERS: &[&str] = &[
        "authorization",
        "credential",
        "passwd",
        "password",
        "private_key",
        "secret",
        "token",
        "connection_string",
        "conn_str",
        "database_url",
        "db_url",
    ];
    const TOKEN_MARKERS: &[&str] = &[
        "auth", "bearer", "cert", "cookie", "dsn", "jwt", "key", "session",
    ];

    SUBSTRING_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
        || lower
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .any(|token| TOKEN_MARKERS.contains(&token))
}
