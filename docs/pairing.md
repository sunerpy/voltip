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
- 从终态直接重来：`pairing_start`（电脑的「再配一台」「重新开始」、Ctrl R）和手机的加入遇到已结束的会话（Trusted / Expired / Rejected / Failed）时，核心先替它 `Reset`，不用界面先发 `pairing_reset`。

## 手机端（Responder）

```
Idle → Joining(code|ticket) → KeyExchange → AwaitingVerification → Trusted
                         ↘ Failed(invalid_code|expired|rate_limited)
```

## 局域网发现

LAN 主机在跑、「局域网发现」开着（`Settings.lan_discovery`，默认开；电脑在「手机」页、手机在「本机」页切换，命令 `settings_set_lan_discovery`）时，每台设备用 mDNS 把自己公布成 `_voltip._tcp.local.` 服务，端口就是 LAN 主机的端口，同时浏览别人的公布（`voltip_core::discovery`，mdns-sd；Android 先拿 `WifiManager.MulticastLock`）。TXT 记录：

| 键 | 值 |
|---|---|
| `v` | `1`，记录格式；别的版本不认 |
| `fp` | 公钥 SHA-256 的前 8 字节，16 位大写十六进制（设备列表里的指纹去掉分隔符） |
| `n` | 设备名，截到 250 字节以内 |
| `pl` | 平台：`windows` / `macos` / `linux` / `android` / `ios` / `other` |
| `t` | 等待配对时的配对票据，去掉了中继地址和局域网地址 |
| `r` | `1`：这个配对会话在中继上等待 |

- **点一下配对**：手机配对页的「附近的电脑」列出还没配对的电脑，等待配对的排在前面。点「配对」（`pairing_join_nearby { fingerprint }`）后，会话在中继上等（`r=1`）就经手机自己的中继加入，和扫码一样；否则直连它被看到的 IPv4 地址。之后照常核对安全码：同一局域网里谁都能看到并加入这个会话，就像谁都能看到屏幕上的二维码，决定信任谁的是安全码。
- **找回已配对的设备**：看到已配对设备时，它此刻的地址排在存下的地址之前拨号，退避重置。地址只是提示：Noise 握手照样验证身份密钥，只有认证过的对方自己发来的 `device_info_update` 才会写进记录。
- **记录里没有的**：票据不带中继地址，所以记录总能放进一条 TXT 字符串（250 字节），中继地址也不在局域网里广播。公布的只有名称、平台和指纹。关掉开关就撤回公布、停止浏览，`UiState.nearby` 清空，只能扫码或输码配对。

## 常开配对

电脑「手机」页配对面板里的「常开配对」开着时（`Settings.pairing_always_on`，默认关，命令 `settings_set_pairing_always_on`；只在电脑上可用，手机的核心拒绝），这台电脑一直有一个配对会话在等手机（`voltip_core` 的 `runtime/always_on.rs`，每个 tick 检查一次）：

- 打开开关或开着启动时马上开一个会话。中继还没连上就先开在 LAN 主机上，中继连上后换成中继上的会话，不在同一局域网的手机也能扫码或输码加入。
- 等待中的会话剩 10 秒时换成新的，界面上不会出现「已过期」，手机拿到的码至少还有 10 秒。
- 一次配对结束（已信任、被拒绝、失败）后，结果在屏幕上停 4 秒，再开下一个会话；开不了（没有中继也没有 LAN 主机）就每 5 秒再试一次，不弹错误。
- 每次配对照样要在电脑上核对安全码并确认：常开只是让窗口一直开着，不会自动信任谁。面板上没有「取消」，由开关关闭窗口。
- 关掉开关时，还没人加入的会话随即关闭；已经有手机加入的配对照常走完。

## Relay 侧防护

- 短码空间 10^6；每连接对 `join_by_code` 失败上限 5 次后断开，每 IP 每分钟 20 次；每会话失败尝试上限 10 次后会话作废（发起方收到 `error{session_expired}`，UI 提示重新生成）。
- 会话 TTL 默认 120 s，最大 300 s；成功 `join` 后短码立即从索引删除。
- 所有计数与 TTL 都在 `voltip-relay::RelayState` 里，单测可直接推进时钟。

## 解除配对

- 任何一边「忘记」对方时，如果对方此刻在线，先经加密通道发一条 `unpair`（`docs/protocol.md`），再删掉记录、断开会话和直连拨号。对方收到后同样忘记发送方（不回发），发出 `Unpaired` 事件，界面提示「「名字」解除了与这台设备的配对」，设备列表随之更新。
- 这条消息只在认证过的会话里出现：能发出它的只有已配对的那一方，它最多只能让对方忘记它自己。
- 对方离线时收不到通知，会一直留着这条记录，直到下一次连接因为这边已不再信任它而失败；重新配对即可恢复。
- 同步（`docs/dictation.md` §20.8）：手机忘记电脑、或在线时收到电脑的 `unpair`，删除这台电脑同步到手机的副本；电脑在手机离线时忘记它，手机上的副本保留，直到在手机上忘记这台电脑（电脑的确认框写明这一点）。

