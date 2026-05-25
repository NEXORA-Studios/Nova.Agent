# Nova Agent IPC 接入文档

## 概述

Nova Agent 是一个无 GUI 的后台进程，负责持有 Minecraft Java 进程。外部程序（Launcher GUI、CLI 工具等）通过 Named Pipe / Unix Socket 与 Agent 通信。

## 1. 启动 Agent

```bash
cargo run --release
```

Agent 启动后监听 IPC 端点，日志输出到 `logs/` 目录。

## 2. 连接方式

| 平台 | 端点地址 |
|------|----------|
| Windows | `\\.\pipe\nova-agent`（Named Pipe） |
| Linux | abstract namespace `nova-agent`（Unix Socket） |

## 3. 协议格式

**JSON Lines**：每条消息是一行 JSON，以 `\n` 结尾。

- 客户端发送 `AgentRequest`
- 服务端回复 `AgentResponse`

## 4. 数据类型

### InstanceState

```json
"Starting" | "Running" | "Stopping" | "StoppingTimedOut" | "Stopped" | "Crashed"
```

| 状态 | 说明 |
|------|------|
| `Starting` | 进程已启动，等待首次输出 |
| `Running` | 正常运行中 |
| `Stopping` | 已发送 stop 命令，等待进程退出 |
| `StoppingTimedOut` | stop 后 60s 未退出，等待用户决定是否 kill |
| `Stopped` | 正常退出 |
| `Crashed` | 非正常退出 |

### ConsoleStream

```json
"Stdout" | "Stderr" | "System"
```

### ConsoleLine

