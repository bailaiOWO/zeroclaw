use super::traits::{Tool, ToolResult};
use crate::channels::{Channel, OneBotV11Channel, SendMessage};
use crate::config::{Config, OneBotV11Config};
use crate::security::SecurityPolicy;
use anyhow::{anyhow, bail, Context};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const ONEBOT_API_TIMEOUT_SECS: u64 = 15;
const DEFAULT_CANDIDATE_LIMIT: usize = 12;
const MAX_CANDIDATE_LIMIT: usize = 30;

#[derive(Clone)]
struct OneBotApiClient {
    config: OneBotV11Config,
    client: reqwest::Client,
}

impl OneBotApiClient {
    fn new(config: OneBotV11Config) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(ONEBOT_API_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, client }
    }

    fn with_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = self
            .config
            .access_token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            req.header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"))
                .header("X-Access-Token", token)
        } else {
            req
        }
    }

    fn api_url(&self) -> String {
        self.config.api_url.trim().trim_end_matches('/').to_string()
    }

    async fn call_api(&self, action: &str, payload: Value) -> anyhow::Result<Value> {
        let api_url = self.api_url();
        if api_url.is_empty() {
            bail!("OneBot API URL 未配置");
        }

        let url = format!("{}/{}", api_url, action.trim_start_matches('/'));
        let resp = self
            .with_auth(self.client.post(&url))
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("OneBot API 请求失败: {action}"))?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("OneBot API {action} HTTP {status}: {body}");
        }

        let parsed: Value = serde_json::from_str(&body)
            .unwrap_or_else(|_| json!({ "status": "ok", "retcode": 0, "raw": body }));

        let retcode = parsed.get("retcode").and_then(|v| v.as_i64()).unwrap_or(0);
        let status_text = parsed
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("ok");

        if retcode != 0 || !status_text.eq_ignore_ascii_case("ok") {
            bail!(
                "OneBot API {action} 返回异常: status={status_text}, retcode={retcode}, body={}",
                truncate_for_log(&body, 500)
            );
        }

        Ok(parsed)
    }

    async fn get_friend_candidates(&self) -> anyhow::Result<Vec<ContactCandidate>> {
        let resp = self.call_api("get_friend_list", json!({})).await?;
        Ok(extract_data_array(&resp)
            .iter()
            .filter_map(parse_friend_candidate)
            .collect())
    }

    async fn get_group_candidates(&self) -> anyhow::Result<Vec<ContactCandidate>> {
        let resp = self.call_api("get_group_list", json!({})).await?;
        Ok(extract_data_array(&resp)
            .iter()
            .filter_map(parse_group_candidate)
            .collect())
    }

    async fn get_group_member_candidates(
        &self,
        group_id: &str,
    ) -> anyhow::Result<Vec<ContactCandidate>> {
        let gid = group_id.trim();
        if gid.is_empty() {
            bail!("group_id 不能为空");
        }

        let resp = self
            .call_api(
                "get_group_member_list",
                json!({ "group_id": numeric_or_string(gid) }),
            )
            .await?;

        Ok(extract_data_array(&resp)
            .iter()
            .filter_map(|item| parse_group_member_candidate(item, gid))
            .collect())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchScope {
    Auto,
    Friend,
    Group,
    GroupMember,
}

impl SearchScope {
    fn parse(raw: Option<&str>) -> anyhow::Result<Self> {
        match raw.unwrap_or("auto").trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "friend" | "friends" | "private" | "user" => Ok(Self::Friend),
            "group" | "groups" => Ok(Self::Group),
            "group_member" | "member" | "members" => Ok(Self::GroupMember),
            other => bail!("不支持的 scope: {other} (可选: auto/friend/group/group_member)"),
        }
    }
}

#[derive(Debug, Clone)]
enum OneBotAttachmentKind {
    Image,
    Document,
    Video,
    Audio,
    Voice,
}

impl OneBotAttachmentKind {
    fn marker_tag(&self) -> &'static str {
        match self {
            Self::Image => "IMAGE",
            Self::Document => "DOCUMENT",
            Self::Video => "VIDEO",
            Self::Audio => "AUDIO",
            Self::Voice => "VOICE",
        }
    }

    fn parse(raw: &str) -> anyhow::Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "image" => Ok(Self::Image),
            "document" | "file" => Ok(Self::Document),
            "video" => Ok(Self::Video),
            "audio" => Ok(Self::Audio),
            "voice" => Ok(Self::Voice),
            other => bail!("不支持的附件类型: {other} (可选: image/document/video/audio/voice)"),
        }
    }
}

