use anyhow::{Context, Result};
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::daemon::{Connection, protocol};
use crate::ipc::{IpcClient, IpcCommand, IpcResponse};

/// CLI 命令处理
pub struct Cli;

impl Cli {
    /// 处理 send 命令 - 直接连接到远程并发送文件
    pub async fn send(path: &str, remote_path: Option<&str>) -> Result<()> {
        // 从配置文件或命令行参数获取远程地址
        // 目前硬编码为方便测试
        let remote_addr = std::env::var("REMOTE_ADDR").unwrap_or_else(|_| "127.0.0.1:18888".to_string());

        let local_path = Path::new(path);
        if !local_path.exists() {
            anyhow::bail!("File not found: {}", path);
        }

        // 连接到远程
        println!("Connecting to {}...", remote_addr);
        let mut connection = Connection::connect(&remote_addr).await?;

        // 发送握手
        let handshake = protocol::HandshakeRequest {
            version: 1,
            hostname: hostname::get()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
        };
        let payload = serde_json::to_vec(&handshake)?;
        connection.send_packet(protocol::PacketType::Handshake, &payload).await?;

        // 等待握手响应
        let (packet_type, resp_payload) = connection.recv_packet().await?;
        if packet_type != protocol::PacketType::HandshakeAck {
            anyhow::bail!("Handshake failed: expected HandshakeAck, got {:?}", packet_type);
        }
        let _ack: protocol::HandshakeResponse = serde_json::from_slice(&resp_payload)?;
        println!("Connected!");

        // 获取文件信息
        let filename: String = if let Some(rp) = remote_path {
            rp.to_string()
        } else {
            local_path.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string())
        };

        let metadata = tokio::fs::metadata(local_path).await?;
        let filesize = metadata.len();

        // 生成 transfer_id
        let transfer_id = Uuid::new_v4();

        // 发送传输请求
        let request = protocol::TransferRequest {
            transfer_id,
            filename: filename.clone(),
            filesize,
            direction: protocol::TransferDirection::Send,
        };
        let request_payload = serde_json::to_vec(&request)?;
        connection.send_packet(protocol::PacketType::TransferRequest, &request_payload).await?;
        println!("Sending {} ({} bytes)...", filename, filesize);

        // 等待响应
        let (resp_type, resp_data) = connection.recv_packet().await?;
        if resp_type != protocol::PacketType::TransferResponse {
            anyhow::bail!("Unexpected response: {:?}", resp_type);
        }
        let response: protocol::TransferResponse = serde_json::from_slice(&resp_data)?;
        if !response.accepted {
            anyhow::bail!("Transfer rejected: {}", response.message.unwrap_or_default());
        }

        let resume_offset = response.resume_offset;
        if resume_offset > 0 {
            println!("Resuming from offset {}", resume_offset);
        }

        // 打开文件并发送
        let mut file = File::open(local_path).await?;
        if resume_offset > 0 {
            use tokio::io::AsyncSeekExt;
            file.seek(tokio::io::SeekFrom::Start(resume_offset)).await?;
        }

        let chunk_size = 64 * 1024; // 64KB
        let mut offset = resume_offset;
        let mut buffer = vec![0u8; chunk_size];

        loop {
            let bytes_read = file.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }

            buffer.truncate(bytes_read);

            let chunk = protocol::Chunk {
                transfer_id,
                offset,
                data: buffer.clone(),
            };

            let chunk_payload = serde_json::to_vec(&chunk)?;
            connection.send_packet(protocol::PacketType::Chunk, &chunk_payload).await?;

            // 等待确认
            let (ack_type, ack_data) = connection.recv_packet().await?;
            match ack_type {
                protocol::PacketType::ChunkAck => {
                    let ack: protocol::ChunkAck = serde_json::from_slice(&ack_data)?;
                    if ack.transfer_id == transfer_id {
                        let progress = (ack.offset as f64 / filesize as f64 * 100.0) as u32;
                        println!("\rProgress: {}/{} ({}%)   ", ack.offset, filesize, progress);
                    }
                }
                protocol::PacketType::Error => {
                    let err: protocol::ErrorPacket = serde_json::from_slice(&ack_data)?;
                    anyhow::bail!("Remote error: {}", err.message);
                }
                _ => {}
            }

