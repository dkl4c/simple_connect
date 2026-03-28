pub mod connection;
pub mod heartbeat;
pub mod protocol;
pub mod session;
pub mod session_store;
pub mod transfer;

use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::sync::RwLock;
use tracing;

pub use connection::{create_listener, Connection};
pub use heartbeat::HeartbeatManager;
pub use protocol::*;
pub use session::{SessionManager, TransferSession};
pub use transfer::TransferEngine;

/// 默认块大小 64KB
pub const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;

/// 默认心跳间隔 (秒)
pub const DEFAULT_HEARTBEAT_INTERVAL: u64 = 30;

/// 默认超时 (秒)
pub const DEFAULT_TIMEOUT: u64 = 60;

/// Daemon 配置
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub listen_addr: Option<String>,
    pub connect_addr: Option<String>,
    pub storage_dir: PathBuf,
    pub chunk_size: usize,
    pub heartbeat_interval: u64,
    pub timeout: u64,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            listen_addr: None,
            connect_addr: None,
            storage_dir: PathBuf::from("./downloads"),
            chunk_size: DEFAULT_CHUNK_SIZE,
            heartbeat_interval: DEFAULT_HEARTBEAT_INTERVAL,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// 全局 Daemon 状态
pub struct DaemonState {
    pub config: DaemonConfig,
    pub transfer_engine: TransferEngine,
    pub heartbeat_manager: HeartbeatManager,
    pub connected: bool,
}

impl DaemonState {
    pub fn new(config: DaemonConfig) -> Self {
        Self {
            transfer_engine: TransferEngine::new(config.chunk_size),
            heartbeat_manager: HeartbeatManager::new(
                config.heartbeat_interval,
                config.timeout,
            ),
            config,
            connected: false,
        }
    }
}

/// 启动监听模式 (Windows 端)
pub async fn start_listener(addr: &str, config: DaemonConfig) -> Result<()> {
    let listener = create_listener(addr).await?;
    tracing::info!("Listening on {}", addr);

    // 创建共享状态
    let state = std::sync::Arc::new(RwLock::new(DaemonState::new(config)));

    loop {
        let (conn, peer_addr) = Connection::accept(&listener).await?;
        tracing::info!("Accepted connection from {}", peer_addr);

        let state_clone = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(conn, state_clone).await {
                tracing::debug!("Connection ended: {}", e);
            }
        });
    }
}

/// 启动连接模式 (Linux 端)
pub async fn start_connector(addr: &str, config: DaemonConfig) -> Result<()> {
    tracing::info!("Connecting to {}", addr);
    let mut connection = Connection::connect(addr).await?;
    tracing::info!("Connected to {}", addr);

    let state = std::sync::Arc::new(RwLock::new(DaemonState::new(config)));

    // 发送握手
    let handshake = protocol::HandshakeRequest {
        version: 1,
        hostname: hostname::get()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
    };
    let payload = serde_json::to_vec(&handshake)?;
    connection.send_packet(protocol::PacketType::Handshake, &payload).await?;
    tracing::debug!("Sent handshake");

    // 等待握手响应
    let (packet_type, resp_payload) = connection.recv_packet().await?;
    if packet_type != protocol::PacketType::HandshakeAck {
        anyhow::bail!("Expected HandshakeAck, got {:?}", packet_type);
    }
    let ack: protocol::HandshakeResponse = serde_json::from_slice(&resp_payload)?;
    if !ack.accepted {
        anyhow::bail!("Handshake rejected: {}", ack.message.unwrap_or_default());
    }
    tracing::info!("Handshake successful, version {}", ack.version);

    // 更新连接状态
    {
        let mut state_guard = state.write().await;
        state_guard.connected = true;
        state_guard.heartbeat_manager.set_connected();
    }

    // 处理数据包循环
    loop {
        let result = connection.recv_packet().await;
        if result.is_err() {
            tracing::warn!("Connection closed: {}", result.err().unwrap());
            break;
        }

        let (packet_type, payload) = result.unwrap();

        match packet_type {
            protocol::PacketType::Heartbeat => {
                tracing::debug!("Received heartbeat");
                connection.send_packet(protocol::PacketType::HeartbeatAck, &[]).await?;
            }
            _ => {
                tracing::debug!("Received packet: {:?}", packet_type);
            }
        }
    }

    Ok(())
}

