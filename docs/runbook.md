# 运行手册

## 本地开发

```bash
make relay                      # 中继：ws://127.0.0.1:47830/ws（debug 构建的默认 Relay）
pnpm install
make desktop-dev                # apps/desktop：Vite 1420 + Tauri 窗口
cd apps/mobile && pnpm tauri android dev   # 需要 Android SDK + NDK
```

无 Relay 时（设置 › 关闭 Relay，或未配置 `VOLTIP_RELAY_URL` 的 release 构建）：桌面端「配对」会在局域网起一个 `DirectHost`，手机扫码即可；6 位码需要 Relay。

### 无显示器主机上的交互测试（`make desktop-vnc`）

开发机没有显示器时，用浏览器远程操作一个真实桌面来手测 `cargo tauri dev`：

```bash
sudo apt-get install -y --no-install-recommends tigervnc-standalone-server tigervnc-tools novnc websockify \
  xfce4-session xfwm4 xfce4-panel xfdesktop4 xfce4-terminal dbus-x11 pulseaudio pulseaudio-utils
make desktop-vnc              # 打印 noVNC 地址与密码；会话里的终端自动运行 make desktop-dev（热重载）
scripts/dev-desktop-vnc.sh speak /path/to/sample.wav   # 按住热键时把样音送进假麦克风
make desktop-vnc-stop
```

- `scripts/dev-desktop-vnc.sh` 在 TigerVNC 显示 `:7` 上起 xfce 会话（真实窗口管理器，热键、悬浮胶囊、粘贴都能手测），Xvnc 只监听本机；浏览器入口是 websockify + noVNC，默认监听本机第一个局域网 IPv4 的 6080 端口，每次连接都要 VNC 密码（首次生成在 `~/.config/voltip-dev/`，权限 600）。会话里有终端，拿到密码就等于拿到本用户的 shell：只在内网使用，或设 `VOLTIP_VNC_BIND=127.0.0.1` 后用 `ssh -L 6080:127.0.0.1:6080 <主机>` 访问。
- 麦克风是私有 PulseAudio 的 null sink 监听（远程浏览器的声音传不进来），用 `speak` 播放样音代替说话。应用用 `make desktop-dev` 启动，所以带 `.env.build` 的内置引擎、`VOLTIP_DEV_SECRET_STORE=memory`（会话里没有 Secret Service，身份每次重建），数据目录是本用户真实的 `~/.local/share/voltip`。
- 2026-09-26 实测：会话里按住 Ctrl+Alt+Space 并 `speak` 公开中文样音，经云端 Qwen3-ASR-1.7B + 润色后粘贴成功（历史记录 `inserted / paste`，「开放时间：早上九点至下午五点。」）。

## 环境与端点

服务端主机名、令牌与模型名都在构建时注入编译期 `option_env!`（`docs/dictation.md` §3）；源码、TS、文档里只出现占位符。本机构建从 git 忽略的 `.env.build` 读取同名变量（`scripts/lib/build-env.sh`，`cp .env.build.example .env.build` 后填值），CI 从仓库 secrets 读取（清单与含义见 `.github/README-secrets.md`）。

| 变量 | 含义 |
|---|---|
| `VOLTIP_RELAY_URL` | 中继 `wss://<relay-host>/ws`；release 构建没有它就没有中继（只剩局域网直连），debug 默认 `ws://127.0.0.1:47830/ws` |
| `VOLTIP_ASR_URL` / `VOLTIP_ASR_TOKEN` / `VOLTIP_ASR_MODEL` | 内置识别服务：`https://<asr-host>`、应用令牌、模型名 |
| `VOLTIP_REFINE_URL` / `VOLTIP_REFINE_API_KEY` / `VOLTIP_REFINE_MODEL` | 内置润色服务：OpenAI 兼容基址、应用令牌、模型名 |
| `VOLTIP_UPDATE_PUBKEY` | 更新器公钥；发布工作流据此打开更新器，更新地址由仓库推出（`docs/dictation.md` §9） |
| `VOLTIP_MODEL_BASE_URL` | 可选：本地模型的第一下载源 |

打包脚本（`make windows-x64` / `make linux-x64` / `make android-apk`）与发布候选（`release-candidate.yml` 的 `prepare`）在任一内置引擎值为空时拒绝出包（`scripts/lib/require-builtin-engines.sh`），显式 `VOLTIP_ALLOW_NO_BUILTIN_ENGINES=1` 才放行，CI 的打包 job 就是这样出无内置服务的包。客户端里唯一的凭据是应用令牌；`scripts/build-windows-x64.sh`、release 的 Linux 腿与 Android 包检查（`.github/scripts/check-android-package.sh`）用 `strings` 扫描二进制，出现 `gsk_…` / `sk-…` 即拒绝出包。生产主机名守卫（`.github/scripts/check-no-production-hosts.sh`）从 `VOLTIP_PRODUCTION_HOSTS` 读要找的词，扫描整棵树，命中时只打印 `文件:行号`。

## 自建内置服务

内置服务只需要两个 OpenAI 兼容端点：`POST /v1/audio/transcriptions`（识别）与 `POST …/chat/completions`（润色，基址由 `VOLTIP_REFINE_URL` 决定）。推荐在前面放一个自己的网关：

