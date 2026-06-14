mod assets;
mod config;
mod generate;
mod home;
mod jobs;
mod result;
mod task_display;
mod tasks;

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::config::AppConfig;
use crate::ui::chat::ChatUiOptions;

use assets::{AssetAction, AssetsView};
use config::{ConfigAction, ConfigForm};
use generate::fields::is_submit_key;
use generate::image::{ImageForm, ImageFormAction};
use generate::preview::ratio_from_index;
use generate::video::{VideoForm, VideoFormAction};
use home::HomeMenu;
use jobs::{ImageJobParams, JobEvent, JobRequest, VideoJobParams, spawn_job};
use result::{PendingSyncOutcome, ResultPanel};
use task_display::{progress_percent, status_icon, task_status_kind};
use tasks::{TaskAction, TasksView};

const LIST_LIMIT: usize = 20;
const TASK_REFRESH_FAST: Duration = Duration::from_secs(1);
const TASK_REFRESH_SLOW: Duration = Duration::from_secs(5);
const OUTPUT_PANEL_HEIGHT: u16 = 7;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Image,
    Video,
    Tasks,
    Assets,
    Config,
    AssetPicker,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GenerateFocus {
    Text,
    Params,
    Media,
    Output,
}

pub struct DashboardApp {
    page: Page,
    status: String,
    home: HomeMenu,
    image_form: ImageForm,
    video_form: VideoForm,
    config_form: ConfigForm,
    tasks: TasksView,
    assets: AssetsView,
    result: ResultPanel,
    job_rx: Option<UnboundedReceiver<JobEvent>>,
    asset_picker_return: Page,
    quit: bool,
    launch_chat: bool,
    last_task_refresh: Instant,
    tick: u64,
    watch_task_id: Option<i64>,
    generate_focus: GenerateFocus,
}

impl DashboardApp {
    fn new() -> Result<Self> {
        let cfg = AppConfig::load()?;
        let mut tasks = TasksView::load(LIST_LIMIT)?;
        if !tasks.rows.is_empty() {
            tasks.table_state.select(Some(0));
        }
        Ok(Self {
            page: Page::Home,
            status: "Ready".into(),
            home: HomeMenu::new(),
            image_form: ImageForm::new(),
            video_form: VideoForm::new(),
            config_form: ConfigForm::from_config(cfg),
            tasks,
            assets: AssetsView::load(LIST_LIMIT)?,
            result: ResultPanel::from_results(Vec::new()),
            job_rx: None,
            asset_picker_return: Page::Image,
            quit: false,
            launch_chat: false,
            last_task_refresh: Instant::now(),
            tick: 0,
            watch_task_id: None,
            generate_focus: GenerateFocus::Text,
        })
    }

