use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    io,
    rc::{Rc, Weak},
    sync::mpsc::{Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use pw::spa::{self, pod::Pod, utils::result::AsyncSeq};
use tokio::sync::mpsc;

const FRAME_BYTES: usize = 2 * size_of::<f32>();

pub struct AudioCapture {
    stop: pw::channel::Sender<()>,
    finished: Receiver<()>,
    thread: thread::JoinHandle<Result<(), String>>,
}

pub fn start(
    appsrc: gst_app::AppSrc,
    exclusions: Vec<String>,
) -> io::Result<(AudioCapture, mpsc::UnboundedReceiver<String>)> {
    let (stop, receiver) = pw::channel::channel();
    let (errors, error_receiver) = mpsc::unbounded_channel();
    let (finished_sender, finished) = std::sync::mpsc::sync_channel(1);
    let thread = thread::Builder::new()
        .name("aercast-audio".to_owned())
        .spawn(move || {
            let result = run(appsrc, exclusions, receiver);
            if let Err(error) = &result {
                let _ = errors.send(error.clone());
            }
            let _ = finished_sender.send(());
            result
        })?;
    Ok((
        AudioCapture {
            stop,
            finished,
            thread,
        },
        error_receiver,
    ))
}

impl AudioCapture {
    pub fn stop(self) -> Result<(), String> {
        let _ = self.stop.send(());
        match self.finished.recv_timeout(Duration::from_secs(6)) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {}
            Err(RecvTimeoutError::Timeout) => {
                return Err("timed out while stopping selective audio".to_owned());
            }
        }
        self.thread
            .join()
            .map_err(|_| "selective-audio thread panicked".to_owned())?
    }
}

struct Port {
    node: u32,
    direction: String,
    channel: Option<String>,
    ignore_latency: bool,
}

