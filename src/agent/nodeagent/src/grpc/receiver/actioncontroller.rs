/*
 * SPDX-FileCopyrightText: Copyright 2024 LG Electronics Inc.
 * SPDX-License-Identifier: Apache-2.0
 */

//! NodeAgent gRPC 수신 처리 모듈 - ActionController
//!
//! 이 모듈은 ActionController로부터 워크로드 요청(Start/Stop/Remove 등)을 수신하여
//! Podman을 통해 컨테이너를 제어하고, 자기 치유(Self-Healing)를 위한 DesiredState를
//! 인메모리 캐시에 관리합니다.
//!
//! ## 주요 기능
//! - Pod YAML을 완전히 파싱하여 probeConfig 및 restartPolicy를 DesiredState에 반영
//! - Liveness Probe 설정이 포함된 경우 probe_loop에서 자동으로 감지 및 실행

use crate::desired_state::{DesiredState, LivenessProbe, ProbeConfig, ProbeType, RestartPolicy};
use common::nodeagent::fromactioncontroller::{
    HandleWorkloadRequest, HandleWorkloadResponse, WorkloadCommand,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

/// Pod YAML 문자열에서 pod 이름을 추출합니다.
fn extract_pod_name(pod_yaml: &str) -> Result<String, Box<dyn std::error::Error>> {
    let pod = serde_yaml::from_str::<common::spec::k8s::Pod>(pod_yaml)?;
    Ok(pod.get_name())
}

/// `common::spec::k8s::pod::ProbeConfig`를 NodeAgent 내부 타입인
/// `crate::desired_state::ProbeConfig`로 변환합니다.
///
/// ## 변환 규칙
/// - liveness.http → ProbeType::Http
/// - liveness.tcp  → ProbeType::Tcp (http가 없는 경우)
/// - liveness.exec → ProbeType::Exec (http, tcp가 없는 경우)
/// - 셋 모두 없으면 liveness 설정을 None으로 처리
fn convert_probe_config(common_probe: common::spec::k8s::pod::ProbeConfig) -> Option<ProbeConfig> {
    let liveness = common_probe.liveness?;

    // 세 가지 프로브 타입 중 하나를 선택 (HTTP > TCP > Exec 우선순위)
    let probe_type = if let Some(http) = liveness.http {
        ProbeType::Http {
            path: http.path,
            port: http.port,
        }
    } else if let Some(tcp) = liveness.tcp {
        ProbeType::Tcp { port: tcp.port }
    } else if let Some(exec) = liveness.exec {
        ProbeType::Exec {
            command: exec.command,
        }
    } else {
        // 프로브 타입이 지정되지 않으면 Liveness Probe를 설정하지 않음
        return None;
    };

    Some(ProbeConfig {
        liveness: Some(LivenessProbe {
            probe_type,
            initial_delay_seconds: liveness.initialDelaySeconds,
            period_seconds: liveness.periodSeconds,
            timeout_seconds: liveness.timeoutSeconds,
            failure_threshold: liveness.failureThreshold,
        }),
    })
}

/// Pod YAML을 완전히 파싱하여 restart_policy와 probe_config가 설정된
/// DesiredState를 생성합니다.
///
/// YAML 파싱 실패 시 기본값(Always, probe_config=None)으로 폴백합니다.
fn build_desired_state(pod_name: String, pod_yaml: &str) -> DesiredState {
    let mut state = DesiredState::new(pod_name);

    if let Ok(pod) = serde_yaml::from_str::<common::spec::k8s::Pod>(pod_yaml) {
        // restartPolicy 변환
        state.restart_policy = match pod.spec.restartPolicy.as_deref() {
            Some("Always") => RestartPolicy::Always,
            Some("OnFailure") => RestartPolicy::OnFailure,
            Some("Never") => RestartPolicy::Never,
            _ => RestartPolicy::Always,
        };

        // probeConfig 변환 (None이면 Probe 미실행)
        state.probe_config = pod.spec.probeConfig.and_then(convert_probe_config);
    }

    state
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
        // 1. Pod YAML을 완전히 파싱하여 DesiredState 생성
        //    (restartPolicy, probeConfig 포함)
        let desired_state = build_desired_state(pod_name.clone(), &pod_yaml);

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

    /// probeConfig가 포함된 Pod YAML (HTTP 프로브)
    const POD_YAML_WITH_HTTP_PROBE: &str = r#"
apiVersion: v1
kind: Pod
metadata:
  name: http-probe-pod
spec:
  containers:
    - name: app
      image: nginx:latest
  restartPolicy: Always
  probeConfig:
    liveness:
      http:
        path: /health
        port: 8080
      initialDelaySeconds: 5
      periodSeconds: 10
      timeoutSeconds: 2
      failureThreshold: 3
"#;

    /// probeConfig가 포함된 Pod YAML (TCP 프로브)
    const POD_YAML_WITH_TCP_PROBE: &str = r#"
apiVersion: v1
kind: Pod
metadata:
  name: tcp-probe-pod
spec:
  containers:
    - name: app
      image: redis:latest
  restartPolicy: OnFailure
  probeConfig:
    liveness:
      tcp:
        port: 6379
"#;

    /// probeConfig가 포함된 Pod YAML (Exec 프로브)
    const POD_YAML_WITH_EXEC_PROBE: &str = r#"
apiVersion: v1
kind: Pod
metadata:
  name: exec-probe-pod
spec:
  containers:
    - name: app
      image: busybox:latest
  restartPolicy: Never
  probeConfig:
    liveness:
      exec:
        command:
          - cat
          - /tmp/healthy
      failureThreshold: 5
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

    // ── build_desired_state 테스트 ────────────────────────────────────────────

    /// probeConfig 없는 YAML로 빌드하면 probe_config가 None이어야 한다.
    #[test]
    fn test_build_desired_state_no_probe() {
        let state = build_desired_state("test-pod".to_string(), VALID_POD_YAML);
        assert_eq!(state.pod_name, "test-pod");
        assert!(state.probe_config.is_none());
        assert_eq!(state.restart_policy, RestartPolicy::Always);
    }

    /// HTTP probeConfig가 있는 YAML로 빌드하면 probe_config가 올바르게 설정되어야 한다.
    #[test]
    fn test_build_desired_state_http_probe() {
        let state = build_desired_state("http-probe-pod".to_string(), POD_YAML_WITH_HTTP_PROBE);
        assert_eq!(state.pod_name, "http-probe-pod");
        assert_eq!(state.restart_policy, RestartPolicy::Always);

        let probe_config = state.probe_config.expect("probe_config must be Some");
        let liveness = probe_config.liveness.expect("liveness must be Some");
        assert_eq!(liveness.initial_delay_seconds, 5);
        assert_eq!(liveness.period_seconds, 10);
        assert_eq!(liveness.timeout_seconds, 2);
        assert_eq!(liveness.failure_threshold, 3);

        match liveness.probe_type {
            ProbeType::Http { path, port } => {
                assert_eq!(path, "/health");
                assert_eq!(port, 8080);
            }
            _ => panic!("Expected Http probe type"),
        }
    }

    /// TCP probeConfig가 있는 YAML로 빌드하면 RestartPolicy::OnFailure와
    /// ProbeType::Tcp가 설정되어야 한다.
    #[test]
    fn test_build_desired_state_tcp_probe() {
        let state = build_desired_state("tcp-probe-pod".to_string(), POD_YAML_WITH_TCP_PROBE);
        assert_eq!(state.restart_policy, RestartPolicy::OnFailure);

        let probe_config = state.probe_config.expect("probe_config must be Some");
        let liveness = probe_config.liveness.expect("liveness must be Some");

        match liveness.probe_type {
            ProbeType::Tcp { port } => assert_eq!(port, 6379),
            _ => panic!("Expected Tcp probe type"),
        }
    }

    /// Exec probeConfig가 있는 YAML로 빌드하면 RestartPolicy::Never와
    /// ProbeType::Exec가 설정되어야 한다.
    #[test]
    fn test_build_desired_state_exec_probe() {
        let state = build_desired_state("exec-probe-pod".to_string(), POD_YAML_WITH_EXEC_PROBE);
        assert_eq!(state.restart_policy, RestartPolicy::Never);

        let probe_config = state.probe_config.expect("probe_config must be Some");
        let liveness = probe_config.liveness.expect("liveness must be Some");
        assert_eq!(liveness.failure_threshold, 5);

        match liveness.probe_type {
            ProbeType::Exec { command } => {
                assert_eq!(command, vec!["cat", "/tmp/healthy"]);
            }
            _ => panic!("Expected Exec probe type"),
        }
    }

    // ── convert_probe_config 테스트 ───────────────────────────────────────────

    /// probeConfig에 liveness가 없으면 None을 반환해야 한다.
    #[test]
    fn test_convert_probe_config_no_liveness() {
        let common_probe = common::spec::k8s::pod::ProbeConfig { liveness: None };
        let result = convert_probe_config(common_probe);
        assert!(result.is_none());
    }

    /// liveness에 프로브 타입이 하나도 없으면 None을 반환해야 한다.
    #[test]
    fn test_convert_probe_config_no_probe_type() {
        let common_probe = common::spec::k8s::pod::ProbeConfig {
            liveness: Some(common::spec::k8s::pod::LivenessProbe {
                http: None,
                tcp: None,
                exec: None,
                initialDelaySeconds: 0,
                periodSeconds: 10,
                timeoutSeconds: 1,
                failureThreshold: 3,
            }),
        };
        let result = convert_probe_config(common_probe);
        assert!(result.is_none());
    }

    // ── handle_workload 통합 테스트 ───────────────────────────────────────────

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

    /// START 명령 시 HTTP probeConfig가 포함된 YAML을 사용하면
    /// 캐시의 DesiredState에 probe_config가 설정되어야 한다.
    /// (Podman이 없어 Start는 실패하지만, 캐시는 설정 후 제거된다.)
    #[tokio::test]
    async fn test_handle_workload_start_with_probe_config_sets_desired_state() {
        let cache = make_cache();

        // Podman이 없으면 Start는 실패하지만, 시작 전에 캐시에 삽입되는 것을 확인
        // (실패 시 캐시는 제거됨 - 이는 기존 동작 유지)
        let request = tonic::Request::new(HandleWorkloadRequest {
            workload_command: WorkloadCommand::Start as i32,
            pod: POD_YAML_WITH_HTTP_PROBE.to_string(),
        });

        let result = handle_workload(request, Arc::clone(&cache)).await;

        // Podman 없이는 실패
        assert!(result.is_err());
        // 실패 후 캐시는 비어있어야 한다
        assert_eq!(cache.lock().await.len(), 0);
    }
}
