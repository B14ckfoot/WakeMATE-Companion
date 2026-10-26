use std::{
    fs, io,
    num::NonZeroU32,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime},
};

use font8x8::{UnicodeFonts, BASIC_FONTS};
use image::ImageReader;
use qrcodegen::{QrCode, QrCodeEcc};
use softbuffer::{Context as SoftbufferContext, Surface as SoftbufferSurface};
use tracing::{error, info, warn};
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu},
    Icon, Rect as TrayRect, TrayIcon, TrayIconBuilder,
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key, KeyCode, NamedKey, PhysicalKey},
    monitor::MonitorHandle,
    platform::windows::{CornerPreference, WindowAttributesExtWindows},
    window::{Window, WindowId, WindowLevel},
};

use crate::{
    config::{AppConfig, SharedConfig},
    desktop::{self, SettingsAction},
    devices::{DeviceRegistry, PairedDevice, Permissions, SharedDeviceRegistry},
    pairing::{NotifierContext, PairingCoordinator},
    pairing_qr,
    server_thread::ServerThread,
    system, theme,
    tls::TlsIdentity,
};

use std::sync::Mutex as StdMutex;

const POPUP_LOGICAL_WIDTH: f64 = 280.0;
const POPUP_LOGICAL_HEIGHT: f64 = 336.0;
const POPUP_SCREEN_MARGIN: i32 = 8;
const POPUP_OFFSET: i32 = 12;
const CONFIG_POLL_INTERVAL: Duration = Duration::from_secs(1);

type PopupContext = SoftbufferContext<Rc<Window>>;
type PopupSurface = SoftbufferSurface<Rc<Window>, Rc<Window>>;

pub fn run(config: SharedConfig) -> Result<(), Box<dyn std::error::Error>> {
    let assets_dir = AppConfig::asset_dir()?;
    std::fs::create_dir_all(&assets_dir)?;

    let registry: SharedDeviceRegistry = Arc::new(StdMutex::new(DeviceRegistry::load_or_default()));
    let mut app = TrayApp::new(config, registry, assets_dir)?;
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    MenuEvent::set_event_handler(Some(forward_menu_events(event_loop.create_proxy())));
    event_loop.run_app(&mut app)?;
    MenuEvent::set_event_handler::<fn(MenuEvent)>(None);

    app.finish()
}

fn forward_menu_events(
    proxy: EventLoopProxy<UserEvent>,
) -> impl Fn(MenuEvent) + Send + Sync + 'static {
    move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }
}

struct TrayApp {
    config: SharedConfig,
    registry: SharedDeviceRegistry,
    assets_dir: PathBuf,
    server: ServerThread,
    tray_icon: Option<TrayIcon>,
    pairing_popup: Option<PairingPopup>,
    pairing_popup_requested: bool,
    tray_menu: Option<TrayMenu>,
    /// Menu id -> paired device id, rebuilt with the submenu.
    device_menu_ids: Vec<(MenuId, String)>,
    permission_menu_ids: Vec<(MenuId, String)>,
    exit_signal: system::ExitSignal,
    /// Snapshot backing change detection for the submenu rebuild; `None`
    /// until the submenu has been populated once.
    last_device_list: Option<Vec<PairedDevice>>,
    startup_error: Option<String>,
    next_config_poll: Instant,
    last_config_modified: Option<SystemTime>,
}

impl TrayApp {
    fn new(
        config: SharedConfig,
        registry: SharedDeviceRegistry,
        assets_dir: PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let server =
            ServerThread::spawn(config.clone(), registry.clone(), windows_pairing_notifier)?;

        Ok(Self {
            config: config.clone(),
            registry,
            assets_dir,
            server,
            tray_icon: None,
            pairing_popup: None,
            pairing_popup_requested: false,
            tray_menu: None,
            device_menu_ids: Vec::new(),
            permission_menu_ids: Vec::new(),
            exit_signal: system::ExitSignal::create().map_err(io::Error::other)?,
            last_device_list: None,
            startup_error: None,
            next_config_poll: Instant::now() + CONFIG_POLL_INTERVAL,
            last_config_modified: config_modified_time()?,
        })
    }

    /// Builds the tray menu.
    ///
    /// The top level is deliberately limited to the handful of things someone
    /// opens the tray to actually do: see whether WakeMATE is working, see
    /// which phones are connected, show the pairing code, and get out of
    /// trouble (restart / quit). Everything that is configuration rather than
    /// an action -- startup behavior, credential rotation, the data folder,
    /// and the destructive reset -- lives under "Settings" so the first level
    /// stays readable. Nothing was removed; every previous item is still
    /// reachable in at most one extra click.
    fn initialize_tray(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = config_snapshot(&self.config)?;
        let menu = Menu::new();
        let status = MenuItem::new(self.status_label(), false, None);
        let paired_devices = Submenu::new("Paired Devices", true);
        let show_pairing_qr = MenuItem::new("View Pairing QR Code", true, None);
        let restart_connection = MenuItem::new("Restart Connection", true, None);
        let quit = MenuItem::new("Quit WakeMATE", true, None);

        // Settings submenu: configuration and recovery, not everyday actions.
        let settings = Submenu::new("Settings", true);
        let launch_on_startup = CheckMenuItem::new(
            "Launch on Windows Startup",
            true,
            snapshot.launch_on_startup,
            None,
        );
        let rotate_pairing_token = MenuItem::new("Revoke All Phones...", true, None);
        let settings_window = MenuItem::new("Open Settings...", true, None);
        let pause_access = CheckMenuItem::new(
            "Pause Remote Access",
            true,
            self.registry
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .paused(),
            None,
        );
        let network_access = CheckMenuItem::new(
            "Enable Private LAN Access",
            true,
            snapshot.allow_remote_connections && snapshot.onboarding_completed,
            None,
        );
        let open_data_folder = MenuItem::new("Open Data Folder", true, None);
        let reset_companion = MenuItem::new("Reset Companion...", true, None);
        let settings_separator = PredefinedMenuItem::separator();

        settings.append(&settings_window)?;
        settings.append(&network_access)?;
        settings.append(&launch_on_startup)?;
        settings.append(&rotate_pairing_token)?;
        settings.append(&settings_separator)?;
        settings.append(&open_data_folder)?;
        settings.append(&reset_companion)?;

        let separator_top = PredefinedMenuItem::separator();
        let separator_bottom = PredefinedMenuItem::separator();

        let tray_menu = TrayMenu {
            status: status.clone(),
            launch_on_startup: launch_on_startup.clone(),
            paired_devices: paired_devices.clone(),
            pause_access: pause_access.clone(),
            network_access: network_access.clone(),
            ids: MenuIds {
                show_pairing_qr: show_pairing_qr.id().clone(),
                restart_connection: restart_connection.id().clone(),
                rotate_pairing_token: rotate_pairing_token.id().clone(),
                launch_on_startup: launch_on_startup.id().clone(),
                open_data_folder: open_data_folder.id().clone(),
                reset_companion: reset_companion.id().clone(),
                quit: quit.id().clone(),
                settings_window: settings_window.id().clone(),
                pause_access: pause_access.id().clone(),
                network_access: network_access.id().clone(),
            },
        };

        menu.append(&status)?;
        menu.append(&separator_top)?;
        menu.append(&pause_access)?;
        menu.append(&paired_devices)?;
        menu.append(&show_pairing_qr)?;
        menu.append(&settings)?;
        menu.append(&restart_connection)?;
        menu.append(&separator_bottom)?;
        menu.append(&quit)?;

        let tooltip = format!("WakeMATE Companion ({})", snapshot.device_name);

        let tray_icon = TrayIconBuilder::new()
            .with_tooltip(tooltip)
            .with_menu(Box::new(menu))
            .with_icon(load_tray_icon(&self.assets_dir)?)
            .build()?;

        self.tray_menu = Some(tray_menu);
        self.tray_icon = Some(tray_icon);
        self.refresh_tray_state()?;
        self.refresh_paired_devices_menu()?;
        info!(assets = %self.assets_dir.display(), "WakeMATE tray icon ready");
        Ok(())
    }

