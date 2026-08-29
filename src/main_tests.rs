use super::*;

fn test_viewers(count: usize, online: bool) -> Vec<web::Viewer> {
    (0..count)
        .map(|key| web::Viewer {
            key: key as u64,
            ip: format!("192.0.2.{}", key + 1).parse().unwrap(),
            online_since: online.then(Instant::now),
            duration: Duration::from_secs(65),
            rtt: None,
            playback_lag: None,
            telemetry_at: None,
        })
        .collect()
}

fn test_audio(enabled: bool) -> AudioSettings {
    let settings = settings::Settings {
        system_audio: enabled,
        ..settings::Settings::default()
    };
    audio_settings(&settings)
}

fn gst_error<T: gst::message::MessageErrorDomain>(
    source: &str,
    error: T,
    details: Option<gst::Structure>,
) -> gst::Message {
    let source = gst::ElementFactory::make("identity")
        .name(source)
        .build()
        .unwrap();
    gst::message::Error::builder(error, "test")
        .src(&source)
        .details_if_some(details)
        .build()
}

fn flow_details(flow: gst::FlowReturn) -> Option<gst::Structure> {
    Some(
        gst::Structure::builder("details")
            .field("flow-return", flow)
            .build(),
    )
}

#[test]
#[ignore = "requires an isolated session bus"]
fn a_later_instance_activates_the_primary() {
    let name = format!("org.aercast.Aercast.Test{}", std::process::id());
    let (primary_activation, mut primary_messages) = iced::futures::channel::mpsc::channel(0);
    let _primary = claim_instance(primary_activation, &name).unwrap().unwrap();
    for _ in 0..2 {
        let (activation, _) = iced::futures::channel::mpsc::channel(0);
        assert!(claim_instance(activation, &name).unwrap().is_none());
    }
    assert!(matches!(
        primary_messages.next().now_or_never(),
        Some(Some(Message::Show))
    ));
    assert!(primary_messages.next().now_or_never().is_none());
    drop(primary_messages);
    let (activation, _) = iced::futures::channel::mpsc::channel(0);
    assert!(claim_instance(activation, &name).is_err());
}

#[test]
fn embedded_cursor_is_preferred_with_hidden_fallback() {
    assert_eq!(cursor_mode(true, true), Some(CursorMode::Embedded));
    assert_eq!(cursor_mode(false, true), Some(CursorMode::Hidden));
    assert_eq!(cursor_mode(false, false), None);
}

#[test]
fn approved_source_uses_only_portal_display_metadata() {
    assert_eq!(approved_source(Some(SourceType::Monitor)), "Screen");
    assert_eq!(approved_source(Some(SourceType::Window)), "Window");
    assert_eq!(approved_source(None), "Selected source");
}

#[test]
fn retry_policy_allows_exactly_three_media_recoveries() {
    let media_failure = Err::<(), ()>(());
    for recoveries in 0..MAX_MEDIA_RECOVERIES {
        assert!(should_retry(&media_failure, recoveries));
    }
    assert!(!should_retry(&media_failure, MAX_MEDIA_RECOVERIES));

    for terminal in [
        ShareStop::Apply(test_audio(false)),
        ShareStop::Sleep,
        ShareStop::Wake,
        ShareStop::End,
        ShareStop::Quit,
        ShareStop::PortalClosed,
        ShareStop::Failed(io::Error::other("host failed").into()),
    ] {
        assert!(!should_retry(&Ok::<_, ()>(terminal), 0));
    }
}

#[test]
fn notifications_follow_user_visible_state_boundaries() {
    use Phase::{Ending, NetworkError, Selecting, Sharing, Starting, Waiting};
    use notification::Kind::{Error, Started, Stopped, ViewerJoined, ViewerLeft};

    let error = NetworkError("occupied".to_owned());
    assert_eq!(
        [
            notification_kind(&Selecting, &Sharing, false, 0, 0),
            notification_kind(&Sharing, &Sharing, true, 0, 1),
            notification_kind(&Sharing, &Sharing, true, 1, 2),
            notification_kind(&Sharing, &Sharing, true, 2, 0),
            notification_kind(&Ending, &Waiting, true, 0, 0),
            notification_kind(&Ending, &Waiting, false, 0, 0),
            notification_kind(&Waiting, &error, false, 1, 0),
            notification_kind(&Starting, &error, false, 0, 0),
        ],
        [
            Some(Started),
            Some(ViewerJoined),
            None,
            Some(ViewerLeft),
            Some(Stopped),
            None,
            Some(Error),
            None,
        ]
    );
}

