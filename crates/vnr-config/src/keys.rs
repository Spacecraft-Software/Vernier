// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Editor actions, key chords and the map between them.
//!
//! Independent of any toolkit: the editor turns its key events into a
//! [`Chord`] and asks the [`KeyMap`] which [`Action`] it triggers.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

/// Something the editor can do from the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    /// Apply the edited layout.
    Apply,
    /// Re-read the configuration from the display server.
    Reload,
    /// Revert a pending change, otherwise discard edits.
    Discard,
    /// Keep a pending change.
    Keep,
    /// Revert a pending change.
    Revert,
    /// Select the next output.
    Next,
    /// Select the previous output.
    Previous,
    /// Move the selected output left (Shift: by 10).
    Left,
    /// Move the selected output right (Shift: by 10).
    Right,
    /// Move the selected output up (Shift: by 10).
    Up,
    /// Move the selected output down (Shift: by 10).
    Down,
    /// Show or hide the screen contents inside each output.
    Previews,
}

impl Action {
    /// Every action, in the order the documentation lists them.
    pub const ALL: [Self; 12] = [
        Self::Apply,
        Self::Reload,
        Self::Discard,
        Self::Keep,
        Self::Revert,
        Self::Next,
        Self::Previous,
        Self::Left,
        Self::Right,
        Self::Up,
        Self::Down,
        Self::Previews,
    ];

    /// The action's name in the configuration file.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Reload => "reload",
            Self::Discard => "discard",
            Self::Keep => "keep",
            Self::Revert => "revert",
            Self::Next => "next",
            Self::Previous => "previous",
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
            Self::Previews => "previews",
        }
    }

    /// The chords bound when the configuration file does not say otherwise:
    /// CUA keys first, then Vim keys.
    #[must_use]
    pub const fn defaults(self) -> &'static [&'static str] {
        match self {
            Self::Apply => &["Ctrl+Enter", "Ctrl+s"],
            Self::Reload => &["F5", "Ctrl+r"],
            Self::Discard => &["Escape"],
            Self::Keep => &["y"],
            Self::Revert => &["n"],
            Self::Next => &["PageDown", "]"],
            Self::Previous => &["PageUp", "["],
            Self::Left => &["Left", "h"],
            Self::Right => &["Right", "l"],
            Self::Up => &["Up", "k"],
            Self::Down => &["Down", "j"],
            Self::Previews => &["Ctrl+p", "p"],
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Action {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|a| a.as_str() == s)
            .ok_or_else(|| {
                let names: Vec<&str> = Self::ALL.iter().map(|a| a.as_str()).collect();
                format!("unknown action `{s}`; expected one of {}", names.join(", "))
            })
    }
}

/// A key without modifiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    /// A key that types a character, stored in lower case.
    Character(char),
    /// Enter or Return.
    Enter,
    /// Escape.
    Escape,
    /// The space bar.
    Space,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Insert.
    Insert,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// A function key, F1 to F24.
    Function(u8),
}

/// Named keys and how they are written, in matching order.
const NAMED: [(&str, Key); 14] = [
    ("Enter", Key::Enter),
    ("Escape", Key::Escape),
    ("Space", Key::Space),
    ("Backspace", Key::Backspace),
    ("Delete", Key::Delete),
    ("Insert", Key::Insert),
    ("Home", Key::Home),
    ("End", Key::End),
    ("PageUp", Key::PageUp),
    ("PageDown", Key::PageDown),
    ("Left", Key::Left),
    ("Right", Key::Right),
    ("Up", Key::Up),
    ("Down", Key::Down),
];

impl Key {
    /// A typed character; letters are folded to lower case, because Shift
    /// changes the step of a move rather than the binding.
    #[must_use]
    pub fn character(c: char) -> Self {
        Self::Character(c.to_lowercase().next().unwrap_or(c))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Character(c) => write!(f, "{c}"),
            Self::Function(n) => write!(f, "F{n}"),
            named => {
                let name = NAMED
                    .iter()
                    .find(|(_, k)| k == named)
                    .map_or("?", |(name, _)| name);
                f.write_str(name)
            }
        }
    }
}

/// A key with the modifiers that must be held. Shift is not part of a
/// chord: it multiplies moves by ten, and it is ignored when matching.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    /// Control held.
    pub ctrl: bool,
    /// Alt held.
    pub alt: bool,
    /// The key pressed.
    pub key: Key,
}

