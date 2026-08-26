use super::*;

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
            "http://127.0.0.1:1",
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
            "http://127.0.0.1:1",
            &events,
        )
        .await,
        ShareStop::Apply(false)
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
fn arguments_accept_repeated_exclusions() {
    let empty = options(std::iter::empty()).unwrap();
    assert!(empty.exclusions.is_empty());
    assert_eq!(
        options(
            [
                "--exclude".to_owned(),
                "org.example.Chat".to_owned(),
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
    assert!(options(["--monitor".to_owned()].into_iter()).is_err());
    assert!(options(["--window".to_owned()].into_iter()).is_err());
    assert!(options(["--bind".to_owned(), "127.0.0.1:9000".to_owned()].into_iter()).is_err());
    assert!(options(["--exclude".to_owned()].into_iter()).is_err());
    assert!(options(["--exclude".to_owned(), "--monitor".to_owned()].into_iter()).is_err());
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
        network_address: "127.0.0.1".to_owned(),
        network_port: "8877".to_owned(),
        share_base_url: String::new(),
        applying_network: false,
        quitting: false,
    };

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
    drop(update(&mut app, Message::Settings(true)));
    assert!(app.settings_open);
    drop(update(&mut app, Message::Settings(false)));
    assert!(!app.settings_open);

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

    let (commands, _) = mpsc::channel(1);
    app.commands = Some(commands);
    let closing = window::Id::unique();
    app.window = Some(closing);
    assert_eq!(update(&mut app, Message::Quit).units(), 1);
    assert!(app.commands.is_none());
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
        1
    );
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
