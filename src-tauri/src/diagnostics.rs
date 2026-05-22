//! Runtime counters for screen-sharing diagnostics.
//!
//! These counters are intentionally process-local and cheap to update from hot
//! paths. The e2e probe prints the snapshot as JSON so we can identify where a
//! black-screen run stops without opening the UI.

use crate::network::protocol::MessageType;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const UNSET_SEQUENCE: u64 = u64::MAX;

static INCOMING_STREAM_COUNT: AtomicU64 = AtomicU64::new(0);
static INCOMING_STREAM_PROTOCOL_FIRST_COUNT: AtomicU64 = AtomicU64::new(0);
static INCOMING_STREAM_UNKNOWN_FIRST_COUNT: AtomicU64 = AtomicU64::new(0);
static INCOMING_FIRST_PAYLOAD_BYTES: AtomicU64 = AtomicU64::new(0);
static ACCEPTED_BI_STREAM_COUNT: AtomicU64 = AtomicU64::new(0);
static ACCEPTED_UNI_STREAM_COUNT: AtomicU64 = AtomicU64::new(0);

static SCREEN_FRAME_RECEIVED_COUNT: AtomicU64 = AtomicU64::new(0);
static SCREEN_FRAME_RECEIVED_BYTES: AtomicU64 = AtomicU64::new(0);
static FIRST_SCREEN_FRAME_SEQUENCE: AtomicU64 = AtomicU64::new(UNSET_SEQUENCE);
static LAST_SCREEN_FRAME_SEQUENCE: AtomicU64 = AtomicU64::new(UNSET_SEQUENCE);

static VIEWER_DECODER_NONE_COUNT: AtomicU64 = AtomicU64::new(0);
static VIEWER_DECODED_FRAME_COUNT: AtomicU64 = AtomicU64::new(0);
static VIEWER_DECODE_ERROR_COUNT: AtomicU64 = AtomicU64::new(0);
static VIEWER_RENDER_SUCCESS_COUNT: AtomicU64 = AtomicU64::new(0);
static VIEWER_RENDER_FAILURE_COUNT: AtomicU64 = AtomicU64::new(0);

static SHARER_CAPTURE_FRAME_COUNT: AtomicU64 = AtomicU64::new(0);
static SHARER_CAPTURE_ERROR_COUNT: AtomicU64 = AtomicU64::new(0);
static SHARER_ENCODED_PACKET_COUNT: AtomicU64 = AtomicU64::new(0);
static SHARER_ENCODED_BYTES: AtomicU64 = AtomicU64::new(0);
static SHARER_EMPTY_PACKET_COUNT: AtomicU64 = AtomicU64::new(0);
static SHARER_FRAME_STREAM_OPEN_COUNT: AtomicU64 = AtomicU64::new(0);
static SHARER_FRAME_STREAM_OPEN_FAILURE_COUNT: AtomicU64 = AtomicU64::new(0);
static SHARER_FRAME_SEND_SUCCESS_COUNT: AtomicU64 = AtomicU64::new(0);
static SHARER_FRAME_SEND_FAILURE_COUNT: AtomicU64 = AtomicU64::new(0);
static SHARER_FRAME_SEND_BYTES: AtomicU64 = AtomicU64::new(0);
static SHARER_FRAME_SEND_WIRE_BYTES: AtomicU64 = AtomicU64::new(0);
static SHARER_FRAME_SEND_DURATION_MICROS: AtomicU64 = AtomicU64::new(0);
static FIRST_KEYFRAME_SEQUENCE: AtomicU64 = AtomicU64::new(UNSET_SEQUENCE);
static FIRST_KEYFRAME_BYTES: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize)]
pub struct ScreenDiagnosticsSnapshot {
    pub incoming_stream_count: u64,
    pub incoming_stream_protocol_first_count: u64,
    pub incoming_stream_unknown_first_count: u64,
    pub incoming_first_payload_bytes: u64,
    pub accepted_bi_stream_count: u64,
    pub accepted_uni_stream_count: u64,

