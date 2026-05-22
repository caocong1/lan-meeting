use anyhow::{Context, Result, anyhow};
use lan_meeting_lib::decoder::{DecoderConfig, OutputFormat, VideoDecoder};
use lan_meeting_lib::diagnostics::{self, ScreenDiagnosticsSnapshot};
use lan_meeting_lib::network::protocol::{
    self, FrameType as ProtocolFrameType, Message, MessageType,
};
use lan_meeting_lib::network::quic::{
    DEFAULT_PORT, QuicConfig, QuicConnection, QuicEndpoint, QuicStream,
};
use lan_meeting_lib::streaming::{Quality, StreamingConfig, StreamingManager};
use parking_lot::Mutex;
use quinn::RecvStream;
use serde::Serialize;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;

#[tokio::main]
async fn main() -> Result<()> {
    if env::args().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", help_text());
        return Ok(());
    }

    let args = Args::parse()?;
    init_logger(args.verbose);
    diagnostics::reset();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let started = Instant::now();
    let role = args.mode.as_str();
    let peer = args.peer.clone();
    let bind_addr = args.bind_addr.to_string();

    let result = match args.mode {
        Mode::Sharer => run_sharer(&args)
            .await
            .map(|sharer| (true, Some(sharer), None)),
        Mode::Viewer => run_viewer(&args).await.map(|viewer| {
            (
                viewer.decoded_frame_count >= args.frames,
                None,
                Some(viewer),
            )
        }),
    };

    let (ok, sharer, viewer, error) = match result {
        Ok((ok, sharer, viewer)) => (ok, sharer, viewer, None),
        Err(error) => {
            log::error!("probe failed: {:#}", error);
            (false, None, None, Some(format!("{:#}", error)))
        }
    };

    let report = ProbeReport {
        role,
        ok,
        elapsed_ms: started.elapsed().as_millis(),
        bind_addr,
        peer,
        diagnostics: diagnostics::snapshot(),
        sharer,
        viewer,
        error,
    };

    println!("{}", serde_json::to_string_pretty(&report)?);

    if ok {
        Ok(())
    } else {
        std::process::exit(2);
    }
}

async fn run_sharer(args: &Args) -> Result<SharerReport> {
    let endpoint = Arc::new(
        QuicEndpoint::new(QuicConfig {
            bind_addr: args.bind_addr,
            ..QuicConfig::default()
        })
        .await
        .context("failed to start QUIC endpoint")?,
    );

    let local_addr = endpoint.local_addr();
    lan_meeting_lib::set_quic_endpoint(endpoint.clone());
    endpoint.clone().start_server(|conn| {
        log::info!(
            "probe sharer accepted connection from {}",
            conn.remote_addr()
        );
        tokio::spawn(async move {
            lan_meeting_lib::handle_incoming_connection(conn).await;
        });
    });

    let capture =
        lan_meeting_lib::capture::create_capture().context("failed to create screen capture")?;
    let config = StreamingConfig {
        fps: args.fps,
        quality: args.quality,
        display_id: args.display_id,
    };

    let manager_arc = lan_meeting_lib::streaming::get_streaming_manager();
    {
        let mut manager = manager_arc.write();
        if manager.is_none() {
            *manager = Some(StreamingManager::new());
        }
        let Some(manager) = manager.as_mut() else {
            return Err(anyhow!("failed to initialize global streaming manager"));
        };
        manager
            .start_sync(config, capture)
            .map_err(|error| anyhow!("failed to start streaming: {}", error))?;
    }

    log::info!(
        "probe sharer running on {} for {}ms",
        local_addr,
        args.duration.as_millis()
    );
    tokio::time::sleep(args.duration).await;

    let manager_frame_count_before_stop = manager_arc
        .read()
        .as_ref()
        .map(|manager| manager.frame_count())
        .unwrap_or(0);
    let is_streaming_after_stop = {
        let mut manager = manager_arc.write();
        if let Some(manager) = manager.as_mut() {
            manager.stop_sync();
            manager.is_streaming()
        } else {
            false
        }
    };
    tokio::time::sleep(Duration::from_millis(500)).await;

    endpoint.close();
    endpoint.wait_idle().await;

    Ok(SharerReport {
        local_addr: local_addr.to_string(),
        display_id: args.display_id,
        fps: args.fps,
        quality: quality_name(args.quality).to_string(),
        duration_ms: args.duration.as_millis(),
        manager_frame_count_before_stop,
        is_streaming_after_stop,
    })
}

