---
name: openclaw-gui-release
description: Use this skill when releasing or publishing a new version of the openclaw-gui project. It covers syncing project versions, validating the Tauri app locally, pushing main and a v* tag, and checking the GitHub Actions CI/Release pipelines and GitHub Release assets for mdddj/openclaw-gui.
---

# OpenClaw GUI Release

## Overview

This is the project-local release workflow for `openclaw-gui`.

Use it for requests like:

- "发新版本"
- "发布到 GitHub Release"
- "把版本升到 0.1.1"
- "重打 release tag"
- "检查 release workflow"

## Project Facts

- GitHub repo: `mdddj/openclaw-gui`
- Release trigger: annotated tag `vX.Y.Z`
- CI workflow: [`.github/workflows/ci.yml`](../../../../.github/workflows/ci.yml)
- Release workflow: [`.github/workflows/release.yml`](../../../../.github/workflows/release.yml)
- Version fields that must stay in sync:
  - [`package.json`](../../../../package.json)
  - [`src-tauri/Cargo.toml`](../../../../src-tauri/Cargo.toml)
  - [`src-tauri/tauri.conf.json`](../../../../src-tauri/tauri.conf.json)

## Release Workflow

1. Confirm the target version. Prefer plain `X.Y.Z`; the Git tag should be `vX.Y.Z`.
2. Check branch and working tree first.
   Release work should normally start from `main`.
   If unrelated local changes are present, stop and ask before mixing them into a release.
3. Sync version numbers with the bundled script:

```bash
python3 .codex/skills/openclaw-gui-release/scripts/set_version.py 0.1.1
```

4. Run local validation before pushing:

```bash
pnpm build
cargo check --manifest-path src-tauri/Cargo.toml
```

5. Commit and push `main`.
   If the release contains only version bumps, staging the three version files is enough.
   If the release also includes feature or fix work, stage those files intentionally rather than only the version files.

```bash
git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "Release v0.1.1"
git push origin main
```

6. Create and push the release tag:

```bash
git tag -a v0.1.1 -m "Release v0.1.1"
git push origin v0.1.1
```

7. Monitor GitHub Actions:

```bash
gh run list --repo mdddj/openclaw-gui --limit 10
gh run view <run-id> --repo mdddj/openclaw-gui
gh run view <run-id> --repo mdddj/openclaw-gui --log-failed
```

8. Verify the Release page:
   The release should appear at [GitHub Releases](https://github.com/mdddj/openclaw-gui/releases).
   Do not claim completion until the release exists or the workflow has clearly reached a successful state.

## Retry Rules

- If `CI` fails on `main`, fix the code on `main` first.
- If the `Release` workflow fails because of workflow configuration, fix the workflow on `main`, delete the old tag, recreate the same tag on the new commit, and push it again.
- macOS assets may appear before Windows or Linux assets. Partial assets do not mean the release is finished.

## Bundled Script

### `scripts/set_version.py`

Use this script to update all project version files in one step.

- Accepts `0.1.1` or `v0.1.1`
- Writes normalized version without the leading `v`
- Supports `--dry-run` for previewing changes