- 网关校验 `Authorization: Bearer <应用令牌>`，通过后换成真正后端的凭据再转发：识别转给模型服务（例如 vLLM 上的 `Qwen/Qwen3-ASR-1.7B`），润色转给 LLM 服务商。服务商密钥只存在网关上。
- 上传体积上限至少 32 MB，读超时至少 120 s（长录音）；`GET /v1/models` 要能用应用令牌访问，引擎页的「测试连接」用它。
- 不要在网关上落盘音频。
- 轮换令牌：生成新令牌（例如 `openssl rand -hex 24`），网关同时接受新旧两个一段时间，更新 secrets / `.env.build` 后重新打包，旧版本淘汰后再删旧令牌。
- 健康检查：`curl -H "Authorization: Bearer $VOLTIP_ASR_TOKEN" https://<asr-host>/v1/models`。

已知取舍：应用令牌是所有客户端共享的静态令牌，任何拿到安装包的人都能从二进制里提取出来，然后直接调用网关。它只授予识别和润色两类转发，可以随时在网关吊销；公开分发时应在网关上做限流与用量上限，或者换成按用户签发的短期令牌。令牌**不会**跟随用户自定义的地址发出（`docs/dictation.md` §3）。

## 中继部署

```bash
# 静态 musl 二进制，任何 glibc 的 Linux 主机都能跑
cargo build --release -p voltip-relay --target x86_64-unknown-linux-musl
scp target/x86_64-unknown-linux-musl/release/voltip-relay <host>:/usr/local/bin/voltip-relay
```

一种部署方式：

- systemd `voltip-relay.service`：`ExecStart=/usr/local/bin/voltip-relay --bind 127.0.0.1:47830`，`Environment=VOLTIP_RELAY_JSON_LOGS=1`，`DynamicUser=yes`，`Restart=always`；`journalctl -u voltip-relay -f` 看 `pairing session created / peer joined` 等结构化日志。
- nginx vhost `<relay-host>`（Let's Encrypt，acme.sh）：`location /ws` 走 `proxy_http_version 1.1; proxy_set_header Upgrade $http_upgrade; proxy_set_header Connection "upgrade"; proxy_read_timeout 3600s;` 到 `127.0.0.1:47830`；`location /healthz` 直通。中继只监听回环，外网只见 443。
- 校验：`curl https://<relay-host>/healthz` → `ok`；再用真实客户端过一遍握手、配对与双向 E2EE 消息：

```bash
VOLTIP_LIVE_RELAY_URL=wss://<relay-host>/ws cargo test -p voltip-core --test e2e live_relay -- --nocapture
```

该测试未设置变量时自动跳过，离线套件保持封闭。2026-09-25 对一台线上中继实测：TLS + WebSocket 升级 + 会合 + 配对 + 双向消息 0.8 s，nginx 记录两条 `GET /ws … 101`。同一天发现并修复：`voltip-transport` 的 rustls 没有绑定加密 provider，不链接 reqwest 的二进制（移动端、core 测试）拨 `wss://` 会 panic（`crates/voltip-transport/src/endpoint.rs@regression_wss_dial_has_exactly_one_crypto_provider`）。

Relay 无状态（内存里只有活动会话 / 频道），可水平扩展前提是同一对设备落在同一实例（按 `channel` / `session_id` 做粘性路由）。

## 发布

工作流都跑在 GitHub 托管的 runner 上：

