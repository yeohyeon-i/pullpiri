/*
 * SPDX-FileCopyrightText: Copyright 2024 LG Electronics Inc.
 * SPDX-License-Identifier: Apache-2.0
 */

//! StateManager gRPC client for sending state change messages from PolicyManager.
//!
//! This module provides a client interface for the PolicyManager to communicate with
//! the StateManager service via gRPC. It manages connection lifecycle, handles
//! request routing, and provides ASIL-compliant state change messaging capabilities.
//!
//! The PolicyManager uses this client to report policy verification results,
//! authorization decisions, and scenario state transitions to the StateManager
//! for proper resource state tracking and audit trails.

use common::statemanager::{
    connect_server, state_manager_connection_client::StateManagerConnectionClient, ResourceType,
    StateChange, StateChangeResponse,
};
use tonic::{Request, Status};

/// StateManager gRPC client for PolicyManager component.
///
/// This client manages the gRPC connection to the StateManager service and provides
/// methods for sending state change requests from PolicyManager operations. It implements
/// lazy connection establishment to optimize resource usage and provides automatic
/// reconnection capabilities.
///
/// # Connection Management
/// - Establishes connections on first use (lazy initialization)
/// - Reuses existing connections for multiple requests
/// - Handles connection failures gracefully with proper error reporting
/// - Provides thread-safe access through cloning capability
///
/// # PolicyManager Integration
/// - Reports policy verification results (allow/deny decisions)
/// - Notifies of authorization state transitions
/// - Provides compliance and audit information
/// - Handles resource access policy enforcement outcomes
///
/// # PICCOLO Compliance
/// - Supports ASIL safety levels from QM to ASIL-D
/// - Maintains nanosecond precision timestamps for timing verification
/// - Provides comprehensive tracking through transition IDs
/// - Includes context information for safety analysis and audit trails
/// - Enforces security and access control policies
#[derive(Clone)]
pub struct StateManagerSender {
    /// Cached gRPC client connection to the StateManager service.
    ///
    /// This connection is established lazily on the first request and reused
    /// for subsequent requests to optimize performance. Set to None initially
    /// and populated when ensure_connected() is called.
    client: Option<StateManagerConnectionClient<tonic::transport::Channel>>,
}

impl Default for StateManagerSender {
    /// Creates a new StateManagerSender with default PolicyManager settings.
    ///
    /// # Returns
    /// * `Self` - New StateManagerSender instance with no active connection
    fn default() -> Self {
        Self::new()
    }
}

impl StateManagerSender {
    /// Creates a new StateManagerSender instance for PolicyManager.
    ///
    /// The connection to the StateManager is established lazily on the first request
    /// to optimize startup time and resource usage. This allows the PolicyManager to
    /// initialize quickly even if the StateManager is temporarily unavailable.
    ///
    /// # Returns
    /// * `Self` - New StateManagerSender instance ready for use
    pub fn new() -> Self {
        Self { client: None }
    }

    /// Ensures a gRPC connection to the StateManager exists and is ready for use.
    ///
    /// This method implements lazy connection establishment by checking if a connection
    /// already exists and creating one if necessary. It uses the common::statemanager
    /// configuration to determine the StateManager's network location.
    ///
    /// # Connection Process
    /// 1. Check if a connection already exists
    /// 2. If not, attempt to establish a new gRPC connection
    /// 3. Store the connection for reuse in subsequent requests
    /// 4. Return success or detailed error information
    ///
    /// # Returns
    /// * `Result<(), Status>` - Success if connection is available, error otherwise
    ///
    /// # Errors
    /// * `Status::unknown` - Connection establishment failed (network, service unavailable, etc.)
    async fn ensure_connected(&mut self) -> Result<(), Status> {
        if self.client.is_none() {
            match StateManagerConnectionClient::connect(connect_server()).await {
                Ok(client) => {
                    self.client = Some(client);
                }
                Err(e) => {
                    return Err(Status::unknown(format!(
                        "Failed to connect to StateManager: {}",
                        e
                    )));
                }
            }
        }
        Ok(())
    }

