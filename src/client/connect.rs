use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use base64::{Engine, engine::general_purpose};
use ffmpeg_next as ffmpeg;
use tokio::sync::{Mutex, mpsc};
use webrtc::{
    peer_connection::sdp::session_description::RTCSessionDescription,
    rtp::{codecs::h264::H264Packet, packetizer::Depacketizer},
    rtp_transceiver::rtp_codec::RTPCodecType,
    track::track_remote::TrackRemote,
};

use super::StreamFrame;
use crate::shared::{MousePosition, SdpData, create_peer_connection};

#[derive(Debug, Clone)]
struct WebRTCPacket {
    data: Vec<u8>,
    timestamp: u32,
}

pub async fn start_webrtc(
    password: Option<String>,
    address: SocketAddr,
    hwaccel: bool,
    frame_tx: mpsc::Sender<StreamFrame>,
) -> Result<()> {
    let (packet_tx, packet_rx) = mpsc::channel::<WebRTCPacket>(2);
    let mouse_position = Arc::new(Mutex::new(None));

    // spawn video processing task
    let frame_tx_clone = frame_tx.clone();
    let mouse_position_clone = mouse_position.clone();
    tokio::spawn(run_video_processor(
        packet_rx,
        frame_tx_clone,
        mouse_position_clone,
        hwaccel,
    ));

    // create peer connection
    let peer_connection = create_peer_connection().await?;

    // add transceiver for video
    peer_connection
        .add_transceiver_from_kind(RTPCodecType::Video, None)
        .await?;

    // handle incoming tracks
    peer_connection.on_track(Box::new(move |track, _, _| {
        if track.kind() == RTPCodecType::Video {
            let tx = packet_tx.clone();
            tokio::spawn(process_video_track(track, tx));
        }
        Box::pin(async {})
    }));

    // create mouse data channel
    let mouse_channel = peer_connection
        .create_data_channel("mouse", None)
        .await
        .unwrap();
    mouse_channel.on_open(Box::new(|| {
        println!("Mouse data channel opened");
        Box::pin(async {})
    }));
    let mouse_pos_clone = mouse_position.clone();
    mouse_channel.on_message(Box::new(move |msg| {
        if let Ok(text) = String::from_utf8(msg.data.to_vec()) {
            if let Ok(pos) = serde_json::from_str::<MousePosition>(&text) {
                // println!("Received mouse position: x={}, y={}", pos.x, pos.y);
                let mouse_pos = mouse_pos_clone.clone();
                tokio::spawn(async move {
                    *mouse_pos.lock().await = Some(pos);
                });
            }
        }

        Box::pin(async {})
    }));

    // create and send offer
    let offer = peer_connection.create_offer(None).await?;
    peer_connection.set_local_description(offer).await?;

    // wait for ICE gathering to complete
    let mut gather_complete = peer_connection.gathering_complete_promise().await;
    let local_description = peer_connection.local_description().await.unwrap();
    let sdp = general_purpose::STANDARD.encode(serde_json::to_string(&local_description)?);
    let _ = gather_complete.recv().await;

    println!("Sending SDP to server at {}...", address);

    let sdp_data = SdpData { sdp, password };
    let client = reqwest::Client::new();
    let res = client
        .post(format!("http://{}:{}/sdp", address.ip(), address.port()))
        .json(&sdp_data)
        .send()
        .await?;

    if !res.status().is_success() {
        eprintln!("Failed to connect to server: {}", res.status());
        return Err(anyhow::anyhow!("Failed to connect to server"));
    }

    // get answer
    let answer_text = res.text().await?;
    let answer_sdp: SdpData = serde_json::from_str(&answer_text)?;

    let answer: RTCSessionDescription = {
        let decoded_sdp = general_purpose::STANDARD.decode(answer_sdp.sdp)?;
        let decoded_sdp_str = String::from_utf8(decoded_sdp)?;
        serde_json::from_str(&decoded_sdp_str)?
    };

    peer_connection.set_remote_description(answer).await?;

    println!("Connected to server at {}", address);

    Ok(())
}

