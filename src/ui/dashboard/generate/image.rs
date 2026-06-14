use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::output::MAX_IMAGE_BATCH_COUNT;
use crate::ratio::{self, ratio_option_display};

use super::fields::{IMAGE_FORM_HELP, InputList, SelectField, TextArea, TextInput, move_field_focus};
use super::layout::{error_line, param_line, render_media_panel, render_params_panel, render_text_box};
use super::preview::{build_image_preview, default_ratio_index, ratio_from_index};

pub const IMAGE_PARAM_COUNT: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImageSection {
    Text,
    Params,
    Media,
}

pub struct ImageForm {
    pub prompt: TextArea,
    pub ratio: SelectField,
    pub ratio_options: Vec<ratio::RatioOption>,
    pub count: SelectField,
    pub seed: TextInput,
    pub inputs: InputList,
    pub save_local: bool,
    pub output_dir: TextInput,
    section: ImageSection,
    param_focus: usize,
    pub editing: bool,
    pub show_help: bool,
    pub field_error: Option<String>,
}

impl ImageForm {
    pub fn new() -> Self {
        let ratio_options = ratio::image_ratio_options();
        let ratio_labels: Vec<String> = ratio_options.iter().map(ratio_option_display).collect();
        let ratio_index = default_ratio_index(&ratio_options, "1:1");
        let count_labels: Vec<String> = (1..=MAX_IMAGE_BATCH_COUNT).map(|n| n.to_string()).collect();
        Self {
            prompt: TextArea::new(),
            ratio: SelectField::new(ratio_labels, ratio_index),
            ratio_options,
            count: SelectField::new(count_labels, 0),
            seed: TextInput::empty(),
            inputs: InputList::new(),
            save_local: false,
            output_dir: TextInput::empty(),
            section: ImageSection::Text,
            param_focus: 0,
            editing: false,
            show_help: false,
            field_error: None,
        }
    }

    pub fn focus_prompt(&mut self) {
        self.section = ImageSection::Text;
        self.editing = true;
        self.field_error = None;
    }

    pub fn focus_params_section(&mut self) {
        self.section = ImageSection::Params;
        self.editing = false;
        self.inputs.adding = false;
    }

    pub fn focus_media_section(&mut self) {
        self.section = ImageSection::Media;
        self.editing = false;
        self.inputs.adding = false;
    }

    pub fn count_value(&self) -> u32 {
        self.count.current().parse().unwrap_or(1)
    }

    pub fn seed_value(&self) -> Option<u32> {
        if self.count_value() > 1 {
            return None;
        }
        let trimmed = self.seed.value.trim();
        if trimmed.is_empty() { None } else { trimmed.parse().ok() }
    }

