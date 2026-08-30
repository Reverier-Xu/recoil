//! Dynamic session observation from OS process state (Linux).
//!
//! The application never intrudes on user operations: it reads the process
//! tree below a session's shell and derives what the user is doing right
//! now —
//!
//! - **ssh state**: an `ssh` process among the descendants means the session is
//!   connected to the host parsed from its command line;
//! - **working directory**: the foreground process's `/proc/<pid>/cwd` (shells
//!   update it on `cd`); with ssh, the remote cwd comes from the remote shell's
//!   title (`user@host: path`, iTerm2-style) until OSC 7 passes through the ssh
//!   channel in G3;
//! - **foreground process**: the process group reported by the tty (`tpgid`),
//!   tmux `automatic-rename` style — so `fish` stays `fish` even when
//!   background helpers (atuin daemons) hang off the shell.
//!
//! Scans are event-driven: they run once at spawn and on every title change.
//! While an ssh connection is active, each scan schedules one delayed
//! follow-up check (a remote disconnect produces no local events); when ssh
//! ends, no timer remains. There is no periodic scanning of local sessions.

use std::path::PathBuf;

use gpui::Context;
use recoil_core::session::SshObservation;

#[cfg(target_os = "linux")]
use crate::stores::sessions::SSH_LIVENESS_INTERVAL;
use crate::stores::sessions::{SessionId, SessionStore};

/// A snapshot of one descendant process of a session's shell.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct ProcessSnapshot {
  /// The process name from `/proc/<pid>/comm` (e.g. `ssh`, `fish`).
  comm: String,
  /// The command line arguments, excluding `argv[0]`.
  args: Vec<String>,
  /// The current working directory, when readable.
  cwd: Option<PathBuf>,
  /// The process group id.
  pgrp: i32,
}

/// One round of observation over a live session.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Observation {
  cwd: Option<PathBuf>,
  ssh: Option<SshObservation>,
  /// The foreground process name (`process - cwd` label).
  process: Option<String>,
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
  let pgrp = read_pgrp(pid)?;
  Some(ProcessSnapshot {
    comm: comm.trim().to_owned(),
    args,
    cwd,
    pgrp,
  })
}

/// Splits `/proc/<pid>/stat` after the comm field, which may itself contain
/// parentheses.
#[cfg(target_os = "linux")]
fn stat_fields(pid: u32) -> Option<Vec<String>> {
  let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
  let after_comm = stat.rsplit_once(')')?.1;
  Some(after_comm.split_whitespace().map(str::to_owned).collect())
}

/// Reads the process group id from `/proc/<pid>/stat` (field 5).
#[cfg(target_os = "linux")]
fn read_pgrp(pid: u32) -> Option<i32> {
  stat_fields(pid)?.get(2)?.parse::<i32>().ok()
}

/// Reads the tty foreground process group from `/proc/<pid>/stat` (field 8).
#[cfg(target_os = "linux")]
fn read_tpgid(pid: u32) -> Option<i32> {
  stat_fields(pid)?.get(5)?.parse::<i32>().ok()
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
    if let Some(ssh) = ssh {
      // The remote cwd comes from the remote shell's title; the remote
      // foreground process is not observable until OSC 133 (G3).
      return Observation {
        cwd: title.and_then(remote_cwd_from_title),
        ssh: Some(ssh),
        process: Some("ssh".to_owned()),
      };
    }

    // Local: the tty foreground process group picks the foreground command
    // (a running `vim`), and the shell itself when the prompt is idle. This
    // is how tmux names panes and it never confuses background helpers
    // (atuin daemons) with the foreground.
    let root = read_snapshot(root_pid);
    let foreground = match read_tpgid(root_pid) {
      Some(tpgid) => snapshots.iter().find(|snapshot| snapshot.pgrp == tpgid),
      None => None,
    };
    let current = foreground.or(root.as_ref());
    Observation {
      cwd: current.and_then(|snapshot| snapshot.cwd.clone()),
      ssh: None,
      process: current.map(|snapshot| snapshot.comm.clone()),
    }
  }
  #[cfg(not(target_os = "linux"))]
  {
    let _ = (root_pid, title);
    Observation {
      cwd: None,
      ssh: None,
      process: None,
    }
  }
}

/// Starts the event-driven observation loop for a session. Linux only for
/// now; other platforms gain observations through G3's terminal-behavior
/// sources. The loop ends by itself when the session leaves the registry.
pub fn start(id: SessionId, store: &mut SessionStore, cx: &mut Context<SessionStore>) {
  #[cfg(not(target_os = "linux"))]
  {
    let _ = (id, store, cx);
  }
  #[cfg(target_os = "linux")]
  {
    let (tx, rx) = async_channel::unbounded::<()>();
    let _ = tx.try_send(()); // initial scan: settle the label right away
    let trigger = tx.clone();

    let scan = cx.spawn(async move |this, cx| {
      loop {
        if rx.recv().await.is_err() {
          return; // trigger channel closed: the session is gone
        }
        while rx.try_recv().is_ok() {} // coalesce bursts
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
        let ssh_active = observed.ssh.is_some();
        let Ok(()) = this.update(cx, |store, cx| {
          match &observed.ssh {
            Some(ssh) => store.observe_ssh(id, ssh.host.clone(), ssh.profile_id.clone(), cx),
            None => store.observe_leave_ssh(id, cx),
          }
          if let Some(cwd) = &observed.cwd {
            store.observe_cwd(id, cwd.clone(), cx);
          }
          if let Some(process) = &observed.process {
            store.observe_process(id, process.clone(), cx);
          }
        }) else {
          return;
        };

        // While ssh is active, a remote disconnect produces no local events,
        // so schedule exactly one follow-up check; the next scan extends the
        // chain only if ssh is still active.
        if ssh_active {
          let tx = tx.clone();
          let executor = cx.background_executor().clone();
          let timer_executor = executor.clone();
          executor
            .spawn(async move {
              timer_executor.timer(SSH_LIVENESS_INTERVAL).await;
              let _ = tx.try_send(());
            })
            .detach();
        }
      }
    });

    store.set_observer(id, vec![scan], trigger);
  }
}
