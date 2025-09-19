## 1. StateManager의 기능 
	- 상태 변경 요청 처리
		- NodeAgent, FilterGateway, PolicyManager, ActionController 등 각 컴포넌트는 자신이 다루는 리소스의 상태를 변화시키기 위해 StateManager에게 상태 변경을 요청합니다.
		- StateManager는 이 요청을 받아 해당 리소스의 상태를 ETCD에 저장합니다.

	- 상위 리소스 연쇄 상태 관리
		- StateManager는 하위 리소스의 상태가 변경될 때 연쇄적으로 상위 리소스의 상태도 변경되는지 확인합니다.
		- 변경이 필요한 경우 상위 리소스의 새로운 상태를 ETCD에 저장합니다.

## 2. package을 위해 StateManager에 구현되어야 하는 것 

```
+---------------------+         +-------------------+
|   StateManager      |   put   |       ETCD        |
|---------------------| ------> |-------------------|
							|
							--> +-------------------+
						gRPC    |  ActionController |
								|-------------------|
```
- **인터페이스:** 내부 함수로부터 수신, 외부 인터페이스(ETCD)로 발신
	- **수신:** 내부 함수로부터 model의 상태 변경 시 연쇄적으로 package의 상태가 변경되는지 확인 요청을 전달받음
	- **조건:** `<model, state>` 리스트가 package의 특정 state 조건과 일치하면 package의 state를 변경
	- **발신:** ETCD에 `<package, state>` put 요청 and package dead 상태 시 ActionController에 reconcile 요청

## 3. pacakge의 state 변경 조건
package의 state는 package에 포함된 model들의 상태가 package의 특정 state 조건과 일치하면 package의 state를 변경해야 합니다.

### 3.1 package 상태 정의 및 상태 전이 조건 요약표
| 상태      | 설명 | 조건 |
|-----------|------|---------------------------------------------------|
| idle      | 맨 처음 package의 상태 | 생성 시 기본 상태 |
| paused    | 모든 model이 paused 상태일 때 | 모든 model이 paused 상태 |
| exited    | 모든 model이 exited 상태일 때 | 모든 model이 exited 상태 |
| degraded  | 일부 model이 dead 상태일 때 | 일부(1개 이상) model이 dead 상태, 단 모든 model이 dead가 아닐 때 |
| error     | 모든 model이 dead 상태일 때 | 모든 model이 dead 상태 |
| running   | 위 조건을 모두 만족하지 않을 때(기본 상태) | 위 조건을 모두 만족하지 않을 때(기본 상태) |

### 3.2 model 상태 정의 및 상태 전이 조건 요약표
| 상태      | 설명 | 조건 |
|-----------|------|---------------------------------------------------|
| Created   | model의 최초 상태 | 생성 시 기본 상태 |
| Paused    | 모든 container가 paused 상태일 때 | 모든 container가 paused 상태 |
| Exited    | 모든 container가 exited 상태일 때 | 모든 container가 exited 상태 |
| Dead      | 하나 이상의 container가 dead 상태이거나, model 정보 조회 실패 | 하나 이상의 container가 dead 상태이거나, model 정보 조회 실패 |
| Running   | 위 조건을 모두 만족하지 않을 때(기본 상태) | 위 조건을 모두 만족하지 않을 때(기본 상태) |

### 3.3 container 상태 정의 및 상태 전이 조건 요약표
| 상태     | 설명                                                                 | 조건                                                         |
|----------|----------------------------------------------------------------------|--------------------------------------------------------------|
| Created  | 아직 실행된 컨테이너가 없는 상태. 컨테이너가 생성되지 않았거나 모두 삭제된 경우 | 컨테이너가 생성되지 않았거나 모두 삭제된 경우                |
| Running  | 하나 이상의 컨테이너가 실행 중인 상태                                 | 하나 이상의 컨테이너가 실행 중                                |
| Stopped  | 하나 이상의 컨테이너가 중지되었고, 실행 중인 컨테이너는 없음          | 하나 이상의 컨테이너가 중지, 실행 중인 컨테이너는 없음        |
| Exited   | Pod 내 모든 컨테이너가 종료된 상태                                    | 모든 컨테이너가 종료됨                                       |
| Dead     | Pod의 상태 정보를 가져오는 데 실패한 경우 (메타데이터 손상, 시스템 오류 등) | 상태 정보 조회 실패, 시스템 오류 등                           |

- **인터페이스:** 외부 인터페이스(gRPC)로부터 수신, 외부 인터페이스(ETCD)로 발신

## 4. 구현 규칙

이 문서를 기반으로 구현할 경우 반드시 따라야 하는 구현 규칙은 다음과 같다. 

### 4.1 state 변경 기능의 구현 위치 규칙

아래 로직은 state_machine.rs 파일 내 StateMachine 구현 블록 내에 구현되어야 한다. 

**조건:** `<container, state>` 리스트가 model의 특정 state 조건과 일치하면 model의 state를 변경
**구현위치:** pub struct StateMachine 구조체의 impl StateMachine 구현 블록에 구현되어야 함


### 4.2 etcd로 put, get 하는 방법 규칙 
etcd에 값을 저장(put)하거나 조회(get)할 때는 문서에 제시된 예시 코드의 지정된 key/value 포맷대로 작성해야 한다. 

예시1 : package의 state put 시 
```
let key = format!("/package/{}/state", package_name);
let value = package_state.as_str_name(); // 예: "Running"
if let Err(e) = common::etcd::put(&key, value).await {
	eprintln!("Failed to save package state: {:?}", e);
}
```
예시2 : get: etcd에서 값 조회

```
let key = "/package/my_package/state";
match common::etcd::get(key).await {
	Ok(value) => println!("Value: {}", value),
	Err(e) => eprintln!("Failed to get: {:?}", e),
}
```

예시3 : get_all_with_prefix: prefix로 여러 값 조회
```
let prefix = "/package/";
match common::etcd::get_all_with_prefix(prefix).await {
	Ok(kvs) => {
		for kv in kvs {
			println!("key: {}, value: {}", kv.key, kv.value);
		}
	}
	Err(e) => eprintln!("Failed to get: {:?}", e),
}
```