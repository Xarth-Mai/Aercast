use std::{
    cell::Cell,
    future::Future,
    io,
    net::SocketAddr,
    os::fd::AsRawFd,
    pin::Pin,
    time::{Duration, Instant},
};

use ashpd::{
    PortalError,
    desktop::{
        PersistMode, ResponseError,
        screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
    },
};
use futures_util::{FutureExt, StreamExt};
use gst::prelude::*;
use gst_app::AppSinkCallbacks;
use iced::{
    Element, Length, Task, Theme, clipboard,
    futures::channel::mpsc::{Receiver, Sender, UnboundedSender},
    widget::{
        button, checkbox, column, container, row, rule, scrollable, space, svg, text, text_input,
    },
    window,
};
use socket2::SockRef;
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot, watch},
};

mod accessibility;
mod appearance;
mod audio;
mod notification;
mod settings;
mod tray;
mod web;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;
type Events = UnboundedSender<HostEvent>;
const INSTANCE_NAME: &str = "org.aercast.Aercast";
const INSTANCE_PATH: &str = "/org/aercast/Aercast";
const SHARE_SCROLL_ID: &str = "viewers";
const SETTINGS_SCROLL_ID: &str = "settings";

#[derive(Clone, Debug, PartialEq)]
enum Command {
    Start(ShareSettings),
    Apply(AudioSettings),
    Network(settings::Settings),
    End,
    Refresh(bool),
    Disconnect(u64),
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
struct AudioSettings {
    enabled: bool,
    exclusions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct ShareSettings {
    audio: AudioSettings,
    video: VideoPlan,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct VideoPlan {
    settings: settings::VideoSettings,
    encoder: Encoder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoder {
    VaApi,
    X264,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Quality {
    P720,
    P1080,
    P1440,
    Custom,
}

const QUALITY_OPTIONS: [Quality; 4] = [
    Quality::P720,
    Quality::P1080,
    Quality::P1440,
    Quality::Custom,
];
const FPS_OPTIONS: [u32; 3] = [30, 60, 120];

impl Quality {
    fn from_video(video: settings::VideoSettings) -> Self {
        QUALITY_OPTIONS
            .into_iter()
            .find(|preset| preset.video(video.encoder) == Some(video))
            .unwrap_or(Self::Custom)
    }

    fn video(self, encoder: settings::VideoEncoder) -> Option<settings::VideoSettings> {
        let (width, height, bitrate_mbps) = match self {
            Self::P720 => (1280, 720, 6),
            Self::P1080 => (1920, 1080, 12),
            Self::P1440 => (2560, 1440, 24),
            Self::Custom => return None,
        };
        Some(settings::VideoSettings {
            width,
            height,
            fps: 60,
            bitrate_mbps: Some(bitrate_mbps),
            encoder,
        })
    }
}

impl std::fmt::Display for Quality {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::P720 => "720p60 / 6 Mbps",
            Self::P1080 => "1080p60 / 12 Mbps",
            Self::P1440 => "1440p60 / 24 Mbps",
            Self::Custom => "Custom",
        })
    }
}

enum ShareStop {
    Apply(AudioSettings),
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
    Sharing(AudioSettings),
    Ending,
    Viewers(Vec<web::Viewer>),
    NetworkApplied(std::result::Result<settings::Settings, String>),
    Stopped(std::result::Result<(), String>),
}

#[derive(Clone, Debug)]
enum Message {
    Host(HostEvent),
    Start,
    End,
    Copy,
    Refresh,
    Disconnect(u64),
    ConfirmRefresh,
    CancelRefresh,
    Show,
    Quit,
    ConfirmQuit,
    CancelQuit,
    QuitQueued(bool),
    BusClosed,
    TrayStopped(std::result::Result<(), String>),
    ApplySystemAudio,
    Settings(bool),
    SystemAudio(bool),
    AudioExclusion(String, bool),
    DeleteAudioExclusion(String),
    RefreshAudioApplications,
    AudioApplications(std::result::Result<Vec<audio::PlaybackApplication>, String>),
    AddAudioExclusion(audio::PlaybackApplication),
    Notifications(bool),
    Notified(Option<String>),
    Appearance(std::result::Result<appearance::Appearance, String>),
    NetworkAddress(String),
    NetworkPort(String),
    ShareBaseUrl(String),
    ApplyNetwork,
    VideoPreset(Quality),
    VideoWidth(String),
    VideoHeight(String),
    VideoFps(u32),
    VideoBitrate(String),
    VideoEncoder(settings::VideoEncoder),
    SaveVideo,
    Focus(bool),
    RevealFocus(f32),
    Tick,
    Close(window::Id),
    Closed(window::Id),
}

struct Activation(Sender<Message>);

#[zbus::interface(name = "org.aercast.Aercast")]
impl Activation {
    fn show(&mut self) -> zbus::fdo::Result<()> {
        match self.0.try_send(Message::Show) {
            Err(error) if !error.is_full() => {
                Err(zbus::fdo::Error::Failed("Aercast is exiting".to_owned()))
            }
            _ => Ok(()),
        }
    }
}

fn claim_instance(
    activation: Sender<Message>,
    name: &str,
) -> Result<Option<zbus::blocking::Connection>> {
    let connection = zbus::blocking::connection::Builder::session()?
        .serve_at(INSTANCE_PATH, Activation(activation))?
        .build()?;
    match connection.request_name_with_flags(name, zbus::fdo::RequestNameFlags::DoNotQueue.into()) {
        Ok(_) => Ok(Some(connection)),
        Err(zbus::Error::NameTaken) => {
            let proxy =
                zbus::blocking::Proxy::new(&connection, name, INSTANCE_PATH, INSTANCE_NAME)?;
            let _: () = proxy.call("Show", &())?;
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Starting,
    NetworkError(String),
    Waiting,
    Selecting,
    Sharing,
    Ending,
    Error(String),
}

struct App {
    phase: Phase,
    link: String,
    viewers: Vec<web::Viewer>,
    commands: Option<mpsc::Sender<Command>>,
    window: Option<window::Id>,
    confirm_refresh: bool,
    confirm_quit: bool,
    settings: settings::Settings,
    settings_open: bool,
    settings_error: Option<String>,
    audio_candidates: Vec<audio::PlaybackApplication>,
    audio_scanning: bool,
    audio_scan_error: Option<String>,
    video_error: Option<String>,
    video_edit_error: Option<String>,
    video_preset: Quality,
    video_width: String,
    video_height: String,
    video_fps: u32,
    video_bitrate: String,
    video_encoder: settings::VideoEncoder,
    appearance: appearance::Appearance,
    approved_source: Option<&'static str>,
    active_audio: Option<AudioSettings>,
    applying_audio: Option<AudioSettings>,
    network_address: String,
    network_port: String,
    share_base_url: String,
    applying_network: bool,
    notifications: UnboundedSender<notification::Kind>,
    tray_updates: Option<watch::Sender<Phase>>,
    tray_stopped: bool,
    host_stopped: bool,
    quitting: bool,
}

type Server = tokio::task::JoinHandle<io::Result<()>>;

struct RunningServer {
    address: SocketAddr,
    shutdown: oneshot::Sender<()>,
    task: Server,
}
const STALLED_CLIENT_TIMEOUT: Duration = Duration::from_secs(15);
const MEDIA_RECOVERY_DELAY: Duration = Duration::from_millis(500);
const MAX_MEDIA_RECOVERIES: u8 = 3;

fn main() -> Result<()> {
    let (activation, activations) = iced::futures::channel::mpsc::channel(0);
    let Some(instance) = claim_instance(activation, INSTANCE_NAME)? else {
        return Ok(());
    };
    validate_arguments(std::env::args().skip(1))?;
    let settings = settings::Settings::load()?;
    gst::init()?;
    let instance = Cell::new(Some((activations, instance.into_inner())));

    iced::daemon(
        move || {
            let (activations, instance) = instance.take().expect("Aercast daemon booted twice");
            boot(settings.clone(), activations, instance)
        },
        update,
        view,
    )
    .title("Aercast")
    .settings(iced::Settings {
        default_text_size: 13.0.into(),
        ..iced::Settings::default()
    })
    .theme(|app: &App, _| app.appearance.theme.clone())
    .subscription(|app| {
        let tick = if app.window.is_some() && app.viewers.iter().any(web::Viewer::online) {
            iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick)
        } else {
            iced::Subscription::none()
        };
        iced::Subscription::batch([
            window::close_requests().map(Message::Close),
            window::close_events().map(Message::Closed),
            iced::keyboard::listen().filter_map(|event| match event {
                iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
                    modifiers,
                    repeat: false,
                    ..
                } => Some(Message::Focus(modifiers.shift())),
                _ => None,
            }),
            tick,
        ])
    })
    .run()?;
    Ok(())
}

fn boot(
    settings: settings::Settings,
    activations: Receiver<Message>,
    instance: zbus::Connection,
) -> (App, Task<Message>) {
    let (events, incoming) = iced::futures::channel::mpsc::unbounded();
    let (notifications, notification_requests) = iced::futures::channel::mpsc::unbounded();
    let (tray_messages, tray_events) = iced::futures::channel::mpsc::unbounded();
    let (commands, command_receiver) = mpsc::channel(8);
    let host_settings = settings.clone();
    let network_address = settings.listen_address.to_string();
    let network_port = settings.listen_port.to_string();
    let share_base_url = settings.share_base_url.clone().unwrap_or_default();
    let video = settings.video;
    let video_error = video_plan(&settings.video)
        .err()
        .map(|error| format!("Video quality unavailable: {error}"));
    let (tray_updates, tray_state) = watch::channel(Phase::Starting);
    let app = App {
        phase: Phase::Starting,
        link: String::new(),
        viewers: Vec::new(),
        commands: Some(commands),
        window: None,
        confirm_refresh: false,
        confirm_quit: false,
        settings,
        settings_open: false,
        settings_error: None,
        audio_candidates: Vec::new(),
        audio_scanning: false,
        audio_scan_error: None,
        video_error,
        video_edit_error: None,
        video_preset: Quality::from_video(video),
        video_width: video.width.to_string(),
        video_height: video.height.to_string(),
        video_fps: video.fps,
        video_bitrate: video
            .bitrate_mbps
            .map_or_else(String::new, |bitrate| bitrate.to_string()),
        video_encoder: video.encoder,
        appearance: appearance::Appearance::default(),
        approved_source: None,
        active_audio: None,
        applying_audio: None,
        network_address,
        network_port,
        share_base_url,
        applying_network: false,
        notifications,
        tray_updates: Some(tray_updates),
        tray_stopped: false,
        host_stopped: false,
        quitting: false,
    };
    (
        app,
        Task::batch([
            Task::done(Message::Show),
            Task::run(activations, |message| message),
            Task::run(appearance::watch(instance.clone()), Message::Appearance),
            Task::run(
                notification::worker(instance.clone(), notification_requests),
                |result| Message::Notified(result.err().map(|error| error.to_string())),
            ),
            Task::run(tray_events, |message| message),
            Task::perform(async move { instance.closed().await }, |_| {
                Message::BusClosed
            }),
            Task::perform(tray::run(tray_messages, tray_state), |result| {
                Message::TrayStopped(result.map_err(|error| error.to_string()))
            }),
            Task::run(incoming, Message::Host),
            Task::perform(
                run_host(host_settings, events, command_receiver),
                |result| {
                    Message::Host(HostEvent::Stopped(
                        result.map_err(|error| error.to_string()),
                    ))
                },
            ),
        ]),
    )
}

fn update(app: &mut App, message: Message) -> Task<Message> {
    let previous_phase = app.phase.clone();
    let was_sharing = app.active_audio.is_some();
    let task = update_app(app, message);
    if let Some(updates) = &app.tray_updates {
        updates.send_if_modified(|current| {
            if *current == app.phase {
                false
            } else {
                current.clone_from(&app.phase);
                true
            }
        });
    }
    if app.window.is_none()
        && app.settings.notifications
        && !app.quitting
        && let Some(kind) = notification_kind(&previous_phase, &app.phase, was_sharing)
        && app.notifications.unbounded_send(kind).is_err()
    {
        eprintln!("Notification worker unavailable");
    }
    task
}

fn notification_kind(
    previous: &Phase,
    current: &Phase,
    was_sharing: bool,
) -> Option<notification::Kind> {
    match (previous, current) {
        (Phase::Selecting, Phase::Sharing) => Some(notification::Kind::Started),
        (_, Phase::Waiting) if was_sharing => Some(notification::Kind::Stopped),
        (_, Phase::NetworkError(_) | Phase::Error(_))
            if *previous != Phase::Starting && previous != current =>
        {
            Some(notification::Kind::Error)
        }
        _ => None,
    }
}

fn update_app(app: &mut App, message: Message) -> Task<Message> {
    if app.quitting
        && !matches!(
            &message,
            Message::QuitQueued(_)
                | Message::TrayStopped(_)
                | Message::Close(_)
                | Message::Closed(_)
                | Message::Host(HostEvent::Stopped(_))
        )
    {
        return Task::none();
    }
    match message {
        Message::Focus(previous) => {
            let focus = accessibility::move_focus(previous);
            return focus.chain(accessibility::reveal_focused(iced::widget::Id::new(
                if app.settings_open {
                    SETTINGS_SCROLL_ID
                } else {
                    SHARE_SCROLL_ID
                },
            )));
        }
        Message::RevealFocus(delta) => {
            return iced::widget::operation::scroll_by(
                iced::widget::Id::new(if app.settings_open {
                    SETTINGS_SCROLL_ID
                } else {
                    SHARE_SCROLL_ID
                }),
                iced::widget::operation::AbsoluteOffset { x: 0.0, y: delta },
            );
        }
        Message::Start if app.phase != Phase::Waiting || app.applying_network => {}
        Message::Start => {
            let video = match video_plan(&app.settings.video) {
                Ok(video) => video,
                Err(error) => {
                    app.video_error = Some(format!("Video quality unavailable: {error}"));
                    return Task::none();
                }
            };
            let share = ShareSettings {
                audio: audio_settings(&app.settings),
                video,
            };
            if send_command(app, Command::Start(share)) {
                app.video_error = None;
                app.phase = Phase::Selecting;
            }
        }
        Message::End if !matches!(app.phase, Phase::Selecting | Phase::Sharing) => {}
        Message::End => {
            app.confirm_refresh = false;
            app.confirm_quit = false;
            app.applying_audio = None;
            if send_command(app, Command::End) {
                app.phase = Phase::Ending;
            }
        }
        Message::Copy => return clipboard::write(app.link.clone()),
        Message::Refresh => {
            let _ = send_command(app, Command::Refresh(false));
        }
        Message::Disconnect(key) => {
            let _ = send_command(app, Command::Disconnect(key));
        }
        Message::ConfirmRefresh => {
            app.confirm_refresh = false;
            let _ = send_command(app, Command::Refresh(true));
        }
        Message::CancelRefresh => app.confirm_refresh = false,
        Message::Show => return show_window(app),
        Message::Quit => {
            if app.phase == Phase::Sharing {
                app.confirm_refresh = false;
                app.confirm_quit = true;
                app.settings_open = false;
                return show_window(app);
            }
            return begin_quit(app);
        }
        Message::ConfirmQuit if app.confirm_quit => return begin_quit(app),
        Message::ConfirmQuit => {}
        Message::CancelQuit => app.confirm_quit = false,
        Message::QuitQueued(true) => {}
        Message::QuitQueued(false) => {
            app.host_stopped = true;
            return finish_quit(app);
        }
        Message::BusClosed => return begin_quit(app),
        Message::TrayStopped(result) => {
            app.tray_updates = None;
            app.tray_stopped = true;
            if let Err(error) = result {
                eprintln!("Tray unavailable: {error}");
            }
            if app.quitting {
                return finish_quit(app);
            }
        }
        Message::ApplySystemAudio => {
            let audio = audio_settings(&app.settings);
            if app.phase == Phase::Sharing
                && app
                    .active_audio
                    .as_ref()
                    .is_some_and(|active| active != &audio)
                && app.applying_audio.is_none()
                && send_command(app, Command::Apply(audio.clone()))
            {
                app.applying_audio = Some(audio);
            }
        }
        Message::Settings(open) => {
            app.settings_open = open;
            app.settings_error = None;
            app.video_edit_error = None;
            if open {
                return scan_audio_applications(app);
            }
        }
        Message::SystemAudio(_)
        | Message::AudioExclusion(..)
        | Message::DeleteAudioExclusion(_)
        | Message::AddAudioExclusion(_)
        | Message::Notifications(_)
            if app.applying_network => {}
        Message::SystemAudio(system_audio) => {
            save_settings(app, |settings| settings.system_audio = system_audio);
        }
        Message::AudioExclusion(identity, enabled) => {
            save_settings(app, |settings| {
                if let Some(exclusion) = settings
                    .audio_exclusions
                    .iter_mut()
                    .find(|exclusion| exclusion.identity == identity)
                {
                    exclusion.enabled = enabled;
                }
            });
        }
        Message::DeleteAudioExclusion(identity) => {
            save_settings(app, |settings| {
                settings
                    .audio_exclusions
                    .retain(|exclusion| exclusion.identity != identity);
            });
        }
        Message::RefreshAudioApplications => return scan_audio_applications(app),
        Message::AudioApplications(result) => {
            app.audio_scanning = false;
            match result {
                Ok(applications) => app.audio_candidates = applications,
                Err(error) => app.audio_scan_error = Some(error),
            }
        }
        Message::AddAudioExclusion(application) => {
            if !app
                .settings
                .audio_exclusions
                .iter()
                .any(|exclusion| exclusion.identity == application.identity)
            {
                save_settings(app, |settings| {
                    settings.audio_exclusions.push(settings::AudioExclusion {
                        label: application.label,
                        identity: application.identity,
                        enabled: true,
                    });
                });
            }
        }
        Message::Notifications(notifications) => {
            save_settings(app, |settings| settings.notifications = notifications);
        }
        Message::Notified(error) => {
            if let Some(error) = error {
                eprintln!("Notification unavailable: {error}");
            }
        }
        Message::Appearance(Ok(appearance)) if appearance != app.appearance => {
            app.appearance = appearance;
        }
        Message::Appearance(Ok(_)) => {}
        Message::Appearance(Err(error)) => eprintln!("Appearance Portal unavailable: {error}"),
        Message::NetworkAddress(address) => {
            app.network_address = address;
            app.settings_error = None;
        }
        Message::NetworkPort(port) => {
            app.network_port = port;
            app.settings_error = None;
        }
        Message::ShareBaseUrl(base_url) => {
            app.share_base_url = base_url;
            app.settings_error = None;
        }
        Message::ApplyNetwork => {
            if matches!(&app.phase, Phase::Waiting | Phase::NetworkError(_))
                && !app.applying_network
            {
                match app.settings.with_network(
                    &app.network_address,
                    &app.network_port,
                    &app.share_base_url,
                ) {
                    Ok(settings) => {
                        if send_command(app, Command::Network(settings)) {
                            app.applying_network = true;
                            app.settings_error = None;
                        }
                    }
                    Err(error) => app.settings_error = Some(error.to_string()),
                }
            }
        }
        Message::VideoPreset(preset) => {
            if let Some(video) = preset.video(app.video_encoder) {
                set_video_draft(app, video);
            } else if app.video_preset != Quality::Custom {
                app.video_bitrate.clear();
            }
            app.video_preset = preset;
            app.video_edit_error = None;
        }
        Message::VideoWidth(width) => {
            app.video_preset = Quality::Custom;
            app.video_width = width;
            app.video_edit_error = None;
        }
        Message::VideoHeight(height) => {
            app.video_preset = Quality::Custom;
            app.video_height = height;
            app.video_edit_error = None;
        }
        Message::VideoFps(fps) => {
            app.video_preset = Quality::Custom;
            app.video_fps = fps;
            app.video_edit_error = None;
        }
        Message::VideoBitrate(bitrate) => {
            app.video_preset = Quality::Custom;
            app.video_bitrate = bitrate;
            app.video_edit_error = None;
        }
        Message::VideoEncoder(encoder) => {
            app.video_encoder = encoder;
            app.video_edit_error = None;
        }
        Message::SaveVideo if app.applying_network => {}
        Message::SaveVideo => {
            let result = app
                .settings
                .with_video(
                    &app.video_width,
                    &app.video_height,
                    app.video_fps,
                    &app.video_bitrate,
                    app.video_encoder,
                )
                .map_err(Error::from)
                .and_then(|settings| {
                    video_plan(&settings.video)?;
                    settings.save()?;
                    Ok(settings)
                });
            match result {
                Ok(settings) => {
                    let video = settings.video;
                    app.settings = settings;
                    app.settings_error = None;
                    app.video_error = None;
                    app.video_edit_error = None;
                    set_video_draft(app, video);
                }
                Err(error) => {
                    app.video_edit_error = Some(format!("Video quality unchanged: {error}"));
                }
            }
        }
        Message::Tick => {}
        Message::Close(id) => {
            if app.window.take_if(|window| *window == id).is_some() {
                return window::close(id);
            }
        }
        Message::Closed(id) => app.window = app.window.filter(|window| *window != id),
        Message::Host(event) => match event {
            HostEvent::NetworkUnavailable(error) => {
                app.link.clear();
                app.approved_source = None;
                app.applying_network = false;
                app.confirm_quit = false;
                app.settings_error = Some(error.clone());
                app.phase = Phase::NetworkError(error);
            }
            HostEvent::Waiting(link) => {
                app.link = link;
                app.confirm_refresh = false;
                app.confirm_quit = false;
                app.approved_source = None;
                app.active_audio = None;
                app.applying_audio = None;
                app.phase = Phase::Waiting;
            }
            HostEvent::Source(source) if app.phase == Phase::Selecting => {
                app.approved_source = Some(source);
            }
            HostEvent::Link(link) => {
                app.link = link;
                app.viewers.clear();
                app.confirm_refresh = false;
            }
            HostEvent::ConfirmRefresh if matches!(app.phase, Phase::Waiting | Phase::Sharing) => {
                app.confirm_refresh = true;
            }
            HostEvent::Sharing(audio) if matches!(app.phase, Phase::Selecting | Phase::Sharing) => {
                if app.applying_audio.as_ref() == Some(&audio) {
                    app.applying_audio = None;
                }
                app.active_audio = Some(audio);
                app.phase = Phase::Sharing;
            }
            HostEvent::Ending => {
                app.confirm_refresh = false;
                app.confirm_quit = false;
                app.applying_audio = None;
                app.phase = Phase::Ending;
            }
            HostEvent::Viewers(viewers) => app.viewers = viewers,
            HostEvent::NetworkApplied(result) => {
                app.applying_network = false;
                match result {
                    Ok(settings) => {
                        app.network_address = settings.listen_address.to_string();
                        app.network_port = settings.listen_port.to_string();
                        app.share_base_url = settings.share_base_url.clone().unwrap_or_default();
                        app.settings = settings;
                        app.settings_error = None;
                    }
                    Err(error) => app.settings_error = Some(error),
                }
            }
            HostEvent::Stopped(result) => {
                app.commands = None;
                app.host_stopped = true;
                app.viewers.clear();
                app.confirm_refresh = false;
                app.confirm_quit = false;
                app.approved_source = None;
                app.active_audio = None;
                app.applying_audio = None;
                app.applying_network = false;
                if app.quitting {
                    if let Err(error) = result {
                        eprintln!("Failed to stop Aercast: {error}");
                    }
                    return finish_quit(app);
                }
                match result {
                    Ok(()) => {
                        app.quitting = true;
                        app.phase = Phase::Ending;
                        app.tray_updates.take();
                        return finish_quit(app);
                    }
                    Err(error) => app.phase = Phase::Error(error),
                }
            }
            HostEvent::Source(_) | HostEvent::Sharing(_) | HostEvent::ConfirmRefresh => {}
        },
    }
    Task::none()
}

fn send_command(app: &mut App, command: Command) -> bool {
    if app
        .commands
        .as_ref()
        .is_some_and(|commands| commands.try_send(command).is_ok())
    {
        true
    } else {
        app.confirm_refresh = false;
        app.confirm_quit = false;
        app.phase = Phase::Error("Host control is unavailable".to_owned());
        false
    }
}

fn save_settings(app: &mut App, edit: impl FnOnce(&mut settings::Settings)) {
    let mut settings = app.settings.clone();
    edit(&mut settings);
    match settings.save() {
        Ok(()) => {
            app.settings = settings;
            app.settings_error = None;
        }
        Err(error) => app.settings_error = Some(error.to_string()),
    }
}

fn scan_audio_applications(app: &mut App) -> Task<Message> {
    if app.audio_scanning {
        return Task::none();
    }
    app.audio_scanning = true;
    app.audio_candidates.clear();
    app.audio_scan_error = None;
    Task::perform(audio::active_applications(), Message::AudioApplications)
}

fn audio_settings(settings: &settings::Settings) -> AudioSettings {
    AudioSettings {
        enabled: settings.system_audio,
        exclusions: settings
            .audio_exclusions
            .iter()
            .filter(|exclusion| exclusion.enabled)
            .map(|exclusion| exclusion.identity.clone())
            .collect(),
    }
}

fn set_video_draft(app: &mut App, video: settings::VideoSettings) {
    app.video_preset = Quality::from_video(video);
    app.video_width = video.width.to_string();
    app.video_height = video.height.to_string();
    app.video_fps = video.fps;
    app.video_bitrate = video
        .bitrate_mbps
        .map_or_else(String::new, |bitrate| bitrate.to_string());
    app.video_encoder = video.encoder;
}

fn begin_quit(app: &mut App) -> Task<Message> {
    app.confirm_quit = false;
    app.quitting = true;
    app.phase = Phase::Ending;
    app.tray_updates.take();
    match app.commands.take() {
        Some(commands) => Task::perform(queue_quit(commands), Message::QuitQueued),
        None => {
            app.host_stopped = true;
            finish_quit(app)
        }
    }
}

fn finish_quit(app: &App) -> Task<Message> {
    if app.host_stopped && app.tray_stopped {
        iced::exit()
    } else {
        Task::none()
    }
}

async fn queue_quit(commands: mpsc::Sender<Command>) -> bool {
    commands.send(Command::Quit).await.is_ok()
}

fn show_window(app: &mut App) -> Task<Message> {
    if let Some(id) = app.window {
        return window::gain_focus(id);
    }
    let (id, open) = window::open(window::Settings {
        size: iced::Size::new(700.0, 440.0),
        resizable: false,
        exit_on_close_request: false,
        ..window::Settings::default()
    });
    app.window = Some(id);
    open.then(window::gain_focus)
}

fn view(app: &App, _: window::Id) -> Element<'_, Message> {
    if app.settings_open {
        return settings_view(app);
    }
    share_view(app)
}

fn share_view(app: &App) -> Element<'_, Message> {
    let focus_ring = app.appearance.focus_ring();
    let status = match &app.phase {
        Phase::Starting => "Starting Aercast…",
        Phase::NetworkError(error) => error,
        Phase::Waiting => app
            .video_error
            .as_deref()
            .unwrap_or("Ready. Capture has not started."),
        Phase::Selecting if app.approved_source.is_some() => "Starting media…",
        Phase::Selecting => "Choose one screen or window in the system picker.",
        Phase::Sharing if app.applying_audio.is_some() => "Restarting media…",
        Phase::Sharing => "Sharing.",
        Phase::Ending => "Ending share…",
        Phase::Error(error) => error,
    };
    let can_start =
        app.phase == Phase::Waiting && !app.applying_network && app.video_error.is_none();
    let can_end = matches!(app.phase, Phase::Selecting | Phase::Sharing);
    let (share_label, share_message) = match app.phase {
        Phase::Selecting => ("Cancel", Some(Message::End)),
        Phase::Sharing => ("Stop Sharing", Some(Message::End)),
        Phase::Ending => ("Stopping…", None),
        _ => ("Start Sharing", can_start.then_some(Message::Start)),
    };
    let refresh_confirmation = if app.confirm_refresh
        && matches!(app.phase, Phase::Waiting | Phase::Sharing)
    {
        column![
            text("Refreshing disconnects every current Viewer."),
            row![
                accessibility::button(
                    button("Cancel").style(|_, status| app.appearance.neutral_button(status)),
                    Some(Message::CancelRefresh),
                    focus_ring,
                ),
                accessibility::button(
                    button("Refresh Link").style(|_, status| app.appearance.danger_button(status)),
                    Some(Message::ConfirmRefresh),
                    focus_ring,
                ),
            ]
            .spacing(12),
        ]
        .spacing(8)
    } else {
        column![]
    };
    let quit_confirmation = if app.confirm_quit && app.phase == Phase::Sharing {
        column![
            text("Quit Aercast and stop the active share?"),
            row![
                accessibility::button(
                    button("Cancel").style(|_, status| app.appearance.neutral_button(status)),
                    Some(Message::CancelQuit),
                    focus_ring,
                ),
                accessibility::button(
                    button("Quit Aercast").style(|_, status| app.appearance.danger_button(status)),
                    Some(Message::ConfirmQuit),
                    focus_ring,
                ),
            ]
            .spacing(12),
        ]
        .spacing(8)
    } else {
        column![]
    };
    let online = app.viewers.iter().filter(|viewer| viewer.online()).count();
    let now = Instant::now();
    let status = column![text(status)];
    let status = if let Some(source) = app.approved_source {
        status.push(
            text(format!("Source: {source}"))
                .size(12)
                .color(app.appearance.muted_text()),
        )
    } else {
        status
    }
    .spacing(4);
    let viewer_rows = app
        .viewers
        .iter()
        .enumerate()
        .fold(column![], |rows, (index, viewer)| {
            let (rtt, playback_lag) = viewer.telemetry(now);
            let rows = if index == 0 {
                rows
            } else {
                rows.push(
                    rule::horizontal(if app.appearance.high_contrast { 2 } else { 1 })
                        .style(|_| app.appearance.separator()),
                )
            };
            rows.push(
                container(
                    column![
                        row![
                            text(viewer.ip.to_string()).size(12),
                            space().width(Length::Fill),
                            accessibility::button(
                                button("Disconnect")
                                    .style(|_, status| { app.appearance.neutral_button(status) }),
                                viewer.online().then_some(Message::Disconnect(viewer.key)),
                                focus_ring,
                            ),
                        ]
                        .spacing(8),
                        text(format!(
                            "{} · {}   RTT {}   Lag {}",
                            if viewer.online() { "Online" } else { "Offline" },
                            format_duration(viewer.duration()),
                            format_milliseconds(rtt),
                            format_milliseconds(playback_lag),
                        ))
                        .size(12)
                        .color(app.appearance.muted_text()),
                    ]
                    .spacing(4),
                )
                .padding([6, 8]),
            )
        });

