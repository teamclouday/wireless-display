use std::{
    fmt::Display,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use ffmpeg_next as ffmpeg;
use mouse_position::mouse_position::Mouse;
use tokio::sync::{broadcast, mpsc};
use webrtc::media::Sample;

use crate::shared::MousePosition;

use super::AppState;

#[derive(Clone)]
#[allow(dead_code)]
pub struct CaptureDevice {
    pub index: usize,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
}

impl Display for CaptureDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}x{})", self.name, self.width, self.height)
    }
}

pub async fn capture_screen(
    state: Arc<AppState>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<()> {
    let (tx, mut rx) = mpsc::channel::<Sample>(2);
    let state_clone = state.clone();

    let shutdown_signal = Arc::new(AtomicBool::new(false));
    let shutdown_signal_clone = shutdown_signal.clone();

    let send_task = tokio::spawn(async move {
        while !shutdown_signal_clone.load(Ordering::Relaxed) {
            if let Some(sample) = rx.recv().await {
                if let Some(video_track) = state_clone.video_track.lock().await.as_mut() {
                    if let Err(err) = video_track.write_sample(&sample).await {
                        eprintln!("Error writing sample: {}", err);
                        continue;
                    }
                }
            }
        }

        Ok(())
    });

    let shutdown_signal_clone = shutdown_signal.clone();
    let capture_task = tokio::task::spawn_blocking(move || {
        unsafe {
            ffmpeg::ffi::av_log_set_level(ffmpeg::ffi::AV_LOG_QUIET);
        }
        ffmpeg::init().map_err(|e| anyhow::anyhow!("Failed to initialize FFmpeg: {}", e))?;

        // create input context
        let ictx = create_input_context(&state.device, state.framerate).map_err(|e| {
            eprintln!("Failed to create input context: {}", e);
            anyhow::anyhow!("Failed to create input context: {}", e)
        })?;
        let mut input = ictx.input();
        let ist = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or_else(|| anyhow::anyhow!("No video stream found"))?;
        let ist_index = ist.index();

        // create decoder
        let mut decoder = ffmpeg::codec::context::Context::from_parameters(ist.parameters())
            .map_err(|e| anyhow::anyhow!("Failed to create video decoder context: {}", e))?
            .decoder()
            .video()
            .map_err(|e| anyhow::anyhow!("Failed to create video decoder: {}", e))?;
        decoder.set_threading(ffmpeg::threading::Config {
            kind: ffmpeg::threading::Type::Frame,
            count: 0,
        });

        // create scaler
        let mut scaler = ffmpeg::software::scaling::Context::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            ffmpeg::format::Pixel::YUV420P,
            decoder.width(),
            decoder.height(),
            ffmpeg::software::scaling::flag::Flags::FAST_BILINEAR,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create video scaler: {}", e))?;

        // set up encoder for WebRTC
        let encoder_codec = ffmpeg::codec::encoder::find(ffmpeg::codec::Id::H264)
            .ok_or(anyhow::anyhow!("H264 encoder not found"))?;

        let mut encoder_ctx = ffmpeg::codec::context::Context::new_with_codec(encoder_codec)
            .encoder()
            .video()
            .map_err(|e| anyhow::anyhow!("Failed to create video encoder context: {}", e))?;

        encoder_ctx.set_height(decoder.height());
        encoder_ctx.set_width(decoder.width());
        encoder_ctx.set_format(ffmpeg::format::Pixel::YUV420P);
        encoder_ctx.set_color_range(ffmpeg::util::color::Range::MPEG);
        encoder_ctx.set_colorspace(ffmpeg::util::color::Space::BT709);

        let encoder_time_base = ffmpeg::Rational(1, 90000);
        encoder_ctx.set_time_base(encoder_time_base);

        let mut opts = ffmpeg::Dictionary::new();
        opts.set("preset", "fast"); // fast encoding
        opts.set("tune", "zerolatency"); // Zero latency tuning
        opts.set("profile", "naub"); // baseline profile
        opts.set("level", "3.1"); // H.264 level
        opts.set("crf", "21"); // Constant Rate Factor (quality, lower is better)
        opts.set("keyint", "15");
        opts.set("sc_threshold", "0");

        let mut encoder = encoder_ctx
            .open_with(opts)
            .map_err(|e| anyhow::anyhow!("Failed to open encoder: {}", e))?;

        println!("Starting capture on monitor: {}", state.device);

        let mut decoded_frame = ffmpeg::frame::Video::empty();
        let mut frame_count: i64 = 0;

        for (stream, packet) in input.packets() {
            if stream.index() == ist_index {
                // decode packet
                decoder.send_packet(&packet)?;
                let mut scaled_frame = ffmpeg::frame::Video::empty();
                while decoder.receive_frame(&mut decoded_frame).is_ok() {
                    // scale to YUV format
                    // let original_pts = decoded_frame.pts().unwrap_or(0);

                    // unsafe {
                    //     scaled_frame.set_pts(Some(ffmpeg::ffi::av_rescale_q(
                    //         original_pts,
                    //         ist_time_base.into(),
                    //         encoder_time_base.into(),
                    //     )));
                    // }
                    let pts = (frame_count as f64 * encoder_time_base.denominator() as f64
                        / state.framerate as f64) as i64;
                    scaled_frame.set_pts(Some(pts));
                    frame_count += 1;
                    scaler.run(&decoded_frame, &mut scaled_frame)?;

                    // encode to H264
                    encoder.send_frame(&scaled_frame)?;
                    let mut encoded_packet = ffmpeg::Packet::empty();
                    while encoder.receive_packet(&mut encoded_packet).is_ok() {
                        if state.video_track.try_lock().is_ok_and(|t| t.is_some()) {
                            // send to WebRTC
                            let packet_data = encoded_packet.data().unwrap().to_vec();
                            let sample_duration =
                                Duration::from_secs_f64(1.0 / state.framerate as f64);

                            let sample = Sample {
                                data: packet_data.into(),
                                duration: sample_duration,
                                ..Default::default()
                            };

                            let _ = tx.try_send(sample);
                        }
                    }
                }
            }

            if shutdown_signal_clone.load(Ordering::Relaxed) {
                break;
            }
        }

        Ok(())
    });

    tokio::select! {
        capture_result = capture_task => {
            capture_result?
        }
        send_result = send_task => {
            send_result?
        }
        _ = shutdown_rx.recv() => {
            println!("Shutting down screen capture...");
            shutdown_signal.store(true, Ordering::Relaxed);
            Ok(())
        }
    }
}

pub async fn capture_mouse(
    state: Arc<AppState>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<()> {
    let (tx, mut rx) = mpsc::channel::<MousePosition>(64);
    let state_clone = state.clone();

    let shutdown_signal = Arc::new(AtomicBool::new(false));
    let shutdown_signal_clone = shutdown_signal.clone();

    let send_task = tokio::spawn(async move {
        while !shutdown_signal_clone.load(Ordering::Relaxed) {
            if let Some(position) = rx.recv().await {
                if let Some(mouse_channel) = state_clone.mouse_channel.lock().await.as_mut() {
                    if mouse_channel.ready_state()
                        == webrtc::data_channel::data_channel_state::RTCDataChannelState::Open
                    {
                        let msg = serde_json::to_string(&position).unwrap();
                        if let Err(err) = mouse_channel.send_text(msg).await {
                            eprintln!("Error sending mouse position: {}", err);
                            continue;
                        }
                    }
                }
            }
        }

        Ok(())
    });

    let shutdown_signal_clone = shutdown_signal.clone();
    let capture_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(16));

        println!("Starting mouse capture...");

        while !shutdown_signal_clone.load(Ordering::Relaxed) {
            interval.tick().await;

            match Mouse::get_mouse_position() {
                Mouse::Position { x, y } => {
                    let mut relative_x = if state.device.width > 0 {
                        (x - state.device.x) as f64 / state.device.width as f64
                    } else {
                        0.0
                    };

                    let mut relative_y = if state.device.height > 0 {
                        (y - state.device.y) as f64 / state.device.height as f64
                    } else {
                        0.0
                    };

                    // set to -1 if out of bounds
                    relative_x = if relative_x >= 0.0 && relative_x <= 1.0 {
                        relative_x
                    } else {
                        -1.0
                    };

                    relative_y = if relative_y >= 0.0 && relative_y <= 1.0 {
                        relative_y
                    } else {
                        -1.0
                    };

                    let current_position = MousePosition {
                        x: relative_x,
                        y: relative_y,
                    };

                    let _ = tx.try_send(current_position);
                }
                Mouse::Error => {
                    eprintln!("Failed to capture mouse position");
                    break;
                }
            };
        }

        Ok(())
    });

    tokio::select! {
        capture_result = capture_task => {
            capture_result?
        }
        send_result = send_task => {
            send_result?
        }
        _ = shutdown_rx.recv() => {
            println!("Shutting down screen capture...");
            shutdown_signal.store(true, Ordering::Relaxed);
            Ok(())
        }
    }
}

