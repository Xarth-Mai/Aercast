use crate::media::{Encoder, VideoPlan, audio_settings, probe_video_plan, same_saved_media};
use crate::share_session::run_host;
use crate::{
    Command, HostEvent, Result, ShareSettings, accessibility, appearance, audio, notification,
    settings, tray, web,
};
use iced::{
    Element, Length, Task, Theme, clipboard,
    futures::channel::mpsc::{Receiver, Sender, UnboundedSender},
    widget::{
        button, checkbox, column, container, row, rule, scrollable, space, svg, text, text_input,
        tooltip,
    },
    window,
};
use std::{
    cell::Cell,
    collections::HashMap,
    io::{self, Cursor},
    net::IpAddr,
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};

const INSTANCE_NAME: &str = "org.aercast.Aercast";
const INSTANCE_PATH: &str = "/org/aercast/Aercast";
const OVERVIEW_SCROLL_ID: &str = "overview";
const VIEWERS_SCROLL_ID: &str = "viewers";
const SETTINGS_SCROLL_ID: &str = "settings";
const NETWORK_ADDRESS_ID: &str = "network-address";
const COPY_FEEDBACK_DURATION: Duration = Duration::from_millis(1_500);
const BLOCK_CONFIRMATION_DURATION: Duration = Duration::from_secs(3);
const MEDIUM_FONT: iced::Font = iced::Font {
    weight: iced::font::Weight::Medium,
    ..iced::Font::DEFAULT
};
const BOLD_FONT: iced::Font = iced::Font {
    weight: iced::font::Weight::Bold,
    ..iced::Font::DEFAULT
};

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

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum VideoProbe {
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
pub(crate) enum Quality {
    P720,
    P1080,
    P1440,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Page {
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

#[derive(Clone, Debug)]
pub(crate) enum Message {
    Host(HostEvent),
    Start,
    RestartHost,
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
pub(crate) enum Phase {
    Starting,
    NetworkError(String),
    Waiting,
    Selecting,
    Sharing,
    Ending,
    Error(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TrayState {
    pub(crate) phase: Phase,
    pub(crate) online_viewers: usize,
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
    media_idle: bool,
    active_share: Option<ShareSettings>,
    applying_share: Option<ShareSettings>,
    apply_share_error: Option<String>,
    notifications: UnboundedSender<notification::Kind>,
    tray_updates: Option<watch::Sender<TrayState>>,
    tray_stopped: bool,
    host_stopped: bool,
    quitting: bool,
}

fn instance_name(new: bool) -> String {
    if new {
        format!("{INSTANCE_NAME}.Instance{}", std::process::id())
    } else {
        INSTANCE_NAME.to_owned()
    }
}

pub(crate) fn run(new: bool) -> Result<()> {
    let (activation, activations) = iced::futures::channel::mpsc::channel(0);
    let Some(instance) = claim_instance(activation, &instance_name(new))? else {
        return Ok(());
    };
    let mut settings = settings::Settings::load()?;
    settings.temporary = new;
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
        default_text_size: 15.0.into(),
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
    let (notifications, notification_requests) = iced::futures::channel::mpsc::unbounded();
    let (tray_messages, tray_events) = iced::futures::channel::mpsc::unbounded();
    let (commands, host_task) = start_host(settings.clone());
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
        media_idle: false,
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
            Task::perform(probe_video_plan(video), move |result| {
                Message::VideoProbed(video_probe, result)
            }),
            host_task,
        ]),
    )
}

fn start_host(settings: settings::Settings) -> (mpsc::Sender<Command>, Task<Message>) {
    let (commands, receiver) = mpsc::channel(8);
    let (events, incoming) = iced::futures::channel::mpsc::unbounded();
    let task = Task::batch([
        Task::run(incoming, Message::Host),
        Task::future(async move {
            let result = run_host(settings, events.clone(), receiver).await;
            let _ = events.unbounded_send(HostEvent::Stopped(
                result.map_err(|error| error.to_string()),
            ));
        })
        .discard(),
    ]);
    (commands, task)
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
        Message::RestartHost => {
            if app.host_stopped && matches!(app.phase, Phase::Error(_)) {
                let (commands, task) = start_host(app.settings.clone());
                app.commands = Some(commands);
                app.host_stopped = false;
                app.phase = Phase::Starting;
                return task;
            }
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
                app.media_idle = false;
                app.phase = Phase::Selecting;
            }
        }
        Message::End if !matches!(app.phase, Phase::Selecting | Phase::Sharing) => {}
        Message::End => {
            if send_command(app, Command::End) {
                app.confirm_refresh = false;
                app.confirm_quit = false;
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
                    Some("Stop sharing before applying Network changes".to_owned());
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
            if send_command(app, Command::Apply(share.clone())) {
                app.applying_share = Some(share);
                app.apply_share_error = None;
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
                        Some("Settings changed during the encoder check; apply again".to_owned());
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
                app.media_idle = false;
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
                app.media_idle = false;
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
            HostEvent::MediaIdle(idle)
                if matches!(app.phase, Phase::Selecting | Phase::Sharing) =>
            {
                app.media_idle = idle;
            }
            HostEvent::Ending => {
                app.media_idle = false;
                app.confirm_refresh = false;
                app.confirm_quit = false;
                app.applying_share = None;
                app.phase = Phase::Ending;
            }
            HostEvent::Viewers(viewers) => {
                app.confirm_block = None;
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
                            "Network applied unexpected settings; restart Aercast".to_owned();
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
                app.link.clear();
                app.copied_at = None;
                app.viewers.clear();
                app.confirm_refresh = false;
                app.confirm_quit = false;
                app.confirm_block = None;
                app.approved_source = None;
                app.media_idle = false;
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
            | HostEvent::MediaIdle(_)
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
            app.media_idle = false;
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

fn overview_status(app: &App) -> String {
    match &app.phase {
        Phase::Starting => "Starting Aercast…",
        Phase::NetworkError(error) => error,
        Phase::Waiting if app.video_probe.is_some() => "Checking video encoder…",
        Phase::Waiting => app
            .video_error
            .as_deref()
            .unwrap_or("Ready. Capture has not started"),
        Phase::Selecting if app.approved_source.is_some() => "Starting media…",
        Phase::Selecting => "Choose one screen or window in the system picker",
        Phase::Sharing if app.applying_share.is_some() => "Applying saved media settings…",
        Phase::Sharing => {
            let mut status = app.approved_source.map_or_else(
                || "Sharing".to_owned(),
                |source| format!("Sharing: {source}"),
            );
            if app.media_idle {
                status.push_str(" · idle");
            }
            return status;
        }
        Phase::Ending => "Ending share…",
        Phase::Error(error) => error,
    }
    .to_owned()
}

fn overview_view(app: &App) -> Element<'_, Message> {
    let focus_ring = app.appearance.focus_ring();
    let status = overview_status(app);
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
        Phase::Error(_) if app.host_stopped => ("Restart Host", Some(Message::RestartHost)),
        _ => ("Start Sharing", can_start.then_some(Message::Start)),
    };
    let refresh_confirmation =
        if app.confirm_refresh && matches!(app.phase, Phase::Waiting | Phase::Sharing) {
            column![
                text("Refreshing disconnects every current Viewer"),
                row![
                    accessibility::button(
                        centered_button(text("Cancel").font(MEDIUM_FONT))
                            .style(|_, status| app.appearance.neutral_button(status)),
                        Some(Message::CancelRefresh),
                        focus_ring,
                    ),
                    accessibility::button(
                        centered_button(text("Refresh Link").font(MEDIUM_FONT))
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
                    centered_button(text("Cancel").font(MEDIUM_FONT))
                        .style(|_, status| app.appearance.neutral_button(status)),
                    Some(Message::CancelQuit),
                    focus_ring,
                ),
                accessibility::button(
                    centered_button(text("Quit Aercast").font(MEDIUM_FONT))
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
    let status_row = row![text("●").size(13).color(status_color), text(status)]
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
        || "No active media pipeline".to_owned(),
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
                .size(14)
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
                text("Copied").size(14).color(app.appearance.success_text())
            } else {
                text("").size(14)
            },
            if device_only {
                row![
                    text("This device only")
                        .size(14)
                        .font(BOLD_FONT)
                        .color(app.appearance.warning_text()),
                    accessibility::button(
                        centered_button(text("Open Network settings").font(MEDIUM_FONT))
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
            text("Trusted LAN only. Use an external HTTPS reverse proxy elsewhere")
                .size(14)
                .color(app.appearance.secondary_text()),
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
                text("Viewer health").size(14).font(MEDIUM_FONT),
                text(format!(
                    "{}/{} online · worst RTT {} · worst Lag {}",
                    health.online,
                    health.total,
                    format_milliseconds(health.worst_rtt),
                    format_milliseconds(health.worst_lag),
                ))
                .size(14)
                .color(app.appearance.secondary_text()),
            ]
            .spacing(4)
            .width(Length::Fill),
            accessibility::button(
                centered_button(text("Open Viewers").font(MEDIUM_FONT))
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
                text("Active media").size(14).font(MEDIUM_FONT),
                if saved_mismatch {
                    text("Saved differs")
                        .size(14)
                        .color(app.appearance.warning_text())
                } else {
                    text("").size(14)
                },
            ]
            .spacing(8),
            text(active_media)
                .size(14)
                .color(app.appearance.secondary_text()),
            if let Some(error) = app.apply_share_error.as_deref() {
                text(format!("⚠ {error}"))
                    .size(14)
                    .color(app.appearance.warning_text())
            } else {
                text("").size(14)
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
                    text(label).size(15).font(MEDIUM_FONT),
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
        Phase::Waiting => "Ready",
        Phase::Selecting => "Selecting…",
        Phase::Sharing => "Sharing",
        Phase::Ending => "Stopping…",
        Phase::NetworkError(_) | Phase::Error(_) => "Error",
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
                    "Settings".to_owned(),
                ),
            ]
            .spacing(2),
            space().height(Length::Fill),
            row![
                text("●").size(11).color(status_color),
                text(status_text)
                    .size(14)
                    .color(app.appearance.secondary_text()),
                space().width(Length::Fill),
                text(crate::version_label(
                    env!("CARGO_PKG_VERSION"),
                    env!("AERCAST_PROFILE"),
                ))
                .size(14)
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
                                    .size(15)
                                    .width(Length::Fill)
                                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                                text(if online { "Online" } else { "Offline" })
                                    .size(14)
                                    .color(state_color)
                                    .align_x(iced::alignment::Horizontal::Right)
                                    .width(STATE_WIDTH),
                                accessibility::button(
                                    centered_button(
                                        text(if confirming { "Confirm block" } else { "Block" })
                                            .font(MEDIUM_FONT)
                                    )
                                    .width(ACTION_WIDTH)
                                    .style(move |_, status| {
                                        if confirming {
                                            app.appearance.danger_button(status)
                                        } else {
                                            app.appearance.neutral_button(status)
                                        }
                                    }),
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
                                    .size(14)
                                    .color(app.appearance.secondary_text())
                                    .width(Length::FillPortion(1)),
                                    text(format!("RTT {}", format_milliseconds(rtt)))
                                        .size(14)
                                        .color(app.appearance.secondary_text())
                                        .align_x(iced::alignment::Horizontal::Center)
                                        .wrapping(iced::widget::text::Wrapping::None)
                                        .width(Length::FillPortion(1)),
                                ]
                                .spacing(8)
                                .width(Length::Fill),
                                text(format!("Lag {}", format_milliseconds(playback_lag)))
                                    .size(14)
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
                text("No Viewers have connected yet")
                    .size(14)
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
                container(text(format!("{online}/{} online", app.viewers.len())).size(14))
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
        centered_button(text(label).font(MEDIUM_FONT))
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
        text(title)
            .size(14)
            .font(MEDIUM_FONT)
            .color(app.appearance.secondary_text()),
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
        settings::VideoEncoder::VaApi => "VA-API",
        settings::VideoEncoder::X264 => "x264",
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
            Phase::Waiting => "Used when the next share starts",
            Phase::Sharing if app.applying_share.is_some() => {
                "Applying saved media settings to the current share…"
            }
            Phase::Sharing if active_dirty => "Saved settings differ from the current share",
            Phase::Sharing => "The current share uses this setting",
            Phase::Selecting => "This share uses the value selected before the Portal opened",
            Phase::Ending => "Ending share… The saved setting will be used next time",
            Phase::Error(error) => error,
        }
    };
    let fps_options = FPS_OPTIONS.into_iter().fold(row![], |options, fps| {
        options.push(settings_option(
            app,
            fps.to_string(),
            app.draft.video_fps == fps,
            Message::VideoFps(fps),
        ))
    });
    let custom_quality = column![
        row![
            column![
                text("Width")
                    .size(14)
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
                    .size(14)
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
                text("Frame rate (FPS)")
                    .size(14)
                    .color(app.appearance.secondary_text()),
                fps_options.spacing(8),
            ]
            .spacing(4)
            .width(Length::Fill),
            column![
                text("Bitrate (Mbps)")
                    .size(14)
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
                "Configured media rate: encoder-default video + {} kbps audio (transport overhead excluded)",
                app.draft.settings.audio_bitrate_kbps
            )
        },
        |video| {
            format!(
                "Configured media rate: about {video}.{:03} Mbps (transport overhead excluded)",
                app.draft.settings.audio_bitrate_kbps
            )
        },
    );
    let quality = quality
        .push(text("Encoder").size(14).font(MEDIUM_FONT))
        .push(encoder_options.spacing(8))
        .push(
            text(if sharing {
                "Apply the full page first, then apply saved media settings to this share"
            } else {
                "Saved quality is used by the next Start"
            })
            .size(14)
            .color(app.appearance.secondary_text()),
        );
    let quality = if let Some(error) = video_input_error
        .as_deref()
        .or(app.video_apply_error.as_deref())
        .or(app.video_error.as_deref())
    {
        quality.push(
            text(format!("⚠ {error}"))
                .size(14)
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
                            centered_button(text("Delete").font(MEDIUM_FONT))
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
            text("Add from active applications")
                .size(14)
                .font(MEDIUM_FONT),
            space().width(Length::Fill),
            accessibility::button(
                centered_button(
                    text(if app.audio_scanning {
                        "Scanning…"
                    } else {
                        "Refresh"
                    })
                    .font(MEDIUM_FONT)
                )
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
                        .size(14)
                        .color(app.appearance.secondary_text())
                        .width(Length::Fill)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                ]
                .spacing(4)
                .width(Length::Fill),
                accessibility::button(
                    centered_button(text("Add").font(MEDIUM_FONT))
                        .style(|_, status| app.appearance.neutral_button(status)),
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
                .size(14)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );
    } else if !app.audio_scanning && !has_application {
        application_rows = application_rows.push(
            text("No other playback applications are active")
                .size(14)
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
        text("Audio bitrate").size(14).font(MEDIUM_FONT),
        audio_bitrate_options.spacing(8),
        text(configured_media_rate)
            .size(14)
            .color(app.appearance.secondary_text()),
        text(hint).size(14).color(app.appearance.secondary_text()),
        text("Excluded applications").size(14).font(MEDIUM_FONT),
        exclusion_rows,
        application_rows,
    ]
    .spacing(12);
    let network = column![
        row![
            column![
                text("Listen address")
                    .size(14)
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
                text("Port").size(14).color(app.appearance.secondary_text()),
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
            .size(14)
            .color(app.appearance.secondary_text()),
        accessibility::text_input(
            text_input("https://host:port", &app.draft.share_base_url)
                .on_input_maybe(editable.then_some(Message::ShareBaseUrl))
                .style(|_, status| app.appearance.text_input(status)),
            editable,
        ),
        text("Network changes apply only while stopped")
            .size(14)
            .color(app.appearance.secondary_text()),
        text("Changing the listener may leave old waiting pages unable to recover")
            .size(14)
            .color(app.appearance.secondary_text()),
        if let Some(error) = network_input_error
            .as_deref()
            .or(app.network_apply_error.as_deref())
        {
            text(format!("⚠ {error}"))
                .size(14)
                .color(app.appearance.warning_text())
        } else {
            text("").size(14)
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
        "⚠ Stop sharing before applying Network changes".to_owned()
    } else if app.video_probe.is_some() {
        "Checking video encoder…".to_owned()
    } else if app.pending_settings.is_some() {
        "Applying settings…".to_owned()
    } else if draft_dirty {
        "Draft has unsaved changes".to_owned()
    } else if active_dirty {
        "Saved settings differ from the active share".to_owned()
    } else if app.settings.temporary {
        "Applied for this instance only".to_owned()
    } else {
        "Saved".to_owned()
    };
    let primary = if sharing && (active_dirty || app.applying_share.is_some()) && !draft_dirty {
        let label = if app.applying_share.is_some() {
            "Applying to current share…"
        } else {
            "Apply to current share"
        };
        accessibility::button(
            centered_button(text(label).font(BOLD_FONT))
                .style(|_, status| app.appearance.primary_button(status)),
            (active_dirty && app.applying_share.is_none()).then_some(Message::ApplyCurrentShare),
            focus_ring,
        )
    } else {
        accessibility::button(
            centered_button(text(if applying { "Applying…" } else { "Apply" }).font(BOLD_FONT))
                .style(|_, status| app.appearance.primary_button(status)),
            can_apply.then_some(Message::ApplySettings),
            focus_ring,
        )
    };
    let footer = row![
        text(footer_status)
            .size(14)
            .color(app.appearance.secondary_text())
            .width(Length::Fill)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        accessibility::button(
            centered_button(text("Revert").font(MEDIUM_FONT))
                .style(|_, status| app.appearance.neutral_button(status)),
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
                container(text("Saved").size(14))
                    .padding([4, 8])
                    .style(|_| app.appearance.metric()),
                container(
                    text(if draft_dirty {
                        "Draft · Changed"
                    } else {
                        "Draft · Saved"
                    })
                    .size(14),
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
                    .size(14),
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

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
