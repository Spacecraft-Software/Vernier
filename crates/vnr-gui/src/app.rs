// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Application state, update logic and view.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{env, fs, io};

use iced::futures::channel::oneshot;
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{
    Canvas, Column, button, checkbox, column, container, pick_list, row, rule, scrollable, text,
    text_input, toggler,
};
use iced::{Border, Element, Font, Length, Subscription, Task, Theme, time};
use vnr_core::geometry::{normalize, reflow, resolve_overlap};
use vnr_core::{
    AdaptiveSync, BackendKind, Capabilities, Issue, Layout, Mode, Output, Position, Saved, Scale,
    Severity, Transform, export, validate,
};

use vnr_config::{Action, Chord, KeyMap};

use crate::Options;
use crate::arrangement::{Arrangement, CanvasEvent, Shot};
use crate::overlay::Overlays;
use crate::preview;
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
    /// Select the next output. (Tab is left to move keyboard focus.)
    Next,
    /// Select the previous output.
    Previous,
    /// Move the selected output by this many pixels.
    Nudge(i32, i32),
    /// Apply.
    Apply,
    /// Revert a pending change, otherwise discard edits.
    Escape,
    /// Reload from the display server.
    Reload,
    /// Keep a pending change.
    Yes,
    /// Revert a pending change.
    No,
    /// Show or hide the screen contents.
    Previews,
}

impl KeyCommand {
    /// The command an action means; `shift` moves ten pixels instead of one.
    const fn from_action(action: Action, shift: bool) -> Self {
        let step = if shift { 10 } else { 1 };
        match action {
            Action::Apply => Self::Apply,
            Action::Reload => Self::Reload,
            Action::Discard => Self::Escape,
            Action::Keep => Self::Yes,
            Action::Revert => Self::No,
            Action::Next => Self::Next,
            Action::Previous => Self::Previous,
            Action::Left => Self::Nudge(-step, 0),
            Action::Right => Self::Nudge(step, 0),
            Action::Up => Self::Nudge(0, -step),
            Action::Down => Self::Nudge(0, step),
            Action::Previews => Self::Previews,
        }
    }
}

/// A key press, before the key map decides what it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPress {
    chord: Chord,
    shift: bool,
    /// No widget used the key press.
    free: bool,
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
    /// Write the applied layout to niri's included `vernier.kdl`.
    SaveNiri,
    /// The niri include file was written, or why not.
    SavedNiri(Result<PathBuf, String>),
    /// Ask the display server (GNOME) to save the applied layout; it shows
    /// its own keep-or-revert dialog.
    SaveServer,
    /// The server's dialog was answered, or why saving failed.
    SavedServer(Result<Saved, String>),
    /// Re-read the configuration from the display server.
    Reload,
    /// Discard edits.
    Reset,
    /// A key was pressed; the key map decides what it does.
    KeyPress(KeyPress),
    /// The screen-contents switch changed.
    SetPreviews(bool),
    /// The capture thread sent a picture, or gave up.
    Preview(preview::Event),
}

/// The editor.
#[derive(Debug)]
pub struct App {
    options: Options,
    keys: KeyMap,
    /// Screen labels; `None` in demo mode, when turned off, or without layer-shell.
    overlays: Option<Overlays>,
    /// Screen contents are shown; never in demo mode.
    previews: bool,
    /// The latest picture of each output, by name.
    shots: BTreeMap<String, Shot>,
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

/// Where "Save for niri" writes: `vernier.kdl` next to niri's config.
fn niri_include_path() -> Option<PathBuf> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("niri").join(export::NIRI_INCLUDE_FILE))
}

