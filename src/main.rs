use std::{
    future::Future,
    io,
    net::SocketAddr,
    os::fd::AsRawFd,
    pin::Pin,
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
    futures::channel::mpsc::UnboundedSender,
    widget::{button, column, container, row, text, text_input},
    window,
};
use socket2::SockRef;
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot},
};

mod audio;
mod web;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;
type Events = UnboundedSender<HostEvent>;

#[derive(Clone)]
struct Options {
    bind: SocketAddr,
    source: Option<SourceType>,
    exclusions: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    Start,
    End,
    Quit,
}

enum ShareStop {
    End,
    Quit,
    PortalClosed,
    Failed(Error),
}

#[derive(Clone, Debug)]
enum HostEvent {
    Waiting(String),
    Sharing,
    Ending,
    Viewers(usize),
    Stopped(std::result::Result<(), String>),
}

#[derive(Clone, Debug)]
enum Message {
    Host(HostEvent),
    Start,
    End,
    Copy,
    Close(window::Id),
}

#[derive(Debug, PartialEq)]
enum Phase {
    Starting,
    Waiting,
    Selecting,
    Sharing,
    Ending,
    Closing,
    Error(String),
}

struct App {
    phase: Phase,
    link: String,
    viewers: usize,
    commands: Option<mpsc::Sender<Command>>,
    closing: Option<window::Id>,
}

type Server = tokio::task::JoinHandle<io::Result<()>>;
const STALLED_CLIENT_TIMEOUT: Duration = Duration::from_secs(15);
const MEDIA_RECOVERY_DELAY: Duration = Duration::from_millis(500);
const MAX_MEDIA_RECOVERIES: u8 = 3;

fn main() -> Result<()> {
    let options = options(std::env::args().skip(1))?;
    gst::init()?;

    iced::application(move || boot(options.clone()), update, view)
        .title("Aercast")
        .theme(Theme::Dark)
        .window_size((560.0, 320.0))
        .exit_on_close_request(false)
        .subscription(|_| window::close_requests().map(Message::Close))
        .run()?;
    Ok(())
}

fn boot(options: Options) -> (App, Task<Message>) {
    let (events, incoming) = iced::futures::channel::mpsc::unbounded();
    let (commands, command_receiver) = mpsc::channel(8);
    (
        App {
            phase: Phase::Starting,
            link: String::new(),
            viewers: 0,
            commands: Some(commands),
            closing: None,
        },
        Task::batch([
            Task::run(incoming, Message::Host),
            Task::perform(run_host(options, events, command_receiver), |result| {
                Message::Host(HostEvent::Stopped(
                    result.map_err(|error| error.to_string()),
                ))
            }),
        ]),
    )
}

fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::Start => {
            if send_command(app, Command::Start) {
                app.phase = Phase::Selecting;
            }
        }
        Message::End => {
            if send_command(app, Command::End) {
                app.phase = Phase::Ending;
            }
        }
        Message::Copy => return clipboard::write(app.link.clone()),
        Message::Close(id) => {
            if app.commands.is_none() {
                return window::close(id);
            }
            app.closing = Some(id);
            let _ = send_command(app, Command::Quit);
            app.phase = Phase::Closing;
        }
        Message::Host(event) => match event {
            HostEvent::Waiting(link) => {
                app.link = link;
                app.viewers = 0;
                if app.closing.is_none() {
                    app.phase = Phase::Waiting;
                }
            }
            HostEvent::Sharing if app.closing.is_none() => app.phase = Phase::Sharing,
            HostEvent::Ending if app.closing.is_none() => app.phase = Phase::Ending,
            HostEvent::Viewers(viewers) => app.viewers = viewers,
            HostEvent::Stopped(result) => {
                app.commands = None;
                app.viewers = 0;
                if let Some(id) = app.closing.take() {
                    return window::close(id);
                }
                match result {
                    Ok(()) => return iced::exit(),
                    Err(error) => app.phase = Phase::Error(error),
                }
            }
            HostEvent::Sharing | HostEvent::Ending => {}
        },
    }
    Task::none()
}

fn send_command(app: &mut App, command: Command) -> bool {
    if app
        .commands
        .as_ref()
        .is_some_and(|commands| commands.try_send(command).is_ok())
    {
        true
    } else {
        app.phase = Phase::Error("Host control is unavailable".to_owned());
        false
    }
}

