use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::DefaultTerminal;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::agent::{LocalTaskRouter, TaskRouter};
use crate::config::AppConfig;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Generate,
    Config,
    Sessions,
    Chat,
}

pub struct App {
    page: Page,
    status: String,
    prompt: String,
    ratio: String,
    kind: GenKind,
    input_paths: String,
    duration: String,
    config: AppConfig,
    sessions: Vec<String>,
    chat_placeholder: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GenKind {
    Image,
    Video,
}

impl App {
    fn new() -> Result<Self> {
        let cfg = AppConfig::load()?;
        let sessions = list_sessions(&cfg)?;
        Ok(Self {
            page: Page::Home,
            status: "Ready".into(),
            prompt: String::new(),
            ratio: "16:9".into(),
            kind: GenKind::Image,
            input_paths: String::new(),
            duration: "5".into(),
            config: cfg,
            sessions,
            chat_placeholder: "Chat with pi_agent_rust — coming in Phase 2".into(),
        })
    }

    fn on_tick(&mut self) {}

    fn handle_key(&mut self, key: KeyCode, mods: KeyModifiers) -> bool {
        if key == KeyCode::Char('q') && mods.contains(KeyModifiers::CONTROL) {
            return true;
        }
        if key == KeyCode::Char('q') && self.page == Page::Home {
            return true;
        }
        match self.page {
            Page::Home => self.handle_home(key),
            Page::Generate => self.handle_generate(key),
            Page::Config => self.handle_nav(key),
            Page::Sessions => self.handle_nav(key),
            Page::Chat => self.handle_nav(key),
        }
        false
    }

    fn handle_home(&mut self, key: KeyCode) {
        self.page = match key {
            KeyCode::Char('1') => Page::Generate,
            KeyCode::Char('2') => {
                self.kind = GenKind::Image;
                Page::Generate
            }
            KeyCode::Char('3') => {
                self.kind = GenKind::Video;
                Page::Generate
            }
            KeyCode::Char('c') => Page::Config,
            KeyCode::Char('s') => {
                if let Ok(items) = list_sessions(&self.config) {
                    self.sessions = items;
                }
                Page::Sessions
            }
            KeyCode::Char('h') => Page::Chat,
            _ => self.page,
        };
    }

    fn handle_nav(&mut self, key: KeyCode) {
        if key == KeyCode::Esc {
            self.page = Page::Home;
        }
    }

    fn handle_generate(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => self.page = Page::Home,
            KeyCode::Tab => {
                self.kind = match self.kind {
                    GenKind::Image => GenKind::Video,
                    GenKind::Video => GenKind::Image,
                };
            }
            KeyCode::Char(c) if !c.is_control() => self.prompt.push(c),
            KeyCode::Backspace => {
                self.prompt.pop();
            }
            KeyCode::Enter => {
                if let Err(e) = self.submit_generate() {
                    self.status = format!("Error: {e:#}");
                }
            }
            _ => {}
        }
    }

    fn submit_generate(&mut self) -> Result<()> {
        if self.prompt.trim().is_empty() {
            anyhow::bail!("prompt is empty");
        }
        self.status = "Generating...".into();
        let router = LocalTaskRouter;
        let inputs: Vec<String> = self
            .input_paths
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let result = match self.kind {
            GenKind::Image => router.route_image(&self.prompt, &self.ratio, &inputs)?,
            GenKind::Video => {
                let duration: f64 = self.duration.parse().unwrap_or(5.0);
                router.route_video(&self.prompt, &self.ratio, duration, &inputs)?
            }
        };
        self.status = format!("Done: {}", result.uri);
        if let Ok(items) = list_sessions(&self.config) {
            self.sessions = items;
        }
        Ok(())
    }

    fn render(&self, frame: &mut ratatui::Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
            .split(frame.area());

        match self.page {
            Page::Home => self.render_home(frame, chunks[0]),
            Page::Generate => self.render_generate(frame, chunks[0]),
            Page::Config => self.render_config(frame, chunks[0]),
            Page::Sessions => self.render_sessions(frame, chunks[0]),
            Page::Chat => self.render_chat(frame, chunks[0]),
        }

        let status = Paragraph::new(self.status.as_str()).block(Block::default().borders(Borders::ALL).title("Status"));
        frame.render_widget(status, chunks[1]);
    }

