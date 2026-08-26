use super::*;
use futures_util::StreamExt as _;
use gst::prelude::*;

const VIDEO_TRACK: u32 = 37;

fn viewers(host: &Host) -> Arc<ViewerGeneration> {
    lock(&host.inner).unwrap().viewers.clone()
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
        if let Some(subscription) = session.hub.subscribe(viewers.clone()).unwrap() {
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
        .map(|_| session.hub.subscribe(generation.clone()).unwrap().unwrap())
        .collect();
    let mut count = host.viewer_count().unwrap();
    assert_eq!(*count.borrow_and_update(), 3);
    let extras: Vec<_> = (3..MAX_VIEWERS)
        .map(|_| session.hub.subscribe(generation.clone()).unwrap().unwrap())
        .collect();
    assert!(matches!(
        session.hub.subscribe(generation),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock
    ));
    drop(extras);
    assert_eq!(*count.borrow_and_update(), 3);
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
    assert_eq!(*count.borrow_and_update(), 2);
    viewers.pop();
    assert_eq!(*count.borrow_and_update(), 1);
    viewers.pop();
    assert_eq!(*count.borrow_and_update(), 0);
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
    assert!(session.hub.subscribe(viewers.clone()).unwrap().is_none());
    session
        .publish(&fragment(VIDEO_TRACK, 0x40, b"recovered"))
        .unwrap();
    assert!(session.hub.subscribe(viewers).unwrap().is_some());
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
        .map(|_| session.hub.subscribe(generation.clone()).unwrap().unwrap())
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
        media_stream(Path("invalid".to_owned()), State(host.clone()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let page = viewer_page(Path(token.clone()), State(host.clone())).await;
    assert_eq!(page.status(), StatusCode::OK);
    assert_eq!(page.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(page.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(page.headers()["referrer-policy"], "no-referrer");
    let html = axum::body::to_bytes(page.into_body(), 100_000)
        .await
        .unwrap();
    assert!(
        !html
            .windows(token.len())
            .any(|window| window == token.as_bytes())
    );
    assert_eq!(
        media_stream(Path(token.clone()), State(host.clone()))
            .await
            .status(),
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
    let mut count = host.viewer_count().unwrap();
    let response = media_stream(Path(token.clone()), State(host.clone())).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream().fuse();
    assert!(body.next().await.unwrap().is_ok());
    assert_eq!(*count.borrow_and_update(), 1);

    host.stop(&session).unwrap();
    assert_eq!(
        viewer_page(Path(token.clone()), State(host.clone()))
            .await
            .status(),
        StatusCode::OK
    );
    assert!(body.next().await.is_none());
    assert_eq!(*count.borrow_and_update(), 0);
    assert_eq!(
        media_stream(Path(token.clone()), State(host.clone()))
            .await
            .status(),
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
    let response = media_stream(Path(token), State(host.clone())).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut next_body = response.into_body().into_data_stream();
    assert_eq!(next_body.next().await.unwrap().unwrap(), next_init);
    assert_eq!(next_body.next().await.unwrap().unwrap(), next_fragment);
    host.stop(&next).unwrap();
    assert!(next_body.next().await.is_none());
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
    for _ in 0..MAX_VIEWERS {
        let response = media_stream(Path(old_token.clone()), State(host.clone())).await;
        assert_eq!(response.status(), StatusCode::OK);
        old_bodies.push(response.into_body().into_data_stream());
    }
    assert_eq!(
        media_stream(Path(old_token.clone()), State(host.clone()))
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
            media_stream(Path(revoked.clone()), State(host.clone()))
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

    let mut count = host.viewer_count().unwrap();
    assert_eq!(*count.borrow_and_update(), 0);
    let response = media_stream(Path(token), State(host.clone())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(*count.borrow_and_update(), 1);
    let mut body = response.into_body().into_data_stream();
    assert_eq!(body.next().await.unwrap().unwrap(), init);
    assert_eq!(body.next().await.unwrap().unwrap(), key);
    host.stop(&session).unwrap();
    assert!(body.next().await.is_none());
}
