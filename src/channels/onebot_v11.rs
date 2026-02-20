use super::traits::{Channel, ChannelMessage, SendMessage};
use crate::security::pairing::constant_time_eq;
use anyhow::{anyhow, bail, Context};
use async_trait::async_trait;
use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use serde_json::json;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use uuid::Uuid;

const DEFAULT_CALLBACK_PATH: &str = "/onebot/v11";
const DEDUP_CAPACITY: usize = 10_000;

/// OneBot v11 channel (NapCat / go-cqhttp compatible).
///
/// - Receive: HTTP POST callback from OneBot implementation.
/// - Send: HTTP API (`send_private_msg` / `send_group_msg`).
pub struct OneBotV11Channel {
    api_url: String,
    access_token: Option<String>,
    listen_host: String,
    listen_port: u16,
    allowed_users: Vec<String>,
    allowed_groups: Vec<String>,
    require_at_in_group: bool,
    client: reqwest::Client,
    dedup: Arc<Mutex<HashSet<String>>>,
}

impl OneBotV11Channel {
    pub fn new(config: crate::config::schema::OneBotV11Config) -> Self {
        Self {
            api_url: normalize_api_url(&config.api_url),
            access_token: config
                .access_token
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            listen_host: if config.listen_host.trim().is_empty() {
                "0.0.0.0".to_string()
            } else {
                config.listen_host
            },
            listen_port: config.listen_port,
            allowed_users: config.allowed_users,
            allowed_groups: config.allowed_groups,
            require_at_in_group: config.require_at_in_group,
            client: reqwest::Client::new(),
            dedup: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn is_user_allowed(&self, user_id: &str) -> bool {
        allowlist_contains(&self.allowed_users, user_id, false)
    }

    fn is_group_allowed(&self, group_id: &str) -> bool {
        allowlist_contains(&self.allowed_groups, group_id, true)
    }

    fn is_duplicate(&self, msg_id: &str) -> bool {
        if msg_id.trim().is_empty() {
            return false;
        }

        let mut dedup = self.dedup.lock().expect("onebot_v11 dedup lock poisoned");

        if dedup.contains(msg_id) {
            return true;
        }

        if dedup.len() >= DEDUP_CAPACITY {
            let keys: Vec<String> = dedup.iter().take(DEDUP_CAPACITY / 2).cloned().collect();
            for key in keys {
                dedup.remove(&key);
            }
        }

        dedup.insert(msg_id.to_string());
        false
    }

    fn parse_event(&self, event: OneBotEvent) -> Option<ChannelMessage> {
        if event.post_type != "message" {
            return None;
        }

        let user_id = event.user_id.map(|v| v.to_string())?;
        if !self.is_user_allowed(&user_id) {
            tracing::warn!("OneBot v11: ignoring unauthorized user: {user_id}");
            return None;
        }

        let self_id = event.self_id.as_ref().and_then(json_value_to_string);
        let raw_message = event.raw_message.clone();
        let parsed_text = extract_message_text(&event.message).or_else(|| raw_message.clone());

        let reply_target = match event.message_type.as_str() {
            "private" => format!("private:{user_id}"),
            "group" => {
                let group_id = event.group_id.map(|v| v.to_string())?;
                if !self.is_group_allowed(&group_id) {
                    tracing::warn!("OneBot v11: ignoring unauthorized group: {group_id}");
                    return None;
                }

                if self.require_at_in_group {
                    let to_me = event.to_me.unwrap_or(false);
                    let mentioned = raw_message
                        .as_deref()
                        .is_some_and(|msg| message_mentions_self(msg, self_id.as_deref()))
                        || message_segments_mention_self(&event.message, self_id.as_deref());
                    if !to_me && !mentioned {
                        return None;
                    }
                }

                format!("group:{group_id}")
            }
            _ => return None,
        };

        let message_id = event
            .message_id
            .as_ref()
            .and_then(json_value_to_string)
            .unwrap_or_default();
        if self.is_duplicate(&message_id) {
            return None;
        }

        let content = parsed_text
            .map(|text| normalize_incoming_message_text(&text))
            .unwrap_or_default()
            .trim()
            .to_string();

        if content.is_empty() {
            return None;
        }

        let sender_name = event.sender.as_ref().and_then(|s| {
            s.card
                .as_deref()
                .filter(|c| !c.trim().is_empty())
                .or_else(|| s.nickname.as_deref().filter(|n| !n.trim().is_empty()))
                .map(ToString::to_string)
        });

        Some(ChannelMessage {
            id: if message_id.is_empty() {
                Uuid::new_v4().to_string()
            } else {
                message_id
            },
            sender: user_id,
            sender_name,
            reply_target,
            content,
            channel: "onebot_v11".to_string(),
            timestamp: event.time.unwrap_or_else(now_unix_secs),
        })
    }

    fn with_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = self.access_token.as_deref() {
            req.header(header::AUTHORIZATION, format!("Bearer {token}"))
        } else {
            req
        }
    }

    async fn call_api(
        &self,
        action: &str,
        payload: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}/{}", self.api_url, action.trim_start_matches('/'));
        let resp = self
            .with_auth(self.client.post(&url))
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("OneBot API request failed: {action}"))?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            bail!("OneBot API {action} failed ({status}): {body}");
        }

        let parsed: serde_json::Value = serde_json::from_str(&body)
            .unwrap_or_else(|_| json!({ "status": "ok", "raw": body.clone() }));

        let retcode = parsed.get("retcode").and_then(|v| v.as_i64()).unwrap_or(0);
        let status_text = parsed
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("ok");

        if retcode != 0 || !status_text.eq_ignore_ascii_case("ok") {
            bail!("OneBot API {action} returned status={status_text} retcode={retcode}: {body}");
        }

        Ok(parsed)
    }

    fn parse_recipient(recipient: &str) -> anyhow::Result<(&'static str, serde_json::Value)> {
        if let Some(group_id) = recipient.strip_prefix("group:") {
            if group_id.trim().is_empty() {
                bail!("OneBot recipient group id is empty");
            }
            return Ok((
                "send_group_msg",
                json!({ "group_id": numeric_or_string(group_id) }),
            ));
        }

        let user_id = recipient
            .strip_prefix("private:")
            .or_else(|| recipient.strip_prefix("user:"))
            .unwrap_or(recipient)
            .trim();

        if user_id.is_empty() {
            bail!("OneBot recipient user id is empty");
        }

        Ok((
            "send_private_msg",
            json!({ "user_id": numeric_or_string(user_id) }),
        ))
    }

    async fn listen_http(
        &self,
        tx: tokio::sync::mpsc::Sender<ChannelMessage>,
    ) -> anyhow::Result<()> {
        #[derive(Clone)]
        struct CallbackState {
            channel: Arc<OneBotV11Channel>,
            tx: tokio::sync::mpsc::Sender<ChannelMessage>,
        }

        async fn handle_onebot_event(
            State(state): State<CallbackState>,
            headers: HeaderMap,
            body: Bytes,
        ) -> impl IntoResponse {
            if let Some(expected) = state.channel.access_token.as_deref() {
                if !callback_is_authorized(expected, &headers, &body) {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({ "status": "failed", "reason": "unauthorized" })),
                    )
                        .into_response();
                }
            }

            let payload: serde_json::Value = match serde_json::from_slice(&body) {
                Ok(v) => v,
                Err(err) => {
                    tracing::warn!("OneBot v11: invalid callback payload: {err}");
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "status": "failed", "reason": "invalid_payload" })),
                    )
                        .into_response();
                }
            };

            let event: OneBotEvent = match serde_json::from_value(payload) {
                Ok(ev) => ev,
                Err(err) => {
                    tracing::warn!("OneBot v11: invalid callback payload: {err}");
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "status": "failed", "reason": "invalid_payload" })),
                    )
                        .into_response();
                }
            };

            if let Some(message) = state.channel.parse_event(event) {
                if state.tx.send(message).await.is_err() {
                    tracing::warn!("OneBot v11: message channel closed");
                }
            }

            (StatusCode::OK, Json(json!({ "status": "ok" }))).into_response()
        }

        let state = CallbackState {
            channel: Arc::new(Self {
                api_url: self.api_url.clone(),
                access_token: self.access_token.clone(),
                listen_host: self.listen_host.clone(),
                listen_port: self.listen_port,
                allowed_users: self.allowed_users.clone(),
                allowed_groups: self.allowed_groups.clone(),
                require_at_in_group: self.require_at_in_group,
                client: self.client.clone(),
                dedup: self.dedup.clone(),
            }),
            tx,
        };

        let app = Router::new()
            .route(DEFAULT_CALLBACK_PATH, post(handle_onebot_event))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind((self.listen_host.as_str(), self.listen_port))
            .await
            .with_context(|| {
                format!(
                    "OneBot v11 failed to bind callback listener on {}:{}",
                    self.listen_host, self.listen_port
                )
            })?;

        let local_addr = listener.local_addr().ok();
        if let Some(addr) = local_addr {
            tracing::info!(
                "OneBot v11 callback listening on http://{}{}",
                addr,
                DEFAULT_CALLBACK_PATH
            );
        } else {
            tracing::info!(
                "OneBot v11 callback listening on {}:{}{}",
                self.listen_host,
                self.listen_port,
                DEFAULT_CALLBACK_PATH
            );
        }

        axum::serve(listener, app)
            .await
            .map_err(|err| anyhow!("OneBot v11 callback server exited: {err}"))
    }
}