async fn run_viewer(args: &Args) -> Result<HeadlessViewerReport> {
    let peer = args
        .peer
        .as_deref()
        .ok_or_else(|| anyhow!("--peer is required in viewer mode"))?;
    let peer_addr = parse_peer_addr(peer)?;

    let endpoint = Arc::new(
        QuicEndpoint::new(QuicConfig {
            bind_addr: args.bind_addr,
            ..QuicConfig::default()
        })
        .await
        .context("failed to start viewer QUIC endpoint")?,
    );
    lan_meeting_lib::set_quic_endpoint(endpoint.clone());

    let conn = endpoint
        .connect(peer_addr)
        .await
        .with_context(|| format!("failed to connect to {}", peer_addr))?;
    log::info!("probe viewer connected to {}", conn.remote_addr());

    let state = Arc::new(HeadlessViewerState::new(args.frames));
    spawn_viewer_accept_loop(conn.clone(), state.clone());

    send_probe_handshake(&conn).await?;
    send_screen_request(&conn, args.display_id, args.fps).await?;

    wait_for_viewer_result(state.clone(), args.duration).await;

    conn.close();
    endpoint.close();
    endpoint.wait_idle().await;

    Ok(state.report.lock().clone())
}

fn spawn_viewer_accept_loop(conn: Arc<QuicConnection>, state: Arc<HeadlessViewerState>) {
    let bi_conn = conn.clone();
    let bi_state = state.clone();
    tokio::spawn(async move {
        loop {
            match bi_conn.accept_bi_stream().await {
                Ok(stream) => {
                    let remote = bi_conn.remote_addr().to_string();
                    let state = bi_state.clone();
                    tokio::spawn(async move {
                        handle_viewer_stream(stream, remote, state).await;
                    });
                }
                Err(error) => {
                    log::info!("viewer bidirectional accept loop ended: {}", error);
                    break;
                }
            }
        }
    });

    tokio::spawn(async move {
        loop {
            match conn.accept_uni_stream().await {
                Ok(stream) => {
                    let remote = conn.remote_addr().to_string();
                    let state = state.clone();
                    handle_viewer_uni_stream(stream, remote, state).await;
                }
                Err(error) => {
                    log::info!("viewer unidirectional accept loop ended: {}", error);
                    break;
                }
            }
        }
    });
}

async fn send_probe_handshake(conn: &Arc<QuicConnection>) -> Result<()> {
    let device_id = lan_meeting_lib::network::discovery::get_our_device_id();
    let name = hostname::get()
        .map(|name| format!("e2e-probe-{}", name.to_string_lossy()))
        .unwrap_or_else(|_| "e2e-probe".to_string());
    let handshake = protocol::create_handshake(device_id, &name);
    let encoded = protocol::encode(&handshake).context("failed to encode handshake")?;

    let mut stream = conn
        .open_bi_stream()
        .await
        .context("failed to open handshake stream")?;
    stream
        .send_framed(&encoded)
        .await
        .context("failed to send handshake")?;

    let response = tokio::time::timeout(Duration::from_secs(5), stream.recv_framed())
        .await
        .context("handshake ack timed out")?
        .context("failed to receive handshake ack")?;
    let ack = protocol::decode(&response).context("failed to decode handshake ack")?;
    match ack {
        Message::HandshakeAck {
            accepted: true,
            name,
            ..
        } => {
            log::info!("probe handshake accepted by {}", name);
            Ok(())
        }
        Message::HandshakeAck {
            accepted: false,
            reason,
            ..
        } => Err(anyhow!(
            "handshake rejected: {}",
            reason.unwrap_or_default()
        )),
        other => Err(anyhow!(
            "unexpected handshake response: {:?}",
            other.message_type()
        )),
    }
}

