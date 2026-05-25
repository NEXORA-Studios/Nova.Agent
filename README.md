# Nova Agent

Minecraft 实例管理 Agent，职责单一：**可靠持有 Minecraft 进程**。

## 特性

- 进程生命周期管理（启动 / 停止 / 强制终止）
- stdin/stdout/stderr 全双工管道
- 控制台输出广播（backlog 5000 行 + 实时订阅）
- IPC API（Named Pipe / Unix Socket，JSON Lines 协议）
- 无 GUI，无状态，可被任意前端驱动

## 快速开始

```bash
# 构建
cargo build --release

# 运行
./target/release/nova-agent
```

Agent 启动后监听 IPC 端点：

| 平台          | 端点                            |
| ------------- | ------------------------------- |
| Windows       | `\\.\pipe\nova-agent`           |
| Linux / macOS | abstract namespace `nova-agent` |

## 架构

```
┌─────────────┐    Named Pipe / Unix Socket    ┌─────────────┐
│  GUI / CLI  │ ◄──────── JSON Lines ────────► │  Nova Agent │
└─────────────┘                                └──────┬──────┘
                                                      │ stdin / stdout / stderr
                                               ┌──────▼──────┐
                                               │  Minecraft  │
                                               │  (Java 进程) │
                                               └─────────────┘
```

## 项目结构

```
src/
├── main.rs        # 入口：logging → state → ipc
├── console.rs     # ConsoleBus（backlog + broadcast）
├── instance.rs    # 状态机 + ManagedInstance + InstanceManager
├── ipc/
│   ├── mod.rs
│   ├── protocol.rs  # AgentRequest / AgentResponse 定义
│   └── server.rs    # IPC Server 实现
├── logging.rs     # 日志初始化
├── process.rs     # 预留扩展点
└── state.rs       # AppState + SharedState
```

## 实例状态机

```
Starting → Running → Stopping → Stopped
                │                  ↑
                │         StoppingTimedOut
                │                  │
                └── KillInstance ──┘
                                   │
              Crashed ←────────────┘
```

| 状态               | 说明                                 |
| ------------------ | ------------------------------------ |
| `Starting`         | 进程已启动，等待首次输出             |
| `Running`          | 正常运行                             |
| `Stopping`         | 已发 stop 命令，等待退出（60s 超时） |
| `StoppingTimedOut` | 超时未退出，等待用户决定是否 kill    |
| `Stopped`          | 正常退出                             |
| `Crashed`          | 非正常退出                           |

## IPC 接入

完整接入文档见 [docs/IPC_INTEGRATION.md](docs/IPC_INTEGRATION.md)。

快速示例（创建实例）：

```json
→ {"CreateInstance":{"java_path":"java","jvm_args":["-Xmx2G"],"jar_path":"server.jar","program_args":["nogui"],"working_dir":null,"id":null}}
← {"OkData":{"InstanceId":"a1b2c3d4-..."}}
```

## 日志

| 文件                  | 说明                       |
| --------------------- | -------------------------- |
| `logs/nova-agent.log` | Agent 自身日志（按天滚动） |

Agent 不持久化实例控制台输出，Minecraft 自身会保存日志。

## 构建

```bash
cargo build --release
```

## License

Copyright (c) 2026 NEXORA Studios
OpenSource under GNU Affero General Public License v3.0
