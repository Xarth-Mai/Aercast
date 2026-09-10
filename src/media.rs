use crate::{
    AudioSettings, Events, HostEvent, Result, ShareSettings, ShareStop, audio, settings, web,
};
use futures_util::{FutureExt, StreamExt};
use gst::prelude::*;
use gst_app::AppSinkCallbacks;
use std::{future::Future, io, pin::Pin, time::Instant};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VideoPlan {
    pub(crate) settings: settings::VideoSettings,
    pub(crate) encoder: Encoder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Encoder {
    VaApi,
    X264,
}

#[derive(Debug)]
pub(crate) struct HardwareVideoFailure(String);

impl std::fmt::Display for HardwareVideoFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for HardwareVideoFailure {}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn serve_video(
    description: &str,
    capture_caps: &mut Option<gst::Caps>,
    share: ShareSettings,
    media: web::MediaSession,
    fragment_ready: watch::Sender<bool>,
    reached_sharing: &mut bool,
    mut control: Pin<&mut impl Future<Output = ShareStop>>,
    events: &Events,
) -> Result<ShareStop> {
    let audio = &share.audio;
    let audio_exclusions = audio.enabled.then(|| audio.exclusions.clone());
    if audio_exclusions.as_ref().is_some_and(Vec::is_empty) {
        eprintln!(
            "No audio exclusions configured; a Host-local Viewer may feed shared audio back into Aercast."
        );
    }
    let started = Instant::now();

    let pipeline = build_pipeline(description)?;
    let portal_video = pipeline
        .by_name("portal-video")
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no Portal video source"))?;
    let portal_format = pipeline
        .by_name("portal-format")
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no Portal video format"))?;
    if let Some(caps) = capture_caps.as_ref() {
        portal_format.set_property("caps", caps);
    }
    let parser_pad = pipeline
        .by_name("h264")
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no H.264 parser"))?
        .static_pad("src")
        .ok_or_else(|| io::Error::other("H.264 parser has no source pad"))?;
    let audio_source = pipeline
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
                let fragment_ready = fragment_ready.clone();
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
                        fragment_ready.send_replace(true);
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
    if let Some(stop) = control.as_mut().now_or_never() {
        return Ok(stop);
    }
    let outcome: Result<ShareStop> = match pipeline.set_state(gst::State::Playing) {
        Err(error) => match messages.next().now_or_never().flatten() {
            Some(message) => queued_media_outcome(Some(message), &mut messages),
            None => Err(error.into()),
        },
        Ok(_) => match audio_exclusions
            .map(|exclusions| audio::start(audio_source, audio.exclude_communication, exclusions))
            .transpose()
        {
            Err(error) => Err(error.into()),
            Ok(mut audio_capture) => {
                println!("Browser stream running.");
                *reached_sharing = true;
                let _ = events.unbounded_send(HostEvent::Sharing(share.clone()));
                let mut audio_failure_reported = false;
                let mut running = tokio::select! {
                    biased;
                    stop = control.as_mut() => Ok(stop),
                    error = async {
                        match audio_capture.as_mut() {
                            Some((_, errors)) => errors.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        audio_failure_reported = error.is_some();
                        Err(io::Error::other(error.unwrap_or_else(||
                            "selective-audio thread stopped unexpectedly".to_owned()
                        )).into())
                    },
                    message = messages.next() => {
                        queued_media_outcome(message, &mut messages)
                    },
                };
                if matches!(
                    &running,
                    Ok(ShareStop::End | ShareStop::Quit | ShareStop::PortalClosed)
                ) {
                    let _ = events.unbounded_send(HostEvent::Ending);
                }
                if matches!(&running, Ok(ShareStop::Sleep))
                    && let Err(error) = pipeline.set_state(gst::State::Paused)
                {
                    running = Ok(ShareStop::Failed(error.into()));
                }
                let stopped: Result<()> = audio_capture
                    .map_or(Ok(()), |(audio, _)| audio.stop(audio_failure_reported))
                    .map_err(|error| io::Error::other(error).into());
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

    if let Some(caps) = portal_video
        .static_pad("src")
        .and_then(|pad| pad.current_caps())
    {
        *capture_caps = Some(caps);
    }
    let stop_result = pipeline.set_state(gst::State::Null);
    if let Err(error) = &stop_result {
        eprintln!("Failed to stop GStreamer pipeline: {error}");
    }
    match stop_result {
        Ok(_) => outcome,
        Err(error) => Ok(ShareStop::Failed(error.into())),
    }
}

pub(crate) async fn probe_video_plan(
    video: settings::VideoSettings,
) -> std::result::Result<VideoPlan, String> {
    tokio::task::spawn_blocking(move || video_plan(&video).map_err(|error| error.to_string()))
        .await
        .map_err(|error| format!("video encoder check failed: {error}"))?
}

fn video_plan(video: &settings::VideoSettings) -> Result<VideoPlan> {
    video.validate()?;
    match video.encoder {
        settings::VideoEncoder::Auto => {
            plan_encoder(*video, Encoder::VaApi).or_else(|_| plan_encoder(*video, Encoder::X264))
        }
        settings::VideoEncoder::VaApi => plan_encoder(*video, Encoder::VaApi),
        settings::VideoEncoder::X264 => plan_encoder(*video, Encoder::X264),
    }
}

pub(crate) fn plan_encoder(video: settings::VideoSettings, encoder: Encoder) -> Result<VideoPlan> {
    let (factory_name, format) = match encoder {
        Encoder::VaApi => ("vah264enc", "NV12"),
        Encoder::X264 => ("x264enc", "I420"),
    };
    let caps = gst::Caps::builder("video/x-raw")
        .field("format", format)
        .field("width", video.width as i32)
        .field("height", video.height as i32)
        .field("framerate", gst::Fraction::new(video.fps as i32, 1))
        .build();
    require_caps(factory_name, gst::PadDirection::Sink, &caps)?;
    let plan = VideoPlan {
        settings: video,
        encoder,
    };
    let pipeline = build_pipeline(&pipeline_description(
        1,
        0,
        plan,
        settings::DEFAULT_AUDIO_BITRATE_KBPS,
    ))?;
    let encoder = pipeline
        .by_name("encoder")
        .ok_or_else(|| io::Error::other("GStreamer pipeline has no video encoder"))?;
    let ready = encoder
        .set_state(gst::State::Ready)
        .and_then(|_| encoder.state(gst::ClockTime::from_seconds(3)).0);
    let reached_ready = encoder.current_state() == gst::State::Ready;
    let stopped = encoder.set_state(gst::State::Null);
    ready?;
    stopped?;
    if !reached_ready {
        return Err(io::Error::other(format!("{factory_name} did not become ready")).into());
    }
    Ok(plan)
}

fn require_caps(factory_name: &str, direction: gst::PadDirection, caps: &gst::Caps) -> Result<()> {
    let factory = gst::ElementFactory::find(factory_name)
        .ok_or_else(|| io::Error::other(format!("missing GStreamer element {factory_name}")))?;
    if factory
        .static_pad_templates()
        .iter()
        .any(|template| template.direction() == direction && template.caps().can_intersect(caps))
    {
        Ok(())
    } else {
        Err(io::Error::other(format!("{factory_name} does not support {caps}")).into())
    }
}

fn build_pipeline(description: &str) -> Result<gst::Pipeline> {
    gst::parse::launch(description)?
        .downcast::<gst::Pipeline>()
        .map_err(|_| io::Error::other("GStreamer did not create a pipeline").into())
}

pub(crate) fn pipeline_description(
    node_id: u32,
    remote_fd: i32,
    plan: VideoPlan,
    audio_bitrate_kbps: u32,
) -> String {
    let video = plan.settings;
    let video_pipeline = match plan.encoder {
        Encoder::VaApi => {
            let bitrate = video.bitrate_mbps.map_or_else(String::new, |bitrate| {
                let bitrate = u32::from(bitrate) * 1_000;
                format!(" bitrate={bitrate} cpb-size={}", bitrate / 10)
            });
            format!(
                "vapostproc name=video-converter add-borders=true ! video/x-raw(memory:VAMemory),format=NV12,width={width},height={height} ! imagefreeze is-live=true allow-replace=true ! video/x-raw(memory:VAMemory),format=NV12,framerate={fps}/1 ! vah264enc name=encoder rate-control=cbr target-usage=7{bitrate} key-int-max={fps} ! video/x-h264,profile=constrained-baseline,stream-format=byte-stream,alignment=au",
                width = video.width,
                height = video.height,
                fps = video.fps,
            )
        }
        Encoder::X264 => {
            let bitrate = video.bitrate_mbps.map_or_else(String::new, |bitrate| {
                format!(
                    " bitrate={} vbv-buf-capacity=100 nal-hrd=cbr",
                    u32::from(bitrate) * 1_000
                )
            });
            format!(
                "videoconvertscale name=video-converter add-borders=true ! video/x-raw,format=I420,width={width},height={height} ! imagefreeze is-live=true allow-replace=true ! video/x-raw,format=I420,framerate={fps}/1 ! x264enc name=encoder tune=zerolatency speed-preset=ultrafast{bitrate} key-int-max={fps}",
                width = video.width,
                height = video.height,
                fps = video.fps,
            )
        }
    };
    format!(
        "mp4mux name=mux fragment-duration=100 ! appsink name=stream sync=false wait-on-eos=false
         audiomixer name=audio-mixer ignore-inactive-pads=true ! audioconvert ! audio/x-raw,format=F32LE,rate=48000,channels=2 ! avenc_aac bitrate={audio_bitrate} ! aacparse ! audio/mpeg,mpegversion=4,stream-format=raw ! queue ! mux.audio_0
         audiotestsrc is-live=true wave=silence ! audio/x-raw,format=F32LE,rate=48000,channels=2 ! queue ! audio-mixer.
         appsrc name=system-audio is-live=true format=time do-timestamp=true block=false max-bytes=384000 leaky-type=downstream ! audio/x-raw,format=F32LE,rate=48000,channels=2,layout=interleaved ! queue ! audio-mixer.
         pipewiresrc name=portal-video fd={remote_fd} path={node_id} on-disconnect=error ! capsfilter name=portal-format ! {video_pipeline} ! h264parse name=h264 ! video/x-h264,stream-format=avc,alignment=au ! queue ! mux.video_0",
        audio_bitrate = audio_bitrate_kbps * 1_000,
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

fn queued_media_outcome(
    first: Option<gst::Message>,
    messages: &mut (impl futures_util::Stream<Item = gst::Message> + Unpin),
) -> Result<ShareStop> {
    media_outcome(first.into_iter().chain(std::iter::from_fn(|| {
        messages.next().now_or_never().flatten()
    })))
}

fn media_outcome(messages: impl IntoIterator<Item = gst::Message>) -> Result<ShareStop> {
    let mut descriptions = Vec::new();
    let mut all_hardware_video = true;
    let mut saw_error = false;
    for message in messages {
        match message.view() {
            gst::MessageView::Eos(..) => {
                descriptions.push("capture stream ended".to_owned());
                all_hardware_video = false;
            }
            gst::MessageView::Error(error) => {
                saw_error = true;
                all_hardware_video &= hardware_video_error(&message);
                let source = message
                    .src()
                    .map(|source| source.path_string().to_string())
                    .unwrap_or_else(|| "unknown".to_owned());
                descriptions.push(format!(
                    "GStreamer error from {source}: {} ({})",
                    error.error(),
                    error.debug().unwrap_or_default(),
                ));
            }
            _ => {
                descriptions.push("unexpected GStreamer bus message".to_owned());
                all_hardware_video = false;
            }
        }
    }
    if descriptions.is_empty() {
        return Err(io::Error::other("GStreamer bus closed").into());
    }
    let description = descriptions.join("; ");
    if saw_error && all_hardware_video {
        Err(HardwareVideoFailure(description).into())
    } else {
        Err(io::Error::other(description).into())
    }
}

fn hardware_video_error(message: &gst::Message) -> bool {
    let gst::MessageView::Error(message_error) = message.view() else {
        return false;
    };
    let Some(source) = message.src().map(|source| source.name().to_string()) else {
        return false;
    };
    let error = message_error.error();
    let details = message_error.details();
    let flow = details
        .filter(|details| details.has_field("flow-return"))
        .and_then(|details| details.get::<gst::FlowReturn>("flow-return").ok());
    let missing_flow = details.is_none_or(|details| !details.has_field("flow-return"));
    match source.as_str() {
        "encoder" => {
            error.matches(gst::StreamError::Encode)
                || error.matches(gst::LibraryError::Init)
                || error.matches(gst::LibraryError::Failed)
                || error.matches(gst::CoreError::Negotiation)
        }
        "video-converter" => {
            error.matches(gst::LibraryError::Init)
                || error.matches(gst::ResourceError::Settings)
                || error.matches(gst::CoreError::Negotiation)
                || error.matches(gst::CoreError::NotImplemented)
        }
        "portal-video" => {
            error.matches(gst::StreamError::Format)
                || (error.matches(gst::StreamError::Failed)
                    && flow == Some(gst::FlowReturn::NotNegotiated))
        }
        "portal-format" => error.matches(gst::StreamError::Format),
        "h264" => {
            error.matches(gst::StreamError::Format)
                || error.matches(gst::StreamError::Decode)
                || error.matches(gst::StreamError::WrongType)
                || (error.matches(gst::StreamError::Failed)
                    && (missing_flow || flow == Some(gst::FlowReturn::NotNegotiated)))
        }
        _ => false,
    }
}

pub(crate) fn audio_settings(settings: &settings::Settings) -> AudioSettings {
    AudioSettings {
        enabled: settings.system_audio,
        bitrate_kbps: settings.audio_bitrate_kbps,
        exclude_communication: settings.exclude_communication_audio,
        exclusions: settings
            .audio_exclusions
            .iter()
            .filter(|exclusion| exclusion.enabled)
            .map(|exclusion| exclusion.identity.clone())
            .collect(),
    }
}

pub(crate) fn same_saved_media(left: &ShareSettings, right: &ShareSettings) -> bool {
    left.audio == right.audio && left.video.settings == right.video.settings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share_session::{MAX_MEDIA_RECOVERIES, should_fallback};
    fn gst_error<T: gst::message::MessageErrorDomain>(
        source: &str,
        error: T,
        details: Option<gst::Structure>,
    ) -> gst::Message {
        let source = gst::ElementFactory::make("identity")
            .name(source)
            .build()
            .unwrap();
        gst::message::Error::builder(error, "test")
            .src(&source)
            .details_if_some(details)
            .build()
    }

    fn flow_details(flow: gst::FlowReturn) -> Option<gst::Structure> {
        Some(
            gst::Structure::builder("details")
                .field("flow-return", flow)
                .build(),
        )
    }

    #[test]
    fn va_fallback_uses_only_the_structured_hardware_error_whitelist() {
        gst::init().unwrap();
        let cases = [
            gst_error("encoder", gst::StreamError::Encode, None),
            gst_error("encoder", gst::LibraryError::Init, None),
            gst_error("encoder", gst::LibraryError::Failed, None),
            gst_error("encoder", gst::CoreError::Negotiation, None),
            gst_error("video-converter", gst::LibraryError::Init, None),
            gst_error("video-converter", gst::ResourceError::Settings, None),
            gst_error("video-converter", gst::CoreError::Negotiation, None),
            gst_error("video-converter", gst::CoreError::NotImplemented, None),
            gst_error("portal-video", gst::StreamError::Format, None),
            gst_error(
                "portal-video",
                gst::StreamError::Failed,
                flow_details(gst::FlowReturn::NotNegotiated),
            ),
            gst_error("portal-format", gst::StreamError::Format, None),
            gst_error("h264", gst::StreamError::Format, None),
            gst_error("h264", gst::StreamError::Decode, None),
            gst_error("h264", gst::StreamError::WrongType, None),
            gst_error("h264", gst::StreamError::Failed, None),
            gst_error(
                "h264",
                gst::StreamError::Failed,
                flow_details(gst::FlowReturn::NotNegotiated),
            ),
        ];
        for message in cases {
            assert!(
                hardware_video_error(&message),
                "expected hardware fallback for {:?}",
                message.src().map(|source| source.name())
            );
        }

        let malformed_flow = Some(
            gst::Structure::builder("details")
                .field("flow-return", "not-a-flow-return")
                .build(),
        );
        let rejected = [
            gst_error("encoder", gst::LibraryError::Encode, None),
            gst_error("video-converter", gst::ResourceError::Failed, None),
            gst_error("portal-video", gst::ResourceError::NotFound, None),
            gst_error("portal-video", gst::StreamError::Failed, None),
            gst_error(
                "portal-video",
                gst::StreamError::Failed,
                flow_details(gst::FlowReturn::Error),
            ),
            gst_error("portal-format", gst::CoreError::Negotiation, None),
            gst_error(
                "h264",
                gst::StreamError::Failed,
                flow_details(gst::FlowReturn::Error),
            ),
            gst_error("h264", gst::StreamError::Failed, malformed_flow),
            gst_error("mux", gst::StreamError::Encode, None),
            gst_error("system-audio", gst::StreamError::Encode, None),
            gst_error("stream", gst::StreamError::Encode, None),
            gst_error("unknown", gst::StreamError::Encode, None),
        ];
        for message in rejected {
            assert!(
                !hardware_video_error(&message),
                "unexpected hardware fallback for {:?}",
                message.src().map(|source| source.name())
            );
        }

        let hardware = media_outcome([
            gst_error("encoder", gst::StreamError::Encode, None),
            gst_error("h264", gst::StreamError::Format, None),
        ])
        .err()
        .unwrap();
        assert!(hardware.is::<HardwareVideoFailure>());
        let mixed = media_outcome([
            gst_error("encoder", gst::StreamError::Encode, None),
            gst_error("mux", gst::StreamError::Mux, None),
        ])
        .err()
        .unwrap();
        assert!(!mixed.is::<HardwareVideoFailure>());
        let eos = media_outcome([
            gst_error("encoder", gst::StreamError::Encode, None),
            gst::message::Eos::new(),
        ])
        .err()
        .unwrap();
        assert!(!eos.is::<HardwareVideoFailure>());

        let bus = gst::Bus::new();
        let message_types = [gst::MessageType::Error, gst::MessageType::Eos];
        let mut messages = bus.stream_filtered(&message_types);
        bus.post(gst_error(
            "video-converter",
            gst::CoreError::Negotiation,
            None,
        ))
        .unwrap();
        let first = messages.next().now_or_never().flatten();
        let queued = queued_media_outcome(first, &mut messages).err().unwrap();
        assert!(queued.is::<HardwareVideoFailure>());

        bus.post(gst_error("encoder", gst::StreamError::Encode, None))
            .unwrap();
        bus.post(gst_error("mux", gst::StreamError::Mux, None))
            .unwrap();
        let first = messages.next().now_or_never().flatten();
        let queued = queued_media_outcome(first, &mut messages).err().unwrap();
        assert!(!queued.is::<HardwareVideoFailure>());

        let automatic = VideoPlan {
            settings: settings::VideoSettings::default(),
            encoder: Encoder::VaApi,
        };
        assert!(should_fallback(automatic, false, 0, &hardware));
        assert!(!should_fallback(automatic, true, 0, &hardware));
        assert!(!should_fallback(
            automatic,
            false,
            MAX_MEDIA_RECOVERIES,
            &hardware
        ));
        assert!(!should_fallback(
            VideoPlan {
                encoder: Encoder::X264,
                ..automatic
            },
            false,
            0,
            &hardware
        ));
        assert!(!should_fallback(
            VideoPlan {
                settings: settings::VideoSettings {
                    encoder: settings::VideoEncoder::VaApi,
                    ..automatic.settings
                },
                ..automatic
            },
            false,
            0,
            &hardware
        ));
        assert!(!should_fallback(automatic, false, 0, &mixed));
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
        let video = settings::VideoSettings::default();
        let description = pipeline_description(
            1,
            0,
            VideoPlan {
                settings: video,
                encoder: Encoder::X264,
            },
            96,
        );
        assert!(description.contains("width=1280,height=720"));
        assert!(description.contains("framerate=60/1"));
        assert!(
            description.contains("bitrate=6000 vbv-buf-capacity=100 nal-hrd=cbr key-int-max=60")
        );
        assert!(description.contains("avenc_aac bitrate=96000"));
        assert!(description.contains("videoconvertscale name=video-converter add-borders=true"));
        assert!(!description.contains("vapostproc"));
        let encoder_default = pipeline_description(
            1,
            0,
            VideoPlan {
                settings: settings::VideoSettings {
                    fps: 30,
                    bitrate_mbps: None,
                    ..video
                },
                encoder: Encoder::X264,
            },
            128,
        );
        assert!(encoder_default.contains(
            "x264enc name=encoder tune=zerolatency speed-preset=ultrafast key-int-max=30"
        ));
        let va_api = pipeline_description(
            1,
            0,
            VideoPlan {
                settings: video,
                encoder: Encoder::VaApi,
            },
            160,
        );
        assert!(va_api.contains("video/x-raw(memory:VAMemory),format=NV12"));
        assert!(va_api.contains("vapostproc name=video-converter add-borders=true"));
        assert!(!va_api.contains("disable-passthrough=true"));
        assert!(va_api.contains(
            "vah264enc name=encoder rate-control=cbr target-usage=7 bitrate=6000 cpb-size=600"
        ));
        assert!(va_api.contains("avenc_aac bitrate=160000"));
        assert!(va_api.contains("profile=constrained-baseline,stream-format=byte-stream"));
        for description in [description, va_api] {
            if let Err(error) = gst::parse::launch(&description) {
                assert_ne!(
                    error.kind::<gst::ParseError>(),
                    Some(gst::ParseError::Syntax)
                );
            }
        }
    }

    #[test]
    #[ignore = "requires the supported host video stack"]
    fn host_video_encoders_are_available() {
        gst::init().unwrap();
        let video = settings::VideoSettings::default();
        let automatic = video_plan(&video).unwrap();
        assert_eq!(automatic.encoder, Encoder::VaApi);
        let beyond_va = settings::VideoSettings {
            width: 5_000,
            height: 3_000,
            encoder: settings::VideoEncoder::VaApi,
            ..video
        };
        assert!(video_plan(&beyond_va).is_err());
        assert_eq!(
            video_plan(&settings::VideoSettings {
                encoder: settings::VideoEncoder::Auto,
                ..beyond_va
            })
            .unwrap()
            .encoder,
            Encoder::X264
        );
        let software = video_plan(&settings::VideoSettings {
            encoder: settings::VideoEncoder::X264,
            ..video
        })
        .unwrap();
        assert_eq!(software.encoder, Encoder::X264);
        let pipeline = build_pipeline(&pipeline_description(
            1,
            0,
            automatic,
            settings::DEFAULT_AUDIO_BITRATE_KBPS,
        ))
        .unwrap();
        let encoder = pipeline.by_name("encoder").unwrap();
        assert_eq!(encoder.property::<u32>("bitrate"), 6_000);
        assert_eq!(encoder.property::<u32>("key-int-max"), 60);
        assert_eq!(encoder.property::<u32>("target-usage"), 7);
        let pipeline = build_pipeline(&pipeline_description(
            1,
            0,
            software,
            settings::DEFAULT_AUDIO_BITRATE_KBPS,
        ))
        .unwrap();
        let encoder = pipeline.by_name("encoder").unwrap();
        assert_eq!(encoder.property::<u32>("bitrate"), 6_000);
        assert_eq!(encoder.property::<u32>("key-int-max"), 60);
    }
}