    container(
        column![
            row![
                text("Aercast").size(36),
                space().width(Length::Fill),
                accessibility::button(
                    button(
                        row![
                            symbolic_icon(include_bytes!("../assets/settings-symbolic.svg")),
                            text("Settings"),
                        ]
                        .spacing(8),
                    )
                    .style(|_, status| app.appearance.neutral_button(status)),
                    Some(Message::Settings(true)),
                    focus_ring,
                ),
            ],
            status,
            row![
                accessibility::button(
                    button("Refresh Link").style(|_, status| app.appearance.neutral_button(status)),
                    (matches!(app.phase, Phase::Waiting | Phase::Sharing) && !app.link.is_empty())
                        .then_some(Message::Refresh),
                    focus_ring,
                ),
                accessibility::text_input(
                    text_input("Share link will appear here", &app.link)
                        .style(|_, status| app.appearance.text_input(status)),
                    false,
                ),
                accessibility::button(
                    button("Copy Link").style(|_, status| app.appearance.neutral_button(status)),
                    (!app.link.is_empty()).then_some(Message::Copy),
                    focus_ring,
                ),
            ]
            .spacing(12),
            accessibility::button(
                button(share_label).style(move |_, status| {
                    if can_end {
                        app.appearance.neutral_button(status)
                    } else {
                        app.appearance.primary_button(status)
                    }
                }),
                share_message,
                focus_ring,
            ),
            refresh_confirmation,
            quit_confirmation,
            text(format!("Viewers: {online}/{}", app.viewers.len())),
            container(
                scrollable(viewer_rows)
                    .id(iced::widget::Id::new(SHARE_SCROLL_ID))
                    .height(Length::Fixed(64.0)),
            )
            .style(|_| app.appearance.boxed_list())
            .width(Length::Fill),
            text("Trusted LAN only. Use an external HTTPS reverse proxy elsewhere.")
                .size(12)
                .color(app.appearance.muted_text()),
        ]
        .spacing(12)
        .max_width(652),
    )
    .padding(24)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn format_milliseconds(duration: Option<Duration>) -> String {
    duration.map_or_else(
        || "—".to_owned(),
        |duration| format!("{} ms", duration.as_millis()),
    )
}

fn settings_option<'a>(
    app: &'a App,
    label: String,
    selected: bool,
    message: Message,
) -> Element<'a, Message> {
    let label = if selected {
        format!("✓ {label}")
    } else {
        label
    };
    accessibility::button(
        button(text(label))
            .width(Length::Fill)
            .style(move |_, status| {
                if selected {
                    app.appearance.primary_button(status)
                } else {
                    app.appearance.neutral_button(status)
                }
            }),
        Some(message),
        app.appearance.focus_ring(),
    )
}

