use crate::console::{ConsoleBus, ConsoleLine, ConsoleStream};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, RwLock};
use tokio::time::{timeout, Duration};

/// 实例状态机
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstanceState {
    Starting,
    Running,
    Stopping,
    StoppingTimedOut,
    Stopped,
    Crashed,
}

/// 实例启动配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceConfig {
    /// 实例 ID（不指定则自动生成）
    #[serde(default)]
    pub id: Option<String>,
    /// Java 可执行文件路径
    pub java_path: String,
    /// JVM 参数
    pub jvm_args: Vec<String>,
    /// Jar 文件路径
    pub jar_path: String,
    /// 程序参数
    pub program_args: Vec<String>,
    /// 工作目录
    pub working_dir: Option<PathBuf>,
}

/// 持有 Java 进程的受管实例
pub struct ManagedInstance {
    pub id: String,
    pub state: Arc<RwLock<InstanceState>>,
    pub child: Arc<Mutex<Option<Child>>>,
    pub stdin: Arc<Mutex<Option<ChildStdin>>>,
    pub console: Arc<ConsoleBus>,
}

impl ManagedInstance {
    /// 向实例 stdin 发送命令
    pub async fn send_console(&self, line: &str) -> Result<()> {
        let mut stdin_guard = self.stdin.lock().await;
        if let Some(ref mut stdin) = *stdin_guard {
            stdin.write_all(format!("{line}\n").as_bytes()).await?;
            stdin.flush().await?;
            Ok(())
        } else {
            bail!("instance {} has no stdin (not running)", self.id)
        }
    }

    /// 优雅停止实例：先发 "stop"，等待 60s
    /// 超时后进入 StoppingTimedOut 状态，由用户决定是否强制 kill
    pub async fn stop(&self) -> Result<()> {
        {
            let mut state = self.state.write().await;
            if *state != InstanceState::Running {
                bail!("instance {} is not running (state={:?})", self.id, *state);
            }
            *state = InstanceState::Stopping;
        }

        // 尝试发送 stop 命令
        let _ = self.send_console("stop").await;

        // 等待 60s
        let wait_result = {
            let mut child_guard = self.child.lock().await;
            if let Some(ref mut child) = *child_guard {
                timeout(Duration::from_secs(60), child.wait()).await
            } else {
                bail!("no child process")
            }
        };

        match wait_result {
            Ok(Ok(status)) => {
                tracing::info!("instance {} exited with status: {}", self.id, status);
                *self.state.write().await = if status.success() {
                    InstanceState::Stopped
                } else {
                    InstanceState::Crashed
                };
                *self.child.lock().await = None;
                *self.stdin.lock().await = None;
                Ok(())
            }
            _ => {
                // 超时：不自动 kill，进入 StoppingTimedOut 等待用户决定
                tracing::warn!("instance {} did not stop within 60s, waiting for user decision", self.id);
                *self.state.write().await = InstanceState::StoppingTimedOut;
                self.console
                    .push(ConsoleLine {
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        stream: ConsoleStream::System,
                        line: format!(
                            "instance {} has not stopped within 60 seconds, waiting for user decision to force kill",
                            self.id
                        ),
                    })
                    .await;
                Ok(())
            }
        }
    }

    /// 强制 kill 实例（可从 StoppingTimedOut 状态调用）
    pub async fn kill(&self) -> Result<()> {
        let current_state = *self.state.read().await;
        if current_state != InstanceState::Running
            && current_state != InstanceState::Stopping
            && current_state != InstanceState::StoppingTimedOut
        {
            bail!(
                "instance {} cannot be killed in state {:?}",
                self.id,
                current_state
            );
        }
        let mut child_guard = self.child.lock().await;
        if let Some(ref mut child) = *child_guard {
            child.kill().await?;
            tracing::info!("instance {} killed", self.id);
        }
        *self.state.write().await = InstanceState::Stopped;
        *self.child.lock().await = None;
        *self.stdin.lock().await = None;
        Ok(())
    }
}

/// 实例管理器
pub struct InstanceManager {
    inner: RwLock<std::collections::HashMap<String, Arc<ManagedInstance>>>,
}

