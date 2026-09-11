use crate::media::{
    Encoder, HardwareVideoFailure, VideoPlan, pipeline_description, plan_encoder, serve_video,
};
use crate::{
    Command, Error, Events, HostEvent, Result, ShareSettings, ShareStop, portal, settings, web,
};
use ashpd::{PortalError, desktop::ResponseError};
use futures_util::{FutureExt, StreamExt};
use socket2::SockRef;
use std::{
    future::Future,
    io,
    net::SocketAddr,
    os::fd::AsRawFd,
    time::{Duration, Instant},
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot, watch},
};

type Server = tokio::task::JoinHandle<io::Result<()>>;

struct RunningServer {
    address: SocketAddr,
    shutdown: oneshot::Sender<()>,
    task: Server,
}
const STALLED_CLIENT_TIMEOUT: Duration = Duration::from_secs(15);
const MEDIA_IDLE_GRACE: Duration = Duration::from_secs(2);
const MEDIA_RECOVERY_DELAY: Duration = Duration::from_millis(500);
pub(crate) const MAX_MEDIA_RECOVERIES: u8 = 3;

pub(crate) async fn run_host(
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
                "Could not listen on {bind}: {error}. Change Network settings and apply them"
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
        if let Some(server) = server
            && !server.task.is_finished()
        {
            server.task.abort();
            let _ = server.task.await;
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

pub(crate) struct MediaSettings {
    pub(crate) current: ShareSettings,
    rollback: Option<ShareSettings>,
    recoveries: u8,
    fallback_attempted: bool,
    capture_caps: Option<gst::Caps>,
}

impl MediaSettings {
    fn apply(&mut self, next: ShareSettings) {
        self.rollback.get_or_insert_with(|| self.current.clone());
        if self.current.video != next.video {
            self.capture_caps = None;
        }
        self.current = next;
        self.recoveries = 0;
        self.fallback_attempted = false;
    }

    pub(crate) fn new(current: ShareSettings) -> Self {
        Self {
            current,
            rollback: None,
            recoveries: 0,
            fallback_attempted: false,
            capture_caps: None,
        }
    }
}

pub(crate) enum ControlMedia<'a> {
    Active(watch::Receiver<bool>),
    Sleeping(&'a mut MediaSettings),
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
    let (portal, session, options) = portal::open().await?;
    let mut closed = session.receive_closed().await?;
    let capture = portal::select(&portal, &session, options);
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

    let mut media_settings = MediaSettings::new(share);
    let mut sleeping = false;
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
                ControlMedia::Sleeping(&mut media_settings),
            )
            .await
            {
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
            ControlMedia::Active(ready),
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
                                    media_settings.current.video,
                                    media_settings.current.audio.bitrate_kbps,
                                );
                                media = Some(active.clone());
                                serve_video(
                                    &description,
                                    &mut media_settings.capture_caps,
                                    media_settings.current.clone(),
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
            media_settings.rollback = None;
        }
        if let Ok(ShareStop::Apply(next)) = &attempt {
            media_settings.apply(next.clone());
            continue;
        }
        if matches!(&attempt, Ok(ShareStop::Sleep)) {
            media_settings.recoveries = 0;
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
                    media_settings.apply(next);
                    continue;
                }
                ShareStop::Sleep => {
                    media_settings.recoveries = 0;
                    sleeping = true;
                    continue;
                }
                ShareStop::Wake => {}
                stop => break Ok(stop),
            }
        }

        if let Some(error) =
            media_apply_failure(&attempt, reached_sharing, media_settings.rollback.is_some())
        {
            let error = error.to_string();
            media_settings.current = media_settings
                .rollback
                .take()
                .expect("media_settings.rollback checked above");
            media_settings.capture_caps = None;
            media_settings.recoveries = 0;
            media_settings.fallback_attempted = false;
            let _ = events.unbounded_send(HostEvent::Sharing(media_settings.current.clone()));
            let _ = events.unbounded_send(HostEvent::ApplyFailed(format!(
                "Could not apply the saved media settings: {error}. Restored the previous active settings"
            )));
            continue;
        }

        if attempt.as_ref().err().is_some_and(|error| {
            should_fallback(
                media_settings.current.video,
                media_settings.fallback_attempted,
                media_settings.recoveries,
                error,
            )
        }) {
            media_settings.fallback_attempted = true;
            eprintln!(
                "VA-API media path failed; probing x264 for recovery {}/{MAX_MEDIA_RECOVERIES}",
                media_settings.recoveries + 1,
            );
            let settings = media_settings.current.video.settings;
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
                    media_settings.apply(next);
                    continue;
                }
                Fallback::Control(ShareStop::Sleep) => {
                    media_settings.fallback_attempted = false;
                    media_settings.recoveries = 0;
                    sleeping = true;
                    continue;
                }
                Fallback::Control(ShareStop::Wake) => {}
                Fallback::Control(stop) => break Ok(stop),
                Fallback::Probe(Ok(Ok(plan))) => {
                    media_settings.recoveries += 1;
                    media_settings.current.video = plan;
                    media_settings.capture_caps = None;
                    continue;
                }
                Fallback::Probe(Ok(Err(error))) => attempt = Err(error),
                Fallback::Probe(Err(error)) => {
                    attempt =
                        Err(io::Error::other(format!("x264 encoder check failed: {error}")).into());
                }
            }
        }

        if !should_retry(&attempt, media_settings.recoveries) {
            break attempt;
        }
        media_settings.recoveries += 1;
        if let Err(error) = &attempt {
            eprintln!(
                "Media attempt failed; recovery {}/{MAX_MEDIA_RECOVERIES}: {error}",
                media_settings.recoveries
            );
        }
        tokio::select! {
            biased;
            stop = control.as_mut() => match stop {
                ShareStop::Apply(next) => {
                    media_settings.apply(next);
                    continue;
                }
                ShareStop::Sleep => {
                    media_settings.recoveries = 0;
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

pub(crate) async fn share_control(
    commands: &mut mpsc::Receiver<Command>,
    session_closed: impl Future<Output = ()>,
    server: &mut Server,
    host: &web::Host,
    link_base: &str,
    events: &Events,
    mut media_state: ControlMedia<'_>,
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
    let _ = events.unbounded_send(HostEvent::MediaIdle(matches!(
        media_state,
        ControlMedia::Sleeping(_)
    )));
    let ready = matches!(&media_state, ControlMedia::Active(ready) if *ready.borrow());
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
        let sleeping = matches!(media_state, ControlMedia::Sleeping(_));
        let idle_at = deadline;
        let event = tokio::select! {
            biased;
            command = commands.recv() => ControlEvent::Command(command),
            _ = &mut session_closed => ControlEvent::PortalClosed,
            result = &mut *server => ControlEvent::Server(result),
            signal = tokio::signal::ctrl_c() => ControlEvent::Signal(signal),
            changed = viewer_updates.changed() => ControlEvent::Viewers(changed.is_ok()),
            changed = async {
                match &mut media_state {
                    ControlMedia::Active(ready) => ready.changed().await.is_ok(),
                    ControlMedia::Sleeping(_) => std::future::pending().await,
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
                Command::Apply(next) => match &mut media_state {
                    ControlMedia::Active(_) => return ShareStop::Apply(next),
                    ControlMedia::Sleeping(state) => {
                        state.apply(next);
                        let _ = events.unbounded_send(HostEvent::Sharing(state.current.clone()));
                    }
                },
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
                        let ready =
                            matches!(&media_state, ControlMedia::Active(ready) if *ready.borrow());
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
                        let ready =
                            matches!(&media_state, ControlMedia::Active(ready) if *ready.borrow());
                        deadline = idle_deadline(deadline, ready, online, Instant::now());
                        let _ = events.unbounded_send(HostEvent::Viewers(viewers));
                    }
                    Err(error) => return ShareStop::Failed(error.into()),
                }
            }
            ControlEvent::Ready(open) => {
                let ControlMedia::Active(ready) = &mut media_state else {
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
                let ready = matches!(&media_state, ControlMedia::Active(ready) if *ready.borrow());
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

pub(crate) fn should_fallback(
    video: VideoPlan,
    attempted: bool,
    recoveries: u8,
    error: &Error,
) -> bool {
    video.settings.encoder == settings::VideoEncoder::Auto
        && video.encoder == Encoder::VaApi
        && !attempted
        && recoveries < MAX_MEDIA_RECOVERIES
        && error.is::<HardwareVideoFailure>()
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

#[cfg(test)]
#[path = "share_session_tests.rs"]
mod tests;
