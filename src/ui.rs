use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    app::{App, TOOL_LOG_MARKER_PREFIX},
    config::Action,
};

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    let input_inner_height = app.editor.preferred_height(5) as u16;
    let command_hints = app.command_hint_lines();
    let help_height = if command_hints.is_empty() { 3 } else { 5 };
    let header_height = 3;
    let input_height = input_inner_height + 2;

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(6),
            Constraint::Length(input_height),
            Constraint::Length(help_height),
        ])
        .split(area);

    let conversation_inner = inner_area(layout[1]);
    let input_inner = inner_area(layout[2]);
    app.sync_viewports(
        conversation_inner.width,
        conversation_inner.height,
        input_inner.width,
        input_inner.height,
    );

    let header = Paragraph::new("mybot · Private Assistant Skeleton")
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Header")
                .title_bottom(Line::from(vec![
                    format!(
                        "scope={} · session={} · profile={} · provider={} · model={} · tools={} · {} 打开配置 · {} 换行 · {}/{} 滚动聊天",
                        app.runtime_scope_label(),
                        app.session_summary_label(),
                        app.profile_name(),
                        app.provider_name(),
                        app.model_name(),
                        app.tool_count(),
                        app.key_label(Action::OpenConfig),
                        app.key_label(Action::InsertNewline),
                        app.key_label(Action::ScrollUp),
                        app.key_label(Action::ScrollDown)
                    )
                    .dark_gray(),
                ])),
        );
    frame.render_widget(header, layout[0]);

    let messages = Paragraph::new(build_conversation_text(app))
        .wrap(Wrap { trim: false })
        .scroll((app.conversation_scroll, 0))
        .block(Block::default().borders(Borders::ALL).title("Conversation"));
    frame.render_widget(messages, layout[1]);

    let input = Paragraph::new(Text::from(
        app.editor
            .visible_lines()
            .into_iter()
            .map(Line::from)
            .collect::<Vec<_>>(),
    ))
    .style(Style::default().fg(Color::Yellow))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Input")
            .title_bottom(Line::from(vec![
                if app.ctrl_c_armed() {
                    format!("{} 再按一次退出", app.key_label(Action::ClearOrExit))
                        .red()
                        .bold()
                } else if !command_hints.is_empty() {
                    format!(
                        "输入 / 命令中... {} 补全 · {} 提交",
                        app.key_label(Action::AutocompleteCommand),
                        app.key_label(Action::SubmitInput)
                    )
                    .cyan()
                    .bold()
                } else if app.is_waiting_for_reply() {
                    format!(
                        "流式输出中... {} 思考显隐 · {} 工具细节",
                        app.key_label(Action::ToggleThinking),
                        app.key_label(Action::ToggleToolDetails)
                    )
                    .cyan()
                    .bold()
                } else {
                    format!("{} lines", app.editor.line_count()).dark_gray()
                },
            ])),
    );
    frame.render_widget(input, layout[2]);

    let mut help_lines = vec![Line::from(vec![
        format!(
            "{}/{} 历史或跨行移动  ",
            app.key_label(Action::NavigateUp),
            app.key_label(Action::NavigateDown)
        )
        .into(),
        format!(
            "{}/{} 光标  ",
            app.key_label(Action::MoveLeft),
            app.key_label(Action::MoveRight)
        )
        .dark_gray(),
        format!(
            "{}/{} 行首尾  ",
            app.key_label(Action::MoveLineStart),
            app.key_label(Action::MoveLineEnd)
        )
        .dark_gray(),
        format!("{} 补全命令  ", app.key_label(Action::AutocompleteCommand)).dark_gray(),
        format!("{} 清空/退出  ", app.key_label(Action::ClearOrExit)).dark_gray(),
        "/help 查看命令  ".dark_gray(),
        "/sessions 会话列表  ".dark_gray(),
        format!("{} 退出", app.key_label(Action::Quit)).dark_gray(),
    ])];
    for hint in command_hints {
        help_lines.push(Line::from(hint));
    }

    let help = Paragraph::new(Text::from(help_lines))
    .block(Block::default().borders(Borders::ALL).title("Help"));
    frame.render_widget(help, layout[3]);

    if !app.is_config_open()
        && !app.has_pending_tool_approval()
        && !app.is_skill_picker_open()
        && !app.is_session_picker_open()
        && !app.is_session_rename_open()
    {
        draw_command_palette(frame, app, layout[1], layout[2]);
    }

    if app.is_config_open() {
        draw_config_modal(frame, app);
    } else if app.is_session_rename_open() {
        draw_session_rename_modal(frame, app);
    } else if app.is_skill_picker_open() {
        draw_skill_picker_modal(frame, app);
    } else if app.is_session_picker_open() {
        draw_session_picker_modal(frame, app);
    } else if app.has_pending_tool_approval() {
        draw_tool_approval_modal(frame, app);
    } else {
        let (cursor_x, cursor_y) = app.editor.cursor_screen_position();
        frame.set_cursor_position(clamp_cursor_position(frame.area(), input_inner, cursor_x, cursor_y));
    }
}

