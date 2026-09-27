# 配对协议与状态机

## 状态机（`voltip-pairing::PairingState`）

```
Idle → CreatingSession → WaitingForPeer → KeyExchange → AwaitingVerification → Trusted
                                   ↘ Expired            ↘ Failed(reason)   ↘ Rejected
```

| 状态 | 进入条件 | 允许事件 | 超时 |
|---|---|---|---|
| Idle | 初始 / 取消 / 完成后重置 | `Start` | — |
| CreatingSession | `Start` | `SessionCreated`, `Error` | 10 s → Failed(timeout) |
| WaitingForPeer | 收到 `session_created`（拿到 code + ticket） | `PeerJoined`, `Tick`, `Cancel` | `expires_at` → Expired |
| KeyExchange | 对端加入，开始 Noise XX | `HandshakeMessage`, `Tick` | 15 s → Failed(handshake_timeout) |
| AwaitingVerification | 握手完成，拿到 safety code | `UserConfirm`, `UserReject`, `PeerConfirm`, `PeerReject`, `Tick` | 120 s → Expired |
| Trusted | 双方都确认 | — | — |
| Expired / Rejected / Failed | 终态；`Reset` 回 Idle | `Reset` | — |

设计约束：
- 状态机是 **sans-IO**：`step(event) -> Vec<Action>`，Action 包括 `SendRelay(frame)`、`SendPeer(bytes)`、`ShowSafetyCode`、`StoreTrusted`、`Emit(ui_event)`；所有网络/时间由调用方注入（`now: Instant`），因此可以在单测里穷举路径。
- 一次性：`session_id`、`code`、`nonce` 在进入 `KeyExchange` 时从 `WaitingForPeer` 的可接受集合中移除；重复 `PeerJoined` → `Failed(replay)`。
- 过期：`Tick(now)` 驱动；过期后收到任何对端消息一律忽略并回 `error{session_expired}`。

## 手机端（Responder）

```
Idle → Joining(code|ticket) → KeyExchange → AwaitingVerification → Trusted
                         ↘ Failed(invalid_code|expired|rate_limited)
```

## Relay 侧防护

- 短码空间 10^6；每连接对 `join_by_code` 失败上限 5 次后断开，每 IP 每分钟 20 次；每会话失败尝试上限 10 次后会话作废（发起方收到 `error{session_expired}`，UI 提示重新生成）。
- 会话 TTL 默认 120 s，最大 300 s；成功 `join` 后短码立即从索引删除。
- 所有计数与 TTL 都在 `voltip-relay::RelayState` 里，单测可直接推进时钟。

## 解除配对

- 任何一边「忘记」对方时，如果对方此刻在线，先经加密通道发一条 `unpair`（`docs/protocol.md`），再删掉记录、断开会话和直连拨号。对方收到后同样忘记发送方（不回发），发出 `Unpaired` 事件，界面提示「「名字」解除了与这台设备的配对」，设备列表随之更新。
- 这条消息只在认证过的会话里出现：能发出它的只有已配对的那一方，它最多只能让对方忘记它自己。
- 对方离线时收不到通知，会一直留着这条记录，直到下一次连接因为这边已不再信任它而失败；重新配对即可恢复。

## 连接自检

设备页（电脑）和已配对设备屏（手机）上的「连接自检」按一次跑一轮，结果是那一刻实际测到的（`voltip_core::connectivity`，`connectivity_check` 命令 → `connectivity` 事件，`UiState.connectivity` 保留上一次的报告）：

- **本机局域网监听**：LAN 主机是否在跑，以及它向已配对设备公布的 `ip:port`。
- **中继**：对配置的中继新建一条连接，走完 `hello` / `hello_ack` 再关掉，报告耗时，或者没有响应 / 拒绝连接 / 连接失败。内置中继连接失败时，原因里可能带着地址，所以只报类型、不带原因（地址是构建期的 secret，不进界面）。
- **每台已配对设备**：在线的，经它当前的加密通道发一次 `ping`，报告往返时间；它公布过的每个局域网地址都用同样的 `hello` 探测一次。和本机同一个 IPv4 /24 网段却连不上时，提示可能是对方防火墙拦住了端口，或者 Wi-Fi 开启了客户端隔离。

每个探测最多 3 秒，全部并行；4 秒后没回应的 `ping` 按没有回应计入报告。自检进行中再点一次会被拒绝。
