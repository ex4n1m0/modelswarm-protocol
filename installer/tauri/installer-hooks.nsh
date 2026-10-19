; installer-hooks.nsh — update-safety for the engine directory
; (live incident 2026-10-19, roadmap row "Update-safety: engine-file locks
; during install").
;
; Any llama-server.exe still running FROM THE INSTALLED ENGINE DIR holds
; ggml-base.dll & friends open, and NSIS extraction stalls at
; "Error opening file for writing: <INSTDIR>\engine\ggml-base.dll" until
; someone clicks Retry. The app's OWN supervised engine dies cleanly with
; the app (the updater closes it first), so the stall only happens when an
; orphaned engine process survives — seen live when a benchmark harness's
; cargo parent died and left its llama-server child behind.
;
; The kill is deliberately PATH-SCOPED: only processes whose image runs
; from $INSTDIR\engine. Never kill by image name alone — the owner may run
; an unrelated llama.cpp build on the same machine.

!macro NSIS_HOOK_PREINSTALL
  nsExec::ExecToLog 'powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "Get-Process llama-server -ErrorAction SilentlyContinue | Where-Object { $$_.Path -like ''$INSTDIR\engine*'' } | Stop-Process -Force -ErrorAction SilentlyContinue"'
  ; handles release asynchronously — give the OS a beat before extraction
  Sleep 500
!macroend
