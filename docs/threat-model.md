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

| 无声断开的连接占住频道（`channel_full`），配对设备一直连不上 | 中继每 10 s ping 每条连接，25 s 没有任何消息就关闭它并通知对方；客户端在 `channel_full` 时每 4 s 重新 attach（`docs/pairing.md`「重连」）。0.1.0 起 App 不再在局域网上监听配对与会合端口 | `regression_a_connection_gone_silent_gives_up_its_place_on_the_channel`、`regression_a_path_that_breaks_without_a_word_heals_on_its_own` |
| 本机语音服务被其他用户、网页或局域网滥用（`docs/dictation.md` §23） | 默认只监听回环地址，`voltip-server` 监听其他地址必须加 `--allow-remote`（并警告明文），App 只监听 127.0.0.1；除 `/healthz` 外每个请求都要 32 字节随机令牌（0600 文件，常量时间比较，可随时更换），没有关闭认证的选项；不返回 CORS 头，网页读不到响应；并发与排队有上限，时长、请求体和字段有界限，原始上传不落盘；日志不记文字、音频和令牌；无头服务端只读钥匙串与 App 的文件；没有钥匙串时可以从 `VOLTIP_KEY_*` 环境变量读取密钥（只在 voltip-server，文档建议 0600 的 EnvironmentFile；同一用户的进程能读到服务的环境变量，这一点写在文档里），`--check` 和日志只给变量名 | `the_token_guards_everything_but_the_health_check`、`malformed_and_oversized_requests_are_refused`、`a_waiting_request_reads_nothing_until_it_has_a_permit_and_a_full_queue_is_refused`（`voltip-serve`）；`the_command_line_names_the_service_and_its_one_shot_actions`、`print_token_creates_the_token_once_and_prints_it_again`（`voltip-server`）；`peek` 的零写入测试（`voltip-identity`）；`keys_from_the_environment_win_over_the_keychain_and_are_named_never_printed`（`voltip-server`） |
| 代理看到或改动 Voltip 的流量（`docs/dictation.md` §24） | 代理只转发：`https://` 的请求（内置服务、服务商的默认地址、更新源、反馈服务）仍是端到端的 TLS（证书照常校验），中继里的内容还有 Noise E2EE，代理能看到的是要连接的主机和流量的大小与时间。用户自填的 `http://` 自定义端点（远程地址也允许）经过代理时是明文，代理能读到并改动录音、文字和密钥，与没有代理时网络路径上的风险相同；隐私页提醒改用 `https://` 或把它列入不使用代理的地址；本机地址（本地模型服务、本机语音服务）从不发给代理；手动代理的密码只在钥匙串，`settings.json`、界面、状态、日志里都没有；日志与错误不记代理的地址和用户名密码；`socks5h` 让代理解析域名，`socks5` 在本机解析 | `a_stream_tunnels_through_an_http_proxy_with_its_credentials`、`a_local_stream_goes_direct_and_a_refused_tunnel_names_no_host`、`a_client_follows_the_plan_from_one_request_to_the_next`（`voltip-net`）；`the_relay_connects_through_the_saved_proxy_with_the_keychain_password`（`voltip-core`）；`no proxy applies at once; a manual proxy is checked, saved tidy, its password kept out of view`（桌面） |
| AI 工具接入（本机 MCP 服务，`docs/dictation.md` §25）被其他程序、网页或历史里的提示注入滥用 | 默认关闭，只监听 127.0.0.1，没有监听其他地址的选项；除 `/healthz` 外每个请求都要 32 字节随机令牌（0600 文件，与本机语音服务的令牌分开，常量时间比较，可随时更换，换后立即生效）；`Host` 只认 `127.0.0.1:{port}` / `localhost:{port}`（防 DNS rebinding），带 `Origin` 的请求一律拒绝，不返回 CORS 头，网页读不到响应；读取分三级（仅词典和规则 / 替换汇总（缺省）/ 完整历史），完整历史要用户在确认框里显式允许并限定时间范围，权限在每次调用时检查、收回立即生效；修改缺省只能写词典，外部工具新建的规则在用户启用前不执行（历史文本是不可信数据，可能带提示注入，规则会改写插入到任何应用里的文字），没有删除工具，每次写入都记在活动记录里并可撤销；最多 16 个会话、每会话每秒 10 次调用、单次结果 256 KiB；日志不记请求和结果里的文字；令牌与接入命令由核心放进剪贴板，不经过 webview | `crates/voltip-mcp/tests/http.rs@requests_without_the_token_or_from_a_web_page_are_refused`、`crates/voltip-mcp/tests/http.rs@the_levels_of_the_moment_decide_each_call`、`crates/voltip-mcp/tests/http.rs@a_clients_rule_waits_and_its_writes_can_be_undone`、`crates/voltip-mcp/tests/http.rs@sessions_and_calls_are_limited`、`crates/voltip-mcp/tests/http.rs@a_new_token_and_switching_off_take_effect_at_once`、`crates/voltip-mcp/tests/http.rs@the_logs_hold_no_text`；`crates/voltip-core/tests/mcp.rs@reads_follow_the_read_level_of_the_moment`、`crates/voltip-core/tests/mcp.rs@a_tools_rule_waits_for_approval_and_runs_once_approved` |

## 已知缺口

- `forward` 洪泛：Relay 对已加入会话 / 频道的转发不限速；应加每连接字节 / 帧速率上限。
- 频道标签可被知道两把公钥的人计算出来（公钥不是秘密），攻击者可占位造成 `channel_full`；缓解：attach 需要对频道所有者的签名挑战（下一版协议）。
- 桌面端 keyring 在 Linux 依赖 Secret Service（无守护进程时 `StoreUnavailable`，应用拒绝降级到明文文件）。
- 本机语音服务的令牌对同一用户的所有进程可读（这就是给其他程序用的）：能读到这个文件的程序就能使用服务，包括它背后的内置服务额度。`--allow-remote` 时令牌以明文 HTTP 传输，应改用 SSH 隧道或 TLS 反向代理。
- AI 工具接入的令牌同样对同一用户的所有进程可读，复制时会经过剪贴板（其他程序可能读剪贴板）；持有令牌的程序能在授权范围内读写。AI 工具读到的内容会发给它所用的模型服务，这一点由完整历史的确认框说明，Voltip 无法约束。