#[test]
fn va_fallback_uses_only_the_structured_hardware_error_whitelist() {
    gst::init().unwrap();
    let cases = [
        gst_error("encoder", gst::StreamError::Encode, None),
        gst_error("encoder", gst::LibraryError::Init, None),
        gst_error("encoder", gst::LibraryError::Failed, None),
        gst_error("encoder", gst::CoreError::Negotiation, None),
        gst_error("video-converter", gst::LibraryError::Init, None),
        gst_error("video-converter", gst::ResourceError::Settings, None),
        gst_error("video-converter", gst::CoreError::Negotiation, None),
        gst_error("video-converter", gst::CoreError::NotImplemented, None),
        gst_error("portal-video", gst::StreamError::Format, None),
        gst_error(
            "portal-video",
            gst::StreamError::Failed,
            flow_details(gst::FlowReturn::NotNegotiated),
        ),
        gst_error("portal-format", gst::StreamError::Format, None),
        gst_error("h264", gst::StreamError::Format, None),
        gst_error("h264", gst::StreamError::Decode, None),
        gst_error("h264", gst::StreamError::WrongType, None),
        gst_error("h264", gst::StreamError::Failed, None),
        gst_error(
            "h264",
            gst::StreamError::Failed,
            flow_details(gst::FlowReturn::NotNegotiated),
        ),
    ];
    for message in cases {
        assert!(
            hardware_video_error(&message),
            "expected hardware fallback for {:?}",
            message.src().map(|source| source.name())
        );
    }

    let malformed_flow = Some(
        gst::Structure::builder("details")
            .field("flow-return", "not-a-flow-return")
            .build(),
    );
    let rejected = [
        gst_error("encoder", gst::LibraryError::Encode, None),
        gst_error("video-converter", gst::ResourceError::Failed, None),
        gst_error("portal-video", gst::ResourceError::NotFound, None),
        gst_error("portal-video", gst::StreamError::Failed, None),
        gst_error(
            "portal-video",
            gst::StreamError::Failed,
            flow_details(gst::FlowReturn::Error),
        ),
        gst_error("portal-format", gst::CoreError::Negotiation, None),
        gst_error(
            "h264",
            gst::StreamError::Failed,
            flow_details(gst::FlowReturn::Error),
        ),
        gst_error("h264", gst::StreamError::Failed, malformed_flow),
        gst_error("mux", gst::StreamError::Encode, None),
        gst_error("system-audio", gst::StreamError::Encode, None),
        gst_error("stream", gst::StreamError::Encode, None),
        gst_error("unknown", gst::StreamError::Encode, None),
    ];
    for message in rejected {
        assert!(
            !hardware_video_error(&message),
            "unexpected hardware fallback for {:?}",
            message.src().map(|source| source.name())
        );
    }

    let hardware = media_outcome([
        gst_error("encoder", gst::StreamError::Encode, None),
        gst_error("h264", gst::StreamError::Format, None),
    ])
    .err()
    .unwrap();
    assert!(hardware.is::<HardwareVideoFailure>());
    let mixed = media_outcome([
        gst_error("encoder", gst::StreamError::Encode, None),
        gst_error("mux", gst::StreamError::Mux, None),
    ])
    .err()
    .unwrap();
    assert!(!mixed.is::<HardwareVideoFailure>());
    let eos = media_outcome([
        gst_error("encoder", gst::StreamError::Encode, None),
        gst::message::Eos::new(),
    ])
    .err()
    .unwrap();
    assert!(!eos.is::<HardwareVideoFailure>());

    let bus = gst::Bus::new();
    let message_types = [gst::MessageType::Error, gst::MessageType::Eos];
    let mut messages = bus.stream_filtered(&message_types);
    bus.post(gst_error(
        "video-converter",
        gst::CoreError::Negotiation,
        None,
    ))
    .unwrap();
    let first = messages.next().now_or_never().flatten();
    let queued = queued_media_outcome(first, &mut messages).err().unwrap();
    assert!(queued.is::<HardwareVideoFailure>());

    bus.post(gst_error("encoder", gst::StreamError::Encode, None))
        .unwrap();
    bus.post(gst_error("mux", gst::StreamError::Mux, None))
        .unwrap();
    let first = messages.next().now_or_never().flatten();
    let queued = queued_media_outcome(first, &mut messages).err().unwrap();
    assert!(!queued.is::<HardwareVideoFailure>());

    let automatic = VideoPlan {
        settings: settings::VideoSettings::default(),
        encoder: Encoder::VaApi,
    };
    assert!(should_fallback(automatic, false, 0, &hardware));
    assert!(!should_fallback(automatic, true, 0, &hardware));
    assert!(!should_fallback(
        automatic,
        false,
        MAX_MEDIA_RECOVERIES,
        &hardware
    ));
    assert!(!should_fallback(
        VideoPlan {
            encoder: Encoder::X264,
            ..automatic
        },
        false,
        0,
        &hardware
    ));
    assert!(!should_fallback(
        VideoPlan {
            settings: settings::VideoSettings {
                encoder: settings::VideoEncoder::VaApi,
                ..automatic.settings
            },
            ..automatic
        },
        false,
        0,
        &hardware
    ));
    assert!(!should_fallback(automatic, false, 0, &mixed));
}

