# StateManager 아키텍처 다이어그램

## 개요
StateManager는 Pullpiri 시스템에서 Scenario, Model, Package의 상태를 관리하는 핵심 컴포넌트입니다. 
gRPC를 통해 다양한 컴포넌트로부터 상태 변경 요청을 수신하고, 이를 처리하여 ETCD에 저장하며, 필요시 ActionController에 reconcile 요청을 보냅니다.

## StateManager 전체 아키텍처 다이어그램

```
                                    ┌─────────────────────────────────────────────────────────────┐
                                    │                    StateManager                              │
                                    │                                                             │
                   gRPC 수신        │  ┌─────────────────┐    ┌──────────────────────────────┐   │
┌─────────────────┐ ────────────────┼─►│  gRPC Receiver  │────│         Manager             │   │
│   NodeAgent     │                 │  │   (receiver.rs) │    │      (manager.rs)           │   │
│                 │                 │  └─────────────────┘    │                              │   │
│  Container 상태  │                 │                         │  ┌─────────────────────────┐ │   │
│  정보 전달      │                 │                         │  │  Container List         │ │   │
└─────────────────┘                 │                         │  │  Processing Task        │ │   │
                                    │                         │  └─────────────────────────┘ │   │
┌─────────────────┐ ────────────────┼─►                      │                              │   │
│   ApiServer     │                 │                         │  ┌─────────────────────────┐ │   │
│                 │                 │                         │  │  State Change           │ │   │
│  Scenario 상태   │                 │                         │  │  Processing Task        │ │   │
│  변경 요청      │                 │                         │  └─────────────────────────┘ │   │
└─────────────────┘                 │                         │                              │   │
                                    │                         │           │                  │   │
┌─────────────────┐ ────────────────┼─►                      │           ▼                  │   │
│ FilterGateway   │                 │                         │  ┌─────────────────────────┐ │   │
│                 │                 │                         │  │    State Machine        │ │   │
│  Scenario 상태   │                 │                         │  │   (state_machine.rs)    │ │   │
│  변경 요청      │                 │                         │  │                         │ │   │
└─────────────────┘                 │                         │  │ • Scenario 상태 전이    │ │   │
                                    │                         │  │ • Model 상태 평가       │ │   │
┌─────────────────┐ ────────────────┼─►                      │  │ • Package 상태 평가     │ │   │
│ActionController │                 │                         │  └─────────────────────────┘ │   │
│                 │                 │                         │                              │   │
│  Scenario 상태   │                 │                         └──────────────────────────────┘   │
│  변경 요청      │                 │                                                             │
└─────────────────┘                 │                                                             │
                                    │                          처리 로직                          │
┌─────────────────┐ ────────────────┼─►                                                          │
│ PolicyManager   │                 │  ┌─────────────────────────────────────────────────────────┐ │
│                 │                 │  │                  상태 처리 흐름                       │ │
│  Scenario 상태   │                 │  │                                                       │ │
│  변경 요청      │                 │  │  1. Scenario 상태 변경 요청                            │ │
└─────────────────┘                 │  │     └─► ETCD에 <scenario_name, state> put           │ │
                                    │  │                                                       │ │
                                    │  │  2. Container 상태 정보 수신                          │ │
                                    │  │     └─► Model 상태 결정                              │ │
                                    │  │     └─► ETCD에 <model, state> put                   │ │
                                    │  │     └─► Package 상태 연쇄 평가                       │ │
                                    │  │         └─► ETCD에 <package, state> put            │ │
                                    │  │         └─► Package가 dead이면 ActionController에   │ │
                                    │  │             reconcile 요청                          │ │
                                    │  │                                                       │ │
                                    │  └─────────────────────────────────────────────────────────┘ │
                                    │                                                             │
                                    │                         발신                                │
                                    │  ┌─────────────────┐                                       │
                 ETCD 저장          │  │  ETCD 연동      │ ──────────────────┐                   │
                ◄───────────────────┼──│  (common::etcd) │                   │                   │
                                    │  └─────────────────┘                   │                   │
                                    │                                        │                   │
                                    │  ┌─────────────────┐                   │                   │
              Reconcile 요청        │  │  gRPC Sender    │ ──────────────────┼─────────────────► │
                ◄───────────────────┼──│   (sender.rs)   │                   │                   │
                                    │  └─────────────────┘                   │                   │
                                    │                                        │                   │
                                    └────────────────────────────────────────┼───────────────────┘
                                                                             │
                                                                             ▼
                                    ┌─────────────────────────────────────────────────────────────┐
                                    │                      ETCD                                   │
                                    │                                                             │
                                    │  /scenario/{scenario_name}/state  →  "waiting"/"allowed"   │
                                    │  /model/{model_name}/state        →  "Running"/"Dead"      │
                                    │  /package/{package_name}/state    →  "running"/"error"     │
                                    │                                                             │
                                    └─────────────────────────────────────────────────────────────┘

                                    ┌─────────────────────────────────────────────────────────────┐
                                    │                ActionController                              │
                                    │                                                             │
                                    │  ← Reconcile Request (package가 dead 상태일 때)            │
                                    │    - scenario_name: 해당 패키지를 포함하는 시나리오         │
                                    │    - current: Failed                                        │
                                    │    - desired: Running                                       │
                                    │                                                             │
                                    └─────────────────────────────────────────────────────────────┘
```