fn video_encoder_label(encoder: settings::VideoEncoder) -> &'static str {
    match encoder {
        settings::VideoEncoder::Auto => "Auto",
        settings::VideoEncoder::VaApi => "VA-API hardware",
        settings::VideoEncoder::X264 => "Software (x264)",
    }
}

fn video_encoder_available(encoder: settings::VideoEncoder) -> bool {
    match encoder {
        settings::VideoEncoder::Auto => true,
        settings::VideoEncoder::VaApi => gst::ElementFactory::find("vah264enc").is_some(),
        settings::VideoEncoder::X264 => gst::ElementFactory::find("x264enc").is_some(),
    }
}

fn settings_view(app: &App) -> Element<'_, Message> {
    let focus_ring = app.appearance.focus_ring();
    let sharing = app.phase == Phase::Sharing;
    let audio_settings = audio_settings(&app.settings);
    let dirty = app
        .active_audio
        .as_ref()
        .is_some_and(|active| active != &audio_settings);
    let applying = app.applying_audio.is_some();
    let network_dirty = app.network_address != app.settings.listen_address.to_string()
        || app.network_port != app.settings.listen_port.to_string()
        || app.share_base_url != app.settings.share_base_url.as_deref().unwrap_or_default();
    let video_dirty = match app.settings.with_video(
        &app.video_width,
        &app.video_height,
        app.video_fps,
        &app.video_bitrate,
        app.video_encoder,
    ) {
        Ok(settings) => settings.video != app.settings.video,
        Err(_) => true,
    };
    let hint = match &app.phase {
        Phase::Starting => "Starting Aercast…",
        Phase::NetworkError(error) => error,
        Phase::Waiting => "Used when the next share starts.",
        Phase::Sharing if applying => "Restarting media with the requested setting…",
        Phase::Sharing if dirty => "Saved. Apply it to the current share when ready.",
        Phase::Sharing => "The current share uses this setting.",
        Phase::Selecting => "This share uses the value selected before the Portal opened.",
        Phase::Ending => "Ending share… The saved setting will be used next time.",
        Phase::Error(error) => error,
    };
    let fps_options = FPS_OPTIONS.into_iter().fold(row![], |options, fps| {
        options.push(settings_option(
            app,
            format!("{fps} FPS"),
            app.video_fps == fps,
            Message::VideoFps(fps),
        ))
    });
    let custom_quality = column![
        row![
            column![
                text("Width").size(12).color(app.appearance.muted_text()),
                accessibility::text_input(
                    text_input("1280", &app.video_width)
                        .on_input(Message::VideoWidth)
                        .style(|_, status| app.appearance.text_input(status)),
                    true,
                ),
            ]
            .spacing(4)
            .width(Length::Fill),
            column![
                text("Height").size(12).color(app.appearance.muted_text()),
                accessibility::text_input(
                    text_input("720", &app.video_height)
                        .on_input(Message::VideoHeight)
                        .style(|_, status| app.appearance.text_input(status)),
                    true,
                ),
            ]
            .spacing(4)
            .width(Length::Fill),
        ]
        .spacing(12),
        row![
            column![
                text("Frame rate")
                    .size(12)
                    .color(app.appearance.muted_text()),
                fps_options.spacing(8),
            ]
            .spacing(4)
            .width(Length::Fill),
            column![
                text("Bitrate (Mbps)")
                    .size(12)
                    .color(app.appearance.muted_text()),
                accessibility::text_input(
                    text_input("Encoder default", &app.video_bitrate)
                        .on_input(Message::VideoBitrate)
                        .style(|_, status| app.appearance.text_input(status)),
                    true,
                ),
            ]
            .spacing(4)
            .width(Length::Fill),
        ]
        .spacing(12),
    ]
    .spacing(12);
    let preset_row = |left: Quality, right: Quality| {
        row![
            settings_option(
                app,
                left.to_string(),
                app.video_preset == left,
                Message::VideoPreset(left),
            ),
            settings_option(
                app,
                right.to_string(),
                app.video_preset == right,
                Message::VideoPreset(right),
            ),
        ]
        .spacing(8)
    };
    let quality = column![
        text("Quality").size(15),
        preset_row(Quality::P720, Quality::P1080),
        preset_row(Quality::P1440, Quality::Custom),
    ]
    .spacing(8);
    let quality = if app.video_preset == Quality::Custom {
        quality.push(custom_quality)
    } else {
        quality
    };
    let encoder_options = [
        settings::VideoEncoder::Auto,
        settings::VideoEncoder::VaApi,
        settings::VideoEncoder::X264,
    ]
    .into_iter()
    .filter(|encoder| *encoder == app.video_encoder || video_encoder_available(*encoder))
    .fold(row![], |options, encoder| {
        options.push(settings_option(
            app,
            video_encoder_label(encoder).to_owned(),
            app.video_encoder == encoder,
            Message::VideoEncoder(encoder),
        ))
    });
    let quality = quality
        .push(text("Encoder").size(13))
        .push(encoder_options.spacing(8))
        .push(
            accessibility::button(
                button("Save quality")
                    .style(|_, status| app.appearance.primary_button(status)),
                ((video_dirty || app.video_error.is_some()) && !app.applying_network)
                    .then_some(Message::SaveVideo),
                focus_ring,
            ),
        )
        .push(
            text(if sharing {
                "The active share keeps its starting quality. Saved changes are used after Stop and the next Start."
            } else {
                "Saved quality is used by the next Start."
            })
            .size(12)
            .color(app.appearance.muted_text()),
        );
    let quality = if let Some(error) = app
        .video_edit_error
        .as_deref()
        .or(app.video_error.as_deref())
    {
        quality.push(
            text(format!("⚠ {error}"))
                .size(12)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        )
    } else {
        quality
    };
    let exclusion_rows = app
        .settings
        .audio_exclusions
        .iter()
        .fold(
            column![
                row![
                    text("Communication audio"),
                    space().width(Length::Fill),
                    text("Always excluded")
                        .size(12)
                        .color(app.appearance.muted_text()),
                ]
                .align_y(iced::Alignment::Center)
            ],
            |rows, exclusion| {
                let identity = exclusion.identity.clone();
                let toggle_identity = identity.clone();
                rows.push(
                    row![
                        accessibility::checkbox(
                            checkbox(exclusion.enabled)
                                .label(exclusion.label.clone())
                                .style(|_, status| app.appearance.checkbox(status)),
                            exclusion.enabled,
                            (!app.applying_network).then_some(move |enabled| {
                                Message::AudioExclusion(toggle_identity.clone(), enabled)
                            }),
                            focus_ring,
                        ),
                        accessibility::button(
                            button("Delete")
                                .style(|_, status| app.appearance.neutral_button(status)),
                            (!app.applying_network)
                                .then_some(Message::DeleteAudioExclusion(identity)),
                            focus_ring,
                        ),
                    ]
                    .spacing(12)
                    .align_y(iced::Alignment::Center),
                )
            },
        )
        .spacing(8);
    let mut application_rows = column![
        row![
            text("Add from active applications").size(13),
            space().width(Length::Fill),
            accessibility::button(
                button(if app.audio_scanning {
                    "Scanning…"
                } else {
                    "Refresh"
                })
                .style(|_, status| app.appearance.neutral_button(status)),
                (!app.audio_scanning).then_some(Message::RefreshAudioApplications),
                focus_ring,
            ),
        ]
        .align_y(iced::Alignment::Center)
    ]
    .spacing(8);
    let mut has_application = false;
    for application in app.audio_candidates.iter().filter(|application| {
        !app.settings
            .audio_exclusions
            .iter()
            .any(|exclusion| exclusion.identity == application.identity)
    }) {
        has_application = true;
        application_rows = application_rows.push(
            row![
                column![
                    text(&application.label)
                        .width(Length::Fill)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                    text(&application.identity)
                        .size(11)
                        .color(app.appearance.muted_text())
                        .width(Length::Fill)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                ]
                .spacing(2)
                .width(Length::Fill),
                accessibility::button(
                    button("Add").style(|_, status| app.appearance.neutral_button(status)),
                    (!app.applying_network)
                        .then_some(Message::AddAudioExclusion(application.clone())),
                    focus_ring,
                ),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
        );
    }
    if let Some(error) = app.audio_scan_error.as_deref() {
        application_rows = application_rows.push(
            text(format!("⚠ {error}"))
                .size(12)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );
    } else if !app.audio_scanning && !has_application {
        application_rows = application_rows.push(
            text("No other playback applications are active.")
                .size(12)
                .color(app.appearance.muted_text()),
        );
    }
    let audio = column![
        text("Audio").size(15),
        accessibility::checkbox(
            checkbox(app.settings.system_audio)
                .label("System audio")
                .style(|_, status| app.appearance.checkbox(status)),
            app.settings.system_audio,
            (!app.applying_network).then_some(Message::SystemAudio as fn(bool) -> Message),
            focus_ring,
        ),
        text(hint).size(12).color(app.appearance.muted_text()),
        text("Excluded applications").size(13),
        exclusion_rows,
        application_rows,
    ]
    .spacing(12);
    let audio = if sharing && (dirty || applying) {
        audio.push(accessibility::button(
            button(if applying {
                "Applying…"
            } else {
                "Apply to Current Share"
            })
            .style(|_, status| app.appearance.primary_button(status)),
            (dirty && !applying).then_some(Message::ApplySystemAudio),
            focus_ring,
        ))
    } else {
        audio
    };
    let network = column![
        text("Network").size(15),
        row![
            column![
                text("Listen address")
                    .size(12)
                    .color(app.appearance.muted_text()),
                accessibility::text_input(
                    text_input("127.0.0.1", &app.network_address)
                        .on_input_maybe((!app.applying_network).then_some(Message::NetworkAddress))
                        .style(|_, status| app.appearance.text_input(status)),
                    !app.applying_network,
                ),
            ]
            .spacing(4)
            .width(Length::FillPortion(3)),
            column![
                text("Port").size(12).color(app.appearance.muted_text()),
                accessibility::text_input(
                    text_input("8877", &app.network_port)
                        .on_input_maybe((!app.applying_network).then_some(Message::NetworkPort))
                        .style(|_, status| app.appearance.text_input(status)),
                    !app.applying_network,
                ),
            ]
            .spacing(4)
            .width(Length::FillPortion(1)),
        ]
        .spacing(12),
        text("Share base URL (optional)")
            .size(12)
            .color(app.appearance.muted_text()),
        accessibility::text_input(
            text_input("https://host:port", &app.share_base_url)
                .on_input_maybe((!app.applying_network).then_some(Message::ShareBaseUrl))
                .style(|_, status| app.appearance.text_input(status)),
            !app.applying_network,
        ),
        accessibility::button(
            button(if app.applying_network {
                "Applying network…"
            } else {
                "Apply Network"
            })
            .style(|_, status| app.appearance.primary_button(status)),
            (((app.phase == Phase::Waiting && network_dirty)
                || matches!(&app.phase, Phase::NetworkError(_)))
                && !app.applying_network)
                .then_some(Message::ApplyNetwork),
            focus_ring,
        ),
        text("Network changes apply only while stopped.")
            .size(12)
            .color(app.appearance.muted_text()),
        text("Changing the listener may leave old waiting pages unable to recover.")
            .size(12)
            .color(app.appearance.muted_text()),
    ]
    .spacing(12);
    let notifications = column![
        text("Notifications").size(15),
        accessibility::checkbox(
            checkbox(app.settings.notifications)
                .label("Desktop notifications")
                .style(|_, status| app.appearance.checkbox(status)),
            app.settings.notifications,
            (!app.applying_network).then_some(Message::Notifications as fn(bool) -> Message),
            focus_ring,
        ),
    ]
    .spacing(12);
    let sections = column![quality, audio, network, notifications].spacing(24);
    let body = if let Some(error) = app.settings_error.as_deref() {
        column![
            text(format!("⚠ {error}"))
                .size(12)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
            sections,
        ]
        .spacing(24)
    } else {
        column![sections]
    }
    .max_width(632);

    container(
        column![
            row![
                accessibility::button(
                    button(
                        row![
                            symbolic_icon(include_bytes!("../assets/back-symbolic.svg")),
                            text("Back"),
                        ]
                        .spacing(8),
                    )
                    .style(|_, status| app.appearance.neutral_button(status)),
                    Some(Message::Settings(false)),
                    focus_ring,
                ),
                text("Settings").size(24),
            ]
            .spacing(16)
            .align_y(iced::Alignment::Center),
            scrollable(body)
                .id(iced::widget::Id::new(SETTINGS_SCROLL_ID))
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .spacing(16)
        .max_width(652)
        .height(Length::Fill),
    )
    .padding(24)
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .into()
}

fn symbolic_icon(bytes: &'static [u8]) -> iced::widget::Svg<'static> {
    svg(svg::Handle::from_memory(bytes))
        .width(18)
        .height(18)
        .style(|theme: &Theme, _| svg::Style {
            color: Some(theme.palette().text),
        })
}

async fn run_host(
    settings: settings::Settings,
    events: Events,
    mut command_receiver: mpsc::Receiver<Command>,
) -> Result<()> {
    let host = web::Host::new()?;
    let bind = settings.bind()?;
    let mut server = match bind_listener(bind).await {
        Ok((listener, address)) => Some(start_server(listener, address, &host)),
        Err(error) => {
            let _ = events.unbounded_send(HostEvent::NetworkUnavailable(format!(
                "Could not listen on {bind}: {error}. Change Network settings and apply them."
            )));
            None
        }
    };
    let mut share_base_url = settings.share_base_url;
    let outcome: Result<()> = async {
        loop {
            if let Some(server) = &server {
                let link = format!(
                    "{}{}",
                    link_base(share_base_url.as_deref(), server.address),
                    host.path()?
                );
                let _ = events.unbounded_send(HostEvent::Waiting(link));
            }
            let command = {
                let server_result = async {
                    match server.as_mut() {
                        Some(server) => (&mut server.task).await,
                        None => std::future::pending().await,
                    }
                };
                tokio::pin!(server_result);
                tokio::select! {
                    result = &mut server_result => return server_outcome(result).map(|_| ()),
                    signal = tokio::signal::ctrl_c() => {
                        signal?;
                        Command::Quit
                    }
                    command = command_receiver.recv() => command.unwrap_or(Command::Quit),
                }
            };
            match command {
                Command::Start(share) => {
                    let Some(server) = server.as_mut() else {
                        continue;
                    };
                    let link_base = link_base(share_base_url.as_deref(), server.address);
                    match share_once(
                        &host,
                        &link_base,
                        share,
                        &mut command_receiver,
                        &mut server.task,
                        &events,
                    )
                    .await?
                    {
                        ShareStop::Apply(_) | ShareStop::End | ShareStop::PortalClosed => {}
                        ShareStop::Quit => break,
                        ShareStop::Failed(error) => return Err(error),
                    }
                }
                Command::Apply(_) => {}
                Command::Network(settings) => {
                    let listener = match async {
                        let listener = prepare_listener(
                            &settings,
                            server.as_ref().map(|server| server.address),
                        )
                        .await?;
                        settings.save().map_err(|error| {
                            io::Error::new(
                                error.kind(),
                                format!("could not save settings: {error}"),
                            )
                        })?;
                        Ok::<_, io::Error>(listener)
                    }
                    .await
                    {
                        Ok(listener) => listener,
                        Err(error) => {
                            let error = format!("Network settings unchanged: {error}");
                            let event = if server.is_some() {
                                HostEvent::NetworkApplied(Err(error))
                            } else {
                                HostEvent::NetworkUnavailable(error)
                            };
                            let _ = events.unbounded_send(event);
                            continue;
                        }
                    };
                    if let Some((listener, new_address)) = listener {
                        let new_server = start_server(listener, new_address, &host);
                        if let Some(old_server) = server.replace(new_server) {
                            stop_server(old_server.task, old_server.shutdown).await?;
                        }
                    }
                    share_base_url = settings.share_base_url.clone();
                    let _ = events.unbounded_send(HostEvent::NetworkApplied(Ok(settings)));
                }
                Command::End => {}
                Command::Refresh(confirmed) => {
                    if server.is_some() && host.refresh(confirmed)?.is_none() {
                        let _ = events.unbounded_send(HostEvent::ConfirmRefresh);
                    }
                }
                Command::Disconnect(key) => host.disconnect_viewer(key)?,
                Command::Quit => break,
            }
        }
        Ok(())
    }
    .await;

    if let Err(error) = outcome {
        if let Some(server) = server {
            server.task.abort();
        }
        return Err(error);
    }
    if let Some(server) = server {
        stop_server(server.task, server.shutdown).await?;
    }
    Ok(())
}

async fn bind_listener(bind: SocketAddr) -> io::Result<(TcpListener, SocketAddr)> {
    let listener = TcpListener::bind(bind).await?;
    SockRef::from(&listener).set_tcp_user_timeout(Some(STALLED_CLIENT_TIMEOUT))?;
    let address = listener.local_addr()?;
    Ok((listener, address))
}

fn start_server(listener: TcpListener, address: SocketAddr, host: &web::Host) -> RunningServer {
    let (shutdown, shutdown_request) = oneshot::channel();
    RunningServer {
        address,
        shutdown,
        task: tokio::spawn(web::serve(listener, host.clone(), shutdown_request)),
    }
}

async fn prepare_listener(
    settings: &settings::Settings,
    current_address: Option<SocketAddr>,
) -> io::Result<Option<(TcpListener, SocketAddr)>> {
    let bind = settings.bind()?;
    let listener = if Some(bind) == current_address {
        None
    } else {
        Some(bind_listener(bind).await.map_err(|error| {
            io::Error::new(error.kind(), format!("could not bind {bind}: {error}"))
        })?)
    };
    Ok(listener)
}

async fn stop_server(mut server: Server, shutdown: oneshot::Sender<()>) -> io::Result<()> {
    let _ = shutdown.send(());
    match tokio::time::timeout(STALLED_CLIENT_TIMEOUT + Duration::from_secs(1), &mut server).await {
        Ok(Ok(result)) => result?,
        Ok(Err(error)) => return Err(io::Error::other(error.to_string())),
        Err(_) => {
            server.abort();
            if let Err(error) = server.await
                && !error.is_cancelled()
            {
                return Err(io::Error::other(error.to_string()));
            }
        }
    }
    Ok(())
}

fn link_base(base_url: Option<&str>, address: SocketAddr) -> String {
    match base_url {
        Some(base_url) => base_url.to_owned(),
        None => format!("http://{address}"),
    }
}

async fn share_once(
    host: &web::Host,
    link_base: &str,
    share: ShareSettings,
    commands: &mut mpsc::Receiver<Command>,
    server: &mut Server,
    events: &Events,
) -> Result<ShareStop> {
    let ShareSettings { mut audio, video } = share;
    let portal = Screencast::new().await?;
    let available_sources = portal.available_source_types().await?;
    let available_cursors = portal.available_cursor_modes().await?;
    println!("Portal version: {}", portal.version());
    println!("Available source types: {available_sources:?}");
    println!("Available cursor modes: {available_cursors:?}");

    let sources = available_sources & (SourceType::Monitor | SourceType::Window);
    if sources.is_empty() {
        return Err(io::Error::other("portal offers neither monitor nor window capture").into());
    }
    let cursor = cursor_mode(
        available_cursors.contains(CursorMode::Embedded),
        available_cursors.contains(CursorMode::Hidden),
    )
    .ok_or_else(|| io::Error::other("portal offers no supported cursor mode"))?;

    let session = portal.create_session(Default::default()).await?;
    let mut closed = session.receive_closed().await?;
    let capture = async {
        portal
            .select_sources(
                &session,
                SelectSourcesOptions::default()
                    .set_sources(sources)
                    .set_multiple(false)
                    .set_cursor_mode(cursor)
                    .set_persist_mode(PersistMode::DoNot),
            )
            .await?
            .response()?;

        let response = portal
            .start(&session, None, Default::default())
            .await?
            .response()?;
        let stream = response
            .streams()
            .first()
            .ok_or_else(|| io::Error::other("portal returned no selected stream"))?;
        let source = approved_source(stream.source_type());
        println!(
            "Selected source: {source}; PipeWire node {}",
            stream.pipe_wire_node_id(),
        );
        Ok((stream.pipe_wire_node_id(), source))
    };
    tokio::pin!(capture);

    enum Selection {
        Capture(ashpd::Result<(u32, &'static str)>),
        Stop(bool),
        Signal(io::Result<()>),
        Server(std::result::Result<io::Result<()>, tokio::task::JoinError>),
    }
    let selection = loop {
        tokio::select! {
            result = &mut capture => break Selection::Capture(result),
            signal = tokio::signal::ctrl_c() => break Selection::Signal(signal),
            result = &mut *server => break Selection::Server(result),
            command = commands.recv() => match command.unwrap_or(Command::Quit) {
                Command::Start(..) => println!("Source selection is already open."),
                Command::Apply(_) => println!("Source selection is still open."),
                Command::Network(_) => {
                    let _ = events.unbounded_send(HostEvent::NetworkApplied(Err(
                        "Stop sharing before applying network settings".to_owned()
                    )));
                }
                Command::End => break Selection::Stop(false),
                Command::Refresh(_) => println!("Source selection is still open."),
                Command::Disconnect(key) => host.disconnect_viewer(key)?,
                Command::Quit => break Selection::Stop(true),
            },
        }
    };
    let (node_id, source) = match selection {
        Selection::Capture(Ok(capture)) => capture,
        Selection::Capture(
            Err(ashpd::Error::Response(ResponseError::Cancelled))
            | Err(ashpd::Error::Portal(PortalError::Cancelled(_))),
        ) => {
            println!("Portal request cancelled.");
            if let Err(error) = session.close().await {
                eprintln!("Failed to close cancelled Portal session: {error}");
            }
            return Ok(ShareStop::End);
        }
        Selection::Capture(Err(error)) => {
            if let Err(close_error) = session.close().await {
                eprintln!("Failed to close Portal session: {close_error}");
            }
            return Err(error.into());
        }
        Selection::Stop(quit) => {
            session.close().await?;
            return Ok(if quit {
                ShareStop::Quit
            } else {
                ShareStop::End
            });
        }
        Selection::Signal(signal) => {
            session.close().await?;
            signal?;
            return Ok(ShareStop::Quit);
        }
        Selection::Server(result) => {
            if let Err(error) = session.close().await {
                eprintln!("Failed to close Portal session: {error}");
            }
            return server_outcome(result);
        }
    };
    let _ = events.unbounded_send(HostEvent::Source(source));

    let mut capture_caps = None;
    let mut recoveries = 0;
    let result = loop {
        let control = share_control(
            commands,
            async {
                let _ = closed.next().await;
            },
            server,
            host,
            link_base,
            events,
        );
        tokio::pin!(control);
        let remote = tokio::select! {
            biased;
            stop = control.as_mut() => match stop {
                ShareStop::Apply(next) => {
                    audio = next;
                    continue;
                }
                stop => break Ok(stop),
            },
            remote = portal.open_pipe_wire_remote(&session, Default::default()) => match remote {
                Ok(remote) => remote,
                Err(error) => break Ok(ShareStop::Failed(error.into())),
            },
        };
        if let Some(stop) = control.as_mut().now_or_never() {
            match stop {
                ShareStop::Apply(next) => {
                    audio = next;
                    continue;
                }
                stop => break Ok(stop),
            }
        }
        let media = match host.start() {
            Ok(media) => media,
            Err(error) => break Ok(ShareStop::Failed(error.into())),
        };
        let description = pipeline_description(node_id, remote.as_raw_fd(), video);
        let attempt = serve_video(
            &description,
            &mut capture_caps,
            audio.clone(),
            media.clone(),
            control.as_mut(),
            events,
        )
        .await;
        let stopped = host.stop(&media).and_then(|()| host.viewers());
        if let Ok(viewers) = &stopped {
            let _ = events.unbounded_send(HostEvent::Viewers(viewers.clone()));
        }
        let attempt = match stopped {
            Ok(_) => attempt,
            Err(error) => {
                eprintln!("Failed to stop media session: {error}");
                Ok(ShareStop::Failed(error.into()))
            }
        };
        if let Ok(ShareStop::Apply(next)) = &attempt {
            audio = next.clone();
            continue;
        }
        let mut apply = None;
        if attempt.is_err()
            && let Some(stop) = control.as_mut().now_or_never()
        {
            match stop {
                ShareStop::Apply(next) => apply = Some(next),
                stop => break Ok(stop),
            }
        }
        if !should_retry(&attempt, recoveries) {
            break attempt;
        }
        recoveries += 1;
        if let Err(error) = &attempt {
            eprintln!(
                "Media attempt failed; recovery {recoveries}/{MAX_MEDIA_RECOVERIES}: {error}"
            );
        }
        if let Some(next) = apply {
            audio = next;
            continue;
        }
        tokio::select! {
            biased;
            stop = control.as_mut() => match stop {
                ShareStop::Apply(next) => {
                    audio = next;
                    continue;
                }
                stop => break Ok(stop),
            },
            _ = tokio::time::sleep(MEDIA_RECOVERY_DELAY) => {}
        }
    };
    let portal_closed = matches!(&result, Ok(ShareStop::PortalClosed));
    let close_result = if portal_closed {
        Ok(())
    } else {
        session.close().await
    };
    if let Err(error) = &close_result {
        eprintln!("Failed to close Portal session: {error}");
    }
    match result {
        Ok(ShareStop::Failed(error)) | Err(error) => Err(error),
        Ok(stop) => {
            close_result?;
            Ok(stop)
        }
    }
}

async fn share_control(
    commands: &mut mpsc::Receiver<Command>,
    session_closed: impl Future<Output = ()>,
    server: &mut Server,
    host: &web::Host,
    link_base: &str,
    events: &Events,
) -> ShareStop {
    let mut viewer_updates = match host.viewer_updates() {
        Ok(viewers) => viewers,
        Err(error) => return ShareStop::Failed(error.into()),
    };
    match host.viewers() {
        Ok(viewers) => {
            let _ = events.unbounded_send(HostEvent::Viewers(viewers));
        }
        Err(error) => return ShareStop::Failed(error.into()),
    }
    tokio::pin!(session_closed);
    loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => return match signal {
                Ok(()) => {
                    println!("Stopping Aercast.");
                    ShareStop::Quit
                }
                Err(error) => ShareStop::Failed(error.into()),
            },
            command = commands.recv() => match command.unwrap_or(Command::Quit) {
                Command::Start(..) => println!("A share is already active."),
                Command::Apply(audio) => return ShareStop::Apply(audio),
                Command::Network(_) => {
                    let _ = events.unbounded_send(HostEvent::NetworkApplied(Err(
                        "Stop sharing before applying network settings".to_owned()
                    )));
                }
                Command::End => {
                    println!("Ending share.");
                    return ShareStop::End;
                }
                Command::Refresh(confirmed) => match host.refresh(confirmed) {
                    Ok(Some(path)) => {
                        viewer_updates = match host.viewer_updates() {
                            Ok(viewers) => viewers,
                            Err(error) => return ShareStop::Failed(error.into()),
                        };
                        let _ = events
                            .unbounded_send(HostEvent::Link(format!("{link_base}{path}")));
                    }
                    Ok(None) => {
                        let _ = events.unbounded_send(HostEvent::ConfirmRefresh);
                    }
                    Err(error) => return ShareStop::Failed(error.into()),
                },
                Command::Disconnect(key) => {
                    if let Err(error) = host.disconnect_viewer(key) {
                        return ShareStop::Failed(error.into());
                    }
                }
                Command::Quit => {
                    println!("Stopping Aercast.");
                    return ShareStop::Quit;
                }
            },
            changed = viewer_updates.changed() => {
                if changed.is_err() {
                    return ShareStop::Failed(io::Error::other(
                        "Viewer update channel closed"
                    ).into());
                }
                viewer_updates.borrow_and_update();
                match host.viewers() {
                    Ok(viewers) => {
                        let _ = events.unbounded_send(HostEvent::Viewers(viewers));
                    }
                    Err(error) => return ShareStop::Failed(error.into()),
                }
            },
            _ = &mut session_closed => {
                println!("Portal session closed; stopping stream.");
                return ShareStop::PortalClosed;
            }
            result = &mut *server => {
                return server_outcome(result).unwrap_or_else(ShareStop::Failed);
            },
        }
    }
}

fn should_retry<T, E>(outcome: &std::result::Result<T, E>, recoveries: u8) -> bool {
    outcome.is_err() && recoveries < MAX_MEDIA_RECOVERIES
}

fn validate_arguments(mut args: impl Iterator<Item = String>) -> io::Result<()> {
    if args.next().is_some() {
        Err(io::Error::other("usage: aercast"))
    } else {
        Ok(())
    }
}

fn cursor_mode(embedded: bool, hidden: bool) -> Option<CursorMode> {
    embedded
        .then_some(CursorMode::Embedded)
        .or_else(|| hidden.then_some(CursorMode::Hidden))
}

fn approved_source(source_type: Option<SourceType>) -> &'static str {
    match source_type {
        Some(SourceType::Monitor) => "Screen",
        Some(SourceType::Window) => "Window",
        _ => "Selected source",
    }
}

async fn serve_video(
    description: &str,
    capture_caps: &mut Option<gst::Caps>,
    audio: AudioSettings,
    media: web::MediaSession,
    mut control: Pin<&mut impl Future<Output = ShareStop>>,
    events: &Events,
) -> Result<ShareStop> {
    let audio_exclusions = audio.enabled.then(|| audio.exclusions.clone());
    if audio_exclusions.as_ref().is_some_and(Vec::is_empty) {
        eprintln!(
            "No audio exclusions configured; a Host-local Viewer may feed shared audio back into Aercast."
        );
    }
    let started = Instant::now();

    let pipeline = build_pipeline(description)?;
    let portal_video = pipeline
        .by_name("portal-video")
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no Portal video source"))?;
    let portal_format = pipeline
        .by_name("portal-format")
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no Portal video format"))?;
    if let Some(caps) = capture_caps.as_ref() {
        portal_format.set_property("caps", caps);
    }
    let parser_pad = pipeline
        .by_name("h264")
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no H.264 parser"))?
        .static_pad("src")
        .ok_or_else(|| io::Error::other("H.264 parser has no source pad"))?;
    let audio_source = pipeline
        .by_name("system-audio")
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no system-audio source"))?
        .downcast::<gst_app::AppSrc>()
        .map_err(|_| io::Error::other("GStreamer system-audio source is not appsrc"))?;
    parser_pad
        .add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, {
            let media = media.clone();
            move |_, info| {
                if let Some(gst::PadProbeData::Event(event)) = &info.data
                    && let gst::EventView::Caps(event) = event.view()
                    && let Some(mime) = h264_mime(event.caps())
                {
                    if let Err(error) = media.set_mime(mime) {
                        eprintln!("Failed to publish media type: {error}");
                    }
                    return gst::PadProbeReturn::Remove;
                }
                gst::PadProbeReturn::Ok
            }
        })
        .ok_or_else(|| io::Error::other("failed to install codec probe"))?;

    let app_sink = pipeline
        .by_name("stream")
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no media sink"))?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| io::Error::other("GStreamer media sink is not an appsink"))?;
    app_sink.set_callbacks(
        AppSinkCallbacks::builder()
            .new_sample({
                let media = media.clone();
                let mut first_fragment = true;
                move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let bytes = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    let fragment = media.publish(bytes.as_slice()).map_err(|error| {
                        eprintln!("Failed to publish fMP4: {error}");
                        gst::FlowError::Error
                    })?;
                    if first_fragment && fragment {
                        println!("First fMP4 fragment: {} ms", started.elapsed().as_millis());
                        first_fragment = false;
                    }
                    Ok(gst::FlowSuccess::Ok)
                }
            })
            .build(),
    );

    let bus = pipeline
        .bus()
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no bus"))?;
    let message_types = [gst::MessageType::Eos, gst::MessageType::Error];
    let mut messages = bus.stream_filtered(&message_types);
    parser_pad
        .add_probe(gst::PadProbeType::BUFFER, move |_, _| {
            println!("First encoded frame: {} ms", started.elapsed().as_millis());
            gst::PadProbeReturn::Remove
        })
        .ok_or_else(|| io::Error::other("failed to install first-frame probe"))?;
    if let Some(stop) = control.as_mut().now_or_never() {
        return Ok(stop);
    }
    let outcome: Result<ShareStop> = match pipeline.set_state(gst::State::Playing) {
        Err(error) => Err(error.into()),
        Ok(_) => match audio_exclusions
            .map(|exclusions| audio::start(audio_source, exclusions))
            .transpose()
        {
            Err(error) => Err(error.into()),
            Ok(mut audio_capture) => {
                println!("Browser stream running.");
                let _ = events.unbounded_send(HostEvent::Sharing(audio.clone()));
                let mut audio_failure_reported = false;
                let running = tokio::select! {
                    biased;
                    stop = control.as_mut() => Ok(stop),
                    message = messages.next() => media_outcome(message),
                    error = async {
                        match audio_capture.as_mut() {
                            Some((_, errors)) => errors.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        audio_failure_reported = error.is_some();
                        Err(io::Error::other(error.unwrap_or_else(||
                            "selective-audio thread stopped unexpectedly".to_owned()
                        )).into())
                    },
                };
                if !matches!(&running, Err(_) | Ok(ShareStop::Apply(_))) {
                    let _ = events.unbounded_send(HostEvent::Ending);
                }
                let stopped: Result<()> = audio_capture
                    .map_or(Ok(()), |(audio, _)| audio.stop(audio_failure_reported))
                    .map_err(|error| io::Error::other(error).into());
                if let Err(error) = &stopped {
                    eprintln!("Failed to clean up selective audio: {error}");
                }
                match stopped {
                    Ok(()) => running,
                    Err(error) => Ok(ShareStop::Failed(error)),
                }
            }
        },
    };

    if let Some(caps) = portal_video
        .static_pad("src")
        .and_then(|pad| pad.current_caps())
    {
        *capture_caps = Some(caps);
    }
    let stop_result = pipeline.set_state(gst::State::Null);
    if let Err(error) = &stop_result {
        eprintln!("Failed to stop GStreamer pipeline: {error}");
    }
    match stop_result {
        Ok(_) => outcome,
        Err(error) => Ok(ShareStop::Failed(error.into())),
    }
}