#[async_trait]
impl Channel for OneBotV11Channel {
    fn name(&self) -> &str {
        "onebot_v11"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        // Strip internal tool-call tags to avoid leaking markup into QQ.
        let content = strip_tool_call_tags(&message.content);

        let (action, mut body) = Self::parse_recipient(&message.recipient)?;

        let (mut text_without_markers, mut attachments) = parse_attachment_markers(&content);
        if attachments.is_empty() {
            if let Some(att) = parse_path_only_attachment(&content) {
                attachments.push(att);
                text_without_markers = String::new();
            }
        }

        let mut media_segments: Vec<serde_json::Value> = Vec::new();
        let mut documents: Vec<OneBotAttachment> = Vec::new();

        for attachment in attachments {
            match attachment.kind {
                OneBotAttachmentKind::Document => documents.push(attachment),
                kind => {
                    media_segments.push(json!({
                        "type": kind.to_segment_type(),
                        "data": { "file": attachment.target }
                    }));
                }
            }
        }

        // OneBot v11 documents/files are typically delivered via upload_*_file actions.
        if !documents.is_empty() {
            let (upload_action, id_key) = if action == "send_group_msg" {
                ("upload_group_file", "group_id")
            } else {
                ("upload_private_file", "user_id")
            };

            let id_value = body.get(id_key).cloned().unwrap_or(serde_json::Value::Null);

            for doc in documents {
                let target = doc.target.strip_prefix("file://").unwrap_or(&doc.target);

                if is_http_url(target) {
                    if !text_without_markers.is_empty() {
                        text_without_markers.push('\n');
                    }
                    text_without_markers.push_str(&format!("文件链接: {target}"));
                    continue;
                }

                if !Path::new(target).exists() {
                    if !text_without_markers.is_empty() {
                        text_without_markers.push('\n');
                    }
                    text_without_markers.push_str(&format!("文件不存在: {target}"));
                    continue;
                }

                let name = Path::new(target)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file");

                let mut payload = json!({
                    "file": target,
                    "name": name,
                });
                payload[id_key] = id_value.clone();

                self.call_api(upload_action, payload).await?;
            }
        }

        // Send regular text / media segments.
        if !text_without_markers.trim().is_empty() || !media_segments.is_empty() {
            if media_segments.is_empty() {
                body["message"] = json!(text_without_markers);
            } else {
                let mut segments = Vec::new();
                if !text_without_markers.trim().is_empty() {
                    segments.push(json!({
                        "type": "text",
                        "data": { "text": text_without_markers }
                    }));
                }
                segments.extend(media_segments);
                body["message"] = json!(segments);
            }

            self.call_api(action, body).await?;
        }

        Ok(())
    }