    pub fn handle_key(
        &mut self,
        key: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> ImageFormAction {
        if key == crossterm::event::KeyCode::Char('?') {
            self.show_help = !self.show_help;
            return ImageFormAction::None;
        }

        if self.section == ImageSection::Media && self.inputs.adding {
            return self.handle_add_input_key(key, modifiers);
        }

        if self.editing {
            return self.handle_edit_key(key, modifiers);
        }

        match key {
            crossterm::event::KeyCode::Esc => ImageFormAction::Back,
            crossterm::event::KeyCode::Enter if modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                ImageFormAction::Submit
            }
            crossterm::event::KeyCode::Enter => match self.section {
                ImageSection::Text => {
                    self.editing = true;
                    ImageFormAction::None
                }
                ImageSection::Params if self.param_focus == IMAGE_PARAM_COUNT - 1 => ImageFormAction::Submit,
                ImageSection::Params => {
                    if self.param_focus == 3 {
                        self.save_local = !self.save_local;
                    } else {
                        self.editing = true;
                    }
                    ImageFormAction::None
                }
                ImageSection::Media => {
                    self.inputs.adding = true;
                    self.inputs.add_buffer = TextInput::empty();
                    ImageFormAction::None
                }
            },
            crossterm::event::KeyCode::Up if self.section == ImageSection::Params => {
                self.param_focus = move_field_focus(self.param_focus, IMAGE_PARAM_COUNT, -1);
                ImageFormAction::None
            }
            crossterm::event::KeyCode::Down if self.section == ImageSection::Params => {
                self.param_focus = move_field_focus(self.param_focus, IMAGE_PARAM_COUNT, 1);
                ImageFormAction::None
            }
            crossterm::event::KeyCode::Up if self.section == ImageSection::Media => {
                if self.inputs.selected > 0 {
                    self.inputs.selected -= 1;
                }
                ImageFormAction::None
            }
            crossterm::event::KeyCode::Down if self.section == ImageSection::Media => {
                if self.inputs.selected + 1 < self.inputs.items.len() {
                    self.inputs.selected += 1;
                }
                ImageFormAction::None
            }
            crossterm::event::KeyCode::Char(' ') if self.section == ImageSection::Params && self.param_focus == 3 => {
                self.save_local = !self.save_local;
                ImageFormAction::None
            }
            crossterm::event::KeyCode::Left if self.section == ImageSection::Params => {
                if self.param_focus == 0 {
                    self.ratio.previous();
                } else if self.param_focus == 1 {
                    self.count.previous();
                }
                ImageFormAction::None
            }
            crossterm::event::KeyCode::Right if self.section == ImageSection::Params => {
                if self.param_focus == 0 {
                    self.ratio.next();
                } else if self.param_focus == 1 {
                    self.count.next();
                }
                ImageFormAction::None
            }
            crossterm::event::KeyCode::Char('a') if self.section == ImageSection::Media => {
                self.inputs.adding = true;
                self.inputs.add_buffer = TextInput::empty();
                ImageFormAction::None
            }
            crossterm::event::KeyCode::Char('A') if self.section == ImageSection::Media => ImageFormAction::PickAsset,
            crossterm::event::KeyCode::Char('d') if self.section == ImageSection::Media => {
                self.inputs.remove_selected();
                ImageFormAction::None
            }
            crossterm::event::KeyCode::Char(c) if !c.is_control() && self.section == ImageSection::Text => {
                self.editing = true;
                self.prompt.push_char(c);
                ImageFormAction::None
            }
            crossterm::event::KeyCode::Backspace if self.section == ImageSection::Text => {
                self.editing = true;
                self.prompt.pop_char();
                ImageFormAction::None
            }
            _ => ImageFormAction::None,
        }
    }

    fn handle_edit_key(
        &mut self,
        key: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> ImageFormAction {
        if key == crossterm::event::KeyCode::Enter && modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
            self.editing = false;
            return ImageFormAction::Submit;
        }
        match self.section {
            ImageSection::Text => match key {
                crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Enter => self.editing = false,
                crossterm::event::KeyCode::Char(c) if !c.is_control() => self.prompt.push_char(c),
                crossterm::event::KeyCode::Backspace => self.prompt.pop_char(),
                crossterm::event::KeyCode::Left => self.prompt.move_left(),
                crossterm::event::KeyCode::Right => self.prompt.move_right(),
                _ => {}
            },
            ImageSection::Params => match key {
                crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Enter => self.editing = false,
                crossterm::event::KeyCode::Char(c) if !c.is_control() => match self.param_focus {
                    2 if self.count_value() == 1 => self.seed.push_char(c),
                    4 => self.output_dir.push_char(c),
                    _ => {}
                },
                crossterm::event::KeyCode::Backspace => match self.param_focus {
                    2 if self.count_value() == 1 => self.seed.pop_char(),
                    4 => self.output_dir.pop_char(),
                    _ => {}
                },
                _ => {}
            },
            ImageSection::Media => {}
        }
        ImageFormAction::None
    }

    fn handle_add_input_key(
        &mut self,
        key: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> ImageFormAction {
        if key == crossterm::event::KeyCode::Enter && modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
            self.inputs.adding = false;
            return ImageFormAction::Submit;
        }
        match key {
            crossterm::event::KeyCode::Esc => self.inputs.adding = false,
            crossterm::event::KeyCode::Enter => {
                let raw = self.inputs.add_buffer.value.trim().to_string();
                if !raw.is_empty() {
                    self.inputs.push_raw(raw, false);
                }
                self.inputs.adding = false;
            }
            crossterm::event::KeyCode::Char(c) if !c.is_control() => self.inputs.add_buffer.push_char(c),
            crossterm::event::KeyCode::Backspace => self.inputs.add_buffer.pop_char(),
            _ => {}
        }
        ImageFormAction::None
    }

