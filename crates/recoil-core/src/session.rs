//! Session identity, metadata, and the lifecycle state machine (ADR-0001).
//!
//! This module is deliberately GPUI-free: the transitions are pure functions
//! over [`SessionEntry`] so the close-path semantics can be exhaustively
//! tested headlessly. The GPUI-side store in `recoil-term` owns the
//! `TerminalSession` handles and applies these transitions.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ulid::Ulid;

/// A stable identifier for one terminal session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SessionId(Ulid);

impl SessionId {
  /// Generates a fresh identifier.
  pub fn generate() -> Self {
    Self(Ulid::new())
  }

  /// Returns the identifier as its canonical string form.
  pub fn as_str(&self) -> String {
    self.0.to_string()
  }
}

impl std::fmt::Display for SessionId {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self.0)
  }
}

impl std::str::FromStr for SessionId {
  type Err = ulid::DecodeError;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    Ulid::from_string(s).map(Self)
  }
}

/// How a session was born. Fixed for the lifetime of the session; used for
/// reopen suggestions after restart. Contrast with [`SessionObservation`],
/// which follows what the user actually does inside the terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SessionOrigin {
  /// Spawned as a local shell.
  Local,
  /// Spawned through an ssh profile (G4).
  SshProfile { profile_id: String },
}

/// The observed ssh connection inside a session.
///
/// A local shell can enter ssh at any time and an ssh session can exit back
/// to a local shell, so this is an observation that changes with terminal
/// behavior, never a fixed attribute (ADR-0001).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshObservation {
  pub host: String,
  pub user: Option<String>,
  /// The profile the session was opened from, when applicable.
  pub profile_id: Option<String>,
}

/// Option letters that consume the following argument when parsing an ssh
/// command line, so the first non-option argument is the destination host.
const SSH_OPTIONS_WITH_VALUE: &[&str] = &[
  "b", "c", "D", "e", "F", "I", "i", "J", "L", "m", "O", "o", "p", "Q", "R", "W", "w",
];

/// Parses the destination host and user from an `ssh` command line.
///
/// `args` excludes `argv[0]`. Returns `None` when the arguments contain no
/// plausible destination (e.g. `ssh -V`). This is best-effort observation:
/// remote commands like `ssh host dmidecode` still yield `host` because the
/// destination comes first, and the application never acts on the command
/// beyond recording where the session is connected.
pub fn parse_ssh_command(args: &[String]) -> Option<SshObservation> {
  let mut destination = None;
  let mut skip_next = false;

  for arg in args {
    if skip_next {
      skip_next = false;
      continue;
    }
    if let Some(stripped) = arg.strip_prefix('-') {
      // `-pvalue` and `--option=value` forms consume no extra argument;
      // `-p value` and long options with values do.
      if !stripped.starts_with('-') && SSH_OPTIONS_WITH_VALUE.contains(&stripped) && arg.len() == 2
      {
        skip_next = true;
      }
      continue;
    }
    destination = Some(arg.clone());
    break;
  }

  let destination = destination?;
  let observation = match destination.split_once('@') {
    Some((user, host)) if !host.is_empty() => SshObservation {
      host: host.to_owned(),
      user: Some(user.to_owned()),
      profile_id: None,
    },
    _ if destination.is_empty() => return None,
    _ => SshObservation {
      host: destination,
      user: None,
      profile_id: None,
    },
  };
  Some(observation)
}

/// What the application currently observes about a session's inner state.
///
/// Observations are best-effort: the application never intrudes on user
/// operations, it derives cwd and ssh state from terminal behavior (OSC 7,
/// shell integration, profile spawn) and updates them as they change.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SessionObservation {
  /// The current working directory, when the terminal reports one. For ssh
  /// sessions this is a remote path, observed heuristically until OSC 7
  /// passes through the ssh channel (G3).
  pub cwd: Option<PathBuf>,
  /// The observed ssh connection, when the session is inside one.
  pub ssh: Option<SshObservation>,
  /// The name of the foreground process (e.g. `fish`, `vim`, `ssh`),
  /// observed from the tty foreground process group.
  pub process: Option<String>,
}

