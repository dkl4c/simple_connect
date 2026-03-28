use anyhow::{Context, Result};
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use uuid::Uuid;

use super::protocol::{Chunk, ChunkAck, TransferRequest, TransferResponse};
use super::session::SessionManager;
use super::Connection;

/// 并发窗口大小
const WINDOW_SIZE: usize = 4;

/// 活跃传输项
#[derive(Debug)]
pub struct ActiveTransfer {
    pub transfer_id: Uuid,
    pub filename: String,
    pub filesize: u64,
    pub offset: u64,
    pub direction: String,
}

/// 传输引擎
pub struct TransferEngine {
    chunk_size: usize,
    session_manager: SessionManager,
}

impl TransferEngine {
    pub fn new(chunk_size: usize) -> Self {
        Self {
            chunk_size,
            session_manager: SessionManager::new(),
        }
    }

    /// 接收传输请求并返回响应
    pub async fn handle_transfer_request(
        &mut self,
        request: &TransferRequest,
        local_path: &Path,
    ) -> TransferResponse {
        let existing_offset = self.get_resume_offset(&request.transfer_id);

        // 确保目录存在
        if let Some(parent) = local_path.parent() {
            if !parent.exists() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return TransferResponse {
                        transfer_id: request.transfer_id,
                        accepted: false,
                        resume_offset: 0,
                        message: Some(format!("Failed to create directory: {}", e)),
                    };
                }
            }
        }

        // 创建会话
        self.session_manager.create_session(
            request.transfer_id,
            request.filename.clone(),
            request.filesize,
            local_path.to_path_buf(),
        );
        self.session_manager.update_offset(&request.transfer_id, existing_offset);

        TransferResponse {
            transfer_id: request.transfer_id,
            accepted: true,
            resume_offset: existing_offset,
            message: None,
        }
    }

    /// 获取恢复偏移量
    pub fn get_resume_offset(&self, transfer_id: &Uuid) -> u64 {
        if let Some(session) = self.session_manager.get_session(transfer_id) {
            match &session.state {
                super::session::SessionState::InProgress { offset } => *offset,
                super::session::SessionState::Paused { offset } => *offset,
                _ => 0,
            }
        } else {
            0
        }
    }

    /// 处理数据块
    pub async fn handle_chunk(&mut self, chunk: &Chunk) -> Result<ChunkAck> {
        // 获取会话
        let session = self
            .session_manager
            .get_session(&chunk.transfer_id)
            .context("Session not found")?;

        // 确保目录存在
        if let Some(parent) = session.local_path.parent() {
            if !parent.exists() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }

        // 打开文件（写入模式，会创建文件）
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&session.local_path)
            .await
            .context("Failed to open file")?;

        // 定位到偏移量
        use tokio::io::AsyncSeekExt;
        file.seek(tokio::io::SeekFrom::Start(chunk.offset))
            .await
            .context("Failed to seek")?;

        // 写入数据
        file.write_all(&chunk.data)
            .await
            .context("Failed to write")?;

        // 确保数据刷到磁盘
        file.sync_all().await.context("Failed to sync")?;

        // 更新进度
        let new_offset = chunk.offset + chunk.data.len() as u64;
        self.session_manager.update_offset(&chunk.transfer_id, new_offset);

        // 构造确认
        let ack = ChunkAck {
            transfer_id: chunk.transfer_id,
            offset: new_offset,
            bytes_received: new_offset,
        };

        Ok(ack)
    }

    /// 发送文件（作为客户端）
    pub async fn send_file(
        &mut self,
        connection: &mut Connection,
        transfer_id: Uuid,
        local_path: &Path,
        resume_offset: u64,
    ) -> Result<()> {
        let mut file = File::open(local_path).await.context("Failed to open file")?;
        let metadata = file.metadata().await.context("Failed to get metadata")?;
        let filesize = metadata.len();

        // 如果有恢复偏移，先跳到那里
        if resume_offset > 0 {
            file.seek(tokio::io::SeekFrom::Start(resume_offset))
                .await
                .context("Failed to seek to resume offset")?;
        }

        let mut offset = resume_offset;
        let mut in_flight = 0u32;

        // 发送文件循环
        loop {
            // 发送 chunk 直到窗口满
            while in_flight < WINDOW_SIZE as u32 {
                let mut chunk_data = vec![0u8; self.chunk_size];
                let bytes_read = file.read(&mut chunk_data).await.context("Failed to read")?;

                // 文件结束
                if bytes_read == 0 {
                    break;
                }

                // 截取实际读取的长度
                chunk_data.truncate(bytes_read);

                let chunk = Chunk {
                    transfer_id,
                    offset,
                    data: chunk_data,
                };

                let payload = serde_json::to_vec(&chunk)?;
                connection
                    .send_packet(super::protocol::PacketType::Chunk, &payload)
                    .await
                    .context("Failed to send chunk")?;

                offset += bytes_read as u64;
                in_flight += 1;
            }

            // 如果没有在飞的包且文件读完了，退出
            if in_flight == 0 {
                break;
            }

            // 等待确认
            let (packet_type, payload) = connection.recv_packet().await.context("Failed to recv")?;
            match packet_type {
                super::protocol::PacketType::ChunkAck => {
                    let ack: ChunkAck = serde_json::from_slice(&payload)?;
                    if ack.transfer_id == transfer_id {
                        in_flight = in_flight.saturating_sub(1);
                        self.session_manager.update_offset(&transfer_id, ack.offset);
                    }
                }
                super::protocol::PacketType::Cancel => {
                    return Err(anyhow::anyhow!("Transfer cancelled by remote"));
                }
                super::protocol::PacketType::Error => {
                    let err: super::protocol::ErrorPacket = serde_json::from_slice(&payload)?;
                    return Err(anyhow::anyhow!("Remote error: {}", err.message));
                }
                _ => {}
            }

            // 检查是否完成
            if offset >= filesize && in_flight == 0 {
                break;
            }
        }

        self.session_manager.complete_session(&transfer_id);
        Ok(())
    }

    /// 接收文件（作为客户端）
    pub async fn recv_file(
        &mut self,
        connection: &mut Connection,
        transfer_id: Uuid,
        filesize: u64,
    ) -> Result<()> {
        loop {
            let (packet_type, payload) = connection.recv_packet().await.context("Failed to recv")?;

            match packet_type {
                super::protocol::PacketType::Chunk => {
                    let chunk: Chunk = serde_json::from_slice(&payload)?;

                    // 检查 transfer_id
                    if chunk.transfer_id != transfer_id {
                        continue;
                    }

                    // 处理 chunk
                    let ack = self.handle_chunk(&chunk).await?;

                    // 发送确认
                    let payload = serde_json::to_vec(&ack)?;
                    connection
                        .send_packet(super::protocol::PacketType::ChunkAck, &payload)
                        .await
                        .context("Failed to send ack")?;

                    // 检查是否完成
                    if ack.bytes_received >= filesize {
                        break;
                    }
                }
                super::protocol::PacketType::Cancel => {
                    self.session_manager.cancel_session(&transfer_id);
                    return Err(anyhow::anyhow!("Transfer cancelled"));
                }
                super::protocol::PacketType::Error => {
                    let err: super::protocol::ErrorPacket = serde_json::from_slice(&payload)?;
                    return Err(anyhow::anyhow!("Remote error: {}", err.message));
                }
                _ => {}
            }
        }

        self.session_manager.complete_session(&transfer_id);
        Ok(())
    }

    /// 获取所有活跃传输
    pub fn get_active_transfers(&self) -> Vec<ActiveTransfer> {
        self.session_manager
            .get_active_sessions()
            .iter()
            .map(|s| {
                let (offset, direction) = match &s.state {
                    super::session::SessionState::InProgress { offset } => (*offset, "in_progress"),
                    super::session::SessionState::Paused { offset } => (*offset, "paused"),
                    super::session::SessionState::Pending => (0, "pending"),
                    _ => (0, "unknown"),
                };
                ActiveTransfer {
                    transfer_id: s.transfer_id,
                    filename: s.filename.clone(),
                    filesize: s.filesize,
                    offset,
                    direction: direction.to_string(),
                }
            })
            .collect()
    }

    /// 暂停传输
    pub fn pause_transfer(&mut self, transfer_id: &Uuid) {
        self.session_manager.pause_session(transfer_id);
    }

    /// 恢复传输
    pub fn resume_transfer(&mut self, transfer_id: &Uuid) {
        self.session_manager.resume_session(transfer_id);
    }

    /// 取消传输
    pub fn cancel_transfer(&mut self, transfer_id: &Uuid) {
        self.session_manager.cancel_session(transfer_id);
    }
}
