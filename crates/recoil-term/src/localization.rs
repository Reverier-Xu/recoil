//! Localization over the woocraft i18n scheme.
//!
//! Per ADR-0003 every user-facing string is an i18n key with complete
//! tier-0 translations. Locale management is owned by woocraft
//! (`woocraft::init` detects the system locale; `woocraft::set_locale`
//! normalizes names like `zh-CN` into `zh-hans` and applies them to the
//! shared rust-i18n global this crate also reads). Application strings live
//! in `locales/en-us.toml` and `locales/zh-hans.toml`.

use gpui::App;
use recoil_core::session::SessionMeta;

rust_i18n::i18n!("locales", fallback = "en-us");

/// The translation macro re-exported for the rest of the crate.
pub use rust_i18n::t;

/// Logs the effective locale. Selection itself is `woocraft::init`'s job.
pub fn init(_cx: &mut App) {
  let locale = woocraft::locale().to_string();
  tracing::info!(locale, "locale initialized");
}

/// The display label for a session, translated for its kind.
///
/// Format per the product contract: `process - last cwd segment`
/// (e.g. `fish - recoil`, `ssh - projects`). Falls back to the OSC title
/// when nothing has been observed yet (e.g. non-Linux platforms), then to
/// the session kind.
pub fn session_label(meta: &SessionMeta) -> String {
  let process = if meta.ssh().is_some() {
    Some("ssh".to_owned())
  } else {
    meta.observation.shell.clone()
  };
  let cwd_last = meta.observation.cwd_last_segment();

  match (process, cwd_last) {
    (Some(process), Some(segment)) if process != segment => format!("{process} - {segment}"),
    (Some(process), Some(segment)) if meta.ssh().is_some() => {
      // "ssh - ssh" would be ambiguous; the host identifies the session.
      format!(
        "{process} - {}",
        meta.ssh().map(|ssh| ssh.host.clone()).unwrap_or(segment)
      )
    }
    (Some(process), _) if meta.ssh().is_some() => {
      format!(
        "{process} - {}",
        meta.ssh().map(|ssh| ssh.host.clone()).unwrap_or_default()
      )
    }
    (Some(process), _) => process,
    (None, Some(segment)) => segment,
    (None, None) => meta
      .title
      .clone()
      .unwrap_or_else(|| t!("session.kind.local").to_string()),
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
    // No observations yet: fall back to the kind label.
    assert_eq!(session_label(&meta), "Local");

    // The product format: process name - last cwd segment.
    meta.observation.shell = Some("fish".to_owned());
    meta.observation.cwd = Some("/home/u/recoil".into());
    assert_eq!(session_label(&meta), "fish - recoil");

    // Entering ssh renames the session; the local cwd observation is
    // cleared, and the remote cwd comes from the remote shell's title.
    meta.observation.ssh = Some(recoil_core::session::SshObservation {
      host: "build.internal".to_owned(),
      user: None,
      profile_id: None,
    });
    meta.observation.shell = Some("ssh".to_owned());
    meta.observation.cwd = None;
    assert_eq!(session_label(&meta), "ssh - build.internal");
    meta.observation.cwd = Some("~/projects".into());
    assert_eq!(session_label(&meta), "ssh - projects");

    assert_eq!(
      t!("panels.sessions.new_terminal").to_string(),
      "New Terminal"
    );
    assert_eq!(t!("tray.open", label = "host1").to_string(), "Open host1");

    woocraft::set_locale("zh-hans");
    assert_eq!(t!("panels.sessions.new_terminal").to_string(), "新建终端");
    assert_eq!(t!("tray.open", label = "host1").to_string(), "打开 host1");

    woocraft::set_locale("en-us");
  }
}
