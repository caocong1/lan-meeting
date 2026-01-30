// GStreamer hardware-accelerated H.264 decoder
//
// Pipeline: appsrc → h264parse → decodebin → videoconvert → appsink
//
// GStreamer automatically selects the best hardware decoder:
// - Windows: d3d11h264dec / nvh264dec
// - macOS: vtdec_hw (VideoToolbox)
// - Linux: vah264dec (VAAPI) / nvh264dec
// - Fallback: avdec_h264 (FFmpeg software)

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use parking_lot::Mutex;

use super::{DecodedFrame, DecoderConfig, DecoderError, OutputFormat, VideoDecoder};

struct GstPipeline {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    appsink: gst_app::AppSink,
    config: DecoderConfig,
    frame_count: u64,
}

pub struct GStreamerDecoder {
    state: Option<Mutex<GstPipeline>>,
}

impl GStreamerDecoder {
    pub fn new() -> Result<Self, DecoderError> {
        // Initialize GStreamer
        gst::init().map_err(|e| DecoderError::InitError(format!("GStreamer init failed: {}", e)))?;

        log::info!("GStreamer initialized: version {}", gst::version_string());

        Ok(Self { state: None })
    }

    fn build_pipeline(config: &DecoderConfig) -> Result<GstPipeline, DecoderError> {
        let pipeline = gst::Pipeline::new();

        // appsrc: receives raw H.264 NAL units from network
        let appsrc = gst_app::AppSrc::builder()
            .name("src")
            .caps(
                &gst::Caps::builder("video/x-h264")
                    .field("stream-format", "byte-stream")
                    .field("alignment", "au")
                    .build(),
            )
            .format(gst::Format::Time)
            .is_live(true)
            .build();

        // h264parse: parses H.264 byte stream into proper NAL units
        let h264parse = gst::ElementFactory::make("h264parse")
            .name("parse")
            .build()
            .map_err(|e| {
                DecoderError::InitError(format!("Failed to create h264parse: {}", e))
            })?;

        // decodebin: auto-selects best decoder (hardware preferred)
        let decodebin = gst::ElementFactory::make("decodebin")
            .name("decode")
            .build()
            .map_err(|e| {
                DecoderError::InitError(format!("Failed to create decodebin: {}", e))
            })?;

        // videoconvert: converts decoded frames to target format
        let videoconvert = gst::ElementFactory::make("videoconvert")
            .name("convert")
            .build()
            .map_err(|e| {
                DecoderError::InitError(format!("Failed to create videoconvert: {}", e))
            })?;

        // appsink: outputs decoded frames to our application
        let video_format = match config.output_format {
            OutputFormat::BGRA => gst_video::VideoFormat::Bgra,
            OutputFormat::YUV420 => gst_video::VideoFormat::I420,
        };

        let appsink = gst_app::AppSink::builder()
            .name("sink")
            .caps(
                &gst_video::VideoCapsBuilder::new()
                    .format(video_format)
                    .width(config.width as i32)
                    .height(config.height as i32)
                    .build(),
            )
            .max_buffers(2)
            .drop(true) // Drop old frames if consumer is slow
            .build();

        // Add elements to pipeline
        pipeline
            .add_many([
                appsrc.upcast_ref(),
                &h264parse,
                &decodebin,
                &videoconvert,
                appsink.upcast_ref(),
            ])
            .map_err(|e| DecoderError::InitError(format!("Failed to add elements: {}", e)))?;

        // Link appsrc → h264parse → decodebin
        gst::Element::link_many([appsrc.upcast_ref(), &h264parse, &decodebin]).map_err(|e| {
            DecoderError::InitError(format!("Failed to link src→parse→decode: {}", e))
        })?;

        // Link videoconvert → appsink
        gst::Element::link_many([&videoconvert, appsink.upcast_ref()]).map_err(|e| {
            DecoderError::InitError(format!("Failed to link convert→sink: {}", e))
        })?;

        // decodebin has dynamic pads - connect when pad is added
        let convert_weak = videoconvert.downgrade();
        decodebin.connect_pad_added(move |_decodebin, src_pad| {
            let Some(convert) = convert_weak.upgrade() else {
                return;
            };

            let sink_pad = convert.static_pad("sink").expect("videoconvert has sink pad");
            if sink_pad.is_linked() {
                return;
            }

            if let Err(e) = src_pad.link(&sink_pad) {
                log::error!("Failed to link decodebin pad: {:?}", e);
            } else {
                log::info!("decodebin linked to videoconvert");
            }
        });

        // Start the pipeline
        pipeline.set_state(gst::State::Playing).map_err(|e| {
            DecoderError::InitError(format!("Failed to start pipeline: {:?}", e))
        })?;

        log::info!(
            "GStreamer pipeline started: {}x{} output={:?}",
            config.width,
            config.height,
            video_format
        );

        Ok(GstPipeline {
            pipeline,
            appsrc,
            appsink,
            config: config.clone(),
            frame_count: 0,
        })
    }
}

impl VideoDecoder for GStreamerDecoder {
    fn init(&mut self, config: DecoderConfig) -> Result<(), DecoderError> {
        let pipeline = Self::build_pipeline(&config)?;
        self.state = Some(Mutex::new(pipeline));
        Ok(())
    }

