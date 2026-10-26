//! Validated commands and checked OS input. Only `input_executor` emits events.

use crate::types::{MediaAction, MouseButtonAction, MouseButtonArg, ScrollDirection};
use std::fmt;

pub const MAX_TEXT_CHARS: usize = 1024;
pub const MAX_MOUSE_DELTA: i32 = 2048;

#[derive(Debug)]
pub enum InputError {
    Invalid(String),
    Busy,
    Cancelled,
    Denied,
    Unsupported(String),
    Execution(String),
    Unavailable,
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(s) | Self::Unsupported(s) | Self::Execution(s) => f.write_str(s),
            Self::Busy => f.write_str("remote input is busy; retry shortly"),
            Self::Cancelled => f.write_str("remote input was cancelled"),
            Self::Denied => f.write_str("this device is no longer authorized for remote input"),
            Self::Unavailable => f.write_str("the remote input worker is unavailable"),
        }
    }
}
impl std::error::Error for InputError {}

#[derive(Debug)]
pub enum InputCommand {
    MouseMove {
        delta_x: i32,
        delta_y: i32,
    },
    MouseClick {
        button: MouseButtonArg,
        double: bool,
    },
    MouseButton {
        button: MouseButtonArg,
        action: MouseButtonAction,
    },
    MouseScroll {
        direction: ScrollDirection,
        amount: i32,
    },
    KeyPress {
        key: String,
    },
    TextInput {
        text: String,
    },
    Media {
        action: MediaAction,
    },
}

pub(crate) enum ValidatedInput {
    Move(i32, i32),
    Click(Button, bool),
    Button(Button, bool),
    Scroll(i32),
    Shortcut(Shortcut),
    Text(String),
}

