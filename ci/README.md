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
| `discover-workspaces.sh` | emit the **workspace entries** (dirs with `Cargo.toml` **and** `link.json`) as JSON / lines — drives the GitHub matrix and the GitLab child pipeline |
| `gen-child-pipeline.sh` | GitLab-only: entries → a child pipeline (lint/build/test/coverage per entry + a merge job) |
| `link-drift.sh` | re-run `workspace.link.js` in every entry with a generated links block; fail if the committed block is stale (the "forgot `link.all.sh`" guard) |
| `coverage-full.sh` | per-entry coverage **including dead code** (LLVM region baseline + unit-test merge) → `.ci-coverage/<slug>.lcov` |
| `merge-coverage.sh` | fuse all entry lcov → `coverage.lcov` + `cobertura.xml` + `html/` + **per-crate `coverage-crates.md`** + self-hosted **`coverage.svg`** badge + a `Coverage: NN.N%` line |
| `build-docs.sh` | landing page → `public/`; folds in the coverage report + badge and lists the reference guides when present |
| `build-site.sh` | full Pages build: run coverage for every entry → merge → `build-docs.sh` (coverage site) |
| `doc-suggest.sh` | PR/MR-only: `claude -p` reviews the diff, posts doc/README suggestions as a comment (advisory, never commits) |
| `gen-report.sh` | compose the markdown promotion "deployment notes" (tests/coverage/lint/drift/doc) |
| `open-promotion-pr.sh` | open/update the upstream-integration→upstream PR/MR with the report (never merges) |
| `package-installer.sh` | **the CD bundle builder** — delegates to `compositor.installer/prepare.sh` to build the full install bundle (installer + compositor/dev/polkit/mx/xwayland binaries + components) → `dist/package.tar.gz` + `SHA256SUMS`. Served at `/release/latest/fedora44/` (what `get.sh` fetches) and attached to Releases |
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

The build/test/coverage unit. An *entry* is a directory (≤ 2 deep, excluding `target/`)
holding both a workspace-root `Cargo.toml` **and** a `link.json`. Today there are 9.
Nothing is hardcoded, so adding/renaming a workspace needs **no pipeline edit**.

## Branch flow

```
feature → upstream-integration ──(CI green)──▶ auto PR →upstream ──(approve & merge)──▶ upstream
        │ candidate bundle artifact (ci.yml installer-bundle)                             │
        │                                                          Publish — one build, two channels:
        │                                              ├─ Pages: site /, docs /docs, bundle /release/latest/fedora44/
        │                                              └─ GitHub Release `latest`: package.tar.gz + SHA256SUMS
```

- **upstream-integration** (candidate): full CI on every push; the `installer-bundle` job
  builds the install bundle as a downloadable artifact so the candidate can be tried before
  merge. On green, the promotion PR to `upstream` is opened/updated with the report.
- **upstream** (the single release action): protected — approve & merge the promotion request
  (set branch protection in the UI). A push here runs **Publish**, which builds the install
  bundle **once** and ships that one artifact to both channels, so they can't drift:
    - **Pages** (`nourish.snowies.com`): marketing site `/`, docs `/docs`, and the bundle at
      `/release/latest/fedora44/` (the URL `compositor.installer/get.sh` fetches).
    - **GitHub Release `latest`**: the same `package.tar.gz` + `SHA256SUMS` as assets, with the
      `latest` tag moved to the merged commit each time — tag + binaries together, no manual step.
  Live host deploy (`environment/build-release.sh remote`) remains a separate, optional, manual job.

### RC channel (release candidates)

A parallel pair of branches cuts **release candidates** without touching the stable channel.
**Promotion is fully manual** (no auto-PR) and **only `candidate` builds anything** — so you
decide exactly when an rc is built and published:

```
feature → candidate-integration ──(you merge by hand; no CI, no artifacts)──▶ candidate
                                                                                  │
                                                                  Publish RC: prerelease GitHub Release
                                                                  └─ v<X.Y.Z-rc.N>  prerelease rc download
```

- **candidate-integration** (rc aggregation): a plain staging branch — **no workflow, no
  artifacts**. Stack commits here; when you choose to cut an rc, merge `candidate-integration`
  into `candidate` yourself (open the PR / fast-forward by hand). That merge is the deliberate
  build-and-publish gate.
- **candidate** (the single rc release action): a push (or `workflow_dispatch`) runs **Publish
  RC** (`release-rc.yml`), which **just builds** the bundle once (no cargo checks) with an
  `X.Y.Z-rc.N` version (`ci/scripts/version-rc.sh` — same VERSION-file mechanics as
  `version.sh`, with the `-rc.N` counter derived from `v…-rc.*` tags) and ships it as one
  **prerelease** GitHub Release: `v<X.Y.Z-rc.N>`. **No Pages deploy** — Pages is the stable
  channel's single site — and no rolling pointer (nothing consumes it). The rc release is
  marked non-latest so it never steals the stable "Latest release" pointer.

Install an rc exactly like a stable release — same command, just the exact rc version's URL.
**No env vars, and the install script is untouched** (`get.sh` / the bundle's `install.sh` are
identical for both channels — only the tarball URL differs, so install never diverges):

```
curl -fsSL https://github.com/<owner>/<repo>/releases/download/v<X.Y.Z-rc.N>/package.tar.gz \
  | tar -xz && y5-install/install.sh
```

> Why not a `--rc` flag in `get.sh`? `get.sh` is a single shared script served only from the
> stable channel, so a per-channel flag would be dead code unless landed on `upstream` —
> coupling rc installs to a stable release. A distinct URL keeps the install path channel-
> agnostic. (If you later want checksum-verified rc installs through the `/install` bootstrap,
> make `get.sh` channel-aware on `upstream` — one canonical script — not on `candidate`.)

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
| `Y5_DEPLOY_SSH_KEY` | GitLab (optional) | deploy-remote | masked SSH key for `yrd.local` |

## Notes / decisions

- **clippy is advisory and there is no fmt gate** — the repo sets `-A warnings` globally in
  `.cargo/config.toml`, so a blocking lint would fight that policy.
- **First run:** the image job must publish `y5-ci:fedora44` before other jobs can run in
  it (trigger `image` once manually / via `workflow_dispatch`).
- **GitHub Pages / branch protection / approvals** are platform settings, not repo files.
