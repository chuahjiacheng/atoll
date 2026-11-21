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
  $page = $null
  for ($i = 0; $i -lt 60 -and -not $page; $i++) {
    $page = $root.FindFirst($TS::Descendants, (New-Object System.Windows.Automation.PropertyCondition($AE::AutomationIdProperty, 'RootWebArea')))
    if (-not $page) { Start-Sleep -Milliseconds 100 }
  }
  # The Claude app shows one session at a time, and it may not be the one that
  # asked (you were watching another session, or a chat in the Claude tab,
  # when you minimised it). Open the asking session first, so the answer can't
  # land in the wrong place; put back what you were looking at afterwards.
  $session = "$env:CREST_SESSION".Trim()
  $backTab = $null
  $back = $null
  $Invoke = [System.Windows.Automation.InvokePattern]::Pattern
  # The app's "Claude" (chats) / "Code" switch at the top.
  function Tab($name) { Find $CT::RadioButton $name }
  function TabOn($name) {
    $t = Tab $name
    if (-not $t) { return $false }
    try { return $t.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Current.IsSelected } catch { return $false }
  }
  function OnTab {
    foreach ($n in 'Code', 'Claude') { if (TabOn $n) { return $n } }
    return $null
  }
  # The session or chat on screen: its header holds a "<title>, rename session"
  # button. Only a Code-tab session counts (a chat can share the title).
  function IsOpen($title) { (TabOn 'Code') -and [bool](Find $CT::Button ($title + ', rename session')) }
  # The page is named "<title> - Claude Code" (or "<title> - Claude" for a chat).
  # (The window holds more than one page; the others are unnamed.)
  function OpenTitle {
    $pages = $root.FindAll($TS::Descendants, (New-Object System.Windows.Automation.PropertyCondition($AE::AutomationIdProperty, 'RootWebArea')))
    foreach ($p in $pages) {
      $n = $p.Current.Name
      foreach ($tail in ' - Claude Code', ' - Claude') {
        if ($n -and $n.EndsWith($tail, [StringComparison]::Ordinal) -and $n.Length -gt $tail.Length) { return $n.Substring(0, $n.Length - $tail.Length) }
      }
    }
    return $null
  }
  # A sidebar entry is a button named "<title>", or "<title> <state>" while it
  # is busy ("... Running"). Exact names first; otherwise the entry with the
  # shortest tail. Two entries alike = can't tell which: none.
  function SidebarEntry($title) {
    $buttons = @($root.FindAll($TS::Descendants, (New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $CT::Button))))
    $hits = @($buttons | Where-Object {
        $n = $_.Current.Name
        $n -and ($n -ceq $title -or ($n.StartsWith($title + ' ', [StringComparison]::Ordinal) -and $n.Length - $title.Length -le 24)) -and $_.Current.IsEnabled
      } | Sort-Object { $_.Current.Name.Length })
    if ($hits.Count -eq 0) { return $null }
    if ($hits.Count -gt 1 -and $hits[0].Current.Name.Length -eq $hits[1].Current.Name.Length) { return $null }
    return $hits[0]
  }
  # Wait up to $ms for $test to hold.
  function WaitFor([scriptblock]$test, $ms) {
    $w = [Diagnostics.Stopwatch]::StartNew()
    while ($w.ElapsedMilliseconds -lt $ms) { if (& $test) { return $true }; Start-Sleep -Milliseconds 100 }
    return [bool](& $test)
  }
  # Back to the tab and session (or chat) you were looking at.
  function GoBack {
    if (-not $backTab) { return }
    try {
      if ($backTab -ne 'Code') {
        $t = Tab $backTab
        if ($t) { $t.GetCurrentPattern($Invoke).Invoke(); [void](WaitFor { TabOn $backTab } 3000) }
      }
      if ($back -and -not ($backTab -eq 'Code' -and $back -ceq $session) -and (OpenTitle) -cne $back) {
        $b = SidebarEntry $back
        if ($b) { $b.GetCurrentPattern($Invoke).Invoke() }
      }
    } catch {}
  }
  if ($session -and $env:CREST_PROBE -eq '1') {
    $e = SidebarEntry $session
    Write-Output ('probe:tab=' + (OnTab) + '|open=' + (OpenTitle) + '|isOpen=' + (IsOpen $session) + '|entry=' + $(if ($e) { $e.Current.Name } else { '' }))
    exit 0
  }
  if ($session -and -not (IsOpen $session)) {
    $backTab = OnTab
    $back = OpenTitle
    if (-not (TabOn 'Code')) {
      $t = Tab 'Code'
      if (-not $t) { Write-Output 'fail:session:no Code tab'; exit 0 }
      $t.GetCurrentPattern($Invoke).Invoke()
      if (-not (WaitFor { TabOn 'Code' } 3000)) { GoBack; Write-Output 'fail:session:Code tab did not open'; exit 0 }
    }
    $entry = $null
    [void](WaitFor { $script:entry = SidebarEntry $session; [bool]$script:entry } 3000)
    if (-not $entry) { GoBack; Write-Output 'fail:session:not in the sidebar'; exit 0 }
    $entry.GetCurrentPattern($Invoke).Invoke()
    if (-not (WaitFor { IsOpen $session } 6000)) {
      # Something else opened: go back to where you were, answer nothing.
      GoBack
      Write-Output 'fail:session:did not open'; exit 0
    }
  }
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
    Start-Sleep -Milliseconds 600
    GoBack
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
  Start-Sleep -Milliseconds 600
  GoBack
  Write-Output 'ok'
} catch { Write-Output ('fail:' + $_.Exception.Message) }
