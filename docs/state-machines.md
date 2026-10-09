# 状态机总览

## Transport（`voltip-transport::ConnectionState`）

```
Disconnected → Connecting → Authenticating → Connected → Reconnecting → (Connecting…)
                     ↘ Closed（用户主动或不可恢复错误）
```

- `Connecting`：TCP/WS 建连，超时 `connect_timeout`（默认 8 s）。
- `Authenticating`：Relay `hello/hello_ack`（或 Direct 的 Noise 握手）。
- `Connected`：心跳 `ping` 每 20 s，`pong` 超时 10 s → `Reconnecting`。
- `Reconnecting`：指数退避 `min(30 s, 0.5 s·2^n) ± 20% 抖动`，`max_attempts` 默认无限（可配置）；期间 UI 显示 `Reconnecting`。
- 任何状态 `Close` → `Closed`（终态）。

## 设备连接状态（UI 可见，`DeviceConnection`）

`Offline` / `Online{via: Direct|Relay}` / `Reconnecting` / `IdentityChanged`

## 路径策略（直连优先，Relay 回退）

- **常驻 LAN 主机**：核心启动即在 `direct_bind`（默认 `0.0.0.0:47831`，被占用则退到临时端口）起一个 `DirectHost`（内嵌 `RelayCore::lan_host()`：同一时刻一个配对会话 + 任意多个频道），并通过 loopback 以普通 `RelayLink` 接到自己的主机上。`direct_enabled=false` 关闭整个 LAN 侧（测试用它模拟局域网不可达）。
- **配对**：Relay 已连接 → 会话建在 Relay 上，二维码与 6 位码都可用，票据同时带 `relay_hint` 和本机 `direct_hints`；Relay 未配置 / 未连接 → 会话建在自己的 LAN 主机上，票据只带 `direct_hints`，手机只能扫码（6 位码没有 Relay 无法定位会话，UI 需提示）。`StartPairing` 在 loopback 链路尚未连上时会排队，链路一到即开始。
- **成为可信设备后**：在承载配对的那条链路上 `attach` 到会合频道并 `leave` 配对会话；同时在自己的 LAN 主机上也 `attach`，让对方以后能拨进来。责任方（扫码方）把票据里的 `direct_hints` 写进可信记录。
- **多路径**：每台可信设备可同时存在三类路径 —— `Relay`（公共 Relay 上的频道）、`Host`（对方拨进我的 LAN 主机）、`Dial(n)`（我拨进对方的 LAN 主机）。每条路径独立走一次 Noise XX 重新握手；UI 汇总规则固定为 `IdentityChanged`（任一路径身份不符，绝不被健康路径掩盖）> `Online·Direct` > `Online·Relay` > `Connecting` > `Offline`。发消息取「安全的 LAN 路径 > 安全的 Relay 路径」。
- **直连优先**：每个 tick，对每台带 `direct_hints` 且没有活跃 LAN 路径的可信设备，最多拨一条出站连接（连接超时 `direct_connect_timeout`，默认 3 s），失败按 `direct_retry`→`direct_retry_max`（默认 5 s→60 s）指数退避，轮换 hints；成功握手或学到新 hints 即重置退避。
- **地址刷新**：每次任一路径握手成功，双方各在加密通道内发 `device_info_update{device, direct_hints}`；对方更新可信记录里的 hints（持久化）并立刻重新拨号。设备换网 / 换端口后，只要有任何一条路径（哪怕只是 Relay）先连上，LAN 地址就会被刷新。
- **回退**：LAN 路径消失（拨号失败、对方主机离线）而 Relay 路径仍在时，UI 立即变为 `Online·Relay`，流量走 Relay；Relay 断开只清理 Relay 路径，LAN 路径不受影响 —— **Relay 关闭后配对设备在同一局域网里照常可用**（`pair_over_lan_without_any_relay` 覆盖双方各重启一次）。
- 回归：`crates/voltip-core/tests/e2e.rs@regression_paired_devices_prefer_direct_and_fall_back_to_relay`（Relay 上配对 → 双端 `Online·Direct`、消息不经 Relay → 手机 LAN 不可达 → 回退 `Online·Relay`、消息经 Relay、桌面端清除失效 hints）。