fn draw_skill_picker_modal(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(frame.area(), 76, 68);
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .title("Skills")
        .title_bottom(Line::from(vec![
            format!(
                "↑/↓ 选择 · {} 查看详情 · r 重新加载 · {} 关闭",
                app.key_label(Action::SubmitInput),
                app.key_label(Action::CloseConfig)
            )
            .dark_gray(),
        ]));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let Some(picker) = app.skill_picker() else {
        return;
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(inner);

    let summary = Paragraph::new(format!(
        "当前共 {} 个 skills · 选中后按 Enter 输出完整内容到会话区",
        picker.skills.len()
    ))
    .block(Block::default().borders(Borders::ALL).title("Summary"))
    .wrap(Wrap { trim: false });
    frame.render_widget(summary, layout[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(layout[1]);

    let visible_len = body[0].height.saturating_sub(2) as usize;
    let visible_len = visible_len.max(1);
    let start = picker.selected.saturating_sub(visible_len.saturating_sub(1));
    let lines = picker
        .skills
        .iter()
        .skip(start)
        .take(visible_len)
        .enumerate()
        .map(|(offset, skill)| {
            let absolute = start + offset;
            let selected = absolute == picker.selected;
            let style = if selected {
                Style::default().fg(Color::Black).bg(Color::Rgb(232, 170, 124))
            } else {
                Style::default().fg(Color::White)
            };

            let summary_style = if selected {
                style
            } else {
                Style::default().fg(Color::DarkGray)
            };

            Line::from(vec![
                Span::styled(format!("{:<22}", skill.name), style),
                Span::styled(skill.description.clone(), summary_style),
            ])
        })
        .collect::<Vec<_>>();

    let list = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title("Available Skills"))
        .wrap(Wrap { trim: false });
    frame.render_widget(list, body[0]);

    if let Some(skill) = picker.skills.get(picker.selected) {
        let preview = Paragraph::new(Text::from(vec![
            Line::from(vec!["name: ".dark_gray(), skill.name.clone().cyan().bold()]),
            Line::from(vec!["description: ".dark_gray(), skill.description.clone().into()]),
            Line::from(vec!["path: ".dark_gray(), skill.path.clone().dark_gray()]),
            Line::from(String::new()),
            Line::from("提示：Enter 会把完整 skill 内容输出到会话区，方便继续参考或复制。"),
        ]))
        .block(Block::default().borders(Borders::ALL).title("Preview"))
        .wrap(Wrap { trim: false });
        frame.render_widget(preview, body[1]);
    }
}

fn draw_session_picker_modal(frame: &mut Frame<'_>, app: &App) {
    let area = centered_rect(frame.area(), 72, 60);
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .title("Sessions")
        .title_bottom(Line::from(vec![
            format!(
                "↑/↓ 选择 · {} 切换 · r 重命名 · {} 关闭",
                app.key_label(Action::SubmitInput),
                app.key_label(Action::CloseConfig)
            )
            .dark_gray(),
        ]));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let Some(picker) = app.session_picker() else {
        return;
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(inner);

    let summary = Paragraph::new(format!(
        "当前会话：{} · 共 {} 个会话",
        app.session_summary_label(),
        picker.sessions.len()
    ))
    .block(Block::default().borders(Borders::ALL).title("Summary"))
    .wrap(Wrap { trim: false });
    frame.render_widget(summary, layout[0]);

    let visible_len = layout[1].height.saturating_sub(2) as usize;
    let visible_len = visible_len.max(1);
    let start = picker.selected.saturating_sub(visible_len.saturating_sub(1));
    let lines = picker
        .sessions
        .iter()
        .skip(start)
        .take(visible_len)
        .enumerate()
        .map(|(offset, session)| {
            let absolute = start + offset;
            let selected = absolute == picker.selected;
            let current = session.id == app.current_session_id();
            let style = if selected {
                Style::default().fg(Color::Black).bg(Color::Rgb(232, 170, 124))
            } else if current {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };

            let marker = if current { "*" } else { " " };
            let id = session.id.chars().take(8).collect::<String>();
            Line::from(vec![
                Span::styled(format!("{} {:<24}", marker, session.title), style),
                Span::styled(
                    format!(" {}  updated={} ", id, session.updated_at),
                    if selected { style } else { Style::default().fg(Color::DarkGray) },
                ),
            ])
        })
        .collect::<Vec<_>>();

    let list = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::ALL).title("Available Sessions"))
        .wrap(Wrap { trim: false });
    frame.render_widget(list, layout[1]);
}

fn draw_session_rename_modal(frame: &mut Frame<'_>, app: &mut App) {
    let area = centered_rect(frame.area(), 64, 28);
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .title("Rename Session")
        .title_bottom(Line::from(vec![
            format!(
                "{} 保存 · {} 取消",
                app.key_label(Action::SubmitInput),
                app.key_label(Action::CloseConfig)
            )
            .dark_gray(),
        ]));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(3),
        ])
        .split(inner);

    let input_inner = inner_area(layout[1]);
    app.sync_session_rename_viewport(input_inner.width, input_inner.height);

    let Some(rename) = app.session_rename() else {
        return;
    };

    let summary = Paragraph::new(format!(
        "session={} · original={}",
        rename.session_id.chars().take(8).collect::<String>(),
        rename.original_title,
    ))
    .block(Block::default().borders(Borders::ALL).title("Target"))
    .wrap(Wrap { trim: false });
    frame.render_widget(summary, layout[0]);

    let editor = Paragraph::new(Text::from(
        rename
            .editor
            .visible_lines()
            .into_iter()
            .map(Line::from)
            .collect::<Vec<_>>(),
    ))
    .style(Style::default().fg(Color::Yellow))
    .block(Block::default().borders(Borders::ALL).title("New Title"));
    frame.render_widget(editor, layout[1]);

    let help = Paragraph::new("输入新名称后按 Enter 保存。Esc 取消。")
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .wrap(Wrap { trim: false });
    frame.render_widget(help, layout[2]);

    let (cursor_x, cursor_y) = rename.editor.cursor_screen_position();
    frame.set_cursor_position(clamp_cursor_position(frame.area(), input_inner, cursor_x, cursor_y));
}

