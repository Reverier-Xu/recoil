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
//! Scans are event-driven (spawn, title changes) plus a one-second
//! heartbeat. Event sources alone cannot be trusted: a scan can catch a
//! transient prompt hook (atuin's fish bindings, starship precmd helpers)
//! while it owns the tty foreground for a few milliseconds, and a remote
//! ssh disconnect produces no local event at all. The heartbeat bounds any
//! wrong label to about a second and notices disconnects; event scans keep
//! reactions instant. Settle-delay confirmation was rejected: it made every
//! real program (vim, ssh, top) wait for the label.

use std::path::PathBuf;

use gpui::Context;
use recoil_core::session::SshObservation;

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
/// Zero-configuration title formats seen in the wild:
///
/// - bash with the Debian-family default bashrc: `user@host:path` — the left
///   side identifies the connection, it is not a program;
/// - fish ≤ 3 style and many custom prompts: `command: path`;
/// - fish 4's default `fish_title`: `[host] command path`, or `[host] path`
///   when idle;
/// - a bare path (`/srv/www`, `~/src`).
///
/// fish and zsh ship without a title by default; servers running them stay
/// at the `ssh - host` fallback until a one-line rc snippet opts in.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Default)]
struct RemoteTitle {
  /// The remote foreground program, when the title carries one.
  program: Option<String>,
  /// The remote working directory.
  cwd: Option<PathBuf>,
}

/// A title token that starts a path.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn is_pathy(token: &str) -> bool {
  token.starts_with('/') || token.starts_with('~')
}

/// Parses the fish 4 title body (`command path`, `path`, `command`): the
/// first pathy token starts the cwd, the words before it name the program.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn space_separated_title(body: &str) -> RemoteTitle {
  let tokens: Vec<&str> = body.split_whitespace().collect();
  match tokens.iter().position(|token| is_pathy(token)) {
    Some(index) => RemoteTitle {
      program: (index > 0).then(|| tokens[..index].join(" ")),
      cwd: Some(PathBuf::from(tokens[index..].join(" "))),
    },
    // A lone word is a program name (a full-screen program's title).
    None => RemoteTitle {
      program: (tokens.len() == 1).then(|| tokens[0].to_owned()),
      cwd: None,
    },
  }
}

/// Parses a remote shell title into the remote program and remote cwd.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_remote_title(title: &str) -> RemoteTitle {
  let title = title.trim();
  if title.is_empty() {
    return RemoteTitle::default();
  }
  // A pathy title containing a colon (`/srv/a:b`) is a path, not a
  // `something: path` pair.
  if !is_pathy(title)
    && let Some((left, right)) = title.split_once(':')
  {
    let path = right.trim();
    let cwd = (!path.is_empty()).then(|| PathBuf::from(path));
    let left = left.trim();
    let program = (!left.is_empty() && !left.contains('@') && !left.contains(char::is_whitespace))
      .then(|| left.to_owned());
    return RemoteTitle { program, cwd };
  }
  // fish 4 marks ssh sessions with a bracketed host; strip it.
  let body = title
    .strip_prefix('[')
    .and_then(|rest| rest.split_once(']'))
    .map(|(_, rest)| rest.trim())
    .unwrap_or(title);
  space_separated_title(body)
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
  // The foreground group may include the root itself: the idle shell owns
  // its group, and a profile-spawned ssh client (G4) IS the session root.
  let foreground: Vec<&ProcessSnapshot> = match tpgid {
    Some(tpgid) => snapshots
      .iter()
      .chain(root)
      .filter(|s| s.pgrp == tpgid)
      .collect(),
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
    }
  }
}

