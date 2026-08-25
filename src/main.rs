use std::{
    convert::Infallible,
    future::Future,
    io,
    os::fd::{AsRawFd, OwnedFd},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use ashpd::{
    PortalError,
    desktop::{
        PersistMode, ResponseError,
        screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
    },
};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::State,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use futures_util::{StreamExt, stream};
use gst::prelude::*;
use gst_app::AppSinkCallbacks;
use tokio::{
    net::TcpListener,
    sync::{Notify, broadcast, watch},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const VIEWER_HTML: &str = include_str!("viewer.html");

#[derive(Clone)]
struct WebState {
    media: broadcast::Sender<Bytes>,
    mime: watch::Receiver<Option<String>>,
    start: Arc<Notify>,
    claimed: Arc<AtomicBool>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let requested_source = requested_source(std::env::args().skip(1))?;
    gst::init()?;

    let portal = Screencast::new().await?;
    let available_sources = portal.available_source_types().await?;
    let available_cursors = portal.available_cursor_modes().await?;
    println!("Portal version: {}", portal.version());
    println!("Available source types: {available_sources:?}");
    println!("Available cursor modes: {available_cursors:?}");

    let requested_sources = match requested_source {
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
    let mut interrupt = Box::pin(tokio::signal::ctrl_c());
    let capture: ashpd::Result<Option<(u32, OwnedFd)>> = tokio::select! {
        result = async {
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

            let remote = portal
                .open_pipe_wire_remote(&session, Default::default())
                .await?;
            Ok((stream.pipe_wire_node_id(), remote))
        } => result.map(Some),
        signal = &mut interrupt => match signal {
            Ok(()) => {
                println!("Ctrl-C received; cancelling Portal request.");
                Ok(None)
            }
            Err(error) => Err(error.into()),
        },
    };

    let result = match capture {
        Ok(Some((node_id, remote))) => {
            serve_video(
                node_id,
                remote.as_raw_fd(),
                async {
                    let _ = closed.next().await;
                },
                interrupt,
            )
            .await
        }
        Ok(None) => Ok(false),
        Err(ashpd::Error::Response(ResponseError::Cancelled))
        | Err(ashpd::Error::Portal(PortalError::Cancelled(_))) => {
            println!("Portal request cancelled.");
            Ok(false)
        }
        Err(error) => Err(error.into()),
    };

    let close_result = if matches!(&result, Ok(true)) {
        Ok(())
    } else {
        session.close().await
    };
    if let Err(error) = &close_result {
        eprintln!("Failed to close Portal session: {error}");
    }
    result?;
    close_result?;
    Ok(())
}

fn requested_source(mut args: impl Iterator<Item = String>) -> io::Result<Option<SourceType>> {
    let source = match args.next().as_deref() {
        None => None,
        Some("--monitor") => Some(SourceType::Monitor),
        Some("--window") => Some(SourceType::Window),
        Some(_) => return Err(io::Error::other("usage: aercast [--monitor|--window]")),
    };
    if args.next().is_some() {
        return Err(io::Error::other("usage: aercast [--monitor|--window]"));
    }
    Ok(source)
}

fn cursor_mode(embedded: bool, hidden: bool) -> Option<CursorMode> {
    embedded
        .then_some(CursorMode::Embedded)
        .or_else(|| hidden.then_some(CursorMode::Hidden))
}

async fn viewer_page() -> Response {
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        Html(VIEWER_HTML),
    )
        .into_response()
}

async fn media_stream(State(state): State<WebState>) -> Response {
    if state
        .claimed
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return (StatusCode::CONFLICT, "Phase 2 supports one viewer").into_response();
    }

    let receiver = state.media.subscribe();
    state.start.notify_one();
    let mut mime = state.mime;
    let content_type = loop {
        if let Some(content_type) = mime.borrow().clone() {
            break content_type;
        }
        if mime.changed().await.is_err() {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let body = Body::from_stream(stream::unfold(receiver, |mut receiver| async move {
        match receiver.recv().await {
            Ok(chunk) => Some((Ok::<Bytes, Infallible>(chunk), receiver)),
            Err(error) => {
                eprintln!("Viewer stream closed: {error}");
                None
            }
        }
    }));

    Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(body)
        .expect("generated media headers are valid")
}

async fn serve_video(
    node_id: u32,
    remote_fd: i32,
    session_closed: impl Future<Output = ()>,
    interrupt: impl Future<Output = io::Result<()>>,
) -> Result<bool> {
    // ponytail: raw mux-buffer queue for one viewer; Phase 4 publishes cached GOPs.
    let (media, _) = broadcast::channel(512);
    let web_media = media.clone();
    let (mime_sender, mime) = watch::channel(None);
    let start = Arc::new(Notify::new());
    let claimed = Arc::new(AtomicBool::new(false));
    let media_started: Arc<OnceLock<Instant>> = Arc::new(OnceLock::new());

    // ponytail: normalize this niri/AMD DMA-BUF at 720p30 for the software proof.
    let pipeline = gst::parse::launch(&format!(
        "mp4mux name=mux fragment-duration=100 ! appsink name=stream sync=false wait-on-eos=false \\
         audiotestsrc is-live=true wave=silence ! audio/x-raw,format=F32LE,rate=48000,channels=2 ! avenc_aac bitrate=128000 ! aacparse ! audio/mpeg,mpegversion=4,stream-format=raw ! queue ! mux.audio_0 \\
         pipewiresrc fd={remote_fd} path={node_id} on-disconnect=error ! vapostproc disable-passthrough=true add-borders=true ! video/x-raw,format=I420,width=1280,height=720 ! imagefreeze is-live=true allow-replace=true ! video/x-raw,framerate=30/1 ! x264enc tune=zerolatency speed-preset=ultrafast bitrate=2500 key-int-max=30 ! h264parse name=h264 ! video/x-h264,stream-format=avc,alignment=au ! queue ! mux.video_0"
    ))?
    .downcast::<gst::Pipeline>()
    .map_err(|_| io::Error::other("GStreamer did not create a pipeline"))?;
    let parser_pad = pipeline
        .by_name("h264")
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no H.264 parser"))?
        .static_pad("src")
        .ok_or_else(|| io::Error::other("H.264 parser has no source pad"))?;
    parser_pad
        .add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
            if let Some(gst::PadProbeData::Event(event)) = &info.data
                && let gst::EventView::Caps(event) = event.view()
                && let Some(mime) = h264_mime(event.caps())
            {
                mime_sender.send_replace(Some(mime));
                return gst::PadProbeReturn::Remove;
            }
            gst::PadProbeReturn::Ok
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
                let media_started = Arc::clone(&media_started);
                let mut first_fragment = true;
                move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let bytes = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    if first_fragment && bytes.as_slice().get(4..8) == Some(b"moof".as_slice()) {
                        if let Some(started) = media_started.get() {
                            println!("First fMP4 fragment: {} ms", started.elapsed().as_millis());
                        }
                        first_fragment = false;
                    }
                    let _ = media.send(Bytes::copy_from_slice(bytes.as_slice()));
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

    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let app = Router::new()
        .route("/", get(viewer_page))
        .route(
            "/stream",
            get(media_stream).head(|| async { StatusCode::METHOD_NOT_ALLOWED }),
        )
        .with_state(WebState {
            media: web_media,
            mime,
            start: Arc::clone(&start),
            claimed,
        });
    let server = async move { axum::serve(listener, app).await };
    println!("Open http://{address}/ and select Play; press Ctrl-C to stop.");

    tokio::pin!(session_closed);
    tokio::pin!(interrupt);
    tokio::pin!(server);
    let outcome: Result<bool> = tokio::select! {
        _ = start.notified() => {
            let started = Instant::now();
            let _ = media_started.set(started);
            parser_pad
                .add_probe(gst::PadProbeType::BUFFER, move |_, _| {
                    println!("First encoded frame: {} ms", started.elapsed().as_millis());
                    gst::PadProbeReturn::Remove
                })
                .ok_or_else(|| io::Error::other("failed to install first-frame probe"))?;
            match pipeline.set_state(gst::State::Playing) {
                Err(error) => Err(error.into()),
                Ok(_) => {
                    println!("Browser stream running.");
                    tokio::select! {
                        signal = &mut interrupt => match signal {
                            Ok(()) => {
                                println!("Ctrl-C received; stopping stream.");
                                Ok(false)
                            }
                            Err(error) => Err(error.into()),
                        },
                        _ = &mut session_closed => {
                            println!("Portal session closed; stopping stream.");
                            Ok(true)
                        }
                        result = &mut server => server_outcome(result),
                        message = messages.next() => media_outcome(message),
                    }
                }
            }
        }
        signal = &mut interrupt => {
            match signal {
                Ok(()) => {
                    println!("Ctrl-C received; stopping server.");
                    Ok(false)
                }
                Err(error) => Err(error.into()),
            }
        }
        _ = &mut session_closed => {
            println!("Portal session closed; stopping server.");
            Ok(true)
        }
        result = &mut server => server_outcome(result),
    };

    let stop_result = pipeline.set_state(gst::State::Null);
    let portal_closed = outcome?;
    stop_result?;
    Ok(portal_closed)
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

fn server_outcome(result: io::Result<()>) -> Result<bool> {
    match result {
        Ok(()) => Err(io::Error::other("HTTP server stopped").into()),
        Err(error) => Err(error.into()),
    }
}

fn media_outcome(message: Option<gst::Message>) -> Result<bool> {
    match message {
        Some(message) => match message.view() {
            gst::MessageView::Eos(..) => {
                println!("Capture stream ended.");
                Ok(false)
            }
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
mod tests {
    use super::*;

    #[test]
    fn embedded_cursor_is_preferred_with_hidden_fallback() {
        assert_eq!(cursor_mode(true, true), Some(CursorMode::Embedded));
        assert_eq!(cursor_mode(false, true), Some(CursorMode::Hidden));
        assert_eq!(cursor_mode(false, false), None);
    }

    #[test]
    fn source_argument_accepts_one_known_value() {
        assert_eq!(requested_source(std::iter::empty()).unwrap(), None);
        assert_eq!(
            requested_source(["--window".to_owned()].into_iter()).unwrap(),
            Some(SourceType::Window)
        );
        assert!(requested_source(["--bad".to_owned()].into_iter()).is_err());
        assert!(
            requested_source(["--monitor".to_owned(), "extra".to_owned()].into_iter()).is_err()
        );
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
}
