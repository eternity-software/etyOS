---
name: Bug report
about: Report a bug in YAWC
title: "[bug] "
labels: bug
assignees: ""
---

## Summary

Describe the problem clearly.

## Environment

- OS:
- Desktop/session:
- GPU/driver:
- Commit or branch:
- Backend: nested `winit` / standalone
- App involved, if any:

## Steps to Reproduce

1.
2.
3.

## Expected Behavior

What should have happened?

## Actual Behavior

What happened instead?

## Logs or Screenshots

Add anything useful here.

Helpful logs for standalone sessions:

```bash
tail -300 ~/.local/state/yawc/session.log
journalctl --user -b -u xdg-desktop-portal -u xdg-desktop-portal-wlr --no-pager
```

If the bug involves config or hotkeys, include the relevant lines from:

```text
~/.config/yawc/config
```
