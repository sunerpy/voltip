# 通信协议 v1

所有帧都带 `version`（整数，当前 1）与 `type`（snake_case）。未知 `version` → 回 `error{code:"unsupported_version"}` 并断开；未知 `type` → 忽略并记录（前向兼容）。序列化：Relay 控制帧用 JSON 文本帧（便于排障），加密载荷用二进制帧。

## 1. Relay 控制帧（明文，Relay 可读）

| type | 方向 | 字段 | 说明 |
|---|---|---|---|
| `hello` | C→R | `client_version` | 连接后第一帧；不携带任何设备身份 |
| `hello_ack` | R→C | `relay_version`, `limits{code_attempts, session_ttl_secs}` | |
| `create_session` | C→R | `ttl_secs?` | 电脑端创建配对会话 |
| `session_created` | R→C | `session_id`, `code`, `expires_at` | `code` 为 6 位数字，只发给创建者 |
| `join_by_code` | C→R | `code` | 手机输入短码 |
| `join_by_session` | C→R | `session_id` | 手机扫码（票据里有 session_id） |
| `joined` | R→C | `session_id`, `peer_conn` | 加入成功；同一会话第二个加入者被拒 |
| `peer_joined` | R→C | `session_id` | 通知创建者 |
| `leave` | C→R | `session_id` | 配对结束（成功 / 失败 / 取消）释放会话槽；对端收到 `peer_left` |
| `forward` | C→R→C | `session_id`, `payload(base64)` | Relay 原样转发，不解析；≤ 70 KiB |
| `attach` | C→R | `channel` (64 hex) | 已配对设备重逢：`channel = hex(SHA-256("voltip rendezvous v1" ‖ min(pubA,pubB) ‖ max(pubA,pubB)))`；一条连接可 attach 多个频道（每个可信设备一个） |
| `attached` | R→C | `session_id`, `peer_online` | 频道绑定的会话 id（`forward` 用它）与对端是否在线 |
| `peer_presence` | R→C | `session_id`, `online` | 频道对端上线 / 下线 |
| `error` | R→C | `code`, `retry_after_secs?` | `invalid_code` / `session_expired` / `session_full` / `rate_limited` / `unsupported_version` / `not_joined` / `hello_required` / `session_already_active` / `invalid_channel` / `channel_full` / `malformed` |
| `bye` | 双向 | | 优雅关闭 |

Relay 永远看不到：Noise 握手之后的明文、静态公钥以外的身份信息、Safety Code。

## 2. 配对票据（二维码内容）

```
voltip://pair?v=1&t=<base64url(CBOR PairingTicket)>
PairingTicket { version:1, session_id, ephemeral_pub(32B, Noise e), nonce(16B), expires_at(unix s), relay_hint?: url, direct_hints: [ip:port] }
```

- 票据一次性：`session_id` 在 Relay 与发起方内存里都只允许一次 `join`。
- 票据不含长期身份公钥、不含任何长期凭据；`ephemeral_pub` 与 Noise 握手第一条消息里的 `e` 必须一致，否则拒绝（防止 QR 被替换）。
- 6 位短码只是 Relay 侧到 `session_id` 的查找键，本身不参与密钥派生；因此短码泄露只允许攻击者「加入会话」，仍要过 Noise 握手 + 用户核对 Safety Code。

## 3. 端到端层（Relay 不可读）

握手：`Noise_XX_25519_ChaChaPoly_SHA256`，发起方 = 创建会话的电脑端。
- 消息 1 `→ e`：与票据 `ephemeral_pub` 绑定。
- 消息 2 `← e, ee, s, es`：手机端静态身份公钥在此加密送达。
- 消息 3 `→ s, se`：电脑端静态身份公钥送达。
- 双方取 `handshake_hash`（32 B）：
  - `fingerprint = hex(handshake_hash[0..8])` 显示为 `A7:C4:19:8E · 3D:F2:61:09`
  - `safety_code = 4 words`，每词取 `handshake_hash` 连续 11 bit 索引 2048 词表（BIP-39 英文词表）
- 用户在两端确认一致 → `Trusted`；任何一端拒绝 → 发送 `pair_reject` 并销毁密钥。

传输：握手后 snow 进入 transport 模式，每帧 `ciphertext = noise.write_message(plaintext)`，nonce 由 Noise 计数器管理（单调递增、不重用）；`payload` 装进 `forward` 帧。内层明文是 `AppMessage`（同样带 `version`）：`pair_confirm{device}` / `pair_reject{reason}` / `ping{seq}` / `pong{seq}` / `text{body}` / `device_info_update{device, direct_hints}`，以及手机作为麦克风的 `take_start{take, sample_rate_hz}` / `take_audio{take, seq, pcm}` / `take_stop{take}` / `take_cancel{take}` / `take_status{take, state}`（语义与上限见 `docs/dictation.md` §20），以及 `unpair{}`：发送方马上要忘记接收方，接收方收到后也忘记发送方（不回发），界面提示是谁解除了配对。同步（`docs/dictation.md` §20.8）另有 `mirror_request{req, epoch?, since, profile?}` / `mirror_changed{head, generation}` / `mirror_revoke{generation}` / `phone_records_ack{ids}`（≤ 200 个）/ `bulk{seq, last, bytes}`（1..=48 KiB）/ `bulk_ack{received}`，以及 `device_info_update` 的可选 `mirror` 标志（电脑提供同步时为真，旧版不带）。`seal` 在密文会恰好是握手消息长度（32、96、64 字节）时，在 CBOR 之后补一个零字节，所以传输帧永远不与握手消息同长：应答方的加密通道收到 32 字节的载荷，就是发起方重新开始的握手（发起方在新通道上一直没解开对方的消息，见 `docs/pairing.md`）；`decode` 只读一个 CBOR 项，各版本照常读出补过的消息。`direct_hints`（≤ 4 个 `ip:port`，只接受字面地址，不接受主机名）是发送方 LAN 主机当前的监听地址，接收方写进可信记录并用于下一次直连；它只在认证过的通道里出现，Relay 看不到。

## 4. 已信任设备的重连

双方各自 `attach` 到由两把静态公钥推出的频道；静态公钥较小的一方发起 Noise XX 握手（不用 IK，避免把静态公钥暴露给 Relay）；握手完成后比较对端静态公钥与 TrustedDevice 记录：
- 一致 → `Connected`
- 不一致 → `IdentityChanged`，连接立即关闭，UI 显式警告，绝不静默信任。

## LAN 主机（`RelayCore::lan_host()`）

每台设备常驻一个内嵌 `RelayCore` 的 LAN WebSocket 主机（默认端口 47831，被占用退到临时端口）。它与公共 Relay 说完全相同的协议：`single_session=true`（同一时刻只有一个配对会话）、`channels_enabled=true`（配对设备用会合频道在这里重逢）。仅配对用的 `RelayConfig::single_session()` 仍然拒绝 `attach`。设备自己通过 loopback 以普通客户端身份接到主机上，所以核心里「Relay」与「直连」只有一套帧处理代码。