#[derive(Debug, Clone)]
struct OneBotAttachmentArg {
    kind: OneBotAttachmentKind,
    target: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ContactCandidate {
    kind: String,
    id: String,
    display_name: String,
    alias: Option<String>,
    reply_target: String,
    group_id: Option<String>,
    group_name: Option<String>,
    score: u8,
    matched_by: String,
}

impl ContactCandidate {
    fn from_raw(
        kind: &str,
        id: String,
        display_name: String,
        alias: Option<String>,
        reply_target: String,
        group_id: Option<String>,
        group_name: Option<String>,
    ) -> Self {
        Self {
            kind: kind.to_string(),
            id,
            display_name,
            alias,
            reply_target,
            group_id,
            group_name,
            score: 0,
            matched_by: String::new(),
        }
    }

    fn display_brief(&self) -> String {
        let mut label = format!(
            "{} ({}) -> {}",
            self.display_name, self.id, self.reply_target
        );
        if let Some(alias) = self.alias.as_deref().filter(|v| !v.trim().is_empty()) {
            label.push_str(&format!(", 别名={alias}"));
        }
        if let Some(group_name) = self.group_name.as_deref().filter(|v| !v.trim().is_empty()) {
            label.push_str(&format!(", 群={group_name}"));
        } else if let Some(group_id) = self.group_id.as_deref().filter(|v| !v.trim().is_empty()) {
            label.push_str(&format!(", 群号={group_id}"));
        }
        label
    }
}

pub struct OneBotContactSearchTool {
    api: Option<OneBotApiClient>,
}

impl OneBotContactSearchTool {
    pub fn new(config: Arc<Config>, _security: Arc<SecurityPolicy>) -> Self {
        Self {
            api: config
                .channels_config
                .onebot_v11
                .as_ref()
                .cloned()
                .map(OneBotApiClient::new),
        }
    }
}

#[async_trait]
impl Tool for OneBotContactSearchTool {
    fn name(&self) -> &str {
        "onebot_contact_search"
    }

    fn description(&self) -> &str {
        "Search QQ contacts/groups via OneBot v11 by QQ号、昵称、群名、备注，返回可发送的 reply_target（private:xxx / group:xxx）。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "关键词：可输入 QQ 号、昵称、备注、群名"
                },
                "scope": {
                    "type": "string",
                    "description": "检索范围：auto(默认) / friend / group / group_member",
                    "enum": ["auto", "friend", "group", "group_member"],
                    "default": "auto"
                },
                "group_id": {
                    "type": "string",
                    "description": "当 scope=group_member 时建议提供群号；auto 时也可提供以额外检索该群成员"
                },
                "limit": {
                    "type": "integer",
                    "description": "最多返回候选数量，默认 12，最大 30",
                    "minimum": 1,
                    "maximum": 30
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let Some(api) = self.api.as_ref() else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("未配置 onebot_v11，无法检索 QQ 联系人。".to_string()),
            });
        };

        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow!("缺少 query 参数"))?;

        let scope = SearchScope::parse(args.get("scope").and_then(Value::as_str))?;
        let group_id = args.get("group_id").and_then(Value::as_str);
        let limit = parse_limit(args.get("limit").and_then(Value::as_u64));

        let matches = search_candidates(api, query, scope, group_id, limit).await?;
        let output = json!({
            "query": query,
            "scope": format_scope(scope),
            "group_id": group_id,
            "count": matches.len(),
            "candidates": matches,
        });

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&output)?,
            error: None,
        })
    }
}

pub struct OneBotSendToTool {
    security: Arc<SecurityPolicy>,
    api: Option<OneBotApiClient>,
}

impl OneBotSendToTool {
    pub fn new(config: Arc<Config>, security: Arc<SecurityPolicy>) -> Self {
        Self {
            security,
            api: config
                .channels_config
                .onebot_v11
                .as_ref()
                .cloned()
                .map(OneBotApiClient::new),
        }
    }
}

#[async_trait]
impl Tool for OneBotSendToTool {
    fn name(&self) -> &str {
        "onebot_send_to"
    }