- `ci.yml`：push `main` / PR → `changes`（`.github/scripts/changed-code.sh`：只改了 `docs/**`、`*.md`、`LICENSE*` 时 `code=false`；新分支、API 失败或一次推送 300 个以上文件都算改了代码）→ 并行的 `verify-rust`（`VERIFY_GATES=rust`：格式、features、clippy、交叉检查、覆盖率下跑一遍测试、cargo-deny、IPC e2e）与 `verify-web`（`VERIFY_GATES=web`：web 门禁、台账、主机名守卫、发布脚本测试，外加带 `VOLTIP_PRODUCTION_HOSTS` 的守卫；fork 与 Dependabot 的 PR 没有 secrets，跳过这一步）、`codecov`（上传覆盖率，只做报告，不阻塞）、`windows-cross`（`make windows-x64`，无内置服务）→ `windows-native`（Windows Server 上无头运行便携包与安装后的 exe）、`hooks-windows`（Windows 桌面上真实的单键钩子）、`smoke-desktop`（Xvfb 与纯 Wayland 冒烟、X11 真实钩子、无头识别）、`android`（对 Android 目标跑 clippy；构建未签名的 release APK 与 AAB，用当次生成的临时密钥签名，再跑与候选相同的包检查）、`ci-success` 聚合（CI 的总结论）；另有 `candidate-status` 给不是发布 PR 的 PR head 写 `Release candidate = success`（见下面的 ruleset）。只改文档时 `verify-web` 照常跑，其余构建、测试、平台 job 跳过；`ci-success` 只在 `changes` 判定为只改文档时接受跳过，其他任何跳过、失败或取消都判失败。CI 不注入任何内置引擎值。
- CI 的速度（2026-09-28 实测后调整）：Rust 测试只在覆盖率门禁里跑一遍（原先 `cargo test` 与 `cargo llvm-cov` 各编一遍、各跑一遍，各约 170 秒）；`verify` 拆成可并行的两半；Cargo 缓存只从 `main` 保存（PR 的缓存只有同一个 PR 能用，而仓库 10 GB 的上限会把 `main` 的挤掉），不缓存 `~/.cargo/bin`；dev / test 构建只留行号表（`CARGO_PROFILE_DEV_DEBUG=line-tables-only`）；清理预装 SDK 只在剩余空间不足 40 GB 时做（`.github/scripts/free-disk-space.sh`，托管 runner 实测开跑时有 86 GB）；CI 的 Windows 包复用按全部输入（锁文件、清单、`about.toml`、生成脚本与许可证文本）缓存的第三方声明，省掉约两分钟的 cargo-about（`VOLTIP_NOTICES_CACHED=1`，发布总是重新生成）；下载失败重试（`CARGO_NET_RETRY`、`RUSTUP_MAX_RETRIES`）。`verify-*.log` 里每道门都记了耗时（`secs=`）。
- `ci.yml` 的 `macos` job（2026-09-28 从 `macos.yml` 并入）：推送 `main` 或手动运行时在 `macos-15`（Apple 芯片）与 `macos-15-intel`（Intel）上构建 ad-hoc 签名的 `.app` 和 `.dmg`（发布包用固定证书，见下面「macOS 签名」），用 `.github/scripts/check-macos-bundle.sh` 检查，并确认发布用的检查会拒绝这个 ad-hoc 包，跑 macOS 单元测试、真实的事件 tap（仅 Apple 芯片，先在 TCC 里给测试程序授予辅助功能）、无头识别，以及桌面上的 `scripts/smoke-tray-macos.sh`（标题栏左端的内容离红绿灯至少 12 pt，菜单栏图标、菜单与各项操作，截图随产物保存）；不跑在 pull request 上。在 push 和手动运行时 `CI Success` 等它（规则在 `.github/scripts/ci-success.sh`，只有 pull request 上允许它被跳过），所以 `main` 上任何一台 Mac 失败，下一个候选都过不了 `source-gate`；Cargo 缓存从 `main` 保存。
- release-please 的 PR（作者 `github-actions[bot]`、分支 `release-please--branches--main--*`、标签 `autorelease: pending`）和它合并到 `main` 的发布提交只改版本号和 CHANGELOG，只跑 `verify-web`：候选在同一个 head 上构建并检查每个安装包，而 head 的基点已经通过 CI（`.github/scripts/changed-code.sh`）。同样的改动换个作者或没有标签，照常跑完整 CI。
- 发布是「构建一次再晋升」（2026-09-28 用户选定）：安装包在发布 PR 上构建、签名、证明并封存，合并后只校验和发布，不再编译。
  - `release.yml`（控制器）：每次推送 `main`，release-please 维护 Release PR（`release-please-config.json`）。PR head 与 `main` 同步时，`dispatch_candidate` 用 `gh workflow run` 派发 `release-candidate.yml`（`mode=automatic`，带 PR 号、head、base）；head 落后 `main` 时不派发，只在 step summary 说明怎么恢复。
  - `release-candidate.yml`（候选）：`prepare` 校验工作流身份（受保护的 `main`）、PR 身份，以及 `.github/scripts/check-release-delta.py` 的 delta 证明：head 相对 `main` 只改了 `CHANGELOG.md`、`.release-please-manifest.json` 和三份 `package.json` 的版本号，任何模式下证明失败都停止，因为构建腿带着生产和签名 secrets。之后做原 preflight 的三件事（`check-config`、内置引擎 secrets、主机名守卫），并把 head 的 `Release candidate` 置为 pending。
  - 五条构建腿（Windows 交叉构建、Linux、Apple 芯片与 Intel macOS、Android）在根目录检出被构建的源码，在 `.release-tooling/` 检出本工作流所在提交的发布脚本（`updater-json.py`、`check-macos-bundle.sh`、`tauri-bundle-retry.sh`、`install-cargo-tauri.sh`、`artefact-checks.sh`、`android-toolchain.sh`、`sign-android-package.sh`、`check-android-package.sh`）；定义产品的脚本（Makefile、构建脚本、第三方声明、`forget-sherpa-onnx-build.sh`）来自源码。缓存沿用 `release-<target>`。
  - `source-gate` 不重跑测试：要求 `ci.yml` 在 head 所在的 `main` 提交（backfill 时是标签提交）上 push 运行的 `CI Success` 通过，最多等 45 分钟。`CI Success` 等 `ci.yml` 里除 Codecov 上传之外的全部作业，包括两条 macOS 腿（`scripts/release/test_release_workflows.py` 检查这一点；v0.0.5 就是在它还不等 macOS 时，从一个 Intel 腿失败的提交发出去的）。`aggregate`（唯一持有 `id-token: write`、不执行项目代码的 job）用 `scripts/release/candidate.py seal` 核对五条腿齐全、每个文件与证据一致，写 `candidate-manifest.json`，证明全部文件和清单，上传 `release-candidate`（保留 14 天）。`gate` 按结果把 `Release candidate` 写成 success 或 failure（只在 automatic 模式写）。
  - 实测（0.0.4 backfill、0.0.5 自动路径，2026-09-28）：候选约 26 分钟，四条腿并行，Intel macOS 腿约 25 分钟是关键路径（Linux 8.6、Windows 8.1、Apple 芯片 10.7 分钟）；合并到公开 2 分 31 秒（同轮构建时是 20–35 分钟）。
  - 合并（squash）后 release-please 打标签并建草稿 Release，控制器的 `resolve_release` 按合并的 PR head 上 `github-actions[bot]` 写的最新 `Release candidate` 成功状态找到候选运行，校验它是 `main` 上成功的 `release-candidate.yml` 运行；`promote`（`environment: release`，不装编译器和包管理器）用 `candidate.py verify` 核对身份（PR、head、tree 等于标签提交的 tree、版本、工作流 SHA、运行号），对每个文件 `gh attestation verify`，用 minisign 校验每个 `.sig`，写 `latest.json`（四个桌面平台，地址都指向本次 Release 的资产；更新器不服务 Android，APK 与 AAB 只进 Release 和 `SHA256SUMS`）与 `SHA256SUMS` 并证明，上传后逐个核对远端字节，再把草稿转为正式。没有 `push: tags` / `release:` 触发。