    async fn listen(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        self.listen_http(tx).await
    }

    async fn health_check(&self) -> bool {
        self.call_api("get_login_info", json!({})).await.is_ok()
    }
}

#[derive(Debug, serde::Deserialize)]
struct OneBotEvent {
    #[serde(default)]
    post_type: String,
    #[serde(default)]
    message_type: String,
    #[serde(default)]
    message: serde_json::Value,
    #[serde(default)]
    raw_message: Option<String>,
    #[serde(default)]
    message_id: Option<serde_json::Value>,
    #[serde(default)]
    user_id: Option<i64>,
    #[serde(default)]
    group_id: Option<i64>,
    #[serde(default)]
    self_id: Option<serde_json::Value>,
    #[serde(default)]
    time: Option<u64>,
    #[serde(default)]
    to_me: Option<bool>,
    #[serde(default)]
    sender: Option<OneBotSender>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct OneBotSender {
    #[serde(default)]
    nickname: Option<String>,
    #[serde(default)]
    card: Option<String>,
}

fn normalize_api_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn allowlist_contains(list: &[String], value: &str, allow_when_empty: bool) -> bool {
    if list.iter().any(|entry| entry == "*") {
        return true;
    }
    if list.is_empty() {
        return allow_when_empty;
    }
    list.iter().any(|entry| entry == value)
}

fn numeric_or_string(value: &str) -> serde_json::Value {
    value
        .trim()
        .parse::<i64>()
        .map(serde_json::Value::from)
        .unwrap_or_else(|_| serde_json::Value::from(value.trim().to_string()))
}

fn json_value_to_string(value: &serde_json::Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(i) = value.as_i64() {
        return Some(i.to_string());
    }
    if let Some(u) = value.as_u64() {
        return Some(u.to_string());
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OneBotAttachmentKind {
    Image,
    Document,
    Video,
    Record,
}

impl OneBotAttachmentKind {
    fn from_marker(kind: &str) -> Option<Self> {
        match kind.trim().to_ascii_uppercase().as_str() {
            "IMAGE" | "PHOTO" => Some(Self::Image),
            "DOCUMENT" | "FILE" => Some(Self::Document),
            "VIDEO" => Some(Self::Video),
            "AUDIO" | "VOICE" | "RECORD" => Some(Self::Record),
            _ => None,
        }
    }

    fn to_segment_type(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Record => "record",
            // OneBot v11 does not standardize "file" message segments across implementations;
            // for documents we use upload_*_file APIs.
            Self::Document => "file",
        }
    }
}

#[derive(Debug, Clone)]
struct OneBotAttachment {
    kind: OneBotAttachmentKind,
    target: String,
}

fn is_http_url(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://")
}

fn infer_attachment_kind_from_target(target: &str) -> OneBotAttachmentKind {
    let normalized = target
        .split('?')
        .next()
        .unwrap_or(target)
        .split('#')
        .next()
        .unwrap_or(target);

    let extension = Path::new(normalized)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" => OneBotAttachmentKind::Image,
        "mp4" | "mov" | "mkv" | "avi" | "webm" => OneBotAttachmentKind::Video,
        "mp3" | "m4a" | "wav" | "flac" | "ogg" | "oga" | "opus" => OneBotAttachmentKind::Record,
        _ => OneBotAttachmentKind::Document,
    }
}

fn parse_path_only_attachment(message: &str) -> Option<OneBotAttachment> {
    let trimmed = message.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return None;
    }