/// 处理连接
async fn handle_connection(mut conn: Connection, state: std::sync::Arc<RwLock<DaemonState>>) -> Result<()> {
    // 等待接收握手
    tracing::debug!("Waiting for handshake");
    let (packet_type, payload) = conn.recv_packet().await?;
    if packet_type != protocol::PacketType::Handshake {
        anyhow::bail!("Expected Handshake, got {:?}", packet_type);
    }

    let _request: protocol::HandshakeRequest = serde_json::from_slice(&payload)?;

    // 发送握手响应
    let response = protocol::HandshakeResponse {
        version: 1,
        accepted: true,
        message: None,
    };
    let response_payload = serde_json::to_vec(&response)?;
    conn.send_packet(protocol::PacketType::HandshakeAck, &response_payload).await?;
    tracing::info!("Handshake completed");

    // 更新连接状态
    {
        let mut state_guard = state.write().await;
        state_guard.connected = true;
        state_guard.heartbeat_manager.set_connected();
    }

    // 处理数据包循环
    loop {
        let result = conn.recv_packet().await;
        if result.is_err() {
            tracing::warn!("Connection closed: {}", result.err().unwrap());
            break;
        }

        let (packet_type, payload) = result.unwrap();

        match packet_type {
            protocol::PacketType::Heartbeat => {
                tracing::debug!("Received heartbeat");
                conn.send_packet(protocol::PacketType::HeartbeatAck, &[]).await?;
                let mut state_guard = state.write().await;
                state_guard.heartbeat_manager.handle_heartbeat_ack();
            }
            protocol::PacketType::HeartbeatAck => {
                tracing::debug!("Received heartbeat ack");
                let mut state_guard = state.write().await;
                state_guard.heartbeat_manager.handle_heartbeat_ack();
            }
            protocol::PacketType::TransferRequest => {
                let request: protocol::TransferRequest = match serde_json::from_slice(&payload) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("Failed to parse TransferRequest: {}", e);
                        continue;
                    }
                };
                tracing::info!("Transfer request: {} ({} bytes)", request.filename, request.filesize);

                let state_guard = state.read().await;
                let local_path = state_guard.config.storage_dir.join(&request.filename);
                drop(state_guard);

                let mut state_guard = state.write().await;
                let response = state_guard.transfer_engine
                    .handle_transfer_request(&request, &local_path)
                    .await;

                let response_payload = serde_json::to_vec(&response)?;
                conn.send_packet(protocol::PacketType::TransferResponse, &response_payload).await?;
                tracing::info!("Transfer response: accepted={}, resume_offset={}", response.accepted, response.resume_offset);
            }
            protocol::PacketType::Chunk => {
                let chunk: protocol::Chunk = match serde_json::from_slice(&payload) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("Failed to parse Chunk: {}", e);
                        continue;
                    }
                };

                let mut state_guard = state.write().await;
                match state_guard.transfer_engine.handle_chunk(&chunk).await {
                    Ok(ack) => {
                        let response_payload = serde_json::to_vec(&ack)?;
                        conn.send_packet(protocol::PacketType::ChunkAck, &response_payload).await?;
                        tracing::debug!("Chunk ack sent for offset {}", ack.offset);
                    }
                    Err(e) => {
                        tracing::error!("Failed to handle chunk: {}", e);
                        let err = protocol::ErrorPacket {
                            code: 1,
                            message: e.to_string(),
                        };
                        let payload = serde_json::to_vec(&err)?;
                        conn.send_packet(protocol::PacketType::Error, &payload).await?;
                    }
                }
            }
            protocol::PacketType::Cancel => {
                tracing::info!("Received cancel");
                break;
            }
            protocol::PacketType::Pause => {
                if let Ok(pause) = serde_json::from_slice::<protocol::PauseResume>(&payload) {
                    let mut state_guard = state.write().await;
                    state_guard.transfer_engine.pause_transfer(&pause.transfer_id);
                    tracing::info!("Paused transfer {}", pause.transfer_id);
                }
            }
            protocol::PacketType::Resume => {
                if let Ok(resume) = serde_json::from_slice::<protocol::PauseResume>(&payload) {
                    let mut state_guard = state.write().await;
                    state_guard.transfer_engine.resume_transfer(&resume.transfer_id);
                    tracing::info!("Resumed transfer {}", resume.transfer_id);
                }
            }
            _ => {
                tracing::debug!("Unhandled packet type: {:?}", packet_type);
            }
        }
    }

    // 断开连接
    let mut state_guard = state.write().await;
    state_guard.connected = false;
    state_guard.heartbeat_manager.set_disconnected();

    Ok(())
}
