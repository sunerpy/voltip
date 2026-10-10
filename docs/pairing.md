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

## 只走中继（0.1.0）

用户 2026-10-10 决定：移除局域网连接，手机和电脑只经中继配对和连接。此前的 LAN 主机（`DirectHost`）、mDNS 局域网发现（「附近的电脑」、`Settings.lan_discovery`、Android 的组播锁）、直连拨号和票据里的局域网地址都已删除。

- 配对只用二维码或 6 位验证码，会话在中继上；没有中继时，开始配对和加入配对都直接报原因（「配对需要中继，请先在设置中开启中继」或「尚未连上中继，请检查网络后再试」）。
- 旧版本留下的数据照常读取：票据和 `device_info_update` 里的 `direct_hints`、可信记录里的 `direct_hints` 和 `last_connection`、设置里的 `lan_discovery` 在读取时忽略，下次写入时去掉。旧版本电脑在没有中继时生成的二维码（票据不带中继地址）会被拒绝，并提示把电脑上的 Voltip 更新后重新显示二维码。
- 界面不再区分「直连」和「经中继」：设备只显示在线、连接中、离线或身份已变化。
- 代价：同一个 Wi-Fi 下的两台设备也经中继通信，多出一段到中继的往返延迟，并受中继线路波动的影响。下面「重连」一节的机制用来缩短断线后的恢复时间。

## 重连

中继是设备之间唯一的通道，所以中继连接断开后要尽快恢复。断开有两种：连接被关闭（对方或中间节点发来关闭或复位），客户端立即知道；连接在路上断了却没有任何通知（手机切换网络、应用在后台被冻结、跨境线路中断），两端都以为它还在。后一种由下面几项处理：

- **心跳**（`voltip_transport::LinkConfig`）：客户端每 15 秒发一次 WebSocket ping，10 秒内没有对应的 pong（按负载匹配，答复旧 ping 的 pong 不算）就判定连接已死并重连。重连退避从 0.5 秒开始加倍，最长 10 秒，无限重试；连上后计数清零。
- **立即检查**（`RelayLink::reconnect_now`，核心命令 `ReconnectRelay`，界面命令 `relay_reconnect`）：正在退避等待的连接立刻重试；已连接的连接立刻发一个 ping，5 秒内没有 pong 就重连。触发时机：
  - 手机：Android 报告默认网络变化时（`ConnectivityManager` 的网络回调，`onAvailable` / `onLost`），以及应用回到前台时（`VoltipNativeModule.kt`）；
  - 电脑：主窗口收到浏览器的 `online` 事件或从隐藏变为可见时（`useRelayRecheck`）；机器从睡眠中醒来时（核心每个 tick 比较墙上时钟，间隔远大于 tick 就判定睡过，`woke_up`）；
  - 两端：点「连接自检」时。
- **频道被占时重试 attach**：重连后的新连接要重新 attach 到与每台已配对设备的频道上。如果中继还留着本设备那条已死的旧连接，频道两个位置都被占，中继回 `channel_full`。此前客户端只记一条日志、不再重试，两台设备一直连不上，直到某一方的应用重启；2026-10-10 用户反馈的「配对后断开」最可能就是这个原因。现在中继的 attach 答复都带上频道名（`attached.channel`、`error.channel`，0.1.0 起），客户端据此找到对应的设备，`channel_full` 时每 4 秒重试一次（`CoreConfig::attach_retry`），`rate_limited` 时按中继给出的等待时间重试，直到成功或连接断开。旧中继的答复不带频道名时，`channel_full` 和 `invalid_channel` 按顺序对应最早发出的 attach。
- **握手重试**：上一节的握手失败重试上限从 60 秒降到 30 秒。

回归测试：`crates/voltip-core/tests/e2e.rs@regression_a_path_that_breaks_without_a_word_heals_on_its_own`（手机的路径无声中断，心跳发现、重连、频道被占、中继放掉旧连接、重试 attach 后两边恢复）、`reconnect_relay_checks_the_socket_at_once_after_the_network_changed`、`crates/voltip-transport/tests/link.rs@reconnect_now_cuts_a_backoff_short`、`reconnect_now_drops_a_socket_that_no_longer_answers`。

## 中继侧的连接检测

中继服务器（`voltip_relay::server`）每 10 秒向每条连接发一次 WebSocket ping（`RelayConfig::ping_interval`），25 秒内收不到这条连接的任何消息（帧或 pong）就关闭它（`RelayConfig::idle_timeout`），并照常通知频道另一方对端离线。所有版本的客户端都会自动回 pong，所以这一项不依赖客户端升级。没有这项检测时，一条无声断开的连接会一直占着频道位置，直到前面的反向代理超时（ALB 和 nginx 都是 3600 秒）。

回归测试：`crates/voltip-relay/tests/socket.rs@regression_a_connection_gone_silent_gives_up_its_place_on_the_channel`、`a_client_that_only_answers_pings_stays_connected`。

## 常开配对

电脑「手机」页配对面板里的「常开配对」开着时（`Settings.pairing_always_on`，默认关，命令 `settings_set_pairing_always_on`；只在电脑上可用，手机的核心拒绝），这台电脑一直有一个配对会话在等手机（`voltip_core` 的 `runtime/always_on.rs`，每个 tick 检查一次）：

- 打开开关或开着启动时马上开一个会话；中继还没连上时等它连上再开。
- 等待中的会话剩 10 秒时换成新的，界面上不会出现「已过期」，手机拿到的码至少还有 10 秒。
- 一次配对结束（已信任、被拒绝、失败）后，结果在屏幕上停 4 秒，再开下一个会话；开不了（中继没有连上）就每 5 秒再试一次，不弹错误。
- 每次配对照样要在电脑上核对安全码并确认：常开只是让窗口一直开着，不会自动信任谁。面板上没有「取消」，由开关关闭窗口。
- 关掉开关时，还没人加入的会话随即关闭；已经有手机加入的配对照常走完。