async fn send_screen_request(conn: &Arc<QuicConnection>, display_id: u32, fps: u32) -> Result<()> {
    let request = Message::ScreenRequest {
        display_id,
        preferred_fps: fps.min(u8::MAX as u32) as u8,
        preferred_quality: 80,
    };
    let encoded = protocol::encode(&request).context("failed to encode ScreenRequest")?;

    let mut stream = conn
        .open_bi_stream()
        .await
        .context("failed to open ScreenRequest stream")?;
    stream
        .send_framed(&encoded)
        .await
        .context("failed to send ScreenRequest")?;
    stream
        .finish()
        .await
        .context("failed to finish ScreenRequest stream")?;

    log::info!("probe viewer sent ScreenRequest");
    Ok(())
}

async fn wait_for_viewer_result(state: Arc<HeadlessViewerState>, duration: Duration) {
    let deadline = Instant::now() + duration;
    loop {
        if state.report.lock().decoded_frame_count >= state.target_frames {
            return;
        }

        let now = Instant::now();
        if now >= deadline {
            return;
        }

        let wait_for = (deadline - now).min(Duration::from_millis(500));
        tokio::select! {
            _ = state.notify.notified() => {}
            _ = tokio::time::sleep(wait_for) => {}
        }
    }
}

async fn handle_viewer_stream(
    mut stream: QuicStream,
    remote_addr: String,
    state: Arc<HeadlessViewerState>,
) {
    state.report.lock().accepted_stream_count += 1;
    let mut is_first_payload = true;

    loop {
        let data = match tokio::time::timeout(Duration::from_secs(10), stream.recv_framed()).await {
            Ok(Ok(data)) => data,
            Ok(Err(error)) => {
                log::debug!("viewer stream from {} ended: {}", remote_addr, error);
                break;
            }
            Err(_) => {
                log::debug!("viewer stream from {} idle timeout", remote_addr);
                break;
            }
        };

        let decoded = protocol::decode(&data);
        if is_first_payload {
            diagnostics::record_incoming_stream_first(
                &remote_addr,
                data.len(),
                decoded.as_ref().map(Message::message_type).ok(),
            );
            is_first_payload = false;
        }

        match decoded {
            Ok(message) => {
                state.report.lock().protocol_message_count += 1;
                process_viewer_message(message, &state);
            }
            Err(error) => {
                state.report.lock().malformed_message_count += 1;
                log::warn!(
                    "failed to decode viewer stream payload from {} ({} bytes): {}",
                    remote_addr,
                    data.len(),
                    error
                );
            }
        }
    }
}

async fn handle_viewer_uni_stream(
    mut stream: RecvStream,
    remote_addr: String,
    state: Arc<HeadlessViewerState>,
) {
    state.report.lock().accepted_stream_count += 1;
    let mut is_first_payload = true;

    loop {
        let data = match tokio::time::timeout(
            Duration::from_secs(10),
            lan_meeting_lib::network::quic::recv_uni_framed(&mut stream),
        )
        .await
        {
            Ok(Ok(data)) => data,
            Ok(Err(error)) => {
                log::debug!("viewer uni stream from {} ended: {}", remote_addr, error);
                break;
            }
            Err(_) => {
                log::debug!("viewer uni stream from {} idle timeout", remote_addr);
                break;
            }
        };

        if data.is_empty() {
            log::trace!(
                "viewer uni stream from {} received warmup frame",
                remote_addr
            );
            continue;
        }

        let decoded = protocol::decode(&data);
        if is_first_payload {
            diagnostics::record_incoming_stream_first(
                &remote_addr,
                data.len(),
                decoded.as_ref().map(Message::message_type).ok(),
            );
            is_first_payload = false;
        }

        match decoded {
            Ok(message) => {
                state.report.lock().protocol_message_count += 1;
                process_viewer_message(message, &state);
            }
            Err(error) => {
                state.report.lock().malformed_message_count += 1;
                log::warn!(
                    "failed to decode viewer uni payload from {} ({} bytes): {}",
                    remote_addr,
                    data.len(),
                    error
                );
            }
        }
    }
}