fn view(app: &App) -> Element<'_, Message> {
    let status = match &app.phase {
        Phase::Starting => "Starting Aercast…",
        Phase::Waiting => "Ready. Capture has not started.",
        Phase::Selecting => "Choose one screen or window in the system picker.",
        Phase::Sharing => "Sharing.",
        Phase::Ending => "Ending share…",
        Phase::Closing => "Cleaning up…",
        Phase::Error(error) => error,
    };
    let can_start = app.phase == Phase::Waiting;
    let can_end = matches!(app.phase, Phase::Selecting | Phase::Sharing);
    let end_label = if app.phase == Phase::Selecting {
        "Cancel"
    } else {
        "End Sharing"
    };

    container(
        column![
            text("Aercast").size(36),
            text(status),
            text_input("Share link will appear here", &app.link),
            row![
                button("Copy Link").on_press_maybe((!app.link.is_empty()).then_some(Message::Copy)),
                button("Start Sharing").on_press_maybe(can_start.then_some(Message::Start)),
                button(end_label).on_press_maybe(can_end.then_some(Message::End)),
            ]
            .spacing(12),
            text(format!("Viewers: {}", app.viewers)),
            text("Trusted LAN only. Use an external HTTPS reverse proxy elsewhere.").size(12),
        ]
        .spacing(16)
        .max_width(520),
    )
    .padding(24)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}

async fn run_host(
    options: Options,
    events: Events,
    mut command_receiver: mpsc::Receiver<Command>,
) -> Result<()> {
    let host = web::Host::new()?;
    let listener = TcpListener::bind(options.bind).await?;
    SockRef::from(&listener).set_tcp_user_timeout(Some(STALLED_CLIENT_TIMEOUT))?;
    let address = listener.local_addr()?;
    let (shutdown, shutdown_request) = oneshot::channel();
    let mut server = tokio::spawn(web::serve(listener, host.clone(), shutdown_request));
    let outcome: Result<()> = async {
        loop {
            let link = format!("http://{address}{}", host.path()?);
            let _ = events.unbounded_send(HostEvent::Waiting(link));
            let command = tokio::select! {
                result = &mut server => return server_outcome(result).map(|_| ()),
                signal = tokio::signal::ctrl_c() => {
                    signal?;
                    Command::Quit
                }
                command = command_receiver.recv() => command.unwrap_or(Command::Quit),
            };
            match command {
                Command::Start => {
                    match share_once(&options, &host, &mut command_receiver, &mut server, &events)
                        .await?
                    {
                        ShareStop::End | ShareStop::PortalClosed => {}
                        ShareStop::Quit => break,
                        ShareStop::Failed(error) => return Err(error),
                    }
                }
                Command::End => {}
                Command::Quit => break,
            }
        }
        Ok(())
    }
    .await;

    if let Err(error) = outcome {
        server.abort();
        return Err(error);
    }
    let _ = shutdown.send(());
    match tokio::time::timeout(STALLED_CLIENT_TIMEOUT + Duration::from_secs(1), &mut server).await {
        Ok(Ok(result)) => result?,
        Ok(Err(error)) => return Err(io::Error::other(error.to_string()).into()),
        Err(_) => {
            server.abort();
            if let Err(error) = server.await
                && !error.is_cancelled()
            {
                return Err(io::Error::other(error.to_string()).into());
            }
        }
    }
    Ok(())
}