- 仓库 ruleset：`main`（24076328）禁止删除与强推，谁都不能绕过；`release-candidate` 要求 `CI Success`（来源固定为 GitHub Actions，app id 15368）和 `Release candidate` 两项检查，并且 PR 必须与 `main` 同步才能合并（strict），只有仓库管理员（所有者）能绕过，所以直接推 `main` 照常可以。
  - 普通 PR 由 `ci.yml` 的 `candidate-status` 写 `Release candidate = success`，`CI Success` 由它自己的 CI 运行给出；fork 与 Dependabot 的只读令牌写不了状态，这类 PR 等 `CI Success` 变绿后由所有者 `gh pr merge --squash --admin` 合并。
  - 发布 PR 是机器人开的，它的 CI 运行停在 `action_required`，要先批准（`gh api -X POST repos/<owner>/voltip/actions/runs/<id>/approve`），否则没有 `CI Success`，PR 无法合并。
  - 发布 PR 只有候选能让它可合并：等该 head 上的 `Release candidate` 变绿，再 `gh pr merge <n> --squash --match-head-commit <head>`。
  - 没有合并队列：`ci.yml` 也没有 `merge_group` 触发。以后要启用，必须同时恢复触发器，并给合并组的 SHA 写 `Release candidate`，否则队列会一直卡住。
- 发布恢复：
  - head 落后（release-please 没改说明时不重写 PR，例如 `ci:`、`docs:` 推送；strict 规则也会拦住合并）：`gh api -X PUT repos/<owner>/voltip/pulls/<n>/update-branch`（或网页上的 Update branch），再 `gh workflow run release.yml -f mode=orchestrate`，为新 head 重建候选。旧 head 上的绿色状态不适用于新 head。
  - `source-gate` 报 `CI Success on <sha> concluded failure`：先 `gh run view <main 上的 CI 运行> --json jobs` 看是哪个作业失败（常见是只在 `main` 上跑的 macOS 腿）。如果是 runner 自身的问题，先 `gh run rerun <main CI> --failed`，再 `gh run rerun <候选> --failed`（`source-gate` 会等新的 `CI Success`）；如果是代码问题，修复后经 PR 合入 `main`，再按下面两条重建候选。
  - 候选失败：看失败的腿，把修复推到 `main`。`feat:` / `fix:` 修复会改说明，release-please 重写 PR 并自动派发；`ci:` 之类不进说明的修复会让 head 落后，按上一条 Update branch 后再 `mode=orchestrate`。
  - 合并后晋升失败、候选已过期（14 天）或已丢失：草稿保留。先对标签补建候选，再晋升：

    ```bash
    sha=$(gh api repos/<owner>/voltip/git/ref/tags/v<版本> --jq .object.sha)
    tree=$(gh api repos/<owner>/voltip/git/commits/$sha --jq .tree.sha)
    gh workflow run release-candidate.yml -f mode=backfill -f release_tag=v<版本> \
      -f expected_head_sha=$sha -f expected_tree_sha=$tree -f release_pr_number=<合并的发布 PR>
    # 候选成功后：
    gh workflow run release.yml -f mode=promote -f release_tag=v<版本> -f candidate_run_id=<运行号> \
      -f release_pr_number=<合并的发布 PR> -f candidate_head_sha=$sha
    ```

    backfill 构建的是标签提交本身，所以不做 delta 证明，也不写 `Release candidate`。
  - 退回同轮发布：revert 这组工作流提交，并删除 `release-candidate` ruleset。
- 更新器是可选的：设置了 `VOLTIP_UPDATE_PUBKEY` 才打开。应用的更新地址是本仓库的 `releases/latest/download/latest.json`，`latest.json` 里的下载地址固定到该次 Release 的资产。预发布不会被标成 latest，所以只有正式版才会推给已安装的用户。
- 版本模型：release-please（`node` 策略）只改根 `package.json` 与两份应用 `package.json`；两份 `tauri.conf.json` 写 `"version": "../../../package.json"` 指向根文件，安装包、更新器、`voltip --version` 与中继握手里的 `client_version` 读的都是它；Cargo 版本固定为 `0.0.0`，发版提交不改 `Cargo.toml` / `Cargo.lock`。候选 `prepare` 的 `check-config --package-json package.json` 校验这条链。1.0 之前 `feat` 和 `fix` 都只加补丁号（`0.0.1` → `0.0.2`，`bump-patch-for-minor-pre-major`），破坏性变更（`feat!` / `BREAKING CHANGE`）才加次版本号（`bump-minor-pre-major`）；要跳到别的版本，在提交的 footer 写 `Release-As: <版本>`。
- 仓库设置：默认分支要有 ruleset（控制器和候选只在受保护的默认分支上运行）；打开「Allow GitHub Actions to create and approve pull requests」让 release-please 能开 PR。release-please 用 `GITHUB_TOKEN` 开的 PR 由 `github-actions[bot]` 提交，GitHub 把它的 CI 停在 `action_required`，要有写权限的人批准才会跑；它只跑 `verify-web`，合并看的是候选写的 `Release candidate`，批准与否不影响合并：

  ```bash
  gh run list -R <owner>/voltip --workflow CI --branch release-please--branches--main--components--voltip-workspace \
    --json databaseId,conclusion --jq '.[] | select(.conclusion == "action_required") | .databaseId' |
    xargs -r -I{} gh api -X POST repos/<owner>/voltip/actions/runs/{}/approve
  ```

  合并后 `main` 的 push 只跑 `verify-web`，Release 直接晋升候选，不再构建。
