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
    pub restartPolicy: Option<String>,
    terminationGracePeriodSeconds: Option<i32>,
    hostIPC: Option<bool>,
    runtimeClassName: Option<String>,
    securityContext: Option<PodSecurityContext>,
    pub probeConfig: Option<ProbeConfig>,
}

/// Top-level probe configuration attached to a Pod specification.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ProbeConfig {
    pub liveness: Option<LivenessProbe>,
}

/// Liveness probe configuration mirroring the Kubernetes liveness probe spec.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct LivenessProbe {
    pub http: Option<HttpProbe>,
    pub tcp: Option<TcpProbe>,
    pub exec: Option<ExecProbe>,
    pub initialDelaySeconds: u32,
    pub periodSeconds: u32,
    pub timeoutSeconds: u32,
    pub failureThreshold: u8,
}

/// HTTP GET probe parameters.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct HttpProbe {
    pub path: String,
    pub port: u16,
}

/// TCP socket probe parameters.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TcpProbe {
    pub port: u16,
}

/// Exec probe parameters (command run inside the container).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ExecProbe {
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

    // Positive Test: Validate that a Pod YAML with probeConfig can be deserialized correctly.
    #[test]
    fn test_podspec_deserialize_with_probe_config() {
        let yaml = r#"
containers:
  - name: nginx
    image: nginx:latest
probeConfig:
  liveness:
    http:
      path: /healthz
      port: 80
    initialDelaySeconds: 5
    periodSeconds: 10
    timeoutSeconds: 3
    failureThreshold: 3
"#;
        let spec: PodSpec = serde_yaml::from_str(yaml).expect("deserialization failed");
        let probe_config = spec.probeConfig.expect("probeConfig should be present");
        let liveness = probe_config.liveness.expect("liveness should be present");
        assert_eq!(liveness.initialDelaySeconds, 5);
        assert_eq!(liveness.periodSeconds, 10);
        assert_eq!(liveness.timeoutSeconds, 3);
        assert_eq!(liveness.failureThreshold, 3);
        let http = liveness.http.expect("http probe should be present");
        assert_eq!(http.path, "/healthz");
        assert_eq!(http.port, 80);
        assert!(liveness.tcp.is_none());
        assert!(liveness.exec.is_none());
    }

    // Positive Test: Validate that a Pod YAML with TCP probe config deserializes correctly.
    #[test]
    fn test_podspec_deserialize_with_tcp_probe_config() {
        let yaml = r#"
containers:
  - name: redis
    image: redis:latest
probeConfig:
  liveness:
    tcp:
      port: 6379
    initialDelaySeconds: 10
    periodSeconds: 15
    timeoutSeconds: 5
    failureThreshold: 3
"#;
        let spec: PodSpec = serde_yaml::from_str(yaml).expect("deserialization failed");
        let probe_config = spec.probeConfig.expect("probeConfig should be present");
        let liveness = probe_config.liveness.expect("liveness should be present");
        let tcp = liveness.tcp.expect("tcp probe should be present");
        assert_eq!(tcp.port, 6379);
        assert!(liveness.http.is_none());
        assert!(liveness.exec.is_none());
    }

    // Positive Test: Validate that a Pod YAML with Exec probe config deserializes correctly.
    #[test]
    fn test_podspec_deserialize_with_exec_probe_config() {
        let yaml = r#"
containers:
  - name: myapp
    image: myapp:latest
probeConfig:
  liveness:
    exec:
      command:
        - cat
        - /tmp/healthy
    initialDelaySeconds: 0
    periodSeconds: 5
    timeoutSeconds: 2
    failureThreshold: 3
"#;
        let spec: PodSpec = serde_yaml::from_str(yaml).expect("deserialization failed");
        let probe_config = spec.probeConfig.expect("probeConfig should be present");
        let liveness = probe_config.liveness.expect("liveness should be present");
        let exec = liveness.exec.expect("exec probe should be present");
        assert_eq!(exec.command, vec!["cat", "/tmp/healthy"]);
        assert!(liveness.http.is_none());
        assert!(liveness.tcp.is_none());
    }

    // Negative Test: Validate that a Pod YAML without probeConfig results in None.
    #[test]
    fn test_podspec_deserialize_without_probe_config() {
        let yaml = r#"
containers:
  - name: nginx
    image: nginx:latest
"#;
        let spec: PodSpec = serde_yaml::from_str(yaml).expect("deserialization failed");
        assert!(spec.probeConfig.is_none());
    }
}
