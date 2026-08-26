use std::{
    convert::Infallible,
    fs::File,
    io::{self, Read},
    sync::{Arc, Mutex, MutexGuard},
};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::stream;
use tokio::{
    net::TcpListener,
    sync::{broadcast, oneshot, watch},
};

const VIEWER_HTML: &str = include_str!("viewer.html");
const MAX_BOX_BYTES: usize = 16 * 1024 * 1024;
const MAX_CACHED_GOP_BYTES: usize = 64 * 1024 * 1024;
const MAX_VIEWERS: usize = 8;

#[derive(Clone)]
pub(crate) struct Host {
    inner: Arc<Mutex<HostState>>,
}

struct HostState {
    token: String,
    media: Option<Arc<MediaHub>>,
}

#[derive(Clone)]
pub(crate) struct MediaSession {
    hub: Arc<MediaHub>,
}

struct MediaHub {
    inner: Mutex<HubState>,
    viewers: watch::Sender<usize>,
    closed: watch::Sender<bool>,
}

struct HubState {
    sender: Option<broadcast::Sender<Bytes>>,
    mime: Option<String>,
    mux: MuxStream,
    init: Option<Bytes>,
    gop: Vec<Bytes>,
}

struct Subscription {
    mime: String,
    replay: std::vec::IntoIter<Bytes>,
    receiver: broadcast::Receiver<Bytes>,
    closed: watch::Receiver<bool>,
    _viewer: ViewerGuard,
}

struct ViewerGuard {
    viewers: watch::Sender<usize>,
}

enum Access {
    Invalid,
    Waiting,
    Sharing(Arc<MediaHub>),
}

#[derive(Default)]
struct MuxStream {
    pending: Vec<u8>,
    ftyp: Option<Vec<u8>>,
    video_track: Option<u32>,
    moof: Option<(Vec<u8>, bool)>,
}

#[derive(Debug, PartialEq)]
enum MuxUnit {
    Init(Bytes),
    Fragment { bytes: Bytes, keyframe: bool },
}

type BoxParts<'a> = ([u8; 4], &'a [u8], &'a [u8]);

impl Host {
    pub(crate) fn new() -> io::Result<Self> {
        Ok(Self {
            inner: Arc::new(Mutex::new(HostState {
                token: share_token()?,
                media: None,
            })),
        })
    }

    pub(crate) fn path(&self) -> io::Result<String> {
        Ok(format!("/s/{}", lock(&self.inner)?.token))
    }

    pub(crate) fn start(&self) -> io::Result<MediaSession> {
        let hub = Arc::new(MediaHub::new());
        let mut state = lock(&self.inner)?;
        if state.media.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a share is already active",
            ));
        }
        state.media = Some(Arc::clone(&hub));
        Ok(MediaSession { hub })
    }

    pub(crate) fn stop(&self, session: &MediaSession) -> io::Result<()> {
        let mut state = lock(&self.inner)?;
        if !state
            .media
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, &session.hub))
        {
            return Ok(());
        }
        state.media = None;
        session.hub.close()
    }

    fn access(&self, candidate: &str) -> io::Result<Access> {
        let state = lock(&self.inner)?;
        if !token_matches(&state.token, candidate) {
            return Ok(Access::Invalid);
        }
        Ok(match &state.media {
            Some(media) => Access::Sharing(Arc::clone(media)),
            None => Access::Waiting,
        })
    }
}

impl MediaSession {
    pub(crate) fn set_mime(&self, mime: String) -> io::Result<()> {
        lock(&self.hub.inner)?.mime = Some(mime);
        Ok(())
    }

    pub(crate) fn publish(&self, data: &[u8]) -> io::Result<bool> {
        let mut state = lock(&self.hub.inner)?;
        let units = state.mux.push(data)?;
        let mut published_fragment = false;
        for unit in units {
            match unit {
                MuxUnit::Init(bytes) => state.init = Some(bytes),
                MuxUnit::Fragment { bytes, keyframe } => {
                    published_fragment = true;
                    if keyframe {
                        state.gop.clear();
                    }
                    if keyframe || !state.gop.is_empty() {
                        let cached = state.gop.iter().map(Bytes::len).sum::<usize>();
                        if cached.saturating_add(bytes.len()) > MAX_CACHED_GOP_BYTES {
                            state.gop.clear();
                        } else {
                            state.gop.push(bytes.clone());
                        }
                    }
                    if let Some(sender) = &state.sender {
                        let _ = sender.send(bytes);
                    }
                }
            }
        }
        Ok(published_fragment)
    }