fn process_viewer_message(message: Message, state: &Arc<HeadlessViewerState>) {
    match message {
        Message::ScreenStart {
            width,
            height,
            fps,
            codec,
        } => {
            log::info!(
                "probe viewer received ScreenStart: {}x{} @ {}fps codec={}",
                width,
                height,
                fps,
                codec
            );

            match lan_meeting_lib::decoder::create_decoder().and_then(|mut decoder| {
                decoder.init(DecoderConfig {
                    width,
                    height,
                    output_format: OutputFormat::BGRA,
                })?;
                Ok(decoder)
            }) {
                Ok(decoder) => {
                    *state.decoder.lock() = Some(decoder);
                    let mut report = state.report.lock();
                    report.screen_start_count += 1;
                    report.last_start_width = Some(width);
                    report.last_start_height = Some(height);
                    report.last_start_fps = Some(fps);
                    report.last_start_codec = Some(codec);
                }
                Err(error) => {
                    let mut report = state.report.lock();
                    report.decoder_init_error_count += 1;
                    report.last_decoder_error = Some(error.to_string());
                    log::error!("probe viewer decoder init failed: {}", error);
                }
            }
        }
        Message::ScreenFrame {
            timestamp,
            frame_type,
            sequence,
            data,
        } => {
            diagnostics::record_screen_frame_received(sequence, data.len());
            update_received_frame_report(state, sequence, frame_type, data.len());

            let decoded = {
                let mut decoder_guard = state.decoder.lock();
                let Some(decoder) = decoder_guard.as_mut() else {
                    state.report.lock().no_decoder_frame_count += 1;
                    log::warn!(
                        "probe viewer received frame {} before decoder init",
                        sequence
                    );
                    return;
                };
                decoder.decode(&data, timestamp)
            };

            match decoded {
                Ok(Some(frame)) => {
                    diagnostics::record_viewer_decoded_frame(sequence);
                    let pixel_stats = decoded_pixel_stats(&frame);

                    let mut report = state.report.lock();
                    report.decoded_frame_count += 1;
                    report.last_decoded_width = Some(frame.width);
                    report.last_decoded_height = Some(frame.height);
                    if report.first_decoded_pixel_stats.is_none() {
                        report.first_decoded_pixel_stats = pixel_stats.clone();
                    }
                    report.last_decoded_pixel_stats = pixel_stats;

                    if report.decoded_frame_count >= state.target_frames {
                        state.notify.notify_waiters();
                    }
                }
                Ok(None) => {
                    diagnostics::record_viewer_decoder_none(sequence);
                    state.report.lock().decoder_none_count += 1;
                }
                Err(error) => {
                    diagnostics::record_viewer_decode_error(sequence, &error.to_string());
                    let mut report = state.report.lock();
                    report.decode_error_count += 1;
                    report.last_decoder_error = Some(error.to_string());
                }
            }
        }
        Message::ScreenStop => {
            log::info!("probe viewer received ScreenStop");
            state.report.lock().screen_stop_count += 1;
            state.notify.notify_waiters();
        }
        other => {
            let message_type = other.message_type();
            if message_type != MessageType::ScreenOffer {
                log::debug!("probe viewer ignored message {:?}", message_type);
            }
            state.report.lock().ignored_message_count += 1;
        }
    }
}

fn update_received_frame_report(
    state: &Arc<HeadlessViewerState>,
    sequence: u32,
    frame_type: ProtocolFrameType,
    bytes: usize,
) {
    let mut report = state.report.lock();
    report.screen_frame_count += 1;
    report.screen_frame_bytes += bytes as u64;
    report.first_sequence.get_or_insert(sequence);
    report.last_sequence = Some(sequence);
    match frame_type {
        ProtocolFrameType::KeyFrame => report.keyframe_count += 1,
        ProtocolFrameType::DeltaFrame => report.delta_frame_count += 1,
    }
}

