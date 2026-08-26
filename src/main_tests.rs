use super::*;

#[test]
fn embedded_cursor_is_preferred_with_hidden_fallback() {
    assert_eq!(cursor_mode(true, true), Some(CursorMode::Embedded));
    assert_eq!(cursor_mode(false, true), Some(CursorMode::Hidden));
    assert_eq!(cursor_mode(false, false), None);
}

#[test]
fn retry_policy_allows_exactly_three_media_recoveries() {
    let media_failure = Err::<(), ()>(());
    for recoveries in 0..MAX_MEDIA_RECOVERIES {
        assert!(should_retry(&media_failure, recoveries));
    }
    assert!(!should_retry(&media_failure, MAX_MEDIA_RECOVERIES));

    for terminal in [
        ShareStop::Apply(false),
        ShareStop::End,
        ShareStop::Quit,
        ShareStop::PortalClosed,
        ShareStop::Failed(io::Error::other("host failed").into()),
    ] {
        assert!(!should_retry(&Ok::<_, ()>(terminal), 0));
    }
}

#[test]
fn media_eos_is_recoverable() {
    gst::init().unwrap();
    assert!(media_outcome(Some(gst::message::Eos::new())).is_err());
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
            "127.0.0.1:1".parse().unwrap(),
            &events,
        )
        .await,
        ShareStop::Failed(_)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn apply_requests_a_media_restart_without_reclassifying_control() {
    let (commands, mut receiver) = mpsc::channel(1);
    commands.send(Command::Apply(false)).await.unwrap();
    let mut server = tokio::spawn(std::future::pending::<io::Result<()>>());
    let host = web::Host::new().unwrap();
    let (events, _) = iced::futures::channel::mpsc::unbounded();
    assert!(matches!(
        share_control(
            &mut receiver,
            std::future::pending(),
            &mut server,
            &host,
            "127.0.0.1:1".parse().unwrap(),
            &events,
        )
        .await,
        ShareStop::Apply(false)
    ));
    server.abort();
}

#[test]
fn arguments_accept_one_source_and_repeated_exclusions() {
    let empty = options(std::iter::empty()).unwrap();
    assert_eq!(empty.bind, SocketAddr::from(([127, 0, 0, 1], 0)));
    assert_eq!(empty.source, None);
    assert!(empty.exclusions.is_empty());
    assert_eq!(
        options(["--window".to_owned()].into_iter()).unwrap().source,
        Some(SourceType::Window)
    );
    assert_eq!(
        options(
            [
                "--exclude".to_owned(),
                "org.example.Chat".to_owned(),
                "--monitor".to_owned(),
                "--exclude".to_owned(),
                "game-bin".to_owned(),
            ]
            .into_iter()
        )
        .unwrap()
        .exclusions,
        ["org.example.Chat", "game-bin"]
    );
    assert!(options(["--bad".to_owned()].into_iter()).is_err());
    assert!(options(["--monitor".to_owned(), "--window".to_owned()].into_iter()).is_err());
    assert!(options(["--exclude".to_owned()].into_iter()).is_err());
    assert!(options(["--exclude".to_owned(), "--monitor".to_owned()].into_iter()).is_err());
    assert_eq!(
        options(["--bind".to_owned(), "192.168.1.10:8080".to_owned()].into_iter())
            .unwrap()
            .bind,
        "192.168.1.10:8080".parse().unwrap()
    );
    assert!(options(["--bind".to_owned(), "0.0.0.0:8080".to_owned()].into_iter()).is_err());
}

