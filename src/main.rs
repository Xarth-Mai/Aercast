use iced::futures::channel::mpsc::UnboundedSender;
use media::VideoPlan;
use std::io;

mod accessibility;
mod appearance;
mod audio;
mod media;
mod notification;
mod portal;
mod settings;
mod share_session;
mod tray;
mod ui;
mod web;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;
type Events = UnboundedSender<HostEvent>;

#[derive(Clone, Debug, PartialEq)]
enum Command {
    Start(ShareSettings),
    Apply(ShareSettings),
    Network(settings::Settings),
    End,
    Refresh(bool),
    Disconnect(u64),
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
struct AudioSettings {
    enabled: bool,
    bitrate_kbps: u32,
    exclude_communication: bool,
    exclusions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct ShareSettings {
    audio: AudioSettings,
    video: VideoPlan,
}

enum ShareStop {
    Apply(ShareSettings),
    Sleep,
    Wake,
    End,
    Quit,
    PortalClosed,
    Failed(Error),
}

#[derive(Clone, Debug)]
enum HostEvent {
    Waiting(String),
    NetworkUnavailable(String),
    Source(&'static str),
    Link(String),
    ConfirmRefresh,
    Sharing(ShareSettings),
    MediaIdle(bool),
    ApplyFailed(String),
    Ending,
    Viewers(Vec<web::Viewer>),
    NetworkApplied(std::result::Result<settings::Settings, String>),
    Stopped(std::result::Result<(), String>),
}

fn main() -> Result<()> {
    validate_arguments(std::env::args().skip(1))?;
    ui::run()
}

fn validate_arguments(mut args: impl Iterator<Item = String>) -> io::Result<()> {
    if args.next().is_some() {
        Err(io::Error::other("usage: aercast"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_line_controls_are_rejected() {
        assert!(validate_arguments(std::iter::empty()).is_ok());
        for arguments in [
            vec!["--exclude".to_owned(), "Discord".to_owned()],
            vec!["--monitor".to_owned()],
            vec!["--bind".to_owned(), "127.0.0.1:9000".to_owned()],
        ] {
            assert!(validate_arguments(arguments.into_iter()).is_err());
        }
    }
}
