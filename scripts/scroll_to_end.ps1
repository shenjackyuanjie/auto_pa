param([int]$MaxRounds = 40, [string]$X1 = "1560", [string]$Y1 = "1500", [string]$Y2 = "600")

$ErrorActionPreference = "Stop"
$dir = ".cache/dev"
New-Item -ItemType Directory -Force -Path $dir | Out-Null

$seen = [System.Collections.Generic.HashSet[string]]::new()
$stable = 0
$start = Get-Date

for ($i = 1; $i -le $MaxRounds; $i++) {
    hdc shell uitest uiInput swipe $X1 $Y1 $X1 $Y2 400 | Out-Null
    Start-Sleep -Milliseconds 900
    hdc shell uitest dumpLayout -p /data/local/tmp/loop.json | Out-Null
    hdc file recv /data/local/tmp/loop.json "$dir/loop.json" | Out-Null
    $raw = Get-Content "$dir/loop.json" -Raw -Encoding UTF8
    $visible = 0
    $added = 0
    foreach ($m in [regex]::Matches($raw, '\{"attributes":\{([^{}]*)\}')) {
        $a = $m.Groups[1].Value
        if ($a -notmatch '"key":"app_name"') { continue }
        $t = [regex]::Match($a, '"text":"([^"]*)"').Groups[1].Value
        if ($t -eq "") { continue }
        $visible++
        if ($seen.Add($t)) { $added++ }
    }
    if ($added -eq 0) { $stable++ } else { $stable = 0 }
    "{0,2}: visible={1,3} added={2,3} total={3,4} stable={4}" -f $i, $visible, $added, $seen.Count, $stable
    if ($stable -ge 2) { "reached end after $i rounds"; break }
}

"elapsed: {0:n1}s" -f ((Get-Date) - $start).TotalSeconds
$seen | Sort-Object | Set-Content -Encoding UTF8 "$dir/scroll_names.txt"
"names: $($seen.Count) -> $dir/scroll_names.txt"