- 一行命令安装：`scripts/install.sh`（Linux 与 macOS，按芯片挑 dmg，Linux 上优先 apt 装 deb，否则 AppImage）和 `scripts/install.ps1`（Windows，静默按用户安装），都从同一个 release 下载安装包和 `SHA256SUMS`，校验不过就不装。正式版发布后跑一遍 `gh workflow run install-scripts.yml -f version=<版本>`：在 Linux（deb 与 AppImage）、两种 Mac 和 Windows PowerShell 5.1 上真的装一次，再确认装好的程序能回答 `--version`。
- macOS 签名（2026-09-29 起）：发布包用项目自己的自签名代码签名证书「Voltip Code Signing」（有效期 100 年，没有公证），这样 Mac 把每次更新都当作同一个应用，麦克风和辅助功能授权都会保留；钥匙串另有分区限制，见下面「钥匙串」。
  - 位置：证书与私钥的 `.p12` 和它的密码只在所有者的密码管理器与仓库 secrets（`MACOS_CERTIFICATE`、`MACOS_CERTIFICATE_PASSWORD`、`MACOS_SIGNING_IDENTITY`，见 `.github/README-secrets.md`）里；仓库只记公开的 SHA-1 与规范的 designated requirement（`.github/release-targets.json` 的 `macos_signing`）。
  - 构建：候选的两条 macOS 腿把证书导入本 job 的临时钥匙串并信任它用于代码签名（`.github/scripts/macos-signing-keychain.sh`；结束时只删除临时钥匙串，不撤销信任设置：撤销要在对话框里授权，runner 上没人应答会一直卡住，runner 随 job 销毁），Tauri 用 `APPLE_SIGNING_IDENTITY` 签名；`hardenedRuntime` 保持关闭（自签名证书没有 Team ID，打开后库校验会拒绝内嵌的 sherpa dylib）。`check-macos-bundle.sh --expect-requirement` 要求 app 的 designated requirement 与 `macos_signing` 一致、每个 Mach-O 由同一证书签名；ad-hoc 包直接失败。本地构建和普通 CI 仍是 ad-hoc。
  - 轮换：换证书会让每位用户再授权一次，只在私钥泄露时做。用 `openssl req -x509 -newkey rsa:2048 -sha256 -days 36525 -nodes -subj "/CN=Voltip Code Signing" -addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning" -addext "basicConstraints=critical,CA:false"` 生成，`openssl pkcs12 -export -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1` 导出（macOS 的 `security import` 不认 OpenSSL 3 默认的 p12 算法），同一个 PR 里更新三个 secrets 与 `macos_signing`，发布说明写明这一版会再要求授权。
  - 丢失：没有备份就只能按轮换处理；已安装的用户在下一版会再被问一次。
  - 钥匙串（2026-09-30 用户从 0.0.12 更新到 0.0.14 仍各问一次）：
    - 原因：登录钥匙串给每个条目一个分区列表（`partition_id`）。自签名证书没有 Apple Team ID，按 securityd 的规则（`clientid.cpp` 的 `partitionIdForProcess`），每个构建的分区都是 `cdhash:<该构建>`。一个构建去读另一个构建创建的条目就会弹窗，即使访问控制信任的签名要求（`identifier … and certificate leaf = H…`）它满足；「始终允许」也只把按下它的那一个构建加进列表。用户的条目分区是 `cdhash:<0.0.12>, cdhash:<0.0.14>`。
    - 0.0.15 起，用证书签名的发布包把每个条目存进自己创建的条目，账户 `<用户名>.signed.<本构建 cdhash>`（`crates/voltip-identity/src/per_build.rs`）。
    - 应用内更新：0.0.15–0.0.16 的「安装后交接」无效——Tauri updater 返回前会删除旧 bundle，旧进程随后用 `SecCodeCopySelf` / `SecCodeCopyGuestWithAttributes` 校验双方时得到 `ENOENT`，静默退回普通重启。修复后，updater 已验签的 `.app.tar.gz` 先解到临时目录；旧版与 staged 新版的 bundle 都存在时，两边校验对方满足同一 designated requirement，旧版通过 socketpair 交出本进程已知的所有条目（含删除标记），staged 新版写入自己 cdhash 的条目并 readback，之后才 ACK；只有 ACK 成功才安装。安装失败保留旧版值，安装成功后的新版读取自己的条目并删掉旧副本。首个带修复的版本仍由 0.0.16 的旧更新代码安装，所以会最后询问一次；从两个都带修复的连续版本开始不弹窗。
    - 没有交接时（手动安装 dmg，或从不带交接的 0.0.14 及更早版本更新上来），新版读最新的一份旧条目，问一次，然后搬进自己的条目。
    - 降级到 0.0.15 之前的版本时，旧版找不到它的条目，会生成新的设备身份，需要重新配对。本地和 CI 的 ad-hoc 构建仍用 `<用户名>` 账户，不参与。
    - 验证：`security create-keychain` 建的钥匙串不做分区检查（`validatePartition` 在 `dbVersion() < version_partition` 时直接返回），所以 0.0.12 时的检查都通过了，却没发现问题。现在 CI 的 `macos` job 用 runner 的登录钥匙串：
      - `.github/scripts/check-keychain-across-updates.sh` 断言后一个构建读不到前一个构建的条目，但能列出和删除；
      - `.github/scripts/check-keychain-preinstall.sh` 用真实应用端到端验证安装前交接：不同 cdhash 的双方都做真实签名校验，staged 新版持久化后才 ACK，安装后的新版不弹窗、身份不变并删除旧条目；错误签名必须在安装前失败；持久化失败由 `persist_handoff` 回归测试证明不 ACK 并回滚本构建条目；
      - `.github/scripts/check-keychain-migration.sh` 在临时钥匙串里验证 ad-hoc 条目的搬移。

      真机在 `docs/acceptance/macos/manual-checklist.md` 第 15 项确认。
    - 查看真机上的状态（只读，不输出密钥）：`security dump-keychain -a ~/Library/Keychains/login.keychain-db`，只看 `"svce"<blob>="dev.voltip.desktop/` 的条目，关注 `acct` 和 `partition_id`。
  - 0.0.7 过渡时的另两项：辅助功能在系统设置里把 Voltip 关掉再打开，不行就 `tccutil reset Accessibility dev.voltip.desktop` 后重新授权；麦克风同样关掉再打开，没有再弹授权时 `tccutil reset Microphone dev.voltip.desktop`。发布说明要写这两项和钥匙串的变化。