/// The heartbeat cadence. One second bounds how long a transient prompt
/// hook (atuin) can masquerade as the foreground and how long an ssh
/// disconnect goes unnoticed, at a negligible cost of a bounded `/proc`
/// read per session per second.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const SCAN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Starts the observation loop for a session. Linux only for now; other
/// platforms gain observations through G3's terminal-behavior sources. The
/// loop ends by itself when the session leaves the registry.
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
      }
    });

    // The heartbeat self-heals whatever the event sources miss: a scan that
    // caught a transient prompt hook in the foreground, or a remote ssh
    // disconnect (which produces no local event). It ends when the scan
    // loop drops the receiver.
    let heartbeat = cx.spawn(async move |_this, cx| {
      loop {
        cx.background_executor().timer(SCAN_INTERVAL).await;
        if tx.try_send(()).is_err() {
          return;
        }
      }
    });

    store.set_observer(id, vec![scan, heartbeat], trigger);
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

    // bash's Debian-family default: no space after the colon.
    let bash = parse_remote_title("ops@srv-07:~/deploy/current");
    assert_eq!(bash.program, None);
    assert_eq!(
      bash.cwd.as_deref(),
      Some(std::path::Path::new("~/deploy/current"))
    );

    // fish 4's default: bracketed host, command and path separated by
    // spaces; the path may itself contain spaces.
    let fish4 = parse_remote_title("[srv-07] vim ~/my dir");
    assert_eq!(fish4.program.as_deref(), Some("vim"));
    assert_eq!(fish4.cwd.as_deref(), Some(std::path::Path::new("~/my dir")));
    let fish4_idle = parse_remote_title("[srv-07] ~");
    assert_eq!(fish4_idle.program, None);
    assert_eq!(fish4_idle.cwd.as_deref(), Some(std::path::Path::new("~")));
    let fish4_multi = parse_remote_title("[srv-07] sudo systemctl status /etc");
    assert_eq!(
      fish4_multi.program.as_deref(),
      Some("sudo systemctl status")
    );

    // Bare paths and lone program names.
    let bare = parse_remote_title("/srv/www");
    assert_eq!(bare.program, None);
    assert_eq!(bare.cwd.as_deref(), Some(std::path::Path::new("/srv/www")));
    let lone = parse_remote_title("htop");
    assert_eq!(lone.program.as_deref(), Some("htop"));
    assert_eq!(lone.cwd, None);

    // A path containing a colon is a path, not a `left: path` pair.
    let colon_path = parse_remote_title("/srv/a:b");
    assert_eq!(colon_path.program, None);
    assert_eq!(
      colon_path.cwd.as_deref(),
      Some(std::path::Path::new("/srv/a:b"))
    );

    // Paths may contain whitespace; titleless and unrecognizable input
    // parses to nothing.
    let spaced = parse_remote_title("fish: ~/my dir");
    assert_eq!(
      spaced.cwd.as_deref(),
      Some(std::path::Path::new("~/my dir"))
    );
    let empty = parse_remote_title("fish:");
    assert_eq!(empty.program.as_deref(), Some("fish"));
    assert_eq!(empty.cwd, None);
    assert!(parse_remote_title("no colon here").cwd.is_none());
    assert!(parse_remote_title("some random words").program.is_none());
    assert!(parse_remote_title("").cwd.is_none());
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

    // Idle prompt: the foreground is the shell itself.
    let observed = observation_from(Some(&shell), &snapshots, Some(100), None);
    assert_eq!(observed.process.as_deref(), Some("fish"));
  }
}

/// End-to-end validation of the `/proc` machinery against real PTYs: the
/// pure heuristics above mean nothing if the kernel plumbing (tpgid, pgrp,
/// children) is misread. A copy of `yes` named `ssh` stands in for a real
/// ssh client — the observer reads `comm` and `args`, never the binary.
#[cfg(all(test, target_os = "linux"))]
mod pty_tests {
  use std::time::Duration;

  use woocraft_terminal::{SpawnOptions, TerminalBounds, TerminalSession};

  use super::*;

  struct TestDir(PathBuf);

