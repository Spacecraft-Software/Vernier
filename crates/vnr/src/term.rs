// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Modern terminal protocols, in the Spacecraft TUI protocols profile.
//!
//! `vnr` has no full-screen terminal interface yet (the `--format explore`
//! viewer, TODO T-017), reads keys only as cooked lines, and draws no images
//! or mouse regions. Of the profile's protocols it therefore speaks one:
//! OSC 9;4 progress, in the terminal's tab or taskbar, while it waits for a
//! change to be kept. The others are listed so the override syntax is the
//! same in every Spacecraft program and `describe` can say why each is off.
//!
//! The rules followed here:
//!
//! - Nothing is sent unless stderr is a terminal, no agent or CI is driving
//!   `vnr`, and `TERM` is set and not `dumb`.
//! - OSC 9;4 has no query, and many terminals show `OSC 9 ; text` as a
//!   desktop notification, so it is sent only to terminals known to draw
//!   progress ([`progress_terminal`]). Nothing is probed, so no reply can
//!   leak into the shell.
//! - `--term-protocols` and `SPACECRAFT_TERM_PROTOCOLS` force a protocol on
//!   or off; the flag beats the variable, name by name.
//! - An indicator that was shown is removed on every exit path: by
//!   [`Progress`]'s `Drop`, by the panic hook through [`restore`], and by
//!   the signal thread started with [`on_signal`].

use std::env;
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

/// The environment variable holding a protocol list, as `--term-protocols`.
pub const ENV: &str = "SPACECRAFT_TERM_PROTOCOLS";

/// The protocols of the Spacecraft TUI protocols profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Kitty keyboard protocol.
    Keyboard,
    /// Kitty graphics, sixel and inline images.
    Graphics,
    /// Kitty graphics animation.
    Animation,
    /// OSC 52 clipboard.
    Clipboard,
    /// OSC 5522 rich clipboard.
    ClipboardRich,
    /// OSC 9;4 progress.
    Progress,
    /// OSC 22 pointer shapes.
    Pointer,
}

impl Protocol {
    /// Every protocol, in the order `describe` lists them.
    pub const ALL: [Self; 7] = [
        Self::Keyboard,
        Self::Graphics,
        Self::Animation,
        Self::Clipboard,
        Self::ClipboardRich,
        Self::Progress,
        Self::Pointer,
    ];

    /// The name used in protocol lists.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Keyboard => "keyboard",
            Self::Graphics => "graphics",
            Self::Animation => "animation",
            Self::Clipboard => "clipboard",
            Self::ClipboardRich => "clipboard-rich",
            Self::Progress => "progress",
            Self::Pointer => "pointer",
        }
    }

    const fn bit(self) -> u8 {
        1 << self as u8
    }

    /// Why `vnr` never sends this protocol, or `None` when it can.
    const fn unused(self) -> Option<&'static str> {
        match self {
            Self::Progress => None,
            Self::Keyboard => Some("vnr reads no keys outside line input"),
            Self::Graphics | Self::Animation => Some("vnr shows no images in the terminal"),
            Self::Clipboard | Self::ClipboardRich => Some("vnr copies nothing to the clipboard"),
            Self::Pointer => Some("vnr has no mouse regions in the terminal"),
        }
    }
}

/// Protocols forced on or off by a list such as `-graphics,+progress`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Overrides {
    on: u8,
    off: u8,
}

