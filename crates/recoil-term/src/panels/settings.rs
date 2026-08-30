//! The settings panel: a dock-hosted form editor for `config.toml`.
//!
//! The panel edits a snapshot of the current configuration. Saving applies
//! the snapshot through `SettingsStore::update_config`, which validates the
//! result, debounces the disk write, and emits `SettingsEvent::Changed` so
//! that live terminal views can react (T-G02-04).

use gpui::{
  App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
  ParentElement as _, Render, SharedString, Styled as _, Window, div, px,
};
use recoil_core::config::{Config, CursorShape, FeaturesConfig, TerminalPalette, ThemeMode};
use woocraft::{
  ActiveTheme as _, Button, ButtonVariants as _, Field, IconName, Input, InputState, Label, Panel,
  PanelEvent, PanelState, ScrollableElement as _, Selectable as _, Switch, field, h_flex, v_flex,
  v_form,
};

use crate::{
  localization::t,
  stores::settings::{SettingsStore, settings_store},
};

const CURSOR_SHAPES: [CursorShape; 4] = [
  CursorShape::Block,
  CursorShape::Underline,
  CursorShape::Bar,
  CursorShape::Hollow,
];

const THEME_MODES: [ThemeMode; 3] = [ThemeMode::Light, ThemeMode::Dark, ThemeMode::System];

/// Panel name used for serialization and registry lookup.
pub const SETTINGS_PANEL: &str = "SettingsPanel";

/// The active settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsPage {
  Terminal,
  Appearance,
  Features,
}

/// A dock panel editing one snapshot of the configuration.
pub struct SettingsPanel {
  store: Entity<SettingsStore>,
  page: SettingsPage,
  diagnostics: Option<String>,

  // Terminal
  font_family: Entity<InputState>,
  font_fallbacks: Entity<InputState>,
  font_size: Entity<InputState>,
  scrolling_history: Entity<InputState>,
  cursor_shape: CursorShape,
  cursor_blink: bool,
  alternate_scroll: bool,

  // Appearance
  theme_mode: ThemeMode,
  palette_enabled: bool,
  palette_inputs: Vec<(SharedString, Entity<InputState>)>,

  // Features
  features: FeaturesConfig,

  focus_handle: FocusHandle,
}

impl SettingsPanel {
  /// Opens a settings panel in the center of the active dock area,
  /// activating it if one already exists.
  pub fn open(cx: &mut App) {
    let Some(dock_area) = crate::workspace::active_dock_area(cx) else {
      return;
    };
    let panel_id = "settings".to_string();
    if dock_area.read(cx).panel_by_id(&panel_id, cx).is_some() {
      let Some(window) = cx.active_window() else {
        return;
      };
      window
        .update(cx, |_, window, cx| {
          dock_area.update(cx, |area, cx| {
            area.activate_panel_by_id(&panel_id, window, cx);
          });
        })
        .ok();
      return;
    }

    let panel = cx.new(Self::new);
    let Some(window) = cx.active_window() else {
      return;
    };
    window
      .update(cx, |_, window, cx| {
        dock_area.update(cx, |area, cx| {
          area.add_to_center(std::sync::Arc::new(panel), window, cx);
          area.activate_panel_by_id(&panel_id, window, cx);
        });
      })
      .ok();
  }

  fn new(cx: &mut Context<Self>) -> Self {
    let store = settings_store(cx);
    let config = store.read(cx).config().clone();

    let font_family =
      cx.new(|cx| InputState::new(cx).default_value(config.terminal.font_family.clone()));
    let font_fallbacks =
      cx.new(|cx| InputState::new(cx).default_value(config.terminal.font_fallbacks.join(", ")));
    let font_size =
      cx.new(|cx| InputState::new(cx).default_value(config.terminal.font_size.to_string()));
    let scrolling_history =
      cx.new(|cx| InputState::new(cx).default_value(config.terminal.scrolling_history.to_string()));

    let palette_enabled = config.theme.terminal_palette.is_some();
    let palette = config.theme.terminal_palette.clone().unwrap_or_default();
    let palette_inputs = palette_input_states(&palette, cx);

    Self {
      store,
      page: SettingsPage::Terminal,
      diagnostics: None,
      font_family,
      font_fallbacks,
      font_size,
      scrolling_history,
      cursor_shape: config.terminal.cursor_shape,
      cursor_blink: config.terminal.cursor_blink,
      alternate_scroll: config.terminal.alternate_scroll,
      theme_mode: config.theme.mode,
      palette_enabled,
      palette_inputs,
      features: config.terminal.features.clone(),
      focus_handle: cx.focus_handle(),
    }
  }