- Android 签名（2026-10-01 起）：GitHub Release 里的 APK 与上传 Google Play 的 AAB 用同一把密钥签名（RSA 4096，有效期 100 年，别名 `voltip`，证书 `CN=Voltip, O=Voltip`）。Play 应用签名保存的也是这把密钥（用户 2026-09-30 选定），所以从 GitHub 安装的应用与从 Play 安装的应用可以互相覆盖升级。
  - 位置：PKCS12 密钥库和它的密码只在所有者的密码管理器与仓库 secrets（`ANDROID_KEYSTORE_BASE64`、`ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_PASSWORD`，见 `.github/README-secrets.md`）里；仓库只记证书的 SHA-256 和密钥别名（`.github/release-targets.json` 的 `android_signing`）。别名不是秘密，只是密钥在密钥库里的名字；0.0.18 的候选把它当作 secret，它的值就是项目名，于是 `prepare` 和 Android 腿日志里的每个 `voltip` 都被遮成了 `***`。`prepare` 要求三个 secrets 都存在。
  - 构建：候选的 `bundle-android` 腿先用 Gradle 构建未签名的 APK 和 AAB。这一步运行全部依赖的构建代码，所以不接触密钥。然后在单独一步把密钥库解码到本 job 的临时目录，用 `.github/scripts/sign-android-package.sh` 签名，步骤结束即删除：APK 先 `zipalign -P 16`，再用 apksigner 做 v2 与 v3 签名（minSdk 26 不需要 v1）；AAB 用 jarsigner 签名。
  - 检查（`.github/scripts/check-android-package.sh`）：两个包都由 `android_signing` 的证书签名，且只有一个签名者；APK 按 16 KB 页对齐，每个 `.so` 的 LOAD 段按 16 KB 对齐（Google Play 要求面向 Android 15 及以上的应用支持 16 KB 页）；包名 `dev.voltip.mobile`，versionName 等于发布版本，versionCode 等于 主版本 × 1000000 + 次版本 × 1000 + 补丁号（Tauri 的算法，所以每次发布都更大）；targetSdk 36；原生库里没有服务商密钥；带本版本的第三方许可声明（`assets/THIRD-PARTY-NOTICES.txt`）。CI 的 `android` job 用当次生成的临时密钥走同一条签名与检查路径。
  - 丢失：GitHub 上的 APK 再也无法用同一签名发布，已安装的用户只能卸载后重装（配对随之丢失）；Play 上的应用可以在 Play Console 申请重置上传密钥，Play 继续用它保存的密钥签名。所以密钥库必须另存一份。
  - 泄露：轮换要用 APK 签名方案 v3 的密钥轮换（`apksigner rotate`）和 Play Console 的应用签名密钥升级，届时单独设计；Android 8.x 不支持 v3 轮换，这些设备只能重装。
- 还没进工作流的：上传 Google Play；macOS 包没有公证（见 `docs/roadmap.md`）。

## 文档站

`voltip.firlab.app` 的页面写在 `docs/site/`（英文；中文在 `zh/`），站点本身（VitePress 配置、主题、部署）在 `sunerpy/firlab` 的 `voltip/`。写作规则、本地预览和截图流程见 `docs/site/README.md`。

