//! Web UI module — serves an embedded single-page dashboard and chat interface.
//!
//! The HTML is compiled into the binary via `include_str!`, so no external
//! files need to be deployed alongside the executable.

use crate::gateway::AppState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
};
use std::collections::HashSet;

/// Embedded HTML for the Web UI dashboard.
const INDEX_HTML: &str = include_str!("index.html");
const STYLE_CSS: &str = include_str!("style.css");
const APP_JS: &str = include_str!("app.js");

/// GET / — serve the Web UI HTML page
pub async fn handle_index() -> impl IntoResponse {
    Html(INDEX_HTML)
}

/// GET /style.css
pub async fn handle_style() -> impl IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "text/css")], STYLE_CSS)
}

/// GET /app.js
pub async fn handle_app() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        APP_JS,
    )
}

/// GET /api/status — returns system status JSON consumed by the Web UI.
pub async fn handle_api_status(State(state): State<AppState>) -> impl IntoResponse {
    let health = crate::health::snapshot();
    let config = state.config.lock();

    let channels = serde_json::json!({
        "cli": true,
        "telegram": config.channels_config.telegram.is_some(),
        "discord": config.channels_config.discord.is_some(),
        "slack": config.channels_config.slack.is_some(),
        "webhook": config.channels_config.webhook.is_some(),
        "whatsapp": config.channels_config.whatsapp.is_some(),
    });

    let configured_model = config.default_model.as_deref().unwrap_or("");
    let runtime_model = state.model.as_str();
    let model_needs_restart =
        !configured_model.trim().is_empty() && configured_model != runtime_model;

    let body = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "pid": health.pid,
        "uptime_seconds": health.uptime_seconds,
        "provider": config.default_provider.as_deref().unwrap_or("openrouter"),
        "model": runtime_model,
        "configured_model": configured_model,
        "model_needs_restart": model_needs_restart,
        "temperature": state.temperature,
        "memory_backend": config.memory.backend,
        "auto_save": state.auto_save,
        "autonomy_level": format!("{:?}", config.autonomy.level),
        "runtime": config.runtime.kind,
        "observability": config.observability.backend,
        "paired": state.pairing.is_paired(),
        "tunnel": config.tunnel.provider,
        "channels": channels,
    });

    Json(body)
}

