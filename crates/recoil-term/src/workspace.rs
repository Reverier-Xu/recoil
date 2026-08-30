//! The workspace root: dock assembly, menu, actions, and layout persistence.

use std::path::PathBuf;

use gpui::{
  App, AppContext as _, Context, Entity, Global, InteractiveElement as _, IntoElement,
  ParentElement as _, Render, Styled as _, Window, WindowOptions, actions,
};
use woocraft::{
  AppMenuBar, DockArea, DockItem, DockPlacement, ThemeMode, TitleBar, v_flex, window_border,
};

use crate::{
  panels,
  stores::{
    sessions::{SessionEvent, session_store, try_session_store},
    settings::settings_store,
  },
  terminal::panel,
};

actions!(
  recoil,
  [
    NewTerminal,
    CloseActiveTab,
    ToggleLeftDock,
    OpenSettings,
    QuitRecoil
  ]
);

/// One terminal session to restore on startup. PTYs never cross process
/// restarts, so a record restores as a fresh local shell.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
struct SessionRecord {
  /// The working directory to start the restored shell in. Only local cwds
  /// are recorded — a remote (ssh) path is meaningless for a local shell.
  cwd: Option<PathBuf>,
}

/// The persisted workspace state (`state.json`).
///
/// Deliberately minimal: which terminal sessions were open (each restores
/// as a fresh local shell in its last local directory) and which one was
/// active. The dock layout itself is not persisted; it is assembled the
/// same way on every startup.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct WorkspaceState {
  sessions: Vec<SessionRecord>,
  /// The active tab's index into `sessions`.
  active: Option<usize>,
}

struct GlobalActiveDockArea(Entity<DockArea>);

impl Global for GlobalActiveDockArea {}

struct GlobalAppMenuBar(Entity<AppMenuBar>);

impl Global for GlobalAppMenuBar {}

/// Returns the dock area of the active workspace, if one exists.
pub fn active_dock_area(cx: &App) -> Option<Entity<DockArea>> {
  cx.try_global::<GlobalActiveDockArea>()
    .map(|global| global.0.clone())
}

/// The workspace state directory: `<config dir>/recoil`.
pub fn state_dir() -> Option<PathBuf> {
  #[cfg(target_os = "macos")]
  let base =
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"));
  #[cfg(target_os = "windows")]
  let base = std::env::var_os("APPDATA").map(PathBuf::from);
  #[cfg(all(unix, not(target_os = "macos")))]
  let base = std::env::var_os("XDG_CONFIG_HOME")
    .map(PathBuf::from)
    .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
  base.map(|dir| dir.join("recoil"))
}

fn state_file() -> Option<PathBuf> {
  state_dir().map(|dir| dir.join("state.json"))
}

/// The workspace root view.
pub struct Workspace {
  dock_area: Entity<DockArea>,
  app_menu_bar: Entity<AppMenuBar>,
}

impl Workspace {
  /// Assembles the workspace inside a freshly opened window.
  pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
    let dock_area = cx.new(|cx| DockArea::new("recoil-dock-area", Some(1), window, cx));
    let app_menu_bar = AppMenuBar::new(cx);

    let workspace = cx.new(|_cx| Self {
      dock_area: dock_area.clone(),
      app_menu_bar: app_menu_bar.clone(),
    });

    build_layout(&dock_area, window, cx);

    cx.set_global(GlobalActiveDockArea(dock_area));
    cx.set_global(GlobalAppMenuBar(app_menu_bar.clone()));
    workspace
  }

  fn on_new_terminal(&mut self, _: &NewTerminal, window: &mut Window, cx: &mut Context<Self>) {
    panel::open_local_terminal(&self.dock_area, None, window, cx);
  }

  fn on_close_active_tab(
    &mut self, _: &CloseActiveTab, window: &mut Window, cx: &mut Context<Self>,
  ) {
    // Tab close detaches the session (ADR-0001): the panel's `on_removed`
    // hook performs the store detach.
    if let Some(active) = center_active_panel(&self.dock_area, cx) {
      let panel_id = active.panel_id(cx);
      self.dock_area.update(cx, |area, cx| {
        area.close_panel_by_id(&panel_id, window, cx);
      });
    }
  }

  fn on_toggle_left_dock(
    &mut self, _: &ToggleLeftDock, window: &mut Window, cx: &mut Context<Self>,
  ) {
    self.dock_area.update(cx, |area, cx| {
      area.toggle_dock(DockPlacement::Left, window, cx);
    });
  }

  fn on_quit(&mut self, _: &QuitRecoil, _: &mut Window, cx: &mut Context<Self>) {
    quit(cx);
  }

  fn on_open_settings(&mut self, _: &OpenSettings, _window: &mut Window, cx: &mut Context<Self>) {
    panels::settings::SettingsPanel::open(cx);
  }
}

