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
| AwaitingVerification | 握手完成，拿到 safety code | `UserConfirm`, `UserReject`, `PeerProof`（电脑）, `PeerConfirm`, `PeerReject`, `Tick` | 120 s → Expired |
| Trusted | 双方都确认（扫码时电脑核对手机的证明后自行确认，见「扫码配对只在手机上确认」） | — | — |
| Expired / Rejected / Failed | 终态；`Reset` 回 Idle | `Reset` | — |

设计约束：
- 状态机是 **sans-IO**：`step(event) -> Vec<Action>`，Action 包括 `SendRelay(frame)`、`SendPeer(bytes)`、`ShowSafetyCode`、`StoreTrusted`、`Emit(ui_event)`；所有网络/时间由调用方注入（`now: Instant`），因此可以在单测里穷举路径。
- 一次性：`session_id`、`code`、`nonce` 在进入 `KeyExchange` 时从 `WaitingForPeer` 的可接受集合中移除；重复 `PeerJoined` → `Failed(replay)`。
- 过期：`Tick(now)` 驱动；过期后收到任何对端消息一律忽略并回 `error{session_expired}`。
- 界面看得到的变化都推送：状态变化，或状态不变而确认情况变化（本机确认、对方确认、收到证明）时，`step` 都返回 `Emit`。2026-10-27 前只在状态变化时推送，两个界面都看不到「已确认」：手机的「确认」点了也没有反应，也不提示还要在电脑上确认（issue #193）。
- 从终态直接重来：`pairing_start`（电脑的「再配一台」「重新开始」、Ctrl R）和手机的加入遇到已结束的会话（Trusted / Expired / Rejected / Failed）时，核心先替它 `Reset`，不用界面先发 `pairing_reset`。

## 手机端（Responder）

```
Idle → Joining(code|ticket) → KeyExchange → AwaitingVerification → Trusted
                         ↘ Failed(invalid_code|expired|rate_limited)
```

## 扫码配对只在手机上确认（issue #193）

用户 2026-10-27 决定：扫码配对只在手机上确认，电脑不用再点（「只在手机端确认即可，反正电脑端可以移除设备，只要保管好配对二维码和密钥即可」）。issue #193 的用户扫码后在手机上点了确认，没有在电脑上再点，两分钟后配对过期。

- **二维码里的密钥**：票据多一个 `secret`，每个会话新生成的 32 字节随机数，只在电脑屏幕上的二维码和「复制链接」里，从不经过中继（`docs/protocol.md` §2）。
- **证明**：手机握手完成后立即发 `pair_proof`：`HMAC-SHA256(secret, "voltip/pairing-proof/v1" ‖ handshake_hash)` 和本机的设备信息（`voltip_crypto::pairing_proof`）。电脑核对通过就自行确认（发 `pair_confirm`，`Snapshot.scanned` 为真），界面写明「已扫码，请在手机上确认」，只留「拒绝」；手机显示电脑的名字，点「确认并信任」即完成配对。证明不对、重复或不该收到时配对失败。证明绑定这一次握手，换一次握手就无效。
- **为什么要证明**：会话号是中继分配的，握手第一条消息里的临时公钥也经过中继，所以只凭「有人加入了会话」，电脑分不清扫码的手机、中继本身和看到 6 位验证码的人；电脑若一律自行确认，这些人都能冒充手机配对，在电脑的光标处输入文字。只有读过二维码的设备才知道 `secret`。
- **手机这边**：票据带着电脑的临时公钥，握手第一条消息必须与它一致，所以手机知道连上的就是屏幕上的那台电脑，不必再核对安全码。两边仍显示安全码，想核对可以核对。
- **6 位验证码**：验证码由中继分配，证明不了手机看过屏幕，所以照旧两边核对安全码、两边确认。手机确认后写明「请在电脑上也点「确认配对」」。
- **版本**：0.1.9 及以前的电脑生成的二维码没有 `secret`，新手机扫到后照旧两边确认；旧手机不认识 `secret`、不发证明，新电脑照旧等它的用户确认。
- **代价**：二维码有效期内，看得到它的人（截图、拍照、共享屏幕）不经电脑确认就能配对。电脑的配对卡片写明这一点，提醒不要截图或在共享屏幕上展示；配对后设备出现在设备列表里，可以随时忘记。常开配对同样如此。

