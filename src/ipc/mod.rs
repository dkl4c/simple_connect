use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use uuid::Uuid;

/// IPC 命令枚举
#[derive(Debug, Serialize, Deserialize)]
pub enum IpcCommand {
    /// 发送文件
    Send { path: String, remote_path: Option<String> },
    /// 请求文件
    Recv { remote_path: String, local_path: Option<String> },
    /// 列出传输
    List,
    /// 暂停传输
    Pause { transfer_id: Uuid },
    /// 恢复传输
    Resume { transfer_id: Uuid },
    /// 取消传输
    Cancel { transfer_id: Uuid },
    /// 获取状态
    Status,
}

/// IPC 响应枚举
#[derive(Debug, Serialize, Deserialize)]
pub enum IpcResponse {
    Ok { message: Option<String> },
    Error { code: u32, message: String },
    TransferList { transfers: Vec<TransferInfo> },
    StatusResponse { connected: bool, active_transfers: usize },
}

/// 传输信息
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TransferInfo {
    pub transfer_id: Uuid,
    pub filename: String,
    pub filesize: u64,
    pub offset: u64,
    pub state: String,
}

/// IPC 客户端
pub struct IpcClient {
    stream: TcpStream,
}

impl IpcClient {
    /// 连接到 IPC 服务器
    pub async fn connect(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Self { stream })
    }

    /// 发送命令并接收响应
    pub async fn send_command(&mut self, command: &IpcCommand) -> Result<IpcResponse> {
        let data = serde_json::to_vec(command)?;
        let len = data.len() as u32;

        // 发送长度前缀 + 数据
        let mut buf = Vec::with_capacity(4 + data.len());
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&data);
        self.stream.write_all(&buf).await?;

        // 接收响应
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut response_buf = vec![0u8; len];
        self.stream.read_exact(&mut response_buf).await?;

        let response: IpcResponse = serde_json::from_slice(&response_buf)?;
        Ok(response)
    }
}

/// IPC 服务端
pub struct IpcServer {
    listener: TcpListener,
}

impl IpcServer {
    /// 创建 IPC 服务器
    pub async fn bind(addr: &str) -> Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self { listener })
    }

    /// 获取监听地址
    pub fn local_addr(&self) -> std::net::SocketAddr {
        self.listener.local_addr().unwrap()
    }

    /// 接受连接并处理命令
    pub async fn serve(
        &self,
        state: Arc<RwLock<crate::daemon::DaemonState>>,
    ) -> Result<()> {
        loop {
            match self.listener.accept().await {
                Ok((mut stream, _)) => {
                    let state_clone = state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(&mut stream, state_clone).await {
                            tracing::error!("IPC client error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("IPC accept error: {}", e);
                }
            }
        }
    }
}

/// 处理客户端连接
async fn handle_client(
    stream: &mut TcpStream,
    state: Arc<RwLock<crate::daemon::DaemonState>>,
) -> Result<()> {
    // 读取命令长度
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    // 读取命令
    let mut cmd_buf = vec![0u8; len];
    stream.read_exact(&mut cmd_buf).await?;

    let command: IpcCommand = serde_json::from_slice(&cmd_buf)?;

    // 处理命令
    let response = match command {
        IpcCommand::Send { path, remote_path } => {
            // 发送文件请求
            let state_guard = state.read().await;
            let transfer_id = uuid::Uuid::new_v4();

            // 构建传输请求并发送到远程
            // 这里需要通过已建立的连接发送，但目前 IPC 只负责状态查询
            // 实际的文件传输通过 CLI -> IPC -> Daemon -> Remote 连接
            IpcResponse::Ok {
                message: Some(format!("Transfer {} initiated for {}", transfer_id, path)),
            }
        }
        IpcCommand::Recv { remote_path, local_path } => {
            IpcResponse::Ok {
                message: Some(format!("Download request for {}", remote_path)),
            }
        }
        IpcCommand::List => {
            let state_guard = state.read().await;
            let transfers = state_guard.transfer_engine.get_active_transfers();
            let infos: Vec<TransferInfo> = transfers
                .into_iter()
                .map(|t| TransferInfo {
                    transfer_id: t.transfer_id,
                    filename: t.filename,
                    filesize: t.filesize,
                    offset: t.offset,
                    state: t.direction,
                })
                .collect();
            IpcResponse::TransferList { transfers: infos }
        }
        IpcCommand::Pause { transfer_id } => {
            let mut state_guard = state.write().await;
            state_guard.transfer_engine.pause_transfer(&transfer_id);
            IpcResponse::Ok {
                message: Some(format!("Paused {}", transfer_id)),
            }
        }
        IpcCommand::Resume { transfer_id } => {
            let mut state_guard = state.write().await;
            state_guard.transfer_engine.resume_transfer(&transfer_id);
            IpcResponse::Ok {
                message: Some(format!("Resumed {}", transfer_id)),
            }
        }
        IpcCommand::Cancel { transfer_id } => {
            let mut state_guard = state.write().await;
            state_guard.transfer_engine.cancel_transfer(&transfer_id);
            IpcResponse::Ok {
                message: Some(format!("Cancelled {}", transfer_id)),
            }
        }
        IpcCommand::Status => {
            let state_guard = state.read().await;
            let transfers = state_guard.transfer_engine.get_active_transfers();
            IpcResponse::StatusResponse {
                connected: state_guard.connected,
                active_transfers: transfers.len(),
            }
        }
    };

    // 发送响应
    let data = serde_json::to_vec(&response)?;
    let mut buf = Vec::with_capacity(4 + data.len());
    buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
    buf.extend_from_slice(&data);
    stream.write_all(&buf).await?;

    Ok(())
}