- PR 改到 `docs/site/**` 或上站的 8 份设计文档时，`docs-site.yml` 签出公开的 firlab `main`，同步后构建一次，只做检查，不用 secret。
- 合并到 `main` 后，`publish-site.yml` 用 `FIRLAB_DOCS_TOKEN` 签出 firlab，运行 `voltip/scripts/sync-voltip-docs.sh`，把 `voltip/src` 的变化提交为 `docs(voltip): sync from voltip@<sha>` 并推送；firlab 的 `deploy-voltip.yml` 接着构建并部署到 Cloudflare Pages（项目 `voltip-docs`）。页脚写着内容来自哪个提交。
- `publish-site` 签出 firlab 失败（401 / 403）：token 过期或权限不对，按 `.github/README-secrets.md` 重建。同步脚本报错时，信息会指出哪一页哪一行（缺另一种语言的页面、未注册的组件、禁用词），在本仓库修正。同步成功但站点没变：看 firlab 的 `deploy-voltip` 运行记录。

## 局域网直连

- 每台设备常驻一个 LAN 主机（默认 TCP 47831，被占用退到临时端口）；配对设备优先在这里重逢，Relay 只是回退。首次运行时 Windows / macOS 防火墙会询问是否允许监听，拒绝只会失去直连（回退 Relay），不影响配对。
- 可信设备的局域网地址持久化在 `trusted-devices.json` 的 `direct_hints`，每次握手成功后由对端在加密通道内刷新；`RUST_LOG=voltip=debug` 可看到 `announcing device info` / `device info from peer`。
- 关掉 Relay 后同一局域网内仍可用；不同网络之间没有 Relay 就没有连接（没有 NAT 穿透）。

## 无头冒烟（Linux）

`make smoke-desktop`：以 debug 构建的真实 Tauri 桌面程序（`--features custom-protocol`，加载打包好的前端）在 Xvfb 里启动，等窗口出现后用 scrot 截 WebView，再点进「手机麦克风」页截一次。需要 `Xvfb xdotool scrot x11-utils python3-pil`。`VOLTIP_DEV_SECRET_STORE=memory` 让 **debug** 构建使用内存密钥存储（没有 Secret Service 的容器 / CI）；release 构建忽略这个变量，始终用平台安全存储，不会降级。

## Android 构建（本机）

```bash
export ANDROID_HOME=<sdk> NDK_HOME=<sdk>/ndk/<27+> JAVA_HOME=<jdk17+>
rustup target add aarch64-linux-android
make android-apk        # scripts/build-android-debug.sh：arm64 debug APK + aapt badging 写入 dist/android/build-info.txt
make android-clippy     # 对 aarch64-linux-android 跑 clippy：手机壳链接的每个工作区 crate，含只在 Android 上编译的代码
```

发布用的包与 CI 的 `android` job 走同一条路（CI 上的工具链由 `.github/scripts/android-toolchain.sh` 固定：JDK 21、platform 36、build-tools 35.0.0、NDK 30.0.16248370（r30，当前的 LTS））：

```bash
python3 scripts/release/third-party-notices.py --app mobile --out apps/mobile/src-tauri/resources/THIRD-PARTY-NOTICES.txt
(cd apps/mobile && cargo tauri android build --ci --target aarch64 --apk --aab --config src-tauri/tauri.package-android.conf.json)
out=apps/mobile/src-tauri/gen/android/app/build/outputs
# 四个 ANDROID_* 环境变量指向密钥库与它的密码（.github/README-secrets.md），不要写进命令历史
.github/scripts/sign-android-package.sh "$out/apk/universal/release/app-universal-release-unsigned.apk" \
  "$out/bundle/universalRelease/app-universal-release.aab" dist/android <版本>
.github/scripts/check-android-package.sh dist/android/apk/Voltip_<版本>_android_arm64.apk \
  dist/android/aab/Voltip_<版本>_android_arm64.aab <证书 SHA-256> <版本>
```

Gradle 工程 `apps/mobile/src-tauri/gen/android` 已提交（`cargo tauri android init --ci` 可重建）；`app/build`、`.gradle`、`jniLibs` 符号链接和 CLI 复制进 `app/src/main/assets/` 的文件不入库，CLI 每次构建都重新生成它们（2026-10-01 在删掉这些文件后从头构建验证过）。release 构建开着 R8，Tauri 生成的 `proguard-tauri.pro` 保留插件类和 `@Command` 方法（同日在 release 的 DEX 里核对过三个插件及其命令）。

## Windows 交叉构建（Linux 主机）

`make windows-x64`：`cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis --features gpu-vulkan`，需要 `cargo-xwin`、`lld-link`、`llvm-rc`、`llvm-readobj`、`llvm-dlltool`、`clang`、`makensis`、`zip`。Vulkan 的构建输入由 `scripts/lib/vulkan-sdk.sh` 下载并校验（Linux 版 SDK 提供头文件和 glslc，Windows 运行时组件提供 `vulkan-1.dll`，导入库从它的导出表生成）。产物：NSIS 安装器与便携 zip（`voltip-desktop.exe` 连同它从自身目录加载的 sherpa-onnx DLL 和 `vulkan-1.dll`；需系统已有 WebView2 运行时，Win10/11 自带），均未做 Authenticode 签名（SmartScreen 会提示「更多信息 → 仍要运行」；代码签名见 `docs/roadmap.md`）。`release.yml` 在 Linux runner 上用同一脚本交叉构建，只给自动更新的 NSIS 包加 Tauri updater 签名（`.sig`），不出 MSI。release 构建没有注入 `VOLTIP_RELAY_URL` 时中继显示「未配置」，局域网直连照常可用。

## 第三方许可声明

