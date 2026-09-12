param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Actions)

$ErrorActionPreference = "Stop"
$dir = ".cache/dev"
New-Item -ItemType Directory -Force -Path $dir | Out-Null

function Show-Nodes([string]$path) {
    $raw = Get-Content $path -Raw -Encoding UTF8
    $ms = [regex]::Matches($raw, '\{"attributes":\{([^{}]*)\}')
    $out = foreach ($m in $ms) {
        $a = $m.Groups[1].Value
        $type = [regex]::Match($a, '"type":"([^"]*)"').Groups[1].Value
        $key = [regex]::Match($a, '"key":"([^"]*)"').Groups[1].Value
        $text = [regex]::Match($a, '"text":"([^"]*)"').Groups[1].Value
        $click = [regex]::Match($a, '"clickable":"([^"]*)"').Groups[1].Value
        $bounds = [regex]::Match($a, '"bounds":"([^"]*)"').Groups[1].Value
        if ($key -ne "" -or $text -ne "" -or $click -eq "true") {
            [pscustomobject]@{ type = $type; key = $key; text = $text; click = $click; bounds = $bounds }
        }
    }
    $lines = $out | Sort-Object { [int]([regex]::Match($_.bounds, '\]\[(\d+)').Groups[1].Value) }, { [int]([regex]::Match($_.bounds, '\[(\d+)').Groups[1].Value) } |
        ForEach-Object { "{0}|{1}|{2}|{3}|{4}" -f $_.type, $_.key, $_.text, $_.click, $_.bounds }
    $lines | Set-Content -Encoding UTF8 "$dir/$([System.IO.Path]::GetFileNameWithoutExtension($path)).txt"
    "===== $path -> $($lines.Count) nodes ====="
}

foreach ($action in $Actions) {
    $parts = $action -split ':', 2
    $verb = $parts[0]
    $arg = if ($parts.Count -gt 1) { $parts[1] } else { "" }
    switch ($verb) {
        "dump" {
            hdc shell uitest dumpLayout -p "/data/local/tmp/$arg.json" | Out-Null
            hdc file recv "/data/local/tmp/$arg.json" "$dir/$arg.json" | Out-Null
            "===== $arg ====="
            Show-Nodes "$dir/$arg.json"
        }
        "click" {
            $xy = $arg -split ','
            hdc shell uitest uiInput click $xy[0] $xy[1] | Out-Null
            "click $arg"
        }
        "input" {
            $p = $arg -split ',', 3
            hdc shell uitest uiInput inputText $p[0] $p[1] $p[2] | Out-Null
            "input $($p[0]),$($p[1]) $($p[2])"
        }
        "key" {
            hdc shell uitest uiInput keyEvent $arg | Out-Null
            "key $arg"
        }
        "swipe" {
            $p = $arg -split ','
            hdc shell uitest uiInput swipe $p[0] $p[1] $p[2] $p[3] $p[4] | Out-Null
            "swipe $arg"
        }
        "up" { hdc shell uitest uiInput swipe 1560 1500 1560 600 400 | Out-Null; "swipe up" }
        "down" { hdc shell uitest uiInput swipe 1560 600 1560 1500 400 | Out-Null; "swipe down" }
        "back" { hdc shell uitest uiInput keyEvent 2 | Out-Null; "back" }
        "home" { hdc shell uitest uiInput keyEvent 1 | Out-Null; "home" }
        "sleep" { Start-Sleep -Milliseconds ([int]$arg); "sleep $arg" }
        "shell" { hdc shell $arg }
        "start" {
            hdc shell aa force-stop com.huawei.hmsapp.appgallery | Out-Null
            Start-Sleep -Seconds 1
            hdc shell aa start -a MainAbility -b com.huawei.hmsapp.appgallery | Out-Null
            "start appgallery"
        }
        default { "unknown action: $action" }
    }
}
