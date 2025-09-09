use anyhow::Result;
use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use warp::Filter;
use webrtc::{
    api::{
        APIBuilder,
        media_engine::{MIME_TYPE_H264, MediaEngine},
    },
    peer_connection::{
        configuration::RTCConfiguration, peer_connection_state::RTCPeerConnectionState,
        sdp::session_description::RTCSessionDescription,
    },
    rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType},
    track::track_local::{TrackLocal, track_local_static_sample::TrackLocalStaticSample},
};

use super::{AppState, ConnectionState};

#[derive(Serialize, Deserialize)]
struct SdpData {
    sdp: String,
    password: Option<String>,
}

#[derive(Debug)]
#[allow(dead_code)]
struct ErrorMessage(pub String);

impl warp::reject::Reject for ErrorMessage {}

pub async fn build_route(
    state: Arc<AppState>,
) -> Result<impl warp::Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone> {
    let cors = warp::cors()
        .allow_any_origin()
        .allow_headers(vec!["content-type"])
        .allow_methods(vec!["POST", "GET", "OPTIONS"]);

    let route = warp::post()
        .and(warp::path("sdp"))
        .and(warp::body::json::<SdpData>())
        .and(with_app_state(state.clone()))
        .and_then(sdp_handler)
        .with(cors);

    Ok(route)
}

fn with_app_state(
    state: Arc<AppState>,
) -> impl warp::Filter<Extract = (Arc<AppState>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || state.clone())
}

async fn sdp_handler(
    sdp_data: SdpData,
    state: Arc<AppState>,
) -> Result<impl warp::Reply, warp::Rejection> {
    // verify password if set
    if let Some(password) = &state.password {
        if password != &sdp_data.password.unwrap_or_default() {
            eprintln!("Invalid password attempt");
            return Err(warp::reject::custom(ErrorMessage(
                "Invalid password".to_string(),
            )));
        }
    }

    // if already connected or connecting, reject new connection
    if *state.connection.lock().await != ConnectionState::Disconnected {
        println!("Connection already in progress or established");
        return Err(warp::reject::custom(ErrorMessage(
            "Connection already in progress or established".to_string(),
        )));
    }

    let offer_bytes = general_purpose::STANDARD.decode(sdp_data.sdp).unwrap();
    let offer = serde_json::from_slice::<RTCSessionDescription>(&offer_bytes).unwrap();

    // create new peer connection
    let pc = create_peer_connection().await.unwrap();
    *state.peer_connection.lock().await = Some(pc.clone());

    // prepare local video track
    let video_track = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(),
            ..Default::default()
        },
        "video".to_owned(),
        "webrtc-rs".to_owned(),
    ));
    *state.video_track.lock().await = Some(video_track.clone());

    let rtp_sender = pc
        .add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
        .await
        .unwrap();

    // read incoming RTCP packets
    tokio::spawn(async move {
        let mut rtcp_buf = vec![0u8; 1500];
        while let Ok((_, _)) = rtp_sender.read(&mut rtcp_buf).await {}
    });

    // set handler for peer connection state
    let state_clone = state.clone();
    pc.on_peer_connection_state_change(Box::new(move |s| {
        println!("Peer connection State has changed: {}", s);
        let state_clone = state_clone.clone();
        Box::pin(async move {
            if s == RTCPeerConnectionState::Disconnected
                || s == RTCPeerConnectionState::Closed
                || s == RTCPeerConnectionState::Failed
            {
                *state_clone.connection.lock().await = ConnectionState::Disconnected;
                *state_clone.peer_connection.lock().await = None;
                *state_clone.video_track.lock().await = None;
            }
        })
    }));

    *state.connection.lock().await = ConnectionState::Connecting;

    // set remote description
    pc.set_remote_description(offer).await.unwrap();
    let answer = pc.create_answer(None).await.unwrap();
    let mut gather_complete = pc.gathering_complete_promise().await;
    pc.set_local_description(answer).await.unwrap();
    let _ = gather_complete.recv().await;

    if let Some(local_desc) = pc.local_description().await {
        let json_str = serde_json::to_string(&local_desc).unwrap();
        let b64 = general_purpose::STANDARD.encode(json_str);
        let response = SdpData {
            sdp: b64,
            password: None,
        };

        *state.connection.lock().await = ConnectionState::Connected;
        println!("Peer connected successfully");

        Ok(warp::reply::json(&response))
    } else {
        eprintln!("Failed to get local description");
        return Err(warp::reject::custom(ErrorMessage(
            "Failed to get local description".to_string(),
        )));
    }
}

async fn create_peer_connection() -> Result<Arc<webrtc::peer_connection::RTCPeerConnection>> {
    let mut m = MediaEngine::default();
    m.register_codec(
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_owned(),
                clock_rate: 90000,
                channels: 0,
                ..Default::default()
            },
            ..Default::default()
        },
        RTPCodecType::Video,
    )?;

    let api = APIBuilder::new().with_media_engine(m).build();
    let config = RTCConfiguration {
        ice_servers: vec![],
        ..Default::default()
    };
    let pc = api.new_peer_connection(config).await?;

    Ok(Arc::new(pc))
}
