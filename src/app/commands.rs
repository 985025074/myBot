#[derive(Debug, Clone, Copy)]
pub struct SlashCommand {
    pub name: &'static str,
    pub usage: &'static str,
    pub summary: &'static str,
}

const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "/help",
        usage: "/help",
        summary: "查看所有斜杠命令",
    },
    SlashCommand {
        name: "/commands",
        usage: "/commands",
        summary: "查看所有斜杠命令",
    },
    SlashCommand {
        name: "/tools",
        usage: "/tools [reload]",
        summary: "列出当前工具，或重新加载 .mybot/tools 自定义工具",
    },
    SlashCommand {
        name: "/tool",
        usage: "/tool <name> <json>",
        summary: "手动调用一个工具",
    },
    SlashCommand {
        name: "/permissions",
        usage: "/permissions",
        summary: "查看当前权限配置与会话记忆",
    },
    SlashCommand {
        name: "/skills",
        usage: "/skills [reload|list]",
        summary: "打开 skills 弹窗，或列出/重新加载 OpenCode 兼容 skills",
    },
    SlashCommand {
        name: "/skill",
        usage: "/skill <name>",
        summary: "查看某个 skill 的完整内容",
    },
    SlashCommand {
        name: "/sessions",
        usage: "/sessions",
        summary: "打开会话选择弹窗",
    },
    SlashCommand {
        name: "/session",
        usage: "/session <current|new|switch|save|rename>",
        summary: "管理本地会话",
    },
    SlashCommand {
        name: "/thinking",
        usage: "/thinking [on|off|toggle]",
        summary: "切换 thinking block 显示",
    },
    SlashCommand {
        name: "/tool-details",
        usage: "/tool-details [on|off|toggle]",
        summary: "切换工具细节展开状态",
    },
    SlashCommand {
        name: "/config",
        usage: "/config",
        summary: "打开交互式配置界面",
    },
    SlashCommand {
        name: "/clear",
        usage: "/clear",
        summary: "清空当前会话显示与临时状态",
    },
    SlashCommand {
        name: "/undo",
        usage: "/undo",
        summary: "撤销当前会话中的上一次操作",
    },
];

pub fn all() -> &'static [SlashCommand] {
    COMMANDS
}

pub fn find(name: &str) -> Option<&'static SlashCommand> {
    COMMANDS.iter().find(|command| command.name == name)
}

pub fn suggestions(input: &str) -> Vec<&'static SlashCommand> {
    let trimmed = input.trim_start();
    if !trimmed.starts_with('/') || trimmed.contains('\n') {
        return Vec::new();
    }

    let token = trimmed.split_whitespace().next().unwrap_or(trimmed);
    let token = token.to_ascii_lowercase();

    let mut matches = COMMANDS
        .iter()
        .filter(|command| token == "/" || command.name.starts_with(&token))
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.name.cmp(right.name));
    matches
}

#[allow(dead_code)]
pub fn autocomplete(input: &str) -> Option<String> {
    autocomplete_selected(input, 0)
}

pub fn autocomplete_selected(input: &str, index: usize) -> Option<String> {
    let trimmed_start = input.trim_start();
    if !trimmed_start.starts_with('/') || trimmed_start.contains('\n') {
        return None;
    }

    let suggestions = suggestions(trimmed_start);
    let command = suggestions.get(index).or_else(|| suggestions.first())?;
    let token_end = trimmed_start
        .find(char::is_whitespace)
        .unwrap_or(trimmed_start.len());
    let suffix = &trimmed_start[token_end..];
    Some(format!("{}{}", command.name, suffix))
}