//! Every SendInput insertion is checked. This module never elevates or bypasses UIPI.
use super::{BackendFailure, Button, InputBackend, InputError, InputEvent, InputKey};
use std::mem::size_of;
use windows_sys::Win32::UI::{
    Input::KeyboardAndMouse::{
        GetAsyncKeyState, GetKeyboardLayout, SendInput, VkKeyScanExW, INPUT, INPUT_0,
        INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
        KEYEVENTF_UNICODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
        MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
        MOUSEEVENTF_WHEEL, MOUSEINPUT,
    },
    WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
};

pub(super) struct WindowsInput;
impl InputBackend for WindowsInput {
    fn local_input_busy(
        &mut self,
        events: &[InputEvent],
        buttons: &[Button],
        keys: &[InputKey],
    ) -> bool {
        let is_down = |key: u16| unsafe { GetAsyncKeyState(i32::from(key)) < 0 };
        if [0x10, 0x11, 0x12, 0x5B, 0x5C]
            .into_iter()
            .any(|key| is_down(key) && !keys.contains(&InputKey::Virtual(key)))
        {
            return true;
        }
        if [
            (Button::Left, 0x01),
            (Button::Right, 0x02),
            (Button::Middle, 0x04),
        ]
        .into_iter()
        .any(|(button, key)| is_down(key) && !buttons.contains(&button))
        {
            return true;
        }
        events.iter().any(|event| match *event {
            InputEvent::Key(InputKey::Virtual(key), true) => {
                is_down(key) && !keys.contains(&InputKey::Virtual(key))
            }
            _ => false,
        })
    }
    fn send(&mut self, events: &[InputEvent]) -> Result<(), BackendFailure> {
        if events.is_empty() {
            return Ok(());
        }
        let inputs: Vec<INPUT> = events.iter().map(to_windows_input).collect();
        // Safety: valid INPUT array, matching discriminators and exact cbSize.
        let accepted = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                size_of::<INPUT>() as i32,
            )
        } as usize;
        if accepted != inputs.len() {
            return Err(BackendFailure { accepted, detail: "Windows rejected remote input; the target may use a protected desktop or higher privileges".into() });
        }
        Ok(())
    }
    fn character_key(&mut self, character: char) -> Result<(u16, Vec<u16>), InputError> {
        if character.len_utf16() != 1 {
            return Err(InputError::Invalid(
                "use text input for this Unicode character".into(),
            ));
        }
        // Use the foreground application's keyboard layout, not this worker's.
        let mapping = unsafe {
            let window = GetForegroundWindow();
            let thread = GetWindowThreadProcessId(window, std::ptr::null_mut());
            VkKeyScanExW(character as u16, GetKeyboardLayout(thread))
        };
        if mapping == -1 {
            return Err(InputError::Invalid(
                "this shortcut key is unavailable in the current keyboard layout; use text input"
                    .into(),
            ));
        }
        let modifiers = (mapping as u16) >> 8;
        if modifiers & !7 != 0 {
            return Err(InputError::Invalid(
                "unsupported keyboard-layout modifiers".into(),
            ));
        }
        let mut keys = Vec::new();
        for (flag, key) in [(1, 0x10), (2, 0x11), (4, 0x12)] {
            if modifiers & flag != 0 {
                keys.push(key);
            }
        }
        Ok((mapping as u16 & 0xff, keys))
    }
}

fn to_windows_input(event: &InputEvent) -> INPUT {
    match *event {
        InputEvent::Key(key, down) => {
            let (virtual_key, scan, flags) = match key {
                InputKey::Virtual(key) => (
                    key,
                    0,
                    if matches!(key, 0x21..=0x28 | 0x2D | 0x2E | 0x5B | 0x5C) {
                        KEYEVENTF_EXTENDEDKEY
                    } else {
                        0
                    },
                ),
                InputKey::Unicode(unit) => (0, unit, KEYEVENTF_UNICODE),
            };
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: virtual_key,
                        wScan: scan,
                        dwFlags: flags | if down { 0 } else { KEYEVENTF_KEYUP },
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            }
        }
        _ => {
            let (dx, dy, data, flags) = match *event {
                InputEvent::Move(x, y) => (x, y, 0, MOUSEEVENTF_MOVE),
                InputEvent::Scroll(amount) => (0, 0, (amount * 120) as u32, MOUSEEVENTF_WHEEL),
                InputEvent::Button(button, down) => (
                    0,
                    0,
                    0,
                    match (button, down) {
                        (Button::Left, true) => MOUSEEVENTF_LEFTDOWN,
                        (Button::Left, false) => MOUSEEVENTF_LEFTUP,
                        (Button::Right, true) => MOUSEEVENTF_RIGHTDOWN,
                        (Button::Right, false) => MOUSEEVENTF_RIGHTUP,
                        (Button::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
                        (Button::Middle, false) => MOUSEEVENTF_MIDDLEUP,
                    },
                ),
                InputEvent::Key(..) => unreachable!("keyboard events handled above"),
            };
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx,
                        dy,
                        mouseData: data,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            }
        }
    }
}
