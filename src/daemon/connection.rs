use anyhow::{Context, Result};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::protocol::{PacketHeader, PacketType};

/// TCP 连接状态
#[derive(Debug)]
pub struct Connection {
    stream: TcpStream,
    remote_addr: SocketAddr,
}

impl Connection {
    /// 连接到远程地址
    pub async fn connect(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr).await.context("Failed to connect")?;
        let remote_addr = stream.peer_addr().context("Failed to get peer addr")?;
        Ok(Self { stream, remote_addr })
    }

    /// 接受连接
    pub async fn accept(listener: &TcpListener) -> Result<(Self, SocketAddr)> {
        let (stream, addr) = listener.accept().await.context("Failed to accept")?;
        Ok((Self { stream, remote_addr: addr }, addr))
    }

    /// 获取远程地址
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    /// 发送原始数据包
    pub async fn send_packet(&mut self, packet_type: PacketType, payload: &[u8]) -> Result<()> {
        let header = PacketHeader {
            packet_type,
            payload_len: payload.len() as u32,
        };
        let mut buf = vec![0u8; PacketHeader::SIZE + payload.len()];
        header.encode(&mut buf);
        buf[PacketHeader::SIZE..].copy_from_slice(payload);
        self.stream.write_all(&buf).await.context("Failed to send")?;
        Ok(())
    }

    /// 接收原始数据包
    pub async fn recv_packet(&mut self) -> Result<(PacketType, Vec<u8>)> {
        let mut header_buf = [0u8; PacketHeader::SIZE];
        self.stream.read_exact(&mut header_buf).await.context("Failed to read header")?;

        let header = PacketHeader::decode(&header_buf).context("Invalid packet header")?;

        let mut payload = vec![0u8; header.payload_len as usize];
        if payload.len() > 0 {
            self.stream.read_exact(&mut payload).await.context("Failed to read payload")?;
        }

        Ok((header.packet_type, payload))
    }
}

/// 创建 TCP 监听器
pub async fn create_listener(addr: &str) -> Result<TcpListener> {
    let listener = TcpListener::bind(addr).await.context("Failed to bind")?;
    Ok(listener)
}
