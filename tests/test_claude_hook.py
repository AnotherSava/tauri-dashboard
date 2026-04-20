"""Tests for integrations/claude_hook.py"""
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "integrations"))

from claude_hook import (
    BUNDLE_IDENTIFIER,
    DEFAULT_PORT,
    build_body,
    classify,
    default_config_path,
    derive_chat_id,
    last_assistant_ends_with_question,
    load_config,
    widget_url,
)


def _write_transcript(lines: list[dict]) -> Path:
    fd, raw_path = tempfile.mkstemp(suffix=".jsonl")
    os.close(fd)
    path = Path(raw_path)
    with path.open("w", encoding="utf-8") as f:
        for obj in lines:
            f.write(json.dumps(obj) + "\n")
    return path


def _assistant_text_line(text: str) -> dict:
    return {"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}}


class DeriveChatIdTests(unittest.TestCase):
    def test_subfolder_of_projects_root_uses_spaced_relpath(self) -> None:
        self.assertEqual(derive_chat_id("D:/projects/bga/assistant", "", "d:/projects"), "bga assistant")

    def test_dashes_and_underscores_become_spaces(self) -> None:
        self.assertEqual(derive_chat_id("d:/projects/foo-bar/sub_dir/leaf", "", "d:/projects"), "foo bar sub dir leaf")

    def test_root_match_is_case_insensitive(self) -> None:
        self.assertEqual(derive_chat_id("D:/PROJECTS/thing", "", "d:/projects"), "thing")

    def test_backslash_separators_are_normalized(self) -> None:
        self.assertEqual(derive_chat_id("D:\\projects\\sub\\deep", "", "d:/projects"), "sub deep")

    def test_trailing_slash_on_cwd_is_tolerated(self) -> None:
        self.assertEqual(derive_chat_id("d:/projects/foo-bar/", "", "d:/projects"), "foo bar")

    def test_exact_root_falls_back_to_basename(self) -> None:
        self.assertEqual(derive_chat_id("d:/projects", "", "d:/projects"), "projects")

    def test_outside_projects_root_uses_basename(self) -> None:
        self.assertEqual(derive_chat_id("c:/Users/foo/bar", "", "d:/projects"), "bar")

    def test_no_projects_root_configured_uses_basename(self) -> None:
        self.assertEqual(derive_chat_id("d:/projects/sub/deep", "", None), "deep")

    def test_no_cwd_uses_session_id_prefix(self) -> None:
        self.assertEqual(derive_chat_id("", "abcdef1234", "d:/projects"), "claude-abcdef12")

    def test_no_cwd_and_no_session_returns_unknown(self) -> None:
        self.assertEqual(derive_chat_id("", "", "d:/projects"), "claude-unknown")

    def test_whitespace_only_cwd_treated_as_missing(self) -> None:
        self.assertEqual(derive_chat_id("   ", "abcdef1234", "d:/projects"), "claude-abcdef12")


class BuildBodyTests(unittest.TestCase):
    def test_working_with_prompt_includes_label(self) -> None:
        body = build_body("working", {"prompt": "fix the bug"}, "demo")
        self.assertEqual(body["action"], "set")
        self.assertEqual(body["id"], "demo")
        self.assertEqual(body["status"], "working")
        self.assertEqual(body["label"], "fix the bug")
        self.assertEqual(body["source"], "claude")
        self.assertIsInstance(body["updated"], int)

    def test_working_preserves_full_prompt_length(self) -> None:
        body = build_body("working", {"prompt": "x" * 200}, "demo")
        self.assertEqual(len(body["label"]), 200)

    def test_working_flattens_multiline_prompt_with_single_spaces(self) -> None:
        body = build_body("working", {"prompt": "first line\nsecond  line\twith\ttabs"}, "demo")
        self.assertEqual(body["label"], "first line second line with tabs")

    def test_working_with_empty_prompt_omits_label(self) -> None:
        body = build_body("working", {"prompt": "   "}, "demo")
        self.assertNotIn("label", body)

    def test_done_without_transcript_emits_done_and_omits_label(self) -> None:
        body = build_body("done", {"prompt": "ignored"}, "demo")
        self.assertEqual(body["status"], "done")
        self.assertNotIn("label", body)

    def test_done_with_question_ending_becomes_awaiting(self) -> None:
        transcript = _write_transcript([_assistant_text_line("Should I proceed?")])
        try:
            body = build_body("done", {"transcript_path": str(transcript)}, "demo")
            self.assertEqual(body["status"], "awaiting")
            self.assertEqual(body["label"], "has a question")
        finally:
            transcript.unlink()

    def test_done_without_question_ending_stays_done(self) -> None:
        transcript = _write_transcript([_assistant_text_line("All tests passing.")])
        try:
            body = build_body("done", {"transcript_path": str(transcript)}, "demo")
            self.assertEqual(body["status"], "done")
            self.assertNotIn("label", body)
        finally:
            transcript.unlink()

    def test_idle_session_start_has_no_notification_type(self) -> None:
        body = build_body("idle", {"cwd": "/some/path"}, "demo")
        self.assertEqual(body["status"], "idle")
        self.assertNotIn("label", body)

    def test_notification_permission_prompt_becomes_awaiting(self) -> None:
        body = build_body(
            "idle",
            {"notification_type": "permission_prompt", "message": "Claude needs your permission to use Bash"},
            "demo",
        )
        self.assertEqual(body["status"], "awaiting")
        self.assertEqual(body["label"], "needs approval: Bash")

    def test_notification_plan_approval_becomes_awaiting(self) -> None:
        body = build_body("idle", {"notification_type": "plan_approval", "message": "ignored"}, "demo")
        self.assertEqual(body["status"], "awaiting")
        self.assertEqual(body["label"], "plan approval")

    def test_notification_idle_prompt_with_question_becomes_awaiting(self) -> None:
        transcript = _write_transcript([_assistant_text_line("What would you like me to do next?")])
        try:
            body = build_body(
                "idle",
                {"notification_type": "idle_prompt", "transcript_path": str(transcript)},
                "demo",
            )
            self.assertEqual(body["status"], "awaiting")
            self.assertEqual(body["label"], "has a question")
        finally:
            transcript.unlink()

    def test_notification_idle_prompt_without_question_becomes_done(self) -> None:
        transcript = _write_transcript([_assistant_text_line("All set.")])
        try:
            body = build_body(
                "idle",
                {"notification_type": "idle_prompt", "transcript_path": str(transcript)},
                "demo",
            )
            self.assertEqual(body["status"], "done")
            self.assertNotIn("label", body)
        finally:
            transcript.unlink()

    def test_notification_without_type_but_with_message_becomes_awaiting(self) -> None:
        body = build_body("idle", {"message": "Claude needs your attention"}, "demo")
        self.assertEqual(body["status"], "awaiting")
        self.assertEqual(body["label"], "Claude needs your attention")

    def test_notification_label_truncates_long_message(self) -> None:
        body = build_body("idle", {"notification_type": "attention", "message": "y" * 200}, "demo")
        self.assertEqual(body["status"], "awaiting")
        self.assertEqual(len(body["label"]), 60)

    def test_clear_returns_clear_action_only(self) -> None:
        body = build_body("clear", {"prompt": "ignored"}, "demo")
        self.assertEqual(body, {"action": "clear", "id": "demo"})

    def test_transcript_path_is_forwarded_when_present(self) -> None:
        body = build_body("working", {"prompt": "x", "transcript_path": "/tmp/t.jsonl"}, "demo")
        self.assertEqual(body["transcript_path"], "/tmp/t.jsonl")

    def test_transcript_path_absent_when_payload_lacks_it(self) -> None:
        body = build_body("working", {"prompt": "x"}, "demo")
        self.assertNotIn("transcript_path", body)

    def test_clear_does_not_forward_transcript_path(self) -> None:
        body = build_body("clear", {"transcript_path": "/tmp/t.jsonl"}, "demo")
        self.assertEqual(body, {"action": "clear", "id": "demo"})


class LastAssistantEndsWithQuestionTests(unittest.TestCase):
    def test_missing_path_returns_false(self) -> None:
        self.assertFalse(last_assistant_ends_with_question(None))
        self.assertFalse(last_assistant_ends_with_question(""))
        self.assertFalse(last_assistant_ends_with_question("   "))

    def test_nonexistent_file_returns_false(self) -> None:
        self.assertFalse(last_assistant_ends_with_question("/nonexistent/transcript.jsonl"))

    def test_trailing_question_mark_detected(self) -> None:
        path = _write_transcript([_assistant_text_line("Does this look right?")])
        try:
            self.assertTrue(last_assistant_ends_with_question(str(path)))
        finally:
            path.unlink()

    def test_no_trailing_question_returns_false(self) -> None:
        path = _write_transcript([_assistant_text_line("Done.")])
        try:
            self.assertFalse(last_assistant_ends_with_question(str(path)))
        finally:
            path.unlink()

    def test_skips_user_entries_and_uses_last_assistant(self) -> None:
        path = _write_transcript([
            _assistant_text_line("First answer?"),
            {"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "follow-up"}]}},
            _assistant_text_line("Ok, done."),
        ])
        try:
            self.assertFalse(last_assistant_ends_with_question(str(path)))
        finally:
            path.unlink()

    def test_benign_closer_not_treated_as_question(self) -> None:
        closers = ("What's next?",)
        for text in ("What's next?", "what's next?", "Done. What's next?"):
            path = _write_transcript([_assistant_text_line(text)])
            try:
                self.assertFalse(last_assistant_ends_with_question(str(path), closers), text)
            finally:
                path.unlink()

    def test_non_matching_closer_still_awaits(self) -> None:
        closers = ("What's next?",)
        path = _write_transcript([_assistant_text_line("Which option do you prefer?")])
        try:
            self.assertTrue(last_assistant_ends_with_question(str(path), closers))
        finally:
            path.unlink()

    def test_empty_assistant_text_is_ignored_for_latest(self) -> None:
        path = _write_transcript([
            _assistant_text_line("Real question?"),
            _assistant_text_line("   "),
        ])
        try:
            self.assertTrue(last_assistant_ends_with_question(str(path)))
        finally:
            path.unlink()

    def test_malformed_json_lines_are_skipped(self) -> None:
        fd, raw_path = tempfile.mkstemp(suffix=".jsonl")
        os.close(fd)
        path = Path(raw_path)
        with path.open("w", encoding="utf-8") as f:
            f.write("not json\n")
            f.write(json.dumps(_assistant_text_line("Proceed?")) + "\n")
        try:
            self.assertTrue(last_assistant_ends_with_question(str(path)))
        finally:
            path.unlink()


class LoadConfigTests(unittest.TestCase):
    def test_missing_file_raises(self) -> None:
        missing = Path(tempfile.gettempdir()) / "definitely-not-there-12345.json"
        self.assertFalse(missing.exists())
        with self.assertRaises(OSError):
            load_config(missing)

    def test_valid_config_returns_loaded_dict(self) -> None:
        cfg = {"server_port": 9077, "projects_root": "d:/projects", "benign_closers": []}
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump(cfg, f)
            path = Path(f.name)
        try:
            self.assertEqual(load_config(path), cfg)
        finally:
            path.unlink()

    def test_empty_object_is_valid(self) -> None:
        # The hook tolerates a missing-keys config and falls back to defaults —
        # tray-created config files may have extra widget-only keys the hook
        # ignores, and hook-relevant keys may be absent entirely.
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump({}, f)
            path = Path(f.name)
        try:
            self.assertEqual(load_config(path), {})
        finally:
            path.unlink()

    def test_malformed_json_raises(self) -> None:
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            f.write("{ not json }")
            path = Path(f.name)
        try:
            with self.assertRaises(json.JSONDecodeError):
                load_config(path)
        finally:
            path.unlink()

    def test_non_object_json_raises(self) -> None:
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump(["not", "an", "object"], f)
            path = Path(f.name)
        try:
            with self.assertRaises(ValueError):
                load_config(path)
        finally:
            path.unlink()


class WidgetUrlTests(unittest.TestCase):
    def test_defaults_to_9077_when_config_empty(self) -> None:
        self.assertEqual(widget_url({}), f"http://127.0.0.1:{DEFAULT_PORT}/api/status")

    def test_uses_configured_port(self) -> None:
        self.assertEqual(widget_url({"server_port": 9100}), "http://127.0.0.1:9100/api/status")

    def test_coerces_string_port(self) -> None:
        self.assertEqual(widget_url({"server_port": "9200"}), "http://127.0.0.1:9200/api/status")

    def test_falls_back_to_default_on_garbage_port(self) -> None:
        self.assertEqual(widget_url({"server_port": "not-a-port"}), f"http://127.0.0.1:{DEFAULT_PORT}/api/status")


class DefaultConfigPathTests(unittest.TestCase):
    def test_windows_uses_appdata_and_bundle_identifier(self) -> None:
        fake_appdata = r"C:\FakeRoaming"
        with mock.patch("claude_hook.sys.platform", "win32"), \
             mock.patch.dict(os.environ, {"APPDATA": fake_appdata}, clear=False):
            path = default_config_path()
        expected_tail = Path(BUNDLE_IDENTIFIER) / "config.json"
        self.assertEqual(path, Path(fake_appdata) / expected_tail)

    def test_macos_uses_application_support(self) -> None:
        with mock.patch("claude_hook.sys.platform", "darwin"):
            path = default_config_path()
        self.assertIn("Library/Application Support", path.as_posix())
        self.assertTrue(path.as_posix().endswith(f"{BUNDLE_IDENTIFIER}/config.json"))

    def test_linux_honours_xdg_config_home(self) -> None:
        fake_xdg = "/opt/fake/xdg"
        with mock.patch("claude_hook.sys.platform", "linux"), \
             mock.patch.dict(os.environ, {"XDG_CONFIG_HOME": fake_xdg}, clear=False):
            path = default_config_path()
        self.assertEqual(path, Path(fake_xdg) / BUNDLE_IDENTIFIER / "config.json")


if __name__ == "__main__":
    unittest.main()