    pub fn validate(&mut self) -> Result<(), String> {
        self.field_error = None;
        if self.prompt.text().trim().is_empty() {
            self.field_error = Some("prompt is required".into());
            self.section = ImageSection::Text;
            return Err(self.field_error.clone().unwrap());
        }
        if self.count_value() > 1 && !self.seed.value.trim().is_empty() {
            self.field_error = Some("seed cannot be used with count > 1".into());
            self.section = ImageSection::Params;
            self.param_focus = 2;
            return Err(self.field_error.clone().unwrap());
        }
        if let Err(err) = ratio_from_index(&self.ratio_options, self.ratio.index) {
            self.field_error = Some(format!("{err:#}"));
            self.section = ImageSection::Params;
            self.param_focus = 0;
            return Err(self.field_error.clone().unwrap());
        }
        Ok(())
    }

    pub fn add_asset_input(&mut self, asset_uri: String) {
        self.inputs.push_raw(asset_uri, false);
        self.section = ImageSection::Media;
    }

    fn param_lines(&self) -> Vec<Line<'static>> {
        let preview = build_image_preview(
            &self.ratio_options,
            self.ratio.index,
            self.inputs.items.len(),
            self.count_value(),
            &self.seed.value,
        );
        let pf = |i: usize| self.section == ImageSection::Params && self.param_focus == i;
        let seed_display = if self.count_value() > 1 {
            "(disabled with batch)".into()
        } else if self.section == ImageSection::Params && self.editing && self.param_focus == 2 {
            self.seed.display_with_cursor(true)
        } else if self.seed.value.trim().is_empty() {
            "random".into()
        } else {
            self.seed.value.clone()
        };
        let out_display = if self.section == ImageSection::Params && self.editing && self.param_focus == 4 {
            self.output_dir.display_with_cursor(true)
        } else if self.output_dir.value.trim().is_empty() {
            "(config default)".into()
        } else {
            self.output_dir.value.clone()
        };
        let mut lines = vec![
            param_line(
                "Ratio",
                pf(0),
                self.section == ImageSection::Params,
                self.ratio.current(),
            ),
            param_line(
                "Size",
                pf(0),
                self.section == ImageSection::Params,
                &format!("{} ({})", preview.size, preview.tier),
            ),
            param_line(
                "Count",
                pf(1),
                self.section == ImageSection::Params,
                self.count.current(),
            ),
            param_line("Seed", pf(2), self.section == ImageSection::Params, &seed_display),
            param_line(
                "Save",
                pf(3),
                self.section == ImageSection::Params,
                if self.save_local { "[x] yes" } else { "[ ] no" },
            ),
            param_line("Output", pf(4), self.section == ImageSection::Params, &out_display),
        ];
        if let Some(ref err) = self.field_error {
            lines.push(error_line(err));
        }
        lines
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, form_focused: bool) {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(if self.show_help { 2 } else { 1 })])
            .split(area);

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
            .split(outer[0]);

        render_text_box(
            frame,
            columns[0],
            "Prompt *",
            form_focused && self.section == ImageSection::Text,
            form_focused && self.editing && self.section == ImageSection::Text,
            &self.prompt,
        );

        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(8), Constraint::Min(4)])
            .split(columns[1]);

        render_params_panel(
            frame,
            right[0],
            form_focused && self.section == ImageSection::Params,
            "Parameters",
            self.param_lines(),
        );

        render_media_panel(
            frame,
            right[1],
            form_focused && self.section == ImageSection::Media,
            &self.inputs,
            self.inputs.adding,
            "a add  A asset  d del  ↑↓ select",
        );

        let help = if self.show_help {
            format!("{IMAGE_FORM_HELP}  ? help")
        } else {
            IMAGE_FORM_HELP.to_string()
        };
        frame.render_widget(
            Paragraph::new(help).style(Style::default().fg(Color::DarkGray)),
            outer[1],
        );
    }
}

pub enum ImageFormAction {
    None,
    Back,
    Submit,
    PickAsset,
}
