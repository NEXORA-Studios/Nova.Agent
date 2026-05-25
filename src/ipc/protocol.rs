use serde::{Deserialize, Serialize};

use crate::console::ConsoleLine;
use crate::instance::{InstanceConfig, InstanceState};

/// IPC 请求
#[derive(Debug, Serialize, Deserialize)]
pub enum AgentRequest {
    /// 创建并启动实例
    CreateInstance(InstanceConfig),
    /// 停止实例
    StopInstance { id: String },
    /// 强制 kill 实例
    KillInstance { id: String },
    /// 移除已停止的实例
    RemoveInstance { id: String },
    /// 列出所有实例
    ListInstances,
    /// 获取实例状态
    GetState { id: String },
    /// 发送控制台命令
    SendConsole { id: String, line: String },
    /// 订阅控制台输出（先发 backlog，再持续 stream）
    SubscribeConsole { id: String },
    /// 获取 backlog
    GetBacklog { id: String },
}

/// IPC 响应/事件
#[derive(Debug, Serialize, Deserialize)]
pub enum AgentResponse {
    /// 操作成功
    Ok,
    /// 操作成功，带数据
    OkData(ResponseData),
    /// 操作失败
    Error { message: String },
}

/// 响应数据
#[derive(Debug, Serialize, Deserialize)]
pub enum ResponseData {
    /// 实例 ID
    InstanceId(String),
    /// 实例列表
    InstanceList(Vec<InstanceInfo>),
    /// 实例状态
    InstanceState(InstanceState),
    /// Backlog
    Backlog(Vec<ConsoleLine>),
    /// 实时控制台行
    ConsoleLine(ConsoleLine),
}

/// 实例信息
#[derive(Debug, Serialize, Deserialize)]
pub struct InstanceInfo {
    pub id: String,
    pub state: InstanceState,
}
