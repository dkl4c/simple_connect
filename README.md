# Simple Connect

一款用于 Windows 和 Linux 之间文件传输的工具，支持断点续传、分块传输和双向传输。

## 架构

```
┌─────────────────────────────────────────────────────────┐
│  CLI (命令行前端)                                         │
│  - send/recv/list/pause/resume/cancel/status            │
│  - 通过 IPC (TCP localhost) 与 Daemon 通信               │
└─────────────────────────────────────────────────────────┘
                           │ IPC (127.0.0.1:18889)
┌─────────────────────────────────────────────────────────┐
│  Daemon (守护进程)                                        │
│  - 连接管理 (Windows=监听, Linux=主动连接)                 │
│  - 文件传输引擎 (分块发送/接收)                           │
│  - 断点续传 (Transfer ID 追踪)                          │
│  - 心跳保活 (keepalive)                                  │
└─────────────────────────────────────────────────────────┘
                           │ TCP
              ┌────────────┴────────────┐
              │                         │
          Windows Server            Linux Host
          (监听模式 0.0.0.0:18888)  (主动连接)
```

## 特性

- **TCP 直连** - 无需中转服务器
- **断点续传** - 基于 Transfer ID + 偏移量，重启后可恢复
- **分块传输** - 64KB 块大小，4 并发窗口
- **双向传输** - 双方都可以主动发起文件传输
- **心跳保活** - 30 秒间隔，60 秒超时检测
- **CLI 界面** - 简洁的命令行操作

## 安装

### 从源码编译

```bash
# 安装 Rust (如果没有)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 克隆并编译
git clone <repo-url>
cd simple_connect
cargo build --release

# 二进制文件位于 target/release/simple_connect
```

## 使用方法

### Windows 端（服务器，监听模式）

```bash
# 启动 daemon
simple_connect daemon --listen 0.0.0.0:18888 --storage ./downloads

# 或使用 cargo 运行
cargo run -- daemon --listen 0.0.0.0:18888 --storage ./downloads
```

### Linux 端（客户端，连接模式）

```bash
# 启动 daemon
simple_connect daemon --connect <Windows公网IP>:18888 --storage ./downloads

# 或使用 cargo 运行
cargo run -- daemon --connect 192.168.1.100:18888 --storage ./downloads
```

### 发送文件

```bash
# 发送文件（自动使用 daemon 配置的远程地址）
simple_connect send /path/to/file.txt

# 指定远程地址发送
REMOTE_ADDR=192.168.1.100:18888 simple_connect send /path/to/file.txt

# 指定远程保存路径
simple_connect send /path/to/file.txt --remote new_name.txt
```

### 接收文件

接收文件需要在 Linux 端执行 send 命令，文件会从 Linux 发送到 Windows。

### 其他命令

```bash
# 查看传输状态
simple_connect status

# 列出活动传输
simple_connect list

# 暂停传输
simple_connect pause --id <uuid>

# 恢复传输
simple_connect resume --id <uuid>

# 取消传输
simple_connect cancel --id <uuid>
```

## 配置文件

配置文件示例位于 `daemon.conf.example`：

```toml
[daemon]
# listen_addr = "0.0.0.0:18888"    # Windows 模式
# connect_addr = "1.2.3.4:18888"    # Linux 模式
storage_dir = "./downloads"
chunk_size = 65536
heartbeat_interval = 30
timeout = 60
```

## 协议设计

### 包格式 (Big Endian)

```
┌─────────────────────────────────────────────────┐
│  4 bytes: 包类型 (PacketType)                     │
│  4 bytes: 载荷长度                               │
│  N bytes: 载荷 (JSON)                            │
└─────────────────────────────────────────────────┘
```

### 包类型

| 类型 | 值 | 说明 |
|------|-----|------|
| Handshake | 0x01 | 握手请求 |
| HandshakeAck | 0x02 | 握手响应 |
| TransferRequest | 0x10 | 传输请求 |
| TransferResponse | 0x11 | 传输响应 |
| Chunk | 0x12 | 数据块 |
| ChunkAck | 0x13 | 块确认 |
| Pause | 0x20 | 暂停传输 |
| Resume | 0x21 | 恢复传输 |
| Cancel | 0x22 | 取消传输 |
| Heartbeat | 0x30 | 心跳 |
| HeartbeatAck | 0x31 | 心跳响应 |
| Error | 0xFF | 错误 |

## 环境变量

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `REMOTE_ADDR` | 远程服务器地址（用于 send 命令） | `127.0.0.1:18888` |
| `RUST_LOG` | 日志级别 | `info` |

设置日志级别：

```bash
RUST_LOG=debug simple_connect send test.txt
RUST_LOG=trace cargo run -- daemon --listen 0.0.0.0:18888
```

## 注意事项

1. **防火墙** - 确保 Windows 端的 18888 端口对外开放
2. **网络** - Linux 端需要能访问 Windows 的公网 IP
3. **目录** - 确保 `storage_dir` 目录存在且有写权限
4. **断开重连** - 连接断开后需要重新建立，daemon 不会自动重连

## License

MIT