async fn process_video_track(track: Arc<TrackRemote>, packet_tx: mpsc::Sender<WebRTCPacket>) {
    let mut h264_packet = H264Packet::default();
    let mut frame_buf: Vec<u8> = Vec::with_capacity(1024 * 1024);
    let start_code: &[u8] = &[0, 0, 0, 1];

    loop {
        // read RTP packet from track
        let (rtp_packet, _) = match track.read_rtp().await {
            Ok(packet) => packet,
            Err(e) => {
                eprintln!("Error reading RTP packet: {}", e);
                break;
            }
        };

        // depacketize RTP payload
        if let Ok(payload) = h264_packet.depacketize(&rtp_packet.payload) {
            if !payload.is_empty() {
                // prepend every NAL unit with a start code
                frame_buf.extend_from_slice(start_code);
                frame_buf.extend_from_slice(&payload);
            }
        }

        // send frame if marker bit is set
        if rtp_packet.header.marker && !frame_buf.is_empty() {
            let raw_packet = WebRTCPacket {
                data: std::mem::take(&mut frame_buf),
                timestamp: rtp_packet.header.timestamp,
            };

            if let Err(err) = packet_tx.send(raw_packet).await {
                eprintln!("Failed to send frame: {}", err);
                break;
            }
        }
    }
}

#[cfg(target_os = "windows")]
const HW_DECODERS: &[&str] = &[
    "h264_cuvid",   // NVIDIA CUVID
    "h264_qsv",     // Intel Quick Sync Video
    "h264_d3d11va", // Microsoft D3D11VA (generic, works on most modern GPUs)
    "h264_dxva2",   // Microsoft DXVA2 (older alternative)
];

#[cfg(target_os = "linux")]
const HW_DECODERS: &[&str] = &[
    "h264_cuvid", // NVIDIA CUVID
    "h264_vaapi", // Intel/AMD VA-API
    "h264_vdpau", // NVIDIA VDPAU (alternative)
];

#[cfg(not(target_os = "macos"))]
fn setup_video_decoder(hwaccel: bool) -> Result<ffmpeg::decoder::Video> {
    let codec = if hwaccel {
        HW_DECODERS
            .iter()
            .find_map(|&name| {
                ffmpeg::codec::decoder::find_by_name(name).and_then(|decoder| {
                    println!("Using hardware decoder: {}", name);
                    Some(decoder)
                })
            })
            .unwrap_or_else(|| {
                println!("No hardware decoders found. Falling back to software decoder (h264).");
                ffmpeg::codec::decoder::find(ffmpeg::codec::Id::H264)
                    .expect("Default H264 software decoder (h264) not found.")
            })
    } else {
        ffmpeg::codec::decoder::find(ffmpeg::codec::Id::H264)
            .ok_or(anyhow::anyhow!("H264 decoder not found"))?
    };

    let context = ffmpeg::codec::context::Context::new_with_codec(codec);
    Ok(context.decoder().video()?)
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn hardware_decoder_format_callback(
    _ctx: *mut ffmpeg::ffi::AVCodecContext,
    pix_fmts: *const ffmpeg::ffi::AVPixelFormat,
) -> ffmpeg::ffi::AVPixelFormat {
    unsafe {
        let mut p = pix_fmts;
        while *p != ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            if *p == ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX {
                return *p;
            }
            p = p.add(1);
        }
        ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE
    }
}

#[cfg(target_os = "macos")]
fn setup_video_decoder(hwaccel: bool) -> Result<ffmpeg::decoder::Video> {
    let codec = ffmpeg::codec::decoder::find(ffmpeg::codec::Id::H264)
        .ok_or(anyhow::anyhow!("H264 decoder not found"))?;
    let mut context = ffmpeg::codec::context::Context::new_with_codec(codec);

    if hwaccel {
        unsafe {
            let ctx_ptr = context.as_mut_ptr();

            let mut hw_device_ctx: *mut ffmpeg::ffi::AVBufferRef = std::ptr::null_mut();
            let ret = ffmpeg::ffi::av_hwdevice_ctx_create(
                &mut hw_device_ctx,
                ffmpeg::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
            );

            if ret < 0 {
                eprintln!("Failed to enable hardware acceleration with videotoolbox.");
            } else {
                (*ctx_ptr).hw_device_ctx = hw_device_ctx;
                (*ctx_ptr).get_format = Some(hardware_decoder_format_callback);
                println!("Using hardware decoder: h264_videotoolbox");
            }
        }
    }

    Ok(context.decoder().video()?)
}

