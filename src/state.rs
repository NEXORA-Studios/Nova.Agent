use crate::instance::InstanceManager;

/// 全局应用状态
pub struct AppState {
    pub instances: InstanceManager,
}

/// 共享引用类型
pub type SharedState = std::sync::Arc<AppState>;

impl AppState {
    pub fn new() -> Self {
        Self {
            instances: InstanceManager::new(),
        }
    }
}