pub fn conversation_plain_lines(app: &App) -> Vec<String> {
    build_conversation_text(app)
        .lines
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        })
        .collect()
}

fn draw_command_palette(frame: &mut Frame<'_>, app: &App, conversation_area: Rect, input_area: Rect) {
    let suggestions = app.command_suggestions();
    let Some(selected_index) = app.selected_command_suggestion_index() else {
        return;
    };

    let palette_bounds = Rect::new(
        conversation_area.x.saturating_add(1),
        conversation_area.y.saturating_add(1),
        conversation_area.width.saturating_sub(2),
        input_area
            .y
            .saturating_sub(conversation_area.y)
            .saturating_sub(1),
    );

    if palette_bounds.width < 4 || palette_bounds.height < 3 {
        return;
    }

    let visible_count = suggestions
        .len()
        .min(6)
        .min(palette_bounds.height.saturating_sub(2) as usize);
    if visible_count == 0 {
        return;
    }

    let width = conversation_area
        .width
        .min(72)
        .clamp(4, palette_bounds.width);
    let height = (visible_count as u16 + 2).min(palette_bounds.height);
    let x = palette_bounds.x;
    let y = palette_bounds
        .y
        .saturating_add(palette_bounds.height.saturating_sub(height));
    let area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Commands")
        .title_bottom(Line::from(vec![
            format!(
                "↑/↓ 选择 · {} 补全 · {} 提交",
                app.key_label(Action::AutocompleteCommand),
                app.key_label(Action::SubmitInput)
            )
            .dark_gray(),
        ]));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_len = visible_count;
    let start = selected_index.saturating_sub(visible_len.saturating_sub(1));
    let lines = suggestions
        .into_iter()
        .skip(start)
        .take(visible_len)
        .enumerate()
        .map(|(index, command)| {
            let absolute_index = start + index;
            let selected = absolute_index == selected_index;
            let style = if selected {
                Style::default().fg(Color::Black).bg(Color::Rgb(232, 170, 124))
            } else {
                Style::default().fg(Color::White)
            };

            let summary_style = if selected {
                style
            } else {
                Style::default().fg(Color::DarkGray)
            };

            Line::from(vec![
                Span::styled(format!("{:<18}", command.name), style),
                Span::styled(command.summary.to_string(), summary_style),
            ])
        })
        .collect::<Vec<_>>();

    let list = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
    frame.render_widget(list, inner);
}