async fn run_video_processor(
    mut packet_rx: mpsc::Receiver<WebRTCPacket>,
    frame_tx: mpsc::Sender<StreamFrame>,
    mouse_position: Arc<Mutex<Option<MousePosition>>>,
    hwaccel: bool,
) -> Result<()> {
    unsafe {
        ffmpeg::ffi::av_log_set_level(ffmpeg::ffi::AV_LOG_QUIET);
    }
    ffmpeg::init()?;

    let mut decoder = setup_video_decoder(hwaccel)?;

    decoder.set_threading(ffmpeg::threading::Config {
        kind: ffmpeg::threading::Type::Frame,
        count: 0,
    });
    decoder.set_flags(ffmpeg::codec::flag::Flags::LOW_DELAY);

    let mut raw_frame = ffmpeg::frame::Video::empty();
    let mut cpu_frame = ffmpeg::frame::Video::empty();
    let mut rgb_frame = ffmpeg::frame::Video::empty();
    let rtp_time_base = ffmpeg::Rational(1, 90000);
    let decoder_time_base = decoder.time_base();

    while let Some(webrtc_packet) = packet_rx.recv().await {
        // Set packet data and timestamp
        let mut packet = ffmpeg::packet::Packet::copy(&webrtc_packet.data);
        unsafe {
            let pts = ffmpeg::ffi::av_rescale_q(
                webrtc_packet.timestamp as i64,
                rtp_time_base.into(),
                decoder_time_base.into(),
            );
            packet.set_pts(Some(pts));
            packet.set_dts(Some(pts));
        }

        // Send packet to decoder
        if decoder.send_packet(&packet).is_err() {
            continue;
        }

        // Receive decoded frames
        while decoder.receive_frame(&mut raw_frame).is_ok() {
            // If the frame is hardware accelerated, transfer it to system memory
            if raw_frame.format() == ffmpeg::format::Pixel::VIDEOTOOLBOX {
                unsafe {
                    let ret = ffmpeg::ffi::av_hwframe_transfer_data(
                        cpu_frame.as_mut_ptr(),
                        raw_frame.as_ptr(),
                        0,
                    );

                    if ret < 0 {
                        // If transfer fails, assume frame is already in system memory
                        cpu_frame = raw_frame.clone();
                    }
                }
            } else {
                cpu_frame = raw_frame.clone();
            }

            // Convert frame to RGB format for pixel buffer
            let mut stream_frame = {
                let mut scaler = ffmpeg::software::scaling::context::Context::get(
                    cpu_frame.format(),
                    cpu_frame.width(),
                    cpu_frame.height(),
                    ffmpeg::format::Pixel::RGBA,
                    cpu_frame.width(),
                    cpu_frame.height(),
                    ffmpeg::software::scaling::Flags::FAST_BILINEAR,
                )?;

                scaler.run(&cpu_frame, &mut rgb_frame)?;

                // copy pixel data out while scaler is still alive
                let width = rgb_frame.width() as usize;
                let height = rgb_frame.height() as usize;
                let data = rgb_frame.data(0).to_vec();

                StreamFrame {
                    data,
                    width: width as u32,
                    height: height as u32,
                    mouse: None,
                }
            };

            let current_mouse_pos = mouse_position.lock().await.clone();
            stream_frame.mouse = current_mouse_pos;

            if frame_tx.send(stream_frame).await.is_err() {
                break;
            }
        }
    }

    Ok(())
}