impl SessionObservation {
  /// The last path segment of the observed cwd, for display labels
  /// (`/home/u/recoil` → `recoil`, `~` → `~`).
  pub fn cwd_last_segment(&self) -> Option<String> {
    let cwd = self.cwd.as_ref()?;
    let text = cwd.to_string_lossy();
    let trimmed = text.trim_end_matches('/');
    if trimmed.is_empty() {
      return Some("/".to_owned());
    }
    trimmed
      .rsplit('/')
      .next()
      .filter(|segment| !segment.is_empty())
      .map(str::to_owned)
  }
}

/// The user-visible metadata of a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMeta {
  pub id: SessionId,
  /// How the session was born (fixed).
  pub origin: SessionOrigin,
  /// The application title (OSC 0/2), if the program set one.
  pub title: Option<String>,
  /// Dynamic observations that follow terminal behavior.
  pub observation: SessionObservation,
  /// The PTY child process id, when the session has one.
  pub pid: Option<u32>,
  pub created_at: DateTime<Utc>,
}

impl SessionMeta {
  /// Creates metadata for a newly spawned local session.
  pub fn new_local(id: SessionId, pid: Option<u32>) -> Self {
    Self {
      id,
      origin: SessionOrigin::Local,
      title: None,
      observation: SessionObservation::default(),
      pid,
      created_at: Utc::now(),
    }
  }

  /// Creates metadata for a session spawned from an ssh profile (G4).
  pub fn new_ssh(id: SessionId, profile_id: String, host: String, pid: Option<u32>) -> Self {
    Self {
      observation: SessionObservation {
        cwd: None,
        ssh: Some(SshObservation {
          host,
          user: None,
          profile_id: Some(profile_id.clone()),
        }),
        process: Some("ssh".to_owned()),
      },
      origin: SessionOrigin::SshProfile { profile_id },
      ..Self::new_local(id, pid)
    }
  }

  /// The observed ssh connection, when present.
  pub fn ssh(&self) -> Option<&SshObservation> {
    self.observation.ssh.as_ref()
  }

  /// The label used for tabs, tray menus, and tree nodes.
  pub fn label(&self) -> String {
    self.title.clone().unwrap_or_else(|| match self.ssh() {
      Some(ssh) => ssh.host.clone(),
      None => "Local".to_owned(),
    })
  }
}

/// The lifecycle state of a session (ADR-0001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
  /// The PTY is being created.
  Spawning,
  /// At least one view shows the session.
  Active,
  /// No view exists; the PTY is alive and controllable.
  Backgrounded,
  /// The root process ended or the session was killed. The entry is retained
  /// until subscribers have been notified, then reaped.
  Exited,
}

/// How the root process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitInfo {
  /// The exit code, when reported.
  pub code: Option<i32>,
  /// The terminating signal number (unix), when applicable.
  pub signal: Option<i32>,
}

/// A lifecycle transition applied to a [`SessionEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionTransition {
  /// A view attached to the session.
  Attach,
  /// The last view detached from the session.
  Detach,
  /// The user asked to terminate the session (dock tree / tray close).
  Kill,
  /// The root process exited on its own.
  RootExit(ExitInfo),
  /// The entry and its resources are dropped.
  Reap,
}

/// What applying a transition did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionOutcome {
  /// The state changed (or stayed the same); subscribers should refresh.
  Notified,
  /// The entry left the registry; subscribers must drop their references.
  Reaped,
  /// The transition changed nothing (already in the target state).
  Noop,
}

/// Errors produced by invalid lifecycle transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TransitionError {
  /// The transition is not valid from the current state (ADR-0001).
  #[error("invalid transition {by:?} from state {from:?}")]
  Invalid {
    by: SessionTransition,
    from: SessionState,
  },
  /// The entry is already gone; the transition is a harmless no-op.
  #[error("session already reaped")]
  AlreadyReaped,
}

/// The headless record of one session: metadata plus lifecycle state.
///
/// The PTY handle lives only in the application store; this type is what the
/// classification projections and persistence layers see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
  pub meta: SessionMeta,
  pub state: SessionState,
}