/// GET /api/config — returns full sanitized configuration for the Web UI.
/// Secrets are masked (only `has_*` booleans exposed), counts replace lists.
pub async fn handle_api_config(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.lock();

    let body = serde_json::json!({
        "core": {
            "provider": config.default_provider.as_deref().unwrap_or("openrouter"),
            "model": config.default_model.as_deref().unwrap_or(""),
            "temperature": config.default_temperature,
            "has_api_key": config.api_key.is_some(),
            "api_url": config.api_url.as_deref().unwrap_or(""),
        },
        "agent": {
            "compact_context": config.agent.compact_context,
            "max_tool_iterations": config.agent.max_tool_iterations,
            "max_history_messages": config.agent.max_history_messages,
            "parallel_tools": config.agent.parallel_tools,
            "tool_dispatcher": &config.agent.tool_dispatcher,
        },
        "gateway": {
            "port": config.gateway.port,
            "host": &config.gateway.host,
            "require_pairing": config.gateway.require_pairing,
            "allow_public_bind": config.gateway.allow_public_bind,
            "paired_tokens_count": config.gateway.paired_tokens.len(),
            "pair_rate_limit": config.gateway.pair_rate_limit_per_minute,
            "webhook_rate_limit": config.gateway.webhook_rate_limit_per_minute,
            "trust_forwarded_headers": config.gateway.trust_forwarded_headers,
        },
        "autonomy": {
            "level": format!("{:?}", config.autonomy.level),
            "workspace_only": config.autonomy.workspace_only,
            "allowed_commands": &config.autonomy.allowed_commands,
            "forbidden_paths_count": config.autonomy.forbidden_paths.len(),
            "max_actions_per_hour": config.autonomy.max_actions_per_hour,
            "max_cost_per_day_cents": config.autonomy.max_cost_per_day_cents,
            "require_approval_for_medium_risk": config.autonomy.require_approval_for_medium_risk,
            "block_high_risk_commands": config.autonomy.block_high_risk_commands,
            "auto_approve": &config.autonomy.auto_approve,
            "always_ask": &config.autonomy.always_ask,
        },
        "memory": {
            "backend": &config.memory.backend,
            "auto_save": config.memory.auto_save,
            "hygiene_enabled": config.memory.hygiene_enabled,
            "archive_after_days": config.memory.archive_after_days,
            "purge_after_days": config.memory.purge_after_days,
            "conversation_retention_days": config.memory.conversation_retention_days,
            "embedding_provider": &config.memory.embedding_provider,
            "embedding_model": &config.memory.embedding_model,
            "embedding_dimensions": config.memory.embedding_dimensions,
            "response_cache_enabled": config.memory.response_cache_enabled,
            "response_cache_ttl_minutes": config.memory.response_cache_ttl_minutes,
            "snapshot_enabled": config.memory.snapshot_enabled,
            "auto_hydrate": config.memory.auto_hydrate,
        },
        "runtime": {
            "kind": &config.runtime.kind,
            "docker_image": &config.runtime.docker.image,
            "docker_network": &config.runtime.docker.network,
            "docker_memory_limit_mb": config.runtime.docker.memory_limit_mb,
            "docker_cpu_limit": config.runtime.docker.cpu_limit,
            "docker_read_only_rootfs": config.runtime.docker.read_only_rootfs,
        },
        "reliability": {
            "provider_retries": config.reliability.provider_retries,
            "provider_backoff_ms": config.reliability.provider_backoff_ms,
            "fallback_providers": &config.reliability.fallback_providers,
            "extra_api_keys_count": config.reliability.api_keys.len(),
            "channel_initial_backoff_secs": config.reliability.channel_initial_backoff_secs,
            "channel_max_backoff_secs": config.reliability.channel_max_backoff_secs,
            "scheduler_poll_secs": config.reliability.scheduler_poll_secs,
            "scheduler_retries": config.reliability.scheduler_retries,
        },
        "scheduler": {
            "enabled": config.scheduler.enabled,
            "max_tasks": config.scheduler.max_tasks,
            "max_concurrent": config.scheduler.max_concurrent,
        },
        "heartbeat": {
            "enabled": config.heartbeat.enabled,
            "interval_minutes": config.heartbeat.interval_minutes,
        },
        "cron": {
            "enabled": config.cron.enabled,
            "max_run_history": config.cron.max_run_history,
        },
        "tunnel": {
            "provider": &config.tunnel.provider,
        },
        "browser": {
            "enabled": config.browser.enabled,
            "backend": &config.browser.backend,
            "allowed_domains": &config.browser.allowed_domains,
            "native_headless": config.browser.native_headless,
        },
        "http_request": {
            "enabled": config.http_request.enabled,
            "allowed_domains": &config.http_request.allowed_domains,
            "max_response_size": config.http_request.max_response_size,
            "timeout_secs": config.http_request.timeout_secs,
        },
        "cost": {
            "enabled": config.cost.enabled,
            "daily_limit_usd": config.cost.daily_limit_usd,
            "monthly_limit_usd": config.cost.monthly_limit_usd,
            "warn_at_percent": config.cost.warn_at_percent,
            "allow_override": config.cost.allow_override,
        },
        "observability": {
            "backend": &config.observability.backend,
            "otel_endpoint": config.observability.otel_endpoint.as_deref().unwrap_or(""),
            "otel_service_name": config.observability.otel_service_name.as_deref().unwrap_or("zeroclaw"),
        },
        "composio": {
            "enabled": config.composio.enabled,
            "has_api_key": config.composio.api_key.is_some(),
        },
        "secrets": {
            "encrypt": config.secrets.encrypt,
        },
        "identity": {
            "format": &config.identity.format,
        },
        "hardware": {
            "enabled": config.hardware.enabled,
            "transport": format!("{}", config.hardware.transport),
        },
        "peripherals": {
            "enabled": config.peripherals.enabled,
            "boards_count": config.peripherals.boards.len(),
        },
        "model_routes_count": config.model_routes.len(),
        "delegate_agents_count": config.agents.len(),
        "channels": {
            "cli": config.channels_config.cli,
            "telegram": config.channels_config.telegram.as_ref().map(|t| serde_json::json!({
                "allowed_users": t.allowed_users.len(),
            })),
            "discord": config.channels_config.discord.as_ref().map(|d| serde_json::json!({
                "guild_id": d.guild_id.is_some(),
                "allowed_users": d.allowed_users.len(),
                "listen_to_bots": d.listen_to_bots,
                "mention_only": d.mention_only,
            })),
            "slack": config.channels_config.slack.as_ref().map(|s| serde_json::json!({
                "channel_id": s.channel_id.is_some(),
                "allowed_users": s.allowed_users.len(),
            })),
            "mattermost": config.channels_config.mattermost.as_ref().map(|m| serde_json::json!({
                "url": &m.url,
                "allowed_users": m.allowed_users.len(),
            })),
            "webhook": config.channels_config.webhook.as_ref().map(|w| serde_json::json!({
                "port": w.port,
                "has_secret": w.secret.is_some(),
            })),
            "imessage": config.channels_config.imessage.as_ref().map(|i| serde_json::json!({
                "contacts": i.allowed_contacts.len(),
            })),
            "matrix": config.channels_config.matrix.as_ref().map(|m| serde_json::json!({
                "homeserver": &m.homeserver,
                "room_id": &m.room_id,
                "allowed_users": m.allowed_users.len(),
            })),
            "signal": config.channels_config.signal.as_ref().map(|s| serde_json::json!({
                "account": &s.account,
                "allowed_from": s.allowed_from.len(),
            })),
            "whatsapp": config.channels_config.whatsapp.as_ref().map(|w| serde_json::json!({
                "phone_number_id": &w.phone_number_id,
                "allowed_numbers": w.allowed_numbers.len(),
            })),
            "email": config.channels_config.email.is_some(),
            "irc": config.channels_config.irc.as_ref().map(|i| serde_json::json!({
                "server": &i.server,
                "nickname": &i.nickname,
                "channels_count": i.channels.len(),
            })),
            "lark": config.channels_config.lark.as_ref().map(|l| serde_json::json!({
                "use_feishu": l.use_feishu,
                "allowed_users": l.allowed_users.len(),
                "receive_mode": format!("{:?}", l.receive_mode),
            })),
            "dingtalk": config.channels_config.dingtalk.as_ref().map(|d| serde_json::json!({
                "allowed_users": d.allowed_users.len(),
            })),
            "qq": config.channels_config.qq.as_ref().map(|q| serde_json::json!({
                "allowed_users": q.allowed_users.len(),
            })),
        },
    });

    Json(body)
}

