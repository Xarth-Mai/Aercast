use ashpd::desktop::settings::{
    ACCENT_COLOR_SCHEME_KEY, APPEARANCE_NAMESPACE, CONTRAST_KEY, Contrast, REDUCED_MOTION_KEY,
    ReducedMotion, Settings as PortalSettings,
};
use futures_util::{SinkExt, Stream, StreamExt};
use iced::{
    Background, Border, Color, Theme,
    theme::{
        Palette,
        palette::{Extended, Pair},
    },
    widget::{button, checkbox, container, pick_list, rule, text_input},
};

const WINDOW_BG: Color = iced::color!(0x242424);
const SURFACE: Color = iced::color!(0x2c2c2c);
const SURFACE_RAISED: Color = iced::color!(0x343434);
const BORDER: Color = iced::color!(0x484848);
const TEXT: Color = iced::color!(0xf6f5f4);
const TEXT_MUTED: Color = iced::color!(0xc0bfbc);
const TEXT_DISABLED: Color = iced::color!(0x9a9996);
const FALLBACK_ACCENT: Color = iced::color!(0xbd425a);
const DARK_ACCENT_TEXT: Color = iced::color!(0x1e1e1e);

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Appearance {
    pub(super) theme: Theme,
    pub(super) high_contrast: bool,
    pub(super) reduced_motion: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self::new(None, false, false)
    }
}

impl Appearance {
    fn new(accent: Option<Color>, high_contrast: bool, reduced_motion: bool) -> Self {
        let accent_bg = accent.unwrap_or(FALLBACK_ACCENT);
        let accent_fg = foreground(accent_bg);
        let accent_standalone = standalone(accent_bg);
        let accent_subtle = mix(SURFACE, accent_bg, 0.15);
        let theme = Theme::custom_with_fn(
            "Aercast",
            Palette {
                background: WINDOW_BG,
                text: TEXT,
                primary: accent_bg,
                ..Palette::DARK
            },
            move |palette| {
                let mut extended = Extended::generate(palette);
                extended.primary.base = Pair {
                    color: accent_bg,
                    text: accent_fg,
                };
                extended.primary.weak = Pair {
                    color: accent_subtle,
                    text: TEXT,
                };
                extended.primary.strong = Pair {
                    color: accent_standalone,
                    text: foreground(accent_standalone),
                };
                extended
            },
        );
        Self {
            theme,
            high_contrast,
            reduced_motion,
        }
    }

    pub(super) fn muted_text(&self) -> Color {
        if self.high_contrast { TEXT } else { TEXT_MUTED }
    }

    fn border_color(&self, normal: Color) -> Color {
        if self.high_contrast {
            TEXT_DISABLED
        } else {
            normal
        }
    }

    pub(super) fn primary_button(&self, status: button::Status) -> button::Style {
        let primary = self.theme.extended_palette().primary;
        self.button(
            status,
            primary.base.color,
            mix(primary.base.color, Color::WHITE, 0.06),
            primary.base.text,
            primary.strong.color,
        )
    }

    pub(super) fn neutral_button(&self, status: button::Status) -> button::Style {
        self.button(status, SURFACE, SURFACE_RAISED, TEXT, BORDER)
    }

    pub(super) fn danger_button(&self, status: button::Status) -> button::Style {
        let background = Palette::DARK.danger;
        self.button(
            status,
            background,
            mix(background, Color::WHITE, 0.06),
            foreground(background),
            background,
        )
    }

    fn button(
        &self,
        status: button::Status,
        background: Color,
        hover: Color,
        text: Color,
        border: Color,
    ) -> button::Style {
        let (background, text_color, border) = match status {
            button::Status::Hovered | button::Status::Pressed => (hover, foreground(hover), border),
            button::Status::Disabled => (SURFACE, TEXT_DISABLED, BORDER),
            button::Status::Active => (background, text, border),
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color,
            border: Border {
                color: self.border_color(border),
                width: if self.high_contrast { 2.0 } else { 1.0 },
                radius: 8.0.into(),
            },
            ..button::Style::default()
        }
    }

