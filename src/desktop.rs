//! Accessible native settings and least-privilege approval dialogs. The tray
//! only applies the selected action; these dialogs never execute OS commands.
use crate::{devices::Permissions, pairing::NotifierContext};
use std::{mem::size_of, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::{
        Controls::{
            TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOG_BUTTON, TDCBF_CANCEL_BUTTON,
            TDF_ALLOW_DIALOG_CANCELLATION, TDF_CALLBACK_TIMER, TDF_USE_COMMAND_LINKS,
            TDM_CLICK_BUTTON, TDN_TIMER,
        },
        WindowsAndMessaging::SendMessageW,
    },
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

unsafe extern "system" fn approval_timer(
    window: HWND,
    message: u32,
    _: WPARAM,
    _: LPARAM,
    context: isize,
) -> i32 {
    if message == TDN_TIMER as u32 && context != 0 {
        // TaskDialogIndirect is synchronous; the borrowed context remains
        // alive until it returns. No callback pointer escapes this dialog.
        let context = unsafe { &*(context as *const NotifierContext) };
        if !context.is_active() {
            unsafe {
                SendMessageW(window, TDM_CLICK_BUTTON as u32, 2, 0);
            }
        }
    }
    0
}

pub fn permissions(
    device: &str,
    current: Permissions,
    pending: Option<&NotifierContext>,
) -> Result<Option<Permissions>, String> {
    let title = wide("WakeMATE — Device permissions");
    let heading = wide(&format!("Allow access for {device}?"));
    let content = wide("Choose what this phone may do. Approve only a phone you recognize.\n\nDesktop control grants keyboard and mouse access to your signed-in account. It can perform actions beyond the power buttons. Windows security prompts remain protected.\n\nPairing requests expire after two minutes.");
    let labels = [
        wide("Status and wake only"),
        wide("Media remote — status, wake and media keys"),
        wide("Desktop control — status, wake, media, keyboard and mouse"),
        wide("Desktop and power — also sleep, lock, restart, shutdown and sign out"),
    ];
    let radios: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(index, text)| TASKDIALOG_BUTTON {
            nButtonID: index as i32 + 201,
            pszButtonText: text.as_ptr(),
        })
        .collect();
    let allow_text = wide("Allow selected access");
    let allow = TASKDIALOG_BUTTON {
        nButtonID: 100,
        pszButtonText: allow_text.as_ptr(),
    };
    let mut config: TASKDIALOGCONFIG = unsafe { std::mem::zeroed() };
    config.cbSize = size_of::<TASKDIALOGCONFIG>() as u32;
    config.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION | TDF_CALLBACK_TIMER;
    config.dwCommonButtons = TDCBF_CANCEL_BUTTON;
    config.pszWindowTitle = title.as_ptr();
    config.pszMainInstruction = heading.as_ptr();
    config.pszContent = content.as_ptr();
    config.cButtons = 1;
    config.pButtons = &allow;
    config.nDefaultButton = 2;
    config.cRadioButtons = radios.len() as u32;
    config.pRadioButtons = radios.as_ptr();
    config.nDefaultRadioButton = 200
        + if current.power {
            4
        } else if current.input {
            3
        } else if current.media {
            2
        } else {
            1
        };
    config.pfCallback = Some(approval_timer);
    config.lpCallbackData = pending.map_or(0, |p| p as *const NotifierContext as isize);
    let mut button = 0;
    let mut radio = 201;
    let result = unsafe { TaskDialogIndirect(&config, &mut button, &mut radio, ptr::null_mut()) };
    if result < 0 {
        return Err(format!(
            "Windows could not open the permission dialog (0x{:08x})",
            result as u32
        ));
    }
    if button == 100 && pending.is_none_or(|p| p.is_active()) {
        Ok(Some(Permissions::preset(radio - 200)))
    } else {
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    Network,
    Pause,
    Pair,
    Adapter,
    None,
}

pub fn settings(details: &str, network: bool, paused: bool) -> Result<SettingsAction, String> {
    let title = wide("WakeMATE Settings");
    let heading = wide("Your computer, your permissions");
    let content = wide(details);
    let labels = [wide(if network { "Disable LAN access\nKeep the companion local to this PC." } else {
        "Enable on my private network\nLet your phone discover and pair with this PC. No public Internet access is needed." }),
        wide(if paused { "Resume remote access\nRestore each phone's approved permissions." } else {
        "Pause remote access\nCancel input and deny requests from every paired phone." }),
        wide("Pair a phone\nShow a fresh one-use QR invitation."),
        wide("Choose network adapter\nSelect the private address used in the pairing QR and listener.")];
    let buttons: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(i, text)| TASKDIALOG_BUTTON {
            nButtonID: 100 + i as i32,
            pszButtonText: text.as_ptr(),
        })
        .collect();
    let mut config: TASKDIALOGCONFIG = unsafe { std::mem::zeroed() };
    config.cbSize = size_of::<TASKDIALOGCONFIG>() as u32;
    config.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION | TDF_USE_COMMAND_LINKS;
    config.dwCommonButtons = TDCBF_CANCEL_BUTTON;
    config.pszWindowTitle = title.as_ptr();
    config.pszMainInstruction = heading.as_ptr();
    config.pszContent = content.as_ptr();
    config.cButtons = buttons.len() as u32;
    config.pButtons = buttons.as_ptr();
    config.nDefaultButton = 2;
    let mut selected = 0;
    let result =
        unsafe { TaskDialogIndirect(&config, &mut selected, ptr::null_mut(), ptr::null_mut()) };
    if result < 0 {
        return Err(format!(
            "Windows could not open settings (0x{:08x})",
            result as u32
        ));
    }
    Ok(match selected {
        100 => SettingsAction::Network,
        101 => SettingsAction::Pause,
        102 => SettingsAction::Pair,
        103 => SettingsAction::Adapter,
        _ => SettingsAction::None,
    })
}