    let candidate = trimmed.trim_matches(|c| matches!(c, '`' | '"' | '\''));
    if candidate.chars().any(char::is_whitespace) {
        return None;
    }

    let candidate = candidate.strip_prefix("file://").unwrap_or(candidate);

    if !is_http_url(candidate) && !Path::new(candidate).exists() {
        return None;
    }

    Some(OneBotAttachment {
        kind: infer_attachment_kind_from_target(candidate),
        target: candidate.to_string(),
    })
}

fn normalize_attachment_marker_whitespace(text: &str) -> String {
    let mut normalized_lines: Vec<String> = Vec::new();
    let mut blank_run = 0usize;

    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches('\r');

        if line.trim().is_empty() {
            blank_run += 1;
            continue;
        }

        if !normalized_lines.is_empty() {
            match blank_run {
                0 => {}
                1 => normalized_lines.push(String::new()),
                // Marker-only attachment lines can create large blank runs.
                // Collapse 2+ consecutive empty lines down to a single line break.
                _ => {}
            }
        }

        normalized_lines.push(line.to_string());
        blank_run = 0;
    }

    normalized_lines.join("\n").trim().to_string()
}

fn parse_attachment_markers(message: &str) -> (String, Vec<OneBotAttachment>) {
    let mut cleaned = String::with_capacity(message.len());
    let mut attachments = Vec::new();
    let mut cursor = 0;

    while cursor < message.len() {
        let Some(open_rel) = message[cursor..].find('[') else {
            cleaned.push_str(&message[cursor..]);
            break;
        };

        let open = cursor + open_rel;
        cleaned.push_str(&message[cursor..open]);

        let Some(close_rel) = message[open..].find(']') else {
            cleaned.push_str(&message[open..]);
            break;
        };

        let close = open + close_rel;
        let marker = &message[open + 1..close];

        let parsed = marker.split_once(':').and_then(|(kind, target)| {
            let kind = OneBotAttachmentKind::from_marker(kind)?;
            let target = target.trim();
            if target.is_empty() {
                return None;
            }
            Some(OneBotAttachment {
                kind,
                target: target.to_string(),
            })
        });

        if let Some(attachment) = parsed {
            attachments.push(attachment);
        } else {
            cleaned.push_str(&message[open..=close]);
        }

        cursor = close + 1;
    }

    let cleaned = if attachments.is_empty() {
        cleaned.trim().to_string()
    } else {
        normalize_attachment_marker_whitespace(cleaned.trim())
    };
    (cleaned, attachments)
}

