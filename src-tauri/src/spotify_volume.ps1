# Prints Spotify's own volume (0-100) read from the Spotify app on this PC,
# through Windows UI Automation. Prints nothing if it can't be found.
# Read only: Spotify ignores writes to this slider, so it is never set here.
$ErrorActionPreference = 'Stop'
try {
  Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
  $AE = [System.Windows.Automation.AutomationElement]
  $p = Get-Process Spotify -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  if (-not $p) { exit 0 }
  $root = $AE::FromHandle($p.MainWindowHandle)
  $cond = New-Object System.Windows.Automation.AndCondition(
    (New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, [System.Windows.Automation.ControlType]::Slider)),
    (New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, 'Change volume')))
  # Spotify builds its accessibility tree only once something asks for it, so
  # the first call can need a few tries.
  foreach ($i in 1..10) {
    $e = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
    if ($e) {
      $rv = $e.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern)
      $max = $rv.Current.Maximum
      if ($max -gt 0) { [int][math]::Round(100 * $rv.Current.Value / $max) }
      exit 0
    }
    Start-Sleep -Milliseconds 500
  }
} catch { }