回归测试：`crates/voltip-pairing/tests/flow.rs@regression_a_scanned_ticket_needs_only_the_phones_confirmation`、`a_proof_under_another_secret_fails_the_pairing`、`a_replayed_proof_fails_the_pairing`、`without_a_proof_both_users_confirm`；`crates/voltip-core/tests/e2e.rs@regression_a_phone_that_scanned_pairs_with_only_its_own_confirmation`、`regression_each_confirmation_reaches_both_screens`。

## 只走中继（0.1.0）

用户 2026-10-10 决定：移除局域网连接，手机和电脑只经中继配对和连接。此前的 LAN 主机（`DirectHost`）、mDNS 局域网发现（「附近的电脑」、`Settings.lan_discovery`、Android 的组播锁）、直连拨号和票据里的局域网地址都已删除。

- 配对只用二维码或 6 位验证码，会话在中继上；没有中继时，开始配对和加入配对都直接报原因（「配对需要中继，请先在设置中开启中继」或「尚未连上中继，请检查网络后再试」）。手机的配对页显示中继连接的状态，没连上时写明上次失败的原因并提供「连接自检」（issue #193，2026-10-27 起；此前手机只在输入验证码或扫码后才说「尚未连上中继」）。
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
- **一个名字的多个地址**（issue #193，2026-10-27）：中继的域名解析出几个地址时，直连按 RFC 8305 的做法依次尝试：先连第一个，250 ms（`voltip_net::ATTEMPT_DELAY`）内没有连上或已经失败，就同时开始下一个，最先连上的那个胜出、其余放弃，整个过程仍受 8 秒连接时限约束。此前 `TcpStream::connect` 在同一个时限内逐个尝试，第一个地址不回应时 8 秒全耗在它上面，后面的地址轮不到，下一次重试多半还是先试它。2026-10-22 内置中继的负载均衡换了节点，其中一个地址从中国大陆连不上，大陆的手机先试到它时就一直「尚未连上中继」；2026-10-27 已把这个节点所在的可用区从负载均衡移除。
- **失败原因**：每次连接失败或连接断开，链路先报告原因的类别（`LinkEvent::Failed`，`voltip_transport::FailureKind`：`timeout` 超时、`dns` 无法解析、`refused` 被拒绝、`unreachable` 网络不通、`tls` 加密握手失败、`not_relay` 不是 Voltip 中继、`proxy` 代理失败、`version` 版本不支持、`too_old` 版本过低、`ticket` 设备未通过验证（「设备票据」）、`lost` 连接中断、`other`），再报告 `Reconnecting`。核心把它记在 `RelayStatus.last_failure`（类别、时间），连上后清除；用户自己填的中继同时带上原话（`detail`），内置中继的原话可能含有地址，只报类别。

回归测试：`crates/voltip-core/tests/e2e.rs@regression_a_path_that_breaks_without_a_word_heals_on_its_own`（手机的路径无声中断，心跳发现、重连、频道被占、中继放掉旧连接、重试 attach 后两边恢复）、`reconnect_relay_checks_the_socket_at_once_after_the_network_changed`、`crates/voltip-transport/tests/link.rs@reconnect_now_cuts_a_backoff_short`、`reconnect_now_drops_a_socket_that_no_longer_answers`。

## 中继侧的连接检测

