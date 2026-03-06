# mybot

一个本地运行的 Rust TUI 代码助手，风格接近 OpenCode，支持：

- 流式回复
- thinking / 工具细节显示切换
- slash 命令与命令补全
- 本地 session 持久化
- `/undo` 回滚最近一次操作
- OpenCode 兼容 skills
- `.mybot` 下的自定义 skills 和 tools

---

## 1. 启动

### 环境要求

- Rust stable
- Linux / macOS 终端
- 可用的 LLM API Key

### 安装依赖并运行

在项目根目录执行：

- `cargo run`

或先检查编译：

- `cargo check`

程序启动后会在工作区下创建 `.mybot/`，用于保存：

- 项目级配置
- 当前 session
- 历史 sessions
- 自定义 skills
- 自定义 tools

如果当前 runtime scope 是 `home`，则会在 `~/.mybot/` 下创建同样的目录结构。

运行目录有一个开关：

- 开发模式默认读“当前项目下的 `.mybot/`”
- 发布/最终模式默认读“`~/.mybot/`”
- 也可以用环境变量强制切换：`MYBOT_RUNTIME_SCOPE=workspace|home`

当前开发版默认读取当前项目下的内容：

- `.mybot/config/*.toml`
- `.mybot/.env`
- `.mybot/skills`
- `.mybot/tools`

---

## 2. 配置

推荐把配置放在 `.mybot/config/` 下：

- `.mybot/config/llm.toml`：模型与 provider 配置
- `.mybot/config/permissions.toml`：工具权限策略
- `.mybot/config/keybindings.toml`：快捷键配置

兼容说明：

- 当前版本只读取当前激活 runtime scope 下的 `.mybot/config/`
- 不再处理旧 `config/` 目录迁移

### LLM 配置

支持：

- OpenAI Compatible
- Anthropic
- Aliyun Coding Plan

推荐把 key 放到环境变量里，例如：

- `OPENAI_API_KEY`
- `ANTHROPIC_API_KEY`
- `BAILIAN_CODING_PLAN_API_KEY`

然后在 `.mybot/config/llm.toml` 中通过 `api_key_env` 引用。

推荐做法：

- 不要把真实 key 直接写进 `llm.toml`
- 把 key 放到 `.mybot/.env`
- 在 `llm.toml` 中只保留 `api_key_env`

示例：

`.mybot/.env`

```env
OPENAI_API_KEY=your-openai-key
ANTHROPIC_API_KEY=your-anthropic-key
BAILIAN_CODING_PLAN_API_KEY=your-bailian-key
```

`.mybot/config/llm.toml`

```toml
active_profile = "openai-default"

[profiles.openai-default]
provider = "open-ai-compatible"
base_url = "https://api.openai.com/v1"
model = "gpt-4.1-mini"
api_key_env = "OPENAI_API_KEY"
anthropic_version = "2023-06-01"
system_prompt = "You are a helpful private CLI assistant. Be concise and practical."
temperature = 0.2
max_tokens = 2048
timeout_seconds = 120
```

建议直接保持 `.mybot/config/llm.toml` 不含真实密钥，只使用 `api_key_env`。

### 权限配置

`.mybot/config/permissions.toml` 控制工具权限，典型例子：

- 默认允许
- `run_command` 需要确认
- 编辑 `src/**` 允许，其它编辑需要确认

程序运行时还支持会话级：

- 始终允许某个工具
- 始终拒绝某个工具

---

## 3. 基本操作

### 常用快捷键

默认快捷键见 `.mybot/config/keybindings.toml`，常用的是：

- `Enter`：发送
- `Alt+Enter`：插入换行
- `Tab`：补全 slash 命令
- `↑ / ↓`：历史输入或命令选择
- `Left / Right`：移动光标
- `Home / End`：到行首 / 行尾
- `PageUp / PageDown`：滚动聊天区
- `F2`：打开配置界面
- `F3`：显示/隐藏 thinking
- `F4`：展开/折叠工具细节
- `Esc`：清空输入 / 关闭弹窗

### slash 命令

常用命令：

- `/help`：查看所有命令
- `/tools`：查看当前工具
- `/tools reload`：重新加载 `.mybot/tools` 自定义工具
- `/tool <name> <json>`：手动调用工具
- `/permissions`：查看权限配置
- `/skills`：打开 skills 弹窗
- `/skills list`：文本列出所有 skills
- `/skills reload`：重新加载 skills
- `/skill <name>`：查看某个 skill 内容
- `/sessions`：打开 session 选择弹窗
- `/session new [title]`：新建 session
- `/session switch <id>`：切换 session
- `/session rename [id] <title>`：重命名 session
- `/undo`：撤销上一次操作
- `/clear`：清空当前会话显示
- `/config`：打开配置界面

---

## 4. Session

所有会话都保存在：

- `.mybot/sessions/`

当前会话指针保存在：

- `.mybot/current.json`

支持：

- 自动保存
- 启动恢复
- 会话切换
- 会话重命名
- 会话选择弹窗

---

## 5. Skills

### 内置发现位置

