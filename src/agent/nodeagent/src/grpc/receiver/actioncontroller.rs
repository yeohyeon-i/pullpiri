/*
 * SPDX-FileCopyrightText: Copyright 2024 LG Electronics Inc.
 * SPDX-License-Identifier: Apache-2.0
 */
use crate::desired_state::DesiredState;
use common::nodeagent::fromactioncontroller::{
    HandleWorkloadRequest, HandleWorkloadResponse, WorkloadCommand,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

/// Extract pod name from pod YAML string.
fn extract_pod_name(pod_yaml: &str) -> Result<String, Box<dyn std::error::Error>> {
    let pod = serde_yaml::from_str::<common::spec::k8s::Pod>(pod_yaml)?;
    Ok(pod.get_name())
}

pub async fn handle_workload(
    request: Request<HandleWorkloadRequest>,
    desired_states_cache: Arc<Mutex<HashMap<String, DesiredState>>>,
) -> Result<Response<HandleWorkloadResponse>, Status> {
    let req = request.into_inner();
    let pod_yaml = req.pod.clone();
    let command = req.workload_command;

    // Extract pod name from the pod YAML for cache keying
    let pod_name = match extract_pod_name(&pod_yaml) {
        Ok(name) => name,
        Err(e) => {
            return Err(Status::invalid_argument(format!(
                "Failed to parse pod YAML: {}",
                e
            )));
        }
    };

    if command == WorkloadCommand::Start as i32 {
        // 1. Create DesiredState struct
        let desired_state = DesiredState::new(pod_name.clone());

        // 2. Insert into memory cache before starting the container
        {
            let mut cache = desired_states_cache.lock().await;
            cache.insert(pod_name.clone(), desired_state);
        }

        // 3. Start the container via Podman API and convert any error to String immediately
        //    to avoid holding Box<dyn Error> (not Send) across the subsequent await points.
        let start_result = crate::runtime::podman::handle_workload(command, &pod_yaml)
            .await
            .map_err(|e| e.to_string());

        match start_result {
            Ok(container_ids) => {
                // Update cache entry with the Podman container ID
                if let Some(first_id) = container_ids.into_iter().next() {
                    let mut cache = desired_states_cache.lock().await;
                    if let Some(state) = cache.get_mut(&pod_name) {
                        state.container_id = first_id;
                    }
                }
                println!(
                    "Workload started and desired state cached for: {}",
                    pod_name
                );
                Ok(Response::new(HandleWorkloadResponse {
                    status: true,
                    desc: format!(
                        "Container started and desired state cached for {}",
                        pod_name
                    ),
                }))
            }
            Err(err_msg) => {
                // Remove from cache on container start failure
                let mut cache = desired_states_cache.lock().await;
                cache.remove(&pod_name);
                println!(
                    "Failed to start container for {}, removed from cache: {:?}",
                    pod_name, err_msg
                );
                Err(Status::internal(format!(
                    "Failed to start container: {}",
                    err_msg
                )))
            }
        }
    } else if command == WorkloadCommand::Stop as i32 || command == WorkloadCommand::Remove as i32 {
        // Remove from memory cache before stopping
        {
            let mut cache = desired_states_cache.lock().await;
            cache.remove(&pod_name);
        }
        println!("Removed desired state from cache for: {}", pod_name);

        // Stop/remove the container via Podman API
        match crate::runtime::podman::handle_workload(command, &pod_yaml).await {
            Ok(_) => Ok(Response::new(HandleWorkloadResponse {
                status: true,
                desc: format!(
                    "Container stopped and desired state removed for {}",
                    pod_name
                ),
            })),
            Err(e) => Err(Status::internal(format!("Failed to stop container: {}", e))),
        }
    } else {
        // For other commands (Restart, Pause, Unpause, etc.), forward to Podman without cache changes
        match crate::runtime::podman::handle_workload(command, &pod_yaml).await {
            Ok(_) => {
                println!("Workload command {} executed for: {}", command, pod_name);
                Ok(Response::new(HandleWorkloadResponse {
                    status: true,
                    desc: format!("Workload command executed for {}", pod_name),
                }))
            }
            Err(e) => Err(Status::unimplemented(format!(
                "handle_workload is not implemented yet: {}",
                e
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::nodeagent::fromactioncontroller::WorkloadCommand;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn make_cache() -> Arc<Mutex<HashMap<String, DesiredState>>> {
        Arc::new(Mutex::new(HashMap::new()))
    }

    const VALID_POD_YAML: &str = r#"
apiVersion: v1
kind: Pod
metadata:
  name: test-pod
spec:
  containers:
    - name: test-container
      image: nginx:latest
"#;

    #[test]
    fn test_extract_pod_name_valid() {
        let result = extract_pod_name(VALID_POD_YAML);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test-pod");
    }

    #[test]
    fn test_extract_pod_name_invalid_yaml() {
        let result = extract_pod_name("not: valid: yaml: [");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_pod_name_empty() {
        let result = extract_pod_name("");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_handle_workload_invalid_yaml_returns_error() {
        let cache = make_cache();
        let request = tonic::Request::new(HandleWorkloadRequest {
            workload_command: WorkloadCommand::Start as i32,
            pod: "invalid yaml [[[".to_string(),
        });

        let result = handle_workload(request, cache).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_handle_workload_stop_removes_from_cache() {
        let cache = make_cache();

        // Pre-populate cache with a desired state
        {
            let mut c = cache.lock().await;
            c.insert(
                "test-pod".to_string(),
                DesiredState::new("test-pod".to_string()),
            );
        }

        // Verify entry exists
        assert_eq!(cache.lock().await.len(), 1);

        // Send STOP command (will fail at podman level since no podman, but cache should be cleared)
        let request = tonic::Request::new(HandleWorkloadRequest {
            workload_command: WorkloadCommand::Stop as i32,
            pod: VALID_POD_YAML.to_string(),
        });

        let _ = handle_workload(request, Arc::clone(&cache)).await;

        // Cache entry should be removed regardless of podman result
        assert_eq!(cache.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_handle_workload_remove_clears_from_cache() {
        let cache = make_cache();

        // Pre-populate cache
        {
            let mut c = cache.lock().await;
            c.insert(
                "test-pod".to_string(),
                DesiredState::new("test-pod".to_string()),
            );
        }

        let request = tonic::Request::new(HandleWorkloadRequest {
            workload_command: WorkloadCommand::Remove as i32,
            pod: VALID_POD_YAML.to_string(),
        });

        // Even if podman fails, the cache should be cleared
        let _ = handle_workload(request, Arc::clone(&cache)).await;
        assert_eq!(cache.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_handle_workload_start_clears_cache_on_podman_failure() {
        let cache = make_cache();

        // START command will fail because podman is not running
        let request = tonic::Request::new(HandleWorkloadRequest {
            workload_command: WorkloadCommand::Start as i32,
            pod: VALID_POD_YAML.to_string(),
        });

        let result = handle_workload(request, Arc::clone(&cache)).await;

        // Should return an error
        assert!(result.is_err());
        // Cache should be empty (cleaned up after failure)
        assert_eq!(cache.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn test_handle_workload_stop_missing_from_cache_is_noop() {
        let cache = make_cache();
        // Cache is empty - stopping should still attempt podman stop

        let request = tonic::Request::new(HandleWorkloadRequest {
            workload_command: WorkloadCommand::Stop as i32,
            pod: VALID_POD_YAML.to_string(),
        });

        // Should not panic even if pod is not in cache
        let _ = handle_workload(request, Arc::clone(&cache)).await;
        assert_eq!(cache.lock().await.len(), 0);
    }
}
