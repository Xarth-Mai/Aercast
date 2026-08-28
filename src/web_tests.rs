use super::*;
use futures_util::StreamExt as _;
use gst::prelude::*;
use std::net::{Ipv4Addr, SocketAddrV4};

const VIDEO_TRACK: u32 = 37;

fn viewers(host: &Host) -> Arc<ViewerGeneration> {
    lock(&host.inner).unwrap().viewers.clone()
}

fn subscribe(session: &MediaSession, generation: Arc<ViewerGeneration>, id: u8) -> Subscription {
    session
        .hub
        .subscribe(generation, [id; 16], IpAddr::V4(Ipv4Addr::LOCALHOST))
        .unwrap()
        .unwrap()
}

fn viewer_headers(id: u8) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        VIEWER_ID_HEADER,
        format!("{id:02x}").repeat(16).parse().unwrap(),
    );
    headers
}

fn peer(ip: Ipv4Addr) -> ConnectInfo<SocketAddr> {
    ConnectInfo(SocketAddrV4::new(ip, 12345).into())
}

async fn stream(host: &Host, token: &str, id: u8, ip: Ipv4Addr) -> Response {
    request(host, token, viewer_headers(id), ip).await
}

async fn request(host: &Host, token: &str, headers: HeaderMap, ip: Ipv4Addr) -> Response {
    media_stream(
        Path(token.to_owned()),
        State(host.clone()),
        peer(ip),
        headers,
    )
    .await
}

async fn telemetry_request(
    host: &Host,
    token: &str,
    headers: HeaderMap,
    body: impl Into<Body>,
) -> Response {
    let mut request = Request::new(body.into());
    *request.headers_mut() = headers;
    viewer_telemetry(Path(token.to_owned()), State(host.clone()), request).await
}

fn online(host: &Host) -> usize {
    host.viewers()
        .unwrap()
        .iter()
        .filter(|viewer| viewer.online())
        .count()
}

fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(payload.len() + 8);
    bytes.extend_from_slice(&u32::try_from(payload.len() + 8).unwrap().to_be_bytes());
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(payload);
    bytes
}

fn track(id: u32, handler: &[u8; 4]) -> Vec<u8> {
    let mut tkhd = vec![0; 16];
    tkhd[12..16].copy_from_slice(&id.to_be_bytes());
    let mut hdlr = vec![0; 12];
    hdlr[8..12].copy_from_slice(handler);
    let mdia = mp4_box(b"mdia", &mp4_box(b"hdlr", &hdlr));
    let mut trak = mp4_box(b"tkhd", &tkhd);
    trak.extend_from_slice(&mdia);
    mp4_box(b"trak", &trak)
}

fn init() -> Vec<u8> {
    let mut tracks = track(11, b"soun");
    tracks.extend_from_slice(&track(VIDEO_TRACK, b"vide"));
    let mut init = mp4_box(b"ftyp", b"test");
    init.extend_from_slice(&mp4_box(b"moov", &tracks));
    init
}

fn fragment(track: u32, sample_flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut tfhd = vec![0; 8];
    tfhd[4..8].copy_from_slice(&track.to_be_bytes());
    let mut trun = vec![0; 24];
    trun[..4].copy_from_slice(&0x701_u32.to_be_bytes());
    trun[4..8].copy_from_slice(&1_u32.to_be_bytes());
    trun[12..16].copy_from_slice(&100_u32.to_be_bytes());
    trun[16..20].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    trun[20..24].copy_from_slice(&sample_flags.to_be_bytes());
    let mut traf = mp4_box(b"tfhd", &tfhd);
    traf.extend_from_slice(&mp4_box(b"trun", &trun));
    let mut fragment = mp4_box(b"moof", &mp4_box(b"traf", &traf));
    fragment.extend_from_slice(&mp4_box(b"mdat", payload));
    fragment
}

