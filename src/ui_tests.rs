use super::*;
use futures_util::{FutureExt, StreamExt};

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

#[test]
fn viewers_view_handles_identical_ips() {
    let (mut app, _) = test_app();
    app.viewers = test_viewers(2, true);
    app.viewers[1].ip = "192.0.2.1".parse().unwrap();
    let _ = viewers_view(&app);
}

#[test]
fn terminal_host_failure_allows_one_restart_without_discarding_draft() {
    let (mut app, old_commands) = test_app();
    app.draft.network_port = "9999".to_owned();
    let original_draft = app.draft.clone();
    let _ = update_app(&mut app, Message::RestartHost);
    assert_eq!(app.phase, Phase::Waiting);
    let _ = update_app(
        &mut app,
        Message::Host(HostEvent::Stopped(Err("audio failed".to_owned()))),
    );
    assert!(app.host_stopped);
    assert!(app.commands.is_none());
    assert!(app.link.is_empty());
    assert!(old_commands.is_closed());
    let restart = update_app(&mut app, Message::RestartHost);
    assert_eq!(app.phase, Phase::Starting);
    assert!(!app.host_stopped);
    let commands = app.commands.as_ref().unwrap().clone();
    let _ = update_app(&mut app, Message::RestartHost);
    assert!(commands.same_channel(app.commands.as_ref().unwrap()));
    assert_eq!(app.draft, original_draft);
    let _ = update_app(
        &mut app,
        Message::Host(HostEvent::Waiting("new link".to_owned())),
    );
    let _ = update_app(&mut app, Message::Start);
    assert_eq!(app.phase, Phase::Selecting);
    drop(restart);

    let (mut quitting, _) = test_app();
    quitting.phase = Phase::Error("stopped".to_owned());
    quitting.host_stopped = true;
    quitting.commands = None;
    quitting.quitting = true;
    let _ = update_app(&mut quitting, Message::RestartHost);
    assert!(quitting.commands.is_none());
    assert!(quitting.host_stopped);
}