#[test]
fn ui_commands_follow_the_host_lifecycle() {
    let (commands, mut receiver) = mpsc::channel(2);
    let window = window::Id::unique();
    let mut app = App {
        phase: Phase::Starting,
        link: String::new(),
        viewers: 0,
        commands: Some(commands),
        window: Some(window),
        confirm_refresh: false,
        settings: settings::Settings::default(),
        settings_open: false,
        settings_error: None,
        active_system_audio: None,
        applying_system_audio: None,
    };

    drop(update(
        &mut app,
        Message::Host(HostEvent::Waiting("http://127.0.0.1/s/token".to_owned())),
    ));
    assert_eq!(app.phase, Phase::Waiting);
    drop(update(&mut app, Message::Settings(true)));
    assert!(app.settings_open);
    drop(update(&mut app, Message::Settings(false)));
    assert!(!app.settings_open);
    app.settings.system_audio = false;
    drop(update(&mut app, Message::Start));
    assert_eq!(receiver.try_recv().unwrap(), Command::Start(false));
    assert_eq!(app.phase, Phase::Selecting);
    app.settings.system_audio = true;
    drop(update(&mut app, Message::ApplySystemAudio));
    assert!(receiver.try_recv().is_err());

    drop(update(&mut app, Message::Host(HostEvent::Sharing(false))));
    assert_eq!(app.active_system_audio, Some(false));
    drop(update(&mut app, Message::ApplySystemAudio));
    assert_eq!(receiver.try_recv().unwrap(), Command::Apply(true));
    assert_eq!(app.applying_system_audio, Some(true));
    drop(update(&mut app, Message::ApplySystemAudio));
    assert!(receiver.try_recv().is_err());
    drop(update(&mut app, Message::Host(HostEvent::Sharing(false))));
    assert_eq!(app.applying_system_audio, Some(true));
    drop(update(&mut app, Message::Host(HostEvent::Sharing(true))));
    assert_eq!(app.active_system_audio, Some(true));
    assert_eq!(app.applying_system_audio, None);

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

    app.viewers = 2;
    drop(update(&mut app, Message::Refresh));
    assert_eq!(receiver.try_recv().unwrap(), Command::Refresh(false));
    assert!(!app.confirm_refresh);
    drop(update(&mut app, Message::Host(HostEvent::ConfirmRefresh)));
    assert!(app.confirm_refresh);
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
    app.viewers = 0;
    drop(update(&mut app, Message::Refresh));
    assert_eq!(receiver.try_recv().unwrap(), Command::Refresh(false));
    assert!(!app.confirm_refresh);
    drop(update(&mut app, Message::End));
    assert_eq!(receiver.try_recv().unwrap(), Command::End);
    assert_eq!(app.phase, Phase::Ending);
    app.settings.system_audio = false;
    drop(update(&mut app, Message::ApplySystemAudio));
    assert!(receiver.try_recv().is_err());
    drop(update(&mut app, Message::Host(HostEvent::Sharing(false))));
    assert_eq!(app.phase, Phase::Ending);
    assert_eq!(app.active_system_audio, Some(true));
    drop(update(
        &mut app,
        Message::Host(HostEvent::Waiting("http://127.0.0.1/s/token".to_owned())),
    ));
    assert_eq!(app.phase, Phase::Waiting);
    assert_eq!(app.link, "http://127.0.0.1/s/token");
    assert_eq!(app.active_system_audio, None);

    assert_eq!(
        update(&mut app, Message::Host(HostEvent::Stopped(Ok(())))).units(),
        1
    );

    let (commands, _) = mpsc::channel(1);
    app.commands = Some(commands);
    app.phase = Phase::Waiting;
    app.viewers = 2;
    assert_eq!(
        update(&mut app, Message::Host(HostEvent::Stopped(Ok(())))).units(),
        1
    );
    assert_eq!(app.viewers, 0);

    let (commands, _) = mpsc::channel(1);
    app.commands = Some(commands);
    app.phase = Phase::Sharing;
    app.viewers = 3;
    app.settings_open = true;
    assert_eq!(
        update(
            &mut app,
            Message::Host(HostEvent::Stopped(Err("stopped".to_owned())))
        )
        .units(),
        0
    );
    assert_eq!(app.phase, Phase::Error("stopped".to_owned()));
    assert_eq!(app.viewers, 0);
    assert_ne!(update(&mut app, Message::Show).units(), 0);
    let reopened = app.window.unwrap();
    assert_eq!(app.phase, Phase::Error("stopped".to_owned()));
    assert_eq!(update(&mut app, Message::Close(reopened)).units(), 1);
    assert_eq!(app.window, None);
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
    if let Err(error) = gst::parse::launch(&pipeline_description(1, 0)) {
        assert_ne!(
            error.kind::<gst::ParseError>(),
            Some(gst::ParseError::Syntax)
        );
    }
}
