use common::statemanager::{ErrorCode, ModelState, PackageState, ResourceType};
use std::collections::HashMap;
use tokio::time::Instant;
// ========================================
// CORE DATA STRUCTURES
// ========================================

/// Action execution command for async processing
#[derive(Debug, Clone)]
pub struct ActionCommand {
    pub action: String,
    pub resource_key: String,
    pub resource_type: ResourceType,
    pub transition_id: String,
    pub context: HashMap<String, String>,
}

/// Represents a state transition in the state machine
#[derive(Debug, Clone, PartialEq)]
pub struct StateTransition {
    pub from_state: i32,
    pub event: String,
    pub to_state: i32,
    pub condition: Option<String>,
    pub action: String,
}

/// Health status tracking for resources
#[derive(Debug, Clone)]
pub struct HealthStatus {
    pub healthy: bool,
    pub status_message: String,
    pub last_check: Instant,
    pub consecutive_failures: u32,
}

/// Represents the current state of a resource with metadata
#[derive(Debug, Clone)]
pub struct ResourceState {
    pub resource_type: ResourceType,
    pub resource_name: String,
    pub current_state: i32,
    pub desired_state: Option<i32>,
    pub last_transition_time: Instant,
    pub transition_count: u64,
    pub metadata: HashMap<String, String>,
    pub health_status: HealthStatus,
}

/// Result of a state transition attempt - aligned with proto StateChangeResponse
#[derive(Debug, Clone)]
pub struct TransitionResult {
    pub new_state: i32,
    pub error_code: ErrorCode,
    pub message: String,
    pub actions_to_execute: Vec<String>,
    pub transition_id: String,
    pub error_details: String,
}

/// Container state information from NodeAgent
#[derive(Debug, Clone)]
pub struct ContainerStateInfo {
    pub container_id: String,
    pub container_names: Vec<String>,
    pub image: String,
    pub state: HashMap<String, String>,
    pub config: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
}

/// Model information with associated containers
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub model_name: String,
    pub package_name: String,
    pub containers: Vec<ContainerStateInfo>,
    pub current_state: ModelState,
    pub last_update: Instant,
}

/// Package information with associated models
#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub package_name: String,
    pub models: HashMap<String, ModelInfo>,
    pub current_state: PackageState,
    pub last_update: Instant,
}
