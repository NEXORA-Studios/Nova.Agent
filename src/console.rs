use serde::{Deserialize, Serialize};

/// 控制台输出流类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsoleStream {
    Stdout,
    Stderr,
    System,
}

/// 控制台单行内容
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleLine {
    pub timestamp: u64,
    pub stream: ConsoleStream,
    pub line: String,
}

/// 控制台广播总线
///
/// backlog 保留最近 5000 行，同时通过 broadcast channel 实时推送。
pub struct ConsoleBus {
    backlog: parking_lot::Mutex<std::collections::VecDeque<ConsoleLine>>,
    tx: tokio::sync::broadcast::Sender<ConsoleLine>,
}

const BACKLOG_CAPACITY: usize = 5000;

impl ConsoleBus {
    pub fn new() -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        Self {
            backlog: parking_lot::Mutex::new(std::collections::VecDeque::new()),
            tx,
        }
    }

    /// 推送一行控制台输出
    ///
    /// - 写入 backlog（超出容量则淘汰最旧行）
    /// - broadcast 给所有订阅者
    pub async fn push(&self, line: ConsoleLine) {
        // backlog
        {
            let mut backlog = self.backlog.lock();
            if backlog.len() >= BACKLOG_CAPACITY {
                backlog.pop_front();
            }
            backlog.push_back(line.clone());
        }

        // broadcast（忽略无接收者的错误）
        let _ = self.tx.send(line);
    }

    /// 订阅实时控制台输出
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ConsoleLine> {
        self.tx.subscribe()
    }

    /// 获取 backlog 快照
    pub fn backlog(&self) -> Vec<ConsoleLine> {
        self.backlog.lock().iter().cloned().collect()
    }
}