  fn build_config(&self, cx: &App) -> Result<Config, String> {
    let font_family = self.font_family.read(cx).value().to_string();
    let font_fallbacks = self
      .font_fallbacks
      .read(cx)
      .value()
      .split(',')
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .map(str::to_owned)
      .collect();
    let font_size = self
      .font_size
      .read(cx)
      .value()
      .parse::<f64>()
      .map_err(|_| t!("settings.font_size").to_string())?;
    let scrolling_history = self
      .scrolling_history
      .read(cx)
      .value()
      .parse::<usize>()
      .map_err(|_| t!("settings.scrolling_history").to_string())?;

    let terminal_palette = if self.palette_enabled {
      Some(build_palette(&self.palette_inputs, cx)?)
    } else {
      None
    };

    let config = Config {
      terminal: recoil_core::config::TerminalConfig {
        font_family,
        font_fallbacks,
        font_size,
        scrolling_history,
        cursor_shape: self.cursor_shape,
        cursor_blink: self.cursor_blink,
        alternate_scroll: self.alternate_scroll,
        features: self.features.clone(),
      },
      theme: recoil_core::config::ThemeConfig {
        mode: self.theme_mode,
        terminal_palette,
      },
    };

    config.validate().map_err(|err| err.to_string())?;
    Ok(config)
  }

  fn save(&mut self, cx: &mut Context<Self>) {
    let config = match self.build_config(cx) {
      Ok(config) => config,
      Err(err) => {
        self.diagnostics = Some(err);
        cx.notify();
        return;
      }
    };

    let store = self.store.clone();
    let result = store.update(cx, |store, cx| {
      store.update_config(|current| *current = config, cx)
    });

    match result {
      Ok(()) => self.diagnostics = None,
      Err(err) => self.diagnostics = Some(err.to_string()),
    }
    cx.notify();
  }

  fn set_page(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
    self.page = page;
    cx.notify();
  }

  fn set_cursor_shape(&mut self, shape: CursorShape, cx: &mut Context<Self>) {
    self.cursor_shape = shape;
    cx.notify();
  }

  fn set_theme_mode(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
    self.theme_mode = mode;
    cx.notify();
  }

  fn toggle_palette(&mut self, enabled: bool, cx: &mut Context<Self>) {
    self.palette_enabled = enabled;
    cx.notify();
  }

  fn set_feature(&mut self, feature: FeatureSwitch, value: bool, cx: &mut Context<Self>) {
    match feature {
      FeatureSwitch::Hyperlink => self.features.hyperlink = value,
      FeatureSwitch::SmartSelect => self.features.smart_select = value,
      FeatureSwitch::MouseReporting => self.features.mouse_reporting = value,
      FeatureSwitch::CopyOnSelect => self.features.copy_on_select = value,
      FeatureSwitch::Osc52 => self.features.osc52 = value,
      FeatureSwitch::Bell => self.features.bell = value,
      FeatureSwitch::BellWhenHiddenNotify => self.features.bell_when_hidden_notify = value,
    }
    cx.notify();
  }
}

#[derive(Debug, Clone, Copy)]
enum FeatureSwitch {
  Hyperlink,
  SmartSelect,
  MouseReporting,
  CopyOnSelect,
  Osc52,
  Bell,
  BellWhenHiddenNotify,
}

