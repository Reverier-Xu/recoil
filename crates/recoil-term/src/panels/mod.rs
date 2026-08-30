//! Left dock panels: paths, history, and sessions.
//!
//! The panels are projections over the global stores; they hold no
//! authoritative state beyond transient UI state. Full classification views,
//! search, and the shared activity store arrive in G5; the sessions panel
//! here already provides the close-path affordances required by ADR-0001.

use gpui::{
  App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
  InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
  StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use woocraft::{
  ActiveTheme as _, ContextMenuExt as _, DockArea, DockPlacement, Icon, IconName, Panel,
  PanelEvent, PopupMenuItem, TableThemeExt as _, h_flex, v_flex,
};

use crate::{
  localization::{session_label, t},
  stores::sessions::{SessionEvent, SessionId, SessionStore, session_store},
};

/// Panel name for the paths panel.
pub const PATHS_PANEL: &str = "PathsPanel";
/// Panel name for the history panel.
pub const HISTORY_PANEL: &str = "HistoryPanel";
/// Panel name for the sessions panel.
pub const SESSIONS_PANEL: &str = "SessionsPanel";

// ---------------------------------------------------------------------------
// Placeholder panels (G5 replaces their bodies with store projections)
// ---------------------------------------------------------------------------

macro_rules! skeleton_panel {
  ($struct_name:ident, $panel_name:literal, $title_key:literal, $icon:expr, $hint_key:literal) => {
    struct $struct_name {
      focus_handle: FocusHandle,
    }

    impl $struct_name {
      fn new(cx: &mut Context<Self>) -> Self {
        Self {
          focus_handle: cx.focus_handle(),
        }
      }
    }

    impl Panel for $struct_name {
      fn panel_name(&self) -> &'static str {
        $panel_name
      }

      fn title(&self, _cx: &App) -> SharedString {
        t!($title_key).into()
      }

      fn tab_name(&self, cx: &App) -> Option<SharedString> {
        Some(self.title(cx))
      }

      fn icon(&self, _cx: &App) -> IconName {
        $icon
      }
    }

    impl EventEmitter<PanelEvent> for $struct_name {}

    impl Focusable for $struct_name {
      fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
      }
    }

    impl Render for $struct_name {
      fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
          .size_full()
          .p_1()
          .bg(cx.theme().background)
          .text_color(cx.theme().muted_foreground)
          .child(div().child(t!($hint_key).to_string()))
      }
    }
  };
}

skeleton_panel!(
  PathsPanel,
  "PathsPanel",
  "panels.paths.title",
  IconName::Folder,
  "panels.paths.hint"
);
skeleton_panel!(
  HistoryPanel,
  "HistoryPanel",
  "panels.history.title",
  IconName::History,
  "panels.history.hint"
);

// ---------------------------------------------------------------------------
// Sessions panel
// ---------------------------------------------------------------------------

/// Activates an existing terminal panel for the session, or creates one in
/// the main area.
fn open_session(id: SessionId, window: &mut Window, cx: &mut App) {
  let store = session_store(cx);
  let Some(entry) = store.read(cx).entry(id) else {
    return;
  };
  if !entry.is_alive() {
    return;
  }
  let Some(dock_area) = crate::workspace::active_dock_area(cx) else {
    return;
  };
  let panel_id = format!("terminal:{id}");
  let activated = dock_area.update(cx, |area, cx| {
    area.activate_panel_by_id(&panel_id, window, cx)
  });
  if !activated {
    let panel = cx.new(|cx| crate::terminal::panel::TerminalPanel::for_session(id, window, cx));
    dock_area.update(cx, |area, cx| {
      area.add_to_center(std::sync::Arc::new(panel), window, cx);
      area.activate_panel_by_id(&panel_id, window, cx);
    });
  }
}

/// The canonical icon for the session's current observation: inside ssh the
/// icon reflects the remote host; otherwise it is a local terminal.
fn kind_icon(observation: &recoil_core::session::SessionObservation) -> IconName {
  match observation.ssh {
    Some(_) => IconName::Server,
    None => IconName::Prompt,
  }
}

/// The active-session tree (minimal G1 form: a flat list with the close-path
/// affordances; time/ssh:cwd/custom-tree classifications arrive in G5).
pub struct SessionsPanel {
  focus_handle: FocusHandle,
  store: Entity<SessionStore>,
}

impl SessionsPanel {
  fn new(cx: &mut Context<Self>) -> Self {
    let store = session_store(cx);
    cx.subscribe(&store, |_, _, event: &SessionEvent, cx| match event {
      SessionEvent::Spawned(_)
      | SessionEvent::StateChanged(_)
      | SessionEvent::MetaChanged(_)
      | SessionEvent::Exited(..)
      | SessionEvent::Removed(_) => cx.notify(),
    })
    .detach();
    Self {
      focus_handle: cx.focus_handle(),
      store,
    }
  }
}

