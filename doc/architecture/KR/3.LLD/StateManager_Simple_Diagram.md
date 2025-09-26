# StateManager 간단 아키텍처 다이어그램

문제 설명에서 요청한 StateManager의 기능을 간단하고 명확하게 보여주는 다이어그램입니다.

## StateManager 기능 흐름도

```
                    ┌─────────────────────────────────────────────────────────────┐
                    │                    StateManager                             │
                    │                                                             │
                    │                     gRPC 수신                              │
                    │  ┌─────────────────────────────────────────────────────┐    │
                    │  │                                                     │    │
   NodeAgent        │  │  Model에 포함된 container의 달라진 상태 정보        │    │        ETCD
       │            │  │                                                     │    │          │
       │ gRPC       │  │  ┌─────────────────────────────────────────────┐    │    │          │
       └────────────┼─►│  │              처리                           │    │    │          │
                    │  │  │                                             │    │    │          │
   ApiServer        │  │  │ • model에 포함된 container의 달라진         │    │    │          │
       │            │  │  │   상태 정보로 model의 state 결정            │────┼────┼─────────►│
       │ gRPC       │  │  │ • Model의 state 결정 및 ETCD put          │    │    │          │
       └────────────┼─►│  │                                             │    │    │          │
                    │  │  │ • package에 포함된 model의 상태가 달라지면   │    │    │          │
 FilterGateway      │  │  │   그에 따라 package의 state 결정            │────┼────┼─────────►│
       │            │  │  │ • Package의 state 결정 및 ETCD put        │    │    │          │
       │ gRPC       │  │  │                                             │    │    │          │
       └────────────┼─►│  └─────────────────────────────────────────────┘    │    │          │
                    │  │                                                     │    │          │
ActionController    │  │  Scenario의 state 변경 요청                        │    │          │
       │            │  │                                                     │    │          │
       │ gRPC       │  │  ┌─────────────────────────────────────────────┐    │    │          │
       └────────────┼─►│  │              처리                           │    │    │          │
                    │  │  │                                             │    │    │          │
 PolicyManager      │  │  │ • Scenario state 변경 요청 전달받으면       │────┼────┼─────────►│
       │            │  │  │   state 변경 요청을 ETCD put               │    │    │          │
       │ gRPC       │  │  │                                             │    │    │          │
       └────────────┼─►│  └─────────────────────────────────────────────┘    │    │          │
                    │  │                                                     │    │          │
                    │  └─────────────────────────────────────────────────────┘    │          │
                    │                                                             │          │
                    │                      발신                                  │          │
                    │  ┌─────────────────────────────────────────────────────┐    │          │
                    │  │                                                     │    │          │
                    │  │ • Scenario, model, package 의 state 변경           │────┼─────────►│
                    │  │   → ETCD 저장                                      │    │          │
                    │  │                                                     │    │          │
                    │  │ • package가 dead 되면                              │    │          │
                    │  │   reconcile 필요하므로                             │    │          │
                    │  │   ActionController에 reconcile 요청                │────┼────────────────┐
                    │  │                                                     │    │                │
                    │  └─────────────────────────────────────────────────────┘    │                │
                    │                                                             │                │
                    └─────────────────────────────────────────────────────────────┘                │
                                                                                                   │
                                                                                                   │
                                                                                                   ▼
                                                                                      ActionController
                                                                                    (reconcile 요청)
```

## 핵심 기능 요약

### gRPC 수신
1. **NodeAgent로부터**: Model에 포함된 container의 달라진 상태 정보
2. **ApiServer, FilterGateway, ActionController, PolicyManager로부터**: Scenario의 state 변경 요청

### 처리
1. **Scenario state 변경 요청 전달받으면**: state 변경 요청을 ETCD put
2. **model에 포함된 container의 달라진 상태 정보 전달 받으면**: 
   - Model에 포함된 container의 달라진 상태 정보로 model의 state 결정 및 ETCD put
3. **package에 포함된 model의 상태가 달라지면**: 그에 따라 package의 state 결정 및 ETCD put
4. **package가 dead 되면**: reconcile 필요하므로 ActionController에 reconcile 요청

### 발신
1. **Scenario, model, package 의 state 변경**: ETCD 저장
2. **Package의 dead state로 인한 reconcile 필요**: ActionController에 reconcile 요청

## 상태 전이 조건

### Model 상태 결정 로직
```
Container 상태들 → Model 상태
• 모든 container가 paused     → Model: Paused
• 모든 container가 exited     → Model: Exited  
• 하나 이상 container가 dead  → Model: Dead
• 그 외의 경우               → Model: Running
```

### Package 상태 결정 로직
```
Model 상태들 → Package 상태
• 모든 model이 paused         → Package: paused
• 모든 model이 exited         → Package: exited
• 일부 model이 dead           → Package: degraded
• 모든 model이 dead           → Package: error (→ ActionController reconcile 요청)
• 그 외의 경우               → Package: running
```

### Scenario 상태 처리
```
상태 변경 요청 → 직접 ETCD에 저장
• idle → waiting → satisfied → allowed/denied → completed
```