## Relay 侧防护

- 短码空间 10^6；每连接对 `join_by_code` 失败上限 5 次后断开，每 IP 每分钟 20 次；每会话失败尝试上限 10 次后会话作废（发起方收到 `error{session_expired}`，UI 提示重新生成）。
- 会话 TTL 默认 120 s，最大 300 s；成功 `join` 后短码立即从索引删除。
- 所有计数与 TTL 都在 `voltip-relay::RelayState` 里，单测可直接推进时钟。

## 解除配对

- 任何一边「忘记」对方时，如果对方此刻在线，先经加密通道发一条 `unpair`（`docs/protocol.md`），再删掉记录、断开会话。对方收到后同样忘记发送方（不回发），发出 `Unpaired` 事件，界面提示「「名字」解除了与这台设备的配对」，设备列表随之更新。
- 这条消息只在认证过的会话里出现：能发出它的只有已配对的那一方，它最多只能让对方忘记它自己。
- 对方离线时收不到通知，会一直留着这条记录，直到下一次连接因为这边已不再信任它而失败；重新配对即可恢复。
- 同步（`docs/dictation.md` §20.8）：手机忘记电脑、或在线时收到电脑的 `unpair`，删除这台电脑同步到手机的副本；电脑在手机离线时忘记它，手机上的副本保留，直到在手机上忘记这台电脑（电脑的确认框写明这一点）。

## 同步与重新配对（2026-10-02）

- 可信记录多两个字段：`sync`（电脑上这部手机的「同步」开关，旧记录读作开）和 `sync_gen`（每开、关一次加一）。同一把公钥重新配对时，替换旧记录的 `trust()` 把 `sync_gen` 加一、`sync` 回到开；先忘记再配对时从 0 开始。手机在每次配对完成时把这台电脑的代次清零、删除旧副本，所以只接受这次配对之后电脑发出的代次。
- 一台电脑最多与 5 部手机同步（`MAX_SYNC_PEERS`）：第 6 部配对时开关默认关；升级时开着同步的手机超过 5 部，按配对时间保留最早的 5 部。
- 握手只接受长度对的载荷：传输帧永远不是握手消息的长度（`docs/protocol.md` §3），空闲的路径只在收到 32 字节的第一条消息时开始应答，等第三条消息的应答方收到新的第一条消息就重新应答，其余丢弃。所以对端断开后频道保留、`session_id` 复用时，旧会话还排在链路队列里的帧到了新连接上，也不会打断新握手。
- 握手失败或超时后，发起方在对端还在频道上时自动重来：2 秒后，之后每次加倍，最长 30 秒，成功后回到 2 秒。
- 发起方发出第三条消息后就有了加密通道，但应答方可能在它到达之前已经超时放弃。所以发起方在新通道上第一次解开对方的消息之前，通道算作未确认；超过握手时限仍未确认，就按握手失败处理并重来。应答方的加密通道收到新的第一条消息（32 字节，传输帧不会是这个长度）时，丢弃旧通道、重新应答。否则两边会一直丢弃对方的消息，直到某一方的连接断开。

## 忘记设备后重新配对（2026-10-02 修正）

中继把一条连接留在频道上，直到这条连接断开；中继没有离开频道的帧，同一连接再次 attach 同一频道会得到 `session_already_active`。频道名由两把公钥决定（`rendezvous_channel`），同一对设备重新配对时还是这个频道。此前忘记设备时只删掉了会话号，所以不重连就立即重新配对时，attach 被拒，两边都要等任一方的中继连接重建才能上线。

现在（`ParkedChannel`，纯客户端，中继不变）：

- 忘记设备时，中继上的频道连同会话号一起保留；对方是否在线随 `peer_presence` 更新，期间收到的最后一个载荷也留着。这条连接断开时一并丢弃，因为中继也已把它移出频道。
- 重新信任这台设备时，attach 直接接回保留的频道，不再发 `attach`：对方在线就开始握手，留下的载荷（先一步重新信任本机的对方发来的第一条握手消息）随后交给握手。
- 一方忘记、另一方仍信任时重新配对，仍信任的一方已在频道上，同样不再 attach，而是重新握手。
- attach 已发出、`attached` 还没回来时就忘记了对方，回来的频道照样保留。

回归测试：`crates/voltip-core/tests/e2e.rs@regression_a_device_forgotten_and_paired_again_at_once_comes_online_over_the_relay`（修正前等不到上线）。

## 连接自检

设备页（电脑）和已配对设备屏（手机）上的「连接自检」按一次跑一轮，结果是那一刻实际测到的（`voltip_core::connectivity`，`connectivity_check` 命令 → `connectivity` 事件，`UiState.connectivity` 保留上一次的报告）：

- **中继**：对配置的中继新建一条连接，走完 `hello` / `hello_ack` 再关掉，报告耗时，或者没有响应 / 拒绝连接 / 连接失败。内置中继连接失败时，原因里可能带着地址，所以只报类型、不带原因（地址是构建期的 secret，不进界面）。
- **每台已配对设备**：在线的，经它的加密通道发一次 `ping`，报告往返时间；不在线的报告离线。

中继探测最多 3 秒；4 秒后没回应的 `ping` 按没有回应计入报告。自检同时让中继连接立即检查一次（上面的「立即检查」）。自检进行中再点一次会被拒绝。
