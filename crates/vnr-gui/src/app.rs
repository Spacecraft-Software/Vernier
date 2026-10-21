// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Application state, update logic and view.

use std::fmt;
use std::time::Duration;

use iced::futures::channel::oneshot;
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{
    Canvas, Column, button, checkbox, column, container, pick_list, row, rule, scrollable, text,
    text_input, toggler,
};
use iced::{Border, Element, Font, Length, Subscription, Task, Theme, time};
use vnr_core::geometry::{normalize, resolve_overlap};
use vnr_core::{
    AdaptiveSync, BackendKind, Capabilities, Issue, Layout, Mode, Output, Position, Scale,
    Severity, Transform, validate,
};

use crate::Options;
use crate::arrangement::{Arrangement, CanvasEvent};
use crate::theme::{self, Resolution, Steelbore};

/// Seconds the user has to confirm a new configuration before it is
/// reverted. Long enough to read the prompt on a monitor that just came
/// up (some take several seconds to sync), short enough that a blank
/// screen recovers without intervention. Matches GNOME and KDE.
const REVERT_SECONDS: u32 = 15;

/// Width of the properties panel in logical pixels.
const PANEL_WIDTH: f32 = 340.0;

/// Headings font (Steelbore Standard §12); falls back to the system
/// monospace font when it is not installed.
const HEADING_FONT: Font = Font::with_name("Share Tech Mono");

/// What a background load returns.
#[derive(Debug, Clone)]
pub struct Loaded {
    kind: Option<BackendKind>,
    caps: Capabilities,
    layout: Layout,
}

/// A configuration that was applied and awaits confirmation.
#[derive(Debug, Clone)]
struct Pending {
    previous: Layout,
    remaining: u32,
}

/// The choices of the mirror picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorChoice {
    /// Not mirroring.
    Off,
    /// Mirroring the named output.
    Of(String),
}

impl fmt::Display for MirrorChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Off => f.write_str("Off (extend)"),
            Self::Of(name) => f.write_str(name),
        }
    }
}

/// Keyboard commands (Steelbore Standard §10: CUA and Vim bindings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCommand {
    /// `]` or Page Down: select the next output. (Tab is left to move
    /// keyboard focus between controls.)
    Next,
    /// `[` or Page Up: select the previous output.
    Previous,
    /// Arrows or h/j/k/l: move the selected output (Shift: by 10).
    Nudge(i32, i32),
    /// Ctrl+Enter or Ctrl+S: apply.
    Apply,
    /// Escape: revert a pending change, otherwise discard edits.
    Escape,
    /// F5 or Ctrl+R: reload from the display server.
    Reload,
    /// y: keep a pending change.
    Yes,
    /// n: revert a pending change.
    No,
}

/// Application messages.
#[derive(Debug, Clone)]
pub enum Message {
    /// A snapshot arrived from the backend.
    Loaded(Result<Loaded, String>),
    /// The canvas reported an interaction.
    Canvas(CanvasEvent),
    /// An output was chosen in the list.
    Select(usize),
    /// The enabled toggle changed.
    SetEnabled(bool),
    /// A mode was picked.
    SetMode(Mode),
    /// A scale was picked.
    SetScale(Scale),
    /// A transform was picked.
    SetTransform(Transform),
    /// An adaptive sync state was picked.
    SetAdaptiveSync(AdaptiveSync),
    /// A mirror source was picked.
    SetMirror(MirrorChoice),
    /// The primary checkbox changed.
    SetPrimary(bool),
    /// The X position field changed.
    EditX(String),
    /// The Y position field changed.
    EditY(String),
    /// Apply the draft.
    Apply,
    /// The backend finished applying.
    Applied(Result<(), String>),
    /// Keep the pending configuration.
    Keep,
    /// Restore the configuration from before the last apply.
    Revert,
    /// The backend finished reverting.
    Reverted(Result<(), String>),
    /// One second passed while a confirmation is pending.
    Tick,
    /// Re-read the configuration from the display server.
    Reload,
    /// Discard edits.
    Reset,
    /// A keyboard command.
    Key(KeyCommand),
}