fn video_plan(video: &settings::VideoSettings) -> Result<VideoPlan> {
    video.validate()?;
    match video.encoder {
        settings::VideoEncoder::Auto => {
            plan_encoder(*video, Encoder::VaApi).or_else(|_| plan_encoder(*video, Encoder::X264))
        }
        settings::VideoEncoder::VaApi => plan_encoder(*video, Encoder::VaApi),
        settings::VideoEncoder::X264 => plan_encoder(*video, Encoder::X264),
    }
}

fn plan_encoder(video: settings::VideoSettings, encoder: Encoder) -> Result<VideoPlan> {
    let (factory_name, format) = match encoder {
        Encoder::VaApi => ("vah264enc", "NV12"),
        Encoder::X264 => ("x264enc", "I420"),
    };
    let caps = gst::Caps::builder("video/x-raw")
        .field("format", format)
        .field("width", video.width as i32)
        .field("height", video.height as i32)
        .field("framerate", gst::Fraction::new(video.fps as i32, 1))
        .build();
    require_caps(factory_name, gst::PadDirection::Sink, &caps)?;
    let plan = VideoPlan {
        settings: video,
        encoder,
    };
    let pipeline = build_pipeline(&pipeline_description(1, 0, plan))?;
    let encoder = pipeline
        .by_name("encoder")
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no video encoder"))?;
    let ready = encoder
        .set_state(gst::State::Ready)
        .and_then(|_| encoder.state(gst::ClockTime::from_seconds(3)).0);
    let reached_ready = encoder.current_state() == gst::State::Ready;
    let stopped = encoder.set_state(gst::State::Null);
    ready?;
    stopped?;
    if !reached_ready {
        return Err(io::Error::other(format!("{factory_name} did not become ready")).into());
    }
    Ok(plan)
}