struct Playback {
    _listener: pw::node::NodeListener,
    _proxy: pw::node::Node,
    policy: PlaybackPolicy,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct PlaybackPolicy {
    identity: Option<String>,
    communication: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Endpoints {
    output_node: u32,
    output_port: u32,
    input_node: u32,
    input_port: u32,
}

#[derive(Clone, Copy)]
struct ObservedLink {
    id: u32,
    serial: Option<u64>,
    endpoints: Endpoints,
    passive: Option<bool>,
    status: Option<LinkStatus>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LinkStatus {
    Pending,
    Active,
    Error,
}

struct GraphLink {
    _listener: pw::link::LinkListener,
    _proxy: pw::link::Link,
    endpoints: Endpoints,
    serial: u64,
    status: Option<LinkStatus>,
}

struct OwnedLink {
    _listener: pw::link::LinkListener,
    _proxy: pw::link::Link,
    expected: Endpoints,
    observed: Option<ObservedLink>,
}

#[derive(Clone, Copy)]
enum Phase {
    Rebuild,
    Links,
    Cleanup,
}

struct State {
    mainloop: pw::main_loop::MainLoopRc,
    core: pw::core::CoreRc,
    registry: pw::registry::RegistryRc,
    stream: pw::stream::StreamRc,
    exclusions: Vec<String>,
    playback: HashMap<u32, Playback>,
    sinks: HashSet<u32>,
    ports: HashMap<u32, Port>,
    graph_links: HashMap<u32, GraphLink>,
    links: Vec<OwnedLink>,
    pending: Option<(AsyncSeq, Phase)>,
    initial_done: bool,
    dirty: bool,
    active: bool,
    received_audio: bool,
    activation_started: Option<Instant>,
    stopping: bool,
    failure: Option<String>,
}

impl State {
    fn global(
        &mut self,
        global: &pw::registry::GlobalObject<&spa::utils::dict::DictRef>,
        weak: &Weak<RefCell<Self>>,
    ) {
        let Some(props) = global.props else {
            if global.type_ == pw::types::ObjectType::Link
                || (global.type_ == pw::types::ObjectType::Node
                    && global.id == self.stream.node_id())
            {
                self.fail(
                    "PipeWire exposed a safety-relevant object without properties".to_owned(),
                );
            }
            return;
        };
        let changed = match global.type_ {
            pw::types::ObjectType::Node => {
                if global.id == self.stream.node_id() {
                    if let Err(error) = validate_exported_node(props) {
                        self.fail(error);
                        return;
                    }
                    true
                } else if props.get(*pw::keys::MEDIA_CLASS) == Some("Stream/Output/Audio") {
                    let proxy = match self.registry.bind::<pw::node::Node, _>(global) {
                        Ok(proxy) => proxy,
                        Err(error) => {
                            self.fail(format!("failed to inspect playback node: {error}"));
                            return;
                        }
                    };
                    let id = global.id;
                    let weak = weak.clone();
                    let listener = proxy
                        .add_listener_local()
                        .info(move |info| {
                            if let Some(state) = weak.upgrade() {
                                state.borrow_mut().node_info(id, info);
                            }
                        })
                        .register();
                    self.playback.insert(
                        id,
                        Playback {
                            _listener: listener,
                            _proxy: proxy,
                            policy: PlaybackPolicy::default(),
                        },
                    );
                    true
                } else if props.get(*pw::keys::MEDIA_CLASS) == Some("Audio/Sink") {
                    self.sinks.insert(global.id)
                } else {
                    false
                }
            }
            pw::types::ObjectType::Port => {
                let (Some(node), Some(direction)) = (
                    u32_property(props, *pw::keys::NODE_ID),
                    props.get(*pw::keys::PORT_DIRECTION),
                ) else {
                    return;
                };
                self.ports.insert(
                    global.id,
                    Port {
                        node,
                        direction: direction.to_owned(),
                        channel: props.get(*pw::keys::AUDIO_CHANNEL).map(str::to_owned),
                        ignore_latency: props.get("port.ignore-latency") == Some("true"),
                    },
                );
                node == self.stream.node_id() || self.playback.contains_key(&node)
            }
            pw::types::ObjectType::Link => {
                let Some(endpoints) = link_endpoints(props) else {
                    self.fail("PipeWire exposed a link with invalid endpoints".to_owned());
                    return;
                };
                let Some(serial) = u64_property(props, "object.serial") else {
                    self.fail("PipeWire exposed a link without an object serial".to_owned());
                    return;
                };
                if self.active && endpoints.input_node == self.stream.node_id() {
                    self.fail("unexpected link entered the Aercast audio capture node".to_owned());
                    return;
                }
                let proxy = match self.registry.bind::<pw::link::Link, _>(global) {
                    Ok(proxy) => proxy,
                    Err(error) => {
                        self.fail(format!("failed to inspect PipeWire link: {error}"));
                        return;
                    }
                };
                let id = global.id;
                let weak = weak.clone();
                let listener = proxy
                    .add_listener_local()
                    .info(move |info| {
                        if let Some(state) = weak.upgrade() {
                            state.borrow_mut().graph_link_info(id, info);
                        }
                    })
                    .register();
                self.graph_links.insert(
                    id,
                    GraphLink {
                        _listener: listener,
                        _proxy: proxy,
                        endpoints,
                        serial,
                        status: None,
                    },
                );
                route_relevant(endpoints, self.stream.node_id(), &self.playback)
            }
            _ => false,
        };
        if changed {
            self.changed();
        }
    }

    fn node_info(&mut self, id: u32, info: &pw::node::NodeInfoRef) {
        if info.id() != id {
            self.fail("PipeWire returned playback properties for the wrong node".to_owned());
            return;
        }
        if !info.change_mask().contains(pw::node::NodeChangeMask::PROPS) {
            return;
        }
        let policy = info.props().map(playback_policy).unwrap_or_default();
        let changed = self.playback.get_mut(&id).is_some_and(|playback| {
            if playback.policy == policy {
                false
            } else {
                playback.policy = policy;
                true
            }
        });
        if changed {
            self.changed();
        }
    }

    fn graph_link_info(&mut self, id: u32, info: &pw::link::LinkInfoRef) {
        if info.id() != id {
            self.fail("PipeWire returned link properties for the wrong object".to_owned());
            return;
        }
        let Some(link) = self.graph_links.get_mut(&id) else {
            return;
        };
        let endpoints = Endpoints {
            output_node: info.output_node_id(),
            output_port: info.output_port_id(),
            input_node: info.input_node_id(),
            input_port: info.input_port_id(),
        };
        if endpoints != link.endpoints {
            self.fail("PipeWire changed the endpoints of an existing link".to_owned());
            return;
        }
        if !info.change_mask().contains(pw::link::LinkChangeMask::STATE) {
            return;
        }
        let status = Some(link_status(info.state()));
        if status == link.status {
            return;
        }
        link.status = status;
        if route_relevant(endpoints, self.stream.node_id(), &self.playback) {
            self.changed();
        }
    }

    fn removed(&mut self, id: u32) {
        let changed_owned = self.links.iter_mut().any(|link| {
            if link.observed.is_some_and(|observed| observed.id == id) {
                link.observed = None;
                true
            } else {
                false
            }
        });
        let changed_node = self.playback.remove(&id).is_some()
            || self.sinks.remove(&id)
            || id == self.stream.node_id();
        let changed_port = self.ports.remove(&id).is_some_and(|port| {
            port.node == self.stream.node_id() || self.playback.contains_key(&port.node)
        });
        let changed_link = self.graph_links.remove(&id).is_some_and(|link| {
            route_relevant(link.endpoints, self.stream.node_id(), &self.playback)
                || (self.active && link.endpoints.input_node == self.stream.node_id())
        });
        if changed_owned || changed_node || changed_port || changed_link {
            self.changed();
        }
    }

    fn changed(&mut self) {
        if !self.initial_done || self.stopping || self.failure.is_some() {
            return;
        }
        self.dirty = true;
        let paused = self.stream.set_active(false);
        let flushed = self.stream.flush(false);
        if let Err(error) = paused {
            self.fail(format!("failed to pause selective audio: {error}"));
            return;
        }
        if let Err(error) = flushed {
            self.fail(format!("failed to flush paused selective audio: {error}"));
            return;
        }
        self.active = false;
        self.received_audio = false;
        self.activation_started = None;
        if self.pending.is_none() {
            self.sync(Phase::Rebuild);
        }
    }

    fn sync(&mut self, phase: Phase) {
        match self.core.sync(0) {
            Ok(seq) => self.pending = Some((seq, phase)),
            Err(error) => self.fail(format!("failed to synchronize PipeWire: {error}")),
        }
    }

    fn done(&mut self, id: u32, seq: AsyncSeq, weak: &Weak<RefCell<Self>>) {
        let Some((pending, phase)) = self.pending else {
            return;
        };
        if id != pw::core::PW_ID_CORE || seq != pending {
            return;
        }
        self.pending = None;
        match phase {
            Phase::Rebuild => {
                self.initial_done = true;
                self.dirty = false;
                self.rebuild(weak);
            }
            Phase::Links if self.dirty => {
                self.dirty = false;
                self.rebuild(weak);
            }
            Phase::Links => self.activate(),
            Phase::Cleanup => self.mainloop.quit(),
        }
    }

    fn rebuild(&mut self, weak: &Weak<RefCell<Self>>) {
        if let Err(error) = self.destroy_links() {
            self.fail(error);
            return;
        }
        if let Err(error) = validate_stream_properties(self.stream.properties().dict()) {
            self.fail(error);
            return;
        }
        let expected = match self.expected_links() {
            Ok(expected) => expected,
            Err(error) => {
                eprintln!("Selective audio remains silent: {error}");
                Vec::new()
            }
        };
        self.activation_started = (!expected.is_empty()).then(Instant::now);

        for endpoints in expected {
            let properties = pw::properties::properties! {
                *pw::keys::LINK_OUTPUT_NODE => endpoints.output_node.to_string(),
                *pw::keys::LINK_OUTPUT_PORT => endpoints.output_port.to_string(),
                *pw::keys::LINK_INPUT_NODE => endpoints.input_node.to_string(),
                *pw::keys::LINK_INPUT_PORT => endpoints.input_port.to_string(),
                *pw::keys::LINK_PASSIVE => "true",
                *pw::keys::OBJECT_LINGER => "false",
            };
            let link = match self
                .core
                .create_object::<pw::link::Link>("link-factory", &properties)
            {
                Ok(link) => link,
                Err(error) => {
                    self.fail(format!("failed to create passive audio link: {error}"));
                    return;
                }
            };
            let slot = self.links.len();
            let weak = weak.clone();
            let listener = link
                .add_listener_local()
                .info(move |info| {
                    let Some(state) = weak.upgrade() else { return };
                    let passive = info
                        .change_mask()
                        .contains(pw::link::LinkChangeMask::PROPS)
                        .then(|| {
                            info.props()
                                .and_then(|props| props.get(*pw::keys::LINK_PASSIVE))
                                == Some("true")
                        });
                    state.borrow_mut().link_info(
                        slot,
                        ObservedLink {
                            id: info.id(),
                            serial: info
                                .change_mask()
                                .contains(pw::link::LinkChangeMask::PROPS)
                                .then(|| {
                                    info.props()
                                        .and_then(|props| u64_property(props, "object.serial"))
                                })
                                .flatten(),
                            endpoints: Endpoints {
                                output_node: info.output_node_id(),
                                output_port: info.output_port_id(),
                                input_node: info.input_node_id(),
                                input_port: info.input_port_id(),
                            },
                            passive,
                            status: info
                                .change_mask()
                                .contains(pw::link::LinkChangeMask::STATE)
                                .then(|| link_status(info.state())),
                        },
                    );
                })
                .register();
            self.links.push(OwnedLink {
                _listener: listener,
                _proxy: link,
                expected: endpoints,
                observed: None,
            });
        }
        self.sync(Phase::Links);
    }

    fn expected_links(&self) -> Result<Vec<Endpoints>, String> {
        let input_node = self.stream.node_id();
        let input = stereo_ports(&self.ports, input_node, "in")?;
        if input
            .iter()
            .any(|id| !self.ports.get(id).is_some_and(|port| port.ignore_latency))
        {
            return Err("capture input ports did not retain ignore-latency=true".to_owned());
        }
        let mut links = Vec::new();
        for (&output_node, playback) in &self.playback {
            if excluded(&playback.policy, &self.exclusions) {
                continue;
            }
            let Some(identity) = playback.policy.identity.as_deref() else {
                eprintln!("playback node {output_node} has no stable application identity");
                continue;
            };
            let output = match stereo_ports(&self.ports, output_node, "out") {
                Ok(output) => output,
                Err(error) => {
                    eprintln!("Skipping selective audio for {identity}: {error}");
                    continue;
                }
            };
            if output.iter().any(|&output_port| {
                !self.graph_links.values().any(|link| {
                    active_sink_route(
                        link.endpoints,
                        link.status,
                        output_node,
                        output_port,
                        input_node,
                        &self.sinks,
                    )
                })
            }) {
                eprintln!(
                    "Skipping selective audio for {identity}: no active stereo speaker route"
                );
                continue;
            }
            for channel in 0..2 {
                links.push(Endpoints {
                    output_node,
                    output_port: output[channel],
                    input_node,
                    input_port: input[channel],
                });
            }
        }
        Ok(links)
    }

    fn link_info(&mut self, slot: usize, observed: ObservedLink) {
        let Some((expected, observed)) = self.links.get_mut(slot).map(|link| {
            let observed = ObservedLink {
                serial: observed
                    .serial
                    .or_else(|| link.observed.and_then(|previous| previous.serial)),
                passive: observed
                    .passive
                    .or_else(|| link.observed.and_then(|previous| previous.passive)),
                status: observed
                    .status
                    .or_else(|| link.observed.and_then(|previous| previous.status)),
                ..observed
            };
            link.observed = Some(observed);
            (link.expected, observed)
        }) else {
            return;
        };
        if self.active && !link_matches(expected, observed) {
            self.fail("active PipeWire link broke the passive exact-link contract".to_owned());
        } else if self.is_ready() && observed.status != Some(LinkStatus::Active) {
            self.changed();
        } else {
            self.maybe_ready();
        }
    }

    fn activate(&mut self) {
        let mut expected = HashMap::new();
        for link in &self.links {
            let Some(observed) = link
                .observed
                .filter(|observed| link_matches(link.expected, *observed))
            else {
                self.fail("PipeWire did not confirm every passive exact link".to_owned());
                return;
            };
            let Some(serial) = observed.serial else {
                self.fail("PipeWire did not confirm every Aercast link serial".to_owned());
                return;
            };
            if expected
                .insert(observed.id, (serial, observed.endpoints))
                .is_some()
            {
                self.fail("PipeWire reused an ID across Aercast audio links".to_owned());
                return;
            }
        }
        let actual: HashMap<_, _> = self
            .graph_links
            .iter()
            .filter(|(_, link)| link.endpoints.input_node == self.stream.node_id())
            .map(|(&id, link)| (id, (link.serial, link.endpoints)))
            .collect();
        if actual != expected {
            self.fail("unexpected link entered the Aercast audio capture node".to_owned());
            return;
        }
        if self.links.is_empty() {
            return;
        }
        if let Err(error) = self.stream.set_active(true) {
            self.fail(format!(
                "failed to activate verified selective audio: {error}"
            ));
            return;
        }
        self.active = true;
        self.received_audio = false;
        self.maybe_ready();
    }

    fn audio_buffer(&mut self) {
        self.received_audio = true;
        self.maybe_ready();
    }

    fn can_forward_audio(&self) -> bool {
        self.active
            && !self.links.is_empty()
            && self.links.iter().all(|link| {
                link.observed
                    .is_some_and(|observed| observed.status == Some(LinkStatus::Active))
            })
    }

    fn is_ready(&self) -> bool {
        self.active && self.activation_started.is_none()
    }

    fn maybe_ready(&mut self) {
        if self.is_ready() || !self.received_audio || !self.can_forward_audio() {
            return;
        }
        self.activation_started = None;
        println!(
            "Selective audio active: {} playback stream(s), {} verified passive links.",
            self.links.len() / 2,
            self.links.len()
        );
    }

    fn check_activation_timeout(&mut self) {
        if self
            .activation_started
            .is_some_and(|started| started.elapsed() >= Duration::from_secs(5))
        {
            self.fail(
                "selective-audio links produced no verified data within 5 seconds".to_owned(),
            );
        }
    }

    fn destroy_links(&mut self) -> Result<(), String> {
        let result = self
            .stream
            .set_active(false)
            .err()
            .map(|error| format!("failed to deactivate selective audio: {error}"));
        let flushed = self
            .stream
            .flush(false)
            .err()
            .map(|error| format!("failed to flush selective audio: {error}"));
        let result = result.or(flushed);
        self.active = false;
        self.received_audio = false;
        self.activation_started = None;
        self.links.clear();
        result.map_or(Ok(()), Err)
    }

    fn fail(&mut self, error: String) {
        if self.failure.is_none() {
            self.failure = Some(error);
            let _ = self.stream.set_active(false);
            let _ = self.stream.flush(false);
            self.active = false;
            self.activation_started = None;
        }
        self.mainloop.quit();
    }

    fn begin_cleanup(&mut self) -> Option<AsyncSeq> {
        self.stopping = true;
        if let Err(error) = self.destroy_links() {
            self.failure.get_or_insert(error);
        }
        if let Err(error) = self.stream.disconnect() {
            self.failure
                .get_or_insert_with(|| format!("failed to disconnect selective audio: {error}"));
        }
        match self.core.sync(0) {
            Ok(seq) => Some(seq),
            Err(error) => {
                self.failure
                    .get_or_insert_with(|| format!("failed to flush audio cleanup: {error}"));
                None
            }
        }
    }
}

fn link_status(state: pw::link::LinkState<'_>) -> LinkStatus {
    match state {
        pw::link::LinkState::Active => LinkStatus::Active,
        pw::link::LinkState::Paused => LinkStatus::Pending,
        pw::link::LinkState::Error(_) | pw::link::LinkState::Unlinked => LinkStatus::Error,
        pw::link::LinkState::Init
        | pw::link::LinkState::Negotiating
        | pw::link::LinkState::Allocating => LinkStatus::Pending,
    }
}

fn route_relevant(
    endpoints: Endpoints,
    capture_node: u32,
    playback: &HashMap<u32, Playback>,
) -> bool {
    endpoints.input_node != capture_node && playback.contains_key(&endpoints.output_node)
}

fn active_sink_route(
    endpoints: Endpoints,
    status: Option<LinkStatus>,
    playback_node: u32,
    playback_port: u32,
    capture_node: u32,
    sinks: &HashSet<u32>,
) -> bool {
    status == Some(LinkStatus::Active)
        && endpoints.output_node == playback_node
        && endpoints.output_port == playback_port
        && endpoints.input_node != capture_node
        && sinks.contains(&endpoints.input_node)
}

fn link_matches(expected: Endpoints, observed: ObservedLink) -> bool {
    matches!(
        observed.status,
        Some(LinkStatus::Pending | LinkStatus::Active)
    ) && observed.serial.is_some()
        && observed.passive == Some(true)
        && observed.endpoints == expected
}

fn run(
    appsrc: gst_app::AppSrc,
    exclusions: Vec<String>,
    stop: pw::channel::Receiver<()>,
) -> Result<(), String> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(|error| error.to_string())?;
    let context =
        pw::context::ContextRc::new(&mainloop, None).map_err(|error| error.to_string())?;
    let core = context
        .connect_rc(None)
        .map_err(|error| error.to_string())?;
    let registry = core.get_registry_rc().map_err(|error| error.to_string())?;
    let stream =
        pw::stream::StreamRc::new(core.clone(), "aercast-selective-audio", stream_properties())
            .map_err(|error| error.to_string())?;

    let state = Rc::new(RefCell::new(State {
        mainloop: mainloop.clone(),
        core: core.clone(),
        registry: registry.clone(),
        stream: stream.clone(),
        exclusions,
        playback: HashMap::new(),
        sinks: HashSet::new(),
        ports: HashMap::new(),
        graph_links: HashMap::new(),
        links: Vec::new(),
        pending: None,
        initial_done: false,
        dirty: false,
        active: false,
        received_audio: false,
        activation_started: None,
        stopping: false,
        failure: None,
    }));
    let weak = Rc::downgrade(&state);

    let core_listener = core
        .add_listener_local()
        .done({
            let weak = weak.clone();
            move |id, seq| {
                if let Some(state) = weak.upgrade() {
                    state.borrow_mut().done(id, seq, &weak);
                }
            }
        })
        .error({
            let weak = weak.clone();
            move |id, _, result, message| {
                if let Some(state) = weak.upgrade() {
                    if vanished_object(id, result) {
                        state.borrow_mut().changed();
                    } else {
                        state.borrow_mut().fail(format!(
                            "PipeWire core error on object {id} ({result}): {message}"
                        ));
                    }
                }
            }
        })
        .register();
    let registry_listener = registry
        .add_listener_local()
        .global({
            let weak = weak.clone();
            move |global| {
                if let Some(state) = weak.upgrade() {
                    state.borrow_mut().global(global, &weak);
                }
            }
        })
        .global_remove({
            let weak = weak.clone();
            move |id| {
                if let Some(state) = weak.upgrade() {
                    state.borrow_mut().removed(id);
                }
            }
        })
        .register();
    let stream_listener = stream
        .add_local_listener_with_user_data(appsrc)
        .state_changed({
            let weak = weak.clone();
            move |_, _, _, new| {
                if let pw::stream::StreamState::Error(error) = new
                    && let Some(state) = weak.upgrade()
                {
                    state
                        .borrow_mut()
                        .fail(format!("PipeWire audio stream failed: {error}"));
                }
            }
        })
        .process({
            let weak = weak.clone();
            move |stream, appsrc| {
                let Some(state) = weak.upgrade() else { return };
                if !state.borrow().can_forward_audio() {
                    return;
                }
                match push_audio(stream, appsrc) {
                    Ok(true) => {
                        state.borrow_mut().audio_buffer();
                    }
                    Ok(false) => {}
                    Err(error) => {
                        state.borrow_mut().fail(error);
                    }
                }
            }
        })
        .register()
        .map_err(|error| error.to_string())?;

    let stop_loop = mainloop.clone();
    let stop_listener = stop.attach(mainloop.loop_(), {
        let weak = weak.clone();
        move |_| {
            if let Some(state) = weak.upgrade() {
                state.borrow_mut().stopping = true;
            }
            stop_loop.quit();
        }
    });

    let format = audio_format();
    let mut params = [Pod::from_bytes(&format).expect("serialized audio format pod")];
    stream
        .connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::INACTIVE
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::DONT_RECONNECT,
            &mut params,
        )
        .map_err(|error| error.to_string())?;
    validate_stream_properties(stream.properties().dict())?;
    state.borrow_mut().sync(Phase::Rebuild);
    let readiness_timer = mainloop.loop_().add_timer({
        let weak = weak.clone();
        move |_| {
            if let Some(state) = weak.upgrade() {
                state.borrow_mut().check_activation_timeout();
            }
        }
    });
    readiness_timer
        .update_timer(Some(Duration::from_secs(1)), Some(Duration::from_secs(1)))
        .into_result()
        .map_err(|error| error.to_string())?;
    mainloop.run();

    let cleanup = state.borrow_mut().begin_cleanup();
    if let Some(seq) = cleanup {
        state.borrow_mut().pending = Some((seq, Phase::Cleanup));
        let cleanup_timeout = mainloop.loop_().add_timer({
            let weak = weak.clone();
            move |_| {
                if let Some(state) = weak.upgrade() {
                    state
                        .borrow_mut()
                        .fail("timed out while flushing selective-audio cleanup".to_owned());
                }
            }
        });
        cleanup_timeout
            .update_timer(Some(Duration::from_secs(5)), None)
            .into_result()
            .map_err(|error| error.to_string())?;
        mainloop.run();
    }
    drop(readiness_timer);
    drop(stop_listener);
    drop(stream_listener);
    drop(registry_listener);
    drop(core_listener);
    state.borrow_mut().failure.take().map_or(Ok(()), Err)
}

fn push_audio(stream: &pw::stream::Stream, appsrc: &gst_app::AppSrc) -> Result<bool, String> {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return Ok(false);
    };
    let Some(data) = buffer.datas_mut().first_mut() else {
        return Err("PipeWire audio buffer has no data plane".to_owned());
    };
    if data
        .chunk()
        .flags()
        .contains(spa::buffer::ChunkFlags::CORRUPTED)
    {
        return Ok(false);
    }
    let offset = data.chunk().offset() as usize;
    let size = data.chunk().size() as usize;
    if size == 0 {
        return Ok(false);
    }
    if !size.is_multiple_of(FRAME_BYTES) {
        return Err("PipeWire returned a partial stereo audio frame".to_owned());
    }
    let bytes = data
        .data()
        .and_then(|data| data.get(offset..offset.checked_add(size)?))
        .ok_or_else(|| "PipeWire audio chunk is outside its mapped buffer".to_owned())?
        .to_vec();
    let mut output = gst::Buffer::from_mut_slice(bytes);
    output
        .get_mut()
        .expect("new audio buffer is writable")
        .set_duration(gst::ClockTime::from_nseconds(
            (size / FRAME_BYTES) as u64 * 1_000_000_000 / 48_000,
        ));
    appsrc
        .push_buffer(output)
        .map_err(|error| format!("failed to feed selective audio to GStreamer: {error:?}"))?;
    Ok(true)
}