fn draw_tool_approval_modal(frame: &mut Frame<'_>, app: &mut App) {
    let area = centered_rect(frame.area(), 70, 42);
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .title("Tool Approval")
        .title_bottom(Line::from(vec![
            format!(
                "{} 批准 · {} 本次会话始终允许 · {} 本次会话始终拒绝 · {} 拒绝",
                app.key_label(Action::ApproveTool),
                app.key_label(Action::AlwaysAllowTool),
                app.key_label(Action::AlwaysDenyTool),
                app.key_label(Action::RejectTool)
            )
            .dark_gray(),
        ]));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let Some(request) = app.pending_tool_approval() else {
        return;
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(4),
        ])
        .split(inner);

    let summary = Paragraph::new(format!(
        "step={} · tool={} · target={}{}",
        request.step,
        request.tool,
        request.summary,
        request
            .thought
            .as_ref()
            .map(|thought| format!(" · {thought}"))
            .unwrap_or_default()
    ))
    .block(Block::default().borders(Borders::ALL).title("Summary"))
    .wrap(Wrap { trim: false });
    frame.render_widget(summary, layout[0]);

    let pretty_input = serde_json::to_string_pretty(&request.input).unwrap_or_else(|_| request.input.to_string());
    let details = Paragraph::new(pretty_input)
        .block(Block::default().borders(Borders::ALL).title("Input JSON"))
        .wrap(Wrap { trim: false });
    frame.render_widget(details, layout[1]);

    let footer = Paragraph::new("自动工具调用命中了 ask 权限规则。批准后本次执行继续；选择“始终允许”会在当前会话内跳过该工具后续审批；选择“始终拒绝”会在当前会话内直接拦截该工具；拒绝后模型会收到失败结果并继续尝试回答。")
        .block(Block::default().borders(Borders::ALL).title("Permission"))
        .wrap(Wrap { trim: false });
    frame.render_widget(footer, layout[2]);
}