/// Outer None means cancelled; inner None selects automatic private adapters.
pub fn network_address(current: Option<&str>) -> Result<Option<Option<String>>, String> {
    let candidates = crate::network::candidates();
    if candidates.is_empty() {
        return Err(
            "No private IPv4 adapter is available. Connect this PC to your private LAN and retry."
                .into(),
        );
    }
    let title = wide("WakeMATE - Private network adapter");
    let heading = wide("Choose the address your phone can reach");
    let content = wide("Use the physical Ethernet or Wi-Fi adapter on your phone's network. A selected address remains fixed until you change it; DHCP changes will show a connection error. Automatic selection uses private adapters and discovery answers from the phone's subnet.");
    let mut labels = vec![wide("Automatic private adapters")];
    labels.extend(
        candidates
            .iter()
            .take(16)
            .map(|n| wide(&format!("{} - {}", n.interface_name, n.local_ip))),
    );
    let radios: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(i, text)| TASKDIALOG_BUTTON {
            nButtonID: i as i32 + 10,
            pszButtonText: text.as_ptr(),
        })
        .collect();
    let select_text = wide("Use this adapter");
    let select = TASKDIALOG_BUTTON {
        nButtonID: 100,
        pszButtonText: select_text.as_ptr(),
    };
    let mut config: TASKDIALOGCONFIG = unsafe { std::mem::zeroed() };
    config.cbSize = size_of::<TASKDIALOGCONFIG>() as u32;
    config.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION;
    config.dwCommonButtons = TDCBF_CANCEL_BUTTON;
    config.pszWindowTitle = title.as_ptr();
    config.pszMainInstruction = heading.as_ptr();
    config.pszContent = content.as_ptr();
    config.cButtons = 1;
    config.pButtons = &select;
    config.nDefaultButton = 2;
    config.cRadioButtons = radios.len() as u32;
    config.pRadioButtons = radios.as_ptr();
    config.nDefaultRadioButton = candidates
        .iter()
        .take(16)
        .position(|n| Some(n.local_ip.as_str()) == current)
        .map_or(10, |i| i as i32 + 11);
    let mut selected = 0;
    let mut radio = 10;
    let result = unsafe { TaskDialogIndirect(&config, &mut selected, &mut radio, ptr::null_mut()) };
    if result < 0 {
        return Err(format!(
            "Windows could not open adapter selection (0x{:08x})",
            result as u32
        ));
    }
    if selected != 100 {
        return Ok(None);
    }
    Ok(Some(if radio == 10 {
        None
    } else {
        Some(
            candidates
                .get((radio - 11) as usize)
                .ok_or("Invalid adapter selection")?
                .local_ip
                .clone(),
        )
    }))
}
