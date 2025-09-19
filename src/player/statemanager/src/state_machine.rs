/*
 * SPDX-FileCopyrightText: Copyright 2024 LG Electronics Inc.
 * SPDX-License-Identifier: Apache-2.0
 */

//! State Machine Implementation for PICCOLO Resource State Management
//!
//! This module implements the core state transition logic for Scenario, Package, and Model resources
//! according to the PICCOLO specification. It provides efficient data structures and algorithms
//! for managing state changes and enforcing the defined state transition tables.
//!
//! # Architecture Overview
//!
//! The state machine follows a table-driven approach where each resource type (Scenario, Package, Model)
//! has its own transition table defining valid state changes. The system supports:
//! - Conditional transitions based on resource state
//! - Action execution during state changes non-blocking
//! - Health monitoring and failure handling
//! - Backoff mechanisms for failed transitions
//!
//! # Usage Example
//!
//! ```rust
//! let mut state_machine = StateMachine::new();
//! let state_change = StateChange { /* ... */ };
//! let result = state_machine.process_state_change(state_change);
//! ```

use crate::types::{ActionCommand, HealthStatus, ResourceState, StateTransition, TransitionResult};
use common::statemanager::{
    ErrorCode, ModelState, PackageState, ResourceType, ScenarioState, StateChange,
};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};

// ========================================
// CONSTANTS AND CONFIGURATION
// ========================================

/// Default backoff duration for CrashLoopBackOff states
const BACKOFF_DURATION_SECS: u64 = 30;

/// Maximum consecutive failures before marking resource as unhealthy
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

impl TransitionResult {
    /// Check if the transition was successful
    pub fn is_success(&self) -> bool {
        matches!(self.error_code, ErrorCode::Success)
    }

    /// Check if the transition failed
    pub fn is_failure(&self) -> bool {
        !self.is_success()
    }

    /// Convert TransitionResult to StateChangeResponse for proto compatibility
    pub fn to_state_change_response(&self) -> common::statemanager::StateChangeResponse {
        common::statemanager::StateChangeResponse {
            message: self.message.clone(),
            transition_id: self.transition_id.clone(),
            timestamp_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as i64,
            error_code: self.error_code as i32,
            error_details: self.error_details.clone(),
        }
    }
}

/// Core state machine implementation for PICCOLO resource management
///
/// This is the central component that manages all resource state transitions,
/// enforces business rules, and maintains the current state of all resources
/// in the system.
///
/// # Design Principles
/// - **Deterministic**: Same inputs always produce same outputs
/// - **Auditable**: All state changes are tracked with timestamps
/// - **Resilient**: Handles failures gracefully with backoff mechanisms
/// - **Extensible**: New resource types can be added with their own transition tables
///
/// # Thread Safety
/// This implementation is not thread-safe. External synchronization is required
/// for concurrent access across multiple threads.
pub struct StateMachine {
    /// State transition tables indexed by resource type
    ///
    /// Each resource type has its own set of valid transitions, allowing
    /// for type-specific state management rules and behaviors.
    transition_tables: HashMap<ResourceType, Vec<StateTransition>>,

    /// Current state tracking for all managed resources
    ///
    /// Resources are keyed by a unique identifier (typically resource name)
    /// and contain complete state information including metadata and health status.
    resource_states: HashMap<String, ResourceState>,

    /// Backoff timers for CrashLoopBackOff and retry management
    ///
    /// Tracks when resources that have failed transitions can be retried,
    /// implementing exponential backoff to prevent resource thrashing.
    backoff_timers: HashMap<String, Instant>,

    /// Action command sender for async execution
    action_sender: Option<mpsc::UnboundedSender<ActionCommand>>,
}

impl StateMachine {
    /// Creates a new StateMachine with predefined transition tables
    ///
    /// Initializes the state machine with empty resource tracking and
    /// populates the transition tables for all supported resource types.
    ///
    /// # Returns
    /// A fully configured StateMachine ready to process state changes
    ///
    /// # Examples
    /// ```rust
    /// let state_machine = StateMachine::new();
    /// ```
    pub fn new() -> Self {
        let mut state_machine = StateMachine {
            transition_tables: HashMap::new(),
            resource_states: HashMap::new(),
            backoff_timers: HashMap::new(),
            action_sender: None,
        };

        // Initialize transition tables for each resource type
        state_machine.initialize_scenario_transitions();
        state_machine.initialize_package_transitions();
        state_machine.initialize_model_transitions();

        state_machine
    }

    /// Initialize async action executor
    pub fn initialize_action_executor(&mut self) -> mpsc::UnboundedReceiver<ActionCommand> {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.action_sender = Some(sender);
        receiver
    }

    // ========================================
    // STATE TRANSITION TABLE INITIALIZATION
    // ========================================

    /// Initialize the state transition table for Scenario resources
    ///
    /// Populates the transition table with all valid state changes for Scenario resources
    /// according to the PICCOLO specification. This includes transitions for:
    /// - Creation and initialization
    /// - Activation and deactivation
    /// - Error handling and recovery
    /// - Cleanup and termination
    ///
    /// # Implementation Note
    /// This method should define transitions like:
    /// - "Inactive" -> "Active" on "activate" event
    /// - "Active" -> "Inactive" on "deactivate" event
    /// - Any state -> "Failed" on "error" event
    fn initialize_scenario_transitions(&mut self) {
        let scenario_transitions = vec![
            StateTransition {
                from_state: ScenarioState::Idle as i32,
                event: "scenario_activation".to_string(),
                to_state: ScenarioState::Waiting as i32,
                condition: None,
                action: "start_condition_evaluation".to_string(),
            },
            StateTransition {
                from_state: ScenarioState::Waiting as i32,
                event: "condition_met".to_string(),
                to_state: ScenarioState::Allowed as i32,
                condition: None,
                action: "start_policy_verification".to_string(),
            },
            StateTransition {
                from_state: ScenarioState::Allowed as i32,
                event: "policy_verification_success".to_string(),
                to_state: ScenarioState::Playing as i32,
                condition: None,
                action: "execute_action_on_target_package".to_string(),
            },
            StateTransition {
                from_state: ScenarioState::Allowed as i32,
                event: "policy_verification_failure".to_string(),
                to_state: ScenarioState::Denied as i32,
                condition: None,
                action: "log_denial_generate_alert".to_string(),
            },
        ];
        self.transition_tables
            .insert(ResourceType::Scenario, scenario_transitions);
    }