            offset += bytes_read as u64;
            buffer.resize(chunk_size, 0);
        }

        println!("\nTransfer complete!");
        Ok(())
    }

    /// 处理 recv 命令
    pub async fn recv(remote_path: &str, local_path: Option<&str>) -> Result<()> {
        println!("Note: To receive a file, run the sender first.");
        println!("The sender will connect to this machine and send the file.");
        println!("This terminal should be running in 'daemon --listen' mode to accept connections.");
        Ok(())
    }

    /// 处理 list 命令
    pub async fn list() -> Result<()> {
        let mut client = IpcClient::connect("127.0.0.1:18889").await?;
        let response = client.send_command(&IpcCommand::List).await?;

        match response {
            IpcResponse::TransferList { transfers } => {
                if transfers.is_empty() {
                    println!("No active transfers");
                } else {
                    for t in transfers {
                        println!(
                            "{} | {} | {}/{} | {}",
                            t.transfer_id,
                            t.filename,
                            t.offset,
                            t.filesize,
                            t.state
                        );
                    }
                }
            }
            IpcResponse::Error { code, message } => {
                eprintln!("Error {}: {}", code, message);
            }
            _ => {
                eprintln!("Unexpected response");
            }
        }

        Ok(())
    }

    /// 处理 pause 命令
    pub async fn pause(transfer_id: Uuid) -> Result<()> {
        let mut client = IpcClient::connect("127.0.0.1:18889").await?;
        let response = client.send_command(&IpcCommand::Pause { transfer_id }).await?;

        match response {
            IpcResponse::Ok { message } => {
                println!("OK: {}", message.unwrap_or_default());
            }
            IpcResponse::Error { code, message } => {
                eprintln!("Error {}: {}", code, message);
            }
            _ => {
                eprintln!("Unexpected response");
            }
        }

        Ok(())
    }

    /// 处理 resume 命令
    pub async fn resume(transfer_id: Uuid) -> Result<()> {
        let mut client = IpcClient::connect("127.0.0.1:18889").await?;
        let response = client.send_command(&IpcCommand::Resume { transfer_id }).await?;

        match response {
            IpcResponse::Ok { message } => {
                println!("OK: {}", message.unwrap_or_default());
            }
            IpcResponse::Error { code, message } => {
                eprintln!("Error {}: {}", code, message);
            }
            _ => {
                eprintln!("Unexpected response");
            }
        }

        Ok(())
    }

    /// 处理 cancel 命令
    pub async fn cancel(transfer_id: Uuid) -> Result<()> {
        let mut client = IpcClient::connect("127.0.0.1:18889").await?;
        let response = client.send_command(&IpcCommand::Cancel { transfer_id }).await?;

        match response {
            IpcResponse::Ok { message } => {
                println!("OK: {}", message.unwrap_or_default());
            }
            IpcResponse::Error { code, message } => {
                eprintln!("Error {}: {}", code, message);
            }
            _ => {
                eprintln!("Unexpected response");
            }
        }

        Ok(())
    }

    /// 处理 status 命令
    pub async fn status() -> Result<()> {
        let mut client = IpcClient::connect("127.0.0.1:18889").await?;
        let response = client.send_command(&IpcCommand::Status).await?;

        match response {
            IpcResponse::StatusResponse { connected, active_transfers } => {
                println!("Connected: {}", connected);
                println!("Active transfers: {}", active_transfers);
            }
            IpcResponse::Error { code, message } => {
                eprintln!("Error {}: {}", code, message);
            }
            _ => {
                eprintln!("Unexpected response");
            }
        }

        Ok(())
    }
}
