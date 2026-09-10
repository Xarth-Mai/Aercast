use crate::Result;
use ashpd::desktop::{
    PersistMode, Session,
    screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
};
use std::io;

pub(crate) async fn open() -> Result<(Screencast, Session<Screencast>, SelectSourcesOptions)> {
    let portal = Screencast::new().await?;
    let available_sources = portal.available_source_types().await?;
    let available_cursors = portal.available_cursor_modes().await?;
    println!("Portal version: {}", portal.version());
    println!("Available source types: {available_sources:?}");
    println!("Available cursor modes: {available_cursors:?}");

    let sources = available_sources & (SourceType::Monitor | SourceType::Window);
    if sources.is_empty() {
        return Err(io::Error::other("portal offers neither monitor nor window capture").into());
    }
    let cursor = cursor_mode(
        available_cursors.contains(CursorMode::Embedded),
        available_cursors.contains(CursorMode::Hidden),
    )
    .ok_or_else(|| io::Error::other("portal offers no supported cursor mode"))?;

    let options = SelectSourcesOptions::default()
        .set_sources(sources)
        .set_multiple(false)
        .set_cursor_mode(cursor)
        .set_persist_mode(PersistMode::DoNot);
    let session = portal.create_session(Default::default()).await?;
    Ok((portal, session, options))
}

pub(crate) async fn select(
    portal: &Screencast,
    session: &Session<Screencast>,
    options: SelectSourcesOptions,
) -> ashpd::Result<(u32, &'static str)> {
    portal.select_sources(session, options).await?.response()?;

    let response = portal
        .start(session, None, Default::default())
        .await?
        .response()?;
    let stream = response
        .streams()
        .first()
        .ok_or_else(|| io::Error::other("portal returned no selected stream"))?;
    let source = approved_source(stream.source_type());
    println!(
        "Selected source: {source}; PipeWire node {}",
        stream.pipe_wire_node_id(),
    );
    Ok((stream.pipe_wire_node_id(), source))
}

fn cursor_mode(embedded: bool, hidden: bool) -> Option<CursorMode> {
    embedded
        .then_some(CursorMode::Embedded)
        .or_else(|| hidden.then_some(CursorMode::Hidden))
}

fn approved_source(source_type: Option<SourceType>) -> &'static str {
    match source_type {
        Some(SourceType::Monitor) => "Screen",
        Some(SourceType::Window) => "Window",
        _ => "Selected source",
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
    fn approved_source_uses_only_portal_display_metadata() {
        assert_eq!(approved_source(Some(SourceType::Monitor)), "Screen");
        assert_eq!(approved_source(Some(SourceType::Window)), "Window");
        assert_eq!(approved_source(None), "Selected source");
    }
}
