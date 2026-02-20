# OneBot Markdown 上下文与会话文件说明

本文档说明 ZeroClaw 在 OneBot 场景下的 Markdown 文件布局与权限策略。

## 目录布局

- `workspace/contexts/*.md`
  - OpenClaw 提示词/上下文文件（`AGENTS.md`、`SOUL.md`、`NON_ADMIN.md` 等）
  - 系统提示词注入优先从 `contexts/` 读取
  - 兼容旧版根目录同名文件（仅作为回退读取）

- `workspace/sessions/*.md`
  - 会话历史文件（每个会话 `session_id` 对应一个 `.md` 文件）
  - 文件名采用 URL-safe base64 编码（避免 Windows 下 `:` 等非法字符问题）
  - 文件内容为 Markdown + JSON 行（便于兼容解析与后续迁移）

## 兼容与迁移

- 提示词文件读取：
  - 优先 `contexts/<NAME>.md`
  - 若不存在则回退 `<workspace>/<NAME>.md`

- 会话历史读取：
  - 优先 `sessions/*.md`
  - 若不存在则回退内存后端中的 `MemoryCategory::Conversation` 记录

- `zeroclaw onboard` 在创建工作区时会将旧版根目录上下文文件迁移到 `contexts/`（若目标不存在）。

## 非管理员权限限制（OneBot）

当 OneBot 启用了管理员策略且当前发送者为非管理员时：

- AI 工具调用将被阻止访问/修改以下受保护路径：
  - `contexts/`（提示词上下文）
  - `sessions/`（会话历史）
  - 以及历史兼容的根目录上下文文件名（如 `AGENTS.md`）

这保证了：

- 非管理员对话仍可正常获得系统注入上下文
- 但模型无法通过工具读写这些受保护 Markdown 文件
