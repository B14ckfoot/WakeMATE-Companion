use enigo::{Enigo, Key, KeyboardControllable, MouseButton, MouseControllable};

use crate::types::{MediaAction, MouseButtonAction, MouseButtonArg, ScrollDirection};

#[derive(Default)]
pub struct InputController;

impl InputController {
    pub fn mouse_move_relative(&self, delta_x: i32, delta_y: i32) -> Result<(), String> {
        let mut enigo = Enigo::new();
        enigo.mouse_move_relative(delta_x, delta_y);
        Ok(())
    }

    pub fn mouse_click(&self, button: MouseButtonArg, double: bool) -> Result<(), String> {
        let mut enigo = Enigo::new();
        let button = map_mouse_button(button);

        enigo.mouse_click(button);
        if double {
            enigo.mouse_click(button);
        }

        Ok(())
    }

    pub fn mouse_button(
        &self,
        button: MouseButtonArg,
        action: MouseButtonAction,
    ) -> Result<(), String> {
        let mut enigo = Enigo::new();
        let button = map_mouse_button(button);

        match action {
            MouseButtonAction::Down => enigo.mouse_down(button),
            MouseButtonAction::Up => enigo.mouse_up(button),
        }

        Ok(())
    }

    pub fn mouse_scroll(&self, direction: ScrollDirection, amount: i32) -> Result<(), String> {
        let mut enigo = Enigo::new();
        let amount = amount.abs().max(1);
        let signed_amount = match direction {
            ScrollDirection::Up => amount,
            ScrollDirection::Down => -amount,
        };

        enigo.mouse_scroll_y(signed_amount);
        Ok(())
    }

    pub fn key_press(&self, key: &str) -> Result<(), String> {
        let mut enigo = Enigo::new();

        if key.contains('+') {
            let parts: Vec<&str> = key
                .split('+')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect();

            if parts.is_empty() {
                return Err("key combination is empty".to_string());
            }

            if is_secure_attention_combo(&parts) {
                return Err(SECURE_ATTENTION_REJECTION.to_string());
            }

            for modifier in parts.iter().take(parts.len().saturating_sub(1)) {
                let parsed = parse_key(modifier)
                    .ok_or_else(|| format!("unsupported modifier: {modifier}"))?;
                enigo.key_down(parsed);
            }

            let last = parts.last().copied().unwrap_or_default();
            let parsed = parse_key(last).ok_or_else(|| format!("unsupported key: {last}"))?;
            enigo.key_click(parsed);

            for modifier in parts.iter().take(parts.len().saturating_sub(1)).rev() {
                let parsed = parse_key(modifier)
                    .ok_or_else(|| format!("unsupported modifier: {modifier}"))?;
                enigo.key_up(parsed);
            }

            return Ok(());
        }

        let parsed = parse_key(key).ok_or_else(|| format!("unsupported key: {key}"))?;
        enigo.key_click(parsed);
        Ok(())
    }

    pub fn text_input(&self, text: &str) -> Result<(), String> {
        let mut enigo = Enigo::new();
        enigo.key_sequence(text);
        Ok(())
    }

    pub fn media_action(&self, action: MediaAction) -> Result<(), String> {
        let key_name = match action {
            MediaAction::PlayPause => "playpause",
            MediaAction::Next => "nexttrack",
            MediaAction::Previous => "prevtrack",
            MediaAction::Mute => "mute",
            MediaAction::VolumeUp => "volumeup",
            MediaAction::VolumeDown => "volumedown",
        };

        self.key_press(key_name)
    }
}

/// Sent back when a caller tries to type the Secure Attention Sequence.
/// Names the supported command so an older phone build gets a usable message
/// rather than a dead end.
pub const SECURE_ATTENTION_REJECTION: &str =
    "Ctrl+Alt+Delete cannot be sent as keystrokes; use the security_screen command instead";

