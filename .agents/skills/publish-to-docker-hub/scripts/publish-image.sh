#!/usr/bin/env bash
# Build and publish one immutable, multi-platform Docker Hub image without
# accepting registry credentials as arguments or environment variables.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  publish-image.sh --check-only NAMESPACE REPOSITORY VERSION
  publish-image.sh NAMESPACE REPOSITORY VERSION CONTEXT [DOCKERFILE]

Environment:
  PLATFORMS       Target list (default: linux/amd64,linux/arm64)
  PUBLISH_LATEST  Set to 0 to avoid moving latest (default: 1)
  ALLOW_OVERWRITE Set to 1 only after approval to replace an existing version
EOF
}

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

manifest_status() {
  local namespace=$1 repository=$2 tag=$3 token
  token=$(curl -fsS \
    "https://auth.docker.io/token?service=registry.docker.io&scope=repository:${namespace}/${repository}:pull" \
    | python3 -c 'import json, sys; print(json.load(sys.stdin)["token"])')
  curl -sS -o /dev/null -w '%{http_code}' \
    -H "Authorization: Bearer ${token}" \
    -H 'Accept: application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json' \
    "https://registry-1.docker.io/v2/${namespace}/${repository}/manifests/${tag}"
}

validate_coordinate() {
  local label=$1 value=$2
  [[ $value =~ ^[a-z0-9]+([._-][a-z0-9]+)*$ ]] \
    || fail "$label contains unsupported characters: $value"
}

[[ $# -gt 0 ]] || { usage; exit 2; }

check_only=0
if [[ ${1:-} == "--check-only" ]]; then
  check_only=1
  shift
fi

if (( check_only )); then
  [[ $# -eq 3 ]] || { usage; exit 2; }
else
  [[ $# -ge 4 && $# -le 5 ]] || { usage; exit 2; }
fi

namespace=$1
repository=$2
version=$3
validate_coordinate namespace "$namespace"
validate_coordinate repository "$repository"
[[ $version =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]] \
  || fail "version contains unsupported characters: $version"

image="${namespace}/${repository}"
command -v docker >/dev/null || fail "docker is not installed"
docker buildx version >/dev/null || fail "docker buildx is unavailable"
docker info >/dev/null || fail "Docker daemon is unavailable"

status=$(manifest_status "$namespace" "$repository" "$version")
exists=0
visibility="unknown"
if [[ $status == 200 ]]; then
  exists=1
  visibility="public"
elif [[ $status == 401 || $status == 404 ]]; then
  # An anonymous 401 can mean either a private tag or a missing repository.
  # Consult the caller's existing Docker login without ever reading its token.
  if docker buildx imagetools inspect "${image}:${version}" >/dev/null 2>&1; then
    exists=1
    visibility="private"
  fi
else
  fail "unexpected registry response while checking ${image}:${version}: HTTP ${status}"
fi

if (( exists )); then
  echo "${image}:${version} already exists (${visibility})."
  if (( check_only )); then
    exit 0
  fi
  [[ ${ALLOW_OVERWRITE:-0} == 1 ]] \
    || fail "refusing to overwrite immutable tag ${image}:${version}; set ALLOW_OVERWRITE=1 only after explicit approval"
elif (( check_only )); then
  echo "${image}:${version} does not exist."
  exit 1
fi

context=$4
dockerfile=${5:-Dockerfile}
[[ -d $context ]] || fail "build context is not a directory: $context"
if [[ $dockerfile != /* ]]; then
  dockerfile="${context%/}/${dockerfile}"
fi
[[ -f $dockerfile ]] || fail "Dockerfile not found: $dockerfile"

platforms=${PLATFORMS:-linux/amd64,linux/arm64}
publish_latest=${PUBLISH_LATEST:-1}
tags=(-t "${image}:${version}")
if [[ $publish_latest == 1 ]]; then
  tags+=(-t "${image}:latest")
elif [[ $publish_latest != 0 ]]; then
  fail "PUBLISH_LATEST must be 0 or 1"
fi

# Registry authentication is intentionally delegated to `docker login`; no
# password or token is accepted by this script, keeping secrets out of argv.
docker buildx build \
  --platform "$platforms" \
  -f "$dockerfile" \
  "${tags[@]}" \
  --push \
  "$context"

docker buildx imagetools inspect "${image}:${version}"

status=$(manifest_status "$namespace" "$repository" "$version")
[[ $status == 200 ]] || fail "published tag is not anonymously readable: HTTP ${status}"
echo "Published public image: ${image}:${version} (${platforms})"
