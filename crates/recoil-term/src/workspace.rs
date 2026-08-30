//! The workspace root: dock assembly, menu, actions, and layout persistence.

use std::path::PathBuf;

use gpui::{
  App, AppContext as _, Context, Entity, Global, InteractiveElement as _, IntoElement,
  ParentElement as _, Render, Styled as _, Window, WindowOptions, actions,
};
use woocraft::{AppMenuBar, DockArea, DockItem, DockPlacement, TitleBar, v_flex, window_border};

use crate::{
  panels,
  stores::sessions::{SessionEvent, session_store},
  terminal::panel,
};

actions!(
  recoil,
  [NewTerminal, CloseActiveTab, ToggleLeftDock, QuitRecoil]
);

/// The persisted workspace state file (`state.json`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
struct WorkspaceState {
  dock_area: Option<woocraft::DockAreaState>,
}

struct GlobalActiveDockArea(Entity<DockArea>);

impl Global for GlobalActiveDockArea {}

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
      app_menu_bar,
    });

    build_layout(&dock_area, window, cx);

    // Persist layout when it changes. `LayoutChanged` fires continuously
    // during drags, so writes are debounced by a minimum interval; the
    // final state is always flushed on quit.
    cx.subscribe(
      &dock_area,
      |_, event: &woocraft::DockEvent, cx: &mut App| {
        if matches!(event, woocraft::DockEvent::LayoutChanged) {
          save_layout_debounced(cx);
        }
      },
    )
    .detach();

    cx.set_global(GlobalActiveDockArea(dock_area));
    // Persist the initial assembly: `LayoutChanged` events fired during
    // construction were seen by no subscriber yet.
    save_layout(cx);
    workspace
  }

  fn on_new_terminal(&mut self, _: &NewTerminal, window: &mut Window, cx: &mut Context<Self>) {
    panel::open_local_terminal(&self.dock_area, window, cx);
    save_layout(cx);
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
      save_layout(cx);
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
  // Restore persisted layout when available; fall back to the default
  // assembly. Terminal panels cannot be resurrected across restarts, so the
  // registered deserializer spawns a fresh local session per stored tab.
  if let Some(state) = load_layout(cx) {
    let loaded = dock_area.update(cx, |area, cx| area.load(state, window, cx));
    if loaded.is_ok() {
      return;
    }
    tracing::warn!("failed to restore the persisted layout; using the default one");
  }

  let store = session_store(cx);
  let initial = store.update(cx, |store, cx| store.spawn_local(cx));
  dock_area.update(cx, |area, cx| {
    if let Ok(id) = initial {
      let p = cx.new(|cx| panel::TerminalPanel::for_session(id, window, cx));
      area.add_to_center(std::sync::Arc::new(p), window, cx);
    }
  });
  panels::build_left_dock(dock_area, window, cx);
}

fn load_layout(_cx: &App) -> Option<woocraft::DockAreaState> {
  let file = std::fs::read_to_string(state_file()?).ok()?;
  let state: WorkspaceState = serde_json::from_str(&file).ok()?;
  state.dock_area
}

struct GlobalLastLayoutSave(std::time::Instant);

impl Global for GlobalLastLayoutSave {}

/// Minimum interval between layout writes while `LayoutChanged` storms.
const LAYOUT_SAVE_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Debounced variant used for `LayoutChanged` storms.
pub fn save_layout_debounced(cx: &mut App) {
  let now = std::time::Instant::now();
  let due = cx
    .try_global::<GlobalLastLayoutSave>()
    .is_none_or(|last| now.duration_since(last.0) >= LAYOUT_SAVE_MIN_INTERVAL);
  if due {
    cx.set_global(GlobalLastLayoutSave(now));
    save_layout(cx);
  }
}

/// Persists the dock layout. G6 adds atomic writes and crash safety.
pub fn save_layout(cx: &App) {
  let Some(path) = state_file() else {
    return;
  };
  let Some(area) = active_dock_area(cx) else {
    return;
  };
  let state = WorkspaceState {
    dock_area: Some(area.read(cx).dump(cx)),
  };
  let Ok(json) = serde_json::to_string_pretty(&state) else {
    return;
  };
  if let Err(err) = std::fs::create_dir_all(path.parent().expect("state file has a parent"))
    .and_then(|_| std::fs::write(&path, json))
  {
    tracing::warn!(error = %err, path = %path.display(), "failed to persist layout");
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
  save_layout(cx);
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
        .on_action(cx.listener(Self::on_quit))
        .child(
          TitleBar::new()
            .title("Recoil")
            .app_menu_bar(self.app_menu_bar.clone())
            .language_button(true),
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

/// Sets up the application menu.
pub fn set_app_menu(cx: &mut App) {
  use gpui::{Menu, MenuItem};
  use woocraft::gpui::SharedString;
  let label = |key: &str| SharedString::from(crate::localization::t!(key).to_string());
  cx.set_menus(vec![
    Menu {
      name: "Recoil".into(),
      disabled: false,
      items: vec![
        MenuItem::action(label("menu.new_terminal"), NewTerminal),
        MenuItem::action(label("menu.close_tab"), CloseActiveTab),
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
pub fn bind_keys(cx: &mut App) {
  use gpui::KeyBinding;
  cx.bind_keys([
    KeyBinding::new("ctrl-shift-t", NewTerminal, None),
    KeyBinding::new("cmd-t", NewTerminal, None),
    KeyBinding::new("ctrl-shift-w", CloseActiveTab, None),
    KeyBinding::new("cmd-w", CloseActiveTab, None),
    KeyBinding::new("ctrl-b", ToggleLeftDock, None),
    KeyBinding::new("cmd-b", ToggleLeftDock, None),
    KeyBinding::new("ctrl-shift-q", QuitRecoil, None),
    KeyBinding::new("cmd-q", QuitRecoil, None),
  ]);
}

/// Subscribes the workspace to store events that affect persisted state.
pub fn observe_sessions(cx: &mut App) {
  let store = session_store(cx);
  cx.subscribe(
    &store,
    |_, event: &SessionEvent, cx: &mut App| match event {
      SessionEvent::Exited(..) | SessionEvent::Removed(_) => save_layout(cx),
      _ => {}
    },
  )
  .detach();
}
