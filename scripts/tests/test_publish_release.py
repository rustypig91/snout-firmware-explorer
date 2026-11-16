"""Exercise draft publication and failures without contacting GitHub."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "publish-release.sh"
FAKE_GH = r'''
import json
import os
from pathlib import Path
import sys

args = sys.argv[1:]
with Path(os.environ["GH_TEST_LOG"]).open("a") as output:
    output.write(json.dumps(args) + "\n")
operation = args[1]
state = Path(os.environ["GH_TEST_STATE"])
if operation == os.environ.get("GH_TEST_FAIL"):
    sys.exit(1)
if operation == "view":
    if not state.exists():
        sys.exit(1)
    print("true" if state.read_text() == "draft" else "false")
elif operation == "create":
    assert "--draft" in args and "--verify-tag" in args
    state.write_text("draft")
elif operation == "upload":
    assert state.read_text() == "draft"
    assert "--clobber" in args
elif operation == "edit":
    assert "--draft=false" in args
    state.write_text("published")
else:
    sys.exit("Unexpected GitHub operation")
'''


@unittest.skipIf(os.name == "nt" or not shutil.which("bash"), "Publication runs on Linux with Bash")
class PublishReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="snout-release-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.assets = self.root / "assets with spaces"
        self.assets.mkdir()
        for name in ["snout.tar.gz", "snout-setup.exe", "snout-screenshot.png", "store-submission.zip"]:
            (self.assets / name).write_bytes(b"asset")
        self.state = self.root / "state"
        self.log = self.root / "calls.jsonl"
        binary = self.root / "bin"
        binary.mkdir()
        gh = binary / "gh"
        gh.write_bytes((f"#!{sys.executable}\n" + FAKE_GH).encode())
        gh.chmod(0o755)
        self.env = dict(
            os.environ,
            PATH=str(binary) + os.pathsep + os.environ["PATH"],
            GH_REPO="example/snout",
            GH_TEST_STATE=str(self.state),
            GH_TEST_LOG=str(self.log),
            GH_TEST_FAIL="",
        )

    def run_release(self):
        return subprocess.run(
            ["bash", str(SCRIPT), "v1.0.0", str(self.assets)],
            env=self.env,
            capture_output=True,
            text=True,
            check=False,
        )

    def calls(self):
        if not self.log.exists():
            return []
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_publication_follows_all_draft_uploads(self):
        result = self.run_release()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        self.assertEqual([args[1] for args in calls], ["view", "create", "upload", "edit"])
        for path in self.assets.iterdir():
            self.assertIn(str(path), calls[2])
        self.assertEqual(self.state.read_text(), "published")

    def test_upload_failure_leaves_private_draft(self):
        self.env["GH_TEST_FAIL"] = "upload"
        self.assertNotEqual(self.run_release().returncode, 0)
        self.assertEqual(self.state.read_text(), "draft")
        self.assertNotIn("edit", [args[1] for args in self.calls()])

    def test_retry_reuses_draft_and_replaces_incomplete_uploads(self):
        self.state.write_text("draft")
        self.assertEqual(self.run_release().returncode, 0)
        self.assertEqual([args[1] for args in self.calls()], ["view", "upload", "edit"])
        self.assertEqual(self.state.read_text(), "published")

    def test_create_failure_cannot_upload_or_publish(self):
        self.env["GH_TEST_FAIL"] = "create"
        self.assertNotEqual(self.run_release().returncode, 0)
        self.assertEqual([args[1] for args in self.calls()], ["view", "create"])
        self.assertFalse(self.state.exists())

    def test_published_release_is_never_modified(self):
        self.state.write_text("published")
        self.assertNotEqual(self.run_release().returncode, 0)
        self.assertEqual([args[1] for args in self.calls()], ["view"])
        self.assertEqual(self.state.read_text(), "published")

    def test_missing_or_empty_assets_do_not_create_release(self):
        for path in self.assets.iterdir():
            path.unlink()
        for missing in [False, True]:
            if missing:
                self.assets.rmdir()
            self.assertNotEqual(self.run_release().returncode, 0)
            self.assertEqual(self.calls(), [])

    def test_publication_error_leaves_draft_for_retry(self):
        self.env["GH_TEST_FAIL"] = "edit"
        self.assertNotEqual(self.run_release().returncode, 0)
        self.assertEqual(self.state.read_text(), "draft")


if __name__ == "__main__":
    unittest.main()
