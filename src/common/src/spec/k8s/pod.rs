// SPDX-License-Identifier: Apache-2.0

use super::Pod;
use crate::spec::artifact::Model;
use crate::spec::MetaData;

impl Pod {
    pub fn new(name: &str, podspec: PodSpec) -> Pod {
        Pod {
            apiVersion: String::from("v1"),
            kind: String::from("Pod"),
            metadata: MetaData {
                name: name.to_string(),
                labels: None,
                annotations: None,
            },
            spec: podspec,
        }
    }

    pub fn get_name(&self) -> String {
        self.metadata.name.clone()
    }
}

impl From<Model> for Pod {
    fn from(model: Model) -> Self {
        Pod::new(&model.get_name(), model.get_podspec())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PodSpec {
    hostNetwork: Option<bool>,
    pub containers: Vec<Container>,
    pub volumes: Option<Vec<Volume>>,
    initContainers: Option<Vec<Container>>,
    /// 컨테이너 재시작 정책 (Always / OnFailure / Never)
    pub restartPolicy: Option<String>,
    terminationGracePeriodSeconds: Option<i32>,
    hostIPC: Option<bool>,
    runtimeClassName: Option<String>,
    securityContext: Option<PodSecurityContext>,
    /// 컨테이너 생존성 프로브 설정 (Liveness Probe)
    #[serde(default)]
    pub probeConfig: Option<ProbeConfig>,
}

// ── Liveness Probe 관련 구조체 ──────────────────────────────────────────────

/// Pod YAML의 `probeConfig` 최상위 구조체.
/// liveness 필드에 LivenessProbe 설정을 담습니다.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ProbeConfig {
    /// 생존성(Liveness) 프로브 설정
    pub liveness: Option<LivenessProbe>,
}

/// Liveness Probe 설정.
/// HTTP / TCP / Exec 세 가지 방식 중 하나를 지정합니다.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct LivenessProbe {
    /// HTTP GET 방식 프로브
    pub http: Option<HttpProbe>,
    /// TCP 연결 방식 프로브
    pub tcp: Option<TcpProbe>,
    /// 컨테이너 내부 명령 실행 방식 프로브
    pub exec: Option<ExecProbe>,
    /// 컨테이너 시작 후 첫 번째 프로브까지 대기 시간(초). 기본값: 0
    #[serde(default)]
    pub initialDelaySeconds: u32,
    /// 프로브 실행 간격(초). 기본값: 10
    #[serde(default = "default_period_seconds")]
    pub periodSeconds: u32,
    /// 프로브 타임아웃(초). 기본값: 1
    #[serde(default = "default_timeout_seconds")]
    pub timeoutSeconds: u32,
    /// 컨테이너를 비정상으로 판단하기 위한 연속 실패 횟수. 기본값: 3
    #[serde(default = "default_failure_threshold")]
    pub failureThreshold: u8,
}

fn default_period_seconds() -> u32 {
    10
}

fn default_timeout_seconds() -> u32 {
    1
}

fn default_failure_threshold() -> u8 {
    3
}

/// HTTP GET 프로브 설정
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct HttpProbe {
    /// 요청 경로 (예: "/health")
    pub path: String,
    /// 대상 포트
    pub port: u16,
}

/// TCP 연결 프로브 설정
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TcpProbe {
    /// 대상 포트
    pub port: u16,
}

