use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::ratio;

use super::fields::{InputList, SelectField, TextArea, TextInput, VIDEO_FORM_HELP, move_field_focus};
use super::layout::{
    error_line, param_line, render_media_panel, render_params_panel, render_task_strip, render_text_box,
};
use super::preview::{build_video_preview, default_ratio_index, ratio_from_index};
use crate::ui::dashboard::task_display::TaskStripData;

pub const VIDEO_PARAM_COUNT: usize = 9;

#[derive(Clone, Copy, PartialEq, Eq)]
enum VideoSection {
    Text,
    Params,
    Media,
}

pub struct VideoForm {
    pub prompt: TextArea,
    pub ratio: SelectField,
    pub ratio_options: Vec<ratio::RatioOption>,
    pub duration: TextInput,
    pub size: SelectField,
    pub model: SelectField,
    pub inputs: InputList,
    pub seed: TextInput,
    pub async_mode: bool,
    pub save_local: bool,
    pub output_dir: TextInput,
    section: VideoSection,
    param_focus: usize,
    pub editing: bool,
    pub show_help: bool,
    pub field_error: Option<String>,
    pub chain_uri: Option<String>,
}

impl VideoForm {
    pub fn new() -> Self {
        let ratio_options = ratio::video_ratio_options();
        let ratio_labels: Vec<String> = ratio_options.iter().map(|option| option.label.clone()).collect();
        let ratio_index = default_ratio_index(&ratio_options, "16:9");
        let configured = crate::config::AppConfig::load().unwrap_or_default().video_model;
        let model_index = usize::from(configured == "agnes-video-2.5-flash");
        let mut form = Self {
            prompt: TextArea::new(),
            ratio: SelectField::new(ratio_labels, ratio_index),
            ratio_options,
            duration: TextInput::new("5"),
            size: SelectField::new(vec!["720P".into(), "1080P".into(), "1K".into(), "2K".into()], 0),
            model: SelectField::new(
                vec!["agnes-video-2.5".into(), "agnes-video-2.5-flash".into()],
                model_index,
            ),
            inputs: InputList::new(),
            seed: TextInput::empty(),
            async_mode: true,
            save_local: false,
            output_dir: TextInput::empty(),
            section: VideoSection::Text,
            param_focus: 0,
            editing: false,
            show_help: false,
            field_error: None,
            chain_uri: None,
        };
        form.sync_sizes();
        form
    }

    pub fn duration_value(&self) -> f64 {
        self.duration.value.trim().parse().unwrap_or(f64::NAN)
    }

    fn sync_sizes(&mut self) {
        let previous = self.size.current().to_string();
        let options: Vec<String> = ratio::video_size_options(self.model.current())
            .iter()
            .map(|s| s.to_string())
            .collect();
        let index = options.iter().position(|size| size == &previous).unwrap_or(0);
        self.size = SelectField::new(options, index);
    }

    pub fn seed_value(&self) -> Option<u32> {
        let trimmed = self.seed.value.trim();
        if trimmed.is_empty() { None } else { trimmed.parse().ok() }
    }

    pub fn set_chain_uri(&mut self, uri: Option<String>) {
        self.chain_uri = uri;
    }

    pub fn chain_from_last_result(&mut self) {
        if let Some(uri) = self.chain_uri.clone() {
            self.inputs.push_raw(uri, true);
            self.section = VideoSection::Media;
        }
    }

    pub fn focus_prompt(&mut self) {
        self.section = VideoSection::Text;
        self.editing = true;
        self.field_error = None;
    }

    pub fn focus_params_section(&mut self) {
        self.section = VideoSection::Params;
        self.editing = false;
        self.inputs.adding = false;
    }

    pub fn focus_media_section(&mut self) {
        self.section = VideoSection::Media;
        self.editing = false;
        self.inputs.adding = false;
    }

    fn active_text_area(&mut self) -> &mut TextArea {
        &mut self.prompt
    }