/// The editor.
#[derive(Debug)]
pub struct App {
    options: Options,
    resolution: Resolution,
    theme: Theme,
    kind: Option<BackendKind>,
    caps: Capabilities,
    applied: Layout,
    draft: Layout,
    selected: Option<usize>,
    x_text: String,
    y_text: String,
    pending: Option<Pending>,
    busy: Option<&'static str>,
    status: String,
    issues: Vec<Issue>,
    overlapping: Vec<String>,
}

/// Runs `work` on its own thread and resolves when it finishes, so backend
/// round trips never block the UI thread.
async fn background<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    receiver
        .await
        .unwrap_or_else(|_| Err("the backend worker stopped unexpectedly".to_owned()))
}

fn load(backend: Option<BackendKind>) -> Result<Loaded, String> {
    let mut backend = vnr_backends::open(backend).map_err(|e| e.to_string())?;
    let layout = backend.snapshot().map_err(|e| e.to_string())?;
    Ok(Loaded {
        kind: Some(backend.kind()),
        caps: backend.capabilities(),
        layout,
    })
}

fn apply(backend: Option<BackendKind>, desired: &Layout) -> Result<(), String> {
    let mut backend = vnr_backends::open(backend).map_err(|e| e.to_string())?;
    if backend.capabilities().test_only {
        backend.test(desired).map_err(|e| e.to_string())?;
    }
    backend.apply(desired).map_err(|e| e.to_string())
}

impl App {
    /// Creates the editor and starts loading the current configuration.
    pub fn new(options: Options) -> (Self, Task<Message>) {
        let resolution = theme::resolve(&theme::Request {
            theme: options.theme.clone(),
            accessible: options.accessible,
        });
        if options.verbose {
            eprintln!("{}", resolution.report());
        }
        let app = Self {
            theme: theme::iced_theme(&resolution.tokens),
            resolution,
            kind: None,
            caps: Capabilities::default(),
            applied: Layout::default(),
            draft: Layout::default(),
            selected: None,
            x_text: String::new(),
            y_text: String::new(),
            pending: None,
            busy: Some("Reading displays"),
            status: String::new(),
            issues: Vec::new(),
            overlapping: Vec::new(),
            options,
        };
        let task = app.reload_task();
        (app, task)
    }

    fn tokens(&self) -> &Steelbore {
        &self.resolution.tokens
    }

    fn reload_task(&self) -> Task<Message> {
        if self.options.demo {
            return Task::done(Message::Loaded(Ok(Loaded {
                kind: None,
                caps: demo_capabilities(),
                layout: demo_layout(),
            })));
        }
        let backend = self.options.backend;
        Task::perform(background(move || load(backend)), Message::Loaded)
    }

    /// The window title.
    pub fn title(&self) -> String {
        let dirty = if self.draft == self.applied {
            ""
        } else {
            " (edited)"
        };
        format!("Vernier{dirty}")
    }

    /// The theme.
    pub fn theme(&self) -> Theme {
        self.theme.clone()
    }

    /// Ticks once a second while a confirmation is pending; listens to the
    /// keyboard always.
    pub fn subscription(&self) -> Subscription<Message> {
        let keys = iced::event::listen_with(key_command);
        if self.pending.is_some() {
            Subscription::batch([
                keys,
                time::every(Duration::from_secs(1)).map(|_| Message::Tick),
            ])
        } else {
            keys
        }
    }

    fn selected_output(&self) -> Option<&Output> {
        self.selected.and_then(|i| self.draft.outputs.get(i))
    }

    fn selected_output_mut(&mut self) -> Option<&mut Output> {
        self.selected.and_then(|i| self.draft.outputs.get_mut(i))
    }

    /// Recomputes issues and refreshes the position fields after an edit.
    fn edited(&mut self) {
        self.issues = validate(&self.draft, &self.caps);
        self.overlapping = self
            .issues
            .iter()
            .filter_map(|issue| match issue {
                Issue::Overlap { a, b } => Some([a.clone(), b.clone()]),
                _ => None,
            })
            .flatten()
            .collect();
        self.sync_position_text();
    }

    fn sync_position_text(&mut self) {
        let position = self.selected_output().map(|o| o.position);
        if let Some(position) = position {
            self.x_text = position.x.to_string();
            self.y_text = position.y.to_string();
        }
    }

    fn geometry_changed(&mut self) {
        if let Some(index) = self.selected {
            resolve_overlap(&mut self.draft, index);
            normalize(&mut self.draft);
        }
        self.edited();
    }

