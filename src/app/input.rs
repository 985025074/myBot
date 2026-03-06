use unicode_width::UnicodeWidthChar;

#[derive(Debug, Default)]
pub struct InputEditor {
    text: String,
    cursor_position: usize,
    preferred_column: Option<usize>,
    history: Vec<String>,
    history_index: Option<usize>,
    history_draft: Option<String>,
    scroll_y: usize,
    viewport_width: usize,
    viewport_height: usize,
}

#[derive(Debug, Clone, Copy)]
struct VisualRow {
    start: usize,
    end: usize,
}

impl InputEditor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor_position = self.char_count();
        self.preferred_column = None;
        self.history_index = None;
        self.history_draft = None;
        self.scroll_y = 0;
        self.ensure_cursor_visible();
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.visual_rows().len().max(1)
    }

    pub fn preferred_height(&self, max_visible_lines: usize) -> usize {
        self.line_count().clamp(1, max_visible_lines.max(1))
    }

    pub fn set_viewport(&mut self, width: usize, height: usize) {
        self.viewport_width = width.max(1);
        self.viewport_height = height.max(1);
        self.ensure_cursor_visible();
    }

    pub fn visible_lines(&self) -> Vec<String> {
        let lines = self.visual_rows();
        let mut visible = Vec::new();

        for line in lines
            .into_iter()
            .skip(self.scroll_y)
            .take(self.viewport_height.max(1))
        {
            visible.push(self.slice_char_range(line.start, line.end));
        }

        if visible.is_empty() {
            visible.push(String::new());
        }

        visible
    }

    pub fn cursor_screen_position(&self) -> (u16, u16) {
        let (line_index, column) = self.cursor_visual_position();
        let visible_y = line_index.saturating_sub(self.scroll_y);
        (column as u16, visible_y as u16)
    }

    pub fn insert_char(&mut self, c: char) {
        let byte_index = self.char_to_byte_index(self.cursor_position);
        self.text.insert(byte_index, c);
        self.cursor_position += 1;
        self.preferred_column = None;
        self.ensure_cursor_visible();
    }

    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    pub fn delete_before_cursor(&mut self) {
        if self.cursor_position == 0 {
            return;
        }

        let current = self.char_to_byte_index(self.cursor_position);
        let previous = self.char_to_byte_index(self.cursor_position - 1);
        self.text.drain(previous..current);
        self.cursor_position -= 1;
        self.preferred_column = None;
        self.ensure_cursor_visible();
    }

    pub fn delete_at_cursor(&mut self) {
        if self.cursor_position >= self.char_count() {
            return;
        }

        let current = self.char_to_byte_index(self.cursor_position);
        let next = self.char_to_byte_index(self.cursor_position + 1);
        self.text.drain(current..next);
        self.preferred_column = None;
        self.ensure_cursor_visible();
    }

    pub fn move_left(&mut self) {
        self.cursor_position = self.cursor_position.saturating_sub(1);
        self.preferred_column = None;
        self.ensure_cursor_visible();
    }

    pub fn move_right(&mut self) {
        self.cursor_position = (self.cursor_position + 1).min(self.char_count());
        self.preferred_column = None;
        self.ensure_cursor_visible();
    }

    pub fn move_to_line_start(&mut self) {
        let rows = self.visual_rows();
        let (row_index, _) = self.cursor_visual_position();
        self.cursor_position = rows[row_index].start;
        self.preferred_column = None;
        self.ensure_cursor_visible();
    }

    pub fn move_to_line_end(&mut self) {
        let rows = self.visual_rows();
        let (row_index, _) = self.cursor_visual_position();
        self.cursor_position = rows[row_index].end;
        self.preferred_column = None;
        self.ensure_cursor_visible();
    }

    pub fn is_cursor_on_first_line(&self) -> bool {
        let (line_index, _) = self.cursor_visual_position();
        line_index == 0
    }

    pub fn is_cursor_on_last_line(&self) -> bool {
        let (line_index, _) = self.cursor_visual_position();
        line_index + 1 >= self.visual_rows().len()
    }

    pub fn move_up(&mut self) {
        let rows = self.visual_rows();
        let (line_index, column) = self.cursor_visual_position();
        if line_index == 0 {
            return;
        }

        let target_column = self.preferred_column.unwrap_or(column);
        let previous = rows[line_index - 1];
        self.cursor_position = self.char_index_for_column(previous, target_column);
        self.preferred_column = Some(target_column);
        self.ensure_cursor_visible();
    }

    pub fn move_down(&mut self) {
        let rows = self.visual_rows();
        let (line_index, column) = self.cursor_visual_position();
        if line_index + 1 >= rows.len() {
            return;
        }

        let target_column = self.preferred_column.unwrap_or(column);
        let next = rows[line_index + 1];
        self.cursor_position = self.char_index_for_column(next, target_column);
        self.preferred_column = Some(target_column);
        self.ensure_cursor_visible();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor_position = 0;
        self.preferred_column = None;
        self.history_index = None;
        self.history_draft = None;
        self.scroll_y = 0;
    }

    pub fn submit(&mut self) -> Option<String> {
        if self.text.trim().is_empty() {
            return None;
        }

        let submitted = self.text.clone();
        self.history.push(submitted.clone());
        self.text.clear();
        self.cursor_position = 0;
        self.preferred_column = None;
        self.history_index = None;
        self.history_draft = None;
        self.scroll_y = 0;
        Some(submitted)
    }

    pub fn use_older_history(&mut self) {
        if self.history.is_empty() {
            return;
        }

        let next_index = match self.history_index {
            Some(0) => 0,
            Some(index) => index.saturating_sub(1),
            None => {
                self.history_draft = Some(self.text.clone());
                self.history.len() - 1
            }
        };

        self.history_index = Some(next_index);
        self.load_text(self.history[next_index].clone());
    }

    pub fn use_newer_history(&mut self) {
        let Some(current_index) = self.history_index else {
            return;
        };

        if current_index + 1 < self.history.len() {
            let next_index = current_index + 1;
            self.history_index = Some(next_index);
            self.load_text(self.history[next_index].clone());
            return;
        }

        self.history_index = None;
        let draft = self.history_draft.take().unwrap_or_default();
        self.load_text(draft);
    }

    fn load_text(&mut self, text: String) {
        self.text = text;
        self.cursor_position = self.char_count();
        self.preferred_column = None;
        self.ensure_cursor_visible();
    }

    fn ensure_cursor_visible(&mut self) {
        let height = self.viewport_height.max(1);
        let (line_index, _) = self.cursor_visual_position();

        if line_index < self.scroll_y {
            self.scroll_y = line_index;
        } else if line_index >= self.scroll_y + height {
            self.scroll_y = line_index + 1 - height;
        }
    }

    fn visual_rows(&self) -> Vec<VisualRow> {
        let width = self.viewport_width.max(1);
        let total_chars = self.char_count();
        let mut rows = Vec::new();
        let mut segment_start = 0;
        let mut segment_width = 0;

        for (index, ch) in self.text.chars().enumerate() {
            if ch == '\n' {
                rows.push(VisualRow {
                    start: segment_start,
                    end: index,
                });
                segment_start = index + 1;
                segment_width = 0;
                continue;
            }

            let ch_width = char_display_width(ch);

            if segment_width > 0 && segment_width + ch_width > width {
                rows.push(VisualRow {
                    start: segment_start,
                    end: index,
                });
                segment_start = index;
                segment_width = 0;
            }

            segment_width += ch_width;
            if segment_width >= width {
                rows.push(VisualRow {
                    start: segment_start,
                    end: index + 1,
                });
                segment_start = index + 1;
                segment_width = 0;
            }
        }

        if total_chars == 0 || self.text.ends_with('\n') || segment_start < total_chars {
            rows.push(VisualRow {
                start: segment_start,
                end: total_chars,
            });
        }

        if rows.is_empty() {
            rows.push(VisualRow { start: 0, end: 0 });
        }

        rows
    }

    fn cursor_visual_position(&self) -> (usize, usize) {
        let rows = self.visual_rows();

        for (index, row) in rows.iter().enumerate() {
            if self.cursor_position < row.end {
                return (index, self.display_width_between(row.start, self.cursor_position));
            }

            if self.cursor_position == row.end {
                if let Some(next) = rows.get(index + 1)
                    && next.start == row.end
                {
                    continue;
                }

                return (index, self.display_width_between(row.start, row.end));
            }
        }

        let last_index = rows.len().saturating_sub(1);
        let last_row = rows[last_index];
        (last_index, self.display_width_between(last_row.start, last_row.end))
    }

    fn char_index_for_column(&self, row: VisualRow, target_column: usize) -> usize {
        let mut column = 0;

        for (offset, ch) in self
            .text
            .chars()
            .skip(row.start)
            .take(row.end.saturating_sub(row.start))
            .enumerate()
        {
            let ch_width = char_display_width(ch);
            let next_column = column + ch_width;

            if next_column > target_column {
                let distance_to_start = target_column.saturating_sub(column);
                let distance_to_end = next_column.saturating_sub(target_column);
                return if distance_to_start < distance_to_end {
                    row.start + offset
                } else {
                    row.start + offset + 1
                };
            }

            column = next_column;
        }

        row.end
    }

    fn display_width_between(&self, start: usize, end: usize) -> usize {
        self.text
            .chars()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(char_display_width)
            .sum()
    }

    fn slice_char_range(&self, start: usize, end: usize) -> String {
        self.text
            .chars()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect()
    }

    fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    fn char_to_byte_index(&self, char_index: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_index)
            .map(|(byte_index, _)| byte_index)
            .unwrap_or(self.text.len())
    }
}

fn char_display_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0).max(1)
}
