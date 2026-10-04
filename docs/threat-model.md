# 威胁模型

假设：公共 Relay **已被攻击者完全控制**；网络可被劫持；攻击者可以枚举短码、重放任何看到的帧。

| 威胁 | 缓解 | 验证 |
|---|---|---|
| Relay 读取用户数据 | 应用层 Noise XX E2EE；Relay 只见 `forward.payload` 密文 | `relay_never_sees_plaintext` 集成测试：中继端截获全部帧，断言不含明文与静态公钥 |
| Relay 伪造已信任设备 | 静态身份公钥在握手内加密传输并由 Noise 认证；TrustedDevice 记录公钥；重连比对 | `identity_change_is_flagged` 回归测试 |
| MITM（含 Relay 替换握手） | XX 模式双向认证 + 用户核对 Safety Code（来自 handshake_hash，攻击者无法让两端一致） | `mitm_yields_different_safety_codes` |
| 短码暴力枚举 | Relay 限流：连接 5 次 / IP 20 次·分钟 / 会话 10 次；TTL 120 s | `brute_force_locks_session`、`ip_rate_limit` |
| 重放票据 / 短码 / 会话 | 一次性 `session_id`；`join` 成功即删除短码；票据 `nonce` 记录；过期即销毁 | `replayed_ticket_rejected`、`expired_session_rejected` |
| 长期私钥泄露 | 只存平台安全存储（keyring / Keystore）；不写配置、不写 DB、不出设备；日志脱敏 | `secret_store_never_serialises_private_key`、代码审计：`grep -r private` 无明文序列化 |
| 客户端内嵌高权限 secret | 客户端只有 Relay URL（非 secret）；签名/发布凭据只在 GitHub Secrets | CI 配置审查 |
| 协议降级 | `version` 强制校验；不支持则断开 | `unsupported_version_disconnects` |
| DoS 单连接刷会话 | 每连接最多 1 个活动会话；`create_session` 每 IP 每分钟 10 次 | `session_create_rate_limit` |

| LAN 主机暴露在局域网 | 主机只承载配对会话与会合频道；`attach` 者仍要过 Noise XX，身份不符即 `IdentityChanged`；频道最多 2 人；`direct_hints` 只接受 `ip:port` 字面量（不解析主机名），且只来自认证通道 | `lan_host_mode_keeps_one_pairing_session_but_serves_channels`、`direct_hints_are_bounded_and_must_be_socket_addrs`、`regression_relay_compromise_impostor_on_channel_is_flagged_not_trusted` |
| 恶意 peer 让我拨任意 `ip:port` | hints 上限 4 个、每 tick 每设备最多一条出站连接、指数退避、连接超时 3 s；连接只做 WebSocket + 本协议 `hello` | `direct_hints_persist_survive_retrust_and_are_validated` |
| 本机语音服务被其他用户、网页或局域网滥用（`docs/dictation.md` §23） | 默认只监听回环地址，`voltip-server` 监听其他地址必须加 `--allow-remote`（并警告明文），App 只监听 127.0.0.1；除 `/healthz` 外每个请求都要 32 字节随机令牌（0600 文件，常量时间比较，可随时更换），没有关闭认证的选项；不返回 CORS 头，网页读不到响应；并发与排队有上限，时长、请求体和字段有界限，原始上传不落盘；日志不记文字、音频和令牌；无头服务端只读钥匙串与 App 的文件 | `the_token_guards_everything_but_the_health_check`、`malformed_and_oversized_requests_are_refused`、`a_waiting_request_reads_nothing_until_it_has_a_permit_and_a_full_queue_is_refused`（`voltip-serve`）；`the_command_line_names_the_service_and_its_one_shot_actions`、`print_token_creates_the_token_once_and_prints_it_again`（`voltip-server`）；`peek` 的零写入测试（`voltip-identity`） |

## 已知缺口

- `forward` 洪泛：Relay 对已加入会话 / 频道的转发不限速；应加每连接字节 / 帧速率上限。
- 频道标签可被知道两把公钥的人计算出来（公钥不是秘密），攻击者可占位造成 `channel_full`（公共 Relay 与 LAN 主机同样适用）；缓解：attach 需要对频道所有者的签名挑战（下一版协议）。
- 桌面端 keyring 在 Linux 依赖 Secret Service（无守护进程时 `StoreUnavailable`，应用拒绝降级到明文文件）。
- 本机语音服务的令牌对同一用户的所有进程可读（这就是给其他程序用的）：能读到这个文件的程序就能使用服务，包括它背后的内置服务额度。`--allow-remote` 时令牌以明文 HTTP 传输，应改用 SSH 隧道或 TLS 反向代理。