fn strip_tool_call_tags(message: &str) -> String {
    let mut result = message.to_string();
    for (open, close) in [
        ("<tool>", "</tool>"),
        ("<toolcall>", "</toolcall>"),
        ("<tool-call>", "</tool-call>"),
    ] {
        while let Some(start) = result.find(open) {
            if let Some(end) = result[start..].find(close) {
                let end = start + end + close.len();
                result = format!("{}{}", &result[..start], &result[end..]);
            } else {
                break;
            }
        }
    }

    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }

    result.trim().to_string()
}

fn extract_message_text(message: &serde_json::Value) -> Option<String> {
    match message {
        serde_json::Value::String(s) => Some(s.to_string()),
        serde_json::Value::Array(segments) => {
            let mut buf = String::new();
            for seg in segments {
                let seg_type = seg.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match seg_type {
                    "text" => {
                        if let Some(text) = seg
                            .get("data")
                            .and_then(|v| v.get("text"))
                            .and_then(|v| v.as_str())
                        {
                            buf.push_str(text);
                        }
                    }
                    "image" | "video" | "record" | "file" => {
                        let data = seg.get("data");
                        let target = data
                            .and_then(|d| d.get("url"))
                            .and_then(|v| v.as_str())
                            .or_else(|| data.and_then(|d| d.get("file")).and_then(|v| v.as_str()))
                            .or_else(|| data.and_then(|d| d.get("name")).and_then(|v| v.as_str()))
                            .unwrap_or("");
                        if target.is_empty() {
                            continue;
                        }
                        if !buf.is_empty() && !buf.ends_with('\n') && !buf.ends_with(' ') {
                            buf.push('\n');
                        }
                        let marker = match seg_type {
                            "image" => format!("[IMAGE:{target}]"),
                            "video" => format!("[VIDEO:{target}]"),
                            "record" => format!("[VOICE:{target}]"),
                            "file" => format!("[DOCUMENT:{target}]"),
                            _ => continue,
                        };
                        buf.push_str(&marker);
                    }
                    _ => {}
                }
            }
            if buf.is_empty() {
                None
            } else {
                Some(buf)
            }
        }
        _ => None,
    }
}

fn cq_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\[CQ:[^\]]+\]").expect("valid cq regex"))
}

fn strip_cq_codes(text: &str) -> String {
    cq_regex().replace_all(text, "").to_string()
}

fn cq_code_to_marker(code: &str) -> Option<String> {
    let code = code.strip_prefix("CQ:")?;
    let mut parts = code.split(',');
    let kind = parts.next()?.trim();

    let mut file: Option<&str> = None;
    let mut url: Option<&str> = None;
    let mut name: Option<&str> = None;

    for part in parts {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "file" => file = Some(v),
            "url" => url = Some(v),
            "name" => name = Some(v),
            _ => {}
        }
    }

    let target = url.or(file).or(name)?.trim();
    if target.is_empty() {
        return None;
    }

    match kind {
        "image" => Some(format!("[IMAGE:{target}]")),
        "record" => Some(format!("[VOICE:{target}]")),
        "video" => Some(format!("[VIDEO:{target}]")),
        "file" => Some(format!("[DOCUMENT:{target}]")),
        _ => None,
    }
}

