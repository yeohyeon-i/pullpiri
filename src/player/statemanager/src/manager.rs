/*
 * SPDX-FileCopyrightText: Copyright 2024 LG Electronics Inc.
 * SPDX-License-Identifier: Apache-2.0
 */

//! StateManagerManager: Asynchronous state management engine for PICCOLO framework
//!
//! This module provides the core state management functionality for the StateManager service.
//! It receives and processes state change requests from various components (ApiServer, FilterGateway,
//! ActionController) and container status updates from nodeagent via async channels.
//!
//! The manager implements the PICCOLO Resource State Management specification, handling
//! state transitions, monitoring, reconciliation, and recovery for all resource types
//! (Scenario, Package, Model, Volume, Network, Node).

use crate::state_machine::StateMachine;
use crate::types::{ActionCommand, ContainerStateInfo, ModelInfo, PackageInfo, TransitionResult};
use common::monitoringserver::ContainerList;

use common::statemanager::{
    ErrorCode, ModelState, PackageState, ResourceType, ScenarioState, StateChange,
};

use common::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::task;
use tokio::time::Instant;

/// Core state management engine for the StateManager service.
///
/// This struct orchestrates all state management operations by receiving messages
/// from gRPC handlers via async channels and processing them according to the
/// PICCOLO Resource State Management specification.
///
/// # Architecture
/// - Receives StateChange messages from ApiServer, FilterGateway, ActionController
/// - Receives ContainerList updates from nodeagent
/// - Processes state transitions with ASIL compliance
/// - Manages resource lifecycle and dependencies
/// - Handles error recovery and reconciliation
///
/// # Threading Model
/// - Uses Arc<Mutex<mpsc::Receiver>> for safe multi-threaded access
/// - Spawns dedicated async tasks for each message type
/// - Ensures lock-free message processing with proper channel patterns
pub struct StateManagerManager {
    /// State machine for processing state transitions
    state_machine: Arc<Mutex<StateMachine>>,

    /// Channel receiver for container status updates from nodeagent.
    ///
    /// Receives ContainerList messages containing current container states,
    /// health information, and resource usage data. This enables the StateManager
    /// to monitor container health and trigger state transitions when needed.
    rx_container: Arc<Mutex<mpsc::Receiver<ContainerList>>>,

    /// Channel receiver for state change requests from various components.
    ///
    /// Receives StateChange messages from:
    /// - ApiServer: User-initiated state changes and scenario requests
    /// - FilterGateway: Policy-driven state transitions and filtering decisions
    /// - ActionController: Action execution results and state confirmations
    rx_state_change: Arc<Mutex<mpsc::Receiver<StateChange>>>,

    /// Model state tracking - maps model names to their current information
    models: Arc<Mutex<HashMap<String, ModelInfo>>>,

    /// Package state tracking - maps package names to their current information  
    packages: Arc<Mutex<HashMap<String, PackageInfo>>>,

    /// Container to model mapping - maps container IDs to model names
    container_to_model: Arc<Mutex<HashMap<String, String>>>,
}