fn require_caps(factory_name: &str, direction: gst::PadDirection, caps: &gst::Caps) -> Result<()> {
    let factory = gst::ElementFactory::find(factory_name)
        .ok_or_else(|| io::Error::other(format!("missing GStreamer element {factory_name}")))?;
    if factory
        .static_pad_templates()
        .iter()
        .any(|template| template.direction() == direction && template.caps().can_intersect(caps))
    {
        Ok(())
    } else {
        Err(io::Error::other(format!("{factory_name} does not support {caps}")).into())
    }
}

fn build_pipeline(description: &str) -> Result<gst::Pipeline> {
    gst::parse::launch(description)?
        .downcast::<gst::Pipeline>()
        .map_err(|_| io::Error::other("GStreamer did not create a pipeline").into())
}

fn pipeline_description(node_id: u32, remote_fd: i32, plan: VideoPlan) -> String {
    let video = plan.settings;
    let bitrate = video
        .bitrate_mbps
        .map(|bitrate| format!(" bitrate={}", u32::from(bitrate) * 1_000))
        .unwrap_or_default();
    let video_pipeline = match plan.encoder {
        Encoder::VaApi => format!(
            "vapostproc disable-passthrough=true add-borders=true ! video/x-raw(memory:VAMemory),format=NV12,width={width},height={height} ! imagefreeze is-live=true allow-replace=true ! video/x-raw(memory:VAMemory),format=NV12,framerate={fps}/1 ! vah264enc name=encoder rate-control=cbr target-usage=7{bitrate} key-int-max={fps} ! video/x-h264,profile=constrained-baseline,stream-format=byte-stream,alignment=au",
            width = video.width,
            height = video.height,
            fps = video.fps,
        ),
        Encoder::X264 => format!(
            "videoconvertscale add-borders=true ! video/x-raw,format=I420,width={width},height={height} ! imagefreeze is-live=true allow-replace=true ! video/x-raw,format=I420,framerate={fps}/1 ! x264enc name=encoder tune=zerolatency speed-preset=ultrafast{bitrate} key-int-max={fps}",
            width = video.width,
            height = video.height,
            fps = video.fps,
        ),
    };
    format!(
        "mp4mux name=mux fragment-duration=100 ! appsink name=stream sync=false wait-on-eos=false
         audiomixer name=audio-mixer ignore-inactive-pads=true ! audioconvert ! audio/x-raw,format=F32LE,rate=48000,channels=2 ! avenc_aac bitrate=128000 ! aacparse ! audio/mpeg,mpegversion=4,stream-format=raw ! queue ! mux.audio_0
         audiotestsrc is-live=true wave=silence ! audio/x-raw,format=F32LE,rate=48000,channels=2 ! queue ! audio-mixer.
         appsrc name=system-audio is-live=true format=time do-timestamp=true block=false max-bytes=384000 leaky-type=downstream ! audio/x-raw,format=F32LE,rate=48000,channels=2,layout=interleaved ! queue ! audio-mixer.
         pipewiresrc name=portal-video fd={remote_fd} path={node_id} on-disconnect=error ! capsfilter name=portal-format ! {video_pipeline} ! h264parse name=h264 ! video/x-h264,stream-format=avc,alignment=au ! queue ! mux.video_0",
    )
}

