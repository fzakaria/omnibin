#!/usr/bin/env bash
# Pushes docker/overview.md to the Docker Hub repository page.
#
# The overview lives in this repository so it is reviewed and versioned like
# everything else, rather than typed into a web form where nobody can see it
# change. This is what copies it over.
#
# Authentication comes from whatever `docker login` already wrote: the token
# in ~/.docker/config.json for index.docker.io is exchanged for a Hub API
# token, which is the one the repository endpoint accepts. The registry
# access-token beside it is not: that one signs pulls and pushes and the web
# API rejects it.
#
# Usage:
#   tools/push-docker-overview.sh                 # fmzakari/omnibin
#   tools/push-docker-overview.sh other/repo
set -euo pipefail

ROOT="${OMNIBIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
REPO="${1:-fmzakari/omnibin}"
OVERVIEW="$ROOT/docker/overview.md"

if [ ! -f "$OVERVIEW" ]; then
  echo "push-docker-overview: no $OVERVIEW" >&2
  exit 1
fi

python3 - "$REPO" "$OVERVIEW" <<'PY'
import base64, json, os, sys, urllib.error, urllib.request

repo, overview_path = sys.argv[1:3]
overview = open(overview_path).read()

config = os.path.expanduser("~/.docker/config.json")
if not os.path.exists(config):
    sys.exit("push-docker-overview: no docker config; run `docker login` first")

auths = json.load(open(config)).get("auths", {})
entry = auths.get("https://index.docker.io/v1/", {})
if "auth" not in entry:
    sys.exit("push-docker-overview: no index.docker.io credential; run `docker login`")

user, secret = base64.b64decode(entry["auth"]).decode().split(":", 1)


def call(url, payload, method, token=None):
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(
        url, data=json.dumps(payload).encode(), headers=headers, method=method
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as e:
        sys.exit(f"push-docker-overview: {url} -> {e.code} {e.read()[:200].decode()}")


_, auth = call(
    "https://hub.docker.com/v2/auth/token",
    {"identifier": user, "secret": secret},
    "POST",
)
token = auth.get("access_token")
if not token:
    sys.exit("push-docker-overview: the credential did not yield an API token")

# The short description is the one line search results show; the full one is
# the page body.
status, body = call(
    f"https://hub.docker.com/v2/repositories/{repo}/",
    {
        "full_description": overview,
        "description": (
            "Every binary nixpkgs ever shipped, on your PATH. "
            "51,468 commands, fetched only when run."
        ),
    },
    "PATCH",
    token,
)
print(f"{repo}: {status}, {len(body.get('full_description') or '')} bytes of overview")
PY