`scripts/release/third-party-notices.py` 生成 `THIRD-PARTY-NOTICES.txt`：cargo-about（`about.toml`，许可证允许列表与 `deny.toml` 相同，只算会进二进制的依赖）列出桌面壳链接的每个 crate，`pnpm licenses list --prod` 列出前端依赖并带上各包自己的许可证文件（三款 OFL 字体在内），原生库的文本在 `scripts/release/licenses/`（ONNX Runtime 1.28.2 的 LICENSE 与 ThirdPartyNotices、Khronos Vulkan loader），transcribe.cpp 与 ggml 的取自 transcribe-cpp-sys 的源码。两个打包脚本和 release 工作流都会生成它：Windows 装进安装目录和便携 zip，Linux 放在 `/usr/share/doc/voltip/`。升级 sherpa-onnx（随之 ONNX Runtime）或 Vulkan 运行时后，要同步更新 `scripts/release/licenses/` 里对应的文本。

## Windows 真机（SSH）

交叉构建只证明能链接；原生 MSVC 构建、Windows 上的单元测试和真实 CPU 上的识别要在 Windows 机器上跑。`scripts/windows-remote.sh` 通过 OpenSSH 把当前提交送过去并在那边执行，推到 CI 之前先用它验证：

```bash
scripts/windows-remote.sh sync            # HEAD 打成 git bundle，scp 过去，在 <dir>\repo 里 detached checkout
scripts/windows-remote.sh gate test       # 原生 cargo test --workspace --all-targets（MSVC），日志拷回 target/windows-remote/
scripts/windows-remote.sh gate clippy     # 原生 cargo clippy --workspace --all-targets -D warnings
scripts/windows-remote.sh gate real       # 真实模型用例（整段 + 流式 + 热词 + VAD）在该机 CPU 上跑；VOLTIP_LOCAL_* 同本机，文件会拷过去
scripts/windows-remote.sh gate mdns       # 局域网发现：两个真实 mDNS 守护进程经 Windows 自己的网络栈互相看到
scripts/windows-remote.sh gate echo       # 混合录音的回声消除（release）：合成房间的指标与每帧耗时（docs/dictation.md §22.6）
make windows-x64 && scripts/windows-remote.sh smoke   # 便携包的无头运行：列出计算设备、下载模型、识别公开样音（smoke-native-cli.ps1）
make windows-remote                       # 以上四步依次执行
```

- 连接：`VOLTIP_WINDOWS_SSH`（ssh 参数，最后一项是主机，必填），工作目录 `VOLTIP_WINDOWS_DIR`（默认 `C:\voltip-ci`）；两者取环境变量，否则取 git 忽略的 `.env.build`。对端需要 OpenSSH Server、管理员账号、Git、Rust（MSVC 工具链）、Visual Studio 2022 Build Tools（C++ 工作负载，含 CMake）和 PowerShell 7。
- 远端 shell 会提前展开 `$`，输出又是控制台代码页（中文系统是 GBK），所以脚本一律用 `-EncodedCommand`（UTF-16LE base64）发送，并把输出切到 UTF-8。
- `gate` 和 `smoke` 都注册成计划任务运行，SSH 断开不影响；脚本每 20 秒查一次日志末尾的 `EXIT=` 行，默认最多等 90 分钟（`VOLTIP_WINDOWS_TIMEOUT`）。`gate` 在已登录用户的会话里跑（用该用户的工具链，没人登录时任务不会启动，脚本会直接报错）；`smoke` 以 SYSTEM 身份跑：应用经 Known Folder API 取数据目录，SYSTEM 的数据目录在系统配置文件下，不会动到真实用户的 Voltip 模型库和设置。
- 只跑命令行入口，不启动 GUI：对端若装着正在运行的 Voltip，同标识符（`dev.voltip.desktop`）的单实例插件会把第二个实例的参数转发给它。
- 原生构建踩过的三个坑（2026-09-26，Windows Server 2025 中文版）已修复并有回归测试（`apps/desktop/src-tauri/tests/bundle.rs` 的 `regression_*`）：sherpa-onnx 运行库的构建脚本顺序、手机壳缺 `icons/icon.ico`、MSVC 在代码页 936 下需要 `/utf-8`。

## 排障

- 桌面端日志：`RUST_LOG=voltip=debug`；日志绝不打印私钥、token、明文（`SecretKey`/`PairCode` 的 `Debug` 已脱敏）。
- 「身份已变化」横幅：对端换了设备身份（重装或攻击）。先在两端「忘记设备」，再重新配对并核对 Safety Code。
- 配对总是过期：核对两端时钟（票据 `expires_at` 用 Unix 秒）。
- CI / Release 的 Windows 包报 ``resource path `resources/windows/onnxruntime_providers_shared.dll` doesn't exist``、Linux 包缺 `libsherpa-onnx-c-api.so`：rust-cache 恢复 `target/` 时保留了 sherpa-onnx-sys 的指纹，却删掉了它的构建脚本下载的运行时（`target/sherpa-onnx-prebuilt/`）和拷到二进制旁边的副本，cargo 认为构建脚本不用再跑。每个 rust-cache 步骤后的 `.github/scripts/forget-sherpa-onnx-build.sh` 删掉它的指纹与下载目录，让它重新下载（约 10 MB）。
- 设备列表显示 `Relay` 而不是 `直连`：两台设备不在同一网段，或 LAN 主机端口被防火墙拦住；`直连` 需要至少一方能连到另一方的 `direct_hints`。
