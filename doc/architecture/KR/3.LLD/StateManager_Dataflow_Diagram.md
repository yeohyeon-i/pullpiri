# StateManager 데이터 흐름 다이어그램

## StateManager 핵심 데이터 흐름

이 다이어그램은 문제 설명에서 요청한 StateManager의 핵심 기능과 데이터 흐름을 보여줍니다.

```
                          ┌────────────────────────────────────────────────────────────────┐
                          │                    StateManager                                │
                          │                                                                │
    gRPC 수신             │                                                                │                    발신
                          │                                                                │
┌─────────────────────────┼────────────────────────────────────────────────────────────────┼─────────────────────────┐
│                         │                                                                │                         │
│  ┌─────────────────┐    │  ┌─────────────────────────────────────────────────────────┐   │    ┌─────────────────┐  │
│  │   NodeAgent     │    │  │                                                         │   │    │      ETCD       │  │
│  │                 │────┼─►│  Model에 포함된 container의 달라진 상태 정보            │   │    │                 │  │
│  │ Container 상태  │    │  │                                                         │   │    │ State 저장:     │  │
│  │ 정보 전달       │    │  │  처리:                                                  │   │    │                 │  │
│  └─────────────────┘    │  │  • Container 상태 → Model 상태 결정                     │───┼───►│ • Scenario      │  │
│                         │  │  • Model 상태 → ETCD put                              │   │    │ • Model         │  │
│  ┌─────────────────┐    │  │  • Model 상태 변경 → Package 상태 연쇄 평가             │   │    │ • Package       │  │
│  │   ApiServer     │    │  │  • Package 상태 → ETCD put                            │   │    │                 │  │
│  │                 │────┼─►│                                                         │   │    └─────────────────┘  │
│  │ Scenario 상태   │    │  │  Scenario state 변경 요청                              │   │                         │
│  │ 변경 요청       │    │  │                                                         │   │    ┌─────────────────┐  │
│  └─────────────────┘    │  │  처리:                                                  │   │    │ActionController │  │
│                         │  │  • State 변경 요청 → ETCD put                         │   │    │                 │  │
│  ┌─────────────────┐    │  │                                                         │   │    │ Reconcile 요청: │  │
│  │ FilterGateway   │    │  │                                                         │   │    │                 │  │
│  │                 │────┼─►│                                                         │───┼───►│ Package dead    │  │
│  │ Scenario 상태   │    │  │                                                         │   │    │ 상태일 때       │  │
│  │ 변경 요청       │    │  │                                                         │   │    │                 │  │
│  └─────────────────┘    │  └─────────────────────────────────────────────────────────┘   │    └─────────────────┘  │
│                         │                                                                │                         │
│  ┌─────────────────┐    │                                                                │                         │
│  │ActionController │    │                                                                │                         │
│  │                 │────┼─►                                                              │                         │
│  │ Scenario 상태   │    │                                                                │                         │
│  │ 변경 요청       │    │                                                                │                         │
│  └─────────────────┘    │                                                                │                         │
│                         │                                                                │                         │
│  ┌─────────────────┐    │                                                                │                         │
│  │ PolicyManager   │    │                                                                │                         │
│  │                 │────┼─►                                                              │                         │
│  │ Scenario 상태   │    │                                                                │                         │
│  │ 변경 요청       │    │                                                                │                         │
│  └─────────────────┘    │                                                                │                         │
│                         │                                                                │                         │
└─────────────────────────┼────────────────────────────────────────────────────────────────┼─────────────────────────┘
                          │                                                                │
                          └────────────────────────────────────────────────────────────────┘
```

## 상세 처리 흐름

### 1. Container 상태 정보 처리 흐름

```
NodeAgent
    │
    │ gRPC: ContainerList
    ▼
┌─────────────────────────────────────────────────────────┐
│                StateManager                             │
│                                                         │
│  Container 상태 정보 수신                                │
│         │                                               │
│         ▼                                               │
│  ┌─────────────────────────────────────────────────┐    │
│  │ Model 상태 결정 로직                             │    │
│  │                                                 │    │
│  │ • 모든 container가 paused → Model: Paused       │    │
│  │ • 모든 container가 exited → Model: Exited       │    │
│  │ • 하나 이상 container가 dead → Model: Dead       │    │
│  │ • 그 외의 경우 → Model: Running                  │    │
│  └─────────────────────────────────────────────────┘    │
│         │                                               │
│         ▼                                               │
│  Model 상태 → ETCD put                                  │
│         │                                               │
│         ▼                                               │
│  ┌─────────────────────────────────────────────────┐    │
│  │ Package 상태 연쇄 평가                           │    │
│  │                                                 │    │
│  │ • 모든 model이 paused → Package: paused         │    │
│  │ • 모든 model이 exited → Package: exited         │    │
│  │ • 일부 model이 dead → Package: degraded         │    │
│  │ • 모든 model이 dead → Package: error            │    │
│  │ • 그 외의 경우 → Package: running               │    │
│  └─────────────────────────────────────────────────┘    │
│         │                                               │
│         ▼                                               │
│  Package 상태 → ETCD put                                │
│         │                                               │
│         ▼                                               │
│  Package가 dead(error) 상태?                            │
│         │                                               │
│         ▼ (Yes)                                        │
│  ActionController에 reconcile 요청                      │
│                                                         │
└─────────────────────────────────────────────────────────┘
    │                    │
    ▼                    ▼
  ETCD              ActionController
  저장              (reconcile 요청)
```

### 2. Scenario 상태 변경 요청 처리 흐름

```
ApiServer / FilterGateway / ActionController / PolicyManager
    │
    │ gRPC: StateChange
    ▼
┌─────────────────────────────────────────────────────────┐
│                StateManager                             │
│                                                         │
│  Scenario 상태 변경 요청 수신                            │
│         │                                               │
│         ▼                                               │
│  상태 변경 요청 → ETCD put                              │
│  (/scenario/{scenario_name}/state)                      │
│                                                         │
└─────────────────────────────────────────────────────────┘
    │
    ▼
  ETCD
  저장
```

## ETCD 저장 형식

### 키-값 형식
```
Scenario: /scenario/{scenario_name}/state  →  "idle" | "waiting" | "satisfied" | "allowed" | "denied" | "completed"
Model:    /model/{model_name}/state        →  "Created" | "Paused" | "Exited" | "Dead" | "Running"
Package:  /package/{package_name}/state    →  "idle" | "paused" | "exited" | "degraded" | "error" | "running"
```

## ActionController Reconcile 요청

### 발생 조건
- Package 상태가 error (모든 model이 dead 상태)로 변경될 때

### 요청 내용
```rust
ReconcileRequest {
    scenario_name: String,    // 해당 package를 포함하는 scenario 이름
    current: PodStatus::Failed,
    desired: PodStatus::Running,
}
```

## 핵심 특징

1. **비동기 처리**: 모든 상태 변경은 비동기적으로 처리됩니다.
2. **연쇄 반응**: Model 상태 변경은 자동으로 Package 상태 평가를 트리거합니다.
3. **자동 복구**: Package가 error 상태가 되면 자동으로 ActionController에 복구를 요청합니다.
4. **중앙집중화**: 모든 상태 정보는 ETCD에 중앙집중식으로 저장됩니다.
5. **조건부 처리**: Container/Model 상태에 따라 상위 리소스 상태가 결정됩니다.