/// Exec 프로브 설정
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ExecProbe {
    /// 컨테이너 내부에서 실행할 명령어 및 인자
    pub command: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Container {
    name: String,
    image: String,
    volumeMounts: Option<Vec<VolumeMount>>,
    env: Option<Vec<Env>>,
    ports: Option<Vec<Port>>,
    pub command: Option<Vec<String>>,
    workingDir: Option<String>,
    resources: Option<Resources>,
    securityContext: Option<SecurityContext>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PodSecurityContext {
    runAsUser: Option<i64>,
    runAsGroup: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Volume {
    name: String,
    hostPath: HostPath,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct HostPath {
    path: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct VolumeMount {
    name: String,
    mountPath: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Env {
    name: String,
    value: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Port {
    containerPort: Option<i32>,
    hostPort: Option<i32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Resources {
    requests: Option<Requests>,
    limits: Option<Limits>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Requests {
    cpu: Option<String>,
    memory: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Limits {
    cpu: Option<String>,
    memory: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SecurityContext {
    privileged: Option<bool>,
    capabilities: Option<Capabilities>,
    runAsUser: Option<i64>,
    runAsGroup: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Capabilities {
    add: Option<Vec<String>>,
    drop: Option<Vec<String>>,
}

impl PodSpec {
    /// Returns the image of the first container in the PodSpec.
    /// If no containers are present, returns `None`.
    pub fn get_image(&self) -> Option<&str> {
        self.containers
            .first()
            .map(|container| container.image.as_str())
    }

    pub fn get_volume(&mut self) -> &Option<Vec<Volume>> {
        &self.volumes
    }
}

//Unit Test Cases
#[cfg(test)]
mod tests {
    use super::*;

    // Positive Test: Validate that `get_image` returns the image of the first container
    // when multiple containers are present in the PodSpec.
    #[tokio::test]
    async fn test_get_image_with_multiple_containers() {
        let container1 = Container {
            name: String::from("container-1"),
            image: String::from("image-1"),
            volumeMounts: None,
            env: None,
            ports: None,
            command: None,
            workingDir: None,
            resources: None,
            securityContext: None,
        };
        let container2 = Container {
            name: String::from("container-2"),
            image: String::from("image-2"),
            volumeMounts: None,
            env: None,
            ports: None,
            command: None,
            workingDir: None,
            resources: None,
            securityContext: None,
        };
        let podspec = PodSpec {
            hostNetwork: None,
            containers: vec![container1, container2],
            volumes: None,
            initContainers: None,
            restartPolicy: None,
            terminationGracePeriodSeconds: None,
            hostIPC: None,
            runtimeClassName: None,
            securityContext: None,
            probeConfig: None,
        };
        assert_eq!(podspec.get_image(), Some("image-1"));
    }

    // Negative Test: Validate that `get_image` returns `None` when no containers are present.
    #[tokio::test]
    async fn test_get_image_with_no_containers() {
        let podspec = PodSpec {
            hostNetwork: None,
            containers: vec![],
            volumes: None,
            initContainers: None,
            restartPolicy: None,
            terminationGracePeriodSeconds: None,
            hostIPC: None,
            runtimeClassName: None,
            securityContext: None,
            probeConfig: None,
        };
        assert_eq!(podspec.get_image(), None);
    }

    // Negative Test: Validate that `get_image` correctly handles containers with an empty
    // image field and returns an empty string.
    #[tokio::test]
    async fn test_get_image_with_null_image_field() {
        let container = Container {
            name: String::from("test-container"),
            image: String::from(""),
            volumeMounts: None,
            env: None,
            ports: None,
            command: None,
            workingDir: None,
            resources: None,
            securityContext: None,
        };
        let podspec = PodSpec {
            hostNetwork: None,
            containers: vec![container],
            volumes: None,
            initContainers: None,
            restartPolicy: None,
            terminationGracePeriodSeconds: None,
            hostIPC: None,
            runtimeClassName: None,
            securityContext: None,
            probeConfig: None,
        };
        assert_eq!(podspec.get_image(), Some(""));
    }

    // Positive Test: Validate that `get_volume` correctly returns all volumes when
    // multiple volumes are present in the PodSpec.
    #[tokio::test]
    async fn test_get_volume_with_multiple_volumes() {
        let volume1 = Volume {
            name: String::from("volume-1"),
            hostPath: HostPath {
                path: String::from("/path/1"),
            },
        };
        let volume2 = Volume {
            name: String::from("volume-2"),
            hostPath: HostPath {
                path: String::from("/path/2"),
            },
        };
        let mut podspec = PodSpec {
            hostNetwork: None,
            containers: vec![],
            volumes: Some(vec![volume1, volume2]),
            initContainers: None,
            restartPolicy: None,
            terminationGracePeriodSeconds: None,
            hostIPC: None,
            runtimeClassName: None,
            securityContext: None,
            probeConfig: None,
        };
        assert_eq!(
            podspec.get_volume(),
            &Some(vec![
                Volume {
                    name: String::from("volume-1"),
                    hostPath: HostPath {
                        path: String::from("/path/1"),
                    },
                },
                Volume {
                    name: String::from("volume-2"),
                    hostPath: HostPath {
                        path: String::from("/path/2"),
                    },
                },
            ])
        );
    }

    // Negative Test: Validate that `get_volume` returns `None` when no volumes are present.
    #[tokio::test]
    async fn test_get_volume_with_no_volumes() {
        let mut podspec = PodSpec {
            hostNetwork: None,
            containers: vec![],
            volumes: None,
            initContainers: None,
            restartPolicy: None,
            terminationGracePeriodSeconds: None,
            hostIPC: None,
            runtimeClassName: None,
            securityContext: None,
            probeConfig: None,
        };
        assert_eq!(podspec.get_volume(), &None);
    }

    // Negative Test: Validate that `get_volume` correctly handles an empty volume list.
    #[tokio::test]
    async fn test_get_volume_with_empty_volume_list() {
        let mut podspec = PodSpec {
            hostNetwork: None,
            containers: vec![],
            volumes: Some(vec![]),
            initContainers: None,
            restartPolicy: None,
            terminationGracePeriodSeconds: None,
            hostIPC: None,
            runtimeClassName: None,
            securityContext: None,
            probeConfig: None,
        };
        assert_eq!(podspec.get_volume(), &Some(vec![]));
    }

    // Negative Test: Validate that `get_volume` correctly handles invalid volume data.
    #[tokio::test]
    async fn test_get_volume_with_invalid_volume() {
        let volume = Volume {
            name: String::from(""),
            hostPath: HostPath {
                path: String::from(""),
            },
        };
        let mut podspec = PodSpec {
            hostNetwork: None,
            containers: vec![],
            volumes: Some(vec![volume]),
            initContainers: None,
            restartPolicy: None,
            terminationGracePeriodSeconds: None,
            hostIPC: None,
            runtimeClassName: None,
            securityContext: None,
            probeConfig: None,
        };
        assert_eq!(
            podspec.get_volume(),
            &Some(vec![Volume {
                name: String::from(""),
                hostPath: HostPath {
                    path: String::from(""),
                },
            }])
        );
    }

    // Positive Test: Validate that `get_image` correctly handles container image names
    // with special characters such as colons and tags.
    #[tokio::test]
    async fn test_get_image_with_special_characters_in_image_name() {
        let container = Container {
            name: String::from("test-container"),
            image: String::from("special:image@tag"),
            volumeMounts: None,
            env: None,
            ports: None,
            command: None,
            workingDir: None,
            resources: None,
            securityContext: None,
        };
        let podspec = PodSpec {
            hostNetwork: None,
            containers: vec![container],
            volumes: None,
            initContainers: None,
            restartPolicy: None,
            terminationGracePeriodSeconds: None,
            hostIPC: None,
            runtimeClassName: None,
            securityContext: None,
            probeConfig: None,
        };
        assert_eq!(podspec.get_image(), Some("special:image@tag"));
    }

    // ── probeConfig 파싱 테스트 ────────────────────────────────────────────

    /// probeConfig가 없는 YAML을 파싱하면 probeConfig는 None이어야 한다.
    #[test]
    fn test_probe_config_none_when_absent() {
        let yaml = r#"
apiVersion: v1
kind: Pod
metadata:
  name: no-probe-pod
spec:
  containers:
    - name: app
      image: nginx:latest
"#;
        let pod: super::super::Pod = serde_yaml::from_str(yaml).unwrap();
        assert!(pod.spec.probeConfig.is_none());
    }

    /// HTTP 프로브가 포함된 YAML을 파싱하면 probeConfig가 올바르게 채워져야 한다.
    #[test]
    fn test_probe_config_http_parsed() {
        let yaml = r#"
apiVersion: v1
kind: Pod
metadata:
  name: http-probe-pod
spec:
  containers:
    - name: app
      image: nginx:latest
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
        let pod: super::super::Pod = serde_yaml::from_str(yaml).unwrap();
        let probe_config = pod.spec.probeConfig.expect("probeConfig must be Some");
        let liveness = probe_config.liveness.expect("liveness must be Some");
        let http = liveness.http.expect("http must be Some");
        assert_eq!(http.path, "/health");
        assert_eq!(http.port, 8080);
        assert_eq!(liveness.initialDelaySeconds, 5);
        assert_eq!(liveness.periodSeconds, 10);
        assert_eq!(liveness.timeoutSeconds, 2);
        assert_eq!(liveness.failureThreshold, 3);
        // TCP 및 Exec는 None이어야 한다
        assert!(liveness.tcp.is_none());
        assert!(liveness.exec.is_none());
    }

    /// TCP 프로브가 포함된 YAML을 파싱하면 TcpProbe가 올바르게 채워져야 한다.
    #[test]
    fn test_probe_config_tcp_parsed() {
        let yaml = r#"
apiVersion: v1
kind: Pod
metadata:
  name: tcp-probe-pod
spec:
  containers:
    - name: app
      image: redis:latest
  probeConfig:
    liveness:
      tcp:
        port: 6379
"#;
        let pod: super::super::Pod = serde_yaml::from_str(yaml).unwrap();
        let probe_config = pod.spec.probeConfig.expect("probeConfig must be Some");
        let liveness = probe_config.liveness.expect("liveness must be Some");
        let tcp = liveness.tcp.expect("tcp must be Some");
        assert_eq!(tcp.port, 6379);
        // 기본값 확인
        assert_eq!(liveness.periodSeconds, 10);
        assert_eq!(liveness.timeoutSeconds, 1);
        assert_eq!(liveness.failureThreshold, 3);
    }

    /// Exec 프로브가 포함된 YAML을 파싱하면 ExecProbe가 올바르게 채워져야 한다.
    #[test]
    fn test_probe_config_exec_parsed() {
        let yaml = r#"
apiVersion: v1
kind: Pod
metadata:
  name: exec-probe-pod
spec:
  containers:
    - name: app
      image: busybox:latest
  probeConfig:
    liveness:
      exec:
        command:
          - cat
          - /tmp/healthy
      failureThreshold: 5
"#;
        let pod: super::super::Pod = serde_yaml::from_str(yaml).unwrap();
        let probe_config = pod.spec.probeConfig.expect("probeConfig must be Some");
        let liveness = probe_config.liveness.expect("liveness must be Some");
        let exec = liveness.exec.expect("exec must be Some");
        assert_eq!(exec.command, vec!["cat", "/tmp/healthy"]);
        assert_eq!(liveness.failureThreshold, 5);
    }

    /// restartPolicy 필드가 YAML에서 올바르게 파싱되어야 한다.
    #[test]
    fn test_restart_policy_parsed() {
        let yaml = r#"
apiVersion: v1
kind: Pod
metadata:
  name: restart-pod
spec:
  containers:
    - name: app
      image: nginx:latest
  restartPolicy: OnFailure
"#;
        let pod: super::super::Pod = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(pod.spec.restartPolicy.as_deref(), Some("OnFailure"));
    }
}
