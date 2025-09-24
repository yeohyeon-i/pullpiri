/*
 * SPDX-FileCopyrightText: Copyright 2024 LG Electronics Inc.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::grpc::sender::statemanager::StateManagerSender;
use common::policymanager::policy_manager_connection_server::PolicyManagerConnection;
use common::policymanager::{CheckPolicyRequest, CheckPolicyResponse};
use tonic::Response;

pub struct PolicyManagerGrpcServer {
    state_manager_sender: StateManagerSender,
}

impl PolicyManagerGrpcServer {
    pub fn new() -> Self {
        Self {
            state_manager_sender: StateManagerSender::new(),
        }
    }
}

#[tonic::async_trait]
impl PolicyManagerConnection for PolicyManagerGrpcServer {
    async fn check_policy(
        &self,
        request: tonic::Request<CheckPolicyRequest>,
    ) -> Result<tonic::Response<CheckPolicyResponse>, tonic::Status> {
        let req = request.into_inner();
        let scenario_name = req.scenario_name; // Renamed for clarity

        // Simulate internal logic
        let (status, desc) = if scenario_name.is_empty() {
            (1, "Scenario name cannot be empty".to_string())
        } else if scenario_name == "test_scenario" {
            (0, "Policy check passed".to_string())
        } else {
            (
                1,
                format!("Policy check failed for scenario: {}", scenario_name),
            )
        };

        // Report policy verification result to StateManager
        let policy_id = format!(
            "check-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );

        let mut sender = self.state_manager_sender.clone();
        let policy_decision = if status == 0 { "allowed" } else { "denied" };

        if let Err(e) = sender
            .report_policy_verification(&scenario_name, policy_decision, &policy_id)
            .await
        {
            println!(
                "Failed to report policy verification to StateManager: {:?}",
                e
            );
            // Don't fail the policy check if StateManager reporting fails
        }

        Ok(Response::new(CheckPolicyResponse { status, desc }))
    }
}