程序会自动发现这些位置的 `SKILL.md`：

- `.mybot/skills`
- `.opencode/skills`
- `.claude/skills`
- `.agents/skills`
- `~/.config/opencode/skills`
- `~/.claude/skills`
- `~/.agents/skills`

### skill 目录结构

一个 skill 对应一个目录，例如：

- `.mybot/skills/rust-refactor/SKILL.md`

`SKILL.md` 示例：

```md
---
name: rust-refactor
description: Help with safe Rust refactoring tasks
license: MIT
---

# Rust Refactor

Use this skill when the task involves restructuring Rust code while keeping behavior unchanged.
```

要求：

- 文件名必须是 `SKILL.md`
- 目录名必须和 `name` 一致
- `name` 只能是小写字母、数字、连字符

### 使用方式

- `/skills` 打开 skills 弹窗
- `Enter` 查看 skill 完整内容
- agent 在任务匹配时也会自动调用 `skill` 工具加载 skill

---

## 6. 自定义 Tools

### 目录

自定义工具放在：

- `.mybot/tools/*.toml`

每个 TOML 文件定义一个工具。

### 示例

文件：`.mybot/tools/echo-json.toml`

```toml
name = "echo_json"
description = "Echo back the input JSON for debugging"
command = "python3 .mybot/tools/echo_json.py"
working_dir = "."
timeout_seconds = 10

input_schema = { type = "object", additionalProperties = true }
```

脚本：`.mybot/tools/echo_json.py`

```python
import json
import os
import sys

raw = sys.stdin.read().strip() or os.environ.get("MYBOT_TOOL_INPUT", "{}")
input_data = json.loads(raw)

print(json.dumps({
    "summary": "echoed input",
    "content": {
        "received": input_data
    }
}, ensure_ascii=False))
```

### 运行约定

执行自定义工具时，mybot 会提供：

- stdin：完整 JSON 输入
- 环境变量 `MYBOT_TOOL_INPUT`
- 环境变量 `MYBOT_TOOL_NAME`
- 环境变量 `MYBOT_WORKSPACE_ROOT`

### 输出约定

如果 stdout 输出如下 JSON：

```json
{
  "summary": "done",
  "content": {
    "ok": true
  }
}
```

则会作为结构化工具结果返回。

如果 stdout 不是该格式，也会被当作普通文本结果收集。

### 重新加载

修改 `.mybot/tools` 后可执行：

- `/tools reload`

---

## 7. Undo

`/undo` 可以撤销最近一次操作，包括：

- 会话状态恢复
- 一部分工具引起的文件系统修改回滚

目前更适合回滚这些变更：

- `write_file`
- `apply_patch`
- `make_directory`
- `move_path`
- `delete_path`

注意：

- `run_command` 的任意副作用目前不能完整回滚

---

## 8. 权限模型

工具执行有三种模式：

- `allow`
- `ask`
- `deny`

对于高风险操作，建议至少设置为 `ask`，例如：

- `run_command`
- 自定义工具
- 工作区写操作

你可以在当前 scope 的 `.mybot/config/permissions.toml` 里配置，也可以在运行时对某些工具做会话级允许/拒绝。

当前推荐修改：

- `.mybot/config/permissions.toml`

---

## 9. 推荐的 `.mybot` 目录结构

```text
.mybot/
  .env
  config/
    llm.toml
    permissions.toml
    keybindings.toml
  current.json
  sessions/
  skills/
    rust-refactor/
      SKILL.md
  tools/
    echo-json.toml
    echo_json.py
```

---

## 10. 开发建议

如果你准备继续扩展这个项目，建议优先做：

- `/redo`
- 自定义 tool 的管理弹窗
- skill 来源标记与更强预览
- 更细粒度的自定义 tool 权限配置
- `run_command` 的隔离执行或沙箱支持

---

## 11. 故障排查

### 看不到自定义 skill

检查：

- 路径是否在 `.mybot/skills/<name>/SKILL.md`
- frontmatter 是否合法
- `name` 和目录名是否一致
- 执行 `/skills reload`

### 看不到自定义 tool

检查：

- 文件是否在 `.mybot/tools/*.toml`
- `name` 是否和内置工具重名
- `command` 是否能在 shell 中运行
- 执行 `/tools reload`

### 工具被拒绝执行

检查：

- `.mybot/config/permissions.toml`
- 当前 session 是否记住了 allow / deny
- `/permissions` 输出

### API key 读取失败

检查：

- `.mybot/.env` 是否存在
- `.mybot/.env` 中变量名是否和 `.mybot/config/llm.toml` 里的 `api_key_env` 一致
- 当前运行 scope 是否正确；如需强制切换，可设置 `MYBOT_RUNTIME_SCOPE=workspace` 或 `MYBOT_RUNTIME_SCOPE=home`

---

## 12. 说明

这是一个本地优先、可扩展的 TUI 助手骨架。

重点不是“把所有能力硬编码进程序里”，而是：

- 用 skills 复用经验
- 用 tools 扩展能力
- 用 `.mybot` 在项目级做定制
