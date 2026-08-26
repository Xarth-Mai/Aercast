use super::*;

#[test]
fn embedded_cursor_is_preferred_with_hidden_fallback() {
    assert_eq!(cursor_mode(true, true), Some(CursorMode::Embedded));
    assert_eq!(cursor_mode(false, true), Some(CursorMode::Hidden));
    assert_eq!(cursor_mode(false, false), None);
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
    let mut app = App {
        phase: Phase::Starting,
        link: String::new(),
        viewers: 0,
        commands: Some(commands),
        closing: None,
    };

    drop(update(
        &mut app,
        Message::Host(HostEvent::Waiting("http://127.0.0.1/s/token".to_owned())),
    ));
    assert_eq!(app.phase, Phase::Waiting);
    drop(update(&mut app, Message::Start));
    assert_eq!(receiver.try_recv().unwrap(), Command::Start);
    assert_eq!(app.phase, Phase::Selecting);

    drop(update(&mut app, Message::Host(HostEvent::Sharing)));
    drop(update(&mut app, Message::End));
    assert_eq!(receiver.try_recv().unwrap(), Command::End);
    assert_eq!(app.phase, Phase::Ending);
    drop(update(
        &mut app,
        Message::Host(HostEvent::Ended("http://127.0.0.1/s/fresh".to_owned())),
    ));
    assert_eq!(app.phase, Phase::Ended);
    assert_eq!(app.link, "http://127.0.0.1/s/fresh");

    let id = window::Id::unique();
    assert_eq!(update(&mut app, Message::Close(id)).units(), 0);
    assert_eq!(receiver.try_recv().unwrap(), Command::Quit);
    assert_eq!(app.phase, Phase::Closing);
    assert_eq!(
        update(&mut app, Message::Host(HostEvent::Stopped(Ok(())))).units(),
        1
    );
    assert_eq!(app.closing, None);

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
    assert_eq!(
        update(&mut app, Message::Close(window::Id::unique())).units(),
        1
    );
    assert_eq!(app.closing, None);
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