    pub fn handle_key(
        &mut self,
        key: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> VideoFormAction {
        if key == crossterm::event::KeyCode::Char('?') {
            self.show_help = !self.show_help;
            return VideoFormAction::None;
        }

        if self.section == VideoSection::Media && self.inputs.adding {
            return self.handle_add_input_key(key, modifiers);
        }

        if self.editing {
            return self.handle_edit_key(key, modifiers);
        }

        match key {
            crossterm::event::KeyCode::Esc => VideoFormAction::Back,
            crossterm::event::KeyCode::Char('t') if self.section != VideoSection::Text => VideoFormAction::OpenTasks,
            crossterm::event::KeyCode::Char('r') if self.section != VideoSection::Text => VideoFormAction::RefreshTasks,
            crossterm::event::KeyCode::Char('g') if self.section != VideoSection::Text => VideoFormAction::GoRunning,
            crossterm::event::KeyCode::Enter if modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                VideoFormAction::Submit
            }
            crossterm::event::KeyCode::Enter => match self.section {
                VideoSection::Text => {
                    self.editing = true;
                    VideoFormAction::None
                }
                VideoSection::Params if self.param_focus == VIDEO_PARAM_COUNT - 1 => VideoFormAction::Submit,
                VideoSection::Params => {
                    if self.param_focus == 4 || self.param_focus == 5 {
                        if self.param_focus == 4 {
                            self.async_mode = !self.async_mode;
                        } else {
                            self.save_local = !self.save_local;
                        }
                    } else if matches!(self.param_focus, 0 | 2 | 7) {
                        self.cycle_selection(true);
                    } else {
                        self.editing = true;
                    }
                    VideoFormAction::None
                }
                VideoSection::Media => {
                    self.inputs.adding = true;
                    self.inputs.add_buffer = TextInput::empty();
                    VideoFormAction::None
                }
            },
            crossterm::event::KeyCode::Up if self.section == VideoSection::Params => {
                self.param_focus = move_field_focus(self.param_focus, VIDEO_PARAM_COUNT, -1);
                VideoFormAction::None
            }
            crossterm::event::KeyCode::Down if self.section == VideoSection::Params => {
                self.param_focus = move_field_focus(self.param_focus, VIDEO_PARAM_COUNT, 1);
                VideoFormAction::None
            }
            crossterm::event::KeyCode::Up if self.section == VideoSection::Media => {
                if self.inputs.selected > 0 {
                    self.inputs.selected -= 1;
                }
                VideoFormAction::None
            }
            crossterm::event::KeyCode::Down if self.section == VideoSection::Media => {
                if self.inputs.selected + 1 < self.inputs.items.len() {
                    self.inputs.selected += 1;
                }
                VideoFormAction::None
            }
            crossterm::event::KeyCode::Char(' ')
                if self.section == VideoSection::Params && (self.param_focus == 4 || self.param_focus == 5) =>
            {
                if self.param_focus == 4 {
                    self.async_mode = !self.async_mode;
                } else {
                    self.save_local = !self.save_local;
                }
                VideoFormAction::None
            }
            crossterm::event::KeyCode::Left
                if self.section == VideoSection::Params && matches!(self.param_focus, 0 | 2 | 7) =>
            {
                self.cycle_selection(false);
                VideoFormAction::None
            }
            crossterm::event::KeyCode::Right
                if self.section == VideoSection::Params && matches!(self.param_focus, 0 | 2 | 7) =>
            {
                self.cycle_selection(true);
                VideoFormAction::None
            }
            crossterm::event::KeyCode::Char('a') if self.section == VideoSection::Media => {
                self.inputs.adding = true;
                self.inputs.add_buffer = TextInput::empty();
                VideoFormAction::None
            }
            crossterm::event::KeyCode::Char('A') if self.section == VideoSection::Media => VideoFormAction::PickAsset,
            crossterm::event::KeyCode::Char('c') if self.section == VideoSection::Media => {
                self.chain_from_last_result();
                VideoFormAction::None
            }
            crossterm::event::KeyCode::Char('d') if self.section == VideoSection::Media => {
                self.inputs.remove_selected();
                VideoFormAction::None
            }
            crossterm::event::KeyCode::Char(c) if !c.is_control() && self.section == VideoSection::Text => {
                self.editing = true;
                self.active_text_area().push_char(c);
                VideoFormAction::None
            }
            crossterm::event::KeyCode::Backspace if self.section == VideoSection::Text => {
                self.editing = true;
                self.active_text_area().pop_char();
                VideoFormAction::None
            }
            _ => VideoFormAction::None,
        }
    }

    fn cycle_selection(&mut self, forward: bool) {
        let field = match self.param_focus {
            0 => &mut self.ratio,
            2 => &mut self.size,
            7 => &mut self.model,
            _ => return,
        };
        if forward {
            field.next();
        } else {
            field.previous();
        }
        if self.param_focus == 7 {
            self.sync_sizes();
        }
    }