    fn description(&self) -> &str {
        "Send QQ message/files to a specific target via OneBot v11. target 支持 private:123 / group:456 / QQ号 / 昵称关键词；支持自动检索并在歧义时返回候选。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "目标：private:QQ号、group:群号、纯 QQ 号、或昵称/备注关键词"
                },
                "message": {
                    "type": "string",
                    "description": "要发送的文本消息，可空（若只发附件）"
                },
                "attachments": {
                    "type": "array",
                    "description": "附件列表（可选）",
                    "items": {
                        "type": "object",
                        "properties": {
                            "kind": {
                                "type": "string",
                                "enum": ["image", "document", "video", "audio", "voice"]
                            },
                            "target": {
                                "type": "string",
                                "description": "附件路径或 URL"
                            }
                        },
                        "required": ["kind", "target"]
                    }
                },
                "scope": {
                    "type": "string",
                    "description": "当 target 不是明确 ID 前缀时的检索范围：auto/friend/group/group_member",
                    "enum": ["auto", "friend", "group", "group_member"],
                    "default": "auto"
                },
                "group_id": {
                    "type": "string",
                    "description": "当 scope=group_member 时可指定群号"
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "仅解析目标并预览发送内容，不真正发送",
                    "default": false
                }
            },
            "required": ["target"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let Some(api) = self.api.as_ref() else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("未配置 onebot_v11，无法发送 QQ 消息。".to_string()),
            });
        };

        let target_raw = args
            .get("target")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow!("缺少 target 参数"))?;

        let message = args
            .get("message")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();

        let attachments = parse_attachments(args.get("attachments"))?;
        if message.is_empty() && attachments.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("message 与 attachments 不能同时为空".to_string()),
            });
        }

        let scope = SearchScope::parse(args.get("scope").and_then(Value::as_str))?;
        let group_id = args.get("group_id").and_then(Value::as_str);
        let dry_run = args
            .get("dry_run")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let resolution = resolve_send_target(api, target_raw, scope, group_id).await?;
        let final_message = compose_outgoing_message(message, &attachments);

        if !dry_run {
            if !self.security.can_act() {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Action blocked: autonomy is read-only".into()),
                });
            }

            if !self.security.record_action() {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Action blocked: rate limit exceeded".into()),
                });
            }

            let channel = OneBotV11Channel::new(api.config.clone());
            channel
                .send(&SendMessage::new(
                    final_message.clone(),
                    &resolution.reply_target,
                ))
                .await
                .with_context(|| format!("发送到 {} 失败", resolution.reply_target))?;
        }

        let output = json!({
            "sent": !dry_run,
            "dry_run": dry_run,
            "resolved_target": resolution.reply_target,
            "resolved_by": resolution.resolved_by,
            "candidate": resolution.candidate,
            "message": final_message,
            "attachments": attachments.iter().map(|att| {
                json!({
                    "kind": att.kind.marker_tag().to_ascii_lowercase(),
                    "target": att.target,
                })
            }).collect::<Vec<_>>()
        });

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&output)?,
            error: None,
        })
    }
}

pub struct OneBotFriendRequestApproveTool {
    security: Arc<SecurityPolicy>,
    api: Option<OneBotApiClient>,
}

impl OneBotFriendRequestApproveTool {
    pub fn new(config: Arc<Config>, security: Arc<SecurityPolicy>) -> Self {
        Self {
            security,
            api: config
                .channels_config
                .onebot_v11
                .as_ref()
                .cloned()
                .map(OneBotApiClient::new),
        }
    }
}

#[async_trait]
impl Tool for OneBotFriendRequestApproveTool {
    fn name(&self) -> &str {
        "onebot_friend_request_approve"
    }

    fn description(&self) -> &str {
        "Approve or reject QQ friend requests via OneBot v11 (`set_friend_add_request`)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "flag": {
                    "type": "string",
                    "description": "好友申请 flag（来自 OneBot request 事件）"
                },
                "approve": {
                    "type": "boolean",
                    "description": "是否同意好友申请，默认 true",
                    "default": true
                },
                "remark": {
                    "type": "string",
                    "description": "通过后备注（可选）"
                }
            },
            "required": ["flag"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let Some(api) = self.api.as_ref() else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("未配置 onebot_v11，无法处理好友申请。".to_string()),
            });
        };

        let flag = args
            .get("flag")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("缺少 flag 参数"))?;

        let approve = args.get("approve").and_then(Value::as_bool).unwrap_or(true);
        let remark = args
            .get("remark")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if !self.security.can_act() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: autonomy is read-only".into()),
            });
        }

        if !self.security.record_action() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: rate limit exceeded".into()),
            });
        }

        let mut payload = json!({
            "flag": flag,
            "approve": approve,
        });
        if let Some(remark) = remark {
            payload["remark"] = json!(remark);
        }

        let resp = api.call_api("set_friend_add_request", payload).await?;
        let output = json!({
            "ok": true,
            "flag": flag,
            "approve": approve,
            "api_response": resp,
        });

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&output)?,
            error: None,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct SendTargetResolution {
    reply_target: String,
    resolved_by: String,
    candidate: Option<ContactCandidate>,
}