    pub(super) fn checkbox(&self, status: checkbox::Status) -> checkbox::Style {
        let primary = self.theme.extended_palette().primary;
        let (checked, hovered, disabled) = match status {
            checkbox::Status::Active { is_checked } => (is_checked, false, false),
            checkbox::Status::Hovered { is_checked } => (is_checked, true, false),
            checkbox::Status::Disabled { is_checked } => (is_checked, false, true),
        };
        let background = if checked {
            primary.base.color
        } else if hovered {
            SURFACE_RAISED
        } else {
            SURFACE
        };
        checkbox::Style {
            background: Background::Color(background),
            icon_color: primary.base.text,
            border: Border {
                color: self.border_color(if checked {
                    primary.strong.color
                } else {
                    BORDER
                }),
                width: if self.high_contrast { 2.0 } else { 1.0 },
                radius: 4.0.into(),
            },
            text_color: Some(if disabled { TEXT_DISABLED } else { TEXT }),
        }
    }

    pub(super) fn text_input(&self, status: text_input::Status) -> text_input::Style {
        let primary = self.theme.extended_palette().primary;
        let focused = matches!(status, text_input::Status::Focused { .. });
        let disabled = status == text_input::Status::Disabled;
        text_input::Style {
            background: Background::Color(SURFACE),
            border: Border {
                color: if focused {
                    primary.strong.color
                } else {
                    self.border_color(BORDER)
                },
                width: if focused {
                    if self.high_contrast { 3.0 } else { 2.0 }
                } else if self.high_contrast {
                    2.0
                } else {
                    1.0
                },
                radius: 8.0.into(),
            },
            icon: self.muted_text(),
            placeholder: if disabled {
                TEXT_DISABLED
            } else {
                self.muted_text()
            },
            value: if disabled { TEXT_DISABLED } else { TEXT },
            selection: primary.weak.color,
        }
    }

    pub(super) fn pick_list(&self, status: pick_list::Status) -> pick_list::Style {
        let opened = matches!(status, pick_list::Status::Opened { .. });
        let hovered = status == pick_list::Status::Hovered;
        pick_list::Style {
            text_color: TEXT,
            placeholder_color: self.muted_text(),
            handle_color: self.muted_text(),
            background: Background::Color(if hovered { SURFACE_RAISED } else { SURFACE }),
            border: Border {
                color: if opened {
                    self.theme.extended_palette().primary.strong.color
                } else {
                    self.border_color(BORDER)
                },
                width: match (opened, self.high_contrast) {
                    (true, true) => 3.0,
                    (false, false) => 1.0,
                    _ => 2.0,
                },
                radius: 8.0.into(),
            },
        }
    }

    pub(super) fn boxed_list(&self) -> container::Style {
        container::Style {
            background: Some(Background::Color(SURFACE)),
            text_color: Some(TEXT),
            border: Border {
                color: self.border_color(BORDER),
                width: if self.high_contrast { 2.0 } else { 1.0 },
                radius: 12.0.into(),
            },
            ..container::Style::default()
        }
    }

    pub(super) fn separator(&self) -> rule::Style {
        rule::Style {
            color: self.border_color(BORDER),
            ..rule::default(&self.theme)
        }
    }
}

pub(super) fn watch(
    connection: zbus::Connection,
) -> impl Stream<Item = Result<Appearance, String>> {
    iced::stream::try_channel(1, async move |mut updates| {
        let portal = PortalSettings::with_connection(connection)
            .await
            .map_err(|error| error.to_string())?;
        let mut changes = portal
            .receive_setting_changed()
            .await
            .map_err(|error| error.to_string())?;
        let color = portal.accent_color().await.ok();
        let mut accent = color.as_ref().and_then(valid_accent);
        let mut high_contrast = matches!(portal.contrast().await, Ok(Contrast::High));
        let mut reduced_motion = matches!(
            portal.reduced_motion().await,
            Ok(ReducedMotion::ReducedMotion)
        );
        if updates
            .send(Appearance::new(accent, high_contrast, reduced_motion))
            .await
            .is_err()
        {
            return Ok(());
        }
        while let Some(setting) = changes.next().await {
            if setting.namespace() != APPEARANCE_NAMESPACE {
                continue;
            }
            match setting.key() {
                ACCENT_COLOR_SCHEME_KEY => {
                    accent = setting
                        .value()
                        .try_clone()
                        .ok()
                        .and_then(|value| <(f64, f64, f64)>::try_from(value).ok())
                        .and_then(valid_channels);
                }
                CONTRAST_KEY => {
                    let Ok(value) = u32::try_from(setting.value()) else {
                        continue;
                    };
                    high_contrast = value == 1;
                }
                REDUCED_MOTION_KEY => {
                    let Ok(value) = u32::try_from(setting.value()) else {
                        continue;
                    };
                    reduced_motion = value == 1;
                }
                _ => continue,
            }
            if updates
                .send(Appearance::new(accent, high_contrast, reduced_motion))
                .await
                .is_err()
            {
                return Ok(());
            }
        }
        Err("Settings Portal appearance stream closed".to_owned())
    })
}

