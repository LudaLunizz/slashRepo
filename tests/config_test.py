#!/usr/bin/env python3
"""Tabs file: parsing and every edit the window and the --add/--remove/--sort flags make."""
import os
import subprocess
import sys
import tempfile
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

# Input checks.
assert rs.repo_name("https://github.com/storytold/pdfcraft.git/") == "storytold/pdfcraft"
assert rs.repo_name(" cli/cli ") == "cli/cli"
for bad in ("cli", "a/b/c", "a b/c", "--x", "a/b;rm"):
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

print("config tests ok")