async fn resolve_send_target(
    api: &OneBotApiClient,
    target_raw: &str,
    scope: SearchScope,
    group_id: Option<&str>,
) -> anyhow::Result<SendTargetResolution> {
    if let Some(explicit) = normalize_explicit_target(target_raw) {
        return Ok(SendTargetResolution {
            reply_target: explicit,
            resolved_by: "explicit_target".to_string(),
            candidate: None,
        });
    }

    let trimmed = target_raw.trim();
    let is_numeric_id = trimmed.chars().all(|ch| ch.is_ascii_digit());
    if is_numeric_id {
        if scope == SearchScope::Group {
            return Ok(SendTargetResolution {
                reply_target: format!("group:{trimmed}"),
                resolved_by: "numeric_group_id".to_string(),
                candidate: None,
            });
        }

        if scope == SearchScope::Friend || scope == SearchScope::GroupMember {
            return Ok(SendTargetResolution {
                reply_target: format!("private:{trimmed}"),
                resolved_by: "numeric_user_id".to_string(),
                candidate: None,
            });
        }
    }

    let matches = search_candidates(
        api,
        trimmed,
        scope,
        group_id,
        parse_limit(Some(DEFAULT_CANDIDATE_LIMIT as u64)),
    )
    .await?;

    if matches.is_empty() {
        if is_numeric_id {
            return Ok(SendTargetResolution {
                reply_target: format!("private:{trimmed}"),
                resolved_by: "numeric_user_id_fallback".to_string(),
                candidate: None,
            });
        }

        bail!("未找到匹配对象：{trimmed}");
    }

    if matches.len() > 1 {
        let hint = matches
            .iter()
            .take(8)
            .map(ContactCandidate::display_brief)
            .collect::<Vec<_>>()
            .join("\n- ");
        bail!(
            "目标 `{trimmed}` 匹配到多个对象，请给出更具体的 QQ 号/群号或使用更窄 scope。\n- {hint}"
        );
    }

    let candidate = matches
        .into_iter()
        .next()
        .expect("non-empty after len check");
    Ok(SendTargetResolution {
        reply_target: candidate.reply_target.clone(),
        resolved_by: "contact_search".to_string(),
        candidate: Some(candidate),
    })
}

async fn search_candidates(
    api: &OneBotApiClient,
    query: &str,
    scope: SearchScope,
    group_id: Option<&str>,
    limit: usize,
) -> anyhow::Result<Vec<ContactCandidate>> {
    let query = query.trim();
    if query.is_empty() {
        bail!("query 不能为空");
    }

    let mut candidates = Vec::new();

    match scope {
        SearchScope::Auto => {
            candidates.extend(api.get_friend_candidates().await?);
            candidates.extend(api.get_group_candidates().await?);
            if let Some(gid) = group_id.map(str::trim).filter(|v| !v.is_empty()) {
                candidates.extend(api.get_group_member_candidates(gid).await?);
            }
        }
        SearchScope::Friend => candidates.extend(api.get_friend_candidates().await?),
        SearchScope::Group => candidates.extend(api.get_group_candidates().await?),
        SearchScope::GroupMember => {
            let gid = group_id
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| anyhow!("scope=group_member 时必须提供 group_id"))?;
            candidates.extend(api.get_group_member_candidates(gid).await?);
        }
    }

    let mut scored = Vec::new();
    for mut candidate in candidates {
        if let Some((score, matched_by)) = match_candidate(query, &candidate) {
            candidate.score = score;
            candidate.matched_by = matched_by;
            scored.push(candidate);
        }
    }

    if scored.is_empty() {
        return Ok(Vec::new());
    }

    let mut dedup: HashMap<String, ContactCandidate> = HashMap::new();
    for candidate in scored {
        match dedup.get(&candidate.reply_target) {
            Some(existing) if existing.score >= candidate.score => {}
            _ => {
                dedup.insert(candidate.reply_target.clone(), candidate);
            }
        }
    }

    let mut out: Vec<ContactCandidate> = dedup.into_values().collect();
    out.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.display_name.cmp(&b.display_name))
            .then_with(|| a.id.cmp(&b.id))
    });
    out.truncate(limit);
    Ok(out)
}