#[cfg(target_os = "windows")]
fn create_input_context(
    capture: &CaptureDevice,
    framerate: u32,
) -> Result<ffmpeg::format::context::Context> {
    // find capture device
    let input_device = ffmpeg::device::input::video()
        .into_iter()
        .find(|d| d.name() == "gdigrab")
        .ok_or(anyhow::anyhow!("gdigrab input device not found"))?;

    // set input options
    let mut input_options = ffmpeg::Dictionary::new();
    input_options.set("offset_x", &capture.x.to_string());
    input_options.set("offset_y", &capture.y.to_string());
    input_options.set(
        "video_size",
        &format!("{}x{}", capture.width, capture.height),
    );
    input_options.set("framerate", &framerate.to_string());
    input_options.set("draw_mouse", "0");

    // set device path
    let video_path = "desktop".to_string();

    let ictx = ffmpeg::format::open_with(&video_path, &input_device, input_options)?;
    Ok(ictx)
}

#[cfg(target_os = "linux")]
fn create_input_context(
    capture: &CaptureDevice,
    framerate: u32,
) -> Result<ffmpeg::format::context::Context> {
    // find capture device
    let input_device = ffmpeg::device::input::video()
        .into_iter()
        .find(|d| d.name() == "x11grab")
        .ok_or(anyhow::anyhow!("x11grab input device not found"))?;

    // set input options
    let mut input_options = ffmpeg::Dictionary::new();
    input_options.set(
        "video_size",
        &format!("{}x{}", capture.width, capture.height),
    );
    input_options.set("framerate", &framerate.to_string());
    input_options.set("draw_mouse", "0");

    // set device path
    let video_path = format!(":0.0+{},{}", capture.x, capture.y);

    let ictx = ffmpeg::format::open_with(&video_path, &input_device, input_options)?;
    Ok(ictx)
}

#[cfg(target_os = "macos")]
fn create_input_context(
    capture: &CaptureDevice,
    framerate: u32,
) -> Result<ffmpeg::format::context::Context> {
    // find capture device
    let input_device = ffmpeg::device::input::video()
        .into_iter()
        .find(|d| d.name() == "avfoundation")
        .ok_or(anyhow::anyhow!("avfoundation input device not found"))?;

    // set input options
    let mut input_options = ffmpeg::Dictionary::new();
    input_options.set("framerate", &framerate.to_string());
    input_options.set("pixel_format", "uyvy422");
    input_options.set("capture_cursor", "0");
    input_options.set("capture_mouse_clicks", "0");

    // set device path
    let video_path = format!("{}:", capture.index + 1);

    let ictx = ffmpeg::format::open_with(&video_path, &input_device, input_options)?;
    Ok(ictx)
}