中继服务器（`voltip_relay::server`）每 10 秒向每条连接发一次 WebSocket ping（`RelayConfig::ping_interval`），25 秒内收不到这条连接的任何消息（帧或 pong）就关闭它（`RelayConfig::idle_timeout`），并照常通知频道另一方对端离线。所有版本的客户端都会自动回 pong，所以这一项不依赖客户端升级。没有这项检测时，一条无声断开的连接会一直占着频道位置，直到前面的反向代理超时（ALB 和 nginx 都是 3600 秒）。

回归测试：`crates/voltip-relay/tests/socket.rs@regression_a_connection_gone_silent_gives_up_its_place_on_the_channel`、`a_client_that_only_answers_pings_stays_connected`。

## 常开配对

电脑「手机」页配对面板里的「常开配对」开着时（`Settings.pairing_always_on`，默认关，命令 `settings_set_pairing_always_on`；只在电脑上可用，手机的核心拒绝），这台电脑一直有一个配对会话在等手机（`voltip_core` 的 `runtime/always_on.rs`，每个 tick 检查一次）：

- 打开开关或开着启动时马上开一个会话；中继还没连上时等它连上再开。
- 等待中的会话剩 10 秒时换成新的，界面上不会出现「已过期」，手机拿到的码至少还有 10 秒。
- 一次配对结束（已信任、被拒绝、失败）后，结果在屏幕上停 4 秒，再开下一个会话；开不了（中继没有连上）就每 5 秒再试一次，不弹错误。
- 扫码的手机只需在手机上确认（「扫码配对只在手机上确认」），输入验证码的仍要在电脑上核对安全码并确认；常开只是让窗口一直开着。面板上没有「取消」，由开关关闭窗口。
- 关掉开关时，还没人加入的会话随即关闭；已经有手机加入的配对照常走完。

## Relay 侧防护

- 短码空间 10^6；每连接对 `join_by_code` 失败上限 5 次后断开，每 IP 每分钟 20 次；每会话失败尝试上限 10 次后会话作废（发起方收到 `error{session_expired}`，UI 提示重新生成）。
- 「每 IP」指客户端的地址（`join` 20 次、`create_session` 10 次、`attach` 30 次，均为每分钟）。中继在反向代理后面时，每条连接的 TCP 对端都是代理，要用 `--trust-proxy` 让中继读代理转来的 `X-Forwarded-For`，否则所有用户共用一份额度（`docs/runbook.md`「中继部署」）。
- 会话 TTL 默认 120 s，最大 300 s；成功 `join` 后短码立即从索引删除。
- 所有计数与 TTL 都在 `voltip-relay::RelayState` 里，单测可直接推进时钟。
- 转发量与连接数的上限见「中继限额」，只让官方构建使用托管中继见「设备票据」。

## 最低版本（2026-10-27）

用户 2026-10-27 要求在管理台设置最低客户端版本，过低的版本连接失败时提示升级；决定：认识新错误码的版本被直接告知，0.1.9 及以前的版本「拦配对和重连」（软拦截）。