    /// Rebuilds the "Paired Devices" submenu when the registry changed
    /// (a new approval from the notifier thread, or a revocation here).
    fn refresh_paired_devices_menu(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let devices = self
            .registry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .list();

        if self.last_device_list.as_ref() == Some(&devices) {
            return Ok(());
        }

        let Some(tray_menu) = &self.tray_menu else {
            return Ok(());
        };

        while tray_menu.paired_devices.remove_at(0).is_some() {}
        self.device_menu_ids.clear();
        self.permission_menu_ids.clear();

        if devices.is_empty() {
            let placeholder = MenuItem::new("No phones paired yet", false, None);
            tray_menu.paired_devices.append(&placeholder)?;
        } else {
            for device in &devices {
                let entry = Submenu::new(popup_device_label(&device.name, 32), true);
                let permission = MenuItem::new(
                    format!("Permissions: {}...", device.permissions.label()),
                    true,
                    None,
                );
                let revoke = MenuItem::new("Revoke this phone...", true, None);
                entry.append(&permission)?;
                entry.append(&revoke)?;
                tray_menu.paired_devices.append(&entry)?;
                self.permission_menu_ids
                    .push((permission.id().clone(), device.id.clone()));
                self.device_menu_ids
                    .push((revoke.id().clone(), device.id.clone()));
            }
        }

        self.last_device_list = Some(devices);
        Ok(())
    }

    fn refresh_tray_state(&self) -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = config_snapshot(&self.config)?;

        if let Some(tray_menu) = &self.tray_menu {
            tray_menu.status.set_text(self.status_label());
            tray_menu.pause_access.set_checked(
                self.registry
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .paused(),
            );
            tray_menu
                .network_access
                .set_checked(snapshot.allow_remote_connections && snapshot.onboarding_completed);
            tray_menu
                .launch_on_startup
                .set_checked(snapshot.launch_on_startup);
        }

        if let Some(tray_icon) = &self.tray_icon {
            tray_icon.set_tooltip(Some(format!(
                "WakeMATE Companion\n{}\n{}",
                self.status_label(),
                self.address_label(&snapshot)
            )))?;
        }

