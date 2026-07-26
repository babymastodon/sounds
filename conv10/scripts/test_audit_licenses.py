#!/usr/bin/env python3
"""Tests for source-license extraction and policy classification."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from audit_licenses import classify_license, page_title


class LicenseAuditTests(unittest.TestCase):
    def test_accepts_plain_attribution_for_commercial_use(self) -> None:
        page = (
            '<h1><a>Test sound</a></h1>'
            '<a title="Go to the full license text" '
            'href="https://creativecommons.org/licenses/by/4.0/">'
            "Attribution 4.0</a>"
        )

        name, url, commercial, attribution, decision = classify_license(
            page, "https://freesound.org/people/test/sounds/1"
        )

        self.assertEqual(name, "Attribution 4.0")
        self.assertEqual(url, "https://creativecommons.org/licenses/by/4.0/")
        self.assertTrue(commercial)
        self.assertTrue(attribution)
        self.assertIn("accepted", decision)
        self.assertEqual(page_title(page, "fallback"), "Test sound")

    def test_rejects_noncommercial_and_share_alike(self) -> None:
        for license_path in ("by-nc/4.0", "by-sa/4.0"):
            page = (
                '<a title="Go to the full license text" '
                f'href="https://creativecommons.org/licenses/{license_path}/">'
                "Restricted</a>"
            )
            _, _, commercial, _, decision = classify_license(
                page, "https://freesound.org/people/test/sounds/1"
            )
            self.assertFalse(commercial)
            self.assertIn("reject", decision)

    def test_accepts_wikimedia_cc0_file_marker(self) -> None:
        page = (
            '<meta property="og:title" content="Public-domain performance">'
            '<a href="https://creativecommons.org/publicdomain/zero/1.0/deed.en">'
            "CC0</a>"
            '<span class="licensetpl_short">CC0</span>'
        )

        _, url, commercial, attribution, _ = classify_license(
            page, "https://commons.wikimedia.org/w/index.php?curid=1"
        )

        self.assertEqual(url, "https://creativecommons.org/publicdomain/zero/1.0/")
        self.assertTrue(commercial)
        self.assertFalse(attribution)
        self.assertEqual(page_title(page, "fallback"), "Public-domain performance")


if __name__ == "__main__":
    unittest.main()