impl InputCommand {
    pub(crate) fn validate(self) -> Result<ValidatedInput, InputError> {
        match self {
            Self::MouseMove { delta_x, delta_y } => {
                if delta_x.unsigned_abs() > MAX_MOUSE_DELTA as u32
                    || delta_y.unsigned_abs() > MAX_MOUSE_DELTA as u32
                {
                    return Err(InputError::Invalid(format!(
                        "mouse movement must be between -{MAX_MOUSE_DELTA} and {MAX_MOUSE_DELTA}"
                    )));
                }
                Ok(ValidatedInput::Move(delta_x, delta_y))
            }
            Self::MouseClick { button, double } => Ok(ValidatedInput::Click(button.into(), double)),
            Self::MouseButton { button, action } => Ok(ValidatedInput::Button(
                button.into(),
                matches!(action, MouseButtonAction::Down),
            )),
            Self::MouseScroll { direction, amount } => {
                let magnitude = amount.unsigned_abs();
                if !(1..=100).contains(&magnitude) {
                    return Err(InputError::Invalid(
                        "scroll amount must have magnitude 1 to 100".into(),
                    ));
                }
                let delta = magnitude as i32;
                Ok(ValidatedInput::Scroll(match direction {
                    ScrollDirection::Up => delta,
                    ScrollDirection::Down => -delta,
                }))
            }
            Self::KeyPress { key } => Ok(ValidatedInput::Shortcut(parse_shortcut(&key)?)),
            Self::TextInput { text } => {
                if text.chars().count() > MAX_TEXT_CHARS || text.contains('\0') {
                    return Err(InputError::Invalid(format!(
                        "text must contain at most {MAX_TEXT_CHARS} characters and no NUL"
                    )));
                }
                Ok(ValidatedInput::Text(text))
            }
            Self::Media { action } => {
                let key = match action {
                    MediaAction::PlayPause => 0xB3,
                    MediaAction::Next => 0xB0,
                    MediaAction::Previous => 0xB1,
                    MediaAction::Mute => 0xAD,
                    MediaAction::VolumeUp => 0xAF,
                    MediaAction::VolumeDown => 0xAE,
                };
                Ok(ValidatedInput::Shortcut(Shortcut {
                    modifiers: Vec::new(),
                    key: ShortcutKey::Virtual(key),
                }))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Button {
    Left,
    Right,
    Middle,
}
impl From<MouseButtonArg> for Button {
    fn from(value: MouseButtonArg) -> Self {
        match value {
            MouseButtonArg::Left => Self::Left,
            MouseButtonArg::Right => Self::Right,
            MouseButtonArg::Middle => Self::Middle,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum InputKey {
    Virtual(u16),
    Unicode(u16),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputEvent {
    Move(i32, i32),
    Button(Button, bool),
    Scroll(i32),
    Key(InputKey, bool),
}

pub(crate) struct Shortcut {
    modifiers: Vec<u16>,
    key: ShortcutKey,
}
enum ShortcutKey {
    Virtual(u16),
    Character(char),
}

/// Preserve the accepted prefix on partial insertion for accurate cleanup.
pub(crate) struct BackendFailure {
    pub accepted: usize,
    pub detail: String,
}
pub(crate) trait InputBackend: Send {
    fn is_supported(&self) -> bool {
        true
    }
    /// Refuse a batch that would borrow/release a key or button already held
    /// locally. This is a best-effort check; the OS may receive local input
    /// immediately after the snapshot.
    fn local_input_busy(&mut self, _: &[InputEvent], _: &[Button], _: &[InputKey]) -> bool {
        false
    }
    fn send(&mut self, events: &[InputEvent]) -> Result<(), BackendFailure>;
    fn character_key(&mut self, character: char) -> Result<(u16, Vec<u16>), InputError>;
}

pub(crate) fn shortcut_events(
    shortcut: &Shortcut,
    backend: &mut dyn InputBackend,
) -> Result<Vec<InputEvent>, InputError> {
    let (key, implied) = match shortcut.key {
        ShortcutKey::Virtual(key) => (key, Vec::new()),
        ShortcutKey::Character(c) => backend.character_key(c)?,
    };
    let mut modifiers = shortcut.modifiers.clone();
    for modifier in implied {
        if !modifiers.contains(&modifier) {
            modifiers.push(modifier);
        }
    }
    let mut events = Vec::with_capacity(modifiers.len() * 2 + 2);
    for modifier in &modifiers {
        events.push(InputEvent::Key(InputKey::Virtual(*modifier), true));
    }
    events.push(InputEvent::Key(InputKey::Virtual(key), true));
    events.push(InputEvent::Key(InputKey::Virtual(key), false));
    for modifier in modifiers.iter().rev() {
        events.push(InputEvent::Key(InputKey::Virtual(*modifier), false));
    }
    Ok(events)
}

pub const SECURE_ATTENTION_REJECTION: &str =
    "Ctrl+Alt+Delete cannot be sent as keystrokes; use the security_screen command instead";

fn parse_shortcut(value: &str) -> Result<Shortcut, InputError> {
    if value.is_empty() || value.len() > 128 {
        return Err(InputError::Invalid(
            "key combination is empty or too long".into(),
        ));
    }
    let parts: Vec<_> = if value == "+" {
        vec![value]
    } else {
        value.split('+').map(str::trim).collect()
    };
    if parts.len() > 5 || parts.iter().any(|part| part.is_empty()) {
        return Err(InputError::Invalid("invalid key combination".into()));
    }
    let (last, prefixes) = parts
        .split_last()
        .ok_or_else(|| InputError::Invalid("key combination is empty".into()))?;
    let mut modifiers = Vec::new();
    for part in prefixes {
        let modifier = modifier_key(part).ok_or_else(|| {
            InputError::Invalid("only modifiers may precede the final shortcut key".into())
        })?;
        if modifiers.contains(&modifier) {
            return Err(InputError::Invalid("duplicate shortcut modifier".into()));
        }
        modifiers.push(modifier);
    }
    let key = named_key(last)
        .map(ShortcutKey::Virtual)
        .or_else(|| {
            let mut chars = last.chars();
            let c = chars.next()?;
            (chars.next().is_none() && !c.is_control()).then_some(ShortcutKey::Character(c))
        })
        .ok_or_else(|| InputError::Invalid("unsupported shortcut key".into()))?;
    if matches!(key, ShortcutKey::Virtual(0x2E))
        && modifiers.contains(&0x11)
        && modifiers.contains(&0x12)
    {
        return Err(InputError::Invalid(SECURE_ATTENTION_REJECTION.into()));
    }
    if !modifiers.is_empty() && modifier_key(last).is_some() {
        return Err(InputError::Invalid(
            "shortcut requires a final non-modifier key".into(),
        ));
    }
    Ok(Shortcut { modifiers, key })
}

fn modifier_key(value: &str) -> Option<u16> {
    match value.to_ascii_lowercase().as_str() {
        "shift" => Some(0x10),
        "ctrl" | "control" => Some(0x11),
        "alt" => Some(0x12),
        "meta" | "win" | "command" | "cmd" | "super" => Some(0x5B),
        _ => None,
    }
}
fn named_key(value: &str) -> Option<u16> {
    let value = value.to_ascii_lowercase();
    if let Some(key) = modifier_key(&value) {
        return Some(key);
    }
    match value.as_str() {
        "enter" | "return" => Some(0x0D),
        "tab" => Some(0x09),
        "space" => Some(0x20),
        "backspace" => Some(0x08),
        "delete" | "del" => Some(0x2E),
        "escape" | "esc" => Some(0x1B),
        "up" | "uparrow" => Some(0x26),
        "down" | "downarrow" => Some(0x28),
        "left" | "leftarrow" => Some(0x25),
        "right" | "rightarrow" => Some(0x27),
        "home" => Some(0x24),
        "end" => Some(0x23),
        "pageup" => Some(0x21),
        "pagedown" => Some(0x22),
        "volumeup" => Some(0xAF),
        "volumedown" => Some(0xAE),
        "mute" | "volumemute" => Some(0xAD),
        "playpause" | "mediaplaypause" => Some(0xB3),
        "nexttrack" | "medianexttrack" => Some(0xB0),
        "prevtrack" | "previoustrack" | "mediaprevtrack" => Some(0xB1),
        _ => value
            .strip_prefix('f')
            .and_then(|n| n.parse::<u16>().ok())
            .filter(|n| (1..=12).contains(n))
            .map(|n| 0x70 + n - 1),
    }
}

#[cfg(all(target_os = "windows", not(test)))]
#[path = "windows_input.rs"]
mod windows_backend;
#[cfg(all(target_os = "windows", not(test)))]
pub(crate) fn platform_backend() -> Box<dyn InputBackend> {
    Box::new(windows_backend::WindowsInput)
}
#[cfg(all(not(target_os = "windows"), not(test)))]
pub(crate) fn platform_backend() -> Box<dyn InputBackend> {
    Box::new(UnsupportedInput)
}
#[cfg(all(not(target_os = "windows"), not(test)))]
struct UnsupportedInput;
#[cfg(all(not(target_os = "windows"), not(test)))]
impl InputBackend for UnsupportedInput {
    fn is_supported(&self) -> bool {
        false
    }
    fn send(&mut self, _: &[InputEvent]) -> Result<(), BackendFailure> {
        Err(BackendFailure {
            accepted: 0,
            detail: "remote input is supported only by the Windows companion".into(),
        })
    }
    fn character_key(&mut self, _: char) -> Result<(u16, Vec<u16>), InputError> {
        Err(InputError::Unsupported(
            "remote input is supported only by the Windows companion".into(),
        ))
    }
}

// Router tests must never type, move the mouse, or change real input state.
#[cfg(test)]
pub(crate) fn platform_backend() -> Box<dyn InputBackend> {
    Box::new(TestInput)
}
#[cfg(test)]
struct TestInput;
#[cfg(test)]
impl InputBackend for TestInput {
    fn send(&mut self, _: &[InputEvent]) -> Result<(), BackendFailure> {
        Ok(())
    }
    fn character_key(&mut self, character: char) -> Result<(u16, Vec<u16>), InputError> {
        if character.is_ascii_alphabetic() {
            Ok((character.to_ascii_uppercase() as u16, Vec::new()))
        } else {
            Err(InputError::Unsupported(
                "test layout does not map this shortcut".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_commands_before_creating_os_events() {
        for key in [
            "",
            "ctrl+bad-key",
            "ctrl++c",
            "a+b",
            "ctrl+ctrl+c",
            "ctrl+alt+delete",
            "ctrl+alt+shift+delete",
            "ctrl+alt",
        ] {
            assert!(parse_shortcut(key).is_err(), "{key}");
        }
        for key in ["+", "ctrl+shift+c", "F12", "win+left", "é", "enter"] {
            assert!(parse_shortcut(key).is_ok(), "{key}");
        }
        assert!(InputCommand::MouseMove {
            delta_x: i32::MIN,
            delta_y: 0
        }
        .validate()
        .is_err());
        assert!(InputCommand::TextInput {
            text: "a".repeat(MAX_TEXT_CHARS + 1)
        }
        .validate()
        .is_err());
        assert!(InputCommand::TextInput {
            text: "a\0b".into()
        }
        .validate()
        .is_err());
        assert!(InputCommand::TextInput {
            text: "😀".repeat(MAX_TEXT_CHARS)
        }
        .validate()
        .is_ok());
    }
    #[test]
    fn scroll_bounds_reject_extremes_instead_of_overflowing() {
        for amount in [i32::MIN, i32::MAX, 0, 101, -101] {
            assert!(InputCommand::MouseScroll {
                direction: ScrollDirection::Up,
                amount
            }
            .validate()
            .is_err());
        }
        assert!(matches!(
            InputCommand::MouseScroll {
                direction: ScrollDirection::Up,
                amount: -3
            }
            .validate(),
            Ok(ValidatedInput::Scroll(3))
        ));
        assert!(matches!(
            InputCommand::MouseScroll {
                direction: ScrollDirection::Down,
                amount: 3
            }
            .validate(),
            Ok(ValidatedInput::Scroll(-3))
        ));
    }
}