fn audio_format() -> Vec<u8> {
    let mut raw = spa::param::audio::AudioInfoRaw::new();
    raw.set_format(spa::param::audio::AudioFormat::F32LE);
    raw.set_rate(48_000);
    raw.set_channels(2);
    let mut positions = [0; spa::param::audio::MAX_CHANNELS];
    positions[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
    positions[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    raw.set_position(positions);
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: spa::param::ParamType::EnumFormat.as_raw(),
            properties: raw.into(),
        }),
    )
    .expect("static SPA pod serializes")
    .0
    .into_inner()
}

fn stream_properties() -> pw::properties::PropertiesBox {
    pw::properties::properties! {
        *pw::keys::APP_ID => "org.aercast.Aercast",
        *pw::keys::APP_NAME => "Aercast",
        *pw::keys::MEDIA_CLASS => "Stream/Input/Audio",
        *pw::keys::NODE_NAME => "aercast-selective-audio",
        *pw::keys::NODE_AUTOCONNECT => "false",
        *pw::keys::NODE_DONT_RECONNECT => "true",
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Screen",
        "node.dont-fallback" => "true",
        "node.dont-move" => "true",
        "port.ignore-latency" => "true",
        "state.restore-props" => "false",
        "state.restore-target" => "false",
    }
}

