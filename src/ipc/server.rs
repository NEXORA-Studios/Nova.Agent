use crate::ipc::protocol::{AgentRequest, AgentResponse, InstanceInfo, ResponseData};
use crate::state::SharedState;
use anyhow::Result;
use interprocess::local_socket::tokio::prelude::*;
use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, ToNsName};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// IPC socket 名称
const SOCKET_NAME: &str = "nova-agent";

/// 启动 IPC Server
pub async fn start(state: SharedState) -> Result<()> {
    let name = SOCKET_NAME.to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new().name(name).create_tokio()?;

    tracing::info!("IPC server listening on {}", SOCKET_NAME);

    loop {
        match listener.accept().await {
            Ok(stream) => {
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, state).await {
                        tracing::warn!("IPC connection error: {}", e);
                    }
                });
            }
            Err(e) => {
                tracing::error!("IPC accept error: {}", e);
            }
        }
    }
}

/// 处理单个 IPC 连接
async fn handle_connection(stream: Stream, state: SharedState) -> Result<()> {
    let (read_half, mut write_half) = stream.split();
    let reader = BufReader::new(read_half);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let request: AgentRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                let resp = AgentResponse::Error {
                    message: format!("invalid request: {}", e),
                };
                let data = serde_json::to_string(&resp)? + "\n";
                write_half.write_all(data.as_bytes()).await?;
                continue;
            }
        };

        match request {
            AgentRequest::CreateInstance(config) => {
                match state.instances.create(config).await {
                    Ok(id) => {
                        send_response(
                            &mut write_half,
                            AgentResponse::OkData(ResponseData::InstanceId(id)),
                        )
                        .await?;
                    }
                    Err(e) => {
                        send_response(
                            &mut write_half,
                            AgentResponse::Error {
                                message: e.to_string(),
                            },
                        )
                        .await?;
                    }
                }
            }
            AgentRequest::StopInstance { id } => {
                match state.instances.stop(&id).await {
                    Ok(()) => send_response(&mut write_half, AgentResponse::Ok).await?,
                    Err(e) => {
                        send_response(
                            &mut write_half,
                            AgentResponse::Error {
                                message: e.to_string(),
                            },
                        )
                        .await?
                    }
                }
            }
            AgentRequest::KillInstance { id } => {
                let instance = state.instances.lookup(&id).await;
                match instance {
                    Some(inst) => match inst.kill().await {
                        Ok(()) => send_response(&mut write_half, AgentResponse::Ok).await?,
                        Err(e) => {
                            send_response(
                                &mut write_half,
                                AgentResponse::Error {
                                    message: e.to_string(),
                                },
                            )
                            .await?
                        }
                    },
                    None => {
                        send_response(
                            &mut write_half,
                            AgentResponse::Error {
                                message: format!("instance {} not found", id),
                            },
                        )
                        .await?
                    }
                }
            }
            AgentRequest::RemoveInstance { id } => {
                match state.instances.remove(&id).await {
                    Ok(()) => send_response(&mut write_half, AgentResponse::Ok).await?,
                    Err(e) => {
                        send_response(
                            &mut write_half,
                            AgentResponse::Error {
                                message: e.to_string(),
                            },
                        )
                        .await?
                    }
                }
            }
            AgentRequest::ListInstances => {
                let list = state.instances.enumerate().await;
                let infos: Vec<InstanceInfo> = list
                    .into_iter()
                    .map(|(id, state)| InstanceInfo { id, state })
                    .collect();
                send_response(
                    &mut write_half,
                    AgentResponse::OkData(ResponseData::InstanceList(infos)),
                )
                .await?;
            }
            AgentRequest::GetState { id } => {
                let instance = state.instances.lookup(&id).await;
                match instance {
                    Some(inst) => {
                        let s = *inst.state.read().await;
                        send_response(
                            &mut write_half,
                            AgentResponse::OkData(ResponseData::InstanceState(s)),
                        )
                        .await?;
                    }
                    None => {
                        send_response(
                            &mut write_half,
                            AgentResponse::Error {
                                message: format!("instance {} not found", id),
                            },
                        )
                        .await?;
                    }
                }
            }
            AgentRequest::SendConsole { id, line } => {
                // 关键：不要持有 InstanceManager 的锁
                let instance = state.instances.lookup(&id).await;
                match instance {
                    Some(inst) => match inst.send_console(&line).await {
                        Ok(()) => send_response(&mut write_half, AgentResponse::Ok).await?,
                        Err(e) => {
                            send_response(
                                &mut write_half,
                                AgentResponse::Error {
                                    message: e.to_string(),
                                },
                            )
                            .await?
                        }
                    },
                    None => {
                        send_response(
                            &mut write_half,
                            AgentResponse::Error {
                                message: format!("instance {} not found", id),
                            },
                        )
                        .await?
                    }
                }
            }
            AgentRequest::SubscribeConsole { id } => {
                let instance = state.instances.lookup(&id).await;
                match instance {
                    Some(inst) => {
                        // 先发 backlog
                        let backlog = inst.console.backlog();
                        send_response(
                            &mut write_half,
                            AgentResponse::OkData(ResponseData::Backlog(backlog)),
                        )
                        .await?;

                        // 持续 stream
                        let mut rx = inst.console.subscribe();
                        loop {
                            match rx.recv().await {
                                Ok(line) => {
                                    let resp =
                                        AgentResponse::OkData(ResponseData::ConsoleLine(line));
                                    let data = serde_json::to_string(&resp)? + "\n";
                                    if write_half.write_all(data.as_bytes()).await.is_err() {
                                        break; // 连接断开
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                    tracing::warn!("console subscriber lagged by {} lines", n);
                                }
                                Err(_) => break,
                            }
                        }
                    }
                    None => {
                        send_response(
                            &mut write_half,
                            AgentResponse::Error {
                                message: format!("instance {} not found", id),
                            },
                        )
                        .await?;
                    }
                }
            }
            AgentRequest::GetBacklog { id } => {
                let instance = state.instances.lookup(&id).await;
                match instance {
                    Some(inst) => {
                        let backlog = inst.console.backlog();
                        send_response(
                            &mut write_half,
                            AgentResponse::OkData(ResponseData::Backlog(backlog)),
                        )
                        .await?;
                    }
                    None => {
                        send_response(
                            &mut write_half,
                            AgentResponse::Error {
                                message: format!("instance {} not found", id),
                            },
                        )
                        .await?;
                    }
                }
            }
        }
    }

    Ok(())
}

async fn send_response<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    response: AgentResponse,
) -> Result<()> {
    let data = serde_json::to_string(&response)? + "\n";
    writer.write_all(data.as_bytes()).await?;
    Ok(())
}