        Ok(())
    }

    /// A short, human-readable pairing/connectivity state for the tray menu
    /// and tooltip -- one of a small set of well-defined states rather than
    /// raw config booleans, so the person looking at the tray icon always
    /// knows roughly what WakeMATE is doing right now.
    fn status_label(&self) -> String {
        let config = match config_snapshot(&self.config) {
            Ok(config) => config,
            Err(_) => return "Status unavailable (see logs)".to_string(),
        };

        if let Some(error) = &self.startup_error {
            return format!("{} — Error: {error}", config.device_name);
        }

        let status = self.server.status.lock().unwrap_or_else(|p| p.into_inner());
        let registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
        let state = if registry.storage_error().is_some() {
            "Storage error - remote access paused"
        } else if !self.server.active {
            "Server not running"
        } else if !status.https_ready {
            "Starting HTTPS..."
        } else if registry.paused() {
            "Remote access paused"
        } else if status.error.is_some() {
            "HTTPS ready - discovery unavailable"
        } else if !config.allow_remote_connections || !config.onboarding_completed {
            "Local only - open Settings to connect"
        } else if registry.list().is_empty() {
            "Ready to pair"
        } else {
            "Ready - device permissions active"
        };

        format!("{} — {state}", config.device_name)
    }

    fn address_label(&self, config: &AppConfig) -> String {
        let port = config.tls_port;
        if config.allow_remote_connections {
            match config
                .preferred_address
                .clone()
                .or_else(|| crate::network::primary().map(|n| n.local_ip))
            {
                Some(ip) => format!("https://{ip}:{port}"),
                None => format!("port {port}"),
            }
        } else {
            format!("https://localhost:{port}")
        }
    }

    fn handle_menu_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: MenuEvent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(menu) = &self.tray_menu {
            if event.id == menu.ids.settings_window {
                return self.open_settings(event_loop);
            }
            if event.id == menu.ids.pause_access {
                return self.toggle_pause();
            }
            if event.id == menu.ids.network_access {
                return self.toggle_network();
            }
        }
        if let Some(id) = self
            .permission_menu_ids
            .iter()
            .find(|(menu, _)| *menu == event.id)
            .map(|(_, id)| id.clone())
        {
            return self.edit_permissions(&id);
        }
        let Some((
            show_pairing_qr,
            restart_connection,
            rotate_pairing_token,
            launch_on_startup,
            open_data_folder,
            reset_companion,
            quit,
        )) = self.tray_menu.as_ref().map(|tray_menu| {
            (
                tray_menu.ids.show_pairing_qr.clone(),
                tray_menu.ids.restart_connection.clone(),
                tray_menu.ids.rotate_pairing_token.clone(),
                tray_menu.ids.launch_on_startup.clone(),
                tray_menu.ids.open_data_folder.clone(),
                tray_menu.ids.reset_companion.clone(),
                tray_menu.ids.quit.clone(),
            )
        })
        else {
            return Ok(());
        };

        if event.id == show_pairing_qr {
            self.toggle_pairing_qr_popup(event_loop)?;
        } else if event.id == restart_connection {
            self.restart_connection()?;
        } else if event.id == rotate_pairing_token {
            self.rotate_pairing_token()?;
        } else if event.id == launch_on_startup {
            self.toggle_launch_on_startup()?;
        } else if event.id == open_data_folder {
            system::open_path(&AppConfig::data_dir()?)?;
        } else if event.id == reset_companion {
            self.reset_companion()?;
            self.open_settings(event_loop)?;
        } else if event.id == quit {
            info!("WakeMATE quit requested from tray");
            event_loop.exit();
        } else if let Some(device_id) = self
            .device_menu_ids
            .iter()
            .find(|(menu_id, _)| *menu_id == event.id)
            .map(|(_, device_id)| device_id.clone())
        {
            self.revoke_paired_device(&device_id)?;
        }

        Ok(())
    }

    fn revoke_paired_device(&mut self, device_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let device_name = self
            .registry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .list()
            .into_iter()
            .find(|device| device.id == device_id)
            .map(|device| device.name);

        let Some(device_name) = device_name else {
            self.refresh_paired_devices_menu()?;
            return Ok(());
        };

        let confirmed = system::confirm_dialog(
            "Revoke this phone?",
            &format!(
                "\"{device_name}\" will immediately lose remote access to this computer and \
                 will need to scan the pairing QR code again to reconnect.\n\nRevoke it?",
            ),
        );

        if !confirmed {
            return Ok(());
        }

        self.cancel_pending_pairing();
        self.server.input.cancel_device(device_id);
        let removed = self
            .registry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .revoke(device_id)?;

        if let Some(removed) = removed {
            info!(device = %removed.name, "WakeMATE paired phone revoked from the tray");
        }

        self.refresh_paired_devices_menu()?;
        Ok(())
    }

    fn reset_companion(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let confirmed = system::confirm_dialog(
            "Reset WakeMATE Companion?",
            "This clears the pairing token and all local WakeMATE settings on this computer, \
             including which capabilities were approved for paired phones. Every phone will \
             need to re-scan a fresh pairing code afterwards.\n\n\
             This does not affect your WakeMATE mobile account or any cloud data.\n\n\
             Reset now?",
        );

        if !confirmed {
            return Ok(());
        }

        self.invalidate_pairing();
        self.server.input.cancel_all();
        self.registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .set_paused(true)?;
        self.registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .revoke_all()?;
        {
            let mut config = self
                .config
                .lock()
                .map_err(|_| io::Error::other("Configuration unavailable"))?;
            config.reset_to_defaults();
            config.save()?;
        }
        self.registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .reset()?;

        if let Err(error) =
            system::sync_launch_on_startup(config_snapshot(&self.config)?.launch_on_startup)
        {
            warn!(%error, "failed to sync the Windows startup preference after reset");
        }

        self.refresh_tray_state()?;
        self.refresh_paired_devices_menu()?;
        self.server
            .restart(self.config.clone(), self.registry.clone())?;
        info!("WakeMATE Companion was reset to defaults from the tray");
        Ok(())
    }

    fn toggle_launch_on_startup(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let current = config_snapshot(&self.config)?.launch_on_startup;
        let next = !current;

        if let Err(error) = system::sync_launch_on_startup(next) {
            if let Some(tray_menu) = &self.tray_menu {
                tray_menu.launch_on_startup.set_checked(current);
            }
            return Err(io::Error::other(error).into());
        }

        {
            let mut config = self
                .config
                .lock()
                .map_err(|_| io::Error::other("failed to access config"))?;
            config.launch_on_startup = next;
            config.save()?;
        }

        self.refresh_tray_state()?;
        info!(enabled = next, "WakeMATE launch on startup updated");
        Ok(())
    }

    fn toggle_pairing_qr_popup(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.pairing_popup.is_some() {
            self.pairing_popup = None;
            return Ok(());
        }

        let config = config_snapshot(&self.config)?;
        if self
            .registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .paused()
        {
            return Err(io::Error::other(
                "Remote access is paused. Resume it in WakeMATE settings before pairing.",
            )
            .into());
        }
        if !config.allow_remote_connections || !config.onboarding_completed {
            self.toggle_network()?;
            if !config_snapshot(&self.config)?.allow_remote_connections {
                return Ok(());
            }
        }
        if !self
            .server
            .status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .https_ready
        {
            self.pairing_popup_requested = true;
            return Ok(());
        }
        self.pairing_popup_requested = false;
        let tray_rect = self.tray_icon.as_ref().and_then(|icon| icon.rect());
        let owner_window = self
            .tray_icon
            .as_ref()
            .map(|icon| icon.window_handle() as isize)
            .unwrap_or_default();

        self.pairing_popup = Some(PairingPopup::new(
            event_loop,
            &self.config,
            self.server.pairing.as_ref(),
            tray_rect,
            owner_window,
        )?);

        Ok(())
    }

    /// Stops and restarts the local server listeners.
    ///
    /// This is the "turn it off and on again" recovery path for the common
    /// case where the listeners survive but are no longer usable -- the PC
    /// changed networks, resumed from sleep on a different adapter, or the
    /// bound address went away underneath the running server. It deliberately
    /// touches nothing else: pairings, tokens, and the TLS identity all
    /// persist, so no phone has to be paired again afterwards.
    ///
    /// This is also the recovery path for a server that already died on its
    /// own (a failed bind, a panic). `refresh_server_health` latches that into
    /// `startup_error`, which otherwise persists for the rest of the session
    /// and is re-raised on quit, so a successful restart clears it -- a stale
    /// error would keep reporting a failure that no longer exists.
    fn restart_connection(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Close the QR popup first: its payload names ports the restarted
        // server may not come back on, and its pairing-session token belongs
        // to the coordinator that is about to be replaced.
        self.pairing_popup = None;

        info!("WakeMATE restarting the local server from the tray");
        // `ServerThread::restart` stops a live server first and respawns
        // either way, so this recovers a crashed one as well.
        self.server
            .restart(self.config.clone(), self.registry.clone())?;
        self.startup_error = None;
        self.refresh_tray_state()?;
        info!("WakeMATE server restarted");
        Ok(())
    }

    fn cancel_pending_pairing(&mut self) {
        self.pairing_popup_requested = false;
        if let Some(pairing) = &self.server.pairing {
            pairing.cancel_pending();
        }
        self.pairing_popup = None;
    }

    fn invalidate_pairing(&mut self) {
        self.pairing_popup_requested = false;
        if let Some(pairing) = &self.server.pairing {
            pairing.invalidate();
        }
        self.pairing_popup = None;
    }

    fn rotate_pairing_token(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if !system::confirm_dialog(
            "Revoke all phones?",
            "Every phone will lose access immediately and need to scan a fresh QR code. Continue?",
        ) {
            return Ok(());
        }
        self.invalidate_pairing();
        self.server.input.cancel_all();
        {
            let mut registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
            let was_paused = registry.paused();
            registry.set_paused(true)?;
            registry.revoke_all()?;
            if !was_paused {
                registry.set_paused(false)?;
            }
        }
        self.refresh_paired_devices_menu()?;
        self.server
            .restart(self.config.clone(), self.registry.clone())?;
        self.refresh_tray_state()?;
        Ok(())
    }

    fn toggle_pause(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.server.input.cancel_all();
        let result = {
            let mut registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
            let next = !registry.paused();
            registry.set_paused(next)
        };
        // Never hold registry while taking the coordinator lifecycle gate.
        self.cancel_pending_pairing();
        result?;
        self.refresh_tray_state()
    }

    fn toggle_network(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let current = config_snapshot(&self.config)?;
        let enable = !current.allow_remote_connections || !current.onboarding_completed;
        if enable && !system::confirm_dialog("Enable WakeMATE on your private LAN?", "Your phone can discover this computer's name and request pairing on the same private network. Remote commands still require your desktop approval and selected device permissions. Enable LAN access?") {
            self.refresh_tray_state()?;
            return Ok(());
        }
        let preferred = if enable {
            let Some(selected) = desktop::network_address(current.preferred_address.as_deref())?
            else {
                self.refresh_tray_state()?;
                return Ok(());
            };
            selected
        } else {
            current.preferred_address.clone()
        };
        self.server.input.cancel_all();
        {
            let mut config = self
                .config
                .lock()
                .map_err(|_| io::Error::other("Configuration unavailable"))?;
            let mut proposed = config.clone();
            proposed.onboarding_completed = true;
            proposed.allow_remote_connections = enable;
            proposed.allow_discovery = enable;
            proposed.preferred_address = preferred;
            proposed.save()?;
            *config = proposed;
        }
        self.restart_connection()
    }

    fn open_settings(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let config = config_snapshot(&self.config)?;
        let (paused, error, count) = {
            let registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
            (
                registry.paused(),
                registry.storage_error().map(str::to_owned),
                registry.list().len(),
            )
        };
        let status = self
            .server
            .status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let details = format!("{}\n{}\n\nPaired phones: {}\nHTTPS: {}\nDiscovery: {}\nNetwork adapter: {}\n\n{}\n\nChange each phone's permissions under Paired Devices in the tray. Offline Wake-on-LAN works directly from the phone; online control starts after Windows sign-in.",
            config.device_name, self.status_label(), count, if status.https_ready { "Listening" } else { "Starting / unavailable" },
            if status.discovery_ready { "Listening" } else { "Disabled / unavailable" },
            config.preferred_address.as_deref().unwrap_or("Automatic (private adapters)"),
            error.or(status.error).unwrap_or_else(|| "Local operation. No cloud accounts or telemetry.".into()));
        match desktop::settings(
            &details,
            config.allow_remote_connections && config.onboarding_completed,
            paused,
        )? {
            SettingsAction::Network => self.toggle_network()?,
            SettingsAction::Pause => self.toggle_pause()?,
            SettingsAction::Pair => self.toggle_pairing_qr_popup(event_loop)?,
            SettingsAction::Adapter => {
                if let Some(preferred) =
                    desktop::network_address(config.preferred_address.as_deref())?
                {
                    let mut current = self
                        .config
                        .lock()
                        .map_err(|_| io::Error::other("Configuration unavailable"))?;
                    let mut proposed = current.clone();
                    proposed.preferred_address = preferred;
                    proposed.save()?;
                    *current = proposed;
                    drop(current);
                    self.restart_connection()?;
                }
            }
            SettingsAction::None => {
                if !config.onboarding_completed {
                    let mut current = self
                        .config
                        .lock()
                        .map_err(|_| io::Error::other("Configuration unavailable"))?;
                    current.allow_remote_connections = false;
                    current.allow_discovery = false;
                    current.onboarding_completed = true;
                    current.save()?;
                }
            }
        }
        self.refresh_tray_state()
    }

    fn edit_permissions(&mut self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let device = self
            .registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .list()
            .into_iter()
            .find(|d| d.id == id);
        let Some(device) = device else {
            return Ok(());
        };
        if let Some(permissions) = desktop::permissions(&device.name, device.permissions, None)? {
            self.cancel_pending_pairing();
            self.server.input.cancel_device(id);
            self.registry
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .set_permissions(id, permissions)?;
            self.refresh_paired_devices_menu()?;
        }
        self.refresh_tray_state()
    }

    fn reload_config_if_changed(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let modified = config_modified_time()?;
        if modified == self.last_config_modified {
            return Ok(());
        }
        self.last_config_modified = modified;

        let current = config_snapshot(&self.config)?;
        let reloaded = AppConfig::load_or_create()?;
        if reloaded == current {
            return Ok(());
        }

        let current_server = ServerRuntimeConfig::from(&current);
        let reloaded_server = ServerRuntimeConfig::from(&reloaded);
        let reloaded_launch_on_startup = reloaded.launch_on_startup;

        {
            let mut config = self
                .config
                .lock()
                .map_err(|_| io::Error::other("failed to access config"))?;
            *config = reloaded;
        }

        if current.launch_on_startup != reloaded_launch_on_startup {
            system::sync_launch_on_startup(reloaded_launch_on_startup).map_err(io::Error::other)?;
        }

        self.refresh_tray_state()?;

        if current_server != reloaded_server {
            info!(
                bind = %reloaded_server.bind_address,
                discovery_port = reloaded_server.discovery_port,
                discovery_enabled = reloaded_server.discovery_enabled,
                "WakeMATE config changed on disk, restarting server"
            );
            self.server
                .restart(self.config.clone(), self.registry.clone())?;
        } else {
            info!("WakeMATE config changed on disk, reloaded in memory");
        }

        Ok(())
    }

    fn finish(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(error) = self.startup_error.take() {
            self.server.shutdown();
            return Err(std::io::Error::other(error).into());
        }

        self.server.finish()
    }

    fn refresh_server_health(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let Some(error) = self.server.take_unexpected_failure() else {
            return Ok(());
        };

        error!(%error, "WakeMATE server stopped unexpectedly");
        self.startup_error = Some(error);
        self.refresh_tray_state()
    }
}