    /// Initialize the state transition table for Package resources
    ///
    /// Configures all valid state transitions for Package resources, including:
    /// - Download and installation states
    /// - Verification and validation phases
    /// - Update and rollback mechanisms
    /// - Cleanup and removal operations
    ///
    /// # Implementation Note
    /// Package transitions typically involve more complex workflows due to
    /// dependency management and rollback requirements.
    fn initialize_package_transitions(&mut self) {
        let package_transitions = vec![
            StateTransition {
                from_state: PackageState::Unspecified as i32,
                event: "launch_request".to_string(),
                to_state: PackageState::Initializing as i32,
                condition: None,
                action: "start_model_creation_allocate_resources".to_string(),
            },
            StateTransition {
                from_state: PackageState::Initializing as i32,
                event: "initialization_complete".to_string(),
                to_state: PackageState::Running as i32,
                condition: Some("all_models_normal".to_string()),
                action: "update_state_announce_availability".to_string(),
            },
            StateTransition {
                from_state: PackageState::Initializing as i32,
                event: "partial_initialization_failure".to_string(),
                to_state: PackageState::Degraded as i32,
                condition: Some("critical_models_normal".to_string()),
                action: "log_warning_activate_partial_functionality".to_string(),
            },
            StateTransition {
                from_state: PackageState::Initializing as i32,
                event: "critical_initialization_failure".to_string(),
                to_state: PackageState::Error as i32,
                condition: Some("critical_models_failed".to_string()),
                action: "log_error_attempt_recovery".to_string(),
            },
            StateTransition {
                from_state: PackageState::Running as i32,
                event: "model_issue_detected".to_string(),
                to_state: PackageState::Degraded as i32,
                condition: Some("non_critical_model_issues".to_string()),
                action: "log_warning_maintain_partial_functionality".to_string(),
            },
            StateTransition {
                from_state: PackageState::Running as i32,
                event: "critical_issue_detected".to_string(),
                to_state: PackageState::Error as i32,
                condition: Some("critical_model_issues".to_string()),
                action: "log_error_attempt_recovery".to_string(),
            },
            StateTransition {
                from_state: PackageState::Running as i32,
                event: "pause_request".to_string(),
                to_state: PackageState::Paused as i32,
                condition: None,
                action: "pause_models_preserve_state".to_string(),
            },
            StateTransition {
                from_state: PackageState::Degraded as i32,
                event: "model_recovery".to_string(),
                to_state: PackageState::Running as i32,
                condition: Some("all_models_recovered".to_string()),
                action: "update_state_restore_full_functionality".to_string(),
            },
            StateTransition {
                from_state: PackageState::Degraded as i32,
                event: "additional_model_issues".to_string(),
                to_state: PackageState::Error as i32,
                condition: Some("critical_models_affected".to_string()),
                action: "log_error_attempt_recovery".to_string(),
            },
            StateTransition {
                from_state: PackageState::Degraded as i32,
                event: "pause_request".to_string(),
                to_state: PackageState::Paused as i32,
                condition: None,
                action: "pause_models_preserve_state".to_string(),
            },
            StateTransition {
                from_state: PackageState::Error as i32,
                event: "recovery_successful".to_string(),
                to_state: PackageState::Running as i32,
                condition: Some("depends_on_recovery_level".to_string()),
                action: "update_state_announce_functionality_restoration".to_string(),
            },
            StateTransition {
                from_state: PackageState::Paused as i32,
                event: "resume_request".to_string(),
                to_state: PackageState::Running as i32,
                condition: Some("depends_on_previous_state".to_string()),
                action: "resume_models_restore_state".to_string(),
            },
            StateTransition {
                from_state: PackageState::Running as i32,
                event: "update_request".to_string(),
                to_state: PackageState::Updating as i32,
                condition: None,
                action: "start_update_process".to_string(),
            },
            StateTransition {
                from_state: PackageState::Updating as i32,
                event: "update_successful".to_string(),
                to_state: PackageState::Running as i32,
                condition: None,
                action: "activate_new_version_update_state".to_string(),
            },
            StateTransition {
                from_state: PackageState::Updating as i32,
                event: "update_failed".to_string(),
                to_state: PackageState::Error as i32,
                condition: Some("depends_on_rollback_settings".to_string()),
                action: "rollback_or_error_handling".to_string(),
            },
        ];

        self.transition_tables
            .insert(ResourceType::Package, package_transitions);
    }

    /// Initialize the state transition table for Model resources
    ///
    /// Sets up state transitions specific to Model resources, covering:
    /// - Model loading and initialization
    /// - Training and inference states
    /// - Model versioning and updates
    /// - Resource allocation and cleanup
    ///
    /// # Implementation Note
    /// Model transitions may include resource-intensive operations and
    /// should account for memory and compute constraints.
    fn initialize_model_transitions(&mut self) {
        let model_transitions = vec![
            StateTransition {
                from_state: ModelState::Unspecified as i32,
                event: "creation_request".to_string(),
                to_state: ModelState::Pending as i32,
                condition: None,
                action: "start_node_selection_and_allocation".to_string(),
            },
            StateTransition {
                from_state: ModelState::Pending as i32,
                event: "node_allocation_complete".to_string(),
                to_state: ModelState::ContainerCreating as i32,
                condition: Some("sufficient_resources".to_string()),
                action: "pull_container_images_mount_volumes".to_string(),
            },
            StateTransition {
                from_state: ModelState::Pending as i32,
                event: "node_allocation_failed".to_string(),
                to_state: ModelState::Failed as i32,
                condition: Some("timeout_or_error".to_string()),
                action: "log_error_retry_or_reschedule".to_string(),
            },
            StateTransition {
                from_state: ModelState::ContainerCreating as i32,
                event: "container_creation_complete".to_string(),
                to_state: ModelState::Running as i32,
                condition: Some("all_containers_started".to_string()),
                action: "update_state_start_readiness_checks".to_string(),
            },
            StateTransition {
                from_state: ModelState::ContainerCreating as i32,
                event: "container_creation_failed".to_string(),
                to_state: ModelState::Failed as i32,
                condition: None,
                action: "log_error_retry_or_reschedule".to_string(),
            },
            StateTransition {
                from_state: ModelState::Running as i32,
                event: "temporary_task_complete".to_string(),
                to_state: ModelState::Succeeded as i32,
                condition: Some("one_time_task".to_string()),
                action: "log_completion_clean_up_resources".to_string(),
            },
            StateTransition {
                from_state: ModelState::Running as i32,
                event: "container_termination".to_string(),
                to_state: ModelState::Failed as i32,
                condition: Some("unexpected_termination".to_string()),
                action: "log_error_evaluate_automatic_restart".to_string(),
            },
            StateTransition {
                from_state: ModelState::Running as i32,
                event: "repeated_crash_detection".to_string(),
                to_state: ModelState::CrashLoopBackOff as i32,
                condition: Some("consecutive_restart_failures".to_string()),
                action: "set_backoff_timer_collect_logs".to_string(),
            },
            StateTransition {
                from_state: ModelState::Running as i32,
                event: "monitoring_failure".to_string(),
                to_state: ModelState::Unknown as i32,
                condition: Some("node_communication_issues".to_string()),
                action: "attempt_diagnostics_restore_communication".to_string(),
            },
            StateTransition {
                from_state: ModelState::CrashLoopBackOff as i32,
                event: "backoff_time_elapsed".to_string(),
                to_state: ModelState::Running as i32,
                condition: Some("restart_successful".to_string()),
                action: "resume_monitoring_reset_counter".to_string(),
            },
            StateTransition {
                from_state: ModelState::CrashLoopBackOff as i32,
                event: "maximum_retries_exceeded".to_string(),
                to_state: ModelState::Failed as i32,
                condition: Some("retry_limit_reached".to_string()),
                action: "log_error_notify_for_manual_intervention".to_string(),
            },
            StateTransition {
                from_state: ModelState::Unknown as i32,
                event: "state_check_recovered".to_string(),
                to_state: ModelState::Running as i32,
                condition: Some("depends_on_actual_state".to_string()),
                action: "synchronize_state_recover_if_needed".to_string(),
            },
            StateTransition {
                from_state: ModelState::Failed as i32,
                event: "manual_automatic_recovery".to_string(),
                to_state: ModelState::Pending as i32,
                condition: Some("according_to_restart_policy".to_string()),
                action: "start_model_recreation".to_string(),
            },
        ];

        self.transition_tables
            .insert(ResourceType::Model, model_transitions);
    }