fn validate_stream_properties(props: &spa::utils::dict::DictRef) -> Result<(), String> {
    for (key, expected) in [
        (*pw::keys::MEDIA_CLASS, "Stream/Input/Audio"),
        (*pw::keys::MEDIA_TYPE, "Audio"),
        (*pw::keys::MEDIA_CATEGORY, "Capture"),
        (*pw::keys::MEDIA_ROLE, "Screen"),
        (*pw::keys::NODE_AUTOCONNECT, "false"),
        (*pw::keys::NODE_DONT_RECONNECT, "true"),
        ("node.dont-fallback", "true"),
        ("node.dont-move", "true"),
        ("port.ignore-latency", "true"),
        ("state.restore-props", "false"),
        ("state.restore-target", "false"),
    ] {
        if props.get(key) != Some(expected) {
            return Err(format!(
                "unsafe PipeWire override: {key} must remain {expected}"
            ));
        }
    }
    for key in [
        "target.object",
        "node.target",
        "target.node",
        "node.force-quantum",
        "node.force-rate",
        "node.lock-quantum",
        "node.lock-rate",
        "node.quantum",
        "node.rate",
        "node.latency",
        "node.max-latency",
        "node.driver",
        "node.always-process",
    ] {
        if props.get(key).is_some() {
            return Err(format!("unsafe PipeWire override set {key}"));
        }
    }
    Ok(())
}

