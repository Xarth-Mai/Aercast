use std::{
    future::Future,
    io,
    os::fd::{AsRawFd, OwnedFd},
    time::Instant,
};

use ashpd::{
    PortalError,
    desktop::{
        PersistMode, ResponseError,
        screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
    },
};
use futures_util::StreamExt;
use gst::prelude::*;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

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
            preview(
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

async fn preview(
    node_id: u32,
    remote_fd: i32,
    session_closed: impl Future<Output = ()>,
    interrupt: impl Future<Output = io::Result<()>>,
) -> Result<bool> {
    // ponytail: normalize this niri/AMD DMA-BUF with the installed VA postprocessor.
    let pipeline = gst::parse::launch(&format!(
        "pipewiresrc name=capture fd={remote_fd} path={node_id} ! vapostproc disable-passthrough=true ! video/x-raw,format=BGRx ! waylandsink sync=false"
    ))?
    .downcast::<gst::Pipeline>()
    .map_err(|_| io::Error::other("GStreamer did not create a pipeline"))?;
    let source = pipeline
        .by_name("capture")
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no capture source"))?;
    let source_pad = source
        .static_pad("src")
        .ok_or_else(|| io::Error::other("PipeWire source has no source pad"))?;
    let started = Instant::now();
    source_pad
        .add_probe(gst::PadProbeType::BUFFER, move |pad, info| {
            let caps = pad
                .current_caps()
                .map(|caps| caps.to_string())
                .unwrap_or_else(|| "unknown".to_owned());
            let memory = info
                .buffer()
                .and_then(|buffer| buffer.memory(0))
                .and_then(|memory| {
                    memory
                        .allocator()
                        .map(|allocator| allocator.name().to_string())
                })
                .unwrap_or_else(|| "unknown".to_owned());
            println!(
                "First frame: {} ms, caps={caps}, memory={memory}",
                started.elapsed().as_millis()
            );
            gst::PadProbeReturn::Remove
        })
        .ok_or_else(|| io::Error::other("failed to install first-frame probe"))?;

    let bus = pipeline
        .bus()
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no bus"))?;
    let message_types = [gst::MessageType::Eos, gst::MessageType::Error];
    let mut messages = bus.stream_filtered(&message_types);
    if let Err(error) = pipeline.set_state(gst::State::Playing) {
        let _ = pipeline.set_state(gst::State::Null);
        return Err(error.into());
    }
    println!("Preview running; press Ctrl-C to stop.");

    tokio::pin!(session_closed);
    tokio::pin!(interrupt);
    let outcome: Result<bool> = tokio::select! {
        signal = &mut interrupt => {
            match signal {
                Ok(()) => {
                    println!("Ctrl-C received; stopping preview.");
                    Ok(false)
                }
                Err(error) => Err(error.into()),
            }
        }
        _ = &mut session_closed => {
            println!("Portal session closed; stopping preview.");
            Ok(true)
        }
        message = messages.next() => match message {
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
                    )).into())
                }
                _ => unreachable!(),
            },
            None => Err(io::Error::other("GStreamer bus closed").into()),
        },
    };

    let stop_result = pipeline.set_state(gst::State::Null);
    let portal_closed = outcome?;
    stop_result?;
    Ok(portal_closed)
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
}