    // ========================================
    // CORE STATE PROCESSING
    // ========================================
    /// Process a state change request with non-blocking action execution
    pub fn process_state_change(&mut self, state_change: StateChange) -> TransitionResult {
        // Validate input parameters
        if let Err(validation_error) = self.validate_state_change(&state_change) {
            return TransitionResult {
                new_state: Self::state_str_to_enum(
                    state_change.current_state.as_str(),
                    state_change.resource_type,
                ),
                error_code: ErrorCode::InvalidRequest,
                message: format!("Invalid state change request: {validation_error}"),
                actions_to_execute: vec![],
                transition_id: state_change.transition_id.clone(),
                error_details: validation_error,
            };
        }

        // Convert i32 to ResourceType enum
        let resource_type = match ResourceType::try_from(state_change.resource_type) {
            Ok(rt) => rt,
            Err(_) => {
                return TransitionResult {
                    new_state: Self::state_str_to_enum(
                        state_change.current_state.as_str(),
                        state_change.resource_type,
                    ),
                    error_code: ErrorCode::InvalidStateTransition,
                    message: format!("Invalid resource type: {}", state_change.resource_type),
                    actions_to_execute: vec![],
                    transition_id: state_change.transition_id.clone(),
                    error_details: format!(
                        "Unsupported resource type ID: {}",
                        state_change.resource_type
                    ),
                };
            }
        };

        let resource_key = self.generate_resource_key(resource_type, &state_change.resource_name);

        // Get current state - use provided current_state for new resources
        let current_state = match self.resource_states.get(&resource_key) {
            Some(existing_state) => existing_state.current_state,
            None => Self::state_str_to_enum(
                state_change.current_state.as_str(),
                state_change.resource_type,
            ),
        };

        // Check for special CrashLoopBackOff handling
        if current_state == ModelState::CrashLoopBackOff as i32 {
            if let Some(backoff_time) = self.backoff_timers.get(&resource_key) {
                if backoff_time.elapsed() < Duration::from_secs(BACKOFF_DURATION_SECS) {
                    return TransitionResult {
                        new_state: current_state,
                        error_code: ErrorCode::PreconditionFailed,
                        message: "Resource is in backoff period".to_string(),
                        actions_to_execute: vec![],
                        transition_id: state_change.transition_id.clone(),
                        error_details: "Backoff timer has not elapsed yet".to_string(),
                    };
                }
            }
        }

        // Find valid transition
        let transition_event = self.infer_event_from_states(
            current_state,
            Self::state_str_to_enum(
                state_change.target_state.as_str(),
                state_change.resource_type,
            ),
            resource_type,
        );

        if let Some(transition) = self.find_valid_transition(
            resource_type,
            current_state,
            &transition_event,
            Self::state_str_to_enum(
                state_change.target_state.as_str(),
                state_change.resource_type,
            ),
        ) {
            // Check conditions if any
            if let Some(ref condition) = transition.condition {
                if !self.evaluate_condition(condition, &state_change) {
                    return TransitionResult {
                        new_state: current_state,
                        error_code: ErrorCode::PreconditionFailed,
                        message: format!("Condition not met: {condition}"),
                        actions_to_execute: vec![],
                        transition_id: state_change.transition_id.clone(),
                        error_details: format!("Failed condition evaluation: {condition}"),
                    };
                }
            }

            // Execute transition - this is immediate and non-blocking
            self.update_resource_state(
                &resource_key,
                &state_change,
                transition.to_state,
                resource_type,
            );

            // **NON-BLOCKING ACTION EXECUTION** - Queue action for async execution
            if let Some(ref sender) = self.action_sender {
                let action_command = ActionCommand {
                    action: transition.action.clone(),
                    resource_key: resource_key.clone(),
                    resource_type,
                    transition_id: state_change.transition_id.clone(),
                    context: self.build_action_context(&state_change, &transition),
                };

                // Send action for async execution (non-blocking)
                if let Err(e) = sender.send(action_command) {
                    eprintln!("Warning: Failed to queue action for execution: {e}");
                }
            }

            let transitioned_state_str = match resource_type {
                ResourceType::Scenario => ScenarioState::try_from(transition.to_state)
                    .map(|s| s.as_str_name())
                    .unwrap_or("UNKNOWN"),
                ResourceType::Package => PackageState::try_from(transition.to_state)
                    .map(|s| s.as_str_name())
                    .unwrap_or("UNKNOWN"),
                ResourceType::Model => ModelState::try_from(transition.to_state)
                    .map(|s| s.as_str_name())
                    .unwrap_or("UNKNOWN"),
                _ => "UNKNOWN",
            };

            // Create successful transition result
            let transition_result = TransitionResult {
                new_state: transition.to_state,
                error_code: ErrorCode::Success,
                message: format!("Successfully transitioned to {transitioned_state_str}"),
                actions_to_execute: vec![transition.action.clone()],
                transition_id: state_change.transition_id.clone(),
                error_details: String::new(),
            };

            self.update_health_status(&resource_key, &transition_result);

            // Handle special state-specific logic
            if transition.to_state == ModelState::CrashLoopBackOff as i32 {
                self.backoff_timers
                    .insert(resource_key.clone(), Instant::now());
            }

            transition_result
        } else {
            let current_state_str = match resource_type {
                ResourceType::Scenario => ScenarioState::try_from(current_state)
                    .map(|s| s.as_str_name())
                    .unwrap_or("UNKNOWN"),
                ResourceType::Package => PackageState::try_from(current_state)
                    .map(|s| s.as_str_name())
                    .unwrap_or("UNKNOWN"),
                ResourceType::Model => ModelState::try_from(current_state)
                    .map(|s| s.as_str_name())
                    .unwrap_or("UNKNOWN"),
                _ => "UNKNOWN",
            };

            let target_state_str = match resource_type {
                ResourceType::Scenario => {
                    let normalized = format!(
                        "SCENARIO_STATE_{}",
                        state_change
                            .target_state
                            .trim()
                            .to_ascii_uppercase()
                            .replace('-', "_")
                    );
                    ScenarioState::from_str_name(&normalized)
                        .map(|s| s.as_str_name())
                        .unwrap_or("UNKNOWN")
                }
                ResourceType::Package => {
                    let normalized = format!(
                        "PACKAGE_STATE_{}",
                        state_change
                            .target_state
                            .trim()
                            .to_ascii_uppercase()
                            .replace('-', "_")
                    );
                    PackageState::from_str_name(&normalized)
                        .map(|s| s.as_str_name())
                        .unwrap_or("UNKNOWN")
                }
                ResourceType::Model => {
                    let normalized = format!(
                        "MODEL_STATE_{}",
                        state_change
                            .target_state
                            .trim()
                            .to_ascii_uppercase()
                            .replace('-', "_")
                    );
                    ModelState::from_str_name(&normalized)
                        .map(|s| s.as_str_name())
                        .unwrap_or("UNKNOWN")
                }
                _ => "UNKNOWN",
            };

            let transition_result = TransitionResult {
                new_state: current_state,
                error_code: ErrorCode::InvalidStateTransition,
                message: format!(
                    "No valid transition from {current_state_str} to {target_state_str} for resource type {resource_type:?}",
                ),
                actions_to_execute: vec![],
                transition_id: state_change.transition_id.clone(),
                error_details: format!(
                    "Invalid state transition attempted: {current_state_str} -> {target_state_str}"
                ),
            };

            self.update_health_status(&resource_key, &transition_result);
            transition_result
        }
    }

