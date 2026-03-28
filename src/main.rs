mod cli;
mod daemon;
mod ipc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "simple_connect")]
#[command(about = "File transfer utility for Windows-Linux connections")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 发送文件到远程
    Send {
        /// 文件路径
        path: String,
        /// 远程保存路径
        #[arg(short, long)]
        remote: Option<String>,
    },
    /// 从远程接收文件
    Recv {
        /// 远程文件路径
        remote: String,
        /// 本地保存路径
        #[arg(short, long)]
        local: Option<String>,
    },
    /// 列出活动传输
    List,
    /// 暂停传输
    Pause {
        /// 传输 ID
        #[arg(short, long)]
        id: Uuid,
    },
    /// 恢复传输
    Resume {
        /// 传输 ID
        #[arg(short, long)]
        id: Uuid,
    },
    /// 取消传输
    Cancel {
        /// 传输 ID
        #[arg(short, long)]
        id: Uuid,
    },
    /// 查看状态
    Status,
    /// 启动守护进程
    Daemon {
        /// 监听地址 (Windows 模式)
        #[arg(short, long)]
        listen: Option<String>,
        /// 连接地址 (Linux 模式)
        #[arg(short, long)]
        connect: Option<String>,
        /// 存储目录
        #[arg(short, long, default_value = "./downloads")]
        storage: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Send { path, remote } => {
            cli::Cli::send(&path, remote.as_deref()).await?;
        }
        Commands::Recv { remote, local } => {
            cli::Cli::recv(&remote, local.as_deref()).await?;
        }
        Commands::List => {
            cli::Cli::list().await?;
        }
        Commands::Pause { id } => {
            cli::Cli::pause(id).await?;
        }
        Commands::Resume { id } => {
            cli::Cli::resume(id).await?;
        }
        Commands::Cancel { id } => {
            cli::Cli::cancel(id).await?;
        }
        Commands::Status => {
            cli::Cli::status().await?;
        }
        Commands::Daemon { listen, connect, storage } => {
            start_daemon(listen, connect, storage).await?;
        }
    }

    Ok(())
}

async fn start_daemon(
    listen: Option<String>,
    connect: Option<String>,
    storage: String,
) -> Result<()> {
    let config = daemon::DaemonConfig {
        listen_addr: listen,
        connect_addr: connect,
        storage_dir: std::path::PathBuf::from(storage),
        ..Default::default()
    };

    // 创建共享状态
    let state: Arc<RwLock<daemon::DaemonState>> =
        Arc::new(RwLock::new(daemon::DaemonState::new(config.clone())));

    // 启动 IPC 服务器 (在 127.0.0.1:18889)
    let ipc_state = state.clone();
    let ipc_handle = tokio::spawn(async move {
        let ipc_server = ipc::IpcServer::bind("127.0.0.1:18889").await?;
        tracing::info!("IPC server listening on 127.0.0.1:18889");
        ipc_server.serve(ipc_state).await
    });

    // 启动 daemon (TCP 连接处理)
    let daemon_handle = tokio::spawn(async move {
        if let Some(addr) = &config.listen_addr {
            tracing::info!("Starting daemon in listener mode on {}", addr);
            daemon::start_listener(addr, config.clone()).await
        } else if let Some(addr) = &config.connect_addr {
            tracing::info!("Starting daemon in connector mode to {}", addr);
            daemon::start_connector(addr, config.clone()).await
        } else {
            anyhow::bail!("Either --listen or --connect must be specified");
        }
    });

    // 等待 IPC 服务器结束（这应该永远不会发生）
    if let Err(e) = ipc_handle.await {
        tracing::error!("IPC server error: {}", e);
    }

    // 等待 daemon 结束
    if let Err(e) = daemon_handle.await {
        tracing::error!("Daemon error: {}", e);
    }

    Ok(())
}