impl SessionEntry {
  /// Creates an entry in the [`SessionState::Spawning`] state.
  pub fn spawning(meta: SessionMeta) -> Self {
    Self {
      meta,
      state: SessionState::Spawning,
    }
  }

  /// Applies a transition per the ADR-0001 state machine.
  ///
  /// ```text
  /// Spawning ──▶ Active ──▶ Backgrounded ──▶ Active (restore)
  ///               │  ▲            │
  ///               ▼  │            ▼
  ///            Exited ◀──────────┘ (kill or root exit, from any state)
  /// ```
  pub fn transition(
    &mut self, by: SessionTransition,
  ) -> Result<TransitionOutcome, TransitionError> {
    use SessionState::*;
    match (by, self.state) {
      (SessionTransition::Attach, Spawning | Backgrounded) => {
        self.state = Active;
        Ok(TransitionOutcome::Notified)
      }
      (SessionTransition::Attach, Active) => Ok(TransitionOutcome::Noop),
      (SessionTransition::Detach, Active) => {
        self.state = Backgrounded;
        Ok(TransitionOutcome::Notified)
      }
      // Detaching a session with no view changes nothing (panel re-parenting
      // race); detaching an exited session is a harmless late cleanup.
      (SessionTransition::Detach, Spawning | Backgrounded | Exited) => Ok(TransitionOutcome::Noop),
      (SessionTransition::Kill, Spawning | Active | Backgrounded) => {
        self.state = Exited;
        Ok(TransitionOutcome::Notified)
      }
      (SessionTransition::RootExit(_), Spawning | Active | Backgrounded) => {
        self.state = Exited;
        Ok(TransitionOutcome::Notified)
      }
      // Idempotent by design: the root-exit and kill paths may both fire.
      (SessionTransition::Kill | SessionTransition::RootExit(_), Exited) => {
        Ok(TransitionOutcome::Noop)
      }
      (SessionTransition::Reap, Exited) => Ok(TransitionOutcome::Reaped),
      (SessionTransition::Reap, _) => Err(TransitionError::Invalid {
        by,
        from: self.state,
      }),
      (SessionTransition::Attach, Exited) => Err(TransitionError::Invalid { by, from: Exited }),
    }
  }