    // ========================================
    // MODEL STATE EVALUATION FROM CONTAINERS
    // ========================================

    /// Evaluate model state based on container states according to Korean documentation
    ///
    /// Implementation follows the state transition conditions defined in
    /// `doc/architecture/KR/3.LLD/StateManager_Model.md` section 4.2:
    /// - Created: model의 최초 상태 (생성 시 기본 상태)
    /// - Paused: 모든 container가 paused 상태일 때
    /// - Exited: 모든 container가 exited 상태일 때  
    /// - Dead: 하나 이상의 container가 dead 상태이거나, model 정보 조회 실패
    /// - Running: 위 조건을 모두 만족하지 않을 때(기본 상태)
    ///
    /// # Parameters
    /// * `model_name` - Name of the model to evaluate
    /// * `container_states` - Map of container states from container list
    ///
    /// # Returns
    /// * `ModelState` - The determined model state based on container conditions
    pub fn evaluate_model_state_from_containers(
        &self,
        model_name: &str,
        container_states: &std::collections::HashMap<String, String>,
    ) -> ModelState {
        if container_states.is_empty() {
            return ModelState::Pending; // Map "Created" to Pending
        }

        let mut all_paused = true;
        let mut all_exited = true;
        let mut has_dead = false;
        let mut _has_running = false;

        // Analyze each container state for the model
        for (container_name, state) in container_states {
            // Skip containers that don't belong to this model
            if !container_name.contains(model_name) {
                continue;
            }

            match state.to_lowercase().as_str() {
                "paused" => {
                    all_exited = false;
                }
                "exited" | "stopped" => {
                    all_paused = false;
                }
                "dead" | "failed" | "error" => {
                    has_dead = true;
                    all_paused = false;
                    all_exited = false;
                }
                "running" | "started" => {
                    _has_running = true;
                    all_paused = false;
                    all_exited = false;
                }
                _ => {
                    // Unknown state treated as not paused/exited
                    all_paused = false;
                    all_exited = false;
                }
            }
        }

        // Apply state transition rules from documentation
        // Map to existing protocol buffer states
        if has_dead {
            ModelState::Failed // Map "Dead" to Failed
        } else if all_paused {
            ModelState::Unknown // Map "Paused" to Unknown (temporary)
        } else if all_exited {
            ModelState::Succeeded // Map "Exited" to Succeeded
        } else {
            // Default state when no specific conditions are met
            ModelState::Running
        }
    }

    /// Process container list to determine model state changes
    ///
    /// Analyzes the container list from NodeAgent and evaluates which models
    /// need state updates based on their container conditions.
    ///
    /// # Parameters
    /// * `container_list` - Container list from NodeAgent
    ///
    /// # Returns
    /// * `Vec<(String, ModelState)>` - List of models that need state updates
    pub async fn process_container_list_for_models(
        &mut self,
        container_list: &common::monitoringserver::ContainerList,
    ) -> Vec<(String, ModelState)> {
        let mut model_state_updates = Vec::new();
        let mut model_containers: std::collections::HashMap<
            String,
            std::collections::HashMap<String, String>,
        > = std::collections::HashMap::new();

        // Group containers by model based on annotations or naming convention
        for container in &container_list.containers {
            // Extract model name from container annotation or name
            let model_name = self.extract_model_name_from_container(container);

            if let Some(model_name) = model_name {
                let container_states = model_containers
                    .entry(model_name.clone())
                    .or_insert_with(std::collections::HashMap::new);

                // Get container state - use the first state entry if multiple exist
                let container_state = container
                    .state
                    .values()
                    .next()
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());

                container_states.insert(container.id.clone(), container_state);
            }
        }