#[test]
fn idle_grace_starts_only_after_media_is_ready_and_does_not_slide() {
    let start = Instant::now();
    assert_eq!(idle_deadline(None, false, 0, start), None);
    let deadline = idle_deadline(None, true, 0, start).unwrap();
    assert_eq!(deadline, start + MEDIA_IDLE_GRACE);
    assert_eq!(
        idle_deadline(Some(deadline), true, 0, start + Duration::from_millis(999)),
        Some(deadline)
    );
    assert_eq!(idle_deadline(Some(deadline), true, 1, start), None);
    let disconnected = start + Duration::from_secs(3);
    assert_eq!(
        idle_deadline(None, true, 0, disconnected),
        Some(disconnected + MEDIA_IDLE_GRACE)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn active_control_sleeps_after_the_fixed_ready_grace() {
    let (_commands, mut receiver) = mpsc::channel(1);
    let mut server = tokio::spawn(std::future::pending::<io::Result<()>>());
    let host = web::Host::new().unwrap();
    let (events, _) = iced::futures::channel::mpsc::unbounded();
    let (ready_sender, ready) = watch::channel(true);
    let started = Instant::now();
    let stop = tokio::time::timeout(
        MEDIA_IDLE_GRACE + Duration::from_secs(1),
        share_control(
            &mut receiver,
            std::future::pending(),
            &mut server,
            &host,
            "http://127.0.0.1:1",
            &events,
            Some(ready),
        ),
    )
    .await
    .unwrap();
    assert!(matches!(stop, ShareStop::Sleep));
    assert!(started.elapsed() >= MEDIA_IDLE_GRACE);
    drop(ready_sender);
    server.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn sleeping_refresh_rotates_the_generation_without_waking() {
    let (commands, mut receiver) = mpsc::channel(2);
    commands.send(Command::Refresh(true)).await.unwrap();
    commands.send(Command::End).await.unwrap();
    let mut server = tokio::spawn(std::future::pending::<io::Result<()>>());
    let host = web::Host::new().unwrap();
    let old_path = host.path().unwrap();
    let (events, _) = iced::futures::channel::mpsc::unbounded();
    assert!(matches!(
        share_control(
            &mut receiver,
            std::future::pending(),
            &mut server,
            &host,
            "http://127.0.0.1:1",
            &events,
            None,
        )
        .await,
        ShareStop::End
    ));
    assert_ne!(host.path().unwrap(), old_path);
    server.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn server_failure_is_terminal_control() {
    let (_commands, mut receiver) = mpsc::channel(1);
    let mut server = tokio::spawn(async { Err(io::Error::other("server failed")) });
    let host = web::Host::new().unwrap();
    let (events, _) = iced::futures::channel::mpsc::unbounded();
    assert!(matches!(
        share_control(
            &mut receiver,
            std::future::pending(),
            &mut server,
            &host,
            "http://127.0.0.1:1",
            &events,
            Some(watch::channel(false).1),
        )
        .await,
        ShareStop::Failed(_)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn apply_requests_a_media_restart_without_reclassifying_control() {
    let (commands, mut receiver) = mpsc::channel(1);
    commands
        .send(Command::Apply(test_audio(false)))
        .await
        .unwrap();
    let mut server = tokio::spawn(std::future::pending::<io::Result<()>>());
    let host = web::Host::new().unwrap();
    let (events, _) = iced::futures::channel::mpsc::unbounded();
    assert!(matches!(
        share_control(
            &mut receiver,
            std::future::pending(),
            &mut server,
            &host,
            "http://127.0.0.1:1",
            &events,
            None,
        )
        .await,
        ShareStop::Apply(audio) if audio == test_audio(false)
    ));
    server.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn quit_waits_for_a_full_control_queue() {
    let (commands, mut receiver) = mpsc::channel(1);
    commands.try_send(Command::End).unwrap();
    let quit = tokio::spawn(queue_quit(commands));
    assert_eq!(receiver.recv().await, Some(Command::End));
    assert!(quit.await.unwrap());
    assert_eq!(receiver.recv().await, Some(Command::Quit));
}

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

#[test]
fn ui_commands_follow_the_host_lifecycle() {
    let (commands, mut receiver) = mpsc::channel(2);
    let (notifications, _notification_requests) = iced::futures::channel::mpsc::unbounded();
    let window = window::Id::unique();
    let settings = settings::Settings::default();
    let video_plan = VideoPlan {
        settings: settings.video,
        encoder: Encoder::X264,
    };
    let mut app = App {
        phase: Phase::Starting,
        link: String::new(),
        viewers: Vec::new(),
        commands: Some(commands),
        window: Some(window),
        confirm_refresh: false,
        confirm_quit: false,
        settings,
        page: Page::Main,
        copied_at: None,
        settings_error: None,
        audio_candidates: Vec::new(),
        audio_scanning: false,
        audio_scan_error: None,
        video_plan: Some(video_plan),
        video_probe: None,
        video_error: None,
        video_edit_error: None,
        video_preset: Quality::P720,
        video_width: "1280".to_owned(),
        video_height: "720".to_owned(),
        video_fps: 60,
        video_bitrate: "6".to_owned(),
        video_encoder: settings::VideoEncoder::Auto,
        appearance: appearance::Appearance::default(),
        approved_source: None,
        active_audio: None,
        applying_audio: None,
        network_address: "127.0.0.1".to_owned(),
        network_port: "8877".to_owned(),
        share_base_url: String::new(),
        applying_network: false,
        notifications,
        tray_updates: None,
        tray_stopped: true,
        host_stopped: false,
        quitting: false,
    };

    app.phase = Phase::Sharing;
    app.confirm_refresh = true;
    app.confirm_quit = true;
    app.commands
        .as_ref()
        .unwrap()
        .try_send(Command::End)
        .unwrap();
    app.commands
        .as_ref()
        .unwrap()
        .try_send(Command::Refresh(false))
        .unwrap();
    drop(update(&mut app, Message::ConfirmRefresh));
    assert_eq!(app.phase, Phase::Sharing);
    assert!(app.confirm_refresh);
    assert!(app.confirm_quit);
    drop(update(&mut app, Message::End));
    assert_eq!(app.phase, Phase::Sharing);
    assert!(app.confirm_refresh);
    assert!(app.confirm_quit);
    assert_eq!(receiver.try_recv().unwrap(), Command::End);
    assert_eq!(receiver.try_recv().unwrap(), Command::Refresh(false));
    app.phase = Phase::Starting;
    app.confirm_refresh = false;
    app.confirm_quit = false;

    app.viewers = test_viewers(1, true);
    assert!(!viewer_tick_enabled(&app));
    app.page = Page::Viewers;
    assert!(viewer_tick_enabled(&app));
    app.page = Page::Main;
    app.window = None;
    assert!(!viewer_tick_enabled(&app));
    app.window = Some(window);
    app.viewers.clear();

    drop(update(
        &mut app,
        Message::Host(HostEvent::NetworkUnavailable(
            "Could not listen on 127.0.0.1:8877".to_owned(),
        )),
    ));
    assert!(matches!(app.phase, Phase::NetworkError(_)));
    assert!(app.settings_error.is_some());
    drop(update(&mut app, Message::ApplyNetwork));
    assert_eq!(
        receiver.try_recv().unwrap(),
        Command::Network(settings::Settings::default())
    );
    drop(update(
        &mut app,
        Message::Host(HostEvent::NetworkUnavailable(
            "Still could not listen on 127.0.0.1:8877".to_owned(),
        )),
    ));
    app.network_port = "9000".to_owned();
    let recovered_network = app
        .settings
        .with_network(&app.network_address, &app.network_port, &app.share_base_url)
        .unwrap();
    drop(update(&mut app, Message::ApplyNetwork));
    assert_eq!(
        receiver.try_recv().unwrap(),
        Command::Network(recovered_network.clone())
    );
    drop(update(
        &mut app,
        Message::Host(HostEvent::NetworkApplied(Ok(recovered_network))),
    ));
    drop(update(
        &mut app,
        Message::Host(HostEvent::Waiting(
            "http://127.0.0.1:9000/s/token".to_owned(),
        )),
    ));
    assert_eq!(app.phase, Phase::Waiting);
    drop(update(&mut app, Message::Refresh));
    assert_eq!(receiver.try_recv().unwrap(), Command::Refresh(false));
    let candidate = audio::PlaybackApplication {
        label: "Player".to_owned(),
        identity: "org.example.Player".to_owned(),
    };
    app.audio_candidates.push(candidate);
    drop(update(&mut app, Message::Page(Page::Settings)));
    assert_eq!(app.page, Page::Settings);
    assert!(app.audio_scanning);
    assert_eq!(app.audio_candidates.len(), 1);
    assert_eq!(app.audio_candidates[0].identity, "org.example.Player");
    drop(update(&mut app, Message::AudioApplications(Ok(Vec::new()))));
    assert!(!app.audio_scanning);
    drop(update(&mut app, Message::Page(Page::Viewers)));
    assert_eq!(app.page, Page::Viewers);

    app.network_address = "0.0.0.0".to_owned();
    drop(update(&mut app, Message::ApplyNetwork));
    assert!(app.settings_error.is_some());
    assert!(receiver.try_recv().is_err());
    assert_eq!(app.link, "http://127.0.0.1:9000/s/token");

    app.network_address = "127.0.0.1".to_owned();
    app.network_port = "9001".to_owned();
    app.share_base_url = "https://share.example:443/".to_owned();
    let network = app
        .settings
        .with_network(&app.network_address, &app.network_port, &app.share_base_url)
        .unwrap();
    drop(update(&mut app, Message::ApplyNetwork));
    assert_eq!(
        receiver.try_recv().unwrap(),
        Command::Network(network.clone())
    );
    assert!(app.applying_network);
    drop(update(&mut app, Message::Start));
    assert!(receiver.try_recv().is_err());
    drop(update(&mut app, Message::SystemAudio(false)));
    assert!(app.settings.system_audio);
    drop(update(
        &mut app,
        Message::Host(HostEvent::NetworkApplied(Err("occupied".to_owned()))),
    ));
    assert!(!app.applying_network);
    assert_eq!(app.settings.listen_port, 9000);
    assert_eq!(app.link, "http://127.0.0.1:9000/s/token");

    drop(update(&mut app, Message::ApplyNetwork));
    assert_eq!(
        receiver.try_recv().unwrap(),
        Command::Network(network.clone())
    );
    drop(update(
        &mut app,
        Message::Host(HostEvent::NetworkApplied(Ok(network))),
    ));
    assert!(!app.applying_network);
    assert_eq!(app.settings.listen_port, 9001);
    assert_eq!(app.share_base_url, "https://share.example:443");

    drop(update(&mut app, Message::VideoPreset(Quality::P1080)));
    assert_eq!(app.video_width, "1920");
    assert_eq!(app.video_height, "1080");
    assert_eq!(app.video_bitrate, "12");
    assert_eq!(app.settings.video, settings::VideoSettings::default());
    drop(update(&mut app, Message::VideoPreset(Quality::Custom)));
    assert_eq!(app.video_preset, Quality::Custom);
    assert!(app.video_bitrate.is_empty());
    drop(update(&mut app, Message::VideoBitrate("9".to_owned())));
    drop(update(&mut app, Message::VideoPreset(Quality::Custom)));
    assert_eq!(app.video_bitrate, "9");
    drop(update(&mut app, Message::VideoFps(30)));
    assert_eq!(app.video_fps, 30);
    drop(update(
        &mut app,
        Message::VideoEncoder(settings::VideoEncoder::X264),
    ));
    drop(update(&mut app, Message::VideoPreset(Quality::P1080)));
    assert_eq!(app.video_encoder, settings::VideoEncoder::X264);

    let current_video = app.settings.video;
    let current_plan = VideoPlan {
        settings: current_video,
        encoder: Encoder::X264,
    };
    app.video_plan = None;
    app.video_probe = Some(VideoProbe::Current(current_video));
    let stale_video = settings::VideoSettings {
        fps: 30,
        ..current_video
    };
    drop(update(
        &mut app,
        Message::VideoProbed(
            VideoProbe::Current(stale_video),
            Ok(VideoPlan {
                settings: stale_video,
                encoder: Encoder::X264,
            }),
        ),
    ));
    assert_eq!(app.video_probe, Some(VideoProbe::Current(current_video)));
    assert_eq!(app.video_plan, None);
    drop(update(
        &mut app,
        Message::VideoProbed(VideoProbe::Current(current_video), Ok(current_plan)),
    ));
    assert_eq!(app.video_probe, None);
    assert_eq!(app.video_plan, Some(current_plan));

    let saved_settings = app.settings.clone();
    let saved_plan = app.video_plan;
    let candidate = app
        .settings
        .with_video(
            &app.video_width,
            &app.video_height,
            app.video_fps,
            &app.video_bitrate,
            app.video_encoder,
        )
        .unwrap()
        .video;
    assert_eq!(update(&mut app, Message::SaveVideo).units(), 1);
    assert_eq!(app.video_probe, Some(VideoProbe::Save(candidate)));
    assert_eq!(app.settings, saved_settings);
    assert_eq!(app.video_plan, saved_plan);
    drop(update(&mut app, Message::Start));
    assert!(receiver.try_recv().is_err());
    assert_eq!(update(&mut app, Message::SaveVideo).units(), 0);
    let network_port = app.network_port.clone();
    app.network_port = "9002".to_owned();
    drop(update(&mut app, Message::ApplyNetwork));
    assert!(!app.applying_network);
    assert!(receiver.try_recv().is_err());
    app.network_port = network_port;
    drop(update(
        &mut app,
        Message::VideoProbed(
            VideoProbe::Save(stale_video),
            Ok(VideoPlan {
                settings: stale_video,
                encoder: Encoder::X264,
            }),
        ),
    ));
    assert_eq!(app.video_probe, Some(VideoProbe::Save(candidate)));
    drop(update(
        &mut app,
        Message::VideoProbed(VideoProbe::Save(candidate), Err("probe failed".to_owned())),
    ));
    assert_eq!(app.video_probe, None);
    assert_eq!(app.settings, saved_settings);
    assert_eq!(app.video_plan, saved_plan);
    assert!(
        app.video_edit_error
            .as_deref()
            .is_some_and(|error| error.contains("probe failed"))
    );
    set_video_draft(&mut app, settings::VideoSettings::default());

    app.settings.system_audio = false;
    app.settings.video.width = 0;
    drop(update(&mut app, Message::Start));
    assert!(receiver.try_recv().is_err());
    assert_eq!(app.phase, Phase::Waiting);
    assert!(app.video_error.is_some());
    app.settings.video = settings::VideoSettings::default();
    app.video_error = None;
    let share = ShareSettings {
        audio: test_audio(false),
        video: saved_plan.unwrap(),
    };
    drop(update(&mut app, Message::Start));
    assert_eq!(receiver.try_recv().unwrap(), Command::Start(share));
    assert_eq!(app.phase, Phase::Selecting);
    drop(update(&mut app, Message::Host(HostEvent::Source("Window"))));
    assert_eq!(app.approved_source, Some("Window"));
    app.settings.system_audio = true;
    drop(update(&mut app, Message::ApplySystemAudio));
    assert!(receiver.try_recv().is_err());

    drop(update(
        &mut app,
        Message::Host(HostEvent::Sharing(test_audio(false))),
    ));
    assert_eq!(app.active_audio, Some(test_audio(false)));
    assert_eq!(app.approved_source, Some("Window"));
    app.settings.audio_bitrate_kbps = 160;
    app.settings.audio_exclusions[0].enabled = false;
    let changed_audio = audio_settings(&app.settings);
    assert_eq!(changed_audio.bitrate_kbps, 160);
    drop(update(&mut app, Message::ApplySystemAudio));
    assert_eq!(
        receiver.try_recv().unwrap(),
        Command::Apply(changed_audio.clone())
    );
    assert_eq!(app.applying_audio, Some(changed_audio.clone()));
    drop(update(&mut app, Message::ApplySystemAudio));
    assert!(receiver.try_recv().is_err());
    drop(update(
        &mut app,
        Message::Host(HostEvent::Sharing(test_audio(false))),
    ));
    assert_eq!(app.applying_audio, Some(changed_audio.clone()));
    drop(update(
        &mut app,
        Message::Host(HostEvent::Sharing(changed_audio.clone())),
    ));
    assert_eq!(app.active_audio, Some(changed_audio.clone()));
    assert_eq!(app.applying_audio, None);

    assert_eq!(update(&mut app, Message::Close(window)).units(), 1);
    assert_eq!(app.window, None);
    assert_eq!(app.phase, Phase::Sharing);
    assert!(receiver.try_recv().is_err());
    assert_ne!(update(&mut app, Message::Show).units(), 0);
    let reopened = app.window.unwrap();
    assert_ne!(reopened, window);
    drop(update(&mut app, Message::Show));
    assert_eq!(app.window, Some(reopened));
    drop(update(&mut app, Message::Closed(window)));
    assert_eq!(app.window, Some(reopened));
    drop(update(&mut app, Message::Closed(reopened)));
    assert_eq!(app.window, None);

    let (tray_updates, tray_state) = watch::channel(TrayState {
        phase: Phase::Sharing,
        online_viewers: 0,
    });
    app.tray_updates = Some(tray_updates);
    drop(update(
        &mut app,
        Message::Host(HostEvent::Viewers(test_viewers(2, true))),
    ));
    assert_eq!(tray_state.borrow().online_viewers, 2);
    let viewer_key = app.viewers[0].key;
    drop(update(&mut app, Message::Disconnect(viewer_key)));
    assert_eq!(
        receiver.try_recv().unwrap(),
        Command::Disconnect(viewer_key)
    );
    drop(update(&mut app, Message::Refresh));
    assert_eq!(receiver.try_recv().unwrap(), Command::Refresh(false));
    assert!(!app.confirm_refresh);
    drop(update(&mut app, Message::Host(HostEvent::ConfirmRefresh)));
    assert!(app.confirm_refresh);
    assert_eq!(app.page, Page::Main);
    drop(update(&mut app, Message::CancelRefresh));
    assert!(!app.confirm_refresh);
    drop(update(&mut app, Message::Host(HostEvent::ConfirmRefresh)));
    drop(update(&mut app, Message::ConfirmRefresh));
    assert_eq!(receiver.try_recv().unwrap(), Command::Refresh(true));
    assert!(!app.confirm_refresh);
    drop(update(
        &mut app,
        Message::Host(HostEvent::Link("http://127.0.0.1/s/replacement".to_owned())),
    ));
    assert_eq!(app.link, "http://127.0.0.1/s/replacement");
    assert_eq!(app.approved_source, Some("Window"));
    assert!(app.viewers.is_empty());
    assert_eq!(tray_state.borrow().online_viewers, 0);
    drop(update(&mut app, Message::Refresh));
    assert_eq!(receiver.try_recv().unwrap(), Command::Refresh(false));
    assert!(!app.confirm_refresh);
    drop(update(&mut app, Message::End));
    assert_eq!(receiver.try_recv().unwrap(), Command::End);
    assert_eq!(app.phase, Phase::Ending);
    drop(update(
        &mut app,
        Message::Host(HostEvent::Source("Late source")),
    ));
    assert_eq!(app.approved_source, Some("Window"));
    app.settings.system_audio = false;
    drop(update(&mut app, Message::ApplySystemAudio));
    assert!(receiver.try_recv().is_err());
    drop(update(
        &mut app,
        Message::Host(HostEvent::Sharing(test_audio(false))),
    ));
    assert_eq!(app.phase, Phase::Ending);
    assert_eq!(app.active_audio, Some(changed_audio));
    let offline = test_viewers(1, false);
    drop(update(
        &mut app,
        Message::Host(HostEvent::Viewers(offline.clone())),
    ));
    drop(update(
        &mut app,
        Message::Host(HostEvent::Waiting("http://127.0.0.1/s/token".to_owned())),
    ));
    assert_eq!(app.phase, Phase::Waiting);
    assert_eq!(app.link, "http://127.0.0.1/s/token");
    drop(update(&mut app, Message::Copy));
    let first_copy = app.copied_at.unwrap();
    let latest_copy = first_copy + Duration::from_millis(1);
    app.copied_at = Some(latest_copy);
    drop(update(&mut app, Message::CopyFeedbackExpired(first_copy)));
    assert_eq!(app.copied_at, Some(latest_copy));
    drop(update(&mut app, Message::CopyFeedbackExpired(latest_copy)));
    assert_eq!(app.copied_at, None);
    assert!(app.approved_source.is_none());
    assert_eq!(app.active_audio, None);
    assert_eq!(app.viewers, offline);
    assert_eq!(format_duration(app.viewers[0].duration()), "1:05");

    drop(receiver);
    app.confirm_refresh = true;
    app.confirm_quit = true;
    assert!(!send_command(&mut app, Command::Refresh(false)));
    assert_eq!(
        app.phase,
        Phase::Error("Host control is unavailable".to_owned())
    );
    assert!(!app.confirm_refresh);
    assert!(!app.confirm_quit);

    assert_eq!(
        update(&mut app, Message::Host(HostEvent::Stopped(Ok(())))).units(),
        1
    );
    app.quitting = false;
    app.host_stopped = false;

    let (commands, _) = mpsc::channel(1);
    app.commands = Some(commands);
    app.phase = Phase::Waiting;
    app.viewers = test_viewers(2, true);
    assert_eq!(
        update(&mut app, Message::Host(HostEvent::Stopped(Ok(())))).units(),
        1
    );
    assert!(app.viewers.is_empty());
    app.quitting = false;
    app.host_stopped = false;

    let (commands, _) = mpsc::channel(1);
    app.commands = Some(commands);
    app.phase = Phase::Sharing;
    app.approved_source = Some("Screen");
    app.viewers = test_viewers(3, true);
    app.page = Page::Settings;
    assert_eq!(
        update(
            &mut app,
            Message::Host(HostEvent::Stopped(Err("stopped".to_owned())))
        )
        .units(),
        0
    );
    assert_eq!(app.phase, Phase::Error("stopped".to_owned()));
    assert!(app.approved_source.is_none());
    assert!(app.viewers.is_empty());
    assert_ne!(update(&mut app, Message::Show).units(), 0);
    let reopened = app.window.unwrap();
    assert_eq!(app.phase, Phase::Error("stopped".to_owned()));
    assert_eq!(update(&mut app, Message::Close(reopened)).units(), 1);
    assert_eq!(app.window, None);

    let (commands, _) = mpsc::channel(1);
    app.commands = Some(commands);
    app.host_stopped = false;
    app.phase = Phase::Sharing;
    let closing = window::Id::unique();
    app.window = Some(closing);
    assert_ne!(update(&mut app, Message::Quit).units(), 0);
    assert!(app.confirm_quit);
    assert!(!app.quitting);
    assert!(app.commands.is_some());
    assert_eq!(app.page, Page::Main);
    drop(update(&mut app, Message::CancelQuit));
    assert!(!app.confirm_quit);
    drop(update(&mut app, Message::Quit));
    assert!(app.confirm_quit);
    let (tray_updates, _) = watch::channel(TrayState {
        phase: Phase::Sharing,
        online_viewers: 0,
    });
    app.tray_updates = Some(tray_updates);
    app.tray_stopped = false;
    assert_eq!(update(&mut app, Message::ConfirmQuit).units(), 1);
    assert!(app.commands.is_none());
    assert!(app.tray_updates.is_none());
    assert_eq!(app.phase, Phase::Ending);
    assert!(app.quitting);
    drop(update(
        &mut app,
        Message::Host(HostEvent::Waiting("http://127.0.0.1/s/stale".to_owned())),
    ));
    drop(update(&mut app, Message::Show));
    assert_eq!(app.phase, Phase::Ending);
    assert_eq!(app.window, Some(closing));
    assert_eq!(update(&mut app, Message::Close(closing)).units(), 1);
    assert!(app.window.is_none());
    assert_eq!(
        update(
            &mut app,
            Message::Host(HostEvent::Stopped(Err("cleanup failed".to_owned())))
        )
        .units(),
        0
    );
    assert_eq!(update(&mut app, Message::TrayStopped(Ok(()))).units(), 1);

    let (commands, _) = mpsc::channel(1);
    app.commands = Some(commands);
    app.host_stopped = false;
    app.tray_stopped = true;
    app.quitting = false;
    app.phase = Phase::Sharing;
    assert_eq!(update(&mut app, Message::BusClosed).units(), 1);
    assert!(!app.confirm_quit);
    assert!(app.quitting);
}

#[tokio::test(flavor = "current_thread")]
async fn occupied_startup_bind_recovers_without_rotating_the_token() {
    let host = web::Host::new().unwrap();
    let path = host.path().unwrap();
    let (occupied, occupied_address) = bind_listener("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let occupied_settings = settings::Settings::default()
        .with_network("127.0.0.1", &occupied_address.port().to_string(), "")
        .unwrap();
    assert!(prepare_listener(&occupied_settings, None).await.is_err());
    drop(occupied);

    let reservation = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let old_address = reservation.local_addr().unwrap();
    drop(reservation);
    let network = settings::Settings::default()
        .with_network(
            "127.0.0.1",
            &old_address.port().to_string(),
            "https://share.example:443/",
        )
        .unwrap();
    let (old_listener, old_address) = prepare_listener(&network, None).await.unwrap().unwrap();
    let old_server = start_server(old_listener, old_address, &host);
    drop(tokio::net::TcpStream::connect(old_address).await.unwrap());
    assert_eq!(host.path().unwrap(), path);

    let reservation = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let new_address = reservation.local_addr().unwrap();
    drop(reservation);
    let network = settings::Settings::default()
        .with_network(
            "127.0.0.1",
            &new_address.port().to_string(),
            "https://share.example:443/",
        )
        .unwrap();
    let (new_listener, rebound_address) = prepare_listener(&network, Some(old_address))
        .await
        .unwrap()
        .unwrap();
    let new_server = start_server(new_listener, rebound_address, &host);
    stop_server(old_server.task, old_server.shutdown)
        .await
        .unwrap();

    assert_eq!(host.path().unwrap(), path);
    drop(
        tokio::net::TcpStream::connect(rebound_address)
            .await
            .unwrap(),
    );
    assert_eq!(
        format!(
            "{}{path}",
            link_base(network.share_base_url.as_deref(), rebound_address)
        ),
        format!("https://share.example:443{path}")
    );
    assert!(tokio::net::TcpStream::connect(old_address).await.is_err());

    stop_server(new_server.task, new_server.shutdown)
        .await
        .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn waiting_refresh_rotates_only_the_token() {
    let reservation = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let settings = settings::Settings::default()
        .with_network("127.0.0.1", &address.port().to_string(), "")
        .unwrap();
    let (events, mut incoming) = iced::futures::channel::mpsc::unbounded();
    let (commands, receiver) = mpsc::channel(2);
    let host = tokio::spawn(run_host(settings, events, receiver));
    let first = match incoming.next().await.unwrap() {
        HostEvent::Waiting(link) => link,
        event => panic!("expected waiting event, got {event:?}"),
    };
    commands.send(Command::Refresh(false)).await.unwrap();
    let second = match incoming.next().await.unwrap() {
        HostEvent::Waiting(link) => link,
        event => panic!("expected refreshed waiting event, got {event:?}"),
    };
    assert_ne!(first, second);
    commands.send(Command::Quit).await.unwrap();
    host.await.unwrap().unwrap();
}

#[test]
fn avc_config_produces_the_codec_parameter() {
    gst::init().unwrap();
    assert_eq!(
        avc_codec(&[1, 0x42, 0xc0, 0x1f]),
        Some("avc1.42c01f".to_owned())
    );
    assert_eq!(avc_codec(&[1, 0x42, 0xc0]), None);
    assert_eq!(avc_codec(&[0, 0x42, 0xc0, 0x1f]), None);

    let caps = gst::Caps::builder("video/x-h264")
        .field("codec_data", gst::Buffer::from_slice([1, 0x42, 0xc0, 0x1f]))
        .build();
    assert_eq!(
        h264_mime(&caps),
        Some("video/mp4; codecs=\"avc1.42c01f, mp4a.40.2\"".to_owned())
    );
}

#[test]
fn av_pipeline_description_has_no_syntax_error() {
    gst::init().unwrap();
    let video = settings::VideoSettings::default();
    let description = pipeline_description(
        1,
        0,
        VideoPlan {
            settings: video,
            encoder: Encoder::X264,
        },
        96,
    );
    assert!(description.contains("width=1280,height=720"));
    assert!(description.contains("framerate=60/1"));
    assert!(description.contains("bitrate=6000 vbv-buf-capacity=100 nal-hrd=cbr key-int-max=60"));
    assert!(description.contains("avenc_aac bitrate=96000"));
    assert!(description.contains("videoconvertscale name=video-converter add-borders=true"));
    assert!(!description.contains("vapostproc"));
    let encoder_default = pipeline_description(
        1,
        0,
        VideoPlan {
            settings: settings::VideoSettings {
                fps: 30,
                bitrate_mbps: None,
                ..video
            },
            encoder: Encoder::X264,
        },
        128,
    );
    assert!(
        encoder_default.contains(
            "x264enc name=encoder tune=zerolatency speed-preset=ultrafast key-int-max=30"
        )
    );
    let va_api = pipeline_description(
        1,
        0,
        VideoPlan {
            settings: video,
            encoder: Encoder::VaApi,
        },
        160,
    );
    assert!(va_api.contains("video/x-raw(memory:VAMemory),format=NV12"));
    assert!(va_api.contains("vapostproc name=video-converter add-borders=true"));
    assert!(!va_api.contains("disable-passthrough=true"));
    assert!(va_api.contains(
        "vah264enc name=encoder rate-control=cbr target-usage=7 bitrate=6000 cpb-size=600"
    ));
    assert!(va_api.contains("avenc_aac bitrate=160000"));
    assert!(va_api.contains("profile=constrained-baseline,stream-format=byte-stream"));
    for description in [description, va_api] {
        if let Err(error) = gst::parse::launch(&description) {
            assert_ne!(
                error.kind::<gst::ParseError>(),
                Some(gst::ParseError::Syntax)
            );
        }
    }
}

#[test]
#[ignore = "requires the supported host video stack"]
fn host_video_encoders_are_available() {
    gst::init().unwrap();
    let video = settings::VideoSettings::default();
    let automatic = video_plan(&video).unwrap();
    assert_eq!(automatic.encoder, Encoder::VaApi);
    let beyond_va = settings::VideoSettings {
        width: 5_000,
        height: 3_000,
        encoder: settings::VideoEncoder::VaApi,
        ..video
    };
    assert!(video_plan(&beyond_va).is_err());
    assert_eq!(
        video_plan(&settings::VideoSettings {
            encoder: settings::VideoEncoder::Auto,
            ..beyond_va
        })
        .unwrap()
        .encoder,
        Encoder::X264
    );
    let software = video_plan(&settings::VideoSettings {
        encoder: settings::VideoEncoder::X264,
        ..video
    })
    .unwrap();
    assert_eq!(software.encoder, Encoder::X264);
    let pipeline = build_pipeline(&pipeline_description(
        1,
        0,
        automatic,
        settings::DEFAULT_AUDIO_BITRATE_KBPS,
    ))
    .unwrap();
    let encoder = pipeline.by_name("encoder").unwrap();
    assert_eq!(encoder.property::<u32>("bitrate"), 6_000);
    assert_eq!(encoder.property::<u32>("key-int-max"), 60);
    assert_eq!(encoder.property::<u32>("target-usage"), 7);
    let pipeline = build_pipeline(&pipeline_description(
        1,
        0,
        software,
        settings::DEFAULT_AUDIO_BITRATE_KBPS,
    ))
    .unwrap();
    let encoder = pipeline.by_name("encoder").unwrap();
    assert_eq!(encoder.property::<u32>("bitrate"), 6_000);
    assert_eq!(encoder.property::<u32>("key-int-max"), 60);
}

#[test]
fn viewers_view_disambiguates_identical_ips() {
    let (notifications, _notification_requests) = iced::futures::channel::mpsc::unbounded();
    let app = App {
        phase: Phase::Starting,
        link: String::new(),
        viewers: vec![
            web::Viewer {
                key: 1,
                ip: "192.0.2.1".parse().unwrap(),
                online_since: Some(Instant::now()),
                duration: Duration::from_secs(10),
                rtt: None,
                playback_lag: None,
                telemetry_at: None,
            },
            web::Viewer {
                key: 2,
                ip: "192.0.2.1".parse().unwrap(),
                online_since: None,
                duration: Duration::from_secs(5),
                rtt: None,
                playback_lag: None,
                telemetry_at: None,
            },
            web::Viewer {
                key: 3,
                ip: "192.0.2.2".parse().unwrap(),
                online_since: Some(Instant::now()),
                duration: Duration::from_secs(20),
                rtt: None,
                playback_lag: None,
                telemetry_at: None,
            },
        ],
        commands: None,
        window: None,
        confirm_refresh: false,
        confirm_quit: false,
        settings: settings::Settings::default(),
        page: Page::Viewers,
        copied_at: None,
        settings_error: None,
        audio_candidates: Vec::new(),
        audio_scanning: false,
        audio_scan_error: None,
        video_plan: None,
        video_probe: None,
        video_error: None,
        video_edit_error: None,
        video_preset: Quality::P720,
        video_width: "1280".to_owned(),
        video_height: "720".to_owned(),
        video_fps: 60,
        video_bitrate: "6".to_owned(),
        video_encoder: settings::VideoEncoder::Auto,
        appearance: appearance::Appearance::default(),
        approved_source: None,
        active_audio: None,
        applying_audio: None,
        network_address: "127.0.0.1".to_owned(),
        network_port: "8877".to_owned(),
        share_base_url: String::new(),
        applying_network: false,
        notifications,
        tray_updates: None,
        tray_stopped: true,
        host_stopped: false,
        quitting: false,
    };
    let _ = viewers_view(&app);
}