fn parse_friend_candidate(item: &Value) -> Option<ContactCandidate> {
    let user_id = value_to_id(item.get("user_id")?)?;
    let nickname = item
        .get("nickname")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let remark = item
        .get("remark")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty());

    let display_name = if !nickname.is_empty() {
        nickname.to_string()
    } else if let Some(remark) = remark {
        remark.to_string()
    } else {
        user_id.clone()
    };

    let alias = remark
        .filter(|v| *v != display_name)
        .map(ToString::to_string);

    Some(ContactCandidate::from_raw(
        "friend",
        user_id.clone(),
        display_name,
        alias,
        format!("private:{user_id}"),
        None,
        None,
    ))
}

fn parse_group_candidate(item: &Value) -> Option<ContactCandidate> {
    let group_id = value_to_id(item.get("group_id")?)?;
    let group_name = item
        .get("group_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string);

    let display_name = group_name
        .clone()
        .unwrap_or_else(|| format!("群 {group_id}"));

    Some(ContactCandidate::from_raw(
        "group",
        group_id.clone(),
        display_name,
        None,
        format!("group:{group_id}"),
        Some(group_id),
        group_name,
    ))
}

fn parse_group_member_candidate(item: &Value, group_id: &str) -> Option<ContactCandidate> {
    let user_id = value_to_id(item.get("user_id")?)?;
    let card = item
        .get("card")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let nickname = item
        .get("nickname")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty());

    let display_name = card
        .or(nickname)
        .map(ToString::to_string)
        .unwrap_or_else(|| user_id.clone());
    let alias = [card, nickname]
        .into_iter()
        .flatten()
        .find(|name| *name != display_name)
        .map(ToString::to_string);

    Some(ContactCandidate::from_raw(
        "group_member",
        user_id.clone(),
        display_name,
        alias,
        format!("private:{user_id}"),
        Some(group_id.to_string()),
        None,
    ))
}

fn extract_data_array(value: &Value) -> Vec<Value> {
    value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn match_candidate(query: &str, candidate: &ContactCandidate) -> Option<(u8, String)> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }

    let q_lower = q.to_ascii_lowercase();

    let id_lower = candidate.id.to_ascii_lowercase();
    if q_lower == id_lower {
        return Some((100, "id_exact".to_string()));
    }

    let reply_lower = candidate.reply_target.to_ascii_lowercase();
    if q_lower == reply_lower {
        return Some((100, "reply_target_exact".to_string()));
    }

    let mut names = Vec::new();
    names.push(candidate.display_name.as_str());
    if let Some(alias) = candidate.alias.as_deref() {
        names.push(alias);
    }
    if let Some(group_name) = candidate.group_name.as_deref() {
        names.push(group_name);
    }

    if names
        .iter()
        .any(|name| !name.trim().is_empty() && name.eq_ignore_ascii_case(q))
    {
        return Some((95, "name_exact".to_string()));
    }

    if names
        .iter()
        .any(|name| name.to_ascii_lowercase().contains(&q_lower))
    {
        return Some((82, "name_contains".to_string()));
    }

    if id_lower.contains(&q_lower) {
        return Some((76, "id_contains".to_string()));
    }

    None
}

fn parse_limit(value: Option<u64>) -> usize {
    value
        .unwrap_or(DEFAULT_CANDIDATE_LIMIT as u64)
        .clamp(1, MAX_CANDIDATE_LIMIT as u64) as usize
}

fn parse_attachments(raw: Option<&Value>) -> anyhow::Result<Vec<OneBotAttachmentArg>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };

    let arr = raw
        .as_array()
        .ok_or_else(|| anyhow!("attachments 必须是数组"))?;

    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let kind = item
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("attachments[].kind 缺失"))?;
        let target = item
            .get("target")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow!("attachments[].target 缺失或为空"))?;

        out.push(OneBotAttachmentArg {
            kind: OneBotAttachmentKind::parse(kind)?,
            target: target.to_string(),
        });
    }

    Ok(out)
}

