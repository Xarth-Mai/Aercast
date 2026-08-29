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

fn test_share(enabled: bool) -> ShareSettings {
    let settings = settings::Settings {
        system_audio: enabled,
        ..settings::Settings::default()
    };
    ShareSettings {
        audio: audio_settings(&settings),
        video: VideoPlan {
            settings: settings.video,
            encoder: Encoder::X264,
        },
    }
}

fn test_app() -> (App, mpsc::Receiver<Command>) {
    let settings = settings::Settings::default();
    let draft = SettingsDraft::from_settings(&settings);
    let video_plan = VideoPlan {
        settings: settings.video,
        encoder: Encoder::X264,
    };
    let (commands, receiver) = mpsc::channel(16);
    let (notifications, _) = iced::futures::channel::mpsc::unbounded();
    (
        App {
            phase: Phase::Waiting,
            link: "http://127.0.0.1:8877/s/token".to_owned(),
            viewers: Vec::new(),
            commands: Some(commands),
            window: Some(window::Id::unique()),
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
            video_plan: Some(video_plan),
            video_probe: None,
            video_error: None,
            video_apply_error: None,
            pending_settings: None,
            appearance: appearance::Appearance::default(),
            approved_source: None,
            active_share: None,
            applying_share: None,
            apply_share_error: None,
            notifications,
            tray_updates: None,
            tray_stopped: true,
            host_stopped: false,
            quitting: false,
        },
        receiver,
    )
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
        ShareStop::Apply(test_share(false)),
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
        .send(Command::Apply(test_share(false)))
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
        ShareStop::Apply(share) if share == test_share(false)
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
fn minimum_window_width_uses_the_logical_sixteen_by_nine_projection() {
    assert_eq!(
        minimum_window_size(iced::Size::new(3_840.0, 2_160.0)),
        iced::Size::new(960.0, 480.0)
    );
    assert_eq!(
        minimum_window_size(iced::Size::new(5_120.0, 2_160.0)),
        iced::Size::new(960.0, 480.0)
    );
    assert_eq!(
        minimum_window_size(iced::Size::new(1_920.0, 1_080.0)),
        iced::Size::new(640.0, 480.0)
    );
    assert_eq!(
        minimum_window_size(iced::Size::new(1_280.0, 720.0)),
        iced::Size::new(640.0, 480.0)
    );
}

#[test]
fn settings_draft_builds_one_candidate_and_reverts_as_a_whole() {
    let saved = settings::Settings::default();
    let mut draft = SettingsDraft::from_settings(&saved);
    assert!(!draft.dirty(&saved));
    assert_eq!(draft.candidate().unwrap(), saved);

    draft.settings.system_audio = false;
    draft.settings.notifications = false;
    draft.video_width = "1920".to_owned();
    draft.video_height = "1080".to_owned();
    draft.video_bitrate = "12".to_owned();
    draft.video_encoder = settings::VideoEncoder::X264;
    draft.network_port = "9000".to_owned();
    draft.share_base_url = "https://share.example:443/".to_owned();
    draft.changed();

    let candidate = draft.candidate().unwrap();
    assert!(draft.dirty(&saved));
    assert!(!candidate.system_audio);
    assert!(!candidate.notifications);
    assert_eq!(candidate.video.width, 1920);
    assert_eq!(candidate.listen_port, 9000);
    assert_eq!(
        candidate.share_base_url.as_deref(),
        Some("https://share.example:443")
    );

    draft.video_width = "1279".to_owned();
    assert!(draft.candidate().is_err());

    let (mut app, _) = test_app();
    drop(update_app(&mut app, Message::Notifications(false)));
    drop(update_app(&mut app, Message::VideoPreset(Quality::P1080)));
    let changed = app.draft.clone();
    drop(update_app(&mut app, Message::Page(Page::Viewers)));
    let window = app.window.unwrap();
    drop(update_app(&mut app, Message::Close(window)));
    assert_eq!(app.draft, changed);
    drop(update_app(&mut app, Message::RevertSettings));
    assert!(!app.draft.dirty(&app.settings));
    assert_eq!(app.draft.candidate().unwrap(), app.settings);
}

#[test]
fn viewer_tick_runs_only_for_visible_overview_or_viewers_with_online_viewers() {
    let (mut app, _) = test_app();
    app.viewers = test_viewers(1, true);
    assert!(viewer_tick_enabled(&app));
    app.page = Page::Viewers;
    assert!(viewer_tick_enabled(&app));
    app.page = Page::Settings;
    assert!(!viewer_tick_enabled(&app));
    app.page = Page::Overview;
    app.window = None;
    assert!(!viewer_tick_enabled(&app));
    app.window = Some(window::Id::unique());
    app.viewers = test_viewers(1, false);
    assert!(!viewer_tick_enabled(&app));
}

#[test]
fn block_requires_the_same_button_twice_and_resets_transient_confirmation() {
    let (mut app, mut commands) = test_app();
    app.page = Page::Viewers;
    app.viewers = test_viewers(2, true);

    drop(update_app(&mut app, Message::Block(0)));
    assert_eq!(
        app.confirm_block.map(|confirmation| confirmation.key),
        Some(0)
    );
    assert!(commands.try_recv().is_err());

    drop(update_app(&mut app, Message::Block(1)));
    assert_eq!(
        app.confirm_block.map(|confirmation| confirmation.key),
        Some(1)
    );
    assert!(commands.try_recv().is_err());
    drop(update_app(&mut app, Message::Block(1)));
    assert_eq!(commands.try_recv().unwrap(), Command::Disconnect(1));
    assert!(app.confirm_block.is_none());

    drop(update_app(&mut app, Message::Block(0)));
    app.confirm_block.as_mut().unwrap().started =
        Instant::now() - BLOCK_CONFIRMATION_DURATION - Duration::from_millis(1);
    drop(update_app(&mut app, Message::Tick));
    assert!(app.confirm_block.is_none());

    drop(update_app(&mut app, Message::Block(0)));
    drop(update_app(&mut app, Message::Page(Page::Overview)));
    assert!(app.confirm_block.is_none());

    app.page = Page::Viewers;
    drop(update_app(&mut app, Message::Block(0)));
    drop(update_app(
        &mut app,
        Message::Host(HostEvent::Viewers(test_viewers(2, true))),
    ));
    assert!(app.confirm_block.is_none());

    drop(update_app(&mut app, Message::Block(0)));
    app.viewers = test_viewers(1, false);
    drop(update_app(&mut app, Message::Block(0)));
    assert!(app.confirm_block.is_none());
    assert!(commands.try_recv().is_err());
}

#[test]
fn start_reads_saved_settings_and_ignores_the_dirty_draft() {
    let (mut app, mut commands) = test_app();
    drop(update_app(&mut app, Message::SystemAudio(false)));
    drop(update_app(&mut app, Message::VideoPreset(Quality::P1080)));
    assert!(app.draft.dirty(&app.settings));

    drop(update_app(&mut app, Message::Start));
    assert_eq!(
        commands.try_recv().unwrap(),
        Command::Start(test_share(true))
    );
    assert_eq!(app.phase, Phase::Selecting);
    assert!(app.draft.dirty(&app.settings));
}

#[test]
fn network_changes_block_sharing_and_commit_the_full_draft_transactionally() {
    let (mut app, mut commands) = test_app();
    let saved = app.settings.clone();
    drop(update_app(&mut app, Message::SystemAudio(false)));
    drop(update_app(&mut app, Message::Notifications(false)));
    drop(update_app(
        &mut app,
        Message::NetworkPort("9000".to_owned()),
    ));
    let candidate = app.draft.candidate().unwrap();

    app.phase = Phase::Sharing;
    drop(update_app(&mut app, Message::ApplySettings));
    assert!(
        app.settings_error
            .as_deref()
            .is_some_and(|error| error.contains("Stop sharing"))
    );
    assert!(app.pending_settings.is_none());
    assert!(commands.try_recv().is_err());
    assert_eq!(app.settings, saved);

    app.phase = Phase::Waiting;
    drop(update_app(&mut app, Message::ApplySettings));
    assert_eq!(
        commands.try_recv().unwrap(),
        Command::Network(candidate.clone())
    );
    assert!(app.pending_settings.is_some());
    drop(update_app(
        &mut app,
        Message::Host(HostEvent::NetworkApplied(Err("occupied".to_owned()))),
    ));
    assert!(app.pending_settings.is_none());
    assert_eq!(app.settings, saved);
    assert!(app.draft.dirty(&app.settings));

    drop(update_app(&mut app, Message::ApplySettings));
    assert_eq!(
        commands.try_recv().unwrap(),
        Command::Network(candidate.clone())
    );
    drop(update_app(
        &mut app,
        Message::Host(HostEvent::NetworkApplied(Ok(candidate.clone()))),
    ));
    assert_eq!(app.settings, candidate);
    assert!(!app.settings.system_audio);
    assert!(!app.settings.notifications);
    assert!(!app.draft.dirty(&app.settings));
    assert!(app.pending_settings.is_none());
}

#[test]
fn stale_apply_probe_cannot_commit_a_newer_draft_revision() {
    let (mut app, _) = test_app();
    let saved = app.settings.clone();
    drop(update_app(&mut app, Message::VideoPreset(Quality::P1080)));
    let candidate = app.draft.candidate().unwrap();
    let probe = VideoProbe::Apply {
        revision: app.draft.revision,
        candidate: candidate.clone(),
    };
    app.video_probe = Some(probe.clone());

    drop(update_app(&mut app, Message::VideoBitrate("13".to_owned())));
    drop(update_app(
        &mut app,
        Message::VideoProbed(
            probe,
            Ok(VideoPlan {
                settings: candidate.video,
                encoder: Encoder::X264,
            }),
        ),
    ));

    assert!(app.video_probe.is_none());
    assert_eq!(app.settings, saved);
    assert_eq!(app.draft.video_bitrate, "13");
    assert!(
        app.settings_error
            .as_deref()
            .is_some_and(|error| error.contains("changed during"))
    );

    drop(update_app(&mut app, Message::RevertSettings));
    drop(update_app(&mut app, Message::VideoPreset(Quality::P1080)));
    let candidate = app.draft.candidate().unwrap();
    let reverted_probe = VideoProbe::Apply {
        revision: app.draft.revision,
        candidate: candidate.clone(),
    };
    app.video_probe = Some(reverted_probe.clone());
    drop(update_app(&mut app, Message::RevertSettings));
    assert!(app.video_probe.is_none());
    drop(update_app(&mut app, Message::VideoPreset(Quality::P1080)));
    drop(update_app(
        &mut app,
        Message::VideoProbed(reverted_probe, Err("old failure".to_owned())),
    ));
    assert_eq!(app.settings, saved);
    assert_eq!(app.draft.candidate().unwrap(), candidate);
    assert!(app.settings_error.is_none());
}

#[test]
fn current_share_apply_confirms_online_viewers_and_tracks_the_full_snapshot() {
    let (mut app, mut commands) = test_app();
    let old = test_share(true);
    let saved = settings::Settings {
        system_audio: false,
        audio_bitrate_kbps: 160,
        video: settings::VideoSettings {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: Some(12),
            encoder: settings::VideoEncoder::X264,
        },
        ..settings::Settings::default()
    };
    let expected = ShareSettings {
        audio: audio_settings(&saved),
        video: VideoPlan {
            settings: saved.video,
            encoder: Encoder::X264,
        },
    };
    app.settings = saved;
    app.draft = SettingsDraft::from_settings(&app.settings);
    app.video_plan = Some(expected.video);
    app.phase = Phase::Sharing;
    app.active_share = Some(old.clone());
    app.viewers = test_viewers(1, true);

    drop(update_app(&mut app, Message::ApplyCurrentShare));
    assert!(app.confirm_apply_current);
    assert!(app.applying_share.is_none());
    assert!(commands.try_recv().is_err());

    drop(update_app(&mut app, Message::ApplyCurrentShare));
    assert_eq!(
        commands.try_recv().unwrap(),
        Command::Apply(expected.clone())
    );
    assert_eq!(app.applying_share, Some(expected.clone()));
    assert!(!app.confirm_apply_current);

    drop(update_app(
        &mut app,
        Message::Host(HostEvent::ApplyFailed("encoder failed".to_owned())),
    ));
    assert_eq!(app.active_share, Some(old));
    assert!(app.applying_share.is_none());
    assert_eq!(app.apply_share_error.as_deref(), Some("encoder failed"));

    app.viewers.clear();
    drop(update_app(&mut app, Message::ApplyCurrentShare));
    assert_eq!(
        commands.try_recv().unwrap(),
        Command::Apply(expected.clone())
    );
    drop(update_app(
        &mut app,
        Message::Host(HostEvent::Sharing(expected.clone())),
    ));
    assert_eq!(app.active_share, Some(expected));
    assert!(app.applying_share.is_none());
    assert!(app.apply_share_error.is_none());
}

#[test]
fn auto_fallback_encoder_is_not_dirty_and_survives_audio_apply() {
    let (mut app, mut commands) = test_app();
    app.video_plan.as_mut().unwrap().encoder = Encoder::VaApi;
    let saved = saved_share(&app).unwrap();
    let mut active = saved.clone();
    active.video.encoder = Encoder::X264;
    assert_ne!(active, saved);
    assert!(same_saved_media(&active, &saved));

    app.phase = Phase::Sharing;
    app.active_share = Some(active.clone());
    drop(update_app(&mut app, Message::ApplyCurrentShare));
    assert!(!app.confirm_apply_current);
    assert!(app.applying_share.is_none());
    assert!(commands.try_recv().is_err());

    app.applying_share = Some(saved);
    drop(update_app(
        &mut app,
        Message::Host(HostEvent::Sharing(active.clone())),
    ));
    assert_eq!(app.active_share, Some(active.clone()));
    assert!(app.applying_share.is_none());

    app.settings.system_audio = !app.settings.system_audio;
    drop(update_app(&mut app, Message::ApplyCurrentShare));
    let Command::Apply(target) = commands.try_recv().unwrap() else {
        panic!("audio apply did not restart media");
    };
    assert_eq!(target.video, active.video);
    assert_eq!(target.audio, audio_settings(&app.settings));
    assert_eq!(app.applying_share, Some(target));
}

#[test]
fn media_apply_preserves_the_previous_full_snapshot_for_one_attempt() {
    gst::init().unwrap();
    let old = test_share(true);
    let mut next = test_share(false);
    next.video.settings.width = 1920;
    next.video.settings.height = 1080;
    next.video.settings.bitrate_mbps = Some(12);
    let mut current = old.clone();
    let mut rollback = None;
    let mut recoveries = MAX_MEDIA_RECOVERIES;
    let mut fallback_attempted = true;
    let mut capture_caps = Some(gst::Caps::new_any());

    begin_media_apply(
        &mut current,
        &mut rollback,
        next.clone(),
        &mut recoveries,
        &mut fallback_attempted,
        &mut capture_caps,
    );

    assert_eq!(current, next);
    assert_eq!(rollback, Some(old));
    assert_eq!(recoveries, 0);
    assert!(!fallback_attempted);
    assert!(capture_caps.is_none());

    let mut sleeping = test_share(true);
    let previous = sleeping.clone();
    let audio_only = test_share(false);
    let mut sleeping_rollback = None;
    let mut retained_caps = Some(gst::Caps::new_any());
    begin_media_apply(
        &mut sleeping,
        &mut sleeping_rollback,
        audio_only.clone(),
        &mut recoveries,
        &mut fallback_attempted,
        &mut retained_caps,
    );
    assert_eq!(sleeping, audio_only);
    assert_eq!(sleeping_rollback, Some(previous));
    assert!(retained_caps.is_some());
}

#[test]
fn media_apply_rolls_back_only_media_start_errors() {
    let media_error: Result<ShareStop> = Err(io::Error::other("media failed").into());
    assert!(media_apply_failure(&media_error, false, true).is_some());
    assert!(media_apply_failure(&media_error, true, true).is_none());
    assert!(media_apply_failure(&media_error, false, false).is_none());

    let terminal: Result<ShareStop> =
        Ok(ShareStop::Failed(io::Error::other("server failed").into()));
    assert!(media_apply_failure(&terminal, false, true).is_none());
}

#[test]
fn overview_summary_uses_only_online_fresh_telemetry() {
    let now = Instant::now();
    let mut viewers = test_viewers(3, true);
    viewers[0].rtt = Some(Duration::from_millis(10));
    viewers[0].playback_lag = Some(Duration::from_millis(100));
    viewers[0].telemetry_at = Some(now);
    viewers[1].rtt = Some(Duration::from_millis(25));
    viewers[1].playback_lag = Some(Duration::from_millis(50));
    viewers[1].telemetry_at = Some(now);
    viewers[2].online_since = None;
    viewers[2].rtt = Some(Duration::from_secs(9));
    viewers[2].playback_lag = Some(Duration::from_secs(9));
    viewers[2].telemetry_at = Some(now);

    assert_eq!(
        viewer_summary(&viewers, now),
        ViewerSummary {
            online: 2,
            total: 3,
            worst_rtt: Some(Duration::from_millis(25)),
            worst_lag: Some(Duration::from_millis(100)),
        }
    );
    assert!(is_device_only(&settings::Settings::default()));
    let public = settings::Settings {
        share_base_url: Some("https://share.example:443".to_owned()),
        ..settings::Settings::default()
    };
    assert!(!is_device_only(&public));
}

#[test]
fn dirty_draft_requires_quit_confirmation_without_being_discarded() {
    let (mut app, _) = test_app();
    drop(update_app(&mut app, Message::Notifications(false)));
    let dirty = app.draft.clone();

    drop(update_app(&mut app, Message::Quit));
    assert!(app.confirm_quit);
    assert!(!app.quitting);
    assert_eq!(app.page, Page::Overview);
    assert_eq!(app.draft, dirty);
    drop(update_app(&mut app, Message::CancelQuit));
    assert!(!app.confirm_quit);
    assert_eq!(app.draft, dirty);
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
    let (mut app, _) = test_app();
    app.commands = None;
    app.window = None;
    app.page = Page::Viewers;
    app.viewers = vec![
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
    ];
    let _ = viewers_view(&app);
}