#[test]
fn mux_frames_arbitrary_chunks_and_marks_only_video_keyframes() {
    let mut input = init();
    input.extend_from_slice(&fragment(11, 0xc0, b"audio"));
    input.extend_from_slice(&fragment(VIDEO_TRACK, 0x40, b"key-moof-payload"));
    input.extend_from_slice(&fragment(VIDEO_TRACK, 0x100c0, b"delta"));
    input.extend_from_slice(&fragment(VIDEO_TRACK, 0x40, b"next-key"));

    let whole = MuxStream::default().push(&input).unwrap();
    let mut split = MuxStream::default();
    let mut bytewise = Vec::new();
    for byte in &input {
        bytewise.extend(split.push(&[*byte]).unwrap());
    }
    assert_eq!(whole, bytewise);
    assert_eq!(whole.len(), 5);
    assert!(matches!(whole[0], MuxUnit::Init(_)));
    assert!(matches!(
        whole[1],
        MuxUnit::Fragment {
            keyframe: false,
            ..
        }
    ));
    assert!(matches!(whole[2], MuxUnit::Fragment { keyframe: true, .. }));
    assert!(matches!(
        whole[3],
        MuxUnit::Fragment {
            keyframe: false,
            ..
        }
    ));
    assert!(matches!(whole[4], MuxUnit::Fragment { keyframe: true, .. }));
    assert!(
        MuxStream::default()
            .push(&[0, 0, 0, 0, b'f', b'r', b'e', b'e'])
            .is_err()
    );
}

#[test]
fn mux_uses_tfhd_default_sample_flags() {
    let fragment = |sample_flags: u32| {
        let mut tfhd = vec![0; 12];
        tfhd[..4].copy_from_slice(&0x20_u32.to_be_bytes());
        tfhd[4..8].copy_from_slice(&VIDEO_TRACK.to_be_bytes());
        tfhd[8..12].copy_from_slice(&sample_flags.to_be_bytes());
        let mut trun = vec![0; 20];
        trun[..4].copy_from_slice(&0x301_u32.to_be_bytes());
        trun[4..8].copy_from_slice(&1_u32.to_be_bytes());
        trun[12..16].copy_from_slice(&100_u32.to_be_bytes());
        trun[16..20].copy_from_slice(&1_u32.to_be_bytes());
        let mut traf = mp4_box(b"tfhd", &tfhd);
        traf.extend_from_slice(&mp4_box(b"trun", &trun));
        let mut fragment = mp4_box(b"moof", &mp4_box(b"traf", &traf));
        fragment.extend_from_slice(&mp4_box(b"mdat", b"x"));
        fragment
    };
    let mut input = init();
    input.extend_from_slice(&fragment(0x40));
    input.extend_from_slice(&fragment(0x100c0));
    let units = MuxStream::default().push(&input).unwrap();
    assert!(matches!(units[1], MuxUnit::Fragment { keyframe: true, .. }));
    assert!(matches!(
        units[2],
        MuxUnit::Fragment {
            keyframe: false,
            ..
        }
    ));
}

#[test]
fn installed_mp4mux_output_builds_a_decodable_replay_boundary() {
    gst::init().unwrap();
    let pipeline = gst::parse::launch(
        "mp4mux name=mux fragment-duration=100 ! appsink name=sink sync=false wait-on-eos=false \
         audiotestsrc num-buffers=100 wave=silence ! audio/x-raw,rate=48000,channels=2 ! avenc_aac ! aacparse ! audio/mpeg,mpegversion=4,stream-format=raw ! queue ! mux.audio_0 \
         videotestsrc num-buffers=40 ! video/x-raw,width=160,height=90,framerate=30/1 ! x264enc tune=zerolatency speed-preset=ultrafast key-int-max=30 ! h264parse ! video/x-h264,stream-format=avc,alignment=au ! queue ! mux.video_0",
    )
    .unwrap()
    .downcast::<gst::Pipeline>()
    .unwrap();
    let sink = pipeline
        .by_name("sink")
        .unwrap()
        .downcast::<gst_app::AppSink>()
        .unwrap();
    let host = Host::new().unwrap();
    let viewers = viewers(&host);
    let session = host.start().unwrap();
    session.set_mime("video/mp4".to_owned()).unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();

    let replay = loop {
        let sample = sink.pull_sample().unwrap();
        let buffer = sample.buffer().unwrap().map_readable().unwrap();
        session.publish(buffer.as_slice()).unwrap();
        if let Some(subscription) = session
            .hub
            .subscribe(viewers.clone(), [1; 16], IpAddr::V4(Ipv4Addr::LOCALHOST))
            .unwrap()
        {
            break subscription.replay.collect::<Vec<_>>();
        }
    };
    pipeline.set_state(gst::State::Null).unwrap();
    assert_eq!(replay[0].get(4..8), Some(b"ftyp".as_slice()));
    assert_eq!(replay[1].get(4..8), Some(b"moof".as_slice()));
    assert!(replay[1].windows(4).any(|bytes| bytes == b"mdat"));
}

