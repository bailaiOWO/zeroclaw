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
        let raw_message = event
            .raw_message
            .clone()
            .or_else(|| extract_message_text(&event.message));

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
                        .is_some_and(|msg| message_mentions_self(msg, self_id.as_deref()));
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

        let parsed_text = extract_message_text(&event.message).or(raw_message);
        let content = parsed_text
            .map(|text| strip_cq_codes(&text))
            .unwrap_or_default()
            .trim()
            .to_string();

        if content.is_empty() {
            return None;
        }

        Some(ChannelMessage {
            id: if message_id.is_empty() {
                Uuid::new_v4().to_string()
            } else {
                message_id
            },
            sender: user_id,
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
        let (action, mut body) = Self::parse_recipient(&message.recipient)?;
        body["message"] = json!(message.content);

        self.call_api(action, body).await?;
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

fn extract_message_text(message: &serde_json::Value) -> Option<String> {
    match message {
        serde_json::Value::String(s) => Some(s.to_string()),
        serde_json::Value::Array(segments) => {
            let mut buf = String::new();
            for seg in segments {
                if seg.get("type").and_then(|v| v.as_str()) == Some("text") {
                    if let Some(text) = seg
                        .get("data")
                        .and_then(|v| v.get("text"))
                        .and_then(|v| v.as_str())
                    {
                        buf.push_str(text);
                    }
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

fn message_mentions_self(message: &str, self_id: Option<&str>) -> bool {
    let Some(id) = self_id else {
        return false;
    };
    message.contains(&format!("[CQ:at,qq={id}]")) || message.contains(&format!("[CQ:at,qq={id},"))
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
        })
    }

    #[test]
    fn channel_name_is_stable() {
        assert_eq!(channel().name(), "onebot_v11");
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
        headers.insert(header::AUTHORIZATION, "Bearer napcat_token".parse().unwrap());

        assert!(callback_is_authorized("napcat_token", &headers, br#"{"k":"v"}"#));
    }

    #[test]
    fn callback_auth_accepts_napcat_x_signature() {
        let token = "napcat_token";
        let body = br#"{"post_type":"message","message_type":"private","user_id":1,"message":"hi"}"#;

        let key =
            ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, token.as_bytes());
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
        assert!(!callback_is_authorized("napcat_token", &headers, br#"{"post_type":"message"}"#));
    }
}
