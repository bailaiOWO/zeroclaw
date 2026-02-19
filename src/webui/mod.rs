//! Web UI module — serves an embedded single-page dashboard and chat interface.
//!
//! The HTML is compiled into the binary via `include_str!`, so no external
//! files need to be deployed alongside the executable.

use crate::gateway::AppState;
use axum::{
    extract::{State, Query},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
};

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
    ([(axum::http::header::CONTENT_TYPE, "application/javascript")], APP_JS)
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

    let body = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "pid": health.pid,
        "uptime_seconds": health.uptime_seconds,
        "provider": config.default_provider.as_deref().unwrap_or("openrouter"),
        "model": state.model,
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
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    let mut config_guard = state.config.lock();
    let workspace_dir = config_guard.workspace_dir.clone();
    let config_path = config_guard.config_path.clone();
    
    *config_guard = new_config;
    config_guard.workspace_dir = workspace_dir;
    config_guard.config_path = config_path;

    if let Err(e) = config_guard.save() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })));
    }
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok", "requires_restart": true })))
}

/// Mutate configuration request payload.
#[derive(serde::Deserialize, Debug)]
#[serde(tag = "action", content = "payload")]
pub enum ConfigUpdate {
    Core { provider: String, model: String, api_key: Option<String> },
    ChannelTelegram { bot_token: String },
    ChannelDiscord { bot_token: String, guild_id: Option<String> },
    ChannelQQ { app_id: String, app_secret: String },
    ChannelWebhook { port: u16, secret: Option<String> },
    DisableChannel { channel: String },
}

/// POST /api/config — Mutates the configuration and saves to disk.
pub async fn handle_api_config_mutate(
    State(state): State<AppState>,
    body: Result<Json<ConfigUpdate>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let Json(req) = match body {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))),
    };

    let mut config = state.config.lock();
    match req {
        ConfigUpdate::Core { provider, model, api_key } => {
            config.default_provider = Some(provider);
            if !model.trim().is_empty() { config.default_model = Some(model); }
            if let Some(key) = api_key {
                if !key.trim().is_empty() { config.api_key = Some(key); }
            }
        }
        ConfigUpdate::ChannelTelegram { bot_token } => {
            config.channels_config.telegram = Some(crate::config::TelegramConfig { bot_token, allowed_users: vec![] });
        }
        ConfigUpdate::ChannelDiscord { bot_token, guild_id } => {
            let gid = guild_id.filter(|s| !s.trim().is_empty());
            config.channels_config.discord = Some(crate::config::DiscordConfig { bot_token, guild_id: gid, allowed_users: vec![], listen_to_bots: false, mention_only: false });
        }
        ConfigUpdate::ChannelQQ { app_id, app_secret } => {
            config.channels_config.qq = Some(crate::config::schema::QQConfig { app_id, app_secret, allowed_users: vec![] });
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
        }
    }
    if let Err(e) = config.save() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })));
    }
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok", "requires_restart": true })))
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

    match state
        .provider
        .simple_chat(&req.message, &state.model, state.temperature)
        .await
    {
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
    let target_file = params.get("file")
        .filter(|f| !f.is_empty())
        .cloned()
        .or_else(|| if !active_file.is_empty() { Some(active_file.clone()) } else { None })
        .or_else(|| files.first().cloned())
        .unwrap_or_default();

    let content = if !target_file.is_empty() {
        std::fs::read_to_string(identities_dir.join(&target_file)).unwrap_or_default()
    } else {
        String::new()
    };

    Json(IdentityListResp { active_file, files, content })
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
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    let mut config = state.config.lock();
    let identities_dir = config.workspace_dir.join("identities");
    let _ = std::fs::create_dir_all(&identities_dir);

    let filename = if req.file.trim().is_empty() { "default.md" } else { req.file.trim() };
    let path = identities_dir.join(filename);

    // Always write (even empty content means clearing the file)
    if let Err(e) = std::fs::write(&path, &req.content) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })));
    }

    if req.set_active {
        // Copy content into IDENTITY.md at workspace root
        let identity_md = config.workspace_dir.join("IDENTITY.md");
        let _ = std::fs::write(&identity_md, &req.content);
        config.identity.aieos_path = Some(filename.to_string());
        if let Err(e) = config.save() {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })));
        }
    }
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

/// GET /api/cron — list all cron jobs
pub async fn handle_api_cron_list(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.lock();
    match crate::cron::list_jobs(&config) {
        Ok(jobs) => (StatusCode::OK, Json(serde_json::json!({ "jobs": jobs }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
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
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    let config = state.config.lock();
    let res = match req {
        CronUpdate::AddShell { expression, command } => {
            crate::cron::add_job(&config, &expression, &command).map(|_| ())
        }
        CronUpdate::Toggle { id, enabled } => {
            crate::cron::update_job(&config, &id, crate::cron::CronJobPatch { enabled: Some(enabled), ..Default::default() }).map(|_| ())
        }
        CronUpdate::Delete { id } => {
            crate::cron::remove_job(&config, &id)
        }
    };
    match res {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
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
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    let cmd = match req.action.as_str() {
        "install" => crate::ServiceCommands::Install,
        "start" => crate::ServiceCommands::Start,
        "stop" => crate::ServiceCommands::Stop,
        "uninstall" => crate::ServiceCommands::Uninstall,
        _ => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "Invalid action" }))),
    };
    let config = state.config.lock();
    match crate::service::handle_command(&cmd, &config) {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
    }
}
