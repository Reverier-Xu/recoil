//! The GPUI session store: the sole owner of `TerminalSession` handles.
//!
//! Lifecycle follows ADR-0001; the state machine itself lives in
//! `recoil-core::session`. This store adds the PTY handles, backgrounded
//! exit watchers, and the event surface for panels and the tray.

use std::{collections::HashMap, path::PathBuf, time::Duration};

use async_channel::Sender;
use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task, WeakEntity};
pub use recoil_core::session::SessionId;
use recoil_core::session::{
  ExitInfo, SessionEntry, SessionMeta, SessionState, SessionTransition, SshObservation,
  TransitionError, TransitionOutcome,
};
use woocraft_terminal::{SpawnOptions, TerminalBounds, TerminalSession};

/// How often the backgrounded-session watcher polls the child status.
///
/// Polling (instead of consuming the session event channel) keeps the store
/// from competing with terminal views for events: while a session is active
/// its view is the only consumer, and while it is backgrounded nobody needs
/// low-latency exit notification.
const WATCHER_INTERVAL: Duration = Duration::from_secs(1);

/// Events emitted by the session store.
#[derive(Debug, Clone)]
pub enum SessionEvent {
  /// A session entered the registry.
  Spawned(SessionId),
  /// A session changed state (attach, detach, kill, root exit).
  StateChanged(SessionId),
  /// Session metadata changed (title, cwd).
  MetaChanged(SessionId),
  /// The root process ended. Emitted once, right before reaping.
  Exited(SessionId, ExitInfo),
  /// The entry left the registry.
  Removed(SessionId),
}

