use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default)]
pub struct RuntimeStatus {
    pub https_ready: bool,
    pub discovery_ready: bool,
    pub error: Option<String>,
}
pub type SharedRuntimeStatus = Arc<Mutex<RuntimeStatus>>;
