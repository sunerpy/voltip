# GitHub Secrets

Every value the workflows read that names a host, carries a token or signs a release is a
repository **secret**, baked in at build time. Source, docs and configuration hold placeholders
only; `.github/scripts/check-no-production-hosts.sh` guards the tree. This file lists names,
meanings and where each value comes from, never a value.

CI (`ci.yml`) needs almost none of them: its packages are built without the built-in
engines on purpose, so pull requests from forks run the same jobs as the default branch. The
release candidate workflow (`release-candidate.yml`, which builds every package once) needs the
engine secrets, the macOS certificate, the Android key and, when the updater is on, the updater's
signing key; the release controller (`release.yml`) needs only the updater pubkey, to verify the
signatures before it publishes.

## Setup

```bash
# In the repository root, logged in with `gh auth login`. Paste each value when prompted, or
# pass --body-file <(printf '%s' "$value"); keep values out of shell history.
repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
for name in VOLTIP_PRODUCTION_HOSTS VOLTIP_RELAY_URL VOLTIP_ASR_URL VOLTIP_ASR_MODEL VOLTIP_ASR_TOKEN \
            VOLTIP_REFINE_URL VOLTIP_REFINE_MODEL VOLTIP_REFINE_API_KEY CODECOV_TOKEN; do
  gh secret set "$name" --repo "$repo"
done
# Optional: the updater (both or neither) and the model mirror.
gh secret set VOLTIP_UPDATE_PUBKEY --repo "$repo" --body-file ~/.tauri/voltip.key.pub
gh secret set TAURI_SIGNING_PRIVATE_KEY --repo "$repo" --body-file ~/.tauri/voltip.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo "$repo"
gh secret set VOLTIP_MODEL_BASE_URL --repo "$repo"
# Optional: the feedback endpoint (both or neither).
gh secret set VOLTIP_FEEDBACK_URL --repo "$repo"
gh secret set VOLTIP_FEEDBACK_TOKEN --repo "$repo"
# The Android release key (see below): the keystore base64 on one line, then its password and alias.
base64 -w0 voltip-release.jks | gh secret set ANDROID_KEYSTORE_BASE64 --repo "$repo"
gh secret set ANDROID_KEYSTORE_PASSWORD --repo "$repo"
gh secret set ANDROID_KEY_ALIAS --repo "$repo" --body voltip
gh secret set ANDROID_KEY_PASSWORD --repo "$repo"
# The documentation site: a fine-grained token for sunerpy/firlab (see below).
gh secret set FIRLAB_DOCS_TOKEN --repo "$repo"
gh secret list --repo "$repo"
```

## Guard and coverage (CI)

| Secret | Meaning | Source |
|---|---|---|
| `VOLTIP_PRODUCTION_HOSTS` | Words the production-host guard looks for: the real host names, or their distinctive labels, space- or comma-separated, at least 4 characters each. CI's `verify` job and the release candidate's `prepare` job run the guard with `--require`; a hit prints `path:line` only. Pull requests from forks skip that step (they get no secrets); `make verify` runs the guard with the local `.env.build`. | your deployment |
| `CODECOV_TOKEN` | Codecov upload token (the `codecov` job in `ci.yml`). Forks upload without it, which Codecov accepts for public repositories. Reporting only: the merge-blocking floors are `make verify`'s. | Codecov → repository settings |

## Documentation site (`publish-site.yml`)

| Secret | Meaning | Source |
|---|---|---|
| `FIRLAB_DOCS_TOKEN` | Pushes the synced pages of voltip.firlab.app into `sunerpy/firlab`, which builds and deploys the site. A fine-grained personal access token for `sunerpy/firlab` only, with **Contents: read and write** and nothing else. GitHub has no API that creates one, so the owner makes it by hand and replaces it before it expires; an expired token shows up as a failed checkout in `publish-site`. `docs-site.yml` (pull requests) reads the public repository and needs no secret. | GitHub → Settings → Developer settings → Fine-grained tokens |

## Built-in engine defaults (release only)

`option_env!` reads these when the shells compile (`docs/dictation.md` §3). The `prepare` job of
`release-candidate.yml` fails the release before any build when one of the required ones is empty;
`scripts/lib/require-builtin-engines.sh` applies the same rule to local packaging, unless
`VOLTIP_ALLOW_NO_BUILTIN_ENGINES=1` asks for an engine-less package on purpose (CI does). The
`verify` job never receives them: `scripts/verify-all.sh` unsets them because its tests assert the
no-built-in state.

| Secret | Meaning | Source |
|---|---|---|
| `VOLTIP_RELAY_URL` | The relay's `wss://<relay-host>/ws` address (`voltip-transport`). | your relay deployment (`docs/runbook.md`) |
| `VOLTIP_ASR_URL` | Base URL of the built-in recognition endpoint (without `/v1`). | your ASR edge |
| `VOLTIP_ASR_MODEL` | Model name the built-in endpoint serves. | your ASR edge |
| `VOLTIP_ASR_TOKEN` | Application token the edge checks before forwarding. Revocable at the edge; never a provider key. | your ASR edge |
| `VOLTIP_REFINE_URL` | Base URL of the built-in clean-up endpoint (OpenAI-compatible, `…/v1`). | your edge |
| `VOLTIP_REFINE_MODEL` | Clean-up model name. | your edge |
| `VOLTIP_REFINE_API_KEY` | Credential for the clean-up endpoint: an application token, never a provider key. The Windows build script and the Linux release leg scan the binary and refuse `gsk_…` / `sk-…` keys. | your edge |
| `VOLTIP_MODEL_BASE_URL` | Optional first download source for the local models (`<base>/<hf-repo>/<file>`); without it the app uses huggingface.co, then hf-mirror.com. | your mirror |