    fn select(&mut self, index: Option<usize>) {
        self.selected = index.filter(|i| *i < self.draft.outputs.len());
        self.sync_position_text();
    }

    fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.severity() == Severity::Error)
    }

    fn can_apply(&self) -> bool {
        self.busy.is_none()
            && self.pending.is_none()
            && self.draft != self.applied
            && !self.has_errors()
    }

    /// Handles a message.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Loaded(Ok(loaded)) => {
                let selected_name = self.selected_output().map(|o| o.name.clone());
                self.kind = loaded.kind;
                self.caps = loaded.caps;
                self.applied = loaded.layout.clone();
                self.draft = loaded.layout;
                self.busy = None;
                let index = selected_name
                    .and_then(|name| self.draft.outputs.iter().position(|o| o.name == name))
                    .or_else(|| self.draft.outputs.iter().position(Output::is_placed));
                self.select(index);
                self.edited();
            }
            Message::Loaded(Err(error)) => {
                self.busy = None;
                self.status = format!("Cannot read displays: {error}");
            }
            Message::Canvas(CanvasEvent::Select(index)) => self.select(index),
            Message::Canvas(CanvasEvent::Move { index, position }) => {
                if let Some(output) = self.draft.outputs.get_mut(index) {
                    output.position = position;
                }
                self.sync_position_text();
            }
            Message::Canvas(CanvasEvent::DragEnded) => {
                normalize(&mut self.draft);
                self.edited();
            }
            Message::Select(index) => self.select(Some(index)),
            edit @ (Message::SetEnabled(_)
            | Message::SetMode(_)
            | Message::SetScale(_)
            | Message::SetTransform(_)
            | Message::SetAdaptiveSync(_)
            | Message::SetMirror(_)
            | Message::SetPrimary(_)
            | Message::EditX(_)
            | Message::EditY(_)) => self.edit(edit),
            Message::Apply => return self.start_apply(),
            Message::Applied(result) => return self.finish_apply(result),
            Message::Keep => {
                if self.pending.take().is_some() {
                    self.status = if self.caps.persistent || self.options.demo {
                        "Kept.".to_owned()
                    } else {
                        "Kept until the compositor restarts. Run `vnr layout export --to niri --write` to make it permanent."
                            .to_owned()
                    };
                }
            }
            Message::Revert => return self.start_revert(),
            Message::Reverted(result) => {
                self.busy = None;
                self.status = match result {
                    Ok(()) => "Reverted to the previous configuration.".to_owned(),
                    Err(error) => format!("Could not revert: {error}"),
                };
                return self.reload_task();
            }
            Message::Tick => {
                if let Some(pending) = &mut self.pending {
                    pending.remaining = pending.remaining.saturating_sub(1);
                    if pending.remaining == 0 {
                        return self.start_revert();
                    }
                }
            }
            Message::Reload => {
                if self.busy.is_none() && self.pending.is_none() {
                    self.busy = Some("Reading displays");
                    return self.reload_task();
                }
            }
            Message::Reset => {
                self.draft = self.applied.clone();
                self.edited();
            }
            Message::Key(command) => return self.key(command),
        }
        Task::none()
    }

    /// Applies a property edit from the side panel to the draft.
    fn edit(&mut self, message: Message) {
        match message {
            Message::SetEnabled(on) => {
                if let Some(name) = self.selected_output().map(|o| o.name.clone()) {
                    if on {
                        self.draft.enable(&name);
                    } else if let Some(output) = self.selected_output_mut() {
                        output.enabled = false;
                    }
                    normalize(&mut self.draft);
                    self.edited();
                }
            }
            Message::SetMode(mode) => {
                if let Some(output) = self.selected_output_mut() {
                    output.mode = Some(mode);
                }
                self.geometry_changed();
            }
            Message::SetScale(scale) => {
                if let Some(output) = self.selected_output_mut() {
                    output.scale = scale;
                }
                self.geometry_changed();
            }
            Message::SetTransform(transform) => {
                if let Some(output) = self.selected_output_mut() {
                    output.transform = transform;
                }
                self.geometry_changed();
            }
            Message::SetAdaptiveSync(sync) => {
                if let Some(output) = self.selected_output_mut() {
                    output.adaptive_sync = Some(sync);
                }
                self.edited();
            }
            Message::SetMirror(choice) => {
                if let Some(output) = self.selected_output_mut() {
                    output.mirror_of = match choice {
                        MirrorChoice::Off => None,
                        MirrorChoice::Of(name) => Some(name),
                    };
                }
                self.geometry_changed();
            }
            Message::SetPrimary(on) => {
                let selected = self.selected;
                for (i, output) in self.draft.outputs.iter_mut().enumerate() {
                    output.primary = on && Some(i) == selected;
                }
                self.edited();
            }
            Message::EditX(value) => {
                if let (Ok(x), Some(output)) = (value.trim().parse(), self.selected_output_mut()) {
                    output.position.x = x;
                    self.issues = validate(&self.draft, &self.caps);
                }
                self.x_text = value;
            }
            Message::EditY(value) => {
                if let (Ok(y), Some(output)) = (value.trim().parse(), self.selected_output_mut()) {
                    output.position.y = y;
                    self.issues = validate(&self.draft, &self.caps);
                }
                self.y_text = value;
            }
            _ => {}
        }
    }

    fn key(&mut self, command: KeyCommand) -> Task<Message> {
        match command {
            KeyCommand::Next | KeyCommand::Previous => {
                let count = self.draft.outputs.len();
                if count > 0 {
                    let current = self.selected.unwrap_or(0);
                    let next = if command == KeyCommand::Next {
                        (current + 1) % count
                    } else {
                        (current + count - 1) % count
                    };
                    self.select(Some(next));
                }
                Task::none()
            }
            KeyCommand::Nudge(dx, dy) => {
                if let Some(output) = self.selected_output_mut().filter(|o| o.is_placed()) {
                    output.position = Position::new(output.position.x + dx, output.position.y + dy);
                    self.edited();
                }
                Task::none()
            }
            KeyCommand::Apply => self.update(Message::Apply),
            KeyCommand::Escape | KeyCommand::No if self.pending.is_some() => {
                self.update(Message::Revert)
            }
            KeyCommand::Yes if self.pending.is_some() => self.update(Message::Keep),
            KeyCommand::Escape => self.update(Message::Reset),
            KeyCommand::Reload => self.update(Message::Reload),
            KeyCommand::Yes | KeyCommand::No => Task::none(),
        }
    }

    fn start_apply(&mut self) -> Task<Message> {
        if !self.can_apply() {
            return Task::none();
        }
        normalize(&mut self.draft);
        self.busy = Some("Applying");
        self.status.clear();
        if self.options.demo {
            return Task::done(Message::Applied(Ok(())));
        }
        let desired = self.draft.clone();
        let backend = self.options.backend;
        Task::perform(
            background(move || apply(backend, &desired)),
            Message::Applied,
        )
    }

    fn finish_apply(&mut self, result: Result<(), String>) -> Task<Message> {
        self.busy = None;
        match result {
            Ok(()) => {
                self.pending = Some(Pending {
                    previous: self.applied.clone(),
                    remaining: REVERT_SECONDS,
                });
                self.applied = self.draft.clone();
                if self.options.demo {
                    Task::none()
                } else {
                    self.reload_task()
                }
            }
            Err(error) => {
                self.status = format!("Not applied: {error}");
                Task::none()
            }
        }
    }

    fn start_revert(&mut self) -> Task<Message> {
        let Some(pending) = self.pending.take() else {
            return Task::none();
        };
        self.draft = pending.previous.clone();
        self.busy = Some("Reverting");
        if self.options.demo {
            self.applied = pending.previous;
            self.edited();
            return Task::done(Message::Reverted(Ok(())));
        }
        let backend = self.options.backend;
        Task::perform(
            background(move || apply(backend, &pending.previous)),
            Message::Reverted,
        )
    }

    /// Builds the window contents.
    pub fn view(&self) -> Element<'_, Message> {
        let canvas = Canvas::new(Arrangement {
            layout: &self.draft,
            selected: self.selected,
            overlapping: &self.overlapping,
            tokens: self.tokens(),
        })
        .width(Length::Fill)
        .height(Length::Fill);
        let canvas: Element<'_, CanvasEvent> = canvas.into();

        let body = row![
            canvas.map(Message::Canvas),
            container(scrollable(self.panel()).height(Length::Fill))
                .width(PANEL_WIDTH)
                .padding(12),
        ]
        .height(Length::Fill);

        column![body, rule::horizontal(1), self.footer()]
            .height(Length::Fill)
            .into()
    }

    /// Replaces the footer while a change awaits confirmation, so the
    /// prompt sits where the Apply button was and the canvas never moves.
    fn confirmation(&self, pending: &Pending) -> Element<'_, Message> {
        let tokens = self.tokens();
        container(
            row![
                text(format!(
                    "Keep these display settings? Reverting in {} s.",
                    pending.remaining
                ))
                .width(Length::Fill),
                button("Keep (y)").on_press(Message::Keep),
                button("Revert (n)")
                    .style(button::secondary)
                    .on_press(Message::Revert),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
        )
        .padding(12)
        .style(|_| container::Style {
            background: Some(tokens.surface.into()),
            border: Border {
                color: tokens.accent,
                width: 2.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
    }

    fn panel(&self) -> Column<'_, Message> {
        let mut panel = column![text("Outputs").font(HEADING_FONT).size(18)].spacing(8);
        for (index, output) in self.draft.outputs.iter().enumerate() {
            let state = match (&output.mirror_of, output.enabled) {
                (_, false) => "off".to_owned(),
                (Some(source), true) => format!("mirrors {source}"),
                (None, true) => "on".to_owned(),
            };
            let marker = if Some(index) == self.selected {
                "> "
            } else {
                ""
            };
            let style = if Some(index) == self.selected {
                button::primary
            } else {
                button::secondary
            };
            panel = panel.push(
                button(text(format!("{marker}{}  {state}", output.name)))
                    .width(Length::Fill)
                    .style(style)
                    .on_press(Message::Select(index)),
            );
        }

        if let Some(output) = self.selected_output() {
            panel = panel
                .push(rule::horizontal(1))
                .push(self.properties(output));
        }

        if !self.issues.is_empty() {
            panel = panel
                .push(rule::horizontal(1))
                .push(text("Problems").font(HEADING_FONT).size(18));
            for issue in &self.issues {
                let label = match issue.severity() {
                    Severity::Error => "Error",
                    Severity::Warning => "Warning",
                };
                panel = panel.push(text(format!("{label}: {issue}")));
            }
        }
        panel
    }

    fn properties<'a>(&'a self, output: &'a Output) -> Column<'a, Message> {
        let mut props = column![
            text(output.name.clone()).font(HEADING_FONT).size(18),
            text(output.label()),
        ]
        .spacing(8);
        if let Some((w, h)) = output.physical_size_mm {
            props = props.push(text(format!("{w} mm x {h} mm")));
        }
        props = props.push(
            toggler(output.enabled)
                .label("Enabled")
                .on_toggle(Message::SetEnabled),
        );
        if !output.enabled {
            return props;
        }

        if !output.modes.is_empty() {
            props = props.push(labeled(
                "Mode",
                pick_list(output.modes.clone(), output.mode, Message::SetMode).width(Length::Fill),
            ));
        }

        let mut scales: Vec<Scale> = Scale::PRESETS
            .iter()
            .filter_map(|m| Scale::from_millis(*m))
            .filter(|s| self.caps.fractional_scale || s.millis() % 1000 == 0)
            .collect();
        if !scales.contains(&output.scale) {
            scales.push(output.scale);
            scales.sort();
        }
        props = props
            .push(labeled(
                "Scale",
                pick_list(scales, Some(output.scale), Message::SetScale).width(Length::Fill),
            ))
            .push(labeled(
                "Rotation",
                pick_list(
                    Transform::ALL,
                    Some(output.transform),
                    Message::SetTransform,
                )
                .width(Length::Fill),
            ));

        if output.mirror_of.is_none() {
            props = props.push(labeled(
                "Position",
                row![
                    text_input("x", &self.x_text)
                        .on_input(Message::EditX)
                        .width(Length::Fill),
                    text_input("y", &self.y_text)
                        .on_input(Message::EditY)
                        .width(Length::Fill),
                ]
                .spacing(8),
            ));
        }

        if self.caps.mirroring {
            let mut choices = vec![MirrorChoice::Off];
            choices.extend(
                self.draft
                    .outputs
                    .iter()
                    .filter(|o| o.name != output.name && o.enabled && o.mirror_of.is_none())
                    .map(|o| MirrorChoice::Of(o.name.clone())),
            );
            let current = output
                .mirror_of
                .clone()
                .map_or(MirrorChoice::Off, MirrorChoice::Of);
            props = props.push(labeled(
                "Mirror",
                pick_list(choices, Some(current), Message::SetMirror).width(Length::Fill),
            ));
        }

        if self.caps.adaptive_sync {
            let choices: Vec<AdaptiveSync> = if self.caps.adaptive_sync_automatic {
                AdaptiveSync::ALL.to_vec()
            } else {
                vec![AdaptiveSync::Disabled, AdaptiveSync::Always]
            };
            props = props.push(labeled(
                "Adaptive sync",
                pick_list(choices, output.adaptive_sync, Message::SetAdaptiveSync)
                    .placeholder("unknown")
                    .width(Length::Fill),
            ));
        }

        if self.caps.primary {
            props = props.push(
                checkbox(output.primary)
                    .label("Primary output")
                    .on_toggle(Message::SetPrimary),
            );
        }
        props
    }

    fn footer(&self) -> Element<'_, Message> {
        if let Some(pending) = &self.pending {
            return self.confirmation(pending);
        }
        let source = match (self.options.demo, self.kind) {
            (true, _) => "demo layout, nothing is changed".to_owned(),
            (false, Some(kind)) => format!("backend: {kind}"),
            (false, None) => "no backend".to_owned(),
        };
        let status = self
            .busy
            .map_or_else(|| self.status.clone(), |b| format!("{b}..."));
        let idle = self.busy.is_none() && self.pending.is_none();
        let edited = self.draft != self.applied;
        container(
            row![
                text(source),
                text(status).width(Length::Fill),
                button("Reload")
                    .style(button::secondary)
                    .on_press_maybe(idle.then_some(Message::Reload)),
                button("Reset")
                    .style(button::secondary)
                    .on_press_maybe((idle && edited).then_some(Message::Reset)),
                button("Apply").on_press_maybe(self.can_apply().then_some(Message::Apply)),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
        )
        .padding(12)
        .into()
    }
}

fn labeled<'a>(label: &'a str, control: impl Into<Element<'a, Message>>) -> Column<'a, Message> {
    column![text(label), control.into()].spacing(4)
}

/// Maps key presses to commands.
///
/// Widgets capture some keys before a subscription sees them (Tab moves
/// keyboard focus, a focused text input takes letters), so this listens to
/// every event and decides per binding: chords with Ctrl, function keys
/// and Escape always apply; plain letters and arrows apply only when no
/// widget used the key, so typing into the position fields never moves
/// an output.
fn key_command(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    let iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event else {
        return None;
    };
    let free = status == iced::event::Status::Ignored;
    let step = if modifiers.shift() { 10 } else { 1 };
    let command = match key.as_ref() {
        Key::Named(Named::Enter) | Key::Character("s") if modifiers.command() => KeyCommand::Apply,
        Key::Character("r") if modifiers.command() => KeyCommand::Reload,
        Key::Named(Named::F5) => KeyCommand::Reload,
        Key::Named(Named::Escape) => KeyCommand::Escape,
        _ if !free || modifiers.command() || modifiers.alt() => return None,
        Key::Named(Named::PageDown) | Key::Character("]") => KeyCommand::Next,
        Key::Named(Named::PageUp) | Key::Character("[") => KeyCommand::Previous,
        Key::Named(Named::ArrowLeft) | Key::Character("h") => KeyCommand::Nudge(-step, 0),
        Key::Named(Named::ArrowRight) | Key::Character("l") => KeyCommand::Nudge(step, 0),
        Key::Named(Named::ArrowUp) | Key::Character("k") => KeyCommand::Nudge(0, -step),
        Key::Named(Named::ArrowDown) | Key::Character("j") => KeyCommand::Nudge(0, step),
        Key::Character("y") => KeyCommand::Yes,
        Key::Character("n") => KeyCommand::No,
        _ => return None,
    };
    Some(Message::Key(command))
}

fn demo_capabilities() -> Capabilities {
    Capabilities {
        mirroring: true,
        adaptive_sync: true,
        adaptive_sync_automatic: true,
        primary: true,
        custom_modes: false,
        fractional_scale: true,
        test_only: true,
        persistent: false,
    }
}

/// A laptop panel, a 1440p monitor to its right, and a disabled TV.
fn demo_layout() -> Layout {
    let mut laptop = Output::new("eDP-1");
    laptop.make = Some("BOE".into());
    laptop.model = Some("NE135A1M".into());
    laptop.physical_size_mm = Some((285, 190));
    laptop.modes = vec![
        Mode {
            preferred: true,
            ..Mode::new(2880, 1920, 120_000)
        },
        Mode::new(2880, 1920, 60_000),
        Mode::new(1920, 1280, 60_000),
    ];
    laptop.mode = laptop.modes.first().copied();
    laptop.scale = Scale::from_millis(2000).unwrap_or(Scale::ONE);
    laptop.adaptive_sync = Some(AdaptiveSync::Automatic);
    laptop.primary = true;

    let mut monitor = Output::new("DP-1");
    monitor.make = Some("Dell Inc.".into());
    monitor.model = Some("U2723QE".into());
    monitor.physical_size_mm = Some((597, 336));
    monitor.modes = vec![
        Mode {
            preferred: true,
            ..Mode::new(2560, 1440, 59_951)
        },
        Mode::new(1920, 1080, 60_000),
    ];
    monitor.mode = monitor.modes.first().copied();
    monitor.position = Position::new(1440, 0);
    monitor.adaptive_sync = Some(AdaptiveSync::Disabled);

    let mut tv = Output::new("HDMI-A-1");
    tv.make = Some("LG Electronics".into());
    tv.model = Some("OLED48C1".into());
    tv.enabled = false;
    tv.modes = vec![
        Mode {
            preferred: true,
            ..Mode::new(3840, 2160, 60_000)
        },
        Mode::new(1920, 1080, 60_000),
    ];
    Layout::new(vec![laptop, monitor, tv])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_app() -> App {
        let (mut app, _task) = App::new(Options {
            demo: true,
            ..Options::default()
        });
        let _ = app.update(Message::Loaded(Ok(Loaded {
            kind: None,
            caps: demo_capabilities(),
            layout: demo_layout(),
        })));
        app
    }

    #[test]
    fn demo_layout_is_valid() {
        assert_eq!(
            validate(&demo_layout(), &demo_capabilities()),
            [] as [vnr_core::Issue; 0]
        );
    }

    #[test]
    fn apply_then_timeout_reverts() {
        let mut app = demo_app();
        let original = app.applied.clone();
        let _ = app.update(Message::Select(0));
        let _ = app.update(Message::SetScale(Scale::ONE));
        assert!(app.can_apply());
        let _ = app.update(Message::Apply);
        let _ = app.update(Message::Applied(Ok(())));
        assert!(app.pending.is_some());
        for _ in 0..REVERT_SECONDS {
            let _ = app.update(Message::Tick);
        }
        let _ = app.update(Message::Reverted(Ok(())));
        assert!(app.pending.is_none());
        assert_eq!(app.applied, original);
    }

    #[test]
    fn keep_stops_the_countdown() {
        let mut app = demo_app();
        let _ = app.update(Message::Select(1));
        let _ = app.update(Message::Key(KeyCommand::Nudge(0, 10)));
        let _ = app.update(Message::Apply);
        let _ = app.update(Message::Applied(Ok(())));
        let _ = app.update(Message::Key(KeyCommand::Yes));
        assert!(app.pending.is_none());
        assert_eq!(app.applied.outputs[1].position.y, 10);
    }

    #[test]
    fn scaling_down_pushes_neighbor_away() {
        let mut app = demo_app();
        let _ = app.update(Message::Select(0));
        let _ = app.update(Message::SetScale(Scale::ONE));
        assert!(app.overlapping.is_empty(), "{:?}", app.issues);
    }

    #[test]
    fn errors_block_apply() {
        let mut app = demo_app();
        let _ = app.update(Message::Select(0));
        let _ = app.update(Message::SetEnabled(false));
        let _ = app.update(Message::Select(1));
        let _ = app.update(Message::SetEnabled(false));
        assert!(app.has_errors());
        assert!(!app.can_apply());
    }
}
