//! Dynamic session observation from OS process state (Linux).
//!
//! The application never intrudes on user operations: it periodically reads
//! the process tree below a session's shell and derives what the user is
//! doing right now —
//!
//! - **ssh state**: an `ssh` process among the descendants means the session is
//!   connected to the host parsed from its command line;
//! - **working directory**: without ssh, the shell's `/proc/<pid>/cwd` is the
//!   current directory (shells update it on `cd`); with ssh, the remote cwd
//!   comes from the remote shell's title (`user@host: path`, iTerm2-style)
//!   until OSC 7 passes through the ssh channel in G3;
//! - **shell name**: the direct shell child's `comm`, tmux `automatic-rename`
//!   style.
//!
//! Other platforms keep the observation unchanged until richer sources (OSC
//! 7, shell integration) land with G3. Observations are best-effort and only
//! ever update metadata; nothing here sends input or alters the terminal.

use std::{path::PathBuf, time::Duration};

use gpui::Context;
use recoil_core::session::SshObservation;

use crate::stores::sessions::{SessionId, SessionStore};

/// How often the process tree of a live session is scanned.
const OBSERVER_INTERVAL: std::time::Duration = Duration::from_secs(2);

/// A snapshot of one descendant process of a session's shell.
struct ProcessSnapshot {
  /// The process name from `/proc/<pid>/comm` (e.g. `ssh`, `fish`).
  comm: String,
  /// The command line arguments, excluding `argv[0]`.
  args: Vec<String>,
  /// The current working directory, when readable.
  cwd: Option<PathBuf>,
}

/// One round of observation over a live session.
struct Observation {
  cwd: Option<PathBuf>,
  ssh: Option<SshObservation>,
  shell: Option<String>,
}

#[cfg(target_os = "linux")]
fn read_children(pid: u32) -> Vec<u32> {
  std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
    .map(|content| {
      content
        .split_whitespace()
        .filter_map(|token| token.parse::<u32>().ok())
        .collect()
    })
    .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn read_snapshot(pid: u32) -> Option<ProcessSnapshot> {
  let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
  let args: Vec<String> = std::fs::read_to_string(format!("/proc/{pid}/cmdline"))
    .map(|content| {
      content
        .split('\0')
        .filter(|part| !part.is_empty())
        .skip(1) // drop argv[0]
        .map(str::to_owned)
        .collect()
    })
    .unwrap_or_default();
  let cwd = std::fs::read_link(format!("/proc/{pid}/cwd"))
    .ok()
    .map(|path| path.to_path_buf());
  Some(ProcessSnapshot {
    comm: comm.trim().to_owned(),
    args,
    cwd,
  })
}

/// Collects every descendant of `root_pid` breadth-first, bounded so a
/// runaway process tree cannot turn into a runaway scan.
#[cfg(target_os = "linux")]
fn descendants(root_pid: u32) -> Vec<ProcessSnapshot> {
  const SCAN_LIMIT: usize = 256;
  let mut queue = read_children(root_pid);
  let mut snapshots = Vec::new();
  while let Some(pid) = queue.pop() {
    if snapshots.len() >= SCAN_LIMIT {
      tracing::warn!(root_pid, "process tree scan limit reached");
      break;
    }
    if let Some(snapshot) = read_snapshot(pid) {
      queue.extend(read_children(pid));
      snapshots.push(snapshot);
    }
  }
  snapshots
}

/// Extracts the remote cwd from an ssh shell title like `user@host: ~/dir`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn remote_cwd_from_title(title: &str) -> Option<PathBuf> {
  let (_user_host, path) = title.split_once(':')?;
  let path = path.trim();
  if path.is_empty() || path.contains(char::is_whitespace) {
    return None;
  }
  Some(PathBuf::from(path))
}

/// One round of observation over a live session.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn observe(root_pid: u32, title: Option<&str>) -> Observation {
  #[cfg(target_os = "linux")]
  {
    let snapshots = descendants(root_pid);
    let ssh = snapshots.iter().find_map(|snapshot| {
      if snapshot.comm == "ssh" || snapshot.comm.starts_with("ssh_") {
        recoil_core::session::parse_ssh_command(&snapshot.args)
      } else {
        None
      }
    });
    match ssh {
      Some(ssh) => {
        // The remote working directory is not observable locally; take it
        // from the remote shell's title when it reports one. The local shell
        // name no longer describes what the user sees.
        let cwd = title.and_then(remote_cwd_from_title);
        Observation {
          cwd,
          ssh: Some(ssh),
          shell: Some("ssh".to_owned()),
        }
      }
      None => {
        // The direct shell child carries the current directory and its name
        // is the shell (tmux automatic-rename style); grandchild cwds (a
        // running vim) would be misleading.
        let direct = read_children(root_pid)
          .last()
          .copied()
          .and_then(read_snapshot);
        Observation {
          cwd: direct.as_ref().and_then(|snapshot| snapshot.cwd.clone()),
          ssh: None,
          shell: direct.map(|snapshot| snapshot.comm),
        }
      }
    }
  }
  #[cfg(not(target_os = "linux"))]
  {
    let _ = (root_pid, title);
    Observation {
      cwd: None,
      ssh: None,
      shell: None,
    }
  }
}

/// Starts the per-session observation loop. Linux only for now; other
/// platforms gain observations through G3's terminal-behavior sources. The
/// loop ends by itself once the session leaves the registry.
pub fn start(id: SessionId, store: &mut SessionStore, cx: &mut Context<SessionStore>) {
  #[cfg(not(target_os = "linux"))]
  {
    let _ = (id, store, cx);
  }
  #[cfg(target_os = "linux")]
  {
    let task = cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(OBSERVER_INTERVAL).await;
        // Stop once the session is gone; skip while it has no live child.
        let Some(root_pid) = this
          .update(cx, |store, _| store.live_root_pid(id))
          .ok()
          .flatten()
        else {
          return;
        };
        let title = this.update(cx, |store, _| store.title(id)).ok().flatten();
        // Filesystem reads run on the background executor; they must never
        // block the UI thread.
        let observed = cx
          .background_executor()
          .spawn(async move { observe(root_pid, title.as_deref()) })
          .await;
        let Ok(()) = this.update(cx, |store, cx| {
          let (cwd, ssh, shell) = (&observed.cwd, &observed.ssh, &observed.shell);
          match ssh {
            Some(ssh) => store.observe_ssh(id, ssh.host.clone(), ssh.profile_id.clone(), cx),
            None => store.observe_leave_ssh(id, cx),
          }
          if let Some(cwd) = cwd {
            store.observe_cwd(id, cwd.clone(), cx);
          }
          if let Some(shell) = shell {
            store.observe_shell(id, shell.clone(), cx);
          }
        }) else {
          return;
        };
      }
    });
    store.set_observer(id, task);
  }
}