/// Writes `layout` as niri `output` blocks to `path`, replacing only a file
/// Vernier generated earlier, never one a person wrote.
fn save_niri(path: &Path, layout: &Layout) -> Result<(), String> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!(
            "{} is a symbolic link; Save for niri writes only regular files",
            path.display()
        ));
    }
    match fs::read_to_string(path) {
        Ok(existing) if !export::is_generated(&existing) => {
            return Err(format!(
                "{} was not written by Vernier; move it aside first",
                path.display()
            ));
        }
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            return Err(format!("cannot read {}: {e}", path.display()));
        }
        _ => {}
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    fs::write(path, export::niri(layout))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Has the server save `desired`, which the editor believes is on screen;
/// blocks until the server's own dialog is answered. Refuses when the
/// displays changed since the editor read them, so a save never overwrites
/// someone else's change, and tests first, as every apply does.
fn save(backend: Option<BackendKind>, desired: &Layout) -> Result<Saved, String> {
    let mut backend = vnr_backends::open(backend).map_err(|e| e.to_string())?;
    if backend.snapshot().map_err(|e| e.to_string())? != *desired {
        return Err("the displays changed since the editor read them; press Reload".to_owned());
    }
    if backend.capabilities().test_only {
        backend.test(desired).map_err(|e| e.to_string())?;
    }
    backend.apply_and_save(desired).map_err(|e| e.to_string())
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
        let overlays = (options.config.overlays && !options.demo)
            .then(|| {
                let tokens = vnr_theme::resolve(&vnr_theme::Request {
                    theme: options.theme.clone(),
                    accessible: options.accessible,
                })
                .tokens;
                Overlays::start(tokens)
            })
            .flatten();
        let app = Self {
            keys: options.config.keys.clone(),
            overlays,
            previews: options.config.previews && !options.demo,
            shots: BTreeMap::new(),
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
            status: options
                .config_problem
                .as_ref()
                .map_or_else(String::new, |p| format!("Default key bindings: {p}")),
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

    /// Listens to the keyboard always, ticks once a second while a
    /// confirmation is pending, and receives screen contents while they
    /// are shown. Turning previews off drops the capture subscription,
    /// which stops its thread.
    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![iced::event::listen_with(key_command)];
        if self.pending.is_some() {
            subscriptions.push(time::every(Duration::from_secs(1)).map(|_| Message::Tick));
        }
        if self.previews {
            subscriptions.push(Subscription::run(preview::start).map(Message::Preview));
        }
        Subscription::batch(subscriptions)
    }

    fn set_previews(&mut self, on: bool) {
        if on && self.options.demo {
            "The demo layout has no screens to show.".clone_into(&mut self.status);
            return;
        }
        self.previews = on;
        if !on {
            self.shots.clear();
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

    fn selected_rect(&self) -> Option<vnr_core::geometry::Rect> {
        self.selected_output()
            .and_then(vnr_core::Output::logical_rect)
    }

    fn geometry_changed(&mut self, old: Option<vnr_core::geometry::Rect>) {
        if let Some(index) = self.selected {
            if let Some(old) = old {
                reflow(&mut self.draft, index, old);
            }
            resolve_overlap(&mut self.draft, index);
            normalize(&mut self.draft);
        }
        self.edited();
    }

    fn select(&mut self, index: Option<usize>) {
        self.selected = index.filter(|i| *i < self.draft.outputs.len());
        self.sync_position_text();
        if let Some(overlays) = &self.overlays {
            overlays.select(self.selected_output().map(|o| o.name.as_str()));
        }
    }

    fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.severity() == Severity::Error)
    }

    /// Whether the session is wlr-based (niri included) and forgets
    /// changes on restart, so saving the niri include file helps.
    fn offers_niri_save(&self) -> bool {
        self.kind == Some(BackendKind::Wlr) && !self.caps.persistent && !self.options.demo
    }

    /// "Save for niri" writes what is on screen, so only with no edits
    /// waiting and no change waiting for keep-or-revert.
    fn can_save_niri(&self) -> bool {
        self.offers_niri_save()
            && self.busy.is_none()
            && self.pending.is_none()
            && self.draft == self.applied
    }

    /// Whether the server can save a layout on request (GNOME).
    fn offers_server_save(&self) -> bool {
        self.caps.save_on_request && !self.options.demo
    }

    /// Saving asks the server to keep what is on screen, so, as with
    /// "Save for niri", only with no edits or confirmation waiting.
    fn can_save_server(&self) -> bool {
        self.offers_server_save()
            && self.busy.is_none()
            && self.pending.is_none()
            && self.draft == self.applied
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
            Message::Keep => self.keep(),
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
            Message::SaveNiri => return self.start_save_niri(),
            Message::SavedNiri(result) => self.finish_save_niri(result),
            Message::SaveServer => return self.start_save_server(),
            Message::SavedServer(result) => return self.finish_save_server(result),
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
            Message::SetPreviews(on) => self.set_previews(on),
            Message::Preview(preview::Event::Frame { output, thumbnail }) => {
                // A picture may still arrive just after previews went off.
                if self.previews {
                    self.shots.insert(output, Shot::new(thumbnail));
                }
            }
            Message::Preview(preview::Event::Unavailable(why)) => {
                self.set_previews(false);
                self.status = format!("Screen contents unavailable: {why}");
            }
            Message::KeyPress(press) => {
                return match self.keys.action(&press.chord) {
                    Some(action) if press.free || press.chord.always_applies() => {
                        self.key(KeyCommand::from_action(action, press.shift))
                    }
                    _ => Task::none(),
                };
            }
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
                let old = self.selected_rect();
                if let Some(output) = self.selected_output_mut() {
                    output.mode = Some(mode);
                }
                self.geometry_changed(old);
            }
            Message::SetScale(scale) => {
                let old = self.selected_rect();
                if let Some(output) = self.selected_output_mut() {
                    output.scale = scale;
                }
                self.geometry_changed(old);
            }
            Message::SetTransform(transform) => {
                let old = self.selected_rect();
                if let Some(output) = self.selected_output_mut() {
                    output.transform = transform;
                }
                self.geometry_changed(old);
            }
            Message::SetAdaptiveSync(sync) => {
                if let Some(output) = self.selected_output_mut() {
                    output.adaptive_sync = Some(sync);
                }
                self.edited();
            }
            Message::SetMirror(choice) => {
                let old = self.selected_rect();
                if let Some(output) = self.selected_output_mut() {
                    output.mirror_of = match choice {
                        MirrorChoice::Off => None,
                        MirrorChoice::Of(name) => Some(name),
                    };
                }
                self.geometry_changed(old);
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
            KeyCommand::Previews => self.update(Message::SetPreviews(!self.previews)),
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

    fn start_save_niri(&mut self) -> Task<Message> {
        if !self.can_save_niri() {
            return Task::none();
        }
        let Some(path) = niri_include_path() else {
            "Cannot find the config directory: HOME is not set.".clone_into(&mut self.status);
            return Task::none();
        };
        self.busy = Some("Saving");
        let layout = self.applied.clone();
        Task::perform(
            background(move || save_niri(&path, &layout).map(|()| path)),
            Message::SavedNiri,
        )
    }

    fn finish_save_niri(&mut self, result: Result<PathBuf, String>) {
        self.busy = None;
        self.status = match result {
            Ok(path) => format!(
                "Saved to {}. niri loads it through `include \"{}\"` in config.kdl.",
                path.display(),
                export::NIRI_INCLUDE_FILE
            ),
            Err(error) => format!("Not saved: {error}"),
        };
    }

    /// Confirms the pending configuration and says how long it lasts.
    fn keep(&mut self) {
        if self.pending.take().is_some() {
            self.status = if self.caps.persistent || self.options.demo {
                "Kept.".to_owned()
            } else if self.caps.save_on_request {
                "Kept until you log out. Save for GNOME to keep it after that.".to_owned()
            } else {
                "Kept until the compositor restarts. Save for niri to make it permanent.".to_owned()
            };
        }
    }

    fn start_save_server(&mut self) -> Task<Message> {
        if !self.can_save_server() {
            return Task::none();
        }
        self.busy = Some("Waiting for GNOME");
        "GNOME is asking whether to keep this layout: choose Keep Changes within 20 seconds to save it."
            .clone_into(&mut self.status);
        let layout = self.applied.clone();
        let backend = self.options.backend;
        Task::perform(
            background(move || save(backend, &layout)),
            Message::SavedServer,
        )
    }

    fn finish_save_server(&mut self, result: Result<Saved, String>) -> Task<Message> {
        self.busy = None;
        self.status = match result {
            Ok(Saved::Kept) => "Saved. GNOME restores this layout at every login.".to_owned(),
            Ok(Saved::Restored) => "Not saved: GNOME restored the previous layout.".to_owned(),
            Ok(Saved::Unverified) => "Kept on screen, but GNOME gave no sign it saved the layout; \
                                      it may last only until you log out."
                .to_owned(),
            Err(error) => format!("Not saved: {error}"),
        };
        self.reload_task()
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
            shots: &self.shots,
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
            row![text(source), text(status).width(Length::Fill)]
                .push((!self.options.demo).then(|| {
                    checkbox(self.previews)
                        .label("Show screen contents")
                        .on_toggle(Message::SetPreviews)
                }))
                .push(
                    row![
                        button("Reload")
                            .style(button::secondary)
                            .on_press_maybe(idle.then_some(Message::Reload)),
                        button("Reset")
                            .style(button::secondary)
                            .on_press_maybe((idle && edited).then_some(Message::Reset)),
                        button("Apply").on_press_maybe(self.can_apply().then_some(Message::Apply)),
                    ]
                    .spacing(12),
                )
                .push(self.offers_niri_save().then(|| {
                    button("Save for niri")
                        .style(button::secondary)
                        .on_press_maybe(self.can_save_niri().then_some(Message::SaveNiri))
                }))
                .push(self.offers_server_save().then(|| {
                    button("Save for GNOME")
                        .style(button::secondary)
                        .on_press_maybe(self.can_save_server().then_some(Message::SaveServer))
                }))
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
    Some(Message::KeyPress(KeyPress {
        chord: Chord::new(
            modifiers.control(),
            modifiers.alt(),
            chord_key(&key.as_ref())?,
        ),
        shift: modifiers.shift(),
        free: status == iced::event::Status::Ignored,
    }))
}

/// The configuration's name for an iced key, if it can be bound.
fn chord_key(key: &Key<&str>) -> Option<vnr_config::Key> {
    use vnr_config::Key as K;
    Some(match key {
        Key::Character(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => K::character(c),
                _ => return None,
            }
        }
        Key::Named(named) => match named {
            Named::Enter => K::Enter,
            Named::Escape => K::Escape,
            Named::Space => K::Space,
            Named::Backspace => K::Backspace,
            Named::Delete => K::Delete,
            Named::Insert => K::Insert,
            Named::Home => K::Home,
            Named::End => K::End,
            Named::PageUp => K::PageUp,
            Named::PageDown => K::PageDown,
            Named::ArrowLeft => K::Left,
            Named::ArrowRight => K::Right,
            Named::ArrowUp => K::Up,
            Named::ArrowDown => K::Down,
            other => K::Function(function_number(*other)?),
        },
        Key::Unidentified => return None,
    })
}

/// F1 to F24 as their number.
fn function_number(named: Named) -> Option<u8> {
    const KEYS: [Named; 24] = [
        Named::F1,
        Named::F2,
        Named::F3,
        Named::F4,
        Named::F5,
        Named::F6,
        Named::F7,
        Named::F8,
        Named::F9,
        Named::F10,
        Named::F11,
        Named::F12,
        Named::F13,
        Named::F14,
        Named::F15,
        Named::F16,
        Named::F17,
        Named::F18,
        Named::F19,
        Named::F20,
        Named::F21,
        Named::F22,
        Named::F23,
        Named::F24,
    ];
    KEYS.iter()
        .position(|k| *k == named)
        .and_then(|i| u8::try_from(i + 1).ok())
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
        save_on_request: false,
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
        let _ = app.key(KeyCommand::Nudge(0, 10));
        let _ = app.update(Message::Apply);
        let _ = app.update(Message::Applied(Ok(())));
        let _ = app.key(KeyCommand::Yes);
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
    fn save_niri_replaces_only_its_own_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("niri").join(export::NIRI_INCLUDE_FILE);
        let layout = demo_layout();
        save_niri(&path, &layout).unwrap();
        let first = fs::read_to_string(&path).unwrap();
        assert!(export::is_generated(&first));
        save_niri(&path, &layout).unwrap();

        fs::write(&path, "output \"eDP-1\" { off; }\n").unwrap();
        assert!(save_niri(&path, &layout).is_err());

        // A symlink, even to a generated file, is never followed.
        let target = dir.path().join("elsewhere.kdl");
        fs::write(&target, &first).unwrap();
        let link = dir.path().join("niri").join("link.kdl");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(save_niri(&link, &layout).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), first);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "output \"eDP-1\" { off; }\n"
        );
    }

    #[test]
    fn server_save_is_offered_only_where_the_server_can_save() {
        let mut app = demo_app();
        app.caps.save_on_request = true;
        assert!(!app.offers_server_save(), "never in demo mode");
        app.options.demo = false;
        assert!(app.can_save_server());
        let _ = app.update(Message::Select(0));
        let _ = app.update(Message::SetScale(Scale::ONE));
        assert!(!app.can_save_server(), "unapplied edits are not saved");
        app.caps.save_on_request = false;
        assert!(!app.offers_server_save());
    }

    #[test]
    fn a_server_answer_is_reported() {
        let mut app = demo_app();
        app.options.demo = false;
        app.busy = Some("Waiting for GNOME");
        let _ = app.update(Message::SavedServer(Ok(Saved::Restored)));
        assert!(app.status.contains("restored"), "{}", app.status);
        let _ = app.update(Message::SavedServer(Ok(Saved::Kept)));
        assert!(app.status.starts_with("Saved."), "{}", app.status);
        let _ = app.update(Message::SavedServer(Ok(Saved::Unverified)));
        assert!(app.status.contains("no sign"), "{}", app.status);
    }

    #[test]
    fn demo_never_offers_niri_save() {
        let mut app = demo_app();
        app.kind = Some(BackendKind::Wlr);
        assert!(!app.offers_niri_save());
        app.options.demo = false;
        assert!(app.can_save_niri());
        let _ = app.update(Message::Select(0));
        let _ = app.update(Message::SetScale(Scale::ONE));
        assert!(!app.can_save_niri(), "unapplied edits are not saved");
    }

    fn press(app: &mut App, text: &str, free: bool) {
        let _ = app.update(Message::KeyPress(KeyPress {
            chord: text.parse().unwrap(),
            shift: false,
            free,
        }));
    }

    #[test]
    fn plain_keys_wait_for_a_free_press_and_rebinding_moves_them() {
        let mut app = demo_app();
        let _ = app.update(Message::Select(1));
        let x = app.draft.outputs[1].position.x;
        press(&mut app, "l", false);
        assert_eq!(
            app.draft.outputs[1].position.x, x,
            "a text field had the key"
        );
        press(&mut app, "l", true);
        assert_eq!(app.draft.outputs[1].position.x, x + 1);

        let right = vec!["d".to_owned()];
        app.keys = KeyMap::with_overrides([("right", right.as_slice())]).unwrap();
        press(&mut app, "l", true);
        assert_eq!(app.draft.outputs[1].position.x, x + 1, "l is unbound now");
        press(&mut app, "d", true);
        assert_eq!(app.draft.outputs[1].position.x, x + 2);
    }

    #[test]
    fn function_keys_map_by_number() {
        assert_eq!(function_number(Named::F5), Some(5));
        assert_eq!(function_number(Named::F24), Some(24));
        assert_eq!(function_number(Named::Tab), None);
        assert_eq!(chord_key(&Key::Named(Named::Tab)), None);
        assert_eq!(
            chord_key(&Key::Character("S")),
            Some(vnr_config::Key::Character('s'))
        );
    }

    fn frame(output: &str) -> Message {
        Message::Preview(preview::Event::Frame {
            output: output.to_owned(),
            thumbnail: preview::Thumbnail {
                width: 1,
                height: 1,
                rgba: vec![0, 0, 0, 255],
            },
        })
    }

    #[test]
    fn the_demo_never_captures_the_screen() {
        let mut app = demo_app();
        press(&mut app, "p", true);
        assert!(!app.previews);
        assert!(app.status.contains("demo"), "{}", app.status);
        let _ = app.update(frame("eDP-1"));
        assert!(app.shots.is_empty());
    }

    #[test]
    fn previews_toggle_and_give_up_when_unavailable() {
        let mut app = demo_app();
        app.options.demo = false;
        press(&mut app, "Ctrl+p", false);
        assert!(app.previews, "Ctrl+p applies even inside a text field");
        let _ = app.update(frame("eDP-1"));
        assert_eq!(app.shots.len(), 1);

        let _ = app.update(Message::SetPreviews(false));
        assert!(app.shots.is_empty(), "turning off forgets the pictures");
        let _ = app.update(frame("eDP-1"));
        assert!(app.shots.is_empty(), "a late picture is ignored");

        let _ = app.update(Message::SetPreviews(true));
        let _ = app.update(Message::Preview(preview::Event::Unavailable(
            "no capture".to_owned(),
        )));
        assert!(!app.previews);
        assert!(app.status.contains("no capture"), "{}", app.status);
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