impl Chord {
    /// A chord for `key` with the given modifiers.
    #[must_use]
    pub const fn new(ctrl: bool, alt: bool, key: Key) -> Self {
        Self { ctrl, alt, key }
    }

    /// Whether the chord acts even while a text field or another control
    /// has used the key press. Plain letters and arrows must not steal
    /// typing in the position fields; chords with Ctrl or Alt, Escape and
    /// function keys are never typing.
    #[must_use]
    pub const fn always_applies(&self) -> bool {
        self.ctrl || self.alt || matches!(self.key, Key::Escape | Key::Function(_))
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        write!(f, "{}", self.key)
    }
}

impl FromStr for Chord {
    type Err = String;

    /// Parses `Ctrl+Alt+Enter`, `F5`, `PageDown`, `]` or `h`. Modifier and
    /// key names ignore case; a single character is the key it types.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let text = s.trim();
        let mut parts: Vec<&str> = text.split('+').collect();
        // `+` itself, and chords ending in it such as `Ctrl++`.
        if text.ends_with("++") || text == "+" {
            parts.truncate(parts.len().saturating_sub(2));
            parts.push("+");
        }
        let Some((key, modifiers)) = parts.split_last() else {
            return Err("empty key".to_owned());
        };
        let (mut ctrl, mut alt) = (false, false);
        for modifier in modifiers {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" => alt = true,
                "shift" => {
                    return Err(format!(
                        "`{text}`: Shift cannot be bound; it makes a move ten times larger"
                    ));
                }
                other => {
                    return Err(format!(
                        "`{text}`: unknown modifier `{other}`; use Ctrl or Alt"
                    ));
                }
            }
        }
        Ok(Self::new(ctrl, alt, parse_key(text, key)?))
    }
}

fn parse_key(chord: &str, key: &str) -> Result<Key, String> {
    if key.is_empty() {
        return Err(format!("`{chord}`: empty key"));
    }
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_whitespace() || c.is_control() {
            return Err(format!("`{chord}`: write the space bar as `Space`"));
        }
        return Ok(Key::character(c));
    }
    if key.eq_ignore_ascii_case("tab") {
        return Err(format!(
            "`{chord}`: Tab moves keyboard focus between controls and cannot be bound"
        ));
    }
    if let Some(n) = key
        .strip_prefix(['F', 'f'])
        .and_then(|n| n.parse::<u8>().ok())
        .filter(|n| (1..=24).contains(n))
    {
        return Ok(Key::Function(n));
    }
    let lower = key.to_ascii_lowercase();
    let alias = match lower.as_str() {
        "return" => "enter",
        "esc" => "escape",
        "pgup" => "pageup",
        "pgdn" | "pgdown" => "pagedown",
        "del" => "delete",
        "ins" => "insert",
        other => other,
    };
    NAMED
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(alias))
        .map(|(_, k)| k.clone())
        .ok_or_else(|| format!("`{chord}`: unknown key `{key}`"))
}

/// Which action each chord triggers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMap {
    bindings: BTreeMap<Action, Vec<Chord>>,
}

impl Default for KeyMap {
    fn default() -> Self {
        let bindings = Action::ALL
            .into_iter()
            .map(|action| {
                let chords = action
                    .defaults()
                    .iter()
                    .map(|c| c.parse().unwrap_or_else(|e| panic!("default binding: {e}")))
                    .collect();
                (action, chords)
            })
            .collect();
        Self { bindings }
    }
}

impl KeyMap {
    /// The defaults with some actions rebound. Each entry replaces that
    /// action's chords; an empty list unbinds it; actions not named keep
    /// their defaults.
    ///
    /// # Errors
    ///
    /// Returns one message per unknown action, unparsable chord, or chord
    /// bound to two actions.
    pub fn with_overrides<'a, I>(overrides: I) -> Result<Self, Vec<String>>
    where
        I: IntoIterator<Item = (&'a str, &'a [String])>,
    {
        let mut map = Self::default();
        let mut problems = Vec::new();
        for (name, chords) in overrides {
            let action = match name.parse::<Action>() {
                Ok(action) => action,
                Err(e) => {
                    problems.push(e);
                    continue;
                }
            };
            let mut parsed = Vec::with_capacity(chords.len());
            for chord in chords {
                match chord.parse::<Chord>() {
                    Ok(chord) if !parsed.contains(&chord) => parsed.push(chord),
                    Ok(_) => {}
                    Err(e) => problems.push(format!("keys.{name}: {e}")),
                }
            }
            map.bindings.insert(action, parsed);
        }
        let mut owner: BTreeMap<&Chord, Action> = BTreeMap::new();
        for (action, chords) in &map.bindings {
            for chord in chords {
                if let Some(other) = owner.insert(chord, *action) {
                    problems.push(format!(
                        "`{chord}` is bound to both `{other}` and `{action}`"
                    ));
                }
            }
        }
        if problems.is_empty() {
            Ok(map)
        } else {
            Err(problems)
        }
    }