impl Panel for SettingsPanel {
  fn panel_name(&self) -> &'static str {
    SETTINGS_PANEL
  }

  fn panel_id(&self, _cx: &App) -> SharedString {
    "settings".into()
  }

  fn title(&self, _cx: &App) -> SharedString {
    t!("settings.title").into()
  }

  fn tab_name(&self, cx: &App) -> Option<SharedString> {
    Some(self.title(cx))
  }

  fn icon(&self, _cx: &App) -> IconName {
    IconName::Settings
  }

  fn dump(&self, _cx: &App) -> PanelState {
    PanelState::new(self)
  }

  fn on_removed(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
}

impl EventEmitter<PanelEvent> for SettingsPanel {}

impl Focusable for SettingsPanel {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl Render for SettingsPanel {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let page = self.page;
    let diagnostics = self.diagnostics.clone();

    v_flex()
      .size_full()
      .min_h_0()
      .p_4()
      .gap_4()
      .child(self.render_tabs(cx))
      .child(
        v_flex()
          .flex_1()
          .overflow_y_scrollbar()
          .gap_4()
          .child(match page {
            SettingsPage::Terminal => self.render_terminal(cx).into_any_element(),
            SettingsPage::Appearance => self.render_appearance(cx).into_any_element(),
            SettingsPage::Features => self.render_features(cx).into_any_element(),
          }),
      )
      .child(diagnostics_label(diagnostics, cx))
      .child(
        h_flex().justify_end().child(
          Button::new("settings-save")
            .primary()
            .label(t!("settings.save").to_string())
            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
        ),
      )
  }
}

impl SettingsPanel {
  fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let page = self.page;
    h_flex().gap_2().children([
      page_button(
        SettingsPage::Terminal,
        page,
        t!("settings.terminal_tab"),
        cx.listener(|this, _, _, cx| this.set_page(SettingsPage::Terminal, cx)),
      )
      .into_any_element(),
      page_button(
        SettingsPage::Appearance,
        page,
        t!("settings.appearance_tab"),
        cx.listener(|this, _, _, cx| this.set_page(SettingsPage::Appearance, cx)),
      )
      .into_any_element(),
      page_button(
        SettingsPage::Features,
        page,
        t!("settings.features_tab"),
        cx.listener(|this, _, _, cx| this.set_page(SettingsPage::Features, cx)),
      )
      .into_any_element(),
    ])
  }

  fn render_terminal(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let cursor_shape = self.cursor_shape;
    v_form()
      .label_width(px(160.))
      .gap_4()
      .child(field_row(
        t!("settings.font_family"),
        Input::new(&self.font_family).cleanable(true),
      ))
      .child(field_row(
        t!("settings.font_fallbacks"),
        Input::new(&self.font_fallbacks).cleanable(true),
      ))
      .child(field_row(
        t!("settings.font_size"),
        Input::new(&self.font_size),
      ))
      .child(field_row(
        t!("settings.scrolling_history"),
        Input::new(&self.scrolling_history),
      ))
      .child(field_row(
        t!("settings.cursor_shape"),
        h_flex().gap_2().children(CURSOR_SHAPES.iter().map(|shape| {
          Button::new(("cursor-shape", *shape as usize))
            .label(cursor_shape_label(*shape))
            .selected(cursor_shape == *shape)
            .on_click(cx.listener(move |this, _, _, cx| this.set_cursor_shape(*shape, cx)))
            .into_any_element()
        })),
      ))
      .child(field_row(
        t!("settings.cursor_blink"),
        Switch::new("cursor-blink")
          .checked(self.cursor_blink)
          .on_click(cx.listener(|this, checked, _, cx| {
            this.cursor_blink = *checked;
            cx.notify();
          })),
      ))
      .child(field_row(
        t!("settings.alternate_scroll"),
        Switch::new("alternate-scroll")
          .checked(self.alternate_scroll)
          .on_click(cx.listener(|this, checked, _, cx| {
            this.alternate_scroll = *checked;
            cx.notify();
          })),
      ))
  }

  fn render_appearance(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let theme_mode = self.theme_mode;
    let palette_enabled = self.palette_enabled;
    let palette_fields: Vec<Field> = if palette_enabled {
      self
        .palette_inputs
        .iter()
        .map(|(label, state)| field_row(label.clone(), Input::new(state).cleanable(true)))
        .collect()
    } else {
      Vec::new()
    };

    let mut form = v_form()
      .label_width(px(160.))
      .gap_4()
      .child(field_row(
        t!("settings.theme_mode"),
        h_flex().gap_2().children(THEME_MODES.iter().map(|mode| {
          Button::new(("theme-mode", *mode as usize))
            .label(theme_mode_label(*mode))
            .selected(theme_mode == *mode)
            .on_click(cx.listener(move |this, _, _, cx| this.set_theme_mode(*mode, cx)))
            .into_any_element()
        })),
      ))
      .child(field_row(
        t!("settings.palette"),
        Switch::new("palette-enabled")
          .checked(self.palette_enabled)
          .on_click(cx.listener(|this, checked, _, cx| this.toggle_palette(*checked, cx))),
      ));
    form = form.children(palette_fields);
    form
  }

  fn render_features(&self, cx: &mut Context<Self>) -> impl IntoElement {
    v_form().label_width(px(160.)).gap_4().children([
      feature_row(FeatureSwitch::Hyperlink, &self.features, cx),
      feature_row(FeatureSwitch::SmartSelect, &self.features, cx),
      feature_row(FeatureSwitch::MouseReporting, &self.features, cx),
      feature_row(FeatureSwitch::CopyOnSelect, &self.features, cx),
      feature_row(FeatureSwitch::Osc52, &self.features, cx),
      feature_row(FeatureSwitch::Bell, &self.features, cx),
      feature_row(FeatureSwitch::BellWhenHiddenNotify, &self.features, cx),
    ])
  }
}