## 컴포넌트별 상세 설명

### 1. gRPC 수신 (Input)

#### NodeAgent로부터:
- **데이터**: Model에 포함된 container의 달라진 상태 정보
- **처리**: Container 상태를 기반으로 Model 상태 결정
- **결과**: ETCD에 model 상태 저장, 연쇄적으로 package 상태 평가

#### ApiServer, FilterGateway, ActionController, PolicyManager로부터:
- **데이터**: Scenario의 state 변경 요청
- **처리**: 상태 변경 요청을 직접 처리
- **결과**: ETCD에 scenario 상태 저장

### 2. 내부 처리 로직

#### Scenario 상태 처리:
```
Scenario 상태 변경 요청 → StateManager → ETCD put
```

#### Model 상태 처리:
```
Container 상태 정보 → Model 상태 결정 → ETCD put → Package 상태 연쇄 평가
```

#### Package 상태 처리:
```
Model 상태 변경 → Package 상태 평가 → ETCD put
                                    ↓ (if package is dead)
                           ActionController reconcile 요청
```

### 3. 상태 전이 조건

#### Model 상태:
- **Created**: 생성 시 기본 상태
- **Paused**: 모든 container가 paused 상태
- **Exited**: 모든 container가 exited 상태  
- **Dead**: 하나 이상의 container가 dead 상태
- **Running**: 기본 상태 (위 조건에 해당하지 않을 때)

#### Package 상태:
- **idle**: 생성 시 기본 상태
- **paused**: 모든 model이 paused 상태
- **exited**: 모든 model이 exited 상태
- **degraded**: 일부 model이 dead 상태 (모든 model이 dead는 아님)
- **error**: 모든 model이 dead 상태
- **running**: 기본 상태 (위 조건에 해당하지 않을 때)

#### Scenario 상태:
- **idle**: 시나리오 초기화 상태
- **waiting**: 조건 등록 상태
- **satisfied**: 조건 만족 상태
- **allowed**: 정책 허용 상태
- **denied**: 정책 거부 상태
- **completed**: 실행 완료 상태

### 4. 발신 (Output)

#### ETCD 저장:
- **Scenario**: `/scenario/{scenario_name}/state`
- **Model**: `/model/{model_name}/state`  
- **Package**: `/package/{package_name}/state`

#### ActionController 통신:
- **조건**: Package가 dead 상태가 될 때
- **요청**: Reconcile request (scenario_name, current: Failed, desired: Running)

## 데이터 흐름 요약

1. **gRPC 수신**: 외부 컴포넌트들로부터 상태 변경 요청 및 컨테이너 상태 정보 수신
2. **상태 처리**: 
   - Scenario: 직접 ETCD 저장
   - Model: Container 상태 → Model 상태 결정 → ETCD 저장
   - Package: Model 상태 변경 → Package 상태 연쇄 평가 → ETCD 저장
3. **외부 통신**: 
   - ETCD에 모든 상태 정보 저장
   - Package dead 시 ActionController에 reconcile 요청

이 아키텍처는 StateManager가 중앙집중식 상태 관리자 역할을 하며, 다양한 리소스의 상태를 일관성 있게 관리하고 필요시 복구 작업을 트리거하는 것을 보여줍니다.