//! Dynamic session observation from OS process state (Linux).
//!
//! The application never intrudes on user operations: it reads the process
//! tree below a session's shell and derives what the user is doing right
//! now —
//!
//! - **ssh state**: an `ssh` client holding the tty foreground process group
//!   means the session is connected to the host parsed from its command line
//!   (gpakosz/.tmux walks the same foreground chain). Background helpers —
//!   git-over-ssh, ControlMaster proxies, `-W` jump channels — never own the
//!   foreground and never mark a session as connected;
//! - **working directory**: the foreground process's `/proc/<pid>/cwd` (shells
//!   update it on `cd`); with ssh, the remote side reports through the shell
//!   title ssh forwards verbatim — fish's default `command: path` or the
//!   iTerm2-style `user@host: path` — until OSC 7 passes through the ssh
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
#[derive(Clone)]
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
  /// The tty foreground is not the root shell: a job is running. Jobs may
  /// be transient prompt hooks (atuin, starship precmd helpers) that hold
  /// the tty foreground for a few milliseconds, so the scan loop confirms
  /// them over a short settle window before trusting the label.
  settle: bool,
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

/// The remote state parsed from the shell title ssh forwards verbatim.
///
/// Two formats cover the common remote shells:
///
/// - fish default: `<command>: <path>` (`vim: ~/src`, idle: `fish: ~`) —
///   carries the remote foreground program and the remote cwd;
/// - iTerm2/bash style: `user@host: <path>` — identifies the connection, the
///   left side is not a program.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Default)]
struct RemoteTitle {
  /// The remote foreground program, when the title carries one.
  program: Option<String>,
  /// The remote working directory.
  cwd: Option<PathBuf>,
}

/// Parses a remote shell title into the remote program and remote cwd.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_remote_title(title: &str) -> RemoteTitle {
  let Some((left, right)) = title.split_once(':') else {
    return RemoteTitle::default();
  };
  let path = right.trim();
  let cwd = if path.is_empty() {
    None
  } else {
    Some(PathBuf::from(path))
  };
  let left = left.trim();
  let program = if left.is_empty() || left.contains('@') || left.contains(char::is_whitespace) {
    None
  } else {
    Some(left.to_owned())
  };
  RemoteTitle { program, cwd }
}

/// Picks the interactive ssh client among the processes sharing the tty
/// foreground process group, gpakosz/.tmux style: the user's own ssh is the
/// connection this session shows, while `-W` channels spawned by ssh itself
/// (jump hosts, ProxyCommand) carry no remote session and are skipped.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn foreground_ssh<'a>(foreground: &[&'a ProcessSnapshot]) -> Option<&'a ProcessSnapshot> {
  let is_ssh =
    |snapshot: &&ProcessSnapshot| snapshot.comm == "ssh" || snapshot.comm.starts_with("ssh_");
  let is_proxy_channel =
    |snapshot: &&ProcessSnapshot| snapshot.args.iter().any(|arg| arg.starts_with("-W"));
  foreground
    .iter()
    .copied()
    .find(|s| is_ssh(s) && !is_proxy_channel(s))
    .or_else(|| foreground.iter().copied().find(is_ssh))
}