fn page_button(
  target: SettingsPage, current: SettingsPage, label: impl Into<SharedString>,
  on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
  let selected = target == current;
  Button::new(("settings-tab", target as usize))
    .label(label)
    .selected(selected)
    .on_click(on_click)
}

fn field_row(label: impl Into<SharedString>, child: impl IntoElement) -> Field {
  field().label(label.into()).child(child)
}

fn feature_row(
  feature: FeatureSwitch, features: &FeaturesConfig, cx: &mut Context<SettingsPanel>,
) -> Field {
  let (label, checked) = match feature {
    FeatureSwitch::Hyperlink => (t!("settings.hyperlink"), features.hyperlink),
    FeatureSwitch::SmartSelect => (t!("settings.smart_select"), features.smart_select),
    FeatureSwitch::MouseReporting => (t!("settings.mouse_reporting"), features.mouse_reporting),
    FeatureSwitch::CopyOnSelect => (t!("settings.copy_on_select"), features.copy_on_select),
    FeatureSwitch::Osc52 => (t!("settings.osc52"), features.osc52),
    FeatureSwitch::Bell => (t!("settings.bell"), features.bell),
    FeatureSwitch::BellWhenHiddenNotify => (
      t!("settings.bell_when_hidden_notify"),
      features.bell_when_hidden_notify,
    ),
  };
  field_row(
    label,
    Switch::new(("feature", feature as usize))
      .checked(checked)
      .on_click(cx.listener(move |this, checked, _, cx| this.set_feature(feature, *checked, cx))),
  )
}

fn diagnostics_label(
  diagnostics: Option<String>, cx: &mut Context<SettingsPanel>,
) -> impl IntoElement {
  let theme = cx.theme();
  let mut container = div().min_h(px(20.));
  if let Some(msg) = diagnostics {
    container = container.child(
      Label::new(format!("{}: {}", t!("settings.diagnostics"), msg))
        .text_color(theme.danger)
        .into_any_element(),
    );
  }
  container
}

fn palette_input_states(
  palette: &TerminalPalette, cx: &mut Context<SettingsPanel>,
) -> Vec<(SharedString, Entity<InputState>)> {
  let entries = palette_entries(palette);
  entries
    .into_iter()
    .map(|(label, value)| {
      let value = value.unwrap_or_default();
      let state = cx.new(|cx| InputState::new(cx).default_value(value));
      (SharedString::from(label), state)
    })
    .collect()
}