```json
{
  "timestamp": 1700000000000,
  "stream": "Stdout",
  "line": "Done (3.521s)! For help, type \"help\""
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `timestamp` | `u64` | Unix 时间戳（毫秒） |
| `stream` | `ConsoleStream` | 输出流来源 |
| `line` | `string` | 一行内容 |

### InstanceConfig

```json
{
  "id": null,
  "java_path": "java",
  "jvm_args": ["-Xmx2G", "-XX:+UseG1GC"],
  "jar_path": "server.jar",
  "program_args": ["nogui"],
  "working_dir": "C:/mc/servers/survival"
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `id` | `string?` | 否 | 不指定则自动生成 UUID |
| `java_path` | `string` | 是 | Java 可执行文件路径 |
| `jvm_args` | `string[]` | 是 | JVM 参数 |
| `jar_path` | `string` | 是 | Jar 文件路径 |
| `program_args` | `string[]` | 是 | 程序参数 |
| `working_dir` | `string?` | 否 | 工作目录 |

### InstanceInfo

```json
{
  "id": "a1b2c3d4-...",
  "state": "Running"
}
```

## 5. 请求 API

### CreateInstance

创建并启动一个 Minecraft 实例。

```json
{"CreateInstance":{"java_path":"java","jvm_args":["-Xmx2G"],"jar_path":"server.jar","program_args":["nogui"],"working_dir":null,"id":null}}
```

**响应**：

```json
{"OkData":{"InstanceId":"a1b2c3d4-..."}}
```

---

### StopInstance

优雅停止实例。发送 `stop` 命令，等待 60s。超时后状态变为 `StoppingTimedOut`，需用户决定是否 KillInstance。

```json
{"StopInstance":{"id":"a1b2c3d4-..."}}
```

**响应**：

```json
{"Ok"}
```

或超时后：

```json
{"Ok"}
```

此时通过 `GetState` 查询会得到 `StoppingTimedOut`。

---

### KillInstance

强制终止实例。可从 `Running`、`Stopping`、`StoppingTimedOut` 状态调用。

```json
{"KillInstance":{"id":"a1b2c3d4-..."}}
```

**响应**：

```json
{"Ok"}
```

---

### RemoveInstance

移除已停止的实例（仅 `Stopped` / `Crashed` 状态可移除）。

```json
{"RemoveInstance":{"id":"a1b2c3d4-..."}}
```

---

### ListInstances

列出所有实例。

```json
"ListInstances"
```

**响应**：

```json
{"OkData":{"InstanceList":[{"id":"a1b2c3d4-...","state":"Running"}]}}
```

---

### GetState

获取单个实例状态。

```json
{"GetState":{"id":"a1b2c3d4-..."}}
```

**响应**：

```json
{"OkData":{"InstanceState":"Running"}}
```

---

### SendConsole

向实例 stdin 发送一行命令。

```json
{"SendConsole":{"id":"a1b2c3d4-...","line":"say hello"}}
```

---

### GetBacklog

获取实例控制台历史输出（最近 5000 行）。

```json
{"GetBacklog":{"id":"a1b2c3d4-..."}}
```

**响应**：

```json
{"OkData":{"Backlog":[{"timestamp":1700000000000,"stream":"Stdout","line":"Done!"}]}}
```

---

### SubscribeConsole

订阅控制台实时输出。连接会保持打开，持续推送。

1. 先一次性发送完整 backlog
2. 然后持续推送新行

```json
{"SubscribeConsole":{"id":"a1b2c3d4-..."}}
```

**响应序列**：

```json
{"OkData":{"Backlog":[...]}}
{"OkData":{"ConsoleLine":{"timestamp":1700000000001,"stream":"Stdout","line":"Player joined"}}}
{"OkData":{"ConsoleLine":{"timestamp":1700000000002,"stream":"Stderr","line":"Warning: ..."}}}
...（持续推送直到连接断开）
```

## 6. 响应格式

```json
// 成功，无数据
"Ok"

// 成功，带数据
{"OkData":{...}}

// 失败
{"Error":{"message":"instance abc not found"}}
```

## 7. 典型集成流程

### 启动并监控实例

```
1. 连接到 \\.\pipe\nova-agent
2. 发送 CreateInstance → 获得 instance id
3. 发送 SubscribeConsole → 接收 backlog + 实时输出
4. 在 GUI 中渲染控制台
5. 用户输入命令 → 发送 SendConsole
6. 用户点停止 → 发送 StopInstance
7. 如果 60s 后状态变为 StoppingTimedOut → 弹窗询问用户是否 KillInstance
```

### 重新连接已有实例

```
1. 连接到 \\.\pipe\nova-agent
2. 发送 ListInstances → 获取所有实例
3. 对目标实例发送 SubscribeConsole → 恢复控制台输出
```

## 8. 各语言连接示例

### Rust（推荐，使用 interprocess）

```rust
use interprocess::local_socket::tokio::prelude::*;
use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::{GenericNamespaced, ToNsName};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn connect() -> anyhow::Result<()> {
    let name = "nova-agent".to_ns_name::<GenericNamespaced>()?;
    let conn = Stream::connect(name).await?;
    let (read, mut write) = conn.split();
    let mut reader = BufReader::new(read);

    // 列出实例
    let req = "\"ListInstances\"\n";
    write.write_all(req.as_bytes()).await?;

    let mut line = String::new();
    reader.read_line(&mut line).await?;
    println!("{}", line);
    Ok(())
}
```

### Node.js

```js
import { connect } from 'net';

const sock = connect('\\\\.\\pipe\\nova-agent', () => {
  sock.write('"ListInstances"\n');
});

let buf = '';
sock.on('data', (data) => {
  buf += data.toString();
  const lines = buf.split('\n');
  buf = lines.pop();
  for (const line of lines) {
    console.log(JSON.parse(line));
  }
});
```

### Python

```python
import win32pipe, win32file, json

handle = win32file.CreateFile(
    r'\\.\pipe\nova-agent',
    win32file.GENERIC_READ | win32file.GENERIC_WRITE,
    0, None, win32file.OPEN_EXISTING, 0, None
)

def send_request(req):
    data = json.dumps(req) + "\n"
    win32file.WriteFile(handle, data.encode())
    _, resp = win32file.ReadFile(handle, 65536)
    return json.loads(resp)

# 列出实例
print(send_request("ListInstances"))

# 创建实例
print(send_request({
    "CreateInstance": {
        "java_path": "java",
        "jvm_args": ["-Xmx2G"],
        "jar_path": "server.jar",
        "program_args": ["nogui"],
        "working_dir": None,
        "id": None
    }
}))
```

### PowerShell（调试用）

```powershell
$pipe = New-Object System.IO.Pipes.NamedPipeClientStream(".", "nova-agent", [System.IO.Pipes.PipeDirection]::InOut)
$pipe.Connect()
$writer = New-Object System.IO.StreamWriter($pipe)
$reader = New-Object System.IO.StreamReader($pipe)
$writer.AutoFlush = $true

$writer.WriteLine('"ListInstances"')
$reader.ReadLine()
```

## 9. 错误处理

所有错误统一返回：

```json
{"Error":{"message":"描述信息"}}
```

常见错误：

| 场景 | message 示例 |
|------|-------------|
| 实例不存在 | `instance xxx not found` |
| 实例未运行 | `instance xxx is not running (state=Stopped)` |
| 无法 kill | `instance xxx cannot be killed in state Stopped` |
| 实例仍在运行 | `instance xxx is still running (state=Running)` |
| 请求格式错误 | `invalid request: ...` |

## 10. 日志文件

| 文件 | 说明 |
|------|------|
| `logs/nova-agent.log` | Agent 自身日志（按天滚动） |

Agent 不持久化实例控制台输出，Minecraft 自身会保存日志。
