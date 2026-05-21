---
name: git-workflow
description: |
  Use when the user wants to commit, branch, rebase, manage Git history,
  or resolve merge conflicts. Provides a safe checklist and the right shell commands.
trigger_patterns: ["commit", "branch", "rebase", "merge conflict", "git", "pull request"]
allowed_tools: [shell, file_read, file_write]
---

# Git Workflow

## When to use

The user wants to interact with a Git repository: commit changes, create or
rename branches, rebase, resolve merge conflicts, prepare a PR.

## Safety rules

Before destructive operations, **always**:

1. Run `git status` to confirm working-tree state.
2. Confirm with the user before:
   - `git reset --hard`, `git push --force`, `git rebase`, `git checkout --`
   - Anything that drops uncommitted work or rewrites pushed history.
3. **Never** use `--no-verify` to skip pre-commit hooks.

## Standard commit flow

1. `git status` — see what changed.
2. `git diff --staged` (or `git diff` for unstaged) — review.
3. `git add <file>...` — stage explicit files; avoid `git add -A` unless you
   have just reviewed everything.
4. Draft a conventional-commit-style message:
   `<type>(<scope>): <subject>` (under 70 chars).
   Types: `feat`, `fix`, `chore`, `docs`, `refactor`, `test`, `perf`, `ci`.
5. `git commit -m "..."` — commit.
6. Show the user the resulting `git log -1`.

## Conflict resolution

1. `git status` → look for "both modified" files.
2. For each conflicted file: locate the `<<<<<<<`, `=======`, `>>>>>>>`
   markers, edit to keep the desired content, remove all markers.
3. `git add <file>` after editing.
4. `git status` again to confirm no conflicts remain.
5. `git commit` (if mid-merge) or `git rebase --continue` (if mid-rebase).

## Reference

- Branch from main: `git switch -c <branch> main`
- Squash last N commits: `git reset --soft HEAD~N && git commit`
- Recover lost commit: `git reflog` then `git checkout <sha>`
- Compare branch to main: `git log main..HEAD --oneline`