impl Overrides {
    /// Parses a comma-separated list of `+name` (force on), `-name` (force
    /// off) or a bare `name` (on). `all` stands for every protocol; later
    /// entries win over earlier ones.
    ///
    /// # Errors
    ///
    /// Returns a message naming the unknown entry and the accepted names.
    pub fn parse(list: &str) -> Result<Self, String> {
        let mut overrides = Self::default();
        for entry in list.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let (on, name) = if let Some(name) = entry.strip_prefix('+') {
                (true, name)
            } else if let Some(name) = entry.strip_prefix('-') {
                (false, name)
            } else {
                (true, entry)
            };
            let bits = if name == "all" {
                Protocol::ALL.iter().fold(0, |bits, p| bits | p.bit())
            } else {
                Protocol::ALL
                    .iter()
                    .find(|p| p.name() == name)
                    .map(|p| p.bit())
                    .ok_or_else(|| {
                        format!(
                            "unknown terminal protocol `{name}`; use {} or all, each with + or -",
                            Protocol::ALL.map(Protocol::name).join(", ")
                        )
                    })?
            };
            if on {
                overrides.on |= bits;
                overrides.off &= !bits;
            } else {
                overrides.off |= bits;
                overrides.on &= !bits;
            }
        }
        Ok(overrides)
    }

    /// `self` with every protocol that `other` names taken from `other`.
    #[must_use]
    pub fn then(self, other: Self) -> Self {
        let named = other.on | other.off;
        Self {
            on: (self.on & !named) | other.on,
            off: (self.off & !named) | other.off,
        }
    }

    /// `Some(true)` when forced on, `Some(false)` when forced off.
    #[must_use]
    pub fn forced(self, protocol: Protocol) -> Option<bool> {
        if self.on & protocol.bit() != 0 {
            Some(true)
        } else if self.off & protocol.bit() != 0 {
            Some(false)
        } else {
            None
        }
    }
}

/// The parts of the environment that decide what may be sent.
#[derive(Debug, Clone, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is one independent environment signal"
)]
pub struct Terminal {
    /// Whether stderr, where the sequences go, is a terminal.
    pub stderr_tty: bool,
    /// Whether an agent or CI is driving `vnr`.
    pub automated: bool,
    /// `TERM`.
    pub term: Option<String>,
    /// `TERM_PROGRAM`.
    pub program: Option<String>,
    /// `TERM_PROGRAM_VERSION`.
    pub version: Option<String>,
    /// `WT_SESSION` is set: Windows Terminal.
    pub windows: bool,
    /// `ConEmuPID` is set or `ConEmuANSI` is `ON`: `ConEmu`.
    pub conemu: bool,
    /// `TMUX`, `ZELLIJ` or `STY` is set, or `TERM` starts with `tmux` or
    /// `screen`: a multiplexer sits in between.
    pub multiplexer: bool,
}

fn var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.is_empty())
}

/// Whether `TERM` names tmux or GNU screen, for wrappers that drop `TMUX`
/// or `STY` but keep the multiplexer's `TERM`.
fn multiplexer_term(term: Option<&str>) -> bool {
    term.is_some_and(|t| t.starts_with("tmux") || t.starts_with("screen"))
}

impl Terminal {
    /// Reads the process environment; `automated` comes from the output
    /// cascade, so agent detection lives in one place.
    #[must_use]
    pub fn read(automated: bool) -> Self {
        Self {
            stderr_tty: io::stderr().is_terminal(),
            automated,
            term: var("TERM"),
            program: var("TERM_PROGRAM"),
            version: var("TERM_PROGRAM_VERSION"),
            windows: var("WT_SESSION").is_some(),
            conemu: var("ConEmuPID").is_some() || var("ConEmuANSI").as_deref() == Some("ON"),
            multiplexer: ["TMUX", "ZELLIJ", "STY"].iter().any(|n| var(n).is_some())
                || multiplexer_term(var("TERM").as_deref()),
        }
    }

    /// Why no sequence at all may be sent, or `None` when some may.
    fn gate(&self) -> Option<&'static str> {
        if !self.stderr_tty {
            Some("stderr is not a terminal")
        } else if self.automated {
            Some("an agent or CI is driving vnr")
        } else if self.term.as_deref().is_none_or(|t| t == "dumb") {
            Some("TERM is dumb or unset")
        } else {
            None
        }
    }
}

/// The first `WezTerm` build that draws OSC 9;4 instead of showing it as a
/// notification (nightly of 2025-02-10; the 20240203 stable notifies).
#[expect(
    clippy::inconsistent_digit_grouping,
    reason = "grouped as a YYYY_MM_DD date"
)]
const WEZTERM_PROGRESS: u32 = 2025_02_10;

/// The first iTerm2 release that draws OSC 9;4 (3.6); older ones notify.
const ITERM_PROGRESS: (u32, u32) = (3, 6);

