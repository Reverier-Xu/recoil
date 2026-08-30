//! The settings store: hot-reloadable user configuration.
//!
//! A background filesystem watcher (the `notify` crate) watches the
//! configuration directory and sends change notifications into the GPUI
//! event loop. Reloads are debounced and writes from this process are
//! suppressed so that saving settings from the UI does not trigger a
//! redundant reload.

use std::{
  path::{Path, PathBuf},
  time::Duration,
};

use async_channel::Sender;
use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task, WeakEntity};
use notify::Watcher as _;
pub use recoil_core::config::Config;

/// Events emitted by the settings store.
#[derive(Debug, Clone)]
pub enum SettingsEvent {
  /// The effective configuration changed.
  Changed,
}

impl EventEmitter<SettingsEvent> for SettingsStore {}

/// GPUI global pointing at the settings store entity.
pub struct GlobalSettingsStore(Entity<SettingsStore>);

impl Global for GlobalSettingsStore {}

/// The application settings store.
pub struct SettingsStore {
  file: SettingsFile,
  #[allow(dead_code)]
  watcher_task: Task<()>,
  pending_write: Option<Task<()>>,
  weak: WeakEntity<Self>,
}

/// Non-GPUI file owner: path, cached config, and write-suppression state.
struct SettingsFile {
  path: PathBuf,
  config: Config,
  last_write: Option<std::time::Instant>,
}

/// How long to wait after the last edit before writing the config file back
/// to disk (debounce for the settings UI).
const WRITE_DEBOUNCE: Duration = Duration::from_millis(500);

/// How long after a process write to ignore filesystem notifications so that
/// the watcher does not reload a file this process just saved.
const WRITE_SUPPRESS: Duration = Duration::from_millis(700);

/// How long to wait after a filesystem notification before reloading, so that
/// rapid bursts of editor save events collapse into a single reload.
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(100);