/// Derives the observation from already-collected process state. Pure, so
/// the heuristics are testable without a live `/proc`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn observation_from(
  root: Option<&ProcessSnapshot>, snapshots: &[ProcessSnapshot], tpgid: Option<i32>,
  title: Option<&str>,
) -> Observation {
  let foreground: Vec<&ProcessSnapshot> = match tpgid {
    Some(tpgid) => snapshots.iter().filter(|s| s.pgrp == tpgid).collect(),
    None => Vec::new(),
  };

  // A session counts as connected only while the user sits in the ssh
  // client: the client holds the tty foreground (tmux follows the same
  // chain). While connected, the remote side is described by the remote
  // shell's title; the local cwd never leaks into the label.
  if let Some(client) = foreground_ssh(&foreground)
    && let Some(ssh) = recoil_core::session::parse_ssh_command(&client.args)
  {
    let remote = title.map(parse_remote_title).unwrap_or_default();
    return Observation {
      cwd: remote.cwd,
      ssh: Some(ssh),
      process: Some(remote.program.unwrap_or_else(|| "ssh".to_owned())),
      settle: true,
    };
  }

  // Local: the tty foreground process group picks the foreground command
  // (a running `vim`), and the shell itself when the prompt is idle. This
  // is how tmux names panes and it never confuses background helpers
  // (atuin daemons) with the foreground.
  let current = foreground.first().copied().or(root);
  Observation {
    cwd: current.and_then(|snapshot| snapshot.cwd.clone()),
    ssh: None,
    process: current.map(|snapshot| snapshot.comm.clone()),
    settle: !foreground.is_empty(),
  }
}

/// One round of observation over a live session.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn observe(root_pid: u32, title: Option<&str>) -> Observation {
  #[cfg(target_os = "linux")]
  {
    let snapshots = descendants(root_pid);
    let root = read_snapshot(root_pid);
    let tpgid = read_tpgid(root_pid);
    observation_from(root.as_ref(), &snapshots, tpgid, title)
  }
  #[cfg(not(target_os = "linux"))]
  {
    let _ = (root_pid, title);
    Observation {
      cwd: None,
      ssh: None,
      process: None,
      settle: false,
    }
  }
}