struct Watcher(#[allow(dead_code)] Task<()>);

/// The per-session observer: a scan task fed by triggers (spawn, title
/// changes, a one-second heartbeat), plus its trigger channel.
struct Observer {
  #[allow(dead_code)]
  tasks: Vec<Task<()>>,
  trigger: Sender<()>,
}

/// The active-session registry.
pub struct SessionStore {
  entries: HashMap<SessionId, SessionEntry>,
  sessions: HashMap<SessionId, TerminalSession>,
  watchers: HashMap<SessionId, Watcher>,
  observers: HashMap<SessionId, Observer>,
  order: Vec<SessionId>,
  weak: WeakEntity<Self>,
}

impl EventEmitter<SessionEvent> for SessionStore {}

/// The GPUI global pointing at the session store entity.
pub struct GlobalSessionStore(Entity<SessionStore>);

impl Global for GlobalSessionStore {}

/// Initializes the global session store.
pub fn init(cx: &mut App) {
  let store = cx.new(|cx| SessionStore {
    entries: HashMap::new(),
    sessions: HashMap::new(),
    watchers: HashMap::new(),
    observers: HashMap::new(),
    order: Vec::new(),
    weak: cx.entity().downgrade(),
  });
  cx.set_global(GlobalSessionStore(store));
}

/// Returns the global session store, if initialized.
pub fn try_session_store(cx: &App) -> Option<Entity<SessionStore>> {
  cx.try_global::<GlobalSessionStore>()
    .map(|global| global.0.clone())
}

/// Returns the global session store.
pub fn session_store(cx: &mut App) -> Entity<SessionStore> {
  if let Some(global) = cx.try_global::<GlobalSessionStore>() {
    return global.0.clone();
  }
  init(cx);
  cx.try_global::<GlobalSessionStore>()
    .map(|global| global.0.clone())
    .expect("session store initialized")
}

impl SessionStore {
  /// Spawns a local shell session and registers it as
  /// [`SessionState::Spawning`]. `cwd` starts the shell in a specific
  /// directory (session restoration); `None` uses the inherited directory.
  pub fn spawn_local(
    &mut self, cwd: Option<PathBuf>, cx: &mut Context<Self>,
  ) -> Result<SessionId, anyhow::Error> {
    let id = SessionId::generate();
    let mut options = SpawnOptions::default_shell_options();
    options.working_directory = cwd;
    let session = TerminalSession::spawn(options, TerminalBounds::default())?;
    let meta = SessionMeta::new_local(id, session.pid());
    let pid = meta.pid;
    self.entries.insert(id, SessionEntry::spawning(meta));
    self.sessions.insert(id, session);
    self.order.push(id);
    super::observer::start(id, self, cx);
    cx.emit(SessionEvent::Spawned(id));
    tracing::info!(session = %id, pid, "spawned local session");
    Ok(id)
  }

  /// The live PTY handle for a session.
  pub fn session(&self, id: SessionId) -> Option<TerminalSession> {
    self.sessions.get(&id).cloned()
  }

  /// The headless entry for a session.
  pub fn entry(&self, id: SessionId) -> Option<&SessionEntry> {
    self.entries.get(&id)
  }

  /// Entries in creation order.
  pub fn entries(&self) -> impl Iterator<Item = &SessionEntry> + '_ {
    self.order.iter().filter_map(|id| self.entries.get(id))
  }

  /// The number of sessions that still own a PTY.
  pub fn live_count(&self) -> usize {
    self.entries.values().filter(|e| e.is_alive()).count()
  }

  /// A view attached to the session: `Spawning`/`Backgrounded` → `Active`.
  pub fn attach(&mut self, id: SessionId, cx: &mut Context<Self>) {
    self.watchers.remove(&id);
    self.apply(id, SessionTransition::Attach, cx);
  }

  /// The last view detached: `Active` → `Backgrounded`; starts the exit
  /// watcher. The PTY is untouched.
  pub fn detach(&mut self, id: SessionId, cx: &mut Context<Self>) {
    self.apply(id, SessionTransition::Detach, cx);
    if self
      .entries
      .get(&id)
      .is_some_and(|entry| entry.state == SessionState::Backgrounded)
    {
      self.start_watcher(id, cx);
    }
  }

  /// The user asked to terminate the session (dock tree / tray close),
  /// including backgrounded sessions.
  pub fn close(&mut self, id: SessionId, cx: &mut Context<Self>) {
    let Some(session) = self.sessions.get(&id) else {
      self.reap(id, cx);
      return;
    };
    session.kill();
    self.apply(id, SessionTransition::Kill, cx);
    tracing::info!(session = %id, "session closed by user");
  }

  /// Called by an attached view when the root process exited.
  pub fn root_exited(
    &mut self, id: SessionId, status: Option<woocraft_terminal::ChildStatus>,
    cx: &mut Context<Self>,
  ) {
    let exit = ExitInfo {
      code: status.map(|s| s.code()),
      #[cfg(unix)]
      signal: status.and_then(|s| s.signal),
      #[cfg(not(unix))]
      signal: None,
    };
    self.apply(id, SessionTransition::RootExit(exit), cx);
    self.finish_exit(id, exit, cx);
  }

  /// Updates the application title (OSC 0/2) metadata.
  pub fn set_title(&mut self, id: SessionId, title: Option<String>, cx: &mut Context<Self>) {
    if let Some(entry) = self.entries.get_mut(&id)
      && entry.meta.title != title
    {
      entry.meta.title = title;
      cx.emit(SessionEvent::MetaChanged(id));
    }
  }

  fn apply(
    &mut self, id: SessionId, by: SessionTransition, cx: &mut Context<Self>,
  ) -> Option<TransitionOutcome> {
    let entry = self.entries.get_mut(&id)?;
    match entry.transition(by) {
      Ok(outcome @ (TransitionOutcome::Notified | TransitionOutcome::Noop)) => {
        if outcome == TransitionOutcome::Notified {
          cx.emit(SessionEvent::StateChanged(id));
        }
        Some(outcome)
      }
      Ok(TransitionOutcome::Reaped) => {
        self.reap(id, cx);
        Some(TransitionOutcome::Reaped)
      }
      Err(TransitionError::AlreadyReaped) => None,
      Err(err @ TransitionError::Invalid { .. }) => {
        tracing::warn!(session = %id, error = %err, "rejected lifecycle transition");
        None
      }
    }
  }

  /// Emits `Exited` and reaps the entry. The PTY handle is dropped, which
  /// closes the PTY if the child is somehow still alive.
  fn finish_exit(&mut self, id: SessionId, exit: ExitInfo, cx: &mut Context<Self>) {
    self.watchers.remove(&id);
    cx.emit(SessionEvent::Exited(id, exit));
    self.reap(id, cx);
  }

  fn reap(&mut self, id: SessionId, cx: &mut Context<Self>) {
    if self.entries.remove(&id).is_none() {
      return;
    }
    self.sessions.remove(&id);
    self.watchers.remove(&id);
    self.observers.remove(&id);
    self.order.retain(|existing| *existing != id);
    cx.emit(SessionEvent::Removed(id));
    tracing::info!(session = %id, "session reaped");
  }

  /// Watches a backgrounded session for root exit. Polls the handle instead
  /// of consuming the event channel so views stay the only channel consumer.
  fn start_watcher(&mut self, id: SessionId, cx: &mut Context<Self>) {
    let Some(session) = self.sessions.get(&id).cloned() else {
      return;
    };
    let weak = self.weak.clone();
    let task = cx.spawn(async move |_this, cx| {
      loop {
        cx.background_executor().timer(WATCHER_INTERVAL).await;
        if session.is_alive() {
          continue;
        }
        let exit = ExitInfo {
          code: session.child_exit_status().map(|s| s.code()),
          #[cfg(unix)]
          signal: session.child_exit_status().and_then(|s| s.signal),
          #[cfg(not(unix))]
          signal: None,
        };
        let Ok(()) = weak.update(cx, |store, cx| {
          store.apply(id, SessionTransition::RootExit(exit), cx);
          store.finish_exit(id, exit, cx);
        }) else {
          return;
        };
        return;
      }
    });
    self.watchers.insert(id, Watcher(task));
  }

  /// Updates the observed working directory (OSC 7, G3).
  pub fn observe_cwd(&mut self, id: SessionId, cwd: PathBuf, cx: &mut Context<Self>) {
    self.observe(id, |meta| meta.observation.cwd = Some(cwd), cx);
  }

  /// Updates the observed foreground process name (tty foreground group).
  pub fn observe_process(&mut self, id: SessionId, process: String, cx: &mut Context<Self>) {
    self.observe(id, |meta| meta.observation.process = Some(process), cx);
  }

  /// Updates the observed ssh connection (process-tree observation, G4
  /// profile spawns). Crossing the locality boundary invalidates the cwd:
  /// a local path must never label a remote session and vice versa.
  pub fn observe_ssh(&mut self, id: SessionId, ssh: SshObservation, cx: &mut Context<Self>) {
    self.observe(id, |meta| meta.observation.set_ssh(Some(ssh)), cx);
  }

  /// The session left ssh (observation, not user intrusion).
  pub fn observe_leave_ssh(&mut self, id: SessionId, cx: &mut Context<Self>) {
    self.observe(id, |meta| meta.observation.set_ssh(None), cx);
  }

  fn observe(
    &mut self, id: SessionId, update: impl FnOnce(&mut SessionMeta), cx: &mut Context<Self>,
  ) {
    if let Some(entry) = self.entries.get_mut(&id) {
      let before = entry.meta.clone();
      update(&mut entry.meta);
      if entry.meta.observation != before.observation {
        cx.emit(SessionEvent::MetaChanged(id));
      }
    }
  }

  /// The root process pid of a live session, for observation scans.
  pub fn live_root_pid(&self, id: SessionId) -> Option<u32> {
    match self.entries.get(&id) {
      Some(entry) if entry.is_alive() => self.sessions.get(&id).and_then(|s| s.pid()),
      _ => None,
    }
  }

  /// The current application title of a session, for remote cwd heuristics.
  pub fn title(&self, id: SessionId) -> Option<String> {
    self
      .entries
      .get(&id)
      .and_then(|entry| entry.meta.title.clone())
  }

  /// Keeps the per-session observation tasks; they self-terminate when the
  /// entry leaves the registry and the trigger channel closes.
  pub fn set_observer(&mut self, id: SessionId, tasks: Vec<Task<()>>, trigger: Sender<()>) {
    self.observers.insert(id, Observer { tasks, trigger });
  }

  /// Requests an immediate observation scan of the session (title changed,
  /// user input, ...). Coalesced: pending triggers are drained per scan.
  pub fn trigger_scan(&self, id: SessionId) {
    if let Some(observer) = self.observers.get(&id) {
      // The channel is unbounded, so a full queue cannot happen; the drain
      // in the observer loop coalesces bursts.
      let _ = observer.trigger.try_send(());
    }
  }

  /// Whether the session is currently inside an ssh connection.
  pub fn ssh_active(&self, id: SessionId) -> Option<bool> {
    self
      .entries
      .get(&id)
      .map(|entry| entry.meta.ssh().is_some())
  }
}
