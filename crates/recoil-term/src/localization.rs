//! Localization over the woocraft i18n scheme.
//!
//! Per ADR-0003 every user-facing string is an i18n key with complete
//! tier-0 translations. Locale management is owned by woocraft
//! (`woocraft::init` detects the system locale; `woocraft::set_locale`
//! normalizes names like `zh-CN` into `zh-hans` and applies them to the
//! shared rust-i18n global this crate also reads). Application strings live
//! in `locales/en-us.toml` and `locales/zh-hans.toml`.

use gpui::App;
use recoil_core::session::{SessionKind, SessionMeta};

rust_i18n::i18n!("locales", fallback = "en-us");

/// The translation macro re-exported for the rest of the crate.
pub use rust_i18n::t;

/// Logs the effective locale. Selection itself is `woocraft::init`'s job.
pub fn init(_cx: &mut App) {
  let locale = woocraft::locale().to_string();
  tracing::info!(locale, "locale initialized");
}

/// The display label for a session, translated for its kind.
pub fn session_label(meta: &SessionMeta) -> String {
  match &meta.title {
    Some(title) if !title.is_empty() => title.clone(),
    _ => match &meta.kind {
      SessionKind::Local => t!("session.kind.local").to_string(),
      SessionKind::Ssh { host, .. } => host.clone(),
    },
  }
}

#[cfg(test)]
mod tests {
  use recoil_core::session::SessionId;

  use super::*;

  /// One sequential test: the locale is a process-global, so tier-0
  /// completeness and label behavior cannot run concurrently.
  #[test]
  fn tier0_locales_and_labels() {
    woocraft::set_locale("en-us");
    let mut meta = SessionMeta::new_local(SessionId::generate(), None);
    assert_eq!(session_label(&meta), "Local");
    assert_eq!(t!("panels.sessions.title").to_string(), "Sessions");

    meta.title = Some("vim".to_owned());
    assert_eq!(session_label(&meta), "vim");
    meta.title = None;

    woocraft::set_locale("zh-hans");
    assert_eq!(session_label(&meta), "本地");
    assert_eq!(t!("panels.sessions.new_terminal").to_string(), "新建终端");
    assert_eq!(t!("tray.open", label = "host1").to_string(), "打开 host1");

    woocraft::set_locale("en-us");
  }
}
