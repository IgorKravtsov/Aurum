---
name: publish-to-docker-hub
description: Safely audit, build, and publish public Docker images to Docker Hub. Use when asked to push, release, republish, or verify an image on Docker Hub or a public Docker registry, including multi-platform amd64/arm64 releases.
compatibility: Requires Git, Docker with Buildx, curl, and Python 3. Gitleaks is run from its public Docker image.
allowed-tools: Bash(docker:*) Bash(git:*) Bash(curl:*) Bash(python3:*)
---

# Publish to Docker Hub

Publish reproducible public images without putting credentials, local data, or runtime configuration into an image.

## Safety rules

- Never print, copy into source, or pass a password/token as a command-line argument.
- Never use `docker login -p ...`; ask the user to run `docker login`, or pipe a token from an existing secret store to `docker login --password-stdin`.
- Never add `.env`, database dumps, backups, private keys, credential files, or user data to a build context.
- Never pass secrets through Docker `ARG`, `ENV`, labels, or ordinary build arguments. Use BuildKit secret mounts when a build genuinely requires a secret.
- Treat version tags as immutable. If a version already exists, stop unless the user explicitly approves replacing it. Moving `latest` is expected.
- Do not publish unrelated dirty-worktree changes. Report them and keep release edits isolated.
- Redact matched secret values in all output. Review scanner findings by rule, file, and line rather than displaying the captured secret.
- Publish every architecture required by the destination. Default server targets are `linux/amd64,linux/arm64`.

## 1. Establish release coordinates

Determine and confirm:

- Docker Hub namespace;
- one or more repository names;
- immutable application version;
- build context and Dockerfile for each image;
- target platforms (default: `linux/amd64,linux/arm64`);
- whether `latest` should move to this release.

Check the worktree before changing anything:

```bash
git status --short
git diff --check
```

Keep pre-existing changes separate from release changes.

## 2. Audit for secrets

Confirm local environment files are ignored and not tracked:

```bash
git ls-files --error-unmatch .env >/dev/null 2>&1 && echo "ERROR: .env is tracked" || true
git check-ignore -v .env 2>/dev/null || true
find . -name .dockerignore -print
```

Scan the complete Git history with Gitleaks. Store the report outside the repository and never print its `Secret` field:

```bash
tmp=$(mktemp -d)
docker run --rm \
  -v "$PWD:/repo:ro" \
  -v "$tmp:/out" \
  zricethezav/gitleaks:latest git \
  --no-banner --redact --report-format json \
  --report-path /out/report.json /repo || true
python3 - "$tmp/report.json" <<'PY'
import collections, json, sys
rows = json.load(open(sys.argv[1]))
for (rule, path, line), count in sorted(collections.Counter(
    (row.get("RuleID"), row.get("File"), row.get("StartLine")) for row in rows
).items()):
    print(f"{count}x {rule}: {path}:{line}")
print(f"total findings: {len(rows)}")
PY
rm -rf "$tmp"
```

Review every finding. Examples and placeholders may be false positives, but do not assume that without checking their source context safely.

Inspect every Dockerfile and `.dockerignore`. Pay special attention to `COPY . .`, generated frontend bundles, package-manager configuration, SSH material, and build arguments. Stop if a real credential or private user data is present.

## 3. Authenticate safely

List registry keys without exposing their values:

```bash
python3 - <<'PY'
import json, os
path = os.path.join(os.environ.get("DOCKER_CONFIG", os.path.expanduser("~/.docker")), "config.json")
if os.path.exists(path):
    for registry in sorted(json.load(open(path)).get("auths", {})):
        print(registry)
PY
```

If Docker Hub is absent, ask the user to run `docker login`. The user must enter the password or access token directly into Docker's prompt, never into chat or source files.

## 4. Check immutable tags

Before publishing, anonymously query each version tag. A `200` means it already exists and must not be overwritten without explicit approval:

```bash
./.agents/skills/publish-to-docker-hub/scripts/publish-image.sh \
  --check-only NAMESPACE REPOSITORY VERSION
```

## 5. Build and publish

Run the bundled helper once per image:

```bash
./.agents/skills/publish-to-docker-hub/scripts/publish-image.sh \
  NAMESPACE REPOSITORY VERSION CONTEXT [DOCKERFILE]
```

Optional environment variables:

```bash
PLATFORMS=linux/amd64,linux/arm64  # default
PUBLISH_LATEST=1                   # default; set 0 to leave latest unchanged
ALLOW_OVERWRITE=1                 # only after explicit user approval
```

The helper validates inputs, refuses an existing version by default, builds with Buildx, pushes the version and optional `latest`, inspects the manifest, and checks anonymous access.

## 6. Verify the release

Required checks after publishing:

1. `docker buildx imagetools inspect namespace/repository:version` lists every requested platform.
2. The anonymous registry manifest request returns HTTP 200.
3. Pull using an empty temporary Docker config succeeds on the current architecture:

```bash
tmp=$(mktemp -d)
docker --config "$tmp" pull NAMESPACE/REPOSITORY:VERSION
rm -rf "$tmp"
```

4. Inspect image configuration for accidentally baked application variables:

```bash
docker image inspect NAMESPACE/REPOSITORY:VERSION --format '{{range .Config.Env}}{{println .}}{{end}}'
```

5. Start the published image or Compose stack with `--no-build`, wait for health checks, and test the real health endpoint.

## 7. Integrate and record

When the repository ships Compose configuration:

- add version-pinned `image:` references;
- retain `build:` only when contributors still need local builds;
- document `docker compose pull` followed by `docker compose up -d --no-build`;
- state clearly that runtime passwords come from the destination server and are not in the image;
- update the project's changelog/update file according to repository instructions;
- commit only the intended release changes.

Report image names, version tags, manifest digests, platforms, anonymous-access result, runtime health result, and the commit hash. Never include registry credentials or application secrets in the report.
