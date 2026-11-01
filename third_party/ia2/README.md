# IAccessible2 interface declarations

Upstream: https://github.com/LinuxA11y/IAccessible2

Pinned commit: `c9ae003d9c85eb707716928de97e055f5b77189c`.
License: BSD-3-Clause; retain `LICENSE` in source and binary distributions.

`api/` contains the four unchanged upstream IDL files used for the base
IAccessible2 interface. `include/owo/compatibility/ia2/` contains the Windows SDK
MIDL-generated C++ declarations, with the same inherited COM ABI on x64/x86.
No proxy DLL is built or installed. The application supplies its existing COM
provider and marshaling. OwO reads only role, state, native window and unique ID;
it does not read accessibility names, values, attributes or document text.
The generator removes duplicate BSTR/HWND user-marshal prototypes: these already
exist with authoritative SAL annotations in the imported SDK headers. Interface
declarations and vtables remain unchanged; no analysis warning is disabled.

Regenerate using the installed SDK/compiler:

```powershell
.\scripts\generate-ia2-headers.ps1
```

The normal build and runtime use the checked-in declarations, never `env/`.
Raw downloads and GitHub version metadata are retained in `env/compatibility/ia2/`.
Download hashes, exact URLs and purposes are listed in the compatibility report.
