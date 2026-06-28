use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::text::Line;

#[derive(Debug, Clone, Default)]
pub(crate) struct ScrollView {
    pub from_bottom: usize,
    pub pinned_to_bottom: bool,
    pub viewport: u16,
}

impl ScrollView {
    pub fn on_new_content(&mut self) {
        if self.pinned_to_bottom {
            self.from_bottom = 0;
        }
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.pinned_to_bottom = false;
        self.from_bottom = self.from_bottom.saturating_add(lines);
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.from_bottom = self.from_bottom.saturating_sub(lines);
        if self.from_bottom == 0 {
            self.pinned_to_bottom = true;
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        self.from_bottom = 0;
        self.pinned_to_bottom = true;
    }

    pub fn handle_key(&mut self, key: KeyCode, mods: KeyModifiers) {
        let page = self.viewport.max(1) as usize;
        match key {
            KeyCode::PageUp => self.scroll_up(page),
            KeyCode::PageDown => self.scroll_down(page),
            KeyCode::Up if mods.contains(KeyModifiers::CONTROL) => self.scroll_up(1),
            KeyCode::Down if mods.contains(KeyModifiers::CONTROL) => self.scroll_down(1),
            KeyCode::End if mods.contains(KeyModifiers::CONTROL) => self.scroll_to_bottom(),
            KeyCode::End => {}
            _ => {}
        }
    }
}

pub(crate) fn slice_transcript_view(
    lines: Vec<Line<'static>>,
    viewport_h: usize,
    scroll_from_bottom: usize,
) -> Vec<Line<'static>> {
    let total = lines.len();
    let end = total.saturating_sub(scroll_from_bottom);
    let start = end.saturating_sub(viewport_h);
    lines[start..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_transcript_respects_scroll_offset() {
        let lines: Vec<Line> = (0..10).map(|idx| Line::from(format!("line {idx}"))).collect();

        let visible = slice_transcript_view(lines, 3, 0);
        assert_eq!(visible.len(), 3);
        assert!(visible[2].to_string().contains("line 9"));

        let visible = slice_transcript_view((0..10).map(|idx| Line::from(format!("line {idx}"))).collect(), 3, 2);
        assert!(visible[2].to_string().contains("line 7"));
    }

    #[test]
    fn pinned_to_bottom_resets_on_new_content() {
        let mut scroll = ScrollView { pinned_to_bottom: true, from_bottom: 5, ..Default::default() };
        scroll.on_new_content();
        assert_eq!(scroll.from_bottom, 0);
    }

    #[test]
    fn unpinned_keeps_offset_on_new_content() {
        let mut scroll = ScrollView { pinned_to_bottom: false, from_bottom: 5, ..Default::default() };
        scroll.on_new_content();
        assert_eq!(scroll.from_bottom, 5);
    }
}