- **策略文件**（`voltip_relay::policy`）：`--policy /etc/voltip/relay-policy.json`（管理台的同步程序写，`docs/runbook.md`「中继部署」）和 `--policy-override /etc/voltip/relay-policy.override.json`（运维手写），内容 `{"schema":1,"revision":n,"min_version":"x.y.z"|null,"known_versions":["0.1.9",…],"require_ticket":false,"limits":{…}}`（`known_versions` 最多 64 个，统计用；`require_ticket` 见「设备票据」，`limits` 见「中继限额」；`revision` 由 `GET /healthz` 报告，管理台据此显示生效的设置）。读取规则与内置服务网关的额度配置相同：覆盖文件存在且有效时整份使用；存在但无效时告警并忽略它；主文件有效就使用并记为「上一份有效配置」；主文件无效或缺失时沿用上一份有效配置；从来没有有效配置时不设最低版本。中继每 30 秒（`POLICY_CHECK`）比较两个文件的修改时间与存在与否，变了就重读；管理台保存后最多约 5.5 分钟生效（同步程序 5 分钟一轮）。
- **版本**：`hello.client_version` 里的 `voltip/x.y.z`（或不带前缀的 `x.y.z`）；读不出版本的客户端照常服务。
- **认识新错误码的版本**（0.2.0 起，`hello.features` 带 `client_too_old`，`docs/protocol.md`）低于最低版本时，中继回 `error{code: client_too_old, min_version}` 后关闭连接。
- **旧版本**（0.1.9 及以前，不带 `features`）低于最低版本时照常收到 `hello_ack`，但 `create_session`、`join_by_code`、`join_by_session` 和 `attach` 一律回 `unsupported_version`（`attach` 的答复带上 `channel`）。旧界面配对时显示它已有的「版本不兼容，请更新 Voltip」；0.1.9 收到 attach 被拒后不再重试这个频道。
- **策略提高时**：中继重新判断每条连接。认识新错误码、版本过低的连接收到 `client_too_old` 并被关闭，它的会话和频道随断开结束；旧版本连接已有的配对会话和 attach 保留到断开为止，新的一律拒绝。
- **客户端**（`voltip_transport::link`）：hello 阶段或已连接后收到 `client_too_old` 都是一次失败，类别为 `too_old`（`FailureKind::TooOld`），带上最低版本；之后不按退避重连，而是等 30 分钟（`LinkConfig::too_old_retry`），`reconnect_now`（回到前台、网络变化、连接自检）可以提前重试。核心把它记进 `RelayStatus.last_failure`（`kind: too_old`、`min_version`）；这时开始配对被核心拒绝：「此版本过低，请更新 Voltip 后再配对」。
- **界面**（Penpot「用量与额度」画板 20 / 20a）：电脑侧栏的状态卡片黄灯「中继 · 需要更新」，写明最低版本，按钮为「更新到 x」（有可用更新时，打开更新对话框）或「检查更新」；「手机」页顶部横幅，配对面板显示「无法创建配对」和错误代码。手机的配对页显示「需要更新 Voltip」卡片（Google Play 安装的写「前往 Google Play 更新」，否则走应用内更新），已配对设备行写「无法连接 · 需要更新这部手机」。

回归测试：`crates/voltip-relay/src/core.rs@a_client_below_the_minimum_that_knows_the_error_is_told_and_closed`、`regression_an_older_client_below_the_minimum_may_stay_but_not_pair_or_reconnect`；`crates/voltip-relay/src/policy.rs@a_policy_parses_and_only_a_known_older_version_is_too_old`、`the_override_wins_while_valid_and_a_broken_main_file_keeps_the_last_valid_policy`；`crates/voltip-relay/tests/socket.rs@the_policy_file_turns_old_clients_away_and_stats_count_them`；`crates/voltip-transport/tests/link.rs@a_relay_that_requires_a_newer_version_makes_the_link_wait`、`a_minimum_raised_while_connected_turns_the_link_too_old`；`crates/voltip-core/tests/e2e.rs@a_minimum_raised_on_the_relay_shows_in_the_status_with_the_version`。

## 设备票据（2026-11-03）

问题：托管中继的地址写在公开的代码和安装包里，fork 改一下地址就能用；内置服务的设备号是客户端自己说的。用户的目标是不让别人使用托管中继和内置服务，不收费（2026-11-03 的决定：设备凭据加短期票据，不要账号，不做平台认证，不做闭源模块；先统计，调高最低版本后再拦截）。