fn draw_config_modal(frame: &mut Frame<'_>, app: &mut App) {
    let area = centered_rect(frame.area(), 80, 75);
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .title("Interactive Config")
        .title_bottom(Line::from(vec![
            format!(
                "{} 保存 · {} 关闭 · {}/{} 切换字段 · profile 字段可切换配置",
                app.key_label(Action::SaveConfig),
                app.key_label(Action::CloseConfig),
                app.key_label(Action::ConfigPreviousField),
                app.key_label(Action::ConfigNextField)
            )
            .dark_gray(),
        ]));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
            Constraint::Length(4),
            Constraint::Length(3),
        ])
        .split(inner);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(layout[0]);

    let fields = app
        .config_editor()
        .map(|editor| editor.field_lines())
        .unwrap_or_default();
    let fields_widget = Paragraph::new(Text::from(
        fields.into_iter().map(Line::from).collect::<Vec<_>>(),
    ))
    .block(Block::default().borders(Borders::ALL).title("Fields"))
    .wrap(Wrap { trim: false });
    frame.render_widget(fields_widget, top[0]);

    let editor_inner = inner_area(top[1]);
    app.sync_config_viewport(editor_inner.width, editor_inner.height);

    let value_lines = app
        .config_editor()
        .map(|editor| editor.visible_lines())
        .unwrap_or_default();
    let selected_label = app
        .config_editor()
        .map(|editor| editor.selected_label().to_string())
        .unwrap_or_else(|| "value".to_string());
    let value_widget = Paragraph::new(Text::from(
        value_lines.into_iter().map(Line::from).collect::<Vec<_>>(),
    ))
    .style(Style::default().fg(Color::Yellow))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Edit · {selected_label}")),
    )
    .wrap(Wrap { trim: false });
    frame.render_widget(value_widget, top[1]);

    let help_text = app
        .config_editor()
        .map(|editor| editor.selected_help().to_string())
        .unwrap_or_default();
    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Field Help"))
        .wrap(Wrap { trim: false });
    frame.render_widget(help, layout[1]);

    let status_text = app
        .config_editor()
        .map(|editor| {
            if editor.dirty() {
                format!("{} · 有未保存修改", editor.status())
            } else {
                editor.status().to_string()
            }
        })
        .unwrap_or_default();
    let status = Paragraph::new(status_text)
        .block(Block::default().borders(Borders::ALL).title("Status"))
        .wrap(Wrap { trim: false });
    frame.render_widget(status, layout[2]);

    if let Some(editor) = app.config_editor() {
        let (cursor_x, cursor_y) = editor.cursor_screen_position();
        frame.set_cursor_position(clamp_cursor_position(frame.area(), editor_inner, cursor_x, cursor_y));
    }
}

fn build_conversation_text(app: &App) -> Text<'static> {
    let mut lines = Vec::new();

    for message in &app.messages {
        if let Some(index_text) = message.strip_prefix(TOOL_LOG_MARKER_PREFIX) {
            if let Ok(index) = index_text.parse::<usize>()
                && let Some(section) = app.tool_logs().get(index)
            {
                render_tool_log_section(&mut lines, section.title.as_str(), &section.events, app.show_tool_details());
                lines.push(Line::from(String::new()));
                continue;
            }
        }

        if let Some(content) = message.strip_prefix("you> ") {
            lines.extend(render_prefixed_block("you>", content, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD), false, app.show_thinking()));
        } else if let Some(content) = message.strip_prefix("skill> ") {
            lines.extend(render_prefixed_block("skill>", content, Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD), false, app.show_thinking()));
        } else if let Some(content) = message.strip_prefix("assistant> ") {
            lines.extend(render_prefixed_block("assistant>", content, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD), true, app.show_thinking()));
        } else {
            lines.extend(render_prefixed_block("system>", message, Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD), false, app.show_thinking()));
        }
        lines.push(Line::from(String::new()));
    }

    if let Some(preview) = app.active_stream_preview()
        && !preview.text.is_empty()
    {
        render_stream_preview(&mut lines, preview.text.as_str(), app.show_thinking());
    }

    if !app.active_tool_events().is_empty() {
        render_tool_log_section(
            &mut lines,
            "工具执行中",
            app.active_tool_events(),
            app.show_tool_details(),
        );
        lines.push(Line::from(String::new()));
    }

    Text::from(lines)
}