fn compose_outgoing_message(text: &str, attachments: &[OneBotAttachmentArg]) -> String {
    let mut lines: Vec<String> = Vec::new();
    if !text.trim().is_empty() {
        lines.push(text.trim().to_string());
    }

    for attachment in attachments {
        lines.push(format!(
            "[{}:{}]",
            attachment.kind.marker_tag(),
            attachment.target
        ));
    }

    lines.join("\n")
}

fn normalize_explicit_target(raw: &str) -> Option<String> {
    let value = raw.trim();
    if let Some(group_id) = value.strip_prefix("group:") {
        let gid = group_id.trim();
        if !gid.is_empty() {
            return Some(format!("group:{gid}"));
        }
        return None;
    }

    if let Some(user_id) = value
        .strip_prefix("private:")
        .or_else(|| value.strip_prefix("user:"))
    {
        let uid = user_id.trim();
        if !uid.is_empty() {
            return Some(format!("private:{uid}"));
        }
        return None;
    }

    None
}

fn value_to_id(value: &Value) -> Option<String> {
    value
        .as_i64()
        .map(|v| v.to_string())
        .or_else(|| value.as_u64().map(|v| v.to_string()))
        .or_else(|| {
            value
                .as_str()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToString::to_string)
        })
}

fn numeric_or_string(value: &str) -> Value {
    value
        .trim()
        .parse::<i64>()
        .map(Value::from)
        .unwrap_or_else(|_| Value::from(value.trim().to_string()))
}

fn format_scope(scope: SearchScope) -> &'static str {
    match scope {
        SearchScope::Auto => "auto",
        SearchScope::Friend => "friend",
        SearchScope::Group => "group",
        SearchScope::GroupMember => "group_member",
    }
}

fn truncate_for_log(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let mut out: String = value.chars().take(max_chars).collect();
    out.push_str(" …");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_explicit_target_works() {
        assert_eq!(
            normalize_explicit_target("private:12345").as_deref(),
            Some("private:12345")
        );
        assert_eq!(
            normalize_explicit_target("user:12345").as_deref(),
            Some("private:12345")
        );
        assert_eq!(
            normalize_explicit_target("group:67890").as_deref(),
            Some("group:67890")
        );
        assert!(normalize_explicit_target("alice").is_none());
    }

    #[test]
    fn compose_outgoing_message_appends_markers() {
        let msg = compose_outgoing_message(
            "你好",
            &[OneBotAttachmentArg {
                kind: OneBotAttachmentKind::Document,
                target: "./report.pdf".to_string(),
            }],
        );
        assert!(msg.contains("你好"));
        assert!(msg.contains("[DOCUMENT:./report.pdf]"));
    }

    #[test]
    fn match_candidate_prefers_exact_id() {
        let candidate = ContactCandidate::from_raw(
            "friend",
            "12345".to_string(),
            "Alice".to_string(),
            Some("A".to_string()),
            "private:12345".to_string(),
            None,
            None,
        );

        let (score, matched_by) = match_candidate("12345", &candidate).expect("should match");
        assert_eq!(score, 100);
        assert_eq!(matched_by, "id_exact");
    }

    #[test]
    fn parse_friend_candidate_uses_nickname() {
        let item = json!({
            "user_id": 10001,
            "nickname": "张三",
            "remark": "老张"
        });
        let c = parse_friend_candidate(&item).expect("candidate");
        assert_eq!(c.reply_target, "private:10001");
        assert_eq!(c.display_name, "张三");
    }

    #[test]
    fn parse_group_candidate_works() {
        let item = json!({
            "group_id": 20002,
            "group_name": "研发群"
        });
        let c = parse_group_candidate(&item).expect("candidate");
        assert_eq!(c.reply_target, "group:20002");
        assert_eq!(c.group_name.as_deref(), Some("研发群"));
    }

    #[test]
    fn parse_attachments_validates_shape() {
        let value = json!([
            {"kind": "image", "target": "https://example.com/a.png"},
            {"kind": "document", "target": "./a.pdf"}
        ]);

        let parsed = parse_attachments(Some(&value)).expect("parse attachments");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].kind.marker_tag(), "IMAGE");
        assert_eq!(parsed[1].kind.marker_tag(), "DOCUMENT");
    }
}
