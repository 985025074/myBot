use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    app::App,
    config::Action,
};

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    let input_inner_height = app.editor.preferred_height(5) as u16;
    let help_height = 3;
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
                        "profile={} · provider={} · model={} · tools={} · {} 打开配置 · {} 换行 · {}/{} 滚动聊天",
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
                } else if app.is_waiting_for_reply() {
                    "等待模型回复...".cyan().bold()
                } else {
                    format!("{} lines", app.editor.line_count()).dark_gray()
                },
            ])),
    );
    frame.render_widget(input, layout[2]);

    let help = Paragraph::new(Line::from(vec![
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
        format!("{} 清空/退出  ", app.key_label(Action::ClearOrExit)).dark_gray(),
        "/tools 查看工具  ".dark_gray(),
        format!("{} 配置  ", app.key_label(Action::OpenConfig)).dark_gray(),
        format!("{} 退出", app.key_label(Action::Quit)).dark_gray(),
    ]))
    .block(Block::default().borders(Borders::ALL).title("Help"));
    frame.render_widget(help, layout[3]);

    if app.is_config_open() {
        draw_config_modal(frame, app);
    } else if app.has_pending_tool_approval() {
        draw_tool_approval_modal(frame, app);
    } else {
        let (cursor_x, cursor_y) = app.editor.cursor_screen_position();
        frame.set_cursor_position((input_inner.x + cursor_x, input_inner.y + cursor_y));
    }
}

fn draw_tool_approval_modal(frame: &mut Frame<'_>, app: &mut App) {
    let area = centered_rect(frame.area(), 70, 42);
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .title("Tool Approval")
        .title_bottom(Line::from(vec![
            format!(
                "{} 批准 · {} 拒绝",
                app.key_label(Action::ApproveTool),
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
        "step={} · tool={}{}",
        request.step,
        request.tool,
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

    let footer = Paragraph::new("自动工具调用命中了 ask 权限规则。批准后会继续执行，拒绝后模型会收到失败结果并继续尝试回答。")
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
        frame.set_cursor_position((editor_inner.x + cursor_x, editor_inner.y + cursor_y));
    }
}

fn build_conversation_text(app: &App) -> Text<'static> {
    let mut lines = Vec::new();

    for message in &app.messages {
        for raw_line in message.split('\n') {
            lines.push(Line::from(raw_line.to_string()));
        }
        lines.push(Line::from(String::new()));
    }

    Text::from(lines)
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
