use anyhow::Result;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::{interval, Instant};

use super::protocol::PacketType;
use super::Connection;

/// 心跳状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeartbeatState {
    Idle,
    Waiting,
    Connected,
    Disconnected,
}

/// 心跳管理器
pub struct HeartbeatManager {
    interval_secs: u64,
    timeout_secs: u64,
    last_heartbeat: Option<Instant>,
    state: HeartbeatState,
}

impl HeartbeatManager {
    pub fn new(interval_secs: u64, timeout_secs: u64) -> Self {
        Self {
            interval_secs,
            timeout_secs,
            last_heartbeat: None,
            state: HeartbeatState::Idle,
        }
    }

    pub fn state(&self) -> HeartbeatState {
        self.state
    }

    pub fn set_connected(&mut self) {
        self.state = HeartbeatState::Connected;
        self.last_heartbeat = Some(Instant::now());
    }

    pub fn set_disconnected(&mut self) {
        self.state = HeartbeatState::Disconnected;
        self.last_heartbeat = None;
    }

    /// 检查是否超时
    pub fn is_timeout(&self) -> bool {
        if let Some(last) = self.last_heartbeat {
            last.elapsed() > Duration::from_secs(self.timeout_secs)
        } else {
            false
        }
    }

    /// 收到心跳响应时调用
    pub fn receive_ack(&mut self) {
        self.last_heartbeat = Some(Instant::now());
        self.state = HeartbeatState::Connected;
    }

    /// 创建心跳任务，返回停止信号
    pub async fn start_heartbeat_loop(
        &mut self,
        connection: &mut Connection,
        mut stop: oneshot::Receiver<()>,
    ) -> Result<()> {
        let mut ticker = interval(Duration::from_secs(self.interval_secs));

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if self.state == HeartbeatState::Disconnected {
                        break;
                    }

                    // 发送心跳
                    connection.send_packet(PacketType::Heartbeat, &[]).await?;
                    self.state = HeartbeatState::Waiting;

                    // 检查超时
                    if self.is_timeout() {
                        self.state = HeartbeatState::Disconnected;
                        break;
                    }
                }
                _ = &mut stop => {
                    break;
                }
            }
        }

        Ok(())
    }

    /// 处理心跳响应
    pub fn handle_heartbeat_ack(&mut self) {
        self.receive_ack();
    }
}