/// The terminal that draws OSC 9;4 progress, when identity says it does.
///
/// kitty draws progress only in newer releases and sets no version
/// variable; reading its version would need an XTVERSION probe, which
/// `vnr` does not send while a cooked-mode prompt owns the terminal. So
/// kitty gets no progress unless `+progress` forces it.
#[must_use]
pub fn progress_terminal(terminal: &Terminal) -> Option<&'static str> {
    if terminal.windows {
        return Some("Windows Terminal");
    }
    if terminal.conemu {
        return Some("ConEmu");
    }
    if terminal
        .term
        .as_deref()
        .is_some_and(|t| t.starts_with("foot"))
    {
        return Some("foot");
    }
    let program = terminal.program.as_deref()?.to_ascii_lowercase();
    let version = terminal.version.as_deref().unwrap_or("");
    for (needle, name) in [
        ("ghostty", "Ghostty"),
        ("vscode", "VS Code"),
        ("rio", "Rio"),
        ("contour", "contour"),
    ] {
        if program.contains(needle) {
            return Some(name);
        }
    }
    if program == "wezterm" {
        let date: String = version.chars().take_while(char::is_ascii_digit).collect();
        return (date.len() == 8 && date.parse::<u32>().is_ok_and(|d| d >= WEZTERM_PROGRESS))
            .then_some("WezTerm");
    }
    if program.contains("iterm") {
        let mut parts = version.split('.').map(|p| p.parse::<u32>().ok());
        let release = (parts.next().flatten(), parts.next().flatten());
        return matches!(release, (Some(major), Some(minor)) if (major, minor) >= ITERM_PROGRESS)
            .then_some("iTerm2");
    }
    None
}

/// One protocol's resolved state, as `describe` reports it.
#[derive(Debug, Clone, Serialize)]
pub struct State {
    /// The protocol's name in protocol lists.
    pub name: &'static str,
    /// Whether `vnr` will send it.
    pub enabled: bool,
    /// Why.
    pub reason: String,
}

/// What may be sent to this terminal.
#[derive(Debug, Clone, Default)]
pub struct Resolved {
    /// Whether OSC 9;4 progress may be sent.
    pub progress: bool,
    states: Vec<State>,
}

impl Resolved {
    /// Every protocol's state, for `describe` and `--verbose`.
    #[must_use]
    pub fn states(&self) -> &[State] {
        &self.states
    }

    /// One `--verbose` line: what is on, and why the rest is off.
    #[must_use]
    pub fn summary(&self) -> String {
        let shown: Vec<String> = self
            .states
            .iter()
            .filter(|s| s.name == Protocol::Progress.name() || s.enabled)
            .map(|s| {
                let state = if s.enabled { "on" } else { "off" };
                format!("{} {state} ({})", s.name, s.reason)
            })
            .collect();
        format!(
            "terminal protocols: {}; the others are not used",
            shown.join(", ")
        )
    }
}

/// Decides each protocol from the gate, the override list and identity.
#[must_use]
pub fn resolve(terminal: &Terminal, overrides: Overrides) -> Resolved {
    let gate = terminal.gate();
    let mut progress = false;
    let states = Protocol::ALL
        .iter()
        .map(|&protocol| {
            let (enabled, reason) = if let Some(why) = protocol.unused() {
                (false, why.to_owned())
            } else if let Some(why) = gate {
                (false, why.to_owned())
            } else {
                match overrides.forced(protocol) {
                    Some(true) => (true, "forced on".to_owned()),
                    Some(false) => (false, "forced off".to_owned()),
                    None if terminal.multiplexer => {
                        (false, "a terminal multiplexer is in between".to_owned())
                    }
                    None => match progress_terminal(terminal) {
                        Some(name) => (true, format!("{name} draws it")),
                        None => (false, "this terminal is not known to draw it".to_owned()),
                    },
                }
            };
            if protocol == Protocol::Progress {
                progress = enabled;
            }
            State {
                name: protocol.name(),
                enabled,
                reason,
            }
        })
        .collect();
    Resolved { progress, states }
}

/// OSC 9;4 with state 0: remove the indicator.
const CLEAR: &[u8] = b"\x1b]9;4;0;0\x1b\\";

/// Set while an indicator is shown, so [`restore`] knows to remove it.
static SHOWN: AtomicBool = AtomicBool::new(false);

