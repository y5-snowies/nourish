# ci/ — the y5 CI/CD pipeline

A single pipeline that runs on **two platforms** — GitHub Actions (production) and GitLab
CI (the current self-hosted remote) — sharing exactly one copy of the real logic.

## Design: thin YAML, portable scripts

```
ci/scripts/*.sh        ← ALL logic. Plain, portable shell.
.github/workflows/*    ← thin: `run: ci/scripts/X.sh`
.gitlab-ci.yml         ← thin: includes ci/.gitlab/*.yml, each `script: ci/scripts/X.sh`
```

The only platform awareness anywhere is the `is_github` / `is_gitlab` predicates in
`ci/scripts/lib.sh`, used by the three scripts that must talk to a platform API (post a
PR/MR comment, open the promotion request, upload nothing else). Discovery, build,
coverage, packaging and report-generation are identical on both.

## Scripts

| Script | Does |
| --- | --- |
| `lib.sh` | shared helpers (repo root, bin-crate discovery, platform predicates) |
| `discover-workspaces.sh` | emit the **workspace entries** (the roots declared in `workspace.catalog.json`) as JSON / lines — drives the GitHub matrix and the GitLab child pipeline |
| `gen-child-pipeline.sh` | GitLab-only: entries → a child pipeline (lint/build/test/coverage per entry + a merge job) |
| `coverage-full.sh` | per-entry coverage **including dead code** (LLVM region baseline + unit-test merge) → `.ci-coverage/<slug>.lcov` |
| `merge-coverage.sh` | fuse all entry lcov → `coverage.lcov` + `cobertura.xml` + `html/` + **per-crate `coverage-crates.md`** + self-hosted **`coverage.svg`** badge + a `Coverage: NN.N%` line |
| `build-docs.sh` | landing page → `public/`; folds in the coverage report + badge and lists the reference guides when present |
| `build-site.sh` | full Pages build: run coverage for every entry → merge → `build-docs.sh` (coverage site) |
| `doc-suggest.sh` | PR/MR-only: `claude -p` reviews the diff, posts doc/README suggestions as a comment (advisory, never commits) |
| `gen-report.sh` | compose the markdown promotion "deployment notes" (tests/coverage/lint/drift/doc) |
| `open-promotion-pr.sh` | open/update the upstream-integration→upstream PR/MR with the report (never merges) |
| `package-installer.sh` | **the CD bundle builder** — delegates to `compositor.installer/prepare.sh` to build the full install bundle (installer + compositor/dev/polkit/mx binaries + components) → `dist/package.tar.gz` + `SHA256SUMS`. Served at `/release/latest/fedora44/` (what `get.sh` fetches) and attached to Releases |
| `package-release.sh` | manual/raw: build just the udev+winit compositor binaries as a tarball (not used by the automated CD path — `package-installer.sh` is) |

All scripts work locally too: `Y5_REPO_ROOT=$(pwd) ci/scripts/discover-workspaces.sh`.

## CI image — `ci/Containerfile`

Lean **Fedora 44**: rustup `stable` + `rustfmt`/`clippy`/`llvm-tools`, the
Wayland/GPU/protobuf **-devel** headers (build needs headers + link libs, no GPU at run
time), `nodejs`, `lcov`, `llvm`, `cargo-llvm-cov`, `lcov_cobertura`, `sccache`,
`gh`, and the `claude` CLI. It copies **no source** and installs **no runtime apps**
(unlike `environment.container/Containerfile`). Pushed to GHCR (GitHub) and the GitLab
registry as `…/y5-ci:fedora44`; every job runs inside it.

`sccache` is enabled via `RUSTC_WRAPPER=sccache` baked into the image — the documented
"turn it on only in CI, no repo changes" hook from `environment/README.md`. The repo's
`.cargo/config.toml` (`-A warnings`) is inherited untouched.

## Workspace entries

The build/test/coverage unit. An *entry* is a workspace root declared in
`workspace.catalog.json` — the same file that drives manifest generation, so the CI
matrix cannot describe a tree different from the one being built. Today there are 29.
Nothing is hardcoded, so adding/renaming a workspace needs **no pipeline edit**.

It used to be "a directory holding both a `Cargo.toml` and a `link.json`". Both halves
became wrong: `Cargo.toml` is a generated artifact and is absent from a fresh checkout,
and the `link.json` marker was stale — four roots never had one, so CI silently skipped
them.

## Branch flow

Two **structurally identical** channels. Same image, same scripts, same bundle matrix, same
release layout — they differ only in the version string, the release tag, and the fact that
the stable channel also deploys Pages:

```
feature → upstream-integration ──(CI green)──▶ auto PR → upstream ──(approve & merge)──▶ upstream
        │ ci.yml: build-udev + installer-bundle                                            │
        │                                                    Publish (docs.yml) — one build:
        │                                        ├─ Pages: site /, docs /docs, /release/latest/fedora44/
        │                                        └─ Release `v<X.Y.Z>` (Latest): every bundle + SHA256SUMS

feature → candidate-integration ──(you merge by hand; no CI, no artifacts)──▶ candidate
                                                                                 │
                                                       ci-rc.yml: build-udev + installer-bundle
                                                       Publish RC (release-rc.yml) — one build:
                                                       ├─ Release `v<X.Y.Z-rc.N>` (prerelease): every bundle + SHA256SUMS
                                                       └─ Release `bundles-rolling` (prerelease): same assets, rolling
```