## 同步与重新配对（2026-10-02）

- 可信记录多两个字段：`sync`（电脑上这部手机的「同步」开关，旧记录读作开）和 `sync_gen`（每开、关一次加一）。同一把公钥重新配对时，替换旧记录的 `trust()` 把 `sync_gen` 加一、`sync` 回到开；先忘记再配对时从 0 开始。手机在每次配对完成时把这台电脑的代次清零、删除旧副本，所以只接受这次配对之后电脑发出的代次。
- 一台电脑最多与 5 部手机同步（`MAX_SYNC_PEERS`）：第 6 部配对时开关默认关；升级时开着同步的手机超过 5 部，按配对时间保留最早的 5 部。
- 握手只接受长度对的载荷：传输帧永远不是握手消息的长度（`docs/protocol.md` §3），空闲的路径只在收到 32 字节的第一条消息时开始应答，等第三条消息的应答方收到新的第一条消息就重新应答，其余丢弃。所以对端断开后频道保留、`session_id` 复用时，旧会话还排在链路队列里的帧到了新连接上，也不会打断新握手。
- 中继路径的握手失败或超时后，发起方在对端还在频道上时自动重来：2 秒后，之后每次加倍，最长 60 秒，成功后回到 2 秒。
- 发起方发出第三条消息后就有了加密通道，但应答方可能在它到达之前已经超时放弃。所以发起方在新通道上第一次解开对方的消息之前，通道算作未确认；超过握手时限仍未确认，就按握手失败处理并重来。应答方的加密通道收到新的第一条消息（32 字节，传输帧不会是这个长度）时，丢弃旧通道、重新应答。否则两边会一直丢弃对方的消息，直到某一方的连接断开。

## 忘记设备后重新配对（2026-10-02 修正）

中继（以及每台设备自己的局域网主机）把一条连接留在频道上，直到这条连接断开；中继没有离开频道的帧，同一连接再次 attach 同一频道会得到 `session_already_active`。频道名由两把公钥决定（`rendezvous_channel`），同一对设备重新配对时还是这个频道。此前忘记设备时只删掉了会话号，所以不重连就立即重新配对时，attach 被拒，两边都要等任一方的中继连接重建才能上线；局域网主机的回环连接不会自己重连，经局域网重新配对的设备要等应用重启。

现在（`ParkedChannel`，纯客户端，中继不变）：

- 忘记设备时，中继和本机局域网主机上的频道连同会话号一起保留（直连拨号随之关闭，不保留）；对方是否在线随 `peer_presence` 更新，期间收到的最后一个载荷也留着。这条连接断开时一并丢弃，因为中继也已把它移出频道。
- 重新信任这台设备时，attach 直接接回保留的频道，不再发 `attach`：对方在线就开始握手，留下的载荷（先一步重新信任本机的对方发来的第一条握手消息）随后交给握手。
- 一方忘记、另一方仍信任时重新配对，仍信任的一方已在频道上，同样不再 attach，而是重新握手。
- attach 已发出、`attached` 还没回来时就忘记了对方，回来的频道照样保留。

回归测试：`crates/voltip-core/tests/e2e.rs@regression_a_device_forgotten_and_paired_again_at_once_comes_online_over_the_relay`、`regression_a_device_forgotten_and_paired_again_at_once_comes_online_on_the_lan`（修正前两条都等不到上线）。

## 连接自检

设备页（电脑）和已配对设备屏（手机）上的「连接自检」按一次跑一轮，结果是那一刻实际测到的（`voltip_core::connectivity`，`connectivity_check` 命令 → `connectivity` 事件，`UiState.connectivity` 保留上一次的报告）：

- **本机局域网监听**：LAN 主机是否在跑，以及它向已配对设备公布的 `ip:port`。
- **中继**：对配置的中继新建一条连接，走完 `hello` / `hello_ack` 再关掉，报告耗时，或者没有响应 / 拒绝连接 / 连接失败。内置中继连接失败时，原因里可能带着地址，所以只报类型、不带原因（地址是构建期的 secret，不进界面）。
- **每台已配对设备**：在线的，经它当前的加密通道发一次 `ping`，报告往返时间；它公布过的每个局域网地址都用同样的 `hello` 探测一次。和本机同一个 IPv4 /24 网段却连不上时，提示可能是对方防火墙拦住了端口，或者 Wi-Fi 开启了客户端隔离。

每个探测最多 3 秒，全部并行；4 秒后没回应的 `ping` 按没有回应计入报告。自检进行中再点一次会被拒绝。