fn render_stream_preview(lines: &mut Vec<Line<'static>>, text: &str, show_thinking: bool) {
    lines.push(Line::from(vec![
        "assistant(stream)> ".cyan().bold(),
        if text.trim().is_empty() {
            "等待首个 token...".dark_gray()
        } else {
            format!("step stream").dark_gray()
        },
    ]));
    lines.extend(render_markdown_lines(text, show_thinking, true));
    lines.push(Line::from(String::new()));
}

fn render_tool_log_section(
    lines: &mut Vec<Line<'static>>,
    title: &str,
    events: &[String],
    show_details: bool,
) {
    let label = if show_details { "▼" } else { "▶" };
    lines.push(Line::from(vec![
        Span::styled(
            format!("tool> {label} {title}"),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if show_details {
        for event in events {
            lines.push(Line::from(vec![
                Span::styled("  • ", Style::default().fg(Color::Magenta)),
                Span::styled(event.clone(), Style::default().fg(Color::Gray)),
            ]));
        }
    } else {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} 条事件，按切换键展开", events.len()),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
}

fn render_prefixed_block(
    prefix: &str,
    content: &str,
    prefix_style: Style,
    markdown: bool,
    show_thinking: bool,
) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    let body_lines = if markdown {
        render_markdown_lines(content, show_thinking, false)
    } else {
        content
            .split('\n')
            .map(|line| Line::from(line.to_string()))
            .collect()
    };

    let mut iter = body_lines.into_iter();
    if let Some(first) = iter.next() {
        let mut spans = vec![Span::styled(format!("{prefix} "), prefix_style)];
        spans.extend(first.spans.into_iter());
        rendered.push(Line::from(spans));
    } else {
        rendered.push(Line::from(vec![Span::styled(format!("{prefix} "), prefix_style)]));
    }

    rendered.extend(iter);
    rendered
}

fn render_markdown_lines(text: &str, show_thinking: bool, dim_plain: bool) -> Vec<Line<'static>> {
    let normalized = normalize_thinking_markup(text);
    let mut lines = Vec::new();
    let mut in_code_block = false;
    let mut in_thinking = false;
    let mut emitted_thinking_placeholder = false;
    let mut emitted_thinking_header = false;

    for raw_line in normalized.split('\n') {
        let mut line = raw_line;

        if let Some((before, after)) = line.split_once("<think>") {
            if !before.is_empty() {
                lines.extend(render_markdown_lines(before, show_thinking, dim_plain));
            }
            in_thinking = true;
            emitted_thinking_placeholder = false;
            emitted_thinking_header = false;
            line = after;
        }

        if let Some((inside, after)) = line.split_once("</think>") {
            if in_thinking {
                render_segment_line(
                    &mut lines,
                    inside,
                    &mut in_code_block,
                    true,
                    show_thinking,
                    dim_plain,
                    &mut emitted_thinking_placeholder,
                    &mut emitted_thinking_header,
                );
                in_thinking = false;
                if !after.is_empty() {
                    render_segment_line(
                        &mut lines,
                        after,
                        &mut in_code_block,
                        false,
                        show_thinking,
                        dim_plain,
                        &mut emitted_thinking_placeholder,
                        &mut emitted_thinking_header,
                    );
                }
                continue;
            }
        }

        render_segment_line(
            &mut lines,
            line,
            &mut in_code_block,
            in_thinking,
            show_thinking,
            dim_plain,
            &mut emitted_thinking_placeholder,
            &mut emitted_thinking_header,
        );
    }

    lines
}

fn render_segment_line(
    lines: &mut Vec<Line<'static>>,
    line: &str,
    in_code_block: &mut bool,
    in_thinking: bool,
    show_thinking: bool,
    dim_plain: bool,
    emitted_thinking_placeholder: &mut bool,
    emitted_thinking_header: &mut bool,
) {
    if in_thinking && !show_thinking {
        if !*emitted_thinking_placeholder {
            lines.push(Line::from(vec![Span::styled(
                "[thinking hidden — 按切换键显示完整思考过程]",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )]));
            *emitted_thinking_placeholder = true;
        }
        return;
    }

    if in_thinking && show_thinking && !*emitted_thinking_header {
        lines.push(Line::from(vec![Span::styled(
            "thinking> 推理过程",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
        )]));
        *emitted_thinking_header = true;
    }

    let trimmed = line.trim_start();
    if trimmed.starts_with("```") {
        *in_code_block = !*in_code_block;
        let content = if in_thinking {
            format!("│ {line}")
        } else {
            line.to_string()
        };
        lines.push(Line::from(vec![Span::styled(
            content,
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )]));
        return;
    }

    let base_style = if in_thinking {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC)
    } else if *in_code_block {
        Style::default().fg(Color::Green)
    } else if dim_plain {
        Style::default().fg(Color::Gray)
    } else {
        Style::default()
    };

    if *in_code_block {
        let content = if in_thinking {
            format!("│ {line}")
        } else {
            line.to_string()
        };
        lines.push(Line::from(vec![Span::styled(content, base_style)]));
        return;
    }

    if let Some(heading) = trimmed.strip_prefix("# ") {
        let content = if in_thinking {
            format!("│ {heading}")
        } else {
            heading.to_string()
        };
        lines.push(Line::from(vec![Span::styled(
            content,
            base_style.fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )]));
        return;
    }

    if let Some(quote) = trimmed.strip_prefix("> ") {
        let content = if in_thinking {
            format!("│ {quote}")
        } else {
            quote.to_string()
        };
        lines.push(Line::from(vec![Span::styled(
            content,
            base_style.fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )]));
        return;
    }

    if let Some(item) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
        let mut spans = vec![Span::styled(
            if in_thinking { "│ • " } else { "• " },
            base_style.fg(Color::Green),
        )];
        spans.extend(render_inline_markdown(item, base_style));
        lines.push(Line::from(spans));
        return;
    }

    let mut spans = Vec::new();
    if in_thinking {
        spans.push(Span::styled("│ ", base_style.fg(Color::Blue)));
    }
    spans.extend(render_inline_markdown(line, base_style));
    lines.push(Line::from(spans));
}