    /// Sends a state change message to the StateManager service.
    ///
    /// This is the primary method for communicating policy-driven state transitions from
    /// the PolicyManager to the StateManager. It handles the complete request lifecycle
    /// including connection management, request transmission, and response processing.
    ///
    /// # Request Processing Flow
    /// 1. Ensure gRPC connection is established and ready
    /// 2. Create gRPC request wrapper with StateChange message
    /// 3. Send request to StateManager via gRPC
    /// 4. Receive and return StateChangeResponse with tracking information
    ///
    /// # Arguments
    /// * `state_change` - Complete StateChange message containing:
    ///   - Resource identification (type enum and name)
    ///   - State transition details (current → target state)
    ///   - Tracking and context information (transition_id, timestamps, source)
    ///
    /// # Returns
    /// * `Result<tonic::Response<StateChangeResponse>, Status>` - Response containing:
    ///   - Descriptive message
    ///   - Original transition_id for tracking
    ///   - Processing timestamp with nanosecond precision
    ///   - Error codes and details if applicable
    ///
    /// # Errors
    /// * `Status::unknown` - Connection failure or client not connected
    /// * `Status::unavailable` - StateManager service unavailable
    /// * `Status::invalid_argument` - Malformed StateChange message
    /// * `Status::deadline_exceeded` - Request timeout (ASIL timing violation)
    ///
    /// # PICCOLO Compliance Notes
    /// - Preserves nanosecond precision timestamps for timing verification
    /// - Maintains transition_id for complete audit trail
    /// - Supports ResourceType enum for type-safe resource identification
    /// - Provides detailed error information for safety analysis
    /// - Enforces security and access control policies
    pub async fn send_state_change(
        &mut self,
        state_change: StateChange,
    ) -> Result<tonic::Response<StateChangeResponse>, Status> {
        // Ensure we have an active gRPC connection before sending
        self.ensure_connected().await?;

        if let Some(client) = &mut self.client {
            // Send the state change message via gRPC
            client.send_state_change(Request::new(state_change)).await
        } else {
            // This should never happen due to ensure_connected, but provide safety fallback
            Err(Status::unknown("Client not connected"))
        }
    }

    /// Reports policy verification result to StateManager.
    ///
    /// This convenience method creates and sends a StateChange message indicating
    /// the result of a policy verification decision (allow/deny).
    ///
    /// # Arguments
    /// * `scenario_name` - Name of the scenario being policy-checked
    /// * `policy_decision` - Result of policy verification ("allowed", "denied")
    /// * `policy_id` - Policy verification identifier
    ///
    /// # Returns
    /// * `Result<tonic::Response<StateChangeResponse>, Status>` - StateManager response
    pub async fn report_policy_verification(
        &mut self,
        scenario_name: &str,
        policy_decision: &str,
        policy_id: &str,
    ) -> Result<tonic::Response<StateChangeResponse>, Status> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i64;

        let state_change = StateChange {
            resource_type: ResourceType::Scenario as i32,
            resource_name: scenario_name.to_string(),
            current_state: "satisfied".to_string(),
            target_state: policy_decision.to_string(),
            transition_id: format!("policy-verification-{}", policy_id),
            timestamp_ns: timestamp,
            source: "policymanager".to_string(),
        };

        self.send_state_change(state_change).await
    }

    /// Reports scenario state change to StateManager.
    ///
    /// General method for reporting scenario state transitions from PolicyManager.
    ///
    /// # Arguments
    /// * `scenario_name` - Name of the scenario
    /// * `current_state` - Current scenario state
    /// * `target_state` - Target scenario state
    /// * `transition_id` - Unique transition identifier
    ///
    /// # Returns
    /// * `Result<tonic::Response<StateChangeResponse>, Status>` - StateManager response
    pub async fn report_scenario_state_change(
        &mut self,
        scenario_name: &str,
        current_state: &str,
        target_state: &str,
        transition_id: &str,
    ) -> Result<tonic::Response<StateChangeResponse>, Status> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i64;

        let state_change = StateChange {
            resource_type: ResourceType::Scenario as i32,
            resource_name: scenario_name.to_string(),
            current_state: current_state.to_string(),
            target_state: target_state.to_string(),
            transition_id: transition_id.to_string(),
            timestamp_ns: timestamp,
            source: "policymanager".to_string(),
        };

        self.send_state_change(state_change).await
    }

    /// Reports policy approval to StateManager (satisfied → allowed transition).
    ///
    /// Called when PolicyManager allows a scenario to proceed.
    ///
    /// # Arguments
    /// * `scenario_name` - Name of the scenario
    /// * `policy_id` - Policy verification identifier
    ///
    /// # Returns
    /// * `Result<tonic::Response<StateChangeResponse>, Status>` - StateManager response
    pub async fn report_policy_approval(
        &mut self,
        scenario_name: &str,
        policy_id: &str,
    ) -> Result<tonic::Response<StateChangeResponse>, Status> {
        let transition_id = format!("policy-approval-{}", policy_id);
        self.report_scenario_state_change(scenario_name, "satisfied", "allowed", &transition_id)
            .await
    }

    /// Reports policy denial to StateManager (satisfied → denied transition).
    ///
    /// Called when PolicyManager denies a scenario from proceeding.
    ///
    /// # Arguments
    /// * `scenario_name` - Name of the scenario
    /// * `policy_id` - Policy verification identifier
    ///
    /// # Returns
    /// * `Result<tonic::Response<StateChangeResponse>, Status>` - StateManager response
    pub async fn report_policy_denial(
        &mut self,
        scenario_name: &str,
        policy_id: &str,
    ) -> Result<tonic::Response<StateChangeResponse>, Status> {
        let transition_id = format!("policy-denial-{}", policy_id);
        self.report_scenario_state_change(scenario_name, "satisfied", "denied", &transition_id)
            .await
    }
}