/// GET /api/config/raw — returns the raw Config struct serialized to JSON.
pub async fn handle_api_config_raw_get(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.lock();
    Json(config.clone())
}

/// POST /api/config/raw — deserializes a full Config struct and saves it.
pub async fn handle_api_config_raw_post(
    State(state): State<AppState>,
    body: Result<Json<crate::config::Config>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let Json(new_config) = match body {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    let mut config_guard = state.config.lock();
    let workspace_dir = config_guard.workspace_dir.clone();
    let config_path = config_guard.config_path.clone();
    let previous_api_key = config_guard.api_key.clone();

    *config_guard = new_config;
    config_guard.workspace_dir = workspace_dir;
    config_guard.config_path = config_path;

    // Defensive merge for legacy/front-end clients:
    // treat empty-string api_key as "unchanged" instead of erasing secrets.
    // (Explicit null can still be used to clear the key.)
    if config_guard
        .api_key
        .as_deref()
        .is_some_and(|s| s.trim().is_empty())
    {
        config_guard.api_key = previous_api_key;
    }

    if let Err(e) = config_guard.save() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        );
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "ok", "requires_restart": true })),
    )
}

#[derive(serde::Deserialize, Default)]
pub struct DiscoverModelsReq {
    pub provider: Option<String>,
    pub api_url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(serde::Serialize)]
pub struct DiscoverModelsResp {
    pub provider: String,
    pub endpoint: String,
    pub models: Vec<String>,
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn detect_provider_family(provider: &str) -> &'static str {
    let normalized = provider.trim().to_lowercase();
    if normalized == "ollama" {
        "ollama"
    } else if normalized == "anthropic" || normalized.starts_with("anthropic-custom:") {
        "anthropic"
    } else if matches!(normalized.as_str(), "gemini" | "google" | "google-gemini") {
        "gemini"
    } else {
        "openai-compatible"
    }
}

fn resolve_default_base_url(provider: &str) -> Option<String> {
    let normalized = provider.trim().to_lowercase();
    match normalized.as_str() {
        "openrouter" => Some("https://openrouter.ai/api/v1".to_string()),
        "openai" => Some("https://api.openai.com/v1".to_string()),
        "anthropic" => Some("https://api.anthropic.com".to_string()),
        "gemini" | "google" | "google-gemini" => {
            Some("https://generativelanguage.googleapis.com/v1beta".to_string())
        }
        "ollama" => Some("http://127.0.0.1:11434".to_string()),
        "compatible" => None,
        _ if crate::providers::is_glm_alias(&normalized) => {
            Some("https://api.z.ai/api/paas/v4".to_string())
        }
        _ if crate::providers::is_minimax_alias(&normalized) => {
            Some("https://api.minimax.io/v1".to_string())
        }
        _ if crate::providers::is_moonshot_alias(&normalized) => {
            Some("https://api.moonshot.cn/v1".to_string())
        }
        _ if crate::providers::is_qwen_alias(&normalized) => {
            Some("https://dashscope.aliyuncs.com/compatible-mode/v1".to_string())
        }
        _ if crate::providers::is_zai_alias(&normalized) => {
            Some("https://api.z.ai/api/coding/paas/v4".to_string())
        }
        _ => None,
    }
}