    pub(crate) fn viewer_count(&self) -> watch::Receiver<usize> {
        self.hub.viewers.subscribe()
    }
}

impl MediaHub {
    fn new() -> Self {
        let (sender, _) = broadcast::channel(32);
        let (viewers, _) = watch::channel(0);
        let (closed, _) = watch::channel(false);
        Self {
            inner: Mutex::new(HubState {
                sender: Some(sender),
                mime: None,
                mux: MuxStream::default(),
                init: None,
                gop: Vec::new(),
            }),
            viewers,
            closed,
        }
    }

    fn subscribe(&self) -> io::Result<Option<Subscription>> {
        let state = lock(&self.inner)?;
        if *self.viewers.borrow() >= MAX_VIEWERS {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Viewer limit reached",
            ));
        }
        let Some(sender) = &state.sender else {
            return Ok(None);
        };
        let (Some(mime), Some(init), false) = (&state.mime, &state.init, state.gop.is_empty())
        else {
            return Ok(None);
        };
        let receiver = sender.subscribe();
        let mut replay = Vec::with_capacity(state.gop.len() + 1);
        replay.push(init.clone());
        replay.extend(state.gop.iter().cloned());
        Ok(Some(Subscription {
            mime: mime.clone(),
            replay: replay.into_iter(),
            receiver,
            closed: self.closed.subscribe(),
            _viewer: ViewerGuard::new(&self.viewers),
        }))
    }

    fn close(&self) -> io::Result<()> {
        self.closed.send_replace(true);
        let mut state = lock(&self.inner)?;
        state.sender.take();
        state.init = None;
        state.gop.clear();
        Ok(())
    }
}

impl ViewerGuard {
    fn new(viewers: &watch::Sender<usize>) -> Self {
        viewers.send_modify(|count| *count += 1);
        Self {
            viewers: viewers.clone(),
        }
    }
}

impl Drop for ViewerGuard {
    fn drop(&mut self) {
        self.viewers
            .send_modify(|count| *count = count.saturating_sub(1));
    }
}

impl MuxStream {
    fn push(&mut self, data: &[u8]) -> io::Result<Vec<MuxUnit>> {
        self.pending.extend_from_slice(data);
        let mut units = Vec::new();
        while let Some(size) = top_level_box_size(&self.pending)? {
            if self.pending.len() < size {
                break;
            }
            let rest = self.pending.split_off(size);
            let frame = std::mem::replace(&mut self.pending, rest);
            self.accept(frame, &mut units)?;
        }
        Ok(units)
    }

    fn accept(&mut self, frame: Vec<u8>, units: &mut Vec<MuxUnit>) -> io::Result<()> {
        let kind = frame
            .get(4..8)
            .ok_or_else(|| invalid_media("truncated ISO-BMFF box header"))?;
        match kind {
            b"ftyp" if self.ftyp.is_none() && self.video_track.is_none() => {
                self.ftyp = Some(frame);
            }
            b"moov" if self.ftyp.is_some() && self.video_track.is_none() => {
                self.video_track = Some(video_track(&frame)?);
                let mut init = self.ftyp.take().expect("ftyp checked above");
                init.extend_from_slice(&frame);
                units.push(MuxUnit::Init(Bytes::from(init)));
            }
            b"moof" if self.video_track.is_some() && self.moof.is_none() => {
                let keyframe = video_fragment_starts_with_keyframe(
                    &frame,
                    self.video_track.expect("video track checked above"),
                )?;
                self.moof = Some((frame, keyframe));
            }
            b"mdat" if self.moof.is_some() => {
                let (mut moof, keyframe) = self.moof.take().expect("moof checked above");
                moof.extend_from_slice(&frame);
                units.push(MuxUnit::Fragment {
                    bytes: Bytes::from(moof),
                    keyframe,
                });
            }
            _ => return Err(invalid_media("unexpected ISO-BMFF box sequence")),
        }
        Ok(())
    }
}

pub(crate) async fn serve(
    listener: TcpListener,
    host: Host,
    shutdown: oneshot::Receiver<()>,
) -> io::Result<()> {
    axum::serve(
        listener,
        Router::new()
            .route("/s/{token}", get(viewer_page))
            .route(
                "/s/{token}/stream",
                get(media_stream).head(|| async { StatusCode::METHOD_NOT_ALLOWED }),
            )
            .with_state(host),
    )
    .with_graceful_shutdown(async {
        let _ = shutdown.await;
    })
    .await
}