fn h264_mime(caps: &gst::CapsRef) -> Option<String> {
    let codec_data = caps.structure(0)?.get::<gst::Buffer>("codec_data").ok()?;
    let bytes = codec_data.map_readable().ok()?;
    avc_codec(bytes.as_slice()).map(|codec| format!("video/mp4; codecs=\"{codec}, mp4a.40.2\""))
}

fn avc_codec(config: &[u8]) -> Option<String> {
    (config.len() >= 4 && config[0] == 1)
        .then(|| format!("avc1.{:02x}{:02x}{:02x}", config[1], config[2], config[3]))
}

fn server_outcome(
    result: std::result::Result<io::Result<()>, tokio::task::JoinError>,
) -> Result<ShareStop> {
    match result {
        Ok(Ok(())) => Err(io::Error::other("HTTP server stopped").into()),
        Ok(Err(error)) => Err(error.into()),
        Err(error) => Err(io::Error::other(format!("HTTP server task failed: {error}")).into()),
    }
}

fn media_outcome(message: Option<gst::Message>) -> Result<ShareStop> {
    match message {
        Some(message) => match message.view() {
            gst::MessageView::Eos(..) => Err(io::Error::other("capture stream ended").into()),
            gst::MessageView::Error(error) => {
                let source = message
                    .src()
                    .map(|source| source.path_string().to_string())
                    .unwrap_or_else(|| "unknown".to_owned());
                Err(io::Error::other(format!(
                    "GStreamer error from {source}: {} ({})",
                    error.error(),
                    error.debug().unwrap_or_default(),
                ))
                .into())
            }
            _ => unreachable!(),
        },
        None => Err(io::Error::other("GStreamer bus closed").into()),
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