    pub screen_frame_received_count: u64,
    pub screen_frame_received_bytes: u64,
    pub first_screen_frame_sequence: Option<u64>,
    pub last_screen_frame_sequence: Option<u64>,

    pub viewer_decoder_none_count: u64,
    pub viewer_decoded_frame_count: u64,
    pub viewer_decode_error_count: u64,
    pub viewer_render_success_count: u64,
    pub viewer_render_failure_count: u64,

    pub sharer_capture_frame_count: u64,
    pub sharer_capture_error_count: u64,
    pub sharer_encoded_packet_count: u64,
    pub sharer_encoded_bytes: u64,
    pub sharer_empty_packet_count: u64,
    pub sharer_frame_stream_open_count: u64,
    pub sharer_frame_stream_open_failure_count: u64,
    pub sharer_frame_send_success_count: u64,
    pub sharer_frame_send_failure_count: u64,
    pub sharer_frame_send_bytes: u64,
    pub sharer_frame_send_wire_bytes: u64,
    pub sharer_frame_send_duration_micros: u64,
    pub first_keyframe_sequence: Option<u64>,
    pub first_keyframe_bytes: Option<u64>,
}

pub fn reset() {
    INCOMING_STREAM_COUNT.store(0, Ordering::Relaxed);
    INCOMING_STREAM_PROTOCOL_FIRST_COUNT.store(0, Ordering::Relaxed);
    INCOMING_STREAM_UNKNOWN_FIRST_COUNT.store(0, Ordering::Relaxed);
    INCOMING_FIRST_PAYLOAD_BYTES.store(0, Ordering::Relaxed);
    ACCEPTED_BI_STREAM_COUNT.store(0, Ordering::Relaxed);
    ACCEPTED_UNI_STREAM_COUNT.store(0, Ordering::Relaxed);

    SCREEN_FRAME_RECEIVED_COUNT.store(0, Ordering::Relaxed);
    SCREEN_FRAME_RECEIVED_BYTES.store(0, Ordering::Relaxed);
    FIRST_SCREEN_FRAME_SEQUENCE.store(UNSET_SEQUENCE, Ordering::Relaxed);
    LAST_SCREEN_FRAME_SEQUENCE.store(UNSET_SEQUENCE, Ordering::Relaxed);

    VIEWER_DECODER_NONE_COUNT.store(0, Ordering::Relaxed);
    VIEWER_DECODED_FRAME_COUNT.store(0, Ordering::Relaxed);
    VIEWER_DECODE_ERROR_COUNT.store(0, Ordering::Relaxed);
    VIEWER_RENDER_SUCCESS_COUNT.store(0, Ordering::Relaxed);
    VIEWER_RENDER_FAILURE_COUNT.store(0, Ordering::Relaxed);

    SHARER_CAPTURE_FRAME_COUNT.store(0, Ordering::Relaxed);
    SHARER_CAPTURE_ERROR_COUNT.store(0, Ordering::Relaxed);
    SHARER_ENCODED_PACKET_COUNT.store(0, Ordering::Relaxed);
    SHARER_ENCODED_BYTES.store(0, Ordering::Relaxed);
    SHARER_EMPTY_PACKET_COUNT.store(0, Ordering::Relaxed);
    SHARER_FRAME_STREAM_OPEN_COUNT.store(0, Ordering::Relaxed);
    SHARER_FRAME_STREAM_OPEN_FAILURE_COUNT.store(0, Ordering::Relaxed);
    SHARER_FRAME_SEND_SUCCESS_COUNT.store(0, Ordering::Relaxed);
    SHARER_FRAME_SEND_FAILURE_COUNT.store(0, Ordering::Relaxed);
    SHARER_FRAME_SEND_BYTES.store(0, Ordering::Relaxed);
    SHARER_FRAME_SEND_WIRE_BYTES.store(0, Ordering::Relaxed);
    SHARER_FRAME_SEND_DURATION_MICROS.store(0, Ordering::Relaxed);
    FIRST_KEYFRAME_SEQUENCE.store(UNSET_SEQUENCE, Ordering::Relaxed);
    FIRST_KEYFRAME_BYTES.store(0, Ordering::Relaxed);
}

