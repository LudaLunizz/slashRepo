#!/usr/bin/env python3
"""Tabs file: parsing and every edit the window and the --add/--remove/--sort flags make."""
import json
import os
import subprocess
import sys
import tempfile
sys.dont_write_bytecode = True  # importing bin/repo-status must not leave a cache file in bin/
from pathlib import Path
import importlib.util
from importlib.machinery import SourceFileLoader

here = os.path.dirname(os.path.abspath(__file__))
script = os.path.join(here, "..", "bin", "repo-status")
loader = SourceFileLoader("repo_status", script)
rs = importlib.util.module_from_spec(importlib.util.spec_from_loader("repo_status", loader))
loader.exec_module(rs)

old = ["me/a  # keep this comment\n", "me/b\n", "\n", "[ArtCraft]\n", "sort = stars\n", "st/x\n", "st/y\n"]

# Old files without tabs still work; comments are ignored.
assert rs.parse(["me/a\n", "# note\n", "me/b # x\n"]) == {"My repos": {"repos": ["me/a", "me/b"], "sort": "mine"}}
assert rs.parse(old) == {"My repos": {"repos": ["me/a", "me/b"], "sort": "mine"},
                         "ArtCraft": {"repos": ["st/x", "st/y"], "sort": "stars"}}

# Add: lands at the end of its own tab, twice is once, unknown tab is made.
lines = rs.add_repo(old, "me/c", "My repos")
assert lines[2] == "me/c\n" and lines[0] == old[0], lines
assert rs.add_repo(lines, "ME/C", "My repos") == lines
lines = rs.add_repo(lines, "new/z", "Friends")
assert rs.parse(lines)["Friends"]["repos"] == ["new/z"]
assert rs.parse(rs.add_repo([], "me/a", "My repos")) == {"My repos": {"repos": ["me/a"], "sort": "mine"}}

# Remove: one tab or all; a tab with its repos.
both = rs.add_repo(old, "me/a", "ArtCraft")
assert "me/a" not in rs.parse(rs.remove_repo(both, "me/a", "ArtCraft"))["ArtCraft"]["repos"]
assert "me/a" in rs.parse(rs.remove_repo(both, "me/a", "ArtCraft"))["My repos"]["repos"]
assert all("me/a" not in t["repos"] for t in rs.parse(rs.remove_repo(both, "me/a")).values())
assert list(rs.parse(rs.remove_tab(old, "ArtCraft"))) == ["My repos"]
assert list(rs.parse(rs.remove_tab(old, "My repos"))) == ["ArtCraft"]

# Own order: move keeps the line (and its comment), stops at the edges.
moved = rs.move_repo(old, "me/b", "My repos", -1)
assert moved[:2] == ["me/b\n", old[0]], moved
assert rs.move_repo(old, "me/a", "My repos", -1) == old
assert rs.move_repo(old, "st/y", "ArtCraft", 1) == old

# Sort: one line per tab, mine removes it.
assert rs.parse(rs.set_sort(old, "ArtCraft", "forks"))["ArtCraft"]["sort"] == "forks"
assert sum("sort" in l for l in rs.set_sort(old, "ArtCraft", "forks")) == 1
assert rs.parse(rs.set_sort(old, "ArtCraft", "mine"))["ArtCraft"]["sort"] == "mine"
assert rs.parse(rs.set_sort(old, "My repos", "stars"))["My repos"] == {"repos": ["me/a", "me/b"], "sort": "stars"}
res = {"a/1": {"stars": 5, "forks": 9}, "a/2": {"stars": 50, "forks": 1}}
assert rs.ordered(["a/1", "a/2", "a/3"], "stars", res) == ["a/2", "a/1", "a/3"]
assert rs.ordered(["a/1", "a/2", "a/3"], "forks", res) == ["a/1", "a/2", "a/3"]
assert rs.ordered(["a/2", "a/1"], "mine", res) == ["a/2", "a/1"]

# Card titles open the repo page, also before (or without) a check.
assert rs.web_url("cli/cli") == "https://github.com/cli/cli"
assert rs.web_url("gitlab.com/g/sub/p") == "https://gitlab.com/g/sub/p"
assert rs.web_url("codeberg.org/o/r") == "https://codeberg.org/o/r"

# Input checks.
assert rs.repo_name("https://github.com/storytold/pdfcraft.git/") == "storytold/pdfcraft"
assert rs.repo_name(" cli/cli ") == "cli/cli"
for bad in ("cli", "a/b/c", "a b/c", "--x", "a/b;rm"):
    assert rs.repo_name(bad) is None, bad
assert rs.repo_name("https://gitlab.com/gitlab-org/cli.git") == "gitlab.com/gitlab-org/cli"
assert rs.repo_name("gitlab.com/group/sub/project") == "gitlab.com/group/sub/project"
assert rs.repo_name("https://git.example.org/team/app/") == "git.example.org/team/app"
assert rs.repo_name("https://codeberg.org/forgejo/forgejo.git") == "codeberg.org/forgejo/forgejo"
assert rs.repo_name("codeberg.org/a/b/c") is None  # Codeberg has no subgroups
for bad in ("gitlab.com/onlygroup", "https://gitlab.com/a/b/-/issues", "gitlab.com/a b/c"):
    assert rs.repo_name(bad) is None, bad
assert rs.tab_name("  ArtCraft ") == "ArtCraft" and rs.tab_name("a]b") is None and rs.tab_name("") is None

