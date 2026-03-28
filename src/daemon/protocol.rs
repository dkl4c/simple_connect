use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 包类型枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PacketType {
    // 握手
    Handshake = 0x01,
    HandshakeAck = 0x02,

    // 文件传输
    TransferRequest = 0x10,
    TransferResponse = 0x11,
    Chunk = 0x12,
    ChunkAck = 0x13,

    // 传输控制
    Pause = 0x20,
    Resume = 0x21,
    Cancel = 0x22,

    // 心跳
    Heartbeat = 0x30,
    HeartbeatAck = 0x31,

    // 错误
    Error = 0xFF,
}

impl PacketType {
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0x01 => Some(PacketType::Handshake),
            0x02 => Some(PacketType::HandshakeAck),
            0x10 => Some(PacketType::TransferRequest),
            0x11 => Some(PacketType::TransferResponse),
            0x12 => Some(PacketType::Chunk),
            0x13 => Some(PacketType::ChunkAck),
            0x20 => Some(PacketType::Pause),
            0x21 => Some(PacketType::Resume),
            0x22 => Some(PacketType::Cancel),
            0x30 => Some(PacketType::Heartbeat),
            0x31 => Some(PacketType::HeartbeatAck),
            0xFF => Some(PacketType::Error),
            _ => None,
        }
    }
}

/// 数据包头
#[derive(Debug)]
pub struct PacketHeader {
    pub packet_type: PacketType,
    pub payload_len: u32,
}

impl PacketHeader {
    pub const SIZE: usize = 8; // 4 bytes type + 4 bytes length

    pub fn encode(&self, buf: &mut [u8]) {
        debug_assert!(buf.len() >= Self::SIZE);
        let be_type = self.packet_type as u32;
        let be_len = self.payload_len;
        buf[0..4].copy_from_slice(&be_type.to_be_bytes());
        buf[4..8].copy_from_slice(&be_len.to_be_bytes());
    }

    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::SIZE {
            return None;
        }
        let be_type = u32::from_be_bytes(buf[0..4].try_into().ok()?);
        let be_len = u32::from_be_bytes(buf[4..8].try_into().ok()?);
        let packet_type = PacketType::from_u32(be_type)?;
        Some(PacketHeader {
            packet_type,
            payload_len: be_len,
        })
    }
}

/// 握手请求
#[derive(Debug, Serialize, Deserialize)]
pub struct HandshakeRequest {
    pub version: u32,
    pub hostname: String,
}

/// 握手响应
#[derive(Debug, Serialize, Deserialize)]
pub struct HandshakeResponse {
    pub version: u32,
    pub accepted: bool,
    pub message: Option<String>,
}

/// 传输方向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    Send, // 客户端 -> 服务器
    Recv, // 服务器 -> 客户端
}

/// 传输请求
#[derive(Debug, Serialize, Deserialize)]
pub struct TransferRequest {
    pub transfer_id: Uuid,
    pub filename: String,
    pub filesize: u64,
    pub direction: TransferDirection,
}

/// 传输响应
#[derive(Debug, Serialize, Deserialize)]
pub struct TransferResponse {
    pub transfer_id: Uuid,
    pub accepted: bool,
    pub resume_offset: u64,
    pub message: Option<String>,
}

/// 数据块
#[derive(Debug, Serialize, Deserialize)]
pub struct Chunk {
    pub transfer_id: Uuid,
    pub offset: u64,
    pub data: Vec<u8>,
}

/// 块确认
#[derive(Debug, Serialize, Deserialize)]
pub struct ChunkAck {
    pub transfer_id: Uuid,
    pub offset: u64,
    pub bytes_received: u64,
}

/// 错误包
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorPacket {
    pub code: u32,
    pub message: String,
}

/// 暂停/恢复请求
#[derive(Debug, Serialize, Deserialize)]
pub struct PauseResume {
    pub transfer_id: Uuid,
}