fn validate_exported_node(props: &spa::utils::dict::DictRef) -> Result<(), String> {
    if props.get(*pw::keys::MEDIA_CLASS) != Some("Stream/Input/Audio") {
        return Err("PipeWire exported Aercast with an unsafe media class".to_owned());
    }
    for key in [
        "node.force-quantum",
        "node.force-rate",
        "node.lock-quantum",
        "node.lock-rate",
        "node.driver",
        "node.always-process",
    ] {
        if props.get(key).is_some() {
            return Err(format!("unsafe PipeWire node rule set {key}"));
        }
    }
    Ok(())
}

fn u32_property(props: &spa::utils::dict::DictRef, key: &str) -> Option<u32> {
    props.get(key)?.parse().ok()
}

fn u64_property(props: &spa::utils::dict::DictRef, key: &str) -> Option<u64> {
    props.get(key)?.parse().ok()
}

fn vanished_object(id: u32, result: i32) -> bool {
    id != pw::core::PW_ID_CORE
        && result.checked_neg().is_some_and(|errno| {
            io::Error::from_raw_os_error(errno).kind() == io::ErrorKind::NotFound
        })
}

fn link_endpoints(props: &spa::utils::dict::DictRef) -> Option<Endpoints> {
    Some(Endpoints {
        output_node: u32_property(props, *pw::keys::LINK_OUTPUT_NODE)?,
        output_port: u32_property(props, *pw::keys::LINK_OUTPUT_PORT)?,
        input_node: u32_property(props, *pw::keys::LINK_INPUT_NODE)?,
        input_port: u32_property(props, *pw::keys::LINK_INPUT_PORT)?,
    })
}