fn resolve_discovery_target(
    provider: &str,
    api_url: Option<&str>,
) -> anyhow::Result<(String, String)> {
    let provider_trimmed = provider.trim();
    if provider_trimmed.is_empty() {
        anyhow::bail!("provider 不能为空");
    }

    let explicit_url = api_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    if let Some(url) = explicit_url {
        return Ok((provider_trimmed.to_string(), url));
    }

    if let Some(raw) = provider_trimmed.strip_prefix("custom:") {
        let url = raw.trim();
        if url.is_empty() {
            anyhow::bail!("自定义 provider 缺少 URL，请先填写 API Base URL");
        }
        return Ok((provider_trimmed.to_string(), url.to_string()));
    }

    if let Some(raw) = provider_trimmed.strip_prefix("anthropic-custom:") {
        let url = raw.trim();
        if url.is_empty() {
            anyhow::bail!("anthropic-custom provider 缺少 URL");
        }
        return Ok((provider_trimmed.to_string(), url.to_string()));
    }

    let Some(base_url) = resolve_default_base_url(provider_trimmed) else {
        anyhow::bail!(
            "不支持自动推断 provider `{provider_trimmed}` 的模型列表地址，请手动填写 API URL 后重试"
        );
    };
    Ok((provider_trimmed.to_string(), base_url))
}

fn ordered_unique(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn normalize_model_names(mut models: Vec<String>) -> Vec<String> {
    models = ordered_unique(models);
    models.sort_unstable();
    models
}

fn build_openai_model_endpoints(base_url: &str) -> Vec<String> {
    let normalized = base_url.trim().trim_end_matches('/');
    if normalized.is_empty() {
        return Vec::new();
    }

    let mut candidates = Vec::new();

    if normalized.ends_with("/models") {
        candidates.push(normalized.to_string());
    } else if let Some(prefix) = normalized.strip_suffix("/chat/completions") {
        let prefix = prefix.trim_end_matches('/');
        if !prefix.is_empty() {
            candidates.push(format!("{prefix}/models"));
            if !prefix.ends_with("/v1") {
                candidates.push(format!("{prefix}/v1/models"));
            }
        }
    } else {
        candidates.push(format!("{normalized}/models"));
        if !normalized.ends_with("/v1") {
            candidates.push(format!("{normalized}/v1/models"));
        }
    }

    ordered_unique(candidates)
        .into_iter()
        .filter(|candidate| reqwest::Url::parse(candidate).is_ok())
        .collect()
}

fn parse_openai_style_models(value: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();

    for key in ["data", "models"] {
        if let Some(items) = value.get(key).and_then(serde_json::Value::as_array) {
            for item in items {
                if let Some(id) = item.get("id").and_then(serde_json::Value::as_str) {
                    out.push(id.to_string());
                } else if let Some(name) = item.get("name").and_then(serde_json::Value::as_str) {
                    out.push(name.to_string());
                } else if let Some(model) = item.get("model").and_then(serde_json::Value::as_str) {
                    out.push(model.to_string());
                }
            }
        }
    }

    out
}

async fn discover_openai_compatible_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> anyhow::Result<(String, Vec<String>)> {
    let endpoints = build_openai_model_endpoints(base_url);
    if endpoints.is_empty() {
        anyhow::bail!("无法从 URL `{base_url}` 推断模型列表接口");
    }

    let mut attempts = Vec::new();
    for endpoint in endpoints {
        let mut req = client.get(&endpoint);
        if let Some(key) = api_key.map(str::trim).filter(|k| !k.is_empty()) {
            req = req.header("Authorization", format!("Bearer {key}"));
        }

        let response = match req.send().await {
            Ok(resp) => resp,
            Err(err) => {
                attempts.push(format!("{endpoint}: {err}"));
                continue;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let sanitized = crate::providers::sanitize_api_error(&body);
            attempts.push(format!("{endpoint}: {status} {sanitized}"));
            continue;
        }

        let value: serde_json::Value = match response.json().await {
            Ok(json) => json,
            Err(err) => {
                attempts.push(format!("{endpoint}: JSON 解析失败 ({err})"));
                continue;
            }
        };

        let models = normalize_model_names(parse_openai_style_models(&value));
        if !models.is_empty() {
            return Ok((endpoint, models));
        }

        attempts.push(format!("{endpoint}: 返回成功但未找到 data[].id"));
    }

    anyhow::bail!("模型发现失败：{}", attempts.join(" | "));
}

async fn discover_ollama_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> anyhow::Result<(String, Vec<String>)> {
    let normalized = base_url.trim().trim_end_matches('/');
    let endpoint = if normalized.ends_with("/api/tags") {
        normalized.to_string()
    } else if normalized.ends_with("/api") {
        format!("{normalized}/tags")
    } else {
        format!("{normalized}/api/tags")
    };
    let mut req = client.get(&endpoint);
    if let Some(key) = api_key.map(str::trim).filter(|k| !k.is_empty()) {
        req = req.header("Authorization", format!("Bearer {key}"));
    }

    let response = req.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let sanitized = crate::providers::sanitize_api_error(&body);
        anyhow::bail!("{status} {sanitized}");
    }

    let value: serde_json::Value = response.json().await?;
    let models = value
        .get("models")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.get("name")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let models = normalize_model_names(models);
    if models.is_empty() {
        anyhow::bail!("Ollama 返回成功，但未包含 models[].name");
    }

    Ok((endpoint, models))
}

async fn discover_anthropic_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> anyhow::Result<(String, Vec<String>)> {
    let key = api_key
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Anthropic 需要 API key 才能拉取模型列表"))?;

    let normalized = base_url.trim().trim_end_matches('/');
    let endpoint = if normalized.ends_with("/models") {
        normalized.to_string()
    } else if normalized.ends_with("/v1") {
        format!("{normalized}/models")
    } else {
        format!("{normalized}/v1/models")
    };
    let mut req = client
        .get(&endpoint)
        .header("anthropic-version", "2023-06-01");

    if key.starts_with("sk-ant-oat01-") {
        req = req
            .header("Authorization", format!("Bearer {key}"))
            .header("anthropic-beta", "oauth-2025-04-20");
    } else {
        req = req.header("x-api-key", key);
    }

    let response = req.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let sanitized = crate::providers::sanitize_api_error(&body);
        anyhow::bail!("{status} {sanitized}");
    }

    let value: serde_json::Value = response.json().await?;
    let models = normalize_model_names(parse_openai_style_models(&value));
    if models.is_empty() {
        anyhow::bail!("Anthropic 返回成功，但未包含 data[].id");
    }

    Ok((endpoint, models))
}