    fn handle_edit_key(
        &mut self,
        key: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> VideoFormAction {
        if key == crossterm::event::KeyCode::Enter && modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
            self.editing = false;
            return VideoFormAction::Submit;
        }
        match self.section {
            VideoSection::Text => match key {
                crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Enter => self.editing = false,
                crossterm::event::KeyCode::Char(c) if !c.is_control() => self.active_text_area().push_char(c),
                crossterm::event::KeyCode::Backspace => self.active_text_area().pop_char(),
                crossterm::event::KeyCode::Left => self.active_text_area().move_left(),
                crossterm::event::KeyCode::Right => self.active_text_area().move_right(),
                _ => {}
            },
            VideoSection::Params => match key {
                crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Enter => self.editing = false,
                crossterm::event::KeyCode::Char(c) if !c.is_control() => match self.param_focus {
                    1 => self.duration.push_char(c),
                    3 => self.seed.push_char(c),
                    6 => self.output_dir.push_char(c),
                    _ => {}
                },
                crossterm::event::KeyCode::Backspace => match self.param_focus {
                    1 => self.duration.pop_char(),
                    3 => self.seed.pop_char(),
                    6 => self.output_dir.pop_char(),
                    _ => {}
                },
                _ => {}
            },
            VideoSection::Media => {}
        }
        VideoFormAction::None
    }

    fn handle_add_input_key(
        &mut self,
        key: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> VideoFormAction {
        if key == crossterm::event::KeyCode::Enter && modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
            self.inputs.adding = false;
            return VideoFormAction::Submit;
        }
        match key {
            crossterm::event::KeyCode::Esc => self.inputs.adding = false,
            crossterm::event::KeyCode::Enter => {
                let raw = self.inputs.add_buffer.value.trim().to_string();
                if !raw.is_empty() {
                    self.inputs.push_raw(raw, true);
                }
                self.inputs.adding = false;
            }
            crossterm::event::KeyCode::Char(c) if !c.is_control() => self.inputs.add_buffer.push_char(c),
            crossterm::event::KeyCode::Backspace => self.inputs.add_buffer.pop_char(),
            _ => {}
        }
        VideoFormAction::None
    }

    pub fn validate(&mut self) -> Result<(), String> {
        self.field_error = None;
        if self.prompt.text().trim().is_empty() {
            self.field_error = Some("prompt is required".into());
            self.section = VideoSection::Text;
            return Err(self.field_error.clone().unwrap());
        }
        for (index, item) in self.inputs.items.iter().enumerate() {
            let valid = if let Some(id) = item.raw.strip_prefix("asset://") {
                !id.trim().is_empty()
            } else {
                crate::media::resolve_video_image_input(&item.raw).is_ok()
            };
            if !valid {
                self.field_error = Some(format!(
                    "invalid video image {}: use an HTTPS URL, asset://, local image, Base64, or Data URI",
                    index + 1
                ));
                self.section = VideoSection::Media;
                return Err(self.field_error.clone().unwrap());
            }
        }
        if let Err(err) = ratio_from_index(&self.ratio_options, self.ratio.index) {
            self.field_error = Some(format!("{err:#}"));
            self.section = VideoSection::Params;
            self.param_focus = 0;
            return Err(self.field_error.clone().unwrap());
        }
        if let Err(err) = ratio::validate_video_duration(self.duration_value()) {
            self.field_error = Some(format!("{err:#}"));
            self.section = VideoSection::Params;
            self.param_focus = 1;
            return Err(self.field_error.clone().unwrap());
        }
        if !self.seed.value.trim().is_empty() && self.seed_value().is_none() {
            self.field_error = Some("seed must be an unsigned 32-bit integer".into());
            self.section = VideoSection::Params;
            self.param_focus = 3;
            return Err(self.field_error.clone().unwrap());
        }
        Ok(())
    }

    pub fn add_asset_input(&mut self, asset_uri: String) {
        self.inputs.push_raw(asset_uri, true);
        self.section = VideoSection::Media;
    }

