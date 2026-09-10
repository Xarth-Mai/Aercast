use super::*;
use crate::media::audio_settings;

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
            ControlMedia::Active(ready),
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
            ControlMedia::Sleeping(&mut MediaSettings::new(test_share(true))),
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
            ControlMedia::Active(watch::channel(false).1),
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
            ControlMedia::Active(watch::channel(false).1),
        )
        .await,
        ShareStop::Apply(share) if share == test_share(false)
    ));
    server.abort();
}

#[test]
fn media_apply_preserves_the_previous_full_snapshot_for_one_attempt() {
    gst::init().unwrap();
    let old = test_share(true);
    let mut next = test_share(false);
    next.video.settings.width = 1920;
    next.video.settings.height = 1080;
    next.video.settings.bitrate_mbps = Some(12);
    let mut state = MediaSettings::new(old.clone());
    state.recoveries = MAX_MEDIA_RECOVERIES;
    state.fallback_attempted = true;
    state.capture_caps = Some(gst::Caps::new_any());
    state.apply(next.clone());
    assert_eq!(state.current, next);
    assert_eq!(state.rollback, Some(old));
    assert_eq!(state.recoveries, 0);
    assert!(!state.fallback_attempted);
    assert!(state.capture_caps.is_none());

    let previous = test_share(true);
    let mut sleeping = MediaSettings::new(previous.clone());
    sleeping.capture_caps = Some(gst::Caps::new_any());
    let audio_only = test_share(false);
    sleeping.apply(audio_only.clone());
    assert_eq!(sleeping.current, audio_only);
    assert_eq!(sleeping.rollback, Some(previous));
    assert!(sleeping.capture_caps.is_some());
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

#[tokio::test(flavor = "current_thread")]
async fn sleeping_apply_waits_for_a_valid_viewer_before_media_can_restart() {
    let host = web::Host::new().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown, stopped) = oneshot::channel();
    let mut server = tokio::spawn(web::serve(listener, host.clone(), stopped));
    let (commands, mut receiver) = mpsc::channel(2);
    let (events, mut received) = iced::futures::channel::mpsc::unbounded();
    let (ready_sender, ready) = watch::channel(true);
    assert!(matches!(
        share_control(
            &mut receiver,
            std::future::pending(),
            &mut server,
            &host,
            "http://127.0.0.1",
            &events,
            ControlMedia::Active(ready)
        )
        .await,
        ShareStop::Sleep
    ));
    drop(ready_sender);

    let previous = test_share(true);
    let mut state = MediaSettings::new(previous.clone());
    let mut next = test_share(false);
    next.audio.bitrate_kbps = 160;
    commands
        .send(Command::Apply(test_share(false)))
        .await
        .unwrap();
    commands.send(Command::Apply(next.clone())).await.unwrap();
    {
        let sleeping = share_control(
            &mut receiver,
            std::future::pending(),
            &mut server,
            &host,
            "http://127.0.0.1",
            &events,
            ControlMedia::Sleeping(&mut state),
        );
        tokio::pin!(sleeping);
        // The production caller cannot open the remote or construct media until this returns
        assert!(
            tokio::time::timeout(Duration::from_millis(50), sleeping.as_mut())
                .await
                .is_err()
        );
        let mut applied = Vec::new();
        while let Some(Some(event)) = received.next().now_or_never() {
            if let HostEvent::Sharing(settings) = event {
                applied.push(settings);
            }
        }
        assert_eq!(applied, vec![test_share(false), next.clone()]);
        let path = host.path().unwrap();
        let request = tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            let mut socket = std::net::TcpStream::connect(address).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(socket, "GET {path}/stream HTTP/1.1\r\nHost: localhost\r\nAercast-Viewer-ID: 11111111111111111111111111111111\r\nConnection: close\r\n\r\n").unwrap();
            let mut response = String::new();
            socket.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 425"));
        });
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), sleeping.as_mut())
                .await
                .unwrap(),
            ShareStop::Wake
        ));
        request.await.unwrap();
    }
    assert_eq!(state.current, next);
    assert_eq!(state.rollback, Some(previous));
    shutdown.send(()).unwrap();
    server.await.unwrap().unwrap();
}