    fn render_home(&self, frame: &mut ratatui::Frame, area: Rect) {
        let text = vec![
            Line::from("Agnes AIGC — Dashboard"),
            Line::from(""),
            Line::from(vec![
                Span::styled("1", Style::default().fg(Color::Cyan)),
                Span::raw(" Generate (default)"),
            ]),
            Line::from(vec![
                Span::styled("2", Style::default().fg(Color::Cyan)),
                Span::raw(" Generate Image"),
            ]),
            Line::from(vec![
                Span::styled("3", Style::default().fg(Color::Cyan)),
                Span::raw(" Generate Video"),
            ]),
            Line::from(vec![
                Span::styled("c", Style::default().fg(Color::Cyan)),
                Span::raw(" Config"),
            ]),
            Line::from(vec![
                Span::styled("s", Style::default().fg(Color::Cyan)),
                Span::raw(" Sessions (output dir)"),
            ]),
            Line::from(vec![
                Span::styled("h", Style::default().fg(Color::Cyan)),
                Span::raw(" Chat (Phase 2)"),
            ]),
            Line::from(""),
            Line::from("q quit | Ctrl-q quit"),
        ];
        let widget = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title("Home"))
            .wrap(Wrap { trim: true });
        frame.render_widget(widget, area);
    }

    fn render_generate(&self, frame: &mut ratatui::Frame, area: Rect) {
        let kind = match self.kind {
            GenKind::Image => "image",
            GenKind::Video => "video",
        };
        let text = format!(
            "Mode: {kind} (Tab toggle)\nratio: {} | duration: {}s | inputs: {}\n\nPrompt:\n{}",
            self.ratio, self.duration, self.input_paths, self.prompt
        );
        let widget = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title("Generate"))
            .wrap(Wrap { trim: true });
        frame.render_widget(widget, area);
    }

    fn render_config(&self, frame: &mut ratatui::Frame, area: Rect) {
        let api = if self.config.api_key_encrypted.is_some() {
            "<configured>"
        } else {
            "<not set>"
        };
        let text = format!(
            "base_url    = {}\ntext_model  = {}\nimage_model = {}\nvideo_model = {}\noutput_dir  = {}\nsave_local  = {}\nmax_retries = {}\napi_key     = {}\n\nEsc back",
            self.config.base_url,
            self.config.text_model,
            self.config.image_model,
            self.config.video_model,
            self.config.output_dir,
            self.config.save_local,
            self.config.max_retries,
            api,
        );
        frame.render_widget(
            Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Config")),
            area,
        );
    }

    fn render_sessions(&self, frame: &mut ratatui::Frame, area: Rect) {
        let items: Vec<ListItem> = self.sessions.iter().map(|s| ListItem::new(s.as_str())).collect();
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Sessions (files in output_dir)"),
        );
        frame.render_widget(list, area);
    }

    fn render_chat(&self, frame: &mut ratatui::Frame, area: Rect) {
        frame.render_widget(
            Paragraph::new(self.chat_placeholder.as_str()).block(Block::default().borders(Borders::ALL).title("Chat")),
            area,
        );
    }
}

fn list_sessions(cfg: &AppConfig) -> Result<Vec<String>> {
    let dir = cfg.resolved_output_dir()?;
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            names.push(entry.path().display().to_string());
        }
    }
    names.sort();
    Ok(names)
}

pub fn run() -> Result<()> {
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal);
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    ratatui::restore();
    result
}

fn run_app(terminal: &mut DefaultTerminal) -> Result<()> {
    let mut app = App::new()?;
    loop {
        terminal.draw(|f| app.render(f))?;
        if event::poll(Duration::from_millis(200))?
            && let Event::Key(key) = event::read()?
            && app.handle_key(key.code, key.modifiers)
        {
            break;
        }
        app.on_tick();
    }
    Ok(())
}
