//! Recoil entry point: bootstrap GPUI, woocraft, the stores, the workspace
//! window, and the tray.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

use gpui::{App, Bounds, WindowBounds, WindowOptions, px, size};
use recoil_term::{APP_NAME, init, tray, workspace};
use woocraft::TitleBar;

fn main() {
  tracing_subscriber::fmt()
    .with_env_filter(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
    )
    .init();

  let app = gpui_platform::application().with_assets(woocraft::Assets);
  app.run(|cx: &mut App| {
    woocraft::init(cx);
    init(cx);
    cx.activate(true);

    let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
    let window = cx
      .open_window(
        WindowOptions {
          window_bounds: Some(WindowBounds::Windowed(bounds)),
          titlebar: Some(TitleBar::title_bar_options()),
          #[cfg(target_os = "linux")]
          window_background: gpui::WindowBackgroundAppearance::Transparent,
          #[cfg(target_os = "linux")]
          window_decorations: Some(gpui::WindowDecorations::Client),
          ..Default::default()
        },
        workspace::Workspace::view,
      )
      .unwrap_or_else(|err| panic!("failed to open the main window: {err}"));

    window
      .update(cx, |_, window, _| {
        window.activate_window();
        window.set_window_title(APP_NAME);
      })
      .unwrap_or_else(|err| panic!("failed to configure the main window: {err}"));

    tray::init(cx);
  });
}