- **构建密钥**：每个版本一把，`K_build(v) = HMAC-SHA256(K_master, "voltip-access/build/v1/" ‖ v)`，v 是根 `package.json` 的版本。主密钥只在 `voltip-auth` 的服务器、GitHub secret `VOLTIP_ACCESS_MASTER` 和维护者本机的私有文件里；CI 每条打包腿只有一个步骤读它，派生出这个版本的密钥、遮住后交给构建（`scripts/lib/build-key.sh`），`crates/voltip-access/build.rs` 把它拆成两份编进程序，再编入标记串 `voltip-access:keyed:<版本>`。打包脚本扫描产物：完整的密钥（十六进制或原始字节）出现就失败，标记串缺失也失败（`voltip_scan_build_key`）。没有密钥的构建（fork、PR 的 CI、调试构建）不证明、不换票，其余照常。
- **证明版本**（`POST /auth/v1/prove`，`voltip-auth`，私有仓库 `voltip-admin/auth/`）：设备号用内置服务的 usage-id（`docs/dictation.md` §27）。请求带 `proof = HMAC(K_build(v), "prove|设备号|client|ts|nonce")`，`client` 是 `voltip/x.y.z (平台)`；已有设备密钥时再带同一串的 `mac`。服务端检查版本不低于最低客户端版本、不在管理台停用的版本里（某个版本的构建密钥泄露时停用它）、时间差不超过 24 小时、nonce 没用过、设备没被撤销、新设备没超过每个 IP 每天的登记上限，成功后记下设备证明过的版本，返回设备密钥 `device_secret = HMAC(K_device, "voltip-access/device/v1/" ‖ 设备号)`。设备密钥在电脑和手机的系统钥匙串里（条目 `voltip.access.device`），本机语音服务放在它自己目录的 `serve/access.json`（0600）。
- **换票**（`POST /auth/v1/ticket`）：带设备密钥的 `mac`。票据 `base64url(payload) "." base64url(HMAC(K_ticket, base64url(payload)))`，`payload = {"v":1,"d":设备号,"ver":证明过的版本,"exp":…,"scope":["relay","builtin"]}`，有效期 1 小时。票据里的版本始终是最近一次用构建密钥证明的版本；`client` 报的版本与它不同时答复 `prove_required`，客户端先证明再换票。低于最低版本或被停用的版本答复 `version_not_allowed`（证明时和换票时都查）。
- **客户端**（`voltip_access::AccessClient`）：在后台续票，用到有效期的 75% 时提前换；失败时等 1、2、4……最多 30 分钟，`version_not_allowed` 和 `revoked` 直接等 30 分钟；同一时间只有一个请求。`TicketSource::refresh_now` 让它尽快重试，但两次尝试至少隔 10 秒。请求走内置服务的 HTTPS 客户端（Android 用 Mozilla 根证书，跟随代理设置）。
- **只给构建自带的中继和内置服务**：中继连接只有在 `RelaySource::Builtin` 时才带票据源（`CoreConfig::tickets`），连接自检从不发；内置服务的请求带 `X-Voltip-Ticket`，用户配置的服务商收不到。
- **中继的连接内认证**：客户端 `hello.features` 带 `ticket`；中继配了 `--ticket-key-file` 时 `hello_ack.features` 带 `ticket`。这时客户端发 `auth{ticket}`，收到 `auth_ack{exp}` 后才报告「已连接」，所以 `attach` 和配对请求都在认证之后；拿到新票后再发一次 `auth`。没有票时先请求一张、最多等 5 秒（`LinkConfig::ticket_wait`），还没有就不带票继续；票被拒（`ticket_invalid`）也不带票继续。中继验 MAC、`scope` 含 `relay`、未过期，记下这条连接的到期时间；有票时，最低版本按票据里的版本判断。
- **门禁**（策略 `require_ticket`，默认关）：只有同时设了 `min_version`、中继有票据密钥时才生效。生效后：
  - `hello` 不带 `ticket` 的客户端按低于最低版本处理（认识 `client_too_old` 的收到它并被关闭，更早的版本被软拦截）；
  - 带 `ticket` 但没认证的连接，`create_session`、`join_*`、`attach` 收到 `ticket_required`；`forward` 收到 `ticket_required` 并被关闭；
  - 票据到期而没有新票的连接收到 `ticket_expired` 后被关闭，所以撤销设备后，已经建立的连接最多 1 小时也会断开；
  - 门禁从关到开的那一刻，中继把所有没有有效票的连接当场撤销（`RelayCore::revoke`）：它的配对会话和频道在中继内立即释放，对方收到 `peer_left` / `peer_presence{online:false}`，此后它发来的任何帧只换来关闭，不依赖关闭消息能否送达。服务端的关闭分两步：读取立即停止，writer 在 2 秒内按顺序发完原因和关闭帧，发不出去（客户端不读）就被强制结束，连接一定关闭。撤销之前已经排进它队列的转发帧仍可能在宽限期内发出。
