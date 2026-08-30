//! The dock panel hosting one terminal session.

use gpui::{
  App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
  ParentElement as _, Render, SharedString, Styled as _, WeakEntity, Window,
};
use woocraft::{
  ActiveTheme as _, ContextMenuExt as _, DockArea, IconName, Panel, PanelEvent, PanelState,
  PopupMenuItem, TerminalView, TerminalViewEvent, h_flex,
};
use woocraft_terminal::TerminalSession;

use crate::{
  localization::{session_label, t},
  stores::sessions::{SessionEvent, SessionId, session_store, try_session_store},
};

/// The panel name used for serialization and registry lookup.
pub const PANEL_NAME: &str = "TerminalPanel";

/// A dock panel observing one session. The panel never owns the PTY; it is
/// created from the session store and detaches on close (ADR-0001).
pub struct TerminalPanel {
  id: SessionId,
  terminal: Entity<TerminalView>,
  focus_handle: FocusHandle,
}

impl TerminalPanel {
  /// Creates the panel for an already-spawned session and attaches it.
  pub fn for_session(id: SessionId, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let store = session_store(cx);
    let session: TerminalSession = store
      .read(cx)
      .session(id)
      .expect("session must exist in the store");
    let terminal = cx.new(|cx| TerminalView::new(session, window, cx));
    cx.subscribe(&terminal, Self::on_terminal_event).detach();
    // Observation updates (cwd, ssh, shell name) re-label the tab live.
    cx.subscribe(&store, move |this, _, event: &SessionEvent, cx| {
      if matches!(event, SessionEvent::MetaChanged(changed) if *changed == id) {
        let _ = this;
        cx.notify();
      }
    })
    .detach();
    store.update(cx, |store, cx| store.attach(id, cx));

    let focus_handle = terminal.focus_handle(cx);
    Self {
      id,
      terminal,
      focus_handle,
    }
  }

  /// Spawns a fresh local session and returns the panel for it.
  pub fn create_local(window: &mut Window, cx: &mut Context<Self>) -> Self {
    let store = session_store(cx);
    let id = store
      .update(cx, |store, cx| store.spawn_local(cx))
      .expect("spawn local session");
    Self::for_session(id, window, cx)
  }

  /// The session this panel observes.
  pub fn session_id(&self) -> SessionId {
    self.id
  }

  fn on_terminal_event(
    &mut self, _: Entity<TerminalView>, event: &TerminalViewEvent, cx: &mut Context<Self>,
  ) {
    match event {
      TerminalViewEvent::TitleChanged(title) => {
        // The title feeds the observation heuristics; the tab label is
        // derived from the observation instead.
        let id = self.id;
        session_store(cx).update(cx, |store, cx| store.set_title(id, title.clone(), cx));
      }
      TerminalViewEvent::Exit(status) => {
        let id = self.id;
        session_store(cx).update(cx, |store, cx| store.root_exited(id, *status, cx));
      }
      TerminalViewEvent::Bell | TerminalViewEvent::ClipboardStored(_) => {}
    }
  }

  /// The tab label: the live observation label (`process - cwd segment`),
  /// falling back to the session kind.
  fn label(&self, cx: &App) -> SharedString {
    let label = try_session_store(cx).and_then(|store| {
      store
        .read(cx)
        .entry(self.id)
        .map(|entry| session_label(&entry.meta))
    });
    SharedString::from(label.unwrap_or_else(|| t!("terminal.default_title").to_string()))
  }
}

impl Panel for TerminalPanel {
  fn panel_name(&self) -> &'static str {
    PANEL_NAME
  }

  fn panel_id(&self, _cx: &App) -> SharedString {
    SharedString::from(format!("terminal:{}", self.id))
  }

  fn tab_name(&self, cx: &App) -> Option<SharedString> {
    Some(self.label(cx))
  }

  fn title(&self, cx: &App) -> SharedString {
    self.label(cx)
  }

  fn icon(&self, _cx: &App) -> IconName {
    IconName::Prompt
  }

  fn dump(&self, _cx: &App) -> PanelState {
    // Persist only the panel identity: PTYs cannot cross process restarts,
    // and G5 owns honest reopen suggestions for dead sessions.
    let mut state = PanelState::new(self);
    state.info = woocraft::PanelInfo::panel(serde_json::json!({
      "session_id": self.id.as_str(),
    }));
    state
  }

  fn on_removed(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
    // Closing the tab detaches the view; the session stays alive (ADR-0001).
    let id = self.id;
    session_store(cx).update(cx, |store, cx| store.detach(id, cx));
  }
}

