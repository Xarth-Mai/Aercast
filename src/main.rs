use iced::futures::channel::mpsc::UnboundedSender;
use media::VideoPlan;

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

const HELP: &str = "Usage: aercast [help|version|new]

Commands:
  help     Show this help
  version  Show version and build information
  new      Start a temporary independent instance

Without a command, start or activate the default instance.
New instances load saved settings but never save changes.
Change Network settings if the port is busy.";

#[derive(Debug, PartialEq)]
enum Launch {
    Default,
    Help,
    Version,
    New,
}

fn parse_arguments(mut args: impl Iterator<Item = std::ffi::OsString>) -> Result<Launch> {
    let command = match args.next() {
        None => Launch::Default,
        Some(value) => match value.to_str() {
            Some("help") => Launch::Help,
            Some("version") => Launch::Version,
            Some("new") => Launch::New,
            _ => return Err(format!("Unknown command: {}", value.to_string_lossy()).into()),
        },
    };
    if let Some(extra) = args.next() {
        return Err(format!("Unexpected argument: {}", extra.to_string_lossy()).into());
    }
    Ok(command)
}

fn version_label(version: &str, profile: &str) -> String {
    let suffix = if profile == "debug" { "+dev" } else { "" };
    format!("v{version}{suffix}")
}

fn main() -> Result<()> {
    match parse_arguments(std::env::args_os().skip(1)) {
        Ok(Launch::Help) => println!("{HELP}"),
        Ok(Launch::Version) => println!(
            "Aercast {}\nTarget: {}\nProfile: {}\nCompiler: {}",
            version_label(env!("CARGO_PKG_VERSION"), env!("AERCAST_PROFILE")),
            env!("AERCAST_TARGET"),
            env!("AERCAST_PROFILE"),
            env!("AERCAST_RUSTC"),
        ),
        Ok(Launch::Default) => return ui::run(false),
        Ok(Launch::New) => return ui::run(true),
        Err(error) => {
            eprintln!("{error}\n\n{HELP}");
            std::process::exit(2);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_distinguishes_dev_and_release() {
        assert_eq!(version_label("0.1.6", "debug"), "v0.1.6+dev");
        assert_eq!(version_label("0.1.6", "release"), "v0.1.6");
    }

    #[test]
    fn command_line_accepts_only_one_known_word() {
        for (args, expected) in [
            (vec![], Launch::Default),
            (vec!["help"], Launch::Help),
            (vec!["version"], Launch::Version),
            (vec!["new"], Launch::New),
        ] {
            assert_eq!(
                parse_arguments(args.into_iter().map(Into::into)).unwrap(),
                expected
            );
        }
        for args in [
            vec!["unknown"],
            vec!["-h"],
            vec!["--help"],
            vec!["-v"],
            vec!["--version"],
            vec!["-n"],
            vec!["--new"],
            vec!["help", "new"],
            vec!["version", "version"],
            vec!["new", "extra"],
        ] {
            assert!(parse_arguments(args.into_iter().map(Into::into)).is_err());
        }
        use std::os::unix::ffi::OsStringExt;
        assert!(parse_arguments([std::ffi::OsString::from_vec(vec![0xff])].into_iter()).is_err());
    }
}
