function Assert-TsfImageArchitecture {
    param([string]$Path, [ValidateSet('x64','x86')][string]$Architecture)
    $stream=[IO.File]::OpenRead([IO.Path]::GetFullPath($Path))
    $reader=[IO.BinaryReader]::new($stream)
    try {
        if($stream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5a4d){throw 'Not a PE image'}
        $stream.Position=0x3c; $offset=$reader.ReadUInt32()
        if($offset -gt $stream.Length-6){throw 'Invalid PE header offset'}
        $stream.Position=$offset
        if($reader.ReadUInt32() -ne 0x4550){throw 'Invalid PE signature'}
        $expected=if($Architecture -eq 'x64'){0x8664}else{0x14c}
        if($reader.ReadUInt16() -ne $expected){throw "TSF architecture mismatch: expected $Architecture"}
    } finally {$reader.Dispose()}
}