async fn discover_gemini_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> anyhow::Result<(String, Vec<String>)> {
    let key = api_key
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Gemini 需要 API key 才能拉取模型列表"))?;

    let normalized = base_url.trim().trim_end_matches('/');
    let endpoint = if normalized.ends_with("/models") {
        normalized.to_string()
    } else {
        format!("{normalized}/models")
    };
    let response = client.get(&endpoint).query(&[("key", key)]).send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let sanitized = crate::providers::sanitize_api_error(&body);
        anyhow::bail!("{status} {sanitized}");
    }

    let value: serde_json::Value = response.json().await?;
    let models = value
        .get("models")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("name").and_then(serde_json::Value::as_str))
                .map(|name| name.trim_start_matches("models/").to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let models = normalize_model_names(models);
    if models.is_empty() {
        anyhow::bail!("Gemini 返回成功，但未包含 models[].name");
    }

    Ok((endpoint, models))
}

/// POST /api/models/discover — infer model list from provider + URL.
pub async fn handle_api_models_discover(
    State(state): State<AppState>,
    body: Result<Json<DiscoverModelsReq>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let Json(req) = match body {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    let config = state.config.lock().clone();
    let provider = nonempty(req.provider)
        .or_else(|| config.default_provider.clone())
        .unwrap_or_else(|| "openrouter".to_string());
    let api_url = nonempty(req.api_url).or_else(|| config.api_url.clone());
    let api_key = nonempty(req.api_key).or_else(|| config.api_key.clone());

    let (provider_name, base_url) = match resolve_discovery_target(&provider, api_url.as_deref()) {
        Ok(target) => target,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": err.to_string() })),
            )
        }
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(8))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let result = match detect_provider_family(&provider_name) {
        "ollama" => discover_ollama_models(&client, &base_url, api_key.as_deref()).await,
        "anthropic" => discover_anthropic_models(&client, &base_url, api_key.as_deref()).await,
        "gemini" => discover_gemini_models(&client, &base_url, api_key.as_deref()).await,
        _ => discover_openai_compatible_models(&client, &base_url, api_key.as_deref()).await,
    };

    match result {
        Ok((endpoint, models)) => (
            StatusCode::OK,
            Json(serde_json::json!(DiscoverModelsResp {
                provider: provider_name,
                endpoint,
                models,
            })),
        ),
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": err.to_string() })),
        ),
    }
}