- **客户端收到 `ticket_required` / `ticket_expired`**：请求新票，以失败类别 `ticket` 结束这条连接，按退避重连；重连后重新认证，核心照常重新 `attach`，频道恢复。进行中的配对随断开失败；`last_failure` 为 `ticket` 时开始配对被核心拒绝：「无法验证这台设备，请稍后重试或更新 Voltip」。内置服务开启门禁后不带有效票的请求得到 401 `ticket_required`，界面写「无法验证这台设备，请稍后重试或更新 Voltip」。
- **统计期**：门禁关着时什么都不拒绝，只计数：`auth.ok`、`auth.invalid`、`auth.expired`、`auth.no_feature`（`hello` 不带 `ticket`）、`auth.unauthenticated`（带 `ticket` 却不带票请求会话或频道，每条连接计一次）、`auth.refused`、`auth.revoked`。管理台据此显示有效票的占比，再决定何时打开。

回归测试：`crates/voltip-relay/src/core.rs@regression_turning_the_gate_on_stops_unauthenticated_connections_forwarding`、`under_the_gate_only_authenticated_clients_are_served`、`an_expired_ticket_closes_the_connection_under_the_gate`、`the_minimum_version_reads_the_proven_version`、`without_the_gate_unauthenticated_clients_are_counted_and_served`；`crates/voltip-relay/tests/socket.rs@regression_a_full_queue_does_not_keep_a_cut_off_connection`、`under_the_gate_ticket_required_comes_before_the_close`；`crates/voltip-relay/src/ticket.rs@the_shared_vector_holds`；`crates/voltip-transport/tests/link.rs@regression_ticket_required_refreshes_reconnects_and_the_attach_succeeds`、`regression_a_relay_without_a_ticket_source_gets_no_ticket_and_neither_does_the_probe`、`the_builds_relay_is_shown_the_ticket_before_connected_and_on_renewal`；`crates/voltip-core/tests/e2e.rs@regression_a_relay_the_user_entered_never_gets_the_device_ticket`；`crates/voltip-access/src/tests.rs@the_shared_vectors_hold`；`scripts/check-build-key.sh`（`make verify`）。

## 中继统计（2026-10-27）

管理台只看汇总的计数，不碰端到端加密的内容。

- `GET /stats`（`voltip_relay::stats`）只在中继自己的监听地址上（`127.0.0.1:47830`），前面的 nginx 只转发 `/ws`，外面访问不到；管理台的同步程序在中继主机上读取。
- 内容（`schema` 1）：此刻的连接数、配对会话数、有设备的频道数、两端都在的频道数；最近 48 小时按小时的计数：同时连接数的峰值、`hello` 数及各版本的数目、创建的配对会话、两端会合的会话、过期的会话、被按地址限流的请求、收到 `client_too_old` 的连接、被软拦截的旧版本请求、成功的 `attach`、转发的帧数。
- 不含 IP、频道、会话号和验证码。版本只按策略里 `known_versions` 列出的已发布版本分别计数，其余记为 `other`；策略没有列出版本时，每小时最多分别记 32 个版本（`UNLISTED_VERSION_SLOTS`），内存有上限。
- 统计按连接计数，不按设备：中继没有设备标识，也不该有。中继看不到用户是否确认了安全码（之后只转发密文），所以只统计「两端会合」（两端都加入了会话），不叫「配对成功」。