# The flags write the real file (a temporary HOME here).
with tempfile.TemporaryDirectory() as home:
    env = dict(os.environ, HOME=home)
    run = lambda *a: subprocess.run([sys.executable, "-I", script, *a], env=env, capture_output=True, text=True)
    cfg = os.path.join(home, ".config", "slashrepo", "repos")
    os.makedirs(os.path.dirname(cfg))
    Path(cfg).write_text("me/a\n")
    assert run("--add", "https://github.com/st/x", "ArtCraft").returncode == 0
    assert run("--sort", "ArtCraft", "stars").returncode == 0
    assert run("--add", "nonsense").returncode != 0
    assert Path(cfg).read_text() == "me/a\n\n[ArtCraft]\nsort = stars\nst/x\n", Path(cfg).read_text()
    assert run("--tabs").stdout == "[My repos]\n  me/a\n[ArtCraft]  sort = stars\n  st/x\n"
    assert run("--remove-tab", "ArtCraft").returncode == 0 and Path(cfg).read_text() == "me/a\n\n"

# GitLab: the token goes only to GITLAB_HOST, and a GitLab project fills the same card as GitHub.
sent = []
class Resp:
    def __init__(self, data): self.data = data
    def __enter__(self): return self
    def __exit__(self, *a): pass
    def read(self): return json.dumps(self.data).encode()
def fake_urlopen(req, timeout):
    sent.append((req.full_url, req.get_header("Authorization")))
    if req.full_url.endswith("/graphql"):
        return Resp({"data": {"project": {
            "webUrl": "https://gitlab.com/g/p", "lastActivityAt": "2026-01-01T00:00:00Z", "starCount": 7,
            "forksCount": 2, "repository": {"rootRef": "main"},
            "releases": {"nodes": [{"tagName": "v1", "releasedAt": "2026-01-01T00:00:00Z", "upcomingRelease": False}]},
            "issues": {"count": 1, "nodes": [{"iid": "4", "title": "bug", "createdAt": "2026-01-01T00:00:00Z",
                                              "labels": {"nodes": [{"title": "bug"}]}}]},
            "mergeRequests": {"nodes": [{"iid": "9", "title": "fix", "draft": False, "author": {"username": "x"},
                                         "headPipeline": {"status": "FAILED"}}]}}}})
    if "/pipelines" in req.full_url:
        return Resp([{"status": "success"}])
    return Resp({"commits": [{}, {}]})
rs.urllib.request.urlopen = fake_urlopen
os.environ.update(GITLAB_TOKEN="secret")
os.environ.pop("GITLAB_HOST", None)
card = rs.check("gitlab.com/g/p")
assert card["ci"] == "SUCCESS" and card["stars"] == 7 and card["unreleased"] == 2, card
assert "MR !9 has failing checks" in card["todo"] and "2 commit(s) on main since v1: release when ready" in card["todo"]
assert rs.summary_lines(card)[1].startswith("MR !9 fix") and rs.light(card) == "orange"
assert all(auth == "Bearer secret" for _, auth in sent), sent
sent.clear()
rs.check("git.example.org/g/p")
assert sent and all(auth is None for _, auth in sent), sent
os.environ.pop("GITLAB_TOKEN")

# Codeberg: same card; CODEBERG_TOKEN goes to codeberg.org only (and never to GitLab hosts).
class CbResp(Resp):
    def __init__(self, data, total=None):
        super().__init__(data)
        self.headers = {"X-Total-Count": total} if total else {}
def cb_urlopen(req, timeout):
    sent.append((req.full_url, req.get_header("Authorization")))
    u = req.full_url.split("/api/v1/repos/o/r", 1)[1]
    if u == "":
        return CbResp({"html_url": "https://codeberg.org/o/r", "updated_at": "2026-01-01T00:00:00+01:00",
                       "stars_count": 3, "forks_count": 1, "default_branch": "main"})
    if u.startswith("/releases"):
        return CbResp([{"tag_name": "v2", "published_at": "2026-01-01T00:00:00+01:00", "prerelease": False}])
    if u.startswith("/commits?sha=main&not=v2"):
        return CbResp([{}], total="4")
    if u.startswith("/issues"):
        return CbResp([{"number": 5, "title": "bug", "created_at": "2026-01-01T00:00:00+01:00",
                        "labels": [{"name": "bug"}]}], total="12")
    if u.startswith("/pulls"):
        return CbResp([{"number": 6, "title": "fix", "draft": False, "user": {"login": "x"}, "head": {"sha": "abc"}}])
    if u == "/commits/abc/status":
        return CbResp({"state": "failure"})
    return CbResp({"state": "success"})  # the default branch
rs.urllib.request.urlopen = cb_urlopen
os.environ.update(CODEBERG_TOKEN="cb", GITLAB_TOKEN="gl")
sent.clear()
card = rs.check("codeberg.org/o/r")
assert card["ci"] == "SUCCESS" and card["stars"] == 3 and card["issue_total"] == 12 and card["unreleased"] == 4, card
assert "PR #6 has failing checks" in card["todo"] and "12 open issue(s) to triage" in card["todo"], card["todo"]
assert sent and all(auth == "token cb" for _, auth in sent), sent
os.environ.pop("CODEBERG_TOKEN"); os.environ.pop("GITLAB_TOKEN")

print("config tests ok")
