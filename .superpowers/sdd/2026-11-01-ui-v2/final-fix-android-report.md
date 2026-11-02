# Final fix wave, Android

All three items held against the code.

- A1: HomeTab Need.PcForgotPhone fired removeForgottenPcs + ScanActivity with no confirmation. Now: honest copy (APP_NAME, em dash as specified); the button opens a destructive RelayDialog (title/body from pure `forgetPrompt(names)` in ui/home/ForgetPc.kt, names the flagged PCs read at tap time). Confirm runs removeForgottenPcs (re-reads flagged peers) then ScanActivity; Cancel/dismiss does nothing.
- D1: extracted `isOpenable` and suspend `openSavedFile(ctx, uri)` (fileState on IO, getType, content:// only, ActivityNotFound/Security messages) into ui/timeline/OpenEntry.kt; Timeline uses it with its double-tap guard; Home Recent uses it with the same guard and a new SnackbarHost ("File moved or deleted"). Behaviour change: Home Recent no longer opens non-file rows (links/clips), matching Timeline (previously a link row with a uri would ACTION_VIEW it).
- D3: openAppDetails now goes through openSettings (runCatching + toast).

Tests: new ForgetPcTest (4: single name, many names, blank/duplicate, isOpenable). Honest note: helper and tests were written together, so there is no separate RED run. The UI wiring has no unit test.
Gates: :app:testDebugUnitTest BUILD SUCCESSFUL; :app:compileReleaseKotlin SUCCESSFUL; :app:lintRelease SUCCESSFUL.