回归测试：`crates/voltip-relay/src/stats.rs@counts_go_to_their_hour_and_48_hours_are_kept`、`regression_versions_are_named_only_from_the_policy_or_up_to_the_hours_slots`；`crates/voltip-relay/src/core.rs@the_statistics_count_what_happened_and_name_no_device`。

## 中继限额（2026-11-03）

策略 `limits` 的四项，`null` 表示不限，默认全是 `null`：先统计一周，再由管理员在管理台按分布图设数值（用户 2026-11-03 的决定）。

- `forward_bytes_per_minute`、`forward_frames_per_minute`：按连接、按 UTC 整分钟（固定窗口）计，字节数是 `forward.payload` 解码后的长度，被拒绝的转发也计入。超过时中继发 `error{rate_limited, retry_after_secs: 本分钟剩余秒数}` 并关闭连接（Noise 的传输不能只丢一帧），像门禁一样当场撤销它的会话和频道。
- `connections_per_ip`、`max_connections`：新连接到来时，这个地址已有的连接数或总连接数达到上限，中继发 `error{rate_limited}` 后关闭它。地址经 `--trust-proxy` 还原。
- 统计（每小时，加进 `GET /stats`，只增加字段）：每个「连接 × 分钟」只要有过转发就记一个样本，分钟结束或连接关闭时写入；`forward_bytes_hist`（分桶边界 64 KiB / 256 KiB / 1 MiB / 4 MiB / 16 MiB / 64 MiB，加溢出桶）、`forward_frames_hist`（60 / 300 / 1200 / 6000，加溢出桶）、精确的 `forward_bytes_max`、`forward_frames_max`、`limited_forward`、`limited_connections`；每个地址这一小时同时连接数的峰值进 `ip_connections_hist`（1 / 2 / 4 / 8 / 16 / 32，加溢出桶）和精确的 `ip_connections_max`，每小时最多跟踪 4096 个地址，其余计入 `ip_overflow`。分桶边界在报告的 `edges` 里。统计里不出现任何地址。

回归测试：`crates/voltip-relay/src/core.rs@forwarding_over_a_minute_limit_closes_the_connection`、`connection_limits_refuse_new_connections`；`crates/voltip-relay/src/stats.rs@samples_and_peaks_fall_into_their_buckets`；`crates/voltip-relay/src/policy.rs@a_policy_parses_and_only_a_known_older_version_is_too_old`。

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

- **中继**：对配置的中继新建一条连接，走完 `hello` / `hello_ack` 再关掉，报告耗时，或者没有响应 / 拒绝连接 / 连接失败。内置中继连接失败时，原话里可能带着地址，所以只报失败的类别（上一节的 `FailureKind`，例如「无法解析中继地址」），不带原话（地址是构建期的 secret，不进界面）。
- **每台已配对设备**：在线的，经它的加密通道发一次 `ping`，报告往返时间；不在线的报告离线。

中继探测最多 8 秒，与中继连接本身的连接时限相同（2026-10-27 前是 3 秒，网络慢时会把连得上的中继报成没有响应；issue #193 的电脑测得 2.5 秒）。探测有了结果后，4 秒内没回应的 `ping` 按没有回应计入报告；探测本身最多等 9 秒，之后按没有响应计入。自检同时让中继连接立即检查一次（上面的「立即检查」）。自检进行中再点一次会被拒绝。手机还没有配对电脑时，自检在配对页上，中继没连上时显示。

回归测试：`crates/voltip-net/src/tests.rs@regression_a_name_whose_first_address_never_answers_still_connects`、`crates/voltip-transport/tests/link.rs@regression_a_relay_whose_first_address_never_answers_is_reached_through_the_next`、`crates/voltip-core/tests/network.rs@regression_the_self_check_waits_for_a_slow_relay_as_long_as_the_link_does`、`apps/mobile-rn/src/App.test.tsx@regression: the pairing page says why the relay is not connected and runs the self-check (issue #193)`。