fn palette_entries(palette: &TerminalPalette) -> Vec<(String, Option<String>)> {
  vec![
    ("foreground".to_owned(), palette.foreground.clone()),
    ("background".to_owned(), palette.background.clone()),
    ("cursor".to_owned(), palette.cursor.clone()),
    ("selection".to_owned(), palette.selection.clone()),
    ("black".to_owned(), palette.black.clone()),
    ("red".to_owned(), palette.red.clone()),
    ("green".to_owned(), palette.green.clone()),
    ("yellow".to_owned(), palette.yellow.clone()),
    ("blue".to_owned(), palette.blue.clone()),
    ("magenta".to_owned(), palette.magenta.clone()),
    ("cyan".to_owned(), palette.cyan.clone()),
    ("white".to_owned(), palette.white.clone()),
    ("bright-black".to_owned(), palette.bright_black.clone()),
    ("bright-red".to_owned(), palette.bright_red.clone()),
    ("bright-green".to_owned(), palette.bright_green.clone()),
    ("bright-yellow".to_owned(), palette.bright_yellow.clone()),
    ("bright-blue".to_owned(), palette.bright_blue.clone()),
    ("bright-magenta".to_owned(), palette.bright_magenta.clone()),
    ("bright-cyan".to_owned(), palette.bright_cyan.clone()),
    ("bright-white".to_owned(), palette.bright_white.clone()),
  ]
}

fn build_palette(
  inputs: &[(SharedString, Entity<InputState>)], cx: &App,
) -> Result<TerminalPalette, String> {
  let mut palette = TerminalPalette::default();
  for (label, state) in inputs {
    let value = state.read(cx).value().to_string();
    if value.is_empty() {
      continue;
    }
    match label.as_ref() {
      "foreground" => palette.foreground = Some(value),
      "background" => palette.background = Some(value),
      "cursor" => palette.cursor = Some(value),
      "selection" => palette.selection = Some(value),
      "black" => palette.black = Some(value),
      "red" => palette.red = Some(value),
      "green" => palette.green = Some(value),
      "yellow" => palette.yellow = Some(value),
      "blue" => palette.blue = Some(value),
      "magenta" => palette.magenta = Some(value),
      "cyan" => palette.cyan = Some(value),
      "white" => palette.white = Some(value),
      "bright-black" => palette.bright_black = Some(value),
      "bright-red" => palette.bright_red = Some(value),
      "bright-green" => palette.bright_green = Some(value),
      "bright-yellow" => palette.bright_yellow = Some(value),
      "bright-blue" => palette.bright_blue = Some(value),
      "bright-magenta" => palette.bright_magenta = Some(value),
      "bright-cyan" => palette.bright_cyan = Some(value),
      "bright-white" => palette.bright_white = Some(value),
      _ => {}
    }
  }
  Ok(palette)
}

fn cursor_shape_label(shape: CursorShape) -> SharedString {
  match shape {
    CursorShape::Block => "Block".into(),
    CursorShape::Underline => "Underline".into(),
    CursorShape::Bar => "Bar".into(),
    CursorShape::Hollow => "Hollow".into(),
  }
}

fn theme_mode_label(mode: ThemeMode) -> SharedString {
  match mode {
    ThemeMode::Light => "Light".into(),
    ThemeMode::Dark => "Dark".into(),
    ThemeMode::System => "System".into(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn palette_entries_match_config_fields() {
    let palette = TerminalPalette::default();
    let entries = palette_entries(&palette);
    assert_eq!(entries.len(), 20);
  }

  #[test]
  fn palette_builds_round_trip() {
    let palette = TerminalPalette {
      foreground: Some("#ffffff".to_owned()),
      background: Some("#000000".to_owned()),
      ..TerminalPalette::default()
    };
    assert_eq!(palette_entries(&palette)[0].1.as_deref(), Some("#ffffff"));
  }
}