    fn param_lines(&self) -> Vec<Line<'static>> {
        let preview = build_video_preview(
            &self.ratio_options,
            self.ratio.index,
            &self.duration.value,
            self.size.current(),
            self.model.current(),
            self.inputs.items.len(),
        );
        let pf = |i: usize| self.section == VideoSection::Params && self.param_focus == i;
        let ratio_display = self.ratio.current().to_string();
        let duration_display = if self.section == VideoSection::Params && self.editing && self.param_focus == 1 {
            self.duration.display_with_cursor(true)
        } else {
            self.duration.value.clone()
        };
        let seed_display = if self.section == VideoSection::Params && self.editing && self.param_focus == 3 {
            self.seed.display_with_cursor(true)
        } else if self.seed.value.trim().is_empty() {
            "omitted".into()
        } else {
            self.seed.value.clone()
        };
        let out_display = if self.section == VideoSection::Params && self.editing && self.param_focus == 6 {
            self.output_dir.display_with_cursor(true)
        } else if self.output_dir.value.trim().is_empty() {
            "(config default)".into()
        } else {
            self.output_dir.value.clone()
        };
        let mut lines = vec![
            param_line("Ratio", pf(0), self.section == VideoSection::Params, &ratio_display),
            param_line(
                "Duration",
                pf(1),
                self.section == VideoSection::Params,
                &format!(
                    "{duration_display}s · {} · max {}",
                    preview.timing, preview.max_duration
                ),
            ),
            param_line(
                "Size",
                pf(2),
                self.section == VideoSection::Params,
                &format!("{} ({}) · {}", preview.size, preview.tier, preview.input_note),
            ),
            param_line("Seed", pf(3), self.section == VideoSection::Params, &seed_display),
            param_line(
                "Async",
                pf(4),
                self.section == VideoSection::Params,
                if self.async_mode {
                    "[x] submit task (poll in Tasks)"
                } else {
                    "[ ] sync wait (result below)"
                },
            ),
            param_line(
                "Save",
                pf(5),
                self.section == VideoSection::Params,
                if self.save_local { "[x] yes" } else { "[ ] no" },
            ),
            param_line("Output", pf(6), self.section == VideoSection::Params, &out_display),
            param_line(
                "Model",
                pf(7),
                self.section == VideoSection::Params,
                self.model.current(),
            ),
            param_line("Generate", pf(8), self.section == VideoSection::Params, "Enter / F5"),
        ];
        if let Some(ref err) = self.field_error {
            lines.push(error_line(err));
        } else if let Some(ref err) = preview.error {
            lines.push(error_line(err));
        }
        lines
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, task_strip: &TaskStripData, form_focused: bool) {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(if self.show_help { 2 } else { 1 })])
            .split(area);

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
            .split(outer[0]);

        render_text_box(
            frame,
            columns[0],
            "Prompt *",
            form_focused && self.section == VideoSection::Text,
            form_focused && self.editing && self.section == VideoSection::Text,
            &self.prompt,
        );

        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(12), Constraint::Length(7), Constraint::Min(4)])
            .split(columns[1]);

        render_params_panel(
            frame,
            right[0],
            form_focused && self.section == VideoSection::Params,
            "Parameters",
            self.param_lines(),
        );

        render_task_strip(frame, right[1], task_strip);

        render_media_panel(
            frame,
            right[2],
            form_focused && self.section == VideoSection::Media,
            &self.inputs,
            self.inputs.adding,
            "a add  A asset  c chain  d del  ↑↓ select",
        );

        let help = if self.show_help {
            format!("{VIDEO_FORM_HELP}  ? help")
        } else {
            VIDEO_FORM_HELP.to_string()
        };
        frame.render_widget(
            Paragraph::new(help).style(Style::default().fg(Color::DarkGray)),
            outer[1],
        );
    }
}

pub enum VideoFormAction {
    None,
    Back,
    Submit,
    PickAsset,
    OpenTasks,
    RefreshTasks,
    GoRunning,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_stays_selectable_for_reference_images() {
        let mut form = VideoForm::new();
        assert!(!form.ratio.disabled);
        form.inputs.push_raw("https://example.com/a.png".into(), true);
        assert!(!form.ratio.disabled);
    }

    #[test]
    fn flash_model_resets_unsupported_resolution() {
        let mut form = VideoForm::new();
        form.model.index = 0;
        form.sync_sizes();
        form.size.index = 3;
        form.param_focus = 7;
        form.cycle_selection(true);
        assert_eq!(form.model.current(), "agnes-video-2.5-flash");
        assert_eq!(form.size.options, ["720P"]);
    }

    #[test]
    fn invalid_duration_and_seed_stay_on_form() {
        let mut form = VideoForm::new();
        form.prompt.push_char('x');
        form.duration = TextInput::new("invalid");
        assert!(form.validate().is_err());
        form.duration = TextInput::new("5");
        form.seed = TextInput::new("-1");
        assert!(form.validate().is_err());
    }

    #[test]
    fn async_mode_defaults_on() {
        assert!(VideoForm::new().async_mode);
    }

    #[test]
    fn video_form_revalidates_inline_images_without_dumping_payloads() {
        use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let mut form = VideoForm::new();
        form.prompt.push_char('x');
        form.inputs
            .push_raw(format!("data:image/png;base64,{}", B64.encode(png)), true);
        assert!(form.validate().is_ok());

        form.inputs.items[0].raw = "data:image/jpeg;base64,SECRET_INVALID_BYTES".into();
        let error = form.validate().unwrap_err();
        assert!(error.contains("invalid video image 1"));
        assert!(!error.contains("SECRET_INVALID_BYTES"));
        assert!(!error.contains("data:image"));
    }
}
