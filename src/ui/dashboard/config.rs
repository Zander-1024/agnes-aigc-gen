use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::config::AppConfig;

use super::generate::fields::{TextInput, form_field_line, move_field_focus};

pub const CONFIG_FIELD_COUNT: usize = 8;

pub struct ConfigForm {
    pub config: AppConfig,
    pub api_key: TextInput,
    pub base_url: TextInput,
    pub image_model: TextInput,
    pub video_model: TextInput,
    pub output_dir: TextInput,
    pub max_retries: TextInput,
    pub save_local: bool,
    pub focus: usize,
    pub editing: bool,
    pub dirty: bool,
    pub message: Option<String>,
}

impl ConfigForm {
    pub fn from_config(config: AppConfig) -> Self {
        Self {
            base_url: TextInput::new(config.base_url.clone()),
            image_model: TextInput::new(config.image_model.clone()),
            video_model: TextInput::new(config.video_model.clone()),
            output_dir: TextInput::new(config.output_dir.clone()),
            max_retries: TextInput::new(config.max_retries.to_string()),
            save_local: config.save_local,
            api_key: TextInput::empty(),
            config,
            focus: 0,
            editing: false,
            dirty: false,
            message: None,
        }
    }

    pub fn reload(&mut self) {
        if let Ok(cfg) = AppConfig::load() {
            *self = Self::from_config(cfg);
        }
    }

    pub fn save(&mut self) -> Result<(), String> {
        let mut cfg = self.config.clone();
        if !self.api_key.value.trim().is_empty() {
            cfg.apply_key("api-key", self.api_key.value.trim())
                .map_err(|e| format!("{e:#}"))?;
        }
        cfg.apply_key("base-url", self.base_url.value.trim())
            .map_err(|e| format!("{e:#}"))?;
        cfg.apply_key("image-model", self.image_model.value.trim())
            .map_err(|e| format!("{e:#}"))?;
        cfg.apply_key("video-model", self.video_model.value.trim())
            .map_err(|e| format!("{e:#}"))?;
        cfg.apply_key("output-dir", self.output_dir.value.trim())
            .map_err(|e| format!("{e:#}"))?;
        cfg.apply_key("max-retries", self.max_retries.value.trim())
            .map_err(|e| format!("{e:#}"))?;
        cfg.apply_key("save-local", if self.save_local { "true" } else { "false" })
            .map_err(|e| format!("{e:#}"))?;
        cfg.save().map_err(|e| format!("{e:#}"))?;
        self.config = cfg;
        self.dirty = false;
        self.message = Some("saved".into());
        Ok(())
    }

    pub fn handle_key(
        &mut self,
        key: crossterm::event::KeyCode,
        _modifiers: crossterm::event::KeyModifiers,
    ) -> ConfigAction {
        if self.editing {
            return self.handle_edit_key(key);
        }
        match key {
            crossterm::event::KeyCode::Esc => ConfigAction::Back,
            crossterm::event::KeyCode::Up | crossterm::event::KeyCode::BackTab => {
                self.focus = move_field_focus(self.focus, CONFIG_FIELD_COUNT, -1);
                ConfigAction::None
            }
            crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Tab => {
                self.focus = move_field_focus(self.focus, CONFIG_FIELD_COUNT, 1);
                ConfigAction::None
            }
            crossterm::event::KeyCode::Enter if self.focus == 7 => ConfigAction::Save,
            crossterm::event::KeyCode::Enter if self.focus != 7 => {
                self.editing = true;
                ConfigAction::None
            }
            crossterm::event::KeyCode::Char(' ') if self.focus == 6 => {
                self.save_local = !self.save_local;
                self.dirty = true;
                ConfigAction::None
            }
            crossterm::event::KeyCode::Char('s') => ConfigAction::Save,
            _ => ConfigAction::None,
        }
    }

    fn handle_edit_key(&mut self, key: crossterm::event::KeyCode) -> ConfigAction {
        match key {
            crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Enter => {
                self.editing = false;
                self.dirty = true;
                ConfigAction::None
            }
            crossterm::event::KeyCode::Char(c) if !c.is_control() => {
                match self.focus {
                    0 => self.api_key.push_char(c),
                    1 => self.base_url.push_char(c),
                    2 => self.image_model.push_char(c),
                    3 => self.video_model.push_char(c),
                    4 => self.output_dir.push_char(c),
                    5 => self.max_retries.push_char(c),
                    _ => {}
                }
                ConfigAction::None
            }
            crossterm::event::KeyCode::Backspace => {
                match self.focus {
                    0 => self.api_key.pop_char(),
                    1 => self.base_url.pop_char(),
                    2 => self.image_model.pop_char(),
                    3 => self.video_model.pop_char(),
                    4 => self.output_dir.pop_char(),
                    5 => self.max_retries.pop_char(),
                    _ => {}
                }
                ConfigAction::None
            }
            _ => ConfigAction::None,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let api_display = if self.config.api_key_encrypted.is_some() {
            "<configured — enter to replace>"
        } else {
            "<not set>"
        };
        let resolved = self
            .config
            .resolved_output_dir()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".into());

        let mut lines = vec![
            form_field_line(
                "API key",
                self.focus == 0,
                self.editing && self.focus == 0,
                &if self.editing && self.focus == 0 {
                    self.api_key.display_with_cursor(true)
                } else if self.api_key.value.is_empty() {
                    api_display.to_string()
                } else {
                    "********".into()
                },
            ),
            form_field_line(
                "Base URL",
                self.focus == 1,
                self.editing && self.focus == 1,
                &self.base_url.display_with_cursor(self.editing && self.focus == 1),
            ),
            form_field_line(
                "Image model",
                self.focus == 2,
                self.editing && self.focus == 2,
                &self.image_model.display_with_cursor(self.editing && self.focus == 2),
            ),
            form_field_line(
                "Video model",
                self.focus == 3,
                self.editing && self.focus == 3,
                &self.video_model.display_with_cursor(self.editing && self.focus == 3),
            ),
            form_field_line(
                "Output dir",
                self.focus == 4,
                self.editing && self.focus == 4,
                &self.output_dir.display_with_cursor(self.editing && self.focus == 4),
            ),
            Line::from(format!("  resolved: {resolved}")),
            form_field_line(
                "Max retries",
                self.focus == 5,
                self.editing && self.focus == 5,
                &self.max_retries.display_with_cursor(self.editing && self.focus == 5),
            ),
            form_field_line(
                "Save local",
                self.focus == 6,
                false,
                if self.save_local { "[x] true" } else { "[ ] false" },
            ),
            form_field_line("Actions", self.focus == 7, false, "Enter/s = save"),
            Line::from(""),
            Line::from(Span::styled(
                "Chat models: use CLI `config set`",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        if let Some(ref msg) = self.message {
            lines.push(Line::from(Span::styled(msg.clone(), Style::default().fg(Color::Green))));
        }
        if self.dirty {
            lines.push(Line::from(Span::styled(
                "unsaved changes",
                Style::default().fg(Color::Yellow),
            )));
        }

        frame.render_widget(
            Paragraph::new(lines)
                .block(Block::default().title("Settings").borders(Borders::ALL))
                .wrap(Wrap { trim: true }),
            area,
        );
    }
}

pub enum ConfigAction {
    None,
    Back,
    Save,
}