async fn share_once(
    options: &Options,
    host: &web::Host,
    commands: &mut mpsc::Receiver<Command>,
    server: &mut Server,
    events: &Events,
) -> Result<ShareStop> {
    let portal = Screencast::new().await?;
    let available_sources = portal.available_source_types().await?;
    let available_cursors = portal.available_cursor_modes().await?;
    println!("Portal version: {}", portal.version());
    println!("Available source types: {available_sources:?}");
    println!("Available cursor modes: {available_cursors:?}");

    let requested_sources = match options.source {
        Some(source) => source.into(),
        None => SourceType::Monitor | SourceType::Window,
    };
    let sources = available_sources & requested_sources;
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
        println!(
            "Selected stream: node={}, source={:?}, size={:?}, position={:?}, id={:?}, mapping_id={:?}",
            stream.pipe_wire_node_id(),
            stream.source_type(),
            stream.size(),
            stream.position(),
            stream.id(),
            stream.mapping_id(),
        );
        Ok(stream.pipe_wire_node_id())
    };
    tokio::pin!(capture);

    enum Selection {
        Capture(ashpd::Result<u32>),
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
                Command::Start => println!("Source selection is already open."),
                Command::End => break Selection::Stop(false),
                Command::Quit => break Selection::Stop(true),
            },
        }
    };
    let node_id = match selection {
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

    let control = share_control(
        commands,
        async {
            let _ = closed.next().await;
        },
        server,
    );
    tokio::pin!(control);
    let mut recoveries = 0;
    let result = loop {
        let remote = tokio::select! {
            biased;
            stop = control.as_mut() => {
                break Ok(stop);
            }
            remote = portal.open_pipe_wire_remote(&session, Default::default()) => match remote {
                Ok(remote) => remote,
                Err(error) => break Ok(ShareStop::Failed(error.into())),
            },
        };
        let media = match host.start() {
            Ok(media) => media,
            Err(error) => break Ok(ShareStop::Failed(error.into())),
        };
        let attempt = serve_video(
            node_id,
            remote.as_raw_fd(),
            options.exclusions.clone(),
            media.clone(),
            control.as_mut(),
            events,
        )
        .await;
        let stopped = host.stop(&media);
        let _ = events.unbounded_send(HostEvent::Viewers(0));
        let attempt = match stopped {
            Ok(()) => attempt,
            Err(error) => {
                eprintln!("Failed to stop media session: {error}");
                Ok(ShareStop::Failed(error.into()))
            }
        };
        if attempt.is_err()
            && let Some(stop) = control.as_mut().now_or_never()
        {
            break Ok(stop);
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
            stop = control.as_mut() => {
                break Ok(stop);
            }
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
) -> ShareStop {
    tokio::pin!(session_closed);
    tokio::select! {
        signal = tokio::signal::ctrl_c() => {
            match signal {
                Ok(()) => {
                    println!("Stopping Aercast.");
                    ShareStop::Quit
                }
                Err(error) => ShareStop::Failed(error.into()),
            }
        },
        quit = stop_command(commands) => {
            if quit {
                println!("Stopping Aercast.");
                ShareStop::Quit
            } else {
                println!("Ending share.");
                ShareStop::End
            }
        },
        _ = &mut session_closed => {
            println!("Portal session closed; stopping stream.");
            ShareStop::PortalClosed
        }
        result = &mut *server => server_outcome(result).unwrap_or_else(ShareStop::Failed),
    }
}

async fn stop_command(commands: &mut mpsc::Receiver<Command>) -> bool {
    loop {
        match commands.recv().await.unwrap_or(Command::Quit) {
            Command::Start => println!("A share is already active."),
            Command::End => return false,
            Command::Quit => return true,
        }
    }
}

fn should_retry<T, E>(outcome: &std::result::Result<T, E>, recoveries: u8) -> bool {
    outcome.is_err() && recoveries < MAX_MEDIA_RECOVERIES
}

fn options(mut args: impl Iterator<Item = String>) -> io::Result<Options> {
    let mut bind = None;
    let mut source = None;
    let mut exclusions = Vec::new();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--bind" if bind.is_none() => {
                let address = args
                    .next()
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(usage)?
                    .parse::<SocketAddr>()
                    .map_err(|_| usage())?;
                if address.ip().is_unspecified() || address.ip().is_multicast() {
                    return Err(usage());
                }
                bind = Some(address);
            }
            "--monitor" if source.is_none() => source = Some(SourceType::Monitor),
            "--window" if source.is_none() => source = Some(SourceType::Window),
            "--exclude" => exclusions.push(
                args.next()
                    .filter(|value| !value.is_empty() && !value.starts_with("--"))
                    .ok_or_else(usage)?,
            ),
            _ => return Err(usage()),
        }
    }
    Ok(Options {
        bind: bind.unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 0))),
        source,
        exclusions,
    })
}

fn usage() -> io::Error {
    io::Error::other(
        "usage: aercast [--bind IP:PORT] [--monitor|--window] [--exclude APPLICATION_ID_OR_NAME]...",
    )
}

fn cursor_mode(embedded: bool, hidden: bool) -> Option<CursorMode> {
    embedded
        .then_some(CursorMode::Embedded)
        .or_else(|| hidden.then_some(CursorMode::Hidden))
}