fn replace_cq_codes_with_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;

    for m in cq_regex().find_iter(text) {
        out.push_str(&text[cursor..m.start()]);
        let code = &text[m.start() + 1..m.end() - 1];
        if let Some(marker) = cq_code_to_marker(code) {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&marker);
        }
        cursor = m.end();
    }
    out.push_str(&text[cursor..]);
    out
}

fn normalize_incoming_message_text(text: &str) -> String {
    let replaced = replace_cq_codes_with_markers(text);
    strip_cq_codes(&replaced)
}

fn message_mentions_self(message: &str, self_id: Option<&str>) -> bool {
    let Some(id) = self_id else {
        return false;
    };
    message.contains(&format!("[CQ:at,qq={id}]")) || message.contains(&format!("[CQ:at,qq={id},"))
}

fn message_segments_mention_self(message: &serde_json::Value, self_id: Option<&str>) -> bool {
    let Some(id) = self_id else {
        return false;
    };
    let serde_json::Value::Array(segments) = message else {
        return false;
    };

    segments.iter().any(|seg| {
        seg.get("type").and_then(|v| v.as_str()) == Some("at")
            && seg
                .get("data")
                .and_then(|d| d.get("qq"))
                .and_then(json_value_to_string)
                .is_some_and(|qq| qq == id)
    })
}

fn extract_access_token(headers: &HeaderMap) -> Option<String> {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .unwrap_or_default();

    if let Some(token) = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("Token "))
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(token.to_string());
    }

    headers
        .get("X-Access-Token")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn extract_napcat_signature(headers: &HeaderMap) -> Option<String> {
    headers
        .get("X-Signature")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn verify_napcat_signature(token: &str, raw_body: &[u8], signature_header: &str) -> bool {
    let provided = signature_header.trim();
    let provided = provided
        .strip_prefix("sha1=")
        .or_else(|| provided.strip_prefix("SHA1="))
        .unwrap_or(provided)
        .trim();

    if provided.is_empty() {
        return false;
    }

    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, token.as_bytes());
    let expected = ring::hmac::sign(&key, raw_body);
    let expected_hex = hex::encode(expected.as_ref());

    constant_time_eq(&expected_hex, &provided.to_ascii_lowercase())
}

