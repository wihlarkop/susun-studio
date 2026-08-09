use std::sync::atomic::{AtomicBool, Ordering};

use log::warn;
use serde::{Deserialize, Serialize};
use tauri::{
    Emitter, Manager,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

pub const TRAY_ID: &str = "susun-runtime";
pub const MAX_NATIVE_LABEL_CHARS: usize = 80;

const MENU_OPEN: &str = "open";
const MENU_RUNTIME_SETTINGS: &str = "runtime-settings";
const MENU_RUNTIME_SETUP: &str = "runtime-setup";
const MENU_START: &str = "start";
const MENU_STOP: &str = "stop";
const MENU_QUIT: &str = "quit";
const EVENT_NAVIGATION: &str = "studio-tray-navigation-requested";
const EVENT_RUNTIME_ACTION: &str = "runtime-tray-action-requested";

#[derive(Default)]
pub struct QuitIntent(AtomicBool);

impl QuitIntent {
    pub fn request(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn requested(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrayRuntimeKind {
    BuiltInManaged,
    External,
    Unconfigured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrayRuntimeState {
    Ready,
    Stopped,
    Unavailable,
    Missing,
    Unconfigured,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrayRuntimeSummary {
    pub title: String,
    pub kind: TrayRuntimeKind,
    pub state: TrayRuntimeState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Open,
    RuntimeSettings,
    RuntimeSetup,
    Start,
    Stop,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TrayNavigationIntent {
    Open,
    RuntimeSettings,
    RuntimeSetup,
    Runtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TrayRuntimeAction {
    Start,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayMenuModel {
    pub status_label: String,
    pub actions: Vec<TrayAction>,
}

pub fn menu_model(summary: TrayRuntimeSummary) -> TrayMenuModel {
    let mut actions = vec![TrayAction::Open];
    match (summary.kind, summary.state) {
        (TrayRuntimeKind::BuiltInManaged, TrayRuntimeState::Stopped) => {
            actions.push(TrayAction::Start);
        }
        (TrayRuntimeKind::BuiltInManaged, TrayRuntimeState::Ready) => {
            actions.push(TrayAction::Stop);
        }
        (TrayRuntimeKind::Unconfigured, _) => actions.push(TrayAction::RuntimeSetup),
        _ => {}
    }
    actions.extend([TrayAction::RuntimeSettings, TrayAction::Quit]);

    TrayMenuModel {
        status_label: bounded_label(&format!(
            "{} - {}",
            summary.title,
            state_label(summary.state)
        )),
        actions,
    }
}

pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    let summary = TrayRuntimeSummary {
        title: "Runtime not configured".to_owned(),
        kind: TrayRuntimeKind::Unconfigured,
        state: TrayRuntimeState::Unconfigured,
    };
    let model = menu_model(summary);
    let menu = native_menu(app.handle(), &model)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip(&model.status_label);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder
        .on_menu_event(|app, event| {
            if let Some(action) = action_from_menu_id(&event.id().0) {
                handle_action(app, action);
            }
        })
        .build(app)?;
    Ok(())
}

pub fn update_summary(app: &tauri::AppHandle, summary: TrayRuntimeSummary) -> tauri::Result<()> {
    let model = menu_model(summary);
    let menu = native_menu(app, &model)?;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(menu))?;
        tray.set_tooltip(Some(model.status_label))?;
    }
    Ok(())
}

fn native_menu(app: &tauri::AppHandle, model: &TrayMenuModel) -> tauri::Result<Menu<tauri::Wry>> {
    let open = menu_item(app, MENU_OPEN, "Open Susun Studio", true)?;
    let status = menu_item(app, "status", &model.status_label, false)?;
    let settings = menu_item(app, MENU_RUNTIME_SETTINGS, "Runtime settings", true)?;
    let quit = menu_item(app, MENU_QUIT, "Quit", true)?;
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = vec![&open, &status];
    let action_item = match model.actions.iter().find(|action| {
        matches!(
            action,
            TrayAction::Start | TrayAction::Stop | TrayAction::RuntimeSetup
        )
    }) {
        Some(TrayAction::Start) => Some(menu_item(app, MENU_START, "Start Susun Runtime", true)?),
        Some(TrayAction::Stop) => Some(menu_item(app, MENU_STOP, "Stop Susun Runtime", true)?),
        Some(TrayAction::RuntimeSetup) => Some(menu_item(
            app,
            MENU_RUNTIME_SETUP,
            "Open Runtime Setup",
            true,
        )?),
        _ => None,
    };
    if let Some(item) = action_item.as_ref() {
        items.push(item);
    }
    items.push(&settings);
    items.push(&quit);
    Menu::with_items(app, &items)
}

fn menu_item(
    app: &tauri::AppHandle,
    id: &str,
    label: &str,
    enabled: bool,
) -> tauri::Result<MenuItem<tauri::Wry>> {
    MenuItem::with_id(app, id, bounded_label(label), enabled, None::<&str>)
}

fn action_from_menu_id(id: &str) -> Option<TrayAction> {
    match id {
        MENU_OPEN => Some(TrayAction::Open),
        MENU_RUNTIME_SETTINGS => Some(TrayAction::RuntimeSettings),
        MENU_RUNTIME_SETUP => Some(TrayAction::RuntimeSetup),
        MENU_START => Some(TrayAction::Start),
        MENU_STOP => Some(TrayAction::Stop),
        MENU_QUIT => Some(TrayAction::Quit),
        _ => None,
    }
}

fn handle_action(app: &tauri::AppHandle, action: TrayAction) {
    match action {
        TrayAction::Quit => {
            app.state::<QuitIntent>().request();
            app.state::<crate::daemon::DaemonSupervisor>().shutdown();
            app.exit(0);
        }
        TrayAction::Open => emit_navigation(app, TrayNavigationIntent::Open),
        TrayAction::RuntimeSettings => emit_navigation(app, TrayNavigationIntent::RuntimeSettings),
        TrayAction::RuntimeSetup => emit_navigation(app, TrayNavigationIntent::RuntimeSetup),
        TrayAction::Start => emit_runtime_action(app, TrayRuntimeAction::Start),
        TrayAction::Stop => emit_runtime_action(app, TrayRuntimeAction::Stop),
    }
}

fn emit_navigation(app: &tauri::AppHandle, intent: TrayNavigationIntent) {
    show_and_focus(app);
    if let Err(error) = app.emit(EVENT_NAVIGATION, intent) {
        warn!("event=tray_navigation_emit_failed error={error}");
    }
}

fn emit_runtime_action(app: &tauri::AppHandle, action: TrayRuntimeAction) {
    show_and_focus(app);
    if let Err(error) = app.emit(EVENT_NAVIGATION, TrayNavigationIntent::Runtime) {
        warn!("event=tray_navigation_emit_failed error={error}");
        return;
    }
    if let Err(error) = app.emit(EVENT_RUNTIME_ACTION, action) {
        warn!("event=tray_runtime_action_emit_failed error={error}");
    }
}

fn show_and_focus(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Err(error) = window.show() {
        warn!("event=tray_window_show_failed error={error}");
    }
    if let Err(error) = window.set_focus() {
        warn!("event=tray_window_focus_failed error={error}");
    }
}

fn state_label(state: TrayRuntimeState) -> &'static str {
    match state {
        TrayRuntimeState::Ready => "Ready",
        TrayRuntimeState::Stopped => "Stopped",
        TrayRuntimeState::Unavailable => "Unavailable",
        TrayRuntimeState::Missing => "Missing",
        TrayRuntimeState::Unconfigured => "Not configured",
    }
}

fn bounded_label(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= MAX_NATIVE_LABEL_CHARS {
        return compact;
    }
    let shortened = compact
        .chars()
        .take(MAX_NATIVE_LABEL_CHARS.saturating_sub(3))
        .collect::<String>();
    format!("{shortened}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_stopped_exposes_only_start() {
        let model = menu_model(TrayRuntimeSummary {
            title: "Susun Runtime".to_owned(),
            kind: TrayRuntimeKind::BuiltInManaged,
            state: TrayRuntimeState::Stopped,
        });

        assert!(model.actions.contains(&TrayAction::Start));
        assert!(!model.actions.contains(&TrayAction::Stop));
    }

    #[test]
    fn built_in_ready_exposes_only_stop() {
        let model = menu_model(TrayRuntimeSummary {
            title: "Susun Runtime".to_owned(),
            kind: TrayRuntimeKind::BuiltInManaged,
            state: TrayRuntimeState::Ready,
        });

        assert!(model.actions.contains(&TrayAction::Stop));
        assert!(!model.actions.contains(&TrayAction::Start));
    }

    #[test]
    fn unavailable_and_external_runtimes_never_expose_lifecycle_actions() {
        for summary in [
            TrayRuntimeSummary {
                title: "Susun Runtime".to_owned(),
                kind: TrayRuntimeKind::BuiltInManaged,
                state: TrayRuntimeState::Unavailable,
            },
            TrayRuntimeSummary {
                title: "Definitely built in".to_owned(),
                kind: TrayRuntimeKind::External,
                state: TrayRuntimeState::Stopped,
            },
        ] {
            let model = menu_model(summary);
            assert!(!model.actions.contains(&TrayAction::Start));
            assert!(!model.actions.contains(&TrayAction::Stop));
        }
    }

    #[test]
    fn unconfigured_opens_runtime_setup_and_bounds_labels() {
        let model = menu_model(TrayRuntimeSummary {
            title: "x".repeat(500),
            kind: TrayRuntimeKind::Unconfigured,
            state: TrayRuntimeState::Unconfigured,
        });

        assert!(model.actions.contains(&TrayAction::RuntimeSetup));
        assert!(model.status_label.chars().count() <= MAX_NATIVE_LABEL_CHARS);
    }
}
