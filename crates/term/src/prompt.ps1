# macaroni's PowerShell integration, run with -NoExit -EncodedCommand after the user's profile.
#
# It wraps the prompt in place: before the prompt it reports the last command's exit code (OSC 133
# D, only after a command ran), the working directory (OSC 7) and where the prompt starts (A);
# after it, where the command line starts (B). PowerShell has no hook between reading a command
# and running it, so PSReadLine's Enter sends C (the output starts), and only for a non-blank
# line, as bash's PS0 does. Written for Windows PowerShell 5 too: no `e escapes.

$global:__mcr_prompt = $function:prompt
$global:__mcr_ran = $false

function global:prompt {
    $ok = $?
    $code = $global:LASTEXITCODE
    $e = [char]27
    $bell = [char]7
    $out = ''
    if ($global:__mcr_ran) {
        $exit = if ($ok) { 0 } elseif ($code) { $code } else { 1 }
        $out += "$e]133;D;$exit$bell"
        $global:__mcr_ran = $false
    }
    if ($PWD.Provider.Name -eq 'FileSystem') {
        $out += "$e]7;file://$env:COMPUTERNAME/$($PWD.ProviderPath -replace '\\', '/')$bell"
    }
    $out += "$e]133;A$bell"
    $out + (& $global:__mcr_prompt) + "$e]133;B$bell"
}

if (Get-Module PSReadLine) {
    Set-PSReadLineKeyHandler -Key Enter -BriefDescription 'AcceptLine (macaroni)' -ScriptBlock {
        $line = $null
        $cursor = $null
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
        if ($line.Trim()) {
            $global:__mcr_ran = $true
            [Console]::Write("$([char]27)]133;C$([char]7)")
        }
    }
}
