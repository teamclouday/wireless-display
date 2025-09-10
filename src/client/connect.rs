use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use ffmpeg_next as ffmpeg;
use tokio::sync::mpsc;
use webrtc::{
    peer_connection::sdp::session_description::RTCSessionDescription,
    rtp_transceiver::rtp_codec::RTPCodecType, track::track_remote::TrackRemote,
};

use super::StreamFrame;
use crate::shared::{SdpData, create_peer_connection};

#[derive(Debug, Clone)]
struct WebRTCPacket {
    data: Vec<u8>,
    timestamp: u32,
}

pub async fn start_webrtc(
    password: Option<String>,
    address: SocketAddr,
    frame_tx: mpsc::Sender<StreamFrame>,
) -> Result<()> {
    let (packet_tx, packet_rx) = mpsc::channel::<WebRTCPacket>(10);

    // spawn video processing task
    let frame_tx_clone = frame_tx.clone();
    tokio::spawn(run_video_processor(packet_rx, frame_tx_clone));

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

    // create and send offer
    let offer = peer_connection.create_offer(None).await?;
    let sdp = serde_json::to_string(&offer)?;
    peer_connection.set_local_description(offer).await?;
    let sdp_data = SdpData { sdp, password };

    println!("Sending SDP to server at {}...", address);

    let client = reqwest::Client::new();
    let res = client
        .post(format!("{}:{}/sdp", address.ip(), address.port()))
        .json(&sdp_data)
        .send()
        .await?;

    if !res.status().is_success() {
        eprintln!("Failed to connect to server: {}", res.status());
        return Err(anyhow::anyhow!("Failed to connect to server"));
    }

    // get answer
    let answer_text = res.text().await?;
    let answer: RTCSessionDescription = serde_json::from_str(&answer_text)?;

    peer_connection.set_remote_description(answer).await?;

    println!("Connected to server at {}", address);

    Ok(())
}

async fn process_video_track(track: Arc<TrackRemote>, packet_tx: mpsc::Sender<WebRTCPacket>) {
    loop {
        // Read RTP packet from track
        let (rtp_packet, _) = match track.read_rtp().await {
            Ok(packet) => packet,
            Err(e) => {
                eprintln!("Error reading RTP packet: {}", e);
                break;
            }
        };

        let raw_packet = WebRTCPacket {
            data: rtp_packet.payload.to_vec(),
            timestamp: rtp_packet.header.timestamp,
        };

        if let Err(err) = packet_tx.send(raw_packet).await {
            eprintln!("Failed to send RTP packet for processing: {}", err);
            break;
        }
    }
}

async fn run_video_processor(
    mut packet_rx: mpsc::Receiver<WebRTCPacket>,
    frame_tx: mpsc::Sender<StreamFrame>,
) -> Result<()> {
    ffmpeg::init()?;

    let codec = ffmpeg::codec::decoder::find(ffmpeg::codec::Id::H264)
        .ok_or(anyhow::anyhow!("H264 decoder not found"))?;

    let context = ffmpeg::codec::context::Context::new_with_codec(codec);
    let mut decoder = context.decoder().video()?;

    decoder.set_threading(ffmpeg_next::threading::Config {
        kind: ffmpeg::threading::Type::Frame,
        count: 0,
    });

    let mut raw_frame = ffmpeg::frame::Video::empty();
    let mut rgb_frame = ffmpeg::frame::Video::empty();

    while let Ok(webrtc_packet) = packet_rx.try_recv() {
        // Set packet data and timestamp
        let mut packet = ffmpeg::packet::Packet::copy(&webrtc_packet.data);
        packet.set_pts(Some(webrtc_packet.timestamp as i64));

        // Send packet to decoder
        if let Err(e) = decoder.send_packet(&packet) {
            eprintln!("Error sending packet to decoder: {}", e);
            continue;
        }

        // Receive decoded frames
        while decoder.receive_frame(&mut raw_frame).is_ok() {
            // Convert frame to RGB format for pixel buffer
            let stream_frame = {
                let mut scaler = ffmpeg::software::scaling::context::Context::get(
                    raw_frame.format(),
                    raw_frame.width(),
                    raw_frame.height(),
                    ffmpeg::format::Pixel::RGBA,
                    raw_frame.width(),
                    raw_frame.height(),
                    ffmpeg::software::scaling::Flags::BILINEAR,
                )?;

                scaler.run(&raw_frame, &mut rgb_frame)?;

                // copy pixel data out while scaler is still alive
                let width = rgb_frame.width() as usize;
                let height = rgb_frame.height() as usize;
                let data = rgb_frame.data(0).to_vec();

                StreamFrame {
                    data,
                    width: width as u32,
                    height: height as u32,
                }
            };

            if let Err(e) = frame_tx.send(stream_frame).await {
                eprintln!("Failed to send decoded frame: {}", e);
                break;
            }
        }
    }

    Ok(())
}
