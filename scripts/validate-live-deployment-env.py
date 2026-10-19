#!/usr/bin/env python3
"""Fail-closed preflight for the self-hosted live API environment file.

This is a configuration sanity check, not a substitute for reviewing the host,
network exposure, authentication model or operational controls.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import sys
from urllib.parse import urlsplit


PLACEHOLDER_DOMAINS = {"api.example.com", "api.example.test", "example.com", "localhost"}
REQUIRED = ("WANXIANG_DOMAIN", "WANXIANG_API_TOKEN", "WANXIANG_CORS_ORIGIN")
PROVIDER_FIELDS = ("WANXIANG_LLM_ENDPOINT", "WANXIANG_LLM_MODEL", "WANXIANG_LLM_API_KEY")
RATE_LIMIT_FIELDS = {
    "WANXIANG_RATE_LIMIT_AGENT_PER_MIN": 600,
    "WANXIANG_RATE_LIMIT_EXPERIMENT_PER_MIN": 600,
    "WANXIANG_RATE_LIMIT_STREAM_PER_MIN": 600,
    "WANXIANG_RATE_LIMIT_SESSIONS_PER_MIN": 600,
    "WANXIANG_RATE_LIMIT_BURST": 30,
}
LOOPBACK_HOSTS = {"localhost", "127.0.0.1", "::1"}


def parse_env_text(text: str) -> dict[str, str]:
    """Parse the simple KEY=VALUE form used by .env.live.example without logging values."""
    values: dict[str, str] = {}
    for number, raw in enumerate(text.splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        key = key.strip()
        if not separator or not re.fullmatch(r"[A-Z][A-Z0-9_]*", key):
            raise ValueError(f"invalid environment assignment on line {number}")
        if key in values:
            raise ValueError(f"duplicate environment variable {key} on line {number}")
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in ("'", '"'):
            value = value[1:-1]
        values[key] = value
    return values


def _origin_error(raw: str) -> str | None:
    if not raw or raw == "*":
        return "must be an explicit HTTPS origin (wildcards are not allowed)"
    parsed = urlsplit(raw)
    if not parsed.scheme or not parsed.hostname or parsed.username or parsed.password:
        return "must be a valid origin without credentials"
    if parsed.scheme not in ("https", "http"):
        return "must use HTTPS (HTTP is allowed only for loopback development origins)"
    if parsed.scheme == "http" and parsed.hostname.lower() not in LOOPBACK_HOSTS:
        return "must use HTTPS unless the origin is loopback"
    if parsed.path not in ("", "/") or parsed.query or parsed.fragment:
        return "must contain only scheme and authority, with no path, query or fragment"
    if any(ch.isspace() for ch in raw):
        return "must not contain whitespace"
    return None


def validate_values(values: dict[str, str]) -> list[str]:
    """Return safe, human-readable errors. Never include token or API-key values."""
    errors: list[str] = []
    for key in REQUIRED:
        if not values.get(key, "").strip():
            errors.append(f"{key} is required and must not be empty")

    domain = values.get("WANXIANG_DOMAIN", "").strip()
    if domain:
        if "://" in domain or "/" in domain or any(ch.isspace() for ch in domain):
            errors.append("WANXIANG_DOMAIN must be a hostname only, without scheme, path or whitespace")
        elif domain.lower() in PLACEHOLDER_DOMAINS or any(domain.lower().endswith(suffix) for suffix in (".example.com", ".example.org", ".example.net", ".example.test", ".invalid", ".test")):
            errors.append("WANXIANG_DOMAIN is still a placeholder; configure a domain you control")
        elif "." not in domain or domain.startswith(".") or domain.endswith("."):
            errors.append("WANXIANG_DOMAIN must be a fully-qualified public hostname")
        elif not re.fullmatch(r"[A-Za-z0-9.-]+", domain) or ".." in domain:
            errors.append("WANXIANG_DOMAIN contains invalid hostname characters")

    token = values.get("WANXIANG_API_TOKEN", "")
    if token:
        if len(token) < 32:
            errors.append("WANXIANG_API_TOKEN must be at least 32 characters; generate a fresh random token")
        if len(set(token)) < 8:
            errors.append("WANXIANG_API_TOKEN has very low character diversity; generate a fresh random token")
        if token.lower() in {"replace-me", "changeme", "secret", "password"}:
            errors.append("WANXIANG_API_TOKEN is a placeholder")
        if any(ch.isspace() for ch in token):
            errors.append("WANXIANG_API_TOKEN must not contain whitespace")

    cors = values.get("WANXIANG_CORS_ORIGIN", "")
    if cors:
        origins = [entry.strip() for entry in cors.split(",")]
        if any(not origin for origin in origins):
            errors.append("WANXIANG_CORS_ORIGIN contains an empty origin")
        for origin in origins:
            issue = _origin_error(origin)
            if issue:
                errors.append(f"WANXIANG_CORS_ORIGIN entry {origin!r} {issue}")

    provider_values = [values.get(key, "").strip() for key in PROVIDER_FIELDS]
    if any(provider_values) and not all(provider_values):
        missing = [key for key, value in zip(PROVIDER_FIELDS, provider_values) if not value]
        errors.append("LLM provider settings must be configured together; missing " + ", ".join(missing))
    endpoint = values.get("WANXIANG_LLM_ENDPOINT", "").strip()
    if endpoint:
        parsed = urlsplit(endpoint)
        if not parsed.scheme or not parsed.hostname or parsed.username or parsed.password:
            errors.append("WANXIANG_LLM_ENDPOINT must be a valid URL without embedded credentials")
        elif parsed.scheme not in ("https", "http"):
            errors.append("WANXIANG_LLM_ENDPOINT must use HTTPS")
        elif parsed.scheme == "http" and parsed.hostname.lower() not in LOOPBACK_HOSTS:
            errors.append("WANXIANG_LLM_ENDPOINT must use HTTPS except for loopback development")
        if any(ch.isspace() for ch in endpoint):
            errors.append("WANXIANG_LLM_ENDPOINT must not contain whitespace")

    tick = values.get("WANXIANG_TICK_MS", "1000").strip()
    try:
        tick_value = int(tick)
        if tick_value < 0:
            errors.append("WANXIANG_TICK_MS must be a non-negative integer (0 disables auto-tick)")
    except ValueError:
        errors.append("WANXIANG_TICK_MS must be a non-negative integer (0 disables auto-tick)")

    seed = values.get("WANXIANG_WORLD_SEED", "42").strip()
    try:
        if int(seed) < 0:
            errors.append("WANXIANG_WORLD_SEED must be a non-negative integer")
    except ValueError:
        errors.append("WANXIANG_WORLD_SEED must be a non-negative integer")

    for key, maximum in RATE_LIMIT_FIELDS.items():
        if key not in values:
            continue
        raw = values[key].strip()
        try:
            number = int(raw)
            if number < 1 or number > maximum:
                errors.append(f"{key} must be an integer between 1 and {maximum}")
        except ValueError:
            errors.append(f"{key} must be an integer between 1 and {maximum}")
    return errors


def validate_env_file(path: Path) -> tuple[dict[str, str] | None, list[str]]:
    if not path.is_file():
        return None, [f"environment file not found: {path}"]
    try:
        values = parse_env_text(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, ValueError) as exc:
        return None, [str(exc)]
    return values, validate_values(values)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("env_file", nargs="?", type=Path, default=Path(".env.live"))
    args = parser.parse_args()
    values, errors = validate_env_file(args.env_file)
    if values is None:
        print("LIVE API PRE-FLIGHT: FAILED")
    elif errors:
        print("LIVE API PRE-FLIGHT: FAILED")
    else:
        print("LIVE API PRE-FLIGHT: PASSED")
        print(f"Environment file: {args.env_file}")
        print(f"LLM provider configured: {'yes' if all(values.get(key, '').strip() for key in PROVIDER_FIELDS) else 'no'}")
        print("Secrets were not displayed.")
        return 0
    for issue in errors:
        print(f"- {issue}", file=sys.stderr)
    print("Do not expose the API publicly until this preflight and the deployment runbook pass.", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