  /// Whether the session is expected to have a live PTY.
  pub fn is_alive(&self) -> bool {
    matches!(
      self.state,
      SessionState::Spawning | SessionState::Active | SessionState::Backgrounded
    )
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn entry() -> SessionEntry {
    SessionEntry::spawning(SessionMeta::new_local(SessionId::generate(), Some(4242)))
  }

  #[test]
  fn happy_path_spawn_attach_detach_restore() {
    let mut e = entry();
    assert_eq!(e.state, SessionState::Spawning);
    assert!(e.transition(SessionTransition::Attach).is_ok());
    assert_eq!(e.state, SessionState::Active);
    assert_eq!(
      e.transition(SessionTransition::Detach),
      Ok(TransitionOutcome::Notified)
    );
    assert_eq!(e.state, SessionState::Backgrounded);
    assert!(e.transition(SessionTransition::Attach).is_ok());
    assert_eq!(e.state, SessionState::Active);
    assert!(e.is_alive());
  }

  #[test]
  fn tab_close_never_exits() {
    // Detach from Spawning or Backgrounded keeps the session alive.
    let mut e = entry();
    e.transition(SessionTransition::Attach).expect("attach");
    e.transition(SessionTransition::Detach).expect("detach");
    assert!(e.is_alive(), "tab close must keep the PTY alive");
  }

  #[test]
  fn kill_from_any_live_state_exits_and_reaps() {
    for state in [
      SessionState::Spawning,
      SessionState::Active,
      SessionState::Backgrounded,
    ] {
      let mut e = entry();
      // Drive the entry into the target state.
      if state != SessionState::Spawning {
        e.transition(SessionTransition::Attach).expect("attach");
        if state == SessionState::Backgrounded {
          e.transition(SessionTransition::Detach).expect("detach");
        }
      }
      assert_eq!(
        e.transition(SessionTransition::Kill),
        Ok(TransitionOutcome::Notified)
      );
      assert_eq!(e.state, SessionState::Exited);
      assert_eq!(
        e.transition(SessionTransition::Reap),
        Ok(TransitionOutcome::Reaped)
      );
    }
  }

  #[test]
  fn root_exit_and_kill_are_idempotent() {
    let mut e = entry();
    e.transition(SessionTransition::Attach).expect("attach");
    let exit = ExitInfo {
      code: Some(0),
      signal: None,
    };
    assert_eq!(
      e.transition(SessionTransition::RootExit(exit)),
      Ok(TransitionOutcome::Notified)
    );
    // A late kill() racing the root exit must be a no-op, not an error.
    assert_eq!(
      e.transition(SessionTransition::Kill),
      Ok(TransitionOutcome::Noop)
    );
    assert_eq!(e.state, SessionState::Exited);
  }

  #[test]
  fn attach_after_exit_is_invalid() {
    let mut e = entry();
    e.transition(SessionTransition::Kill).expect("kill");
    assert_eq!(
      e.transition(SessionTransition::Attach),
      Err(TransitionError::Invalid {
        by: SessionTransition::Attach,
        from: SessionState::Exited,
      })
    );
  }

  #[test]
  fn observations_follow_terminal_behavior() {
    let mut meta = SessionMeta::new_local(SessionId::generate(), None);
    assert!(meta.ssh().is_none());
    assert!(meta.observation.cwd.is_none());

    // The user cd's somewhere: the observation updates.
    meta.observation.cwd = Some(PathBuf::from("/tmp"));
    assert_eq!(
      meta.observation.cwd.as_deref(),
      Some(std::path::Path::new("/tmp"))
    );

    // The user ssh's to a host: the observation follows.
    meta.observation.ssh = Some(SshObservation {
      host: "build.internal".to_owned(),
      user: None,
      profile_id: None,
    });
    assert_eq!(meta.label(), "build.internal");

    // The connection drops back to a local shell.
    meta.observation.ssh = None;
    assert_eq!(meta.label(), "Local");
  }

  #[test]
  fn parses_ssh_destinations_from_command_lines() {
    let args =
      |values: &[&str]| -> Vec<String> { values.iter().map(|value| value.to_string()).collect() };

    let plain = parse_ssh_command(&args(&["build.internal"])).expect("plain host");
    assert_eq!(plain.host, "build.internal");
    assert_eq!(plain.user, None);

    let with_user = parse_ssh_command(&args(&["root@10.0.0.9", "-p", "2222"])).expect("user@host");
    assert_eq!(with_user.host, "10.0.0.9");
    assert_eq!(with_user.user.as_deref(), Some("root"));

    // Options that take a value must not swallow the destination.
    let with_port = parse_ssh_command(&args(&["-p", "2222", "host"])).expect("after port");
    assert_eq!(with_port.host, "host");

    // Attached values and long options do not consume the next argument.
    let attached = parse_ssh_command(&args(&["-p2222", "host"])).expect("attached value");
    assert_eq!(attached.host, "host");

    // No destination at all.
    assert!(parse_ssh_command(&args(&["-V"])).is_none());
    assert!(parse_ssh_command(&args(&[])).is_none());
  }

  #[test]
  fn reap_of_live_session_is_invalid() {
    let mut e = entry();
    assert!(e.transition(SessionTransition::Reap).is_err());
    assert!(e.is_alive());
  }

  #[test]
  fn label_prefers_title_and_falls_back_to_kind() {
    let mut meta = SessionMeta::new_local(SessionId::generate(), None);
    assert_eq!(meta.label(), "Local");
    meta.title = Some("vim ~".to_owned());
    assert_eq!(meta.label(), "vim ~");
    let ssh = SessionMeta::new_ssh(meta.id, "p1".to_owned(), "build.internal".to_owned(), None);
    assert_eq!(ssh.label(), "build.internal");
  }
}
