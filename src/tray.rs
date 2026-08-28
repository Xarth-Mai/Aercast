use std::{io::Cursor, sync::LazyLock};

use iced::futures::channel::mpsc::UnboundedSender;
use ksni::{MenuItem, ToolTip, TrayMethods as _, menu::StandardItem};
use tokio::sync::watch;

use super::{Message, Phase, TrayState};

const ICON_SIZE: usize = 64;

pub(super) async fn run(
    messages: UnboundedSender<Message>,
    mut state: watch::Receiver<TrayState>,
) -> Result<(), ksni::Error> {
    let handle = AercastTray {
        messages,
        state: state.clone(),
    }
    .assume_sni_available(true)
    .spawn()
    .await?;
    while state.changed().await.is_ok() {
        let _ = handle.update(|_| ()).await;
    }
    let _ = handle.shutdown().await;
    Ok(())
}

struct AercastTray {
    messages: UnboundedSender<Message>,
    state: watch::Receiver<TrayState>,
}

impl AercastTray {
    fn action(label: &str, message: Message) -> MenuItem<Self> {
        StandardItem {
            label: label.to_owned(),
            activate: Box::new(move |tray: &mut Self| {
                let _ = tray.messages.unbounded_send(message.clone());
            }),
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for AercastTray {
    fn id(&self) -> String {
        "aercast".to_owned()
    }

    fn title(&self) -> String {
        "Aercast".to_owned()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Aercast".to_owned(),
            ..ToolTip::default()
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![tray_icon().clone()]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.messages.unbounded_send(Message::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let state = self.state.borrow();
        let mut menu = vec![
            StandardItem {
                label: if state.phase == Phase::Sharing && state.online_viewers > 0 {
                    format!("Status: Sharing: {}", state.online_viewers)
                } else {
                    format!("Status: {}", phase_label(&state.phase))
                },
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            Self::action("Show", Message::Show),
        ];
        match &state.phase {
            Phase::Waiting => menu.push(Self::action("Start", Message::Start)),
            Phase::Selecting => menu.push(Self::action("Stop", Message::End)),
            Phase::Sharing => menu.extend([
                Self::action("Copy Link", Message::Copy),
                Self::action("Stop", Message::End),
            ]),
            Phase::Starting | Phase::NetworkError(_) | Phase::Ending | Phase::Error(_) => {}
        }
        menu.extend([MenuItem::Separator, Self::action("Quit", Message::Quit)]);
        menu
    }

    fn menu_about_to_show(&mut self) {
        // Overriding ksni's default makes it rebuild from the latest watch value.
    }
}

fn phase_label(phase: &Phase) -> &'static str {
    match phase {
        Phase::Starting => "Starting",
        Phase::NetworkError(_) => "Network unavailable",
        Phase::Waiting => "Ready",
        Phase::Selecting => "Choosing a source",
        Phase::Sharing => "Sharing",
        Phase::Ending => "Ending share",
        Phase::Error(_) => "Error",
    }
}

fn tray_icon() -> &'static ksni::Icon {
    static ICON: LazyLock<ksni::Icon> = LazyLock::new(|| {
        let decoder = png::Decoder::new(Cursor::new(include_bytes!("../assets/aercast-icon.png")));
        let mut reader = decoder.read_info().expect("bundled tray icon is valid PNG");
        let mut source = vec![0; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut source)
            .expect("bundled tray icon decodes");
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        let width = info.width as usize;
        let height = info.height as usize;
        let mut data = Vec::with_capacity(ICON_SIZE * ICON_SIZE * 4);
        for y in 0..ICON_SIZE {
            for x in 0..ICON_SIZE {
                let offset = ((y * height / ICON_SIZE) * width + x * width / ICON_SIZE) * 4;
                let pixel = &source[offset..offset + 4];
                data.extend_from_slice(&[pixel[3], pixel[0], pixel[1], pixel[2]]);
            }
        }
        ksni::Icon {
            width: ICON_SIZE as i32,
            height: ICON_SIZE as i32,
            data,
        }
    });
    &ICON
}

#[cfg(test)]
mod tests {
    use futures_util::{FutureExt as _, StreamExt as _};
    use ksni::Tray as _;

    use super::*;

    #[test]
    fn menu_tracks_phase_and_uses_the_bundled_icon() {
        let icon = tray_icon();
        assert_eq!((icon.width, icon.height, icon.data.len()), (64, 64, 16_384));
        assert!(
            icon.data
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| pixel[0] != 0)
                .count()
                > 700
        );
        let (messages, mut received) = iced::futures::channel::mpsc::unbounded();
        let (updates, state) = watch::channel(TrayState {
            phase: Phase::Waiting,
            online_viewers: 0,
        });
        let mut tray = AercastTray { messages, state };

        let labels = |tray: &AercastTray| {
            tray.menu()
                .into_iter()
                .filter_map(|item| match item {
                    MenuItem::Standard(item) => Some(item.label),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(tray.title(), "Aercast");
        assert_eq!(tray.tool_tip().title, "Aercast");
        assert_eq!(labels(&tray), ["Status: Ready", "Show", "Start", "Quit"]);
        updates.send_replace(TrayState {
            phase: Phase::Sharing,
            online_viewers: 0,
        });
        assert_eq!(
            labels(&tray),
            ["Status: Sharing", "Show", "Copy Link", "Stop", "Quit"]
        );
        updates.send_replace(TrayState {
            phase: Phase::Sharing,
            online_viewers: 2,
        });
        assert_eq!(
            labels(&tray),
            ["Status: Sharing: 2", "Show", "Copy Link", "Stop", "Quit"]
        );
        tray.activate(0, 0);
        assert!(matches!(
            received.next().now_or_never(),
            Some(Some(Message::Show))
        ));
    }

    #[tokio::test]
    #[ignore = "requires an isolated session bus"]
    async fn service_stops_without_a_status_notifier_watcher() {
        let (messages, _) = iced::futures::channel::mpsc::unbounded();
        let (updates, state) = watch::channel(TrayState {
            phase: Phase::Waiting,
            online_viewers: 0,
        });
        drop(updates);
        tokio::time::timeout(std::time::Duration::from_secs(2), run(messages, state))
            .await
            .unwrap()
            .unwrap();
    }
}