/// The active panel of the center area, when the center is a tab layout.
fn center_active_panel(
  dock_area: &Entity<DockArea>, cx: &App,
) -> Option<std::sync::Arc<dyn woocraft::PanelView>> {
  let center = dock_area.read(cx).center();
  match center {
    DockItem::Tabs { view, .. } => view.read(cx).active_panel(cx),
    _ => None,
  }
}

fn build_layout(dock_area: &Entity<DockArea>, window: &mut Window, cx: &mut App) {
  // Shown whenever the main area has no terminal panels left.
  dock_area.update(cx, |area, cx| {
    area.set_center_placeholder(crate::welcome::view(window, cx), window, cx);
  });
  panels::build_left_dock(dock_area, window, cx);

  // Restore the sessions that were open at quit as fresh local shells; PTYs
  // never cross process restarts. With nothing persisted, spawn the default
  // terminal so the app never opens into an empty workbench.
  let state = load_state();
  let mut records = state.sessions;
  if records.is_empty() {
    records.push(SessionRecord::default());
  }
  let store = session_store(cx);
  for record in &records {
    let spawned = store.update(cx, |store, cx| store.spawn_local(record.cwd.clone(), cx));
    match spawned {
      Ok(id) => {
        let p = cx.new(|cx| panel::TerminalPanel::for_session(id, window, cx));
        dock_area.update(cx, |area, cx| {
          area.add_to_center(std::sync::Arc::new(p), window, cx);
        });
      }
      Err(err) => {
        tracing::warn!(error = %err, "failed to restore a session; skipping the record")
      }
    }
  }
  if let Some(index) = state.active {
    let ids: Vec<String> = store
      .read(cx)
      .entries()
      .map(|entry| format!("terminal:{}", entry.meta.id))
      .collect();
    if let Some(panel_id) = ids.get(index) {
      dock_area.update(cx, |area, cx| {
        area.activate_panel_by_id(panel_id, window, cx);
      });
    }
  }
}

fn load_state() -> WorkspaceState {
  let Some(path) = state_file() else {
    return WorkspaceState::default();
  };
  let Ok(file) = std::fs::read_to_string(&path) else {
    return WorkspaceState::default();
  };
  serde_json::from_str(&file).unwrap_or_else(|err| {
    tracing::warn!(error = %err, path = %path.display(), "failed to parse the workspace state");
    WorkspaceState::default()
  })
}

/// Persists which sessions are open and which one is active. G6 adds atomic
/// writes and crash safety.
pub fn save_state(cx: &App) {
  let Some(path) = state_file() else {
    return;
  };
  let Some(store) = try_session_store(cx) else {
    return;
  };
  let store = store.read(cx);
  let sessions: Vec<SessionRecord> = store
    .entries()
    .filter(|entry| entry.is_alive())
    .map(|entry| SessionRecord {
      cwd: match entry.meta.ssh() {
        // A remote path is meaningless for the local shell a session
        // restores as.
        Some(_) => None,
        None => entry.meta.observation.cwd.clone(),
      },
    })
    .collect();
  let active = active_dock_area(cx)
    .and_then(|area| center_active_panel(&area, cx))
    .and_then(|panel| {
      let panel_id = panel.panel_id(cx);
      let id = panel_id.strip_prefix("terminal:")?;
      store
        .entries()
        .position(|entry| entry.meta.id.as_str() == id)
    });
  let state = WorkspaceState { sessions, active };
  let Ok(json) = serde_json::to_string_pretty(&state) else {
    return;
  };
  if let Err(err) = std::fs::create_dir_all(path.parent().expect("state file has a parent"))
    .and_then(|_| std::fs::write(&path, json))
  {
    tracing::warn!(error = %err, path = %path.display(), "failed to persist the workspace state");
  }
}

/// Quits the application. Live sessions are terminated when the process
/// dies; the G6 tray flow adds explicit confirmation.
pub fn quit(cx: &mut App) {
  let store = session_store(cx);
  let live = store.read(cx).live_count();
  if live > 0 {
    tracing::info!(sessions = live, "quitting with live sessions");
  }
  save_state(cx);
  cx.quit();
}

impl Render for Workspace {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    window_border().child(
      v_flex()
        .size_full()
        .min_h_0()
        .on_action(cx.listener(Self::on_new_terminal))
        .on_action(cx.listener(Self::on_close_active_tab))
        .on_action(cx.listener(Self::on_toggle_left_dock))
        .on_action(cx.listener(Self::on_open_settings))
        .on_action(cx.listener(Self::on_quit))
        .child(
          TitleBar::new()
            .title(crate::localization::t!("app.name").to_string())
            .app_menu_bar(self.app_menu_bar.clone())
            .language_button(true)
            .on_language_button_click(|_, _, cx| rebuild_for_locale(cx)),
        )
        .child(self.dock_area.clone()),
    )
  }
}

