use enigo::{Enigo, Key, KeyboardControllable, MouseButton, MouseControllable};

use crate::types::{MediaAction, MouseButtonArg, ScrollDirection};

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
        "playpause" | "mediaplaypause" => Some(Key::MediaPlayPause),
        "nexttrack" | "medianexttrack" => Some(Key::MediaNextTrack),
        "prevtrack" | "previoustrack" | "mediaprevtrack" => Some(Key::MediaPrevTrack),
        _ if key.chars().count() == 1 => Some(Key::Layout(key.chars().next()?)),
        _ => None,
    }
}
