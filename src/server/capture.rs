use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use anyhow::Result;
use openh264::{encoder::Encoder, formats::YUVBuffer};
use webrtc::media::Sample;
use windows_capture::{
    capture::{Context, GraphicsCaptureApiHandler},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    monitor::Monitor,
    settings::Settings,
};
use yuv::{YuvChromaSubsampling, YuvPlanarImageMut, rgba_to_yuv420};

use super::AppState;

struct Capture {
    state: Arc<AppState>,
    encoder: Encoder,
    timer: Instant,
}

impl GraphicsCaptureApiHandler for Capture {
    type Flags = Arc<AppState>;
    type Error = anyhow::Error;

    fn new(ctx: Context<Self::Flags>) -> Result<Self> {
        let encoder = Encoder::new()?;
        Ok(Self {
            state: ctx.flags.clone(),
            encoder,
            timer: Instant::now(),
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _: InternalCaptureControl,
    ) -> std::result::Result<(), Self::Error> {
        // check if we are connected
        let connection_state = self.state.connection.try_lock()?;
        if *connection_state != super::ConnectionState::Connected {
            return Ok(());
        }
        drop(connection_state);

        let video_track = self.state.video_track.try_lock()?;
        let track = match video_track.as_ref() {
            Some(track) => track.clone(),
            None => return Ok(()),
        };
        drop(video_track);

        // process frame
        let width = frame.width();
        let height = frame.height();
        let mut buffer = frame.buffer()?;

        let mut yuv_frame =
            YuvPlanarImageMut::<u8>::alloc(width, height, YuvChromaSubsampling::Yuv420);
        rgba_to_yuv420(
            &mut yuv_frame,
            buffer.as_raw_buffer(),
            width * 4,
            yuv::YuvRange::Limited,
            yuv::YuvStandardMatrix::Bt709,
            yuv::YuvConversionMode::Balanced,
        )?;
        let y_plane = yuv_frame.y_plane.borrow();
        let u_plane = yuv_frame.u_plane.borrow();
        let v_plane = yuv_frame.v_plane.borrow();

        let mut yuv_data = Vec::with_capacity(y_plane.len() + u_plane.len() + v_plane.len());
        yuv_data.extend_from_slice(&y_plane);
        yuv_data.extend_from_slice(&u_plane);
        yuv_data.extend_from_slice(&v_plane);

        let yuv_buffer = YUVBuffer::from_vec(yuv_data, width as usize, height as usize);
        let encoded_frame = self.encoder.encode(&yuv_buffer)?.to_vec();

        // calculate duration
        let now = Instant::now();
        let actual_duration = now.duration_since(self.timer);
        let target_duration = Duration::from_millis(16); // 60 FPS

        let frame_duration = if actual_duration.as_millis() > 0 && actual_duration.as_millis() < 100
        {
            actual_duration
        } else {
            target_duration
        };

        // send encoded frame as sample
        let num_bytes = encoded_frame.len();
        if num_bytes > 0 {
            let sample = Sample {
                data: encoded_frame.into(),
                timestamp: SystemTime::now(),
                duration: frame_duration,
                ..Default::default()
            };

            tokio::spawn(async move {
                if let Err(err) = track.write_sample(&sample).await {
                    eprintln!("Error writing sample: {}", err);
                }
            });

            // println!("Sent frame: {} bytes", num_bytes);
        } else {
            // eprintln!("Encoded frame is empty, skipping");
        }

        self.timer = now;

        Ok(())
    }
}

pub async fn capture_screen(state: Arc<AppState>) -> Result<()> {
    // get the selected screen and configure settings
    let monitor = Monitor::from_index(state.screen_index)?;
    let settings = Settings::new(
        monitor,
        windows_capture::settings::CursorCaptureSettings::WithCursor,
        windows_capture::settings::DrawBorderSettings::WithoutBorder,
        windows_capture::settings::SecondaryWindowSettings::Exclude,
        windows_capture::settings::MinimumUpdateIntervalSettings::Custom(Duration::from_millis(16)), // 60 FPS
        windows_capture::settings::DirtyRegionSettings::Default,
        windows_capture::settings::ColorFormat::Rgba8,
        state.clone(),
    );

    println!(
        "Starting capture on monitor: {} ({}x{})",
        monitor.name().unwrap_or("Unknown".to_string()),
        monitor.width()?,
        monitor.height()?
    );
    Capture::start(settings)?;

    Ok(())
}