fn valid_accent(color: &ashpd::desktop::Color) -> Option<Color> {
    valid_channels((color.red(), color.green(), color.blue()))
}

fn valid_channels((red, green, blue): (f64, f64, f64)) -> Option<Color> {
    [red, green, blue]
        .into_iter()
        .all(|channel| channel.is_finite() && (0.0..=1.0).contains(&channel))
        .then(|| Color::from_rgb(red as f32, green as f32, blue as f32))
}

fn foreground(background: Color) -> Color {
    let foreground = if background.relative_contrast(Color::WHITE)
        >= background.relative_contrast(DARK_ACCENT_TEXT)
    {
        Color::WHITE
    } else {
        DARK_ACCENT_TEXT
    };
    if background.relative_contrast(foreground) >= 4.5 {
        foreground
    } else {
        Color::BLACK
    }
}

fn standalone(base: Color) -> Color {
    if base.relative_contrast(WINDOW_BG) >= 4.5 {
        return base;
    }
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..12 {
        let middle = (low + high) / 2.0;
        if mix(base, Color::WHITE, middle).relative_contrast(WINDOW_BG) >= 4.5 {
            high = middle;
        } else {
            low = middle;
        }
    }
    mix(base, Color::WHITE, high)
}

fn mix(from: Color, to: Color, amount: f32) -> Color {
    Color::from_rgba(
        from.r + (to.r - from.r) * amount,
        from.g + (to.g - from.g) * amount,
        from.b + (to.b - from.b) * amount,
        from.a + (to.a - from.a) * amount,
    )
}

#[cfg(test)]
mod tests {
    use zbus::{
        object_server::SignalEmitter,
        zvariant::{OwnedValue, Value},
    };

    use super::*;

    struct Settings;

    #[zbus::interface(name = "org.freedesktop.portal.Settings")]
    impl Settings {
        #[zbus(property(emits_changed_signal = "const"), name = "version")]
        fn version(&self) -> u32 {
            2
        }

        #[zbus(out_args("value"))]
        fn read(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
            if namespace != APPEARANCE_NAMESPACE {
                return Err(zbus::fdo::Error::Failed("unknown namespace".to_owned()));
            }
            match key {
                ACCENT_COLOR_SCHEME_KEY => OwnedValue::try_from(Value::from((f64::NAN, 0.0, 0.0)))
                    .map_err(|error| zbus::fdo::Error::Failed(error.to_string())),
                CONTRAST_KEY | REDUCED_MOTION_KEY => Ok(OwnedValue::from(0_u32)),
                _ => Err(zbus::fdo::Error::Failed("unknown setting".to_owned())),
            }
        }

        #[zbus(signal)]
        async fn setting_changed(
            emitter: &SignalEmitter<'_>,
            namespace: &str,
            key: &str,
            value: Value<'_>,
        ) -> zbus::Result<()>;
    }

    #[test]
    fn derives_accessible_dark_tokens_and_contrast_styles() {
        for (high_contrast, reduced_motion) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let appearance = Appearance::new(None, high_contrast, reduced_motion);
            let primary = appearance.theme.extended_palette().primary;
            assert_eq!(appearance.theme.palette().background, WINDOW_BG);
            assert_eq!(primary.base.color, FALLBACK_ACCENT);
            assert_eq!(appearance.high_contrast, high_contrast);
            assert_eq!(appearance.reduced_motion, reduced_motion);
            assert!(primary.strong.color.relative_contrast(WINDOW_BG) >= 4.5);
            assert_eq!(
                appearance
                    .text_input(text_input::Status::Focused { is_hovered: false })
                    .border
                    .width,
                if high_contrast { 3.0 } else { 2.0 }
            );
            assert_eq!(
                appearance
                    .pick_list(pick_list::Status::Opened { is_hovered: false })
                    .border
                    .width,
                if high_contrast { 3.0 } else { 2.0 }
            );
        }