## Feedback endpoint (release only, optional)

The 反馈 dialog posts to this endpoint (`docs/feedback.md`, the Worker in `services/feedback`). A
build without it offers the repository's issue page instead.

| Secret | Meaning | Source |
|---|---|---|
| `VOLTIP_FEEDBACK_URL` | The endpoint, `https://<feedback-host>/v1/feedback`. | your feedback Worker |
| `VOLTIP_FEEDBACK_TOKEN` | The application token the Worker checks (its `FEEDBACK_TOKEN` secret); it ships inside the app, so it only keeps casual abuse out, the Worker's rate limit does the rest. | generated, same value as the Worker's |

## Updater (release only, optional)

The updater is on exactly when `VOLTIP_UPDATE_PUBKEY` is set. The release then builds the app with
`VOLTIP_UPDATE_URL = https://github.com/<owner>/<repo>/releases/latest/download/latest.json`,
signs the NSIS installer and the AppImage, verifies every `.sig` against the pubkey, and attaches
`latest.json` (URLs pinned to that release's assets) to the release. A pre-release is never marked
latest, so installed copies are offered an update only when a stable release is published.

| Secret | Meaning | Source |
|---|---|---|
| `VOLTIP_UPDATE_PUBKEY` | Updater public key: the full base64 content of the `.pub` file. `apps/desktop/src-tauri/src/update.rs` injects it into `plugins.updater` at run time; `tauri.conf.json` does not carry it. | `cargo tauri signer generate -w ~/.tauri/voltip.key` |
| `TAURI_SIGNING_PRIVATE_KEY` | Updater private key, present only on the `cargo tauri bundle` step. Losing it ends updates for every installed copy: back it up outside GitHub. | same command |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Its password; an empty string when it has none. | same command |

Do not put `bundle.createUpdaterArtifacts` in `tauri.conf.json`: local `make windows-x64` would
then demand the private key. The release candidate's `prepare` job refuses such a configuration.

## macOS release certificate (release only, required)

The macOS packages are signed with one self-signed code signing certificate, "Voltip Code Signing"
(100 years), so a Mac sees every update as the same app and keeps its microphone and Accessibility
grants and its keychain access. `release-candidate.yml` imports it into a keychain of the job
(`.github/scripts/macos-signing-keychain.sh`) and checks the app and every Mach-O in it against
`macos_signing` in `.github/release-targets.json` (`scripts/release/macos-signature.py`). Local and
ordinary CI builds stay ad hoc. Rotation and recovery: `docs/runbook.md` (发布 · macOS 签名).

| Secret | Meaning | Source |
|---|---|---|
| `MACOS_CERTIFICATE` | The certificate and its private key: the `.p12`, base64 on one line. | kept in the owner's password manager |
| `MACOS_CERTIFICATE_PASSWORD` | The `.p12` password. | same place |
| `MACOS_SIGNING_IDENTITY` | The certificate's SHA-1 (upper case, no colons); must equal `macos_signing.certificate_sha1`. | `openssl x509 -noout -fingerprint -sha1` |

## Android release key (release only, required)

The Android packages are signed with one key (RSA 4096, valid for 100 years, alias `voltip`): the
APK on the releases page and the AAB uploaded to Google Play, whose app signing keeps a copy of the
same key. An APK signed with another key cannot update the installed app. `release-candidate.yml`
builds both packages unsigned and signs them in a step of its own
(`.github/scripts/sign-android-package.sh`), so the key is never present while the dependencies'
build code runs; the checks require the certificate `android_signing.certificate_sha256` in
`.github/release-targets.json` names. CI signs with a key it makes for the run. Backup and recovery:
`docs/runbook.md` (发布 · Android 签名).

| Secret | Meaning | Source |
|---|---|---|
| `ANDROID_KEYSTORE_BASE64` | The PKCS12 keystore holding the key, base64 on one line. | kept in the owner's password manager |
| `ANDROID_KEYSTORE_PASSWORD` | The keystore password. | same place |
| `ANDROID_KEY_ALIAS` | The key's alias, `voltip`. | same place |
| `ANDROID_KEY_PASSWORD` | The key password; PKCS12 keeps one password, so it equals the keystore password. | same place |

## Repository settings the release relies on

- Protect the default branch (ruleset or branch protection) before the first release:
  `.github/scripts/resolve-release-gate.sh` runs the release only from the protected default branch.
- Settings → Actions → General → "Allow GitHub Actions to create and approve pull requests", so
  release-please can open its release PR with `GITHUB_TOKEN`.
- The `release` environment (created on first use) holds the jobs that attach assets and publish;
  restrict it to the default branch.

`GITHUB_TOKEN` itself needs no setup.