fn playback_identity(props: &spa::utils::dict::DictRef) -> Option<String> {
    [
        *pw::keys::APP_ID,
        *pw::keys::APP_PROCESS_BINARY,
        *pw::keys::APP_NAME,
    ]
    .into_iter()
    .find_map(|key| {
        props
            .get(key)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

fn playback_policy(props: &spa::utils::dict::DictRef) -> PlaybackPolicy {
    PlaybackPolicy {
        identity: playback_identity(props),
        communication: props.get(*pw::keys::MEDIA_ROLE) == Some("Communication"),
    }
}

fn excluded(policy: &PlaybackPolicy, exclusions: &[String]) -> bool {
    policy.communication
        || policy.identity.as_deref().is_some_and(|identity| {
            identity == "org.aercast.Aercast"
                || exclusions.iter().any(|excluded| excluded == identity)
        })
}

fn stereo_ports(
    ports: &HashMap<u32, Port>,
    node: u32,
    direction: &str,
) -> Result<[u32; 2], String> {
    let mut result = [None, None];
    for (&id, port) in ports
        .iter()
        .filter(|(_, port)| port.node == node && port.direction == direction)
    {
        let slot = match port.channel.as_deref() {
            Some("FL") => 0,
            Some("FR") => 1,
            Some(channel) => return Err(format!("unsupported {channel} channel")),
            None => continue,
        };
        if result[slot].replace(id).is_some() {
            return Err(format!(
                "duplicate {} port",
                if slot == 0 { "FL" } else { "FR" }
            ));
        }
    }
    match result {
        [Some(left), Some(right)] => Ok([left, right]),
        _ => Err("waiting for exactly one FL and one FR port".to_owned()),
    }
}

#[cfg(test)]
#[path = "audio_tests.rs"]
mod tests;