/// True for Ctrl+Alt+Delete in any order or spelling.
///
/// Windows claims the real sequence in kernel mode and filters synthetic
/// input out of it, so pressing these three through `SendInput` never reaches
/// winlogon. What it *does* do is deliver a bare Delete to whatever window
/// has focus while Ctrl and Alt are held -- destroying a selection, a file, or
/// a row of a table with no way to tell the user it happened. Refusing is
/// both the honest answer and the safe one; [`crate::secure_attention`]
/// explains the sanctioned alternative.
fn is_secure_attention_combo(parts: &[&str]) -> bool {
    if parts.len() != 3 {
        return false;
    }

    let mut has_control = false;
    let mut has_alt = false;
    let mut has_delete = false;

    for part in parts {
        match part.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => has_control = true,
            "alt" => has_alt = true,
            "delete" | "del" => has_delete = true,
            _ => return false,
        }
    }

    has_control && has_alt && has_delete
}

fn map_mouse_button(button: MouseButtonArg) -> MouseButton {
    match button {
        MouseButtonArg::Left => MouseButton::Left,
        MouseButtonArg::Right => MouseButton::Right,
        MouseButtonArg::Middle => MouseButton::Middle,
    }
}

fn parse_key(raw: &str) -> Option<Key> {
    let key = raw.trim().to_ascii_lowercase();

    match key.as_str() {
        "enter" | "return" => Some(Key::Return),
        "tab" => Some(Key::Tab),
        "space" => Some(Key::Space),
        "backspace" => Some(Key::Backspace),
        "delete" | "del" => Some(Key::Delete),
        "escape" | "esc" => Some(Key::Escape),
        "up" | "uparrow" => Some(Key::UpArrow),
        "down" | "downarrow" => Some(Key::DownArrow),
        "left" | "leftarrow" => Some(Key::LeftArrow),
        "right" | "rightarrow" => Some(Key::RightArrow),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "pageup" => Some(Key::PageUp),
        "pagedown" => Some(Key::PageDown),
        "shift" => Some(Key::Shift),
        "ctrl" | "control" => Some(Key::Control),
        "alt" => Some(Key::Alt),
        "meta" | "win" | "command" | "cmd" | "super" => Some(Key::Meta),
        "f1" => Some(Key::F1),
        "f2" => Some(Key::F2),
        "f3" => Some(Key::F3),
        "f4" => Some(Key::F4),
        "f5" => Some(Key::F5),
        "f6" => Some(Key::F6),
        "f7" => Some(Key::F7),
        "f8" => Some(Key::F8),
        "f9" => Some(Key::F9),
        "f10" => Some(Key::F10),
        "f11" => Some(Key::F11),
        "f12" => Some(Key::F12),
        "volumeup" => Some(Key::VolumeUp),
        "volumedown" => Some(Key::VolumeDown),
        "mute" | "volumemute" => Some(Key::VolumeMute),
        // enigo only defines the transport keys on Windows and Linux, so
        // these arms have to carry the same gate or the crate stops compiling
        // on a macOS dev machine -- which is where the tests get run.
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        "playpause" | "mediaplaypause" => Some(Key::MediaPlayPause),
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        "nexttrack" | "medianexttrack" => Some(Key::MediaNextTrack),
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        "prevtrack" | "previoustrack" | "mediaprevtrack" => Some(Key::MediaPrevTrack),
        _ if key.chars().count() == 1 => Some(Key::Layout(key.chars().next()?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::is_secure_attention_combo;

    #[test]
    fn detects_the_secure_attention_sequence_in_any_spelling_or_order() {
        for combo in [
            vec!["ctrl", "alt", "delete"],
            vec!["control", "alt", "del"],
            vec!["CTRL", "Alt", "Delete"],
            vec!["delete", "ctrl", "alt"],
            vec!["alt", "delete", "control"],
        ] {
            assert!(
                is_secure_attention_combo(&combo),
                "{combo:?} is Ctrl+Alt+Delete and must be refused"
            );
        }
    }

    #[test]
    fn leaves_ordinary_shortcuts_alone() {
        for combo in [
            vec!["ctrl", "c"],
            vec!["ctrl", "alt", "t"],
            vec!["ctrl", "shift", "delete"],
            vec!["alt", "delete"],
            vec!["ctrl", "alt", "shift", "delete"],
        ] {
            assert!(
                !is_secure_attention_combo(&combo),
                "{combo:?} is a normal shortcut and must still be sent"
            );
        }
    }
}
