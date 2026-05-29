import json
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace


sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))

import account_history_agent as agent


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests" / "fixtures"

REQUEST = {
    "ids": {"bet_id": "paper-bet-1", "coupon_simulation_id": None},
    "selection": {
        "event_name": "Team Fog Næstved - Bakken Bears",
        "event_names": ["Team Fog Næstved - Bakken Bears"],
        "sport_key": "basketball",
        "market_name": "Kampvinder (Inkl. OT)",
        "outcome_name": "Bakken Bears",
    },
    "evidence_template": {
        "source_key": "danskespil_account_history",
        "bet_id": "paper-bet-1",
        "event_name": "Team Fog Næstved - Bakken Bears",
        "sport_key": "basketball",
        "market_name": "Kampvinder (Inkl. OT)",
        "outcome_name": "Bakken Bears",
        "settle": False,
    },
}

COUPON_REQUEST = {
    "ids": {"bet_id": None, "coupon_simulation_id": "paper-coupon-1"},
    "selection": {
        "event_name": None,
        "event_names": [
            "Team Fog Næstved - Bakken Bears",
            "Fajing Sun - Jay Dylan Hara Friend",
        ],
        "sport_key": "mixed",
        "market_name": "Double",
        "outcome_name": "Coupon",
        "legs": [
            {"event_name": "Team Fog Næstved - Bakken Bears"},
            {"event_name": "Fajing Sun - Jay Dylan Hara Friend"},
        ],
    },
    "evidence_template": {
        "source_key": "danskespil_account_history",
        "bet_id": None,
        "coupon_simulation_id": "paper-coupon-1",
        "event_name": None,
        "settle": False,
    },
}