/// Set by [`restore`]: the process is ending, so nothing more is shown.
static CLOSED: AtomicBool = AtomicBool::new(false);

/// How long a signal waits for [`restore`] before ending the process
/// anyway, in case output to the terminal is stopped (Ctrl-S).
const RESTORE_WAIT: Duration = Duration::from_millis(200);

/// Writes a progress update unless [`restore`] has run. Both hold the
/// stderr lock while deciding, so no update can follow the final clear.
fn show(sequence: &[u8]) {
    let mut stderr = io::stderr().lock();
    if CLOSED.load(Ordering::SeqCst) {
        return;
    }
    SHOWN.store(true, Ordering::SeqCst);
    let _ = stderr.write_all(sequence);
    let _ = stderr.flush();
}

/// Removes a progress indicator left by a [`Progress`] that could not drop,
/// and stops any further update: the panic hook and the signal thread call
/// this. A second call does nothing.
pub fn restore() {
    let mut stderr = io::stderr().lock();
    CLOSED.store(true, Ordering::SeqCst);
    if SHOWN.swap(false, Ordering::SeqCst) {
        let _ = stderr.write_all(CLEAR);
        let _ = stderr.flush();
    }
}

/// Removes the indicator when the [`Progress`] showing it is done; later
/// updates may show a new one.
fn clear() {
    let mut stderr = io::stderr().lock();
    if SHOWN.swap(false, Ordering::SeqCst) {
        let _ = stderr.write_all(CLEAR);
        let _ = stderr.flush();
    }
}

/// Starts [`restore`] on a thread of its own and returns what to wait on
/// for it, so the signal thread never blocks on a paused terminal.
fn start_restore() -> mpsc::Receiver<()> {
    let (done, finished) = mpsc::channel();
    thread::spawn(move || {
        restore();
        let _ = done.send(());
    });
    finished
}

/// An OSC 9;4 indicator in the terminal's tab or taskbar, removed on drop.
///
/// Decorative only: whatever it shows, `vnr` also says in text.
#[derive(Debug)]
pub struct Progress {
    enabled: bool,
    last: Option<u8>,
}

impl Progress {
    /// An indicator that sends anything only when `enabled`.
    #[must_use]
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last: None,
        }
    }

    /// Shows `percent` (capped at 100), sending only when it changed.
    pub fn percent(&mut self, percent: u8) {
        let percent = percent.min(100);
        if self.enabled && self.last != Some(percent) {
            self.last = Some(percent);
            show(format!("\x1b]9;4;1;{percent}\x1b\\").as_bytes());
        }
    }

    /// Shows a spinner: busy, with no known end.
    pub fn busy(&mut self) {
        if self.enabled {
            // Never equal to a percentage, so a later `percent` is sent.
            self.last = Some(u8::MAX);
            show(b"\x1b]9;4;3;0\x1b\\");
        }
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        if self.enabled {
            clear();
        }
    }
}