// ========================================
// UNIT TESTS
// ========================================
// Comprehensive test suite for PolicyManager StateManagerSender functionality

#[cfg(test)]
mod tests {
    use super::*;
    use common::statemanager::{ResourceType, StateChange};
    use std::time::Duration;

    /// Tests successful state change message transmission to StateManager.
    ///
    /// This test verifies the complete end-to-end communication flow between
    /// the PolicyManager and StateManager, including connection establishment,
    /// message transmission, and response processing.
    #[tokio::test]
    async fn test_send_state_change_success() {
        // Add startup delay to ensure StateManager service is ready
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut sender = StateManagerSender::default();

        // Create unique timestamp for this test run
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i64;

        // Create StateChange message for policy verification
        let state_change = StateChange {
            resource_type: ResourceType::Scenario as i32,
            resource_name: "emergency-brake-scenario".to_string(),
            current_state: "satisfied".to_string(),
            target_state: "allowed".to_string(),
            transition_id: format!("policy-verification-{}", timestamp),
            timestamp_ns: timestamp,
            source: "policymanager".to_string(),
        };

        // Send the message and verify successful response
        let result = sender.send_state_change(state_change).await;
        assert!(result.is_ok(), "StateChange request should succeed");

        if let Ok(response) = result {
            let state_response = response.into_inner();
            assert!(
                !state_response.message.is_empty(),
                "Response should include a message"
            );
            assert!(
                !state_response.transition_id.is_empty(),
                "Response should include transition ID"
            );
            assert!(
                state_response.timestamp_ns > 0,
                "Response should include processing timestamp"
            );

            println!("PolicyManager StateChange test completed successfully:");
            println!("  Message: {}", state_response.message);
            println!("  Transition ID: {}", state_response.transition_id);
            println!("  Processing time: {} ns", state_response.timestamp_ns);
        }
    }

    /// Tests policy verification reporting convenience method.
    ///
    /// This test verifies that the report_policy_verification method correctly
    /// creates and sends StateChange messages for policy verification decisions.
    #[tokio::test]
    async fn test_report_policy_verification() {
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut sender = StateManagerSender::new();

        let result = sender
            .report_policy_verification("security-scenario", "allowed", "policy-123")
            .await;

        assert!(result.is_ok(), "Policy verification report should succeed");

        if let Ok(response) = result {
            let state_response = response.into_inner();
            assert!(state_response
                .transition_id
                .contains("policy-verification-policy-123"));
            println!(
                "Policy verification report test completed: {}",
                state_response.message
            );
        }
    }

    /// Tests policy approval reporting convenience method.
    #[tokio::test]
    async fn test_report_policy_approval() {
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut sender = StateManagerSender::new();

        let result = sender
            .report_policy_approval("emergency-scenario", "policy-456")
            .await;

        assert!(result.is_ok(), "Policy approval report should succeed");

        if let Ok(response) = result {
            let state_response = response.into_inner();
            assert!(state_response
                .transition_id
                .contains("policy-approval-policy-456"));
            println!(
                "Policy approval report test completed: {}",
                state_response.message
            );
        }
    }

    /// Tests policy denial reporting convenience method.
    #[tokio::test]
    async fn test_report_policy_denial() {
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut sender = StateManagerSender::new();

        let result = sender
            .report_policy_denial("restricted-scenario", "policy-789")
            .await;

        assert!(result.is_ok(), "Policy denial report should succeed");

        if let Ok(response) = result {
            let state_response = response.into_inner();
            assert!(state_response
                .transition_id
                .contains("policy-denial-policy-789"));
            println!(
                "Policy denial report test completed: {}",
                state_response.message
            );
        }
    }
}