/// Mutate configuration request payload.
#[derive(serde::Deserialize, Debug)]
#[serde(tag = "action", content = "payload")]
pub enum ConfigUpdate {
    Core {
        provider: String,
        model: String,
        api_key: Option<String>,
    },
    ChannelTelegram {
        bot_token: String,
    },
    ChannelDiscord {
        bot_token: String,
        guild_id: Option<String>,
    },
    ChannelQQ {
        app_id: String,
        app_secret: String,
    },
    ChannelWebhook {
        port: u16,
        secret: Option<String>,
    },
    DisableChannel {
        channel: String,
    },
}

/// POST /api/config — Mutates the configuration and saves to disk.
pub async fn handle_api_config_mutate(
    State(state): State<AppState>,
    body: Result<Json<ConfigUpdate>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let Json(req) = match body {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };

    let mut config = state.config.lock();
    match req {
        ConfigUpdate::Core {
            provider,
            model,
            api_key,
        } => {
            config.default_provider = Some(provider);
            if !model.trim().is_empty() {
                config.default_model = Some(model);
            }
            if let Some(key) = api_key {
                if !key.trim().is_empty() {
                    config.api_key = Some(key);
                }
            }
        }
        ConfigUpdate::ChannelTelegram { bot_token } => {
            config.channels_config.telegram = Some(crate::config::TelegramConfig {
                bot_token,
                allowed_users: vec![],
            });
        }
        ConfigUpdate::ChannelDiscord {
            bot_token,
            guild_id,
        } => {
            let gid = guild_id.filter(|s| !s.trim().is_empty());
            config.channels_config.discord = Some(crate::config::DiscordConfig {
                bot_token,
                guild_id: gid,
                allowed_users: vec![],
                listen_to_bots: false,
                mention_only: false,
            });
        }
        ConfigUpdate::ChannelQQ { app_id, app_secret } => {
            config.channels_config.qq = Some(crate::config::schema::QQConfig {
                app_id,
                app_secret,
                allowed_users: vec![],
            });
        }
        ConfigUpdate::ChannelWebhook { port, secret } => {
            let s = secret.filter(|x| !x.trim().is_empty());
            config.channels_config.webhook = Some(crate::config::WebhookConfig { port, secret: s });
        }
        ConfigUpdate::DisableChannel { channel } => match channel.as_str() {
            "telegram" => config.channels_config.telegram = None,
            "discord" => config.channels_config.discord = None,
            "qq" => config.channels_config.qq = None,
            "webhook" => config.channels_config.webhook = None,
            _ => {}
        },
    }
    if let Err(e) = config.save() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        );
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "ok", "requires_restart": true })),
    )
}

/// Chat request body for the Web UI.
#[derive(serde::Deserialize)]
pub struct ChatRequest {
    pub message: String,
}

/// POST /api/chat — simple chat endpoint for the Web UI.
pub async fn handle_api_chat(
    State(state): State<AppState>,
    body: Result<Json<ChatRequest>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let Json(req) = match body {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("WebUI chat JSON parse error: {e}");
            let err = serde_json::json!({
                "error": "无效的请求格式，需要: {\"message\": \"...\"}"
            });
            return (StatusCode::BAD_REQUEST, Json(err));
        }
    };

    if req.message.trim().is_empty() {
        let err = serde_json::json!({ "error": "消息不能为空" });
        return (StatusCode::BAD_REQUEST, Json(err));
    }

    let config = state.config.lock().clone();
    match crate::agent::process_message(config, &req.message).await {
        Ok(response) => {
            let body = serde_json::json!({
                "response": response,
                "model": state.model
            });
            (StatusCode::OK, Json(body))
        }
        Err(e) => {
            let sanitized = crate::providers::sanitize_api_error(&e.to_string());
            tracing::error!("WebUI chat provider error: {sanitized}");
            let err = serde_json::json!({ "error": "AI 请求失败，请稍后再试" });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(err))
        }
    }
}

#[derive(serde::Serialize)]
pub struct IdentityListResp {
    pub active_file: String,
    pub files: Vec<String>,
    pub content: String,
}

