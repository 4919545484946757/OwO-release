param(
    [Parameter(Mandatory=$true)][string]$SeedFile,
    [Parameter(Mandatory=$true)][string]$OutputFile,
    [ValidateSet('full','explicit','initials','mixed','incomplete','typo','prefix')][string]$Kind='full',
    [string]$IdPrefix='case',
    [int]$Maximum=10000
)
$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false)
$rows = [Collections.Generic.List[string]]::new()
$number = 0
function Add-Case([string]$Code,[string]$Text,[string[]]$Tags,[string]$Suffix,[Nullable[int]]$Consumed=$null,[bool]$AssertPreview=$false) {
    $entry = [ordered]@{id=($IdPrefix+'-'+$number.ToString('D4')+'-'+$Suffix);input=$Code;expected=@($Text);tags=@($Tags)}
    if ($null -ne $Consumed) { $entry.consumed = [int]$Consumed }
    if ($AssertPreview) { $entry.segmentation = $Code }
    $rows.Add(($entry | ConvertTo-Json -Depth 8 -Compress))
}
foreach ($line in [IO.File]::ReadLines([IO.Path]::GetFullPath($SeedFile),[Text.Encoding]::UTF8)) {
    if (-not $line.Trim() -or $line.StartsWith('#')) { continue }
    $parts = $line.Split('|')
    if ($parts.Count -ne 3) { throw "Invalid seed: $line" }
    $text = $parts[0]; $syllables = @($parts[1].Split(' ',[StringSplitOptions]::RemoveEmptyEntries)); $category = $parts[2]
    if (-not $text -or -not $syllables.Count -or @($syllables | Where-Object { $_ -notmatch '^[a-z]+$' }).Count) { throw "Invalid text/reading: $line" }
    $number++
    if ($number -gt $Maximum) { break }
    $code = $syllables -join ''
    $tags = @('human_seed',$category,$Kind)
    switch ($Kind) {
        full { Add-Case $code $text $tags 'full' }
        explicit { Add-Case ($syllables -join "'") $text $tags 'explicit' $null $true }
        initials { if ($syllables.Count -ge 2) { Add-Case (($syllables | ForEach-Object { $_.Substring(0,1) }) -join '') $text ($tags+@('ambiguous')) 'initials' } }
        mixed {
            if ($syllables.Count -ge 2) {
                $firstFull = @($syllables[0]) + @($syllables | Select-Object -Skip 1 | ForEach-Object { $_.Substring(0,1) })
                $lastFull = @($syllables | Select-Object -SkipLast 1 | ForEach-Object { $_.Substring(0,1) }) + @($syllables[-1])
                Add-Case ($firstFull -join '') $text ($tags+@('ambiguous')) 'first-full'
                Add-Case ($lastFull -join '') $text ($tags+@('ambiguous')) 'last-full'
                if ($syllables.Count -ge 3) {
                    $alternating = for ($index=0;$index -lt $syllables.Count;$index++) { if ($index%2) { $syllables[$index].Substring(0,1) } else { $syllables[$index] } }
                    Add-Case ($alternating -join '') $text ($tags+@('ambiguous')) 'alternating'
                }
            }
        }
        incomplete { if ($syllables[-1].Length -ge 2) { Add-Case $code.Substring(0,$code.Length-1) $text ($tags+@('ambiguous')) 'tail' } }
        typo {
            $eligible = @($syllables | Where-Object { $_.Length -ge 3 -and $_[-1] -ne $_[-2] })
            if ($eligible.Count) {
                $target = $eligible[0]; $offset = $code.IndexOf($target) + $target.Length - 2
                $chars = $code.ToCharArray(); $swap = $chars[$offset]; $chars[$offset]=$chars[$offset+1]; $chars[$offset+1]=$swap
                Add-Case ($chars -join '') $text ($tags+@('transposition','ambiguous')) 'swap'
                Add-Case $code.Remove($offset,1) $text ($tags+@('deletion','ambiguous')) 'delete'
            }
        }
        prefix {
            if ($syllables.Count -ge 3 -and $text.Length -eq $syllables.Count) {
                foreach ($count in @(1,2)) {
                    $prefix = ($syllables | Select-Object -First $count) -join ''
                    Add-Case $code $text.Substring(0,$count) ($tags+@('partial_consumption','ambiguous')) ('prefix-'+$count) $prefix.Length
                }
            }
        }
    }
}
if (-not $rows.Count) { throw 'No cases generated.' }
$destination = [IO.Path]::GetFullPath($OutputFile)
if ([IO.File]::Exists($destination)) { throw 'Refusing to overwrite an existing dataset.' }
[void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
[IO.File]::WriteAllLines($destination,$rows,$utf8)
Write-Output ('Generated '+$rows.Count+' cases: '+$destination)