pub fn snapshot() -> ScreenDiagnosticsSnapshot {
    ScreenDiagnosticsSnapshot {
        incoming_stream_count: INCOMING_STREAM_COUNT.load(Ordering::Relaxed),
        incoming_stream_protocol_first_count: INCOMING_STREAM_PROTOCOL_FIRST_COUNT
            .load(Ordering::Relaxed),
        incoming_stream_unknown_first_count: INCOMING_STREAM_UNKNOWN_FIRST_COUNT
            .load(Ordering::Relaxed),
        incoming_first_payload_bytes: INCOMING_FIRST_PAYLOAD_BYTES.load(Ordering::Relaxed),
        accepted_bi_stream_count: ACCEPTED_BI_STREAM_COUNT.load(Ordering::Relaxed),
        accepted_uni_stream_count: ACCEPTED_UNI_STREAM_COUNT.load(Ordering::Relaxed),

        screen_frame_received_count: SCREEN_FRAME_RECEIVED_COUNT.load(Ordering::Relaxed),
        screen_frame_received_bytes: SCREEN_FRAME_RECEIVED_BYTES.load(Ordering::Relaxed),
        first_screen_frame_sequence: optional_sequence(
            FIRST_SCREEN_FRAME_SEQUENCE.load(Ordering::Relaxed),
        ),
        last_screen_frame_sequence: optional_sequence(
            LAST_SCREEN_FRAME_SEQUENCE.load(Ordering::Relaxed),
        ),

        viewer_decoder_none_count: VIEWER_DECODER_NONE_COUNT.load(Ordering::Relaxed),
        viewer_decoded_frame_count: VIEWER_DECODED_FRAME_COUNT.load(Ordering::Relaxed),
        viewer_decode_error_count: VIEWER_DECODE_ERROR_COUNT.load(Ordering::Relaxed),
        viewer_render_success_count: VIEWER_RENDER_SUCCESS_COUNT.load(Ordering::Relaxed),
        viewer_render_failure_count: VIEWER_RENDER_FAILURE_COUNT.load(Ordering::Relaxed),

        sharer_capture_frame_count: SHARER_CAPTURE_FRAME_COUNT.load(Ordering::Relaxed),
        sharer_capture_error_count: SHARER_CAPTURE_ERROR_COUNT.load(Ordering::Relaxed),
        sharer_encoded_packet_count: SHARER_ENCODED_PACKET_COUNT.load(Ordering::Relaxed),
        sharer_encoded_bytes: SHARER_ENCODED_BYTES.load(Ordering::Relaxed),
        sharer_empty_packet_count: SHARER_EMPTY_PACKET_COUNT.load(Ordering::Relaxed),
        sharer_frame_stream_open_count: SHARER_FRAME_STREAM_OPEN_COUNT.load(Ordering::Relaxed),
        sharer_frame_stream_open_failure_count: SHARER_FRAME_STREAM_OPEN_FAILURE_COUNT
            .load(Ordering::Relaxed),
        sharer_frame_send_success_count: SHARER_FRAME_SEND_SUCCESS_COUNT.load(Ordering::Relaxed),
        sharer_frame_send_failure_count: SHARER_FRAME_SEND_FAILURE_COUNT.load(Ordering::Relaxed),
        sharer_frame_send_bytes: SHARER_FRAME_SEND_BYTES.load(Ordering::Relaxed),
        sharer_frame_send_wire_bytes: SHARER_FRAME_SEND_WIRE_BYTES.load(Ordering::Relaxed),
        sharer_frame_send_duration_micros: SHARER_FRAME_SEND_DURATION_MICROS
            .load(Ordering::Relaxed),
        first_keyframe_sequence: optional_sequence(FIRST_KEYFRAME_SEQUENCE.load(Ordering::Relaxed)),
        first_keyframe_bytes: optional_nonzero(FIRST_KEYFRAME_BYTES.load(Ordering::Relaxed)),
    }
}