#[test]
fn three_subscribers_get_one_atomic_replay_then_live_media() {
    let host = Host::new().unwrap();
    let generation = viewers(&host);
    let session = host.start().unwrap();
    session
        .set_mime("video/mp4; codecs=\"test\"".to_owned())
        .unwrap();
    let init = init();
    let key = fragment(VIDEO_TRACK, 0x40, b"key");
    let delta = fragment(VIDEO_TRACK, 0x100c0, b"delta-before-join");
    session.publish(&init).unwrap();
    session.publish(&key).unwrap();
    session.publish(&delta).unwrap();
    let mut viewers: Vec<_> = (0..3)
        .map(|id| subscribe(&session, generation.clone(), id))
        .collect();
    assert_eq!(online(&host), 3);
    let extras: Vec<_> = (3..MAX_VIEWERS)
        .map(|id| subscribe(&session, generation.clone(), id as u8))
        .collect();
    assert!(matches!(
        session.hub.subscribe(
            generation,
            [MAX_VIEWERS as u8; 16],
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        ),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock
    ));
    drop(extras);
    assert_eq!(online(&host), 3);
    assert!(viewers.iter_mut().all(|viewer| {
        viewer.replay.by_ref().collect::<Vec<_>>()
            == [
                Bytes::from(init.clone()),
                Bytes::from(key.clone()),
                Bytes::from(delta.clone()),
            ]
    }));

    let live = fragment(VIDEO_TRACK, 0x100c0, b"live-once");
    session.publish(&live).unwrap();
    assert!(
        viewers
            .iter_mut()
            .all(|viewer| { viewer.receiver.try_recv().unwrap().as_ref() == live.as_slice() })
    );
    viewers.pop();
    assert_eq!(online(&host), 2);
    viewers.pop();
    assert_eq!(online(&host), 1);
    viewers.pop();
    assert_eq!(online(&host), 0);
}

#[test]
fn oversized_gop_waits_for_the_next_keyframe() {
    let host = Host::new().unwrap();
    let viewers = viewers(&host);
    let session = host.start().unwrap();
    session.set_mime("video/mp4".to_owned()).unwrap();
    session.publish(&init()).unwrap();
    session
        .publish(&fragment(VIDEO_TRACK, 0x40, b"key"))
        .unwrap();
    let shared_megabyte = Bytes::from(vec![0; 1024 * 1024]);
    session.hub.inner.lock().unwrap().gop =
        vec![shared_megabyte; MAX_CACHED_GOP_BYTES / (1024 * 1024)];

    session
        .publish(&fragment(VIDEO_TRACK, 0x100c0, b"overflow"))
        .unwrap();
    assert!(
        session
            .hub
            .subscribe(viewers.clone(), [1; 16], IpAddr::V4(Ipv4Addr::LOCALHOST))
            .unwrap()
            .is_none()
    );
    session
        .publish(&fragment(VIDEO_TRACK, 0x40, b"recovered"))
        .unwrap();
    assert!(
        session
            .hub
            .subscribe(viewers, [1; 16], IpAddr::V4(Ipv4Addr::LOCALHOST))
            .unwrap()
            .is_some()
    );
}

#[test]
fn lagged_viewer_does_not_harm_fast_viewers() {
    let host = Host::new().unwrap();
    let generation = viewers(&host);
    let session = host.start().unwrap();
    session
        .set_mime("video/mp4; codecs=\"test\"".to_owned())
        .unwrap();
    session.publish(&init()).unwrap();
    session
        .publish(&fragment(VIDEO_TRACK, 0x40, b"key"))
        .unwrap();
    let mut viewers: Vec<_> = (0..3)
        .map(|id| subscribe(&session, generation.clone(), id))
        .collect();
    for viewer in &mut viewers {
        viewer.replay.by_ref().for_each(drop);
    }
    let mut slow = viewers.pop().unwrap();

    for sequence in 0..40_u8 {
        let fragment = fragment(VIDEO_TRACK, 0x100c0, &[sequence]);
        session.publish(&fragment).unwrap();
        for viewer in &mut viewers {
            assert_eq!(viewer.receiver.try_recv().unwrap().as_ref(), fragment);
        }
    }
    assert!(matches!(
        slow.receiver.try_recv(),
        Err(broadcast::error::TryRecvError::Lagged(_))
    ));
    let final_fragment = fragment(VIDEO_TRACK, 0x100c0, b"final");
    session.publish(&final_fragment).unwrap();
    for viewer in &mut viewers {
        assert_eq!(viewer.receiver.try_recv().unwrap().as_ref(), final_fragment);
    }
}

