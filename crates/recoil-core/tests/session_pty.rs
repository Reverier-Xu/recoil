//! Headless lifecycle evidence with real PTYs (E2E-01 core semantics).
//!
//! These tests exercise the woocraft-terminal session handle exactly the way
//! the application store does: spawn, attach/detach (there is no window
//! here), kill, and root exit.

use std::time::Duration;

use recoil_core::session::{ExitInfo, SessionTransition};
use woocraft_terminal::{SpawnOptions, TerminalBounds, TerminalSession};

/// An explicit shell keeps the tests independent of the user's `$SHELL`
/// (e.g. fish, which does not support POSIX arithmetic expansion).
fn shell() -> SpawnOptions {
  SpawnOptions::with_shell(("sh".to_owned(), vec![]))
}

fn wait_until(deadline: Duration, mut predicate: impl FnMut() -> bool) -> bool {
  let start = std::time::Instant::now();
  while start.elapsed() < deadline {
    if predicate() {
      return true;
    }
    std::thread::sleep(Duration::from_millis(20));
  }
  predicate()
}

#[test]
fn session_survives_without_a_view_and_processes_input() {
  let session =
    TerminalSession::spawn(shell(), TerminalBounds::default()).expect("spawn pty session");

  // The store owns the handle; no view exists in this test at all.
  assert!(session.is_alive());
  session.input_str("echo recoil-detached-$((41 + 1))\r");
  assert!(
    wait_until(Duration::from_secs(5), || session
      .text()
      .contains("recoil-detached-42")),
    "a session without a view must keep processing input"
  );

  session.kill();
  assert!(
    wait_until(Duration::from_secs(5), || !session.is_alive()),
    "kill must terminate the root process"
  );
}

#[test]
fn root_exit_is_observable_after_the_fact() {
  let session =
    TerminalSession::spawn(shell(), TerminalBounds::default()).expect("spawn pty session");

  // The child exits on its own, like a user typing `exit` in a hidden tab.
  session.input_str("exit\r");
  assert!(
    wait_until(Duration::from_secs(5), || !session.is_alive()),
    "root exit must be observable via the handle"
  );
  let status = session
    .child_exit_status()
    .expect("exited child must report a status");
  let exit = ExitInfo {
    code: Some(status.code()),
    signal: None,
  };
  let mut entry = recoil_core::session::SessionEntry::spawning(
    recoil_core::session::SessionMeta::new_local(recoil_core::session::SessionId::generate(), None),
  );
  entry.transition(SessionTransition::Attach).expect("attach");
  entry
    .transition(SessionTransition::RootExit(exit))
    .expect("root exit transition");
  assert!(!entry.is_alive());
}