fn normalize_thinking_markup(text: &str) -> String {
    let normalized = text
        .replace("<thinking>", "<think>")
        .replace("</thinking>", "</think>");

    let mut output = String::new();
    let mut in_thinking_fence = false;

    for line in normalized.split('\n') {
        let trimmed = line.trim();
        if trimmed == "```thinking" || trimmed == "```thought" || trimmed == "```reasoning" {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str("<think>");
            in_thinking_fence = true;
            continue;
        }

        if in_thinking_fence && trimmed == "```" {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str("</think>");
            in_thinking_fence = false;
            continue;
        }

        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(line);
    }

    if in_thinking_fence {
        output.push_str("\n</think>");
    }

    output
}

fn render_inline_markdown(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut in_code = false;

    for segment in text.split('`') {
        if in_code {
            spans.push(Span::styled(
                segment.to_string(),
                base_style
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(segment.to_string(), base_style));
        }
        in_code = !in_code;
    }

    if text.ends_with('`') {
        spans.push(Span::styled(String::new(), base_style));
    }

    spans
}

fn inner_area(area: Rect) -> Rect {
    area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    })
}

fn centered_rect(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100 - height_percent) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}

fn clamp_cursor_position(frame_area: Rect, area: Rect, cursor_x: u16, cursor_y: u16) -> (u16, u16) {
    let max_x = area.width.saturating_sub(1);
    let max_y = area.height.saturating_sub(1);
    let absolute_x = area.x.saturating_add(cursor_x.min(max_x));
    let absolute_y = area.y.saturating_add(cursor_y.min(max_y));
    let frame_max_x = frame_area.right().saturating_sub(1);
    let frame_max_y = frame_area.bottom().saturating_sub(1);

    (
        absolute_x.clamp(frame_area.x, frame_max_x),
        absolute_y.clamp(frame_area.y, frame_max_y),
    )
}