/// Handles SIGINT, SIGTERM and SIGHUP from now on, on a thread of their own:
/// each removes any indicator, then calls `handled`; when that returns
/// false, the signal does what it would have done (usually, end `vnr`).
///
/// Handling cannot be undone, so call this only where every later signal
/// may go to `handled`. Failing to install is not an error: signals then
/// keep their default effect.
pub fn on_signal(handled: impl Fn(i32) -> bool + Send + 'static) {
    let Ok(mut signals) = Signals::new([SIGINT, SIGTERM, SIGHUP]) else {
        return;
    };
    thread::spawn(move || {
        // The signal thread never waits before handing a signal on: a
        // signal arriving while it waited would be merged with the next.
        let mut restoring = None;
        for signal in signals.forever() {
            let finished = restoring.get_or_insert_with(start_restore);
            if !handled(signal) {
                // Give the indicator a moment to go, but end regardless.
                let _ = finished.recv_timeout(RESTORE_WAIT);
                let _ = signal_hook::low_level::emulate_default_handler(signal);
                // The default for these three ends the process; were it
                // ignored, end it the way a shell reports a signal.
                std::process::exit(128 + signal);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tty(program: &str, version: &str) -> Terminal {
        Terminal {
            stderr_tty: true,
            term: Some("xterm-256color".into()),
            program: Some(program.into()),
            version: Some(version.into()),
            ..Terminal::default()
        }
    }

    fn progress(terminal: &Terminal, list: &str) -> bool {
        resolve(terminal, Overrides::parse(list).expect("valid")).progress
    }

    #[test]
    fn lists_parse_and_later_entries_win() {
        let o = Overrides::parse("-all,+progress").expect("valid");
        assert_eq!(o.forced(Protocol::Progress), Some(true));
        assert_eq!(o.forced(Protocol::Keyboard), Some(false));
        let o = Overrides::parse(" progress , -progress").expect("valid");
        assert_eq!(o.forced(Protocol::Progress), Some(false));
        assert_eq!(Overrides::parse("").expect("valid"), Overrides::default());
        let e = Overrides::parse("+progres").expect_err("unknown");
        assert!(
            e.contains("`progres`") && e.contains("clipboard-rich"),
            "{e}"
        );
    }

    #[test]
    fn the_flag_beats_the_variable_name_by_name() {
        let env = Overrides::parse("-progress,-graphics").expect("valid");
        let flag = Overrides::parse("+progress").expect("valid");
        let both = env.then(flag);
        assert_eq!(both.forced(Protocol::Progress), Some(true));
        assert_eq!(both.forced(Protocol::Graphics), Some(false));
    }

    #[test]
    fn progress_goes_only_to_terminals_that_draw_it() {
        assert!(progress(&tty("WezTerm", "20250815-123456-abcdef"), ""));
        // The 2024 stable shows OSC 9 as a notification.
        assert!(!progress(&tty("WezTerm", "20240203-110809-5046fc22"), ""));
        assert!(!progress(&tty("WezTerm", ""), ""));
        assert!(progress(&tty("ghostty", "1.2.0"), ""));
        assert!(progress(&tty("iTerm.app", "3.6.1"), ""));
        assert!(!progress(&tty("iTerm.app", "3.5.10"), ""));
        assert!(!progress(&tty("Apple_Terminal", "455"), ""));
        let kitty = Terminal {
            program: None,
            term: Some("xterm-kitty".into()),
            ..tty("", "")
        };
        assert!(!progress(&kitty, ""));
        assert!(progress(&kitty, "+progress"));
        let foot = Terminal {
            program: None,
            term: Some("foot-extra".into()),
            ..tty("", "")
        };
        assert!(progress(&foot, ""));
        let windows = Terminal {
            windows: true,
            ..tty("", "")
        };
        assert!(progress(&windows, ""));
    }

    #[test]
    fn overrides_never_beat_the_gate() {
        let agent = Terminal {
            automated: true,
            ..tty("ghostty", "")
        };
        assert!(!progress(&agent, "+progress"));
        let piped = Terminal {
            stderr_tty: false,
            ..tty("ghostty", "")
        };
        assert!(!progress(&piped, "+all"));
        let dumb = Terminal {
            term: Some("dumb".into()),
            ..tty("ghostty", "")
        };
        assert!(!progress(&dumb, "+progress"));
        let unset = Terminal {
            term: None,
            ..tty("ghostty", "")
        };
        assert!(!progress(&unset, ""));
    }

    #[test]
    fn multiplexers_get_progress_only_when_forced() {
        let tmux = Terminal {
            multiplexer: true,
            ..tty("ghostty", "")
        };
        assert!(!progress(&tmux, ""));
        assert!(progress(&tmux, "+progress"));
        for term in ["tmux-256color", "screen", "screen.xterm-256color"] {
            assert!(multiplexer_term(Some(term)), "{term}");
        }
        assert!(!multiplexer_term(Some("xterm-256color")));
        assert!(!multiplexer_term(None));
        assert!(!progress(&tty("ghostty", ""), "-progress"));
    }

    #[test]
    fn unused_protocols_stay_off_and_say_why() {
        let r = resolve(
            &tty("ghostty", ""),
            Overrides::parse("+all").expect("valid"),
        );
        assert_eq!(r.states().len(), Protocol::ALL.len());
        for state in r.states() {
            assert_eq!(state.enabled, state.name == "progress", "{state:?}");
        }
        assert_eq!(
            r.summary(),
            "terminal protocols: progress on (forced on); the others are not used"
        );
    }
}