The one structural difference: the stable channel splits validation (`upstream-integration`)
from publication (`upstream`) with an automatic promotion PR between them. The rc channel puts
both on `candidate` and keeps the promotion manual, so cutting an rc is always a deliberate act.

- **upstream-integration** (stable aggregation): full CI on every push (`ci.yml`); the
  `installer-bundle` job builds the install bundle as a downloadable artifact so the candidate
  can be tried before merge. On green, the promotion PR to `upstream` is opened/updated with
  the report.
- **upstream** (the single stable release action): protected — approve & merge the promotion
  request (set branch protection in the UI). A push runs **Publish** (`docs.yml`), which builds
  the Fedora bundle once plus one native bundle per (distro, arch) and ships them to both
  channels so they can't drift:
    - **Pages** (`nourish.snowies.com`): marketing site `/`, docs `/docs`, and the Fedora bundle
      at `/release/latest/fedora44/` (the URL `compositor.installer/get.sh` fetches).
    - **GitHub Release `v<X.Y.Z>`**, marked *Latest*: `package.tar.gz`, every
      `package-<distro>-<arch>.tar.gz`, one combined `SHA256SUMS` covering all of them, and a
      `bootstrap.sh` pinned to the newest stable.

### RC channel (release candidates)

The same pipeline with an rc tag. **Promotion is deliberately manual** (no auto-PR) so you
decide exactly when an rc is cut — that is the one intentional difference in flow:

- **candidate-integration** (rc aggregation): a plain staging trunk — **no workflow, no
  artifacts**. Stack commits here; when you choose to cut an rc, merge `candidate-integration`
  into `candidate` yourself (open the PR / fast-forward by hand). That merge is the deliberate
  build-and-publish gate.
- **candidate** (the single rc release action): a push runs **`ci-rc.yml`** — `ci.yml` pointed at
  this branch, so the rc gets the same `build-udev` compile of the real product that the stable
  channel gets, and an rc can never ship something that was never compiled. The same push runs
  **Publish RC** (`release-rc.yml`) — job for job the same as `docs.yml` minus `site`/`deploy` —
  with an
  `X.Y.Z-rc.N` version (`ci/scripts/version-rc.sh`: same VERSION-file mechanics as `version.sh`,
  plus an `-rc.N` counter derived from `v…-rc.*` tags). It publishes:
    - **`v<X.Y.Z-rc.N>`**, prerelease and explicitly non-latest so it can never steal the stable
      "Latest release" pointer: exactly the asset set the stable release carries, including the
      combined `SHA256SUMS`, with a `bootstrap.sh` pinned to that rc.
    - **`bundles-rolling`**, prerelease, recreated each push — the rc channel's stand-in for
      GitHub's `Latest` pointer (which belongs to stable), carrying the same assets with a
      `bootstrap.sh` that tracks the rolling tag. Opt-in only.
  **No Pages deploy** — Pages is the stable channel's single site.

Install an rc exactly like a stable release — same installer, same bootstrap, just a tag:

```
# auto-detect distro/arch, checksum-verified
Y5_RELEASE_TAG=v<X.Y.Z-rc.N> bash <(curl -fsSL https://nourish.snowies.com/install)
# or the newest rc, whatever it is
Y5_RELEASE_TAG=bundles-rolling bash <(curl -fsSL https://nourish.snowies.com/install)
# or the Fedora tarball directly
curl -fsSL https://github.com/<owner>/<repo>/releases/download/v<X.Y.Z-rc.N>/package.tar.gz \
  | tar -xz && y5-install/install.sh
```

`bootstrap.sh`'s built-in default is the newest **stable** release, so nothing installs an rc
by accident — an rc is only ever reached by naming its tag.

> Why not a `--rc` flag in `get.sh`? `get.sh` is a single shared script served only from the
> stable channel, so a per-channel flag would be dead code unless landed on `upstream` —
> coupling rc installs to a stable release. A tag keeps the install path channel-agnostic.

The install bundle is built by `compositor.installer/prepare.sh` (via `package-installer.sh`)
and contains every shipped binary + component + the interactive `y5-install`; building it in
CI is why the image carries the dev-tool window's GTK/WebKit `-devel` deps.

> GitLab mirror: `ci/.gitlab/release.yml` still cuts a release on a `v*` tag (not yet aligned
> to this upstream-driven model). GitHub Pages + the `latest` Release are the primary channel.

## Required secrets / variables (set in the platform UI, never in the repo)

| Name | Platform | Used by | Notes |
| --- | --- | --- | --- |
| `GITHUB_TOKEN` | GitHub | image (GHCR), coverage, doc-review, promote, release | auto-provided |
| `ANTHROPIC_API_KEY` | both | doc-review | masked; without it doc-review skips cleanly |
| `CI_REGISTRY_*` | GitLab | image | auto-provided |
| `GITLAB_TOKEN` | GitLab | doc-review, promote | project access token, `api` scope, masked |

## Notes / decisions

- **clippy is advisory and there is no fmt gate** — the repo sets `-A warnings` globally in
  `.cargo/config.toml`, so a blocking lint would fight that policy.
- **First run:** the image job must publish `y5-ci:fedora44` before other jobs can run in
  it (trigger `image` once manually / via `workflow_dispatch`).
- **GitHub Pages / branch protection / approvals** are platform settings, not repo files.