fn decoded_pixel_stats(
    frame: &lan_meeting_lib::decoder::DecodedFrame,
) -> Option<DecodedPixelStats> {
    let data = frame.cpu_data()?;
    if frame.format != OutputFormat::BGRA {
        return None;
    }

    let mut rgb_sum = 0u64;
    let mut rgb_nonzero_pixels = 0u64;
    let mut sampled_pixels = 0u64;

    for pixel in data.chunks_exact(4) {
        let b = pixel[0] as u64;
        let g = pixel[1] as u64;
        let r = pixel[2] as u64;
        rgb_sum += r + g + b;
        if r != 0 || g != 0 || b != 0 {
            rgb_nonzero_pixels += 1;
        }
        sampled_pixels += 1;
    }

    Some(DecodedPixelStats {
        width: frame.width,
        height: frame.height,
        rgb_sum,
        rgb_nonzero_pixels,
        sampled_pixels,
    })
}

fn parse_peer_addr(peer: &str) -> Result<SocketAddr> {
    if let Ok(addr) = peer.parse() {
        return Ok(addr);
    }
    format!("{}:{}", peer, DEFAULT_PORT)
        .parse()
        .with_context(|| format!("invalid peer address: {}", peer))
}

fn quality_name(quality: Quality) -> &'static str {
    match quality {
        Quality::Auto => "auto",
        Quality::High => "high",
        Quality::Medium => "medium",
        Quality::Low => "low",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Sharer,
    Viewer,
}

impl Mode {
    fn as_str(&self) -> &'static str {
        match self {
            Mode::Sharer => "sharer",
            Mode::Viewer => "viewer",
        }
    }
}

#[derive(Debug, Clone)]
struct Args {
    mode: Mode,
    peer: Option<String>,
    bind_addr: SocketAddr,
    duration: Duration,
    frames: u64,
    display_id: u32,
    fps: u32,
    quality: Quality,
    verbose: bool,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut mode = None;
        let mut peer = None;
        let mut bind_addr = None;
        let mut duration = Duration::from_millis(15_000);
        let mut frames = 30u64;
        let mut display_id = 0u32;
        let mut fps = 30u32;
        let mut quality = Quality::Auto;
        let mut verbose = false;

        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--mode" => {
                    mode = Some(match take_value(&mut args, "--mode")?.as_str() {
                        "sharer" => Mode::Sharer,
                        "viewer" => Mode::Viewer,
                        value => return Err(anyhow!("invalid --mode: {}", value)),
                    });
                }
                "--peer" => peer = Some(take_value(&mut args, "--peer")?),
                "--bind" => {
                    let value = take_value(&mut args, "--bind")?;
                    bind_addr = Some(
                        value
                            .parse()
                            .with_context(|| format!("invalid --bind address: {}", value))?,
                    );
                }
                "--duration-ms" => {
                    let value = take_value(&mut args, "--duration-ms")?;
                    duration = Duration::from_millis(value.parse()?);
                }
                "--duration-seconds" => {
                    let value = take_value(&mut args, "--duration-seconds")?;
                    duration = Duration::from_secs(value.parse()?);
                }
                "--frames" => {
                    let value = take_value(&mut args, "--frames")?;
                    frames = value.parse()?;
                }
                "--display-id" => {
                    let value = take_value(&mut args, "--display-id")?;
                    display_id = value.parse()?;
                }
                "--fps" => {
                    let value = take_value(&mut args, "--fps")?;
                    fps = value.parse()?;
                }
                "--quality" => {
                    quality = match take_value(&mut args, "--quality")?.as_str() {
                        "auto" => Quality::Auto,
                        "high" => Quality::High,
                        "medium" => Quality::Medium,
                        "low" => Quality::Low,
                        value => return Err(anyhow!("invalid --quality: {}", value)),
                    };
                }
                "--json" => {}
                "--verbose" => verbose = true,
                value => return Err(anyhow!("unknown argument: {}", value)),
            }
        }

        let mode = mode.ok_or_else(|| anyhow!("--mode is required"))?;
        let bind_addr = bind_addr.unwrap_or_else(|| match mode {
            Mode::Sharer => format!("0.0.0.0:{}", DEFAULT_PORT).parse().unwrap(),
            Mode::Viewer => "0.0.0.0:0".parse().unwrap(),
        });

        Ok(Self {
            mode,
            peer,
            bind_addr,
            duration,
            frames,
            display_id,
            fps,
            quality,
            verbose,
        })
    }
}