/// GET /api/identity — list identities from `identities/` dir and read one.
pub async fn handle_api_identity_get(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let config = state.config.lock();
    let identities_dir = config.workspace_dir.join("identities");
    let _ = std::fs::create_dir_all(&identities_dir);

    // Determine which file is currently active
    let active_file = config.identity.aieos_path.clone().unwrap_or_default();

    // List all files in identities/
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&identities_dir) {
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                if !name.starts_with('.') && entry.path().is_file() {
                    files.push(name);
                }
            }
        }
    }
    files.sort();

    // Read the requested file (or active, or first available)
    let target_file = params
        .get("file")
        .filter(|f| !f.is_empty())
        .cloned()
        .or_else(|| {
            if !active_file.is_empty() {
                Some(active_file.clone())
            } else {
                None
            }
        })
        .or_else(|| files.first().cloned())
        .unwrap_or_default();

    let content = if !target_file.is_empty() {
        std::fs::read_to_string(identities_dir.join(&target_file)).unwrap_or_default()
    } else {
        String::new()
    };

    Json(IdentityListResp {
        active_file,
        files,
        content,
    })
}

#[derive(serde::Deserialize)]
pub struct IdentityUpdateReq {
    pub file: String,
    pub content: String,
    pub set_active: bool,
}

/// POST /api/identity — save to identities/ dir and optionally activate (copy to IDENTITY.md).
pub async fn handle_api_identity_post(
    State(state): State<AppState>,
    body: Result<Json<IdentityUpdateReq>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let Json(req) = match body {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };
    let mut config = state.config.lock();
    let identities_dir = config.workspace_dir.join("identities");
    let _ = std::fs::create_dir_all(&identities_dir);

    let filename = if req.file.trim().is_empty() {
        "default.md"
    } else {
        req.file.trim()
    };
    let path = identities_dir.join(filename);

    // Always write (even empty content means clearing the file)
    if let Err(e) = std::fs::write(&path, &req.content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        );
    }

    if req.set_active {
        // Copy content into IDENTITY.md at workspace root
        let identity_md = config.workspace_dir.join("IDENTITY.md");
        let _ = std::fs::write(&identity_md, &req.content);
        config.identity.aieos_path = Some(filename.to_string());
        if let Err(e) = config.save() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            );
        }
    }
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

#[derive(serde::Serialize)]
pub struct ContextFileItemResp {
    pub id: String,
    pub label: String,
    pub path: String,
    pub exists: bool,
    pub size_bytes: Option<u64>,
    pub modified_unix: Option<u64>,
    pub active_in_prompt: bool,
    pub is_virtual: bool,
}

#[derive(serde::Serialize)]
pub struct ContextFilesResp {
    pub mode: String,
    pub selected: String,
    pub content: String,
    pub files: Vec<ContextFileItemResp>,
    pub notes: Vec<String>,
}

struct ContextFileCandidate {
    id: String,
    label: String,
    path: String,
    abs_path: Option<std::path::PathBuf>,
    inline_content: Option<String>,
    exists: bool,
    size_bytes: Option<u64>,
    modified_unix: Option<u64>,
    active_in_prompt: bool,
    is_virtual: bool,
}

fn file_metadata(path: &std::path::Path) -> (bool, Option<u64>, Option<u64>) {
    let Ok(meta) = std::fs::metadata(path) else {
        return (false, None, None);
    };

    let modified_unix = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());

    (true, Some(meta.len()), modified_unix)
}

fn push_file_candidate(
    out: &mut Vec<ContextFileCandidate>,
    id: impl Into<String>,
    label: impl Into<String>,
    path_display: impl Into<String>,
    abs_path: std::path::PathBuf,
    active_in_prompt: bool,
) {
    let (exists, size_bytes, modified_unix) = file_metadata(&abs_path);
    out.push(ContextFileCandidate {
        id: id.into(),
        label: label.into(),
        path: path_display.into(),
        abs_path: Some(abs_path),
        inline_content: None,
        exists,
        size_bytes,
        modified_unix,
        active_in_prompt,
        is_virtual: false,
    });
}

