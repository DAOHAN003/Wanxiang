#!/usr/bin/env python3
"""Offline tests for live deployment environment preflight."""
from __future__ import annotations

import hashlib
import importlib.util
from pathlib import Path
import unittest


SCRIPT = Path(__file__).with_name("validate-live-deployment-env.py")
SPEC = importlib.util.spec_from_file_location("validate_live_deployment_env", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)


def config(**overrides):
    values = {
        "WANXIANG_DOMAIN": "api.wanxiang.dev",
        "WANXIANG_API_TOKEN": hashlib.sha256(b"wanxiang preflight unit test token").hexdigest(),
        "WANXIANG_CORS_ORIGIN": "https://app.wanxiang.dev",
        "WANXIANG_TICK_MS": "1000",
        "WANXIANG_WORLD_SEED": "42",
        "WANXIANG_RATE_LIMIT_AGENT_PER_MIN": "12",
        "WANXIANG_RATE_LIMIT_EXPERIMENT_PER_MIN": "30",
        "WANXIANG_RATE_LIMIT_STREAM_PER_MIN": "6",
        "WANXIANG_RATE_LIMIT_SESSIONS_PER_MIN": "12",
        "WANXIANG_RATE_LIMIT_BURST": "3",
    }
    values.update(overrides)
    return values


class LiveDeploymentPreflightTests(unittest.TestCase):
    def test_valid_minimal_api_configuration(self):
        self.assertEqual(VALIDATOR.validate_values(config()), [])

    def test_accepts_optional_complete_llm_configuration(self):
        values = config(
            WANXIANG_LLM_ENDPOINT="https://api.provider.example/v1",
            WANXIANG_LLM_MODEL="test-model",
            WANXIANG_LLM_API_KEY="not-empty-test-secret",
        )
        self.assertEqual(VALIDATOR.validate_values(values), [])

    def test_rejects_placeholder_domain(self):
        for domain in ("api.example.com", "api.example.org", "api.example.net", "api.wanxiang.invalid"):
            with self.subTest(domain=domain):
                errors = VALIDATOR.validate_values(config(WANXIANG_DOMAIN=domain))
                self.assertTrue(any("placeholder" in error for error in errors))

    def test_rejects_short_token_without_echoing_it(self):
        errors = VALIDATOR.validate_values(config(WANXIANG_API_TOKEN="too-short"))
        self.assertTrue(any("at least 32 characters" in error for error in errors))
        self.assertFalse(any("too-short" in error for error in errors))

    def test_rejects_long_but_low_diversity_token(self):
        errors = VALIDATOR.validate_values(config(WANXIANG_API_TOKEN="a" * 64))
        self.assertTrue(any("low character diversity" in error for error in errors))

    def test_rejects_wildcard_and_path_in_cors_origin(self):
        wildcard = VALIDATOR.validate_values(config(WANXIANG_CORS_ORIGIN="*"))
        self.assertTrue(any("wildcards are not allowed" in error for error in wildcard))
        path = VALIDATOR.validate_values(config(WANXIANG_CORS_ORIGIN="https://app.example.org/path"))
        self.assertTrue(any("no path" in error for error in path))

    def test_allows_http_only_for_loopback_cors_and_provider(self):
        local = config(
            WANXIANG_CORS_ORIGIN="http://127.0.0.1:3000",
            WANXIANG_LLM_ENDPOINT="http://localhost:1234/v1",
            WANXIANG_LLM_MODEL="local-model",
            WANXIANG_LLM_API_KEY="local-test-key",
        )
        self.assertEqual(VALIDATOR.validate_values(local), [])
        remote_http = config(
            WANXIANG_LLM_ENDPOINT="http://provider.example.org/v1",
            WANXIANG_LLM_MODEL="test-model",
            WANXIANG_LLM_API_KEY="test-key",
        )
        self.assertTrue(any("must use HTTPS except for loopback" in e
                            for e in VALIDATOR.validate_values(remote_http)))

    def test_rejects_partial_provider_configuration(self):
        errors = VALIDATOR.validate_values(config(WANXIANG_LLM_MODEL="only-model"))
        self.assertTrue(any("configured together" in error for error in errors))

    def test_rejects_invalid_tick_and_seed(self):
        errors = VALIDATOR.validate_values(config(WANXIANG_TICK_MS="-1", WANXIANG_WORLD_SEED="abc"))
        self.assertTrue(any("WANXIANG_TICK_MS" in error for error in errors))
        self.assertTrue(any("WANXIANG_WORLD_SEED" in error for error in errors))

    def test_rejects_invalid_rate_limit_settings(self):
        errors = VALIDATOR.validate_values(config(
            WANXIANG_RATE_LIMIT_AGENT_PER_MIN="0",
            WANXIANG_RATE_LIMIT_STREAM_PER_MIN="lots",
            WANXIANG_RATE_LIMIT_BURST="31",
        ))
        self.assertTrue(any("WANXIANG_RATE_LIMIT_AGENT_PER_MIN" in error for error in errors))
        self.assertTrue(any("WANXIANG_RATE_LIMIT_STREAM_PER_MIN" in error for error in errors))
        self.assertTrue(any("WANXIANG_RATE_LIMIT_BURST" in error for error in errors))

    def test_accepts_rate_limit_upper_bounds(self):
        errors = VALIDATOR.validate_values(config(
            WANXIANG_RATE_LIMIT_AGENT_PER_MIN="600",
            WANXIANG_RATE_LIMIT_EXPERIMENT_PER_MIN="600",
            WANXIANG_RATE_LIMIT_STREAM_PER_MIN="600",
            WANXIANG_RATE_LIMIT_SESSIONS_PER_MIN="600",
            WANXIANG_RATE_LIMIT_BURST="30",
        ))
        self.assertEqual(errors, [])

    def test_parser_rejects_duplicate_assignments(self):
        with self.assertRaisesRegex(ValueError, "duplicate environment variable"):
            VALIDATOR.parse_env_text("WANXIANG_API_TOKEN=a\nWANXIANG_API_TOKEN=b\n")

    def test_parser_accepts_comments_and_quoted_values(self):
        parsed = VALIDATOR.parse_env_text('# comment\nWANXIANG_API_TOKEN="secret text"\n')
        self.assertEqual(parsed["WANXIANG_API_TOKEN"], "secret text")


if __name__ == "__main__":
    unittest.main(verbosity=2)