/// How long the scan loop waits before confirming a foreground job. Long
/// enough that prompt hooks (atuin, starship) have returned the tty
/// foreground to the shell; short enough that real programs (vim, ssh,
/// top) are never missed.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const FOREGROUND_SETTLE_DELAY: std::time::Duration = std::time::Duration::from_millis(150);

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
        let mut observed = cx
          .background_executor()
          .spawn(async move { observe(root_pid, title.as_deref()) })
          .await;
        if observed.settle {
          // A foreground job may be a transient prompt hook (atuin history
          // hooks, starship precmd helpers) that owns the tty foreground
          // for a few milliseconds. Confirm the job survives a short settle
          // window; the fresh rescan then describes either the stable job
          // or the shell that took the foreground back.
          cx.background_executor()
            .timer(FOREGROUND_SETTLE_DELAY)
            .await;
          let Some(root_pid) = this
            .update(cx, |store, _| store.live_root_pid(id))
            .ok()
            .flatten()
          else {
            return;
          };
          let title = this.update(cx, |store, _| store.title(id)).ok().flatten();
          observed = cx
            .background_executor()
            .spawn(async move { observe(root_pid, title.as_deref()) })
            .await;
        }
        let ssh_active = observed.ssh.is_some();
        let Ok(()) = this.update(cx, |store, cx| {
          match observed.ssh.clone() {
            Some(ssh) => store.observe_ssh(id, ssh, cx),
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

#[cfg(test)]
mod tests {
  use super::*;

  fn process(comm: &str, args: &[&str], cwd: Option<&str>, pgrp: i32) -> ProcessSnapshot {
    ProcessSnapshot {
      comm: comm.to_owned(),
      args: args.iter().map(|arg| arg.to_string()).collect(),
      cwd: cwd.map(PathBuf::from),
      pgrp,
    }
  }

  #[test]
  fn parses_remote_titles() {
    // fish's default: command and path, the remote foreground included.
    let fish = parse_remote_title("vim: ~/src");
    assert_eq!(fish.program.as_deref(), Some("vim"));
    assert_eq!(fish.cwd.as_deref(), Some(std::path::Path::new("~/src")));

    let idle = parse_remote_title("fish: ~");
    assert_eq!(idle.program.as_deref(), Some("fish"));
    assert_eq!(idle.cwd.as_deref(), Some(std::path::Path::new("~")));

    // iTerm2-style: the left side identifies the connection, not a program.
    let iterm = parse_remote_title("root@build.internal: /srv/www");
    assert_eq!(iterm.program, None);
    assert_eq!(iterm.cwd.as_deref(), Some(std::path::Path::new("/srv/www")));

    // Paths may contain whitespace; titleless and colonless input parses to
    // nothing.
    let spaced = parse_remote_title("fish: ~/my dir");
    assert_eq!(
      spaced.cwd.as_deref(),
      Some(std::path::Path::new("~/my dir"))
    );
    let empty = parse_remote_title("fish:");
    assert_eq!(empty.program.as_deref(), Some("fish"));
    assert_eq!(empty.cwd, None);
    assert!(parse_remote_title("no colon here").cwd.is_none());
  }

  #[test]
  fn ssh_detection_follows_the_tty_foreground() {
    let shell = process("fish", &[], Some("/home/u"), 100);
    let client = process("ssh", &["build.internal"], Some("/home/u"), 200);
    let snapshots = vec![client.clone()];

    // ssh holding the tty foreground: connected.
    let observed = observation_from(Some(&shell), &snapshots, Some(200), None);
    let ssh = observed.ssh.expect("foreground ssh is a connection");
    assert_eq!(ssh.host, "build.internal");
    assert_eq!(observed.process.as_deref(), Some("ssh"));
    assert!(observed.settle);

    // ssh in the background (git-over-ssh, ControlMaster): not a connection.
    let observed = observation_from(Some(&shell), &snapshots, Some(100), None);
    assert!(observed.ssh.is_none());
    assert_eq!(observed.process.as_deref(), Some("fish"));
  }

  #[test]
  fn ssh_detection_skips_proxy_channels() {
    // `ssh -o ProxyCommand='ssh -W %h:%p jump' host` shares the foreground
    // process group with its proxy channel; the user's client wins.
    let client = process("ssh", &["-o", "ProxyCommand=...", "host"], None, 200);
    let proxy = process("ssh", &["-W", "host:22", "jump"], None, 200);
    let group = [&proxy, &client];
    let picked = foreground_ssh(&group).expect("a client exists");
    assert_eq!(picked.args.last().map(String::as_str), Some("host"));
  }

  #[test]
  fn remote_title_drives_the_remote_label_parts() {
    let shell = process("fish", &[], Some("/home/u"), 100);
    let client = process("ssh", &["build.internal"], Some("/home/u"), 200);
    let snapshots = vec![client];

    // Without a title the transport names the session and the cwd stays
    // unknown — the local cwd must never leak into a remote label.
    let observed = observation_from(Some(&shell), &snapshots, Some(200), None);
    assert_eq!(observed.process.as_deref(), Some("ssh"));
    assert_eq!(observed.cwd, None);

    // A fish remote reports program and cwd through its title.
    let observed = observation_from(
      Some(&shell),
      &snapshots,
      Some(200),
      Some("fish: ~/projects"),
    );
    assert_eq!(observed.process.as_deref(), Some("fish"));
    assert_eq!(
      observed.cwd.as_deref(),
      Some(std::path::Path::new("~/projects"))
    );
  }

  #[test]
  fn local_jobs_come_from_the_foreground_group() {
    let shell = process("fish", &[], Some("/home/u/recoil"), 100);
    let editor = process("vim", &["src/main.rs"], Some("/home/u/recoil"), 300);
    let snapshots = vec![editor];

    let observed = observation_from(Some(&shell), &snapshots, Some(300), None);
    assert_eq!(observed.process.as_deref(), Some("vim"));
    assert_eq!(
      observed.cwd.as_deref(),
      Some(std::path::Path::new("/home/u/recoil"))
    );
    assert!(observed.settle, "a foreground job needs the settle window");

    // Idle prompt: the foreground is the shell itself, nothing to settle.
    let observed = observation_from(Some(&shell), &snapshots, Some(100), None);
    assert_eq!(observed.process.as_deref(), Some("fish"));
    assert!(!observed.settle);
  }
}