/// GET /api/context-files — preview prompt-injected identity/context files.
pub async fn handle_api_context_files_get(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let config = state.config.lock().clone();
    let workspace = config.workspace_dir.clone();
    let is_aieos = config.identity.format.trim().eq_ignore_ascii_case("aieos");

    let mut candidates: Vec<ContextFileCandidate> = Vec::new();
    let openclaw_files = [
        "AGENTS.md",
        "SOUL.md",
        "TOOLS.md",
        "IDENTITY.md",
        "USER.md",
        "HEARTBEAT.md",
        "BOOTSTRAP.md",
        "MEMORY.md",
    ];

    for name in openclaw_files {
        push_file_candidate(
            &mut candidates,
            format!("openclaw:{name}"),
            name,
            name,
            workspace.join(name),
            !is_aieos,
        );
    }

    if is_aieos {
        if let Some(path) = config
            .identity
            .aieos_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let abs = if std::path::Path::new(path).is_absolute() {
                std::path::PathBuf::from(path)
            } else {
                workspace.join(path)
            };
            push_file_candidate(
                &mut candidates,
                format!("aieos:path:{path}"),
                "AIEOS 文件",
                path,
                abs,
                true,
            );
        }

        if let Some(inline) = config
            .identity
            .aieos_inline
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            candidates.push(ContextFileCandidate {
                id: "aieos:inline".to_string(),
                label: "AIEOS 内联配置".to_string(),
                path: "[inline] identity.aieos_inline".to_string(),
                abs_path: None,
                inline_content: Some(inline.to_string()),
                exists: true,
                size_bytes: Some(inline.len() as u64),
                modified_unix: None,
                active_in_prompt: true,
                is_virtual: true,
            });
        }
    }

    let requested = params.get("file").map(String::as_str).unwrap_or_default();
    let selected = candidates
        .iter()
        .find(|candidate| candidate.id == requested)
        .map(|candidate| candidate.id.clone())
        .or_else(|| {
            candidates
                .iter()
                .find(|candidate| candidate.active_in_prompt)
                .map(|candidate| candidate.id.clone())
        })
        .or_else(|| candidates.first().map(|candidate| candidate.id.clone()))
        .unwrap_or_default();

    let content = candidates
        .iter()
        .find(|candidate| candidate.id == selected)
        .map(|candidate| {
            if candidate.is_virtual {
                candidate.inline_content.clone().unwrap_or_default()
            } else {
                candidate
                    .abs_path
                    .as_ref()
                    .and_then(|path| std::fs::read_to_string(path).ok())
                    .unwrap_or_default()
            }
        })
        .unwrap_or_default();

    let files = candidates
        .into_iter()
        .map(|candidate| ContextFileItemResp {
            id: candidate.id,
            label: candidate.label,
            path: candidate.path,
            exists: candidate.exists,
            size_bytes: candidate.size_bytes,
            modified_unix: candidate.modified_unix,
            active_in_prompt: candidate.active_in_prompt,
            is_virtual: candidate.is_virtual,
        })
        .collect();

    Json(ContextFilesResp {
        mode: if is_aieos { "aieos" } else { "openclaw" }.to_string(),
        selected,
        content,
        files,
        notes: vec![
            "这里展示的是磁盘上的实时文件内容；点击刷新可立即看到外部修改。".to_string(),
            "WebUI Chat / agent::process_message 会在每次请求前重新读取这些文件。".to_string(),
            "Channels 通道在启动时构建系统提示词，修改 SOUL/IDENTITY 后通常需要重启 daemon/channels 才会全面生效。".to_string(),
        ],
    })
}

/// GET /api/cron — list all cron jobs
pub async fn handle_api_cron_list(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.lock();
    match crate::cron::list_jobs(&config) {
        Ok(jobs) => (StatusCode::OK, Json(serde_json::json!({ "jobs": jobs }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "action", content = "payload")]
pub enum CronUpdate {
    AddShell { expression: String, command: String },
    Toggle { id: String, enabled: bool },
    Delete { id: String },
}

/// POST /api/cron — mutate cron jobs
pub async fn handle_api_cron_mutate(
    State(state): State<AppState>,
    body: Result<Json<CronUpdate>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let Json(req) = match body {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };
    let config = state.config.lock();
    let res = match req {
        CronUpdate::AddShell {
            expression,
            command,
        } => crate::cron::add_job(&config, &expression, &command).map(|_| ()),
        CronUpdate::Toggle { id, enabled } => crate::cron::update_job(
            &config,
            &id,
            crate::cron::CronJobPatch {
                enabled: Some(enabled),
                ..Default::default()
            },
        )
        .map(|_| ()),
        CronUpdate::Delete { id } => crate::cron::remove_job(&config, &id),
    };
    match res {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

#[derive(serde::Deserialize)]
pub struct ServiceActionReq {
    pub action: String,
}

/// POST /api/service — start/stop/install system daemon
pub async fn handle_api_service_mutate(
    State(state): State<AppState>,
    body: Result<Json<ServiceActionReq>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let Json(req) = match body {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
        }
    };
    let cmd = match req.action.as_str() {
        "install" => crate::ServiceCommands::Install,
        "start" => crate::ServiceCommands::Start,
        "stop" => crate::ServiceCommands::Stop,
        "status" => crate::ServiceCommands::Status,
        "uninstall" => crate::ServiceCommands::Uninstall,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid action" })),
            )
        }
    };
    let config = state.config.lock();
    match crate::service::handle_command(&cmd, &config) {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}