impl EventEmitter<PanelEvent> for TerminalPanel {}

impl Focusable for TerminalPanel {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for TerminalPanel {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let exited = session_store(cx)
      .read(cx)
      .entry(self.id)
      .map(|entry| !entry.is_alive())
      .unwrap_or(false);

    if exited {
      h_flex()
        .size_full()
        .items_center()
        .justify_center()
        .bg(cx.theme().background)
        .text_color(cx.theme().muted_foreground)
        .child(t!("terminal.exited").to_string())
        .into_any_element()
    } else {
      let terminal = self.terminal.clone();
      h_flex()
        .size_full()
        .child(self.terminal.clone())
        // Right-click operations on the terminal surface. The menu operates
        // on the view through its public API only; keystrokes and mouse
        // reporting remain the terminal program's business.
        .context_menu(move |menu, _window, cx| {
          let clipboard = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default();
          menu
            .item(
              PopupMenuItem::new(t!("terminal.menu.copy").to_string()).on_click({
                let terminal = terminal.clone();
                move |_, _window, cx| {
                  terminal.update(cx, |view, cx| {
                    view.copy(cx);
                  });
                }
              }),
            )
            .item(
              PopupMenuItem::new(t!("terminal.menu.paste").to_string())
                .disabled(clipboard.is_empty())
                .on_click({
                  let terminal = terminal.clone();
                  move |_, _window, cx| {
                    terminal.update(cx, |view, cx| view.paste(&clipboard, cx));
                  }
                }),
            )
            .item(
              PopupMenuItem::new(t!("terminal.menu.select_all").to_string()).on_click({
                let terminal = terminal.clone();
                move |_, _window, cx| {
                  terminal.update(cx, |view, cx| view.select_all(cx));
                }
              }),
            )
            .separator()
            .item(
              PopupMenuItem::new(t!("terminal.menu.clear").to_string()).on_click({
                let terminal = terminal.clone();
                move |_, _window, cx| {
                  terminal.update(cx, |view, cx| view.clear(cx));
                }
              }),
            )
        })
        .into_any_element()
    }
  }
}

/// Rebuilds a terminal panel from persisted dock state.
///
/// The stored session id is not resurrectable across restarts, so a fresh
/// local session is spawned in the same tab position. Honest reopen
/// suggestions for dead sessions are G5 behavior.
pub fn deserialize_terminal_panel(
  _dock_area: WeakEntity<DockArea>, _state: &PanelState, _info: &woocraft::PanelInfo,
  window: &mut Window, cx: &mut App,
) -> Box<dyn woocraft::PanelView> {
  let panel = cx.new(|cx| TerminalPanel::create_local(window, cx));
  Box::new(panel)
}

/// Registers all application panels with the woocraft panel registry.
pub fn register_panels(cx: &mut App) {
  woocraft::register_panel(cx, PANEL_NAME, deserialize_terminal_panel);
}

/// Convenience: spawn a local session and add its panel to the center of the
/// dock area, activating it.
pub fn open_local_terminal(
  dock_area: &Entity<woocraft::DockArea>, window: &mut Window, cx: &mut App,
) -> SessionId {
  let store = session_store(cx);
  let id = store
    .update(cx, |store, cx| store.spawn_local(cx))
    .expect("spawn local session");
  let panel = cx.new(|cx| TerminalPanel::for_session(id, window, cx));
  dock_area.update(cx, |dock_area, cx| {
    dock_area.add_to_center(std::sync::Arc::new(panel), window, cx);
    dock_area.activate_panel_by_id(&format!("terminal:{id}"), window, cx);
  });
  id
}
