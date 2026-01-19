#!/usr/bin/env bash
# Generate a scoped GitHub token using the GitHub App
# This script runs on the HOST before starting the container

set -e

CREDS_DIR="$(pwd)/.devcontainer/.credentials"
TOKEN_FILE="$CREDS_DIR/github_token.json"
CONFIG_FILE="$HOME/.config/drmaciver-project/github_app.json"

# Check if GitHub App is configured
if [ ! -f "$CONFIG_FILE" ]; then
    echo "GitHub App not configured - skipping token generation"
    echo "To enable GitHub access, run: drmaciver-project setup-github-app"
    exit 0
fi

# Check if uv is available
if ! command -v uv &> /dev/null; then
    echo "WARNING: uv not found - cannot generate GitHub token"
    exit 0
fi

mkdir -p "$CREDS_DIR"

# Generate token using inline Python with uv
uv run --quiet --with pyjwt --with httpx --with cryptography python << 'PYTHON_SCRIPT'
import json
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

import httpx
import jwt

TEMPLATE_REPO = "drmaciver-project"
PROJECT_NAME = "claude-reliability"
CONFIG_PATH = Path.home() / ".config" / "drmaciver-project" / "github_app.json"
CREDS_DIR = Path(".devcontainer/.credentials")
TOKEN_FILE = CREDS_DIR / "github_token.json"


def generate_jwt(app_id: int, private_key: str) -> str:
    now = int(time.time())
    payload = {"iat": now - 60, "exp": now + 600, "iss": app_id}
    return jwt.encode(payload, private_key, algorithm="RS256")


def get_installation(app_jwt: str, owner: str, repo: str) -> dict | None:
    url = f"https://api.github.com/repos/{owner}/{repo}/installation"
    response = httpx.get(
        url,
        headers={
            "Authorization": f"Bearer {app_jwt}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    if response.status_code == 404:
        return None
    response.raise_for_status()
    return response.json()


def generate_token(
    app_jwt: str, installation_id: int, repos: list[str]
) -> tuple[str, str]:
    url = f"https://api.github.com/app/installations/{installation_id}/access_tokens"
    response = httpx.post(
        url,
        headers={
            "Authorization": f"Bearer {app_jwt}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        },
        json={
            "repositories": repos,
            "permissions": {
                "contents": "write",
                "issues": "write",
                "pull_requests": "write",
                "metadata": "read",
                "administration": "write",
            },
        },
    )
    response.raise_for_status()
    data = response.json()
    token = data.get("token")
    if not token:
        print(f"ERROR: No token in GitHub API response: {data}")
        sys.exit(1)
    return token, data.get("expires_at", "")


def get_repo_from_git() -> tuple[str, str]:
    try:
        result = subprocess.run(
            ["git", "remote", "get-url", "origin"],
            capture_output=True,
            text=True,
            check=True,
        )
        remote_url = result.stdout.strip()
        if "github.com" in remote_url:
            if remote_url.startswith("git@"):
                parts = remote_url.split(":")[-1].replace(".git", "").split("/")
            else:
                parts = remote_url.replace(".git", "").split("/")[-2:]
            return parts[0], parts[1]
    except (subprocess.CalledProcessError, IndexError, ValueError):
        pass
    return "DRMacIver", PROJECT_NAME


if not CONFIG_PATH.exists():
    print("GitHub App config not found")
    sys.exit(0)

try:
    config = json.loads(CONFIG_PATH.read_text())
except json.JSONDecodeError as e:
    print(f"ERROR: Invalid JSON in {CONFIG_PATH}: {e}")
    sys.exit(1)

app_id = config.get("app_id")
private_key_path_str = config.get("private_key_path")
if not app_id or not private_key_path_str:
    print(f"ERROR: Missing app_id or private_key_path in {CONFIG_PATH}")
    sys.exit(1)
private_key_path = Path(private_key_path_str)

if not private_key_path.exists():
    print(f"Private key not found: {private_key_path}")
    sys.exit(1)

private_key = private_key_path.read_text()
app_jwt = generate_jwt(app_id, private_key)
owner, repo = get_repo_from_git()

installation = get_installation(app_jwt, owner, repo)
if installation is None:
    installation = get_installation(app_jwt, "DRMacIver", TEMPLATE_REPO)
    if installation is None:
        print(f"GitHub App not installed on {owner}/{repo} or template repo")
        sys.exit(1)

installation_id = installation.get("id")
if not installation_id:
    print(f"ERROR: No installation ID in response: {installation}")
    sys.exit(1)
repos = list(set([repo, TEMPLATE_REPO]))
token, expires_at = generate_token(app_jwt, installation_id, repos)

CREDS_DIR.mkdir(parents=True, exist_ok=True)
TOKEN_FILE.write_text(
    json.dumps(
        {
            "token": token,
            "expires_at": expires_at,
            "repos": repos,
            "generated_at": datetime.now(timezone.utc).isoformat(),
        },
        indent=2,
    )
    + "\n"
)
TOKEN_FILE.chmod(0o600)
print(f"GitHub token generated (expires: {expires_at[:19] if expires_at else 'unknown'})")
PYTHON_SCRIPT