impl ApplicationHandler<UserEvent> for TrayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.tray_icon.is_some() {
            return;
        }

        if let Err(error) = self.initialize_tray() {
            self.startup_error = Some(error.to_string());
            error!(error = %error, "failed to initialize tray icon");
            event_loop.exit();
            return;
        }
        if config_snapshot(&self.config)
            .map(|c| !c.onboarding_completed)
            .unwrap_or(false)
        {
            if let Err(error) = self.open_settings(event_loop) {
                system::show_error_dialog("WakeMATE setup", &error.to_string());
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Menu(menu_event) => {
                if let Err(error) = self.handle_menu_event(event_loop, menu_event) {
                    error!(error = %error, "failed to handle tray menu event");
                    system::show_error_dialog("WakeMATE could not complete this change", &format!("{error}\n\nIf you were revoking a phone or resetting pairing, quit WakeMATE until the data folder is writable and the disk has space. Retry the change before resuming normal use."));
                }
            }
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let mut close_popup = false;

        if let Some(popup) = self.pairing_popup.as_mut() {
            if popup.window_id() != window_id {
                return;
            }

            match event {
                WindowEvent::RedrawRequested => {
                    if let Err(error) = popup.render() {
                        error!(error = %error, "failed to render pairing popup");
                        close_popup = true;
                    }
                }
                WindowEvent::Resized(_) => popup.request_redraw(),
                WindowEvent::CloseRequested | WindowEvent::Focused(false) => close_popup = true,
                WindowEvent::KeyboardInput { event, .. }
                    if is_escape_press(event.state, &event.physical_key, &event.logical_key) =>
                {
                    close_popup = true;
                }
                _ => {}
            }
        }

        if close_popup {
            self.pairing_popup = None;
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if self.exit_signal.requested() {
            event_loop.exit();
            return;
        }
        if self
            .pairing_popup
            .as_ref()
            .is_some_and(|popup| popup.created_at.elapsed() >= Duration::from_secs(600))
        {
            self.pairing_popup = None;
        }
        if now >= self.next_config_poll {
            if let Some(pairing) = &self.server.pairing {
                pairing.approval_state();
            }
            let _ = self.refresh_tray_state();
            if let Err(error) = self.refresh_server_health() {
                error!(error = %error, "failed to refresh WakeMATE server health");
            }
            if let Err(error) = self.reload_config_if_changed() {
                error!(error = %error, "failed to reload config changes from disk");
            }
            // Approvals land on the notifier thread; pick them up here so
            // the "Paired Devices" submenu stays current.
            if let Err(error) = self.refresh_paired_devices_menu() {
                error!(error = %error, "failed to refresh the paired-devices menu");
            }
            self.next_config_poll = Instant::now() + CONFIG_POLL_INTERVAL;
        }

        if self.pairing_popup_requested {
            let ready = self
                .server
                .status
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .https_ready;
            if !self.server.active {
                self.pairing_popup_requested = false;
            } else if ready {
                if let Err(error) = self.toggle_pairing_qr_popup(event_loop) {
                    self.pairing_popup_requested = false;
                    system::show_error_dialog("Pairing code unavailable", &error.to_string());
                }
            }
        }
        let mut close_popup = false;
        let popup_deadline = Instant::now() + Duration::from_millis(16);

        if let Some(popup) = self.pairing_popup.as_mut() {
            close_popup = popup.should_close_for_click_away();
        }

        let next_deadline =
            if self.pairing_popup.is_some() && popup_deadline < self.next_config_poll {
                popup_deadline
            } else {
                self.next_config_poll
            };
        event_loop.set_control_flow(ControlFlow::WaitUntil(next_deadline));

        if close_popup {
            self.pairing_popup = None;
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.pairing_popup = None;
        self.server.shutdown();
    }
}

enum UserEvent {
    Menu(MenuEvent),
}

struct MenuIds {
    show_pairing_qr: MenuId,
    restart_connection: MenuId,
    rotate_pairing_token: MenuId,
    launch_on_startup: MenuId,
    open_data_folder: MenuId,
    reset_companion: MenuId,
    quit: MenuId,
    settings_window: MenuId,
    pause_access: MenuId,
    network_access: MenuId,
}

struct TrayMenu {
    status: MenuItem,
    launch_on_startup: CheckMenuItem,
    paired_devices: Submenu,
    pause_access: CheckMenuItem,
    network_access: CheckMenuItem,
    ids: MenuIds,
}

struct PairingPopup {
    window: Rc<Window>,
    _context: PopupContext,
    surface: PopupSurface,
    qr_payload: String,
    created_at: Instant,
    device_name: String,
    last_mouse_buttons: MouseButtons,
}

impl PairingPopup {
    fn new(
        event_loop: &ActiveEventLoop,
        config: &SharedConfig,
        pairing: Option<&Arc<PairingCoordinator>>,
        tray_rect: Option<TrayRect>,
        owner_window: isize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let snapshot = config_snapshot(config)?;
        let pairing = pairing.ok_or_else(|| {
            io::Error::other("WakeMATE server is not running; pairing QR is unavailable")
        })?;
        let mut attributes = Window::default_attributes()
            .with_title("WakeMATE Pairing")
            .with_active(true)
            .with_visible(false)
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_inner_size(LogicalSize::new(POPUP_LOGICAL_WIDTH, POPUP_LOGICAL_HEIGHT))
            .with_skip_taskbar(true)
            .with_drag_and_drop(false)
            .with_undecorated_shadow(true)
            .with_corner_preference(CornerPreference::Round);

        if owner_window != 0 {
            attributes = attributes.with_owner_window(owner_window);
        }

        let window = Rc::new(event_loop.create_window(attributes)?);
        let context = PopupContext::new(window.clone())?;
        let surface = PopupSurface::new(&context, window.clone())?;

        let popup_size = window.inner_size();
        let position = popup_position_from_tray_rect(
            tray_rect,
            popup_size,
            event_loop.available_monitors().collect(),
        );
        window.set_outer_position(position);
        window.set_visible(true);
        window.focus_window();
        window.request_redraw();

        let tls_identity = TlsIdentity::load_or_create()?;
        // Prefer a fresh one-time pairing-session token minted from the
        // coordinator actually serving this process's HTTP requests, so the
        // QR code's credential is short-lived and single-use (see
        // pairing.rs `PairingSessionSlot`). If the server stopped, refuse to
        // show a QR instead of exposing the long-lived shared token in a code
        // that no active local coordinator could approve.
        let qr_token = pairing.mint_pairing_session_token();
        let qr_payload = pairing_qr_payload(
            &qr_token,
            &snapshot.device_name,
            snapshot.bind_port(),
            snapshot.tls_port,
            tls_identity.fingerprint(),
            snapshot.preferred_address.as_deref(),
        );

        Ok(Self {
            window,
            _context: context,
            surface,
            qr_payload,
            created_at: Instant::now(),
            device_name: snapshot.device_name,
            last_mouse_buttons: current_mouse_buttons(),
        })
    }

    fn window_id(&self) -> WindowId {
        self.window.id()
    }

    fn request_redraw(&self) {
        self.window.request_redraw();
    }

    fn should_close_for_click_away(&mut self) -> bool {
        let mouse_buttons = current_mouse_buttons();
        let should_close = mouse_buttons.has_new_press(self.last_mouse_buttons)
            && current_cursor_position()
                .map(|point| !self.contains_screen_point(point))
                .unwrap_or(false);

        self.last_mouse_buttons = mouse_buttons;
        should_close
    }

    fn contains_screen_point(&self, point: ScreenPoint) -> bool {
        let Ok(position) = self.window.outer_position() else {
            return false;
        };

        let size = self.window.outer_size();
        screen_rect_contains_point(position.x, position.y, size.width, size.height, point)
    }

    fn render(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let size = self.window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        self.surface.resize(
            NonZeroU32::new(width).unwrap(),
            NonZeroU32::new(height).unwrap(),
        )?;

        let frame = render_pairing_popup(&self.qr_payload, &self.device_name, width, height)
            .map_err(std::io::Error::other)?;

        let mut buffer = self.surface.buffer_mut()?;
        buffer[..].copy_from_slice(&frame);
        buffer.present()?;
        Ok(())
    }
}

struct MonitorBounds {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

struct PopupAnchor {
    center_x: i32,
    top_y: i32,
    bottom_y: i32,
}

struct PopupLayout {
    qr_x: i32,
    qr_y: i32,
    qr_size: u32,
    title_y: i32,
    subtitle_y: i32,
    device_y: i32,
    footer_y: i32,
    footer2_y: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct MouseButtons {
    left: bool,
    right: bool,
    middle: bool,
}

impl MouseButtons {
    fn has_new_press(self, previous: Self) -> bool {
        (!previous.left && self.left)
            || (!previous.right && self.right)
            || (!previous.middle && self.middle)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScreenPoint {
    x: i32,
    y: i32,
}

fn popup_position_from_tray_rect(
    tray_rect: Option<TrayRect>,
    popup_size: PhysicalSize<u32>,
    monitors: Vec<MonitorHandle>,
) -> PhysicalPosition<i32> {
    let Some(fallback_monitor) = monitors.first().cloned() else {
        return PhysicalPosition::new(0, 0);
    };

    let fallback_bounds = monitor_bounds(&fallback_monitor);
    let anchor = tray_rect
        .as_ref()
        .map(popup_anchor_from_tray)
        .unwrap_or_else(|| PopupAnchor {
            center_x: fallback_bounds.right - popup_size.width as i32 / 2 - POPUP_SCREEN_MARGIN,
            top_y: fallback_bounds.bottom - POPUP_SCREEN_MARGIN,
            bottom_y: fallback_bounds.bottom - POPUP_SCREEN_MARGIN,
        });

    let monitor = monitors
        .into_iter()
        .find(|monitor| point_in_monitor(anchor.center_x, anchor.bottom_y, monitor))
        .unwrap_or(fallback_monitor);

    position_popup(anchor, popup_size, monitor_bounds(&monitor))
}

fn popup_anchor_from_tray(rect: &TrayRect) -> PopupAnchor {
    let left = rect.position.x.round() as i32;
    let top = rect.position.y.round() as i32;

    PopupAnchor {
        center_x: left + rect.size.width as i32 / 2,
        top_y: top,
        bottom_y: top + rect.size.height as i32,
    }
}

fn position_popup(
    anchor: PopupAnchor,
    popup_size: PhysicalSize<u32>,
    bounds: MonitorBounds,
) -> PhysicalPosition<i32> {
    let popup_width = popup_size.width as i32;
    let popup_height = popup_size.height as i32;
    let min_x = bounds.left + POPUP_SCREEN_MARGIN;
    let max_x = (bounds.right - popup_width - POPUP_SCREEN_MARGIN).max(min_x);
    let min_y = bounds.top + POPUP_SCREEN_MARGIN;
    let max_y = (bounds.bottom - popup_height - POPUP_SCREEN_MARGIN).max(min_y);

    let x = (anchor.center_x - popup_width / 2).clamp(min_x, max_x);
    let above_y = anchor.top_y - popup_height - POPUP_OFFSET;
    let below_y = anchor.bottom_y + POPUP_OFFSET;
    let y = if above_y >= min_y { above_y } else { below_y };

    PhysicalPosition::new(x, y.clamp(min_y, max_y))
}

fn point_in_monitor(x: i32, y: i32, monitor: &MonitorHandle) -> bool {
    let bounds = monitor_bounds(monitor);
    x >= bounds.left && x < bounds.right && y >= bounds.top && y < bounds.bottom
}

fn monitor_bounds(monitor: &MonitorHandle) -> MonitorBounds {
    let position = monitor.position();
    let size = monitor.size();

    MonitorBounds {
        left: position.x,
        top: position.y,
        right: position.x + size.width as i32,
        bottom: position.y + size.height as i32,
    }
}

fn render_pairing_popup(
    qr_payload: &str,
    device_name: &str,
    width: u32,
    height: u32,
) -> Result<Vec<u32>, String> {
    let qr = QrCode::encode_text(qr_payload, QrCodeEcc::Medium)
        .map_err(|error| format!("failed to encode pairing payload as QR: {error:?}"))?;
    let layout = popup_layout(width, height, qr.size());
    let title_scale = text_scale(width);
    let subtitle_scale = title_scale.saturating_sub(1).max(1);
    let device_label = popup_device_label(device_name, 22);
    let mut pixels = vec![0; (width * height) as usize];

    fill_vertical_gradient(&mut pixels, width, height, theme::BG_BASE, theme::SURFACE);
    fill_rect(&mut pixels, width, height, 0, 0, width, 4, theme::PRIMARY);
    draw_rect_outline(
        &mut pixels,
        width,
        height,
        0,
        0,
        width,
        height,
        1,
        theme::BORDER,
    );
    fill_rect(
        &mut pixels,
        width,
        height,
        layout.qr_x - 16,
        layout.qr_y - 16,
        layout.qr_size + 32,
        layout.qr_size + 32,
        theme::BG_ELEVATED,
    );
    fill_rect(
        &mut pixels,
        width,
        height,
        layout.qr_x - 12,
        layout.qr_y - 12,
        layout.qr_size + 24,
        layout.qr_size + 24,
        0x00FFFFFF,
    );
    draw_rect_outline(
        &mut pixels,
        width,
        height,
        layout.qr_x - 12,
        layout.qr_y - 12,
        layout.qr_size + 24,
        layout.qr_size + 24,
        1,
        theme::BORDER,
    );
    draw_centered_text(
        &mut pixels,
        width,
        height,
        layout.title_y,
        "WakeMATE",
        title_scale,
        theme::TEXT_PRIMARY,
    );
    draw_centered_text(
        &mut pixels,
        width,
        height,
        layout.subtitle_y,
        "PAIR FROM YOUR PHONE",
        subtitle_scale,
        theme::ACCENT_LIGHT,
    );
    draw_centered_text(
        &mut pixels,
        width,
        height,
        layout.device_y,
        &device_label,
        1,
        theme::TEXT_MUTED,
    );
    draw_qr_code(
        &mut pixels,
        width,
        height,
        layout.qr_x,
        layout.qr_y,
        layout.qr_size,
        &qr,
    );
    draw_centered_text(
        &mut pixels,
        width,
        height,
        layout.footer_y,
        "SCAN IN THE WAKEMATE APP",
        1,
        theme::TEXT_MUTED,
    );
    draw_centered_text(
        &mut pixels,
        width,
        height,
        layout.footer2_y,
        "CLICK OUTSIDE OR PRESS ESC",
        1,
        theme::TEXT_FAINT,
    );

    Ok(pixels)
}

fn popup_layout(width: u32, height: u32, qr_modules: i32) -> PopupLayout {
    let title_scale = text_scale(width);
    let subtitle_scale = title_scale.saturating_sub(1).max(1);
    let title_height = glyph_height(title_scale);
    let subtitle_height = glyph_height(subtitle_scale);
    let header_gap = (4 * title_scale as i32).max(8);
    let title_y = 22;
    let subtitle_y = title_y + title_height + header_gap;
    let device_y = subtitle_y + subtitle_height + 14;

    let qr_total_modules = (qr_modules + 8).max(1) as u32;
    let max_qr_size = width
        .saturating_sub(64)
        .min(height.saturating_sub((device_y + 86).max(0) as u32));
    let qr_size = max_qr_size - (max_qr_size % qr_total_modules);
    let qr_size = qr_size.max(qr_total_modules);
    let qr_x = ((width - qr_size) / 2) as i32;
    let qr_y = device_y + 20;
    let footer_y = qr_y + qr_size as i32 + 22;
    let footer2_y = footer_y + glyph_height(1) + 8;

    PopupLayout {
        qr_x,
        qr_y,
        qr_size,
        title_y,
        subtitle_y,
        device_y,
        footer_y,
        footer2_y,
    }
}

fn text_scale(width: u32) -> u32 {
    if width >= 340 {
        3
    } else {
        2
    }
}

fn draw_qr_code(
    pixels: &mut [u32],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    qr_size: u32,
    qr: &QrCode,
) {
    let border = 4;
    let modules = (qr.size() + border * 2) as u32;
    let module_px = (qr_size / modules).max(1);
    let used_size = modules * module_px;
    let origin_x = x + ((qr_size - used_size) / 2) as i32;
    let origin_y = y + ((qr_size - used_size) / 2) as i32;

    for module_y in 0..modules {
        for module_x in 0..modules {
            let actual_x = module_x as i32 - border;
            let actual_y = module_y as i32 - border;
            let is_dark = actual_x >= 0
                && actual_y >= 0
                && actual_x < qr.size()
                && actual_y < qr.size()
                && qr.get_module(actual_x, actual_y);

            if is_dark {
                fill_rect(
                    pixels,
                    width,
                    height,
                    origin_x + (module_x * module_px) as i32,
                    origin_y + (module_y * module_px) as i32,
                    module_px,
                    module_px,
                    rgb(17, 17, 17),
                );
            }
        }
    }
}

fn popup_device_label(device_name: &str, max_chars: usize) -> String {
    let cleaned = device_name
        .chars()
        .map(|ch| {
            let upper = ch.to_ascii_uppercase();
            if upper.is_ascii_alphanumeric() || matches!(upper, ' ' | '-' | '_' | '.') {
                upper
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if cleaned.is_empty() {
        return "THIS DEVICE".to_string();
    }

    let count = cleaned.chars().count();
    if count <= max_chars {
        return cleaned;
    }

    let keep = max_chars.saturating_sub(3);
    format!("{}...", cleaned.chars().take(keep).collect::<String>())
}

fn is_escape_press(state: ElementState, physical_key: &PhysicalKey, logical_key: &Key) -> bool {
    state == ElementState::Pressed
        && (matches!(physical_key, PhysicalKey::Code(KeyCode::Escape))
            || matches!(logical_key, Key::Named(NamedKey::Escape)))
}

fn screen_rect_contains_point(
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    point: ScreenPoint,
) -> bool {
    let right = left.saturating_add(width as i32);
    let bottom = top.saturating_add(height as i32);
    point.x >= left && point.x < right && point.y >= top && point.y < bottom
}

fn current_cursor_position() -> Option<ScreenPoint> {
    let mut point = WinPoint { x: 0, y: 0 };

    unsafe {
        if GetCursorPos(&mut point) == 0 {
            return None;
        }
    }

    Some(ScreenPoint {
        x: point.x,
        y: point.y,
    })
}

fn current_mouse_buttons() -> MouseButtons {
    MouseButtons {
        left: is_virtual_key_pressed(VK_LBUTTON),
        right: is_virtual_key_pressed(VK_RBUTTON),
        middle: is_virtual_key_pressed(VK_MBUTTON),
    }
}

fn is_virtual_key_pressed(key_code: i32) -> bool {
    unsafe { (GetAsyncKeyState(key_code) as u16 & 0x8000) != 0 }
}

#[repr(C)]
struct WinPoint {
    x: i32,
    y: i32,
}

unsafe extern "system" {
    fn GetAsyncKeyState(key_code: i32) -> i16;
    fn GetCursorPos(point: *mut WinPoint) -> i32;
}

const VK_LBUTTON: i32 = 0x01;
const VK_RBUTTON: i32 = 0x02;
const VK_MBUTTON: i32 = 0x04;

fn glyph_height(scale: u32) -> i32 {
    (8 * scale.max(1)) as i32
}

fn draw_centered_text(
    pixels: &mut [u32],
    width: u32,
    height: u32,
    y: i32,
    text: &str,
    scale: u32,
    color: u32,
) {
    let text_width = text_pixel_width(text, scale);
    let x = (width as i32 - text_width) / 2;
    draw_text(pixels, width, height, x, y, text, scale, color);
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    pixels: &mut [u32],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    text: &str,
    scale: u32,
    color: u32,
) {
    let mut cursor_x = x;
    let scale = scale.max(1);
    let advance = (8 * scale + scale) as i32;

    for ch in text.chars() {
        if let Some(glyph) = BASIC_FONTS.get(ch) {
            draw_glyph(pixels, width, height, cursor_x, y, glyph, scale, color);
        }
        cursor_x += advance;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_glyph(
    pixels: &mut [u32],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    glyph: [u8; 8],
    scale: u32,
    color: u32,
) {
    for (row_index, row_bits) in glyph.iter().copied().enumerate() {
        for column in 0..8 {
            if row_bits & (1 << column) == 0 {
                continue;
            }

            fill_rect(
                pixels,
                width,
                height,
                x + (column * scale as i32),
                y + (row_index as i32 * scale as i32),
                scale,
                scale,
                color,
            );
        }
    }
}

fn text_pixel_width(text: &str, scale: u32) -> i32 {
    let scale = scale.max(1) as i32;
    let glyphs = text.chars().count() as i32;

    if glyphs == 0 {
        0
    } else {
        glyphs * 8 * scale + (glyphs - 1) * scale
    }
}

fn fill_vertical_gradient(pixels: &mut [u32], width: u32, height: u32, top: u32, bottom: u32) {
    let (top_r, top_g, top_b) = split_rgb(top);
    let (bottom_r, bottom_g, bottom_b) = split_rgb(bottom);

    for y in 0..height {
        let mix = if height <= 1 {
            0.0
        } else {
            y as f32 / (height - 1) as f32
        };
        let color = rgb(
            lerp_channel(top_r, bottom_r, mix),
            lerp_channel(top_g, bottom_g, mix),
            lerp_channel(top_b, bottom_b, mix),
        );

        for x in 0..width {
            pixels[(y * width + x) as usize] = color;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_rect_outline(
    pixels: &mut [u32],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    rect_width: u32,
    rect_height: u32,
    thickness: u32,
    color: u32,
) {
    if rect_width == 0 || rect_height == 0 || thickness == 0 {
        return;
    }

    fill_rect(pixels, width, height, x, y, rect_width, thickness, color);
    fill_rect(
        pixels,
        width,
        height,
        x,
        y + rect_height as i32 - thickness as i32,
        rect_width,
        thickness,
        color,
    );
    fill_rect(pixels, width, height, x, y, thickness, rect_height, color);
    fill_rect(
        pixels,
        width,
        height,
        x + rect_width as i32 - thickness as i32,
        y,
        thickness,
        rect_height,
        color,
    );
}

#[allow(clippy::too_many_arguments)]
fn fill_rect(
    pixels: &mut [u32],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    rect_width: u32,
    rect_height: u32,
    color: u32,
) {
    if rect_width == 0 || rect_height == 0 {
        return;
    }

    let start_x = x.max(0) as u32;
    let start_y = y.max(0) as u32;
    let end_x = (x + rect_width as i32).min(width as i32).max(0) as u32;
    let end_y = (y + rect_height as i32).min(height as i32).max(0) as u32;

    for row in start_y..end_y {
        let row_offset = row * width;
        for column in start_x..end_x {
            pixels[(row_offset + column) as usize] = color;
        }
    }
}

const fn rgb(red: u8, green: u8, blue: u8) -> u32 {
    ((red as u32) << 16) | ((green as u32) << 8) | blue as u32
}

const fn split_rgb(color: u32) -> (u8, u8, u8) {
    (
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
    )
}

fn lerp_channel(start: u8, end: u8, mix: f32) -> u8 {
    let start = start as f32;
    let end = end as f32;
    (start + (end - start) * mix).round().clamp(0.0, 255.0) as u8
}

fn config_snapshot(config: &SharedConfig) -> Result<AppConfig, Box<dyn std::error::Error>> {
    config
        .lock()
        .map(|guard| guard.clone())
        .map_err(|_| std::io::Error::other("failed to access application config").into())
}

/// Shows the native "allow this device to pair?" prompt on its own thread
/// (so the HTTP handler that triggered it never blocks) and, only on an
/// explicit "Yes", flips on input/power control and persists it.
/// Builds the structured JSON contents of the pairing QR code (pairing
/// contract v3), carrying everything the phone needs to save this computer,
/// pin its local TLS identity, and pair in one scan without a web redirect.
/// `token` should be a fresh one-time pairing-session token minted via
/// `PairingCoordinator::mint_pairing_session_token` wherever one is
/// available (see `toggle_pairing_qr_popup`).
fn pairing_qr_payload(
    token: &str,
    device_name: &str,
    api_port: u16,
    tls_port: u16,
    tls_fingerprint: &str,
    preferred_address: Option<&str>,
) -> String {
    let network = crate::network::candidates()
        .into_iter()
        .find(|n| preferred_address.is_none_or(|ip| ip == n.local_ip));
    let ip = network.as_ref().map(|info| info.local_ip.clone());
    let mac = network.as_ref().and_then(|info| info.mac_address.clone());

    pairing_qr::payload_from_parts(
        token,
        device_name,
        api_port,
        tls_port,
        tls_fingerprint,
        ip,
        mac,
    )
}

fn windows_pairing_notifier(context: NotifierContext) {
    thread::spawn(move || {
        let device_hint = context
            .enrollment
            .pending_device_name()
            .map(|name| format!("\"{name}\" (at {})", context.info.peer))
            .unwrap_or_else(|| context.info.peer.to_string());
        let choice = desktop::permissions(&device_hint, Permissions::default(), Some(&context));
        let result = choice.and_then(|permissions| context.complete_permissions(permissions));
        match result {
            Ok(true) => info!(peer = %context.info.peer, "WakeMATE pairing saved and approved"),
            Ok(false) => {
                info!(peer = %context.info.peer, "WakeMATE pairing denied, expired, or cancelled")
            }
            Err(error) => {
                error!(%error, "failed to save pairing approval");
                system::show_error_dialog("Pairing could not be saved", "WakeMATE could not save this pairing. Check that the data folder is writable and the disk has space, then scan a new QR code.");
            }
        }
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ServerRuntimeConfig {
    bind_address: String,
    tls_bind_address: String,
    allow_insecure_http: bool,
    discovery_port: u16,
    discovery_enabled: bool,
}

impl From<&AppConfig> for ServerRuntimeConfig {
    fn from(config: &AppConfig) -> Self {
        Self {
            bind_address: config.effective_bind_address(),
            tls_bind_address: config.effective_tls_bind_address(),
            allow_insecure_http: config.allow_insecure_http,
            discovery_port: config.discovery_port,
            discovery_enabled: config.discovery_enabled(),
        }
    }
}

fn config_modified_time() -> io::Result<Option<SystemTime>> {
    let path = AppConfig::path()?;
    match fs::metadata(path) {
        Ok(metadata) => metadata.modified().map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn load_tray_icon(assets_dir: &Path) -> Result<Icon, Box<dyn std::error::Error>> {
    for file_name in ["tray-icon.ico", "tray-icon.png"] {
        let candidate = assets_dir.join(file_name);
        if candidate.exists() {
            info!(path = %candidate.display(), "loading tray icon from assets");
            return load_icon_from_path(&candidate);
        }
    }

    warn!(
        assets = %assets_dir.display(),
        "no custom tray icon found, using built-in fallback"
    );
    default_icon()
}

fn load_icon_from_path(path: &Path) -> Result<Icon, Box<dyn std::error::Error>> {
    let image = ImageReader::open(path)?.decode()?.into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height)
        .map_err(|error| format!("invalid tray icon data in {}: {error}", path.display()).into())
}

fn default_icon() -> Result<Icon, Box<dyn std::error::Error>> {
    let width = 32u32;
    let height = 32u32;
    let mut rgba = vec![0u8; (width * height * 4) as usize];

    for y in 0..height {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            let border = x < 2 || y < 2 || x >= width - 2 || y >= height - 2;
            let left_stroke = (6..=10).contains(&x) && (7..=24).contains(&y);
            let right_stroke = (21..=25).contains(&x) && (7..=24).contains(&y);
            let center_left = (11..=14).contains(&x) && (14..=24).contains(&y);
            let center_right = (17..=20).contains(&x) && (14..=24).contains(&y);
            let pixel_on = border || left_stroke || right_stroke || center_left || center_right;

            let (r, g, b) = if pixel_on {
                theme::channels(theme::TEXT_PRIMARY)
            } else {
                theme::channels(theme::PRIMARY)
            };
            let a = 255;

            rgba[idx] = r;
            rgba[idx + 1] = g;
            rgba[idx + 2] = b;
            rgba[idx + 3] = a;
        }
    }

    Icon::from_rgba(rgba, width, height)
        .map_err(|error| format!("failed to build fallback tray icon: {error}").into())
}

#[cfg(test)]
mod tests {
    use super::{
        is_escape_press, popup_device_label, position_popup, render_pairing_popup,
        screen_rect_contains_point, MonitorBounds, MouseButtons, PopupAnchor, ScreenPoint,
    };
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    use winit::{
        event::ElementState,
        keyboard::{Key, KeyCode, NamedKey, PhysicalKey},
    };

    const TEST_TLS_FINGERPRINT: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn popup_prefers_space_above_the_tray() {
        let position = position_popup(
            PopupAnchor {
                center_x: 1880,
                top_y: 1040,
                bottom_y: 1060,
            },
            PhysicalSize::new(252, 320),
            MonitorBounds {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
        );

        assert_eq!(position, PhysicalPosition::new(1660, 708));
    }

    #[test]
    fn popup_flips_below_when_the_tray_is_near_the_top() {
        let position = position_popup(
            PopupAnchor {
                center_x: 420,
                top_y: 4,
                bottom_y: 28,
            },
            PhysicalSize::new(252, 320),
            MonitorBounds {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
        );

        assert_eq!(position, PhysicalPosition::new(294, 40));
    }

    #[test]
    fn popup_frame_renders_full_pixel_buffer() {
        let frame = render_pairing_popup("test-token", "Desk Rig", 252, 320).unwrap();
        assert_eq!(frame.len(), 252 * 320);
        assert!(frame.contains(&0x00111111));
    }

    #[test]
    fn render_pairing_popup_encodes_a_v3_payload() {
        let payload = crate::pairing_qr::payload_from_parts(
            "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
            "Desk Rig",
            7777,
            7778,
            TEST_TLS_FINGERPRINT,
            Some("192.168.1.50".to_string()),
            Some("00:11:22:33:44:55".to_string()),
        );
        // The payload is much longer than the old bare token; make sure it
        // still fits in a QR code at the popup's render size.
        let frame = render_pairing_popup(&payload, "Desk Rig", 252, 320).unwrap();
        assert_eq!(frame.len(), (252 * 320) as usize);
    }

    #[test]
    fn popup_device_label_trims_long_names() {
        let label = popup_device_label("A ridiculously long desktop device name", 12);
        assert_eq!(label, "A RIDICUL...");
    }

    #[test]
    fn escape_press_matches_physical_escape() {
        assert!(is_escape_press(
            ElementState::Pressed,
            &PhysicalKey::Code(KeyCode::Escape),
            &Key::Character("x".into()),
        ));
    }

    #[test]
    fn escape_press_matches_logical_escape() {
        assert!(is_escape_press(
            ElementState::Pressed,
            &PhysicalKey::Code(KeyCode::KeyQ),
            &Key::Named(NamedKey::Escape),
        ));
    }

    #[test]
    fn escape_press_ignores_releases() {
        assert!(!is_escape_press(
            ElementState::Released,
            &PhysicalKey::Code(KeyCode::Escape),
            &Key::Named(NamedKey::Escape),
        ));
    }

    #[test]
    fn mouse_buttons_detect_new_press_edges() {
        assert!(MouseButtons {
            left: true,
            ..MouseButtons::default()
        }
        .has_new_press(MouseButtons::default()));

        assert!(!MouseButtons {
            left: true,
            ..MouseButtons::default()
        }
        .has_new_press(MouseButtons {
            left: true,
            ..MouseButtons::default()
        }));
    }

    #[test]
    fn screen_rect_contains_point_uses_exclusive_bottom_right_edge() {
        assert!(screen_rect_contains_point(
            100,
            200,
            50,
            60,
            ScreenPoint { x: 100, y: 200 },
        ));
        assert!(screen_rect_contains_point(
            100,
            200,
            50,
            60,
            ScreenPoint { x: 149, y: 259 },
        ));
        assert!(!screen_rect_contains_point(
            100,
            200,
            50,
            60,
            ScreenPoint { x: 150, y: 260 },
        ));
    }
}