    fn handle_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        if key == KeyCode::Char('q') && modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return;
        }

        if matches!(self.page, Page::Image | Page::Video) && is_submit_key(key, modifiers) {
            match self.page {
                Page::Image => self.submit_image(),
                Page::Video => self.submit_video(),
                _ => {}
            }
            return;
        }

        if matches!(self.page, Page::Image | Page::Video) {
            if key == KeyCode::Tab {
                self.cycle_generate_focus(1);
                return;
            }
            if key == KeyCode::BackTab {
                self.cycle_generate_focus(-1);
                return;
            }
        }

        if matches!(self.page, Page::Image | Page::Video) && self.generate_focus == GenerateFocus::Output {
            self.handle_output_focus_keys(key, modifiers);
            return;
        }

        match self.page {
            Page::Home => self.handle_home(key),
            Page::Image => self.handle_image(key, modifiers),
            Page::Video => self.handle_video(key, modifiers),
            Page::Tasks => self.handle_tasks(key, modifiers),
            Page::Assets => self.handle_assets(key, modifiers),
            Page::Config => self.handle_config(key, modifiers),
            Page::AssetPicker => self.handle_asset_picker(key, modifiers),
        }
    }

    fn handle_home(&mut self, key: KeyCode) {
        match key {
            KeyCode::Up => self.home.select_previous(),
            KeyCode::Down => self.home.select_next(),
            KeyCode::Enter => self.activate_home_item(self.home.selected()),
            KeyCode::Char('q') if self.home.selected() == 6 => self.quit = true,
            KeyCode::Char(c @ '1'..='6') => {
                self.home.select_by_key(c);
                self.activate_home_item(self.home.selected());
            }
            KeyCode::Char('7') => self.quit = true,
            _ => {}
        }
    }

    fn activate_home_item(&mut self, index: usize) {
        match index {
            0 => {
                self.page = Page::Image;
                self.apply_generate_focus(GenerateFocus::Text);
            }
            1 => {
                if let Some(uri) = self.result.latest_asset_uri() {
                    self.video_form.set_chain_uri(Some(uri));
                }
                self.page = Page::Video;
                self.apply_generate_focus(GenerateFocus::Text);
            }
            2 => {
                self.tasks.refresh(LIST_LIMIT);
                self.page = Page::Tasks;
            }
            3 => {
                self.assets.refresh(LIST_LIMIT);
                self.page = Page::Assets;
            }
            4 => {
                self.config_form.reload();
                self.page = Page::Config;
            }
            5 => self.launch_chat = true,
            6 => self.quit = true,
            _ => {}
        }
    }

    fn handle_image(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        match self.image_form.handle_key(key, modifiers) {
            ImageFormAction::None => {}
            ImageFormAction::Back => {
                self.page = Page::Home;
                self.generate_focus = GenerateFocus::Text;
                self.status = "Ready".into();
            }
            ImageFormAction::PickAsset => {
                self.asset_picker_return = Page::Image;
                self.assets.refresh(LIST_LIMIT);
                self.page = Page::AssetPicker;
            }
            ImageFormAction::Submit => self.submit_image(),
        }
    }

    fn handle_video(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        if let Some(uri) = self.result.latest_asset_uri() {
            self.video_form.set_chain_uri(Some(uri));
        }
        match self.video_form.handle_key(key, modifiers) {
            VideoFormAction::None => {}
            VideoFormAction::Back => {
                self.page = Page::Home;
                self.generate_focus = GenerateFocus::Text;
                self.status = "Ready".into();
            }
            VideoFormAction::PickAsset => {
                self.asset_picker_return = Page::Video;
                self.assets.refresh(LIST_LIMIT);
                self.page = Page::AssetPicker;
            }
            VideoFormAction::OpenTasks => {
                self.tasks.refresh(LIST_LIMIT);
                self.page = Page::Tasks;
            }
            VideoFormAction::RefreshTasks => {
                self.tasks.refresh(LIST_LIMIT);
                self.status = "Tasks refreshed".into();
            }
            VideoFormAction::GoRunning => {
                self.tasks.refresh(LIST_LIMIT);
                if self.tasks.select_primary_running() {
                    self.page = Page::Tasks;
                    self.status = "Jumped to running task".into();
                } else {
                    self.status = "No running tasks".into();
                }
            }
            VideoFormAction::Submit => self.submit_video(),
        }
    }

    fn handle_tasks(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        match self.tasks.handle_key(key, modifiers) {
            TaskAction::None => {}
            TaskAction::Back => self.page = Page::Home,
            TaskAction::Refresh => {
                self.tasks.refresh(LIST_LIMIT);
                self.status = "Tasks refreshed".into();
            }
            TaskAction::SelectRunning => {
                if self.tasks.select_primary_running() {
                    self.status = "Selected running task".into();
                } else {
                    self.status = "No running tasks".into();
                }
            }
        }
    }

    fn handle_assets(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        match self.assets.handle_key(key, modifiers) {
            AssetAction::None => {}
            AssetAction::Back => self.page = Page::Home,
            AssetAction::Refresh => {
                self.assets.refresh(LIST_LIMIT);
                self.status = "Assets refreshed".into();
            }
            AssetAction::UseForVideo => {
                if let Some(uri) = self.assets.selected_asset_uri() {
                    self.video_form.add_asset_input(uri);
                    self.page = Page::Video;
                    self.apply_generate_focus(GenerateFocus::Media);
                    self.status = "Added asset to video form".into();
                }
            }
        }
    }

    fn handle_config(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        match self.config_form.handle_key(key, modifiers) {
            ConfigAction::None => {}
            ConfigAction::Back => self.page = Page::Home,
            ConfigAction::Save => match self.config_form.save() {
                Ok(()) => self.status = "Config saved".into(),
                Err(err) => self.status = format!("Config error: {err}"),
            },
        }
    }

    fn handle_asset_picker(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        if key == KeyCode::Esc {
            self.page = self.asset_picker_return;
            return;
        }
        if key == KeyCode::Enter {
            if let Some(uri) = self.assets.selected_asset_uri() {
                match self.asset_picker_return {
                    Page::Image => self.image_form.add_asset_input(uri),
                    Page::Video => self.video_form.add_asset_input(uri),
                    _ => {}
                }
                self.page = self.asset_picker_return;
                self.status = "Asset selected".into();
            }
            return;
        }
        match self.assets.handle_key(key, modifiers) {
            AssetAction::Back => self.page = self.asset_picker_return,
            AssetAction::Refresh => self.assets.refresh(LIST_LIMIT),
            _ => {}
        }
    }

    fn cycle_generate_focus(&mut self, delta: i32) {
        let idx = match self.generate_focus {
            GenerateFocus::Text => 0,
            GenerateFocus::Params => 1,
            GenerateFocus::Media => 2,
            GenerateFocus::Output => 3,
        };
        let next = (idx + delta).rem_euclid(4) as u32;
        let focus = match next {
            1 => GenerateFocus::Params,
            2 => GenerateFocus::Media,
            3 => GenerateFocus::Output,
            _ => GenerateFocus::Text,
        };
        self.apply_generate_focus(focus);
    }

    fn apply_generate_focus(&mut self, focus: GenerateFocus) {
        self.generate_focus = focus;
        self.image_form.editing = false;
        self.video_form.editing = false;
        self.image_form.inputs.adding = false;
        self.video_form.inputs.adding = false;
        match self.page {
            Page::Image => match focus {
                GenerateFocus::Text => self.image_form.focus_prompt(),
                GenerateFocus::Params => self.image_form.focus_params_section(),
                GenerateFocus::Media => self.image_form.focus_media_section(),
                GenerateFocus::Output => {}
            },
            Page::Video => match focus {
                GenerateFocus::Text => self.video_form.focus_prompt(),
                GenerateFocus::Params => self.video_form.focus_params_section(),
                GenerateFocus::Media => self.video_form.focus_media_section(),
                GenerateFocus::Output => {}
            },
            _ => {}
        }
    }

    fn handle_output_focus_keys(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        let nav = self.result.can_navigate();
        match key {
            KeyCode::Left if nav => self.result.field_previous(),
            KeyCode::Right if nav => self.result.field_next(),
            KeyCode::Up if nav => self.result.result_previous(),
            KeyCode::Down if nav => self.result.result_next(),
            KeyCode::Enter if nav => self.result.copy_field(),
            KeyCode::Char('c') if nav && !modifiers.contains(KeyModifiers::CONTROL) => {
                self.result.copy_asset_uri();
            }
            _ => {}
        }
    }

    fn submit_image(&mut self) {
        self.result.set_ack("Submit received — validating…");
        self.image_form.editing = false;

        if self.job_rx.is_some() {
            self.status = "Job already running".into();
            self.result
                .set_error("A job is already running — wait for Output panel");
            return;
        }
        if let Err(err) = self.image_form.validate() {
            self.status = err.clone();
            self.result.set_error(err);
            self.image_form.focus_prompt();
            return;
        }
        self.result.clear_error();
        let ratio = match ratio_from_index(&self.image_form.ratio_options, self.image_form.ratio.index) {
            Ok(r) => r,
            Err(err) => {
                self.status = format!("{err:#}");
                return;
            }
        };
        let output_dir = {
            let v = self.image_form.output_dir.value.trim();
            if v.is_empty() { None } else { Some(v.to_string()) }
        };
        let handle = spawn_job(JobRequest::Image(ImageJobParams {
            prompt: self.image_form.prompt.text(),
            ratio,
            inputs: self.image_form.inputs.raw_values(),
            count: self.image_form.count_value(),
            seed: self.image_form.seed_value(),
            output_dir,
            save_local: self.image_form.save_local,
        }));
        self.result.set_local_job("Generating image…");
        self.job_rx = Some(handle.rx);
        self.status = "Generating image…".into();
    }

    fn submit_video(&mut self) {
        self.result.set_ack("Submit received — validating…");
        self.video_form.editing = false;

        if self.job_rx.is_some() {
            self.status = "Job already running".into();
            self.result
                .set_error("A job is already running — wait for Output panel");
            return;
        }
        if let Err(err) = self.video_form.validate() {
            self.status = err.clone();
            self.result.set_error(err);
            if self.video_form.prompt.text().trim().is_empty() {
                self.video_form.focus_prompt();
            }
            return;
        }
        self.result.clear_error();
        let ratio = if self.video_form.ratio_disabled() {
            crate::ratio::AspectRatio::parse("16:9").expect("16:9")
        } else {
            match ratio_from_index(&self.video_form.ratio_options, self.video_form.ratio.index) {
                Ok(r) => r,
                Err(err) => {
                    self.status = format!("{err:#}");
                    return;
                }
            }
        };
        let output_dir = {
            let v = self.video_form.output_dir.value.trim();
            if v.is_empty() { None } else { Some(v.to_string()) }
        };
        let negative = {
            let text = self.video_form.negative.text();
            let v = text.trim();
            if v.is_empty() { None } else { Some(v.to_string()) }
        };
        let async_mode = self.video_form.async_mode;
        let handle = spawn_job(JobRequest::Video(VideoJobParams {
            prompt: self.video_form.prompt.text(),
            negative_prompt: negative,
            ratio,
            duration: self.video_form.duration_value(),
            frame_rate: self.video_form.frame_rate_value(),
            images: self.video_form.inputs.raw_values(),
            seed: self.video_form.seed_value(),
            output_dir,
            save_local: self.video_form.save_local,
            async_mode,
        }));
        self.result.set_local_job(if async_mode {
            "Submitting video task…"
        } else {
            "Generating video (sync)…"
        });
        self.job_rx = Some(handle.rx);
        self.status = if async_mode {
            "Submitting async video…".into()
        } else {
            "Generating video…".into()
        };
    }

    fn poll_jobs(&mut self) {
        let mut done = false;
        let mut focus_output = false;
        if let Some(rx) = self.job_rx.as_mut() {
            while let Ok(event) = rx.try_recv() {
                match event {
                    JobEvent::ImageDone { results, error } => {
                        done = true;
                        self.result.clear_local_job();
                        if let Some(err) = error {
                            self.status = format!("Error: {err}");
                            self.result.set_error(format!("Error: {err}"));
                        } else if results.is_empty() {
                            self.status = "No results".into();
                        } else {
                            self.result = ResultPanel::from_results(results);
                            focus_output = true;
                            self.status = format!("Done — {} result(s)", self.result.results.len());
                        }
                    }
                    JobEvent::VideoSubmitted { local_id, error } => {
                        done = true;
                        if let Some(err) = error {
                            self.result.clear_local_job();
                            self.status = format!("Error: {err}");
                        } else {
                            self.tasks.refresh(LIST_LIMIT);
                            self.tasks.select_task_id(local_id);
                            self.watch_task_id = Some(local_id);
                            self.result.set_pending_task(local_id);
                            self.page = Page::Video;
                            self.status = format!("Task #{local_id} running");
                            self.last_task_refresh = Instant::now();
                        }
                    }
                    JobEvent::VideoDone { results, error } => {
                        done = true;
                        self.result.clear_local_job();
                        if let Some(err) = error {
                            self.status = format!("Error: {err}");
                            self.result.set_error(format!("Error: {err}"));
                        } else if results.is_empty() {
                            self.status = "No results".into();
                        } else {
                            self.result = ResultPanel::from_results(results);
                            focus_output = true;
                            self.status = format!("Done — {} result(s)", self.result.results.len());
                        }
                    }
                }
            }
        }
        if done {
            self.job_rx = None;
        }
        if focus_output {
            self.apply_generate_focus(GenerateFocus::Output);
        }
    }

    fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.poll_jobs();
        if let Some(interval) = self.task_refresh_interval()
            && self.last_task_refresh.elapsed() >= interval
        {
            self.tasks.refresh(LIST_LIMIT);
            self.sync_watched_task();
            self.last_task_refresh = Instant::now();
        }
    }

    fn task_refresh_interval(&self) -> Option<Duration> {
        if self.tasks.has_processing() || self.result.pending_task_id().is_some() || self.watch_task_id.is_some() {
            return Some(TASK_REFRESH_FAST);
        }
        if matches!(self.page, Page::Tasks | Page::Video) {
            return Some(TASK_REFRESH_SLOW);
        }
        None
    }

    fn sync_watched_task(&mut self) {
        let Some(local_id) = self.watch_task_id else {
            if let Some(pending_id) = self.result.pending_task_id()
                && let Some(row) = self.tasks.row_by_id(pending_id).cloned()
            {
                self.apply_pending_sync(&row);
            }
            return;
        };
        let Some(row) = self.tasks.row_by_id(local_id).cloned() else {
            return;
        };
        self.apply_pending_sync(&row);
    }

    fn apply_pending_sync(&mut self, row: &crate::db::VideoTaskRecord) {
        match self.result.sync_pending(row) {
            PendingSyncOutcome::Unchanged => {}
            PendingSyncOutcome::Updated => {
                if !matches!(self.page, Page::Image | Page::Video) {
                    self.status = format!("Task #{} {} {}", row.id, row.status, progress_percent(row.progress));
                }
            }
            PendingSyncOutcome::Completed(result) => {
                self.watch_task_id = None;
                self.apply_generate_focus(GenerateFocus::Output);
                self.status = format!("Task #{} completed", row.id);
                let _ = result;
            }
            PendingSyncOutcome::Failed(err) => {
                self.watch_task_id = None;
                self.status = format!("Task #{} failed: {err}", row.id);
            }
        }
    }

    fn render(&mut self, frame: &mut ratatui::Frame) {
        let show_output = matches!(self.page, Page::Image | Page::Video);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(if show_output { OUTPUT_PANEL_HEIGHT } else { 0 }),
                Constraint::Length(3),
            ])
            .split(frame.area());

        let output_focused = self.generate_focus == GenerateFocus::Output;
        let form_focused = !output_focused;

        match self.page {
            Page::Home => self.home.render(frame, chunks[0], self.tasks.processing_count()),
            Page::Image => self.image_form.render(frame, chunks[0], form_focused),
            Page::Video => {
                let task_strip = self.tasks.task_strip_data(self.tick);
                self.video_form.render(frame, chunks[0], &task_strip, form_focused);
            }
            Page::Tasks => self.tasks.render(frame, chunks[0], self.tick),
            Page::Assets | Page::AssetPicker => self.assets.render(frame, chunks[0]),
            Page::Config => self.config_form.render(frame, chunks[0]),
        }

        if show_output {
            self.result.render(frame, chunks[1], self.tick, output_focused);
        }

        frame.render_widget(
            Paragraph::new(self.status_line()).block(Block::default().borders(Borders::ALL).title("Status")),
            chunks[2],
        );
    }

    fn status_line(&self) -> Line<'static> {
        let breadcrumb = match self.page {
            Page::Home => "Home",
            Page::Image => "Home > Generate Image",
            Page::Video => "Home > Generate Video",
            Page::Tasks => "Home > Video Tasks",
            Page::Assets => "Home > Assets",
            Page::Config => "Home > Settings",
            Page::AssetPicker => "Pick asset (Enter select, Esc cancel)",
        };
        if matches!(self.page, Page::Image | Page::Video) {
            return Line::from(format!("{breadcrumb}  |  {}", self.status));
        }
        let mut spans = Vec::new();
        let run_count = self.tasks.processing_count();
        if run_count > 0 {
            spans.push(Span::styled(
                format!("[RUN {run_count}] "),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        }
        if let Some(row) = self.tasks.primary_running() {
            let kind = task_status_kind(row);
            let icon = status_icon(kind, self.tick);
            spans.push(Span::styled(
                format!(
                    "#{} {icon} {} {} · ",
                    row.id,
                    row.status,
                    progress_percent(row.progress)
                ),
                Style::default().fg(Color::Cyan),
            ));
        }
        spans.push(Span::raw(format!("{breadcrumb}  |  {}", self.status)));
        Line::from(spans)
    }
}

pub fn run() -> Result<()> {
    let mut terminal = ratatui::try_init()?;
    let result = run_app(&mut terminal);
    ratatui::restore();
    result
}

fn run_app(terminal: &mut DefaultTerminal) -> Result<()> {
    let mut app = DashboardApp::new()?;
    loop {
        terminal.draw(|f| app.render(f))?;
        if app.quit {
            break;
        }
        if app.launch_chat {
            ratatui::restore();
            return crate::ui::chat::run(ChatUiOptions {
                resume: None,
                auto: false,
                max_turns: 32,
                overrides: Default::default(),
            });
        }
        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            app.handle_key(key.code, key.modifiers);
        }
        app.on_tick();
    }
    Ok(())
}
