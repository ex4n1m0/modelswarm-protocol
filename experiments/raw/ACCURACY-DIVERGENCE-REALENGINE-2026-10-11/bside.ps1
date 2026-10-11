$ErrorActionPreference = 'Stop'
$root = 'C:\Users\Admin\Desktop\ModelSwarm-ACC-v7'
$prompts = @{
  'p1-short' = 'modelswarm synthetic pass-1 short prompt: summarize the following sequence of labeled measurements in one sentence; '
  'p1-medium' = 'modelswarm synthetic pass-1 medium prompt: the quick brown fox jumps over the lazy dog while five wizards judge twelve identical quanta of hedged, speculative, exactly verified inference; summarize the trade-offs in two sentences and keep every number intact; '
  'p1-long' = 'modelswarm synthetic pass-1 long prompt: the quick brown fox jumps over the lazy dog while five wizards judge twelve identical quanta of hedged, speculative, exactly verified inference; the sluggish amber whale drifts beneath the eager cormorant as seven clerks audit fifteen mismatched ledgers of drained, verified, exactly cancelled speculation; summarize the trade-offs in three sentences, keep every number intact, and end with the word done; '
  'p1-xl' = 'modelswarm synthetic pass-1 extra-long prompt: the quick brown fox jumps over the lazy dog while five wizards judge twelve identical quanta of hedged, speculative, exactly verified inference; the sluggish amber whale drifts beneath the eager cormorant as seven clerks audit fifteen mismatched ledgers of drained, verified, exactly cancelled speculation; the nimble jade heron strides beside the placid ox while nine archivists index nineteen divergent folios of queued, verified, exactly reconciled attribution; summarize the trade-offs in four sentences, keep every number intact, and end with the word done; '
}
function Sha16([string]$s) {
  $sha = [System.Security.Cryptography.SHA256]::Create()
  $bytes = $sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($s))
  ($bytes[0..7] | ForEach-Object { $_.ToString('x2') }) -join ''
}
foreach ($round in 1..2) {
  foreach ($id in @('p1-short','p1-medium','p1-long','p1-xl')) {
    # FRESH process per prompt: cache-cold, the E0-comparable discipline.
    $proc = Start-Process -FilePath "$root\engine\llama-server.exe" -ArgumentList @('-m',"$root\model\SmolLM2-135M-Instruct-Q4_K_M.gguf",'--host','127.0.0.1','--port','8905','-c','2048') -WindowStyle Hidden -PassThru
    $healthy = $false
    for ($i=0; $i -lt 120; $i++) {
      try { $null = Invoke-RestMethod -Uri 'http://127.0.0.1:8905/health' -TimeoutSec 2; $healthy = $true; break } catch { Start-Sleep -Milliseconds 500 }
    }
    if (-not $healthy) { Write-Output "ERR $id round$round no-health"; Stop-Process -Id $proc.Id -Force; continue }
    $body = @{ prompt = $prompts[$id]; temperature = 0.0; seed = 7; max_tokens = 48; stream = $false } | ConvertTo-Json
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $resp = Invoke-RestMethod -Uri 'http://127.0.0.1:8905/v1/completions' -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 120
    $sw.Stop()
    $text = $resp.choices[0].text
    Write-Output ("RESULT round=$round instance=b$round prompt=$id sha16=$(Sha16 $text) chars=$($text.Length) ms=$([math]::Round($sw.Elapsed.TotalMilliseconds,1))")
    Stop-Process -Id $proc.Id -Force
    Start-Sleep -Milliseconds 500
  }
}