        // Evaluate state for each model
        for (model_name, container_states) in model_containers {
            let new_state =
                self.evaluate_model_state_from_containers(&model_name, &container_states);

            // Check if state has changed from current state
            let current_state = self.get_current_model_state(&model_name);
            if current_state != new_state {
                model_state_updates.push((model_name.clone(), new_state));

                // Update internal state tracking
                self.update_model_state(&model_name, new_state);

                // Persist to etcd
                if let Err(e) = self
                    .persist_model_state_to_etcd(&model_name, new_state)
                    .await
                {
                    eprintln!(
                        "Failed to persist model {} state to etcd: {:?}",
                        model_name, e
                    );
                }
            }
        }

        model_state_updates
    }

    /// Extract model name from container information
    ///
    /// Uses container annotations or naming conventions to identify which model
    /// a container belongs to.
    ///
    /// # Parameters  
    /// * `container` - Container information from container list
    ///
    /// # Returns
    /// * `Option<String>` - Model name if identifiable, None otherwise
    fn extract_model_name_from_container(
        &self,
        container: &common::monitoringserver::ContainerInfo,
    ) -> Option<String> {
        // Check container annotations for model name
        if let Some(model_name) = container.annotation.get("pullpiri.model") {
            return Some(model_name.clone());
        }

        // Check container config for model name
        if let Some(model_name) = container.config.get("model") {
            return Some(model_name.clone());
        }

        // Extract from container name using naming convention
        // Assumes containers are named like: <model_name>-<suffix>
        for name in &container.names {
            if let Some(model_name) = name.split('-').next() {
                if !model_name.is_empty() && model_name != "pod" && model_name != "container" {
                    return Some(model_name.to_string());
                }
            }
        }

        None
    }

    /// Get current model state from internal tracking
    fn get_current_model_state(&self, model_name: &str) -> ModelState {
        if let Some(resource_state) = self.resource_states.get(model_name) {
            // Convert i32 state back to ModelState enum
            match resource_state.current_state {
                s if s == ModelState::Pending as i32 => ModelState::Pending,
                s if s == ModelState::Unknown as i32 => ModelState::Unknown,
                s if s == ModelState::Succeeded as i32 => ModelState::Succeeded,
                s if s == ModelState::Failed as i32 => ModelState::Failed,
                s if s == ModelState::Running as i32 => ModelState::Running,
                _ => ModelState::Pending, // Default fallback
            }
        } else {
            ModelState::Pending // Default for new models
        }
    }

    /// Update internal model state tracking
    fn update_model_state(&mut self, model_name: &str, new_state: ModelState) {
        let resource_state = self
            .resource_states
            .entry(model_name.to_string())
            .or_insert_with(|| crate::types::ResourceState {
                resource_type: ResourceType::Model,
                resource_name: model_name.to_string(),
                current_state: ModelState::Pending as i32,
                desired_state: None,
                last_transition_time: tokio::time::Instant::now(),
                transition_count: 0,
                metadata: std::collections::HashMap::new(),
                health_status: crate::types::HealthStatus {
                    healthy: true,
                    status_message: "Model state updated".to_string(),
                    last_check: tokio::time::Instant::now(),
                    consecutive_failures: 0,
                },
            });

        resource_state.current_state = new_state as i32;
        resource_state.last_transition_time = tokio::time::Instant::now();
        resource_state.transition_count += 1;
    }

    /// Persist model state to etcd following the documentation format
    ///
    /// Uses the format specified in section 5 of StateManager_Model.md:
    /// - Key: `/model/{model_name}/state`
    /// - Value: state name as string (e.g., "Running")
    async fn persist_model_state_to_etcd(
        &self,
        model_name: &str,
        state: ModelState,
    ) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let key = format!("/model/{}/state", model_name);
        let value = match state {
            ModelState::Pending => "Pending", // Maps to "Created" from documentation
            ModelState::Unknown => "Paused",  // Maps to "Paused" from documentation
            ModelState::Succeeded => "Exited", // Maps to "Exited" from documentation
            ModelState::Failed => "Dead",     // Maps to "Dead" from documentation
            ModelState::Running => "Running",
            _ => "Unknown",
        };

        if let Err(e) = common::etcd::put(&key, value).await {
            eprintln!("Failed to save model state to etcd: {:?}", e);
            return Err(Box::new(e));
        }

        Ok(())
    }

    // ========================================
    // VALIDATION AND UTILITY METHODS
    // ========================================

    /// Find a valid transition rule for the given parameters
    ///
    /// Searches the appropriate transition table for a rule that matches
    /// the specified resource type, current state, event, and target state.
    ///
    /// # Parameters
    /// - `resource_type`: The type of resource to check transitions for
    /// - `from_state`: The current state of the resource
    /// - `event`: The event triggering the transition
    /// - `to_state`: The desired target state
    ///
    /// # Returns
    /// - `Some(StateTransition)`: If a valid transition rule is found
    /// - `None`: If no valid transition exists for the given parameters
    ///
    /// # Implementation Details
    /// This method performs exact matching on all transition parameters.
    /// Wildcard or pattern matching is not currently supported.
    fn find_valid_transition(
        &self,
        resource_type: ResourceType,
        from_state: i32,
        event: &str,
        to_state: i32,
    ) -> Option<StateTransition> {
        if let Some(transitions) = self.transition_tables.get(&resource_type) {
            for transition in transitions {
                if transition.from_state == from_state
                    && transition.event == event
                    && transition.to_state == to_state
                {
                    return Some(transition.clone());
                }
            }
        }
        None
    }

    /// Validate state change request parameters
    fn validate_state_change(&self, state_change: &StateChange) -> Result<(), String> {
        if state_change.resource_name.trim().is_empty() {
            return Err("Resource name cannot be empty".to_string());
        }

        if state_change.transition_id.trim().is_empty() {
            return Err("Transition ID cannot be empty".to_string());
        }

        if state_change.current_state == state_change.target_state {
            return Err("Current and target states cannot be the same".to_string());
        }

        if state_change.source.trim().is_empty() {
            return Err("Source cannot be empty".to_string());
        }

        Ok(())
    }

    /// Generate a unique resource key
    fn generate_resource_key(&self, resource_type: ResourceType, resource_name: &str) -> String {
        format!("{resource_type:?}::{resource_name}")
    }

    /// Build context for action execution
    fn build_action_context(
        &self,
        state_change: &StateChange,
        transition: &StateTransition,
    ) -> HashMap<String, String> {
        let mut context = HashMap::new();

        let resource_type = match ResourceType::try_from(state_change.resource_type) {
            Ok(rt) => rt,
            Err(_) => ResourceType::Scenario, // fallback, adjust as needed
        };

        let from_state_str = match resource_type {
            ResourceType::Scenario => ScenarioState::try_from(transition.from_state)
                .map(|s| s.as_str_name())
                .unwrap_or("UNKNOWN"),
            ResourceType::Package => PackageState::try_from(transition.from_state)
                .map(|s| s.as_str_name())
                .unwrap_or("UNKNOWN"),
            ResourceType::Model => ModelState::try_from(transition.from_state)
                .map(|s| s.as_str_name())
                .unwrap_or("UNKNOWN"),
            _ => "UNKNOWN",
        };

        let to_state_str = match resource_type {
            ResourceType::Scenario => ScenarioState::try_from(transition.to_state)
                .map(|s| s.as_str_name())
                .unwrap_or("UNKNOWN"),
            ResourceType::Package => PackageState::try_from(transition.to_state)
                .map(|s| s.as_str_name())
                .unwrap_or("UNKNOWN"),
            ResourceType::Model => ModelState::try_from(transition.to_state)
                .map(|s| s.as_str_name())
                .unwrap_or("UNKNOWN"),
            _ => "UNKNOWN",
        };

        context.insert("from_state".to_string(), from_state_str.to_string());
        context.insert("to_state".to_string(), to_state_str.to_string());
        context.insert("event".to_string(), transition.event.clone());
        context.insert(
            "resource_name".to_string(),
            state_change.resource_name.clone(),
        );
        context.insert("source".to_string(), state_change.source.clone());
        context.insert(
            "timestamp_ns".to_string(),
            state_change.timestamp_ns.to_string(),
        );
        context
    }

    /// Updates health status based on transition result
    fn update_health_status(&mut self, resource_key: &str, transition_result: &TransitionResult) {
        if let Some(resource_state) = self.resource_states.get_mut(resource_key) {
            let now = Instant::now();
            resource_state.health_status.last_check = now;

            if transition_result.is_success() {
                resource_state.health_status.healthy = true;
                resource_state.health_status.consecutive_failures = 0;
                resource_state.health_status.status_message = "Healthy".to_string();
            } else {
                resource_state.health_status.consecutive_failures += 1;
                resource_state.health_status.status_message = transition_result.message.clone();

                // Mark as unhealthy if we have multiple consecutive failures
                if resource_state.health_status.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    resource_state.health_status.healthy = false;
                }
            }
        }
    }

    /// Infer the appropriate event type from state transition
    ///
    /// When an explicit event is not provided, this method attempts to
    /// determine the most appropriate event based on the current and target states.
    ///
    /// # Parameters
    /// - `current_state`: The current state of the resource
    /// - `target_state`: The desired target state
    ///
    /// # Returns
    /// A string representing the inferred event type
    ///
    /// # Examples
    /// - "Inactive" -> "Active" might infer "activate"
    /// - "Running" -> "Stopped" might infer "stop"
    /// - Any state -> "Failed" might infer "error"
    ///
    /// # Fallback Behavior
    /// If no specific event can be inferred, returns a generic event name
    /// based on the target state (e.g., "transition_to_active").
    fn infer_event_from_states(
        &self,
        current_state: i32,
        target_state: i32,
        resource_type: ResourceType,
    ) -> String {
        match resource_type {
            ResourceType::Scenario => match (current_state, target_state) {
                (x, y) if x == ScenarioState::Idle as i32 && y == ScenarioState::Waiting as i32 => {
                    "scenario_activation".to_string()
                }
                (x, y)
                    if x == ScenarioState::Waiting as i32 && y == ScenarioState::Allowed as i32 =>
                {
                    "condition_met".to_string()
                }
                (x, y)
                    if x == ScenarioState::Allowed as i32 && y == ScenarioState::Playing as i32 =>
                {
                    "policy_verification_success".to_string()
                }
                (x, y)
                    if x == ScenarioState::Allowed as i32 && y == ScenarioState::Denied as i32 =>
                {
                    "policy_verification_failure".to_string()
                }
                _ => format!("transition_{current_state}_{target_state}"),
            },
            ResourceType::Package => match (current_state, target_state) {
                (x, y)
                    if x == PackageState::Unspecified as i32
                        && y == PackageState::Initializing as i32 =>
                {
                    "launch_request".to_string()
                }
                (x, y)
                    if x == PackageState::Initializing as i32
                        && y == PackageState::Running as i32 =>
                {
                    "initialization_complete".to_string()
                }
                (x, y)
                    if x == PackageState::Initializing as i32
                        && y == PackageState::Degraded as i32 =>
                {
                    "partial_initialization_failure".to_string()
                }
                (x, y)
                    if x == PackageState::Initializing as i32
                        && y == PackageState::Error as i32 =>
                {
                    "critical_initialization_failure".to_string()
                }
                (x, y)
                    if x == PackageState::Running as i32 && y == PackageState::Degraded as i32 =>
                {
                    "model_issue_detected".to_string()
                }
                (x, y) if x == PackageState::Running as i32 && y == PackageState::Error as i32 => {
                    "critical_issue_detected".to_string()
                }
                (x, y) if x == PackageState::Running as i32 && y == PackageState::Paused as i32 => {
                    "pause_request".to_string()
                }
                (x, y)
                    if x == PackageState::Running as i32 && y == PackageState::Updating as i32 =>
                {
                    "update_request".to_string()
                }
                (x, y)
                    if x == PackageState::Degraded as i32 && y == PackageState::Running as i32 =>
                {
                    "model_recovery".to_string()
                }
                (x, y) if x == PackageState::Degraded as i32 && y == PackageState::Error as i32 => {
                    "additional_model_issues".to_string()
                }
                (x, y)
                    if x == PackageState::Degraded as i32 && y == PackageState::Paused as i32 =>
                {
                    "pause_request".to_string()
                }
                (x, y) if x == PackageState::Error as i32 && y == PackageState::Running as i32 => {
                    "recovery_successful".to_string()
                }
                (x, y) if x == PackageState::Paused as i32 && y == PackageState::Running as i32 => {
                    "resume_request".to_string()
                }
                (x, y)
                    if x == PackageState::Updating as i32 && y == PackageState::Running as i32 =>
                {
                    "update_successful".to_string()
                }
                (x, y) if x == PackageState::Updating as i32 && y == PackageState::Error as i32 => {
                    "update_failed".to_string()
                }
                _ => format!("transition_{current_state}_{target_state}"),
            },
            ResourceType::Model => match (current_state, target_state) {
                (x, y)
                    if x == ModelState::Unspecified as i32 && y == ModelState::Pending as i32 =>
                {
                    "creation_request".to_string()
                }
                (x, y)
                    if x == ModelState::Pending as i32
                        && y == ModelState::ContainerCreating as i32 =>
                {
                    "node_allocation_complete".to_string()
                }
                (x, y) if x == ModelState::Pending as i32 && y == ModelState::Failed as i32 => {
                    "node_allocation_failed".to_string()
                }
                (x, y)
                    if x == ModelState::ContainerCreating as i32
                        && y == ModelState::Running as i32 =>
                {
                    "container_creation_complete".to_string()
                }
                (x, y)
                    if x == ModelState::ContainerCreating as i32
                        && y == ModelState::Failed as i32 =>
                {
                    "container_creation_failed".to_string()
                }
                (x, y) if x == ModelState::Running as i32 && y == ModelState::Succeeded as i32 => {
                    "temporary_task_complete".to_string()
                }
                (x, y) if x == ModelState::Running as i32 && y == ModelState::Failed as i32 => {
                    "container_termination".to_string()
                }
                (x, y)
                    if x == ModelState::Running as i32
                        && y == ModelState::CrashLoopBackOff as i32 =>
                {
                    "repeated_crash_detection".to_string()
                }
                (x, y) if x == ModelState::Running as i32 && y == ModelState::Unknown as i32 => {
                    "monitoring_failure".to_string()
                }
                (x, y)
                    if x == ModelState::CrashLoopBackOff as i32
                        && y == ModelState::Running as i32 =>
                {
                    "backoff_time_elapsed".to_string()
                }
                (x, y)
                    if x == ModelState::CrashLoopBackOff as i32
                        && y == ModelState::Failed as i32 =>
                {
                    "maximum_retries_exceeded".to_string()
                }
                (x, y) if x == ModelState::Unknown as i32 && y == ModelState::Running as i32 => {
                    "state_check_recovered".to_string()
                }
                (x, y) if x == ModelState::Failed as i32 && y == ModelState::Pending as i32 => {
                    "manual_automatic_recovery".to_string()
                }
                _ => format!("transition_{current_state}_{target_state}"),
            },
            _ => format!("transition_{current_state}_{target_state}"),
        }
    }

    /// Evaluate whether a transition condition is satisfied
    ///
    /// Processes conditional logic attached to state transitions to determine
    /// if the transition should be allowed to proceed.
    ///
    /// # Parameters
    /// - `condition`: The condition string to evaluate (e.g., "resource_count > 0")
    /// - `_state_change`: The state change request providing context for evaluation
    ///
    /// # Returns
    /// - `true`: If the condition is satisfied or no condition exists
    /// - `false`: If the condition fails evaluation
    ///
    /// # Supported Conditions
    /// The condition language should support:
    /// - Resource property comparisons
    /// - Metadata key existence checks
    /// - Numeric and string comparisons
    /// - Boolean logic operators
    ///
    /// # Error Handling
    /// Malformed conditions should be logged and default to `false` for safety.
    fn evaluate_condition(&self, condition: &str, _state_change: &StateChange) -> bool {
        // TODO: Implement real condition evaluation logic
        match condition {
            "all_models_normal" => true,
            "critical_models_normal" => true,
            "critical_models_failed" => false,
            "non_critical_model_issues" => true,
            "critical_model_issues" => false,
            "all_models_recovered" => true,
            "critical_models_affected" => false,
            "depends_on_recovery_level" => true,
            "depends_on_previous_state" => true,
            "depends_on_rollback_settings" => true,
            "sufficient_resources" => true,
            "timeout_or_error" => false,
            "all_containers_started" => true,
            "one_time_task" => true,
            "unexpected_termination" => false,
            "consecutive_restart_failures" => false,
            "node_communication_issues" => false,
            "restart_successful" => true,
            "retry_limit_reached" => false,
            "depends_on_actual_state" => true,
            "according_to_restart_policy" => true,
            _ => true, // Default to allow transition for unknown conditions
        }
    }

    /// Update the internal resource state after a successful transition
    ///
    /// Performs all necessary bookkeeping when a state transition succeeds,
    /// including updating timestamps, incrementing counters, and managing metadata.
    ///
    /// # Parameters
    /// - `resource_key`: Unique identifier for the resource
    /// - `state_change`: The original state change request
    /// - `new_state`: The state the resource has transitioned to
    /// - `resource_type`: The type of the resource
    ///
    /// # Side Effects
    /// - Updates or creates the resource state entry
    /// - Increments transition counter
    /// - Updates last transition timestamp
    /// - Clears any active backoff timers on successful transition
    /// - Updates health status if applicable
    fn update_resource_state(
        &mut self,
        resource_key: &str,
        state_change: &StateChange,

        new_state: i32,
        resource_type: ResourceType,
    ) {
        let now = Instant::now();

        let resource_state = self
            .resource_states
            .entry(resource_key.to_string())
            .or_insert_with(|| ResourceState {
                resource_type,
                resource_name: state_change.resource_name.clone(),
                current_state: Self::state_str_to_enum(
                    state_change.current_state.as_str(),
                    state_change.resource_type,
                ),
                desired_state: Some(Self::state_str_to_enum(
                    state_change.target_state.as_str(),
                    state_change.resource_type,
                )),
                last_transition_time: now,
                transition_count: 0,
                metadata: HashMap::new(),
                health_status: HealthStatus {
                    healthy: true,
                    status_message: "Healthy".to_string(),
                    last_check: now,
                    consecutive_failures: 0,
                },
            });

        resource_state.current_state = new_state;
        resource_state.last_transition_time = now;
        resource_state.transition_count += 1;
        resource_state.metadata.insert(
            "last_transition_id".to_string(),
            state_change.transition_id.clone(),
        );
        resource_state
            .metadata
            .insert("source".to_string(), state_change.source.clone());
    }

    // ========================================
    // PUBLIC QUERY METHODS
    // ========================================

    /// Retrieve the current state information for a specific resource
    ///
    /// Provides read-only access to the complete state information for
    /// a resource, including metadata and health status.
    ///
    /// # Parameters
    /// - `resource_name`: The unique name of the resource
    /// - `resource_type`: The type of the resource (for validation)
    ///
    /// # Returns
    /// - `Some(&ResourceState)`: If the resource exists and types match
    /// - `None`: If the resource doesn't exist or type mismatch
    ///
    /// # Usage
    /// This method is primarily used for:
    /// - Status queries from external systems
    /// - Health check implementations
    /// - Audit and monitoring purposes
    pub fn get_resource_state(
        &self,
        resource_name: &str,
        resource_type: ResourceType,
    ) -> Option<&ResourceState> {
        let resource_key = self.generate_resource_key(resource_type, resource_name);
        self.resource_states.get(&resource_key)
    }

    /// List all resources currently in a specific state
    ///
    /// Provides a filtered view of all managed resources based on their
    /// current state, optionally filtered by resource type.
    ///
    /// # Parameters
    /// - `resource_type`: Optional filter for resource type (None = all types)
    /// - `state`: The state to filter by
    ///
    /// # Returns
    /// A vector of references to all matching resource states
    ///
    /// # Performance Note
    /// This method performs a linear scan of all resources. For large numbers
    /// of resources, consider implementing indexed lookups by state.
    ///
    /// # Usage Examples
    /// - Find all failed resources: `list_resources_by_state(None, "Failed")`
    /// - Find active scenarios: `list_resources_by_state(Some(ResourceType::Scenario), "Active")`
    pub fn list_resources_by_state(
        &self,
        resource_type: Option<ResourceType>,

        state: i32,
    ) -> Vec<&ResourceState> {
        self.resource_states
            .values()
            .filter(|resource| {
                resource.current_state == state
                    && (resource_type.is_none() || resource_type == Some(resource.resource_type))
            })
            .collect()
    }

    // Utility: Convert state string to proto enum value
    fn state_str_to_enum(state: &str, resource_type: i32) -> i32 {
        // Map "idle" -> "SCENARIO_STATE_IDLE", etc.
        let normalized = match ResourceType::try_from(resource_type) {
            Ok(ResourceType::Scenario) => format!(
                "SCENARIO_STATE_{}",
                state.trim().to_ascii_uppercase().replace('-', "_")
            ),
            Ok(ResourceType::Package) => format!(
                "PACKAGE_STATE_{}",
                state.trim().to_ascii_uppercase().replace('-', "_")
            ),
            Ok(ResourceType::Model) => format!(
                "MODEL_STATE_{}",
                state.trim().to_ascii_uppercase().replace('-', "_")
            ),
            _ => state.trim().to_ascii_uppercase().replace('-', "_"),
        };
        match ResourceType::try_from(resource_type) {
            Ok(ResourceType::Scenario) => ScenarioState::from_str_name(&normalized)
                .map(|s| s as i32)
                .unwrap_or(ScenarioState::Unspecified as i32),
            Ok(ResourceType::Package) => PackageState::from_str_name(&normalized)
                .map(|s| s as i32)
                .unwrap_or(PackageState::Unspecified as i32),
            Ok(ResourceType::Model) => ModelState::from_str_name(&normalized)
                .map(|s| s as i32)
                .unwrap_or(ModelState::Unspecified as i32),
            _ => 0,
        }
    }
}

