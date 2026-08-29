use std::{
    cell::Cell,
    collections::HashMap,
    future::Future,
    io::{self, Cursor},
    net::{IpAddr, SocketAddr},
    os::fd::AsRawFd,
    pin::Pin,
    sync::LazyLock,
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
        tooltip,
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
const OVERVIEW_SCROLL_ID: &str = "overview";
const VIEWERS_SCROLL_ID: &str = "viewers";
const SETTINGS_SCROLL_ID: &str = "settings";
const NETWORK_ADDRESS_ID: &str = "network-address";
const COPY_FEEDBACK_DURATION: Duration = Duration::from_millis(1_500);
const BLOCK_CONFIRMATION_DURATION: Duration = Duration::from_secs(3);
const BOLD_FONT: iced::Font = iced::Font {
    weight: iced::font::Weight::Bold,
    ..iced::Font::DEFAULT
};

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

#[derive(Clone, Debug, PartialEq)]
struct SettingsDraft {
    settings: settings::Settings,
    video_preset: Quality,
    video_width: String,
    video_height: String,
    video_fps: u32,
    video_bitrate: String,
    video_encoder: settings::VideoEncoder,
    network_address: String,
    network_port: String,
    share_base_url: String,
    revision: u64,
}

impl SettingsDraft {
    fn from_settings(settings: &settings::Settings) -> Self {
        let video = settings.video;
        Self {
            settings: settings.clone(),
            video_preset: Quality::from_video(video),
            video_width: video.width.to_string(),
            video_height: video.height.to_string(),
            video_fps: video.fps,
            video_bitrate: video
                .bitrate_mbps
                .map_or_else(String::new, |bitrate| bitrate.to_string()),
            video_encoder: video.encoder,
            network_address: settings.listen_address.to_string(),
            network_port: settings.listen_port.to_string(),
            share_base_url: settings.share_base_url.clone().unwrap_or_default(),
            revision: 0,
        }
    }

    fn candidate(&self) -> io::Result<settings::Settings> {
        self.settings
            .with_video(
                &self.video_width,
                &self.video_height,
                self.video_fps,
                &self.video_bitrate,
                self.video_encoder,
            )?
            .with_network(
                &self.network_address,
                &self.network_port,
                &self.share_base_url,
            )
    }

    fn dirty(&self, saved: &settings::Settings) -> bool {
        match self.candidate() {
            Ok(candidate) => candidate != *saved,
            Err(_) => true,
        }
    }

    fn network_dirty(&self, saved: &settings::Settings) -> bool {
        match self.settings.with_network(
            &self.network_address,
            &self.network_port,
            &self.share_base_url,
        ) {
            Ok(candidate) => {
                candidate.listen_address != saved.listen_address
                    || candidate.listen_port != saved.listen_port
                    || candidate.share_base_url != saved.share_base_url
            }
            Err(_) => true,
        }
    }

    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}

struct PendingSettings {
    candidate: settings::Settings,
    video: VideoPlan,
}

#[derive(Clone, Copy)]
struct BlockConfirmation {
    key: u64,
    started: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct VideoPlan {
    settings: settings::VideoSettings,
    encoder: Encoder,
}

#[derive(Clone, Debug, PartialEq)]
enum VideoProbe {
    Current(settings::VideoSettings),
    Apply {
        revision: u64,
        candidate: settings::Settings,
    },
}

impl VideoProbe {
    fn video(&self) -> settings::VideoSettings {
        match self {
            Self::Current(video) => *video,
            Self::Apply { candidate, .. } => candidate.video,
        }
    }
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Overview,
    Viewers,
    Settings,
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
    Apply(ShareSettings),
    Sleep,
    Wake,
    End,
    Quit,
    PortalClosed,
    Failed(Error),
}

#[derive(Debug)]
struct HardwareVideoFailure(String);

impl std::fmt::Display for HardwareVideoFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for HardwareVideoFailure {}

#[derive(Clone, Debug)]
enum HostEvent {
    Waiting(String),
    NetworkUnavailable(String),
    Source(&'static str),
    Link(String),
    ConfirmRefresh,
    Sharing(ShareSettings),
    ApplyFailed(String),
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
    CopyFeedbackExpired(Instant),
    Refresh,
    Block(u64),
    ConfirmRefresh,
    CancelRefresh,
    Show,
    Quit,
    ConfirmQuit,
    CancelQuit,
    QuitQueued(bool),
    BusClosed,
    TrayStopped(std::result::Result<(), String>),
    Page(Page),
    NetworkSettings,
    SystemAudio(bool),
    AudioBitrate(u32),
    CommunicationAudio(bool),
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
    VideoPreset(Quality),
    VideoWidth(String),
    VideoHeight(String),
    VideoFps(u32),
    VideoBitrate(String),
    VideoEncoder(settings::VideoEncoder),
    ApplySettings,
    RevertSettings,
    ApplyCurrentShare,
    VideoProbed(VideoProbe, std::result::Result<VideoPlan, String>),
    Focus(bool),
    RevealFocus(f32),
    Tick,
    WindowResized(window::Id),
    MonitorSize(window::Id, Option<iced::Size>),
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

#[derive(Clone, Debug, PartialEq)]
struct TrayState {
    phase: Phase,
    online_viewers: usize,
}

struct App {
    phase: Phase,
    link: String,
    viewers: Vec<web::Viewer>,
    commands: Option<mpsc::Sender<Command>>,
    window: Option<window::Id>,
    monitor_size: Option<iced::Size>,
    confirm_refresh: bool,
    confirm_quit: bool,
    confirm_apply_current: bool,
    confirm_block: Option<BlockConfirmation>,
    settings: settings::Settings,
    draft: SettingsDraft,
    page: Page,
    copied_at: Option<Instant>,
    settings_error: Option<String>,
    network_apply_error: Option<String>,
    audio_candidates: Vec<audio::PlaybackApplication>,
    audio_scanning: bool,
    audio_scan_error: Option<String>,
    video_plan: Option<VideoPlan>,
    video_probe: Option<VideoProbe>,
    video_error: Option<String>,
    video_apply_error: Option<String>,
    pending_settings: Option<PendingSettings>,
    appearance: appearance::Appearance,
    approved_source: Option<&'static str>,
    active_share: Option<ShareSettings>,
    applying_share: Option<ShareSettings>,
    apply_share_error: Option<String>,
    notifications: UnboundedSender<notification::Kind>,
    tray_updates: Option<watch::Sender<TrayState>>,
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
const MEDIA_IDLE_GRACE: Duration = Duration::from_secs(2);
const MEDIA_RECOVERY_DELAY: Duration = Duration::from_millis(500);
const MAX_MEDIA_RECOVERIES: u8 = 3;

fn main() -> Result<()> {
    validate_arguments(std::env::args().skip(1))?;
    let (activation, activations) = iced::futures::channel::mpsc::channel(0);
    let Some(instance) = claim_instance(activation, INSTANCE_NAME)? else {
        return Ok(());
    };
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
        default_text_size: 14.0.into(),
        ..iced::Settings::default()
    })
    .theme(|app: &App, _| app.appearance.theme.clone())
    .subscription(|app| {
        let tick = if viewer_tick_enabled(app) {
            iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick)
        } else {
            iced::Subscription::none()
        };
        iced::Subscription::batch([
            window::close_requests().map(Message::Close),
            window::close_events().map(Message::Closed),
            window::resize_events().map(|(id, _)| Message::WindowResized(id)),
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

fn viewer_tick_enabled(app: &App) -> bool {
    app.window.is_some()
        && matches!(app.page, Page::Overview | Page::Viewers)
        && app.viewers.iter().any(web::Viewer::online)
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
    let draft = SettingsDraft::from_settings(&settings);
    let video = settings.video;
    let video_probe = VideoProbe::Current(video);
    let (tray_updates, tray_state) = watch::channel(TrayState {
        phase: Phase::Starting,
        online_viewers: 0,
    });
    let app = App {
        phase: Phase::Starting,
        link: String::new(),
        viewers: Vec::new(),
        commands: Some(commands),
        window: None,
        monitor_size: None,
        confirm_refresh: false,
        confirm_quit: false,
        confirm_apply_current: false,
        confirm_block: None,
        settings,
        draft,
        page: Page::Overview,
        copied_at: None,
        settings_error: None,
        network_apply_error: None,
        audio_candidates: Vec::new(),
        audio_scanning: false,
        audio_scan_error: None,
        video_plan: None,
        video_probe: Some(video_probe.clone()),
        video_error: None,
        video_apply_error: None,
        pending_settings: None,
        appearance: appearance::Appearance::default(),
        approved_source: None,
        active_share: None,
        applying_share: None,
        apply_share_error: None,
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
            Task::perform(probe_video_plan(video), move |result| {
                Message::VideoProbed(video_probe, result)
            }),
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
    let was_sharing = app.active_share.is_some();
    let previous_online = app.viewers.iter().filter(|viewer| viewer.online()).count();
    let task = update_app(app, message);
    let online = app.viewers.iter().filter(|viewer| viewer.online()).count();
    if let Some(updates) = &app.tray_updates {
        let next = TrayState {
            phase: app.phase.clone(),
            online_viewers: online,
        };
        updates.send_if_modified(|current| {
            if *current == next {
                false
            } else {
                current.clone_from(&next);
                true
            }
        });
    }
    if app.window.is_none()
        && app.settings.notifications
        && !app.quitting
        && let Some(kind) = notification_kind(
            &previous_phase,
            &app.phase,
            was_sharing,
            previous_online,
            online,
        )
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
    previous_online: usize,
    online: usize,
) -> Option<notification::Kind> {
    match (previous, current) {
        (Phase::Selecting, Phase::Sharing) => Some(notification::Kind::Started),
        (_, Phase::Waiting) if was_sharing => Some(notification::Kind::Stopped),
        (_, Phase::NetworkError(_) | Phase::Error(_))
            if *previous != Phase::Starting && previous != current =>
        {
            Some(notification::Kind::Error)
        }
        _ if previous_online == 0 && online > 0 => Some(notification::Kind::ViewerJoined),
        _ if previous_online > 0 && online == 0 => Some(notification::Kind::ViewerLeft),
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
                match app.page {
                    Page::Settings => SETTINGS_SCROLL_ID,
                    Page::Overview => OVERVIEW_SCROLL_ID,
                    Page::Viewers => VIEWERS_SCROLL_ID,
                },
            )));
        }
        Message::RevealFocus(delta) => {
            return iced::widget::operation::scroll_by(
                iced::widget::Id::new(match app.page {
                    Page::Settings => SETTINGS_SCROLL_ID,
                    Page::Overview => OVERVIEW_SCROLL_ID,
                    Page::Viewers => VIEWERS_SCROLL_ID,
                }),
                iced::widget::operation::AbsoluteOffset { x: 0.0, y: delta },
            );
        }
        Message::Start
            if app.phase != Phase::Waiting
                || app.pending_settings.is_some()
                || app.video_probe.is_some() => {}
        Message::Start => {
            let Some(video) = app
                .video_plan
                .filter(|plan| plan.settings == app.settings.video)
            else {
                app.video_error = Some("Video quality has not been checked".to_owned());
                return Task::none();
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
            if send_command(app, Command::End) {
                app.confirm_refresh = false;
                app.confirm_quit = false;
                app.confirm_apply_current = false;
                app.applying_share = None;
                app.phase = Phase::Ending;
            }
        }
        Message::Copy if app.link.is_empty() => {}
        Message::Copy => {
            let copied_at = Instant::now();
            app.copied_at = Some(copied_at);
            return Task::batch([
                clipboard::write(app.link.clone()),
                Task::perform(
                    async move { tokio::time::sleep(COPY_FEEDBACK_DURATION).await },
                    move |_| Message::CopyFeedbackExpired(copied_at),
                ),
            ]);
        }
        Message::CopyFeedbackExpired(copied_at) => {
            if app.copied_at == Some(copied_at) {
                app.copied_at = None;
            }
        }
        Message::Refresh => {
            let _ = send_command(app, Command::Refresh(false));
        }
        Message::Block(key) => {
            let now = Instant::now();
            let online = app
                .viewers
                .iter()
                .any(|viewer| viewer.key == key && viewer.online());
            if !online {
                app.confirm_block = None;
            } else if app.confirm_block.is_some_and(|confirmation| {
                confirmation.key == key
                    && now.saturating_duration_since(confirmation.started)
                        <= BLOCK_CONFIRMATION_DURATION
            }) {
                if send_command(app, Command::Disconnect(key)) {
                    app.confirm_block = None;
                }
            } else {
                app.confirm_block = Some(BlockConfirmation { key, started: now });
            }
        }
        Message::ConfirmRefresh => {
            if send_command(app, Command::Refresh(true)) {
                app.confirm_refresh = false;
            }
        }
        Message::CancelRefresh => app.confirm_refresh = false,
        Message::Show => return show_window(app),
        Message::Quit => {
            if app.phase == Phase::Sharing || app.draft.dirty(&app.settings) {
                app.confirm_refresh = false;
                app.confirm_quit = true;
                app.page = Page::Overview;
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
        Message::Page(page) => {
            let open_settings = page == Page::Settings && app.page != Page::Settings;
            app.page = page;
            app.confirm_block = None;
            app.confirm_apply_current = false;
            if open_settings {
                return scan_audio_applications(app);
            }
        }
        Message::NetworkSettings => {
            let open_settings = app.page != Page::Settings;
            app.page = Page::Settings;
            app.confirm_block = None;
            let focus = iced::widget::operation::focus(iced::widget::Id::new(NETWORK_ADDRESS_ID))
                .chain(accessibility::reveal_focused(iced::widget::Id::new(
                    SETTINGS_SCROLL_ID,
                )));
            return if open_settings {
                Task::batch([scan_audio_applications(app), focus])
            } else {
                focus
            };
        }
        Message::SystemAudio(_)
        | Message::AudioBitrate(_)
        | Message::CommunicationAudio(_)
        | Message::AudioExclusion(..)
        | Message::DeleteAudioExclusion(_)
        | Message::AddAudioExclusion(_)
        | Message::Notifications(_)
        | Message::NetworkAddress(_)
        | Message::NetworkPort(_)
        | Message::ShareBaseUrl(_)
        | Message::VideoPreset(_)
        | Message::VideoWidth(_)
        | Message::VideoHeight(_)
        | Message::VideoFps(_)
        | Message::VideoBitrate(_)
        | Message::VideoEncoder(_)
            if app.pending_settings.is_some() => {}
        Message::SystemAudio(system_audio) => {
            app.draft.settings.system_audio = system_audio;
            app.draft.changed();
            app.settings_error = None;
        }
        Message::AudioBitrate(bitrate_kbps) => {
            app.draft.settings.audio_bitrate_kbps = bitrate_kbps;
            app.draft.changed();
            app.settings_error = None;
        }
        Message::CommunicationAudio(enabled) => {
            app.draft.settings.exclude_communication_audio = enabled;
            app.draft.changed();
            app.settings_error = None;
        }
        Message::AudioExclusion(identity, enabled) => {
            if let Some(exclusion) = app
                .draft
                .settings
                .audio_exclusions
                .iter_mut()
                .find(|exclusion| exclusion.identity == identity)
            {
                exclusion.enabled = enabled;
                app.draft.changed();
                app.settings_error = None;
            }
        }
        Message::DeleteAudioExclusion(identity) => {
            let before = app.draft.settings.audio_exclusions.len();
            app.draft
                .settings
                .audio_exclusions
                .retain(|exclusion| exclusion.identity != identity);
            if app.draft.settings.audio_exclusions.len() != before {
                app.draft.changed();
                app.settings_error = None;
            }
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
                .draft
                .settings
                .audio_exclusions
                .iter()
                .any(|exclusion| exclusion.identity == application.identity)
            {
                app.draft
                    .settings
                    .audio_exclusions
                    .push(settings::AudioExclusion {
                        label: application.label,
                        identity: application.identity,
                        enabled: true,
                    });
                app.draft.changed();
                app.settings_error = None;
            }
        }
        Message::Notifications(notifications) => {
            app.draft.settings.notifications = notifications;
            app.draft.changed();
            app.settings_error = None;
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
            app.draft.network_address = address;
            app.draft.changed();
            app.settings_error = None;
            app.network_apply_error = None;
        }
        Message::NetworkPort(port) => {
            app.draft.network_port = port;
            app.draft.changed();
            app.settings_error = None;
            app.network_apply_error = None;
        }
        Message::ShareBaseUrl(base_url) => {
            app.draft.share_base_url = base_url;
            app.draft.changed();
            app.settings_error = None;
            app.network_apply_error = None;
        }
        Message::VideoPreset(preset) => {
            if let Some(video) = preset.video(app.draft.video_encoder) {
                set_video_draft(&mut app.draft, video);
            } else if app.draft.video_preset != Quality::Custom {
                app.draft.video_bitrate.clear();
            }
            app.draft.video_preset = preset;
            app.draft.changed();
            app.settings_error = None;
            app.video_apply_error = None;
        }
        Message::VideoWidth(width) => {
            app.draft.video_preset = Quality::Custom;
            app.draft.video_width = width;
            app.draft.changed();
            app.settings_error = None;
            app.video_apply_error = None;
        }
        Message::VideoHeight(height) => {
            app.draft.video_preset = Quality::Custom;
            app.draft.video_height = height;
            app.draft.changed();
            app.settings_error = None;
            app.video_apply_error = None;
        }
        Message::VideoFps(fps) => {
            app.draft.video_preset = Quality::Custom;
            app.draft.video_fps = fps;
            app.draft.changed();
            app.settings_error = None;
            app.video_apply_error = None;
        }
        Message::VideoBitrate(bitrate) => {
            app.draft.video_preset = Quality::Custom;
            app.draft.video_bitrate = bitrate;
            app.draft.changed();
            app.settings_error = None;
            app.video_apply_error = None;
        }
        Message::VideoEncoder(encoder) => {
            app.draft.video_encoder = encoder;
            app.draft.changed();
            app.settings_error = None;
            app.video_apply_error = None;
        }
        Message::ApplySettings if app.pending_settings.is_some() || app.video_probe.is_some() => {}
        Message::ApplySettings => {
            if !app.draft.dirty(&app.settings) {
                return Task::none();
            }
            app.video_apply_error = None;
            app.network_apply_error = None;
            let candidate = match app.draft.candidate() {
                Ok(candidate) => candidate,
                Err(error) => {
                    app.settings_error = Some(error.to_string());
                    return Task::none();
                }
            };
            if app.draft.network_dirty(&app.settings)
                && !matches!(app.phase, Phase::Waiting | Phase::NetworkError(_))
            {
                app.settings_error =
                    Some("Stop sharing before applying Network changes.".to_owned());
                return Task::none();
            }
            if let Some(plan) = app
                .video_plan
                .filter(|plan| plan.settings == candidate.video)
            {
                apply_settings_candidate(app, candidate, plan);
                return Task::none();
            }
            let probe = VideoProbe::Apply {
                revision: app.draft.revision,
                candidate,
            };
            let video = probe.video();
            app.video_probe = Some(probe.clone());
            app.settings_error = None;
            return Task::perform(probe_video_plan(video), move |result| {
                Message::VideoProbed(probe, result)
            });
        }
        Message::RevertSettings if app.pending_settings.is_some() => {}
        Message::RevertSettings => {
            if matches!(app.video_probe, Some(VideoProbe::Apply { .. })) {
                app.video_probe = None;
            }
            app.draft = SettingsDraft::from_settings(&app.settings);
            app.settings_error = None;
            app.video_apply_error = None;
            app.network_apply_error = None;
            app.confirm_apply_current = false;
        }
        Message::ApplyCurrentShare => {
            let Some(mut share) = saved_share(app) else {
                return Task::none();
            };
            if let Some(active) = app
                .active_share
                .as_ref()
                .filter(|active| active.video.settings == share.video.settings)
            {
                share.video = active.video;
            }
            if app.phase != Phase::Sharing
                || app
                    .active_share
                    .as_ref()
                    .is_some_and(|active| same_saved_media(active, &share))
                || app.applying_share.is_some()
            {
                return Task::none();
            }
            let online = app.viewers.iter().any(web::Viewer::online);
            if online && !app.confirm_apply_current {
                app.confirm_apply_current = true;
                return Task::none();
            }
            if send_command(app, Command::Apply(share.clone())) {
                app.applying_share = Some(share);
                app.apply_share_error = None;
                app.confirm_apply_current = false;
            }
        }
        Message::VideoProbed(probe, result) => {
            if app.video_probe.as_ref() != Some(&probe) {
                return Task::none();
            }
            if let VideoProbe::Apply {
                revision,
                candidate,
            } = &probe
                && (app.draft.revision != *revision
                    || !app.draft.candidate().is_ok_and(|draft| draft == *candidate))
            {
                app.video_probe = None;
                if app.draft.dirty(&app.settings) {
                    app.settings_error =
                        Some("Settings changed during the encoder check; apply again.".to_owned());
                }
                return Task::none();
            }
            app.video_probe = None;
            let video = probe.video();
            let result = result.and_then(|plan| {
                (plan.settings == video)
                    .then_some(plan)
                    .ok_or_else(|| "video encoder check returned the wrong settings".to_owned())
            });
            let plan = match result {
                Ok(plan) => plan,
                Err(error) => {
                    match &probe {
                        VideoProbe::Current(_) => {
                            app.video_error = Some(format!("Video quality unavailable: {error}"));
                        }
                        VideoProbe::Apply { .. } => {
                            let error = format!("Video quality unavailable: {error}");
                            app.video_apply_error = Some(error.clone());
                            app.settings_error = Some(error);
                        }
                    }
                    return Task::none();
                }
            };
            match &probe {
                VideoProbe::Current(_) if app.settings.video == video => {
                    app.video_plan = Some(plan);
                    app.video_error = None;
                }
                VideoProbe::Current(_) => {
                    app.video_error = Some("Saved video quality has not been checked".to_owned());
                }
                VideoProbe::Apply { candidate, .. } => {
                    apply_settings_candidate(app, candidate.clone(), plan);
                }
            }
        }
        Message::Tick => {
            if app.confirm_block.is_some_and(|confirmation| {
                confirmation.started.elapsed() > BLOCK_CONFIRMATION_DURATION
            }) {
                app.confirm_block = None;
            }
        }
        Message::WindowResized(id) if app.window == Some(id) => {
            return window::monitor_size(id).map(move |size| Message::MonitorSize(id, size));
        }
        Message::WindowResized(_) => {}
        Message::MonitorSize(id, Some(size))
            if app.window == Some(id) && app.monitor_size != Some(size) =>
        {
            app.monitor_size = Some(size);
            return window::set_min_size(id, Some(minimum_window_size(size)));
        }
        Message::MonitorSize(..) => {}
        Message::Close(id) => {
            if app.window.take_if(|window| *window == id).is_some() {
                app.monitor_size = None;
                return window::close(id);
            }
        }
        Message::Closed(id) => {
            if app.window == Some(id) {
                app.window = None;
                app.monitor_size = None;
            }
        }
        Message::Host(event) => match event {
            HostEvent::NetworkUnavailable(error) => {
                app.link.clear();
                app.copied_at = None;
                app.approved_source = None;
                app.pending_settings = None;
                app.confirm_quit = false;
                app.settings_error = Some(error.clone());
                app.network_apply_error = Some(error.clone());
                app.phase = Phase::NetworkError(error);
            }
            HostEvent::Waiting(link) => {
                app.link = link;
                app.copied_at = None;
                app.confirm_refresh = false;
                app.confirm_quit = false;
                app.approved_source = None;
                app.active_share = None;
                app.applying_share = None;
                app.apply_share_error = None;
                app.phase = Phase::Waiting;
            }
            HostEvent::Source(source) if app.phase == Phase::Selecting => {
                app.approved_source = Some(source);
            }
            HostEvent::Link(link) => {
                app.link = link;
                app.copied_at = None;
                app.viewers.clear();
                app.confirm_refresh = false;
                app.confirm_block = None;
            }
            HostEvent::ConfirmRefresh if matches!(app.phase, Phase::Waiting | Phase::Sharing) => {
                app.confirm_refresh = true;
                app.page = Page::Overview;
            }
            HostEvent::Sharing(share) if matches!(app.phase, Phase::Selecting | Phase::Sharing) => {
                if app
                    .applying_share
                    .as_ref()
                    .is_some_and(|applying| same_saved_media(applying, &share))
                {
                    app.applying_share = None;
                    app.apply_share_error = None;
                }
                app.active_share = Some(share);
                app.phase = Phase::Sharing;
            }
            HostEvent::ApplyFailed(error) if app.phase == Phase::Sharing => {
                app.applying_share = None;
                app.apply_share_error = Some(error);
            }
            HostEvent::Ending => {
                app.confirm_refresh = false;
                app.confirm_quit = false;
                app.confirm_apply_current = false;
                app.applying_share = None;
                app.phase = Phase::Ending;
            }
            HostEvent::Viewers(viewers) => {
                app.confirm_block = None;
                if !viewers.iter().any(web::Viewer::online) {
                    app.confirm_apply_current = false;
                }
                app.viewers = viewers;
            }
            HostEvent::NetworkApplied(result) => match result {
                Ok(settings) => {
                    let plan = app.pending_settings.take().and_then(|pending| {
                        (pending.candidate == settings).then_some(pending.video)
                    });
                    if let Some(plan) = plan {
                        app.settings = settings;
                        app.draft = SettingsDraft::from_settings(&app.settings);
                        app.video_plan = Some(plan);
                        app.video_error = None;
                        app.video_apply_error = None;
                        app.settings_error = None;
                        app.network_apply_error = None;
                    } else {
                        let error =
                            "Network applied unexpected settings; restart Aercast.".to_owned();
                        app.network_apply_error = Some(error.clone());
                        app.settings_error = Some(error);
                    }
                }
                Err(error) => {
                    app.pending_settings = None;
                    app.network_apply_error = Some(error.clone());
                    app.settings_error = Some(error);
                }
            },
            HostEvent::Stopped(result) => {
                app.commands = None;
                app.host_stopped = true;
                app.viewers.clear();
                app.confirm_refresh = false;
                app.confirm_quit = false;
                app.confirm_apply_current = false;
                app.confirm_block = None;
                app.approved_source = None;
                app.active_share = None;
                app.applying_share = None;
                app.pending_settings = None;
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
            HostEvent::Source(_)
            | HostEvent::Sharing(_)
            | HostEvent::ApplyFailed(_)
            | HostEvent::ConfirmRefresh => {}
        },
    }
    Task::none()
}

fn send_command(app: &mut App, command: Command) -> bool {
    let result = app
        .commands
        .as_ref()
        .map(|commands| commands.try_send(command));
    match result {
        Some(Ok(())) => true,
        Some(Err(mpsc::error::TrySendError::Full(_))) => false,
        None | Some(Err(mpsc::error::TrySendError::Closed(_))) => {
            app.confirm_refresh = false;
            app.confirm_quit = false;
            app.phase = Phase::Error("Host control is unavailable".to_owned());
            false
        }
    }
}

fn scan_audio_applications(app: &mut App) -> Task<Message> {
    if app.audio_scanning {
        return Task::none();
    }
    app.audio_scanning = true;
    app.audio_scan_error = None;
    Task::perform(audio::active_applications(), Message::AudioApplications)
}

fn audio_settings(settings: &settings::Settings) -> AudioSettings {
    AudioSettings {
        enabled: settings.system_audio,
        bitrate_kbps: settings.audio_bitrate_kbps,
        exclude_communication: settings.exclude_communication_audio,
        exclusions: settings
            .audio_exclusions
            .iter()
            .filter(|exclusion| exclusion.enabled)
            .map(|exclusion| exclusion.identity.clone())
            .collect(),
    }
}

fn set_video_draft(draft: &mut SettingsDraft, video: settings::VideoSettings) {
    draft.video_preset = Quality::from_video(video);
    draft.video_width = video.width.to_string();
    draft.video_height = video.height.to_string();
    draft.video_fps = video.fps;
    draft.video_bitrate = video
        .bitrate_mbps
        .map_or_else(String::new, |bitrate| bitrate.to_string());
    draft.video_encoder = video.encoder;
}

fn saved_share(app: &App) -> Option<ShareSettings> {
    app.video_plan
        .filter(|plan| plan.settings == app.settings.video)
        .map(|video| ShareSettings {
            audio: audio_settings(&app.settings),
            video,
        })
}

fn apply_settings_candidate(app: &mut App, candidate: settings::Settings, video: VideoPlan) {
    if app.draft.network_dirty(&app.settings) || matches!(app.phase, Phase::NetworkError(_)) {
        app.pending_settings = Some(PendingSettings {
            candidate: candidate.clone(),
            video,
        });
        if !send_command(app, Command::Network(candidate)) {
            app.pending_settings = None;
        }
    } else if let Err(error) = candidate.save() {
        app.settings_error = Some(format!("Settings unchanged: {error}"));
    } else {
        app.settings = candidate;
        app.draft = SettingsDraft::from_settings(&app.settings);
        app.video_plan = Some(video);
        app.video_error = None;
        app.video_apply_error = None;
        app.settings_error = None;
        app.network_apply_error = None;
        app.confirm_apply_current = false;
    }
}

fn minimum_window_size(monitor: iced::Size) -> iced::Size {
    let projected_width = monitor.width.min(monitor.height * 16.0 / 9.0);
    iced::Size::new((projected_width / 4.0).max(640.0), 480.0)
}

fn window_icon() -> window::Icon {
    static ICON: LazyLock<window::Icon> = LazyLock::new(|| {
        let decoder = png::Decoder::new(Cursor::new(include_bytes!("../assets/aercast-icon.png")));
        let mut reader = decoder
            .read_info()
            .expect("bundled window icon is valid PNG");
        let mut rgba = vec![0; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut rgba)
            .expect("bundled window icon decodes");
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        rgba.truncate(info.buffer_size());
        window::icon::from_rgba(rgba, info.width, info.height)
            .expect("bundled window icon has valid dimensions")
    });
    ICON.clone()
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
        return Task::batch([
            raise_window(id),
            window::monitor_size(id).map(move |size| Message::MonitorSize(id, size)),
        ]);
    }
    let (id, open) = window::open(window::Settings {
        size: iced::Size::new(960.0, 640.0),
        min_size: Some(iced::Size::new(640.0, 480.0)),
        resizable: true,
        icon: Some(window_icon()),
        platform_specific: window::settings::PlatformSpecific {
            application_id: "aercast".to_owned(),
            ..window::settings::PlatformSpecific::default()
        },
        exit_on_close_request: false,
        ..window::Settings::default()
    });
    app.window = Some(id);
    open.then(move |id| {
        Task::batch([
            raise_window(id),
            window::monitor_size(id).map(move |size| Message::MonitorSize(id, size)),
        ])
    })
}

fn raise_window(id: window::Id) -> Task<Message> {
    Task::batch([
        window::request_user_attention(id, Some(window::UserAttention::Informational)),
        window::gain_focus(id),
    ])
}

fn view(app: &App, _id: window::Id) -> Element<'_, Message> {
    let sidebar = sidebar(app);
    let content = match app.page {
        Page::Overview => overview_view(app),
        Page::Viewers => viewers_view(app),
        Page::Settings => settings_view(app),
    };
    row![sidebar, content]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn centered_button<'a>(
    content: impl Into<Element<'a, Message>>,
) -> iced::widget::Button<'a, Message> {
    button(
        container(content)
            .height(Length::Fill)
            .align_y(iced::alignment::Vertical::Center),
    )
}

fn overview_view(app: &App) -> Element<'_, Message> {
    let focus_ring = app.appearance.focus_ring();
    let status = match &app.phase {
        Phase::Starting => "Starting Aercast…",
        Phase::NetworkError(error) => error,
        Phase::Waiting if app.video_probe.is_some() => "Checking video encoder…",
        Phase::Waiting => app
            .video_error
            .as_deref()
            .unwrap_or("Ready. Capture has not started."),
        Phase::Selecting if app.approved_source.is_some() => "Starting media…",
        Phase::Selecting => "Choose one screen or window in the system picker.",
        Phase::Sharing if app.applying_share.is_some() => "Applying saved media settings…",
        Phase::Sharing => "Sharing.",
        Phase::Ending => "Ending share…",
        Phase::Error(error) => error,
    };
    let can_start = app.phase == Phase::Waiting
        && app.pending_settings.is_none()
        && app.video_probe.is_none()
        && app
            .video_plan
            .is_some_and(|plan| plan.settings == app.settings.video)
        && app.video_error.is_none();
    let (share_label, share_message) = match app.phase {
        Phase::Selecting => ("Cancel", Some(Message::End)),
        Phase::Sharing => ("Stop Sharing", Some(Message::End)),
        Phase::Ending => ("Stopping…", None),
        _ => ("Start Sharing", can_start.then_some(Message::Start)),
    };
    let refresh_confirmation =
        if app.confirm_refresh && matches!(app.phase, Phase::Waiting | Phase::Sharing) {
            column![
                text("Refreshing disconnects every current Viewer."),
                row![
                    accessibility::button(
                        centered_button("Cancel")
                            .style(|_, status| app.appearance.neutral_button(status)),
                        Some(Message::CancelRefresh),
                        focus_ring,
                    ),
                    accessibility::button(
                        centered_button("Refresh Link")
                            .style(|_, status| app.appearance.danger_button(status)),
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
    let quit_confirmation = if app.confirm_quit {
        let message = match (app.phase == Phase::Sharing, app.draft.dirty(&app.settings)) {
            (true, true) => "Quit Aercast, stop sharing, and discard unsaved settings?",
            (true, false) => "Quit Aercast and stop the active share?",
            (false, true) => "Quit Aercast and discard unsaved settings?",
            (false, false) => "Quit Aercast?",
        };
        column![
            text(message),
            row![
                accessibility::button(
                    centered_button("Cancel")
                        .style(|_, status| app.appearance.neutral_button(status)),
                    Some(Message::CancelQuit),
                    focus_ring,
                ),
                accessibility::button(
                    centered_button("Quit Aercast")
                        .style(|_, status| app.appearance.danger_button(status)),
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
    let status_color = if app.phase == Phase::Sharing {
        app.appearance.theme.extended_palette().primary.strong.color
    } else {
        app.appearance.secondary_text()
    };
    let status_row = column![text(status)];
    let status_row = if let Some(source) = app.approved_source {
        status_row.push(
            text(format!("Source: {source}"))
                .size(13)
                .color(app.appearance.secondary_text()),
        )
    } else {
        status_row
    }
    .spacing(4);
    let status_row = row![text("●").size(13).color(status_color), status_row]
        .spacing(8)
        .align_y(iced::Alignment::Start);
    let share_icon = if matches!(app.phase, Phase::Selecting | Phase::Sharing | Phase::Ending) {
        include_bytes!("../assets/stop-symbolic.svg").as_slice()
    } else {
        include_bytes!("../assets/play-symbolic.svg").as_slice()
    };
    let share_status = if share_message.is_some() {
        button::Status::Active
    } else {
        button::Status::Disabled
    };
    let share_icon_color = app.appearance.primary_button(share_status).text_color;
    let share_action = accessibility::button(
        centered_button(
            row![
                symbolic_icon(share_icon).style(move |_, _| svg::Style {
                    color: Some(share_icon_color)
                }),
                text(share_label).font(BOLD_FONT)
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        )
        .padding([0, 20])
        .style(move |_, status| app.appearance.primary_button(status)),
        share_message,
        focus_ring,
    );
    let copy_icon = if app.copied_at.is_some() {
        include_bytes!("../assets/check-symbolic.svg").as_slice()
    } else {
        include_bytes!("../assets/copy-symbolic.svg").as_slice()
    };
    let now = Instant::now();
    let health = viewer_summary(&app.viewers, now);
    let device_only = is_device_only(&app.settings);
    let active_media = app.active_share.as_ref().map_or_else(
        || "No active media pipeline.".to_owned(),
        |share| {
            let video = share.video.settings;
            let bitrate = video.bitrate_mbps.map_or_else(
                || "encoder default".to_owned(),
                |rate| format!("{rate} Mbps"),
            );
            let encoder = match share.video.encoder {
                Encoder::VaApi => "VA-API",
                Encoder::X264 => "x264",
            };
            let audio = if share.audio.enabled {
                format!("{} kbps audio", share.audio.bitrate_kbps)
            } else {
                "audio off".to_owned()
            };
            format!(
                "{}×{} · {} FPS · {bitrate} · {encoder} · {audio} · {} exclusions",
                video.width,
                video.height,
                video.fps,
                share.audio.exclusions.len()
            )
        },
    );
    let saved = saved_share(app);
    let saved_mismatch = app
        .active_share
        .as_ref()
        .zip(saved.as_ref())
        .is_some_and(|(active, saved)| !same_saved_media(active, saved));

    let lead = container(
        column![
            row![container(status_row).width(Length::Fill), share_action]
                .spacing(12)
                .align_y(iced::Alignment::Center),
            quit_confirmation,
        ]
        .spacing(8),
    )
    .padding(16)
    .width(Length::Fill)
    .style(|_| app.appearance.card());
    let details = container(
        column![
            text("Share link")
                .size(13)
                .color(app.appearance.secondary_text()),
            row![
                accessibility::text_input(
                    text_input("Share link will appear here", &app.link)
                        .style(|_, status| app.appearance.text_input(status)),
                    false,
                ),
                icon_button(
                    app,
                    include_bytes!("../assets/refresh-symbolic.svg"),
                    "Refresh link",
                    (matches!(app.phase, Phase::Waiting | Phase::Sharing) && !app.link.is_empty())
                        .then_some(Message::Refresh),
                ),
                icon_button(
                    app,
                    copy_icon,
                    if app.copied_at.is_some() {
                        "Link copied"
                    } else {
                        "Copy link"
                    },
                    (!app.link.is_empty()).then_some(Message::Copy),
                ),
            ]
            .spacing(8),
            if app.copied_at.is_some() {
                text("Copied").size(13).color(app.appearance.success_text())
            } else {
                text("").size(13)
            },
            if device_only {
                row![
                    text("This device only")
                        .size(13)
                        .font(BOLD_FONT)
                        .color(app.appearance.warning_text()),
                    accessibility::button(
                        centered_button("Open Network settings")
                            .style(|_, status| app.appearance.neutral_button(status)),
                        Some(Message::NetworkSettings),
                        focus_ring,
                    ),
                ]
                .spacing(12)
                .align_y(iced::Alignment::Center)
            } else {
                row![]
            },
            refresh_confirmation,
        ]
        .spacing(8),
    )
    .padding(16)
    .width(Length::Fill)
    .style(|_| app.appearance.card());

    let viewer_health = container(
        row![
            column![
                text("Viewer health").font(BOLD_FONT),
                text(format!(
                    "{}/{} online · worst RTT {} · worst Lag {}",
                    health.online,
                    health.total,
                    format_milliseconds(health.worst_rtt),
                    format_milliseconds(health.worst_lag),
                ))
                .size(13)
                .color(app.appearance.secondary_text()),
            ]
            .spacing(4)
            .width(Length::Fill),
            accessibility::button(
                centered_button("Open Viewers")
                    .style(|_, status| app.appearance.neutral_button(status)),
                Some(Message::Page(Page::Viewers)),
                focus_ring,
            ),
        ]
        .align_y(iced::Alignment::Center),
    )
    .padding(16)
    .width(Length::Fill)
    .style(|_| app.appearance.card());
    let media = container(
        column![
            row![
                text("Active media").font(BOLD_FONT),
                if saved_mismatch {
                    text("Saved differs")
                        .size(12)
                        .color(app.appearance.warning_text())
                } else {
                    text("").size(12)
                },
            ]
            .spacing(8),
            text(active_media)
                .size(13)
                .color(app.appearance.secondary_text()),
            if let Some(error) = app.apply_share_error.as_deref() {
                text(format!("⚠ {error}"))
                    .size(13)
                    .color(app.appearance.warning_text())
            } else {
                text("").size(13)
            },
        ]
        .spacing(4),
    )
    .padding(16)
    .width(Length::Fill)
    .style(|_| app.appearance.card());
    let body = column![
        text("Overview").size(20).font(BOLD_FONT),
        lead,
        details,
        viewer_health,
        media,
        text("Trusted LAN only. Use an external HTTPS reverse proxy elsewhere.")
            .size(13)
            .color(app.appearance.secondary_text()),
    ]
    .spacing(12)
    .max_width(960);
    let body = container(body).center_x(Length::Fill);

    container(
        scrollable(body)
            .id(iced::widget::Id::new(OVERVIEW_SCROLL_ID))
            .direction(scrollable::Direction::Vertical(hidden_scrollbar()))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .padding(20)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn sidebar(app: &App) -> Element<'_, Message> {
    let focus_ring = app.appearance.focus_ring();
    let online = app.viewers.iter().filter(|viewer| viewer.online()).count();
    let sidebar_item = |page: Page, icon: &'static [u8], label: String| {
        let selected = app.page == page;
        accessibility::button(
            centered_button(
                row![
                    container(space())
                        .width(2)
                        .height(18)
                        .style(move |_| app.appearance.sidebar_indicator(selected)),
                    symbolic_icon(icon)
                        .width(16)
                        .height(16)
                        .style(move |_, _| svg::Style {
                            color: Some(if selected {
                                app.appearance.theme.palette().text
                            } else {
                                app.appearance.secondary_text()
                            }),
                        }),
                    text(label).size(14),
                ]
                .spacing(10)
                .align_y(iced::Alignment::Center),
            )
            .width(Length::Fill)
            .padding([6, 10])
            .style(move |_, status| app.appearance.sidebar_item(selected, status)),
            Some(Message::Page(page)),
            focus_ring,
        )
    };
    let status_text = match &app.phase {
        Phase::Starting => "Starting…",
        Phase::NetworkError(_) => "Network error",
        Phase::Waiting => "Ready",
        Phase::Selecting => "Selecting…",
        Phase::Sharing => "Sharing",
        Phase::Ending => "Stopping…",
        Phase::Error(_) => "Error",
    };
    let status_color = if app.phase == Phase::Sharing {
        app.appearance.theme.extended_palette().primary.strong.color
    } else {
        app.appearance.secondary_text()
    };
    container(
        column![
            text("Aercast")
                .size(14)
                .color(app.appearance.secondary_text()),
            column![
                sidebar_item(
                    Page::Overview,
                    include_bytes!("../assets/overview-symbolic.svg"),
                    "Overview".to_owned(),
                ),
                sidebar_item(
                    Page::Viewers,
                    include_bytes!("../assets/viewers-symbolic.svg"),
                    format!("Viewers ({online})"),
                ),
                sidebar_item(
                    Page::Settings,
                    include_bytes!("../assets/settings-symbolic.svg"),
                    if app.draft.dirty(&app.settings) {
                        "Settings · Changed".to_owned()
                    } else {
                        "Settings".to_owned()
                    },
                ),
            ]
            .spacing(2),
            space().height(Length::Fill),
            row![
                text("●").size(11).color(status_color),
                text(status_text)
                    .size(12)
                    .color(app.appearance.secondary_text()),
                space().width(Length::Fill),
                text(concat!("v", env!("CARGO_PKG_VERSION")))
                    .size(12)
                    .color(app.appearance.secondary_text()),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(16)
        .height(Length::Fill),
    )
    .padding([16, 12])
    .width(appearance::SIDEBAR_WIDTH)
    .height(Length::Fill)
    .style(|_| app.appearance.sidebar())
    .into()
}

fn icon_button<'a>(
    app: &'a App,
    icon: &'static [u8],
    label: &'a str,
    message: Option<Message>,
) -> Element<'a, Message> {
    let appearance = app.appearance.clone();
    tooltip(
        accessibility::button(
            centered_button(
                container(symbolic_icon(icon).width(14).height(14))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .center(Length::Fill),
            )
            .width(appearance::CONTROL_HEIGHT)
            .padding(0)
            .style(move |_, status| appearance.neutral_button(status)),
            message,
            app.appearance.focus_ring(),
        ),
        text(label),
        tooltip::Position::Bottom,
    )
    .gap(8)
    .padding(8)
    .delay(Duration::from_millis(400))
    .into()
}

fn hidden_scrollbar() -> scrollable::Scrollbar {
    scrollable::Scrollbar::new().width(0).scroller_width(0)
}

fn viewers_view(app: &App) -> Element<'_, Message> {
    const BULLET_WIDTH: f32 = 13.0;
    const STATE_WIDTH: f32 = 88.0;
    const ACTION_WIDTH: f32 = 112.0;

    let focus_ring = app.appearance.focus_ring();
    let now = Instant::now();
    let online = app.viewers.iter().filter(|viewer| viewer.online()).count();
    let ip_counts =
        app.viewers
            .iter()
            .fold(HashMap::<IpAddr, usize>::new(), |mut counts, viewer| {
                *counts.entry(viewer.ip).or_default() += 1;
                counts
            });
    let mut ip_seen = HashMap::<IpAddr, usize>::new();
    let viewer_rows = app
        .viewers
        .iter()
        .enumerate()
        .fold(column![], |rows, (index, viewer)| {
            let online = viewer.online();
            let state_color = if online {
                app.appearance.theme.extended_palette().primary.strong.color
            } else {
                app.appearance.secondary_text()
            };
            let (rtt, playback_lag) = viewer.telemetry(now);
            let confirming = online
                && app.confirm_block.is_some_and(|confirmation| {
                    confirmation.key == viewer.key
                        && now.saturating_duration_since(confirmation.started)
                            <= BLOCK_CONFIRMATION_DURATION
                });
            let total_for_ip = *ip_counts.get(&viewer.ip).unwrap_or(&1);
            let ip_label = if total_for_ip > 1 {
                let count = ip_seen.entry(viewer.ip).or_insert(0);
                *count += 1;
                format!("{} #{}", viewer.ip, *count)
            } else {
                viewer.ip.to_string()
            };
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
                            text("●").size(13).color(state_color).width(BULLET_WIDTH),
                            row![
                                text(ip_label)
                                    .size(14)
                                    .width(Length::Fill)
                                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                                text(if online { "Online" } else { "Offline" })
                                    .size(13)
                                    .color(state_color)
                                    .align_x(iced::alignment::Horizontal::Right)
                                    .width(STATE_WIDTH),
                                accessibility::button(
                                    centered_button(if confirming {
                                        "Confirm block"
                                    } else {
                                        "Block"
                                    })
                                    .width(ACTION_WIDTH)
                                    .style(
                                        move |_, status| {
                                            if confirming {
                                                app.appearance.danger_button(status)
                                            } else {
                                                app.appearance.neutral_button(status)
                                            }
                                        }
                                    ),
                                    online.then_some(Message::Block(viewer.key)),
                                    focus_ring,
                                ),
                            ]
                            .spacing(8)
                            .align_y(iced::Alignment::Center)
                            .width(Length::Fill),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                        row![
                            space().width(BULLET_WIDTH),
                            row![
                                row![
                                    text(format!(
                                        "Connected {}",
                                        format_duration(viewer.duration())
                                    ))
                                    .size(13)
                                    .color(app.appearance.secondary_text())
                                    .width(Length::FillPortion(1)),
                                    text(format!("RTT {}", format_milliseconds(rtt)))
                                        .size(13)
                                        .color(app.appearance.secondary_text())
                                        .align_x(iced::alignment::Horizontal::Center)
                                        .wrapping(iced::widget::text::Wrapping::None)
                                        .width(Length::FillPortion(1)),
                                ]
                                .spacing(8)
                                .width(Length::Fill),
                                text(format!("Lag {}", format_milliseconds(playback_lag)))
                                    .size(13)
                                    .color(app.appearance.secondary_text())
                                    .align_x(iced::alignment::Horizontal::Right)
                                    .wrapping(iced::widget::text::Wrapping::None)
                                    .width(STATE_WIDTH),
                                space().width(ACTION_WIDTH),
                            ]
                            .spacing(8)
                            .width(Length::Fill),
                        ]
                        .spacing(8),
                    ]
                    .spacing(4),
                )
                .padding([8, 12]),
            )
        });
    let viewer_rows = if app.viewers.is_empty() {
        viewer_rows.push(
            container(
                text("No Viewers have connected yet.")
                    .size(13)
                    .color(app.appearance.secondary_text()),
            )
            .padding(16),
        )
    } else {
        viewer_rows
    };

    container(
        column![
            row![
                text("Viewers")
                    .size(20)
                    .font(BOLD_FONT)
                    .color(app.appearance.theme.palette().text),
                container(text(format!("{online}/{} online", app.viewers.len())).size(13))
                    .padding([4, 8])
                    .style(|_| app.appearance.metric()),
                space().width(Length::Fill),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            container(
                scrollable(viewer_rows)
                    .id(iced::widget::Id::new(VIEWERS_SCROLL_ID))
                    .direction(scrollable::Direction::Vertical(hidden_scrollbar(),))
                    .height(Length::Fill),
            )
            .style(|_| app.appearance.card())
            .height(Length::Fill)
            .width(Length::Fill),
        ]
        .spacing(12)
        .max_width(960)
        .height(Length::Fill),
    )
    .padding(20)
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .into()
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[derive(Debug, PartialEq)]
struct ViewerSummary {
    online: usize,
    total: usize,
    worst_rtt: Option<Duration>,
    worst_lag: Option<Duration>,
}

fn viewer_summary(viewers: &[web::Viewer], now: Instant) -> ViewerSummary {
    let mut summary = ViewerSummary {
        online: 0,
        total: viewers.len(),
        worst_rtt: None,
        worst_lag: None,
    };
    for viewer in viewers.iter().filter(|viewer| viewer.online()) {
        summary.online += 1;
        let (rtt, lag) = viewer.telemetry(now);
        summary.worst_rtt = summary.worst_rtt.max(rtt);
        summary.worst_lag = summary.worst_lag.max(lag);
    }
    summary
}

fn is_device_only(settings: &settings::Settings) -> bool {
    settings.listen_address.is_loopback() && settings.share_base_url.is_none()
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
        centered_button(text(label))
            .width(Length::Fill)
            .style(move |_, status| {
                if selected {
                    app.appearance.selected_button(status)
                } else {
                    app.appearance.neutral_button(status)
                }
            }),
        app.pending_settings.is_none().then_some(message),
        app.appearance.focus_ring(),
    )
}

fn settings_section<'a>(
    app: &'a App,
    title: &'a str,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    column![
        text(title).size(14).color(app.appearance.secondary_text()),
        container(content)
            .padding(16)
            .width(Length::Fill)
            .style(|_| app.appearance.card()),
    ]
    .spacing(8)
    .into()
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
    let draft_dirty = app.draft.dirty(&app.settings);
    let video_input_error = app
        .draft
        .settings
        .with_video(
            &app.draft.video_width,
            &app.draft.video_height,
            app.draft.video_fps,
            &app.draft.video_bitrate,
            app.draft.video_encoder,
        )
        .err()
        .map(|error| error.to_string());
    let network_input_error = app
        .draft
        .settings
        .with_network(
            &app.draft.network_address,
            &app.draft.network_port,
            &app.draft.share_base_url,
        )
        .err()
        .map(|error| error.to_string());
    let candidate = app.draft.candidate();
    let candidate_valid = candidate.is_ok();
    let network_dirty = app.draft.network_dirty(&app.settings);
    let applying = app.pending_settings.is_some() || app.video_probe.is_some();
    let editable = app.pending_settings.is_none();
    let saved = saved_share(app);
    let active_dirty = app
        .active_share
        .as_ref()
        .zip(saved.as_ref())
        .is_some_and(|(active, saved)| !same_saved_media(active, saved));
    let hint = if app.video_probe.is_some() {
        "Checking video encoder…"
    } else {
        match &app.phase {
            Phase::Starting => "Starting Aercast…",
            Phase::NetworkError(error) => error,
            Phase::Waiting => "Used when the next share starts.",
            Phase::Sharing if app.applying_share.is_some() => {
                "Applying saved media settings to the current share…"
            }
            Phase::Sharing if active_dirty => "Saved settings differ from the current share.",
            Phase::Sharing => "The current share uses this setting.",
            Phase::Selecting => "This share uses the value selected before the Portal opened.",
            Phase::Ending => "Ending share… The saved setting will be used next time.",
            Phase::Error(error) => error,
        }
    };
    let fps_options = FPS_OPTIONS.into_iter().fold(row![], |options, fps| {
        options.push(settings_option(
            app,
            format!("{fps} FPS"),
            app.draft.video_fps == fps,
            Message::VideoFps(fps),
        ))
    });
    let custom_quality = column![
        row![
            column![
                text("Width")
                    .size(13)
                    .color(app.appearance.secondary_text()),
                accessibility::text_input(
                    text_input("1280", &app.draft.video_width)
                        .on_input_maybe(editable.then_some(Message::VideoWidth))
                        .style(|_, status| app.appearance.text_input(status)),
                    editable,
                ),
            ]
            .spacing(4)
            .width(Length::Fill),
            column![
                text("Height")
                    .size(13)
                    .color(app.appearance.secondary_text()),
                accessibility::text_input(
                    text_input("720", &app.draft.video_height)
                        .on_input_maybe(editable.then_some(Message::VideoHeight))
                        .style(|_, status| app.appearance.text_input(status)),
                    editable,
                ),
            ]
            .spacing(4)
            .width(Length::Fill),
        ]
        .spacing(12),
        row![
            column![
                text("Frame rate")
                    .size(13)
                    .color(app.appearance.secondary_text()),
                fps_options.spacing(8),
            ]
            .spacing(4)
            .width(Length::Fill),
            column![
                text("Bitrate (Mbps)")
                    .size(13)
                    .color(app.appearance.secondary_text()),
                accessibility::text_input(
                    text_input("Encoder default", &app.draft.video_bitrate)
                        .on_input_maybe(editable.then_some(Message::VideoBitrate))
                        .style(|_, status| app.appearance.text_input(status)),
                    editable,
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
                app.draft.video_preset == left,
                Message::VideoPreset(left),
            ),
            settings_option(
                app,
                right.to_string(),
                app.draft.video_preset == right,
                Message::VideoPreset(right),
            ),
        ]
        .spacing(8)
    };
    let quality = column![
        preset_row(Quality::P720, Quality::P1080),
        preset_row(Quality::P1440, Quality::Custom),
        custom_quality,
    ]
    .spacing(8);
    let encoder_options = [
        settings::VideoEncoder::Auto,
        settings::VideoEncoder::VaApi,
        settings::VideoEncoder::X264,
    ]
    .into_iter()
    .filter(|encoder| *encoder == app.draft.video_encoder || video_encoder_available(*encoder))
    .fold(row![], |options, encoder| {
        options.push(settings_option(
            app,
            video_encoder_label(encoder).to_owned(),
            app.draft.video_encoder == encoder,
            Message::VideoEncoder(encoder),
        ))
    });
    let audio_bitrate_options =
        settings::AUDIO_BITRATES_KBPS
            .into_iter()
            .fold(row![], |options, bitrate| {
                options.push(settings_option(
                    app,
                    format!("{bitrate} kbps"),
                    app.draft.settings.audio_bitrate_kbps == bitrate,
                    Message::AudioBitrate(bitrate),
                ))
            });
    let configured_media_rate = candidate.as_ref().ok().and_then(|settings| settings.video.bitrate_mbps).map_or_else(
        || {
            format!(
                "Configured media rate: encoder-default video + {} kbps audio (transport overhead excluded).",
                app.draft.settings.audio_bitrate_kbps
            )
        },
        |video| {
            format!(
                "Configured media rate: about {video}.{:03} Mbps (transport overhead excluded).",
                app.draft.settings.audio_bitrate_kbps
            )
        },
    );
    let quality = quality
        .push(text("Encoder").size(14))
        .push(encoder_options.spacing(8))
        .push(
            text(if sharing {
                "Apply the full page first, then apply saved media settings to this share."
            } else {
                "Saved quality is used by the next Start."
            })
            .size(13)
            .color(app.appearance.secondary_text()),
        );
    let quality = if let Some(error) = video_input_error
        .as_deref()
        .or(app.video_apply_error.as_deref())
        .or(app.video_error.as_deref())
    {
        quality.push(
            text(format!("⚠ {error}"))
                .size(13)
                .color(app.appearance.warning_text())
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        )
    } else {
        quality
    };
    let exclusion_rows = app
        .draft
        .settings
        .audio_exclusions
        .iter()
        .fold(
            column![accessibility::checkbox(
                checkbox(app.draft.settings.exclude_communication_audio)
                    .label("Communication audio")
                    .style(|_, status| app.appearance.checkbox(status)),
                app.draft.settings.exclude_communication_audio,
                editable.then_some(Message::CommunicationAudio as fn(bool) -> Message),
                focus_ring,
            )],
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
                            editable.then_some(move |enabled| {
                                Message::AudioExclusion(toggle_identity.clone(), enabled)
                            }),
                            focus_ring,
                        ),
                        accessibility::button(
                            centered_button("Delete")
                                .style(|_, status| app.appearance.neutral_button(status)),
                            editable.then_some(Message::DeleteAudioExclusion(identity)),
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
            text("Add from active applications").size(14),
            space().width(Length::Fill),
            accessibility::button(
                centered_button(if app.audio_scanning {
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
        !app.draft
            .settings
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
                        .size(12)
                        .color(app.appearance.secondary_text())
                        .width(Length::Fill)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                ]
                .spacing(4)
                .width(Length::Fill),
                accessibility::button(
                    centered_button("Add").style(|_, status| app.appearance.neutral_button(status)),
                    editable.then_some(Message::AddAudioExclusion(application.clone())),
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
                .size(13)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );
    } else if !app.audio_scanning && !has_application {
        application_rows = application_rows.push(
            text("No other playback applications are active.")
                .size(13)
                .color(app.appearance.secondary_text()),
        );
    }
    let audio = column![
        accessibility::checkbox(
            checkbox(app.draft.settings.system_audio)
                .label("System audio")
                .style(|_, status| app.appearance.checkbox(status)),
            app.draft.settings.system_audio,
            editable.then_some(Message::SystemAudio as fn(bool) -> Message),
            focus_ring,
        ),
        text("Audio bitrate").size(14),
        audio_bitrate_options.spacing(8),
        text(configured_media_rate)
            .size(13)
            .color(app.appearance.secondary_text()),
        text(hint).size(13).color(app.appearance.secondary_text()),
        text("Excluded applications").size(14),
        exclusion_rows,
        application_rows,
    ]
    .spacing(12);
    let network = column![
        row![
            column![
                text("Listen address")
                    .size(13)
                    .color(app.appearance.secondary_text()),
                accessibility::text_input(
                    text_input("127.0.0.1", &app.draft.network_address)
                        .id(iced::widget::Id::new(NETWORK_ADDRESS_ID))
                        .on_input_maybe(editable.then_some(Message::NetworkAddress))
                        .style(|_, status| app.appearance.text_input(status)),
                    editable,
                ),
            ]
            .spacing(4)
            .width(Length::FillPortion(3)),
            column![
                text("Port").size(13).color(app.appearance.secondary_text()),
                accessibility::text_input(
                    text_input("8877", &app.draft.network_port)
                        .on_input_maybe(editable.then_some(Message::NetworkPort))
                        .style(|_, status| app.appearance.text_input(status)),
                    editable,
                ),
            ]
            .spacing(4)
            .width(Length::FillPortion(1)),
        ]
        .spacing(12),
        text("Share base URL (optional)")
            .size(13)
            .color(app.appearance.secondary_text()),
        accessibility::text_input(
            text_input("https://host:port", &app.draft.share_base_url)
                .on_input_maybe(editable.then_some(Message::ShareBaseUrl))
                .style(|_, status| app.appearance.text_input(status)),
            editable,
        ),
        text("Network changes apply only while stopped.")
            .size(13)
            .color(app.appearance.secondary_text()),
        text("Changing the listener may leave old waiting pages unable to recover.")
            .size(13)
            .color(app.appearance.secondary_text()),
        if let Some(error) = network_input_error
            .as_deref()
            .or(app.network_apply_error.as_deref())
        {
            text(format!("⚠ {error}"))
                .size(13)
                .color(app.appearance.warning_text())
        } else {
            text("").size(13)
        },
    ]
    .spacing(12);
    let notifications = column![accessibility::checkbox(
        checkbox(app.draft.settings.notifications)
            .label("Desktop notifications")
            .style(|_, status| app.appearance.checkbox(status)),
        app.draft.settings.notifications,
        editable.then_some(Message::Notifications as fn(bool) -> Message),
        focus_ring,
    ),]
    .spacing(12);
    let sections = column![
        settings_section(app, "Quality", quality),
        settings_section(app, "Audio", audio),
        settings_section(app, "Network", network),
        settings_section(app, "Notifications", notifications),
    ]
    .spacing(20);
    let body = sections.max_width(960);
    let blocked_by_network =
        network_dirty && !matches!(app.phase, Phase::Waiting | Phase::NetworkError(_));
    let can_apply = draft_dirty && candidate_valid && !applying && !blocked_by_network;
    let footer_status = if let Some(error) = app.settings_error.as_deref() {
        format!("⚠ {error}")
    } else if let Some(error) = app.apply_share_error.as_deref() {
        format!("⚠ {error}")
    } else if blocked_by_network {
        "⚠ Stop sharing before applying Network changes.".to_owned()
    } else if app.video_probe.is_some() {
        "Checking video encoder…".to_owned()
    } else if app.pending_settings.is_some() {
        "Applying settings…".to_owned()
    } else if draft_dirty {
        "Draft has unsaved changes.".to_owned()
    } else if active_dirty {
        "Saved settings differ from the active share.".to_owned()
    } else {
        "Saved".to_owned()
    };
    let primary = if sharing && (active_dirty || app.applying_share.is_some()) && !draft_dirty {
        let label = if app.applying_share.is_some() {
            "Applying to current share…"
        } else if app.confirm_apply_current {
            "Confirm apply to current share"
        } else {
            "Apply to current share"
        };
        accessibility::button(
            centered_button(label).style(|_, status| app.appearance.primary_button(status)),
            (active_dirty && app.applying_share.is_none()).then_some(Message::ApplyCurrentShare),
            focus_ring,
        )
    } else {
        accessibility::button(
            centered_button(if applying { "Applying…" } else { "Apply" })
                .style(|_, status| app.appearance.primary_button(status)),
            can_apply.then_some(Message::ApplySettings),
            focus_ring,
        )
    };
    let footer = row![
        text(footer_status)
            .size(13)
            .color(app.appearance.secondary_text())
            .width(Length::Fill)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        accessibility::button(
            centered_button("Revert").style(|_, status| app.appearance.neutral_button(status)),
            (draft_dirty && app.pending_settings.is_none()).then_some(Message::RevertSettings),
            focus_ring,
        ),
        primary,
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    container(
        column![
            text("Settings")
                .size(20)
                .font(BOLD_FONT)
                .color(app.appearance.theme.palette().text),
            row![
                container(text("Saved").size(12))
                    .padding([4, 8])
                    .style(|_| app.appearance.metric()),
                container(
                    text(if draft_dirty {
                        "Draft · Changed"
                    } else {
                        "Draft · Saved"
                    })
                    .size(12),
                )
                .padding([4, 8])
                .style(|_| app.appearance.metric()),
                container(
                    text(if app.active_share.is_none() {
                        "Active · None"
                    } else if active_dirty {
                        "Active · Differs"
                    } else {
                        "Active · Matches"
                    })
                    .size(12),
                )
                .padding([4, 8])
                .style(|_| app.appearance.metric()),
            ]
            .spacing(8),
            scrollable(body)
                .id(iced::widget::Id::new(SETTINGS_SCROLL_ID))
                .direction(scrollable::Direction::Vertical(hidden_scrollbar(),))
                .width(Length::Fill)
                .height(Length::Fill),
            rule::horizontal(if app.appearance.high_contrast { 2 } else { 1 })
                .style(|_| app.appearance.separator()),
            footer,
        ]
        .spacing(12)
        .max_width(960)
        .height(Length::Fill),
    )
    .padding(20)
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .into()
}

fn symbolic_icon<'a>(bytes: &'static [u8]) -> iced::widget::Svg<'a> {
    svg(svg::Handle::from_memory(bytes))
        .width(16)
        .height(16)
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
                        ShareStop::Sleep | ShareStop::Wake => {
                            return Err(
                                io::Error::other("internal media state escaped the share").into()
                            );
                        }
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

fn begin_media_apply(
    current: &mut ShareSettings,
    rollback: &mut Option<ShareSettings>,
    next: ShareSettings,
    recoveries: &mut u8,
    fallback_attempted: &mut bool,
    capture_caps: &mut Option<gst::Caps>,
) {
    *rollback = Some(current.clone());
    if current.video != next.video {
        *capture_caps = None;
    }
    *current = next;
    *recoveries = 0;
    *fallback_attempted = false;
}

fn same_saved_media(left: &ShareSettings, right: &ShareSettings) -> bool {
    left.audio == right.audio && left.video.settings == right.video.settings
}

fn media_apply_failure(
    attempt: &Result<ShareStop>,
    reached_sharing: bool,
    rollback_pending: bool,
) -> Option<&Error> {
    if reached_sharing || !rollback_pending {
        None
    } else {
        attempt.as_ref().err()
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
    let mut current = share;
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
    let mut fallback_attempted = false;
    let mut sleeping = false;
    let mut rollback = None;
    let result = loop {
        if sleeping {
            match share_control(
                commands,
                async {
                    let _ = closed.next().await;
                },
                server,
                host,
                link_base,
                events,
                None,
            )
            .await
            {
                ShareStop::Apply(next) => {
                    begin_media_apply(
                        &mut current,
                        &mut rollback,
                        next,
                        &mut recoveries,
                        &mut fallback_attempted,
                        &mut capture_caps,
                    );
                    sleeping = false;
                }
                ShareStop::Wake => {
                    if let Err(error) = host.clear_media_demand() {
                        break Ok(ShareStop::Failed(error.into()));
                    }
                    sleeping = false;
                }
                ShareStop::Sleep => continue,
                stop => break Ok(stop),
            }
        }

        let (fragment_ready, ready) = watch::channel(false);
        let control = share_control(
            commands,
            async {
                let _ = closed.next().await;
            },
            server,
            host,
            link_base,
            events,
            Some(ready),
        );
        tokio::pin!(control);
        let mut media = None;
        let mut reached_sharing = false;
        let mut attempt: Result<ShareStop> = tokio::select! {
            biased;
            stop = control.as_mut() => Ok(stop),
            remote = portal.open_pipe_wire_remote(&session, Default::default()) => match remote {
                Err(error) => Err(error.into()),
                Ok(remote) => {
                    if let Some(stop) = control.as_mut().now_or_never() {
                        Ok(stop)
                    } else {
                        match host.start() {
                            Err(error) => Err(error.into()),
                            Ok(active) => {
                                let description = pipeline_description(
                                    node_id,
                                    remote.as_raw_fd(),
                                    current.video,
                                    current.audio.bitrate_kbps,
                                );
                                media = Some(active.clone());
                                serve_video(
                                    &description,
                                    &mut capture_caps,
                                    current.clone(),
                                    active,
                                    fragment_ready,
                                    &mut reached_sharing,
                                    control.as_mut(),
                                    events,
                                )
                                .await
                            }
                        }
                    }
                }
            },
        };

        if matches!(&attempt, Ok(ShareStop::Sleep))
            && let Err(error) = host.clear_media_demand()
        {
            attempt = Ok(ShareStop::Failed(error.into()));
        }
        if let Some(media) = media {
            let stopped = host.stop(&media).and_then(|()| host.viewers());
            if let Ok(viewers) = &stopped {
                let _ = events.unbounded_send(HostEvent::Viewers(viewers.clone()));
            }
            if let Err(error) = stopped {
                eprintln!("Failed to stop media session: {error}");
                attempt = Ok(ShareStop::Failed(error.into()));
            }
        }

        if reached_sharing {
            rollback = None;
        }
        if let Ok(ShareStop::Apply(next)) = &attempt {
            begin_media_apply(
                &mut current,
                &mut rollback,
                next.clone(),
                &mut recoveries,
                &mut fallback_attempted,
                &mut capture_caps,
            );
            continue;
        }
        if matches!(&attempt, Ok(ShareStop::Sleep)) {
            recoveries = 0;
            sleeping = true;
            continue;
        }
        if matches!(&attempt, Ok(ShareStop::Wake)) {
            attempt = Ok(ShareStop::Failed(
                io::Error::other("media woke while already active").into(),
            ));
        }
        if attempt.is_err()
            && let Some(stop) = control.as_mut().now_or_never()
        {
            match stop {
                ShareStop::Apply(next) => {
                    begin_media_apply(
                        &mut current,
                        &mut rollback,
                        next,
                        &mut recoveries,
                        &mut fallback_attempted,
                        &mut capture_caps,
                    );
                    continue;
                }
                ShareStop::Sleep => {
                    recoveries = 0;
                    sleeping = true;
                    continue;
                }
                ShareStop::Wake => {}
                stop => break Ok(stop),
            }
        }

        if let Some(error) = media_apply_failure(&attempt, reached_sharing, rollback.is_some()) {
            let error = error.to_string();
            current = rollback.take().expect("rollback checked above");
            capture_caps = None;
            recoveries = 0;
            fallback_attempted = false;
            let _ = events.unbounded_send(HostEvent::ApplyFailed(format!(
                "Could not apply the saved media settings: {error}. Restored the previous active settings."
            )));
            continue;
        }

        if attempt.as_ref().err().is_some_and(|error| {
            should_fallback(current.video, fallback_attempted, recoveries, error)
        }) {
            fallback_attempted = true;
            eprintln!(
                "VA-API media path failed; probing x264 for recovery {}/{MAX_MEDIA_RECOVERIES}",
                recoveries + 1,
            );
            let settings = current.video.settings;
            let mut probe =
                tokio::task::spawn_blocking(move || plan_encoder(settings, Encoder::X264));
            enum Fallback<T> {
                Control(ShareStop),
                Probe(T),
            }
            let fallback = tokio::select! {
                biased;
                stop = control.as_mut() => Fallback::Control(stop),
                result = &mut probe => Fallback::Probe(result),
            };
            match fallback {
                Fallback::Control(ShareStop::Apply(next)) => {
                    begin_media_apply(
                        &mut current,
                        &mut rollback,
                        next,
                        &mut recoveries,
                        &mut fallback_attempted,
                        &mut capture_caps,
                    );
                    continue;
                }
                Fallback::Control(ShareStop::Sleep) => {
                    fallback_attempted = false;
                    recoveries = 0;
                    sleeping = true;
                    continue;
                }
                Fallback::Control(ShareStop::Wake) => {}
                Fallback::Control(stop) => break Ok(stop),
                Fallback::Probe(Ok(Ok(plan))) => {
                    recoveries += 1;
                    current.video = plan;
                    capture_caps = None;
                    continue;
                }
                Fallback::Probe(Ok(Err(error))) => attempt = Err(error),
                Fallback::Probe(Err(error)) => {
                    attempt =
                        Err(io::Error::other(format!("x264 encoder check failed: {error}")).into());
                }
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
        tokio::select! {
            biased;
            stop = control.as_mut() => match stop {
                ShareStop::Apply(next) => {
                    begin_media_apply(
                        &mut current,
                        &mut rollback,
                        next,
                        &mut recoveries,
                        &mut fallback_attempted,
                        &mut capture_caps,
                    );
                    continue;
                }
                ShareStop::Sleep => {
                    recoveries = 0;
                    sleeping = true;
                    continue;
                }
                ShareStop::Wake => {}
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
        Ok(ShareStop::Sleep | ShareStop::Wake) => {
            Err(io::Error::other("internal media state escaped the share").into())
        }
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
    mut media_ready: Option<watch::Receiver<bool>>,
) -> ShareStop {
    let mut viewer_updates = match host.viewer_updates() {
        Ok(viewers) => viewers,
        Err(error) => return ShareStop::Failed(error.into()),
    };
    let mut media_demand = match host.media_demand() {
        Ok(demand) => demand,
        Err(error) => return ShareStop::Failed(error.into()),
    };
    let viewers = match host.viewers() {
        Ok(viewers) => viewers,
        Err(error) => return ShareStop::Failed(error.into()),
    };
    let mut online = viewers.iter().filter(|viewer| viewer.online()).count();
    let _ = events.unbounded_send(HostEvent::Viewers(viewers));
    let ready = media_ready.as_ref().is_some_and(|ready| *ready.borrow());
    let mut deadline = idle_deadline(None, ready, online, Instant::now());
    tokio::pin!(session_closed);
    loop {
        enum ControlEvent {
            Command(Option<Command>),
            Signal(io::Result<()>),
            Viewers(bool),
            Ready(bool),
            Demand(bool),
            Idle,
            PortalClosed,
            Server(std::result::Result<io::Result<()>, tokio::task::JoinError>),
        }
        let sleeping = media_ready.is_none();
        let idle_at = deadline;
        let event = tokio::select! {
            biased;
            command = commands.recv() => ControlEvent::Command(command),
            _ = &mut session_closed => ControlEvent::PortalClosed,
            result = &mut *server => ControlEvent::Server(result),
            signal = tokio::signal::ctrl_c() => ControlEvent::Signal(signal),
            changed = viewer_updates.changed() => ControlEvent::Viewers(changed.is_ok()),
            changed = async {
                match media_ready.as_mut() {
                    Some(ready) => ready.changed().await.is_ok(),
                    None => std::future::pending().await,
                }
            } => ControlEvent::Ready(changed),
            changed = async {
                if sleeping {
                    let requested = *media_demand.borrow() != 0;
                    if requested {
                        true
                    } else {
                        media_demand.changed().await.is_ok()
                    }
                } else {
                    std::future::pending().await
                }
            } => ControlEvent::Demand(changed),
            _ = async {
                match idle_at {
                    Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                    None => std::future::pending().await,
                }
            } => ControlEvent::Idle,
        };
        match event {
            ControlEvent::Signal(signal) => {
                return match signal {
                    Ok(()) => {
                        println!("Stopping Aercast.");
                        ShareStop::Quit
                    }
                    Err(error) => ShareStop::Failed(error.into()),
                };
            }
            ControlEvent::Command(command) => match command.unwrap_or(Command::Quit) {
                Command::Start(..) => println!("A share is already active."),
                Command::Apply(audio) => return ShareStop::Apply(audio),
                Command::Network(_) => {
                    let _ = events.unbounded_send(HostEvent::NetworkApplied(Err(
                        "Stop sharing before applying network settings".to_owned(),
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
                        media_demand = match host.media_demand() {
                            Ok(demand) => demand,
                            Err(error) => return ShareStop::Failed(error.into()),
                        };
                        let viewers = match host.viewers() {
                            Ok(viewers) => viewers,
                            Err(error) => return ShareStop::Failed(error.into()),
                        };
                        online = viewers.iter().filter(|viewer| viewer.online()).count();
                        let ready = media_ready.as_ref().is_some_and(|ready| *ready.borrow());
                        deadline = idle_deadline(deadline, ready, online, Instant::now());
                        let _ = events.unbounded_send(HostEvent::Viewers(viewers));
                        let _ =
                            events.unbounded_send(HostEvent::Link(format!("{link_base}{path}")));
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
            ControlEvent::Viewers(open) => {
                if !open {
                    return ShareStop::Failed(
                        io::Error::other("Viewer update channel closed").into(),
                    );
                }
                viewer_updates.borrow_and_update();
                match host.viewers() {
                    Ok(viewers) => {
                        online = viewers.iter().filter(|viewer| viewer.online()).count();
                        let ready = media_ready.as_ref().is_some_and(|ready| *ready.borrow());
                        deadline = idle_deadline(deadline, ready, online, Instant::now());
                        let _ = events.unbounded_send(HostEvent::Viewers(viewers));
                    }
                    Err(error) => return ShareStop::Failed(error.into()),
                }
            }
            ControlEvent::Ready(open) => {
                let Some(ready) = media_ready.as_mut() else {
                    return ShareStop::Failed(
                        io::Error::other("sleeping media received a ready event").into(),
                    );
                };
                if !open {
                    return ShareStop::Failed(
                        io::Error::other("Media readiness channel closed").into(),
                    );
                }
                ready.borrow_and_update();
                deadline = idle_deadline(deadline, *ready.borrow(), online, Instant::now());
            }
            ControlEvent::Demand(open) => {
                if !open {
                    return ShareStop::Failed(
                        io::Error::other("Media demand channel closed").into(),
                    );
                }
                let requested = *media_demand.borrow_and_update() != 0;
                if requested {
                    return ShareStop::Wake;
                }
            }
            ControlEvent::Idle => {
                let ready = media_ready.as_ref().is_some_and(|ready| *ready.borrow());
                let viewers = match host.viewers() {
                    Ok(viewers) => viewers,
                    Err(error) => return ShareStop::Failed(error.into()),
                };
                online = viewers.iter().filter(|viewer| viewer.online()).count();
                if ready && online == 0 {
                    return ShareStop::Sleep;
                }
                deadline = idle_deadline(None, ready, online, Instant::now());
                let _ = events.unbounded_send(HostEvent::Viewers(viewers));
            }
            ControlEvent::PortalClosed => {
                println!("Portal session closed; stopping stream.");
                return ShareStop::PortalClosed;
            }
            ControlEvent::Server(result) => {
                return server_outcome(result).unwrap_or_else(ShareStop::Failed);
            }
        }
    }
}

fn idle_deadline(
    current: Option<Instant>,
    ready: bool,
    online: usize,
    now: Instant,
) -> Option<Instant> {
    if ready && online == 0 {
        current.or(Some(now + MEDIA_IDLE_GRACE))
    } else {
        None
    }
}

fn should_retry<T, E>(outcome: &std::result::Result<T, E>, recoveries: u8) -> bool {
    outcome.is_err() && recoveries < MAX_MEDIA_RECOVERIES
}

fn should_fallback(video: VideoPlan, attempted: bool, recoveries: u8, error: &Error) -> bool {
    video.settings.encoder == settings::VideoEncoder::Auto
        && video.encoder == Encoder::VaApi
        && !attempted
        && recoveries < MAX_MEDIA_RECOVERIES
        && error.is::<HardwareVideoFailure>()
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

#[allow(clippy::too_many_arguments)]
async fn serve_video(
    description: &str,
    capture_caps: &mut Option<gst::Caps>,
    share: ShareSettings,
    media: web::MediaSession,
    fragment_ready: watch::Sender<bool>,
    reached_sharing: &mut bool,
    mut control: Pin<&mut impl Future<Output = ShareStop>>,
    events: &Events,
) -> Result<ShareStop> {
    let audio = &share.audio;
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
                let fragment_ready = fragment_ready.clone();
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
                        fragment_ready.send_replace(true);
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
        Err(error) => match messages.next().now_or_never().flatten() {
            Some(message) => queued_media_outcome(Some(message), &mut messages),
            None => Err(error.into()),
        },
        Ok(_) => match audio_exclusions
            .map(|exclusions| audio::start(audio_source, audio.exclude_communication, exclusions))
            .transpose()
        {
            Err(error) => Err(error.into()),
            Ok(mut audio_capture) => {
                println!("Browser stream running.");
                *reached_sharing = true;
                let _ = events.unbounded_send(HostEvent::Sharing(share.clone()));
                let mut audio_failure_reported = false;
                let mut running = tokio::select! {
                    biased;
                    stop = control.as_mut() => Ok(stop),
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
                    message = messages.next() => {
                        queued_media_outcome(message, &mut messages)
                    },
                };
                if matches!(
                    &running,
                    Ok(ShareStop::End | ShareStop::Quit | ShareStop::PortalClosed)
                ) {
                    let _ = events.unbounded_send(HostEvent::Ending);
                }
                if matches!(&running, Ok(ShareStop::Sleep))
                    && let Err(error) = pipeline.set_state(gst::State::Paused)
                {
                    running = Ok(ShareStop::Failed(error.into()));
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

async fn probe_video_plan(
    video: settings::VideoSettings,
) -> std::result::Result<VideoPlan, String> {
    tokio::task::spawn_blocking(move || video_plan(&video).map_err(|error| error.to_string()))
        .await
        .map_err(|error| format!("video encoder check failed: {error}"))?
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
    let pipeline = build_pipeline(&pipeline_description(
        1,
        0,
        plan,
        settings::DEFAULT_AUDIO_BITRATE_KBPS,
    ))?;
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

fn pipeline_description(
    node_id: u32,
    remote_fd: i32,
    plan: VideoPlan,
    audio_bitrate_kbps: u32,
) -> String {
    let video = plan.settings;
    let video_pipeline = match plan.encoder {
        Encoder::VaApi => {
            let bitrate = video.bitrate_mbps.map_or_else(String::new, |bitrate| {
                let bitrate = u32::from(bitrate) * 1_000;
                format!(" bitrate={bitrate} cpb-size={}", bitrate / 10)
            });
            format!(
                "vapostproc name=video-converter add-borders=true ! video/x-raw(memory:VAMemory),format=NV12,width={width},height={height} ! imagefreeze is-live=true allow-replace=true ! video/x-raw(memory:VAMemory),format=NV12,framerate={fps}/1 ! vah264enc name=encoder rate-control=cbr target-usage=7{bitrate} key-int-max={fps} ! video/x-h264,profile=constrained-baseline,stream-format=byte-stream,alignment=au",
                width = video.width,
                height = video.height,
                fps = video.fps,
            )
        }
        Encoder::X264 => {
            let bitrate = video.bitrate_mbps.map_or_else(String::new, |bitrate| {
                format!(
                    " bitrate={} vbv-buf-capacity=100 nal-hrd=cbr",
                    u32::from(bitrate) * 1_000
                )
            });
            format!(
                "videoconvertscale name=video-converter add-borders=true ! video/x-raw,format=I420,width={width},height={height} ! imagefreeze is-live=true allow-replace=true ! video/x-raw,format=I420,framerate={fps}/1 ! x264enc name=encoder tune=zerolatency speed-preset=ultrafast{bitrate} key-int-max={fps}",
                width = video.width,
                height = video.height,
                fps = video.fps,
            )
        }
    };
    format!(
        "mp4mux name=mux fragment-duration=100 ! appsink name=stream sync=false wait-on-eos=false
         audiomixer name=audio-mixer ignore-inactive-pads=true ! audioconvert ! audio/x-raw,format=F32LE,rate=48000,channels=2 ! avenc_aac bitrate={audio_bitrate} ! aacparse ! audio/mpeg,mpegversion=4,stream-format=raw ! queue ! mux.audio_0
         audiotestsrc is-live=true wave=silence ! audio/x-raw,format=F32LE,rate=48000,channels=2 ! queue ! audio-mixer.
         appsrc name=system-audio is-live=true format=time do-timestamp=true block=false max-bytes=384000 leaky-type=downstream ! audio/x-raw,format=F32LE,rate=48000,channels=2,layout=interleaved ! queue ! audio-mixer.
         pipewiresrc name=portal-video fd={remote_fd} path={node_id} on-disconnect=error ! capsfilter name=portal-format ! {video_pipeline} ! h264parse name=h264 ! video/x-h264,stream-format=avc,alignment=au ! queue ! mux.video_0",
        audio_bitrate = audio_bitrate_kbps * 1_000,
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

fn queued_media_outcome(
    first: Option<gst::Message>,
    messages: &mut (impl futures_util::Stream<Item = gst::Message> + Unpin),
) -> Result<ShareStop> {
    media_outcome(first.into_iter().chain(std::iter::from_fn(|| {
        messages.next().now_or_never().flatten()
    })))
}

fn media_outcome(messages: impl IntoIterator<Item = gst::Message>) -> Result<ShareStop> {
    let mut descriptions = Vec::new();
    let mut all_hardware_video = true;
    let mut saw_error = false;
    for message in messages {
        match message.view() {
            gst::MessageView::Eos(..) => {
                descriptions.push("capture stream ended".to_owned());
                all_hardware_video = false;
            }
            gst::MessageView::Error(error) => {
                saw_error = true;
                all_hardware_video &= hardware_video_error(&message);
                let source = message
                    .src()
                    .map(|source| source.path_string().to_string())
                    .unwrap_or_else(|| "unknown".to_owned());
                descriptions.push(format!(
                    "GStreamer error from {source}: {} ({})",
                    error.error(),
                    error.debug().unwrap_or_default(),
                ));
            }
            _ => {
                descriptions.push("unexpected GStreamer bus message".to_owned());
                all_hardware_video = false;
            }
        }
    }
    if descriptions.is_empty() {
        return Err(io::Error::other("GStreamer bus closed").into());
    }
    let description = descriptions.join("; ");
    if saw_error && all_hardware_video {
        Err(HardwareVideoFailure(description).into())
    } else {
        Err(io::Error::other(description).into())
    }
}

fn hardware_video_error(message: &gst::Message) -> bool {
    let gst::MessageView::Error(message_error) = message.view() else {
        return false;
    };
    let Some(source) = message.src().map(|source| source.name().to_string()) else {
        return false;
    };
    let error = message_error.error();
    let details = message_error.details();
    let flow = details
        .filter(|details| details.has_field("flow-return"))
        .and_then(|details| details.get::<gst::FlowReturn>("flow-return").ok());
    let missing_flow = details.is_none_or(|details| !details.has_field("flow-return"));
    match source.as_str() {
        "encoder" => {
            error.matches(gst::StreamError::Encode)
                || error.matches(gst::LibraryError::Init)
                || error.matches(gst::LibraryError::Failed)
                || error.matches(gst::CoreError::Negotiation)
        }
        "video-converter" => {
            error.matches(gst::LibraryError::Init)
                || error.matches(gst::ResourceError::Settings)
                || error.matches(gst::CoreError::Negotiation)
                || error.matches(gst::CoreError::NotImplemented)
        }
        "portal-video" => {
            error.matches(gst::StreamError::Format)
                || (error.matches(gst::StreamError::Failed)
                    && flow == Some(gst::FlowReturn::NotNegotiated))
        }
        "portal-format" => error.matches(gst::StreamError::Format),
        "h264" => {
            error.matches(gst::StreamError::Format)
                || error.matches(gst::StreamError::Decode)
                || error.matches(gst::StreamError::WrongType)
                || (error.matches(gst::StreamError::Failed)
                    && (missing_flow || flow == Some(gst::FlowReturn::NotNegotiated)))
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
