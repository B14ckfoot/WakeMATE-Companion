//! Shared WakeMATE design tokens.
//!
//! This is a complete palette extracted from the mobile app, not only the
//! subset the current tray popup happens to use -- future Companion UI
//! (macOS tray, a settings window, richer pairing states) should pull from
//! here rather than re-deriving colors, so a few tokens are unused today.
#![allow(dead_code)]
//!
//! These values are taken directly from the WakeMATE mobile app's actual
//! screens (`Wakemate-Mobile/src/components/*.tsx`, `app/**/*.tsx`) rather
//! than invented for the desktop app, so the Companion's tray UI reads as
//! part of the same product rather than a generic developer tool.
//!
//! Colors are `0x00RRGGBB` to match the `u32` framebuffer format the
//! hand-rolled pairing-popup renderer in `tray.rs` already uses. If the
//! rendering surface ever moves to a real UI toolkit, these are the same
//! tokens to feed it -- keep this module as the single source of truth
//! instead of scattering hex literals through the rendering code.

/// Deepest app background (mobile: screen backgrounds like `#05090c`).
pub const BG_BASE: u32 = 0x00050A0C;
/// Slightly raised background (mobile: `#0b1217`).
pub const BG_ELEVATED: u32 = 0x000B1217;
/// Card / panel surface (mobile: `#0f171c`).
pub const SURFACE: u32 = 0x000F171C;
/// Surface borders and dividers (mobile: `#16313a` / `#17323b`).
pub const BORDER: u32 = 0x00173239;

/// Brand primary -- WakeMATE cyan (mobile: `#0891b2`, used for primary
/// buttons and active/selected states throughout the app).
pub const PRIMARY: u32 = 0x000891B2;
/// Brighter accent cyan (mobile: `#22d3ee`, used for links and highlights).
pub const ACCENT: u32 = 0x0022D3EE;
/// Lightest accent tint (mobile: `#67e8f9`, used for glow/emphasis text).
pub const ACCENT_LIGHT: u32 = 0x0067E8F9;

/// Primary text on a dark surface (mobile: `#f8fbff`).
pub const TEXT_PRIMARY: u32 = 0x00F8FBFF;
/// Secondary / muted text on a dark surface (mobile: `#7f97a1`).
pub const TEXT_MUTED: u32 = 0x007F97A1;
/// Faint tertiary text, e.g. footnotes (mobile: `#6f8791`).
pub const TEXT_FAINT: u32 = 0x006F8791;

/// Success state (mobile: `#4ade80`).
pub const SUCCESS: u32 = 0x004ADE80;
/// Warning state (mobile: `#fbbf24`).
pub const WARNING: u32 = 0x00FBBF24;
/// Error / danger state (mobile: `#ef4444`).
pub const DANGER: u32 = 0x00EF4444;

/// Splits a packed `0x00RRGGBB` token into `(r, g, b)` bytes for the
/// pixel-drawing helpers in `tray.rs`.
pub const fn channels(color: u32) -> (u8, u8, u8) {
    (
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
    )
}