/// Window options for the main workspace window.
pub fn window_options() -> WindowOptions {
  WindowOptions {
    titlebar: Some(TitleBar::title_bar_options()),
    #[cfg(target_os = "linux")]
    window_background: gpui::WindowBackgroundAppearance::Transparent,
    #[cfg(target_os = "linux")]
    window_decorations: Some(gpui::WindowDecorations::Client),
    ..Default::default()
  }
}

/// Rebuilds every locale-dependent surface: the application menu (the
/// AppMenuBar caches `cx.get_menus()`, so it must be reloaded), the native
/// menu on macOS, and the tray menu.
pub fn rebuild_for_locale(cx: &mut App) {
  set_app_menu(cx);
  let bar = cx
    .try_global::<GlobalAppMenuBar>()
    .map(|global| global.0.clone());
  if let Some(bar) = bar {
    bar.update(cx, |bar, cx| bar.reload(cx));
  }
  #[cfg(feature = "tray")]
  crate::tray::rebuild(cx);
}

/// Sets up the application menu.
pub fn set_app_menu(cx: &mut App) {
  use gpui::{Menu, MenuItem};
  use woocraft::gpui::SharedString;
  let label = |key: &str| SharedString::from(crate::localization::t!(key).to_string());
  cx.set_menus(vec![
    Menu {
      name: label("menu.file"),
      disabled: false,
      items: vec![
        MenuItem::action(label("menu.new_terminal"), NewTerminal),
        MenuItem::action(label("menu.close_tab"), CloseActiveTab),
        MenuItem::separator(),
        MenuItem::action(label("menu.settings"), OpenSettings),
        MenuItem::separator(),
        MenuItem::action(label("menu.quit"), QuitRecoil),
      ],
    },
    Menu {
      name: label("menu.view"),
      disabled: false,
      items: vec![MenuItem::action(
        label("menu.toggle_left_dock"),
        ToggleLeftDock,
      )],
    },
  ]);
}

/// Registers the default keymap.
///
/// In gpui keystrokes `cmd` is the platform key — Cmd on macOS but the
/// Super/Win key on Linux and Windows, where window managers already own
/// those combinations. Only macOS gets the ⌘ aliases; every platform keeps
/// the Ctrl-based bindings.
pub fn bind_keys(cx: &mut App) {
  use gpui::KeyBinding;
  #[allow(unused_mut)]
  let mut keys = vec![
    KeyBinding::new("ctrl-shift-t", NewTerminal, None),
    KeyBinding::new("ctrl-shift-w", CloseActiveTab, None),
    KeyBinding::new("ctrl-b", ToggleLeftDock, None),
    KeyBinding::new("ctrl-shift-q", QuitRecoil, None),
  ];
  #[cfg(target_os = "macos")]
  keys.extend([
    KeyBinding::new("cmd-t", NewTerminal, None),
    KeyBinding::new("cmd-w", CloseActiveTab, None),
    KeyBinding::new("cmd-b", ToggleLeftDock, None),
    KeyBinding::new("cmd-q", QuitRecoil, None),
  ]);
  cx.bind_keys(keys);
}

/// Subscribes the workspace to store events that affect persisted state and
/// tab visibility.
pub fn observe_sessions(cx: &mut App) {
  let store = session_store(cx);
  cx.subscribe(
    &store,
    |_, event: &SessionEvent, cx: &mut App| match event {
      // A reaped session must not leave a dangling tab behind (ADR-0001:
      // root-exit closes any attached panel automatically).
      SessionEvent::Exited(id, _) | SessionEvent::Removed(id) => {
        close_terminal_panel(*id, cx);
        save_state(cx);
      }
      SessionEvent::Spawned(_) => save_state(cx),
      _ => {}
    },
  )
  .detach();
}

/// Subscribes the workspace to settings changes that affect the global theme.
pub fn observe_settings(cx: &mut App) {
  let store = settings_store(cx);
  cx.subscribe(
    &store,
    |_, _event: &crate::stores::settings::SettingsEvent, cx: &mut App| {
      let config = settings_store(cx).read(cx).config().clone();
      let mode = match config.theme.mode {
        recoil_core::config::ThemeMode::Light => ThemeMode::Light,
        recoil_core::config::ThemeMode::Dark => ThemeMode::Dark,
        recoil_core::config::ThemeMode::System => ThemeMode::from(cx.window_appearance()),
      };
      woocraft::Theme::set_mode(mode, cx);
    },
  )
  .detach();
}

fn close_terminal_panel(id: recoil_core::session::SessionId, cx: &mut App) {
  let panel_id = format!("terminal:{id}");
  let Some(window) = cx.active_window() else {
    return;
  };
  window
    .update(cx, |_, window, cx| {
      if let Some(area) = active_dock_area(cx) {
        area.update(cx, |area, cx| {
          area.close_panel_by_id(&panel_id, window, cx);
        });
      }
    })
    .ok();
}
