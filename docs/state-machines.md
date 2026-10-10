# 状态机总览

## Transport（`voltip-transport::ConnectionState`）

```
Disconnected → Connecting → Authenticating → Connected → Reconnecting → (Connecting…)
                     ↘ Closed（用户主动或不可恢复错误）
```

- `Connecting`：TCP/WS 建连，超时 `connect_timeout`（默认 8 s）。
- `Authenticating`：Relay `hello/hello_ack`。
- `Connected`：心跳 `ping` 每 15 s，`pong` 超时 10 s → `Reconnecting`；`reconnect_now` 立即发一个 ping，5 s 内没有 pong 也 → `Reconnecting`（`docs/pairing.md`「重连」）。
- `Reconnecting`：指数退避 `min(10 s, 0.5 s·2^n) ± 20% 抖动`，`max_attempts` 默认无限（可配置）；`reconnect_now` 跳过剩余的等待；期间 UI 显示 `Reconnecting`。
- 任何状态 `Close` → `Closed`（终态）。

## 设备连接状态（UI 可见，`DeviceConnection`）

`Offline` / `Connecting` / `Online` / `IdentityChanged`

## 路径（只走中继）

- 每台设备只有一条到中继的链路（`LinkId::Relay`），配对和已配对设备的会合频道都在上面（`docs/pairing.md`「只走中继」）。
- **配对**：中继已连接 → 会话建在中继上，二维码与 6 位码都可用，票据带 `relay_hint`；中继未开启或未连上 → 直接报原因，不开会话。
- **成为可信设备后**：在中继链路上 `attach` 到会合频道并 `leave` 配对会话。
- **每台可信设备一条路径**：频道上走一次 Noise XX 重新握手；UI 汇总为 `IdentityChanged`（身份不符，绝不被掩盖）> `Online` > `Connecting` > `Offline`。
- **链路断开**：清理所有频道会话；重新连上后对每台可信设备重新 `attach`，`channel_full` / `rate_limited` 时稍后重试（`docs/pairing.md`「重连」）。