fn callback_is_authorized(expected_token: &str, headers: &HeaderMap, raw_body: &[u8]) -> bool {
    extract_access_token(headers)
        .as_deref()
        .is_some_and(|provided| constant_time_eq(provided.trim(), expected_token.trim()))
        || extract_napcat_signature(headers)
            .as_deref()
            .is_some_and(|sig| verify_napcat_signature(expected_token.trim(), raw_body, sig))
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel() -> OneBotV11Channel {
        OneBotV11Channel::new(crate::config::schema::OneBotV11Config {
            api_url: "http://127.0.0.1:3000".to_string(),
            access_token: None,
            listen_host: "127.0.0.1".to_string(),
            listen_port: 8096,
            allowed_users: vec!["*".to_string()],
            allowed_groups: vec![],
            require_at_in_group: true,
            admin_users: vec![],
            admin_only_tools: vec![],
            command_external_network_access: crate::config::OneBotCommandExternalNetworkAccess::Off,
            non_admin_context_file: "NON_ADMIN.md".to_string(),
        })
    }

    #[test]
    fn channel_name_is_stable() {
        assert_eq!(channel().name(), "onebot_v11");
    }

    #[test]
    fn parse_attachment_markers_collapses_marker_induced_blank_runs() {
        let message = "主人，图片下载好了。\n这就发给你哦：\n[IMAGE:/tmp/a.png]\n[IMAGE:/tmp/b.png]\n[IMAGE:/tmp/c.png]\n[IMAGE:/tmp/d.png]\n\n如果还要继续下载，告诉我就好。";

        let (cleaned, attachments) = parse_attachment_markers(message);

        assert_eq!(attachments.len(), 4);
        assert_eq!(
            cleaned,
            "主人，图片下载好了。\n这就发给你哦：\n如果还要继续下载，告诉我就好。"
        );
    }

    #[test]
    fn parse_attachment_markers_preserves_normal_single_paragraph_break() {
        let message = "第一段。\n\n第二段。\n[IMAGE:/tmp/a.png]\n[IMAGE:/tmp/b.png]\n第三段。";

        let (cleaned, attachments) = parse_attachment_markers(message);

        assert_eq!(attachments.len(), 2);
        assert_eq!(cleaned, "第一段。\n\n第二段。\n第三段。");
    }

    #[test]
    fn parse_attachment_markers_keeps_inline_text() {
        let message = "前缀[IMAGE:/tmp/a.png]后缀";

        let (cleaned, attachments) = parse_attachment_markers(message);

        assert_eq!(attachments.len(), 1);
        assert_eq!(cleaned, "前缀后缀");
    }

    #[test]
    fn parse_private_message() {
        let ch = channel();
        let event: OneBotEvent = serde_json::from_value(json!({
            "post_type": "message",
            "message_type": "private",
            "user_id": 10001,
            "message_id": 123,
            "message": "hello",
            "raw_message": "hello",
            "time": 1700000000
        }))
        .unwrap();

        let msg = ch.parse_event(event).expect("message should parse");
        assert_eq!(msg.reply_target, "private:10001");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.channel, "onebot_v11");
    }

    #[test]
    fn parse_group_message_requires_mention() {
        let ch = channel();

        let ignored: OneBotEvent = serde_json::from_value(json!({
            "post_type": "message",
            "message_type": "group",
            "user_id": 10001,
            "group_id": 20001,
            "self_id": 30001,
            "message_id": 1,
            "message": "大家好",
            "raw_message": "大家好"
        }))
        .unwrap();
        assert!(ch.parse_event(ignored).is_none());

        let accepted: OneBotEvent = serde_json::from_value(json!({
            "post_type": "message",
            "message_type": "group",
            "user_id": 10001,
            "group_id": 20001,
            "self_id": 30001,
            "message_id": 2,
            "message": "[CQ:at,qq=30001] 你好",
            "raw_message": "[CQ:at,qq=30001] 你好"
        }))
        .unwrap();

        let msg = ch.parse_event(accepted).expect("mentioned group message");
        assert_eq!(msg.reply_target, "group:20001");
        assert_eq!(msg.content, "你好");
    }

    #[test]
    fn allowlist_can_block_users() {
        let ch = OneBotV11Channel::new(crate::config::schema::OneBotV11Config {
            api_url: "http://127.0.0.1:3000".to_string(),
            access_token: None,
            listen_host: "127.0.0.1".to_string(),
            listen_port: 8096,
            allowed_users: vec!["42".to_string()],
            allowed_groups: vec![],
            require_at_in_group: false,
            admin_users: vec![],
            admin_only_tools: vec![],
            command_external_network_access: crate::config::OneBotCommandExternalNetworkAccess::Off,
            non_admin_context_file: "NON_ADMIN.md".to_string(),
        });

        let event: OneBotEvent = serde_json::from_value(json!({
            "post_type": "message",
            "message_type": "private",
            "user_id": 7,
            "message_id": 123,
            "message": "hello"
        }))
        .unwrap();

        assert!(ch.parse_event(event).is_none());
    }

    #[test]
    fn callback_auth_accepts_authorization_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer napcat_token".parse().unwrap(),
        );

        assert!(callback_is_authorized(
            "napcat_token",
            &headers,
            br#"{"k":"v"}"#
        ));
    }

    #[test]
    fn callback_auth_accepts_napcat_x_signature() {
        let token = "napcat_token";
        let body =
            br#"{"post_type":"message","message_type":"private","user_id":1,"message":"hi"}"#;

        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, token.as_bytes());
        let signature = ring::hmac::sign(&key, body);

        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Signature",
            format!("sha1={}", hex::encode(signature.as_ref()))
                .parse()
                .unwrap(),
        );

        assert!(callback_is_authorized(token, &headers, body));
    }

    #[test]
    fn callback_auth_rejects_invalid_napcat_x_signature() {
        let token = "napcat_token";
        let body = br#"{"post_type":"message"}"#;

        let mut headers = HeaderMap::new();
        headers.insert("X-Signature", "sha1=deadbeef".parse().unwrap());

        assert!(!callback_is_authorized(token, &headers, body));
    }

    #[test]
    fn callback_auth_rejects_when_no_auth_headers_exist() {
        let headers = HeaderMap::new();
        assert!(!callback_is_authorized(
            "napcat_token",
            &headers,
            br#"{"post_type":"message"}"#
        ));
    }
}