#[test]
fn reconnect_merges_history_and_stale_disconnect_cannot_win() {
    let generation = Arc::new(ViewerGeneration::new());
    let id = [7; 16];
    let start = Instant::now();
    let (old_epoch, mut old_close) = generation
        .connect(id, "192.0.2.1".parse().unwrap(), start)
        .unwrap();
    let (new_epoch, _) = generation
        .connect(
            id,
            "192.0.2.2".parse().unwrap(),
            start + Duration::from_secs(3),
        )
        .unwrap();
    assert!(*old_close.borrow_and_update());
    assert_eq!(
        generation
            .snapshots()
            .unwrap()
            .iter()
            .filter(|viewer| viewer.online())
            .count(),
        1
    );
    let viewers = generation.snapshots().unwrap();
    assert_eq!(viewers.len(), 1);
    assert_eq!(viewers[0].ip, "192.0.2.2".parse::<IpAddr>().unwrap());
    assert_eq!(viewers[0].duration, Duration::from_secs(3));

    generation
        .disconnect(id, old_epoch, start + Duration::from_secs(4))
        .unwrap();
    assert!(generation.snapshots().unwrap()[0].online());
    generation
        .disconnect(id, new_epoch, start + Duration::from_secs(5))
        .unwrap();
    let viewer = generation.snapshots().unwrap().pop().unwrap();
    assert!(!viewer.online());
    assert_eq!(viewer.duration, Duration::from_secs(5));

    let (later_epoch, _) = generation
        .connect(
            id,
            "192.0.2.3".parse().unwrap(),
            start + Duration::from_secs(20),
        )
        .unwrap();
    generation
        .disconnect(id, later_epoch, start + Duration::from_secs(22))
        .unwrap();
    assert_eq!(
        generation.snapshots().unwrap()[0].duration,
        Duration::from_secs(7)
    );
}

#[test]
fn history_caps_at_100_and_evicts_only_the_oldest_offline_record() {
    let generation = Arc::new(ViewerGeneration::new());
    let start = Instant::now();
    for value in 0..MAX_VIEWER_RECORDS {
        let id = (value as u128).to_be_bytes();
        let now = start + Duration::from_secs(value as u64);
        let (epoch, _) = generation
            .connect(id, IpAddr::V4(Ipv4Addr::LOCALHOST), now)
            .unwrap();
        generation
            .disconnect(id, epoch, now + Duration::from_millis(1))
            .unwrap();
    }
    let peer: IpAddr = "203.0.113.9".parse().unwrap();
    generation
        .connect(
            (MAX_VIEWER_RECORDS as u128).to_be_bytes(),
            peer,
            start + Duration::from_secs(MAX_VIEWER_RECORDS as u64),
        )
        .unwrap();

    let viewers = generation.snapshots().unwrap();
    assert_eq!(viewers.len(), MAX_VIEWER_RECORDS);
    assert!(viewers[0].online());
    assert_eq!(viewers[0].key, 101);
    assert_eq!(viewers[0].ip, peer);
    assert!(!viewers.iter().any(|viewer| viewer.key == 1));
    assert!(viewers.iter().any(|viewer| viewer.key == 2));
}