impl Panel for SessionsPanel {
  fn panel_name(&self) -> &'static str {
    SESSIONS_PANEL
  }

  fn title(&self, _cx: &App) -> SharedString {
    t!("panels.sessions.title").into()
  }

  fn tab_name(&self, cx: &App) -> Option<SharedString> {
    Some(self.title(cx))
  }

  fn icon(&self, _cx: &App) -> IconName {
    IconName::Prompt
  }
}

impl EventEmitter<PanelEvent> for SessionsPanel {}

impl Focusable for SessionsPanel {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for SessionsPanel {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme();
    let entries: Vec<_> = self
      .store
      .read(cx)
      .entries()
      .map(|entry| {
        (
          entry.meta.id,
          session_label(&entry.meta),
          entry.is_alive(),
          entry.state == recoil_core::session::SessionState::Active,
          kind_icon(&entry.meta.observation),
        )
      })
      .collect();

    let rows = entries
      .into_iter()
      .map(|(id, label, alive, attached, icon)| {
        let status_color = if alive {
          theme.success
        } else {
          theme.muted_foreground
        };

        h_flex()
          .id(SharedString::from(format!("session-{id}")))
          .p_1()
          .items_center()
          .rounded_sm()
          .cursor_pointer()
          .hover(|s| s.bg(theme.table_hover()))
          .on_click(move |_, window, cx| open_session(id, window, cx))
          // Right-click operations follow the ADR-0001 close-path semantics.
          .context_menu(move |menu, _window, _cx| {
            menu
              .item(
                PopupMenuItem::new(t!("panels.sessions.open").to_string())
                  .disabled(!alive)
                  .on_click(move |_, window, cx| open_session(id, window, cx)),
              )
              .item(
                PopupMenuItem::new(t!("panels.sessions.background").to_string())
                  .disabled(!attached)
                  .on_click(move |_, _window, cx| {
                    session_store(cx).update(cx, |store, cx| store.detach(id, cx));
                  }),
              )
              .separator()
              .item(
                PopupMenuItem::new(t!("panels.sessions.close_session").to_string())
                  .disabled(!alive)
                  .on_click(move |_, _window, cx| {
                    session_store(cx).update(cx, |store, cx| store.close(id, cx));
                  }),
              )
          })
          .child(
            // Session-kind icon with a presence dot at the icon's bottom-right.
            div()
              .relative()
              .flex_none()
              .child(Icon::new(icon).size_4().text_color(theme.muted_foreground))
              .child(
                div()
                  .absolute()
                  .bottom_0()
                  .right_0()
                  .size_1p5()
                  .rounded_full()
                  .bg(status_color)
                  .border_1()
                  .border_color(theme.background),
              ),
          )
          .child(
            div()
              .ml_2()
              .flex_1()
              .min_w_0()
              .text_color(theme.foreground)
              .text_ellipsis()
              .child(label),
          )
          .child(
            div()
              .id(SharedString::from(format!("close-{id}")))
              .mr_1()
              .cursor_pointer()
              .text_color(theme.muted_foreground)
              .hover(|s| s.text_color(theme.danger))
              .child(IconName::Dismiss)
              .on_click(move |_, _window, cx| {
                session_store(cx).update(cx, |store, cx| store.close(id, cx));
              }),
          )
      });

    v_flex()
      .size_full()
      .p_1()
      .bg(theme.background)
      .child(v_flex().gap_px().children(rows))
      // The blank-area menu lives on the spacer below the rows, not on this
      // panel root: nested context menus in woocraft both react to a right
      // click and would open two menus over a row.
      .child(
        div()
          .id("sessions-blank")
          .flex_1()
          .context_menu(|menu, _window, _cx| {
            menu.item(
              PopupMenuItem::new(t!("panels.sessions.new_terminal").to_string()).on_click(
                |_, window, cx| {
                  if let Some(dock_area) = crate::workspace::active_dock_area(cx) {
                    crate::terminal::panel::open_local_terminal(&dock_area, None, window, cx);
                  }
                },
              ),
            )
          }),
      )
  }
}

/// Builds the left dock content: three panels in tabs.
pub fn build_left_dock(dock_area: &Entity<DockArea>, window: &mut Window, cx: &mut App) {
  let paths = cx.new(PathsPanel::new);
  let history = cx.new(HistoryPanel::new);
  let sessions = cx.new(SessionsPanel::new);

  dock_area.update(cx, |area, cx| {
    area.add_to_left_dock(std::sync::Arc::new(paths), window, cx);
    area.add_to_left_dock(std::sync::Arc::new(history), window, cx);
    area.add_to_left_dock(std::sync::Arc::new(sessions), window, cx);
    area.set_dock_size(DockPlacement::Left, px(280.), window, cx);
  });
}
