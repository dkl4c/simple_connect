use anyhow::Result;
use std::path::PathBuf;
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use super::session::{SessionManager, SessionState, TransferSession};

/// 会话存储 - 持久化传输会话
pub struct SessionStore {
    file_path: PathBuf,
}

impl SessionStore {
    pub fn new(file_path: PathBuf) -> Self {
        Self { file_path }
    }

    /// 从磁盘加载会话
    pub async fn load(&self) -> Result<SessionManager> {
        if !self.file_path.exists() {
            return Ok(SessionManager::new());
        }

        let mut file = File::open(&self.file_path).await?;
        let mut contents = vec![0u8; 8192];
        let bytes_read = file.read(&mut contents).await?;
        contents.truncate(bytes_read);

        if contents.is_empty() {
            return Ok(SessionManager::new());
        }

        let sessions: Vec<StoredSession> = serde_json::from_slice(&contents)?;
        let mut manager = SessionManager::new();

        for stored in sessions {
            let session = TransferSession {
                transfer_id: stored.transfer_id,
                filename: stored.filename,
                filesize: stored.filesize,
                local_path: stored.local_path,
                state: stored.state,
                created_at: stored.created_at,
                updated_at: stored.updated_at,
            };
            manager.restore_session(session);
        }

        Ok(manager)
    }

    /// 保存会话到磁盘
    pub async fn save(&self, manager: &SessionManager) -> Result<()> {
        // 确保目录存在
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let active_sessions = manager.get_active_sessions();
        let stored_sessions: Vec<StoredSession> = active_sessions
            .iter()
            .map(|s| StoredSession {
                transfer_id: s.transfer_id,
                filename: s.filename.clone(),
                filesize: s.filesize,
                local_path: s.local_path.clone(),
                state: s.state.clone(),
                created_at: s.created_at,
                updated_at: s.updated_at,
            })
            .collect();

        let json = serde_json::to_vec(&stored_sessions)?;

        let mut file = File::create(&self.file_path).await?;
        file.write_all(&json).await?;
        file.sync_all().await?;

        Ok(())
    }
}

/// 存储格式
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StoredSession {
    transfer_id: Uuid,
    filename: String,
    filesize: u64,
    local_path: PathBuf,
    state: SessionState,
    created_at: u64,
    updated_at: u64,
}

/// 为 SessionManager 添加 restore_session 方法
impl SessionManager {
    /// 恢复会话（从持久化存储加载）
    pub fn restore_session(&mut self, session: TransferSession) {
        self.sessions.insert(session.transfer_id, session);
    }
}
