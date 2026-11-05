# The Windows twin of tools/fake_agent.sh: a stand-in for Claude Code in
# `vy-dev serve --agent-cmd` with no network and no credentials.
#
#   vy-dev serve --agent-cmd "powershell -NoProfile -ExecutionPolicy Bypass -File tools\fake_agent.ps1"
#
# The mission arrives on stdin, one stream-json event per line goes to
# stdout, src/lib.rs in the working directory is edited. Records its
# arguments in fake-agent-args.txt and the mission in fake-agent-mission.txt.
# A mission containing SLOW sleeps 30 s in a child process (its pid in
# fake-agent-sleep.pid) first; one containing FAIL ends with exit 3.
# ASCII only: Windows PowerShell 5.1 reads a script without a BOM as ANSI,
# so non-ASCII text is written with \u escapes in the JSON.
$ErrorActionPreference = 'Stop'
$utf8 = New-Object System.Text.UTF8Encoding($false)
[Console]::InputEncoding = $utf8
[Console]::OutputEncoding = $utf8

[IO.File]::WriteAllText((Join-Path $PWD 'fake-agent-args.txt'), (($args -join "`n") + "`n"), $utf8)
$mission = [Console]::In.ReadToEnd()
[IO.File]::WriteAllText((Join-Path $PWD 'fake-agent-mission.txt'), $mission, $utf8)

function Say([string]$line) {
    [Console]::Out.WriteLine($line)
    [Console]::Out.Flush()
}

Say '{"type":"system","subtype":"init","cwd":".","session_id":"fake","model":"fake-agent","tools":["Read","Edit","Bash"]}'
Say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Jag läser sidan och ändrar texten."}]}}'
Say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"src/lib.rs"}}]}}'
Say '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"..."}]}}'

if ($mission.Contains('SLOW')) {
    $sleeper = Start-Process -FilePath 'powershell' -ArgumentList '-NoProfile', '-Command', 'Start-Sleep -Seconds 30' -NoNewWindow -PassThru
    [IO.File]::WriteAllText((Join-Path $PWD 'fake-agent-sleep.pid'), "$($sleeper.Id)`n", $utf8)
    $sleeper.WaitForExit()
}

Say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"src/lib.rs","old_string":"Inga klick","new_string":"Agenten var har. Inga klick"}}]}}'
$lib = Join-Path $PWD 'src\lib.rs'
$text = [IO.File]::ReadAllText($lib, $utf8)
[IO.File]::WriteAllText($lib, $text.Replace('Inga klick', 'Agenten var har. Inga klick'), $utf8)

if ($mission.Contains('FAIL')) {
    Say '{"type":"result","subtype":"error_during_execution","is_error":true,"result":"Något gick fel."}'
    exit 3
}
Say '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Klart: sidan hälsar från agenten."}]}}'
Say '{"type":"result","subtype":"success","is_error":false,"result":"Sidan är ändrad.","total_cost_usd":0}'