async fn viewer_page(Path(token): Path<String>, State(host): State<Host>) -> Response {
    match host.access(&token) {
        Ok(Access::Invalid) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        Ok(Access::Waiting | Access::Sharing(_)) => Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-store")
            .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
            .header("referrer-policy", "no-referrer")
            .header(
                "content-security-policy",
                "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; media-src 'self' blob:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
            )
            .body(Body::from(VIEWER_HTML))
            .expect("static Viewer headers are valid"),
    }
}

async fn media_stream(Path(token): Path<String>, State(host): State<Host>) -> Response {
    let media = match host.access(&token) {
        Ok(Access::Sharing(media)) => media,
        Ok(Access::Waiting) => return waiting(),
        Ok(Access::Invalid) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let subscription = match media.subscribe() {
        Ok(Some(subscription)) => subscription,
        Ok(None) => return waiting(),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            return StatusCode::TOO_MANY_REQUESTS.into_response();
        }
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let mime = subscription.mime.clone();
    let body = Body::from_stream(stream::unfold(
        subscription,
        |mut subscription| async move {
            if *subscription.closed.borrow() {
                return None;
            }
            if let Some(chunk) = subscription.replay.next() {
                return Some((Ok::<Bytes, Infallible>(chunk), subscription));
            }
            tokio::select! {
                biased;
                _ = subscription.closed.changed() => None,
                result = subscription.receiver.recv() => match result {
                    Ok(chunk) => Some((Ok(chunk), subscription)),
                    Err(error) => {
                        eprintln!("Viewer stream closed: {error}");
                        None
                    }
                }
            }
        },
    ));
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(body)
        .expect("generated media headers are valid")
}

fn waiting() -> Response {
    Response::builder()
        .status(StatusCode::TOO_EARLY)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from("Share has not started"))
        .expect("static waiting response is valid")
}

fn share_token() -> io::Result<String> {
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let mut token = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        token.push(HEX[(byte >> 4) as usize] as char);
        token.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(token)
}

