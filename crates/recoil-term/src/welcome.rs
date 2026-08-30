//! The welcome page shown in the main area when every terminal is closed.

use gpui::{
  App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
  Styled as _, Window, div,
};
use woocraft::{ActiveTheme as _, Icon, IconName, StyledExt as _, h_flex, v_flex};

use crate::localization::t;

/// The empty-state view for the center area.
pub struct Welcome;

impl Render for Welcome {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
      .size_full()
      .items_center()
      .justify_center()
      .gap_3()
      .bg(theme.background)
      .text_color(theme.muted_foreground)
      .child(
        h_flex()
          .items_center()
          .gap_2()
          .child(
            Icon::new(IconName::Prompt)
              .size_6()
              .text_color(theme.primary),
          )
          .child(
            div()
              .font_semibold()
              .text_color(theme.foreground)
              .child(t!("app.name").to_string()),
          ),
      )
      .child(div().child(t!("welcome.closed").to_string()))
      .child(div().child({
        let open_hint = t!("welcome.open_hint");
        let tree_hint = t!("welcome.tree_hint");
        SharedString::from(format!("{open_hint} · {tree_hint}"))
      }))
  }
}

/// Creates the welcome placeholder view.
pub fn view(_window: &mut Window, cx: &mut App) -> Entity<Welcome> {
  cx.new(|_| Welcome)
}
