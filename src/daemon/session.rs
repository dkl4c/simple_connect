use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

/// 传输会话状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionState {
    Pending,
    InProgress { offset: u64 },
    Paused { offset: u64 },
    Completed,
    Cancelled,
}

/// 传输会话
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferSession {
    pub transfer_id: Uuid,
    pub filename: String,
    pub filesize: u64,
    pub local_path: PathBuf,
    pub state: SessionState,
    pub created_at: u64,
    pub updated_at: u64,
}

/// 会话管理器
pub struct SessionManager {
    pub sessions: HashMap<Uuid, TransferSession>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// 创建新会话
    pub fn create_session(
        &mut self,
        transfer_id: Uuid,
        filename: String,
        filesize: u64,
        local_path: PathBuf,
    ) -> &TransferSession {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let session = TransferSession {
            transfer_id,
            filename,
            filesize,
            local_path,
            state: SessionState::Pending,
            created_at: now,
            updated_at: now,
        };

        self.sessions.insert(transfer_id, session);
        self.sessions.get(&transfer_id).unwrap()
    }

    /// 获取会话
    pub fn get_session(&self, transfer_id: &Uuid) -> Option<&TransferSession> {
        self.sessions.get(transfer_id)
    }

    /// 获取可变的会话
    pub fn get_session_mut(&mut self, transfer_id: &Uuid) -> Option<&mut TransferSession> {
        self.sessions.get_mut(transfer_id)
    }

    /// 更新会话偏移量
    pub fn update_offset(&mut self, transfer_id: &Uuid, offset: u64) {
        if let Some(session) = self.sessions.get_mut(transfer_id) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            session.state = SessionState::InProgress { offset };
            session.updated_at = now;
        }
    }

    /// 暂停会话
    pub fn pause_session(&mut self, transfer_id: &Uuid) {
        if let Some(session) = self.sessions.get_mut(transfer_id) {
            let offset = match &session.state {
                SessionState::InProgress { offset } => *offset,
                _ => return,
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            session.state = SessionState::Paused { offset };
            session.updated_at = now;
        }
    }

    /// 恢复会话
    pub fn resume_session(&mut self, transfer_id: &Uuid) {
        if let Some(session) = self.sessions.get_mut(transfer_id) {
            let offset = match &session.state {
                SessionState::Paused { offset } => *offset,
                _ => return,
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            session.state = SessionState::InProgress { offset };
            session.updated_at = now;
        }
    }

    /// 完成会话
    pub fn complete_session(&mut self, transfer_id: &Uuid) {
        if let Some(session) = self.sessions.get_mut(transfer_id) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            session.state = SessionState::Completed;
            session.updated_at = now;
        }
    }

    /// 取消会话
    pub fn cancel_session(&mut self, transfer_id: &Uuid) {
        if let Some(session) = self.sessions.get_mut(transfer_id) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            session.state = SessionState::Cancelled;
            session.updated_at = now;
        }
    }

    /// 获取所有进行中的会话
    pub fn get_active_sessions(&self) -> Vec<&TransferSession> {
        self.sessions
            .values()
            .filter(|s| {
                matches!(
                    s.state,
                    SessionState::Pending | SessionState::InProgress { .. } | SessionState::Paused { .. }
                )
            })
            .collect()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}