async fn serve_video(
    node_id: u32,
    remote_fd: i32,
    exclusions: Vec<String>,
    media: web::MediaSession,
    mut control: Pin<&mut impl Future<Output = ShareStop>>,
    events: &Events,
) -> Result<ShareStop> {
    if exclusions.is_empty() {
        eprintln!(
            "No audio exclusions configured; a Host-local Viewer may feed shared audio back into Aercast."
        );
    }
    let started = Instant::now();

    // ponytail: normalize this niri/AMD DMA-BUF at 720p30 for the software proof.
    let pipeline = gst::parse::launch(&pipeline_description(node_id, remote_fd))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| io::Error::other("GStreamer did not create a pipeline"))?;
    let parser_pad = pipeline
        .by_name("h264")
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no H.264 parser"))?
        .static_pad("src")
        .ok_or_else(|| io::Error::other("H.264 parser has no source pad"))?;
    let system_audio = pipeline
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
    let outcome: Result<ShareStop> = match pipeline.set_state(gst::State::Playing) {
        Err(error) => Err(error.into()),
        Ok(_) => match audio::start(system_audio, exclusions) {
            Err(error) => Err(error.into()),
            Ok((audio, mut audio_errors)) => {
                println!("Browser stream running.");
                let _ = events.unbounded_send(HostEvent::Sharing);
                let mut viewers = media.viewer_count();
                let _ = events.unbounded_send(HostEvent::Viewers(*viewers.borrow_and_update()));
                let running = loop {
                    let result = tokio::select! {
                        biased;
                        stop = control.as_mut() => Ok(stop),
                        message = messages.next() => media_outcome(message),
                        error = audio_errors.recv() => Err(io::Error::other(
                            error.unwrap_or_else(|| "selective-audio thread stopped unexpectedly".to_owned())
                        ).into()),
                        changed = viewers.changed() => {
                            if changed.is_err() {
                                Err(io::Error::other("Viewer count channel closed").into())
                            } else {
                                let _ = events.unbounded_send(HostEvent::Viewers(
                                    *viewers.borrow_and_update()
                                ));
                                continue;
                            }
                        }
                    };
                    break result;
                };
                if running.is_ok() {
                    let _ = events.unbounded_send(HostEvent::Ending);
                }
                let stopped: Result<()> =
                    audio.stop().map_err(|error| io::Error::other(error).into());
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

    let stop_result = pipeline.set_state(gst::State::Null);
    if let Err(error) = &stop_result {
        eprintln!("Failed to stop GStreamer pipeline: {error}");
    }
    match stop_result {
        Ok(_) => outcome,
        Err(error) => Ok(ShareStop::Failed(error.into())),
    }
}

fn pipeline_description(node_id: u32, remote_fd: i32) -> String {
    format!(
        "mp4mux name=mux fragment-duration=100 ! appsink name=stream sync=false wait-on-eos=false
         audiomixer name=audio-mixer ignore-inactive-pads=true ! audioconvert ! audio/x-raw,format=F32LE,rate=48000,channels=2 ! avenc_aac bitrate=128000 ! aacparse ! audio/mpeg,mpegversion=4,stream-format=raw ! queue ! mux.audio_0
         audiotestsrc is-live=true wave=silence ! audio/x-raw,format=F32LE,rate=48000,channels=2 ! queue ! audio-mixer.
         appsrc name=system-audio is-live=true format=time do-timestamp=true block=false max-bytes=384000 leaky-type=downstream ! audio/x-raw,format=F32LE,rate=48000,channels=2,layout=interleaved ! queue ! audio-mixer.
         pipewiresrc fd={remote_fd} path={node_id} on-disconnect=error ! vapostproc disable-passthrough=true add-borders=true ! video/x-raw,format=I420,width=1280,height=720 ! imagefreeze is-live=true allow-replace=true ! video/x-raw,framerate=30/1 ! x264enc tune=zerolatency speed-preset=ultrafast bitrate=2500 key-int-max=30 ! h264parse name=h264 ! video/x-h264,stream-format=avc,alignment=au ! queue ! mux.video_0"
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

fn media_outcome(message: Option<gst::Message>) -> Result<ShareStop> {
    match message {
        Some(message) => match message.view() {
            gst::MessageView::Eos(..) => Err(io::Error::other("capture stream ended").into()),
            gst::MessageView::Error(error) => {
                let source = message
                    .src()
                    .map(|source| source.path_string().to_string())
                    .unwrap_or_else(|| "unknown".to_owned());
                Err(io::Error::other(format!(
                    "GStreamer error from {source}: {} ({})",
                    error.error(),
                    error.debug().unwrap_or_default(),
                ))
                .into())
            }
            _ => unreachable!(),
        },
        None => Err(io::Error::other("GStreamer bus closed").into()),
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