    /// The action `chord` triggers, if any.
    #[must_use]
    pub fn action(&self, chord: &Chord) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(_, chords)| chords.contains(chord))
            .map(|(action, _)| *action)
    }

    /// The chords bound to `action`, in the order they were given.
    #[must_use]
    pub fn chords(&self, action: Action) -> &[Chord] {
        self.bindings.get(&action).map_or(&[], Vec::as_slice)
    }

    /// Every action with its chords, in [`Action::ALL`] order.
    pub fn iter(&self) -> impl Iterator<Item = (Action, &[Chord])> {
        Action::ALL
            .into_iter()
            .map(|action| (action, self.chords(action)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> Chord {
        text.parse().unwrap()
    }

    #[test]
    fn defaults_parse_and_do_not_collide() {
        let map = KeyMap::with_overrides([]).unwrap();
        assert_eq!(map, KeyMap::default());
        assert_eq!(map.action(&chord("Ctrl+Enter")), Some(Action::Apply));
        assert_eq!(map.action(&chord("ctrl+S")), Some(Action::Apply));
        assert_eq!(map.action(&chord("]")), Some(Action::Next));
        assert_eq!(map.action(&chord("H")), Some(Action::Left));
        assert_eq!(map.action(&chord("x")), None);
    }

    #[test]
    fn chords_round_trip_through_text() {
        for text in [
            "Ctrl+Enter",
            "Alt+F12",
            "PageDown",
            "]",
            "Ctrl+Alt+Delete",
            "Ctrl++",
        ] {
            assert_eq!(chord(text).to_string(), text);
        }
        assert_eq!(chord("esc"), chord("Escape"));
        assert_eq!(chord("control+return"), chord("Ctrl+Enter"));
    }

    #[test]
    fn unbindable_chords_are_explained() {
        for (text, needle) in [
            ("Shift+h", "Shift cannot be bound"),
            ("Tab", "Tab moves keyboard focus"),
            ("Super+h", "unknown modifier"),
            ("F25", "unknown key"),
            ("Hyper", "unknown key"),
            (" ", "empty key"),
            ("Ctrl+", "empty key"),
        ] {
            let error = text.parse::<Chord>().unwrap_err();
            assert!(error.contains(needle), "{text}: {error}");
        }
    }

    #[test]
    fn overrides_replace_unbind_and_keep_the_rest() {
        let left = vec!["a".to_owned(), "Left".to_owned()];
        let none: Vec<String> = Vec::new();
        let map =
            KeyMap::with_overrides([("left", left.as_slice()), ("keep", none.as_slice())]).unwrap();
        assert_eq!(map.action(&chord("a")), Some(Action::Left));
        assert_eq!(map.action(&chord("h")), None, "h is no longer bound");
        assert_eq!(map.chords(Action::Keep), &[] as &[Chord]);
        assert_eq!(map.action(&chord("F5")), Some(Action::Reload));
    }

    #[test]
    fn conflicts_and_unknown_names_are_all_reported() {
        let next = vec!["h".to_owned(), "Shift+x".to_owned()];
        let bogus = vec!["z".to_owned()];
        let problems =
            KeyMap::with_overrides([("next", next.as_slice()), ("jump", bogus.as_slice())])
                .unwrap_err();
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("unknown action `jump`")));
        assert!(problems.iter().any(|p| p.contains("keys.next: `Shift+x`")));
        assert!(
            problems
                .iter()
                .any(|p| p.contains("`h` is bound to both `next` and `left`"))
        );
    }

    #[test]
    fn only_modified_escape_and_function_chords_always_apply() {
        assert!(chord("Ctrl+s").always_applies());
        assert!(chord("Escape").always_applies());
        assert!(chord("F5").always_applies());
        assert!(!chord("h").always_applies());
        assert!(!chord("PageDown").always_applies());
        assert!(!chord("Left").always_applies());
    }
}
