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
  let process = meta.observation.process.clone();
  let cwd_last = meta.observation.cwd_last_segment();
  let ssh = meta.ssh();

  match (process, cwd_last, ssh) {
    (Some(process), Some(segment), _) if process != segment => format!("{process} - {segment}"),
    // "ssh - ssh" would be ambiguous; the host identifies the session.
    (Some(process), _, Some(ssh)) => format!("{process} - {}", ssh.host),
    (Some(process), _, None) => process,
    (None, Some(segment), _) => segment,
    // No observations yet: fall back to the kind label. The OSC title is
    // metadata for the heuristics, never a label (it is often just a path).
    (None, None, Some(ssh)) => ssh.host.clone(),
    (None, None, None) => t!("session.kind.local").to_string(),
  }
}

#[cfg(test)]
mod tests {
  use recoil_core::session::SessionId;

  use super::*;

  /// Tier-0 locales must carry identical key sets; a key present in one and
  /// missing from the other fails the gate (ADR-0003).
  #[test]
  fn tier0_locale_key_sets_are_equal() {
    use std::collections::BTreeMap;

    fn flatten(prefix: &str, value: &toml::Value, keys: &mut Vec<String>) {
      match value {
        toml::Value::Table(table) => {
          for (key, child) in table {
            flatten(&format!("{prefix}.{key}"), child, keys);
          }
        }
        toml::Value::String(text) => {
          assert!(!text.is_empty(), "empty translation for key {prefix}");
          keys.push(prefix.trim_start_matches('.').to_owned());
        }
        _ => panic!("locale values must be strings: {prefix}"),
      }
    }

    fn keys(file: &str) -> BTreeMap<String, bool> {
      let value: toml::Value = toml::from_str(file).expect("locale file must parse");
      let mut keys = Vec::new();
      flatten("", &value, &mut keys);
      keys.into_iter().map(|key| (key, true)).collect()
    }

    let en = keys(include_str!("../locales/en-us.toml"));
    let zh = keys(include_str!("../locales/zh-hans.toml"));
    let missing_in_zh: Vec<_> = en.keys().filter(|key| !zh.contains_key(*key)).collect();
    let missing_in_en: Vec<_> = zh.keys().filter(|key| !en.contains_key(*key)).collect();
    assert!(
      missing_in_zh.is_empty() && missing_in_en.is_empty(),
      "locale key mismatch: missing in zh-hans {missing_in_zh:?}, missing in en-us {missing_in_en:?}"
    );
  }

  /// One sequential test: the locale is a process-global, so tier-0
  /// completeness and label behavior cannot run concurrently.
  #[test]
  fn tier0_locales_and_labels() {
    woocraft::set_locale("en-us");
    let mut meta = SessionMeta::new_local(SessionId::generate(), None);
    // No observations yet: fall back to the kind label, never a raw path.
    assert_eq!(session_label(&meta), "Local");
    meta.title = Some("/home/u/somewhere".to_owned());
    assert_eq!(session_label(&meta), "Local", "osc titles are never labels");

    // The product format: foreground process - last cwd segment.
    meta.observation.process = Some("fish".to_owned());
    meta.observation.cwd = Some("/home/u/recoil".into());
    assert_eq!(session_label(&meta), "fish - recoil");

    // Entering ssh: the label becomes "ssh - host" until the remote cwd
    // arrives through the remote shell's title.
    meta.observation.ssh = Some(recoil_core::session::SshObservation {
      host: "build.internal".to_owned(),
      user: None,
      profile_id: None,
    });
    meta.observation.process = Some("ssh".to_owned());
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