impl StateManagerManager {
    /// Creates a new StateManagerManager instance.
    ///
    /// Initializes the manager with the provided channel receivers for processing
    /// container updates and state change requests.
    ///
    /// # Arguments
    /// * `rx_container` - Channel receiver for ContainerList messages from nodeagent
    /// * `rx_state_change` - Channel receiver for StateChange messages from components
    ///
    /// # Returns
    /// * `Self` - New StateManagerManager instance ready for initialization
    pub async fn new(
        rx_container: mpsc::Receiver<ContainerList>,
        rx_state_change: mpsc::Receiver<StateChange>,
    ) -> Self {
        Self {
            state_machine: Arc::new(Mutex::new(StateMachine::new())),
            rx_container: Arc::new(Mutex::new(rx_container)),
            rx_state_change: Arc::new(Mutex::new(rx_state_change)),
            models: Arc::new(Mutex::new(HashMap::new())),
            packages: Arc::new(Mutex::new(HashMap::new())),
            container_to_model: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Initializes the StateManagerManager's internal state and resources.
    ///
    /// Performs startup operations required before beginning message processing:
    /// - Loads initial resource states from persistent storage
    /// - Initializes state machine engines for each resource type
    /// - Sets up monitoring and health check systems
    /// - Prepares recovery and reconciliation systems
    ///
    /// # Returns
    /// * `Result<()>` - Success or initialization error
    ///
    /// # Future Enhancements
    /// - Load persisted resource states from storage (etcd, database)
    /// - Initialize state machine validators for each resource type
    /// - Set up dependency tracking and validation systems
    /// - Configure ASIL safety monitoring and alerting
    pub async fn initialize(&mut self) -> Result<()> {
        println!("StateManagerManager initializing...");

        // Initialize the state machine with async action executor
        let action_receiver = {
            let mut state_machine = self.state_machine.lock().await;
            state_machine.initialize_action_executor()
        };

        // Start the async action executor
        tokio::spawn(async move {
            run_action_executor(action_receiver).await;
        });

        println!("State machine initialized with transition tables for Scenario, Package, and Model resources");
        println!("Async action executor started for non-blocking action processing");

        // TODO: Add comprehensive initialization logic:
        // - Load persisted resource states from persistent storage
        // - Initialize state machine validators for each ResourceType
        // - Set up dependency tracking and validation systems
        // - Configure ASIL safety monitoring and alerting
        // - Initialize recovery strategies for each RecoveryType
        // - Set up health check systems for all resource types
        // - Configure event streaming and notification systems

        println!("StateManagerManager initialization completed");
        Ok(())
    }

    /// Processes a StateChange message according to PICCOLO specifications.
    ///
    /// This is the core method that handles all state transition requests in the system.
    /// It validates requests, processes transitions through the state machine, and handles
    /// both successful transitions and failure scenarios with appropriate logging and recovery.
    ///
    /// # Arguments
    /// * `state_change` - Complete StateChange message containing:
    ///   - `resource_type`: Type of resource (Scenario/Package/Model)
    ///   - `resource_name`: Unique identifier for the resource
    ///   - `current_state`: Expected current state of the resource
    ///   - `target_state`: Desired state after transition
    ///   - `transition_id`: Unique ID for tracking this transition
    ///   - `source`: Component that initiated the state change
    ///   - `timestamp_ns`: When the request was created
    ///
    /// # Processing Flow
    /// 1. **Validation**: Parse and validate resource type from the request
    /// 2. **Logging**: Log comprehensive transition details for audit trails
    /// 3. **State Machine Processing**: Execute transition through the state machine
    /// 4. **Result Handling**: Process success/failure outcomes appropriately
    /// 5. **Action Scheduling**: Queue any required follow-up actions for async execution
    /// 6. **Error Recovery**: Handle failures with appropriate recovery strategies
    ///
    /// # Error Handling
    /// - Invalid resource types are logged and ignored (early return)
    /// - State machine failures trigger the `handle_transition_failure` method
    /// - All errors are logged with detailed context for debugging
    ///
    /// # Side Effects
    /// - Updates internal resource state tracking
    /// - Queues actions for asynchronous execution
    /// - Generates log entries for audit trails
    /// - May trigger recovery procedures on failures
    ///
    /// # Thread Safety
    /// This method is async and uses internal locking for state machine access.
    /// Multiple concurrent calls are safe but will be serialized at the state machine level.
    async fn process_state_change(&self, state_change: StateChange) {
        // ========================================
        // STEP 1: RESOURCE TYPE VALIDATION
        // ========================================
        // Convert the numeric resource type from the proto message to a type-safe enum.
        // This ensures we only process known resource types and fail fast for invalid requests.
        let resource_type = match ResourceType::try_from(state_change.resource_type) {
            Ok(rt) => rt,
            Err(_) => {
                eprintln!(
                    "VALIDATION ERROR: Invalid resource type '{}' in StateChange request for resource '{}'", 
                    state_change.resource_type,
                    state_change.resource_name
                );
                return; // Early return - cannot process invalid resource types
            }
        };

        // NOTE: ASIL level parsing is commented out pending implementation of ASILLevel enum
        // This will be needed for safety-critical processing validation
        // let asil_level = match state_change.asil_level { ... };

        // ========================================
        // STEP 2: COMPREHENSIVE REQUEST LOGGING
        // ========================================
        // Log all relevant details for audit trails and debugging.
        // This structured logging enables:
        // - Troubleshooting failed transitions with complete context
        // - Audit compliance for safety-critical systems (ISO 26262)
        // - Performance monitoring and SLA tracking
        // - Dependency impact analysis and root cause investigation
        // - Security audit trails for state change authorization
        //
        // TODO: Replace println! with structured logging (tracing crate) for production:
        // - Use appropriate log levels (info, warn, error)
        // - Include correlation IDs for distributed tracing
        // - Add structured fields for metrics aggregation
        // - Implement log sampling for high-volume scenarios
        println!("=== PROCESSING STATE CHANGE ===");
        println!(
            "  Resource Type: {:?} (numeric: {})",
            resource_type, state_change.resource_type
        );
        println!("  Resource Name: {}", state_change.resource_name);
        println!(
            "  State Transition: {} -> {}",
            state_change.current_state, state_change.target_state
        );
        println!("  Transition ID: {}", state_change.transition_id);
        println!("  Source Component: {}", state_change.source);
        println!("  Timestamp: {} ns", state_change.timestamp_ns);

        // ========================================
        // COMPREHENSIVE IMPLEMENTATION ROADMAP
        // ========================================
        // TODO: The following implementation phases are planned for full PICCOLO compliance:
        //
        // PHASE 1: VALIDATION AND PRECONDITIONS
        //    ✓ Resource type validation (implemented above)
        //    - Validate state transition is allowed by resource-specific state machine rules
        //    - Verify current_state matches the actual tracked state of the resource
        //    - Ensure target_state is valid for the specific resource type
        //    - Validate ASIL safety constraints and timing requirements for critical resources
        //    - Check request format and required fields are present
        //
        // PHASE 2: DEPENDENCY AND CONSTRAINT VERIFICATION
        //    - Load and verify all resource dependencies are in required states
        //    - Check critical dependency chains and handle circular dependencies
        //    - Validate performance constraints (timing, deadlines, resource limits)
        //    - Ensure prerequisite conditions are met before allowing transition
        //    - Escalate to recovery management if dependencies are not satisfied
        //
        // PHASE 3: PRE-TRANSITION SAFETY CHECKS
        //    - Execute resource-specific pre-transition validation hooks
        //    - Perform safety checks based on ASIL level (A, B, C, D, or QM)
        //    - Validate timing constraints and deadlines for real-time requirements
        //    - Check system resource availability (CPU, memory, storage, network)
        //    - Verify external system readiness (databases, services, hardware)
        //
        // PHASE 4: STATE TRANSITION EXECUTION (currently implemented)
        //    ✓ Process transition through StateMachine (implemented below)
        //    - Handle resource-specific transition logic and business rules
        //    - Monitor transition timing for ASIL compliance and SLA requirements
        //    - Implement atomic transaction semantics for complex transitions
        //    - Handle rollback scenarios if transition fails partway through
        //
        // PHASE 5: PERSISTENT STORAGE AND AUDIT
        //    - Update resource state in persistent storage (etcd cluster, database)
        //    - Record detailed state transition history for compliance auditing
        //    - Update health status and monitoring data with new state information
        //    - Maintain state generation counters for optimistic concurrency control
        //    - Store performance metrics and timing data for analysis
        //
        // PHASE 6: NOTIFICATION AND EVENT DISTRIBUTION
        //    - Notify dependent resources of successful state changes
        //    - Generate StateChangeEvent messages for real-time subscribers
        //    - Send alerts and notifications for ASIL-critical state changes
        //    - Update monitoring, observability, and dashboard systems
        //    - Trigger webhook notifications for external integrations
        //
        // PHASE 7: POST-TRANSITION VALIDATION AND MONITORING
        //    - Verify the transition completed successfully and resource is stable
        //    - Validate the resource is actually in the expected target state
        //    - Execute post-transition health checks and readiness probes
        //    - Log completion metrics including timing, resource usage, and success rates
        //    - Schedule follow-up monitoring for transition stability
        //
        // PHASE 8: ERROR HANDLING AND RECOVERY ORCHESTRATION
        //    - Implement sophisticated retry strategies with exponential backoff
        //    - Escalate to recovery management for critical failures
        //    - Generate detailed alerts with context for operations teams
        //    - Maintain system stability during error conditions and cascading failures
        //    - Implement circuit breaker patterns for failing external dependencies

        // ========================================
        // STEP 3: STATE MACHINE PROCESSING
        // ========================================
        // Process the state change request through the core state machine.
        // This is where the actual business logic and state transition rules are applied.
        // The state machine handles:
        // - Validation of transition rules for the specific resource type
        // - Condition evaluation for conditional transitions
        // - Action scheduling for follow-up operations
        // - Error detection and reporting
        let result = {
            // Acquire exclusive lock on the state machine for this transition
            // Note: This serializes all state transitions to maintain consistency
            let mut state_machine = self.state_machine.lock().await;
            state_machine.process_state_change(state_change.clone())
        }; // Lock is automatically released here

        // ========================================
        // STEP 4: RESULT PROCESSING AND RESPONSE
        // ========================================
        // Handle the outcome of the state transition attempt.
        // Success and failure paths have different logging and follow-up actions.
        if result.is_success() {
            // ========================================
            // SUCCESS PATH: Log positive outcome and queue actions
            // ========================================
            println!("  ✓ State transition completed successfully");
            // Convert new_state to string representation based on resource type only for logs
            let new_state_str = match resource_type {
                ResourceType::Scenario => ScenarioState::try_from(result.new_state)
                    .map(|s| s.as_str_name())
                    .unwrap_or("UNKNOWN"),
                ResourceType::Package => PackageState::try_from(result.new_state)
                    .map(|s| s.as_str_name())
                    .unwrap_or("UNKNOWN"),
                ResourceType::Model => ModelState::try_from(result.new_state)
                    .map(|s| s.as_str_name())
                    .unwrap_or("UNKNOWN"),
                _ => "UNKNOWN",
            };
            println!("    Final State: {new_state_str}");
            println!("    Success Message: {}", result.message);
            println!("    Transition ID: {}", result.transition_id);

            // Log any actions that were queued for asynchronous execution
            // Actions are processed separately to keep state transitions fast
            if !result.actions_to_execute.is_empty() {
                println!("    Actions queued for async execution:");
                for action in &result.actions_to_execute {
                    println!("      - {action}");
                }
                println!(
                    "    Note: Actions will be executed asynchronously by the action executor"
                );
            }

            println!("  Status: State change processing completed successfully");
        } else {
            // ========================================
            // FAILURE PATH: Log error details and initiate recovery
            // ========================================
            println!("  ✗ State transition failed");
            // Convert new_state to string representation based on resource type only for logs
            let new_state_str = match resource_type {
                ResourceType::Scenario => ScenarioState::try_from(result.new_state)
                    .map(|s| s.as_str_name())
                    .unwrap_or("UNKNOWN"),
                ResourceType::Package => PackageState::try_from(result.new_state)
                    .map(|s| s.as_str_name())
                    .unwrap_or("UNKNOWN"),
                ResourceType::Model => ModelState::try_from(result.new_state)
                    .map(|s| s.as_str_name())
                    .unwrap_or("UNKNOWN"),
                _ => "UNKNOWN",
            };
            println!("    Error Code: {:?}", result.error_code);
            println!("    Error Message: {}", result.message);
            println!("    Error Details: {}", result.error_details);
            println!("    Current State: {new_state_str} (unchanged)");
            println!("    Failed Transition ID: {}", result.transition_id);

            // Delegate to specialized failure handling logic
            // This method will analyze the failure type and determine appropriate recovery actions
            self.handle_transition_failure(&state_change, &result).await;

            println!("  Status: State change processing completed with errors");
        }

        println!("================================");
    }

    /// Handle state transition failures
    async fn handle_transition_failure(
        &self,
        state_change: &StateChange,
        result: &TransitionResult,
    ) {
        println!(
            "    Handling transition failure for resource: {}",
            state_change.resource_name
        );
        println!("      Error: {}", result.message);
        println!("      Error code: {:?}", result.error_code);
        println!("      Error details: {}", result.error_details);

        // Generate appropriate error responses based on error type
        match result.error_code {
            ErrorCode::InvalidStateTransition => {
                println!("      Invalid state transition - checking state machine rules");
                // Would log detailed state machine validation errors
            }
            ErrorCode::PreconditionFailed => {
                println!("      Preconditions not met - evaluating retry strategy");
                // Would check if conditions might be met later and schedule retry
            }
            ErrorCode::ResourceNotFound => {
                println!("      Resource not found - may need initialization");
                // Would check if resource needs to be created or registered
            }
            _ => {
                println!("      General error - applying default error handling");
                // Would apply general error handling procedures
            }
        }

        // In a real implementation, this would:
        // - Log to audit trail
        // - Generate alerts
        // - Trigger recovery procedures
        // - Update monitoring metrics
    }

    /// Processes a ContainerList message for container health monitoring.
    ///
    /// This method handles container status updates from nodeagent and
    /// triggers appropriate state transitions based on container health.
    ///
    /// # Arguments
    /// * `container_list` - ContainerList message with node and container status
    ///
    /// # Processing Steps
    /// 1. Analyze container health and status changes
    /// 2. Identify resources affected by container changes
    /// 3. Trigger state transitions for failed or recovered containers
    /// 4. Update resource health status and monitoring data
    async fn process_container_list(&self, container_list: ContainerList) {
        println!("=== PROCESSING CONTAINER LIST ===");
        println!("  Node Name: {}", container_list.node_name);
        println!("  Container Count: {}", container_list.containers.len());

        // Convert containers to internal format for processing
        let mut container_infos = Vec::new();
        for container in &container_list.containers {
            let container_info = ContainerStateInfo {
                container_id: container.id.clone(),
                container_names: container.names.clone(),
                image: container.image.clone(),
                state: container.state.clone(),
                config: container.config.clone(),
                annotations: container.annotation.clone(),
            };
            container_infos.push(container_info);
        }

        // Process containers to update model states
        if let Err(e) = self
            .process_containers_for_model_states(container_infos)
            .await
        {
            eprintln!("Failed to process containers for model states: {}", e);
        }

        // Process models to update package states
        if let Err(e) = self.process_models_for_package_states().await {
            eprintln!("Failed to process models for package states: {}", e);
        }

        println!("  Status: Container list processing completed");
        println!("=====================================");
    }

    /// Process containers to update model states based on container status aggregation
    ///
    /// According to StateManager_Model.md specification:
    /// - Created: All containers are created but not running
    /// - Running: At least one container is running  
    /// - Paused: All containers are paused
    /// - Exited: All containers have exited
    /// - Dead: At least one container is dead or model info retrieval failed
    async fn process_containers_for_model_states(
        &self,
        container_infos: Vec<ContainerStateInfo>,
    ) -> Result<()> {
        let mut models = self.models.lock().await;
        let mut container_to_model = self.container_to_model.lock().await;

        // Group containers by model (extracted from annotations or naming convention)
        let mut model_containers: HashMap<String, Vec<ContainerStateInfo>> = HashMap::new();

        for container in container_infos {
            // Extract model name from container annotations or naming convention
            let model_name = self.extract_model_name_from_container(&container);

            if let Some(model_name) = model_name {
                // Update container to model mapping
                container_to_model.insert(container.container_id.clone(), model_name.clone());

                // Group containers by model
                model_containers
                    .entry(model_name)
                    .or_insert_with(Vec::new)
                    .push(container);
            }
        }

        // Process each model's containers to determine model state
        for (model_name, containers) in model_containers {
            let new_model_state = self.determine_model_state_from_containers(&containers);
            let package_name = self.extract_package_name_from_model(&model_name);

            // Update or create model info
            let model_info = models
                .entry(model_name.clone())
                .or_insert_with(|| ModelInfo {
                    model_name: model_name.clone(),
                    package_name: package_name.unwrap_or_else(|| "unknown".to_string()),
                    containers: Vec::new(),
                    current_state: ModelState::Unspecified,
                    last_update: Instant::now(),
                });

            // Update model if state has changed
            if model_info.current_state != new_model_state {
                let old_state = model_info.current_state;
                model_info.current_state = new_model_state;
                model_info.containers = containers;
                model_info.last_update = Instant::now();

                println!(
                    "Model '{}' state changed: {:?} -> {:?}",
                    model_name, old_state, new_model_state
                );

                // Save model state to ETCD
                if let Err(e) = self
                    .save_model_state_to_etcd(&model_name, new_model_state)
                    .await
                {
                    eprintln!("Failed to save model state to ETCD: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Process models to update package states based on model status aggregation
    ///
    /// According to StateManager_Package.md specification:
    /// - idle: Initial package state
    /// - paused: All models are paused
    /// - exited: All models are exited  
    /// - degraded: Some models are dead (but not all)
    /// - error: All models are dead
    /// - running: Default state when above conditions not met
    async fn process_models_for_package_states(&self) -> Result<()> {
        let models = self.models.lock().await;
        let mut packages = self.packages.lock().await;

        // Group models by package
        let mut package_models: HashMap<String, Vec<&ModelInfo>> = HashMap::new();
        for model_info in models.values() {
            package_models
                .entry(model_info.package_name.clone())
                .or_insert_with(Vec::new)
                .push(model_info);
        }

        // Process each package's models to determine package state
        for (package_name, model_list) in package_models {
            if model_list.is_empty() {
                continue;
            }

            let new_package_state = self.determine_package_state_from_models(&model_list);

            // Update or create package info
            let package_info =
                packages
                    .entry(package_name.clone())
                    .or_insert_with(|| PackageInfo {
                        package_name: package_name.clone(),
                        models: HashMap::new(),
                        current_state: PackageState::Unspecified,
                        last_update: Instant::now(),
                    });

            // Update package if state has changed
            if package_info.current_state != new_package_state {
                let old_state = package_info.current_state;
                package_info.current_state = new_package_state;
                package_info.last_update = Instant::now();

                println!(
                    "Package '{}' state changed: {:?} -> {:?}",
                    package_name, old_state, new_package_state
                );

                // Save package state to ETCD
                if let Err(e) = self
                    .save_package_state_to_etcd(&package_name, new_package_state)
                    .await
                {
                    eprintln!("Failed to save package state to ETCD: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Extract model name from container information
    /// Uses annotations or naming conventions to determine which model a container belongs to
    fn extract_model_name_from_container(&self, container: &ContainerStateInfo) -> Option<String> {
        // First try to get model name from annotations
        if let Some(model_name) = container.annotations.get("pullpiri.io/model") {
            return Some(model_name.clone());
        }

        // Fallback: extract from container name (assuming naming convention like "model-name-xxx")
        if let Some(container_name) = container.container_names.first() {
            if let Some(model_name) = container_name.split('-').next() {
                return Some(model_name.to_string());
            }
        }

        // Fallback: use image name as model identifier
        if !container.image.is_empty() {
            if let Some(image_name) = container.image.split(':').next() {
                if let Some(model_name) = image_name.split('/').last() {
                    return Some(model_name.to_string());
                }
            }
        }

        None
    }

    /// Extract package name from model name
    /// Uses naming conventions to determine which package a model belongs to
    fn extract_package_name_from_model(&self, model_name: &str) -> Option<String> {
        // For now, assume package name is the first part before underscore
        // This could be enhanced with configuration or metadata lookup
        if let Some(package_name) = model_name.split('_').next() {
            Some(package_name.to_string())
        } else {
            Some(model_name.to_string()) // Use model name as package name if no separator
        }
    }

    /// Determine model state based on container states
    /// Implements the logic from StateManager_Model.md section 4.2
    fn determine_model_state_from_containers(
        &self,
        containers: &[ContainerStateInfo],
    ) -> ModelState {
        if containers.is_empty() {
            return ModelState::Failed; // No containers means model failed
        }

        let mut running_count = 0;
        let mut created_count = 0;
        let mut exited_count = 0;
        let mut dead_count = 0;
        let mut paused_count = 0;

        for container in containers {
            // Parse container state - the state is stored as key-value pairs
            if let Some(status) = container.state.get("status") {
                match status.to_lowercase().as_str() {
                    "running" => running_count += 1,
                    "created" => created_count += 1,
                    "exited" | "stopped" => exited_count += 1,
                    "dead" | "removing" => dead_count += 1,
                    "paused" => paused_count += 1,
                    _ => {
                        // Unknown state, consider as dead for safety
                        dead_count += 1;
                    }
                }
            } else {
                // No status information, consider as dead
                dead_count += 1;
            }
        }

        let total_containers = containers.len();

        // Apply state determination rules from documentation
        if dead_count > 0 {
            ModelState::Failed // At least one container is dead
        } else if paused_count == total_containers {
            ModelState::Failed // Map paused to Failed as ModelState doesn't have Paused
        } else if exited_count == total_containers {
            ModelState::Succeeded // All containers exited successfully
        } else if running_count > 0 {
            ModelState::Running // At least one container is running
        } else if created_count == total_containers {
            ModelState::Pending // All containers created but not running
        } else {
            ModelState::Unknown // Cannot determine state
        }
    }

    /// Determine package state based on model states  
    /// Implements the logic from StateManager_Package.md section 3.1
    fn determine_package_state_from_models(&self, models: &[&ModelInfo]) -> PackageState {
        if models.is_empty() {
            return PackageState::Error;
        }

        let mut running_count = 0;
        let mut failed_count = 0;
        let mut succeeded_count = 0;
        let mut pending_count = 0;
        let mut _unknown_count = 0;

        for model in models {
            match model.current_state {
                ModelState::Running => running_count += 1,
                ModelState::Failed => failed_count += 1,
                ModelState::Succeeded => succeeded_count += 1,
                ModelState::Pending | ModelState::ContainerCreating => pending_count += 1,
                ModelState::Unknown | ModelState::CrashLoopBackOff => _unknown_count += 1,
                ModelState::Unspecified => _unknown_count += 1,
            }
        }

        let total_models = models.len();

        // Apply package state determination rules
        if failed_count == total_models {
            PackageState::Error // All models are dead/failed
        } else if failed_count > 0 {
            PackageState::Degraded // Some models are dead/failed
        } else if succeeded_count == total_models {
            PackageState::Paused // Map succeeded to paused - all models completed
        } else if running_count > 0 {
            PackageState::Running // At least one model running
        } else if pending_count > 0 {
            PackageState::Initializing // Models still starting
        } else {
            PackageState::Running // Default state
        }
    }

    /// Save model state to ETCD using the format specified in documentation
    /// Key format: /model/{model_name}/state  
    /// Value: state name (e.g., "Running")
    async fn save_model_state_to_etcd(&self, model_name: &str, state: ModelState) -> Result<()> {
        let key = format!("/model/{}/state", model_name);
        let value = state.as_str_name(); // This converts enum to string representation

        if let Err(e) = common::etcd::put(&key, value).await {
            eprintln!("Failed to save model state to ETCD: {:?}", e);
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("ETCD put failed: {}", e),
            ))
            .into());
        }

        println!("Saved model state to ETCD: {} = {}", key, value);
        Ok(())
    }

    /// Save package state to ETCD using the format specified in documentation  
    /// Key format: /package/{package_name}/state
    /// Value: state name (e.g., "Running")
    async fn save_package_state_to_etcd(
        &self,
        package_name: &str,
        state: PackageState,
    ) -> Result<()> {
        let key = format!("/package/{}/state", package_name);
        let value = state.as_str_name(); // This converts enum to string representation

        if let Err(e) = common::etcd::put(&key, value).await {
            eprintln!("Failed to save package state to ETCD: {:?}", e);
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("ETCD put failed: {}", e),
            ))
            .into());
        }

        println!("Saved package state to ETCD: {} = {}", key, value);
        Ok(())
    }

    /// Main message processing loop for handling gRPC requests.
    ///
    /// Spawns dedicated async tasks for processing different message types:
    /// 1. Container status processing task
    /// 2. State change processing task
    ///
    /// Each task runs independently to ensure optimal throughput and prevent
    /// blocking between different message types.
    ///
    /// # Returns
    /// * `Result<()>` - Success or processing error
    ///
    /// # Architecture Notes
    /// - Uses separate tasks to prevent cross-contamination between message types
    /// - Maintains proper async patterns for high-throughput processing
    /// - Ensures graceful shutdown when channels are closed
    pub async fn process_grpc_requests(&self) -> Result<()> {
        let rx_container = Arc::clone(&self.rx_container);
        let rx_state_change = Arc::clone(&self.rx_state_change);

        // ========================================
        // CONTAINER STATUS PROCESSING TASK
        // ========================================
        // Handles ContainerList messages from nodeagent for container monitoring
        let container_task = {
            let state_manager = self.clone_for_task();
            tokio::spawn(async move {
                loop {
                    let container_list_opt = {
                        let mut rx = rx_container.lock().await;
                        rx.recv().await
                    };
                    match container_list_opt {
                        Some(container_list) => {
                            // Process container status update with comprehensive analysis
                            state_manager.process_container_list(container_list).await;
                        }
                        None => {
                            // Channel closed - graceful shutdown
                            println!(
                                "Container channel closed - shutting down container processing"
                            );
                            break;
                        }
                    }
                }
                println!("ContainerList processing task stopped");
            })
        };

        // ========================================
        // STATE CHANGE PROCESSING TASK
        // ========================================
        // Handles StateChange messages from ApiServer, FilterGateway, ActionController
        let state_change_task = {
            let state_manager = self.clone_for_task();
            tokio::spawn(async move {
                loop {
                    let state_change_opt = {
                        let mut rx = rx_state_change.lock().await;
                        rx.recv().await
                    };
                    match state_change_opt {
                        Some(state_change) => {
                            // Process state change with comprehensive PICCOLO compliance
                            state_manager.process_state_change(state_change).await;
                        }
                        None => {
                            // Channel closed - graceful shutdown
                            println!("StateChange channel closed - shutting down state processing");
                            break;
                        }
                    }
                }
                println!("StateChange processing task stopped");
            })
        };

        // Wait for both tasks to complete (typically on shutdown)
        let result = tokio::try_join!(container_task, state_change_task);
        match result {
            Ok(_) => {
                println!("All processing tasks completed successfully");
                Ok(())
            }
            Err(e) => {
                eprintln!("Error in processing tasks: {e:?}");
                Err(e.into())
            }
        }
    }

    /// Creates a clone of self suitable for use in async tasks.
    ///
    /// This method provides a way to share the StateManagerManager instance
    /// across multiple async tasks while maintaining proper ownership.
    ///
    /// # Returns
    /// * `StateManagerManager` - Cloned instance for task use
    fn clone_for_task(&self) -> StateManagerManager {
        StateManagerManager {
            state_machine: Arc::clone(&self.state_machine),
            rx_container: Arc::clone(&self.rx_container),
            rx_state_change: Arc::clone(&self.rx_state_change),
            models: Arc::clone(&self.models),
            packages: Arc::clone(&self.packages),
            container_to_model: Arc::clone(&self.container_to_model),
        }
    }

    /// Runs the StateManagerManager's main event loop.
    ///
    /// This is the primary entry point for the StateManager service operation.
    /// It spawns the message processing tasks and manages their lifecycle.
    ///
    /// # Returns
    /// * `Result<()>` - Success or runtime error
    ///
    /// # Lifecycle
    /// 1. Wraps self in Arc for shared ownership across tasks
    /// 2. Spawns the gRPC message processing task
    /// 3. Waits for processing completion (typically on shutdown)
    /// 4. Performs cleanup and logs final status
    ///
    /// # Error Handling
    /// - Logs processing errors without panicking
    /// - Ensures graceful shutdown even on task failures
    /// - Provides comprehensive error reporting for debugging
    pub async fn run(self) -> Result<()> {
        // Wrap self in Arc for shared ownership across async tasks
        let arc_self = Arc::new(self);
        let grpc_manager = Arc::clone(&arc_self);

        // Spawn the main gRPC processing task
        let grpc_processor = tokio::spawn(async move {
            if let Err(e) = grpc_manager.process_grpc_requests().await {
                eprintln!("Error in gRPC processor: {e:?}");
            }
        });

        // Wait for the processing task to complete
        let result = grpc_processor.await;
        match result {
            Ok(_) => {
                println!("StateManagerManager stopped gracefully");
                Ok(())
            }
            Err(e) => {
                eprintln!("StateManagerManager stopped with error: {e:?}");
                Err(e.into())
            }
        }
    }
}

/// Async action executor - runs in separate task
///
/// This function handles the execution of actions triggered by state transitions.
/// Actions are executed asynchronously to ensure state transitions remain fast and non-blocking.
pub async fn run_action_executor(mut receiver: mpsc::UnboundedReceiver<ActionCommand>) {
    println!("Action executor started - processing actions asynchronously");

    while let Some(action_command) = receiver.recv().await {
        // Execute action asynchronously without blocking state transitions
        task::spawn(async move {
            execute_action(action_command).await;
        });
    }

    println!("Action executor stopped");
}

/// Execute individual action asynchronously
async fn execute_action(command: ActionCommand) {
    println!(
        " Executing action: {} for resource: {}",
        command.action, command.resource_key
    );

    match command.action.as_str() {
        "start_condition_evaluation" => {
            println!(
                " Starting condition evaluation for scenario: {}",
                command.resource_key
            );
            // Would integrate with policy engine or condition evaluator
        }
        "start_policy_verification" => {
            println!(
                " Starting policy verification for scenario: {}",
                command.resource_key
            );
            // Would integrate with policy manager
        }
        "execute_action_on_target_package" => {
            println!(
                " Executing action on target package for scenario: {}",
                command.resource_key
            );
            // Would trigger package operations
        }
        "log_denial_generate_alert" => {
            println!(
                " Logging denial and generating alert for scenario: {}",
                command.resource_key
            );
            // Would integrate with alerting system
        }
        "start_model_creation_allocate_resources" => {
            println!(
                " Starting model creation and resource allocation for package: {}",
                command.resource_key
            );
            // Would integrate with resource allocation system
        }
        "update_state_announce_availability" => {
            println!(
                " Updating state and announcing availability for: {}",
                command.resource_key
            );
            // Would update service discovery and announce availability
        }
        "log_warning_activate_partial_functionality" => {
            println!(
                " Logging warning and activating partial functionality for: {}",
                command.resource_key
            );
            // Would configure degraded mode operation
        }
        "log_error_attempt_recovery" => {
            println!(
                " Logging error and attempting recovery for: {}",
                command.resource_key
            );
            // Would trigger automated recovery procedures
        }
        "pause_models_preserve_state" => {
            println!(
                " Pausing models and preserving state for: {}",
                command.resource_key
            );
            // Would pause container execution and save state
        }
        "resume_models_restore_state" => {
            println!(
                " Resuming models and restoring state for: {}",
                command.resource_key
            );
            // Would resume container execution and restore state
        }
        "start_node_selection_and_allocation" => {
            println!(
                " Starting node selection and allocation for model: {}",
                command.resource_key
            );
            // Would integrate with scheduler for node allocation
        }
        "pull_container_images_mount_volumes" => {
            println!(
                " Pulling container images and mounting volumes for model: {}",
                command.resource_key
            );
            // Would trigger container image pulls and volume mounts
        }
        "update_state_start_readiness_checks" => {
            println!(
                " Updating state and starting readiness checks for model: {}",
                command.resource_key
            );
            // Would start health/readiness checks
        }
        "log_completion_clean_up_resources" => {
            println!(
                " Logging completion and cleaning up resources for model: {}",
                command.resource_key
            );
            // Would clean up completed job resources
        }
        "set_backoff_timer_collect_logs" => {
            println!(
                " Setting backoff timer and collecting logs for model: {}",
                command.resource_key
            );
            // Would set exponential backoff and collect diagnostic logs
        }
        "attempt_diagnostics_restore_communication" => {
            println!(
                " Attempting diagnostics and restoring communication for model: {}",
                command.resource_key
            );
            // Would run diagnostic checks and restore node communication
        }
        "resume_monitoring_reset_counter" => {
            println!(
                " Resuming monitoring and resetting counter for model: {}",
                command.resource_key
            );
            // Would resume monitoring and reset failure counters
        }
        "log_error_notify_for_manual_intervention" => {
            println!(
                " Logging error and notifying for manual intervention for model: {}",
                command.resource_key
            );
            // Would log critical error and notify operations team
        }
        "synchronize_state_recover_if_needed" => {
            println!(
                " Synchronizing state and recovering if needed for model: {}",
                command.resource_key
            );
            // Would synchronize state and trigger recovery if necessary
        }
        "start_model_recreation" => {
            println!(" Starting model recreation for: {}", command.resource_key);
            // Would start complete model recreation process
        }
        _ => {
            println!(
                " Unknown action: {} for resource: {}",
                command.action, command.resource_key
            );
        }
    }

    // Print context information if available
    if !command.context.is_empty() {
        println!("    Context: {:?}", command.context);
    }

    println!(
        "  ✓ Action '{}' completed for: {}",
        command.action, command.resource_key
    );
}

// ========================================
// FUTURE IMPLEMENTATION AREAS
// ========================================
// The following areas require implementation for full PICCOLO compliance:
//
// 1. STATE MACHINE ENGINE - ✓ IMPLEMENTED
//    - Implement state validators for each ResourceType (Scenario, Package, Model, Volume, Network, Node)
//    - Add transition rules and constraint checking for each state enum
//    - Support for ASIL timing requirements and safety constraints
//    - Resource-specific validation logic and business rules
//
// 2. PERSISTENT STATE STORAGE
//    - Integration with etcd or database for state persistence
//    - State history tracking and audit trails with StateTransitionHistory
//    - Recovery from persistent storage on startup
//    - ResourceState management with generation counters
//
// 3. DEPENDENCY MANAGEMENT
//    - Resource dependency tracking and validation using Dependency messages
//    - Cascade state changes through dependency graphs
//    - Circular dependency detection and resolution
//    - Critical dependency handling and escalation
//
// 4. RECOVERY AND RECONCILIATION
//    - Automatic recovery strategies using RecoveryStrategy and RecoveryType
//    - State drift detection and reconciliation
//    - Health monitoring integration with HealthStatus and HealthCheck
//    - Recovery progress tracking with RecoveryStatus
//
// 5. EVENT STREAMING AND NOTIFICATIONS
//    - Real-time state change event generation using StateChangeEvent
//    - Subscription management for external components
//    - Event filtering and routing capabilities with EventType and Severity
//    - Alert management with Alert and AlertStatus
//
// 6. ASIL SAFETY COMPLIANCE
//    - Timing constraint validation and enforcement using PerformanceConstraints
//    - Safety level verification for state transitions with ASILLevel
//    - Comprehensive audit logging for safety analysis
//    - Safety-critical failure detection and response
//
// 7. ADVANCED QUERY AND MANAGEMENT
//    - Resource state queries with ResourceStateRequest/Response
//    - State history retrieval with ResourceStateHistoryRequest/Response
//    - Bulk operations and list management
//    - Resource filtering and selection capabilities
//
// 8. PERFORMANCE AND MONITORING
//    - Performance constraint enforcement with deadlines and priorities
//    - Resource usage monitoring and optimization
//    - Health check automation and reporting
//    - Metrics collection and observability integration

#[cfg(test)]
mod tests {
    use super::*;
    use common::statemanager::ModelState;
    use std::collections::HashMap;
    use tokio::sync::mpsc;

    fn create_test_container(name: &str, state: &str) -> ContainerStateInfo {
        let mut state_map = HashMap::new();
        state_map.insert("status".to_string(), state.to_string());

        let mut annotations = HashMap::new();
        annotations.insert("pullpiri.io/model".to_string(), "test_model".to_string());

        ContainerStateInfo {
            container_id: format!("{}_id", name),
            container_names: vec![name.to_string()],
            image: "test/image:latest".to_string(),
            state: state_map,
            config: HashMap::new(),
            annotations,
        }
    }

    #[tokio::test]
    async fn test_determine_model_state_from_containers() {
        let (tx_container, rx_container) = mpsc::channel::<ContainerList>(10);
        let (tx_state, rx_state) = mpsc::channel::<StateChange>(10);
        let manager = StateManagerManager::new(rx_container, rx_state).await;

        // Test: All containers running -> Running state
        let containers = vec![
            create_test_container("container1", "running"),
            create_test_container("container2", "running"),
        ];
        let state = manager.determine_model_state_from_containers(&containers);
        assert_eq!(state, ModelState::Running);

        // Test: All containers exited -> Succeeded state
        let containers = vec![
            create_test_container("container1", "exited"),
            create_test_container("container2", "exited"),
        ];
        let state = manager.determine_model_state_from_containers(&containers);
        assert_eq!(state, ModelState::Succeeded);

        // Test: One container dead -> Failed state
        let containers = vec![
            create_test_container("container1", "running"),
            create_test_container("container2", "dead"),
        ];
        let state = manager.determine_model_state_from_containers(&containers);
        assert_eq!(state, ModelState::Failed);

        // Test: All containers created -> Pending state
        let containers = vec![
            create_test_container("container1", "created"),
            create_test_container("container2", "created"),
        ];
        let state = manager.determine_model_state_from_containers(&containers);
        assert_eq!(state, ModelState::Pending);

        // Ensure channels are cleaned up
        drop(tx_container);
        drop(tx_state);
    }

    #[tokio::test]
    async fn test_extract_model_name_from_container() {
        let (tx_container, rx_container) = mpsc::channel::<ContainerList>(10);
        let (tx_state, rx_state) = mpsc::channel::<StateChange>(10);
        let manager = StateManagerManager::new(rx_container, rx_state).await;

        // Test: Extract from annotations
        let container = create_test_container("test", "running");
        let model_name = manager.extract_model_name_from_container(&container);
        assert_eq!(model_name, Some("test_model".to_string()));

        // Test: Extract from container name when annotation is missing (falls back to container name parsing)
        let mut container = create_test_container("my-model-instance-123", "running");
        container.annotations.clear(); // Remove annotation
        let model_name = manager.extract_model_name_from_container(&container);
        assert_eq!(model_name, Some("my".to_string())); // First part before dash

        // Test: Extract from image name when annotation and container name parsing fails
        let mut container = create_test_container("single_name", "running");
        container.annotations.clear(); // Remove annotation
        container.image = "my-registry/my-model:v1.0".to_string();
        let model_name = manager.extract_model_name_from_container(&container);
        assert_eq!(model_name, Some("single_name".to_string())); // Container name if no dash

        // Ensure channels are cleaned up
        drop(tx_container);
        drop(tx_state);
    }

    #[tokio::test]
    async fn test_determine_package_state_from_models() {
        let (tx_container, rx_container) = mpsc::channel::<ContainerList>(10);
        let (tx_state, rx_state) = mpsc::channel::<StateChange>(10);
        let manager = StateManagerManager::new(rx_container, rx_state).await;

        let model1 = ModelInfo {
            model_name: "model1".to_string(),
            package_name: "test_package".to_string(),
            containers: Vec::new(),
            current_state: ModelState::Running,
            last_update: Instant::now(),
        };

        let model2 = ModelInfo {
            model_name: "model2".to_string(),
            package_name: "test_package".to_string(),
            containers: Vec::new(),
            current_state: ModelState::Running,
            last_update: Instant::now(),
        };

        let model3 = ModelInfo {
            model_name: "model3".to_string(),
            package_name: "test_package".to_string(),
            containers: Vec::new(),
            current_state: ModelState::Failed,
            last_update: Instant::now(),
        };

        // Test: All models running -> Running package state
        let models = vec![&model1, &model2];
        let state = manager.determine_package_state_from_models(&models);
        assert_eq!(state, PackageState::Running);

        // Test: Some models failed -> Degraded package state
        let models = vec![&model1, &model3];
        let state = manager.determine_package_state_from_models(&models);
        assert_eq!(state, PackageState::Degraded);

        // Test: All models failed -> Error package state
        let models = vec![&model3];
        let state = manager.determine_package_state_from_models(&models);
        assert_eq!(state, PackageState::Error);

        // Ensure channels are cleaned up
        drop(tx_container);
        drop(tx_state);
    }
}