    fn decode(
        &mut self,
        data: &[u8],
        timestamp: u64,
    ) -> Result<Option<DecodedFrame>, DecoderError> {
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| DecoderError::DecodeError("Decoder not initialized".into()))?;

        let mut state = state.lock();

        // Push H.264 data into appsrc
        let mut buffer = gst::Buffer::with_size(data.len()).map_err(|e| {
            DecoderError::DecodeError(format!("Failed to create buffer: {}", e))
        })?;

        {
            let buffer_ref = buffer.get_mut().unwrap();
            buffer_ref.set_pts(gst::ClockTime::from_nseconds(timestamp * 1_000_000));
            let mut map = buffer_ref.map_writable().map_err(|e| {
                DecoderError::DecodeError(format!("Failed to map buffer: {}", e))
            })?;
            map.copy_from_slice(data);
        }

        state
            .appsrc
            .push_buffer(buffer)
            .map_err(|e| DecoderError::DecodeError(format!("Failed to push buffer: {}", e)))?;

        // Try to pull a decoded frame (non-blocking)
        match state.appsink.try_pull_sample(gst::ClockTime::from_mseconds(0)) {
            Some(sample) => {
                let frame = sample_to_frame(&sample, &state.config, timestamp)?;
                state.frame_count += 1;

                if state.frame_count == 1 {
                    // Log decoder info on first frame
                    log_decoder_info(&state.pipeline);
                }

                Ok(Some(frame))
            }
            None => {
                // Check for pipeline errors
                let bus = state.pipeline.bus().unwrap();
                while let Some(msg) = bus.pop() {
                    match msg.view() {
                        gst::MessageView::Error(err) => {
                            return Err(DecoderError::DecodeError(format!(
                                "GStreamer error: {} ({})",
                                err.error(),
                                err.debug().unwrap_or_default()
                            )));
                        }
                        gst::MessageView::Warning(warn) => {
                            log::warn!(
                                "GStreamer warning: {} ({})",
                                warn.error(),
                                warn.debug().unwrap_or_default()
                            );
                        }
                        _ => {}
                    }
                }
                Ok(None)
            }
        }
    }

    fn flush(&mut self) -> Result<Vec<DecodedFrame>, DecoderError> {
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| DecoderError::DecodeError("Decoder not initialized".into()))?;

        let state = state.lock();
        let mut frames = Vec::new();

        // Signal EOS to flush the pipeline
        let _ = state.appsrc.end_of_stream();

        // Pull remaining frames
        while let Some(sample) =
            state.appsink.try_pull_sample(gst::ClockTime::from_mseconds(100))
        {
            if let Ok(frame) = sample_to_frame(&sample, &state.config, 0) {
                frames.push(frame);
            }
        }

        // Reset pipeline for reuse
        let _ = state.pipeline.set_state(gst::State::Null);

        Ok(frames)
    }

    fn info(&self) -> &str {
        "GStreamer (auto hardware selection)"
    }
}

impl Drop for GStreamerDecoder {
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let state = state.lock();
            let _ = state.appsrc.end_of_stream();
            let _ = state.pipeline.set_state(gst::State::Null);
        }
    }
}

/// Convert a GStreamer sample to our DecodedFrame
fn sample_to_frame(
    sample: &gst::Sample,
    config: &DecoderConfig,
    timestamp: u64,
) -> Result<DecodedFrame, DecoderError> {
    let buffer = sample
        .buffer()
        .ok_or_else(|| DecoderError::DecodeError("No buffer in sample".into()))?;

    let caps = sample
        .caps()
        .ok_or_else(|| DecoderError::DecodeError("No caps in sample".into()))?;

    let video_info = gst_video::VideoInfo::from_caps(caps)
        .map_err(|e| DecoderError::DecodeError(format!("Invalid video caps: {}", e)))?;

    let map = buffer
        .map_readable()
        .map_err(|e| DecoderError::DecodeError(format!("Failed to map buffer: {}", e)))?;

    let width = video_info.width();
    let height = video_info.height();

    let ts = buffer
        .pts()
        .map(|pts| pts.nseconds() / 1_000_000)
        .unwrap_or(timestamp);

    match config.output_format {
        OutputFormat::BGRA => Ok(DecodedFrame::bgra(width, height, ts, map.to_vec())),
        OutputFormat::YUV420 => {
            let strides = [
                video_info.stride()[0] as usize,
                video_info.stride()[1] as usize,
                video_info.stride()[2] as usize,
            ];
            Ok(DecodedFrame::yuv420(width, height, ts, map.to_vec(), strides))
        }
    }
}

/// Log which decoder GStreamer actually selected
fn log_decoder_info(pipeline: &gst::Pipeline) {
    // Walk the pipeline to find the actual decoder element
    let mut iter = pipeline.iterate_recurse();
    while let Ok(Some(element)) = iter.next() {
        let factory = element.factory();
        if let Some(factory) = factory {
            let klass = factory.metadata("klass").unwrap_or_default();
            if klass.contains("Decoder") && klass.contains("Video") {
                log::info!(
                    "GStreamer selected decoder: {} ({})",
                    factory.name(),
                    factory.metadata("long-name").unwrap_or_default()
                );
                return;
            }
        }
    }
}