#[tokio::test]
async fn stream_identity_is_canonical_and_uses_reverse_proxy_ip() {
    let host = Host::new().unwrap();
    let token = host.path().unwrap().trim_start_matches("/s/").to_owned();
    assert_eq!(
        request(&host, "invalid", HeaderMap::new(), Ipv4Addr::LOCALHOST,)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&host, &token, HeaderMap::new(), Ipv4Addr::LOCALHOST,)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    for invalid in ["AA".repeat(16), "0".repeat(31)] {
        let mut headers = HeaderMap::new();
        headers.insert(VIEWER_ID_HEADER, invalid.parse().unwrap());
        assert_eq!(
            request(&host, &token, headers, Ipv4Addr::LOCALHOST)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let mut duplicate = viewer_headers(1);
    duplicate.append(VIEWER_ID_HEADER, "11".repeat(16).parse().unwrap());
    assert_eq!(
        request(&host, &token, duplicate, Ipv4Addr::LOCALHOST)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let mut opaque = HeaderMap::new();
    opaque.insert(
        VIEWER_ID_HEADER,
        axum::http::HeaderValue::from_bytes(&[0xff; 32]).unwrap(),
    );
    assert_eq!(
        request(&host, &token, opaque, Ipv4Addr::LOCALHOST)
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );

    let session = host.start().unwrap();
    session.set_mime("video/mp4".to_owned()).unwrap();
    session.publish(&init()).unwrap();
    session
        .publish(&fragment(VIDEO_TRACK, 0x40, b"peer"))
        .unwrap();
    let actual = Ipv4Addr::new(203, 0, 113, 7);
    let mut headers = viewer_headers(1);
    let forwarded = Ipv4Addr::new(198, 51, 100, 8);
    headers.insert(
        "x-forwarded-for",
        format!("{forwarded}, 192.0.2.1").parse().unwrap(),
    );
    let response = request(&host, &token, headers, actual).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(host.viewers().unwrap()[0].ip, IpAddr::V4(forwarded));

    let mut headers = viewer_headers(2);
    let real = Ipv4Addr::new(192, 0, 2, 8);
    headers.insert("x-real-ip", real.to_string().parse().unwrap());
    headers.insert("x-forwarded-for", forwarded.to_string().parse().unwrap());
    assert_eq!(
        request(&host, &token, headers, actual).await.status(),
        StatusCode::OK
    );
    assert!(
        host.viewers()
            .unwrap()
            .iter()
            .any(|viewer| viewer.ip == real)
    );
}

#[tokio::test]
async fn token_routes_wait_between_isolated_media_sessions() {
    let host = Host::new().unwrap();
    let token = host.path().unwrap().trim_start_matches("/s/").to_owned();
    assert_eq!(token.len(), 64);
    assert!(
        token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    assert!(token_matches(&token, &token));
    let mut other = token.clone();
    other.replace_range(..1, if token.starts_with('0') { "1" } else { "0" });
    assert!(!token_matches(&token, &other));
    assert_eq!(
        viewer_page(Path("invalid".to_owned()), State(host.clone()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        stream(&host, "invalid", 1, Ipv4Addr::LOCALHOST)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let page = viewer_page(Path(token.clone()), State(host.clone())).await;
    assert_eq!(page.status(), StatusCode::OK);
    assert_eq!(page.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(page.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(page.headers()["referrer-policy"], "no-referrer");
    assert!(
        page.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("img-src 'self'")
    );
    let html = axum::body::to_bytes(page.into_body(), 100_000)
        .await
        .unwrap();
    assert!(
        !html
            .windows(token.len())
            .any(|window| window == token.as_bytes())
    );
    for required in [
        b"href=\"/assets/aercast-icon.png\"".as_slice(),
        b"html, body, video { width: 100%; height: 100%; }".as_slice(),
        b"<video playsinline></video>".as_slice(),
        b"const MediaSourceClass = self.ManagedMediaSource ?? self.MediaSource;".as_slice(),
        b"const media = new MediaSourceClass();".as_slice(),
        b"if (MediaSourceClass === self.ManagedMediaSource) video.disableRemotePlayback = true;"
            .as_slice(),
        b"if (!mime || !MediaSourceClass.isTypeSupported(mime))".as_slice(),
        b"localStorage.getItem(MUTED_KEY) === \"true\"".as_slice(),
        b"video.addEventListener(\"volumechange\"".as_slice(),
        b"if (policyMuted === video.muted)".as_slice(),
        b"localStorage.setItem(MUTED_KEY, String(preferredMuted))".as_slice(),
        b"await video.play()".as_slice(),
        b"else if (!automaticPlay && !video.paused)".as_slice(),
        b"if (!video.seeking) seekTo(Math.max(start, end - 0.15))".as_slice(),
        b"else if (lag > 0.35)".as_slice(),
        b"video.playbackRate = 1.0 + Math.min(0.15, lag * 0.08)".as_slice(),
        b"seekTo(Math.max(start, end - 0.15))".as_slice(),
        b"video.controls = false;".as_slice(),
        b"positioned = true;\n          setMutedByPolicy(preferredMuted);\n          video.controls = true;\n          void playAutomatically(attempt);".as_slice(),
        b"setMutedByPolicy(true)".as_slice(),
        b"attempt.controller.signal.aborted || currentAttempt !== attempt".as_slice(),
        b"if (MediaSourceClass) {\n    void connect(beginAttempt());".as_slice(),
        b"showError(new Error(\"This browser does not support Media Source playback.\"))"
            .as_slice(),
        b"video.addEventListener(\"play\"".as_slice(),
        b"video.addEventListener(\"seeking\"".as_slice(),
        b"seekTo(Math.max(start, end - 0.1))".as_slice(),
        b"console.log(\"Viewer media type:\", mime)".as_slice(),
        b"console.error(\"Unsupported Viewer media type:\", mime ?? \"missing\")".as_slice(),
        b"crypto.getRandomValues(new Uint8Array(16))".as_slice(),
        b"localStorage.getItem(\"aercast-viewer-id\")".as_slice(),
        b"new BroadcastChannel(\"aercast-viewer-session\")".as_slice(),
        b"viewerId = tabId".as_slice(),
        b"data?.type === \"claim\" && data.tabId !== tabId".as_slice(),
        b"Playback transferred to another tab; press Play to resume here.".as_slice(),
        b"\"Aercast-Viewer-ID\": viewerId".as_slice(),
        b"response.status === 409".as_slice(),
        b"blockedByHost = true".as_slice(),
        b"fetch(`${location.pathname}/telemetry`".as_slice(),
        b"method: \"POST\"".as_slice(),
        b"await withAbort(delay(2000), signal)".as_slice(),
        b"buffer.buffered.end(buffer.buffered.length - 1) - video.currentTime".as_slice(),
        b"performance.now() - started".as_slice(),
    ] {
        assert!(
            html.windows(required.len())
                .any(|window| window == required)
        );
    }
    for removed in [
        b"border-radius: 10px".as_slice(),
        b"status.textContent = mime".as_slice(),
        b"document.body.dataset.mime".as_slice(),
        b"button.textContent".as_slice(),
        b"Unsupported media type: ${mime".as_slice(),
        b"<button".as_slice(),
        b"id=\"status\"".as_slice(),
        b"textContent".as_slice(),
        b"video.src = attempt.source;\n    void playAutomatically(attempt);".as_slice(),
        b"<video playsinline controls>".as_slice(),
        b"heldSource".as_slice(),
        b"new MediaSource()".as_slice(),
        b"MediaSource.isTypeSupported".as_slice(),
        b"await playAutomatically(attempt)".as_slice(),
    ] {
        assert!(!html.windows(removed.len()).any(|window| window == removed));
    }
    let disable_remote = html
        .windows(b"video.disableRemotePlayback = true".len())
        .position(|window| window == b"video.disableRemotePlayback = true")
        .unwrap();
    let attach_source = html
        .windows(b"video.src = attempt.source".len())
        .position(|window| window == b"video.src = attempt.source")
        .unwrap();
    assert!(disable_remote < attach_source);

    let icon = viewer_icon().await;
    assert_eq!(icon.status(), StatusCode::OK);
    assert_eq!(icon.headers()[header::CONTENT_TYPE], "image/png");
    assert_eq!(icon.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(
        axum::body::to_bytes(icon.into_body(), VIEWER_ICON.len())
            .await
            .unwrap(),
        VIEWER_ICON
    );
    assert_eq!(
        stream(&host, &token, 1, Ipv4Addr::LOCALHOST).await.status(),
        StatusCode::TOO_EARLY
    );

    let session = host.start().unwrap();
    session
        .set_mime("video/mp4; codecs=\"test\"".to_owned())
        .unwrap();
    session.publish(&init()).unwrap();
    session
        .publish(&fragment(VIDEO_TRACK, 0x40, b"old-session"))
        .unwrap();
    let response = stream(&host, &token, 1, Ipv4Addr::LOCALHOST).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream().fuse();
    assert!(body.next().await.unwrap().is_ok());
    let first_key = host.viewers().unwrap()[0].key;
    assert_eq!(online(&host), 1);

    host.stop(&session).unwrap();
    assert_eq!(
        viewer_page(Path(token.clone()), State(host.clone()))
            .await
            .status(),
        StatusCode::OK
    );
    assert!(body.next().await.is_none());
    assert_eq!(online(&host), 0);
    assert_eq!(host.viewers().unwrap().len(), 1);
    assert_eq!(
        stream(&host, &token, 1, Ipv4Addr::LOCALHOST).await.status(),
        StatusCode::TOO_EARLY
    );

    assert_eq!(host.path().unwrap(), format!("/s/{token}"));
    let next = host.start().unwrap();
    next.set_mime("video/mp4; codecs=\"test\"".to_owned())
        .unwrap();
    let next_init = init();
    let next_fragment = fragment(VIDEO_TRACK, 0x40, b"new-session");
    next.publish(&next_init).unwrap();
    next.publish(&next_fragment).unwrap();
    assert!(body.next().await.is_none());
    let response = stream(&host, &token, 1, Ipv4Addr::LOCALHOST).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(host.viewers().unwrap().len(), 1);
    assert_eq!(host.viewers().unwrap()[0].key, first_key);
    let mut next_body = response.into_body().into_data_stream();
    assert_eq!(next_body.next().await.unwrap().unwrap(), next_init);
    assert_eq!(next_body.next().await.unwrap().unwrap(), next_fragment);
    host.stop(&next).unwrap();
    assert!(next_body.next().await.is_none());

    let refreshed = host.refresh(false).unwrap();
    assert!(refreshed.is_some());
    assert!(host.viewers().unwrap().is_empty());
}

#[tokio::test]
async fn refresh_revokes_old_viewers_without_restarting_media() {
    let host = Host::new().unwrap();
    let old_token = host.path().unwrap().trim_start_matches("/s/").to_owned();
    let session = host.start().unwrap();
    session
        .set_mime("video/mp4; codecs=\"test\"".to_owned())
        .unwrap();
    let init = init();
    let key = fragment(VIDEO_TRACK, 0x40, b"same-media-session");
    session.publish(&init).unwrap();
    session.publish(&key).unwrap();

    let mut old_bodies = Vec::new();
    for id in 0..MAX_VIEWERS {
        let response = stream(&host, &old_token, id as u8, Ipv4Addr::LOCALHOST).await;
        assert_eq!(response.status(), StatusCode::OK);
        old_bodies.push(response.into_body().into_data_stream());
    }
    assert_eq!(
        stream(&host, &old_token, MAX_VIEWERS as u8, Ipv4Addr::LOCALHOST,)
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert!(host.refresh(false).unwrap().is_none());
    assert_eq!(host.path().unwrap(), format!("/s/{old_token}"));

    let middle_token = host
        .refresh(true)
        .unwrap()
        .unwrap()
        .trim_start_matches("/s/")
        .to_owned();
    let token = host
        .refresh(true)
        .unwrap()
        .unwrap()
        .trim_start_matches("/s/")
        .to_owned();
    for revoked in [old_token, middle_token] {
        assert_eq!(
            viewer_page(Path(revoked.clone()), State(host.clone()))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            stream(&host, &revoked, 1, Ipv4Addr::LOCALHOST)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            media_head(Path(revoked), State(host.clone())).await,
            StatusCode::NOT_FOUND
        );
    }
    for mut body in old_bodies {
        assert!(body.next().await.is_none());
    }

    assert!(host.viewers().unwrap().is_empty());
    let response = stream(&host, &token, 1, Ipv4Addr::LOCALHOST).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(online(&host), 1);
    let mut body = response.into_body().into_data_stream();
    assert_eq!(body.next().await.unwrap().unwrap(), init);
    assert_eq!(body.next().await.unwrap().unwrap(), key);
    host.stop(&session).unwrap();
    assert!(body.next().await.is_none());
}

#[tokio::test]
async fn host_disconnect_permanently_blocks_viewer_and_old_keys_do_not_cross_refresh() {
    let host = Host::new().unwrap();
    let token = host.path().unwrap().trim_start_matches("/s/").to_owned();
    assert!(host.viewers().unwrap().is_empty());
    let session = host.start().unwrap();
    session.set_mime("video/mp4".to_owned()).unwrap();
    let init = init();
    let keyframe = fragment(VIDEO_TRACK, 0x40, b"key");
    session.publish(&init).unwrap();
    session.publish(&keyframe).unwrap();

    let response = stream(&host, &token, 1, Ipv4Addr::LOCALHOST).await;
    let mut blocked_body = response.into_body().into_data_stream();
    assert_eq!(blocked_body.next().await.unwrap().unwrap(), init);
    let old_key = host.viewers().unwrap()[0].key;
    host.disconnect_viewer(old_key).unwrap();
    assert!(blocked_body.next().await.is_none());

    let mut other_bodies = Vec::new();
    for id in 2..=9 {
        let response = stream(&host, &token, id, Ipv4Addr::LOCALHOST).await;
        assert_eq!(response.status(), StatusCode::OK);
        other_bodies.push(response.into_body().into_data_stream());
    }
    let response = stream(&host, &token, 1, Ipv4Addr::LOCALHOST).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");

    host.stop(&session).unwrap();
    for body in &mut other_bodies {
        assert!(body.next().await.is_none());
    }
    assert_eq!(
        stream(&host, &token, 1, Ipv4Addr::LOCALHOST).await.status(),
        StatusCode::CONFLICT
    );
    let unready = host.start().unwrap();
    assert_eq!(
        stream(&host, &token, 1, Ipv4Addr::LOCALHOST).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        stream(&host, &token, 10, Ipv4Addr::LOCALHOST)
            .await
            .status(),
        StatusCode::TOO_EARLY
    );
    host.stop(&unready).unwrap();

    let next = host.start().unwrap();
    next.set_mime("video/mp4".to_owned()).unwrap();
    next.publish(&init).unwrap();
    next.publish(&keyframe).unwrap();
    assert_eq!(
        stream(&host, &token, 1, Ipv4Addr::LOCALHOST).await.status(),
        StatusCode::CONFLICT
    );

    let new_token = host
        .refresh(true)
        .unwrap()
        .unwrap()
        .trim_start_matches("/s/")
        .to_owned();
    let response = stream(&host, &new_token, 1, Ipv4Addr::LOCALHOST).await;
    assert_eq!(response.status(), StatusCode::OK);
    let new_key = host.viewers().unwrap()[0].key;
    assert_ne!(new_key, old_key);
    let mut new_body = response.into_body().into_data_stream();
    assert_eq!(new_body.next().await.unwrap().unwrap(), init);
    assert_eq!(new_body.next().await.unwrap().unwrap(), keyframe);

    host.disconnect_viewer(old_key).unwrap();
    let live = fragment(VIDEO_TRACK, 0x100c0, b"live");
    next.publish(&live).unwrap();
    assert_eq!(new_body.next().await.unwrap().unwrap(), live);
    assert!(host.viewers().unwrap()[0].online());
    host.stop(&next).unwrap();
    assert!(new_body.next().await.is_none());
}

#[tokio::test]
async fn telemetry_is_bounded_and_expires_with_the_current_stream() {
    let host = Host::new().unwrap();
    let token = host.path().unwrap().trim_start_matches("/s/").to_owned();
    assert_eq!(
        telemetry_request(&host, "invalid", HeaderMap::new(), vec![b'x'; 1_000],)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        telemetry_request(&host, &token, HeaderMap::new(), "not JSON")
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        telemetry_request(
            &host,
            &token,
            viewer_headers(9),
            r#"{"rtt_ms":null,"playback_lag_ms":0}"#,
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert!(host.viewers().unwrap().is_empty());

    let session = host.start().unwrap();
    session.set_mime("video/mp4".to_owned()).unwrap();
    session.publish(&init()).unwrap();
    session
        .publish(&fragment(VIDEO_TRACK, 0x40, b"key"))
        .unwrap();
    let stream_response = stream(&host, &token, 1, Ipv4Addr::LOCALHOST).await;
    assert_eq!(stream_response.status(), StatusCode::OK);

    for invalid in [
        r#"{"rtt_ms":60001,"playback_lag_ms":0}"#,
        r#"{"rtt_ms":null,"playback_lag_ms":60001}"#,
        r#"{"rtt_ms":1,"playback_lag_ms":2,"extra":3}"#,
    ] {
        assert_eq!(
            telemetry_request(&host, &token, viewer_headers(1), invalid)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let response = telemetry_request(
        &host,
        &token,
        viewer_headers(1),
        r#"{"rtt_ms":42,"playback_lag_ms":1500}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let measured_at = Instant::now();
    let viewer = host.viewers().unwrap().remove(0);
    assert_eq!(
        viewer.telemetry(measured_at),
        (
            Some(Duration::from_millis(42)),
            Some(Duration::from_millis(1_500)),
        )
    );
    assert_eq!(
        viewer.telemetry(measured_at + TELEMETRY_STALE_AFTER),
        (None, None)
    );

    drop(stream_response);
    let viewer = host.viewers().unwrap().remove(0);
    assert!(!viewer.online());
    assert_eq!(viewer.telemetry(Instant::now()), (None, None));
    let reconnected = stream(&host, &token, 1, Ipv4Addr::LOCALHOST).await;
    assert_eq!(reconnected.status(), StatusCode::OK);
    assert_eq!(
        host.viewers().unwrap().remove(0).telemetry(Instant::now()),
        (None, None)
    );
    drop(reconnected);
}
