use std::{fmt::Display, sync::Arc, time::Duration};

use anyhow::Result;
use ffmpeg_next as ffmpeg;
use tokio::runtime::Handle;
use webrtc::media::Sample;

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

pub async fn capture_screen(state: Arc<AppState>) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let rt = Handle::current();

        ffmpeg::init()?;

        // create input context
        let ictx = create_input_context(&state.device, state.framerate)?;
        let mut input = ictx.input();
        let ist = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or_else(|| anyhow::anyhow!("No video stream found"))?;
        let ist_index = ist.index();
        let ist_time_base = ist.time_base();

        // create decoder
        let mut decoder = ffmpeg::codec::context::Context::from_parameters(ist.parameters())?
            .decoder()
            .video()?;

        // create scaler
        let mut scaler = ffmpeg::software::scaling::Context::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            ffmpeg::format::Pixel::YUV420P,
            decoder.width(),
            decoder.height(),
            ffmpeg::software::scaling::flag::Flags::BILINEAR,
        )?;

        // set up VP9 encoder for WebRTC
        let encoder_codec = ffmpeg::codec::encoder::find(ffmpeg::codec::Id::VP9)
            .ok_or(anyhow::anyhow!("VP9 encoder not found"))?;

        let mut encoder_ctx = ffmpeg::codec::context::Context::new_with_codec(encoder_codec)
            .encoder()
            .video()?;

        encoder_ctx.set_height(decoder.height());
        encoder_ctx.set_width(decoder.width());
        encoder_ctx.set_format(ffmpeg::format::Pixel::YUV420P);

        let encoder_time_base = ffmpeg::Rational(1, 90000);
        encoder_ctx.set_time_base(encoder_time_base);

        let mut opts = ffmpeg::Dictionary::new();
        opts.set("deadline", "realtime");
        opts.set("cpu-used", "8");
        opts.set("g", "120");

        let mut encoder = encoder_ctx.open_with(opts)?;

        println!("Starting capture on monitor: {}", state.device);

        let mut decoded_frame = ffmpeg::frame::Video::empty();
        let mut last_pts: i64 = 0;

        for (stream, packet) in input.packets() {
            if stream.index() == ist_index {
                // decode packet
                decoder.send_packet(&packet)?;
                let mut scaled_frame = ffmpeg::frame::Video::empty();
                while decoder.receive_frame(&mut decoded_frame).is_ok() {
                    // scale to YUV format
                    let original_pts = decoded_frame.pts().unwrap_or(0);
                    unsafe {
                        scaled_frame.set_pts(Some(ffmpeg_next::ffi::av_rescale_q(
                            original_pts,
                            ist_time_base.into(),
                            encoder_time_base.into(),
                        )));
                    }
                    scaler.run(&decoded_frame, &mut scaled_frame)?;

                    // encode to VP9
                    encoder.send_frame(&scaled_frame)?;
                    let mut encoded_packet = ffmpeg::Packet::empty();
                    while encoder.receive_packet(&mut encoded_packet).is_ok() {
                        if let Some(video_track) = state.video_track.blocking_lock().as_mut() {
                            // send to WebRTC
                            let packet_data = encoded_packet.data().unwrap().to_vec();
                            let current_pts = encoded_packet.pts().unwrap_or(last_pts);

                            let pts_duration = if last_pts == 0 {
                                1500 // Default duration for the first frame (90000 / 60fps)
                            } else {
                                current_pts - last_pts
                            };

                            let sample_duration = Duration::from_secs_f64(
                                pts_duration as f64 / encoder_time_base.1 as f64,
                            );
                            last_pts = current_pts;

                            let sample = Sample {
                                data: packet_data.into(),
                                duration: sample_duration,
                                ..Default::default()
                            };
                            rt.block_on(async { video_track.write_sample(&sample).await })?;
                        }
                    }
                }
            }
        }

        Ok(())
    })
    .await?
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
    input_options.set("capture_cursor", "1");
    input_options.set("capture_mouse_clicks", "0");

    // set device path
    let video_path = format!("{}:", capture.index + 1);

    let ictx = ffmpeg::format::open_with(&video_path, &input_device, input_options)?;
    Ok(ictx)
}