fn take_value(args: &mut impl Iterator<Item = String>, name: &str) -> Result<String> {
    args.next()
        .ok_or_else(|| anyhow!("{} requires a value", name))
}

#[derive(Serialize)]
struct ProbeReport {
    role: &'static str,
    ok: bool,
    elapsed_ms: u128,
    bind_addr: String,
    peer: Option<String>,
    diagnostics: ScreenDiagnosticsSnapshot,
    sharer: Option<SharerReport>,
    viewer: Option<HeadlessViewerReport>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SharerReport {
    local_addr: String,
    display_id: u32,
    fps: u32,
    quality: String,
    duration_ms: u128,
    manager_frame_count_before_stop: u32,
    is_streaming_after_stop: bool,
}

struct HeadlessViewerState {
    report: Mutex<HeadlessViewerReport>,
    decoder: Mutex<Option<Box<dyn VideoDecoder>>>,
    notify: Notify,
    target_frames: u64,
}

impl HeadlessViewerState {
    fn new(target_frames: u64) -> Self {
        Self {
            report: Mutex::new(HeadlessViewerReport::default()),
            decoder: Mutex::new(None),
            notify: Notify::new(),
            target_frames,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize)]
struct HeadlessViewerReport {
    accepted_stream_count: u64,
    protocol_message_count: u64,
    malformed_message_count: u64,
    ignored_message_count: u64,

    screen_start_count: u64,
    screen_stop_count: u64,
    last_start_width: Option<u32>,
    last_start_height: Option<u32>,
    last_start_fps: Option<u8>,
    last_start_codec: Option<String>,

    screen_frame_count: u64,
    screen_frame_bytes: u64,
    keyframe_count: u64,
    delta_frame_count: u64,
    first_sequence: Option<u32>,
    last_sequence: Option<u32>,

    no_decoder_frame_count: u64,
    decoder_init_error_count: u64,
    decoder_none_count: u64,
    decoded_frame_count: u64,
    decode_error_count: u64,
    last_decoder_error: Option<String>,
    last_decoded_width: Option<u32>,
    last_decoded_height: Option<u32>,
    first_decoded_pixel_stats: Option<DecodedPixelStats>,
    last_decoded_pixel_stats: Option<DecodedPixelStats>,

    render_upload_count: u64,
}

#[derive(Debug, Clone, Serialize)]
struct DecodedPixelStats {
    width: u32,
    height: u32,
    rgb_sum: u64,
    rgb_nonzero_pixels: u64,
    sampled_pixels: u64,
}

struct ProbeLogger;

static PROBE_LOGGER: ProbeLogger = ProbeLogger;

impl log::Log for ProbeLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            let millis = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or(0);
            eprintln!(
                "{} {:<5} [{}] {}",
                millis,
                record.level(),
                record.target(),
                record.args()
            );
        }
    }

    fn flush(&self) {}
}

fn init_logger(verbose: bool) {
    let level = if verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };

    if log::set_logger(&PROBE_LOGGER).is_ok() {
        log::set_max_level(level);
    }
}

fn help_text() -> &'static str {
    "LAN Meeting e2e screen probe\n\
\n\
USAGE:\n\
  cargo run --manifest-path src-tauri/Cargo.toml --bin e2e_probe -- --mode sharer [options]\n\
  cargo run --manifest-path src-tauri/Cargo.toml --bin e2e_probe -- --mode viewer --peer <ip[:port]> [options]\n\
\n\
OPTIONS:\n\
  --mode sharer|viewer       Probe role\n\
  --peer <ip[:port]>         Sharer address for viewer mode; default port is 19876\n\
  --bind <ip:port>           Bind address; sharer default 0.0.0.0:19876, viewer default 0.0.0.0:0\n\
  --duration-ms <ms>         Probe duration, default 15000\n\
  --duration-seconds <sec>   Probe duration in seconds\n\
  --frames <count>           Viewer target decoded frame count, default 30\n\
  --display-id <id>          Display id, default 0\n\
  --fps <fps>                Requested/streaming fps, default 30\n\
  --quality auto|high|medium|low\n\
  --json                     Accepted for scripts; output is always JSON\n\
  --verbose                  Include debug logs on stderr\n"
}
