---
name: verify
description: Check a 모눈 change before committing it — rustfmt, clippy, xtask layers and assets, the tests of the touched crates, a release build and a screenshot of the running app. Use before every commit, and whenever asked to verify, test or check a change (확인, 검증, 테스트).
context: fork
agent: verifier
---

# Verify a 모눈 change

What the caller wants checked: $ARGUMENTS

If that is empty, work it out from the change itself: `git status --short` and `git diff HEAD`
(or the last commit, `git show --stat HEAD`, when the tree is clean).

## Steps

1. Read the change far enough to know which crates it touches and what should look different in
   the app.
2. Run `.claude/skills/verify/verify.ps1` from the repository root in the PowerShell tool, one phase
   per call with `timeout: 600000` (a phase can take minutes; they share one target directory, so
   never two at once):
   - `& ./.claude/skills/verify/verify.ps1 -Phase check`
   - `& ./.claude/skills/verify/verify.ps1 -Phase test` (packages with uncommitted changes; for a
     change already committed add `-Base HEAD~1`; `-Crates monun-functions,monun-ui-egui` picks
     them by hand)
   - `& ./.claude/skills/verify/verify.ps1 -Phase app -Setup '<method>={json}', …` with setup
     commands that bring the change on screen. Skip this phase when nothing in the change can
     show in the app (docs, xtask, `.claude`, tests only) and say so.
3. For each FAIL, open the log the summary names and find the cause (see the verifier's rules).
4. Read the screenshot (`target\verify\app.png`) and say whether it shows what the change should.

## Setup commands for the app phase

Each `-Setup` item is `method` or `method={json}`; the app runs them in order before the screenshot.

| Goal | Setup |
|---|---|
| Type into a cell | `cell.set={"cell":"A1","input":"한글 ABC 123"}` |
| Select, then act on the selection | `selection.set={"cell":"B2"}`, `home.bold` |
| A formula | `cell.set={"cell":"A2","input":"=NUMBERSTRING(1234,1)"}` |
| Open a dialog | `ui.dialog={"name":"…"}` |
| Click or type like the user | `ui.click={"x":120,"y":40}`, `ui.text={"text":"abc"}`, `ui.key={"key":"Enter"}` |
| Switch to the Korean interface | `app.language.set={"language":"ko"}` |

To find a command or its parameters, run the app phase with `-KeepOpen`, then
`target\release\monun-cli.exe send 7979 engine.commands` (all commands with their parameters) or
`ui.inspect` (what is on screen and where), and close it with `… send 7979 app.quit`.

## Report

```
verify: PASS | FAIL | BLOCKED
- check: …
- test: … (packages, counts; known failures named as such)
- app: … — screenshot C:\Project\monun\target\verify\app.png: what it shows
Problems (only on FAIL): file:line — the error — the likely cause in the change
```