/// Default implementation that creates a new StateMachine
///
/// Provides a convenient way to create a StateMachine with default
/// configuration using the `Default` trait.
impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_evaluate_model_state_from_containers_empty() {
        let state_machine = StateMachine::new();
        let container_states = HashMap::new();
        
        let result = state_machine.evaluate_model_state_from_containers("test_model", &container_states);
        assert_eq!(result, ModelState::Pending); // Should map to "Created" from documentation
    }

    #[test]
    fn test_evaluate_model_state_from_containers_all_running() {
        let state_machine = StateMachine::new();
        let mut container_states = HashMap::new();
        container_states.insert("test_model_container1".to_string(), "running".to_string());
        container_states.insert("test_model_container2".to_string(), "running".to_string());
        
        let result = state_machine.evaluate_model_state_from_containers("test_model", &container_states);
        assert_eq!(result, ModelState::Running);
    }

    #[test] 
    fn test_evaluate_model_state_from_containers_all_paused() {
        let state_machine = StateMachine::new();
        let mut container_states = HashMap::new();
        container_states.insert("test_model_container1".to_string(), "paused".to_string());
        container_states.insert("test_model_container2".to_string(), "paused".to_string());
        
        let result = state_machine.evaluate_model_state_from_containers("test_model", &container_states);
        assert_eq!(result, ModelState::Unknown); // Maps to "Paused" from documentation
    }

    #[test]
    fn test_evaluate_model_state_from_containers_all_exited() {
        let state_machine = StateMachine::new();
        let mut container_states = HashMap::new();
        container_states.insert("test_model_container1".to_string(), "exited".to_string());
        container_states.insert("test_model_container2".to_string(), "stopped".to_string());
        
        let result = state_machine.evaluate_model_state_from_containers("test_model", &container_states);
        assert_eq!(result, ModelState::Succeeded); // Maps to "Exited" from documentation
    }

    #[test]
    fn test_evaluate_model_state_from_containers_has_dead() {
        let state_machine = StateMachine::new();
        let mut container_states = HashMap::new();
        container_states.insert("test_model_container1".to_string(), "running".to_string());
        container_states.insert("test_model_container2".to_string(), "dead".to_string());
        
        let result = state_machine.evaluate_model_state_from_containers("test_model", &container_states);
        assert_eq!(result, ModelState::Failed); // Maps to "Dead" from documentation
    }

    #[test]
    fn test_evaluate_model_state_from_containers_mixed_states() {
        let state_machine = StateMachine::new();
        let mut container_states = HashMap::new();
        container_states.insert("test_model_container1".to_string(), "running".to_string());
        container_states.insert("test_model_container2".to_string(), "paused".to_string());
        
        let result = state_machine.evaluate_model_state_from_containers("test_model", &container_states);
        assert_eq!(result, ModelState::Running); // Default when conditions don't match
    }

    #[test]
    fn test_extract_model_name_from_container_annotation() {
        let state_machine = StateMachine::new();
        let mut container = common::monitoringserver::ContainerInfo {
            id: "test_id".to_string(),
            names: vec!["test_container".to_string()],
            image: "test_image".to_string(),
            state: HashMap::new(),
            config: HashMap::new(),
            annotation: HashMap::new(),
            stats: HashMap::new(),
        };
        
        container.annotation.insert("pullpiri.model".to_string(), "my_model".to_string());
        
        let result = state_machine.extract_model_name_from_container(&container);
        assert_eq!(result, Some("my_model".to_string()));
    }

    #[test]
    fn test_extract_model_name_from_container_name() {
        let state_machine = StateMachine::new();
        let container = common::monitoringserver::ContainerInfo {
            id: "test_id".to_string(),
            names: vec!["my_model-worker".to_string()],
            image: "test_image".to_string(),
            state: HashMap::new(),
            config: HashMap::new(),
            annotation: HashMap::new(),
            stats: HashMap::new(),
        };
        
        let result = state_machine.extract_model_name_from_container(&container);
        assert_eq!(result, Some("my_model".to_string()));
    }
}