class AccountHistoryAgentTests(unittest.TestCase):
    def test_matches_event_with_danish_alias_normalization(self) -> None:
        text = """
        Kuponhistorik
        Team FOG Naestved
        Bakken Bears
        Vundet
        """
        extracted = agent.history_text_to_extracted(text, "fixture", None, "test-session")
        context = agent.find_context(
            extracted["lines"],
            ["Team Fog Næstved - Bakken Bears"],
            radius=3,
        )

        self.assertIsNotNone(context)
        self.assertEqual(agent.infer_status(context or ""), ("won", "vundet"))

    def test_ambiguous_status_requires_manual_review(self) -> None:
        context = "Team Fog Næstved Bakken Bears Vundet Tabt"

        self.assertIsNone(agent.infer_status(context))

    def test_payload_sanitizes_account_url_query(self) -> None:
        extracted = {
            "title": "Danske Spil history",
            "url": "https://danskespil.dk/konto/spilhistorik?ticket=redacted#details",
            "session_name": "test-session",
        }
        payload = agent.build_payload(
            REQUEST,
            "refunded",
            "refunderet",
            "Team FOG Naestved Bakken Bears refunderet",
            extracted,
            settle=False,
        )

        self.assertEqual(payload["source_url"], "https://danskespil.dk/konto/spilhistorik")
        self.assertEqual(payload["settlement_result"], "refunded")
        self.assertFalse(payload["settle"])
        self.assertEqual(payload["bet_id"], "paper-bet-1")

    def test_coupon_payload_preserves_leg_event_names(self) -> None:
        payload = agent.build_payload(
            COUPON_REQUEST,
            "won",
            "vundet",
            "Team FOG Naestved Bakken Bears Fajing Sun Jay Dylan Hara Friend vundet",
            {"title": "fixture", "url": None, "session_name": "test-session"},
            settle=False,
        )

        self.assertEqual(payload["coupon_simulation_id"], "paper-coupon-1")
        self.assertEqual(
            payload["event_name"],
            "Coupon: Team Fog Næstved - Bakken Bears / Fajing Sun - Jay Dylan Hara Friend",
        )
        self.assertEqual(
            payload["event_names"],
            [
                "Team Fog Næstved - Bakken Bears",
                "Fajing Sun - Jay Dylan Hara Friend",
            ],
        )
        self.assertNotIn("bet_id", payload)

    def test_coupon_context_requires_all_leg_events(self) -> None:
        lines = [
            "Team FOG Naestved",
            "Bakken Bears",
            "Vundet",
        ]

        context = agent.find_context(
            lines,
            agent.request_event_names(COUPON_REQUEST),
            radius=3,
            require_all_events=True,
        )

        self.assertIsNone(context)

    def test_coupon_context_can_span_all_leg_events(self) -> None:
        lines = [
            "Kupon",
            "Team FOG Naestved",
            "Bakken Bears",
            "Fajing Sun",
            "Jay Dylan Hara Friend",
            "Vundet",
        ]

        context = agent.find_context(
            lines,
            agent.request_event_names(COUPON_REQUEST),
            radius=5,
            require_all_events=True,
        )

        self.assertIsNotNone(context)
        self.assertEqual(agent.infer_status(context or ""), ("won", "vundet"))

    def test_text_fixture_loads_lines(self) -> None:
        extracted = agent.history_text_to_extracted(
            "A\n\nB\n",
            "fixture",
            None,
            "test-session",
        )

        self.assertEqual(extracted["lines"], ["A", "B"])
        self.assertEqual(extracted["line_count"], 2)

    def test_run_once_can_use_fixtures_without_browser_or_api(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            requests_json = Path(directory) / "requests.json"
            extracted_json = Path(directory) / "history.json"
            requests_json.write_text(
                '{"items": [' + json.dumps(REQUEST) + "]}",
                encoding="utf-8",
            )
            extracted_json.write_text(
                json.dumps(
                    {
                        "title": "fixture",
                        "url": "https://danskespil.dk/konto/spilhistorik?ticket=local",
                        "text": "Team FOG Naestved\nBakken Bears\nVundet",
                    }
                ),
                encoding="utf-8",
            )
            args = SimpleNamespace(
                api="http://127.0.0.1:1",
                requests_json=str(requests_json),
                extracted_json=str(extracted_json),
                history_text_file=None,
                session_name="test-session",
                history_url="https://danskespil.dk/oddset",
                wait_ms=0,
                no_open=True,
                limit=10,
                context_radius=3,
                include_nonterminal=False,
                settle=False,
                dry_run=True,
            )

            summary = agent.run_once(args)

        self.assertEqual(summary["evidence_count"], 1)
        self.assertEqual(summary["posted_count"], 0)
        self.assertEqual(summary["results"][0]["payload"]["settlement_result"], "won")

    def test_run_once_skips_conflicting_visible_account_history_statuses(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            requests_json = Path(directory) / "requests.json"
            extracted_json = Path(directory) / "history.json"
            requests_json.write_text(
                '{"items": [' + json.dumps(REQUEST) + "]}",
                encoding="utf-8",
            )
            extracted_json.write_text(
                json.dumps(
                    {
                        "title": "fixture",
                        "url": "https://danskespil.dk/konto/spilhistorik",
                        "text": "\n".join(
                            [
                                "Team FOG Naestved Bakken Bears",
                                "Vundet",
                                "older row",
                                "Team FOG Naestved Bakken Bears",
                                "Tabt",
                            ]
                        ),
                    }
                ),
                encoding="utf-8",
            )
            args = SimpleNamespace(
                api="http://127.0.0.1:1",
                requests_json=str(requests_json),
                extracted_json=str(extracted_json),
                history_text_file=None,
                session_name="test-session",
                history_url="https://danskespil.dk/oddset",
                wait_ms=0,
                no_open=True,
                limit=10,
                context_radius=1,
                include_nonterminal=False,
                settle=False,
                dry_run=True,
            )

            summary = agent.run_once(args)

        self.assertEqual(summary["evidence_count"], 0)
        self.assertEqual(summary["skipped_count"], 1)
        self.assertEqual(summary["skipped"][0]["reason"], "ambiguous_account_history_status")

    def test_checked_in_fixture_dry_run_matches_expected_statuses(self) -> None:
        args = SimpleNamespace(
            api="http://127.0.0.1:1",
            requests_json=str(FIXTURES / "account_history_requests.json"),
            extracted_json=None,
            history_text_file=str(FIXTURES / "account_history_text.txt"),
            session_name="test-session",
            history_url="https://danskespil.dk/oddset",
            wait_ms=0,
            no_open=True,
            limit=10,
            context_radius=0,
            include_nonterminal=False,
            settle=False,
            dry_run=True,
        )

        summary = agent.run_once(args)

        self.assertEqual(summary["evidence_count"], 1)
        self.assertEqual(summary["skipped_count"], 1)
        self.assertEqual(
            [item["payload"]["settlement_result"] for item in summary["results"]],
            ["won"],
        )
        self.assertEqual(summary["skipped"][0]["reason"], "nonterminal_bookmaker_status")

    def test_include_nonterminal_fixture_can_emit_unresolved_for_diagnostics(self) -> None:
        args = SimpleNamespace(
            api="http://127.0.0.1:1",
            requests_json=str(FIXTURES / "account_history_requests.json"),
            extracted_json=None,
            history_text_file=str(FIXTURES / "account_history_text.txt"),
            session_name="test-session",
            history_url="https://danskespil.dk/oddset",
            wait_ms=0,
            no_open=True,
            limit=10,
            context_radius=0,
            include_nonterminal=True,
            settle=False,
            dry_run=True,
        )

        summary = agent.run_once(args)

        self.assertEqual(summary["evidence_count"], 2)
        self.assertEqual(
            [item["payload"]["settlement_result"] for item in summary["results"]],
            ["won", "unresolved"],
        )


if __name__ == "__main__":
    unittest.main()
