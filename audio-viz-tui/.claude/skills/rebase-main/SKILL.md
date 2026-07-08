---
name: rebase-main
description: Rebase the current branch onto origin/main, stashing any uncommitted changes first and restoring them after.
---

Rebase the current branch onto `origin/main`, preserving any uncommitted work.

## What to do

1. Run `git stash` to save any uncommitted changes (it's a no-op if the tree is clean).

2. Run `git rebase origin/main`.

3. If the rebase **succeeds**, run `git stash pop` (skip if step 1 produced "No local changes to save").

4. If the rebase **has conflicts**:
   - List the conflicting files.
   - Do NOT attempt to resolve them automatically.
   - Tell the user which files conflict and ask how they'd like to proceed.

5. Report the final result: new HEAD commit, and whether stashed changes were restored.