pub fn record_incoming_stream_first(
    remote_addr: &str,
    payload_len: usize,
    message_type: Option<MessageType>,
) {
    INCOMING_STREAM_COUNT.fetch_add(1, Ordering::Relaxed);
    INCOMING_FIRST_PAYLOAD_BYTES.fetch_add(payload_len as u64, Ordering::Relaxed);

    if let Some(message_type) = message_type {
        INCOMING_STREAM_PROTOCOL_FIRST_COUNT.fetch_add(1, Ordering::Relaxed);
        log::info!(
            "[diagnostics] incoming_stream_first remote={} type={:?} bytes={}",
            remote_addr,
            message_type,
            payload_len
        );
    } else {
        INCOMING_STREAM_UNKNOWN_FIRST_COUNT.fetch_add(1, Ordering::Relaxed);
        log::warn!(
            "[diagnostics] incoming_stream_first remote={} type=unknown bytes={}",
            remote_addr,
            payload_len
        );
    }
}

pub fn record_screen_frame_received(sequence: u32, bytes: usize) {
    SCREEN_FRAME_RECEIVED_COUNT.fetch_add(1, Ordering::Relaxed);
    SCREEN_FRAME_RECEIVED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    set_first_sequence(&FIRST_SCREEN_FRAME_SEQUENCE, sequence as u64);
    LAST_SCREEN_FRAME_SEQUENCE.store(sequence as u64, Ordering::Relaxed);
}

pub fn record_viewer_decoder_none(sequence: u32) {
    VIEWER_DECODER_NONE_COUNT.fetch_add(1, Ordering::Relaxed);
    if sequence < 5 || sequence % 50 == 0 {
        log::info!("[diagnostics] decoder_output_none sequence={}", sequence);
    }
}

pub fn record_viewer_decoded_frame(sequence: u32) {
    VIEWER_DECODED_FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
    if sequence < 5 || sequence % 50 == 0 {
        log::info!("[diagnostics] decoder_output_frame sequence={}", sequence);
    }
}

pub fn record_viewer_decode_error(sequence: u32, error: &str) {
    VIEWER_DECODE_ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
    log::warn!(
        "[diagnostics] decoder_error sequence={} error={}",
        sequence,
        error
    );
}

pub fn record_viewer_render_success(sequence: u32) {
    VIEWER_RENDER_SUCCESS_COUNT.fetch_add(1, Ordering::Relaxed);
    if sequence < 5 || sequence % 50 == 0 {
        log::info!("[diagnostics] render_upload_success sequence={}", sequence);
    }
}

pub fn record_viewer_render_failure(sequence: u32, error: &str) {
    VIEWER_RENDER_FAILURE_COUNT.fetch_add(1, Ordering::Relaxed);
    log::warn!(
        "[diagnostics] render_upload_failure sequence={} error={}",
        sequence,
        error
    );
}

pub fn record_sharer_capture_frame() {
    SHARER_CAPTURE_FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub fn record_sharer_capture_error(error: &str) {
    SHARER_CAPTURE_ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
    log::warn!("[diagnostics] capture_error error={}", error);
}

pub fn record_sharer_empty_packet(sequence: u32) {
    SHARER_EMPTY_PACKET_COUNT.fetch_add(1, Ordering::Relaxed);
    if sequence < 5 || sequence % 100 == 0 {
        log::info!("[diagnostics] encoded_empty_packet sequence={}", sequence);
    }
}

pub fn record_sharer_encoded_packet(sequence: u32, bytes: usize, is_keyframe: bool) {
    SHARER_ENCODED_PACKET_COUNT.fetch_add(1, Ordering::Relaxed);
    SHARER_ENCODED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);

    if is_keyframe {
        if set_first_sequence(&FIRST_KEYFRAME_SEQUENCE, sequence as u64) {
            FIRST_KEYFRAME_BYTES.store(bytes as u64, Ordering::Relaxed);
            log::info!(
                "[diagnostics] first_keyframe sequence={} bytes={}",
                sequence,
                bytes
            );
        }
    }
}