  impl TestDir {
    /// A unique directory per test: tests in one process share the pid, so
    /// a plain pid-based path would let one test's cleanup delete another
    /// test's staged binaries.
    fn new() -> Self {
      static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
      let path = std::env::temp_dir().join(format!(
        "recoil-observer-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
      ));
      std::fs::create_dir_all(&path).expect("create temp dir");
      Self(path)
    }
  }

  impl Drop for TestDir {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.0);
    }
  }

  /// Polls the observer until `ok` holds (at most ~10s).
  fn observe_until(
    stage: &str, session: &TerminalSession, root_pid: u32, ok: impl Fn(&Observation) -> bool,
  ) -> Observation {
    let mut last = None;
    for _ in 0..100 {
      let observed = observe(root_pid, None);
      if ok(&observed) {
        return observed;
      }
      last = Some(observed);
      std::thread::sleep(Duration::from_millis(100));
    }
    let last = last.map(|o| (o.process, o.cwd, o.ssh.map(|s| s.host)));
    let tree: Vec<_> = descendants(root_pid)
      .iter()
      .map(|s| (s.comm.clone(), s.pgrp))
      .collect();
    let tpgid = read_tpgid(root_pid);
    let screen = session.last_n_non_empty_lines(8).join(" | ");
    panic!(
      "stage {stage}: condition not met within 10s; last: {last:?}, tpgid: {tpgid:?}, tree: {tree:?}, screen: {screen}"
    );
  }

  #[test]
  fn detects_foreground_processes_in_a_real_pty() {
    let dir = TestDir::new();
    let fake = dir.0.join("ssh");
    std::fs::copy("/usr/bin/sleep", &fake).expect("stage the fake ssh binary");
    let session = TerminalSession::spawn(
      SpawnOptions {
        shell: Some(("bash".to_owned(), vec!["-i".to_owned()])),
        env: vec![("TERM".to_owned(), "dumb".to_owned())],
        ..Default::default()
      },
      TerminalBounds::default(),
    )
    .expect("spawn bash");
    let root = session.pid().expect("shell pid");

    // Idle: the foreground is the shell itself.
    let idle = observe_until("idle", &session, root, |o| o.process.is_some());
    assert_eq!(idle.process.as_deref(), Some("bash"));
    assert!(idle.ssh.is_none());

    // A foreground job is picked by its process group.
    session.input_str("sleep 30\n");
    let job = observe_until("sleep", &session, root, |o| {
      o.process.as_deref() == Some("sleep")
    });
    assert!(job.ssh.is_none());
    session.input_str("\x03"); // Ctrl-C
    observe_until("back-to-bash", &session, root, |o| {
      o.process.as_deref() == Some("bash")
    });

    // An ssh client in the foreground is a connection; its command line
    // names the host. The full path keeps the test independent of the
    // shell's PATH setup; the kernel still reports the comm as `ssh`.
    session.input_str(&format!("{} 3600\n", fake.display()));
    let connected = observe_until("ssh", &session, root, |o| o.ssh.is_some());
    let ssh = connected.ssh.expect("ssh detected");
    assert_eq!(ssh.host, "3600");
    assert_eq!(connected.process.as_deref(), Some("ssh"));

    session.kill();
  }

  #[test]
  fn detects_ssh_as_the_session_root() {
    // Profile-spawned sessions (G4) run the ssh client AS the root process:
    // detection must not depend on the client being a descendant.
    let dir = TestDir::new();
    let fake = dir.0.join("ssh");
    std::fs::copy("/usr/bin/sleep", &fake).expect("stage the fake ssh binary");
    let session = TerminalSession::spawn(
      SpawnOptions {
        shell: Some((fake.to_string_lossy().into_owned(), vec!["3600".to_owned()])),
        ..Default::default()
      },
      TerminalBounds::default(),
    )
    .expect("spawn fake ssh");
    let root = session.pid().expect("client pid");

    let connected = observe_until("ssh", &session, root, |o| o.ssh.is_some());
    assert_eq!(connected.ssh.expect("ssh detected").host, "3600");

    session.kill();
  }
}