/// Returns the configuration directory (`<config dir>/recoil`).
pub fn config_dir() -> Option<PathBuf> {
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

/// Returns the path to the user configuration file.
pub fn config_file() -> Option<PathBuf> {
  config_dir().map(|dir| dir.join("config.toml"))
}

/// Initializes the global settings store.
pub fn init(cx: &mut App) {
  let Some(path) = config_file() else {
    tracing::warn!("no configuration directory available; using default settings");
    let file = SettingsFile::in_memory(Config::default());
    let store = cx.new(|cx| SettingsStore {
      file,
      watcher_task: Task::ready(()),
      pending_write: None,
      weak: cx.entity().downgrade(),
    });
    cx.set_global(GlobalSettingsStore(store));
    return;
  };

  let file = SettingsFile::load_or_default(&path);
  let (sender, receiver) = async_channel::bounded(16);
  start_watcher_thread(path.clone(), sender);

  let store = cx.new(|cx| {
    let mut store = SettingsStore {
      file,
      watcher_task: Task::ready(()),
      pending_write: None,
      weak: cx.entity().downgrade(),
    };
    let weak = store.weak.clone();
    store.watcher_task = cx.spawn(async move |_this, cx| {
      while let Ok(()) = receiver.recv().await {
        cx.background_executor().timer(RELOAD_DEBOUNCE).await;
        while receiver.try_recv().is_ok() {}
        let _ = weak.update(cx, |store, cx| {
          let _ = store.reload(cx);
        });
      }
    });
    store
  });
  cx.set_global(GlobalSettingsStore(store));
}

/// Returns the global settings store, if initialized.
pub fn try_settings_store(cx: &App) -> Option<Entity<SettingsStore>> {
  cx.try_global::<GlobalSettingsStore>()
    .map(|global| global.0.clone())
}

/// Returns the global settings store.
pub fn settings_store(cx: &mut App) -> Entity<SettingsStore> {
  if let Some(global) = cx.try_global::<GlobalSettingsStore>() {
    return global.0.clone();
  }
  init(cx);
  cx.try_global::<GlobalSettingsStore>()
    .map(|global| global.0.clone())
    .expect("settings store initialized")
}

impl SettingsStore {
  /// The currently effective configuration.
  pub fn config(&self) -> &Config {
    self.file.config()
  }

  /// The path to the configuration file.
  pub fn path(&self) -> &Path {
    self.file.path()
  }

  /// Applies a validated mutation to the configuration, persists it after a
  /// debounce, and emits [`SettingsEvent::Changed`].
  pub fn update_config(
    &mut self, f: impl FnOnce(&mut Config), cx: &mut Context<Self>,
  ) -> Result<(), recoil_core::error::Error> {
    self.file.update(f)?;
    cx.emit(SettingsEvent::Changed);
    self.schedule_write(cx);
    Ok(())
  }

  /// Reloads the configuration from disk and emits
  /// [`SettingsEvent::Changed`] when it actually changed. Process writes are
  /// suppressed for a short window.
  pub fn reload(&mut self, cx: &mut Context<Self>) -> Result<(), recoil_core::error::Error> {
    let changed = self.file.reload()?;
    if changed {
      cx.emit(SettingsEvent::Changed);
    }
    Ok(())
  }

  fn schedule_write(&mut self, cx: &mut Context<Self>) {
    let weak = self.weak.clone();
    self.pending_write = Some(cx.spawn(async move |_this, cx| {
      cx.background_executor().timer(WRITE_DEBOUNCE).await;
      let _ = weak.update(cx, |store, _cx| {
        if let Err(err) = store.file.write() {
          tracing::warn!(error = %err, path = %store.file.path().display(), "failed to write settings");
        }
      });
    }));
  }
}

impl SettingsFile {
  fn in_memory(config: Config) -> Self {
    Self {
      path: PathBuf::new(),
      config,
      last_write: None,
    }
  }

  fn load_or_default(path: &Path) -> Self {
    if let Some(parent) = path.parent()
      && let Err(err) = std::fs::create_dir_all(parent)
    {
      tracing::warn!(error = %err, path = %parent.display(), "failed to create config directory");
    }
    match Config::load(&path.to_path_buf()) {
      Ok(config) => Self {
        path: path.to_path_buf(),
        config,
        last_write: None,
      },
      Err(err) => {
        tracing::warn!(error = %err, path = %path.display(), "failed to load config; writing defaults");
        let config = Config::default();
        let mut file = Self {
          path: path.to_path_buf(),
          config,
          last_write: None,
        };
        if let Err(err) = file.write() {
          tracing::warn!(error = %err, path = %path.display(), "failed to write default config");
        }
        file
      }
    }
  }

  fn config(&self) -> &Config {
    &self.config
  }

  fn path(&self) -> &Path {
    &self.path
  }

  fn reload(&mut self) -> Result<bool, recoil_core::error::Error> {
    if self
      .last_write
      .is_some_and(|t| t.elapsed() < WRITE_SUPPRESS)
    {
      return Ok(false);
    }
    let loaded = Config::load(&self.path.to_path_buf())?;
    let changed = loaded != self.config;
    self.config = loaded;
    Ok(changed)
  }

  fn update(&mut self, f: impl FnOnce(&mut Config)) -> Result<(), recoil_core::error::Error> {
    let mut config = self.config.clone();
    f(&mut config);
    config.validate()?;
    self.config = config;
    Ok(())
  }

  fn write(&mut self) -> Result<(), recoil_core::error::Error> {
    if self.path.as_os_str().is_empty() {
      return Ok(());
    }
    let tmp = self.path.with_extension("toml.tmp");
    let raw =
      toml::to_string_pretty(&self.config).map_err(recoil_core::error::Error::TomlSerialize)?;
    std::fs::write(&tmp, raw).map_err(recoil_core::error::Error::Io)?;
    std::fs::rename(&tmp, &self.path).map_err(recoil_core::error::Error::Io)?;
    self.last_write = Some(std::time::Instant::now());
    Ok(())
  }
}

fn start_watcher_thread(path: PathBuf, sender: Sender<()>) {
  std::thread::spawn(move || {
    let parent = path
      .parent()
      .map(Path::to_path_buf)
      .unwrap_or_else(|| PathBuf::from("."));
    let filename = path
      .file_name()
      .map(|s| s.to_os_string())
      .unwrap_or_default();
    let watcher = notify::RecommendedWatcher::new(
      move |res: Result<notify::Event, notify::Error>| {
        let Ok(event) = res else { return };
        let is_target = event.paths.iter().any(|p| p.file_name() == Some(&filename));
        if !is_target {
          return;
        }
        match event.kind {
          notify::EventKind::Create(_)
          | notify::EventKind::Modify(_)
          | notify::EventKind::Remove(_) => {
            let _ = sender.try_send(());
          }
          _ => {}
        }
      },
      notify::Config::default(),
    );
    let mut watcher = match watcher {
      Ok(watcher) => watcher,
      Err(err) => {
        tracing::error!(error = %err, "failed to create config file watcher");
        return;
      }
    };
    if let Err(err) = watcher.watch(&parent, notify::RecursiveMode::NonRecursive) {
      tracing::error!(error = %err, "failed to watch config directory");
    }
    loop {
      std::thread::park();
    }
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  fn temp_config_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
      "recoil-settings-tests-{}-{}",
      name,
      std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join("config.toml")
  }

  fn clean(path: &Path) {
    let _ = std::fs::remove_dir_all(path.parent().unwrap_or(path));
  }

  #[test]
  fn load_or_default_creates_default_config_when_missing() {
    let path = temp_config_path("default");
    clean(&path);
    let file = SettingsFile::load_or_default(&path);
    assert!(path.exists());
    assert_eq!(file.config().terminal.font_size, 16.0);
    assert_eq!(file.config().terminal.scrolling_history, 10_000);
    clean(&path);
  }

  #[test]
  fn update_changes_config_and_can_be_reloaded() {
    let path = temp_config_path("update");
    clean(&path);
    let mut file = SettingsFile::load_or_default(&path);
    file
      .update(|config| config.terminal.font_size = 24.0)
      .expect("update");
    file.write().expect("write");

    let mut reloaded = SettingsFile::load_or_default(&path);
    reloaded.reload().expect("reload");
    assert_eq!(reloaded.config().terminal.font_size, 24.0);
    clean(&path);
  }

  #[test]
  fn reload_suppresses_changes_right_after_a_write() {
    let path = temp_config_path("suppress");
    clean(&path);
    let mut file = SettingsFile::load_or_default(&path);
    file
      .update(|config| config.terminal.font_size = 32.0)
      .expect("update");
    file.write().expect("write");

    // Overwrite the file externally right away; our own write suppression
    // window should make the reload a no-op.
    std::fs::write(&path, "[terminal]\nfont-size = 8.0\n").expect("external write");
    let changed = file.reload().expect("reload");
    assert!(
      !changed,
      "reload should be suppressed right after a process write"
    );
    assert_eq!(file.config().terminal.font_size, 32.0);
    clean(&path);
  }

  #[test]
  fn update_rejects_invalid_config() {
    let path = temp_config_path("invalid");
    clean(&path);
    let mut file = SettingsFile::load_or_default(&path);
    let result = file.update(|config| config.terminal.font_size = 1.0);
    assert!(
      result.is_err(),
      "font-size below minimum should be rejected"
    );
    assert_eq!(file.config().terminal.font_size, 16.0);
    clean(&path);
  }

  #[test]
  fn unlimited_scrollback_round_trips() {
    let path = temp_config_path("unlimited");
    clean(&path);
    let mut file = SettingsFile::load_or_default(&path);
    file
      .update(|config| {
        config.terminal.scrolling_history = recoil_core::config::UNLIMITED_SCROLLING_HISTORY
      })
      .expect("update");
    file.write().expect("write");

    let mut reloaded = SettingsFile::load_or_default(&path);
    reloaded.reload().expect("reload");
    assert_eq!(
      reloaded.config().terminal.scrolling_history,
      recoil_core::config::UNLIMITED_SCROLLING_HISTORY
    );
    clean(&path);
  }
}