pub fn record_frame_stream_opened(peer: &str) {
    SHARER_FRAME_STREAM_OPEN_COUNT.fetch_add(1, Ordering::Relaxed);
    log::debug!("[diagnostics] frame_stream_opened peer={}", peer);
}

pub fn record_frame_stream_open_failed(peer: &str, error: &str) {
    SHARER_FRAME_STREAM_OPEN_FAILURE_COUNT.fetch_add(1, Ordering::Relaxed);
    log::warn!(
        "[diagnostics] frame_stream_open_failed peer={} error={}",
        peer,
        error
    );
}

pub fn record_frame_send_success(
    peer: &str,
    sequence: u32,
    payload_bytes: usize,
    wire_bytes: usize,
    duration: Duration,
) {
    SHARER_FRAME_SEND_SUCCESS_COUNT.fetch_add(1, Ordering::Relaxed);
    SHARER_FRAME_SEND_BYTES.fetch_add(payload_bytes as u64, Ordering::Relaxed);
    SHARER_FRAME_SEND_WIRE_BYTES.fetch_add(wire_bytes as u64, Ordering::Relaxed);
    SHARER_FRAME_SEND_DURATION_MICROS.fetch_add(duration.as_micros() as u64, Ordering::Relaxed);

    if sequence < 5 || sequence % 50 == 0 {
        log::info!(
            "[diagnostics] frame_send_success peer={} sequence={} payload_bytes={} wire_bytes={} duration_ms={:.2}",
            peer,
            sequence,
            payload_bytes,
            wire_bytes,
            duration.as_secs_f64() * 1000.0
        );
    }
}

pub fn record_frame_send_failure(peer: &str, sequence: u32, error: &str) {
    SHARER_FRAME_SEND_FAILURE_COUNT.fetch_add(1, Ordering::Relaxed);
    log::warn!(
        "[diagnostics] frame_send_failure peer={} sequence={} error={}",
        peer,
        sequence,
        error
    );
}

pub fn record_bi_stream_accepted() {
    ACCEPTED_BI_STREAM_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub fn record_uni_stream_accepted() {
    ACCEPTED_UNI_STREAM_COUNT.fetch_add(1, Ordering::Relaxed);
}

fn set_first_sequence(target: &AtomicU64, sequence: u64) -> bool {
    target
        .compare_exchange(
            UNSET_SEQUENCE,
            sequence,
            Ordering::Relaxed,
            Ordering::Relaxed,
        )
        .is_ok()
}

fn optional_sequence(value: u64) -> Option<u64> {
    if value == UNSET_SEQUENCE {
        None
    } else {
        Some(value)
    }
}

fn optional_nonzero(value: u64) -> Option<u64> {
    if value == 0 { None } else { Some(value) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn snapshot_reports_first_and_last_received_sequence() {
        let _guard = TEST_LOCK.lock().expect("diagnostics test lock poisoned");
        reset();

        record_screen_frame_received(42, 100);
        record_screen_frame_received(43, 120);

        let snapshot = snapshot();
        assert_eq!(snapshot.screen_frame_received_count, 2);
        assert_eq!(snapshot.screen_frame_received_bytes, 220);
        assert_eq!(snapshot.first_screen_frame_sequence, Some(42));
        assert_eq!(snapshot.last_screen_frame_sequence, Some(43));
    }

    #[test]
    fn reset_clears_first_sequence_sentinels() {
        let _guard = TEST_LOCK.lock().expect("diagnostics test lock poisoned");
        reset();
        record_sharer_encoded_packet(7, 2048, true);
        assert_eq!(snapshot().first_keyframe_sequence, Some(7));

        reset();
        let snapshot = snapshot();
        assert_eq!(snapshot.first_keyframe_sequence, None);
        assert_eq!(snapshot.first_keyframe_bytes, None);
    }
}