fn token_matches(expected: &str, candidate: &str) -> bool {
    expected.len() == candidate.len()
        && !expected.is_empty()
        && expected
            .bytes()
            .zip(candidate.bytes())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

fn top_level_box_size(data: &[u8]) -> io::Result<Option<usize>> {
    if data.len() < 8 {
        return Ok(None);
    }
    let short = u32::from_be_bytes(data[..4].try_into().expect("length checked"));
    let (size, header) = if short == 1 {
        if data.len() < 16 {
            return Ok(None);
        }
        (
            usize::try_from(u64::from_be_bytes(
                data[8..16].try_into().expect("length checked"),
            ))
            .map_err(|_| invalid_media("ISO-BMFF box size exceeds this platform"))?,
            16,
        )
    } else {
        (short as usize, 8)
    };
    if size == 0 || size < header || size > MAX_BOX_BYTES {
        return Err(invalid_media("invalid ISO-BMFF box size"));
    }
    Ok(Some(size))
}

fn video_track(moov: &[u8]) -> io::Result<u32> {
    let (kind, mut children, rest) =
        next_box(moov)?.ok_or_else(|| invalid_media("missing moov box"))?;
    if kind != *b"moov" || !rest.is_empty() {
        return Err(invalid_media("invalid moov box"));
    }
    let mut found = None;
    while let Some((kind, payload, rest)) = next_box(children)? {
        children = rest;
        if kind != *b"trak" {
            continue;
        }
        let Some(mdia) = find_child(payload, b"mdia")? else {
            continue;
        };
        let Some(handler) = find_child(mdia, b"hdlr")? else {
            continue;
        };
        if handler.get(8..12) != Some(b"vide".as_slice()) {
            continue;
        }
        let track = find_child(payload, b"tkhd")?
            .ok_or_else(|| invalid_media("video track has no tkhd box"))?;
        let offset = match track.first() {
            Some(0) => 12,
            Some(1) => 20,
            _ => return Err(invalid_media("unsupported tkhd version")),
        };
        let id = read_u32(track, offset, "truncated tkhd track ID")?;
        if id == 0 || found.replace(id).is_some() {
            return Err(invalid_media("invalid number of video tracks"));
        }
    }
    found.ok_or_else(|| invalid_media("moov has no video track"))
}

fn video_fragment_starts_with_keyframe(moof: &[u8], video_track: u32) -> io::Result<bool> {
    let (kind, mut children, rest) =
        next_box(moof)?.ok_or_else(|| invalid_media("missing moof box"))?;
    if kind != *b"moof" || !rest.is_empty() {
        return Err(invalid_media("invalid moof box"));
    }
    while let Some((kind, payload, rest)) = next_box(children)? {
        children = rest;
        if kind != *b"traf" {
            continue;
        }
        let Some(tfhd) = find_child(payload, b"tfhd")? else {
            continue;
        };
        if read_u32(tfhd, 4, "truncated tfhd track ID")? != video_track {
            continue;
        }
        let tfhd_flags = read_u32(tfhd, 0, "truncated tfhd flags")? & 0x00ff_ffff;
        let mut tfhd_offset = 8;
        if tfhd_flags & 0x000001 != 0 {
            tfhd_offset += 8;
        }
        for flag in [0x000002, 0x000008, 0x000010] {
            if tfhd_flags & flag != 0 {
                tfhd_offset += 4;
            }
        }
        let default_sample_flags = if tfhd_flags & 0x000020 != 0 {
            Some(read_u32(
                tfhd,
                tfhd_offset,
                "truncated default sample flags",
            )?)
        } else {
            None
        };
        let trun = find_child(payload, b"trun")?
            .ok_or_else(|| invalid_media("video traf has no trun box"))?;
        let flags = read_u32(trun, 0, "truncated trun flags")? & 0x00ff_ffff;
        if read_u32(trun, 4, "truncated trun sample count")? == 0 {
            return Err(invalid_media("video trun has no samples"));
        }
        let mut offset = 8;
        if flags & 0x000001 != 0 {
            offset += 4;
        }
        let sample_flags = if flags & 0x000004 != 0 {
            read_u32(trun, offset, "truncated first-sample flags")?
        } else {
            if flags & 0x000100 != 0 {
                offset += 4;
            }
            if flags & 0x000200 != 0 {
                offset += 4;
            }
            if flags & 0x000400 != 0 {
                read_u32(trun, offset, "truncated video sample flags")?
            } else {
                default_sample_flags
                    .ok_or_else(|| invalid_media("video fragment has no sample flags"))?
            }
        };
        return Ok(sample_flags & 0x0001_0000 == 0);
    }
    Ok(false)
}

fn find_child<'a>(mut data: &'a [u8], wanted: &[u8; 4]) -> io::Result<Option<&'a [u8]>> {
    while let Some((kind, payload, rest)) = next_box(data)? {
        if kind == *wanted {
            return Ok(Some(payload));
        }
        data = rest;
    }
    Ok(None)
}

fn next_box(data: &[u8]) -> io::Result<Option<BoxParts<'_>>> {
    if data.is_empty() {
        return Ok(None);
    }
    if data.len() < 8 {
        return Err(invalid_media("truncated nested ISO-BMFF box"));
    }
    let short = u32::from_be_bytes(data[..4].try_into().expect("length checked"));
    let (size, header) = match short {
        0 => (data.len(), 8),
        1 => {
            if data.len() < 16 {
                return Err(invalid_media("truncated extended ISO-BMFF box"));
            }
            (
                usize::try_from(u64::from_be_bytes(
                    data[8..16].try_into().expect("length checked"),
                ))
                .map_err(|_| invalid_media("nested box exceeds this platform"))?,
                16,
            )
        }
        size => (size as usize, 8),
    };
    if size < header || size > data.len() {
        return Err(invalid_media("invalid nested ISO-BMFF box size"));
    }
    Ok(Some((
        data[4..8].try_into().expect("length checked"),
        &data[header..size],
        &data[size..],
    )))
}

fn read_u32(data: &[u8], offset: usize, error: &'static str) -> io::Result<u32> {
    Ok(u32::from_be_bytes(
        data.get(offset..offset + 4)
            .ok_or_else(|| invalid_media(error))?
            .try_into()
            .expect("length checked"),
    ))
}

fn invalid_media(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn lock<T>(mutex: &Mutex<T>) -> io::Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| io::Error::other("shared media state is poisoned"))
}

#[cfg(test)]
#[path = "web_tests.rs"]
mod tests;
