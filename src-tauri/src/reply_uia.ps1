# Sends $env:CREST_TEXT to the Claude desktop app without focusing it, through
# Windows UI Automation: fill the "Prompt" box, then press "Send".
$ErrorActionPreference = 'Stop'
# Rust reads this output as UTF-8; Claude's labels can hold dashes and the like.
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
try {
  Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
  $AE = [System.Windows.Automation.AutomationElement]
  $CT = [System.Windows.Automation.ControlType]
  $TS = [System.Windows.Automation.TreeScope]
  $root = $AE::FromHandle([IntPtr][int64]$env:CREST_HWND)
  function Find($type, $name) {
    $c = New-Object System.Windows.Automation.AndCondition(
      (New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $type)),
      (New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, $name)))
    $root.FindFirst($TS::Descendants, $c)
  }
  # Pressing a button makes Claude pull its window to the front for a moment.
  # Move the window fully off-screen first so there is nothing to see; the
  # island puts it back afterwards (it remembered where it was). SetWindowPos,
  # not UI Automation's Move: Claude is often maximised, and Move refuses to
  # move a maximised window.
  function Hide {
    try {
      if (-not ('CrestWin' -as [type])) {
        Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class CrestWin { [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags); }'
      }
      # SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE
      [void][CrestWin]::SetWindowPos([IntPtr][int64]$env:CREST_HWND, [IntPtr]::Zero, -30000, -30000, 0, 0, 0x15)
    } catch {}
  }
  # The first request after Claude starts switches on its accessibility tree,
  # which can take a few seconds to fill in; later requests take ~40 ms.
  $edit = $null
  for ($i = 0; $i -lt 60 -and -not $edit; $i++) { $edit = Find $CT::Edit 'Prompt'; if (-not $edit) { Start-Sleep -Milliseconds 100 } }
  if (-not $edit) { Write-Output 'fail:no prompt box'; exit 0 }
  # Claude's own "next step" buttons: pressing the one with this exact label is
  # what a click in the app does, so the message box is never touched. Search
  # the whole window by exact name (cheap: the match runs inside Claude), and
  # take the match nearest the message box, so a sidebar session with the same
  # title is never pressed.
  # Claude updates a window in the background slowly: its buttons can show up
  # in the accessibility tree ~15 s after the island already has the question.
  # So for a real choice keep looking for up to 25 s (each look takes ~40 ms).
  $boxTop = $edit.Current.BoundingRectangle.Top
  $waitMs = if ($env:CREST_CHOICE -eq '1') { 25000 } else { 0 }
  $look = [Diagnostics.Stopwatch]::StartNew()
  do {
    $choice = $root.FindAll($TS::Descendants, (New-Object System.Windows.Automation.AndCondition(
        (New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $CT::Button)),
        (New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, $env:CREST_TEXT))))) |
      Where-Object { $_.Current.IsEnabled -and $_.Current.BoundingRectangle.Left -ge $edit.Current.BoundingRectangle.Left - 200 } |
      Sort-Object { [Math]::Abs($boxTop - $_.Current.BoundingRectangle.Bottom) } | Select-Object -First 1
    if ($choice) { break }
    Start-Sleep -Milliseconds 250
  } while ($look.ElapsedMilliseconds -lt $waitMs)
  $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
  $scope = $edit
  for ($i = 0; $i -lt 5; $i++) { $p = $walker.GetParent($scope); if (-not $p) { break }; $scope = $p }
  if ($choice) {
    Hide
    $choice.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    Write-Output 'ok'; exit 0
  }
  $vp = $edit.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
  $draft = $vp.Current.Value.Trim()
  # Focus the box inside Claude's page (not the window): its Send button only
  # lights up for input the page sees as typed into the focused box.
  try { $edit.SetFocus() } catch {}
  Start-Sleep -Milliseconds 100
  $vp.SetValue($env:CREST_TEXT)
  Start-Sleep -Milliseconds 250
  # A window in the background renders slowly: give Send up to 6 s to light up.
  # Search only around the message box (the whole window has dozens of buttons
  # and scanning it every poll is slow), and give up well inside Rust's timeout.
  $isBtn = New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $CT::Button)
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $btn = $null
  while ($sw.ElapsedMilliseconds -lt 8000) {
    # "Send" mid-turn; it may be worded differently when idle.
    $btn = $scope.FindAll($TS::Descendants, $isBtn) | Where-Object { $_.Current.Name -match '^(Send|Submit)' } | Select-Object -First 1
    if ($btn -and $btn.Current.IsEnabled) { break }
    Start-Sleep -Milliseconds 100
  }
  if (-not $btn -or -not $btn.Current.IsEnabled) {
    # Undo what we typed, so a fallback can't double it.
    $vp.SetValue($draft)
    Write-Output 'fail:no send button'; exit 0
  }
  Hide
  $btn.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
  # Put back whatever you had half-typed before the reply went out.
  if ($draft) { Start-Sleep -Milliseconds 500; $vp.SetValue($draft) }
  Write-Output 'ok'
} catch { Write-Output ('fail:' + $_.Exception.Message) }