impl InstanceManager {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// 创建并启动实例
    pub async fn create(&self, config: InstanceConfig) -> Result<String> {
        let id = config
            .id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let mut cmd = Command::new(&config.java_path);
        cmd.args(&config.jvm_args)
            .arg("-jar")
            .arg(&config.jar_path)
            .args(&config.program_args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        if let Some(ref dir) = config.working_dir {
            cmd.current_dir(dir);
        }

        #[cfg(target_os = "windows")]
        {
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let mut child = cmd.spawn()?;

        // 立即取出 stdout/stderr
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let stdin_handle = child.stdin.take().unwrap();

        let console = Arc::new(ConsoleBus::new());
        let state = Arc::new(RwLock::new(InstanceState::Starting));
        let child_arc = Arc::new(Mutex::new(Some(child)));
        let stdin_arc = Arc::new(Mutex::new(Some(stdin_handle)));

        let instance = Arc::new(ManagedInstance {
            id: id.clone(),
            state: state.clone(),
            child: child_arc.clone(),
            stdin: stdin_arc.clone(),
            console: console.clone(),
        });

        // 启动 stdout reader task
        let id_clone = id.clone();
        let console_stdout = console.clone();
        let state_stdout = state.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // 检测 Minecraft 启动完成标志：Done (<任意内容>)! For help, type "help"
                if *state_stdout.read().await == InstanceState::Starting {
                    if line.contains("Done (") && line.contains(")! For help, type \"help\"") {
                        *state_stdout.write().await = InstanceState::Running;
                        tracing::info!("instance {} started: {}", id_clone, line);
                    }
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                console_stdout
                    .push(ConsoleLine {
                        timestamp: now,
                        stream: ConsoleStream::Stdout,
                        line,
                    })
                    .await;
            }
            tracing::info!("instance {} stdout EOF", id_clone);
        });

        // 启动 stderr reader task
        let id_clone = id.clone();
        let console_stderr = console.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                console_stderr
                    .push(ConsoleLine {
                        timestamp: now,
                        stream: ConsoleStream::Stderr,
                        line,
                    })
                    .await;
            }
            tracing::info!("instance {} stderr EOF", id_clone);
        });

        // 启动退出检测 task
        let id_clone = id.clone();
        let console_exit = console.clone();
        let state_exit = state.clone();
        let child_exit = child_arc.clone();
        let stdin_exit = stdin_arc.clone();
        tokio::spawn(async move {
            let result = {
                let mut guard = child_exit.lock().await;
                if let Some(ref mut child) = *guard {
                    child.wait().await
                } else {
                    return;
                }
            };

            match result {
                Ok(status) => {
                    let new_state = if status.success() {
                        InstanceState::Stopped
                    } else {
                        InstanceState::Crashed
                    };
                    *state_exit.write().await = new_state;

                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    console_exit
                        .push(ConsoleLine {
                            timestamp: now,
                            stream: ConsoleStream::System,
                            line: format!(
                                "instance {} exited with code: {}",
                                id_clone,
                                status.code().unwrap_or(-1)
                            ),
                        })
                        .await;

                    *child_exit.lock().await = None;
                    *stdin_exit.lock().await = None;
                }
                Err(e) => {
                    tracing::error!("instance {} wait error: {}", id_clone, e);
                    *state_exit.write().await = InstanceState::Crashed;
                }
            }
        });

        // 注册到管理器
        self.inner.write().await.insert(id.clone(), instance);

        Ok(id)
    }

    /// 查找实例
    pub async fn lookup(&self, id: &str) -> Option<Arc<ManagedInstance>> {
        self.inner.read().await.get(id).cloned()
    }

    /// 停止实例
    pub async fn stop(&self, id: &str) -> Result<()> {
        let instance = { self.inner.read().await.get(id).cloned() };
        match instance {
            Some(inst) => inst.stop().await,
            None => bail!("instance {} not found", id),
        }
    }

    /// 移除已停止的实例
    pub async fn remove(&self, id: &str) -> Result<()> {
        let instance = { self.inner.read().await.get(id).cloned() };
        match instance {
            Some(inst) => {
                let state = *inst.state.read().await;
                if state != InstanceState::Stopped && state != InstanceState::Crashed {
                    bail!("instance {} is still running (state={:?})", id, state);
                }
                self.inner.write().await.remove(id);
                Ok(())
            }
            None => bail!("instance {} not found", id),
        }
    }

    /// 列出所有实例
    pub async fn enumerate(&self) -> Vec<(String, InstanceState)> {
        let inner = self.inner.read().await;
        let mut result = Vec::new();
        for (id, inst) in inner.iter() {
            let state = *inst.state.read().await;
            result.push((id.clone(), state));
        }
        result
    }

    /// 关闭所有实例（优雅 stop，超时后 kill）
    pub async fn shutdown_all(&self) {
        let instances: Vec<Arc<ManagedInstance>> = {
            let inner = self.inner.read().await;
            inner.values().cloned().collect()
        };

        if instances.is_empty() {
            return;
        }

        tracing::info!("shutting down {} instance(s)...", instances.len());

        // 先对所有实例发送 stop
        for inst in &instances {
            let state = *inst.state.read().await;
            if state == InstanceState::Running || state == InstanceState::Starting {
                *inst.state.write().await = InstanceState::Stopping;
                let _ = inst.send_console("stop").await;
            }
        }

        // 等待最多 60s
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let all_stopped = {
                let inner = self.inner.read().await;
                let mut stopped = true;
                for inst in inner.values() {
                    let s = *inst.state.read().await;
                    if s != InstanceState::Stopped && s != InstanceState::Crashed {
                        stopped = false;
                        break;
                    }
                }
                stopped
            };
            if all_stopped || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // 超时后 kill 剩余
        for inst in &instances {
            let state = *inst.state.read().await;
            if state != InstanceState::Stopped && state != InstanceState::Crashed {
                tracing::warn!("force killing instance {} on shutdown", inst.id);
                let _ = inst.kill().await;
            }
        }
    }
}
