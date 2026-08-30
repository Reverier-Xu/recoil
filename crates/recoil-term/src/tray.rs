//! System tray integration.
//!
//! The tray mirrors the session store: a show/hide entry, a new-terminal
//! entry, one entry per live session (raise it), and quit. Menu clicks are
//! polled at a fixed cadence from the main executor; G6 revisits latency and
//! dynamic menus in depth.

use gpui::{App, AppContext as _, Entity, Global};
use woocraft::{Tray, TrayAppContext as _, TrayEvent, TrayMenuItem, tray_events};

use crate::{
  localization::t,
  stores::sessions::{SessionEvent, session_store},
  workspace::{NewTerminal, QuitRecoil, save_layout},
};

/// How often the tray event queue is polled from the main executor.
const TRAY_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

struct GlobalTrayService(#[allow(dead_code)] Entity<TrayService>);

impl Global for GlobalTrayService {}

/// Owns the tray configuration and its event loop.
pub struct TrayService {
  _private: (),
}

/// Initializes the tray service on platforms that support it.
pub fn init(cx: &mut App) {
  let service = cx.new(|_| TrayService { _private: () });
  let weak = service.downgrade();
  cx.set_global(GlobalTrayService(service));

  rebuild(cx);

  let Some(events) = tray_events(cx) else {
    tracing::warn!("tray events unavailable despite successful setup");
    return;
  };
  cx.spawn(async move |cx| {
    loop {
      cx.background_executor().timer(TRAY_POLL_INTERVAL).await;
      while let Ok(event) = events.try_recv() {
        let Ok(()) = weak.update(cx, |_, cx| handle_tray_event(event.clone(), cx)) else {
          return;
        };
      }
    }
  })
  .detach();
}

fn handle_tray_event(event: TrayEvent, cx: &mut App) {
  let TrayEvent::MenuClicked { id } = event else {
    return;
  };
  match id.as_str() {
    "show" | "hide" => toggle_window(cx),
    "new-terminal" => cx.dispatch_action(&NewTerminal),
    "quit" => cx.dispatch_action(&QuitRecoil),
    id if id.starts_with("session:") => {
      if let Ok(session_id) = id
        .trim_start_matches("session:")
        .parse::<recoil_core::session::SessionId>()
      {
        raise_session(session_id, cx);
      }
    }
    _ => {}
  }
}

fn toggle_window(cx: &mut App) {
  let Some(window) = cx.active_window() else {
    return;
  };
  window
    .update(cx, |_, window, _| {
      if window.is_window_active() {
        window.minimize_window();
      } else {
        window.activate_window();
      }
    })
    .ok();
}

/// Raises a session: activates its panel when a view exists, otherwise
/// restores one into the main area by dispatching the workspace action flow.
fn raise_session(id: recoil_core::session::SessionId, cx: &mut App) {
  let store = session_store(cx);
  let alive = store
    .read(cx)
    .entry(id)
    .map(recoil_core::session::SessionEntry::is_alive)
    .unwrap_or(false);
  if !alive {
    return;
  }
  let Some(window) = cx.active_window() else {
    return;
  };
  window
    .update(cx, |_, window, cx| {
      window.activate_window();
      if let Some(dock_area) = crate::workspace::active_dock_area(cx) {
        let panel_id = format!("terminal:{id}");
        let activated = dock_area.update(cx, |area, cx| {
          area.activate_panel_by_id(&panel_id, window, cx)
        });
        if !activated {
          crate::terminal::panel::open_local_terminal(&dock_area, window, cx);
        }
      }
    })
    .ok();
}

/// Rebuilds the tray menu from the session store.
pub fn rebuild(cx: &mut App) {
  let store = session_store(cx);
  let entries: Vec<_> = store
    .read(cx)
    .entries()
    .filter(|entry| entry.is_alive())
    .map(|entry| {
      TrayMenuItem::action(
        format!("session:{}", entry.meta.id),
        t!(
          "tray.open",
          label = crate::localization::session_label(&entry.meta)
        )
        .to_string(),
      )
    })
    .collect();

  let menu = vec![
    TrayMenuItem::action("new-terminal", t!("tray.new_terminal").to_string()),
    TrayMenuItem::action("show", t!("tray.show_hide").to_string()),
    TrayMenuItem::separator(),
    TrayMenuItem::submenu(t!("tray.sessions").to_string(), entries),
    TrayMenuItem::separator(),
    TrayMenuItem::action("quit", t!("tray.quit").to_string()),
  ];

  if let Err(err) = cx.set_tray(Tray::new().tooltip("Recoil").menu(menu)) {
    tracing::warn!(error = %err, "failed to configure tray");
  }
}

/// Refreshes the tray when the session set or titles change.
pub fn observe_sessions(cx: &mut App) {
  let store = session_store(cx);
  cx.subscribe(
    &store,
    |_, event: &SessionEvent, cx: &mut App| match event {
      SessionEvent::Spawned(_)
      | SessionEvent::Exited(..)
      | SessionEvent::Removed(_)
      | SessionEvent::MetaChanged(_) => {
        rebuild(cx);
        if matches!(event, SessionEvent::Exited(..) | SessionEvent::Removed(_)) {
          save_layout(cx);
        }
      }
      SessionEvent::StateChanged(_) => {
        // Background/foreground changes do not alter the menu set.
      }
    },
  )
  .detach();
}