        for invalid in [(f64::NAN, 0.0, 0.0), (-0.1, 0.0, 0.0), (0.0, 1.1, 0.0)] {
            assert!(valid_channels(invalid).is_none());
        }
        let accent = valid_channels((0.2, 0.3, 0.4)).unwrap();
        let appearance = Appearance::new(Some(accent), false, false);
        let primary = appearance.theme.extended_palette().primary;
        assert_eq!(primary.base.text, Color::WHITE);
        assert!(primary.strong.color.r >= accent.r);
        assert!(primary.strong.color.g >= accent.g);
        assert!(primary.strong.color.b >= accent.b);
        assert!((primary.weak.color.r - (SURFACE.r * 0.85 + accent.r * 0.15)).abs() < 1e-6);
        assert_eq!(
            Appearance::new(Some(Color::WHITE), false, false)
                .theme
                .extended_palette()
                .primary
                .base
                .text,
            DARK_ACCENT_TEXT
        );
        let middle = Color::from_rgb8(0x77, 0x77, 0x77);
        assert_eq!(foreground(middle), Color::BLACK);
        assert!(middle.relative_contrast(foreground(middle)) >= 4.5);
        assert_eq!(
            appearance
                .neutral_button(button::Status::Hovered)
                .background,
            Some(Background::Color(SURFACE_RAISED))
        );
        assert_eq!(
            appearance
                .primary_button(button::Status::Disabled)
                .border
                .color,
            BORDER
        );
    }

    #[tokio::test]
    #[ignore = "requires an isolated session bus"]
    async fn follows_portal_appearance_changes() {
        let server = zbus::connection::Builder::session()
            .unwrap()
            .name("org.freedesktop.portal.Desktop")
            .unwrap()
            .serve_at("/org/freedesktop/portal/desktop", Settings)
            .unwrap()
            .build()
            .await
            .unwrap();
        let mut updates = Box::pin(watch(zbus::Connection::session().await.unwrap()));

        let initial = updates.next().await.unwrap().unwrap();
        assert_eq!(
            initial.theme.extended_palette().primary.base.color,
            FALLBACK_ACCENT
        );
        assert!(!initial.high_contrast && !initial.reduced_motion);

        let interface = server
            .object_server()
            .interface::<_, Settings>("/org/freedesktop/portal/desktop")
            .await
            .unwrap();
        Settings::setting_changed(
            interface.signal_emitter(),
            APPEARANCE_NAMESPACE,
            ACCENT_COLOR_SCHEME_KEY,
            Value::from((0.2_f64, 0.3_f64, 0.4_f64)),
        )
        .await
        .unwrap();
        let accent = updates.next().await.unwrap().unwrap();
        assert_eq!(
            accent.theme.extended_palette().primary.base.color,
            Color::from_rgb(0.2, 0.3, 0.4)
        );

        Settings::setting_changed(
            interface.signal_emitter(),
            APPEARANCE_NAMESPACE,
            CONTRAST_KEY,
            Value::from(1_u32),
        )
        .await
        .unwrap();
        let contrast = updates.next().await.unwrap().unwrap();
        assert!(contrast.high_contrast);

        Settings::setting_changed(
            interface.signal_emitter(),
            APPEARANCE_NAMESPACE,
            CONTRAST_KEY,
            Value::from("invalid"),
        )
        .await
        .unwrap();
        Settings::setting_changed(
            interface.signal_emitter(),
            APPEARANCE_NAMESPACE,
            REDUCED_MOTION_KEY,
            Value::from(1_u32),
        )
        .await
        .unwrap();
        let motion = updates.next().await.unwrap().unwrap();
        assert!(motion.high_contrast && motion.reduced_motion);
    }
}
