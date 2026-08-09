use serde::{Deserialize, Serialize};
use tauri::{
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
    let menu = native_menu(app, &model)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip(&model.status_label);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn native_menu(app: &tauri::App, model: &TrayMenuModel) -> tauri::Result<Menu<tauri::Wry>> {
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
    app: &tauri::App,
    id: &str,
    label: &str,
    enabled: bool,
) -> tauri::Result<MenuItem<tauri::Wry>> {
    MenuItem::with_id(app, id, bounded_label(label), enabled, None::<&str>)
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
